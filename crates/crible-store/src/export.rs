// SPDX-License-Identifier: Apache-2.0

//! Export of an index: the live data of one generation as a portable
//! `replace_all` batch.
//!
//! References: `docs/04-donnees-et-ingestion.md` ("Export, instantanés,
//! restauration"), `docs/format-lot-v1.md` ("Lot d'export"),
//! `docs/05-api.md` (error codes), ADR-0012 (public batch format), ADR-0061
//! (manifest of an exported batch), `docs/08-invariants-et-tests.md` (INV-01).
//!
//! # What an export is
//!
//! A batch directory `<exchange>/outbox/<index>-g<G>/` whose `manifest.json`
//! declares the mode `replace_all`: the documents it lists are the whole
//! content of the index at generation G, and it carries no deletion file. Every
//! row is a business row — `doc_id`, `ord`, `text`, the declared fields and the
//! reference vector — so the batch is readable without Crible (DuckDB,
//! pyarrow) and re-attachable into an empty index (**INV-08**).
//!
//! No internal identifier leaves the engine (**INV-01**): `iid` is only used to
//! address the rows of the epoch, never written to a column, a file name, the
//! manifest, an error message or a log line. Every diagnostic names the
//! business key `doc_id#ord`.
//!
//! # One generation, and the writer keeps running
//!
//! The export holds the published [`Snapshot`] for its whole run and reads the
//! rows of that snapshot's epoch through a **separate read-only connection** and
//! one SQLite read transaction (`docs/04`: "L'écrivain continue"). The
//! generation is read inside that transaction and must be the generation of the
//! snapshot: the document rows, the passage rows and the segment references then
//! all come from one committed state. The vectors are read from the segments the
//! snapshot names, which are immutable files of the same epoch. The database runs
//! in WAL (`records.rs`), so a reader never blocks the writer, and the store's
//! own writable connection is never used here. A commit that lands between the
//! snapshot and the first read is retried on a fresh snapshot, at most three
//! times, and then reported as [`ErrorCode::Overloaded`] (ADR-0061, décision 5).
//!
//! # Contiguous `ord` values
//!
//! `docs/format-lot-v1.md` requires the `ord` of a document to run from 0
//! without a hole, and `BatchReader::validate` enforces it. The document is the
//! unit of update (`docs/04`, "Le découpage : qui fait quoi"): a changed text
//! tombstones **all** the old passages of its document and adds a fresh suite
//! from 0, and a deletion tombstones the whole document
//! ([`crate::attach`], `Plan::retire_passages`). The live passages of a live
//! document therefore always form a complete suite, and the export renumbers
//! nothing. A store whose rows break that property — only reachable through
//! [`crate::generation::GenerationChange`], which lets a caller tombstone any
//! iid — is refused with [`ErrorCode::Internal`] instead of producing a batch
//! that no reader accepts.
//!
//! # Existing export
//!
//! The batch is written under a staging name and renamed into place, and an
//! existing `<index>-g<G>` directory is **refused**:
//! [`ErrorCode::InvalidRequest`], exactly as `GenerationStore::create` refuses
//! an existing index (ADR-0061, décision 2). An export therefore never destroys
//! the copy an operator already moved out, nor a half-written one, silently.
//!
//! The staging directory is `outbox/.tmp-<batch_id>-<pid>-<n>-<nanos>`, the name
//! `docs/04` and `docs/format-lot-v1.md` describe: two exports of the same
//! batch never write in the same place, and an export only ever removes a
//! staging directory it created itself. A staging directory left behind by a
//! crash is inert — it is never renamed — and nothing removes it: the recovery
//! of `docs/02-architecture.md` covers the index, not `outbox`, so an operator
//! cleans it by hand.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow::array::{ArrayRef, FixedSizeBinaryArray, StringArray, UInt32Array};
use arrow::datatypes::{DataType, Field, Schema as ArrowSchema, SchemaRef};
use arrow::record_batch::RecordBatch;
use crible_core::batch::{
    BATCH_FORMAT, BATCH_FORMAT_VERSION, BatchManifest, BatchMode, BatchModel, Chunker, Counts,
    Producer, Template, VectorDtype,
};
use crible_core::config::{FieldLevel, IndexConfig, ReferencePrecision, Schema};
use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::Generation;
use crible_core::model::ModelConfig;
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;
use rusqlite::Connection;
use serde_json::{Map, Value};

use crate::batch_reader::{DEFAULT_BATCH_ROWS, parquet_type, vector_width};
use crate::compact::EpochVectors;
use crate::generation::{
    GenerationStore, RECORDS_FILE_NAME, Snapshot, epoch_directory, segments_directory,
};
use crate::online::{document_fields, field_columns, hash_file, store_vector};
use crate::records;

/// Sub-directory of the exchange root that holds the exports (`docs/02`).
pub(crate) const OUTBOX_DIRECTORY: &str = "outbox";

/// Passages per part of an export (`docs/04`: "en parties d'au plus 1 M
/// passages").
///
/// This is the limit of the batch format, not the one of a segment
/// ([`crate::compact::MAX_SEGMENT_PASSAGES`]): the two numbers agree today, and
/// a change of one must not silently move the other.
pub(crate) const MAX_PART_PASSAGES: usize = 1_000_000;

/// Prefix of the staging directory of an export, inside `outbox/`.
const STAGING_PREFIX: &str = ".tmp-";

/// Attempts at reading one generation before the export reports the writer as
/// too fast to follow.
///
/// A commit that lands between [`GenerationStore::snapshot`] and the first read
/// of the reference database makes the two generations disagree. That is a
/// transient race, not a corrupt state, so the export takes a fresh snapshot and
/// starts again a few times. The number is an internal mitigation, outside the
/// contract (ADR-0061, décision 5).
const MAX_GENERATION_ATTEMPTS: usize = 3;

/// Distinguishes two exports of the same batch inside one process.
static STAGING_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Bytes of one row group of an exported Parquet file.
///
/// A part holds up to [`MAX_PART_PASSAGES`] rows, so the writer must not buffer
/// a row group per file: 64 MiB keeps the memory of one write bounded whatever
/// the size of the export.
const MAX_ROW_GROUP_BYTES: usize = 64 * 1024 * 1024;

/// Producer name of an export: the engine itself produced the batch.
///
/// `producer.name`, `producer.host` and `producer.version` are the values
/// ADR-0061 (décision 4) fixes for an exported batch; `version` is
/// `CARGO_PKG_VERSION`, the binary that wrote the batch.
const PRODUCER_NAME: &str = "crible";

/// Producer host of an export.
///
/// `docs/format-lot-v1.md` and ADR-0036 let `producer.host` carry "un
/// identifiant de la machine, ou du producteur". The engine does not read the
/// host name — `docs/07` lists no crate for it — so the export names itself, the
/// way the online lot of ADR-0055 does.
const PRODUCER_HOST: &str = "crible-export";

/// Name and version the export declares for `chunker` and `template`.
///
/// The export splits no document and composes no text: it copies the encoder
/// inputs the producers already stored. The batch therefore declares that it
/// applies none of the two transformations, which is the convention the online
/// lot of ADR-0055 also uses. ADR-0061 (décision 1) fixes these two values for
/// an exported batch and states their meaning: "no chunking or recomposition by
/// the producer of this batch". What the original producers did is history,
/// kept in `batches.manifest`, and deliberately not carried by the export.
const NO_TRANSFORM: &str = "none";
const NO_TRANSFORM_VERSION: &str = "1";

/// Report of one export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReport {
    /// Generation the export read: the generation of the snapshot it held.
    pub generation: Generation,
    /// Batch directory actually written: `<exchange>/outbox/<index>-g<G>/`.
    pub directory: PathBuf,
    /// Document rows written.
    pub documents: u64,
    /// Passage rows written.
    pub passages: u64,
}

/// Writes a `replace_all` batch of the live data of the generation `store`
/// currently publishes, into `<exchange>/outbox/<index>-g<G>/`.
///
/// `index_config` names the index and gives its schema, its reference precision
/// and its frozen settings; `model_config` fills the model member of the
/// manifest, which the manifest format requires (`docs/format-lot-v1.md`) and
/// which `IndexConfig` does not carry (`dims`, `normalize`, and the encoder
/// fields of an `onnx` model).
///
/// `exchange` is the `[paths] exchange` root of `crible.toml`
/// (`docs/07-stack-et-deploiement.md`); `outbox/` is created under it when it
/// does not exist yet.
///
/// The store is taken by shared reference: the export only reads, through the
/// published snapshot and a read-only connection, so the writer is not blocked.
///
/// # Errors
///
/// - [`ErrorCode::InvalidRequest`] when `<exchange>/outbox/<index>-g<G>/` already
///   exists: an export never overwrites a batch;
/// - [`ErrorCode::Overloaded`] when the index commits faster than the export can
///   read one generation: after `MAX_GENERATION_ATTEMPTS` attempts, each on a
///   fresh snapshot, the writer had moved every time. The cause is a retryable
///   race with the writer, not a corrupt state (ADR-0061, décision 5), and
///   `docs/05-api.md` attaches `Retry-After` to the code at the API layer;
/// - [`ErrorCode::Internal`] when the store holds a state no commit could have
///   produced (a live document without a live passage, a hole in the `ord` of a
///   document, a passage whose document row is gone) or when the disk fails.
pub fn export(
    store: &GenerationStore,
    index_config: &IndexConfig,
    model_config: &ModelConfig,
    exchange: &Path,
) -> Result<ExportReport, CribleError> {
    export_with_part_size(
        store,
        index_config,
        model_config,
        exchange,
        MAX_PART_PASSAGES,
    )
}

/// [`export`], with the part limit as a parameter so a test can reach the
/// rollover without a fixture of a million rows.
fn export_with_part_size(
    store: &GenerationStore,
    index_config: &IndexConfig,
    model_config: &ModelConfig,
    exchange: &Path,
    part_passages: usize,
) -> Result<ExportReport, CribleError> {
    if part_passages == 0 {
        return Err(internal(
            "an export part must hold at least one passage".to_owned(),
        ));
    }
    let outbox = exchange.join(OUTBOX_DIRECTORY);
    fs::create_dir_all(&outbox)
        .map_err(|error| io_error("create the outbox directory", &outbox, &error))?;

    retrying_stale_generation(|| {
        attempt_export(store, index_config, model_config, &outbox, part_passages)
    })
}

/// Runs one export attempt, again on a fresh snapshot when the writer committed
/// under it, and reports a generation that never settles as
/// [`ErrorCode::Overloaded`] (ADR-0061, décision 5).
///
/// `docs/05-api.md` attaches `Retry-After` to that code at the API layer; the
/// store only returns the code, and the CLI prints it.
fn retrying_stale_generation<T>(
    mut attempt: impl FnMut() -> Result<T, BatchFailure>,
) -> Result<T, CribleError> {
    let mut attempts = 0_usize;
    loop {
        attempts = attempts.saturating_add(1);
        match attempt() {
            Ok(value) => return Ok(value),
            Err(BatchFailure::StaleGeneration(generation))
                if attempts < MAX_GENERATION_ATTEMPTS =>
            {
                // Transient: the writer committed between the snapshot and the
                // first read, so nothing of this attempt is kept.
                tracing::debug!(
                    generation = generation.get(),
                    attempt = attempts,
                    "the writer committed under the export; starting again on a fresh snapshot"
                );
            }
            Err(BatchFailure::StaleGeneration(generation)) => {
                return Err(CribleError::new(
                    ErrorCode::Overloaded,
                    format!(
                        "the index committed a new generation while the export of generation {} \
                         was starting, {MAX_GENERATION_ATTEMPTS} times in a row; retry the export",
                        generation.get()
                    ),
                ));
            }
            Err(BatchFailure::Fatal(error)) => return Err(error),
        }
    }
}

/// Writes the batch of the generation `store` publishes into one staging
/// directory of `outbox`, then renames it into place.
fn attempt_export(
    store: &GenerationStore,
    index_config: &IndexConfig,
    model_config: &ModelConfig,
    outbox: &Path,
    part_passages: usize,
) -> Result<ExportReport, BatchFailure> {
    let name = index_config.name.as_str();
    let snapshot = store.snapshot();
    let generation = snapshot.generation();
    let batch_id = format!("{name}-g{}", generation.get());

    let directory = outbox.join(&batch_id);
    if directory.exists() {
        return Err(BatchFailure::Fatal(invalid(format!(
            "the export batch {} already exists; move it away or export to another exchange \
             root, because an export never overwrites a batch (ADR-0061)",
            directory.display()
        ))));
    }

    // The batch is built under a staging name of its own and renamed once
    // whole, so a failure leaves no half batch under the name an operator would
    // pick up, and a concurrent export of the same batch never shares a staging
    // directory with this one.
    let staging = staging_path(outbox, &batch_id);
    fs::create_dir(&staging).map_err(|error| {
        io_error(
            "create the staging directory of the export (a fresh name is drawn for every export)",
            &staging,
            &error,
        )
    })?;
    let guard = StagingGuard {
        path: staging.clone(),
        armed: true,
    };

    let epoch_dir = epoch_directory(store.index_dir(), snapshot.epoch());
    let counts = write_batch(
        &staging,
        &snapshot,
        &epoch_dir,
        index_config,
        model_config,
        &batch_id,
        part_passages,
    )?;

    // Every file is durable before the directory entry names it: a crash after
    // the rename leaves a batch whose bytes match the blake3 of its manifest,
    // not a published batch with short files.
    crate::segment::sync_directory(&staging)?;
    publish(&staging, &directory)?;
    guard.disarm();
    crate::segment::sync_directory(outbox)?;

    Ok(ExportReport {
        generation,
        directory,
        documents: counts.documents,
        passages: counts.passages,
    })
}

/// Renames the finished staging directory into the name of the batch.
///
/// The pre-check of [`attempt_export`] gives the usual message, but it is not
/// atomic: two exports of the same generation can both pass it, and only one
/// rename wins because the directory entry is created once. The loser is a
/// request this store cannot carry out, the same [`ErrorCode::InvalidRequest`]
/// the pre-check returns (ADR-0061, `docs/04`), not an internal disk fault; the
/// rename is what decides, so the outcome does not depend on the order of the
/// two checks.
///
/// A rename that fails for any other reason stays a disk error, and so does a
/// fault whose target is absent: the fault, not a lost race, is then the cause.
fn publish(staging: &Path, directory: &Path) -> Result<(), BatchFailure> {
    match fs::rename(staging, directory) {
        Ok(()) => Ok(()),
        Err(_error) if directory.exists() => Err(BatchFailure::Fatal(invalid(format!(
            "the export batch {} was created by another export of the same generation while this \
             one was being written; an export never overwrites a batch (ADR-0061)",
            directory.display()
        )))),
        Err(error) => Err(BatchFailure::Fatal(io_error(
            "rename the export into place",
            staging,
            &error,
        ))),
    }
}

/// Draws a staging directory name no other export of `batch_id` can hold.
///
/// The process id separates two processes, the sequence number separates two
/// exports of one process, and the clock separates two processes that reuse a
/// pid after a restart. `fs::create_dir` still decides: a collision fails
/// instead of sharing a directory.
fn staging_path(outbox: &Path, batch_id: &str) -> PathBuf {
    let sequence = STAGING_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    outbox.join(format!(
        "{STAGING_PREFIX}{batch_id}-{}-{sequence}-{nanos}",
        std::process::id()
    ))
}

/// Why one attempt at writing a batch did not produce a batch.
enum BatchFailure {
    /// The writer committed between the snapshot and the first read, so the
    /// snapshot no longer describes the rows the transaction holds. Carries the
    /// generation the attempt was writing.
    StaleGeneration(Generation),
    /// Any other failure, which the caller returns as it is.
    Fatal(CribleError),
}

impl From<CribleError> for BatchFailure {
    fn from(error: CribleError) -> Self {
        Self::Fatal(error)
    }
}

/// Writes `manifest.json` and the Parquet files of one batch into `staging`.
///
/// The read transaction opened on the reference database of the snapshot's
/// epoch stays open for the whole run: every row of the batch comes from that
/// one committed state, and its generation is checked against the snapshot's
/// before the first row is written.
fn write_batch(
    staging: &Path,
    snapshot: &Snapshot,
    epoch_dir: &Path,
    index_config: &IndexConfig,
    model_config: &ModelConfig,
    batch_id: &str,
    part_passages: usize,
) -> Result<Counts, BatchFailure> {
    let records_path = epoch_dir.join(RECORDS_FILE_NAME);

    // A read-only connection, not the writable one of the store: the export
    // never takes the writer's connection and never blocks it.
    let mut connection = records::open_read_only(&records_path)?;
    let transaction = connection.transaction().map_err(|error| {
        records::database_error("open the read transaction of the export", &error)
    })?;

    let meta = records::read_meta(&transaction)?;
    if meta.generation != snapshot.generation() {
        tracing::debug!(
            read = meta.generation.get(),
            snapshot = snapshot.generation().get(),
            "the export read a generation newer than its snapshot"
        );
        return Err(BatchFailure::StaleGeneration(snapshot.generation()));
    }
    let created_at = utc_now(&transaction)?;

    let documents = records::all_documents(&transaction)?;
    // The export copies the encoder input, the document id and the ord from
    // the identity rows, then decodes each whole passage row with
    // `passage_by_iid` below, because the batch writes the declared passage
    // fields. The identity query therefore leaves those fields undecoded
    // (`live_identities(…, false)`), so every passage JSON is decoded once, not
    // twice (`compact.rs` does the same when it does not cluster on them).
    let mut identities = records::live_identities(&transaction, false)?;
    identities.sort_by(|left, right| {
        left.doc_id
            .cmp(&right.doc_id)
            .then(left.ord.cmp(&right.ord))
    });

    let mut vectors = EpochVectors::new(segments_directory(epoch_dir), snapshot.segments());
    let mut writer = BatchWriter::create(staging, index_config, model_config, part_passages)?;
    let mut document_index = 0_usize;
    let mut passage_index = 0_usize;

    while let Some(identity) = identities.get(passage_index) {
        let doc_id = identity.doc_id.as_str();
        // The documents are sorted by `doc_id`, and so are the passages: the
        // two cursors advance together, so neither role is held twice. A
        // document the passages have passed by is alive without a live
        // passage, which no commit can produce.
        if let Some(document) = documents.get(document_index)
            && document.doc_id.as_str() < doc_id
        {
            return Err(BatchFailure::Fatal(internal(format!(
                "document '{}' is alive in generation {} but holds no live passage",
                document.doc_id,
                snapshot.generation().get()
            ))));
        }
        match documents.get(document_index) {
            Some(document) if document.doc_id == doc_id => {
                writer.push_document(document)?;
                document_index = document_index.saturating_add(1);
            }
            _ => {
                return Err(BatchFailure::Fatal(internal(format!(
                    "the passages of document '{doc_id}' are alive in generation {} but its row is \
                     gone",
                    snapshot.generation().get()
                ))));
            }
        }

        let mut expected_ord = 0_u32;
        while let Some(identity) = identities.get(passage_index) {
            if identity.doc_id != doc_id {
                break;
            }
            if identity.ord != expected_ord {
                return Err(BatchFailure::Fatal(internal(format!(
                    "the live passages of document '{doc_id}' do not run from 0 without a hole: \
                     expected ord {expected_ord}, found {}; the batch format requires contiguous \
                     ord values, so no export can be written",
                    identity.ord
                ))));
            }
            let passage = records::passage_by_iid(&transaction, identity.iid)?.ok_or_else(|| {
                tracing::debug!(
                    doc_id = %doc_id,
                    ord = identity.ord,
                    "a live passage has no row in the reference state"
                );
                internal(format!(
                    "passage '{doc_id}#{}' is alive in the snapshot but absent from the reference \
                     state of its epoch",
                    identity.ord
                ))
            })?;
            if passage.doc_id != doc_id || passage.ord != identity.ord {
                tracing::debug!(
                    doc_id = %doc_id,
                    ord = identity.ord,
                    stored_doc_id = %passage.doc_id,
                    stored_ord = passage.ord,
                    "a live passage is stored under another business key"
                );
                return Err(BatchFailure::Fatal(internal(format!(
                    "passage '{doc_id}#{}' is indexed under that key but stored under '{}#{}'",
                    identity.ord, passage.doc_id, passage.ord
                ))));
            }
            let vector = vectors.vector(identity.iid).map_err(|error| {
                tracing::debug!(
                    doc_id = %doc_id,
                    ord = identity.ord,
                    error = %error,
                    "the reference vector of a live passage cannot be read"
                );
                internal(format!(
                    "the reference vector of passage '{doc_id}#{}' cannot be read from the \
                     segments of its epoch",
                    identity.ord
                ))
            })?;
            writer.push_passage(&passage, &vector)?;
            expected_ord = expected_ord.saturating_add(1);
            passage_index = passage_index.saturating_add(1);
        }
    }

    if let Some(document) = documents.get(document_index) {
        return Err(BatchFailure::Fatal(internal(format!(
            "document '{}' is alive in generation {} but holds no live passage",
            document.doc_id,
            snapshot.generation().get()
        ))));
    }

    // The read transaction has now read everything the batch holds; closing it
    // releases the WAL reader, and the writer of the index was never blocked.
    drop(transaction);
    drop(connection);

    Ok(writer.finish(model_config, index_config, batch_id, created_at)?)
}

/// Current UTC time in RFC 3339, read from SQLite.
///
/// `created_at` is obligatory in a manifest and no control reads its value
/// (`docs/format-lot-v1.md`). ADR-0061 (décision 2) makes it the real time of
/// the export, so two exports of one generation are two distinct batches.
/// SQLite already provides the clock the store uses for `batches.applied_at`
/// (`records.rs`), so the export takes it from there rather than adding a date
/// crate, which `docs/07` does not list.
fn utc_now(connection: &Connection) -> Result<String, CribleError> {
    connection
        .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')", [], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|error| records::database_error("read the current UTC time", &error))
}

/// One row of the documents file, waiting for its record batch.
struct PendingDocument {
    doc_id: String,
    doc_hash: String,
    fields: Map<String, Value>,
}

/// One row of the passages file, waiting for its record batch.
struct PendingPassage {
    doc_id: String,
    ord: u32,
    text: String,
    fields: Map<String, Value>,
    vector: Vec<u8>,
}

/// One Parquet file of the batch, open for writing.
struct PartFile {
    /// Name of the file inside the batch directory.
    name: String,
    /// Path of the file, hashed once it is closed.
    path: PathBuf,
    /// Writer of its rows.
    writer: ArrowWriter<File>,
    /// Rows written so far, which is what the part limit counts.
    rows: usize,
}

impl PartFile {
    /// Creates `name` in `directory` with the Arrow schema `schema`.
    fn create(directory: &Path, name: &str, schema: SchemaRef) -> Result<Self, CribleError> {
        let path = directory.join(name);
        let file = File::create(&path)
            .map_err(|error| io_error("create a Parquet file of the export", &path, &error))?;
        let properties = WriterProperties::builder()
            // One bounded row group: the memory held by the writer does not
            // grow with the part, which holds up to `MAX_PART_PASSAGES` rows.
            .set_max_row_group_bytes(Some(MAX_ROW_GROUP_BYTES))
            .build();
        let writer = ArrowWriter::try_new(file, schema, Some(properties))
            .map_err(|error| parquet_error("open a Parquet file of the export", &path, &error))?;
        Ok(Self {
            name: name.to_owned(),
            path,
            writer,
            rows: 0,
        })
    }

    /// Closes the file, makes its bytes durable, and returns its name with its
    /// `blake3:<hex>` digest.
    fn finish(self) -> Result<(String, String), CribleError> {
        let Self {
            name, path, writer, ..
        } = self;
        writer.close().map_err(|error| {
            internal(format!(
                "cannot close the Parquet file {}: {error}",
                path.display()
            ))
        })?;
        // The manifest carries the digest of this file and is written before
        // the batch is renamed: the bytes must survive a crash as well.
        crate::compact::sync_file(&path)?;
        let checksum = format!("blake3:{}", hash_file(&path)?);
        Ok((name, checksum))
    }
}

/// Name of one part of a role: `<role>-NNNN.parquet`, in at least four digits.
fn part_name(role: &str, part: usize) -> String {
    format!("{role}-{part:04}.parquet")
}

/// Streams the rows of the batch into its Parquet files.
///
/// The documents go to one file; the passages roll over to the next part once
/// a part holds `part_passages` rows. A role always has at least one file, so
/// the manifest lists a `documents…` and a `passages…` file even for an empty
/// index, which `docs/format-lot-v1.md` requires.
struct BatchWriter<'a> {
    directory: &'a Path,
    schema: &'a Schema,
    precision: ReferencePrecision,
    /// Width of one stored vector, in bytes, as Arrow wants it.
    width: i32,
    part_passages: usize,
    documents_schema: SchemaRef,
    passages_schema: SchemaRef,
    documents: Option<PartFile>,
    passages: Option<PartFile>,
    passages_part: usize,
    pending_documents: Vec<PendingDocument>,
    pending_passages: Vec<PendingPassage>,
    counts: Counts,
    files: BTreeMap<String, String>,
}

impl<'a> BatchWriter<'a> {
    /// Creates the first part of each role.
    fn create(
        directory: &'a Path,
        index_config: &'a IndexConfig,
        model_config: &ModelConfig,
        part_passages: usize,
    ) -> Result<Self, CribleError> {
        let width = i32::try_from(vector_width(
            model_config.dims,
            index_config.reference_precision,
        ))
        .map_err(|_| {
            internal(format!(
                "a vector of {} dimensions is too wide for the batch format",
                model_config.dims
            ))
        })?;
        let documents_schema = Arc::new(ArrowSchema::new(document_fields(&index_config.schema)));
        let mut fields = vec![
            Field::new("doc_id", DataType::Utf8, false),
            Field::new("ord", DataType::UInt32, false),
            Field::new("text", DataType::Utf8, false),
        ];
        for (name, level, spec) in index_config.schema.fields() {
            if level == FieldLevel::Passage {
                fields.push(Field::new(name, parquet_type(spec.field_type), true));
            }
        }
        fields.push(Field::new(
            "vector",
            DataType::FixedSizeBinary(width),
            false,
        ));
        let passages_schema = Arc::new(ArrowSchema::new(fields));

        let documents = PartFile::create(
            directory,
            &part_name("documents", 1),
            documents_schema.clone(),
        )?;
        let mut writer = Self {
            directory,
            schema: &index_config.schema,
            precision: index_config.reference_precision,
            width,
            part_passages,
            documents_schema,
            passages_schema,
            documents: Some(documents),
            passages: None,
            passages_part: 0,
            pending_documents: Vec::new(),
            pending_passages: Vec::new(),
            counts: Counts {
                documents: 0,
                passages: 0,
                deletions: 0,
            },
            files: BTreeMap::new(),
        };
        writer.open_passages_part()?;
        Ok(writer)
    }

    /// Buffers one document row.
    fn push_document(&mut self, document: &records::DocumentRecord) -> Result<(), CribleError> {
        let fields = match document.fields.as_object() {
            Some(fields) => fields.clone(),
            None => {
                return Err(internal(format!(
                    "the fields of document '{}' are not a JSON object",
                    document.doc_id
                )));
            }
        };
        self.pending_documents.push(PendingDocument {
            doc_id: document.doc_id.clone(),
            doc_hash: document.doc_hash.clone(),
            fields,
        });
        self.counts.documents = self.counts.documents.saturating_add(1);
        if self.pending_documents.len() >= DEFAULT_BATCH_ROWS {
            self.flush_documents()?;
        }
        Ok(())
    }

    /// Buffers one passage row, with its reference vector.
    fn push_passage(
        &mut self,
        passage: &records::PassageRecord,
        vector: &[f32],
    ) -> Result<(), CribleError> {
        let fields = match passage.fields.as_object() {
            Some(fields) => fields.clone(),
            None => {
                return Err(internal(format!(
                    "the fields of passage '{}#{}' are not a JSON object",
                    passage.doc_id, passage.ord
                )));
            }
        };
        self.pending_passages.push(PendingPassage {
            doc_id: passage.doc_id.clone(),
            ord: passage.ord,
            text: passage.text.clone(),
            fields,
            vector: store_vector(vector, self.precision),
        });
        self.counts.passages = self.counts.passages.saturating_add(1);
        if self.pending_passages.len() >= DEFAULT_BATCH_ROWS {
            self.flush_passages()?;
        }
        Ok(())
    }

    /// Writes the pending document rows.
    fn flush_documents(&mut self) -> Result<(), CribleError> {
        if self.pending_documents.is_empty() {
            return Ok(());
        }
        let rows = std::mem::take(&mut self.pending_documents);
        let ids = rows
            .iter()
            .map(|row| row.doc_id.as_str())
            .collect::<Vec<_>>();
        let hashes = rows
            .iter()
            .map(|row| row.doc_hash.as_str())
            .collect::<Vec<_>>();
        let keys = ids.clone();
        let values = rows.iter().map(|row| &row.fields).collect::<Vec<_>>();
        let mut columns: Vec<ArrayRef> = vec![
            Arc::new(StringArray::from(ids)),
            Arc::new(StringArray::from(hashes)),
        ];
        columns.extend(field_columns(
            self.schema,
            FieldLevel::Document,
            &keys,
            &values,
        )?);
        let batch =
            RecordBatch::try_new(self.documents_schema.clone(), columns).map_err(|error| {
                internal(format!("cannot build the documents record batch: {error}"))
            })?;
        let part = self
            .documents
            .as_mut()
            .ok_or_else(|| internal("the documents file is already closed".to_owned()))?;
        part.writer
            .write(&batch)
            .map_err(|error| internal(format!("cannot write {}: {error}", part.name)))?;
        part.rows = part.rows.saturating_add(batch.num_rows());
        Ok(())
    }

    /// Writes the pending passage rows, part by part, never above
    /// `part_passages` in one file.
    ///
    /// A part is filled to its limit and then closed, so a document may span
    /// two parts: `docs/format-lot-v1.md` allows it as long as its `ord` values
    /// continue from one part to the next (ADR-0039).
    fn flush_passages(&mut self) -> Result<(), CribleError> {
        while !self.pending_passages.is_empty() {
            self.open_passages_part()?;
            let filled = self.passages.as_ref().map_or(0, |part| part.rows);
            let remaining = self.part_passages.saturating_sub(filled).max(1);
            let take = self.pending_passages.len().min(remaining);
            let rows: Vec<PendingPassage> = self.pending_passages.drain(..take).collect();
            self.write_passage_rows(&rows)?;
            let full = self
                .passages
                .as_ref()
                .is_some_and(|part| part.rows >= self.part_passages);
            if full {
                self.close_passages_part()?;
            }
        }
        Ok(())
    }

    /// Writes one record batch of passage rows into the open part.
    fn write_passage_rows(&mut self, rows: &[PendingPassage]) -> Result<(), CribleError> {
        let ids = rows
            .iter()
            .map(|row| row.doc_id.as_str())
            .collect::<Vec<_>>();
        let ords = rows.iter().map(|row| row.ord).collect::<Vec<_>>();
        let texts = rows.iter().map(|row| row.text.as_str()).collect::<Vec<_>>();
        let keys = rows
            .iter()
            .map(|row| format!("{}#{}", row.doc_id, row.ord))
            .collect::<Vec<_>>();
        let key_refs = keys.iter().map(String::as_str).collect::<Vec<_>>();
        let values = rows.iter().map(|row| &row.fields).collect::<Vec<_>>();
        let vectors = rows
            .iter()
            .map(|row| Some(row.vector.as_slice()))
            .collect::<Vec<_>>();
        let width = self.width;
        let vector =
            FixedSizeBinaryArray::try_from_sparse_iter_with_size(vectors.into_iter(), width)
                .map_err(|error| internal(format!("cannot build the vector column: {error}")))?;
        let mut columns: Vec<ArrayRef> = vec![
            Arc::new(StringArray::from(ids)),
            Arc::new(UInt32Array::from(ords)),
            Arc::new(StringArray::from(texts)),
        ];
        columns.extend(field_columns(
            self.schema,
            FieldLevel::Passage,
            &key_refs,
            &values,
        )?);
        columns.push(Arc::new(vector));
        let batch =
            RecordBatch::try_new(self.passages_schema.clone(), columns).map_err(|error| {
                internal(format!("cannot build the passages record batch: {error}"))
            })?;
        let part = self
            .passages
            .as_mut()
            .ok_or_else(|| internal("the passages file is already closed".to_owned()))?;
        part.writer
            .write(&batch)
            .map_err(|error| internal(format!("cannot write {}: {error}", part.name)))?;
        part.rows = part.rows.saturating_add(batch.num_rows());
        Ok(())
    }

    /// Opens the next passages part when none is open.
    fn open_passages_part(&mut self) -> Result<(), CribleError> {
        if self.passages.is_some() {
            return Ok(());
        }
        self.passages_part = self.passages_part.saturating_add(1);
        let name = part_name("passages", self.passages_part);
        self.passages = Some(PartFile::create(
            self.directory,
            &name,
            self.passages_schema.clone(),
        )?);
        Ok(())
    }

    /// Closes the open passages part and records its checksum.
    fn close_passages_part(&mut self) -> Result<(), CribleError> {
        let Some(part) = self.passages.take() else {
            return Ok(());
        };
        let (name, checksum) = part.finish()?;
        self.files.insert(name, checksum);
        Ok(())
    }

    /// Closes every file, then writes the manifest of the batch.
    ///
    /// The manifest is filled with the values ADR-0061 fixes for an exported
    /// batch: `chunker` and `template` at `none`/`1` (décision 1), `created_at`
    /// at the real time of the export (décision 2), `index` at the name of the
    /// source index (décision 3), `producer.version` at the binary version
    /// (décision 4). `format_version` stays 1: no key is added.
    fn finish(
        mut self,
        model_config: &ModelConfig,
        index_config: &IndexConfig,
        batch_id: &str,
        created_at: String,
    ) -> Result<Counts, CribleError> {
        self.flush_documents()?;
        self.flush_passages()?;
        self.close_passages_part()?;
        let Some(documents) = self.documents.take() else {
            return Err(internal("the documents file is already closed".to_owned()));
        };
        let (name, checksum) = documents.finish()?;
        self.files.insert(name, checksum);

        let manifest = BatchManifest {
            format: BATCH_FORMAT.to_owned(),
            format_version: BATCH_FORMAT_VERSION,
            batch_id: batch_id.to_owned(),
            index: index_config.name.clone(),
            mode: BatchMode::ReplaceAll,
            created_at,
            producer: Producer {
                name: PRODUCER_NAME.to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                host: PRODUCER_HOST.to_owned(),
            },
            model: BatchModel {
                hf_id: model_config.hf_id.clone(),
                revision: model_config.revision.clone(),
                dims: model_config.dims,
                pooling: model_config.pooling.clone(),
                normalize: model_config.normalize,
                prompt_document: model_config
                    .prompts
                    .as_ref()
                    .map(|prompts| prompts.document.clone()),
                vector_dtype: VectorDtype::from_reference_precision(
                    index_config.reference_precision,
                ),
                runtime: BTreeMap::new(),
            },
            chunker: Chunker {
                name: NO_TRANSFORM.to_owned(),
                version: NO_TRANSFORM_VERSION.to_owned(),
                params: Map::new(),
            },
            template: Template {
                name: NO_TRANSFORM.to_owned(),
                version: NO_TRANSFORM_VERSION.to_owned(),
            },
            counts: self.counts,
            files: self.files,
        };
        let raw = serde_json::to_string(&manifest).map_err(|error| {
            internal(format!(
                "cannot serialize the manifest of the export: {error}"
            ))
        })?;
        let path = self.directory.join("manifest.json");
        crate::segment::write_file_synced(&path, raw.as_bytes())?;
        Ok(self.counts)
    }
}

/// Removes the staging directory unless the export renamed it into place.
struct StagingGuard {
    path: PathBuf,
    armed: bool,
}

impl StagingGuard {
    /// Stops the guard from removing a directory the export succeeded with.
    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if self.armed {
            // Only this export's own staging directory: its name carries the
            // pid, a sequence number and the clock, so no other export shares
            // it. A failed write therefore leaves nothing behind.
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

/// Error raised when the request itself is malformed.
fn invalid(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::InvalidRequest, message)
}

/// Error raised when the process cannot carry out the request.
fn internal(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::Internal, message)
}

/// Wraps an I/O fault with the path it concerns.
fn io_error(action: &str, path: &Path, error: &std::io::Error) -> CribleError {
    internal(format!("cannot {action} {}: {error}", path.display()))
}

/// Wraps a Parquet fault with the path it concerns.
fn parquet_error(action: &str, path: &Path, error: &parquet::errors::ParquetError) -> CribleError {
    internal(format!("cannot {action} {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    // Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
    #![allow(clippy::panic)]

    use std::collections::BTreeMap;

    use crible_core::ids::Iid;

    use super::*;
    use crate::batch_reader::BatchReader;
    use crate::generation::GenerationChange;
    use crate::generation::write_index_config;
    use crate::generation::{INDEXES_DIRECTORY, INITIAL_EPOCH};
    use crate::records::{DocumentRecord, PassageRecord};
    use crate::segment::{SegmentSpec, SegmentWriter};

    /// Name of the fixture index, and prefix of its export directory.
    const INDEX: &str = "export";

    /// Configuration of the fixture index: one field per level, 4 dimensions,
    /// fp16, no clustering.
    const CONFIG: &str = r#"{
        "name": "export",
        "model": "m",
        "reference_precision": "fp16",
        "schema": {
            "document_fields": { "title": { "type": "keyword" } },
            "passage_fields": { "part": { "type": "integer" } }
        },
        "cluster_by": [],
        "lexical": { "enabled": false, "language": "fr", "positions": false },
        "search": {
            "scan": "none",
            "rescore_candidates": 10,
            "exact_threshold": 10,
            "candidates_per_branch": 10,
            "fusion": { "method": "rrf" }
        },
        "ingest": { "acceptance_sample": 1, "acceptance_min_cosine": 0.9 },
        "compaction": { "dead_ratio": 0.1 }
    }"#;

    /// The fixture model: no encoder, four dimensions, normalized vectors.
    const MODEL: &str = r#"{"name":"m","kind":"external","dims":4,"normalize":true}"#;

    /// One temporary instance: the index, the exchange root, and the two
    /// configurations the export takes.
    struct Fixture {
        _temp: tempfile::TempDir,
        exchange: PathBuf,
        store: GenerationStore,
        index_config: IndexConfig,
        model_config: ModelConfig,
    }

    /// Builds the fixture: two documents, seven live passages, in iid order
    /// `doc-b#0`, `doc-a#0`, `doc-a#1`, … so the row order is not the iid order.
    fn fixture() -> Fixture {
        let temp = tempfile::tempdir()
            .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
        let exchange = temp.path().join("exchange");
        let index_dir = temp.path().join(INDEXES_DIRECTORY).join(INDEX);
        let index_config = IndexConfig::from_json_str(CONFIG)
            .unwrap_or_else(|error| panic!("the fixture configuration must be valid: {error}"));
        write_index_config(&index_dir, CONFIG)
            .unwrap_or_else(|error| panic!("the fixture configuration must be written: {error}"));
        let mut store = GenerationStore::create(&index_dir, INITIAL_EPOCH, &index_config.schema)
            .unwrap_or_else(|error| panic!("the fixture index must be created: {error}"));

        let vectors = [
            &[1.0_f32, 0.0, 0.0, 0.0][..],
            &[0.0, 1.0, 0.0, 0.0],
            &[0.0, 0.0, 1.0, 0.0],
            &[0.0, 0.0, 0.0, 1.0],
            &[1.0, 0.0, 0.0, 0.0],
            &[0.0, 1.0, 0.0, 0.0],
            &[0.0, 0.0, 1.0, 0.0],
        ];
        let segments_dir = crate::generation::segments_directory(
            &crate::generation::epoch_directory(&index_dir, INITIAL_EPOCH),
        );
        fs::create_dir_all(&segments_dir)
            .unwrap_or_else(|error| panic!("the segments directory must be creatable: {error}"));
        let mut writer = SegmentWriter::create(
            &segments_dir,
            SegmentSpec {
                segment_id: "seg-1".to_owned(),
                iid_start: Iid::new(0),
                dims: 4,
                generation: Generation::new(1),
                batch_id: "batch-1".to_owned(),
                precision: ReferencePrecision::Fp16,
            },
        )
        .unwrap_or_else(|error| panic!("the fixture segment must start: {error}"));
        for vector in vectors {
            writer
                .append(vector)
                .unwrap_or_else(|error| panic!("a fixture vector must be appended: {error}"));
        }
        let segment = writer
            .finish()
            .unwrap_or_else(|error| panic!("the fixture segment must be finished: {error}"));

        // iid order deliberately differs from the (doc_id, ord) order.
        let layout = [
            ("doc-b", 0_u32),
            ("doc-a", 0),
            ("doc-a", 1),
            ("doc-a", 2),
            ("doc-a", 3),
            ("doc-a", 4),
            ("doc-b", 1),
        ];
        let passages = layout
            .iter()
            .enumerate()
            .map(|(iid, (doc_id, ord))| PassageRecord {
                iid: Iid::new(u32::try_from(iid).unwrap_or(0)),
                doc_id: (*doc_id).to_owned(),
                ord: *ord,
                text: format!("{doc_id}#{ord}"),
                passage_hash: format!("blake3:passage-{doc_id}-{ord}"),
                fields: serde_json::json!({ "part": ord }),
                segment_id: "seg-1".to_owned(),
                created_generation: Generation::new(1),
                dead_since_gen: None,
            })
            .collect();

        store
            .commit(GenerationChange {
                segments: vec![segment],
                documents: vec![
                    DocumentRecord {
                        doc_id: "doc-a".to_owned(),
                        doc_hash: "blake3:doc-a".to_owned(),
                        fields: serde_json::json!({ "title": "a" }),
                        updated_generation: Generation::new(1),
                    },
                    DocumentRecord {
                        doc_id: "doc-b".to_owned(),
                        doc_hash: "blake3:doc-b".to_owned(),
                        fields: serde_json::json!({ "title": "b" }),
                        updated_generation: Generation::new(1),
                    },
                ],
                passages,
                deletions: Vec::new(),
            })
            .unwrap_or_else(|error| panic!("the fixture generation must commit: {error}"));

        Fixture {
            _temp: temp,
            exchange,
            store,
            index_config,
            model_config: ModelConfig::from_json_str(MODEL)
                .unwrap_or_else(|error| panic!("the fixture model must be valid: {error}")),
        }
    }

    /// Names of the Parquet files of a batch directory, sorted.
    fn files(directory: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(directory)
            .unwrap_or_else(|error| panic!("{} must be listable: {error}", directory.display()))
            .map(|entry| {
                entry
                    .unwrap_or_else(|error| panic!("cannot read an entry: {error}"))
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| name.ends_with(".parquet"))
            .collect();
        names.sort_unstable();
        names
    }

    /// Rows of one Parquet file, read from its metadata.
    fn rows_of(path: &Path) -> usize {
        let file = File::open(path)
            .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
        let builder = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file)
            .unwrap_or_else(|error| {
                panic!(
                    "{} must be a readable Parquet file: {error}",
                    path.display()
                )
            });
        usize::try_from(builder.metadata().file_metadata().num_rows()).unwrap_or(0)
    }

    /// Names of the staging entries an operator would see under `outbox`.
    fn staging_entries(outbox: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(outbox)
            .unwrap_or_else(|error| panic!("{} must be listable: {error}", outbox.display()))
            .map(|entry| {
                entry
                    .unwrap_or_else(|error| panic!("cannot read an entry: {error}"))
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| name.starts_with(STAGING_PREFIX))
            .collect();
        names.sort_unstable();
        names
    }

    /// The part limit of the batch format is one million passages, and it is
    /// not the segment limit by accident: this pins the value the export uses.
    #[test]
    fn the_part_limit_is_one_million_passages() {
        assert_eq!(MAX_PART_PASSAGES, 1_000_000);
    }

    /// A generation that moves between the snapshot and the first read is a
    /// transient race: the export starts again, up to the limit, and no attempt
    /// is kept.
    #[test]
    fn a_stale_generation_is_retried_on_a_fresh_snapshot() {
        let mut calls = 0_u32;
        let result = retrying_stale_generation(|| {
            calls = calls.saturating_add(1);
            match calls {
                1 | 2 => Err(BatchFailure::StaleGeneration(Generation::new(u64::from(
                    calls,
                )))),
                _ => Ok(calls),
            }
        });
        assert_eq!(
            result.unwrap_or_else(|error| panic!("the third attempt must settle: {error}")),
            3,
            "the export only succeeds once the snapshot still describes the rows"
        );
        assert_eq!(
            calls, 3,
            "a stale generation is retried, not reported before the limit"
        );
    }

    /// A generation that never settles is reported as `overloaded` (503): the
    /// writer is faster than the export, which is transient, not corrupt.
    #[test]
    fn a_generation_that_never_settles_is_reported_as_overloaded() {
        let mut calls = 0_usize;
        let error = retrying_stale_generation::<()>(|| {
            calls = calls.saturating_add(1);
            Err(BatchFailure::StaleGeneration(Generation::new(7)))
        })
        .err()
        .unwrap_or_else(|| panic!("an unsettled generation must be reported"));
        assert_eq!(
            error.code(),
            ErrorCode::Overloaded,
            "a race with the writer is retryable, not corrupt: {error}"
        );
        assert_eq!(
            calls, MAX_GENERATION_ATTEMPTS,
            "the export stops after the announced number of attempts"
        );
        assert!(
            error.to_string().contains("retry the export"),
            "the message must tell the caller what to do: {error}"
        );
    }

    /// Any other failure stops the export at once: only the generation race is
    /// retried.
    #[test]
    fn a_fatal_failure_is_not_retried() {
        let mut calls = 0_usize;
        let error = retrying_stale_generation::<()>(|| {
            calls = calls.saturating_add(1);
            Err(BatchFailure::Fatal(internal("the disk is full".to_owned())))
        })
        .err()
        .unwrap_or_else(|| panic!("a fatal failure must be reported"));
        assert_eq!(error.code(), ErrorCode::Internal);
        assert_eq!(calls, 1, "only a stale generation is retried");
    }

    /// Two exports of the same batch never draw the same staging name: the
    /// name carries the pid, a sequence number and the clock.
    #[test]
    fn every_staging_name_is_unique() {
        let outbox = Path::new("/nowhere/outbox");
        let first = staging_path(outbox, "idx-g1");
        let second = staging_path(outbox, "idx-g1");
        assert_ne!(
            first, second,
            "two exports of one batch must not share a staging directory"
        );
        let name = first
            .file_name()
            .unwrap_or_else(|| panic!("a staging path has a file name"))
            .to_string_lossy()
            .into_owned();
        assert!(
            name.starts_with(&format!("{STAGING_PREFIX}idx-g1-{}-", std::process::id())),
            "the staging name names the process that created it: {name}"
        );
    }

    /// An export removes only the staging directory it created: a directory a
    /// concurrent export is writing (or a crashed one left behind) is never
    /// touched.
    #[test]
    fn a_foreign_staging_directory_is_left_alone() {
        let fixture = fixture();
        let outbox = fixture.exchange.join(OUTBOX_DIRECTORY);
        fs::create_dir_all(&outbox)
            .unwrap_or_else(|error| panic!("the outbox must be creatable: {error}"));
        let foreign = outbox.join(format!("{STAGING_PREFIX}export-g1-999-1"));
        fs::create_dir(&foreign).unwrap_or_else(|error| {
            panic!("the foreign staging directory must be creatable: {error}")
        });
        let marker = foreign.join("passages-0001.parquet");
        fs::write(&marker, b"another export")
            .unwrap_or_else(|error| panic!("the marker must be writable: {error}"));

        export(
            &fixture.store,
            &fixture.index_config,
            &fixture.model_config,
            &fixture.exchange,
        )
        .unwrap_or_else(|error| panic!("the export must succeed: {error}"));

        assert!(
            marker.exists(),
            "an export must not delete a staging directory it did not create"
        );
        assert_eq!(
            staging_entries(&outbox),
            vec![format!("{STAGING_PREFIX}export-g1-999-1")],
            "the exported batch renamed its own staging directory and left the other one alone"
        );
    }

    /// A reduced part limit rolls the passages over to `passages-0002.parquet`,
    /// and the batch stays valid: the `ord` of a document continues from one
    /// part to the next (`docs/format-lot-v1.md`, control 6).
    #[test]
    fn passages_roll_over_to_the_next_part_at_the_limit() {
        let fixture = fixture();
        let report = export_with_part_size(
            &fixture.store,
            &fixture.index_config,
            &fixture.model_config,
            &fixture.exchange,
            3,
        )
        .unwrap_or_else(|error| panic!("the export must succeed: {error}"));

        assert_eq!(
            (report.documents, report.passages),
            (2, 7),
            "the report counts what the export wrote"
        );
        assert_eq!(
            files(&report.directory),
            vec![
                "documents-0001.parquet".to_owned(),
                "passages-0001.parquet".to_owned(),
                "passages-0002.parquet".to_owned(),
                "passages-0003.parquet".to_owned(),
            ],
            "seven passages with a limit of three fill three parts"
        );
        let rows: Vec<usize> = [
            "passages-0001.parquet",
            "passages-0002.parquet",
            "passages-0003.parquet",
        ]
        .iter()
        .map(|name| rows_of(&report.directory.join(name)))
        .collect();
        assert_eq!(rows, vec![3, 3, 1], "each part holds at most the limit");

        let reader = BatchReader::open(
            &report.directory,
            &fixture.index_config,
            &fixture.model_config,
        )
        .unwrap_or_else(|error| panic!("the split batch must open: {error}"));
        let counts = reader
            .validate()
            .unwrap_or_else(|error| panic!("the split batch must validate: {error}"));
        assert_eq!(
            (counts.documents, counts.passages, counts.deletions),
            (2, 7, 0),
            "a document may span two parts as long as its ord values continue"
        );
    }

    /// A rename that loses the race to another export of the same generation is
    /// the request's fault, not a disk fault: the pre-check above is not
    /// atomic, and the loser must get the code ADR-0061 promises. The staging
    /// directory of the loser is left to its guard, never renamed.
    #[test]
    fn a_rename_that_loses_the_race_is_an_invalid_request() {
        let temp = tempfile::tempdir()
            .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
        let staging = temp.path().join("export-g1-tmp");
        fs::create_dir(&staging)
            .unwrap_or_else(|error| panic!("the staging directory must be creatable: {error}"));
        fs::write(staging.join("manifest.json"), b"{}")
            .unwrap_or_else(|error| panic!("the staging manifest must be writable: {error}"));

        // The directory the other export renamed into place first.
        let directory = temp.path().join("export-g1");
        fs::create_dir(&directory)
            .unwrap_or_else(|error| panic!("the batch directory must be creatable: {error}"));
        fs::write(directory.join("manifest.json"), b"{}")
            .unwrap_or_else(|error| panic!("the batch manifest must be writable: {error}"));

        let error = publish(&staging, &directory)
            .err()
            .unwrap_or_else(|| panic!("the losing rename must fail"));
        let BatchFailure::Fatal(error) = error else {
            panic!("a taken name is not a generation the writer moved");
        };
        assert_eq!(
            error.code(),
            ErrorCode::InvalidRequest,
            "the loser of a creation race gets the code of an existing batch: {error}"
        );
        assert!(
            error
                .message()
                .contains("another export of the same generation"),
            "the message must name the race: {error}"
        );
        assert!(
            staging.exists(),
            "the loser keeps its own staging directory, which its guard removes"
        );
    }

    /// An existing batch of the same generation is refused, and its files are
    /// left untouched: an export never overwrites a copy an operator already
    /// moved out.
    #[test]
    fn an_existing_export_of_the_same_generation_is_refused() {
        let fixture = fixture();
        let first = export(
            &fixture.store,
            &fixture.index_config,
            &fixture.model_config,
            &fixture.exchange,
        )
        .unwrap_or_else(|error| panic!("the first export must succeed: {error}"));
        let before: BTreeMap<String, Vec<u8>> = files(&first.directory)
            .into_iter()
            .map(|name| {
                let path = first.directory.join(&name);
                let bytes = fs::read(&path)
                    .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
                (name, bytes)
            })
            .collect();

        let error = export(
            &fixture.store,
            &fixture.index_config,
            &fixture.model_config,
            &fixture.exchange,
        )
        .err()
        .unwrap_or_else(|| panic!("a second export of the same generation must be refused"));
        assert_eq!(
            error.code(),
            ErrorCode::InvalidRequest,
            "an existing export is a request this store cannot carry out: {error}"
        );

        let after: BTreeMap<String, Vec<u8>> = files(&first.directory)
            .into_iter()
            .map(|name| {
                let path = first.directory.join(&name);
                let bytes = fs::read(&path)
                    .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
                (name, bytes)
            })
            .collect();
        assert_eq!(
            after, before,
            "the refused export must not touch the batch already in place"
        );
    }

    /// A hole in the `ord` of a document is refused before anything is
    /// published: the batch would not pass `BatchReader::validate`, so the
    /// export never renames a directory an operator would pick up.
    #[test]
    fn a_hole_in_the_ord_of_a_document_is_refused() {
        let mut fixture = fixture();
        // iid 2 is `doc-a#1`. `attach` tombstones a whole document at once, so
        // only `GenerationChange` can leave the hole this test needs.
        fixture
            .store
            .commit(GenerationChange {
                segments: Vec::new(),
                documents: Vec::new(),
                passages: Vec::new(),
                deletions: vec![Iid::new(2)],
            })
            .unwrap_or_else(|error| panic!("the tombstone must commit: {error}"));

        let error = export(
            &fixture.store,
            &fixture.index_config,
            &fixture.model_config,
            &fixture.exchange,
        )
        .err()
        .unwrap_or_else(|| panic!("a hole in a document must be refused"));
        assert_eq!(
            error.code(),
            ErrorCode::Internal,
            "a hole is a state no commit could have produced: {error}"
        );
        assert!(
            error
                .to_string()
                .contains("do not run from 0 without a hole"),
            "the message must name the broken property: {error}"
        );

        let outbox = fixture.exchange.join(OUTBOX_DIRECTORY);
        assert!(
            !outbox.join("export-g2").exists(),
            "a refused export must not publish a batch"
        );
        assert!(
            staging_entries(&outbox).is_empty(),
            "a refused export must remove its own staging directory"
        );
    }

    /// A document row whose passages are all tombstoned is refused: a live
    /// document always has at least one live passage, so the export never
    /// writes a document without a passage.
    #[test]
    fn a_live_document_without_a_live_passage_is_refused() {
        let mut fixture = fixture();
        // iids 0 and 6 are the two passages of `doc-b`; its row stays.
        fixture
            .store
            .commit(GenerationChange {
                segments: Vec::new(),
                documents: Vec::new(),
                passages: Vec::new(),
                deletions: vec![Iid::new(0), Iid::new(6)],
            })
            .unwrap_or_else(|error| panic!("the tombstone must commit: {error}"));

        let error = export(
            &fixture.store,
            &fixture.index_config,
            &fixture.model_config,
            &fixture.exchange,
        )
        .err()
        .unwrap_or_else(|| panic!("an orphan document row must be refused"));
        assert_eq!(
            error.code(),
            ErrorCode::Internal,
            "an orphan document row is a state no commit could have produced: {error}"
        );
        assert!(
            error.to_string().contains("doc-b")
                && error.to_string().contains("holds no live passage"),
            "the message must name the document and the broken property: {error}"
        );

        let outbox = fixture.exchange.join(OUTBOX_DIRECTORY);
        assert!(!outbox.join("export-g2").exists());
        assert!(staging_entries(&outbox).is_empty());
    }

    /// An index with no committed row still exports a batch: both roles have a
    /// file, and the manifest counts zero.
    #[test]
    fn an_empty_index_exports_both_roles_with_zero_rows() {
        let temp = tempfile::tempdir()
            .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
        let exchange = temp.path().join("exchange");
        let index_dir = temp.path().join(INDEXES_DIRECTORY).join(INDEX);
        let index_config = IndexConfig::from_json_str(CONFIG)
            .unwrap_or_else(|error| panic!("the fixture configuration must be valid: {error}"));
        write_index_config(&index_dir, CONFIG)
            .unwrap_or_else(|error| panic!("the fixture configuration must be written: {error}"));
        let store = GenerationStore::create(&index_dir, INITIAL_EPOCH, &index_config.schema)
            .unwrap_or_else(|error| panic!("the empty index must be created: {error}"));
        let model_config = ModelConfig::from_json_str(MODEL)
            .unwrap_or_else(|error| panic!("the fixture model must be valid: {error}"));

        let report = export(&store, &index_config, &model_config, &exchange)
            .unwrap_or_else(|error| panic!("an empty index must export: {error}"));

        assert_eq!(report.generation, Generation::new(0));
        assert_eq!((report.documents, report.passages), (0, 0));
        assert_eq!(
            files(&report.directory),
            vec![
                "documents-0001.parquet".to_owned(),
                "passages-0001.parquet".to_owned(),
            ],
            "both roles are present even when the index holds nothing"
        );
        for name in files(&report.directory) {
            assert_eq!(
                rows_of(&report.directory.join(&name)),
                0,
                "{name} must hold no row"
            );
        }
    }
}
