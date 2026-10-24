// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `ENCODER-DOCUMENTS-EN-LIGNE`: the internal batch of the online mode.
//!
//! References: `docs/phases/P2-encodeur-et-recette.md` (task
//! `ENCODER-DOCUMENTS-EN-LIGNE`), ADR-0055, `docs/02-architecture.md`
//! ("Reprise"), `docs/04-donnees-et-ingestion.md` ("Mode en ligne"),
//! `docs/05-api.md` ("Lot en ligne") and `docs/format-lot-v1.md` ("Lot interne
//! du mode en ligne").
//!
//! These tests need no ONNX Runtime: the index model is `external`, so every
//! passage brings its vector. They pin the input file, the deterministic
//! manifest, the default `doc_hash`, the refusal of `replace_all`, the cap of
//! 10 000 passages, the guard that removes the temporary directory, the
//! presence lock of ADR-0056 (contention, live lot, orphan lock) and the
//! recovery of stale online batches. The encoding path of an `onnx` model is
//! covered by `crates/crible/tests/lot_en_ligne.rs`, whose ONNX tests are
//! `#[ignore]`d.

mod common;

use std::fs::{self, File};
use std::path::{Path, PathBuf};

use crible_core::batch::BatchMode;
use crible_core::config::{IndexConfig, Schema};
use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::Epoch;
use crible_core::model::{ModelConfig, ModelKind};
use crible_store::attach::AttachReport;
use crible_store::generation::{GenerationStore, write_index_config};
use crible_store::online::{self, OnlineDocument, OnlinePassage};
use serde_json::{Map, Value, json};

/// Base64 of the unit vector of the first dimension, four fp32 components.
const VECTOR_1: &str = "AACAPwAAAAAAAAAAAAAAAA==";

/// Base64 of the unit vector of the second dimension.
const VECTOR_2: &str = "AAAAAAAAgD8AAAAAAAAAAA==";

/// Base64 of the unit vector of the third dimension.
const VECTOR_3: &str = "AAAAAAAAAAAAAIA/AAAAAA==";

/// Base64 of the unit vector of the fourth dimension.
const VECTOR_4: &str = "AAAAAAAAAAAAAAAAAACAPw==";

/// Base64 of three fp32 components: one dimension short.
const VECTOR_SHORT: &str = "AACAPwAAAAAAgD8=";

/// Schema of these tests: every field type, at both levels.
fn online_schema() -> Schema {
    Schema::from_json_str(
        r#"{
            "document_fields": {
                "titre": { "type": "keyword" },
                "annee": { "type": "integer" }
            },
            "passage_fields": {
                "partie": { "type": "keyword" },
                "jour": { "type": "date" },
                "poids": { "type": "float" },
                "utile": { "type": "bool" }
            }
        }"#,
    )
    .unwrap_or_else(|error| panic!("the online schema must be valid: {error}"))
}

/// `external` model of four dimensions, normalized: no encoder, so every
/// passage of the online lot must bring its vector.
fn external_model() -> ModelConfig {
    ModelConfig::from_json_str(r#"{"name":"m","kind":"external","dims":4,"normalize":true}"#)
        .unwrap_or_else(|error| panic!("the external model configuration must be valid: {error}"))
}

/// Creates the index of these tests and returns its configuration and store.
fn create_index(temp: &tempfile::TempDir) -> (PathBuf, IndexConfig, GenerationStore) {
    let schema = online_schema();
    let index = temp.path().join("indexes").join("invariants");
    let raw = common::test_config("invariants", &schema);
    write_index_config(&index, &raw)
        .unwrap_or_else(|error| panic!("the index configuration must be written: {error}"));
    let store = GenerationStore::create(&index, Epoch::new(common::EPOCH), &schema)
        .unwrap_or_else(|error| panic!("the index must be created: {error}"));
    let config = IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("the index configuration must parse: {error}"));
    (index, config, store)
}

/// One passage with all its fields and a provided vector.
fn passage(text: &str, part: &str, vector: &str) -> OnlinePassage {
    let mut fields = Map::new();
    fields.insert("partie".to_owned(), json!(part));
    fields.insert("jour".to_owned(), json!("2020-01-31"));
    fields.insert("poids".to_owned(), json!(1.5));
    fields.insert("utile".to_owned(), json!(true));
    OnlinePassage {
        text: text.to_owned(),
        fields,
        vector: Some(vector.to_owned()),
    }
}

/// One document with two document fields.
fn document(doc_id: &str, hash: Option<&str>, passages: Vec<OnlinePassage>) -> OnlineDocument {
    let mut fields = Map::new();
    fields.insert("titre".to_owned(), json!("un titre"));
    fields.insert("annee".to_owned(), json!(2020));
    OnlineDocument {
        doc_id: doc_id.to_owned(),
        doc_hash: hash.map(str::to_owned),
        fields,
        passages,
    }
}

/// Writes the JSONL file of `documents` under `directory`.
fn write_documents(directory: &Path, name: &str, documents: &[OnlineDocument]) -> PathBuf {
    let mut raw = String::new();
    for document in documents {
        let line = serde_json::to_string(document)
            .unwrap_or_else(|error| panic!("a document must serialize: {error}"));
        raw.push_str(&line);
        raw.push('\n');
    }
    let path = directory.join(name);
    fs::write(&path, raw)
        .unwrap_or_else(|error| panic!("the input file must be writable: {error}"));
    path
}

/// Three documents, four passages in all, one supplied `doc_hash`.
fn fixture_documents() -> Vec<OnlineDocument> {
    vec![
        document(
            "D1",
            None,
            vec![
                passage("article un", "corps", VECTOR_1),
                passage("article deux", "expose", VECTOR_2),
            ],
        ),
        document(
            "D2",
            None,
            vec![passage("article trois", "corps", VECTOR_3)],
        ),
        document(
            "D3",
            Some("blake3:custom"),
            vec![passage("article quatre", "annexe", VECTOR_4)],
        ),
    ]
}

/// Ingests one file with the external model of these tests.
fn ingest(
    store: &mut GenerationStore,
    documents: &Path,
    data: &Path,
    config: &IndexConfig,
    mode: BatchMode,
) -> Result<AttachReport, CribleError> {
    let model = external_model();
    assert_eq!(model.kind, ModelKind::External);
    online::ingest(
        store,
        &online::OnlineIngest {
            documents,
            data_dir: data,
            index_config: config,
            model_config: &model,
            token_counter: None,
            document_encoder: None,
            mode,
        },
    )
}

/// Reads one text column of one row of the epoch database.
fn query_string(records: &Path, sql: &str, parameter: &str) -> String {
    let connection =
        rusqlite::Connection::open_with_flags(records, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap_or_else(|error| panic!("the records database must open: {error}"));
    connection
        .query_row(sql, [parameter], |row| row.get::<_, String>(0))
        .unwrap_or_else(|error| panic!("the query must return one row: {error}"))
}

/// Reads the stored document hash of `doc_id`.
fn stored_doc_hash(records: &Path, doc_id: &str) -> String {
    query_string(
        records,
        "SELECT doc_hash FROM documents WHERE doc_id = ?1",
        doc_id,
    )
}

/// Reads the stored manifest of `batch_id` and returns it parsed.
fn stored_manifest(records: &Path, batch_id: &str) -> Value {
    let raw = query_string(
        records,
        "SELECT manifest FROM batches WHERE batch_id = ?1",
        batch_id,
    );
    serde_json::from_str(&raw).unwrap_or_else(|error| panic!("the manifest must parse: {error}"))
}

/// Names the `online-*` entries of `<data>/tmp`.
fn online_entries(data: &Path) -> Vec<String> {
    let root = data.join(online::TEMPORARY_DIRECTORY);
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("online-"))
        .collect();
    names.sort();
    names
}

/// Canonical JSON of the passages of `document`, spelled independently of the
/// implementation: the ADR fixes these bytes.
fn canonical_passages(document: &OnlineDocument) -> String {
    let passages: Vec<Value> = document
        .passages
        .iter()
        .enumerate()
        .map(|(ord, passage)| {
            json!({
                "ord": ord,
                "text": passage.text,
                "fields": passage.fields,
            })
        })
        .collect();
    serde_json::to_string(&Value::Array(passages))
        .unwrap_or_else(|error| panic!("the passages must serialize: {error}"))
}

#[test]
fn online_ingest_with_provided_vectors_attaches_and_removes_the_temporary_batch() {
    let temp = tempfile::tempdir().expect("a temporary directory is creatable");
    let (index, config, mut store) = create_index(&temp);
    let documents = fixture_documents();
    let input = write_documents(temp.path(), "documents.jsonl", &documents);

    let report = ingest(&mut store, &input, temp.path(), &config, BatchMode::Upsert)
        .expect("the online lot must attach");

    assert_eq!(report.generation.get(), 1);
    assert_eq!(report.added, 3, "three documents are added");
    let state = store.state().expect("the state is readable");
    assert_eq!(state.documents, 3);
    assert_eq!(state.live_passages, 4);
    assert_eq!(
        online_entries(temp.path()),
        Vec::<String>::new(),
        "the temporary batch must be removed, on success too"
    );

    // The manifest of the internal lot is the one of ADR-0055.
    let manifest = stored_manifest(&store.records_path(), &report.batch_id);
    assert_eq!(manifest["format"], "crible-batch");
    assert_eq!(manifest["format_version"], 1);
    assert_eq!(manifest["index"], "invariants");
    assert_eq!(manifest["mode"], "upsert");
    assert_eq!(manifest["created_at"], "1970-01-01T00:00:00Z");
    assert_eq!(manifest["producer"]["name"], "crible-online");
    assert_eq!(manifest["producer"]["version"], "1");
    assert_eq!(manifest["producer"]["host"], "crible-online");
    assert_eq!(manifest["chunker"]["name"], "none");
    assert_eq!(manifest["chunker"]["version"], "1");
    assert_eq!(manifest["template"]["name"], "none");
    assert_eq!(manifest["template"]["version"], "1");
    assert_eq!(manifest["counts"]["documents"], 3);
    assert_eq!(manifest["counts"]["passages"], 4);
    assert_eq!(manifest["counts"]["deletions"], 0);
    // The index stores fp16: the internal lot converts the fp32 vectors and
    // declares that precision, so control 3 of `docs/04` accepts it.
    assert_eq!(manifest["model"]["vector_dtype"], "fp16");
    assert!(
        report.batch_id.starts_with("online-"),
        "the batch id is derived from the file, got '{}'",
        report.batch_id
    );
    assert!(
        manifest["files"]["documents.parquet"]
            .as_str()
            .is_some_and(|checksum| checksum.starts_with("blake3:")),
        "the manifest carries the checksums of both files"
    );

    // A supplied doc_hash is kept; an absent one is derived from the passages.
    let records = store.records_path();
    assert_eq!(stored_doc_hash(&records, "D3"), "blake3:custom");
    let expected = format!(
        "blake3:{}",
        blake3::hash(canonical_passages(&documents[0]).as_bytes()).to_hex()
    );
    assert_eq!(stored_doc_hash(&records, "D1"), expected);
    assert!(index.join("CURRENT_EPOCH").is_file());
}

#[test]
fn replaying_the_same_file_changes_nothing() {
    let temp = tempfile::tempdir().expect("a temporary directory is creatable");
    let (_index, config, mut store) = create_index(&temp);
    let input = write_documents(temp.path(), "documents.jsonl", &fixture_documents());

    let first = ingest(&mut store, &input, temp.path(), &config, BatchMode::Upsert)
        .expect("the first ingest must attach");
    let before = store.state().expect("the state is readable");
    let second = ingest(&mut store, &input, temp.path(), &config, BatchMode::Upsert)
        .expect("replaying the same file must report the original attach");

    let after = store.state().expect("the state is readable");
    assert_eq!(second.batch_id, first.batch_id);
    assert_eq!(second.added, first.added);
    assert_eq!(after.generation, before.generation, "INV-10: no generation");
    assert_eq!(after.documents, before.documents);
    assert_eq!(after.live_passages, before.live_passages);
    assert_eq!(after.segments.len(), before.segments.len());
    assert_eq!(online_entries(temp.path()), Vec::<String>::new());
}

#[test]
fn a_copy_of_the_same_bytes_is_the_same_batch() {
    let temp = tempfile::tempdir().expect("a temporary directory is creatable");
    let (_index, config, mut store) = create_index(&temp);
    let input = write_documents(temp.path(), "documents.jsonl", &fixture_documents());
    let copy = temp.path().join("copied.jsonl");
    fs::copy(&input, &copy).expect("the copy must be writable");

    let first = ingest(&mut store, &input, temp.path(), &config, BatchMode::Upsert)
        .expect("the first ingest must attach");
    let generation = store.state().expect("the state is readable").generation;
    let second = ingest(&mut store, &copy, temp.path(), &config, BatchMode::Upsert)
        .expect("the same bytes under another path is the same batch");

    assert_eq!(
        second.batch_id, first.batch_id,
        "the batch id comes from the bytes of the file, not from its path"
    );
    assert_eq!(
        store.state().expect("the state is readable").generation,
        generation,
        "a copy replays the same batch, so no generation is committed"
    );
}

#[test]
fn the_doc_hash_defaults_to_the_canonical_passages() {
    let temp = tempfile::tempdir().expect("a temporary directory is creatable");
    let (_index, config, mut store) = create_index(&temp);
    let before = document("D1", None, vec![passage("article un", "corps", VECTOR_1)]);
    let after = document("D2", None, vec![passage("article un", "expose", VECTOR_1)]);
    let input = write_documents(temp.path(), "documents.jsonl", &[before.clone(), after]);
    ingest(&mut store, &input, temp.path(), &config, BatchMode::Upsert)
        .expect("the online lot must attach");

    let records = store.records_path();
    let hash = stored_doc_hash(&records, "D1");
    assert_eq!(
        hash,
        format!(
            "blake3:{}",
            blake3::hash(canonical_passages(&before).as_bytes()).to_hex()
        )
    );
    assert_ne!(
        hash,
        stored_doc_hash(&records, "D2"),
        "a change of a passage field alone must change the doc_hash"
    );
}

#[test]
fn replace_all_is_refused_without_touching_the_index() {
    let temp = tempfile::tempdir().expect("a temporary directory is creatable");
    let (_index, config, mut store) = create_index(&temp);
    let input = write_documents(temp.path(), "documents.jsonl", &fixture_documents());

    let error = ingest(
        &mut store,
        &input,
        temp.path(),
        &config,
        BatchMode::ReplaceAll,
    )
    .expect_err("replace_all is refused in the online lot");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(
        error.message().contains("replace_all"),
        "the message names the refused mode, got: {}",
        error.message()
    );
    let state = store.state().expect("the state is readable");
    assert_eq!(state.generation.get(), 0);
    assert_eq!(state.documents, 0);
    assert_eq!(
        online_entries(temp.path()),
        Vec::<String>::new(),
        "a refused mode writes nothing"
    );
}

#[test]
fn a_vector_of_the_wrong_width_is_refused() {
    let temp = tempfile::tempdir().expect("a temporary directory is creatable");
    let (_index, config, mut store) = create_index(&temp);
    let broken = passage("article un", "corps", VECTOR_SHORT);
    let input = write_documents(
        temp.path(),
        "documents.jsonl",
        &[document("D1", None, vec![broken])],
    );

    let error = ingest(&mut store, &input, temp.path(), &config, BatchMode::Upsert)
        .expect_err("a vector of three components for four dimensions is refused");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(
        error.message().contains("D1#0"),
        "the message names the passage, got: {}",
        error.message()
    );
    assert_eq!(
        store
            .state()
            .expect("the state is readable")
            .generation
            .get(),
        0
    );
    assert_eq!(online_entries(temp.path()), Vec::<String>::new());
}

#[test]
fn a_passage_without_a_vector_is_refused_on_an_external_model() {
    let temp = tempfile::tempdir().expect("a temporary directory is creatable");
    let (_index, config, mut store) = create_index(&temp);
    let mut without = passage("article un", "corps", VECTOR_1);
    without.vector = None;
    let input = write_documents(
        temp.path(),
        "documents.jsonl",
        &[document("D1", None, vec![without])],
    );

    let error = ingest(&mut store, &input, temp.path(), &config, BatchMode::Upsert)
        .expect_err("an external model has no encoder");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(error.message().contains("D1#0"));
    assert!(
        error.message().contains("external"),
        "the message says the model has no encoder, got: {}",
        error.message()
    );
    assert_eq!(online_entries(temp.path()), Vec::<String>::new());
}

#[test]
fn an_undeclared_field_is_refused_without_writing() {
    let temp = tempfile::tempdir().expect("a temporary directory is creatable");
    let (_index, config, mut store) = create_index(&temp);
    let mut document = document("D1", None, vec![passage("article un", "corps", VECTOR_1)]);
    document
        .fields
        .insert("inconnu".to_owned(), json!("valeur"));
    let input = write_documents(temp.path(), "documents.jsonl", &[document]);

    let error = ingest(&mut store, &input, temp.path(), &config, BatchMode::Upsert)
        .expect_err("an undeclared field is refused");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(
        error.message().contains("inconnu"),
        "the message names the field, got: {}",
        error.message()
    );
    assert!(
        !error.message().contains("valeur"),
        "the message must not carry a field value"
    );
    assert_eq!(online_entries(temp.path()), Vec::<String>::new());
}

#[test]
fn an_empty_input_file_is_refused() {
    let temp = tempfile::tempdir().expect("a temporary directory is creatable");
    let (_index, config, mut store) = create_index(&temp);
    let input = write_documents(temp.path(), "documents.jsonl", &[]);

    let error = ingest(&mut store, &input, temp.path(), &config, BatchMode::Upsert)
        .expect_err("an empty file holds no document");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert_eq!(
        store
            .state()
            .expect("the state is readable")
            .generation
            .get(),
        0
    );
}

#[test]
fn an_unlocked_online_batch_is_removed_with_its_lock() {
    let temp = tempfile::tempdir().expect("a temporary directory is creatable");
    let root = temp.path().join(online::TEMPORARY_DIRECTORY);
    let stale = root.join("online-0123456789abcdef");
    fs::create_dir_all(&stale).expect("the stale directory is creatable");
    fs::write(stale.join("manifest.json"), "{}").expect("the stale file is writable");
    let lock = root.join("online-0123456789abcdef.lock");
    fs::write(&lock, "").expect("the lock file is creatable");
    let kept = root.join("autre");
    fs::create_dir_all(&kept).expect("the other directory is creatable");

    online::remove_stale_batches(temp.path()).expect("the recovery must run");
    assert!(
        !stale.exists(),
        "an online-* leftover no writer holds is removed"
    );
    assert!(!lock.exists(), "the presence lock goes with the directory");
    assert!(kept.exists(), "an entry of another name is kept");

    online::remove_stale_batches(temp.path()).expect("the recovery is idempotent");
    assert!(!stale.exists());
    assert!(kept.exists());
}

#[test]
fn a_locked_online_batch_is_not_removed_by_the_cleanup() {
    let temp = tempfile::tempdir().expect("a temporary directory is creatable");
    let root = temp.path().join(online::TEMPORARY_DIRECTORY);
    let batch = root.join("online-0123456789abcdef");
    fs::create_dir_all(&batch).expect("the live directory is creatable");
    let lock_path = root.join("online-0123456789abcdef.lock");
    let held = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .expect("the lock file is creatable");
    held.try_lock().expect("the test takes the lock of the lot");

    online::remove_stale_batches(temp.path()).expect("the recovery must run");
    assert!(
        batch.exists(),
        "a lot whose presence lock is held is a live lot, not a leftover"
    );
    assert!(lock_path.exists(), "its presence lock is kept too");

    held.unlock().expect("the test releases the lock");
    drop(held);
    online::remove_stale_batches(temp.path()).expect("the recovery must run again");
    assert!(!batch.exists(), "the released lot is a leftover");
    assert!(
        !lock_path.exists(),
        "the presence lock goes with the directory"
    );
}

#[test]
fn a_missing_input_file_is_refused_as_an_invalid_request() {
    let temp = tempfile::tempdir().expect("a temporary directory is creatable");
    let (_, config, mut store) = create_index(&temp);
    let missing = temp.path().join("absent.jsonl");

    let error = ingest(
        &mut store,
        &missing,
        temp.path(),
        &config,
        BatchMode::Upsert,
    )
    .expect_err("a missing input file is a caller fault");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert_eq!(
        store
            .state()
            .expect("the state is readable")
            .generation
            .get(),
        0,
        "nothing is written"
    );
    assert!(online_entries(temp.path()).is_empty(), "nothing is created");
}

#[test]
fn more_than_ten_thousand_passages_are_refused() {
    let temp = tempfile::tempdir().expect("a temporary directory is creatable");
    let (_, config, mut store) = create_index(&temp);
    let passages: Vec<Value> = (0..10_001).map(|_| json!({ "text": "droit" })).collect();
    let line = serde_json::to_string(&json!({
        "doc_id": "D1",
        "passages": passages,
    }))
    .expect("the document must serialize");
    let input = temp.path().join("documents.jsonl");
    fs::write(&input, format!("{line}\n")).expect("the input file must be writable");

    let error = ingest(&mut store, &input, temp.path(), &config, BatchMode::Upsert)
        .expect_err("more than 10 000 passages is refused");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    let message = error.to_string();
    assert!(
        message.contains("10000") && message.contains("10001"),
        "the message names the cap and the count: {message}"
    );
    assert!(online_entries(temp.path()).is_empty(), "nothing is created");
}

#[test]
fn an_orphan_presence_lock_is_collected_and_a_held_one_is_kept() {
    let temp = tempfile::tempdir().expect("a temporary directory is creatable");
    let root = temp.path().join(online::TEMPORARY_DIRECTORY);
    fs::create_dir_all(&root).expect("the temporary directory is creatable");
    let free = root.join("online-0123456789abcdef.lock");
    fs::write(&free, "").expect("the orphan lock is creatable");
    let held_path = root.join("online-fedcba9876543210.lock");
    let held = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&held_path)
        .expect("the lock file is creatable");
    held.try_lock().expect("the test takes the lock of the lot");

    online::remove_stale_batches(temp.path()).expect("the recovery must run");
    assert!(
        !free.exists(),
        "an orphan lock file no writer holds is collected"
    );
    assert!(
        held_path.exists(),
        "a held lock file belongs to a writer whose directory is not created yet"
    );

    held.unlock().expect("the test releases the lock");
    drop(held);
    online::remove_stale_batches(temp.path()).expect("the recovery must run again");
    assert!(!held_path.exists(), "the released orphan lock is collected");
}

#[test]
fn a_second_ingestion_of_the_same_file_fails_overloaded_while_the_lock_is_held() {
    let temp = tempfile::tempdir().expect("a temporary directory is creatable");
    let (_, config, mut store) = create_index(&temp);
    let input = write_documents(temp.path(), "documents.jsonl", &fixture_documents());
    let bytes = fs::read(&input).expect("the input file must be readable");
    let batch_id = format!("online-{}", blake3::hash(&bytes).to_hex());
    let root = temp.path().join(online::TEMPORARY_DIRECTORY);
    fs::create_dir_all(&root).expect("the temporary directory is creatable");
    let lock_path = root.join(format!("{batch_id}.lock"));
    let held = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .expect("the lock file is creatable");
    held.try_lock().expect("the test takes the lock of the lot");

    let error = ingest(&mut store, &input, temp.path(), &config, BatchMode::Upsert)
        .expect_err("the same bytes cannot be ingested twice at once");
    assert_eq!(error.code(), ErrorCode::Overloaded);
    assert!(
        !root.join(&batch_id).exists(),
        "a contended ingestion creates no batch directory"
    );
    assert!(
        lock_path.exists(),
        "the loser never unlinks the presence lock of the writer that holds it"
    );
    assert_eq!(
        store
            .state()
            .expect("the state is readable")
            .generation
            .get(),
        0,
        "a contended ingestion writes no batch line"
    );

    held.unlock().expect("the test releases the lock");
    drop(held);
    let report = ingest(&mut store, &input, temp.path(), &config, BatchMode::Upsert)
        .expect("the ingestion succeeds once the lock is released");
    assert_eq!(
        report.added, 3,
        "the three documents of the fixture are attached"
    );
    assert!(
        !root.join(&batch_id).exists() && !lock_path.exists(),
        "the guard removes the directory and its lock"
    );
}
