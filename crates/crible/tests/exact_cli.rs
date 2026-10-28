// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `--exact` of `crible search` at the CLI boundary (`PLANIFIER-STRATEGIE`,
//! phase P4).
//!
//! References: `docs/phases/P4-recherche-rapide.md` (`PLANIFIER-STRATEGIE`),
//! `docs/05-api.md` (the `exact` field of the search request) and ADR-0074
//! (`exact: true` forces EXACT whatever the pool) with ADR-0072 (one
//! calculation of the dot product, so a BINARY plan whose `N` covers the pool
//! returns the exact answer bit for bit).
//!
//! The flag has no test of its own yet, and the library tests only pin
//! `dense::plan` and `dense::search`; this test pins the command: the flag is
//! parsed, it reaches the pipeline, and on a pool above the threshold the two
//! calls return the same keys, order and score bits because `N` covers the
//! pool (ADR-0072, decision 3).
//!
//! | Test | Criterion |
//! |---|---|
//! | `exact_flag_matches_the_default_answer_on_a_binary_pool` | `--exact` is accepted and both paths agree when `N` covers the pool |

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
const INDEX: &str = "exact-cli";

/// Dimensions of the fixture vectors.
const DIMS: u32 = 4;

/// Number of passages, above [`EXACT_THRESHOLD`] so the default plan is BINARY.
const PASSAGES: u32 = 5;

/// Threshold of the fixture, below the pool size.
const EXACT_THRESHOLD: u32 = 2;

/// Candidates the scan keeps: above the pool, so BINARY rescores every passage.
const RESCORE_CANDIDATES: u32 = 10;

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
    /// Runs `crible index create`, then commits generation 1 with one document
    /// per passage.
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
                documents: (0..PASSAGES).map(document).collect(),
                passages: (0..PASSAGES).map(passage).collect(),
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

/// Configuration of the index, with a BINARY default on the fixture pool.
fn configuration(name: &str) -> String {
    format!(
        r#"{{"name":"{name}","model":"m","reference_precision":"fp16","schema":{SCHEMA},"cluster_by":[],
            "lexical":{{"enabled":false,"language":"fr","positions":false}},
            "search":{{"scan":"binary","rescore_candidates":{RESCORE_CANDIDATES},"exact_threshold":{EXACT_THRESHOLD},
                      "candidates_per_branch":10,"fusion":{{"method":"rrf"}}}},
            "ingest":{{"acceptance_sample":1,"acceptance_min_cosine":0.9}},
            "compaction":{{"dead_ratio":0.1}}}}"#
    )
}

/// Builds one `documents` row of generation 1.
fn document(position: u32) -> DocumentRecord {
    let doc_id = format!("D{position}");
    DocumentRecord {
        doc_id,
        doc_hash: format!("blake3:doc-{position}"),
        fields: serde_json::json!({ "fonds": "LEGI" }),
        updated_generation: Generation::new(1),
    }
}

/// Builds one `passages` row of generation 1.
fn passage(position: u32) -> PassageRecord {
    let doc_id = format!("D{position}");
    PassageRecord {
        iid: Iid::new(position),
        doc_id,
        ord: 0,
        text: format!("passage {position}"),
        passage_hash: format!("blake3:passage-{position}"),
        fields: serde_json::json!({ "partie": "corps" }),
        segment_id: "seg-1".to_owned(),
        created_generation: Generation::new(1),
        dead_since_gen: None,
    }
}

/// Writes the only segment of the fixture and returns its metadata.
///
/// The vectors are unnormalised on purpose: the score is the first component,
/// so the answer is readable without computing a cosine.
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

/// Keys and score **bits** of the `results` array, in order.
///
/// The bits, not the `f32`, so the comparison is an equality and not a
/// tolerance that could hide a different arithmetic (ADR-0072).
fn keys_and_bits(response: &Value) -> Vec<(String, u32)> {
    response["results"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer must hold a results array: {response}"))
        .iter()
        .map(|result| {
            let key = result["key"]
                .as_str()
                .unwrap_or_else(|| panic!("each result carries its key: {result}"))
                .to_owned();
            let score = result["score"]
                .as_f64()
                .unwrap_or_else(|| panic!("each result carries its score: {result}"))
                as f32;
            (key, score.to_bits())
        })
        .collect()
}

#[test]
fn exact_flag_matches_the_default_answer_on_a_binary_pool() {
    let instance = Instance::new();

    let default = instance.search(&["--k", "5"]);
    assert_eq!(
        default["pool_size"], PASSAGES,
        "the fixture pool must be above the threshold, so the default plan is BINARY"
    );
    assert_eq!(
        keys(&default).len(),
        PASSAGES as usize,
        "the fixture must return every passage it holds"
    );

    let forced = instance.search(&["--k", "5", "--exact"]);
    assert_eq!(
        keys_and_bits(&forced),
        keys_and_bits(&default),
        "N covers the pool, so the forced exact call and the binary one must agree \
         on keys, order and score bits (ADR-0072, decision 3)"
    );
}
