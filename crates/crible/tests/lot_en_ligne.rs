// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `ENCODER-DOCUMENTS-EN-LIGNE` (phase P2): `crible ingest`.
//!
//! References: `docs/phases/P2-encodeur-et-recette.md` (task
//! `ENCODER-DOCUMENTS-EN-LIGNE`), ADR-0053, ADR-0054, ADR-0055,
//! `docs/02-architecture.md` ("Reprise"), `docs/04-donnees-et-ingestion.md`
//! ("Mode en ligne"), `docs/05-api.md` ("Lot en ligne").
//!
//! # What each test pins
//!
//! | Test | Runs in CI? | Criterion |
//! |---|---|---|
//! | `ingest_refuses_a_document_over_max_tokens_before_encoding` | yes | the length refusal of the online mode happens before the graph is loaded: ONNX Runtime is not even configured, and the index is untouched |
//! | `ingest_refuses_a_document_that_is_not_a_document_without_leaking_its_values` | yes | a malformed line is refused by its line number, without echoing a value |
//! | `only_a_write_opening_removes_the_stale_online_batches` | yes | a read command leaves `<data>/tmp` alone, and a write opening removes a leftover `online-*` with its lock (ADR-0056) |
//! | `a_failed_presence_lock_writes_a_json_line_on_stderr` | yes | the journal installed by the binary writes a JSON object on stderr for a warning (docs/07, ADR-0056) |
//! | `the_online_lot_matches_the_equivalent_batch_with_the_same_vectors` | yes | an online lot and the equivalent batch carrying the same vectors return the same search response |
//! | `one_hundred_documents_ingested_online_are_found_by_a_text_query` | `#[ignore]` | 100 documents encoded online are found by `crible search --query` |
//!
//! # Why the CLI
//!
//! The task ships `crible ingest --data <dir> --models <dir> --index <i>
//! documents.jsonl`; the tests drive that command, so they pin the behaviour
//! and not an internal API. The online lot writes a real batch, so the
//! equivalence test can also attach a batch written by the test with the same
//! vectors and compare the two responses.
//!
//! # Model and ONNX Runtime
//!
//! The versioned fake encoder of `crates/crible-encode/tests/fixtures/fake-model`
//! is copied under the models root. Its WordLevel tokenizer has a
//! `WhitespaceSplit` pre-tokenizer and `max_tokens: 32`, so counting the tokens
//! of a passage needs the tokenizer only. An `external` model copy of 768
//! dimensions, normalized, serves the tests that bring their own vectors: they
//! need no ONNX Runtime at all.
//!
//! The one test that encodes in fp32 is `#[ignore]`d and runs with:
//!
//! ```text
//! ORT_DYLIB_PATH=$PWD/tools/.venv/lib/python3.13/site-packages/onnxruntime/capi/libonnxruntime.1.31.0.dylib \
//!   cargo test -p crible --test lot_en_ligne -- --ignored --nocapture
//! ```

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use arrow::array::{FixedSizeBinaryArray, StringArray, UInt32Array};
use arrow::datatypes::{DataType, Field, Schema as ArrowSchema, SchemaRef};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};

use common::{DIMS as COMMON_DIMS, fake_vector, fixture_directory, vocabulary};

/// Dimensions of the fake encoder (`model.json` of the fixture).
const DIMS: u32 = COMMON_DIMS as u32;

/// Index of the online lot.
const ONLINE_INDEX: &str = "en-ligne";

/// Index that receives the equivalent batch written by a test.
const REFERENCE_INDEX: &str = "reference";

/// Epoch created by `crible index create`.
const EPOCH: u32 = 1;

/// Name of the model directory under the models root.
const MODEL: &str = "fake-model";

/// Name of the `external` model directory: no encoder, no tokenizer.
const EXTERNAL_MODEL: &str = "external";

// --- Fixture ---

/// A data root, a models root holding the fake model, and their temporary
/// directory.
struct Fixture {
    temp: tempfile::TempDir,
    data: PathBuf,
    models: PathBuf,
}

impl Fixture {
    fn root(&self) -> &Path {
        self.temp.path()
    }
}

/// Copies the fake model under the models root and writes the `external` one.
fn prepare() -> Fixture {
    let temp = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
    let data = temp.path().join("data");
    let models = temp.path().join("models");
    let model = models.join(MODEL);
    fs::create_dir_all(&model)
        .unwrap_or_else(|error| panic!("the model directory must be created: {error}"));
    for name in [
        "model.onnx",
        "model.int8.onnx",
        "tokenizer.json",
        "model.json",
    ] {
        let source = fixture_directory().join(name);
        assert!(
            source.is_file(),
            "{} is missing; the fake model fixture is versioned",
            source.display()
        );
        fs::copy(&source, model.join(name))
            .unwrap_or_else(|error| panic!("{name} must be copied: {error}"));
    }
    let external = models.join(EXTERNAL_MODEL);
    fs::create_dir_all(&external)
        .unwrap_or_else(|error| panic!("the external model directory must be created: {error}"));
    fs::write(
        external.join("model.json"),
        format!(
            r#"{{"name":"{EXTERNAL_MODEL}","kind":"external","dims":{DIMS},"normalize":true}}"#
        ),
    )
    .unwrap_or_else(|error| panic!("the external model configuration must be written: {error}"));
    Fixture { temp, data, models }
}

/// `crible index create` on an empty schema, at the reference precision `fp16`.
fn create_index(fixture: &Fixture, name: &str, model: &str) -> PathBuf {
    let config = fixture.root().join(format!("{name}.json"));
    let raw = format!(
        r#"{{
  "name": "{name}",
  "model": "{model}",
  "reference_precision": "fp16",
  "schema": {{ "document_fields": {{}}, "passage_fields": {{}} }},
  "cluster_by": [],
  "lexical": {{ "enabled": false, "language": "fr", "positions": false }},
  "search": {{
    "scan": "none",
    "rescore_candidates": 10,
    "exact_threshold": 10,
    "candidates_per_branch": 10,
    "fusion": {{ "method": "rrf" }}
  }},
  "ingest": {{ "acceptance_sample": 1, "acceptance_min_cosine": 0.9 }},
  "compaction": {{ "dead_ratio": 0.1 }}
}}"#
    );
    fs::write(&config, raw)
        .unwrap_or_else(|error| panic!("the index configuration must be written: {error}"));
    let output = crible(&[
        "index",
        "create",
        "--data",
        &fixture.data.display().to_string(),
        "--config",
        &config.display().to_string(),
    ]);
    assert!(
        output.status.success(),
        "crible index create must succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    fixture.data.join("indexes").join(name)
}

/// Runs the binary of the tests.
fn crible(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_crible"))
        .args(arguments)
        .output()
        .unwrap_or_else(|error| panic!("cannot run crible {arguments:?}: {error}"))
}

/// Runs `crible ingest` on `documents` and returns the output.
fn ingest(fixture: &Fixture, index: &str, documents: &Path) -> Output {
    crible(&[
        "ingest",
        "--data",
        &fixture.data.display().to_string(),
        "--models",
        &fixture.models.display().to_string(),
        "--index",
        index,
        &documents.display().to_string(),
    ])
}

/// Runs `crible search --vector-file` on `index` and returns the response.
fn search_vector(fixture: &Fixture, index: &str, vector: &Path, k: usize) -> Value {
    let output = crible(&[
        "search",
        "--data",
        &fixture.data.display().to_string(),
        "--index",
        index,
        "--vector-file",
        &vector.display().to_string(),
        "--k",
        &k.to_string(),
    ]);
    assert!(
        output.status.success(),
        "crible search must succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("the search response must be JSON: {error}"))
}

/// Runs `crible search --query` on `index` and returns the response.
fn search_query(fixture: &Fixture, index: &str, query: &str, k: usize) -> Value {
    let output = crible(&[
        "search",
        "--data",
        &fixture.data.display().to_string(),
        "--models",
        &fixture.models.display().to_string(),
        "--index",
        index,
        "--query",
        query,
        "--k",
        &k.to_string(),
    ]);
    assert!(
        output.status.success(),
        "crible search --query must succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("the search response must be JSON: {error}"))
}

// --- Input files ---

/// Writes one JSON object per line and returns the path.
fn write_lines(directory: &Path, name: &str, lines: &[Value]) -> PathBuf {
    let mut raw = String::new();
    for line in lines {
        raw.push_str(
            &serde_json::to_string(line)
                .unwrap_or_else(|error| panic!("a document must serialize: {error}")),
        );
        raw.push('\n');
    }
    let path = directory.join(name);
    fs::write(&path, raw)
        .unwrap_or_else(|error| panic!("the input file must be writable: {error}"));
    path
}

/// `count` repetitions of `word`, separated by one space.
fn repetition(count: usize, word: &str) -> String {
    vec![word; count].join(" ")
}

/// Base64 of the little-endian fp32 components of `vector`.
fn base64_vector(vector: &[f32]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for component in vector {
        bytes.extend_from_slice(&component.to_le_bytes());
    }
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let first = group[0];
        let second = group.get(1).copied().unwrap_or(0);
        let third = group.get(2).copied().unwrap_or(0);
        encoded.push(char::from(ALPHABET[usize::from(first >> 2)]));
        encoded.push(char::from(
            ALPHABET[usize::from(((first & 0b11) << 4) | (second >> 4))],
        ));
        if group.len() > 1 {
            encoded.push(char::from(
                ALPHABET[usize::from(((second & 0b1111) << 2) | (third >> 6))],
            ));
        } else {
            encoded.push('=');
        }
        if group.len() > 2 {
            encoded.push(char::from(ALPHABET[usize::from(third & 0b11_1111)]));
        } else {
            encoded.push('=');
        }
    }
    encoded
}

/// JSON file holding a query vector, for `crible search --vector-file`.
fn write_vector(directory: &Path, name: &str, vector: &[f32]) -> PathBuf {
    let path = directory.join(name);
    let raw = serde_json::to_string(vector)
        .unwrap_or_else(|error| panic!("the vector must serialize: {error}"));
    fs::write(&path, raw)
        .unwrap_or_else(|error| panic!("the vector file must be writable: {error}"));
    path
}

// --- Store state, read from outside the binary ---

/// Path of `records.sqlite` of one index.
fn records_path(fixture: &Fixture, index: &str) -> PathBuf {
    fixture
        .data
        .join("indexes")
        .join(index)
        .join("epochs")
        .join(EPOCH.to_string())
        .join("records.sqlite")
}

/// Reads one integer of the epoch database.
fn scalar(fixture: &Fixture, index: &str, sql: &str) -> i64 {
    let connection = Connection::open_with_flags(
        records_path(fixture, index),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap_or_else(|error| panic!("the records database must open: {error}"));
    connection
        .query_row(sql, [], |row| row.get(0))
        .unwrap_or_else(|error| panic!("the query must return one row: {error}"))
}

/// Names the `online-*` entries of `<data>/tmp`.
fn online_entries(data: &Path) -> Vec<String> {
    let root = data.join("tmp");
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

// --- Equivalent batch of the reference test ---

/// Width in bytes of one stored vector of the fixture (`fp16`).
fn vector_width() -> i32 {
    i32::try_from(DIMS * 2).unwrap_or_else(|_| panic!("the vector width must fit an i32"))
}

/// Arrow schema of the `documents` file: the reserved columns only, since the
/// schema of these tests declares no field.
fn document_schema() -> SchemaRef {
    Arc::new(ArrowSchema::new(vec![
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("doc_hash", DataType::Utf8, false),
    ]))
}

/// Arrow schema of the `passages` file.
fn passage_schema() -> SchemaRef {
    Arc::new(ArrowSchema::new(vec![
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("ord", DataType::UInt32, false),
        Field::new("text", DataType::Utf8, false),
        Field::new("vector", DataType::FixedSizeBinary(vector_width()), false),
    ]))
}

/// Writes a batch of `documents` for the `external` model of the reference
/// index, with the vectors of the fake encoder in fp16, and returns its
/// directory.
///
/// This is the lot a producer writes: same columns, same manifest members as
/// `BatchWriter` of `clients/python`, and the vectors of the online lot here
/// are the very same fp16 bytes.
fn write_reference_batch(root: &Path, texts: &[&str]) -> PathBuf {
    let directory = root.join("reference-batch");
    fs::create_dir_all(&directory)
        .unwrap_or_else(|error| panic!("the batch directory must be created: {error}"));
    let ids: Vec<String> = (0..texts.len()).map(|index| format!("D{index}")).collect();
    let hashes: Vec<String> = (0..texts.len())
        .map(|index| format!("blake3:{index:064x}"))
        .collect();
    let id_refs: StringArray = ids.iter().map(|id| Some(id.as_str())).collect();
    let hash_refs: StringArray = hashes.iter().map(|hash| Some(hash.as_str())).collect();
    common::write_parquet(
        &directory.join("documents.parquet"),
        &document_schema(),
        vec![Arc::new(id_refs), Arc::new(hash_refs)],
    );

    let vocabulary = vocabulary();
    let passage_ids: StringArray = ids.iter().map(|id| Some(id.as_str())).collect();
    let ords = UInt32Array::from(vec![0_u32; texts.len()]);
    let texts_column: StringArray = texts.iter().map(|text| Some(*text)).collect();
    let vectors = FixedSizeBinaryArray::try_from_sparse_iter_with_size(
        texts
            .iter()
            .map(|text| Some(stored_vector(&vocabulary, text)))
            .collect::<Vec<_>>()
            .into_iter(),
        vector_width(),
    )
    .unwrap_or_else(|error| panic!("the reference vectors must share one width: {error}"));
    common::write_parquet(
        &directory.join("passages.parquet"),
        &passage_schema(),
        vec![
            Arc::new(passage_ids),
            Arc::new(ords),
            Arc::new(texts_column),
            Arc::new(vectors),
        ],
    );

    let mut files = serde_json::Map::new();
    for name in ["documents.parquet", "passages.parquet"] {
        let path = directory.join(name);
        let bytes = fs::read(&path)
            .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
        files.insert(
            name.to_owned(),
            Value::String(format!("blake3:{}", blake3::hash(&bytes).to_hex())),
        );
    }
    let manifest = json!({
        "format": "crible-batch",
        "format_version": 1,
        "batch_id": "reference-batch",
        "index": REFERENCE_INDEX,
        "mode": "upsert",
        "created_at": "2026-01-01T00:00:00Z",
        "producer": { "name": "lot-en-ligne-test", "version": "1", "host": "test" },
        "model": {
            "dims": DIMS,
            "normalize": true,
            "vector_dtype": "fp16",
        },
        "chunker": { "name": "test", "version": "1", "params": {} },
        "template": { "name": "test", "version": "1" },
        "counts": {
            "documents": texts.len(),
            "passages": texts.len(),
            "deletions": 0,
        },
        "files": files,
    });
    fs::write(
        directory.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)
            .unwrap_or_else(|error| panic!("the manifest must be serializable: {error}")),
    )
    .unwrap_or_else(|error| panic!("manifest.json must be written: {error}"));
    directory
}

/// Vector the fake encoder produces for the document side of `text`, in
/// little-endian fp16.
fn stored_vector(vocabulary: &std::collections::BTreeMap<String, u32>, text: &str) -> Vec<u8> {
    fake_vector(vocabulary, text, true)
        .into_iter()
        .flat_map(|component| half::f16::from_f32(component).to_le_bytes())
        .collect()
}

// --- Tests that run in CI ---

#[test]
fn ingest_refuses_a_document_over_max_tokens_before_encoding() {
    let fixture = prepare();
    create_index(&fixture, ONLINE_INDEX, MODEL);
    // 32 words of text are 33 tokens with the `document: ` prompt, above
    // `max_tokens: 32`. ONNX Runtime is not configured here: a refusal after
    // the graph load would fail with another error code.
    let documents = write_lines(
        fixture.root(),
        "documents.jsonl",
        &[
            json!({"doc_id": "court", "passages": [{"text": "droit contrat"}]}),
            json!({"doc_id": "long", "passages": [{"text": repetition(32, "droit")}]}),
        ],
    );

    let output = ingest(&fixture, ONLINE_INDEX, &documents);
    assert!(!output.status.success(), "an over-long document is refused");
    let stderr = String::from_utf8_lossy(&output.stderr);
    // The journal writes warnings on stderr too (JSON lines since NETTOYER-P2),
    // so the error is not necessarily the first line: look for its code.
    assert!(
        stderr.contains("invalid_request:"),
        "the refusal is an invalid_request, got: {stderr}"
    );
    assert!(
        stderr.contains("long#0"),
        "the message names the offending passage, got: {stderr}"
    );
    assert!(
        stderr.contains("max_tokens = 32"),
        "the message names the limit, got: {stderr}"
    );
    assert_eq!(
        scalar(&fixture, ONLINE_INDEX, "SELECT generation FROM meta"),
        0,
        "a refused file writes no generation"
    );
    assert_eq!(
        scalar(&fixture, ONLINE_INDEX, "SELECT COUNT(*) FROM batches"),
        0
    );
    assert_eq!(
        online_entries(&fixture.data),
        Vec::<String>::new(),
        "no temporary batch is left behind"
    );
}

#[test]
fn ingest_refuses_a_document_that_is_not_a_document_without_leaking_its_values() {
    let fixture = prepare();
    create_index(&fixture, ONLINE_INDEX, MODEL);
    let mut raw = String::new();
    raw.push_str(r#"{"doc_id":"a","passages":[{"text":"droit"}]}"#);
    raw.push('\n');
    // A value of the wrong shape: `serde_json` would echo `SECRET-VALUE` in its
    // own message, which must never reach the log.
    raw.push_str(r#"{"doc_id":"b","passages":"SECRET-VALUE"}"#);
    raw.push('\n');
    let documents = fixture.root().join("documents.jsonl");
    fs::write(&documents, raw)
        .unwrap_or_else(|error| panic!("the input file must be writable: {error}"));

    let output = ingest(&fixture, ONLINE_INDEX, &documents);
    assert!(!output.status.success(), "a malformed line is refused");
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Same as above: the journal may have written a JSON line first.
    assert!(
        stderr.contains("invalid_request:"),
        "the refusal is an invalid_request, got: {stderr}"
    );
    assert!(
        stderr.contains("line 2"),
        "the message names the line, got: {stderr}"
    );
    assert!(
        !stderr.contains("SECRET-VALUE"),
        "the message must not echo a value, got: {stderr}"
    );
    assert_eq!(
        scalar(&fixture, ONLINE_INDEX, "SELECT generation FROM meta"),
        0
    );
    assert_eq!(online_entries(&fixture.data), Vec::<String>::new());
}

#[test]
fn only_a_write_opening_removes_the_stale_online_batches() {
    let fixture = prepare();
    create_index(&fixture, ONLINE_INDEX, EXTERNAL_MODEL);
    let root = fixture.data.join("tmp");
    let stale = root.join("online-deadbeef");
    fs::create_dir_all(&stale).expect("the stale directory must be creatable");
    fs::write(stale.join("manifest.json"), "{}").expect("the stale file must be writable");

    let inspected = crible(&[
        "inspect",
        "--data",
        &fixture.data.display().to_string(),
        "--index",
        ONLINE_INDEX,
    ]);
    assert!(
        inspected.status.success(),
        "crible inspect must succeed, stderr: {}",
        String::from_utf8_lossy(&inspected.stderr)
    );
    let vocabulary = vocabulary();
    let query = write_vector(
        fixture.root(),
        "query.json",
        &fake_vector(&vocabulary, "droit contrat", false),
    );
    let _response = search_vector(&fixture, ONLINE_INDEX, &query, 5);
    assert!(
        stale.exists(),
        "a read command never deletes a lot of <data>/tmp (ADR-0056)"
    );

    let documents = write_lines(
        fixture.root(),
        "documents.jsonl",
        &[json!({
            "doc_id": "D1",
            "passages": [{
                "text": "droit contrat",
                "vector": base64_vector(&fake_vector(&vocabulary, "droit contrat", true)),
            }],
        })],
    );
    let ingested = ingest(&fixture, ONLINE_INDEX, &documents);
    assert!(
        ingested.status.success(),
        "crible ingest must succeed, stderr: {}",
        String::from_utf8_lossy(&ingested.stderr)
    );
    assert!(
        !stale.exists(),
        "a write opening removes the leftover online batches of <data>/tmp"
    );
    assert!(
        !root.join("online-deadbeef.lock").exists(),
        "the presence lock goes with the directory"
    );
    assert_eq!(online_entries(&fixture.data), Vec::<String>::new());
}

#[cfg(unix)]
#[test]
fn a_failed_presence_lock_writes_a_json_line_on_stderr() {
    // `docs/07` lists `tracing-subscriber` "(JSON)": the binary installs a JSON
    // journal on stderr. The warning of `remove_stale_batches` is the only one
    // the CLI can reach today, and a lock path that cannot be opened makes
    // `lock_presence` fail (ADR-0056). A Unix socket is such a path, and the
    // lot directory is absent, so the entry takes the orphan-lock branch.
    let fixture = prepare();
    create_index(&fixture, ONLINE_INDEX, EXTERNAL_MODEL);
    let root = fixture.data.join("tmp");
    fs::create_dir_all(&root).expect("the temporary directory must be creatable");
    let _socket = std::os::unix::net::UnixListener::bind(root.join("online-deadbeef.lock"))
        .expect("the lock path must be creatable as a socket");

    let vocabulary = vocabulary();
    let documents = write_lines(
        fixture.root(),
        "documents.jsonl",
        &[json!({
            "doc_id": "D1",
            "passages": [{
                "text": "droit contrat",
                "vector": base64_vector(&fake_vector(&vocabulary, "droit contrat", true)),
            }],
        })],
    );
    let ingested = ingest(&fixture, ONLINE_INDEX, &documents);
    assert!(
        ingested.status.success(),
        "crible ingest must succeed despite the failed lock, stderr: {}",
        String::from_utf8_lossy(&ingested.stderr)
    );

    let stderr = String::from_utf8_lossy(&ingested.stderr);
    let warning = stderr
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .next()
        .unwrap_or_else(|| panic!("the journal must write a JSON line, got: {stderr}"));
    assert_eq!(warning["level"], "WARN", "the line is a warning: {warning}");
    assert_eq!(
        warning["fields"]["message"],
        "cannot take the presence lock of an online batch; the lot is left in place"
    );
    assert!(
        warning["fields"]["path"]
            .as_str()
            .is_some_and(|path| path.ends_with("online-deadbeef.lock")),
        "the line names the lock file: {warning}"
    );
}

#[test]
fn the_online_lot_matches_the_equivalent_batch_with_the_same_vectors() {
    let fixture = prepare();
    create_index(&fixture, ONLINE_INDEX, EXTERNAL_MODEL);
    create_index(&fixture, REFERENCE_INDEX, EXTERNAL_MODEL);
    let texts = [
        "droit contrat",
        "personne responsabilite",
        "code article la",
        "du des fait",
        "choses tribunal juge",
        "droit droit contrat",
    ];
    let vocabulary = vocabulary();
    let documents: Vec<Value> = texts
        .iter()
        .enumerate()
        .map(|(index, text)| {
            json!({
                "doc_id": format!("D{index}"),
                "doc_hash": format!("blake3:{index:064x}"),
                "passages": [{
                    "text": text,
                    "vector": base64_vector(&fake_vector(&vocabulary, text, true)),
                }],
            })
        })
        .collect();
    let input = write_lines(fixture.root(), "documents.jsonl", &documents);

    let online = ingest(&fixture, ONLINE_INDEX, &input);
    assert!(
        online.status.success(),
        "the online lot must attach, stderr: {}",
        String::from_utf8_lossy(&online.stderr)
    );
    let batch = write_reference_batch(fixture.root(), &texts);
    let attached = crible(&[
        "attach",
        "--data",
        &fixture.data.display().to_string(),
        "--models",
        &fixture.models.display().to_string(),
        "--index",
        REFERENCE_INDEX,
        &batch.display().to_string(),
    ]);
    assert!(
        attached.status.success(),
        "the equivalent batch must attach, stderr: {}",
        String::from_utf8_lossy(&attached.stderr)
    );

    let query = write_vector(
        fixture.root(),
        "query.json",
        &fake_vector(&vocabulary, "droit contrat code", false),
    );
    let mut online = search_vector(&fixture, ONLINE_INDEX, &query, 10);
    let mut reference = search_vector(&fixture, REFERENCE_INDEX, &query, 10);
    // The only member that legitimately differs is the wall-clock duration.
    online
        .as_object_mut()
        .expect("the response is an object")
        .remove("took_ms");
    reference
        .as_object_mut()
        .expect("the response is an object")
        .remove("took_ms");
    assert_eq!(
        online, reference,
        "an online lot and a batch carrying the same vectors return the same response"
    );
    assert_eq!(
        online["results"].as_array().map(Vec::len),
        Some(texts.len()),
        "every document is retrieved"
    );
}

// --- Test that needs ONNX Runtime ---

#[test]
#[ignore = "needs ONNX Runtime: ORT_DYLIB_PATH=$PWD/tools/.venv/lib/python3.13/site-packages/onnxruntime/capi/libonnxruntime.1.31.0.dylib cargo test -p crible --test lot_en_ligne -- --ignored"]
fn one_hundred_documents_ingested_online_are_found_by_a_text_query() {
    let fixture = prepare();
    create_index(&fixture, ONLINE_INDEX, MODEL);
    let words = [
        "droit",
        "contrat",
        "personne",
        "responsabilite",
        "code",
        "article",
        "la",
        "du",
        "des",
        "fait",
        "choses",
        "tribunal",
    ];
    let texts: Vec<String> = (0..100)
        .map(|index| {
            let word = words[index % words.len()];
            repetition(index % 8 + 1, word)
        })
        .collect();
    let documents: Vec<Value> = texts
        .iter()
        .enumerate()
        .map(
            |(index, text)| json!({"doc_id": format!("doc-{index}"), "passages": [{"text": text}]}),
        )
        .collect();
    let input = write_lines(fixture.root(), "documents.jsonl", &documents);

    let output = ingest(&fixture, ONLINE_INDEX, &input);
    assert!(
        output.status.success(),
        "the online lot must attach, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        scalar(&fixture, ONLINE_INDEX, "SELECT COUNT(*) FROM passages"),
        100
    );

    // Three distinct queries each return the whole pool: every document
    // ingested online is retrievable. The fixture encoder compresses every
    // vector close to `[1, 0, …, 0]`, so its ranking does not separate
    // documents by meaning; the point of this test is that the passages were
    // encoded, stored at the reference precision, indexed and returned.
    let mut expected: Vec<String> = (0..100).map(|index| format!("doc-{index}")).collect();
    expected.sort();
    for query in ["droit", "code article", "tribunal juge des"] {
        let response = search_query(&fixture, ONLINE_INDEX, query, 1000);
        assert_eq!(response["pool_size"], 100, "the pool holds every passage");
        let mut found: Vec<String> = response["results"]
            .as_array()
            .expect("the results are an array")
            .iter()
            .map(|hit| {
                hit["doc_id"]
                    .as_str()
                    .expect("a result carries a doc_id")
                    .to_owned()
            })
            .collect();
        found.sort();
        assert_eq!(
            found, expected,
            "the query '{query}' must retrieve the 100 documents ingested online"
        );
    }
}
