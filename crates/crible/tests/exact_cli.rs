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
//! `dense::plan` and `dense::search`; this test pins the command. `N` is below
//! the pool, so BINARY keeps the 100 passages whose code matches the query and
//! rescores them: it drops `D100` and never sees the four high-score passages.
//! `--exact` scores the whole pool, so a flag that never reached the pipeline
//! would return the BINARY answer and the test would fail.
//!
//! | Test | Criterion |
//! |---|---|
//! | `exact_flag_returns_the_passages_the_binary_scan_lost` | `--exact` is accepted, reaches the pipeline, and returns the passages below `N` |

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

/// Number of passages: above the 100 candidates the dense branch always asks
/// for (`group::candidates_per_branch`), and above [`EXACT_THRESHOLD`], so the
/// default plan is BINARY and the scan keeps 100 of the 105 passages.
const PASSAGES: u32 = 105;

/// Threshold of the fixture, below the pool size.
const EXACT_THRESHOLD: u32 = 2;

/// Candidates of the configuration: `max(N, 100)` is what the scan keeps, so
/// this value is a floor and the test relies on the 100 above, not on it.
const RESCORE_CANDIDATES: u32 = 2;

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

/// One fixture vector: the low group first, the high scores behind it.
///
/// Scores are distinct so the exact answer has no tie, and the first components
/// stay far from the fp16 rounding step.
fn vector(position: u32) -> [f32; 4] {
    if position < 101 {
        [0.001 + position as f32 * 0.000001, -5.0, 0.0, 0.0]
    } else {
        [2.0 + (position - 101) as f32 * 0.1, 1.0, 0.0, 0.0]
    }
}

/// Writes the only segment of the fixture and returns its metadata.
///
/// The vectors are unnormalised on purpose: the score is the first component of
/// the cosine, so the answer is readable without computing it. The code of a
/// vector sets a bit where the component is strictly positive.
///
/// Passage `i < 101` has a tiny score and the query's code: the scan keeps the
/// 100 lowest identifiers of that group and drops `D100`. Passages `101..105`
/// carry a high score but a second positive component, so their Hamming
/// distance is one: the scan never sees them, and the exact path returns two of
/// them.
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
    for position in 0..PASSAGES {
        writer
            .append(&vector(position))
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

#[test]
fn exact_flag_returns_the_passages_the_binary_scan_lost() {
    let instance = Instance::new();

    let default = instance.search(&["--k", "2"]);
    assert_eq!(
        default["pool_size"], PASSAGES,
        "the fixture pool must be above the threshold, so the default plan is BINARY"
    );
    assert_eq!(
        keys(&default),
        ["D99#0", "D98#0"],
        "the scan keeps 100 of the 101 passages whose code equals the query's, \
         so BINARY never reaches a high score"
    );

    let forced = instance.search(&["--k", "2", "--exact"]);
    assert_eq!(
        keys(&forced),
        ["D104#0", "D103#0"],
        "--exact must score the whole pool and return the passages below the \
         scan; a flag the pipeline ignored would return the binary answer"
    );
}
