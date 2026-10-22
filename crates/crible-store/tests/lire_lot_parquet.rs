// SPDX-License-Identifier: Apache-2.0

//! `crible-store::batch_reader`: reading a batch written by the Python
//! `BatchWriter` and applying controls 4 to 7 of `docs/04`.
//!
//! References: `docs/phases/P1-stockage-et-recherche-exacte.md` (task
//! `LIRE-LOT-PARQUET`), `docs/format-lot-v1.md`,
//! `docs/04-donnees-et-ingestion.md` (controls 4 to 7), ADR-0012, ADR-0034,
//! ADR-0039 (order of the rows of a document), ADR-0040 (unique document ids
//! and parts in a batch v1).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow::array::{Array, ArrayRef, FixedSizeBinaryArray, Int64Array, StringArray, UInt32Array};
use arrow::datatypes::{DataType, Field, Schema as ArrowSchema, SchemaRef};
use arrow::record_batch::RecordBatch;
use crible_core::config::IndexConfig;
use crible_core::error::{CribleError, ErrorCode};
use crible_core::model::ModelConfig;
use crible_store::batch_reader::{BatchCounts, BatchReader, DEFAULT_BATCH_ROWS};
use half::f16;
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;

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

/// Index configuration of the fixtures: two dimensions, one field per level.
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

/// Resolved model configuration of the fixtures.
const MINI_MODEL_JSON: &str =
    r#"{ "name": "mini-model", "kind": "external", "dims": 2, "normalize": true }"#;

/// Model configuration of a fixture index that does not normalize its vectors.
const MINI_MODEL_UNNORMALIZED_JSON: &str =
    r#"{ "name": "mini-model", "kind": "external", "dims": 2, "normalize": false }"#;

fn index() -> IndexConfig {
    IndexConfig::from_json_str(MINI_INDEX_JSON)
        .unwrap_or_else(|error| panic!("the fixture index configuration is valid: {error}"))
}

fn model() -> ModelConfig {
    ModelConfig::from_json_str(MINI_MODEL_JSON)
        .unwrap_or_else(|error| panic!("the fixture model configuration is valid: {error}"))
}

fn unnormalized_model() -> ModelConfig {
    ModelConfig::from_json_str(MINI_MODEL_UNNORMALIZED_JSON)
        .unwrap_or_else(|error| panic!("the fixture model configuration is valid: {error}"))
}

/// Resolved model configuration of `synth-small` (768 dimensions, external).
fn dataset_model() -> ModelConfig {
    ModelConfig::from_json_str(
        r#"{ "name": "synthetic-external", "kind": "external", "dims": 768, "normalize": true }"#,
    )
    .unwrap_or_else(|error| panic!("the dataset model configuration is valid: {error}"))
}

fn document_schema() -> SchemaRef {
    Arc::new(ArrowSchema::new(vec![
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("doc_hash", DataType::Utf8, false),
        Field::new("annee", DataType::Int64, true),
    ]))
}

fn passage_schema() -> SchemaRef {
    passage_schema_of_width(4)
}

fn passage_schema_of_width(width: i32) -> SchemaRef {
    Arc::new(ArrowSchema::new(vec![
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("ord", DataType::UInt32, false),
        Field::new("text", DataType::Utf8, false),
        Field::new("partie", DataType::Utf8, true),
        Field::new("vector", DataType::FixedSizeBinary(width), false),
    ]))
}

/// Encodes one vector as little-endian fp16 components.
fn fp16(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| f16::from_f32(*value).to_bits().to_le_bytes())
        .collect()
}

/// A normalized two-dimensional fixture vector.
fn unit_vector() -> Vec<u8> {
    fp16(&[1.0, 0.0])
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

type DocumentRow<'a> = (&'a str, &'a str, Option<i64>);

fn write_documents(dir: &Path, rows: &[DocumentRow<'_>]) {
    write_documents_named(dir, "documents.parquet", rows);
}

/// Writes one `documents-*.parquet` part of a batch.
fn write_documents_named(dir: &Path, name: &str, rows: &[DocumentRow<'_>]) {
    write_documents_named_with(dir, name, &document_schema(), rows);
}

fn write_documents_named_with(
    dir: &Path,
    name: &str,
    schema: &SchemaRef,
    rows: &[DocumentRow<'_>],
) {
    let ids: StringArray = rows.iter().map(|row| Some(row.0)).collect();
    let hashes: StringArray = rows.iter().map(|row| Some(row.1)).collect();
    let years = Int64Array::from(rows.iter().map(|row| row.2).collect::<Vec<_>>());
    write_parquet(
        &dir.join(name),
        schema,
        vec![Arc::new(ids), Arc::new(hashes), Arc::new(years)],
    );
}

/// Writes `deletions.parquet` with one `doc_id` per row.
fn write_deletions(dir: &Path, ids: &[&str]) {
    let schema = Arc::new(ArrowSchema::new(vec![Field::new(
        "doc_id",
        DataType::Utf8,
        false,
    )]));
    let column: StringArray = ids.iter().map(|id| Some(*id)).collect();
    write_parquet(
        &dir.join("deletions.parquet"),
        &schema,
        vec![Arc::new(column)],
    );
}

type PassageRow<'a> = (&'a str, u32, &'a str, Option<&'a str>, Vec<u8>);

fn write_passages(dir: &Path, rows: &[PassageRow<'_>]) {
    write_passages_with(dir, &passage_schema(), rows);
}

fn write_passages_with(dir: &Path, schema: &SchemaRef, rows: &[PassageRow<'_>]) {
    write_passages_named(dir, "passages.parquet", schema, rows);
}

/// Writes one `passages-*.parquet` part of a batch.
fn write_passages_named(dir: &Path, name: &str, schema: &SchemaRef, rows: &[PassageRow<'_>]) {
    let ids: StringArray = rows.iter().map(|row| Some(row.0)).collect();
    let ords = UInt32Array::from(rows.iter().map(|row| row.1).collect::<Vec<_>>());
    let texts: StringArray = rows.iter().map(|row| Some(row.2)).collect();
    let parts: StringArray = rows.iter().map(|row| row.3).collect();
    let vectors = FixedSizeBinaryArray::try_from_iter(rows.iter().map(|row| row.4.clone()))
        .unwrap_or_else(|error| panic!("the fixture vectors share one width: {error}"));
    write_parquet(
        &dir.join(name),
        schema,
        vec![
            Arc::new(ids),
            Arc::new(ords),
            Arc::new(texts),
            Arc::new(parts),
            Arc::new(vectors),
        ],
    );
}

/// Writes `manifest.json` with the checksums of every Parquet file present.
fn write_manifest(dir: &Path, counts: (u64, u64, u64), mode: &str) {
    write_manifest_with(dir, counts, mode, true);
}

/// Writes `manifest.json` for a batch whose model does, or does not, normalize.
fn write_manifest_with(dir: &Path, counts: (u64, u64, u64), mode: &str, normalize: bool) {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("the batch directory is readable: {error}"))
        .map(|entry| entry.unwrap_or_else(|error| panic!("the entry is readable: {error}")))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.ends_with(".parquet"))
        .collect();
    names.sort();

    let mut files = BTreeMap::new();
    for name in names {
        let bytes = fs::read(dir.join(&name))
            .unwrap_or_else(|error| panic!("the file {name} of the batch is readable: {error}"));
        files.insert(name, format!("blake3:{}", blake3::hash(&bytes).to_hex()));
    }

    let manifest = serde_json::json!({
        "format": "crible-batch",
        "format_version": 1,
        "batch_id": "mini",
        "index": "mini",
        "mode": mode,
        "created_at": "2026-01-01T00:00:00Z",
        "producer": { "name": "test", "version": "1", "host": "test" },
        "model": { "dims": 2, "normalize": normalize, "vector_dtype": "fp16" },
        "chunker": { "name": "test", "version": "1", "params": {} },
        "template": { "name": "test", "version": "1" },
        "counts": {
            "documents": counts.0,
            "passages": counts.1,
            "deletions": counts.2,
        },
        "files": files,
    });
    fs::write(
        dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)
            .unwrap_or_else(|error| panic!("the manifest is serializable: {error}")),
    )
    .unwrap_or_else(|error| panic!("manifest.json is written: {error}"));
}

/// A batch of one document and one normalized passage.
fn minimal_batch(root: &Path) -> PathBuf {
    let dir = root.join("mini");
    fs::create_dir_all(&dir)
        .unwrap_or_else(|error| panic!("the fixture batch directory is created: {error}"));
    write_documents(&dir, &[("doc-a", "hash-a", Some(2020))]);
    write_passages(
        &dir,
        &[("doc-a", 0, "text-a", Some("titre"), unit_vector())],
    );
    write_manifest(&dir, (1, 1, 0), "upsert");
    dir
}

fn open(dir: &Path) -> Result<BatchReader, CribleError> {
    open_with(dir, &model())
}

fn open_with(dir: &Path, model: &ModelConfig) -> Result<BatchReader, CribleError> {
    BatchReader::open(dir, &index(), model)
}

fn assert_invalid<T: std::fmt::Debug>(
    result: Result<T, CribleError>,
    context: &str,
) -> CribleError {
    match result {
        Ok(value) => panic!("{context}: expected an invalid_request error, got {value:?}"),
        Err(error) => {
            assert_eq!(
                error.code(),
                ErrorCode::InvalidRequest,
                "{context}: wrong error code: {}",
                error.message()
            );
            error
        }
    }
}

// --- Criterion 2: a missing batch is an error that says how to generate it ---

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

/// A missing batch is never skipped: the message says how to generate it.
#[test]
fn a_missing_batch_fails_with_the_generation_command() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let message = dataset_batch(root.path(), "l1").expect_err("an absent batch is an error");
    assert!(
        message.contains("gen_synth.py --out ../data/synth-small"),
        "the message must give the generation command: {message}"
    );
    assert!(
        message.contains("cargo test"),
        "the message must say when the dataset is needed: {message}"
    );
}

// --- Criterion 1: Python -> Rust round trip on the three synth-small batches ---

/// The three batches written by `tools/gen_synth.py` pass the manifest checks,
/// the blake3 verification and the schema check, then are read entirely.
#[test]
fn synth_small_batches_pass_and_are_read_entirely() {
    let root = dataset_root();
    let index = IndexConfig::from_json_str(
        &fs::read_to_string(root.join("index.json")).expect("the dataset index.json is readable"),
    )
    .expect("the dataset index.json is valid");

    for (name, documents, passages, deletions) in SYNTH_BATCHES {
        let dir = require_batch(name);
        let reader = BatchReader::open(&dir, &index, &dataset_model())
            .unwrap_or_else(|error| panic!("batch {name} must open: {}", error.message()));

        let counts = reader
            .validate()
            .unwrap_or_else(|error| panic!("batch {name} must validate: {}", error.message()));
        assert_eq!(
            counts,
            BatchCounts {
                documents,
                passages,
                deletions,
            },
            "batch {name} must read the counts it declares"
        );

        assert_eq!(
            rows(reader.documents()),
            documents,
            "batch {name}: every document row is read"
        );
        assert_eq!(
            rows(reader.passages()),
            passages,
            "batch {name}: every passage row is read"
        );
        assert_eq!(
            rows(reader.deletions()),
            deletions,
            "batch {name}: every deletion row is read"
        );
    }
}

fn rows(chunks: crible_store::batch_reader::RowChunks) -> u64 {
    let mut total = 0_u64;
    for chunk in chunks {
        let chunk = chunk.unwrap_or_else(|error| panic!("a chunk of the batch is read: {error}"));
        total += chunk.num_rows() as u64;
    }
    total
}

// --- Criterion 4: bounded memory, one part at a time ---

/// Passages are read in record batches of bounded size, never as one table.
#[test]
fn passages_are_read_in_bounded_chunks() {
    let dir = require_batch("l1");
    let reader = BatchReader::open(
        &dir,
        &IndexConfig::from_json_str(
            &fs::read_to_string(dataset_root().join("index.json"))
                .expect("the dataset index.json is readable"),
        )
        .expect("the dataset index.json is valid"),
        &dataset_model(),
    )
    .expect("batch l1 opens");

    let mut sizes = Vec::new();
    for chunk in reader.passages() {
        let chunk = chunk.expect("a chunk of the passages is read");
        sizes.push(chunk.num_rows());
    }
    assert_eq!(
        sizes.iter().sum::<usize>(),
        10_000,
        "every passage of l1 is read once"
    );
    assert!(
        sizes.len() >= 2,
        "10 000 passages cannot fit in one chunk of {DEFAULT_BATCH_ROWS} rows: {sizes:?}"
    );
    assert!(
        sizes.iter().all(|rows| *rows <= DEFAULT_BATCH_ROWS),
        "no chunk may exceed {DEFAULT_BATCH_ROWS} rows: {sizes:?}"
    );
}

// --- Criterion 3: each control has a failing test with its error code ---

/// A checksum must be `blake3:` and 64 lowercase hexadecimal digits (control 4).
#[test]
fn open_refuses_a_checksum_of_the_wrong_form() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = minimal_batch(root.path());
    let original = fs::read_to_string(dir.join("manifest.json")).expect("the manifest is readable");
    let manifest: serde_json::Value =
        serde_json::from_str(&original).expect("the fixture manifest is valid JSON");
    let digest = manifest["files"]["documents.parquet"]
        .as_str()
        .expect("the fixture declares a checksum for its documents")
        .to_owned();
    let hex = digest
        .strip_prefix("blake3:")
        .expect("the fixture checksum has the documented prefix");

    let cases = [
        ("uppercase", format!("blake3:{}", hex.to_uppercase())),
        ("too short", format!("blake3:{}", &hex[..63])),
        (
            "not hexadecimal",
            format!("blake3:{}", hex.replace('0', "z")),
        ),
        ("no prefix", hex.to_owned()),
        ("empty digest", "blake3:".to_owned()),
    ];
    for (label, checksum) in cases {
        let mut altered = manifest.clone();
        altered["files"]["documents.parquet"] = serde_json::Value::String(checksum);
        fs::write(
            dir.join("manifest.json"),
            serde_json::to_string_pretty(&altered).expect("the altered manifest is serializable"),
        )
        .expect("the altered manifest is written");

        let error = assert_invalid(open(&dir), &format!("{label} checksum"));
        assert!(
            error.message().contains("checksum"),
            "{label} checksum: the message must name the checksum: {}",
            error.message()
        );
    }
}

/// A well-formed checksum that does not match the file is refused (control 4).
#[test]
fn open_refuses_a_checksum_that_does_not_match_the_file() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = minimal_batch(root.path());
    let original = fs::read_to_string(dir.join("manifest.json")).expect("the manifest is readable");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&original).expect("the fixture manifest is valid JSON");
    manifest["files"]["documents.parquet"] =
        serde_json::Value::String(format!("blake3:{}", "0".repeat(64)));
    fs::write(
        dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).expect("the altered manifest is serializable"),
    )
    .expect("the altered manifest is written");

    let error = assert_invalid(open(&dir), "checksum mismatch");
    assert!(
        error.message().contains("does not match its checksum"),
        "the message must report the mismatch: {}",
        error.message()
    );
}

/// A file listed in the manifest but absent from the directory is refused
/// (control 4).
#[test]
fn open_refuses_a_missing_file() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = minimal_batch(root.path());
    fs::remove_file(dir.join("passages.parquet")).expect("the passage file is removed");

    let error = assert_invalid(open(&dir), "missing file");
    assert!(
        error.message().contains("is missing"),
        "the message must report the missing file: {}",
        error.message()
    );
}

/// A column absent from the index schema is refused (control 5).
#[test]
fn open_refuses_a_column_the_index_does_not_declare() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = root.path().join("mini");
    fs::create_dir_all(&dir).expect("the fixture batch directory is created");
    let schema = Arc::new(ArrowSchema::new(vec![
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("doc_hash", DataType::Utf8, false),
        Field::new("annee", DataType::Int64, true),
        Field::new("inconnu", DataType::Utf8, true),
    ]));
    let ids = StringArray::from(vec!["doc-a"]);
    let hashes = StringArray::from(vec!["hash-a"]);
    let years = Int64Array::from(vec![Some(2020_i64)]);
    let unknown = StringArray::from(vec![Some("x")]);
    write_parquet(
        &dir.join("documents.parquet"),
        &schema,
        vec![
            Arc::new(ids),
            Arc::new(hashes),
            Arc::new(years),
            Arc::new(unknown),
        ],
    );
    write_passages(&dir, &[("doc-a", 0, "text-a", None, unit_vector())]);
    write_manifest(&dir, (1, 1, 0), "upsert");

    let error = assert_invalid(open(&dir), "unknown column");
    assert!(
        error.message().contains("inconnu"),
        "the message must name the column: {}",
        error.message()
    );
}

/// A declared field missing from the Parquet file is refused (control 5).
#[test]
fn open_refuses_a_missing_column() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = root.path().join("mini");
    fs::create_dir_all(&dir).expect("the fixture batch directory is created");
    let schema = Arc::new(ArrowSchema::new(vec![
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("doc_hash", DataType::Utf8, false),
    ]));
    let ids = StringArray::from(vec!["doc-a"]);
    let hashes = StringArray::from(vec!["hash-a"]);
    write_parquet(
        &dir.join("documents.parquet"),
        &schema,
        vec![Arc::new(ids), Arc::new(hashes)],
    );
    write_passages(&dir, &[("doc-a", 0, "text-a", None, unit_vector())]);
    write_manifest(&dir, (1, 1, 0), "upsert");

    let error = assert_invalid(open(&dir), "missing column");
    assert!(
        error.message().contains("annee"),
        "the message must name the column: {}",
        error.message()
    );
}

/// A column of the wrong type is refused (control 5).
#[test]
fn open_refuses_a_column_of_the_wrong_type() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = root.path().join("mini");
    fs::create_dir_all(&dir).expect("the fixture batch directory is created");
    let schema = Arc::new(ArrowSchema::new(vec![
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("doc_hash", DataType::Utf8, false),
        Field::new("annee", DataType::Utf8, true),
    ]));
    let ids = StringArray::from(vec!["doc-a"]);
    let hashes = StringArray::from(vec!["hash-a"]);
    let years = StringArray::from(vec![Some("2020")]);
    write_parquet(
        &dir.join("documents.parquet"),
        &schema,
        vec![Arc::new(ids), Arc::new(hashes), Arc::new(years)],
    );
    write_passages(&dir, &[("doc-a", 0, "text-a", None, unit_vector())]);
    write_manifest(&dir, (1, 1, 0), "upsert");

    let error = assert_invalid(open(&dir), "wrong column type");
    assert!(
        error.message().contains("Int64"),
        "the message must give the expected type: {}",
        error.message()
    );
}

/// A vector column whose width is not `dims × 2` bytes is refused (control 5).
#[test]
fn open_refuses_a_vector_of_the_wrong_width() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = root.path().join("mini");
    fs::create_dir_all(&dir).expect("the fixture batch directory is created");
    write_documents(&dir, &[("doc-a", "hash-a", Some(2020))]);
    let schema = passage_schema_of_width(6);
    write_passages_with(
        &dir,
        &schema,
        &[("doc-a", 0, "text-a", None, fp16(&[1.0, 0.0, 0.0]))],
    );
    write_manifest(&dir, (1, 1, 0), "upsert");

    let error = assert_invalid(open(&dir), "wrong vector width");
    assert!(
        error.message().contains("FixedSizeBinary"),
        "the message must give the expected type: {}",
        error.message()
    );
}

/// The counts of the manifest must match the rows read (control 6).
#[test]
fn validate_refuses_a_count_that_differs_from_the_files() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = minimal_batch(root.path());
    write_manifest(&dir, (1, 2, 0), "upsert");

    let reader = open(&dir).expect("the schema is still valid");
    let error = assert_invalid(reader.validate(), "count mismatch");
    assert!(
        error.message().contains("2 passages"),
        "the message must give the declared count: {}",
        error.message()
    );
    assert!(
        error.message().contains("1 rows"),
        "the message must give the read count: {}",
        error.message()
    );
}

/// The `ord`s of a document must be contiguous from 0 (control 6).
#[test]
fn validate_refuses_a_non_contiguous_ord() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = root.path().join("mini");
    fs::create_dir_all(&dir).expect("the fixture batch directory is created");
    write_documents(&dir, &[("doc-a", "hash-a", Some(2020))]);
    write_passages(
        &dir,
        &[
            ("doc-a", 0, "text-a", None, unit_vector()),
            ("doc-a", 2, "text-b", None, unit_vector()),
        ],
    );
    write_manifest(&dir, (1, 2, 0), "upsert");

    let reader = open(&dir).expect("the batch opens");
    let error = assert_invalid(reader.validate(), "non-contiguous ord");
    assert!(
        error.message().contains("ascending ord order"),
        "the message must tell the producer that the rows must arrive in ascending ord \
         order: {}",
        error.message()
    );
}

/// The rows of one document must arrive in ascending `ord` order. Format v1
/// requires it (ADR-0039): the reader then holds one entry per document instead
/// of a state proportional to the passages.
#[test]
fn validate_refuses_the_rows_of_a_document_out_of_ord_order() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = root.path().join("mini");
    fs::create_dir_all(&dir).expect("the fixture batch directory is created");
    write_documents(&dir, &[("doc-a", "hash-a", Some(2020))]);
    write_passages(
        &dir,
        &[
            ("doc-a", 1, "text-b", None, unit_vector()),
            ("doc-a", 0, "text-a", None, unit_vector()),
        ],
    );
    write_manifest(&dir, (1, 2, 0), "upsert");

    let reader = open(&dir).expect("the batch opens");
    let error = assert_invalid(reader.validate(), "rows permuted within a document");
    assert!(
        error.message().contains("ascending ord order"),
        "the message must tell the producer to write the rows in ord order: {}",
        error.message()
    );
}

/// Every document has at least one passage (control 6).
#[test]
fn validate_refuses_a_document_without_a_passage() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = root.path().join("mini");
    fs::create_dir_all(&dir).expect("the fixture batch directory is created");
    write_documents(
        &dir,
        &[("doc-a", "hash-a", Some(2020)), ("doc-b", "hash-b", None)],
    );
    write_passages(
        &dir,
        &[
            ("doc-a", 0, "text-a", None, unit_vector()),
            ("doc-a", 1, "text-b", None, unit_vector()),
        ],
    );
    write_manifest(&dir, (2, 2, 0), "upsert");

    let reader = open(&dir).expect("the batch opens");
    let error = assert_invalid(reader.validate(), "document without a passage");
    assert!(
        error.message().contains("doc-b"),
        "the message must name the document: {}",
        error.message()
    );
}

/// A passage must belong to a document declared by the batch (control 6).
#[test]
fn validate_refuses_a_passage_of_an_undeclared_document() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = root.path().join("mini");
    fs::create_dir_all(&dir).expect("the fixture batch directory is created");
    write_documents(&dir, &[("doc-a", "hash-a", Some(2020))]);
    write_passages(&dir, &[("doc-b", 0, "text-a", None, unit_vector())]);
    write_manifest(&dir, (1, 1, 0), "upsert");

    let reader = open(&dir).expect("the batch opens");
    let error = assert_invalid(reader.validate(), "undeclared document");
    assert!(
        error.message().contains("doc-b"),
        "the message must name the document: {}",
        error.message()
    );
}

/// Vectors must be finite (control 7).
#[test]
fn validate_refuses_a_non_finite_vector() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = root.path().join("mini");
    fs::create_dir_all(&dir).expect("the fixture batch directory is created");
    write_documents(&dir, &[("doc-a", "hash-a", Some(2020))]);
    write_passages(
        &dir,
        &[("doc-a", 0, "text-a", None, fp16(&[f32::NAN, 0.0]))],
    );
    write_manifest(&dir, (1, 1, 0), "upsert");

    let reader = open(&dir).expect("the batch opens");
    let error = assert_invalid(reader.validate(), "non-finite vector");
    assert!(
        error.message().contains("not finite"),
        "the message must report the component: {}",
        error.message()
    );
}

/// A normalized batch must hold vectors of L2 norm close to 1 (control 7).
#[test]
fn validate_refuses_an_unnormalized_vector() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = root.path().join("mini");
    fs::create_dir_all(&dir).expect("the fixture batch directory is created");
    write_documents(&dir, &[("doc-a", "hash-a", Some(2020))]);
    write_passages(&dir, &[("doc-a", 0, "text-a", None, fp16(&[1.0, 1.0]))]);
    write_manifest(&dir, (1, 1, 0), "upsert");

    let reader = open(&dir).expect("the batch opens");
    let error = assert_invalid(reader.validate(), "unnormalized vector");
    assert!(
        error.message().contains("L2 norm"),
        "the message must give the norm: {}",
        error.message()
    );
}

/// `normalize: false` accepts a vector whose L2 norm is far from 1: the norm
/// check is conditioned by the model, not applied to every batch.
#[test]
fn validate_accepts_unnormalized_vectors_when_the_model_does_not_normalize() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = root.path().join("mini");
    fs::create_dir_all(&dir).expect("the fixture batch directory is created");
    write_documents(&dir, &[("doc-a", "hash-a", Some(2020))]);
    write_passages(&dir, &[("doc-a", 0, "text-a", None, fp16(&[1.0, 1.0]))]);
    write_manifest_with(&dir, (1, 1, 0), "upsert", false);

    let reader = open_with(&dir, &unnormalized_model()).expect("the batch opens");
    let counts = reader
        .validate()
        .expect("a norm of about 1.414 is accepted when the batch does not normalize");
    assert_eq!(counts.passages, 1);
}

/// The norm tolerance is 1e-2: just inside is accepted, just outside is
/// refused.
#[test]
fn validate_applies_the_norm_tolerance_at_its_boundary() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let accepted = root.path().join("accepted");
    fs::create_dir_all(&accepted).expect("the fixture batch directory is created");
    write_documents(&accepted, &[("doc-a", "hash-a", Some(2020))]);
    write_passages(
        &accepted,
        &[("doc-a", 0, "text-a", None, fp16(&[1.009, 0.0]))],
    );
    write_manifest(&accepted, (1, 1, 0), "upsert");

    let reader = open(&accepted).expect("the batch opens");
    reader
        .validate()
        .expect("a norm of 1.009 is within the 1e-2 tolerance");

    let refused = root.path().join("refused");
    fs::create_dir_all(&refused).expect("the fixture batch directory is created");
    write_documents(&refused, &[("doc-a", "hash-a", Some(2020))]);
    write_passages(
        &refused,
        &[("doc-a", 0, "text-a", None, fp16(&[1.011, 0.0]))],
    );
    write_manifest(&refused, (1, 1, 0), "upsert");

    let reader = open(&refused).expect("the batch opens");
    let error = assert_invalid(reader.validate(), "norm just outside the tolerance");
    assert!(
        error.message().contains("L2 norm"),
        "the message must give the norm: {}",
        error.message()
    );
}

/// A document may be split across two `passages-*.parquet` parts: the reader
/// visits the parts in increasing suffix order and the `ord` sequence continues
/// across the boundary.
#[test]
fn validate_reads_a_document_split_across_two_parts() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = root.path().join("mini");
    fs::create_dir_all(&dir).expect("the fixture batch directory is created");
    write_documents(&dir, &[("doc-a", "hash-a", Some(2020))]);
    write_passages_named(
        &dir,
        "passages-0001.parquet",
        &passage_schema(),
        &[("doc-a", 0, "text-a", None, unit_vector())],
    );
    write_passages_named(
        &dir,
        "passages-0002.parquet",
        &passage_schema(),
        &[("doc-a", 1, "text-b", None, unit_vector())],
    );
    write_manifest(&dir, (1, 2, 0), "upsert");

    let reader = open(&dir).expect("the batch opens");
    let counts = reader
        .validate()
        .expect("the ord sequence continues from one part to the next");
    assert_eq!(counts.passages, 2);

    let mut chunks = reader.passages();
    let first = chunks
        .next()
        .expect("the first part is read")
        .expect("the first part decodes");
    let second = chunks
        .next()
        .expect("the second part is read")
        .expect("the second part decodes");
    assert!(chunks.next().is_none(), "the batch holds two parts only");
    assert_eq!(ords_of(&first), vec![0], "the parts are visited in order");
    assert_eq!(ords_of(&second), vec![1], "the ords continue across parts");
}

/// The parts of a role are visited in increasing numeric value of their
/// suffix, whatever the number of digits (ADR-0040): `passages-9999.parquet`
/// comes before `passages-10000.parquet`, so the `ord` sequence of a document
/// continues across the digit boundary.
#[test]
fn validate_reads_parts_in_numeric_suffix_order_across_a_digit_boundary() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = root.path().join("mini");
    fs::create_dir_all(&dir).expect("the fixture batch directory is created");
    write_documents(&dir, &[("doc-a", "hash-a", Some(2020))]);
    write_passages_named(
        &dir,
        "passages-9999.parquet",
        &passage_schema(),
        &[("doc-a", 0, "text-a", None, unit_vector())],
    );
    write_passages_named(
        &dir,
        "passages-10000.parquet",
        &passage_schema(),
        &[("doc-a", 1, "text-b", None, unit_vector())],
    );
    write_manifest(&dir, (1, 2, 0), "upsert");

    let reader = open(&dir).expect("the batch opens");
    let counts = reader
        .validate()
        .expect("the ord sequence continues across the digit boundary");
    assert_eq!(counts.passages, 2);

    let mut chunks = reader.passages();
    let first = chunks
        .next()
        .expect("the first part is read")
        .expect("the first part decodes");
    assert_eq!(
        ords_of(&first),
        vec![0],
        "passages-9999.parquet is read before passages-10000.parquet"
    );
}

/// Two parts of one role with the same numeric suffix leave the part order
/// undefined and are refused (ADR-0040).
#[test]
fn open_refuses_two_parts_with_the_same_numeric_suffix() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = root.path().join("mini");
    fs::create_dir_all(&dir).expect("the fixture batch directory is created");
    write_documents(&dir, &[("doc-a", "hash-a", Some(2020))]);
    write_passages_named(
        &dir,
        "passages-0001.parquet",
        &passage_schema(),
        &[("doc-a", 0, "text-a", None, unit_vector())],
    );
    write_passages_named(
        &dir,
        "passages-00001.parquet",
        &passage_schema(),
        &[("doc-a", 1, "text-b", None, unit_vector())],
    );
    write_manifest(&dir, (1, 2, 0), "upsert");

    let error = assert_invalid(open(&dir), "two parts numbered 1");
    assert!(
        error.message().contains("same numeric suffix"),
        "the message must explain the repeated value: {}",
        error.message()
    );
}

/// A part suffix that no integer can hold is refused as a fault of the batch,
/// never by a panic (ADR-0040).
#[test]
fn open_refuses_a_part_suffix_too_large_for_an_integer() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = root.path().join("mini");
    fs::create_dir_all(&dir).expect("the fixture batch directory is created");
    write_documents(&dir, &[("doc-a", "hash-a", Some(2020))]);
    write_passages(&dir, &[("doc-a", 0, "text-a", None, unit_vector())]);
    let name = "passages-99999999999999999999.parquet";
    fs::write(dir.join(name), b"").expect("the over-long part is created");
    write_manifest(&dir, (1, 1, 0), "upsert");

    let error = assert_invalid(open(&dir), "a suffix above u64");
    assert!(
        error.message().contains("too large"),
        "the message must say the suffix is too large: {}",
        error.message()
    );
}

// --- ADR-0040: a doc_id appears at most once in documents and in deletions ---

/// A `doc_id` declared twice inside one `documents` part is refused, naming
/// the identifier (ADR-0040).
#[test]
fn validate_refuses_a_doc_id_duplicated_within_one_documents_part() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = root.path().join("mini");
    fs::create_dir_all(&dir).expect("the fixture batch directory is created");
    write_documents(
        &dir,
        &[
            ("doc-a", "hash-a", Some(2020)),
            ("doc-a", "hash-b", Some(2021)),
        ],
    );
    write_passages(&dir, &[("doc-a", 0, "text-a", None, unit_vector())]);
    write_manifest(&dir, (1, 1, 0), "upsert");

    let reader = open(&dir).expect("the batch opens");
    let error = assert_invalid(reader.validate(), "duplicated doc_id in documents");
    assert!(
        error.message().contains("doc-a"),
        "the message must name the doc_id: {}",
        error.message()
    );
    assert!(
        error.message().contains("twice"),
        "the message must say the doc_id is declared twice: {}",
        error.message()
    );
}

/// The uniqueness of a `doc_id` in `documents` spans every part of the batch
/// (ADR-0040).
#[test]
fn validate_refuses_a_doc_id_duplicated_across_two_documents_parts() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = root.path().join("mini");
    fs::create_dir_all(&dir).expect("the fixture batch directory is created");
    write_documents_named(
        &dir,
        "documents-0001.parquet",
        &[("doc-a", "hash-a", Some(2020))],
    );
    write_documents_named(
        &dir,
        "documents-0002.parquet",
        &[("doc-b", "hash-b", None), ("doc-a", "hash-a", Some(2020))],
    );
    write_passages(
        &dir,
        &[
            ("doc-a", 0, "text-a", None, unit_vector()),
            ("doc-b", 0, "text-b", None, unit_vector()),
        ],
    );
    write_manifest(&dir, (2, 2, 0), "upsert");

    let reader = open(&dir).expect("the batch opens");
    let error = assert_invalid(reader.validate(), "duplicated doc_id across two parts");
    assert!(
        error.message().contains("doc-a"),
        "the message must name the doc_id: {}",
        error.message()
    );
    assert!(
        error.message().contains("twice"),
        "the message must say the doc_id is declared twice: {}",
        error.message()
    );
}

/// A `doc_id` declared twice in `deletions` is refused, naming the identifier
/// (ADR-0040).
#[test]
fn validate_refuses_a_doc_id_duplicated_in_deletions() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = root.path().join("mini");
    fs::create_dir_all(&dir).expect("the fixture batch directory is created");
    write_documents(&dir, &[("doc-a", "hash-a", Some(2020))]);
    write_passages(&dir, &[("doc-a", 0, "text-a", None, unit_vector())]);
    write_deletions(&dir, &["doc-b", "doc-b"]);
    write_manifest(&dir, (1, 1, 2), "upsert");

    let reader = open(&dir).expect("the batch opens");
    let error = assert_invalid(reader.validate(), "duplicated doc_id in deletions");
    assert!(
        error.message().contains("doc-b"),
        "the message must name the doc_id: {}",
        error.message()
    );
    assert!(
        error.message().contains("twice"),
        "the message must say the doc_id is declared twice: {}",
        error.message()
    );
}

/// A batch with distinct deletions and distinct documents stays accepted.
#[test]
fn validate_accepts_distinct_deletions() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = root.path().join("mini");
    fs::create_dir_all(&dir).expect("the fixture batch directory is created");
    write_documents(&dir, &[("doc-a", "hash-a", Some(2020))]);
    write_passages(&dir, &[("doc-a", 0, "text-a", None, unit_vector())]);
    write_deletions(&dir, &["doc-b", "doc-c"]);
    write_manifest(&dir, (1, 1, 2), "upsert");

    let reader = open(&dir).expect("the batch opens");
    let counts = reader
        .validate()
        .expect("two distinct deletions are part of the format");
    assert_eq!(counts.deletions, 2);
}

/// A `deletions.parquet` that holds more rows than `counts.deletions` is
/// refused before the excess ids are stored, so the memory of validation stays
/// bounded by the manifest (mirror of the documents count).
#[test]
fn validate_refuses_more_deletions_than_the_manifest_declares() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = root.path().join("mini");
    fs::create_dir_all(&dir).expect("the fixture batch directory is created");
    write_documents(&dir, &[("doc-a", "hash-a", Some(2020))]);
    write_passages(&dir, &[("doc-a", 0, "text-a", None, unit_vector())]);
    write_deletions(&dir, &["doc-b", "doc-c", "doc-d"]);
    write_manifest(&dir, (1, 1, 1), "upsert");

    let reader = open(&dir).expect("the batch opens");
    let error = assert_invalid(reader.validate(), "deletions above the declared count");
    assert!(
        error
            .message()
            .contains("more deletions than counts.deletions"),
        "the message must report the excess: {}",
        error.message()
    );
    assert!(
        error.message().contains("(1)"),
        "the message must give the declared count: {}",
        error.message()
    );
}

/// The `ord` column of one record batch.
fn ords_of(chunk: &RecordBatch) -> Vec<u32> {
    let column = chunk
        .column_by_name("ord")
        .and_then(|column| column.as_any().downcast_ref::<UInt32Array>())
        .unwrap_or_else(|| panic!("the fixture batch has an ord column"));
    (0..column.len()).map(|index| column.value(index)).collect()
}
