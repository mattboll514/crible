// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `REGROUPER-RESULTATS` (phase P4) at the CLI boundary: `--group-by document`,
//! `--passages-per-document` and `--dedup content` of `crible search`.
//!
//! References: the task sheet (`docs/phases/P4-recherche-rapide.md:73-82`),
//! `docs/03-recherche.md` ("Regroupement"), `docs/05-api.md` (the three request
//! fields and the `invalid_request` code) and ADR-0074 (the dense branch is
//! asked for over-sampled candidates).
//!
//! The library tests (`crates/crible-search/tests/regrouper_resultats.rs`) pin
//! the regrouping itself. These tests pin what only the command can show: the
//! flags exist, they reach the pipeline, the JSON answer holds the grouped
//! results in the flat `results` list of `docs/05-api.md`, and a value outside
//! the published domain is refused with a message that names what the flag
//! accepts.
//!
//! | Test | Criterion / decision |
//! |---|---|
//! | `group_by_document_returns_k_documents_and_their_extra_passages` | criteria 1, 2 and 4 |
//! | `dedup_content_fuses_identical_passages_before_grouping` | criterion 3, and step 7 order |
//! | `an_out_of_domain_grouping_option_is_refused` | `docs/05-api.md`, `invalid_request` |

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crible_core::config::{ReferencePrecision, Schema};
use crible_core::ids::{Epoch, Generation, Iid};
use crible_store::generation::{GenerationChange, GenerationStore};
use crible_store::records::{DocumentRecord, PassageRecord};
use crible_store::segment::{SegmentMeta, SegmentSpec, SegmentWriter};
use serde_json::Value;

/// Name of the index.
const INDEX: &str = "regroup-cli";

/// Dimensions of the fixture vectors.
const DIMS: u32 = 4;

/// Digest shared by `D2#0` and `D3#0`, so `dedup` has a duplicate to fuse.
const SHARED_DIGEST: &str = "blake3:shared-content";

/// Schema of the index: one document field, one passage field.
const SCHEMA: &str = r#"{
    "document_fields": { "fonds": { "type": "keyword" } },
    "passage_fields": { "partie": { "type": "keyword" } }
}"#;

/// One temporary instance: the data root, the index and the query vector.
struct Instance {
    /// Keeps the temporary directory alive for the whole test.
    _temp: tempfile::TempDir,
    /// Root of the data directory, passed as `--data`.
    data: PathBuf,
    /// Query vector, identical for every search.
    vector: PathBuf,
}

impl Instance {
    /// Runs `crible index create`, then commits generation 1 with four documents
    /// and five passages.
    ///
    /// | Key | Score | Digest |
    /// |---|---|---|
    /// | `D1#0` | 4.0 | its own |
    /// | `D1#1` | 3.0 | its own |
    /// | `D2#0` | 2.5 | `SHARED_DIGEST` |
    /// | `D3#0` | 2.0 | `SHARED_DIGEST` |
    /// | `D4#0` | 1.0 | its own |
    fn new() -> Self {
        let temp = tempfile::tempdir()
            .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
        let data = temp.path().join("data");
        fs::create_dir_all(&data)
            .unwrap_or_else(|error| panic!("the data root must be creatable: {error}"));
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
        let mut store = GenerationStore::open(&index, &schema())
            .unwrap_or_else(|error| panic!("the fixture index must open: {error}"));
        let segment = write_segment(&index);
        store
            .commit(GenerationChange {
                segments: vec![segment],
                documents: vec![
                    document("D1"),
                    document("D2"),
                    document("D3"),
                    document("D4"),
                ],
                passages: vec![
                    passage(0, "D1", 0, "blake3:d1-0"),
                    passage(1, "D1", 1, "blake3:d1-1"),
                    passage(2, "D2", 0, SHARED_DIGEST),
                    passage(3, "D3", 0, SHARED_DIGEST),
                    passage(4, "D4", 0, "blake3:d4-0"),
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
            vector,
        }
    }

    /// Runs `crible search --vector-file` with the given arguments and returns
    /// its output.
    fn run(&self, arguments: &[&str]) -> Output {
        let mut full: Vec<&str> = vec![
            "search",
            "--data",
            path(&self.data),
            "--index",
            INDEX,
            "--vector-file",
            path(&self.vector),
        ];
        full.extend_from_slice(arguments);
        crible(&full)
    }

    /// Runs `crible search` and returns the JSON response, asserting success.
    fn search(&self, arguments: &[&str]) -> Value {
        let output = self.run(arguments);
        assert!(
            output.status.success(),
            "search {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("the search answer must be JSON: {error}"))
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
fn document(doc_id: &str) -> DocumentRecord {
    DocumentRecord {
        doc_id: doc_id.to_owned(),
        doc_hash: format!("blake3:doc-{doc_id}"),
        fields: serde_json::json!({ "fonds": "LEGI" }),
        updated_generation: Generation::new(1),
    }
}

/// Builds one `passages` row of generation 1.
fn passage(iid: u32, doc_id: &str, ord: u32, passage_hash: &str) -> PassageRecord {
    PassageRecord {
        iid: Iid::new(iid),
        doc_id: doc_id.to_owned(),
        ord,
        text: format!("{doc_id}#{ord}"),
        passage_hash: passage_hash.to_owned(),
        fields: serde_json::json!({ "partie": "corps" }),
        segment_id: "seg-1".to_owned(),
        created_generation: Generation::new(1),
        dead_since_gen: None,
    }
}

/// Writes the only segment of the fixture and returns its metadata.
///
/// The vectors are unnormalised on purpose: the score is the first component,
/// so each sample's rank is readable from [`passage`].
fn write_segment(index: &Path) -> SegmentMeta {
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
    for score in [4.0_f32, 3.0, 2.5, 2.0, 1.0] {
        writer
            .append(&[score, 0.0, 0.0, 0.0])
            .unwrap_or_else(|error| panic!("the fixture vector must be appended: {error}"));
    }
    writer
        .finish()
        .unwrap_or_else(|error| panic!("the fixture segment must be finished: {error}"))
}

/// Runs the binary of the tests.
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

/// Keys of the `results` array, in order.
fn keys(response: &Value) -> Vec<String> {
    response["results"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer must hold a results array: {response}"))
        .iter()
        .map(|result| {
            result["key"]
                .as_str()
                .unwrap_or_else(|| panic!("each result carries its key: {result}"))
                .to_owned()
        })
        .collect()
}

/// Document identifiers of the `results` array, without repetition, in order.
fn documents(response: &Value) -> Vec<String> {
    let mut documents: Vec<String> = Vec::new();
    for result in response["results"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer must hold a results array: {response}"))
    {
        let doc_id = result["doc_id"]
            .as_str()
            .unwrap_or_else(|| panic!("each result carries its doc_id: {result}"))
            .to_owned();
        if !documents.contains(&doc_id) {
            documents.push(doc_id);
        }
    }
    documents
}

#[test]
fn group_by_document_returns_k_documents_and_their_extra_passages() {
    let instance = Instance::new();

    let grouped = instance.search(&["--group-by", "document", "--k", "2"]);
    assert_eq!(
        keys(&grouped),
        vec!["D1#0", "D2#0"],
        "k counts documents, and each one is represented by its best passage"
    );
    assert_eq!(documents(&grouped), vec!["D1", "D2"]);
    assert_eq!(
        grouped["pool_size"], 5,
        "the pool still counts every live passage the filter keeps"
    );

    let more = instance.search(&[
        "--group-by",
        "document",
        "--passages-per-document",
        "2",
        "--k",
        "2",
    ]);
    assert_eq!(
        keys(&more),
        vec!["D1#0", "D1#1", "D2#0"],
        "passages_per_document adds the other matching passages of the same document"
    );
    assert_eq!(documents(&more), vec!["D1", "D2"]);
}

#[test]
fn dedup_content_fuses_identical_passages_before_grouping() {
    let instance = Instance::new();

    let deduplicated = instance.search(&["--dedup", "content", "--k", "5"]);
    assert_eq!(
        keys(&deduplicated),
        vec!["D1#0", "D1#1", "D2#0", "D4#0"],
        "the duplicate content keeps its best copy (D2#0) and loses D3#0"
    );

    // Deduplication runs before the grouping: `D3` loses its only passage to the
    // fused copy, so `D4` fills the third document and the answer still holds
    // k = 3 distinct documents.
    let both = instance.search(&["--group-by", "document", "--dedup", "content", "--k", "3"]);
    assert_eq!(keys(&both), vec!["D1#0", "D2#0", "D4#0"]);
    assert_eq!(documents(&both), vec!["D1", "D2", "D4"]);
}

#[test]
fn an_out_of_domain_grouping_option_is_refused() {
    let instance = Instance::new();

    for arguments in [
        vec!["--passages-per-document", "6"],
        vec!["--passages-per-document", "0"],
        vec!["--group-by", "partie"],
        vec!["--dedup", "text"],
    ] {
        let output = instance.run(&arguments);
        assert!(
            !output.status.success(),
            "crible search {arguments:?} must be refused"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.starts_with("invalid_request: "),
            "the error names its code: {stderr}"
        );
    }
}
