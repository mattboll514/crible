// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `FUSIONNER-HYBRIDE` (phase P5) at the CLI boundary: `--mode`, `--fusion` and
//! `--alpha` of `crible search`.
//!
//! References: the task sheet (`docs/phases/P5-lexical-et-hybride.md:78-90`),
//! `docs/03-recherche.md` ("Fusion (mode `hybrid`)" and the pipeline table),
//! `docs/05-api.md` (`mode`, `fusion`, `alpha`, `score_type`), ADR-0008 (both
//! branches on the same pool), ADR-0075 and ADR-0089 (decision 3: an index whose
//! `lexical.enabled` is false refuses a text mode in `invalid_request` and keeps
//! serving `dense`).
//!
//! The fusion itself and the pipeline are pinned by the library tests
//! (`crates/crible-search/src/fusion.rs`, `crates/crible-search/tests/
//! fusionner_hybride.rs`, `hybrid_refusal_without_reader.rs`). These tests pin
//! what only the command can show: the flags exist, they reach the pipeline, the
//! default mode is read from the configuration, and a flag outside its domain is
//! refused before any work.
//!
//! # What these tests can run without ONNX Runtime
//!
//! A `hybrid` search encodes its text with the model of the index, so an
//! end-to-end `hybrid` request needs ONNX Runtime, which the fixture does not
//! have (`query_cli.rs` is `#[ignore]` for the same reason). The tests therefore
//! avoid the encoder and still prove the default mode: on an index whose lexical
//! branch is enabled, a request that names no mode and passes a vector file is
//! refused as `--mode hybrid`, which no other default would produce; on an index
//! that disables the branch, the same request is served in `cosine`.
//!
//! | Test | Criterion / decision |
//! |---|---|
//! | `lexical_mode_reports_bm25_scores` | `score_type` per mode, `lexical` alone |
//! | `dense_mode_reports_cosine_scores` | `score_type` per mode, `dense` alone |
//! | `the_default_mode_is_dense_on_an_index_without_the_lexical_branch` | ADR-0089 §3 |
//! | `the_default_mode_is_hybrid_on_an_index_with_the_lexical_branch` | ADR-0089 §3 |
//! | `hybrid_is_refused_when_the_lexical_branch_is_disabled` | ADR-0089 §3, no degradation |
//! | `a_lexical_mode_refuses_a_vector_file` | ADR-0087 §2 |
//! | `hybrid_accepts_a_vector_file_with_the_query_text` | ADR-0109 §4 |
//! | `a_text_mode_needs_the_query_text` | `invalid_request` on a missing input |
//! | `the_fusion_flags_are_checked_in_every_mode` | ADR-0075, `docs/05-api.md` |
//! | `the_alpha_domain_is_checked_by_the_cli` | ADR-0090 §2 |

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
const INDEX: &str = "hybride-cli";

/// Dimensions of the fixture vectors.
const DIMS: u32 = 4;

/// Text of `V1#0`; holds `contrat`, which the lexical queries match.
const TEXT_1: &str = "contrat de travail";

/// Text of `V2#0`; holds `contrat` and `vente`.
const TEXT_2: &str = "contrat de vente";

/// Text of `V3#0`; holds `vente` only.
const TEXT_3: &str = "vente immobiliere";

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
    /// Runs `crible index create`, then commits generation 1 with three
    /// documents and three passages, with or without the lexical branch.
    ///
    /// | Key | Dense score | Text |
    /// |---|---|---|
    /// | `V1#0` | 4.0 | `TEXT_1` |
    /// | `V2#0` | 2.0 | `TEXT_2` |
    /// | `V3#0` | 1.0 | `TEXT_3` |
    ///
    /// The dense scores are the first component of the vectors, with a query
    /// vector of `[1.0, 0.0, 0.0, 0.0]`, so the expected order is readable from
    /// the table. With `lexical` on, the commit builds the tantivy index of the
    /// epoch, so `--mode lexical` finds the passages without an encoder.
    fn new(lexical: bool) -> Self {
        let temp = tempfile::tempdir()
            .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
        let data = temp.path().join("data");
        fs::create_dir_all(&data)
            .unwrap_or_else(|error| panic!("the data root must be creatable: {error}"));
        let config = temp.path().join("index.json");
        fs::write(&config, configuration(INDEX, lexical))
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
                documents: vec![document("V1"), document("V2"), document("V3")],
                passages: vec![
                    passage(0, "V1", 0, TEXT_1),
                    passage(1, "V2", 0, TEXT_2),
                    passage(2, "V3", 0, TEXT_3),
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

    /// Runs `crible search` with the given arguments and returns its output.
    fn run(&self, arguments: &[&str]) -> Output {
        let mut full: Vec<&str> = vec!["search", "--data", path(&self.data), "--index", INDEX];
        full.extend_from_slice(arguments);
        crible(&full)
    }

    /// Runs `crible search`, asserting success, and returns the JSON response.
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

    /// Runs `crible search`, asserting an `invalid_request` refusal, and returns
    /// the message.
    fn refused(&self, arguments: &[&str]) -> String {
        let output = self.run(arguments);
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(
            !output.status.success(),
            "search {arguments:?} must be refused"
        );
        assert!(
            stderr.starts_with("invalid_request: "),
            "search {arguments:?} must refuse in invalid_request, got: {stderr}"
        );
        stderr
    }

    /// Runs `crible search` with the fixture vector file.
    fn with_vector(&self, arguments: &[&str]) -> Output {
        let mut full: Vec<&str> = vec!["--vector-file", path(&self.vector)];
        full.extend_from_slice(arguments);
        self.run(&full)
    }
}

/// Schema of the index.
fn schema() -> Schema {
    Schema::from_json_str(SCHEMA)
        .unwrap_or_else(|error| panic!("the fixture schema must be valid: {error}"))
}

/// Configuration of the index, with the settings the store requires.
fn configuration(name: &str, lexical: bool) -> String {
    format!(
        r#"{{"name":"{name}","model":"m","reference_precision":"fp16","schema":{SCHEMA},"cluster_by":[],
            "lexical":{{"enabled":{lexical},"language":"fr","positions":false}},
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
fn passage(iid: u32, doc_id: &str, ord: u32, text: &str) -> PassageRecord {
    PassageRecord {
        iid: Iid::new(iid),
        doc_id: doc_id.to_owned(),
        ord,
        text: text.to_owned(),
        passage_hash: format!("blake3:{doc_id}-{ord}"),
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
    for score in [4.0_f32, 2.0, 1.0] {
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

/// `score_type` of the answer, which every result repeats.
fn score_type(response: &Value) -> String {
    response["results"]
        .as_array()
        .and_then(|results| results.first())
        .and_then(|result| result["score_type"].as_str())
        .unwrap_or_else(|| panic!("each result carries its score_type: {response}"))
        .to_owned()
}

#[test]
fn dense_mode_reports_cosine_scores() {
    // The lexical branch is on, so a mode-less request would run `hybrid`; the
    // flag alone makes the request dense, and the vector file is its input.
    let instance = Instance::new(true);

    let dense = instance.search(&["--vector-file", path(&instance.vector), "--mode", "dense"]);
    assert_eq!(
        keys(&dense),
        vec!["V1#0", "V2#0", "V3#0"],
        "the dense branch ranks by the first component of the vectors"
    );
    assert_eq!(score_type(&dense), "cosine");
}

#[test]
fn lexical_mode_reports_bm25_scores() {
    let instance = Instance::new(true);

    let lexical = instance.search(&["--mode", "lexical", "--query", "contrat"]);
    assert_eq!(
        keys(&lexical),
        vec!["V1#0", "V2#0"],
        "only the passages that hold the term are returned, best first"
    );
    assert_eq!(score_type(&lexical), "bm25");

    let vente = instance.search(&["--mode", "lexical", "--query", "vente"]);
    assert_eq!(keys(&vente), vec!["V2#0", "V3#0"]);
    assert_eq!(score_type(&vente), "bm25");
}

#[test]
fn the_default_mode_is_dense_on_an_index_without_the_lexical_branch() {
    // ADR-0089 decision 3: `lexical.enabled` false means the default mode is
    // `dense`, which serves the index alone. A vector file is enough, so the
    // test runs without ONNX Runtime.
    let instance = Instance::new(false);

    let served = instance.with_vector(&[]);
    let stderr = String::from_utf8_lossy(&served.stderr);
    assert!(
        served.status.success(),
        "an index without the lexical branch must keep serving dense: {stderr}"
    );
    let response: Value = serde_json::from_slice(&served.stdout)
        .unwrap_or_else(|error| panic!("the search answer must be JSON: {error}"));
    assert_eq!(keys(&response), vec!["V1#0", "V2#0", "V3#0"]);
    assert_eq!(score_type(&response), "cosine");
}

#[test]
fn the_default_mode_is_hybrid_on_an_index_with_the_lexical_branch() {
    // ADR-0089 decision 3: the default mode of an index whose lexical branch is
    // enabled is `hybrid`. `hybrid` scores the query text, so a vector file alone
    // is refused by name; no other default mode would refuse it.
    let instance = Instance::new(true);

    let stderr = {
        let output = instance.with_vector(&[]);
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(
            !output.status.success(),
            "a mode-less request on a lexical index must run hybrid and refuse a vector file"
        );
        stderr
    };
    assert!(
        stderr.starts_with("invalid_request: "),
        "the refusal is an invalid_request, got: {stderr}"
    );
    assert!(
        stderr.contains("--mode hybrid"),
        "the refusal must name the default mode, got: {stderr}"
    );
}

#[test]
fn hybrid_is_refused_when_the_lexical_branch_is_disabled() {
    // ADR-0089 decision 3: the flag is refused in `invalid_request`, before any
    // work, and the refusal says what the index allows. `hybrid` never degrades
    // into the dense ranking (ADR-0087 decision 2).
    let instance = Instance::new(false);

    for mode in ["hybrid", "lexical"] {
        let stderr = instance.refused(&["--mode", mode, "--query", "contrat"]);
        assert!(
            stderr.contains("lexical branch is disabled for this index"),
            "--mode {mode} must name the disabled branch, got: {stderr}"
        );
    }
}

#[test]
fn a_lexical_mode_refuses_a_vector_file() {
    // ADR-0087 decision 2: the lexical branch scores the query text, so a vector
    // file alone gives it nothing to score; `lexical` is refused rather than
    // silently dropping the branch. `hybrid` accepts the vector for its dense
    // branch (ADR-0109, decision 4).
    let instance = Instance::new(true);

    let stderr = instance.refused(&["--mode", "lexical", "--vector-file", path(&instance.vector)]);
    assert!(
        stderr.contains("--mode lexical runs the lexical branch"),
        "--mode lexical must refuse a vector file by name, got: {stderr}"
    );
}

#[test]
fn hybrid_accepts_a_vector_file_with_the_query_text() {
    // ADR-0109, decision 4: in `hybrid`, `--query` serves the lexical branch and
    // `--vector-file` serves the dense one in place of the encoding, so the
    // search runs without ONNX Runtime.
    let instance = Instance::new(true);

    let output = instance.with_vector(&["--mode", "hybrid", "--query", "contrat"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "hybrid with --query and --vector-file must run: {stderr}"
    );
    let response: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("the search answer must be JSON: {error}"));
    assert_eq!(score_type(&response), "rrf");
    assert!(
        keys(&response)
            .iter()
            .any(|key| key.starts_with("V1#") || key.starts_with("V2#")),
        "the fused answer holds the dense and lexical results: {response}"
    );
}

#[test]
fn a_text_mode_needs_the_query_text() {
    let instance = Instance::new(true);

    let stderr = instance.refused(&["--mode", "lexical"]);
    assert!(
        stderr.contains("--mode lexical needs --query <text>"),
        "the refusal must name the missing input, got: {stderr}"
    );
}

#[test]
fn the_alpha_domain_is_checked_by_the_cli() {
    // ADR-0090, decision 2: `alpha` is in `[0, 1]`, bounds included, on a finite
    // value. The flag reuses the control of `crible-core`, so the message names
    // `fusion.alpha` and the CLI refuses exactly what the configuration refuses.
    let instance = Instance::new(true);

    for accepted in ["0", "1", "0.5"] {
        let answer = instance.search(&[
            "--mode",
            "lexical",
            "--query",
            "contrat",
            "--fusion",
            "relative_score",
            "--alpha",
            accepted,
        ]);
        assert_eq!(
            score_type(&answer),
            "bm25",
            "the fusion only acts in hybrid, but alpha = {accepted} is in the domain"
        );
    }

    for refused in ["-0.1", "1.1", "NaN", "inf", "-inf"] {
        // Both spellings are checked: `--alpha=-0.1` and `--alpha -0.1`. The
        // flag takes hyphen values, so a negative weight reaches the domain
        // control and is refused in `invalid_request`, not as a usage error.
        for arguments in [
            vec!["--alpha".to_owned(), refused.to_owned()],
            vec![format!("--alpha={refused}")],
        ] {
            let mut full: Vec<&str> = vec![
                "--mode",
                "lexical",
                "--query",
                "contrat",
                "--fusion",
                "relative_score",
            ];
            full.extend(arguments.iter().map(String::as_str));
            let stderr = instance.refused(&full);
            assert!(
                stderr.contains("fusion.alpha"),
                "alpha = {refused} must be refused by a message naming fusion.alpha, got: {stderr}"
            );
        }
    }
}

#[test]
fn the_fusion_flags_are_checked_in_every_mode() {
    // The fusion only acts in `hybrid`, but its domain is checked in every mode,
    // as the grouping options are (ADR-0075): a caller never believes an option
    // took effect. `lexical` needs no encoder, so the checks run without ONNX
    // Runtime.
    let instance = Instance::new(true);

    let alpha_without_fusion =
        instance.refused(&["--mode", "lexical", "--query", "contrat", "--alpha", "0.5"]);
    assert!(
        alpha_without_fusion.contains("--alpha needs --fusion relative_score"),
        "got: {alpha_without_fusion}"
    );

    let alpha_with_rrf = instance.refused(&[
        "--mode", "lexical", "--query", "contrat", "--fusion", "rrf", "--alpha", "0.5",
    ]);
    assert!(
        alpha_with_rrf.contains("alpha is not allowed with rrf"),
        "got: {alpha_with_rrf}"
    );

    let relative_without_alpha = instance.refused(&[
        "--mode",
        "lexical",
        "--query",
        "contrat",
        "--fusion",
        "relative_score",
    ]);
    assert!(
        relative_without_alpha.contains("--fusion relative_score needs --alpha"),
        "got: {relative_without_alpha}"
    );

    // The accepted pair is legal in `lexical` too, and leaves the score of the
    // branch alone: the fusion has nothing to fuse.
    let accepted = instance.search(&[
        "--mode",
        "lexical",
        "--query",
        "contrat",
        "--fusion",
        "relative_score",
        "--alpha",
        "0.3",
    ]);
    assert_eq!(keys(&accepted), vec!["V1#0", "V2#0"]);
    assert_eq!(score_type(&accepted), "bm25");
}
