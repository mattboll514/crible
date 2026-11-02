// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `EXPORTER-LOT` (phase P3): `crible export`.
//!
//! References: `docs/phases/P3-cycle-de-vie.md` (task `EXPORTER-LOT`),
//! `docs/04-donnees-et-ingestion.md` ("Export, instantanés, restauration"),
//! `docs/format-lot-v1.md`, `docs/07-stack-et-deploiement.md` (`[paths]
//! exchange`), `docs/08-invariants-et-tests.md` (INV-01).
//!
//! The tests drive the command the task ships, so they pin the CLI contract and
//! not only the store API: the `--exchange` root, the report on stdout, the exit
//! code and the wire name of the error. The index is built by the test with the
//! store API, at generation 1, one segment of four dimensions, two documents and
//! three passages.
//!
//! | Test | Criterion / decision |
//! |---|---|
//! | `export_writes_a_replace_all_batch_the_reader_accepts` | the batch of `LIRE-LOT-PARQUET`, under `exchange/outbox/<index>-g<G>/` |
//! | `a_second_export_of_the_same_generation_is_refused` | an existing export is never overwritten silently |

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crible_core::batch::BatchMode;
use crible_core::config::{ReferencePrecision, Schema};
use crible_core::ids::{Epoch, Generation, Iid};
use crible_core::model::ModelConfig;
use crible_store::batch_reader::BatchReader;
use crible_store::generation::{GenerationChange, GenerationStore};
use crible_store::records::{DocumentRecord, PassageRecord};
use crible_store::segment::{SegmentSpec, SegmentWriter};
use serde_json::{Value, json};

/// Name of the index, and prefix of its export directory.
const INDEX: &str = "export-cli";

/// Dimensions of the fixture vectors.
const DIMS: u32 = 4;

/// Schema of the index: one document field, one passage field.
const SCHEMA: &str = r#"{
    "document_fields": { "juridiction": { "type": "keyword" } },
    "passage_fields": { "partie": { "type": "keyword" } }
}"#;

/// One temporary instance: the data root, the models root, the exchange root
/// and the index.
struct Instance {
    /// Keeps the temporary directory alive for the whole test.
    _temp: tempfile::TempDir,
    /// Root of the data directory, passed as `--data`.
    data: PathBuf,
    /// Root of the models directory, passed as `--models`.
    models: PathBuf,
    /// Root of the exchange directory, passed as `--exchange`.
    exchange: PathBuf,
}

impl Instance {
    /// Runs `crible index create`, then commits generation 1 with two documents
    /// and three passages.
    fn new() -> Self {
        let temp = tempfile::tempdir()
            .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
        let data = temp.path().join("data");
        let models = temp.path().join("models");
        let exchange = temp.path().join("exchange");
        fs::create_dir_all(&data)
            .unwrap_or_else(|error| panic!("the data root must be creatable: {error}"));
        fs::create_dir_all(&models)
            .unwrap_or_else(|error| panic!("the models root must be creatable: {error}"));
        let schema = Schema::from_json_str(SCHEMA)
            .unwrap_or_else(|error| panic!("the fixture schema must be valid: {error}"));
        write_model(&models, "m");
        let config = temp.path().join("index.json");
        fs::write(&config, configuration(INDEX))
            .unwrap_or_else(|error| panic!("the configuration must be written: {error}"));

        let created = crible(&[
            "index",
            "create",
            "--data",
            path(&data),
            "--config",
            path(&config),
        ]);
        assert!(
            created.status.success(),
            "index create failed: {}",
            String::from_utf8_lossy(&created.stderr)
        );

        let index = data.join("indexes").join(INDEX);
        let mut store = GenerationStore::open(&index, &schema)
            .unwrap_or_else(|error| panic!("the fixture index must open: {error}"));
        let segment = write_segment(&index);
        store
            .commit(GenerationChange {
                segments: vec![segment],
                documents: vec![
                    document("D1", "judiciaire"),
                    document("D2", "administratif"),
                ],
                passages: vec![
                    passage(0, "D1", 0),
                    passage(1, "D1", 1),
                    passage(2, "D2", 0),
                ],
                deletions: Vec::new(),
            })
            .unwrap_or_else(|error| panic!("the fixture generation must commit: {error}"));

        Self {
            _temp: temp,
            data,
            models,
            exchange,
        }
    }

    /// Directory of the fixture index.
    fn index_dir(&self) -> PathBuf {
        self.data.join("indexes").join(INDEX)
    }

    /// Directory the export of generation 1 must write.
    fn expected_directory(&self) -> PathBuf {
        self.exchange.join("outbox").join(format!("{INDEX}-g1"))
    }

    /// Runs `crible export` on the fixture index.
    fn export(&self) -> Output {
        crible(&[
            "export",
            "--data",
            path(&self.data),
            "--models",
            path(&self.models),
            "--exchange",
            path(&self.exchange),
            "--index",
            INDEX,
        ])
    }
}

/// Configuration of the index, with the settings the store requires.
fn configuration(name: &str) -> String {
    format!(
        r#"{{"name":"{name}","model":"m","reference_precision":"fp16","schema":{SCHEMA},"cluster_by":[],
            "lexical":{{"enabled":false,"language":"fr","positions":false}},
            "search":{{"scan":"none","rescore_candidates":10,"exact_threshold":10,
                      "candidates_per_branch":10,"fusion":{{"method":"rrf"}}}},
            "ingest":{{"acceptance_sample":1,"acceptance_min_cosine":0.9}},
            "compaction":{{"dead_ratio":0.1}}}}"#
    )
}

/// Writes the `model.json` of the fixture model: no encoder, four dimensions,
/// normalized vectors.
fn write_model(models: &Path, name: &str) {
    let directory = models.join(name);
    fs::create_dir_all(&directory)
        .unwrap_or_else(|error| panic!("the model directory must be creatable: {error}"));
    fs::write(
        directory.join("model.json"),
        r#"{"name":"m","kind":"external","dims":4,"normalize":true}"#,
    )
    .unwrap_or_else(|error| panic!("the model configuration must be written: {error}"));
}

/// Reads the model configuration the export declares.
fn model_config() -> ModelConfig {
    ModelConfig::from_json_str(r#"{"name":"m","kind":"external","dims":4,"normalize":true}"#)
        .unwrap_or_else(|error| panic!("the fixture model must be valid: {error}"))
}

/// Builds one `documents` row of generation 1.
fn document(doc_id: &str, juridiction: &str) -> DocumentRecord {
    DocumentRecord {
        doc_id: doc_id.to_owned(),
        doc_hash: format!("blake3:doc-{doc_id}"),
        fields: json!({ "juridiction": juridiction }),
        updated_generation: Generation::new(1),
    }
}

/// Builds one `passages` row of generation 1.
fn passage(iid: u32, doc_id: &str, ord: u32) -> PassageRecord {
    PassageRecord {
        iid: Iid::new(iid),
        doc_id: doc_id.to_owned(),
        ord,
        text: format!("{doc_id}#{ord}"),
        passage_hash: format!("blake3:passage-{doc_id}-{ord}"),
        fields: json!({ "partie": "motifs" }),
        segment_id: "seg-1".to_owned(),
        created_generation: Generation::new(1),
        dead_since_gen: None,
    }
}

/// Writes the only segment of the fixture and returns its metadata.
fn write_segment(index: &Path) -> crible_store::segment::SegmentMeta {
    let segments = index
        .join("epochs")
        .join(Epoch::new(1).get().to_string())
        .join("segments");
    fs::create_dir_all(&segments)
        .unwrap_or_else(|error| panic!("the segments directory must be creatable: {error}"));
    let mut writer = SegmentWriter::create(
        &segments,
        SegmentSpec {
            segment_id: "seg-1".to_owned(),
            iid_start: Iid::new(0),
            dims: DIMS,
            generation: Generation::new(1),
            batch_id: "batch-cli".to_owned(),
            precision: ReferencePrecision::Fp16,
        },
    )
    .unwrap_or_else(|error| panic!("the segment writer must start: {error}"));
    for vector in [
        &[1.0_f32, 0.0, 0.0, 0.0][..],
        &[0.0, 1.0, 0.0, 0.0],
        &[0.0, 0.0, 1.0, 0.0],
    ] {
        writer
            .append(vector)
            .unwrap_or_else(|error| panic!("the fixture vector must be appended: {error}"));
    }
    writer
        .finish()
        .unwrap_or_else(|error| panic!("the fixture segment must be finished: {error}"))
}

/// Runs the binary with absolute paths, as the other CLI tests do.
fn crible(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_crible"))
        .args(arguments)
        .output()
        .unwrap_or_else(|error| panic!("cannot run crible {arguments:?}: {error}"))
}

/// Path as a `&str`, the tests building it from a temporary directory.
fn path(path: &Path) -> &str {
    path.to_str()
        .unwrap_or_else(|| panic!("{} must be UTF-8", path.display()))
}

#[test]
fn export_writes_a_replace_all_batch_the_reader_accepts() {
    // `docs/04`: the export is a `replace_all` batch under
    // `exchange/outbox/<index>-g<G>/`, and it passes the validation of
    // `LIRE-LOT-PARQUET`.
    let instance = Instance::new();
    let output = instance.export();
    assert!(
        output.status.success(),
        "export failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let directory = instance.expected_directory();
    assert!(
        stdout.contains(&directory.display().to_string()),
        "the report must name the batch directory '{}', got {stdout:?}",
        directory.display()
    );
    assert!(
        stdout.contains("generation 1") && stdout.contains("3 passage(s)"),
        "the report must carry the generation and the counts, got {stdout:?}"
    );

    let index_config = crible_core::config::IndexConfig::from_json_str(&configuration(INDEX))
        .unwrap_or_else(|error| panic!("the fixture configuration must be valid: {error}"));
    let reader = BatchReader::open(&directory, &index_config, &model_config())
        .unwrap_or_else(|error| panic!("the exported batch must open: {error}"));
    assert_eq!(
        reader.manifest().mode,
        BatchMode::ReplaceAll,
        "the export is the complete content of the index"
    );
    let counts = reader
        .validate()
        .unwrap_or_else(|error| panic!("the exported batch must validate: {error}"));
    assert_eq!(
        (counts.documents, counts.passages, counts.deletions),
        (2, 3, 0),
        "the export carries every live row and no deletion"
    );

    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(directory.join("manifest.json"))
            .unwrap_or_else(|error| panic!("the manifest must be readable: {error}")),
    )
    .unwrap_or_else(|error| panic!("the manifest must be valid JSON: {error}"));
    assert_eq!(
        manifest.get("batch_id").and_then(Value::as_str),
        Some(format!("{INDEX}-g1").as_str()),
        "the batch_id is also the name of its directory: {manifest}"
    );
    assert!(
        !directory.join("deletions.parquet").exists(),
        "a replace_all batch holds no deletion file"
    );
}

#[test]
fn a_second_export_of_the_same_generation_is_refused() {
    // An export never overwrites the copy of the same generation in silence: a
    // second run fails on the wire code `invalid_request` and the first batch
    // stays in place.
    let instance = Instance::new();
    let first = instance.export();
    assert!(
        first.status.success(),
        "the first export failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let directory = instance.expected_directory();
    let manifest_before = fs::read(directory.join("manifest.json"))
        .unwrap_or_else(|error| panic!("the manifest must be readable: {error}"));

    let second = instance.export();
    assert!(
        !second.status.success(),
        "a second export of the same generation must fail"
    );
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        stderr.starts_with("invalid_request: "),
        "the refusal must carry the wire name of an error code, got {stderr:?}"
    );
    let manifest_after = fs::read(directory.join("manifest.json"))
        .unwrap_or_else(|error| panic!("the manifest must still be readable: {error}"));
    assert_eq!(
        manifest_before, manifest_after,
        "the refused export must not touch the batch already in place"
    );
}

// --- A read-only command leaves the index untouched (ADR-0116, decision 1) ---

/// One entry of the recursive inventory of an index directory.
#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexEntry {
    /// Path relative to the root of the index, with `/` separators.
    path: String,
    /// Size in bytes: `0` for a directory.
    size: u64,
    /// `blake3:<hex>` of the bytes of a file; `<directory>` for a directory.
    digest: String,
}

/// Recursive inventory of `root`: every path, its size and its blake3, sorted
/// by path, `-wal` and `-shm` excepted (ADR-0116, decision 1).
fn index_inventory(root: &Path) -> Vec<IndexEntry> {
    let mut entries = Vec::new();
    collect_index_entries(root, root, &mut entries);
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    entries
}

/// Recursive body of [`index_inventory`].
fn collect_index_entries(root: &Path, directory: &Path, entries: &mut Vec<IndexEntry>) {
    let listing = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("{} must be listable: {error}", directory.display()));
    for entry in listing {
        let entry = entry.unwrap_or_else(|error| {
            panic!(
                "an entry of {} must be readable: {error}",
                directory.display()
            )
        });
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with("-wal") || name.ends_with("-shm") {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or_else(|error| {
                panic!(
                    "{} must be under {}: {error}",
                    path.display(),
                    root.display()
                )
            })
            .to_string_lossy()
            .replace('\\', "/");
        let metadata = fs::symlink_metadata(&path)
            .unwrap_or_else(|error| panic!("{} must be inspectable: {error}", path.display()));
        if metadata.is_dir() {
            entries.push(IndexEntry {
                path: relative,
                size: 0,
                digest: "<directory>".to_owned(),
            });
            collect_index_entries(root, &path, entries);
            continue;
        }
        let bytes = fs::read(&path)
            .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
        entries.push(IndexEntry {
            path: relative,
            size: metadata.len(),
            digest: format!("blake3:{}", blake3::hash(&bytes).to_hex()),
        });
    }
}

#[test]
fn export_on_a_dirty_index_leaves_the_index_directory_untouched() {
    // `crible export` is a read command (ADR-0116, decision 1): on an index that
    // an interrupted creation or commit left dirty, it opens for read, writes
    // its batch under `exchange/outbox/`, and leaves the index directory
    // identical byte for byte, `-wal` and `-shm` excepted.
    let instance = Instance::new();
    let index = instance.index_dir();
    let epoch = index.join("epochs").join(Epoch::new(1).get().to_string());

    // A `tmp-*` file at the root of the index, as an interrupted creation
    // leaves one.
    fs::write(index.join("tmp-CURRENT_EPOCH"), b"1\n")
        .unwrap_or_else(|error| panic!("the tmp-* fixture file must be writable: {error}"));
    // A segment directory the `segments` table does not reference.
    let orphan = epoch.join("segments").join("seg-orphan");
    fs::create_dir_all(&orphan)
        .unwrap_or_else(|error| panic!("the orphan segment directory must be creatable: {error}"));
    fs::write(orphan.join("segment.json"), b"{}")
        .unwrap_or_else(|error| panic!("the orphan segment file must be writable: {error}"));
    // A missing manifest of the committed generation.
    let manifest = epoch
        .join("manifests")
        .join(format!("{}.json", Generation::new(1).get()));
    fs::remove_file(&manifest)
        .unwrap_or_else(|error| panic!("the manifest fixture must be removable: {error}"));

    let before = index_inventory(&index);
    let output = instance.export();
    assert!(
        output.status.success(),
        "the export of a dirty index must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let after = index_inventory(&index);

    assert!(
        index.join("tmp-CURRENT_EPOCH").exists(),
        "the export removed the tmp-* file at the root of the index"
    );
    assert!(
        orphan.exists(),
        "the export removed the segment SQLite does not reference"
    );
    assert!(
        !manifest.exists(),
        "the export rewrote the missing manifest of generation 1"
    );
    assert_eq!(
        after, before,
        "crible export must not touch the index directory (ADR-0116, decision 1): \
         the inventory of paths, sizes and blake3 must be identical, -wal and -shm \
         excepted"
    );
}
