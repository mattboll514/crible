// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `PATCHER-METADONNEES` (phase P3): `crible patch`.
//!
//! References: `docs/phases/P3-cycle-de-vie.md` (task PATCHER-METADONNEES),
//! ADR-0060, `docs/04-donnees-et-ingestion.md` ("Patch de métadonnées"),
//! `docs/05-api.md` ("Patch de métadonnées").
//!
//! The tests drive the command the task ships, so they pin the CLI contract and
//! not only the store API: the JSONL file, the report on stdout, the exit code
//! and the wire name of the error. The index is built by the test with the store
//! API, at generation 1, one segment of four dimensions and one document `D1`
//! with one passage.
//!
//! | Test | Criterion / decision |
//! |---|---|
//! | `patch_changes_what_a_search_returns` | criterion 1 end to end, ADR-0060 decisions 8 and 9 |
//! | `an_unknown_document_fails_the_whole_file` | decision 4 |
//! | `a_mis_typed_value_is_refused_without_a_report` | criterion 3 |
//! | `a_patch_above_the_entry_limit_is_refused` | decision 7, on the file |

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crible_core::config::{ReferencePrecision, Schema};
use crible_core::ids::{Epoch, Generation, Iid};
use crible_store::generation::{GenerationChange, GenerationStore};
use crible_store::records::{DocumentRecord, PassageRecord};
use crible_store::segment::{SegmentSpec, SegmentWriter};
use serde_json::{Value, json};

/// Name of the index, and of its directory under `<data>/indexes`.
const INDEX: &str = "patch-cli";

/// Dimensions of the fixture vectors.
const DIMS: u32 = 4;

/// Schema of the index: one document field, one passage field.
const SCHEMA: &str = r#"{
    "document_fields": { "juridiction": { "type": "keyword" } },
    "passage_fields": { "partie": { "type": "keyword" } }
}"#;

/// One temporary instance: the data root, the configuration and the index.
struct Instance {
    /// Keeps the temporary directory alive for the whole test.
    _temp: tempfile::TempDir,
    /// Root of the data directory, passed as `--data`.
    data: PathBuf,
    /// Directory of the index.
    index: PathBuf,
}

impl Instance {
    /// Runs `crible index create`, then commits generation 1 with one document.
    fn new() -> Self {
        let temp = tempfile::tempdir()
            .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
        let data = temp.path().join("data");
        fs::create_dir_all(&data)
            .unwrap_or_else(|error| panic!("the data root must be creatable: {error}"));
        let schema = Schema::from_json_str(SCHEMA)
            .unwrap_or_else(|error| panic!("the fixture schema must be valid: {error}"));
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
        let document = DocumentRecord {
            doc_id: "D1".to_owned(),
            doc_hash: "blake3:doc-d1".to_owned(),
            fields: json!({ "juridiction": "judiciaire" }),
            updated_generation: Generation::new(1),
        };
        let passage = PassageRecord {
            iid: Iid::new(0),
            doc_id: "D1".to_owned(),
            ord: 0,
            text: "D1#0".to_owned(),
            passage_hash: "blake3:passage-0".to_owned(),
            fields: json!({ "partie": "motifs" }),
            segment_id: "seg-1".to_owned(),
            created_generation: Generation::new(1),
            dead_since_gen: None,
        };
        store
            .commit(GenerationChange {
                segments: vec![segment],
                documents: vec![document],
                passages: vec![passage],
                deletions: Vec::new(),
            })
            .unwrap_or_else(|error| panic!("the fixture generation must commit: {error}"));

        Self {
            _temp: temp,
            data,
            index,
        }
    }

    /// Writes `lines` into a JSONL file of the instance and returns its path.
    fn jsonl(&self, name: &str, lines: &str) -> PathBuf {
        let path = self.index.join(name);
        fs::write(&path, lines)
            .unwrap_or_else(|error| panic!("{} must be writable: {error}", path.display()));
        path
    }

    /// Runs `crible patch` on `file`.
    fn patch(&self, file: &Path) -> Output {
        crible(&[
            "patch",
            "--data",
            path(&self.data),
            "--index",
            INDEX,
            path(file),
        ])
    }

    /// Searches the index with the first axis as the query vector.
    fn search(&self) -> Value {
        let query = self.index.join("query.json");
        fs::write(&query, "[1.0, 0.0, 0.0, 0.0]")
            .unwrap_or_else(|error| panic!("the query vector must be written: {error}"));
        let output = crible(&[
            "search",
            "--data",
            path(&self.data),
            "--index",
            INDEX,
            "--vector-file",
            path(&query),
        ]);
        assert!(
            output.status.success(),
            "search failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("the search response must be JSON: {error}"))
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
    writer
        .append(&[1.0, 0.0, 0.0, 0.0])
        .unwrap_or_else(|error| panic!("the fixture vector must be appended: {error}"));
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
fn patch_changes_what_a_search_returns() {
    // Criterion 1 end to end, and the report of ADR-0060 decision 8: a search
    // after the patch returns the new document field, with the new generation,
    // and the patch prints `{updated, unchanged, generation}`.
    let instance = Instance::new();
    let before = instance.search();
    assert_eq!(before["results"][0]["fields"]["juridiction"], "judiciaire");
    assert_eq!(before["generation"], 1);

    let file = instance.jsonl(
        "patch.jsonl",
        "{\"doc_id\":\"D1\",\"fields\":{\"juridiction\":\"administratif\"}}\n",
    );
    let output = instance.patch(&file);
    assert!(
        output.status.success(),
        "patch failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("the report must be JSON: {error}"));
    assert_eq!(
        report,
        json!({ "updated": 1, "unchanged": 0, "generation": 2 }),
        "the report holds the three keys of ADR-0060, and no id"
    );

    let after = instance.search();
    assert_eq!(
        after["results"][0]["fields"]["juridiction"], "administratif",
        "a search after the patch returns the new value"
    );
    assert_eq!(after["results"][0]["fields"]["partie"], "motifs");
    assert_eq!(after["generation"], 2);

    // ADR-0060, decision 9: the same file applied again changes nothing, so no
    // generation is created, and the report says so.
    let again = instance.patch(&file);
    assert!(
        again.status.success(),
        "a patch without effect must succeed: {}",
        String::from_utf8_lossy(&again.stderr)
    );
    let report: Value = serde_json::from_slice(&again.stdout)
        .unwrap_or_else(|error| panic!("the report must be JSON: {error}"));
    assert_eq!(
        report,
        json!({ "updated": 0, "unchanged": 1, "generation": 2 }),
        "a patch without effect creates no generation"
    );
    assert_eq!(instance.search()["generation"], 2);
}

#[test]
fn an_unknown_document_fails_the_whole_file() {
    // ADR-0060, decision 4: one unknown `doc_id` refuses the file, and the valid
    // entry of the same file is not applied either.
    let instance = Instance::new();
    let file = instance.jsonl(
        "patch.jsonl",
        "{\"doc_id\":\"D1\",\"fields\":{\"juridiction\":\"administratif\"}}\n\
         {\"doc_id\":\"D999\",\"fields\":{\"juridiction\":\"administratif\"}}\n",
    );
    let output = instance.patch(&file);
    assert!(!output.status.success(), "the unknown document must fail");
    assert!(
        output.stdout.is_empty(),
        "a refused patch prints no report: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not_found"), "{stderr}");

    let after = instance.search();
    assert_eq!(
        after["results"][0]["fields"]["juridiction"], "judiciaire",
        "the valid entry of the refused file must not be applied"
    );
    assert_eq!(after["generation"], 1, "no generation may be created");
}

#[test]
fn a_mis_typed_value_is_refused_without_a_report() {
    // Criterion 3 through the CLI: the fault is `invalid_request`, the report is
    // absent, and the index does not move.
    let instance = Instance::new();
    let file = instance.jsonl(
        "patch.jsonl",
        "{\"doc_id\":\"D1\",\"fields\":{\"juridiction\":42}}\n",
    );
    let output = instance.patch(&file);
    assert!(!output.status.success(), "an ill-typed value must fail");
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid_request"), "{stderr}");
    assert!(stderr.contains("juridiction"), "{stderr}");
    assert!(
        !stderr.contains("42"),
        "the message must not echo the value: {stderr}"
    );

    let after = instance.search();
    assert_eq!(after["results"][0]["fields"]["juridiction"], "judiciaire");
    assert_eq!(after["generation"], 1);
}

#[test]
fn a_patch_above_the_entry_limit_is_refused() {
    // ADR-0060, decision 7: the file is bounded at 10 000 entries, and the bound
    // is checked while the file is parsed, before any write.
    let instance = Instance::new();
    let mut text = String::new();
    for index in 0..=crible_store::patch::MAX_ENTRIES {
        text.push_str(&format!(
            "{{\"doc_id\":\"D{index}\",\"fields\":{{\"juridiction\":\"x\"}}}}\n"
        ));
    }
    let file = instance.jsonl("patch.jsonl", &text);
    let output = instance.patch(&file);
    assert!(!output.status.success(), "the limit must be refused");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("payload_too_large"), "{stderr}");

    assert_eq!(
        instance.search()["generation"],
        1,
        "no generation may be created"
    );
}
