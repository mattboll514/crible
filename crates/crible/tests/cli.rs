// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! End-to-end test of the first CLI commands.
//!
//! References: `docs/phases/P1-stockage-et-recherche-exacte.md` (task
//! `CLI-PREMIERES-COMMANDES`), `docs/jalons/J1-demo.md` (the demo played here on
//! `synth-small`) and `docs/05-api.md` ("Recherche : réponse").
//!
//! The test chains the commands the J1 demo runs: `index create`, `attach` on
//! the three batches `l1`, `l2`, `l3`, `search` with the first query of
//! `queries.jsonl`, then `inspect`. It also pins the two acceptance criteria
//! that are not about a happy path: the demo scenario works as written, and a
//! failure prints its `ErrorCode` wire name and a message.
//!
//! The dataset comes from `tools/gen_synth.py`; the CI generates it before
//! `cargo test`. A missing batch fails the test with the command that writes it,
//! as the store tests do.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

/// Command that generates the reference dataset, quoted by the failure message
/// of a missing batch.
const DATASET_COMMAND: &str =
    "cd tools && uv sync && uv run python gen_synth.py --out ../data/synth-small";

/// External model configuration of `synth-small` (768 dimensions, `normalize`).
const MODEL_JSON: &str =
    r#"{ "name": "synthetic-external", "kind": "external", "dims": 768, "normalize": true }"#;

/// Live documents and passages after `l1`, `l2` and `l3`.
const FINAL_DOCUMENTS: &str = "documents: 1950";
const FINAL_PASSAGES: &str = "passages: 9750 live, 2250 dead";

fn dataset_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/synth-small")
}

/// Fails with the generator command when the dataset is missing.
fn require_dataset() {
    let root = dataset_root();
    for name in ["index.json", "queries.jsonl", "expected.jsonl"] {
        assert!(
            root.join(name).is_file(),
            "{} is missing; generate the dataset before running `cargo test`:\n  {DATASET_COMMAND}",
            root.join(name).display()
        );
    }
    for batch in ["l1", "l2", "l3"] {
        assert!(
            root.join(batch).join("manifest.json").is_file(),
            "batch '{batch}' is missing; generate the dataset before running `cargo test`:\n  {DATASET_COMMAND}"
        );
    }
}

/// Runs the binary with absolute paths.
fn crible(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_crible"))
        .args(arguments)
        .output()
        .unwrap_or_else(|error| panic!("cannot run crible {arguments:?}: {error}"))
}

/// Runs a command and returns its stdout, failing on a non-zero exit.
fn crible_ok(arguments: &[&str]) -> String {
    let output = crible(arguments);
    assert!(
        output.status.success(),
        "crible {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Creates the data root of one test and the sibling models volume it refers
/// to (`docs/02-architecture.md`, `docs/07-stack-et-deploiement.md`).
fn prepare_data() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
    let data = temp.path().join("data");
    let models = temp.path().join("models");
    let model = models.join("synthetic-external");
    fs::create_dir_all(&model)
        .unwrap_or_else(|error| panic!("the model directory must be created: {error}"));
    fs::write(model.join("model.json"), MODEL_JSON)
        .unwrap_or_else(|error| panic!("the model file must be written: {error}"));
    (temp, data, models)
}

#[test]
fn the_j1_demo_creates_attaches_searches_and_inspects() {
    require_dataset();
    let dataset = dataset_root();
    let (_temp, data, models) = prepare_data();
    let data_arg = data.to_str().expect("the data path is UTF-8");
    let index_dir = data.join("indexes").join("synth-small");

    // `crible index create --data <dir> --config <json>`
    let created = crible_ok(&[
        "index",
        "create",
        "--data",
        data_arg,
        "--config",
        dataset.join("index.json").to_str().expect("UTF-8 path"),
    ]);
    assert!(
        created.contains("index 'synth-small' created"),
        "unexpected creation output: {created}"
    );
    assert!(
        index_dir.join("CURRENT_EPOCH").is_file(),
        "creation must leave CURRENT_EPOCH"
    );

    // `crible attach --data <dir> --index <i> <batch-dir>`, three times.
    for (batch, generation) in [
        ("l1", "generation 1"),
        ("l2", "generation 2"),
        ("l3", "generation 3"),
    ] {
        let output = crible_ok(&[
            "attach",
            "--data",
            data_arg,
            "--models",
            models.to_str().expect("UTF-8 path"),
            "--index",
            "synth-small",
            dataset.join(batch).to_str().expect("UTF-8 path"),
        ]);
        assert!(
            output.contains(generation),
            "attaching '{batch}' must reach {generation}: {output}"
        );
    }

    // `crible inspect --data <dir> --index <i>`
    let inspected = crible_ok(&["inspect", "--data", data_arg, "--index", "synth-small"]);
    for expected in [
        "epoch: 1",
        "generation: 3",
        FINAL_DOCUMENTS,
        FINAL_PASSAGES,
        "segments: 2",
        "bitmaps:",
    ] {
        assert!(
            inspected.contains(expected),
            "inspect output must contain {expected:?}: {inspected}"
        );
    }

    // `crible search --vector-file <npy|json> --filter <json> --k 10`, with the
    // first query of the reference set (no filter, k = 10).
    let query = first_json_line(&dataset.join("queries.jsonl"));
    let expected = first_json_line(&dataset.join("expected.jsonl"));
    let vector_path = data.join("query.json");
    fs::write(
        &vector_path,
        serde_json::to_string(&query["vector"]).expect("the vector serializes"),
    )
    .expect("the query vector is written");
    let searched = crible_ok(&[
        "search",
        "--data",
        data_arg,
        "--index",
        "synth-small",
        "--vector-file",
        vector_path.to_str().expect("UTF-8 path"),
        "--k",
        "10",
    ]);
    let response: Value = serde_json::from_str(&searched)
        .unwrap_or_else(|error| panic!("the search output must be JSON: {error}\n{searched}"));
    assert_eq!(
        response["pool_size"], expected["pool_size"],
        "the pool of the first query must match the reference"
    );
    assert_eq!(response["generation"], 3);
    let keys = response["results"]
        .as_array()
        .expect("results is an array")
        .iter()
        .map(|result| {
            result["key"]
                .as_str()
                .expect("a key is a string")
                .to_owned()
        })
        .collect::<Vec<_>>();
    let expected_keys = expected["results"]
        .as_array()
        .expect("reference results is an array")
        .iter()
        .map(|result| {
            result["key"]
                .as_str()
                .expect("a key is a string")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(keys, expected_keys, "the top-k must match the reference");

    // `crible search --filter '{"categorie": "cat-03"}' --k 5`: every result
    // belongs to a `cat-03` document, the pool is narrower than the live set,
    // and the scores decrease.
    let filtered = crible_ok(&[
        "search",
        "--data",
        data_arg,
        "--index",
        "synth-small",
        "--vector-file",
        vector_path.to_str().expect("UTF-8 path"),
        "--filter",
        r#"{"categorie": "cat-03"}"#,
        "--k",
        "5",
    ]);
    let filtered: Value = serde_json::from_str(&filtered)
        .unwrap_or_else(|error| panic!("the filtered search must be JSON: {error}"));
    let pool_size = filtered["pool_size"]
        .as_u64()
        .expect("pool_size is a number");
    assert!(
        pool_size < 9750,
        "the filter must narrow the pool: {pool_size}"
    );
    let results = filtered["results"].as_array().expect("results is an array");
    assert!(!results.is_empty(), "the filter must keep some results");
    assert!(results.len() <= 5, "k = 5 must bound the results");
    for result in results {
        assert_eq!(
            result["fields"]["categorie"], "cat-03",
            "every result must satisfy the filter: {result}"
        );
    }
    for pair in results.windows(2) {
        let (first, second) = (
            pair[0]["score"].as_f64().expect("a score is a number"),
            pair[1]["score"].as_f64().expect("a score is a number"),
        );
        assert!(
            first >= second,
            "the scores must decrease: {first} then {second}"
        );
    }

    // `crible inspect --data <dir>` lists every index.
    let all = crible_ok(&["inspect", "--data", data_arg]);
    assert!(
        all.contains("index synth-small"),
        "inspect must list the index: {all}"
    );
}

#[test]
fn a_refused_creation_reports_its_code_and_leaves_the_index_intact() {
    require_dataset();
    let dataset = dataset_root();
    let (_temp, data, _models) = prepare_data();
    let data_arg = data.to_str().expect("the data path is UTF-8");
    crible_ok(&[
        "index",
        "create",
        "--data",
        data_arg,
        "--config",
        dataset.join("index.json").to_str().expect("UTF-8 path"),
    ]);

    let output = crible(&[
        "index",
        "create",
        "--data",
        data_arg,
        "--config",
        dataset.join("index.json").to_str().expect("UTF-8 path"),
    ]);
    assert!(
        !output.status.success(),
        "creating an existing index must fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("invalid_request: "),
        "the error must name its code: {stderr}"
    );
    assert!(
        stderr.contains("already exists"),
        "the message must say what was refused: {stderr}"
    );
    assert!(
        stderr.contains("attach") && stderr.contains("search") && stderr.contains("inspect"),
        "the message must point at the commands of an existing index: {stderr}"
    );
    assert!(
        !stderr.contains("remove"),
        "a complete index is usable, so the refusal must not invite to remove it: {stderr}"
    );

    let inspected = crible_ok(&["inspect", "--data", data_arg, "--index", "synth-small"]);
    assert!(
        inspected.contains("generation: 0"),
        "the refusal must leave the index intact: {inspected}"
    );
}

#[test]
fn a_malformed_request_reports_its_code_with_a_message() {
    require_dataset();
    let dataset = dataset_root();
    let (_temp, data, _models) = prepare_data();
    let data_arg = data.to_str().expect("the data path is UTF-8");

    let missing = crible(&[
        "search",
        "--data",
        data_arg,
        "--index",
        "unknown",
        "--vector-file",
        "missing.npy",
    ]);
    assert!(!missing.status.success());
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(
        stderr.starts_with("unknown_index: "),
        "the error must name its code: {stderr}"
    );
    assert!(
        stderr.contains("cannot read the configuration of index 'unknown'"),
        "the message must name the missing index: {stderr}"
    );

    let bad_k = crible(&[
        "search",
        "--data",
        data_arg,
        "--index",
        "unknown",
        "--vector-file",
        "missing.npy",
        "--k",
        "0",
    ]);
    assert!(!bad_k.status.success());
    assert!(
        String::from_utf8_lossy(&bad_k.stderr).starts_with("invalid_request: "),
        "k out of range must be invalid_request: {}",
        String::from_utf8_lossy(&bad_k.stderr)
    );

    // With an existing index, an unreadable vector file is a readable
    // `invalid_request`, as the J1 demo writes it.
    crible_ok(&[
        "index",
        "create",
        "--data",
        data_arg,
        "--config",
        dataset.join("index.json").to_str().expect("UTF-8 path"),
    ]);
    let unreadable = crible(&[
        "search",
        "--data",
        data_arg,
        "--index",
        "synth-small",
        "--vector-file",
        "missing.npy",
    ]);
    assert!(!unreadable.status.success());
    let stderr = String::from_utf8_lossy(&unreadable.stderr);
    assert!(
        stderr.starts_with("invalid_request: "),
        "an unreadable vector must be invalid_request: {stderr}"
    );
    assert!(
        stderr.contains("cannot read the vector file missing.npy"),
        "the message must name the vector file: {stderr}"
    );
}

/// Reads the first line of a JSON-lines file.
fn first_json_line(path: &Path) -> Value {
    let raw = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
    let line = raw
        .lines()
        .next()
        .unwrap_or_else(|| panic!("{} must hold at least one line", path.display()));
    serde_json::from_str(line).unwrap_or_else(|error| {
        panic!("the first line of {} must be JSON: {error}", path.display())
    })
}
