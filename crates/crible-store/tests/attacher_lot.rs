// SPDX-License-Identifier: Apache-2.0

//! `ATTACHER-LOT`: attaching a validated batch, its invariants and its
//! acceptance criteria.
//!
//! References: `docs/phases/P1-stockage-et-recherche-exacte.md` (task
//! `ATTACHER-LOT`), `docs/04-donnees-et-ingestion.md` ("Modes", "Sémantique
//! d'attache", "Contrôles et recette"), `docs/02-architecture.md` ("Tables
//! SQLite", "Générations et point de commit", "Suppressions, pierres tombales
//! et compaction"), `docs/08-invariants-et-tests.md` (INV-03, INV-10),
//! ADR-0016 (the SQLite commit is the only commit point).
//!
//! # What each test pins
//!
//! | Test | Criterion or invariant |
//! |---|---|
//! | `attaching_l1_then_l2_then_l3_gives_the_expected_counts` | "L1, L2 puis L3 donnent les comptes attendus", plus the three rules of the semantic table the batches exercise |
//! | `inv_10_replaying_an_applied_batch_changes_nothing` | INV-10, first bullet of "Idempotence" |
//! | `inv_10_same_batch_id_with_another_manifest_is_a_batch_conflict` | INV-10, `409 batch_conflict` |
//! | `inv_03_failed_attach_leaves_the_records_database_untouched` | INV-03, atomicity of an attach |
//! | `no_sqlite_write_follows_the_commit_point` | ADR-0016 and ADR-0042: the report is final at the commit point |
//! | `inv_03_doc_id_in_documents_and_in_deletions_rejects_the_batch` | INV-03, cross-check of the two roles |
//! | `replace_all_deletes_the_documents_absent_from_the_batch` | `replace_all` mode |
//! | `readding_a_deleted_document_brings_it_back_as_a_new_document` | ADR-0041 decision 1 |
//! | `same_content_under_a_new_batch_id_counts_as_unchanged` | `unchanged` counter of `docs/04` |
//! | `a_deletion_of_an_unknown_doc_id_is_ignored` | `ignored` counter of `docs/04` |
//!
//! # API under test
//!
//! `crible_store::attach::attach` validates `batch_dir`
//! (`BatchReader::open` + `validate`, controls 1 to 7), applies the semantic
//! table of `docs/04`, writes the segment, then commits one generation. Its
//! report carries the six per-document counters of ADR-0041, the generation and
//! the duration of each step. The durations are call telemetry, returned to the
//! caller that ran the attach and never stored: the persisted report holds the
//! counters and the generation only, so a replay returns it without durations
//! (ADR-0042).
//!
//! `index_config` and `model_config` are the configuration of the target index
//! and the configuration resolved for its model, exactly as
//! [`BatchReader::open`] wants them. The mode comes from `manifest.mode`
//! (`upsert` or `replace_all`): the batch declares how it applies, so `attach`
//! takes no mode argument.
//!
//! The counters count documents, the unit of update. `metadata_updated` means
//! the `documents` row is rewritten because its fields differ while its
//! `doc_hash` did not: no passage is added or killed, and the kept passages
//! inherit the new document fields through their document. `ignored` counts
//! deletions naming a `doc_id` the index does not know.
//!
//! Errors, all from `docs/04`:
//!
//! - a `doc_id` present both in `documents` and in `deletions` is a fault of the
//!   batch: [`ErrorCode::InvalidRequest`], with no write;
//! - the same `batch_id` with another `manifest_blake3`:
//!   [`ErrorCode::BatchConflict`] (HTTP 409), with no write;
//! - a replay of an applied `batch_id` with the same manifest: no error, no
//!   write, and the report of the original attach (INV-10).
//!
//! # Failpoints
//!
//! Cargo feature `failpoints` (declared in `crates/crible-store/Cargo.toml`,
//! never enabled in a release build) and the process-wide variable
//! `CRIBLE_FAILPOINT`, as `crible_store::generation` already uses them. Two
//! steps fire **inside the single transaction of the attach**, once its rows of
//! that role are written and before the next role:
//!
//! | Step | Position |
//! |---|---|
//! | `after_attach_documents` | document rows written, passages not yet |
//! | `after_attach_passages` | passage rows written, tombstones, `batches`, `segments` row and `meta` not yet |
//!
//! At that step the attach panics with a message containing
//! `CRIBLE_FAILPOINT`, and the unwinding rolls the transaction back. The
//! segment files are not covered by that rollback: they are written, fsynced
//! and renamed before the transaction, so an attach stopped at either step
//! leaves an orphan segment directory that recovery removes on the next
//! `GenerationStore::open` (INV-04).
//!
//! The steps must be reachable only from an attach: a plain
//! `GenerationStore::commit` (the `GERER-GENERATIONS` tests) never reaches them.
//! One more test arms the store's `after_sqlite_commit` step during an attach:
//! the `batches` row must already hold the final report there, and a restart
//! must change nothing in SQLite, which proves no write follows the commit
//! point (ADR-0016, ADR-0042).
//!
//! # Layout
//!
//! Paths follow `docs/02-architecture.md`: `<index>/epochs/<epoch>/records.sqlite`,
//! `segments/` and `manifests/`. The fixtures of `tests/common` already encode
//! them.
//!
//! # Datasets
//!
//! The `synth-small` batches `l1`, `l2`, `l3` are generated by
//! `tools/gen_synth.py`; the CI generates them before `cargo test`. After the
//! three attaches the reference state is 1 950 live documents and 9 750 live
//! passages. The mini fixture batches (two dimensions) are the ones of
//! `lire_lot_parquet.rs`, written here in full so this file stands alone.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use arrow::array::{Array, ArrayRef, FixedSizeBinaryArray, Int64Array, StringArray, UInt32Array};
use arrow::datatypes::{DataType, Field, Schema as ArrowSchema, SchemaRef};
use arrow::record_batch::RecordBatch;
use crible_core::config::IndexConfig;
use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::Generation;
use crible_core::model::ModelConfig;
use crible_store::attach::{AttachReport, attach};
use crible_store::batch_reader::BatchReader;
use crible_store::generation::GenerationStore;
use half::f16;
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, OptionalExtension};

/// Command that generates the reference dataset, quoted by the failure
/// message of a missing batch.
const DATASET_COMMAND: &str =
    "cd tools && uv sync && uv run python gen_synth.py --out ../data/synth-small";

/// Batches of `synth-small`, with the counts of the generator.
const SYNTH_BATCHES: [(&str, u64, u64, u64); 3] = [
    ("l1", 2_000, 10_000, 0),
    ("l2", 400, 2_000, 0),
    ("l3", 50, 250, 150),
];

/// Live documents and live passages of `synth-small` after `l1`, `l2` and `l3`
/// (`docs/08-invariants-et-tests.md`, "Jeux de données", and the reference
/// state given by the task sheet).
const SYNTH_FINAL_DOCUMENTS: u64 = 1_950;
const SYNTH_FINAL_PASSAGES: u64 = 9_750;

/// Documents that `l2` rewrites and documents that it creates.
const L2_REPLACED_DOCUMENTS: u64 = 300;
const L2_ADDED_DOCUMENTS: u64 = 100;

/// Documents whose metadata `l3` rewrites, and documents that it deletes.
const L3_METADATA_UPDATED_DOCUMENTS: u64 = 50;
const L3_DELETED_DOCUMENTS: u64 = 150;

/// Attach steps of the fault injection; see "Assumed failpoints".
#[cfg(feature = "failpoints")]
const ATTACH_FAILPOINTS: [&str; 2] = ["after_attach_documents", "after_attach_passages"];

// --- Paths of the reference dataset ---

fn dataset_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/synth-small")
}

/// Locates one batch of `synth-small`, or explains how to generate the dataset.
fn dataset_batch(root: &Path, name: &str) -> Result<PathBuf, String> {
    let dir = root.join(name);
    if dir.join("manifest.json").is_file() {
        Ok(dir)
    } else {
        Err(format!(
            "the synthetic batch '{name}' is missing at {}; generate the dataset before running \
             `cargo test`:\n  {DATASET_COMMAND}",
            dir.display()
        ))
    }
}

fn require_batch(name: &str) -> PathBuf {
    dataset_batch(&dataset_root(), name).unwrap_or_else(|message| panic!("{message}"))
}

fn dataset_index() -> IndexConfig {
    let path = dataset_root().join("index.json");
    let raw = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{} must be readable; generate the dataset before running `cargo test`:\n  \
             {DATASET_COMMAND}: {error}",
            path.display()
        )
    });
    IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("the dataset index.json must be valid: {error}"))
}

/// Resolved model configuration of `synth-small` (768 dimensions, external, so
/// `check_manifest` compares only `dims`, `normalize` and the vector type).
fn dataset_model() -> ModelConfig {
    ModelConfig::from_json_str(
        r#"{ "name": "synthetic-external", "kind": "external", "dims": 768, "normalize": true }"#,
    )
    .unwrap_or_else(|error| panic!("the dataset model configuration is valid: {error}"))
}

// --- Attach, with a readable failure ---

/// Attaches one batch and returns its report, naming the batch on failure.
fn attach_batch(
    store: &mut GenerationStore,
    batch_dir: &Path,
    index: &IndexConfig,
    model: &ModelConfig,
) -> AttachReport {
    attach(store, batch_dir, index, model).unwrap_or_else(|error| {
        panic!(
            "the attach of {} must succeed, got {} ({}): {}",
            batch_dir.display(),
            error.code().as_str(),
            error.code().http_status(),
            error.message()
        )
    })
}

/// Attaches one batch of `synth-small` and returns its report.
fn attach_synth(
    store: &mut GenerationStore,
    name: &str,
    index: &IndexConfig,
    model: &ModelConfig,
) -> AttachReport {
    let batch = require_batch(name);
    let report = attach_batch(store, &batch, index, model);
    assert_eq!(
        report.batch_id, name,
        "the report must name the batch it applied"
    );
    report
}

/// The counters of one report, so a replay can be compared field by field
/// without pinning the fields this task adds later (durations, acceptance).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Counters {
    generation: u64,
    added: u64,
    replaced: u64,
    unchanged: u64,
    metadata_updated: u64,
    deleted: u64,
    ignored: u64,
}

fn counters(report: &AttachReport) -> Counters {
    Counters {
        generation: report.generation.get(),
        added: report.added,
        replaced: report.replaced,
        unchanged: report.unchanged,
        metadata_updated: report.metadata_updated,
        deleted: report.deleted,
        ignored: report.ignored,
    }
}

// --- Reference state of `records.sqlite`, read from outside the store ---

/// Logical state of `records.sqlite`: the counts the criteria name, and a
/// digest of every row of every table.
///
/// The digest is the level at which "exactly in its state before" is asserted:
/// page allocation and the free list may differ after a rolled-back transaction,
/// but no row, no `meta` value and no `batches` history may. Rows are read in
/// primary-key order, so the digest does not depend on the physical order.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DbState {
    generation: u64,
    documents_rows: u64,
    passages_rows: u64,
    live_passages: u64,
    live_documents: u64,
    segments_rows: u64,
    batches_rows: u64,
    digest: String,
}

/// Opens `records.sqlite` for reading.
///
/// The pool of `records::ReadPool` is not used here on purpose: a test reads the
/// reference state from outside the store, so it asserts what is committed on
/// disk, not what a store object remembers. The epoch directory is writable, so
/// SQLite may create the `-shm` file of the WAL, as `ReadPool` relies on.
fn open_records(records: &Path) -> Connection {
    let connection = Connection::open_with_flags(records, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap_or_else(|error| {
            panic!("{} must be readable: {error}", records.display());
        });
    connection
        .busy_timeout(Duration::from_millis(1_000))
        .unwrap_or_else(|error| panic!("the busy timeout must be settable: {error}"));
    connection
}

/// Counts the rows `sql` returns.
fn count(connection: &Connection, sql: &str) -> u64 {
    let result: i64 = connection
        .query_row(sql, [], |row| row.get(0))
        .unwrap_or_else(|error| panic!("the count query '{sql}' must run: {error}"));
    u64::try_from(result).unwrap_or_else(|_| panic!("the count of '{sql}' must not be negative"))
}

/// Reads one `meta` column.
fn meta_u64(connection: &Connection, column: &str) -> u64 {
    let sql = format!("SELECT {column} FROM meta WHERE id = 1");
    count(connection, &sql)
}

/// Hashes every row of the result of `sql`, in the order the query returns them.
fn digest_rows(connection: &Connection, sql: &str, hasher: &mut blake3::Hasher) -> u64 {
    let mut statement = connection
        .prepare(sql)
        .unwrap_or_else(|error| panic!("the query '{sql}' must prepare: {error}"));
    let columns = statement.column_count();
    let mut rows = statement
        .query([])
        .unwrap_or_else(|error| panic!("the query '{sql}' must run: {error}"));
    let mut count = 0_u64;
    while let Some(row) = rows
        .next()
        .unwrap_or_else(|error| panic!("a row of '{sql}' must be readable: {error}"))
    {
        count += 1;
        for index in 0..columns {
            let value = row
                .get_ref(index)
                .unwrap_or_else(|error| panic!("a column of '{sql}' must be readable: {error}"));
            match value {
                ValueRef::Null => {
                    hasher.update(b"null\x01");
                }
                ValueRef::Integer(number) => {
                    hasher.update(b"int\x01");
                    hasher.update(number.to_string().as_bytes());
                    hasher.update(b"\x01");
                }
                ValueRef::Real(number) => {
                    hasher.update(b"real\x01");
                    hasher.update(format!("{number:?}").as_bytes());
                    hasher.update(b"\x01");
                }
                ValueRef::Text(text) => {
                    hasher.update(b"text\x01");
                    hasher.update(text);
                    hasher.update(b"\x01");
                }
                ValueRef::Blob(bytes) => {
                    hasher.update(b"blob\x01");
                    hasher.update(bytes);
                    hasher.update(b"\x01");
                }
            }
        }
        hasher.update(b"\x02");
    }
    count
}

/// Every table of the reference state, in a fixed order.
///
/// `meta` comes first so the generation is part of the digest, then the four
/// tables a generation change writes, each read in primary-key order.
const DUMP_QUERIES: [&str; 5] = [
    "SELECT generation, epoch, format_version, next_iid FROM meta",
    "SELECT batch_id, generation, applied_at, manifest_blake3, manifest, report \
     FROM batches ORDER BY batch_id",
    "SELECT segment_id, iid_start, count, generation, batch_id, checksums \
     FROM segments ORDER BY segment_id",
    "SELECT doc_id, doc_hash, fields, updated_generation FROM documents ORDER BY doc_id",
    "SELECT iid, doc_id, ord, text, passage_hash, fields, segment_id, created_generation, \
     dead_since_gen FROM passages ORDER BY iid",
];

fn db_state(records: &Path) -> DbState {
    let connection = open_records(records);
    let mut hasher = blake3::Hasher::new();
    for sql in DUMP_QUERIES {
        digest_rows(&connection, sql, &mut hasher);
    }
    DbState {
        generation: meta_u64(&connection, "generation"),
        documents_rows: count(&connection, "SELECT count(*) FROM documents"),
        passages_rows: count(&connection, "SELECT count(*) FROM passages"),
        live_passages: count(
            &connection,
            "SELECT count(*) FROM passages WHERE dead_since_gen IS NULL",
        ),
        live_documents: count(
            &connection,
            "SELECT count(DISTINCT doc_id) FROM passages WHERE dead_since_gen IS NULL",
        ),
        segments_rows: count(&connection, "SELECT count(*) FROM segments"),
        batches_rows: count(&connection, "SELECT count(*) FROM batches"),
        digest: hasher.finalize().to_hex().to_string(),
    }
}

/// `(doc_id, doc_hash)` of every document row, read before a batch that may
/// rewrite or remove rows.
fn document_hashes(records: &Path) -> BTreeMap<String, String> {
    let connection = open_records(records);
    let mut statement = connection
        .prepare("SELECT doc_id, doc_hash FROM documents ORDER BY doc_id")
        .unwrap_or_else(|error| panic!("the documents query must prepare: {error}"));
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap_or_else(|error| panic!("the documents query must run: {error}"));
    rows.collect::<Result<BTreeMap<_, _>, _>>()
        .unwrap_or_else(|error| panic!("the document rows must be readable: {error}"))
}

/// `(live, total)` passages of one document.
fn passages_of(records: &Path, doc_id: &str) -> (u64, u64) {
    let connection = open_records(records);
    let total: i64 = connection
        .query_row(
            "SELECT count(*) FROM passages WHERE doc_id = ?1",
            [doc_id],
            |row| row.get(0),
        )
        .unwrap_or_else(|error| panic!("the passages of {doc_id} must be counted: {error}"));
    let live: i64 = connection
        .query_row(
            "SELECT count(*) FROM passages WHERE doc_id = ?1 AND dead_since_gen IS NULL",
            [doc_id],
            |row| row.get(0),
        )
        .unwrap_or_else(|error| panic!("the live passages of {doc_id} must be counted: {error}"));
    (
        u64::try_from(live).unwrap_or_else(|_| panic!("the live count of {doc_id} is negative")),
        u64::try_from(total).unwrap_or_else(|_| panic!("the total count of {doc_id} is negative")),
    )
}

/// The `updated_generation` of one document, or `None` when the row is gone.
fn document_generation(records: &Path, doc_id: &str) -> Option<u64> {
    let connection = open_records(records);
    let generation: Option<i64> = connection
        .query_row(
            "SELECT updated_generation FROM documents WHERE doc_id = ?1",
            [doc_id],
            |row| row.get(0),
        )
        .optional()
        .unwrap_or_else(|error| panic!("the row of {doc_id} must be readable: {error}"));
    generation.map(|value| {
        u64::try_from(value).unwrap_or_else(|_| panic!("the generation of {doc_id} is negative"))
    })
}

/// The stored `fields` JSON of one document, or `None` when the row is gone.
fn document_fields(records: &Path, doc_id: &str) -> Option<String> {
    let connection = open_records(records);
    connection
        .query_row(
            "SELECT fields FROM documents WHERE doc_id = ?1",
            [doc_id],
            |row| row.get(0),
        )
        .optional()
        .unwrap_or_else(|error| panic!("the fields of {doc_id} must be readable: {error}"))
}

/// The `batches.report` JSON of one applied batch.
#[cfg(feature = "failpoints")]
fn stored_report(records: &Path, batch_id: &str) -> String {
    let connection = open_records(records);
    connection
        .query_row(
            "SELECT report FROM batches WHERE batch_id = ?1",
            [batch_id],
            |row| row.get(0),
        )
        .unwrap_or_else(|error| panic!("the report of {batch_id} must be readable: {error}"))
}

// --- Reading the identifiers of a batch, through the reader of LIRE-LOT-PARQUET ---

/// `doc_id`s of a batch, read through `BatchReader`, so the test never parses
/// Parquet by hand where the engine already does.
fn batch_ids(
    batch_dir: &Path,
    index: &IndexConfig,
    model: &ModelConfig,
    role: &str,
) -> Vec<String> {
    let reader = BatchReader::open(batch_dir, index, model).unwrap_or_else(|error| {
        panic!(
            "the batch {} must open: {}",
            batch_dir.display(),
            error.message()
        )
    });
    let chunks = match role {
        "documents" => reader.documents(),
        "deletions" => reader.deletions(),
        other => panic!("unknown role {other}"),
    };
    let mut ids = Vec::new();
    for chunk in chunks {
        let chunk = chunk.unwrap_or_else(|error| panic!("a chunk of {role} is read: {error}"));
        let column = chunk
            .column_by_name("doc_id")
            .unwrap_or_else(|| panic!("the {role} files hold a doc_id column"))
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap_or_else(|| panic!("the doc_id column of {role} is UTF-8"));
        for row in 0..column.len() {
            ids.push(column.value(row).to_owned());
        }
    }
    ids
}

// --- Mini fixture batches (two dimensions), as `lire_lot_parquet.rs` writes them ---

/// Index configuration of the mini fixtures: two dimensions, one field per
/// level, normalized vectors.
const MINI_INDEX_JSON: &str = r#"{
  "name": "mini",
  "model": "mini-model",
  "reference_precision": "fp16",
  "schema": {
    "document_fields": { "annee": { "type": "integer" } },
    "passage_fields": { "partie": { "type": "keyword" } }
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
  "ingest": { "acceptance_sample": 2, "acceptance_min_cosine": 0.999 },
  "compaction": { "dead_ratio": 0.1 }
}"#;

/// Resolved model configuration of the mini fixtures.
const MINI_MODEL_JSON: &str =
    r#"{ "name": "mini-model", "kind": "external", "dims": 2, "normalize": true }"#;

fn mini_index() -> IndexConfig {
    IndexConfig::from_json_str(MINI_INDEX_JSON)
        .unwrap_or_else(|error| panic!("the fixture index configuration is valid: {error}"))
}

fn mini_model() -> ModelConfig {
    ModelConfig::from_json_str(MINI_MODEL_JSON)
        .unwrap_or_else(|error| panic!("the fixture model configuration is valid: {error}"))
}

fn document_schema() -> SchemaRef {
    Arc::new(ArrowSchema::new(vec![
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("doc_hash", DataType::Utf8, false),
        Field::new("annee", DataType::Int64, true),
    ]))
}

fn passage_schema() -> SchemaRef {
    Arc::new(ArrowSchema::new(vec![
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("ord", DataType::UInt32, false),
        Field::new("text", DataType::Utf8, false),
        Field::new("partie", DataType::Utf8, true),
        Field::new("vector", DataType::FixedSizeBinary(4), false),
    ]))
}

/// Encodes one normalized two-dimensional vector as little-endian fp16.
fn unit_vector() -> Vec<u8> {
    [1.0_f32, 0.0]
        .iter()
        .flat_map(|value| f16::from_f32(*value).to_bits().to_le_bytes())
        .collect()
}

fn write_parquet(path: &Path, schema: &SchemaRef, columns: Vec<ArrayRef>) {
    let batch = RecordBatch::try_new(schema.clone(), columns)
        .unwrap_or_else(|error| panic!("the fixture batch is valid: {error}"));
    let file =
        File::create(path).unwrap_or_else(|error| panic!("the fixture Parquet file: {error}"));
    let properties = WriterProperties::builder().build();
    let mut writer = ArrowWriter::try_new(file, schema.clone(), Some(properties))
        .unwrap_or_else(|error| panic!("the fixture Parquet writer opens: {error}"));
    writer
        .write(&batch)
        .unwrap_or_else(|error| panic!("the fixture batch is written: {error}"));
    writer
        .close()
        .unwrap_or_else(|error| panic!("the fixture Parquet file closes: {error}"));
}

/// Rows of one mini batch.
struct MiniBatch<'a> {
    /// Directory name, and `batch_id` of the manifest.
    batch_id: &'a str,
    /// `mode` of the manifest: `upsert` or `replace_all`.
    mode: &'a str,
    /// `(doc_id, doc_hash)` per document.
    documents: &'a [(&'a str, &'a str)],
    /// `(doc_id, ord, text)` per passage; the `ord`s of a document start at 0
    /// and are contiguous, as control 6 requires.
    passages: &'a [(&'a str, u32, &'a str)],
    /// `doc_id` of every deletion; empty writes no `deletions.parquet`.
    deletions: &'a [&'a str],
}

/// Writes a complete mini batch and returns its directory.
fn write_mini_batch(root: &Path, spec: MiniBatch<'_>) -> PathBuf {
    let dir = root.join(spec.batch_id);
    fs::create_dir_all(&dir)
        .unwrap_or_else(|error| panic!("the fixture batch directory is created: {error}"));

    let ids: StringArray = spec
        .documents
        .iter()
        .map(|(doc_id, _)| Some(*doc_id))
        .collect();
    let hashes: StringArray = spec
        .documents
        .iter()
        .map(|(_, doc_hash)| Some(*doc_hash))
        .collect();
    let years = Int64Array::from(vec![None::<i64>; spec.documents.len()]);
    write_parquet(
        &dir.join("documents.parquet"),
        &document_schema(),
        vec![Arc::new(ids), Arc::new(hashes), Arc::new(years)],
    );

    let passage_ids: StringArray = spec
        .passages
        .iter()
        .map(|(doc_id, _, _)| Some(*doc_id))
        .collect();
    let ords = UInt32Array::from(
        spec.passages
            .iter()
            .map(|(_, ord, _)| *ord)
            .collect::<Vec<_>>(),
    );
    let texts: StringArray = spec
        .passages
        .iter()
        .map(|(_, _, text)| Some(*text))
        .collect();
    let parts: StringArray = spec.passages.iter().map(|_| Some("titre")).collect();
    let vectors = FixedSizeBinaryArray::try_from_sparse_iter_with_size(
        spec.passages
            .iter()
            .map(|_| Some(unit_vector()))
            .collect::<Vec<_>>()
            .into_iter(),
        i32::try_from(unit_vector().len())
            .unwrap_or_else(|error| panic!("a two-dimensional fp16 vector width: {error}")),
    )
    .unwrap_or_else(|error| panic!("the fixture vectors share one width: {error}"));
    write_parquet(
        &dir.join("passages.parquet"),
        &passage_schema(),
        vec![
            Arc::new(passage_ids),
            Arc::new(ords),
            Arc::new(texts),
            Arc::new(parts),
            Arc::new(vectors),
        ],
    );

    if !spec.deletions.is_empty() {
        let schema = Arc::new(ArrowSchema::new(vec![Field::new(
            "doc_id",
            DataType::Utf8,
            false,
        )]));
        let column: StringArray = spec.deletions.iter().map(|id| Some(*id)).collect();
        write_parquet(
            &dir.join("deletions.parquet"),
            &schema,
            vec![Arc::new(column)],
        );
    }

    let mut files = BTreeMap::new();
    for name in ["documents.parquet", "passages.parquet", "deletions.parquet"] {
        let path = dir.join(name);
        if !path.is_file() {
            continue;
        }
        let bytes = fs::read(&path).unwrap_or_else(|error| {
            panic!("{} must be readable: {error}", path.display());
        });
        files.insert(
            name.to_owned(),
            format!("blake3:{}", blake3::hash(&bytes).to_hex()),
        );
    }

    let manifest = serde_json::json!({
        "format": "crible-batch",
        "format_version": 1,
        "batch_id": spec.batch_id,
        "index": mini_index().name,
        "mode": spec.mode,
        "created_at": "2026-01-01T00:00:00Z",
        "producer": { "name": "attacher-lot-test", "version": "1", "host": "test" },
        "model": { "dims": 2, "normalize": true, "vector_dtype": "fp16" },
        "chunker": { "name": "test", "version": "1", "params": {} },
        "template": { "name": "test", "version": "1" },
        "counts": {
            "documents": spec.documents.len(),
            "passages": spec.passages.len(),
            "deletions": spec.deletions.len(),
        },
        "files": files,
    });
    fs::write(
        dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)
            .unwrap_or_else(|error| panic!("the manifest is serializable: {error}")),
    )
    .unwrap_or_else(|error| panic!("manifest.json is written: {error}"));
    dir
}

/// Copies a batch directory, files only, and returns the copy.
fn copy_batch(source: &Path, destination: &Path) -> PathBuf {
    fs::create_dir_all(destination).unwrap_or_else(|error| {
        panic!("{} must be creatable: {error}", destination.display());
    });
    for entry in fs::read_dir(source).unwrap_or_else(|error| {
        panic!("{} must be listable: {error}", source.display());
    }) {
        let entry = entry.unwrap_or_else(|error| panic!("a batch entry is readable: {error}"));
        if !entry.path().is_file() {
            continue;
        }
        fs::copy(entry.path(), destination.join(entry.file_name()))
            .unwrap_or_else(|error| panic!("{} must be copyable: {error}", entry.path().display()));
    }
    destination.to_path_buf()
}

/// Edits `manifest.json` in place, keeping its `batch_id`.
fn edit_manifest(dir: &Path, edit: impl FnOnce(&mut serde_json::Value)) {
    let path = dir.join("manifest.json");
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
    let mut value: serde_json::Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("{} must be valid JSON: {error}", path.display()));
    edit(&mut value);
    fs::write(
        &path,
        serde_json::to_string_pretty(&value)
            .unwrap_or_else(|error| panic!("the edited manifest is serializable: {error}")),
    )
    .unwrap_or_else(|error| panic!("the edited manifest is written: {error}"));
}

/// Rewrites `manifest.json` with the **same values** in another form: another
/// indentation, and the top-level keys in reverse order.
///
/// ADR-0041 decision 2 digests the bytes of `manifest.json` as read, never the
/// parsed structure: a batch whose manifest only changed shape must therefore
/// conflict when its `batch_id` was already applied.
fn reformat_manifest(dir: &Path) {
    let path = dir.join("manifest.json");
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
    let value: serde_json::Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("{} must be valid JSON: {error}", path.display()));
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("manifest.json must be a JSON object"));

    let mut text = String::from("{\n");
    for (index, (key, member)) in object.iter().rev().enumerate() {
        if index > 0 {
            text.push_str(",\n");
        }
        text.push_str("  ");
        text.push_str(
            &serde_json::to_string(key)
                .unwrap_or_else(|error| panic!("a manifest key is serializable: {error}")),
        );
        text.push_str(": ");
        text.push_str(
            &serde_json::to_string(member)
                .unwrap_or_else(|error| panic!("a manifest value is serializable: {error}")),
        );
    }
    text.push_str("\n}\n");

    let reformatted: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("the reformatted manifest is valid JSON: {error}"));
    assert_eq!(
        reformatted, value,
        "reformatting a manifest must change no value, only its form"
    );
    assert_ne!(
        text, raw,
        "reformatting a manifest must produce other bytes, or it proves nothing"
    );
    fs::write(&path, text)
        .unwrap_or_else(|error| panic!("the reformatted manifest is written: {error}"));
}

// --- Test 1: the counts of L1, L2 and L3 ---

/// L1, L2 then L3 give the reference counts, and each batch exercises one row
/// of the semantic table of `docs/04`: creation, replacement on a different
/// `doc_hash`, metadata-only update on an identical `doc_hash`, deletion.
#[test]
fn attaching_l1_then_l2_then_l3_gives_the_expected_counts() {
    let _serial = common::SerialGuard::acquire();
    let index = dataset_index();
    let model = dataset_model();
    let (_temp, index_dir) = common::temp_index();
    let records = common::records_path(&index_dir);
    let mut store = common::create_store(&index_dir);

    // L1: creation.
    let report = attach_synth(&mut store, "l1", &index, &model);
    assert_eq!(
        counters(&report),
        Counters {
            generation: 1,
            added: SYNTH_BATCHES[0].1,
            replaced: 0,
            unchanged: 0,
            metadata_updated: 0,
            deleted: 0,
            ignored: 0,
        },
        "L1 creates every document of the dataset"
    );
    assert_eq!(store.generation(), Generation::new(1));
    let state = db_state(&records);
    assert_eq!(state.live_documents, 2_000, "L1: live documents");
    assert_eq!(state.live_passages, 10_000, "L1: live passages");
    assert_eq!(
        u64::try_from(store.snapshot().live_iids().len()).expect("a length"),
        10_000,
        "the published snapshot must see every live passage of L1"
    );

    // L2: 300 documents rewritten because their doc_hash changed, 100 created.
    let report = attach_synth(&mut store, "l2", &index, &model);
    assert_eq!(
        counters(&report),
        Counters {
            generation: 2,
            added: L2_ADDED_DOCUMENTS,
            replaced: L2_REPLACED_DOCUMENTS,
            unchanged: 0,
            metadata_updated: 0,
            deleted: 0,
            ignored: 0,
        },
        "L2 replaces the 300 documents whose doc_hash changed and adds 100"
    );
    assert_eq!(store.generation(), Generation::new(2));
    let state = db_state(&records);
    assert_eq!(state.live_documents, 2_100, "after L2: live documents");
    assert_eq!(state.live_passages, 10_500, "after L2: live passages");
    assert_eq!(
        state.passages_rows - state.live_passages,
        1_500,
        "the 300 replaced documents must leave their 5 old passages tombstoned"
    );

    // L3: 50 metadata-only updates and 150 deletions.
    let hashes_before = document_hashes(&records);
    let l3_documents = batch_ids(&require_batch("l3"), &index, &model, "documents");
    let l3_deletions = batch_ids(&require_batch("l3"), &index, &model, "deletions");
    let passages_before: BTreeMap<String, (u64, u64)> = l3_documents
        .iter()
        .map(|doc_id| (doc_id.clone(), passages_of(&records, doc_id)))
        .collect();
    let fields_before: BTreeMap<String, String> = l3_documents
        .iter()
        .map(|doc_id| {
            (
                doc_id.clone(),
                document_fields(&records, doc_id).unwrap_or_else(|| {
                    panic!("the metadata-only document {doc_id} must exist before L3")
                }),
            )
        })
        .collect();
    assert_eq!(
        u64::try_from(l3_documents.len()).expect("a length"),
        L3_METADATA_UPDATED_DOCUMENTS,
        "the l3 fixture must carry the 50 metadata-only updates"
    );
    assert_eq!(
        u64::try_from(l3_deletions.len()).expect("a length"),
        L3_DELETED_DOCUMENTS,
        "the l3 fixture must carry the 150 deletions"
    );

    let report = attach_synth(&mut store, "l3", &index, &model);
    assert_eq!(
        counters(&report),
        Counters {
            generation: 3,
            added: 0,
            replaced: 0,
            unchanged: 0,
            metadata_updated: L3_METADATA_UPDATED_DOCUMENTS,
            deleted: L3_DELETED_DOCUMENTS,
            ignored: 0,
        },
        "L3 updates 50 metadata rows and deletes 150 documents"
    );
    assert_eq!(store.generation(), Generation::new(3));

    let state = db_state(&records);
    assert_eq!(
        (state.live_documents, state.live_passages),
        (SYNTH_FINAL_DOCUMENTS, SYNTH_FINAL_PASSAGES),
        "the reference state of synth-small after L1, L2 and L3"
    );
    assert_eq!(
        u64::try_from(store.snapshot().live_iids().len()).expect("a length"),
        SYNTH_FINAL_PASSAGES,
        "the published snapshot must see exactly the live passages of generation 3"
    );

    // A metadata-only update keeps the doc_hash and every passage of the
    // document, and rewrites only the document row.
    let connection = open_records(&records);
    for doc_id in &l3_documents {
        let before = hashes_before.get(doc_id).unwrap_or_else(|| {
            panic!("the metadata-only document {doc_id} must already exist before L3")
        });
        let after: String = connection
            .query_row(
                "SELECT doc_hash FROM documents WHERE doc_id = ?1",
                [doc_id],
                |row| row.get(0),
            )
            .unwrap_or_else(|error| {
                panic!("the document {doc_id} must still exist after L3: {error}")
            });
        assert_eq!(
            &after, before,
            "a metadata-only update of {doc_id} must keep its doc_hash"
        );
        assert_eq!(
            document_generation(&records, doc_id),
            Some(3),
            "L3 must rewrite the document row of {doc_id}"
        );
        let fields_after = document_fields(&records, doc_id).unwrap_or_else(|| {
            panic!("the metadata-only document {doc_id} must still exist after L3")
        });
        assert_ne!(
            &fields_after,
            fields_before
                .get(doc_id)
                .expect("the fields of the document before L3"),
            "a metadata-only update of {doc_id} must write the new fields, not the old ones"
        );
        assert_eq!(
            passages_of(&records, doc_id),
            *passages_before
                .get(doc_id)
                .expect("the passages of the document"),
            "a metadata-only update of {doc_id} must add no passage and kill none"
        );
        assert_eq!(
            passages_of(&records, doc_id).0,
            5,
            "the document {doc_id} must keep its 5 live passages"
        );
    }

    // A deletion tombstones every passage of the removed document.
    for doc_id in &l3_deletions {
        assert_eq!(
            passages_of(&records, doc_id).0,
            0,
            "the deleted document {doc_id} must keep no live passage"
        );
    }
}

// --- Test 2: INV-10, replaying an applied batch ---

/// Replaying L2 after L1 and L2 writes nothing: the published generation, the
/// counters, the reference state and the segment directories are unchanged, and
/// the report is the one of the original attach.
#[test]
fn inv_10_replaying_an_applied_batch_changes_nothing() {
    let _serial = common::SerialGuard::acquire();
    let index = dataset_index();
    let model = dataset_model();
    let (_temp, index_dir) = common::temp_index();
    let records = common::records_path(&index_dir);
    let mut store = common::create_store(&index_dir);

    attach_synth(&mut store, "l1", &index, &model);
    let first = attach_synth(&mut store, "l2", &index, &model);
    let state = db_state(&records);
    let segments_before = common::directory_entries(&common::segments_dir(&index_dir));
    let live_before = store.snapshot().live_iids().len();

    let replay = attach_synth(&mut store, "l2", &index, &model);

    let durations = first
        .durations
        .as_ref()
        .expect("the caller of the attach receives the durations of its own call");

    for (step, duration) in [
        ("validation", durations.validation),
        ("segment", durations.segment),
        ("transaction", durations.transaction),
        ("manifest", durations.manifest),
        ("snapshot", durations.snapshot),
    ] {
        assert!(
            !duration.is_zero(),
            "the '{step}' step of the first attach must be measured, got {duration:?}"
        );
    }
    assert_eq!(
        replay.durations, None,
        "a replay returns the persisted report, which carries no durations (ADR-0042)"
    );

    assert_eq!(
        counters(&replay),
        counters(&first),
        "a replay must return the report of the original attach, not a fresh one"
    );
    assert_eq!(
        store.generation(),
        Generation::new(2),
        "a replay must not advance the published generation (INV-10)"
    );
    assert_eq!(
        db_state(&records),
        state,
        "a replay must leave records.sqlite exactly as it was: no row, no meta change"
    );
    assert_eq!(
        state.batches_rows, 2,
        "the history must hold one row per applied batch"
    );
    assert_eq!(
        common::directory_entries(&common::segments_dir(&index_dir)),
        segments_before,
        "a replay must not write a second segment for the same batch"
    );
    assert_eq!(
        store.snapshot().live_iids().len(),
        live_before,
        "the published view of a replay must be the one of the original attach"
    );
    assert!(
        !common::manifest_path(&index_dir, Generation::new(3)).exists(),
        "a replay must not write the manifest of a generation it did not commit"
    );
}

// --- Test 3: INV-10, conflict of manifest ---

/// The same `batch_id` with another manifest is a `batch_conflict`, and the
/// reference state is untouched.
#[test]
fn inv_10_same_batch_id_with_another_manifest_is_a_batch_conflict() {
    let _serial = common::SerialGuard::acquire();
    let index = dataset_index();
    let model = dataset_model();
    let (_temp, index_dir) = common::temp_index();
    let records = common::records_path(&index_dir);
    let mut store = common::create_store(&index_dir);

    attach_synth(&mut store, "l2", &index, &model);
    let state = db_state(&records);
    let segments_before = common::directory_entries(&common::segments_dir(&index_dir));

    // Same batch_id, same files, same values: only the form of the manifest
    // differs. A digest recomputed from the parsed structure would be equal, so
    // this case pins "the digest of the bytes read" (ADR-0041 decision 2).
    let copied_root = tempfile::tempdir().expect("a temporary directory");
    let copied = copy_batch(&require_batch("l2"), &copied_root.path().join("l2"));
    reformat_manifest(&copied);

    let outcome = attach(&mut store, &copied, &index, &model);
    let error: CribleError = match outcome {
        Ok(report) => panic!(
            "the same batch_id with another manifest must be refused, but the attach returned \
             generation {}",
            report.generation.get()
        ),
        Err(error) => error,
    };
    assert_eq!(
        error.code(),
        ErrorCode::BatchConflict,
        "another manifest under an applied batch_id is a batch conflict (409): {}",
        error.message()
    );
    assert!(
        error.message().contains("l2"),
        "the conflict must name the batch_id 'l2': {}",
        error.message()
    );

    assert_eq!(
        store.generation(),
        Generation::new(1),
        "a refused attach must not advance the generation"
    );
    assert_eq!(
        db_state(&records),
        state,
        "a refused attach must leave records.sqlite exactly as it was"
    );
    assert_eq!(
        common::directory_entries(&common::segments_dir(&index_dir)),
        segments_before,
        "a refused attach must not leave a new segment behind"
    );
}

// --- Test 4: INV-03, an attach that fails in the middle of its writes ---

/// An attach that fails after it started writing leaves `records.sqlite` as it
/// was: no new row in any table, `meta.generation` unchanged, and the published
/// view still on the previous generation.
///
/// The failure is injected at each of the two attach failpoints, so the rollback
/// is proven for a failure after the document rows and for one after the
/// passage rows. The segment, written before the transaction, survives as an
/// orphan until the next open removes it (INV-04).
#[cfg(feature = "failpoints")]
#[test]
fn inv_03_failed_attach_leaves_the_records_database_untouched() {
    let _serial = common::SerialGuard::acquire();
    let index = dataset_index();
    let model = dataset_model();

    for step in ATTACH_FAILPOINTS {
        let (_temp, index_dir) = common::temp_index();
        let records = common::records_path(&index_dir);
        let mut store = common::create_store(&index_dir);

        attach_synth(&mut store, "l1", &index, &model);
        let state = db_state(&records);
        assert_eq!(state.generation, 1, "the fixture attach must commit first");
        let segments_before = common::directory_entries(&common::segments_dir(&index_dir));

        let batch = require_batch("l2");
        let halted = {
            let _armed = AttachFailpoint::arm(step);
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                attach(&mut store, &batch, &index, &model)
            }));
            assert_halted_at_failpoint(outcome, step)
        };

        // The segment is written, fsynced and renamed before the transaction:
        // a stop at either failpoint must leave it on disk, unreferenced.
        let segments_after_failure = common::directory_entries(&common::segments_dir(&index_dir));
        assert!(
            segments_after_failure.iter().any(|name| name == "seg-l2"),
            "a failed attach at '{step}' must leave the segment written before the transaction \
             (segments: {segments_after_failure:?}); the attach stopped with {halted}"
        );

        assert_eq!(
            db_state(&records),
            state,
            "a failed attach at '{step}' must leave records.sqlite exactly as it was: \
             no new row in documents, passages, segments or batches, and an unchanged \
             meta.generation (INV-03); the attach stopped with {halted}"
        );
        assert_eq!(
            store.generation(),
            Generation::new(1),
            "a failed attach at '{step}' must not publish a new generation (INV-03)"
        );
        assert!(
            !common::manifest_path(&index_dir, Generation::new(2)).exists(),
            "a failed attach at '{step}' must not write the manifest of generation 2"
        );

        // A restart must find the same reference state and remove the orphan
        // segment: nothing is rebuilt from a half-written generation (INV-03
        // and INV-04).
        drop(store);
        let store = common::open_store(&index_dir);
        assert_eq!(
            db_state(&records),
            state,
            "after a restart following a failed attach at '{step}', records.sqlite must be unchanged"
        );
        assert_eq!(
            common::directory_entries(&common::segments_dir(&index_dir)),
            segments_before,
            "the next open must remove the orphan segment left at '{step}' (INV-04)"
        );
        assert_eq!(store.generation(), Generation::new(1));
    }
}

/// Arms one attach step for the duration of the guard.
///
/// `common::FailpointGuard` cannot be reused here: it takes the serial guard
/// itself, and this test already holds it while it commits L1 normally.
#[cfg(feature = "failpoints")]
struct AttachFailpoint;

#[cfg(feature = "failpoints")]
impl AttachFailpoint {
    /// Name of the variable the store reads.
    const ENV: &'static str = "CRIBLE_FAILPOINT";

    fn arm(step: &str) -> Self {
        // SAFETY: the test holds `common::SerialGuard`, and every test of this
        // binary that commits holds it too, so no other thread of this process
        // reads or writes the environment concurrently.
        unsafe { std::env::set_var(Self::ENV, step) };
        Self
    }
}

#[cfg(feature = "failpoints")]
impl Drop for AttachFailpoint {
    fn drop(&mut self) {
        // SAFETY: see `AttachFailpoint::arm`; the serial guard is still held.
        unsafe { std::env::remove_var(Self::ENV) };
    }
}

/// Checks that the attach stopped at the armed step, and returns a short
/// description of how it stopped.
///
/// Only the convention of `GenerationStore::commit` is accepted: a panic whose
/// message names `CRIBLE_FAILPOINT`. The unwinding is what rolls the
/// transaction back, so accepting an ordinary error would prove a mechanism the
/// code does not use. Anything else, in particular a completed attach, fails
/// the test.
#[cfg(feature = "failpoints")]
fn assert_halted_at_failpoint(
    outcome: Result<Result<AttachReport, CribleError>, Box<dyn std::any::Any + Send>>,
    step: &str,
) -> String {
    match outcome {
        Ok(Ok(report)) => panic!(
            "CRIBLE_FAILPOINT={step} was armed but the attach completed generation {}; \
             the failpoint is not implemented, or it is placed outside the writes of the attach",
            report.generation.get()
        ),
        Ok(Err(error)) => panic!(
            "the attach failed at '{step}' with an error instead of panicking at the failpoint, \
             so the transaction may not have been rolled back by unwinding: {} ({})",
            error.message(),
            error.code().as_str()
        ),
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .map(|text| (*text).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<a panic payload that is not a string>".to_owned());
            assert!(
                message.contains("CRIBLE_FAILPOINT"),
                "the attach at '{step}' panicked, but not at a failpoint: {message}"
            );
            format!("panic: {message}")
        }
    }
}

// --- Test 10: no SQLite write follows the commit point (ADR-0016, ADR-0042) ---

/// An attach stopped just after its SQLite commit has already written its
/// definitive `batches` row, report included, and a restart changes nothing in
/// SQLite: no write follows the commit point.
///
/// The report is checked at the commit point, not only after the call returns,
/// which is what proves that the durations are call telemetry and not a second
/// write.
#[cfg(feature = "failpoints")]
#[test]
fn no_sqlite_write_follows_the_commit_point() {
    let _serial = common::SerialGuard::acquire();
    let index = mini_index();
    let model = mini_model();
    let (_temp, index_dir) = common::temp_index();
    let records = common::records_path(&index_dir);
    let mut store = common::create_store(&index_dir);
    let batch_root = tempfile::tempdir().expect("a temporary directory");

    let batch = write_mini_batch(
        batch_root.path(),
        MiniBatch {
            batch_id: "commit-proof",
            mode: "upsert",
            documents: &[("doc-a", "hash-a")],
            passages: &[("doc-a", 0, "text-a")],
            deletions: &[],
        },
    );

    let halted = {
        let _armed = AttachFailpoint::arm("after_sqlite_commit");
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            attach(&mut store, &batch, &index, &model)
        }));
        assert_halted_at_failpoint(outcome, "after_sqlite_commit")
    };

    let state = db_state(&records);
    assert_eq!(
        (state.generation, state.batches_rows),
        (1, 1),
        "the generation and its batch row must be committed before the failpoint: {halted}"
    );
    let report: serde_json::Value = serde_json::from_str(&stored_report(&records, "commit-proof"))
        .unwrap_or_else(|error| panic!("the stored report must be valid JSON: {error}"));
    assert_eq!(
        report,
        serde_json::json!({
            "generation": 1,
            "added": 1,
            "replaced": 0,
            "unchanged": 0,
            "metadata_updated": 0,
            "deleted": 0,
            "ignored": 0,
        }),
        "the batches row must already hold the definitive report at the commit point, \
         with no durations"
    );

    // A restart must not change SQLite: nothing was written after the commit
    // point, so there is nothing to repair in the reference state (INV-04).
    drop(store);
    let store = common::open_store(&index_dir);
    assert_eq!(
        db_state(&records),
        state,
        "a restart must find the same records.sqlite: no SQLite write follows the commit point"
    );
    assert_eq!(store.generation(), Generation::new(1));
}

// --- Test 5: INV-03, a doc_id in both roles of the batch ---

/// A batch that declares the same `doc_id` in `documents` and in `deletions` is
/// refused, and nothing of it reaches `records.sqlite`.
///
/// The reader of `LIRE-LOT-PARQUET` checks the uniqueness of a `doc_id` inside
/// each role (ADR-0040) but not across the two roles; the cross-check belongs to
/// this task, and it must happen before the first write so the refusal leaves no
/// trace.
#[test]
fn inv_03_doc_id_in_documents_and_in_deletions_rejects_the_batch() {
    let _serial = common::SerialGuard::acquire();
    let index = mini_index();
    let model = mini_model();
    let (_temp, index_dir) = common::temp_index();
    let batch_root = tempfile::tempdir().expect("a temporary directory");
    let records = common::records_path(&index_dir);
    let mut store = common::create_store(&index_dir);
    let state = db_state(&records);
    assert_eq!(state.generation, 0, "a fresh index is on generation 0");

    let batch = write_mini_batch(
        batch_root.path(),
        MiniBatch {
            batch_id: "crossed",
            mode: "upsert",
            documents: &[("doc-a", "hash-a"), ("doc-b", "hash-b")],
            passages: &[("doc-a", 0, "text-a"), ("doc-b", 0, "text-b")],
            deletions: &["doc-b"],
        },
    );

    let outcome = attach(&mut store, &batch, &index, &model);
    let message = match outcome {
        Ok(report) => panic!(
            "a doc_id in both documents and deletions must be refused, but the attach \
             returned generation {}",
            report.generation.get()
        ),
        Err(error) => {
            assert_eq!(
                error.code(),
                ErrorCode::InvalidRequest,
                "a batch that contradicts itself is a fault of the batch: {}",
                error.message()
            );
            assert!(
                error.message().contains("doc-b"),
                "the refusal must name the doc_id 'doc-b': {}",
                error.message()
            );
            error.message().to_owned()
        }
    };

    assert_eq!(
        db_state(&records),
        state,
        "a refused batch must leave no trace: no document, no passage, no segment, no batch row \
         and an unchanged meta.generation; the refusal said: {message}"
    );
    assert_eq!(store.generation(), Generation::new(0));
    assert!(
        !common::manifest_path(&index_dir, Generation::new(1)).exists(),
        "a refused batch must not write a manifest"
    );
}

// --- Test 7: a re-added document is a new document (ADR-0041) ---

/// Removing a document deletes its row, so adding it back with the same
/// `doc_hash` creates a new document and its passages instead of hitting the
/// "same hash, passages ignored" rule on a row whose passages are all dead.
#[test]
fn readding_a_deleted_document_brings_it_back_as_a_new_document() {
    let _serial = common::SerialGuard::acquire();
    let index = mini_index();
    let model = mini_model();
    let (_temp, index_dir) = common::temp_index();
    let records = common::records_path(&index_dir);
    let mut store = common::create_store(&index_dir);
    let batch_root = tempfile::tempdir().expect("a temporary directory");

    let created = write_mini_batch(
        batch_root.path(),
        MiniBatch {
            batch_id: "create-doc-a",
            mode: "upsert",
            documents: &[("doc-a", "hash-a")],
            passages: &[("doc-a", 0, "first text")],
            deletions: &[],
        },
    );
    let report = attach_batch(&mut store, &created, &index, &model);
    assert_eq!(report.added, 1, "the first attach creates doc-a");
    assert_eq!(document_generation(&records, "doc-a"), Some(1));

    let removal = write_mini_batch(
        batch_root.path(),
        MiniBatch {
            batch_id: "remove-doc-a",
            mode: "upsert",
            documents: &[],
            passages: &[],
            deletions: &["doc-a"],
        },
    );
    let report = attach_batch(&mut store, &removal, &index, &model);
    assert_eq!(report.deleted, 1, "the deletion removes doc-a");
    assert_eq!(
        document_generation(&records, "doc-a"),
        None,
        "a removed document keeps no documents row (ADR-0041)"
    );
    assert_eq!(
        passages_of(&records, "doc-a"),
        (0, 1),
        "its passage stays on disk as a tombstone"
    );

    let again = write_mini_batch(
        batch_root.path(),
        MiniBatch {
            batch_id: "readd-doc-a",
            mode: "upsert",
            documents: &[("doc-a", "hash-a")],
            passages: &[("doc-a", 0, "first text")],
            deletions: &[],
        },
    );
    let report = attach_batch(&mut store, &again, &index, &model);
    assert_eq!(
        (report.added, report.unchanged),
        (1, 0),
        "a re-added document is a new document, even with the same doc_hash"
    );
    assert_eq!(document_generation(&records, "doc-a"), Some(3));
    assert_eq!(
        passages_of(&records, "doc-a"),
        (1, 2),
        "the new passage is alive, the old one stays dead"
    );
    assert_eq!(
        document_hashes(&records).get("doc-a").map(String::as_str),
        Some("hash-a"),
        "the new document takes the doc_hash of the batch"
    );
}

// --- Test 6: replace_all ---

/// `replace_all` makes the batch the whole content of the index: every document
/// absent from it disappears, its passages tombstoned.
#[test]
fn replace_all_deletes_the_documents_absent_from_the_batch() {
    let _serial = common::SerialGuard::acquire();
    let index = dataset_index();
    let model = dataset_model();
    let (_temp, index_dir) = common::temp_index();
    let records = common::records_path(&index_dir);
    let mut store = common::create_store(&index_dir);

    attach_synth(&mut store, "l1", &index, &model);
    let before: BTreeSet<String> = document_hashes(&records).into_keys().collect();

    // L2 as the complete content of the index: its 400 documents, nothing else.
    let copied_root = tempfile::tempdir().expect("a temporary directory");
    let full = copy_batch(&require_batch("l2"), &copied_root.path().join("l2"));
    edit_manifest(&full, |manifest| {
        manifest["mode"] = serde_json::json!("replace_all");
    });
    let kept: BTreeSet<String> = batch_ids(&full, &index, &model, "documents")
        .into_iter()
        .collect();

    let report = attach_batch(&mut store, &full, &index, &model);
    assert_eq!(
        (report.added, report.replaced),
        (L2_ADDED_DOCUMENTS, L2_REPLACED_DOCUMENTS),
        "the 300 documents of L2 whose hash changed are replaced, its 100 new ones are added"
    );
    assert_eq!(
        report.deleted,
        u64::try_from(before.difference(&kept).count()).expect("a length"),
        "every document of the index absent from a replace_all batch is deleted"
    );

    let state = db_state(&records);
    assert_eq!(
        state.live_documents,
        u64::try_from(kept.len()).expect("a length"),
        "after replace_all, the live documents are exactly the documents of the batch"
    );
    assert_eq!(
        state.live_passages,
        u64::try_from(kept.len()).expect("a length") * 5,
        "after replace_all, only the passages of the batch are alive"
    );

    let absent = before
        .difference(&kept)
        .next()
        .expect("L2 keeps 400 of the 2 000 documents of L1, so some are absent");
    assert_eq!(
        passages_of(&records, absent),
        (0, 5),
        "the document {absent}, absent from a replace_all batch, must keep no live passage"
    );
}

// --- Test 8: `unchanged`, same content under a new batch_id ---

/// A new batch whose documents have the `doc_hash` and the fields of the
/// reference state is fully `unchanged`: no document row is restamped, no
/// passage is written, and no segment appears.
#[test]
fn same_content_under_a_new_batch_id_counts_as_unchanged() {
    let _serial = common::SerialGuard::acquire();
    let index = mini_index();
    let model = mini_model();
    let (_temp, index_dir) = common::temp_index();
    let records = common::records_path(&index_dir);
    let mut store = common::create_store(&index_dir);
    let batch_root = tempfile::tempdir().expect("a temporary directory");

    let first = write_mini_batch(
        batch_root.path(),
        MiniBatch {
            batch_id: "first",
            mode: "upsert",
            documents: &[("doc-a", "hash-a")],
            passages: &[("doc-a", 0, "same text")],
            deletions: &[],
        },
    );
    let report = attach_batch(&mut store, &first, &index, &model);
    assert_eq!(report.added, 1, "the first attach creates doc-a");
    let segments_before = common::directory_entries(&common::segments_dir(&index_dir));
    let passages_before = passages_of(&records, "doc-a");

    // The same document, under another batch_id: same doc_hash, same fields,
    // same passages.
    let second = write_mini_batch(
        batch_root.path(),
        MiniBatch {
            batch_id: "second",
            mode: "upsert",
            documents: &[("doc-a", "hash-a")],
            passages: &[("doc-a", 0, "same text")],
            deletions: &[],
        },
    );
    let report = attach_batch(&mut store, &second, &index, &model);

    assert_eq!(
        counters(&report),
        Counters {
            generation: 2,
            added: 0,
            replaced: 0,
            unchanged: 1,
            metadata_updated: 0,
            deleted: 0,
            ignored: 0,
        },
        "a document with the same doc_hash and the same fields is unchanged (docs/04)"
    );
    assert_eq!(
        common::directory_entries(&common::segments_dir(&index_dir)),
        segments_before,
        "an unchanged document must write no segment"
    );
    assert_eq!(
        document_generation(&records, "doc-a"),
        Some(1),
        "an unchanged document must not be restamped with the new generation"
    );
    assert_eq!(
        passages_of(&records, "doc-a"),
        passages_before,
        "an unchanged document keeps its live passages and gains none"
    );
    assert_eq!(
        db_state(&records).batches_rows,
        2,
        "the new batch still enters the history, even when it changes no document"
    );
}

// --- Test 9: `ignored`, a deletion of an unknown doc_id ---

/// A deletion naming a `doc_id` the index does not know is counted as
/// `ignored`, and nothing is written: no document row, no passage, no segment.
#[test]
fn a_deletion_of_an_unknown_doc_id_is_ignored() {
    let _serial = common::SerialGuard::acquire();
    let index = mini_index();
    let model = mini_model();
    let (_temp, index_dir) = common::temp_index();
    let records = common::records_path(&index_dir);
    let mut store = common::create_store(&index_dir);
    let batch_root = tempfile::tempdir().expect("a temporary directory");

    let removal = write_mini_batch(
        batch_root.path(),
        MiniBatch {
            batch_id: "ghost-removal",
            mode: "upsert",
            documents: &[],
            passages: &[],
            deletions: &["ghost"],
        },
    );
    let report = attach_batch(&mut store, &removal, &index, &model);

    assert_eq!(
        counters(&report),
        Counters {
            generation: 1,
            added: 0,
            replaced: 0,
            unchanged: 0,
            metadata_updated: 0,
            deleted: 0,
            ignored: 1,
        },
        "a deletion of an unknown doc_id is ignored (docs/04)"
    );
    let state = db_state(&records);
    assert_eq!(
        (state.documents_rows, state.passages_rows),
        (0, 0),
        "an ignored deletion must write neither a document row nor a passage row"
    );
    assert!(
        common::directory_entries(&common::segments_dir(&index_dir)).is_empty(),
        "an ignored deletion must write no segment"
    );
    assert_eq!(state.batches_rows, 1, "the batch still enters the history");
}
