// SPDX-License-Identifier: Apache-2.0

//! End-to-end check of `crible search --query` (`ENCODER-REQUETES`, phase P2).
//!
//! The test builds the `synth-small` index exactly as the J1 demo does, with the
//! `synthetic-external` model the dataset declares, then searches it by text
//! through the versioned fake encoder of
//! `crates/crible-encode/tests/fixtures/fake-model`. A second call with an
//! over-long query checks that truncation is reported as `truncated_query`.
//!
//! ONNX Runtime is required, through `ORT_DYLIB_PATH`, and CI has none, so the
//! test is `#[ignore]`d:
//!
//! ```sh
//! ORT_DYLIB_PATH=/path/to/libonnxruntime.dylib \
//!   cargo test -p crible --test query_cli -- --ignored
//! ```

#![allow(clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

/// Command that generates the reference dataset, quoted by the failure message
/// of a missing batch.
const DATASET_COMMAND: &str =
    "cd tools && uv sync && uv run python gen_synth.py --out ../data/synth-small";

/// External model configuration of `synth-small` (768 dimensions, `normalize`),
/// which the attach of the dataset needs.
const EXTERNAL_MODEL_JSON: &str =
    r#"{ "name": "synthetic-external", "kind": "external", "dims": 768, "normalize": true }"#;

fn dataset_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/synth-small")
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../crible-encode/tests/fixtures/fake-model")
}

/// Fails with the generator command when a file of the dataset is missing.
fn require_dataset() {
    let root = dataset_root();
    for name in ["index.json", "queries.jsonl", "expected.jsonl"] {
        assert!(
            root.join(name).is_file(),
            "{} is missing; generate the dataset before running this test:\n  {DATASET_COMMAND}",
            root.join(name).display()
        );
    }
    for batch in ["l1", "l2", "l3"] {
        assert!(
            root.join(batch).join("manifest.json").is_file(),
            "batch '{batch}' is missing; generate the dataset before running this test:\n  {DATASET_COMMAND}"
        );
    }
}

fn crible(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_crible"))
        .args(arguments)
        .output()
        .unwrap_or_else(|error| panic!("cannot run crible {arguments:?}: {error}"))
}

fn crible_ok(arguments: &[&str]) -> String {
    let output = crible(arguments);
    assert!(
        output.status.success(),
        "crible {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Creates the data root, the external model of `synth-small`, and a copy of
/// the fake encoder under the models root.
fn prepare() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
    let data = temp.path().join("data");
    let models = temp.path().join("models");
    let external = models.join("synthetic-external");
    fs::create_dir_all(&external)
        .unwrap_or_else(|error| panic!("the external model directory is created: {error}"));
    fs::write(external.join("model.json"), EXTERNAL_MODEL_JSON)
        .unwrap_or_else(|error| panic!("the external model.json is written: {error}"));

    let fixture = fixture_root();
    assert!(
        fixture.join("model.json").is_file(),
        "{} is missing; the fake model fixture is versioned",
        fixture.display()
    );
    let fake = models.join("fake-model");
    fs::create_dir_all(&fake).unwrap_or_else(|error| panic!("the fake model directory: {error}"));
    for name in [
        "model.json",
        "model.onnx",
        "model.int8.onnx",
        "tokenizer.json",
    ] {
        fs::copy(fixture.join(name), fake.join(name))
            .unwrap_or_else(|error| panic!("{name} is copied: {error}"));
    }
    (temp, data, models)
}

/// Creates the index and attaches the three batches of `synth-small`.
fn build_index(data: &Path, models: &Path, dataset: &Path) {
    let data_arg = data
        .to_str()
        .unwrap_or_else(|| panic!("the data path {} is UTF-8", data.display()));
    let models_arg = models
        .to_str()
        .unwrap_or_else(|| panic!("the models path {} is UTF-8", models.display()));
    let config = dataset.join("index.json");
    let config = config
        .to_str()
        .unwrap_or_else(|| panic!("the dataset path {} is UTF-8", config.display()));
    crible_ok(&["index", "create", "--data", data_arg, "--config", config]);
    for batch in ["l1", "l2", "l3"] {
        let batch = dataset.join(batch);
        let batch = batch
            .to_str()
            .unwrap_or_else(|| panic!("the batch path {} is UTF-8", batch.display()));
        crible_ok(&[
            "attach",
            "--data",
            data_arg,
            "--models",
            models_arg,
            "--index",
            "synth-small",
            batch,
        ]);
    }
}

#[test]
#[ignore = "needs ONNX Runtime (ORT_DYLIB_PATH); run: ORT_DYLIB_PATH=/path/to/libonnxruntime.dylib cargo test -p crible --test query_cli -- --ignored"]
fn search_by_query_works_end_to_end_on_synth_small() {
    // Acceptance criterion: `crible search --query` works end to end on
    // `synth-small`, with a test model (`ENCODER-REQUETES`).
    require_dataset();
    let dataset = dataset_root();
    let (_temp, data, models) = prepare();
    build_index(&data, &models, &dataset);

    let data_arg = data.to_str().expect("the data path is UTF-8");
    let models_arg = models.to_str().expect("the models path is UTF-8");
    let searched = crible_ok(&[
        "search",
        "--data",
        data_arg,
        "--index",
        "synth-small",
        "--models",
        models_arg,
        "--model",
        "fake-model",
        "--query",
        "droit du contrat",
        "--k",
        "10",
    ]);
    let response: Value = serde_json::from_str(&searched)
        .unwrap_or_else(|error| panic!("the search output must be JSON: {error}\n{searched}"));
    assert_eq!(response["generation"], 3);
    assert_eq!(
        response["pool_size"], 9750,
        "the whole live set is the pool"
    );
    assert_eq!(
        response["truncated_query"], false,
        "a short query is not truncated"
    );
    let results = response["results"].as_array().expect("results is an array");
    assert_eq!(results.len(), 10, "k results come back");
    for result in results {
        assert!(
            result["key"].is_string() && result["score"].is_number(),
            "each result carries its key and score: {result}"
        );
    }

    // The fake model declares `max_tokens: 32`: a 40-word query is truncated,
    // and the response says so instead of losing its tail silently (INV-11).
    let long_query = vec!["droit"; 40].join(" ");
    let truncated = crible_ok(&[
        "search",
        "--data",
        data_arg,
        "--index",
        "synth-small",
        "--models",
        models_arg,
        "--model",
        "fake-model",
        "--query",
        &long_query,
        "--k",
        "5",
    ]);
    let truncated: Value = serde_json::from_str(&truncated)
        .unwrap_or_else(|error| panic!("the truncated search must be JSON: {error}"));
    assert_eq!(
        truncated["truncated_query"], true,
        "the over-long query is reported as truncated"
    );
    assert_eq!(
        truncated["results"]
            .as_array()
            .expect("results is an array")
            .len(),
        5,
        "a truncated query still returns its k results"
    );
}

#[test]
#[ignore = "needs ONNX Runtime (ORT_DYLIB_PATH); run: ORT_DYLIB_PATH=/path/to/libonnxruntime.dylib cargo test -p crible --test query_cli -- --ignored"]
fn search_by_query_refuses_an_external_index_without_an_encoder() {
    // Without `--model`, the model of the index is used; `synth-small` declares
    // an `external` model, which has no encoder, and the message points at the
    // flag that fixes it.
    require_dataset();
    let dataset = dataset_root();
    let (_temp, data, models) = prepare();
    build_index(&data, &models, &dataset);

    let output = crible(&[
        "search",
        "--data",
        data.to_str().expect("UTF-8 path"),
        "--index",
        "synth-small",
        "--models",
        models.to_str().expect("UTF-8 path"),
        "--query",
        "droit",
    ]);
    assert!(!output.status.success(), "an external model has no encoder");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("invalid_request: "),
        "the error names its code: {stderr}"
    );
    assert!(
        stderr.contains("--model"),
        "the message points at the flag: {stderr}"
    );
}
