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
//! | `the_report_is_closed_and_carries_no_query_text` | closed JSON, `format_version: 2`, `source`, ids only (decisions 11 and 16 of ADR-0125, decision 7 of ADR-0131) |
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
    let instance = Instance::new(&quality_fixture());
    let lines = [
        query("q1", &[1.0, 0.0, 0.0, 0.0], &["B", "C"]),
        query("q2", &[1.0, 0.0, 0.0, 0.0], &["C", "F"]),
        query("q3", &[1.0, 0.0, 0.0, 0.0], &[]),
    ];
    let (json_path, _) = eval_ok(&instance, &lines, &["--modes", "dense", "--k", "3"]);
    let report = read_report(&json_path);

    assert_eq!(report["format_version"], 2);
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
    let instance = Instance::new(&quality_fixture());
    let line = json!({
        "id": "q1",
        "vector": [1.0, 0.0, 0.0, 0.0],
        "filter": { "marqueur": FILTER_MARKER },
        "expected": ["A"],
    });
    let (json_path, markdown_path) = eval_ok(&instance, &[line], &["--modes", "dense"]);
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
            "modes",
        ]),
        "the report's members are fixed (decision 11)"
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
        .take(SYNTH_QUERIES)
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
