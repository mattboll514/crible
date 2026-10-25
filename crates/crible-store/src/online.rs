// SPDX-License-Identifier: Apache-2.0

//! Online ingestion: documents encoded by the engine (ADR-0055).
//!
//! The caller hands over a JSONL file holding one document per line, in the
//! shape of the `documents` member of `docs/05-api.md`. Passages without a
//! vector are encoded here, in fp32, on CPU; passages with a vector keep it.
//! Every document then goes through the same door as a batch produced
//! elsewhere: this module writes a real batch directory (format v1) into
//! `<data>/tmp/online-<hash>/`, and [`attach`] validates and applies it. The
//! controls, the semantic table, the report and the recovery are those of
//! `attach`, unchanged (ADR-0055).
//!
//! # Determinism and idempotence
//!
//! The batch identity is derived from the bytes of the input file:
//! `batch_id = online-<blake3 of the file>`, and the manifest is a constant
//! except for the counts and the checksums of the two Parquet files. For a
//! given build of Crible, replaying the same file therefore regenerates the
//! same manifest and `attach` reports the original attach without writing
//! anything (INV-10). The producer version is the version of the online lot
//! format, not the version of the binary, so a replay after an upgrade keeps
//! the same identity and the same producer version.
//!
//! Determinism beyond one build is not claimed: the checksums cover the Parquet
//! bytes, and their encoding belongs to the `arrow` and `parquet` versions
//! (`created_by` is fixed here, the rest is not under our control). A replay
//! across an upgrade of those crates may therefore be refused as a
//! `batch_conflict`; ADR-0055 carries that residual risk in "À surveiller".
//!
//! # Length control before encoding
//!
//! The file is read twice. The first pass hashes the bytes, parses and
//! validates every document, and counts the tokens of every passage with the
//! [`TokenCounter`] of the model (control 8 of `docs/04`, **INV-11**). A passage
//! longer than `max_tokens` is refused there, before the fp32 graph is loaded
//! and before anything is written: a refused file never pays an ONNX Runtime
//! session. `attach` re-checks the lengths of the written batch, so the two
//! controls agree by construction.
//!
//! # Confidentiality and cleanup
//!
//! No document text, no field value and no vector ever reaches a message: the
//! errors name the document key (`doc_id#ord`), the line of the file and
//! figures only. The temporary directory is removed by a guard, on success as
//! on failure. A crash leaves an `online-*` pair behind; the next **write**
//! opening of an index deletes it with [`remove_stale_batches`], and only if it
//! can take the presence lock of that batch (ADR-0056): the lot of a writer
//! still at work is never deleted, and a read-only command never touches
//! `<data>/tmp` at all. The cleanup collects a leftover directory and an
//! **orphan lock file** alike. A guard whose directory cannot be removed keeps
//! its lock file on purpose, so the pair remains collectable instead of
//! leaving a directory no cleanup can prove dead.
//!
//! # Presence lock
//!
//! An ingestion takes an exclusive [`File::try_lock`] on
//! `<data>/tmp/online-<batch_id>.lock` before it creates the batch directory,
//! and holds it until after [`attach`]. Two ingestions of the same file
//! therefore serialise: the second fails with [`ErrorCode::Overloaded`] without
//! creating anything, and the caller retries once the first has finished
//! (ADR-0056). [`remove_stale_batches`] uses the same lock to tell a live lot
//! from a leftover: it deletes only what it can lock.

use std::collections::BTreeMap;
use std::fs::{self, File, TryLockError};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow::array::{
    ArrayRef, BooleanArray, Date32Array, FixedSizeBinaryArray, Float64Array, Int64Array,
    StringArray, UInt32Array,
};
use arrow::datatypes::{DataType, Field, Schema as ArrowSchema, SchemaRef};
use arrow::record_batch::RecordBatch;
use crible_core::batch::{
    BATCH_FORMAT, BATCH_FORMAT_VERSION, BatchManifest, BatchMode, BatchModel, Chunker, Counts,
    Producer, Template, VectorDtype,
};
use crible_core::config::{FieldLevel, FieldType, IndexConfig, ReferencePrecision, Schema};
use crible_core::error::{CribleError, ErrorCode};
use crible_core::model::{ModelConfig, ModelKind};
use crible_encode::{Model, Side, TokenCounter};
use half::f16;
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::attach::{AttachReport, attach};
use crate::batch_reader::{parquet_type, vector_width};
use crate::filters_index::date_days;
use crate::generation::GenerationStore;
use crate::records;

/// Prefix of the batch identifier, and of its temporary directory.
const BATCH_PREFIX: &str = "online-";

/// Suffix of the presence lock of one online batch (ADR-0056).
const LOCK_SUFFIX: &str = ".lock";

/// Highest number of passages accepted by one online request
/// (`docs/05-api.md`, "Lot en ligne": 10 000).
const MAX_ONLINE_PASSAGES: u64 = 10_000;

/// Sub-directory of the data root that holds the temporary batches.
pub const TEMPORARY_DIRECTORY: &str = "tmp";

/// Version of the online lot format (ADR-0055). Incremented only when the
/// manifest of an online batch changes shape, never with the binary version.
const ONLINE_FORMAT_VERSION: &str = "1";

/// Producer name of an online batch, constant by decision (ADR-0055).
const PRODUCER_NAME: &str = "crible-online";

/// Producer host of an online batch, constant by decision (ADR-0055).
const PRODUCER_HOST: &str = "crible-online";

/// Sentinel creation time of an online batch: no control and no report reads
/// it, and a file modification time is not stable across copies (ADR-0055).
const CREATED_AT: &str = "1970-01-01T00:00:00Z";

/// Name of the `chunker` and `template` blocks: an online document is not
/// split and its text is encoded as given (ADR-0055).
const NO_TRANSFORM: &str = "none";

/// Version of the `chunker` and `template` blocks.
const NO_TRANSFORM_VERSION: &str = "1";

/// Rows per record batch in the Parquet files of the internal lot.
const ROWS_PER_RECORD_BATCH: usize = 8_192;

/// One document of the online lot, as the `documents` member of
/// `docs/05-api.md` carries it.
///
/// The HTTP route of P6 and `crible ingest` share this type, so that no
/// parallel format grows between them. The envelope refuses unknown members; a
/// key of `fields` is checked against the schema of the index, never silently
/// dropped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnlineDocument {
    /// External identifier of the document.
    pub doc_id: String,
    /// Producer-provided digest; derived from the passages when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_hash: Option<String>,
    /// Document fields, inherited by every passage.
    #[serde(default)]
    pub fields: Map<String, Value>,
    /// Passages of the document, in `ord` order; at least one.
    pub passages: Vec<OnlinePassage>,
}

/// One passage of an online document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnlinePassage {
    /// Exact text indexed in BM25 and encoded when no vector is provided.
    pub text: String,
    /// Passage fields, specific to this passage.
    #[serde(default)]
    pub fields: Map<String, Value>,
    /// Vector of the passage, base64 of the little-endian fp32 components.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector: Option<String>,
}

/// Digest of a document whose `doc_hash` is absent: the blake3 of the canonical
/// JSON of its passages (ADR-0055).
///
/// The form is `blake3:<hex>`. The JSON array holds one object per passage, in
/// `ord` order, with the keys `ord`, `text` and `fields`, sorted at every level
/// as in ADR-0019. A change of a passage field alone therefore changes the
/// digest, which a digest over the texts alone would lose.
///
/// # Errors
///
/// [`ErrorCode::Internal`] when a document holds more passages than `u32::MAX`,
/// a size the batch format cannot number.
pub fn document_hash(document: &OnlineDocument) -> Result<String, CribleError> {
    if let Some(hash) = &document.doc_hash {
        return Ok(hash.clone());
    }
    let mut passages = Vec::with_capacity(document.passages.len());
    for (ord, passage) in document.passages.iter().enumerate() {
        let ord = u32::try_from(ord).map_err(|_| too_many_passages(&document.doc_id))?;
        passages.push(json!({
            "ord": ord,
            "text": passage.text,
            "fields": passage.fields,
        }));
    }
    let canonical = records::canonical_json(&Value::Array(passages))?;
    Ok(format!(
        "blake3:{}",
        blake3::hash(canonical.as_bytes()).to_hex()
    ))
}

/// One online ingestion, as `crible ingest` and the route of P6 describe it.
///
/// The members are the inputs of [`ingest`]; grouping them keeps the call site
/// readable and lets P6 build the same request from an HTTP body without
/// touching the signature.
pub struct OnlineIngest<'a> {
    /// JSONL file holding one document per line.
    pub documents: &'a Path,
    /// Root data directory of the instance; the internal batch goes to its
    /// `tmp/` sub-directory.
    pub data_dir: &'a Path,
    /// Configuration of the target index.
    pub index_config: &'a IndexConfig,
    /// Configuration resolved for the model of the index.
    pub model_config: &'a ModelConfig,
    /// Tokenizer of an `onnx` model (control 8, INV-11); required for `onnx`.
    pub token_counter: Option<&'a TokenCounter>,
    /// Loader of the fp32 encoder of an `onnx` model (control 9, INV-12);
    /// required for `onnx`.
    pub document_encoder: Option<&'a dyn Fn() -> Result<Arc<Model>, CribleError>>,
    /// Mode of the internal manifest: `upsert` only, `replace_all` is refused.
    pub mode: BatchMode,
}

/// Ingests one JSONL file of documents into `store`, then applies the internal
/// batch with [`attach`].
///
/// `data_dir` is the root data directory of the instance: the internal batch is
/// built under `<data_dir>/tmp`, outside the index, and removed before the call
/// returns.
///
/// `token_counter` and `document_encoder` follow [`attach`]: the tokenizer
/// counts the passages (control 8, INV-11), the loader gives the fp32 encoder
/// (control 9, INV-12). Both are required for an `onnx` model and absent for an
/// `external` one. The loader is called at most once here: the encoder it
/// returns serves the online encoding and the acceptance check of `attach`, so
/// a single fp32 graph is loaded.
///
/// `mode` is the mode of the internal manifest. The online lot accepts `upsert`
/// only, for the CLI as for the route of P6: `replace_all` is refused with
/// [`ErrorCode::InvalidRequest`], naming the mode (ADR-0055).
///
/// # Errors
///
/// - [`ErrorCode::InvalidRequest`] for a missing or invalid file, a file that
///   changes while it is read, a file holding more than [`MAX_ONLINE_PASSAGES`]
///   passages, an empty file, a passage without a vector on an `external`
///   model, a vector that is not base64 of `dims` fp32 components, an undeclared
///   or mistyped field, a passage longer than `max_tokens`, or `replace_all`;
/// - [`ErrorCode::Overloaded`] when another ingestion already holds the
///   presence lock of this batch (same input bytes, ADR-0056);
/// - the errors of [`attach`], which validates the internal batch (model,
///   checksums, lengths, acceptance, conflict).
pub fn ingest(
    store: &mut GenerationStore,
    request: &OnlineIngest<'_>,
) -> Result<AttachReport, CribleError> {
    let OnlineIngest {
        documents,
        data_dir,
        index_config,
        model_config,
        token_counter,
        document_encoder,
        mode,
    } = *request;
    if mode != BatchMode::Upsert {
        return Err(invalid(format!(
            "the online lot accepts mode '{}' only, got '{}': at most 10 000 passages per \
             request make '{}' unusable, and a batch deposited by a producer keeps that mode \
             (ADR-0055)",
            BatchMode::Upsert.as_str(),
            mode.as_str(),
            BatchMode::ReplaceAll.as_str()
        )));
    }
    // Caller faults of an `onnx` model, reported before any work as `attach`
    // does, without loading anything: the tokenizer and the loader are only
    // required here, never called.
    if model_config.kind == ModelKind::Onnx {
        if token_counter.is_none() {
            return Err(internal(
                "the index model is onnx, whose passage lengths must be checked, but the caller \
                 passed no token counter (INV-11)",
            ));
        }
        if document_encoder.is_none() {
            return Err(internal(
                "the index model is onnx, whose vectors must pass the acceptance check, but the \
                 caller passed no fp32 encoder (INV-12)",
            ));
        }
    }

    let survey = survey(documents, index_config, model_config, token_counter)?;
    // The presence lock is taken before anything is created and before the
    // fp32 graph is loaded: a contended ingestion fails without creating a
    // directory and without paying a session (ADR-0056).
    let directory = TemporaryBatch::create(data_dir, &survey.batch_id)?;
    // The graph is loaded only when a passage needs encoding: a file that
    // brings every vector never pays a session for its own encoding.
    let encoder = if survey.encoded_passage.is_some() {
        Some(load_encoder(model_config, document_encoder)?)
    } else {
        None
    };
    let mut batch = InternalBatch::new(directory.path(), index_config, model_config)?;
    let digest = visit_documents(documents, |document, _number| {
        batch.push(&document, encoder.as_deref())
    })?;
    // The batch identity comes from the first pass. A file modified in between
    // would make `attach` store bytes that the `batch_id` does not describe, so
    // the second pass recomputes the digest and refuses the lot (TOCTOU).
    ensure_unchanged(&digest, &survey.digest, documents)?;
    let (counts, files) = batch.finish()?;
    write_manifest(
        directory.path(),
        &survey.batch_id,
        index_config,
        model_config,
        counts,
        files,
    )?;
    if counts != survey.counts {
        return Err(internal(format!(
            "the internal batch of '{}' holds {} document(s) and {} passage(s), but the file \
             survey counted {} and {}",
            survey.batch_id,
            counts.documents,
            counts.passages,
            survey.counts.documents,
            survey.counts.passages
        )));
    }

    // One fp32 graph serves the online encoding and the acceptance check of
    // `attach` (ADR-0054): the second loader hands out the same session.
    let reload = encoder.as_ref().map(|model| {
        let model = Arc::clone(model);
        move || Ok(Arc::clone(&model))
    });
    let loader: Option<&dyn Fn() -> Result<Arc<Model>, CribleError>> = match &reload {
        Some(closure) => Some(closure),
        None => document_encoder,
    };
    // The guard removes the directory and its presence lock when this function
    // returns, on success as on failure.
    attach(
        store,
        directory.path(),
        index_config,
        model_config,
        token_counter,
        loader,
    )
}

/// Deletes the `online-*` leftovers of a crash in `<data_dir>/tmp`.
///
/// A lot is **live** while a writer holds the exclusive lock of its
/// `<data_dir>/tmp/online-<batch_id>.lock` neighbour (ADR-0056). For every
/// `online-*` entry, this function takes that lock, then:
///
/// - on success, no writer holds the lot: it is a leftover, and the directory
///   (when it still exists) and the lock file are removed;
/// - on `WouldBlock`, a writer holds it: it is left untouched.
///
/// The lock is taken on the file the path currently names (see
/// [`lock_presence`]), so a path replaced under the caller never yields a lock
/// on a vanished inode.
///
/// Two kinds of residue are handled: the usual pair
/// `online-<batch_id>/` + `online-<batch_id>.lock`, and an **orphan lock
/// file** left when the directory was already gone (a crash between the two
/// deletions of the guard, or a failed acquisition). An orphan lock is removed
/// only if it can be locked, like any other residue.
///
/// Any other lock fault, such as a file system that cannot lock (a network
/// mount), removes nothing for that entry and logs a warning. The asymmetry is
/// deliberate: a leftover that accumulates is a cost, a deleted live lot is an
/// outage.
///
/// The caller runs this at a **write** opening only: a read-only command never
/// modifies `<data>/tmp` (ADR-0056).
///
/// # Errors
///
/// [`ErrorCode::Internal`] when the directory cannot be listed, or an unlocked
/// leftover cannot be removed.
pub fn remove_stale_batches(data_dir: &Path) -> Result<(), CribleError> {
    let root = data_dir.join(TEMPORARY_DIRECTORY);
    if !root.exists() {
        return Ok(());
    }
    let entries = fs::read_dir(&root)
        .map_err(|error| io_error("list the temporary directory", &root, &error))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| io_error("read the temporary directory", &root, &error))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(BATCH_PREFIX) {
            continue;
        }
        let path = entry.path();
        // One directory `online-<id>` and its lock `online-<id>.lock` are the
        // two names of one lot. The lock is taken once, from whichever name is
        // seen: the directory while it exists, the orphan lock file otherwise.
        let (lock_path, directory) = if path.is_dir() {
            (root.join(format!("{name}{LOCK_SUFFIX}")), Some(path))
        } else if let Some(stem) = name.strip_suffix(LOCK_SUFFIX) {
            if !stem.starts_with(BATCH_PREFIX) {
                continue;
            }
            let directory = root.join(stem);
            if directory.exists() {
                // The directory entry handles this lot, with its lock.
                continue;
            }
            (path, None)
        } else {
            continue;
        };
        let _lock = match lock_presence(&lock_path) {
            Ok(LockAttempt::Locked(lock)) => lock,
            Ok(LockAttempt::Busy) => {
                tracing::debug!(
                    path = %lock_path.display(),
                    "an online batch is held by another writer; it is left in place"
                );
                continue;
            }
            Err(error) => {
                tracing::warn!(
                    path = %lock_path.display(),
                    %error,
                    "cannot take the presence lock of an online batch; the lot is left in place"
                );
                continue;
            }
        };
        if let Some(directory) = directory {
            match fs::remove_dir_all(&directory) {
                Ok(()) => {}
                // A concurrent writer finished between the listing and the
                // lock: there is nothing left to remove but the lock.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(io_error("remove the temporary batch", &directory, &error));
                }
            }
        }
        // Cleaning up deletes the lock file only while holding its lock, and
        // `lock_presence` has just checked that this handle is the file the
        // path names. Since no one but a lock holder unlinks the path, it still
        // names this file here: the two deletions (directory, then lock) cannot
        // leave a lock on a vanished inode behind them.
        match fs::remove_file(&lock_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(io_error(
                    "remove the presence lock of a temporary batch",
                    &lock_path,
                    &error,
                ));
            }
        }
    }
    Ok(())
}

/// What the first pass over the input file learned.
struct Survey {
    /// `online-<blake3 of the file bytes>`.
    batch_id: String,
    /// Hex digest of the file bytes, recomputed by the second pass.
    digest: String,
    /// Rows the internal batch will hold.
    counts: Counts,
    /// Key of the first passage that needs the encoder (`doc_id#ord`), if any.
    encoded_passage: Option<String>,
}

/// First pass: hashes the file, validates every document and counts the tokens
/// of every passage, without writing anything and without loading the graph.
fn survey(
    documents: &Path,
    index_config: &IndexConfig,
    model_config: &ModelConfig,
    token_counter: Option<&TokenCounter>,
) -> Result<Survey, CribleError> {
    let counter = match model_config.kind {
        ModelKind::External => None,
        ModelKind::Onnx => Some(token_counter.ok_or_else(|| {
            internal(
                "the index model is onnx, whose passage lengths must be checked, but the caller \
                 passed no token counter (INV-11)",
            )
        })?),
    };
    let limit = counter.map(TokenCounter::max_tokens);
    let file = open_input(documents)?;
    let mut reader = BufReader::new(file);
    let mut hasher = blake3::Hasher::new();
    let mut line = Vec::new();
    let mut number = 0_u64;
    let mut counts = Counts {
        documents: 0,
        passages: 0,
        deletions: 0,
    };
    let mut encoded_passage = None;
    loop {
        line.clear();
        let read = reader
            .read_until(b'\n', &mut line)
            .map_err(|error| io_error("read the input file", documents, &error))?;
        if read == 0 {
            break;
        }
        hasher.update(&line);
        number += 1;
        let document = parse_document(&line, number)?;
        check_fields(
            &document.fields,
            index_config,
            FieldLevel::Document,
            &format!("document '{}'", document.doc_id),
        )?;
        counts.documents = counts.documents.saturating_add(1);
        let after = counts
            .passages
            .saturating_add(u64::try_from(document.passages.len()).unwrap_or(u64::MAX));
        if after > MAX_ONLINE_PASSAGES {
            return Err(invalid(format!(
                "line {number} would bring the online lot to {after} passages, above the \
                 {MAX_ONLINE_PASSAGES} passages of one request (docs/05-api.md, \"Lot en \
                 ligne\"); split the file"
            )));
        }
        for (ord, passage) in document.passages.iter().enumerate() {
            let ord = u32::try_from(ord).map_err(|_| too_many_passages(&document.doc_id))?;
            let key = format!("{}#{ord}", document.doc_id);
            check_fields(
                &passage.fields,
                index_config,
                FieldLevel::Passage,
                &format!("passage '{key}'"),
            )?;
            if let Some(limit) = limit {
                let counter = counter.ok_or_else(|| {
                    internal("the token limit of an onnx model needs its token counter (INV-11)")
                })?;
                let tokens = counter.count_document(&passage.text)?;
                if u64::try_from(tokens).unwrap_or(u64::MAX) > u64::from(limit) {
                    return Err(invalid(format!(
                        "passage '{key}' of the online lot holds {tokens} tokens, above \
                         max_tokens = {limit} (control 8, INV-11); no model truncates a passage, \
                         shorten it or split it"
                    )));
                }
            }
            if passage.vector.is_none() {
                match model_config.kind {
                    ModelKind::External => {
                        return Err(invalid(format!(
                            "passage '{key}' of the online lot holds no vector, and the model '{}' \
                             of the index is external, which has no encoder; provide a vector",
                            model_config.name
                        )));
                    }
                    ModelKind::Onnx => {
                        encoded_passage.get_or_insert(key.clone());
                    }
                }
            }
            counts.passages = counts.passages.saturating_add(1);
        }
    }
    if counts.documents == 0 {
        return Err(invalid(format!(
            "{} holds no document; the online lot needs one JSON document per line",
            documents.display()
        )));
    }
    let digest = hasher.finalize().to_hex().to_string();
    Ok(Survey {
        batch_id: format!("{BATCH_PREFIX}{digest}"),
        digest,
        counts,
        encoded_passage,
    })
}

/// Loads the fp32 encoder once, for the online encoding; `attach` reuses it.
fn load_encoder(
    model_config: &ModelConfig,
    document_encoder: Option<&dyn Fn() -> Result<Arc<Model>, CribleError>>,
) -> Result<Arc<Model>, CribleError> {
    let loader = document_encoder.ok_or_else(|| {
        internal(format!(
            "the model '{}' is onnx and the online lot holds a passage without a vector, but the \
             caller passed no fp32 encoder",
            model_config.name
        ))
    })?;
    loader()
}

/// Reads the input file and hands every parsed document to `visit`, with its
/// one-based line number. Returns the hex digest of the bytes it read, which
/// the second pass compares with the survey (TOCTOU).
fn visit_documents(
    documents: &Path,
    mut visit: impl FnMut(OnlineDocument, u64) -> Result<(), CribleError>,
) -> Result<String, CribleError> {
    let file = open_input(documents)?;
    let mut reader = BufReader::new(file);
    let mut hasher = blake3::Hasher::new();
    let mut line = Vec::new();
    let mut number = 0_u64;
    loop {
        line.clear();
        let read = reader
            .read_until(b'\n', &mut line)
            .map_err(|error| io_error("read the input file", documents, &error))?;
        if read == 0 {
            break;
        }
        hasher.update(&line);
        number += 1;
        visit(parse_document(&line, number)?, number)?;
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Refuses a file whose second pass did not read the surveyed bytes.
fn ensure_unchanged(seen: &str, surveyed: &str, documents: &Path) -> Result<(), CribleError> {
    if seen == surveyed {
        return Ok(());
    }
    Err(invalid(format!(
        "the input file {} changed while it was being ingested; the batch identity covers the \
         surveyed bytes, so write a stable file and retry",
        documents.display()
    )))
}

/// Opens the input file: a missing file is a caller fault, any other I/O fault
/// is the environment's.
///
/// The rule is the one of `vector::load` and of the batch reader: the caller
/// who names a file that does not exist passed a wrong request, while a refused
/// read (permissions, device) is an internal fault.
fn open_input(path: &Path) -> Result<File, CribleError> {
    File::open(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => invalid(format!(
            "the input file {} does not exist: {error}",
            path.display()
        )),
        _ => io_error("read the input file", path, &error),
    })
}

/// Parses one line of the input file.
///
/// The message names the line, the column and the category of the fault, never
/// the value that caused it: `serde_json` echoes the offending value of a type
/// error, which would put a document text or a field value in a log.
fn parse_document(raw: &[u8], number: u64) -> Result<OnlineDocument, CribleError> {
    let document: OnlineDocument = serde_json::from_slice(raw).map_err(|error| {
        invalid(format!(
            "documents.jsonl line {number}, column {}: not a valid online document ({:?})",
            error.column(),
            error.classify()
        ))
    })?;
    if document.doc_id.is_empty() {
        return Err(invalid(format!(
            "documents.jsonl line {number}: doc_id is empty"
        )));
    }
    if document.passages.is_empty() {
        return Err(invalid(format!(
            "documents.jsonl line {number}: document '{}' holds no passage",
            document.doc_id
        )));
    }
    Ok(document)
}

/// Rejects a key of `fields` the schema does not declare at `level`.
///
/// A mistyped value is refused when the column is built; an undeclared key is
/// refused here, at the survey, so that a file is refused before anything is
/// written. The message names the field, never its value.
fn check_fields(
    fields: &Map<String, Value>,
    index_config: &IndexConfig,
    level: FieldLevel,
    what: &str,
) -> Result<(), CribleError> {
    for name in fields.keys() {
        match index_config.schema.field(name) {
            Some((_, declared)) if declared == level => {}
            Some((_, declared)) => {
                return Err(invalid(format!(
                    "{what}: field '{name}' is declared as a {} field, not a {} field",
                    declared.as_str(),
                    level.as_str()
                )));
            }
            None => {
                return Err(invalid(format!(
                    "{what}: field '{name}' is not declared by the schema of index '{}'; \
                     declared {} fields: {}",
                    index_config.name,
                    level.as_str(),
                    names_at(&index_config.schema, level)
                )));
            }
        }
    }
    Ok(())
}

/// Names of the declared fields of one level, for an error message.
fn names_at(schema: &Schema, level: FieldLevel) -> String {
    let names: Vec<&str> = schema
        .fields()
        .filter(|(_, declared, _)| *declared == level)
        .map(|(name, _, _)| name)
        .collect();
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(", ")
    }
}

/// Highest number of times the lock is reopened when the path is replaced
/// under the caller. A single replacement is expected; more means something
/// keeps deleting the lock file, and guessing is worse than failing.
const LOCK_ATTEMPTS: usize = 8;

/// Outcome of [`lock_presence`] on one lock path.
enum LockAttempt {
    /// The exclusive lock is held on the file the path currently names.
    Locked(File),
    /// Another writer holds the lock: the caller must wait and retry.
    Busy,
}

/// Opens `path` (creating it) and takes an exclusive lock on the **current**
/// file of that path.
///
/// A lock is not enough on its own: the path may have been unlinked and
/// recreated between the open and the `try_lock`, so the handle would hold a
/// lock on a file no name points at any more, and two writers would each hold a
/// lock they believe exclusive (ADR-0056). After a successful `try_lock`, the
/// identity of the open file is therefore compared with the identity of the
/// path; on a mismatch the handle is dropped and the open is retried.
///
/// # Errors
///
/// [`ErrorCode::Internal`] when the lock file cannot be opened or locked, or
/// when the path keeps being replaced.
fn lock_presence(path: &Path) -> Result<LockAttempt, CribleError> {
    for _ in 0..LOCK_ATTEMPTS {
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|error| {
                io_error("open the presence lock of the online batch", path, &error)
            })?;
        match file.try_lock() {
            Ok(()) => {
                if names_same_file(&file, path) {
                    return Ok(LockAttempt::Locked(file));
                }
                // The path was replaced under this handle: retry, and let the
                // handle drop at the end of the iteration.
            }
            Err(TryLockError::WouldBlock) => return Ok(LockAttempt::Busy),
            Err(TryLockError::Error(error)) => {
                return Err(internal(format!(
                    "cannot lock the presence lock {} of the online batch: {error}",
                    path.display()
                )));
            }
        }
    }
    Err(internal(format!(
        "the presence lock {} was replaced {LOCK_ATTEMPTS} times while it was being taken; \
         refusing to guess which file it protects",
        path.display()
    )))
}

/// Tells whether `file` is still the file `path` names.
///
/// The identity is the pair (device, inode) of the two metadata reads, which is
/// how a Unix kernel designates a file. `false` on any fault, including a path
/// that no longer exists, so the caller retries instead of locking a vanished
/// file.
#[cfg(unix)]
fn names_same_file(file: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    let (Ok(held), Ok(current)) = (file.metadata(), fs::metadata(path)) else {
        return false;
    };
    held.dev() == current.dev() && held.ino() == current.ino()
}

/// On a platform without a portable file identity, the check is skipped.
///
/// Crible is deployed on Linux (`docs/07-stack-et-deploiement.md`, Docker), and
/// its tests run on Linux and macOS, where the Unix check above applies.
#[cfg(not(unix))]
fn names_same_file(_file: &File, _path: &Path) -> bool {
    true
}

/// Exclusive presence lock of one internal batch (ADR-0056).
///
/// The lock is the neighbour file `<data_dir>/tmp/online-<batch_id>.lock`, held
/// from before the batch directory exists until the guard that owns it is
/// dropped, after `attach`.
struct BatchLock {
    /// Handle holding the lock; dropping it releases the lock.
    _file: File,
    /// Path of the lock file, removed while the lock is still held.
    path: PathBuf,
}

impl BatchLock {
    /// Opens the lock file (creating it) and takes its exclusive lock.
    ///
    /// A lock file is never unlinked here, whatever the outcome: on a
    /// contention the file belongs to the writer that holds it, and unlinking it
    /// would let a third writer lock a new inode and delete the live lot
    /// (ADR-0056). A lock file this call created and left behind is an orphan,
    /// collected by the next [`remove_stale_batches`].
    ///
    /// A held lock is a contention, not a fault: the same file is being ingested
    /// by another writer, and the caller retries (ADR-0056).
    fn acquire(data_dir: &Path, batch_id: &str) -> Result<Self, CribleError> {
        let root = data_dir.join(TEMPORARY_DIRECTORY);
        fs::create_dir_all(&root)
            .map_err(|error| io_error("create the temporary directory", &root, &error))?;
        let path = root.join(format!("{batch_id}{LOCK_SUFFIX}"));
        match lock_presence(&path)? {
            LockAttempt::Locked(file) => Ok(Self { _file: file, path }),
            LockAttempt::Busy => Err(CribleError::new(
                ErrorCode::Overloaded,
                format!(
                    "the online batch '{batch_id}' is being ingested by another writer (lock \
                     {}); retry once it has finished",
                    path.display()
                ),
            )),
        }
    }
}

/// Guarded internal batch: its directory and its presence lock, removed when
/// the guard leaves the scope, on success as on failure.
struct TemporaryBatch {
    /// `<data_dir>/tmp/online-<hash>`.
    directory: PathBuf,
    /// Presence lock, held for the whole life of the batch.
    lock: BatchLock,
}

impl TemporaryBatch {
    /// Takes the presence lock, then creates the directory, replacing a
    /// leftover of the same input file.
    ///
    /// The lock comes first: a second ingestion of the same bytes must fail
    /// before it creates anything (ADR-0056).
    fn create(data_dir: &Path, batch_id: &str) -> Result<Self, CribleError> {
        let lock = BatchLock::acquire(data_dir, batch_id)?;
        let batch = Self {
            directory: data_dir.join(TEMPORARY_DIRECTORY).join(batch_id),
            lock,
        };
        if batch.directory.exists() {
            // Same input bytes, so a leftover is a batch a crash interrupted:
            // it is rebuilt from scratch rather than appended to. No writer can
            // hold it, since its lock is ours.
            fs::remove_dir_all(&batch.directory).map_err(|error| {
                io_error("remove a stale temporary batch", &batch.directory, &error)
            })?;
        }
        fs::create_dir_all(&batch.directory)
            .map_err(|error| io_error("create the temporary batch", &batch.directory, &error))?;
        Ok(batch)
    }

    /// Directory of the batch, as `attach` wants it.
    fn path(&self) -> &Path {
        &self.directory
    }
}

impl Drop for TemporaryBatch {
    fn drop(&mut self) {
        // A failure here has no better report than the error the caller is
        // already returning, and the next write opening collects the leftover.
        // The directory goes first, then the lock file, which stays held while
        // it is removed (ADR-0056). Only the holder of the lock unlinks it, so
        // the path still names this file here; releasing the handle comes last,
        // with the end of this `drop`.
        //
        // When the directory survives, the lock file is kept on purpose: the
        // pair stays a residue that the next write opening can lock and remove.
        // Deleting the lock file alone would leave a directory no cleanup can
        // prove dead.
        match fs::remove_dir_all(&self.directory) {
            Ok(()) => {
                let _ = fs::remove_file(&self.lock.path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let _ = fs::remove_file(&self.lock.path);
            }
            Err(_) => {}
        }
    }
}

/// One pending document row.
struct PendingDocument {
    doc_id: String,
    doc_hash: String,
    fields: Map<String, Value>,
}

/// One pending passage row.
struct PendingPassage {
    doc_id: String,
    ord: u32,
    text: String,
    fields: Map<String, Value>,
    vector: Vec<u8>,
}

/// Writer of the internal batch: streams the rows into the two Parquet files.
struct InternalBatch<'a> {
    directory: PathBuf,
    schema: &'a Schema,
    model_config: &'a ModelConfig,
    precision: ReferencePrecision,
    documents_schema: SchemaRef,
    passages_schema: SchemaRef,
    documents: ArrowWriter<File>,
    passages: ArrowWriter<File>,
    pending_documents: Vec<PendingDocument>,
    pending_passages: Vec<PendingPassage>,
    counts: Counts,
}

impl<'a> InternalBatch<'a> {
    /// Creates the two Parquet files and their Arrow schemas.
    fn new(
        directory: &Path,
        index_config: &'a IndexConfig,
        model_config: &'a ModelConfig,
    ) -> Result<Self, CribleError> {
        let schema = &index_config.schema;
        let precision = index_config.reference_precision;
        let width = i32::try_from(vector_width(model_config.dims, precision)).map_err(|_| {
            internal(format!(
                "a vector of {} dimensions is too wide for the batch format",
                model_config.dims
            ))
        })?;
        let documents_schema = Arc::new(ArrowSchema::new(document_fields(schema)));
        let mut fields = vec![
            Field::new("doc_id", DataType::Utf8, false),
            Field::new("ord", DataType::UInt32, false),
            Field::new("text", DataType::Utf8, false),
        ];
        for (name, level, spec) in schema.fields() {
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
        let properties = WriterProperties::builder()
            // Fixed so that the Parquet bytes of an online batch do not carry
            // the version of the `parquet` crate: the checksums of the manifest
            // then depend only on the rows and on the encoding itself.
            .set_created_by(format!("{PRODUCER_NAME}/{ONLINE_FORMAT_VERSION}"))
            .build();
        let documents_path = directory.join("documents.parquet");
        let passages_path = directory.join("passages.parquet");
        let documents_file = File::create(&documents_path)
            .map_err(|error| io_error("create a Parquet file", &documents_path, &error))?;
        let passages_file = File::create(&passages_path)
            .map_err(|error| io_error("create a Parquet file", &passages_path, &error))?;
        let documents = ArrowWriter::try_new(
            documents_file,
            documents_schema.clone(),
            Some(properties.clone()),
        )
        .map_err(|error| parquet_error("open a Parquet file", &documents_path, &error))?;
        let passages =
            ArrowWriter::try_new(passages_file, passages_schema.clone(), Some(properties))
                .map_err(|error| parquet_error("open a Parquet file", &passages_path, &error))?;
        Ok(Self {
            directory: directory.to_path_buf(),
            schema,
            model_config,
            precision,
            documents_schema,
            passages_schema,
            documents,
            passages,
            pending_documents: Vec::new(),
            pending_passages: Vec::new(),
            counts: Counts {
                documents: 0,
                passages: 0,
                deletions: 0,
            },
        })
    }

    /// Buffers one document and its passages, encoding the passages without a
    /// vector with `encoder`.
    fn push(
        &mut self,
        document: &OnlineDocument,
        encoder: Option<&Model>,
    ) -> Result<(), CribleError> {
        let hash = document_hash(document)?;
        for (ord, passage) in document.passages.iter().enumerate() {
            let ord = u32::try_from(ord).map_err(|_| too_many_passages(&document.doc_id))?;
            let key = format!("{}#{ord}", document.doc_id);
            let vector = self.stored_vector(passage, &key, encoder)?;
            self.pending_passages.push(PendingPassage {
                doc_id: document.doc_id.clone(),
                ord,
                text: passage.text.clone(),
                fields: passage.fields.clone(),
                vector,
            });
            if self.pending_passages.len() >= ROWS_PER_RECORD_BATCH {
                self.flush_passages()?;
            }
        }
        self.pending_documents.push(PendingDocument {
            doc_id: document.doc_id.clone(),
            doc_hash: hash,
            fields: document.fields.clone(),
        });
        if self.pending_documents.len() >= ROWS_PER_RECORD_BATCH {
            self.flush_documents()?;
        }
        Ok(())
    }

    /// Returns the stored bytes of one passage vector.
    ///
    /// A provided vector is decoded from base64, checked for width and
    /// converted to the reference precision of the index; a passage without a
    /// vector is encoded in fp32 by `encoder`. In both cases the stored width
    /// and the `vector_dtype` of the manifest are those of the index, so control
    /// 3 of `docs/04` accepts the internal batch, even when the index stores
    /// fp16 and the encoder works in fp32.
    fn stored_vector(
        &self,
        passage: &OnlinePassage,
        key: &str,
        encoder: Option<&Model>,
    ) -> Result<Vec<u8>, CribleError> {
        let dims = self.model_config.dims;
        let vector = match &passage.vector {
            Some(encoded) => {
                let bytes = decode_base64(encoded)
                    .map_err(|fault| invalid(format!("the vector of passage '{key}' {fault}")))?;
                let expected = usize::try_from(dims)
                    .unwrap_or(usize::MAX)
                    .saturating_mul(4);
                if bytes.len() != expected {
                    return Err(invalid(format!(
                        "the vector of passage '{key}' is {} bytes, but {dims} fp32 dimensions \
                         need {expected}",
                        bytes.len()
                    )));
                }
                bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|component| {
                        let mut word = [0_u8; 4];
                        word.copy_from_slice(component);
                        f32::from_le_bytes(word)
                    })
                    .collect::<Vec<f32>>()
            }
            None => {
                let encoder = encoder.ok_or_else(|| {
                    invalid(format!(
                        "passage '{key}' of the online lot holds no vector, and the model '{}' of \
                         the index is external, which has no encoder; provide a vector",
                        self.model_config.name
                    ))
                })?;
                let vector = encoder.encode(Side::Document, &passage.text)?;
                if vector.len() != usize::try_from(dims).unwrap_or(usize::MAX) {
                    return Err(internal(format!(
                        "the encoder returned {} dimensions for passage '{key}', but the index \
                         declares {dims}",
                        vector.len()
                    )));
                }
                vector
            }
        };
        Ok(store_vector(&vector, self.precision))
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
        self.documents
            .write(&batch)
            .map_err(|error| internal(format!("cannot write documents.parquet: {error}")))?;
        self.counts.documents = self
            .counts
            .documents
            .saturating_add(u64::try_from(rows.len()).unwrap_or(u64::MAX));
        Ok(())
    }

    /// Writes the pending passage rows.
    fn flush_passages(&mut self) -> Result<(), CribleError> {
        if self.pending_passages.is_empty() {
            return Ok(());
        }
        let rows = std::mem::take(&mut self.pending_passages);
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
        let width = i32::try_from(vector_width(self.model_config.dims, self.precision))
            .map_err(|_| internal("the width of a stored vector does not fit in i32"))?;
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
        self.passages
            .write(&batch)
            .map_err(|error| internal(format!("cannot write passages.parquet: {error}")))?;
        self.counts.passages = self
            .counts
            .passages
            .saturating_add(u64::try_from(rows.len()).unwrap_or(u64::MAX));
        Ok(())
    }

    /// Writes the remaining rows, closes the files and returns their checksums.
    fn finish(mut self) -> Result<(Counts, BTreeMap<String, String>), CribleError> {
        self.flush_documents()?;
        self.flush_passages()?;
        let counts = self.counts;
        self.documents
            .close()
            .map_err(|error| internal(format!("cannot close documents.parquet: {error}")))?;
        self.passages
            .close()
            .map_err(|error| internal(format!("cannot close passages.parquet: {error}")))?;
        let mut files = BTreeMap::new();
        for name in ["documents.parquet", "passages.parquet"] {
            let path = self.directory.join(name);
            files.insert(name.to_owned(), format!("blake3:{}", hash_file(&path)?));
        }
        Ok((counts, files))
    }
}

/// Arrow fields of the `documents` file: the reserved columns, then the document
/// fields of the schema in its own order.
pub(crate) fn document_fields(schema: &Schema) -> Vec<Field> {
    let mut fields = vec![
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("doc_hash", DataType::Utf8, false),
    ];
    for (name, level, spec) in schema.fields() {
        if level == FieldLevel::Document {
            fields.push(Field::new(name, parquet_type(spec.field_type), true));
        }
    }
    fields
}

/// Builds one Arrow column per declared field of `level`.
///
/// `keys` names the row for an error message (`doc_id` or `doc_id#ord`); a null
/// value stays null, a value of another JSON type than the declared one is
/// refused with [`ErrorCode::InvalidRequest`]. The message never carries the
/// value.
pub(crate) fn field_columns(
    schema: &Schema,
    level: FieldLevel,
    keys: &[&str],
    rows: &[&Map<String, Value>],
) -> Result<Vec<ArrayRef>, CribleError> {
    let mut columns = Vec::new();
    for (name, declared, spec) in schema.fields() {
        if declared != level {
            continue;
        }
        columns.push(field_column(name, spec.field_type, keys, rows)?);
    }
    Ok(columns)
}

/// Builds one Arrow column of a declared type.
fn field_column(
    name: &str,
    field_type: FieldType,
    keys: &[&str],
    rows: &[&Map<String, Value>],
) -> Result<ArrayRef, CribleError> {
    match field_type {
        FieldType::Keyword => {
            let mut values: Vec<Option<&str>> = Vec::with_capacity(rows.len());
            for (key, row) in keys.iter().zip(rows) {
                values.push(match row.get(name) {
                    None | Some(Value::Null) => None,
                    Some(Value::String(text)) => Some(text.as_str()),
                    Some(_) => return Err(type_error(key, name, "a string")),
                });
            }
            Ok(Arc::new(StringArray::from(values)))
        }
        FieldType::Integer => {
            let mut values: Vec<Option<i64>> = Vec::with_capacity(rows.len());
            for (key, row) in keys.iter().zip(rows) {
                values.push(match row.get(name) {
                    None | Some(Value::Null) => None,
                    Some(Value::Number(number)) => {
                        Some(number.as_i64().ok_or_else(|| {
                            type_error(key, name, "an integer that fits in 64 bits")
                        })?)
                    }
                    Some(_) => return Err(type_error(key, name, "an integer")),
                });
            }
            Ok(Arc::new(Int64Array::from(values)))
        }
        FieldType::Float => {
            let mut values: Vec<Option<f64>> = Vec::with_capacity(rows.len());
            for (key, row) in keys.iter().zip(rows) {
                values.push(match row.get(name) {
                    None | Some(Value::Null) => None,
                    Some(Value::Number(number)) => Some(
                        number
                            .as_f64()
                            .ok_or_else(|| type_error(key, name, "a number"))?,
                    ),
                    Some(_) => return Err(type_error(key, name, "a number")),
                });
            }
            Ok(Arc::new(Float64Array::from(values)))
        }
        FieldType::Date => {
            let mut values: Vec<Option<i32>> = Vec::with_capacity(rows.len());
            for (key, row) in keys.iter().zip(rows) {
                values.push(match row.get(name) {
                    None | Some(Value::Null) => None,
                    Some(Value::String(text)) => Some(
                        date_days(text)
                            .ok_or_else(|| type_error(key, name, "a date written YYYY-MM-DD"))?,
                    ),
                    Some(_) => {
                        return Err(type_error(key, name, "a date written YYYY-MM-DD"));
                    }
                });
            }
            Ok(Arc::new(Date32Array::from(values)))
        }
        FieldType::Bool => {
            let mut values: Vec<Option<bool>> = Vec::with_capacity(rows.len());
            for (key, row) in keys.iter().zip(rows) {
                values.push(match row.get(name) {
                    None | Some(Value::Null) => None,
                    Some(Value::Bool(value)) => Some(*value),
                    Some(_) => return Err(type_error(key, name, "a boolean")),
                });
            }
            Ok(Arc::new(BooleanArray::from(values)))
        }
    }
}

/// Error of a field whose JSON value has not the declared type.
fn type_error(key: &str, name: &str, expected: &str) -> CribleError {
    invalid(format!(
        "the field '{name}' of '{key}' must be {expected}, matching the schema of the index"
    ))
}

/// Converts one encoded vector to the reference precision of the index, in
/// little-endian bytes.
pub(crate) fn store_vector(vector: &[f32], precision: ReferencePrecision) -> Vec<u8> {
    match precision {
        ReferencePrecision::Fp16 => vector
            .iter()
            .flat_map(|component| f16::from_f32(*component).to_le_bytes())
            .collect(),
        ReferencePrecision::Fp32 => vector
            .iter()
            .flat_map(|component| component.to_le_bytes())
            .collect(),
    }
}

/// Decodes the standard base64 alphabet of `docs/05-api.md`.
///
/// The crate list of `docs/07-stack-et-deploiement.md` holds no base64 decoder
/// and adding one for twenty lines would need an ADR, so the canonical form is
/// implemented here: groups of four, `=` only at the end of the last group, at
/// most two of them, and the bits a padding character covers must be zero. A
/// non-zero remainder is refused, so `Zh==` is not another spelling of `Zg==`.
/// The returned message never quotes the text.
fn decode_base64(text: &str) -> Result<Vec<u8>, String> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err(format!(
            "is not valid base64: {} characters, not a multiple of 4",
            bytes.len()
        ));
    }
    let groups = bytes.len() / 4;
    let mut decoded = Vec::with_capacity(groups * 3);
    for (index, group) in bytes.as_chunks::<4>().0.iter().enumerate() {
        let padding = group.iter().rev().take_while(|byte| **byte == b'=').count();
        if padding > 2 {
            return Err("is not valid base64: more than two padding characters".to_owned());
        }
        if padding > 0 && index + 1 != groups {
            return Err("is not valid base64: padding before the last group".to_owned());
        }
        let first = sextet(group[0])?;
        let second = sextet(group[1])?;
        let third = if padding >= 2 { 0 } else { sextet(group[2])? };
        let fourth = if padding >= 1 { 0 } else { sextet(group[3])? };
        // Canonical form: the bits under the padding are zero, as in RFC 4648,
        // section 3.5. `Zh==` would otherwise decode `f` like `Zg==`.
        if (padding == 2 && second & 0x0f != 0) || (padding == 1 && third & 0x03 != 0) {
            return Err("is not valid base64: non-zero bits under the padding".to_owned());
        }
        decoded.push((first << 2) | (second >> 4));
        if padding < 2 {
            decoded.push((second << 4) | (third >> 2));
        }
        if padding < 1 {
            decoded.push((third << 6) | fourth);
        }
    }
    Ok(decoded)
}

/// Value of one base64 character, or the fault that names its position.
fn sextet(byte: u8) -> Result<u8, String> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err("is not valid base64: an unexpected character".to_owned()),
    }
}

/// Writes the deterministic `manifest.json` of the internal batch (ADR-0055).
fn write_manifest(
    directory: &Path,
    batch_id: &str,
    index_config: &IndexConfig,
    model_config: &ModelConfig,
    counts: Counts,
    files: BTreeMap<String, String>,
) -> Result<(), CribleError> {
    let manifest = BatchManifest {
        format: BATCH_FORMAT.to_owned(),
        format_version: BATCH_FORMAT_VERSION,
        batch_id: batch_id.to_owned(),
        index: index_config.name.clone(),
        mode: BatchMode::Upsert,
        created_at: CREATED_AT.to_owned(),
        producer: Producer {
            name: PRODUCER_NAME.to_owned(),
            version: ONLINE_FORMAT_VERSION.to_owned(),
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
            vector_dtype: VectorDtype::from_reference_precision(index_config.reference_precision),
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
        counts,
        files,
    };
    let raw = serde_json::to_string(&manifest)
        .map_err(|error| internal(format!("cannot serialize the internal manifest: {error}")))?;
    let path = directory.join("manifest.json");
    fs::write(&path, raw).map_err(|error| io_error("write the internal manifest", &path, &error))
}

/// Hashes a file with blake3, streaming it.
pub(crate) fn hash_file(path: &Path) -> Result<String, CribleError> {
    let mut file = File::open(path).map_err(|error| io_error("read a batch file", path, &error))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| io_error("read a batch file", path, &error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Error of a document with more passages than the batch format can number.
fn too_many_passages(doc_id: &str) -> CribleError {
    internal(format!(
        "document '{doc_id}' holds more passages than the batch format can number"
    ))
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
    // Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions.
    #![allow(clippy::panic)]

    use super::*;

    #[test]
    fn base64_decodes_the_standard_alphabet() {
        assert_eq!(decode_base64("").unwrap_or_default(), Vec::<u8>::new());
        assert_eq!(decode_base64("Zg==").unwrap_or_default(), b"f".to_vec());
        assert_eq!(decode_base64("Zm8=").unwrap_or_default(), b"fo".to_vec());
        assert_eq!(decode_base64("Zm9v").unwrap_or_default(), b"foo".to_vec());
        assert_eq!(
            decode_base64("AAECAwQ=").unwrap_or_default(),
            vec![0, 1, 2, 3, 4]
        );
    }

    #[test]
    fn base64_refuses_the_malformed_forms() {
        for text in [
            "Zg=",      // not a multiple of 4
            "Z===",     // more than two padding characters
            "Zg==Zg==", // padding before the last group
            "Z!==",     // an unexpected character
            "Zg=a",     // an unexpected character after padding
            "=AAA",     // padding at the head of a group
            "Zh==",     // non-zero bits under the padding: non-canonical `f`
            "Zm9=",     // non-zero bits under the padding: non-canonical `fo`
        ] {
            assert!(
                decode_base64(text).is_err(),
                "'{text}' must be refused as base64"
            );
        }
    }

    #[test]
    fn a_file_that_changed_between_the_two_passes_is_refused() {
        let documents = Path::new("documents.jsonl");
        assert!(
            ensure_unchanged("aa", "aa", documents).is_ok(),
            "the same digest is accepted"
        );
        let error = ensure_unchanged("aa", "bb", documents)
            .expect_err("a different digest must be refused");
        assert_eq!(error.code(), ErrorCode::InvalidRequest);
        assert!(
            error.to_string().contains("changed"),
            "the message names the cause: {error}"
        );
    }

    #[test]
    fn the_presence_lock_is_only_valid_on_the_file_the_path_names() {
        let temp = tempfile::tempdir().expect("a temporary directory is creatable");
        let path = temp.path().join("online-test.lock");
        fs::write(&path, "").expect("the lock file must be creatable");
        let held = File::options()
            .read(true)
            .write(true)
            .open(&path)
            .expect("the lock file must open");
        held.try_lock().expect("the test takes the lock");
        assert!(
            names_same_file(&held, &path),
            "an open handle names its own path"
        );

        fs::remove_file(&path).expect("the lock file must be removable");
        fs::write(&path, "").expect("a new lock file must be creatable");
        assert!(
            !names_same_file(&held, &path),
            "a path recreated under an open handle is another file, so its lock proves nothing"
        );
    }

    #[test]
    fn a_contended_presence_lock_is_never_removed_by_the_loser() {
        for _ in 0..16 {
            let temp = tempfile::tempdir().expect("a temporary directory is creatable");
            let data = temp.path().to_path_buf();
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let starts: Vec<_> = (0..2)
                .map(|_| {
                    let data = data.clone();
                    let barrier = Arc::clone(&barrier);
                    std::thread::spawn(move || {
                        barrier.wait();
                        BatchLock::acquire(&data, "online-test")
                    })
                })
                .collect();
            let outcomes: Vec<Result<BatchLock, CribleError>> = starts
                .into_iter()
                .map(|start| start.join().expect("the thread must finish"))
                .collect();
            assert_eq!(
                outcomes.iter().filter(|outcome| outcome.is_ok()).count(),
                1,
                "exactly one writer takes the lock of the same batch"
            );
            let loser = outcomes
                .iter()
                .find_map(|outcome| outcome.as_ref().err())
                .expect("the other writer is refused");
            assert_eq!(loser.code(), ErrorCode::Overloaded);

            let path = data
                .join(TEMPORARY_DIRECTORY)
                .join(format!("online-test{LOCK_SUFFIX}"));
            assert!(
                path.exists(),
                "the loser never unlinks the lock of the writer that took it"
            );
            let probe = File::options()
                .read(true)
                .write(true)
                .open(&path)
                .expect("the lock file must open");
            assert!(
                matches!(probe.try_lock(), Err(TryLockError::WouldBlock)),
                "the winner still holds the lock on the file the path names"
            );
        }
    }

    #[test]
    fn a_missing_input_file_is_a_caller_fault_and_a_read_fault_is_internal() {
        let missing = Path::new("/nonexistent/crible/documents.jsonl");
        let error = open_input(missing).expect_err("a missing file must be refused");
        assert_eq!(error.code(), ErrorCode::InvalidRequest);

        let temp = tempfile::tempdir().expect("a temporary directory is creatable");
        let plain = temp.path().join("plain");
        fs::write(&plain, "not a directory").expect("the file must be writable");
        let error = open_input(&plain.join("documents.jsonl"))
            .expect_err("a path below a regular file must be refused");
        assert_eq!(
            error.code(),
            ErrorCode::Internal,
            "only a missing file is a caller fault"
        );
    }

    #[test]
    fn the_doc_hash_covers_the_passage_fields() {
        let before: OnlineDocument = serde_json::from_str(
            r#"{"doc_id":"D1","passages":[{"text":"article","fields":{"partie":"corps"}}]}"#,
        )
        .expect("the document is valid");
        let after: OnlineDocument = serde_json::from_str(
            r#"{"doc_id":"D1","passages":[{"text":"article","fields":{"partie":"expose"}}]}"#,
        )
        .expect("the document is valid");
        let hash = document_hash(&before).expect("the hash is computed");
        assert!(hash.starts_with("blake3:"), "the form is 'blake3:<hex>'");
        assert_ne!(
            hash,
            document_hash(&after).expect("the hash is computed"),
            "a change of a passage field alone must change the doc_hash"
        );
    }

    #[test]
    fn a_provided_doc_hash_is_kept_verbatim() {
        let document: OnlineDocument = serde_json::from_str(
            r#"{"doc_id":"D1","doc_hash":"blake3:00","passages":[{"text":"article"}]}"#,
        )
        .expect("the document is valid");
        assert_eq!(
            document_hash(&document).expect("the hash is kept"),
            "blake3:00"
        );
    }
}
