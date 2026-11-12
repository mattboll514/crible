// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `crible eval` at the CLI boundary (`COMMANDE-EVAL`, phase P7).
//!
//! References: the task sheet (`docs/phases/P7-observabilite.md:113-124` with
//! its two dated notes), `docs/06-observabilite.md` ("Éval"), and ADR-0125
//! (seventeen decisions: local mode, input format, metrics, in-memory overrides,
//! latency, report, baseline, `--out`, CI tests, demo, confidentiality).
//!
//! | Test | Criterion / decision |
//! |---|---|
//! | `metrics_on_a_hand_built_case_are_exact` | quality at the document, hand-computed recall@k and MRR@k (decisions 4 and 5) |
//! | `fidelity_compares_the_plan_against_exact` | fidelity against `exact: true`, hand-computed recall@10 and overlap@10 (decision 6) |
//! | `latencies_are_reported_per_stage_at_nearest_rank` | keys per stage, p50 ≤ p95 ≤ p99, absent stage omitted (decision 9) |
//! | `the_report_is_closed_and_carries_no_query_text` | closed JSON, `format_version: 3`, `source`, ids only (decisions 11 and 16 of ADR-0125, decision 7 of ADR-0131, decisions 2 to 4 of ADR-0155) |
//! | `an_empty_exact_answer_is_excluded_and_counted` | empty exact answer excluded and counted, denominator written when not 10 (complément du 2026-11-01, point C1) |
//! | `a_text_query_never_reaches_the_reports` | no query text in either report, lexical fixture without an encoder (decision 16) |
//! | `an_invalid_line_stops_the_command_with_its_number` | closed input, line number and cause (decision 3) |
//! | `an_existing_report_is_never_overwritten` | no overwrite, nothing written on refusal (decision 13) |
//! | `a_baseline_of_an_unknown_version_or_another_k_is_refused` | clear refusal of a foreign report (decision 12) |
//! | `a_baseline_detects_the_rescore_candidates_20_regression` | injected regression, exit code 1, lost query named (decisions 8 and 12) |
//! | `an_identical_replay_regresses_nothing` | exit code 0 on a replay (decision 12) |
//! | `the_command_writes_nothing_under_the_data_directory` | read-only, ADR-0116 (decision 1) |
//! | `synth_small_regression_is_detected_by_the_baseline` | CI test of the regression on `synth-small` (decisions 8 and 14) |
//! | `the_help_states_when_models_is_needed` | `--models` only for an encoding mode (decision 2) |
//!
//! The metric fixtures are built by hand: the binary codes of their vectors are
//! chosen so that the scan of the fixture, its exact path and the grouped
//! answer are predictable without an encoder and without ONNX Runtime.
//!
//! # Why the tests of this binary are serialised
//!
//! Every test of this binary holds [`common::SerialGuard`] for its whole body,
//! because the fixture opens a store for writing in this process while another
//! test spawns a `crible eval` child: a `fork` duplicates the `WRITER_LOCK` of
//! that store, which would refuse a later reopen as `overloaded` (ADR-0116,
//! decision 2; see [`common::SerialGuard`]).
//!
//! ```text
//! cargo test -p crible --test eval_cli
//! ```

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use crible_core::config::{ReferencePrecision, Schema};
use crible_core::ids::{Epoch, Generation, Iid};
use crible_core::model::ModelConfig;
use crible_store::generation::{GenerationChange, GenerationStore};
use crible_store::records::{DocumentRecord, PassageRecord};
use crible_store::segment::{SegmentMeta, SegmentSpec, SegmentWriter};
use serde_json::{Value, json};

use crate::common::{crible, path, require_synth, synth_root};

/// Schema of the hand-built indexes: two keyword document fields, one passage
/// field.
const SCHEMA: &str = r#"{
    "document_fields": { "fonds": { "type": "keyword" }, "marqueur": { "type": "keyword" } },
    "passage_fields": { "partie": { "type": "keyword" } }
}"#;

/// Dimensions of the hand-built vectors.
const DIMS: u32 = 4;

/// Value of the filter marker of the confidentiality test.
const FILTER_MARKER: &str = "MARQUEUR-FILTRE-7f1c";

/// Text of the query of the confidentiality test: it must never reach a report.
const QUERY_MARKER: &str = "MARQUEUR-REQUETE-b3e5";

/// One hand-built index: documents, passages and vectors, all in file order.
struct Index {
    /// Name of the index.
    name: &'static str,
    /// Configuration of the index, written as `index.json` by `index create`.
    configuration: String,
    /// One document per passage, in `documents`.
    documents: Vec<DocumentRecord>,
    /// One passage per document, in the order of `documents`.
    passages: Vec<PassageRecord>,
    /// One vector per passage, in the order of `passages`.
    vectors: Vec<[f32; 4]>,
}

/// One temporary instance: the data root and the out directory.
struct Instance {
    /// Keeps the temporary directory alive for the whole test.
    temp: tempfile::TempDir,
    /// Root of the data directory, passed as `--data`.
    data: PathBuf,
    /// Directory the reports are written into, outside the data root.
    out: PathBuf,
}

impl Instance {
    /// Writes the configuration and commits generation 1 of `index`.
    fn new(index: &Index) -> Self {
        let instance = Self::with_data(index);
        let config = instance.temp.path().join("index.json");
        fs::write(&config, &index.configuration)
            .unwrap_or_else(|error| panic!("the configuration must be written: {error}"));
        let created = crible(&[
            "index",
            "create",
            "--data",
            path(&instance.data),
            "--config",
            path(&config),
        ]);
        assert!(
            created.status.success(),
            "index create failed: {}",
            String::from_utf8_lossy(&created.stderr)
        );
        let directory = instance.data.join("indexes").join(index.name);
        let mut store = GenerationStore::open(&directory, &schema())
            .unwrap_or_else(|error| panic!("the fixture index must open: {error}"));
        let segment = write_segment(&directory, &index.vectors);
        store
            .commit(GenerationChange {
                segments: vec![segment],
                documents: index.documents.clone(),
                passages: index.passages.clone(),
                deletions: Vec::new(),
            })
            .unwrap_or_else(|error| panic!("the fixture generation must commit: {error}"));
        instance
    }

    /// An empty instance: the data root and the out directory, both created.
    fn with_data(_index: &Index) -> Self {
        let temp = tempfile::tempdir()
            .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
        let data = temp.path().join("data");
        let out = temp.path().join("out");
        for directory in [&data, &out] {
            fs::create_dir_all(directory).unwrap_or_else(|error| {
                panic!("{} must be creatable: {error}", directory.display())
            });
        }
        Self { temp, data, out }
    }

    /// Writes a JSONL file of eval queries under the temporary root.
    fn queries(&self, name: &str, lines: &[Value]) -> PathBuf {
        let path = self.temp.path().join(name);
        let body: String = lines
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<Vec<_>>()
            .join("");
        fs::write(&path, body)
            .unwrap_or_else(|error| panic!("the queries file must be written: {error}"));
        path
    }

    /// Runs `crible eval` on `lines`, writing the reports into `out`.
    fn eval_at(&self, index: &str, lines: &[Value], out: &Path, extra: &[&str]) -> Output {
        let queries = self.queries("queries.jsonl", lines);
        self.eval_file_at(index, &queries, out, extra)
    }

    /// Runs `crible eval` on an existing queries file, into `out`.
    fn eval_file_at(&self, index: &str, queries: &Path, out: &Path, extra: &[&str]) -> Output {
        let mut arguments: Vec<String> = vec![
            "eval".to_owned(),
            "--data".to_owned(),
            path(&self.data).to_owned(),
            "--index".to_owned(),
            index.to_owned(),
            "--queries".to_owned(),
            path(queries).to_owned(),
            "--out".to_owned(),
            path(out).to_owned(),
        ];
        arguments.extend(extra.iter().map(|argument| (*argument).to_owned()));
        let borrowed: Vec<&str> = arguments.iter().map(String::as_str).collect();
        crible(&borrowed)
    }
}

/// The two report paths the command prints, in order.
fn reports(output: &Output) -> (PathBuf, PathBuf) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut json_path = None;
    let mut markdown_path = None;
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("json: ") {
            json_path = Some(PathBuf::from(rest));
        }
        if let Some(rest) = line.strip_prefix("markdown: ") {
            markdown_path = Some(PathBuf::from(rest));
        }
    }
    (
        json_path.unwrap_or_else(|| panic!("eval prints its JSON path: {stdout}")),
        markdown_path.unwrap_or_else(|| panic!("eval prints its Markdown path: {stdout}")),
    )
}

/// Runs `crible eval` and asserts success, returning its two report paths.
fn eval_ok(instance: &Instance, lines: &[Value], extra: &[&str]) -> (PathBuf, PathBuf) {
    let output = instance.eval_at("fixture", lines, &instance.out, extra);
    assert!(
        output.status.success(),
        "eval {extra:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    reports(&output)
}

/// Reads a JSON report.
fn read_report(path: &Path) -> Value {
    let raw = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("{} must be valid JSON: {error}", path.display()))
}

/// Schema of the fixture index, parsed.
fn schema() -> Schema {
    Schema::from_json_str(SCHEMA)
        .unwrap_or_else(|error| panic!("the fixture schema must be valid: {error}"))
}

/// Configuration of the fixture index: BINARY available, N above the pool.
///
/// `scan: "binary"` is what makes the strategy a choice (ADR-0074): with
/// `scan: "none"` the planner always answers EXACT, and neither the fidelity
/// nor the injected regression could be observed. `rescore_candidates: 5000`
/// keeps the default faithful on a pool of 105 passages: a run is faithful
/// unless the command line lowers N, which is exactly what the fidelity test
/// and the injected regression do (`--rescore-candidates 100` and `20`).
fn configuration(name: &str, lexical: bool) -> String {
    format!(
        r#"{{"name":"{name}","model":"fixture","reference_precision":"fp16","schema":{SCHEMA},
            "cluster_by":[],"lexical":{{"enabled":{lexical},"language":"fr","positions":false}},
            "search":{{"scan":"binary","rescore_candidates":5000,"exact_threshold":5000,
                      "candidates_per_branch":100,"fusion":{{"method":"rrf"}}}},
            "ingest":{{"acceptance_sample":1,"acceptance_min_cosine":0.9}},
            "compaction":{{"dead_ratio":0.1}}}}"#
    )
}

/// Writes the only segment of a fixture index and returns its metadata.
fn write_segment(index: &Path, vectors: &[[f32; 4]]) -> SegmentMeta {
    let vectors: Vec<Vec<f32>> = vectors.iter().map(|vector| vector.to_vec()).collect();
    write_segment_dims(index, &vectors, DIMS)
}

/// Writes the only segment of a fixture index of `dims` dimensions.
fn write_segment_dims(index: &Path, vectors: &[Vec<f32>], dims: u32) -> SegmentMeta {
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
            dims,
            generation: Generation::new(1),
            batch_id: "batch-eval".to_owned(),
            precision: ReferencePrecision::Fp16,
        },
    )
    .unwrap_or_else(|error| panic!("the segment writer must start: {error}"));
    for vector in vectors {
        writer
            .append(vector)
            .unwrap_or_else(|error| panic!("the fixture vector must be appended: {error}"));
    }
    writer
        .finish()
        .unwrap_or_else(|error| panic!("the fixture segment must be finished: {error}"))
}

/// Builds the fixture from `(doc_id, vector)` pairs, one passage per document.
fn fixture(name: &'static str, entries: &[(String, [f32; 4])]) -> Index {
    let documents = entries
        .iter()
        .enumerate()
        .map(|(position, (doc_id, _))| DocumentRecord {
            doc_id: doc_id.clone(),
            doc_hash: format!("blake3:doc-{position}"),
            fields: json!({ "fonds": "LEGI", "marqueur": FILTER_MARKER }),
            updated_generation: Generation::new(1),
        })
        .collect();
    let passages = entries
        .iter()
        .enumerate()
        .map(|(position, (doc_id, _))| PassageRecord {
            iid: Iid::new(u32::try_from(position).unwrap_or_default()),
            doc_id: doc_id.clone(),
            ord: 0,
            text: format!("passage {position}"),
            passage_hash: format!("blake3:passage-{position}"),
            fields: json!({ "partie": "corps" }),
            segment_id: "seg-1".to_owned(),
            created_generation: Generation::new(1),
            dead_since_gen: None,
        })
        .collect();
    Index {
        name,
        configuration: configuration(name, false),
        documents,
        passages,
        vectors: entries.iter().map(|(_, vector)| *vector).collect(),
    }
}

/// Six passages, one per document, whose ranking the dot product fixes alone.
///
/// With a pool below the 100 candidates the branch always asks for, every
/// passage is rescored whatever the strategy: the answer of this fixture is the
/// exact ranking in every mode, and the quality figures are computable by hand.
fn quality_fixture() -> Index {
    fixture(
        "fixture",
        &[
            ("A".to_owned(), [1.0, 0.0, 0.0, 0.0]),
            ("B".to_owned(), [0.9, 0.1, 0.0, 0.0]),
            ("C".to_owned(), [0.5, 0.5, 0.0, 0.0]),
            ("D".to_owned(), [0.0, 1.0, 0.0, 0.0]),
            ("E".to_owned(), [0.0, 0.0, 1.0, 0.0]),
            ("F".to_owned(), [0.0, 0.0, 0.0, 1.0]),
        ],
    )
}

/// 305 passages whose binary scan drops the best cosine.
///
/// The query is `[1, 0.1, 0, 0]`, of code `(1, 1, 0, 0)`.
///
/// - passages `O0` … `O99` carry `[0.05 + i×0.0001, 1, 0, 0]`: their code is
///   the query's, so their Hamming distance is 0, and their cosine grows with
///   `i`, so the best of them are `O95` … `O99`;
/// - passages `W0` … `W4` carry `[1, -0.05, 0.02, 0]`: their code is
///   `(1, 0, 1, 0)`, two bits away, and their cosine is the highest of the
///   fixture;
/// - passages `F0` … `F199` carry `[-1, 0.5, 0, 0]`: their code is one bit
///   away and their cosine is negative. They are the filler that keeps the
///   `W` passages out of the 200 candidates a **grouped** run asks for
///   (`clamp(k × 20, 200, 1000)`): under `N = 20` the scan then keeps the 100
///   `O` and 100 `F` passages, never a `W` one, so the injected regression is
///   visible at the document too.
///
/// The flat run asks for 100 candidates. With `N = 100` the scan keeps the 100
/// passages of distance 0 — the `O` ones — and rescores them, so the binary
/// top-10 holds ten `O` passages while the exact top-10 holds the five `W` and
/// the best five `O`: **recall@10 = 5/10 = 0,5** and **overlap@10 = 5/10 =
/// 0,5**. With an `N` above the pool the scan keeps everything, and both
/// figures become 1.
fn fidelity_fixture() -> Index {
    let mut entries = Vec::new();
    for position in 0..100 {
        let offset = position as f32 * 0.0001;
        entries.push((format!("O{position}"), [0.05 + offset, 1.0, 0.0, 0.0]));
    }
    for position in 0..5 {
        entries.push((format!("W{position}"), [1.0, -0.05, 0.02, 0.0]));
    }
    for position in 0..200 {
        entries.push((format!("F{position}"), [-1.0, 0.5, 0.0, 0.0]));
    }
    fixture("fixture", &entries)
}

/// Three passages, one per document, with the lexical branch built by the
/// commit: a text query can be scored without any encoder or ONNX Runtime.
fn lexical_fixture() -> Index {
    let mut index = fixture(
        "fixture",
        &[
            ("A".to_owned(), [1.0, 0.0, 0.0, 0.0]),
            ("B".to_owned(), [0.9, 0.1, 0.0, 0.0]),
            ("C".to_owned(), [0.5, 0.5, 0.0, 0.0]),
        ],
    );
    index.configuration = configuration("fixture", true);
    index
}

/// The vector of the fidelity fixture's query.
const FIDELITY_QUERY: [f32; 4] = [1.0, 0.1, 0.0, 0.0];

/// One eval query line with a provided vector.
fn query(id: &str, vector: &[f32], expected: &[&str]) -> Value {
    let expected: Vec<&str> = expected.to_vec();
    json!({ "id": id, "vector": vector, "expected": expected })
}

/// The two queries of the fidelity fixture.
fn fidelity_lines() -> Vec<Value> {
    vec![
        query("q1", &FIDELITY_QUERY, &["W0"]),
        query("q2", &FIDELITY_QUERY, &["O99"]),
    ]
}

/// A number of a report, failing with its path when absent.
fn number(value: &Value, pointer: &str) -> f64 {
    value
        .pointer(pointer)
        .and_then(Value::as_f64)
        .unwrap_or_else(|| panic!("{pointer} must be a number in {value}"))
}

/// Asserts two figures are equal to `1e-9`.
fn assert_close(actual: f64, expected: f64, label: &str) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "{label}: expected {expected}, got {actual}"
    );
}

// --- Metrics ---------------------------------------------------------------

#[test]
fn metrics_on_a_hand_built_case_are_exact() {
    let _serial = common::SerialGuard::acquire();
    let instance = Instance::new(&quality_fixture());
    let lines = [
        query("q1", &[1.0, 0.0, 0.0, 0.0], &["B", "C"]),
        query("q2", &[1.0, 0.0, 0.0, 0.0], &["C", "F"]),
        query("q3", &[1.0, 0.0, 0.0, 0.0], &[]),
    ];
    let (json_path, _) = eval_ok(&instance, &lines, &["--modes", "dense", "--k", "3"]);
    let report = read_report(&json_path);

    assert_eq!(
        report["format_version"], 3,
        "the replayable top-k is a version 3 report (ADR-0155, decision 1)"
    );
    assert_eq!(report["k"], 3);
    assert_eq!(report["queries"], 3);
    assert_eq!(
        report["queries_with_expected"], 2,
        "a query without `expected` stays out of the quality means"
    );

    // top-3 documents: A (cos 1), B (0.994), C (0.707).
    // q1 expects B and C: recall 2/2, and B is at rank 2.
    // q2 expects C and F: recall 1/2, and C is at rank 3.
    assert_close(
        number(&report, "/modes/dense/quality/recall_at_k"),
        (1.0 + 0.5) / 2.0,
        "mean recall@3",
    );
    assert_close(
        number(&report, "/modes/dense/quality/mrr_at_k"),
        (0.5 + 1.0 / 3.0) / 2.0,
        "mean MRR@3",
    );
    assert_eq!(report["modes"]["dense"]["quality"]["n"], 2);

    let rows = report["modes"]["dense"]["queries"]
        .as_array()
        .unwrap_or_else(|| panic!("the mode holds its per-query rows: {report}"));
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["id"], "q1");
    assert_close(
        number(&rows[0], "/fidelity_recall_at_10"),
        1.0,
        "q1 fidelity",
    );
    assert_close(number(&rows[0], "/quality_recall_at_k"), 1.0, "q1 recall");
    assert_close(number(&rows[0], "/quality_mrr_at_k"), 0.5, "q1 MRR");
    // Six passages and an exact top-10 of six: the denominator is 6, not 10,
    // and 6/6 is 1.0 (ADR-0125, complément du 2026-11-01, point C1).
    assert_eq!(
        rows[0]["fidelity_denominator"], 6,
        "the report writes the denominator when it differs from 10"
    );
    assert_close(number(&rows[1], "/quality_recall_at_k"), 0.5, "q2 recall");
    assert_eq!(rows[2]["id"], "q3");
    assert!(
        rows[2]["quality_recall_at_k"].is_null(),
        "a query without `expected` carries no quality figure: {rows:?}"
    );
}

#[test]
fn fidelity_compares_the_plan_against_exact() {
    let _serial = common::SerialGuard::acquire();
    // The denominator is the size of the exact top-10, not a fixed 10
    // (ADR-0125, complément du 2026-11-01, point C1): on this pool of 305
    // passages it is 10, so the figures are those of ADR-0076, decision 1.
    let instance = Instance::new(&fidelity_fixture());
    let lines = fidelity_lines();
    let (json_path, _) = eval_ok(
        &instance,
        &lines,
        &[
            "--modes",
            "dense",
            "--k",
            "10",
            "--rescore-candidates",
            "100",
        ],
    );
    let report = read_report(&json_path);

    assert_eq!(
        report["exact_threshold"], 0,
        "the fidelity runs with `exact_threshold = 0` by default (decision 7)"
    );
    assert_eq!(
        report["modes"]["dense"]["dense_strategy"]["binary"], 2,
        "every query of a non-empty pool runs the binary strategy: {report}"
    );
    assert_close(
        number(&report, "/modes/dense/fidelity/recall_at_10"),
        0.5,
        "recall@10 of the scan: five of the ten exact passages survive",
    );
    assert_close(
        number(&report, "/modes/dense/fidelity/overlap_at_10"),
        0.5,
        "overlap@10 at the document: five of the ten documents survive",
    );
    // The quality side sees the same gap: `W0` is first in exact and absent
    // from the binary answer.
    assert_close(
        number(&report, "/modes/dense/quality/recall_at_k"),
        0.5,
        "q1 loses W0, q2 keeps O99",
    );

    // The same index, with `N` above the pool: the scan keeps every passage and
    // the fidelity becomes 1.0.
    let out = instance.temp.path().join("wide");
    fs::create_dir_all(&out).unwrap_or_else(|error| panic!("{error}"));
    let output = instance.eval_at(
        "fixture",
        &lines,
        &out,
        &["--modes", "dense", "--rescore-candidates", "5000"],
    );
    assert!(
        output.status.success(),
        "the wide run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let (wide_path, _) = reports(&output);
    let wide = read_report(&wide_path);
    assert_close(
        number(&wide, "/modes/dense/fidelity/recall_at_10"),
        1.0,
        "N above the pool rescores every passage, so the scan is exact",
    );
}

#[test]
fn latencies_are_reported_per_stage_at_nearest_rank() {
    let _serial = common::SerialGuard::acquire();
    let instance = Instance::new(&quality_fixture());
    let lines = [query("q1", &[1.0, 0.0, 0.0, 0.0], &["B"])];
    let (json_path, _) = eval_ok(&instance, &lines, &["--modes", "dense", "--repeat", "3"]);
    let report = read_report(&json_path);

    let dense = &report["modes"]["dense"]["latency"];
    for stage in ["validate", "filter", "dense", "group", "fetch", "total"] {
        let entry = dense
            .get(stage)
            .unwrap_or_else(|| panic!("the dense mode reports {stage}: {dense}"));
        let p50 = number(entry, "/p50");
        let p95 = number(entry, "/p95");
        let p99 = number(entry, "/p99");
        assert!(
            p50 <= p95 && p95 <= p99,
            "{stage}: p50 {p50} ≤ p95 {p95} ≤ p99 {p99}"
        );
    }
    for stage in ["encode", "lexical", "fusion"] {
        assert!(
            dense.get(stage).is_none(),
            "`{stage}` did not run, so it has no sample and no column: {dense}"
        );
    }
    assert!(
        number(dense, "/total/p95") > 0.0,
        "the total is measured: {dense}"
    );

    // The fixture has no lexical index, so a lexical campaign is refused
    // before any work, as `crible search` refuses it (ADR-0089, decision 3).
    let output = instance.eval_at(
        "fixture",
        &lines,
        &instance.temp.path().join("lexical"),
        &["--modes", "lexical"],
    );
    assert!(!output.status.success(), "a disabled branch is refused");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("lexical branch is disabled") && stderr.contains("fixture"),
        "the refusal names the branch and the index: {stderr}"
    );
}

// --- Format ----------------------------------------------------------------

#[test]
fn the_report_is_closed_and_carries_no_query_text() {
    let _serial = common::SerialGuard::acquire();
    let instance = Instance::new(&quality_fixture());
    let line = json!({
        "id": "q1",
        "vector": [1.0, 0.0, 0.0, 0.0],
        "filter": { "marqueur": FILTER_MARKER },
        "expected": ["A"],
    });
    let models = model_root(&instance, "fixture", &onnx_model("fixture", DIMS));
    let models = path(&models);
    let (json_path, markdown_path) = eval_ok(
        &instance,
        &[line],
        &["--models", models, "--modes", "dense"],
    );
    let report = read_report(&json_path);

    let members: BTreeSet<&str> = report
        .as_object()
        .unwrap_or_else(|| panic!("the report is an object: {report}"))
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        members,
        BTreeSet::from([
            "format_version",
            "command",
            "source",
            "date",
            "game",
            "index",
            "generation",
            "version",
            "pool_size",
            "k",
            "repeat",
            "exact_threshold",
            "rescore_candidates",
            "rescore_candidates_source",
            "queries",
            "queries_with_expected",
            "queries_fingerprint",
            "model_fingerprint",
            "modes",
        ]),
        "the report's members are fixed (decision 11; ADR-0155, decisions 3 and 4)"
    );
    let mode: BTreeSet<&str> = report["modes"]["dense"]
        .as_object()
        .unwrap_or_else(|| panic!("the mode is an object: {report}"))
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        mode,
        BTreeSet::from([
            "dense_strategy",
            "fidelity",
            "fidelity_excluded",
            "quality",
            "latency",
            "queries",
        ])
    );
    assert_eq!(report["index"], "fixture");
    assert_eq!(report["repeat"], 3);
    assert!(report["generation"].as_u64().unwrap_or_default() >= 1);
    assert_eq!(
        report["rescore_candidates_source"], "configuration",
        "without the flag, N comes from the configuration of the index"
    );
    let command = report["command"]
        .as_str()
        .unwrap_or_else(|| panic!("the report carries its command: {report}"));
    for part in [
        "crible eval",
        "--data ",
        "--index fixture",
        "--queries ",
        "--out ",
    ] {
        assert!(
            command.contains(part),
            "the command is replayable, so it carries {part:?}: {command}"
        );
    }
    assert!(
        !command.contains("--rescore-candidates"),
        "the command omits a flag the campaign did not use: {command}"
    );

    // The names are `eval-<jeu>-<AAAA-MM-JJ>.json` and `.md`, dated on the day
    // of the campaign (decision 11); the test never pins the clock.
    let name = json_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_else(|| panic!("the report has a name: {}", json_path.display()));
    let stem = name
        .strip_prefix("eval-queries-")
        .and_then(|rest| rest.strip_suffix(".json"))
        .unwrap_or_else(|| panic!("the report is `eval-queries-<date>.json`: {name}"));
    assert_eq!(stem.len(), 10, "the date is `AAAA-MM-JJ`: {stem}");
    assert_eq!(stem.as_bytes()[4], b'-', "the date is `AAAA-MM-JJ`: {stem}");
    assert_eq!(stem.as_bytes()[7], b'-', "the date is `AAAA-MM-JJ`: {stem}");
    assert_eq!(
        markdown_path.with_extension("json"),
        json_path,
        "the two reports share their name"
    );

    let json_body = fs::read_to_string(&json_path).unwrap_or_else(|error| panic!("{error}"));
    let markdown = fs::read_to_string(&markdown_path)
        .unwrap_or_else(|error| panic!("the Markdown report must exist: {error}"));
    assert!(
        !json_body.contains(FILTER_MARKER),
        "the filter value never reaches the JSON report"
    );
    assert!(
        !markdown.contains(FILTER_MARKER),
        "the filter value never reaches the Markdown report"
    );
    assert!(
        markdown.contains("## Lecture"),
        "the Markdown follows the model of `docs/mesures/README.md`: {markdown}"
    );
}

#[test]
fn an_empty_exact_answer_is_excluded_and_counted() {
    let _serial = common::SerialGuard::acquire();
    let instance = Instance::new(&quality_fixture());
    let lines = [
        query("q1", &[1.0, 0.0, 0.0, 0.0], &["A"]),
        json!({
            "id": "q2",
            "vector": [1.0, 0.0, 0.0, 0.0],
            "filter": { "fonds": { "eq": "ABSENT" } },
        }),
    ];
    let (json_path, markdown_path) = eval_ok(&instance, &lines, &["--modes", "dense"]);
    let report = read_report(&json_path);

    // ADR-0125, complément du 2026-11-01 (C1): a query whose exact answer is
    // empty has no denominator, so it is excluded from the mean and counted
    // apart with its `id`.
    assert_eq!(
        report["modes"]["dense"]["fidelity"]["n"], 1,
        "only q1 carries a fidelity figure: {report}"
    );
    assert_eq!(
        report["modes"]["dense"]["fidelity_excluded"],
        json!(["q2"]),
        "q2 is counted apart, with its id"
    );
    let rows = report["modes"]["dense"]["queries"]
        .as_array()
        .unwrap_or_else(|| panic!("the mode holds its per-query rows: {report}"));
    assert_eq!(rows[1]["id"], "q2");
    assert!(
        rows[1].get("fidelity_recall_at_10").is_none(),
        "an empty exact answer carries no recall: {rows:?}"
    );
    // The fixture has six passages, so the denominator is 6, not 10: the report
    // writes it because it differs (C1).
    assert_eq!(
        rows[0]["fidelity_denominator"], 6,
        "the exact top-10 holds the six passages of the pool"
    );

    let markdown = fs::read_to_string(&markdown_path)
        .unwrap_or_else(|error| panic!("the Markdown report must exist: {error}"));
    assert!(
        markdown.contains("liste exacte vide") && markdown.contains("`q2`"),
        "the Markdown counts the excluded query: {markdown}"
    );
    assert!(
        markdown.contains("sur les requêtes dont la liste exacte n'est pas vide"),
        "the reading states the mean of the complemented C1: {markdown}"
    );
    assert!(
        !markdown.contains("ABSENT"),
        "the filter value never reaches the Markdown report"
    );
}

#[test]
fn a_text_query_never_reaches_the_reports() {
    let _serial = common::SerialGuard::acquire();
    // A lexical index is scored without an encoder, so a text query can be
    // exercised without ONNX Runtime and without `--models`.
    let instance = Instance::new(&lexical_fixture());
    let line = json!({ "id": "q1", "query": QUERY_MARKER, "expected": ["A"] });
    let (json_path, markdown_path) = eval_ok(&instance, &[line], &["--modes", "lexical"]);
    let report = read_report(&json_path);
    assert!(
        report["modes"]["lexical"]["fidelity_excluded"].is_null(),
        "a lexical run has no fidelity, so no excluded fidelity query either: {report}"
    );
    let json_body = fs::read_to_string(&json_path).unwrap_or_else(|error| panic!("{error}"));
    let markdown = fs::read_to_string(&markdown_path)
        .unwrap_or_else(|error| panic!("the Markdown report must exist: {error}"));
    assert!(
        !json_body.contains(QUERY_MARKER),
        "the query text never reaches the JSON report"
    );
    assert!(
        !markdown.contains(QUERY_MARKER),
        "the query text never reaches the Markdown report"
    );
}

#[test]
fn an_invalid_line_stops_the_command_with_its_number() {
    let _serial = common::SerialGuard::acquire();
    let instance = Instance::new(&quality_fixture());

    let cases = [
        (
            "an unknown member",
            "{\"id\": \"q1\", \"vector\": [1.0, 0.0, 0.0, 0.0], \"mode\": \"dense\"}",
        ),
        (
            "both a query and a vector",
            "{\"id\": \"q1\", \"query\": \"t\", \"vector\": [1.0, 0.0, 0.0, 0.0]}",
        ),
        ("invalid JSON", "{\"id\": \"q1\", "),
        (
            "neither a query nor a vector",
            "{\"id\": \"q1\", \"expected\": [\"A\"]}",
        ),
    ];
    for (label, line) in cases {
        let queries = instance.temp.path().join("invalid.jsonl");
        let body = [
            "{\"id\": \"ok\", \"vector\": [1.0, 0.0, 0.0, 0.0]}\n",
            line,
            "\n",
        ]
        .concat();
        fs::write(&queries, body).unwrap_or_else(|error| panic!("{error}"));
        let out = instance.temp.path().join("invalid-out");
        fs::create_dir_all(&out).unwrap_or_else(|error| panic!("{error}"));
        let output = instance.eval_file_at("fixture", &queries, &out, &[]);
        assert!(
            !output.status.success(),
            "{label}: a malformed line must stop the command"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("line 2"),
            "{label}: the message names the line number: {stderr}"
        );
        assert!(
            stderr.starts_with("invalid_request: "),
            "{label}: the refusal carries the category of `docs/05`: {stderr}"
        );
    }
}

#[test]
fn an_existing_report_is_never_overwritten() {
    let _serial = common::SerialGuard::acquire();
    let instance = Instance::new(&quality_fixture());
    let lines = [query("q1", &[1.0, 0.0, 0.0, 0.0], &["A"])];
    let (json_path, markdown_path) = eval_ok(&instance, &lines, &["--modes", "dense"]);
    let before = fs::read(&json_path).unwrap_or_else(|error| panic!("{error}"));
    fs::remove_file(&markdown_path).unwrap_or_else(|error| panic!("{error}"));

    let output = instance.eval_at("fixture", &lines, &instance.out, &["--modes", "dense"]);
    assert!(
        !output.status.success(),
        "an existing report is not replaced"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(
            json_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
        ),
        "the refusal names the existing path: {stderr}"
    );
    assert_eq!(
        fs::read(&json_path).unwrap_or_else(|error| panic!("{error}")),
        before,
        "the report is untouched"
    );
    assert!(
        !markdown_path.exists(),
        "the command writes neither file when one of them exists"
    );
}

// --- Baseline --------------------------------------------------------------

#[test]
fn a_baseline_of_an_unknown_version_or_another_k_is_refused() {
    let _serial = common::SerialGuard::acquire();
    let instance = Instance::new(&quality_fixture());
    let lines = [query("q1", &[1.0, 0.0, 0.0, 0.0], &["A"])];
    let (reference, _) = eval_ok(&instance, &lines, &["--modes", "dense", "--k", "3"]);

    let mut foreign = read_report(&reference);
    foreign["format_version"] = json!(7);
    let path_seven = instance.temp.path().join("version-seven.json");
    fs::write(&path_seven, foreign.to_string()).unwrap_or_else(|error| panic!("{error}"));
    let out = instance.temp.path().join("version-seven");
    fs::create_dir_all(&out).unwrap_or_else(|error| panic!("{error}"));
    let output = instance.eval_at(
        "fixture",
        &lines,
        &out,
        &[
            "--modes",
            "dense",
            "--k",
            "3",
            "--baseline",
            path(&path_seven),
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains('7'),
        "the refusal names the version it read: {stderr}"
    );

    let mut other_k = read_report(&reference);
    other_k["k"] = json!(20);
    let path_k = instance.temp.path().join("other-k.json");
    fs::write(&path_k, other_k.to_string()).unwrap_or_else(|error| panic!("{error}"));
    let out = instance.temp.path().join("other-k");
    fs::create_dir_all(&out).unwrap_or_else(|error| panic!("{error}"));
    let output = instance.eval_at(
        "fixture",
        &lines,
        &out,
        &["--modes", "dense", "--k", "3", "--baseline", path(&path_k)],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("20") && stderr.contains('3'),
        "the refusal names both `k`: {stderr}"
    );
}

#[test]
fn a_baseline_detects_the_rescore_candidates_20_regression() {
    let _serial = common::SerialGuard::acquire();
    let instance = Instance::new(&fidelity_fixture());
    let lines = fidelity_lines();
    let (reference, _) = eval_ok(&instance, &lines, &["--modes", "dense"]);

    let out = instance.temp.path().join("regressed");
    fs::create_dir_all(&out).unwrap_or_else(|error| panic!("{error}"));
    let output = instance.eval_at(
        "fixture",
        &lines,
        &out,
        &[
            "--modes",
            "dense",
            "--rescore-candidates",
            "20",
            "--baseline",
            path(&reference),
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "a mean below the reference by more than 0,005 exits 1: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let (json_path, _) = reports(&output);
    let report = read_report(&json_path);
    assert_eq!(report["baseline"]["regression"], true);
    assert_eq!(report["rescore_candidates"], 20);
    assert_eq!(
        report["rescore_candidates_source"], "--rescore-candidates",
        "the report says the campaign overrode N"
    );
    assert!(
        report["command"]
            .as_str()
            .unwrap_or_default()
            .contains("--rescore-candidates 20"),
        "the command line carries the override: {report}"
    );
    let lost = report["baseline"]["modes"]["dense"]["lost"]
        .as_array()
        .unwrap_or_else(|| panic!("the baseline names the lost queries: {report}"));
    assert_eq!(
        lost,
        &vec![json!("q1")],
        "q1 loses `W0`, which the reference found"
    );
    assert!(
        number(&report, "/baseline/modes/dense/fidelity/recall_at_10/delta") < -0.005,
        "the fidelity delta is the regression: {report}"
    );
}

#[test]
fn an_identical_replay_regresses_nothing() {
    let _serial = common::SerialGuard::acquire();
    let instance = Instance::new(&fidelity_fixture());
    let lines = fidelity_lines();
    let (reference, _) = eval_ok(&instance, &lines, &["--modes", "dense"]);

    let out = instance.temp.path().join("replay");
    fs::create_dir_all(&out).unwrap_or_else(|error| panic!("{error}"));
    let output = instance.eval_at(
        "fixture",
        &lines,
        &out,
        &["--modes", "dense", "--baseline", path(&reference)],
    );
    assert!(
        output.status.success(),
        "a replay exits 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let (json_path, _) = reports(&output);
    let report = read_report(&json_path);
    assert_eq!(report["baseline"]["regression"], false);
}

// --- Read-only -------------------------------------------------------------

/// Every path under `root`, with its size, relative to `root`.
fn inventory(root: &Path) -> BTreeSet<String> {
    let mut entries = BTreeSet::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let listing = match fs::read_dir(&directory) {
            Ok(listing) => listing,
            Err(error) => panic!("{} must be readable: {error}", directory.display()),
        };
        for entry in listing {
            let entry = entry.unwrap_or_else(|error| panic!("{error}"));
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            if path.is_dir() {
                entries.insert(format!("{relative}/"));
                pending.push(path);
            } else {
                let size = entry.metadata().map(|meta| meta.len()).unwrap_or_default();
                entries.insert(format!("{relative} {size}"));
            }
        }
    }
    entries
}

#[test]
fn the_command_writes_nothing_under_the_data_directory() {
    let _serial = common::SerialGuard::acquire();
    let instance = Instance::new(&fidelity_fixture());
    let before = inventory(&instance.data);
    let (json_path, markdown_path) = eval_ok(&instance, &fidelity_lines(), &["--modes", "dense"]);
    let after = inventory(&instance.data);

    // ADR-0116 makes an exception of the two files SQLite manages in WAL mode.
    let managed = |entry: &str| entry.ends_with("-wal") || entry.ends_with("-shm");
    let added: Vec<&String> = after
        .iter()
        .filter(|entry| !managed(entry))
        .filter(|entry| !before.contains(*entry))
        .collect();
    assert!(
        added.is_empty(),
        "`crible eval` writes nothing under the data root: {added:?}"
    );
    let removed: Vec<&String> = before
        .iter()
        .filter(|entry| !managed(entry))
        .filter(|entry| !after.contains(*entry))
        .collect();
    assert!(
        removed.is_empty(),
        "`crible eval` removes nothing under the data root: {removed:?}"
    );
    assert!(
        json_path.is_file() && markdown_path.is_file(),
        "the two reports are written into `--out`, outside the data root"
    );
}

// --- `synth-small`, the CI dataset (decision 14) ---------------------------

/// External model of `synth-small`, as the dataset declares it.
const SYNTH_MODEL_JSON: &str =
    r#"{ "name": "synthetic-external", "kind": "external", "dims": 768, "normalize": true }"#;

/// Number of queries of `synth-small` the regression test replays.
const SYNTH_QUERIES: usize = 50;

/// Builds `synth-small` in a temporary instance and returns it with its queries.
///
/// The configuration is the one of the dataset, with `scan: "binary"`: the
/// dataset's own `index.json` says `none`, which removes the choice of the
/// strategy (ADR-0074), and neither the fidelity nor the injected regression
/// could then be observed. Both runs of the test share this index, so the
/// regression stays injected by the command line alone (ADR-0125, decision 8).
fn synth_instance() -> (Instance, PathBuf) {
    synth_instance_with(SYNTH_QUERIES)
}

/// Builds `synth-small` and replays its first `count` queries.
///
/// The determinism test of ADR-0155, decision 9, replays a handful of them: the
/// fixture and its vectors are the ones of the CI dataset either way.
fn synth_instance_with(count: usize) -> (Instance, PathBuf) {
    let dataset = synth_root();
    require_synth(&dataset);

    let instance = Instance::with_data(&fixture("synth-small", &[]));
    let models = instance.temp.path().join("models");
    fs::create_dir_all(models.join("synthetic-external")).unwrap_or_else(|error| panic!("{error}"));
    fs::write(
        models.join("synthetic-external").join("model.json"),
        SYNTH_MODEL_JSON,
    )
    .unwrap_or_else(|error| panic!("{error}"));

    let mut configuration: Value = serde_json::from_str(
        &fs::read_to_string(dataset.join("index.json")).unwrap_or_else(|error| panic!("{error}")),
    )
    .unwrap_or_else(|error| panic!("{error}"));
    configuration["search"]["scan"] = json!("binary");
    let config = instance.temp.path().join("index.json");
    fs::write(&config, configuration.to_string()).unwrap_or_else(|error| panic!("{error}"));

    let created = crible(&[
        "index",
        "create",
        "--data",
        path(&instance.data),
        "--config",
        path(&config),
    ]);
    assert!(
        created.status.success(),
        "index create failed: {}",
        String::from_utf8_lossy(&created.stderr)
    );
    for batch in ["l1", "l2", "l3"] {
        let attached = crible(&[
            "attach",
            "--data",
            path(&instance.data),
            "--models",
            path(&models),
            "--index",
            "synth-small",
            path(&dataset.join(batch)),
        ]);
        assert!(
            attached.status.success(),
            "attach {batch} failed: {}",
            String::from_utf8_lossy(&attached.stderr)
        );
    }

    // The queries keep their vectors and take the first expected document of
    // the exact reference as their `expected`.
    let queries = common::read_json_lines(&dataset.join("queries.jsonl"));
    let expected = common::read_json_lines(&dataset.join("expected.jsonl"));
    let lines: Vec<Value> = queries
        .iter()
        .take(count)
        .zip(expected.iter())
        .map(|(row, reference)| {
            json!({
                "id": format!("q-{:03}", row["query_id"]),
                "vector": row["vector"],
                "expected": [reference["results"][0]["doc_id"]],
            })
        })
        .collect();
    let queries = instance.queries("synth-queries.jsonl", &lines);
    (instance, queries)
}

#[test]
fn synth_small_regression_is_detected_by_the_baseline() {
    let _serial = common::SerialGuard::acquire();
    let (instance, queries) = synth_instance();
    let reference = instance.eval_file_at(
        "synth-small",
        &queries,
        &instance.out,
        &["--modes", "dense"],
    );
    assert!(
        reference.status.success(),
        "the reference run failed: {}",
        String::from_utf8_lossy(&reference.stderr)
    );
    let (reference, _) = reports(&reference);
    let report = read_report(&reference);
    assert!(
        number(&report, "/modes/dense/fidelity/recall_at_10") > 0.99,
        "the default N of the index is faithful on the fixture: {report}"
    );

    let out = instance.temp.path().join("regressed");
    fs::create_dir_all(&out).unwrap_or_else(|error| panic!("{error}"));
    let output = instance.eval_file_at(
        "synth-small",
        &queries,
        &out,
        &[
            "--modes",
            "dense",
            "--rescore-candidates",
            "20",
            "--baseline",
            path(&reference),
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "`--rescore-candidates 20` with `k = 10` regresses and exits 1: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let (json_path, _) = reports(&output);
    let report = read_report(&json_path);
    assert_eq!(report["baseline"]["regression"], true);
    assert!(
        !report["baseline"]["modes"]["dense"]["lost"]
            .as_array()
            .unwrap_or_else(|| panic!("the lost queries are reported: {report}"))
            .is_empty(),
        "at least one query loses an expected document: {report}"
    );
}

#[test]
fn the_help_states_when_models_is_needed() {
    let _serial = common::SerialGuard::acquire();
    // ADR-0125, decision 2: `--models` is required only when a `dense` or
    // `hybrid` mode has to encode a textual query, never for a `lexical` run
    // (`@relecteur`, NETTOYER-P7).
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_crible"))
        .args(["eval", "--help"])
        .output()
        .expect("crible eval --help must run");
    assert!(output.status.success(), "the help of eval must succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // The help wraps its text at the terminal width, so the assertion runs on
    // the words: a fragment copied from the `--models` paragraph proves that
    // the sentence is the one of ADR-0125, decision 2, not the vocabulary of
    // `--modes` (`dense`, `lexical`, `hybrid`), which the help already held.
    let help = stdout.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        help.contains("never for a `lexical` one"),
        "the `--models` help says a lexical run needs no encoder, got {stdout}"
    );
}

// --- Top-k rejouable (ADR-0155) --------------------------------------------
//
// ADR-0155 replaces the `format_version: 2` report with a version 3 that keeps
// the ordered top-k of the grouped run (`doc_ids`), attributes an evolution
// (`model_fingerprint`, `query_precision`, `queries_fingerprint`) and compares
// the top-k of two campaigns without changing the exit code.
//
// The member names of the comparison are written down here because the ADR
// fixes their meaning, not their spelling: `baseline.modes.<mode>.top_k` holds
// `overlap_at_k`, `n`, `changed` and `excluded`, and `baseline.attribution`
// holds the three mentions of decision 6. A version 1 or 2 reference carries
// `top_k_not_compared` instead, on the model of `latency_not_compared`.

/// A well-formed blake3 digest, for the declared files of the ONNX fixture.
const FP32_BLAKE3: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A second well-formed digest, for the tokenizer of the ONNX fixture.
const TOKENIZER_BLAKE3: &str = "aabbccddaabbccddaabbccddaabbccddaabbccddaabbccddaabbccddaabbccdd";

/// Model configuration of kind `onnx`, as `docs/04` writes it.
///
/// A campaign of `vector` queries never builds the encoder, so this
/// configuration needs no ONNX Runtime and no model file: only `model.json` is
/// read, for its fingerprint (ADR-0155, decision 3).
fn onnx_model(name: &str, dims: u32) -> Value {
    json!({
        "name": name,
        "kind": "onnx",
        "hf_id": "crible/fixture",
        "revision": "fixture",
        "dims": dims,
        "pooling": "cls",
        "normalize": true,
        "max_tokens": 32,
        "prompts": { "query": "query: ", "document": "document: " },
        "files": {
            "fp32": { "path": "model.onnx", "blake3": FP32_BLAKE3 },
            "tokenizer": { "path": "tokenizer.json", "blake3": TOKENIZER_BLAKE3 },
        },
    })
}

/// Model configuration of kind `external`: no encoder and no fingerprint.
fn external_model(name: &str, dims: u32) -> Value {
    json!({ "name": name, "kind": "external", "dims": dims, "normalize": true })
}

/// Writes `<models>/<name>/model.json` under the temporary root and returns the
/// models root, as `--models` receives it.
fn model_root(instance: &Instance, name: &str, config: &Value) -> PathBuf {
    let root = instance.temp.path().join("models");
    let directory = root.join(name);
    fs::create_dir_all(&directory)
        .unwrap_or_else(|error| panic!("{} must be creatable: {error}", directory.display()));
    fs::write(directory.join("model.json"), config.to_string())
        .unwrap_or_else(|error| panic!("the model configuration must be written: {error}"));
    root
}

/// Creates and returns a report directory under the temporary root.
fn out_dir(instance: &Instance, name: &str) -> PathBuf {
    let out = instance.temp.path().join(name);
    fs::create_dir_all(&out)
        .unwrap_or_else(|error| panic!("{} must be creatable: {error}", out.display()));
    out
}

/// Writes a derived JSON report, as a reference of `--baseline`.
fn write_report(path: &Path, report: &Value) {
    fs::write(path, format!("{report}\n"))
        .unwrap_or_else(|error| panic!("{} must be writable: {error}", path.display()));
}

/// Asserts `value` is a bare, lowercase, 64-character blake3 digest (ADR-0028).
fn assert_fingerprint(value: &Value, label: &str) -> String {
    let text = value
        .as_str()
        .unwrap_or_else(|| panic!("{label} must be a bare hexadecimal string: {value}"));
    assert_eq!(
        text.len(),
        64,
        "{label} is the blake3 of the canonical JSON, so 64 characters: {text}"
    );
    assert!(
        text.chars()
            .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character)),
        "{label} is lowercase hexadecimal, with no prefix: {text}"
    );
    text.to_owned()
}

/// The three queries of the top-k baseline tests, in file order.
///
/// `q1` and `q3` share the same answer on the fixture: `A`, `B`, `C`. `q2`
/// filters on an absent value, so its measured top-k is empty and its reference
/// is empty too: it is the query decision 5 excludes from the mean.
fn top_k_lines() -> Vec<Value> {
    vec![
        query("q1", &[1.0, 0.0, 0.0, 0.0], &["A"]),
        json!({
            "id": "q2",
            "vector": [1.0, 0.0, 0.0, 0.0],
            "filter": { "fonds": { "eq": "ABSENT" } },
        }),
        query("q3", &[1.0, 0.0, 0.0, 0.0], &["C"]),
    ]
}

/// The quality fixture with an `onnx` model configuration, so the report can
/// carry a real `model_fingerprint` without an encoder (vector queries only).
fn top_k_setup() -> (Instance, String) {
    let instance = Instance::new(&quality_fixture());
    let root = model_root(&instance, "fixture", &onnx_model("fixture", DIMS));
    (instance, path(&root).to_owned())
}

#[test]
fn inv_13_eval_local_report_never_writes_query_text_or_filter_value() {
    let _serial = common::SerialGuard::acquire();
    // INV-13 and ADR-0155, decision 8: a version 3 report writes external
    // `doc_id`s of results, and neither a query text, nor a filter value, nor
    // an internal id or a `doc_id#ord` key. The marked passage makes the
    // query text match, so the report really holds a non-empty `doc_ids`.
    let mut index = lexical_fixture();
    index.passages[0].text = format!("{QUERY_MARKER} corps de l'article");
    let instance = Instance::new(&index);
    let line = json!({
        "id": "q-marqueur",
        "query": QUERY_MARKER,
        "filter": { "marqueur": FILTER_MARKER },
        "expected": ["A"],
    });
    let (json_path, markdown_path) = eval_ok(&instance, &[line], &["--modes", "lexical"]);
    let report = read_report(&json_path);
    assert_eq!(
        report["format_version"], 3,
        "the replayable top-k is a version 3 report (ADR-0155, decision 1): {report}"
    );
    let rows = report["modes"]["lexical"]["queries"]
        .as_array()
        .unwrap_or_else(|| panic!("the mode holds its per-query rows: {report}"));
    assert_eq!(rows.len(), 1);
    assert_eq!(
        &rows[0]["doc_ids"],
        &json!(["A"]),
        "the marked passage is found, and its external `doc_id` is written: {report}"
    );

    let json_body = fs::read_to_string(&json_path).unwrap_or_else(|error| panic!("{error}"));
    let markdown = fs::read_to_string(&markdown_path)
        .unwrap_or_else(|error| panic!("the Markdown report must exist: {error}"));
    assert!(
        json_body.contains("\"doc_ids\""),
        "the report really carries the member under test: {json_body}"
    );
    assert!(
        !json_body.contains(QUERY_MARKER),
        "the query text never reaches the JSON report (INV-13)"
    );
    assert!(
        !markdown.contains(QUERY_MARKER),
        "the query text never reaches the Markdown report (INV-13)"
    );
    assert!(
        !json_body.contains(FILTER_MARKER) && !markdown.contains(FILTER_MARKER),
        "the filter value never reaches a report (INV-13)"
    );
    assert!(
        !json_body.contains('#'),
        "no `doc_id#ord` key nor passage key reaches the JSON report: {json_body}"
    );
}

#[test]
fn report_v3_rows_carry_ordered_doc_ids_in_every_mode() {
    let _serial = common::SerialGuard::acquire();
    // ADR-0155, decision 2: every row of every mode carries `doc_ids`, the
    // ordered external `doc_id`s of the grouped run, `[]` when the run rendered
    // nothing, and it stays when the query has no `expected`.
    let instance = Instance::new(&quality_fixture());
    let lines = [
        query("q1", &[1.0, 0.0, 0.0, 0.0], &["A"]),
        json!({
            "id": "q2",
            "vector": [1.0, 0.0, 0.0, 0.0],
            "filter": { "fonds": { "eq": "ABSENT" } },
        }),
    ];
    let (json_path, _) = eval_ok(&instance, &lines, &["--modes", "dense", "--k", "3"]);
    let report = read_report(&json_path);
    assert_eq!(report["format_version"], 3);
    let rows = report["modes"]["dense"]["queries"]
        .as_array()
        .unwrap_or_else(|| panic!("the mode holds its per-query rows: {report}"));
    assert_eq!(
        &rows[0]["doc_ids"],
        &json!(["A", "B", "C"]),
        "the top-3 of `[1, 0, 0, 0]` is A, B, C, best first: {report}"
    );
    assert_eq!(
        &rows[1]["doc_ids"],
        &json!([]),
        "an empty pool is measured, and its top-k is `[]`, not an absent member: {report}"
    );
    for row in rows {
        let doc_ids = row["doc_ids"]
            .as_array()
            .unwrap_or_else(|| panic!("each row carries `doc_ids`: {row}"));
        for doc_id in doc_ids {
            let doc_id = doc_id
                .as_str()
                .unwrap_or_else(|| panic!("a `doc_id` is a string: {row}"));
            assert!(
                !doc_id.contains('#'),
                "the list holds external `doc_id`s, not `doc_id#ord` keys: {row}"
            );
        }
    }

    // The lexical mode writes the member too, and a query without `expected`
    // keeps its top-k: the ranking is a measurement of its own.
    let instance = Instance::new(&lexical_fixture());
    let (json_path, _) = eval_ok(
        &instance,
        &[json!({ "id": "q-text", "query": "passage" })],
        &["--modes", "lexical"],
    );
    let report = read_report(&json_path);
    assert_eq!(report["format_version"], 3);
    let row = &report["modes"]["lexical"]["queries"][0];
    let doc_ids = row["doc_ids"]
        .as_array()
        .unwrap_or_else(|| panic!("a lexical row carries `doc_ids`: {report}"));
    assert!(
        !doc_ids.is_empty(),
        "the query matches the three passages, so its measured top-k is not empty: {report}"
    );
    assert!(
        doc_ids.len() <= 10,
        "the list holds at most `k` documents: {report}"
    );
}

#[test]
fn report_v3_header_carries_the_model_fingerprint_of_an_onnx_model() {
    let _serial = common::SerialGuard::acquire();
    // ADR-0155, decision 3: `model_fingerprint` is the bare blake3 of the model
    // configuration, and `query_precision` is absent because a campaign of
    // `vector` queries encoded no text.
    let (instance, models) = top_k_setup();
    let lines = [query("q1", &[1.0, 0.0, 0.0, 0.0], &["A"])];
    let (json_path, _) = eval_ok(
        &instance,
        &lines,
        &["--models", models.as_str(), "--modes", "dense"],
    );
    let report = read_report(&json_path);
    assert_eq!(report["format_version"], 3);
    let expected = ModelConfig::from_json_str(&onnx_model("fixture", DIMS).to_string())
        .unwrap_or_else(|error| panic!("the fixture model must parse: {error}"))
        .fingerprint();
    assert_eq!(
        assert_fingerprint(&report["model_fingerprint"], "model_fingerprint"),
        expected,
        "the report carries `ModelConfig::fingerprint`, lowercase hexadecimal: {report}"
    );
    assert!(
        report.get("query_precision").is_none(),
        "a campaign of `vector` queries encodes no text, so it writes no precision: {report}"
    );
    assert_fingerprint(&report["queries_fingerprint"], "queries_fingerprint");
}

#[test]
fn report_v3_header_omits_model_fingerprint_for_an_external_model() {
    let _serial = common::SerialGuard::acquire();
    // ADR-0155, decision 3: an `external` model carries neither files nor
    // revision, so its fingerprint would say nothing about the encoder, and the
    // report omits it.
    let instance = Instance::new(&quality_fixture());
    let root = model_root(&instance, "fixture", &external_model("fixture", DIMS));
    let models = path(&root).to_owned();
    let lines = [query("q1", &[1.0, 0.0, 0.0, 0.0], &["A"])];
    let (json_path, _) = eval_ok(
        &instance,
        &lines,
        &["--models", models.as_str(), "--modes", "dense"],
    );
    let report = read_report(&json_path);
    assert_eq!(report["format_version"], 3);
    assert!(
        report.get("model_fingerprint").is_none(),
        "an `external` model has no fingerprint worth writing: {report}"
    );
    assert!(
        report.get("query_precision").is_none(),
        "a campaign of `vector` queries encodes no text: {report}"
    );
    assert_fingerprint(&report["queries_fingerprint"], "queries_fingerprint");
}

#[test]
fn report_v3_header_omits_query_precision_when_no_dense_text_is_encoded() {
    let _serial = common::SerialGuard::acquire();
    // ADR-0155, decision 3: `query_precision` is written only when the campaign
    // encoded a dense text. A lexical campaign encodes nothing, even with a
    // textual query: writing `"fp32"` there would be false.
    let instance = Instance::new(&lexical_fixture());
    let (json_path, _) = eval_ok(
        &instance,
        &[json!({ "id": "q-text", "query": "passage" })],
        &["--modes", "lexical"],
    );
    let report = read_report(&json_path);
    assert_eq!(report["format_version"], 3);
    assert!(
        report.get("query_precision").is_none(),
        "a lexical campaign encodes no dense text: {report}"
    );
    assert_fingerprint(&report["queries_fingerprint"], "queries_fingerprint");
}

#[test]
fn a_lexical_campaign_reads_no_encoder_configuration_even_with_models() {
    let _serial = common::SerialGuard::acquire();
    // ADR-0143 and ADR-0155, decision 3: a purely lexical campaign reads no
    // encoder configuration, so `--models` keeps having no effect there. The
    // models root does not hold the model of the index, and the campaign
    // succeeds without writing `model_fingerprint`.
    let instance = Instance::new(&lexical_fixture());
    let models = path(&instance.temp.path().join("empty-models")).to_owned();
    let lines = [json!({ "id": "q-text", "query": "passage" })];
    let (json_path, _) = eval_ok(
        &instance,
        &lines,
        &["--models", models.as_str(), "--modes", "lexical"],
    );
    let report = read_report(&json_path);
    assert_eq!(report["format_version"], 3);
    assert!(
        report.get("model_fingerprint").is_none(),
        "a campaign without a dense branch reads no encoder configuration: {report}"
    );
    assert!(
        report.get("query_precision").is_none(),
        "a lexical campaign encodes no dense text: {report}"
    );
    assert_fingerprint(&report["queries_fingerprint"], "queries_fingerprint");
}

#[test]
#[ignore = "needs ONNX Runtime (ORT_DYLIB_PATH) for the query encoding of a text; run: ORT_DYLIB_PATH=/path/to/libonnxruntime.dylib cargo test -p crible --test eval_cli -- --ignored"]
fn query_precision_is_fp32_when_a_dense_text_is_encoded() {
    let _serial = common::SerialGuard::acquire();
    // ADR-0155, decision 3 and ADR-0052: a dense text is encoded in fp32, and
    // the report says so. The fake encoder of `crible-encode` produces the
    // vectors, so this test needs ONNX Runtime and is `#[ignore]`d like the
    // other ignored tests of the crate (ADR-0057).
    let (instance, models) = onnx_text_instance();
    let lines = [json!({
        "id": "q-texte",
        "query": "responsabilité du fait des choses",
        "expected": ["A"],
    })];
    let (json_path, _) = eval_ok(
        &instance,
        &lines,
        &["--models", models.as_str(), "--modes", "dense"],
    );
    let report = read_report(&json_path);
    assert_eq!(
        report["query_precision"], "fp32",
        "the campaign encoded a dense text, in fp32 (ADR-0052): {report}"
    );
    assert_fingerprint(&report["model_fingerprint"], "model_fingerprint");
}

/// Builds an index served by the fake ONNX model of `crible-encode`.
///
/// The graph is the versioned fixture of the crate: no data of production is
/// read, and the model directory is a copy made inside the temporary root of the
/// test (ADR-0125, decision 14).
fn onnx_text_instance() -> (Instance, String) {
    let mut index = fixture("fixture", &[]);
    index.configuration = configuration("fixture", false)
        .replace("\"model\":\"fixture\"", "\"model\":\"fake-model\"");
    let instance = Instance::with_data(&index);
    let config = instance.temp.path().join("index.json");
    fs::write(&config, &index.configuration)
        .unwrap_or_else(|error| panic!("the configuration must be written: {error}"));
    let data = path(&instance.data);
    let config = path(&config);
    let created = crible(&["index", "create", "--data", data, "--config", config]);
    assert!(
        created.status.success(),
        "index create failed: {}",
        String::from_utf8_lossy(&created.stderr)
    );

    let directory = instance.data.join("indexes").join("fixture");
    // The store opens before the segment is written: the recovery of a write
    // open drops a segment directory SQLite does not reference yet
    // (`GenerationStore::open`, ADR-0116, decision 1), which is the order of
    // `Instance::new`.
    let mut store = GenerationStore::open(&directory, &schema())
        .unwrap_or_else(|error| panic!("the fixture index must open: {error}"));
    let mut first = vec![0.0_f32; 768];
    first[0] = 1.0;
    let mut second = vec![0.0_f32; 768];
    second[0] = 0.9;
    second[1] = 0.1;
    let segment = write_segment_dims(&directory, &[first, second], 768);
    let doc_ids = ["A", "B"];
    let documents = doc_ids
        .iter()
        .enumerate()
        .map(|(position, doc_id)| DocumentRecord {
            doc_id: (*doc_id).to_owned(),
            doc_hash: format!("blake3:doc-{position}"),
            fields: json!({ "fonds": "LEGI", "marqueur": FILTER_MARKER }),
            updated_generation: Generation::new(1),
        })
        .collect();
    let passages = doc_ids
        .iter()
        .enumerate()
        .map(|(position, doc_id)| PassageRecord {
            iid: Iid::new(u32::try_from(position).unwrap_or_default()),
            doc_id: (*doc_id).to_owned(),
            ord: 0,
            text: format!("passage {position}"),
            passage_hash: format!("blake3:passage-{position}"),
            fields: json!({ "partie": "corps" }),
            segment_id: "seg-1".to_owned(),
            created_generation: Generation::new(1),
            dead_since_gen: None,
        })
        .collect();
    store
        .commit(GenerationChange {
            segments: vec![segment],
            documents,
            passages,
            deletions: Vec::new(),
        })
        .unwrap_or_else(|error| panic!("the fixture generation must commit: {error}"));

    let models = instance.temp.path().join("models");
    let model_directory = models.join("fake-model");
    fs::create_dir_all(&model_directory)
        .unwrap_or_else(|error| panic!("the model directory must be creatable: {error}"));
    for file in [
        "model.json",
        "model.onnx",
        "tokenizer.json",
        "model.int8.onnx",
    ] {
        fs::copy(
            common::fixture_directory().join(file),
            model_directory.join(file),
        )
        .unwrap_or_else(|error| panic!("the fixture model file {file} must be copied: {error}"));
    }
    (instance, path(&models).to_owned())
}

#[test]
fn queries_fingerprint_ignores_the_formatting_of_the_game_file() {
    let _serial = common::SerialGuard::acquire();
    // ADR-0155, decision 3: the hash is the blake3 of the canonical parsed game,
    // not of the bytes. The same game, formatted differently, keeps its
    // fingerprint; a changed query changes it.
    let instance = Instance::new(&quality_fixture());
    let compact = concat!(
        "{\"id\":\"q1\",\"vector\":[1.0,0.0,0.0,0.0],\"filter\":{\"fonds\":\"LEGI\"},",
        "\"expected\":[\"A\"]}\n",
        "{\"id\":\"q2\",\"vector\":[0.0,1.0,0.0,0.0]}\n",
    );
    let pretty = concat!(
        "{ \"expected\" : [ \"A\" ] , \"filter\" : { \"fonds\" : \"LEGI\" } , ",
        "\"vector\" : [ 1.0 , 0.0 , 0.0 , 0.0 ] , \"id\" : \"q1\" }\n",
        "{ \"vector\" : [ 0.0 , 1.0 , 0.0 , 0.0 ] , \"id\" : \"q2\" }\n",
    );
    let changed = concat!(
        "{\"id\":\"q1\",\"vector\":[1.0,0.0,0.0,0.0],\"filter\":{\"fonds\":\"LEGI\"},",
        "\"expected\":[\"A\"]}\n",
        "{\"id\":\"q2\",\"vector\":[0.0,0.0,1.0,0.0]}\n",
    );

    let mut fingerprints = Vec::new();
    for (name, body) in [
        ("game-compact", compact),
        ("game-pretty", pretty),
        ("game-changed", changed),
    ] {
        let game = instance.temp.path().join(format!("{name}.jsonl"));
        fs::write(&game, body).unwrap_or_else(|error| panic!("the game must be written: {error}"));
        let out = out_dir(&instance, &format!("out-{name}"));
        let output = instance.eval_file_at("fixture", &game, &out, &["--modes", "dense"]);
        assert!(
            output.status.success(),
            "eval {name} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let (json_path, _) = reports(&output);
        let report = read_report(&json_path);
        assert_eq!(report["format_version"], 3);
        fingerprints.push(assert_fingerprint(
            &report["queries_fingerprint"],
            "queries_fingerprint",
        ));
    }
    assert_eq!(
        fingerprints[0], fingerprints[1],
        "spaces and key order do not change the parsed game, so the fingerprint is the same"
    );
    assert_ne!(
        fingerprints[1], fingerprints[2],
        "a changed vector changes the parsed game, so the fingerprint changes"
    );
}

#[test]
fn synth_small_two_local_runs_render_the_same_doc_ids() {
    let _serial = common::SerialGuard::acquire();
    // ADR-0155, decision 9: at the same generation and the same game, the
    // grouped top-k is deterministic. The test proves determinism, never a
    // recall target.
    let (instance, queries) = synth_instance_with(10);
    let first_out = out_dir(&instance, "out-first");
    let first = instance.eval_file_at("synth-small", &queries, &first_out, &["--modes", "dense"]);
    assert!(
        first.status.success(),
        "the first campaign failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let (first_json, _) = reports(&first);
    let second_out = out_dir(&instance, "out-second");
    let second = instance.eval_file_at("synth-small", &queries, &second_out, &["--modes", "dense"]);
    assert!(
        second.status.success(),
        "the second campaign failed: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let (second_json, _) = reports(&second);

    let doc_ids = |report: &Value| -> Vec<Vec<String>> {
        report["modes"]["dense"]["queries"]
            .as_array()
            .unwrap_or_else(|| panic!("the mode holds its per-query rows: {report}"))
            .iter()
            .map(|row| {
                row["doc_ids"]
                    .as_array()
                    .unwrap_or_else(|| panic!("each row carries `doc_ids`: {row}"))
                    .iter()
                    .map(|doc_id| {
                        doc_id
                            .as_str()
                            .unwrap_or_else(|| panic!("a `doc_id` is a string: {row}"))
                            .to_owned()
                    })
                    .collect()
            })
            .collect()
    };
    let first_report = read_report(&first_json);
    let second_report = read_report(&second_json);
    let first_ids = doc_ids(&first_report);
    let second_ids = doc_ids(&second_report);
    assert_eq!(
        first_ids, second_ids,
        "two campaigns on the same generation render the same ordered top-k"
    );
    assert!(
        first_ids.iter().any(|ids| !ids.is_empty()),
        "at least one query rendered a document, so the equality is not vacuous"
    );
    assert_eq!(
        first_report["queries_fingerprint"], second_report["queries_fingerprint"],
        "the same game keeps the same fingerprint"
    );
    assert_eq!(first_report["generation"], second_report["generation"]);
}

#[test]
fn a_baseline_compares_top_k_without_changing_the_exit_code() {
    let _serial = common::SerialGuard::acquire();
    // ADR-0155, decisions 5 and 7: `overlap_at_k` per query and its mean with
    // its `n`, an empty reference excluded and counted apart, the `id`s whose
    // ordered list changed, and the Markdown section. The comparison is
    // informative: only the means of quality and fidelity can exit 1.
    let (instance, models) = top_k_setup();
    let lines = top_k_lines();
    let (reference_json, _) = eval_ok(
        &instance,
        &lines,
        &["--models", models.as_str(), "--modes", "dense", "--k", "3"],
    );
    let mut reference = read_report(&reference_json);
    let rows = reference["modes"]["dense"]["queries"]
        .as_array_mut()
        .unwrap_or_else(|| panic!("the reference holds its per-query rows"));
    for row in rows {
        match row["id"].as_str().unwrap_or_default() {
            // The same three documents in another order: an exchange of ranks
            // counts, and the overlap stays 1.
            "q1" => row["doc_ids"] = json!(["B", "A", "C"]),
            // One document in common with the current `A`, `B`, `C`.
            "q3" => row["doc_ids"] = json!(["D", "E", "B"]),
            _ => {}
        }
    }
    let reference_path = instance.temp.path().join("reference-top-k.json");
    write_report(&reference_path, &reference);
    let baseline = path(&reference_path).to_owned();

    let out = out_dir(&instance, "out-top-k");
    let output = instance.eval_at(
        "fixture",
        &lines,
        &out,
        &[
            "--models",
            models.as_str(),
            "--modes",
            "dense",
            "--k",
            "3",
            "--baseline",
            baseline.as_str(),
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "a top-k change alone never exits 1: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let (json_path, markdown_path) = reports(&output);
    let report = read_report(&json_path);
    assert_eq!(report["baseline"]["regression"], false);
    assert_close(
        number(&report, "/baseline/modes/dense/top_k/overlap_at_k"),
        (1.0 + 1.0 / 3.0) / 2.0,
        "mean overlap_at_k: q1 keeps its three documents, q3 keeps one of three",
    );
    assert_eq!(
        report["baseline"]["modes"]["dense"]["top_k"]["n"], 2,
        "q2 is excluded: its reference list is empty: {report}"
    );
    assert_eq!(
        &report["baseline"]["modes"]["dense"]["top_k"]["changed"],
        &json!(["q1", "q3"]),
        "a rank exchange and a lost document both count: {report}"
    );
    assert_eq!(
        &report["baseline"]["modes"]["dense"]["top_k"]["excluded"],
        &json!(["q2"]),
        "an empty reference is counted apart with its id: {report}"
    );
    let attribution = &report["baseline"]["attribution"];
    assert_eq!(
        attribution["generation"]["reference"], report["generation"],
        "the same generation is mentioned, not refused: {report}"
    );
    assert_eq!(attribution["generation"]["current"], report["generation"]);
    assert_eq!(attribution["model_fingerprint"], "same");
    assert_eq!(attribution["queries_fingerprint"], "same");

    let markdown = fs::read_to_string(&markdown_path)
        .unwrap_or_else(|error| panic!("the Markdown report must exist: {error}"));
    let lowered = markdown.to_lowercase();
    assert!(
        lowered.contains("top-k") && lowered.contains("recouvrement"),
        "the Markdown carries the top-k overlap section (decision 7): {markdown}"
    );
    assert!(
        lowered.contains("informative"),
        "the section says the comparison is informative: {markdown}"
    );
    assert!(
        markdown.contains("`q1`") && markdown.contains("`q3`"),
        "the section names the changed queries: {markdown}"
    );
}

#[test]
fn a_query_present_in_one_report_only_is_counted_apart() {
    let _serial = common::SerialGuard::acquire();
    // ADR-0155, decision 5: a query present in one report only is counted
    // apart, whichever report it comes from. The reference loses `q3` and gains
    // `q4`, so the current `q3` is absent from the reference and the reference
    // `q4` is absent from the current campaign.
    let (instance, models) = top_k_setup();
    let lines = top_k_lines();
    let (reference_json, _) = eval_ok(
        &instance,
        &lines,
        &["--models", models.as_str(), "--modes", "dense", "--k", "3"],
    );
    let mut reference = read_report(&reference_json);
    let rows = reference["modes"]["dense"]["queries"]
        .as_array_mut()
        .unwrap_or_else(|| panic!("the reference holds its per-query rows"));
    rows.retain(|row| row["id"].as_str() != Some("q3"));
    rows.push(json!({
        "id": "q4",
        "recall_at_k": 1.0,
        "mrr_at_k": 1.0,
        "doc_ids": ["A"],
    }));
    let reference_path = instance.temp.path().join("reference-one-sided.json");
    write_report(&reference_path, &reference);
    let baseline = path(&reference_path).to_owned();

    let out = out_dir(&instance, "out-one-sided");
    let output = instance.eval_at(
        "fixture",
        &lines,
        &out,
        &[
            "--models",
            models.as_str(),
            "--modes",
            "dense",
            "--k",
            "3",
            "--baseline",
            baseline.as_str(),
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "one-sided queries never refuse the reference: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let (json_path, markdown_path) = reports(&output);
    let report = read_report(&json_path);
    let top_k = &report["baseline"]["modes"]["dense"]["top_k"];
    let mut not_compared: Vec<&str> = top_k["not_compared"]
        .as_array()
        .unwrap_or_else(|| panic!("the top-k names the queries it did not compare: {report}"))
        .iter()
        .filter_map(Value::as_str)
        .collect();
    not_compared.sort_unstable();
    assert_eq!(
        not_compared,
        ["q3", "q4"],
        "a current-only query and a reference-only query are both counted apart: {report}"
    );
    assert_eq!(
        top_k["n"], 1,
        "only `q1` is compared, `q2` excluded and the one-sided queries counted apart: {report}"
    );
    let markdown = fs::read_to_string(&markdown_path)
        .unwrap_or_else(|error| panic!("the Markdown report must exist: {error}"));
    assert!(
        markdown.contains("`q3`") && markdown.contains("`q4`"),
        "the Markdown names both one-sided queries: {markdown}"
    );
}

#[test]
fn a_mode_absent_from_the_reference_is_said_not_compared_in_the_top_k_section() {
    let _serial = common::SerialGuard::acquire();
    // ADR-0131, decision 7 and ADR-0155, decision 7: a mode the reference does
    // not carry is not compared, and the top-k section says it on its own line
    // instead of disappearing with the mode.
    let (instance, models) = top_k_setup();
    let lines = top_k_lines();
    let (reference_json, _) = eval_ok(
        &instance,
        &lines,
        &["--models", models.as_str(), "--modes", "dense", "--k", "3"],
    );
    let mut reference = read_report(&reference_json);
    let removed = reference
        .get_mut("modes")
        .and_then(Value::as_object_mut)
        .and_then(|modes| modes.remove("dense"));
    assert!(
        removed.is_some(),
        "the reference holds its `dense` mode: {reference}"
    );
    let reference_path = instance.temp.path().join("reference-no-mode.json");
    write_report(&reference_path, &reference);
    let baseline = path(&reference_path).to_owned();

    let out = out_dir(&instance, "out-no-mode");
    let output = instance.eval_at(
        "fixture",
        &lines,
        &out,
        &[
            "--models",
            models.as_str(),
            "--modes",
            "dense",
            "--k",
            "3",
            "--baseline",
            baseline.as_str(),
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "a mode absent from the reference is not refused: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let (json_path, markdown_path) = reports(&output);
    let report = read_report(&json_path);
    assert_eq!(
        &report["baseline"]["modes_not_compared"],
        &json!(["dense"]),
        "the mode absent from the reference is named: {report}"
    );
    assert!(
        report["baseline"]["modes"].get("dense").is_none(),
        "the mode is not compared: {report}"
    );
    let markdown = fs::read_to_string(&markdown_path)
        .unwrap_or_else(|error| panic!("the Markdown report must exist: {error}"));
    assert!(
        markdown.contains("mode absent de la référence"),
        "the top-k section says the mode was not compared: {markdown}"
    );
}

#[test]
fn a_baseline_of_format_version_1_or_2_skips_the_top_k_comparison() {
    let _serial = common::SerialGuard::acquire();
    // ADR-0155, decision 5: a version 1 or 2 reference carries no `doc_ids`. The
    // means stay compared, the top-k is not, and the report says so instead of
    // leaving a section that would read as a zero.
    let (instance, models) = top_k_setup();
    let lines = top_k_lines();
    let (reference_json, _) = eval_ok(
        &instance,
        &lines,
        &["--models", models.as_str(), "--modes", "dense", "--k", "3"],
    );
    let reference = read_report(&reference_json);
    for version in [1_u64, 2] {
        let mut old = reference.clone();
        old["format_version"] = json!(version);
        let object = old
            .as_object_mut()
            .unwrap_or_else(|| panic!("the reference is a JSON object"));
        for member in [
            "model_fingerprint",
            "query_precision",
            "queries_fingerprint",
        ] {
            object.remove(member);
        }
        for row in old["modes"]["dense"]["queries"]
            .as_array_mut()
            .unwrap_or_else(|| panic!("the reference holds its per-query rows"))
        {
            row.as_object_mut()
                .unwrap_or_else(|| panic!("a per-query row is a JSON object"))
                .remove("doc_ids");
        }
        if version == 1 {
            old["modes"]["dense"]["latency"]
                .as_object_mut()
                .unwrap_or_else(|| panic!("the mode holds its latency object"))
                .remove("latency_basis");
        }
        let baseline_path = instance
            .temp
            .path()
            .join(format!("reference-v{version}.json"));
        write_report(&baseline_path, &old);
        let baseline = path(&baseline_path).to_owned();
        let out = out_dir(&instance, &format!("out-v{version}"));
        let output = instance.eval_at(
            "fixture",
            &lines,
            &out,
            &[
                "--models",
                models.as_str(),
                "--modes",
                "dense",
                "--k",
                "3",
                "--baseline",
                baseline.as_str(),
            ],
        );
        assert_eq!(
            output.status.code(),
            Some(0),
            "a version {version} reference is read: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let (json_path, markdown_path) = reports(&output);
        let report = read_report(&json_path);
        let entry = &report["baseline"]["modes"]["dense"];
        assert!(
            entry.pointer("/quality/recall_at_k/reference").is_some()
                && entry.pointer("/quality/recall_at_k/current").is_some(),
            "the means stay compared for a version {version} reference: {report}"
        );
        assert!(
            entry.get("top_k").is_none(),
            "a version {version} reference has no `doc_ids` to compare: {report}"
        );
        assert!(
            entry.get("top_k_not_compared").is_some(),
            "the report says the top-k was not compared: {report}"
        );
        assert_eq!(
            report["baseline"]["attribution"]["model_fingerprint"], "unknown",
            "a version {version} reference names no model fingerprint: {report}"
        );
        assert_eq!(
            report["baseline"]["attribution"]["queries_fingerprint"], "unknown",
            "a version {version} reference names no game fingerprint: {report}"
        );
        let markdown = fs::read_to_string(&markdown_path)
            .unwrap_or_else(|error| panic!("the Markdown report must exist: {error}"));
        let lowered = markdown.to_lowercase();
        assert!(
            lowered.contains("top-k") && lowered.contains("non compar"),
            "the Markdown says the top-k was not compared, for version {version}: {markdown}"
        );
    }
}

#[test]
fn a_baseline_of_format_version_4_is_refused_naming_it() {
    let _serial = common::SerialGuard::acquire();
    // ADR-0155, decision 1: the reader accepts 1, 2 and 3, and refuses any other
    // version by a message that names the version it read.
    let (instance, models) = top_k_setup();
    let lines = top_k_lines();
    let (reference_json, _) = eval_ok(
        &instance,
        &lines,
        &["--models", models.as_str(), "--modes", "dense", "--k", "3"],
    );
    let mut reference = read_report(&reference_json);
    reference["format_version"] = json!(4);
    let baseline_path = instance.temp.path().join("reference-v4.json");
    write_report(&baseline_path, &reference);
    let baseline = path(&baseline_path).to_owned();

    let out = out_dir(&instance, "out-v4");
    let output = instance.eval_at(
        "fixture",
        &lines,
        &out,
        &[
            "--models",
            models.as_str(),
            "--modes",
            "dense",
            "--k",
            "3",
            "--baseline",
            baseline.as_str(),
        ],
    );
    assert!(!output.status.success(), "a version 4 reference is refused");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("format_version") && stderr.contains('4'),
        "the refusal names the version it read: {stderr}"
    );
    let written: Vec<_> = fs::read_dir(&out)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", out.display()))
        .collect();
    assert!(
        written.is_empty(),
        "a refused baseline writes no report: {written:?}"
    );
}

#[test]
fn a_baseline_attributes_a_generation_or_fingerprint_difference_without_refusing() {
    let _serial = common::SerialGuard::acquire();
    // ADR-0155, decision 6: a generation, model or game fingerprint difference
    // makes the cause ambiguous, not the figures false. The reference is read,
    // the top-k is compared, and the mismatch is named.
    let (instance, models) = top_k_setup();
    let lines = top_k_lines();
    let (reference_json, _) = eval_ok(
        &instance,
        &lines,
        &["--models", models.as_str(), "--modes", "dense", "--k", "3"],
    );
    let mut reference = read_report(&reference_json);
    let generation = reference["generation"]
        .as_u64()
        .unwrap_or_else(|| panic!("the reference names its generation: {reference}"));
    reference["generation"] = json!(generation + 1);
    reference["model_fingerprint"] = json!("f".repeat(64));
    reference["queries_fingerprint"] = json!("e".repeat(64));
    let baseline_path = instance.temp.path().join("reference-other.json");
    write_report(&baseline_path, &reference);
    let baseline = path(&baseline_path).to_owned();

    let out = out_dir(&instance, "out-other");
    let output = instance.eval_at(
        "fixture",
        &lines,
        &out,
        &[
            "--models",
            models.as_str(),
            "--modes",
            "dense",
            "--k",
            "3",
            "--baseline",
            baseline.as_str(),
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "a generation or fingerprint difference never refuses the reference: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let (json_path, _) = reports(&output);
    let report = read_report(&json_path);
    let attribution = &report["baseline"]["attribution"];
    assert_eq!(attribution["generation"]["reference"], generation + 1);
    assert_eq!(attribution["generation"]["current"], generation);
    assert_eq!(
        attribution["model_fingerprint"], "differs",
        "the encoder changed, and the report says so: {report}"
    );
    assert_eq!(
        attribution["queries_fingerprint"], "differs",
        "the game changed, and the report says so: {report}"
    );
    // The comparison itself still runs: the lists are unchanged, so the overlap
    // is 1 and no query moved.
    assert_close(
        number(&report, "/baseline/modes/dense/top_k/overlap_at_k"),
        1.0,
        "the lists did not move, and their overlap is 1",
    );
    assert_eq!(
        &report["baseline"]["modes"]["dense"]["top_k"]["changed"],
        &json!([]),
        "no list moved: {report}"
    );
    assert_eq!(report["baseline"]["regression"], false);
}
