// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `CREER-ET-RESTAURER-INSTANTANE` (phase P3): `crible snapshot` and
//! `crible restore` at the CLI boundary.
//!
//! References: the task sheet (`docs/phases/P3-cycle-de-vie.md:121-130`),
//! `docs/04-donnees-et-ingestion.md` ("Règles de l'instantané", "Règles de la
//! restauration"), `docs/05-api.md` (error codes), ADR-0064 and ADR-0065.
//!
//! The store tests (`crates/crible-store/tests/instantane.rs`) cover every
//! refusal of `snapshot.json` and the write protocol. These tests pin what only
//! the command can show: the shape of the two commands, their report, the wire
//! name of a refusal, and the task criterion "une restauration redonne la
//! génération et des résultats identiques", measured on the JSON answer of the
//! exact dense search before and after the rollback.
//!
//! | Test | Criterion / decision |
//! |---|---|
//! | `snapshot_then_restore_gives_the_same_generation_and_search_results` | task criterion 1; ADR-0065 §2, §3, §4, §6 |
//! | `a_second_snapshot_of_the_same_epoch_is_refused` | ADR-0064 §3; `invalid_request` |
//! | `restore_into_a_fresh_data_root_creates_the_index` | ADR-0065 §2, §3 |
//! | `restore_refuses_an_incomplete_snapshot` | task criterion 2; ADR-0064 §1 |

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crible_core::config::{ReferencePrecision, Schema};
use crible_core::ids::{Epoch, Generation, Iid};
use crible_store::generation::{GenerationChange, GenerationStore};
use crible_store::records::{DocumentRecord, PassageRecord};
use crible_store::segment::{SegmentSpec, SegmentWriter};
use serde_json::Value;

/// Name of the index, and prefix of its snapshot directory.
const INDEX: &str = "snapshot-cli";

/// Dimensions of the fixture vectors.
const DIMS: u32 = 4;

/// Schema of the index: one document field, one passage field.
const SCHEMA: &str = r#"{
    "document_fields": { "juridiction": { "type": "keyword" } },
    "passage_fields": { "partie": { "type": "keyword" } }
}"#;

/// One temporary instance: the data root, the exchange root, the index, and a
/// vector file both searches use.
struct Instance {
    /// Keeps the temporary directory alive for the whole test.
    _temp: tempfile::TempDir,
    /// Root of the data directory, passed as `--data`.
    data: PathBuf,
    /// Root of the exchange directory, passed as `--exchange`.
    exchange: PathBuf,
    /// Query vector, identical for the searches compared.
    vector: PathBuf,
}

impl Instance {
    /// Runs `crible index create`, then commits generation 1 with two documents
    /// and three passages.
    fn new() -> Self {
        let temp = tempfile::tempdir()
            .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
        let data = temp.path().join("data");
        let exchange = temp.path().join("exchange");
        fs::create_dir_all(&data)
            .unwrap_or_else(|error| panic!("the data root must be creatable: {error}"));
        let schema = schema();
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

        let vector = temp.path().join("vector.json");
        fs::write(&vector, "[1.0, 0.0, 0.0, 0.0]")
            .unwrap_or_else(|error| panic!("the query vector must be written: {error}"));

        Self {
            _temp: temp,
            data,
            exchange,
            vector,
        }
    }

    /// Index directory of the fixture.
    fn index(&self) -> PathBuf {
        self.data.join("indexes").join(INDEX)
    }

    /// Directory `crible snapshot` must write for the first generation.
    fn snapshot_directory(&self) -> PathBuf {
        self.exchange
            .join("snapshots")
            .join(format!("{INDEX}-e1-g1"))
    }

    /// Runs `crible snapshot` on the fixture index.
    fn snapshot(&self) -> Output {
        crible(&[
            "snapshot",
            "--data",
            path(&self.data),
            "--exchange",
            path(&self.exchange),
            "--index",
            INDEX,
        ])
    }

    /// Runs `crible restore` into this instance.
    fn restore(&self, directory: &Path) -> Output {
        self.restore_into(&self.data, directory)
    }

    /// Runs `crible restore` into another data root.
    fn restore_into(&self, data: &Path, directory: &Path) -> Output {
        crible(&[
            "restore",
            "--data",
            path(data),
            "--snapshot",
            path(directory),
        ])
    }

    /// Runs `crible search --vector-file` and returns the JSON response, with the
    /// elapsed time removed: it is the only field the restore cannot promise.
    fn search(&self, data: &Path) -> Value {
        let output = crible(&[
            "search",
            "--data",
            path(data),
            "--index",
            INDEX,
            "--vector-file",
            path(&self.vector),
            "--k",
            "3",
        ]);
        assert!(
            output.status.success(),
            "search failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut response: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("the search answer must be JSON: {error}"));
        if let Some(object) = response.as_object_mut() {
            object.remove("took_ms");
        }
        response
    }

    /// Runs `crible inspect` and returns its stdout.
    fn inspect(&self) -> String {
        let output = crible(&["inspect", "--data", path(&self.data), "--index", INDEX]);
        assert!(
            output.status.success(),
            "inspect failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// Commits generation 2 by tombstoning the second passage of `D1`, so the
    /// visible state really moves past the snapshot.
    fn advance(&self) {
        let mut store = GenerationStore::open(&self.index(), &schema())
            .unwrap_or_else(|error| panic!("the fixture index must reopen: {error}"));
        store
            .commit(GenerationChange {
                segments: Vec::new(),
                documents: Vec::new(),
                passages: Vec::new(),
                deletions: vec![Iid::new(2)],
            })
            .unwrap_or_else(|error| panic!("generation 2 must commit: {error}"));
        assert_eq!(store.generation(), Generation::new(2));
    }

    /// Bytes of the `CURRENT_EPOCH` pointer of the fixture index.
    fn pointer(&self) -> String {
        fs::read_to_string(self.index().join("CURRENT_EPOCH"))
            .unwrap_or_else(|error| panic!("the pointer must be readable: {error}"))
    }
}

/// Schema of the index.
fn schema() -> Schema {
    Schema::from_json_str(SCHEMA)
        .unwrap_or_else(|error| panic!("the fixture schema must be valid: {error}"))
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

/// Builds one `documents` row of generation 1.
fn document(doc_id: &str, juridiction: &str) -> DocumentRecord {
    DocumentRecord {
        doc_id: doc_id.to_owned(),
        doc_hash: format!("blake3:doc-{doc_id}"),
        fields: serde_json::json!({ "juridiction": juridiction }),
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
        fields: serde_json::json!({ "partie": "motifs" }),
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

/// Reads the attestation of a snapshot.
fn attestation(directory: &Path) -> Value {
    let text = fs::read_to_string(directory.join("snapshot.json"))
        .unwrap_or_else(|error| panic!("snapshot.json must be readable: {error}"));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("snapshot.json must be valid JSON: {error}"))
}

#[test]
fn snapshot_then_restore_gives_the_same_generation_and_search_results() {
    // The task criterion: the restore gives back the generation of the snapshot
    // and answers the same queries identically. The index first moves to
    // generation 2, so the comparison is meaningful.
    let instance = Instance::new();
    let before = instance.search(&instance.data);
    assert_eq!(before["generation"], 1);
    assert_eq!(before["pool_size"], 3);

    let output = instance.snapshot();
    assert!(
        output.status.success(),
        "snapshot failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let directory = instance.snapshot_directory();
    assert!(
        stdout.contains(&directory.display().to_string()),
        "the report names the snapshot directory '{}': {stdout:?}",
        directory.display()
    );
    for expected in [
        "epoch 1",
        "generation 1",
        "2 document(s)",
        "3 passage(s)",
        "bytes",
    ] {
        assert!(
            stdout.contains(expected),
            "the report carries {expected:?} (ADR-0065 §6): {stdout:?}"
        );
    }
    let attestation = attestation(&directory);
    assert_eq!(attestation["format_version"], 1);
    assert_eq!(attestation["index"], INDEX);
    assert_eq!(attestation["epoch"], 1);
    assert_eq!(attestation["generation"], 1);

    instance.advance();
    let after_advance = instance.search(&instance.data);
    assert_eq!(
        after_advance["generation"], 2,
        "the index moved past the snapshot before the restore"
    );
    assert_ne!(
        after_advance["results"], before["results"],
        "the visible state really changed between the snapshot and the restore"
    );

    let output = instance.restore(&directory);
    assert!(
        output.status.success(),
        "restore failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "replaced epoch 1 (generation 2)",
        "new epoch 2",
        "generation 1",
        "2 document(s)",
        "3 passage(s)",
    ] {
        assert!(
            stdout.contains(expected),
            "the report carries {expected:?} (ADR-0065 §6): {stdout:?}"
        );
    }
    assert!(
        stdout.contains(INDEX),
        "the report names the index: {stdout:?}"
    );

    let inspected = instance.inspect();
    assert!(
        inspected.contains("generation: 1") && inspected.contains("epoch: 2"),
        "the restored index publishes generation 1 on epoch 2: {inspected:?}"
    );
    assert_eq!(
        instance.search(&instance.data),
        before,
        "a restore answers the same queries with the same keys, scores and payload"
    );
    assert!(
        instance.index().join("epochs").join("1").is_dir(),
        "the replaced epoch is kept on disk, never reclaimed by the restore (ADR-0065 §4)"
    );
}

#[test]
fn a_second_snapshot_of_the_same_epoch_is_refused() {
    // ADR-0064 decision 3: the destination is refused in `invalid_request`, and
    // the first snapshot stays exactly as it was.
    let instance = Instance::new();
    let first = instance.snapshot();
    assert!(
        first.status.success(),
        "the first snapshot failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let directory = instance.snapshot_directory();
    let before = attestation(&directory);

    let second = instance.snapshot();
    assert!(
        !second.status.success(),
        "a second snapshot of the same epoch must fail"
    );
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        stderr.starts_with("invalid_request: "),
        "the refusal carries the wire name of the code, got {stderr:?}"
    );
    assert!(
        stderr.contains(&directory.display().to_string()),
        "the refusal names the occupied directory: {stderr:?}"
    );
    assert_eq!(
        attestation(&directory),
        before,
        "the refused snapshot must not touch the copy already in place"
    );
}

#[test]
fn restore_into_a_fresh_data_root_creates_the_index() {
    // ADR-0065 decision 3: the name comes from `snapshot.json`; an absent index
    // is created from the snapshot's `index.json`, at the initial epoch and with
    // the generation of the snapshot.
    let instance = Instance::new();
    let taken = instance.snapshot();
    assert!(
        taken.status.success(),
        "the snapshot failed: {}",
        String::from_utf8_lossy(&taken.stderr)
    );
    let before = instance.search(&instance.data);
    let directory = instance.snapshot_directory();

    let fresh = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("a temporary data root must be creatable: {error}"));
    let data = fresh.path().join("data");
    let output = instance.restore_into(&data, &directory);
    assert!(
        output.status.success(),
        "restore into a fresh data root failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("no epoch replaced") && stdout.contains("new epoch 1"),
        "the report says nothing was replaced and the epoch is the initial one: {stdout:?}"
    );
    assert_eq!(
        instance.search(&data),
        before,
        "the index created from the snapshot answers the same queries identically"
    );
}

#[test]
fn restore_refuses_an_incomplete_snapshot() {
    // The task criterion 2 at the CLI boundary: a snapshot missing one of its
    // files is refused in `invalid_request`, and the target is not touched.
    let instance = Instance::new();
    let taken = instance.snapshot();
    assert!(
        taken.status.success(),
        "the snapshot failed: {}",
        String::from_utf8_lossy(&taken.stderr)
    );
    let directory = instance.snapshot_directory();
    fs::remove_file(directory.join("segments/seg-1/vectors.f16"))
        .unwrap_or_else(|error| panic!("a segment file must be removable: {error}"));
    let pointer = instance.pointer();

    let restored = instance.restore(&directory);
    assert!(
        !restored.status.success(),
        "an incomplete snapshot must fail"
    );
    let stderr = String::from_utf8_lossy(&restored.stderr);
    assert!(
        stderr.starts_with("invalid_request: "),
        "the refusal carries the wire name of the code, got {stderr:?}"
    );
    assert!(
        stderr.contains("segments/seg-1/vectors.f16"),
        "the refusal names the missing file: {stderr:?}"
    );
    assert_eq!(
        instance.pointer(),
        pointer,
        "a refused restore writes no pointer"
    );
    assert!(
        !instance.index().join("epochs").join("2").exists(),
        "a refused restore writes no epoch"
    );
}
