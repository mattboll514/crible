// SPDX-License-Identifier: Apache-2.0

// The test of this file reads the segment reader hook of the `failpoints` build
// (`crible_store::testhooks`, ADR-0126, decision 6), which the `crible` crate
// forwards to `crible-store` under its own `failpoints` feature, so a plain
// `cargo test` compiles no test here.
#![cfg(feature = "failpoints")]
// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the checks of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! The CLI keeps one opening per process (ADR-0126, decision 3).
//!
//! The acceptance criterion of `REUTILISER-LECTEURS-DENSES` moves the opening of
//! the dense readers out of the query path, and ADR-0126, decision 3, keeps the
//! CLI on the same code: "la CLI ne tient pas de cache entre commandes : un
//! processus, une ouverture". This test proves it where it is observable: a
//! `crible search` process appends exactly one `open` and one `seal` per segment
//! of the generation it serves, and nothing more. The answer itself is checked
//! too, so a change of the opening path that broke the search fails here rather
//! than in the demo.
//!
//! The dataset comes from `tools/gen_synth.py`; the CI generates it before
//! `cargo test`, as `cli.rs` requires.
//!
//! ```text
//! cargo test -p crible --features failpoints --test search_lecteurs_denses
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crible_store::testhooks::{ReaderEvent, SEGMENT_READERS_ENV, read_events, reset_events};
use serde_json::Value;

/// Command that generates the reference dataset, quoted by the failure message
/// of a missing batch.
const DATASET_COMMAND: &str =
    "cd tools && uv sync && uv run python gen_synth.py --out ../data/synth-small";

/// External model configuration of `synth-small` (768 dimensions, `normalize`).
const MODEL_JSON: &str =
    r#"{ "name": "synthetic-external", "kind": "external", "dims": 768, "normalize": true }"#;

/// Root of the reference dataset of the CLI tests.
fn dataset_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/synth-small")
}

/// Fails with the generator command when the dataset is missing.
fn require_dataset() {
    let root = dataset_root();
    for name in ["index.json", "queries.jsonl"] {
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

/// Runs the binary with the segment reader hook armed on `log`.
fn crible_with_hook(arguments: &[&str], log: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_crible"))
        .args(arguments)
        .env(SEGMENT_READERS_ENV, log)
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

/// Creates the data root of the test and the sibling models volume it refers to
/// (`docs/02-architecture.md`, `docs/07-stack-et-deploiement.md`).
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

/// The first line of a JSON Lines file.
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

/// The value of the `segments: N` line of `crible inspect`.
fn inspected_segments(output: &str) -> usize {
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("segments: "))
        .unwrap_or_else(|| panic!("`crible inspect` prints its segment count: {output}"))
        .trim()
        .parse()
        .unwrap_or_else(|error| panic!("the segment count of `crible inspect` parses: {error}"))
}

/// A `crible search` process opens each segment of the served generation once,
/// and seals it once.
///
/// ADR-0126, decisions 2 and 3: the CLI pays the opening once per process, the
/// blake3 verification stays at that opening, and no query reopens a reader.
#[test]
fn the_cli_search_opens_each_segment_once_per_process() {
    require_dataset();
    let dataset = dataset_root();
    let (_temp, data, models) = prepare_data();
    let data_arg = data.to_str().expect("the data path is UTF-8");
    let models_arg = models.to_str().expect("the models path is UTF-8");

    crible_ok(&[
        "index",
        "create",
        "--data",
        data_arg,
        "--config",
        dataset.join("index.json").to_str().expect("UTF-8 path"),
    ]);
    for batch in ["l1", "l2", "l3"] {
        crible_ok(&[
            "attach",
            "--data",
            data_arg,
            "--models",
            models_arg,
            "--index",
            "synth-small",
            dataset.join(batch).to_str().expect("UTF-8 path"),
        ]);
    }
    let inspected = crible_ok(&["inspect", "--data", data_arg, "--index", "synth-small"]);
    let segments = inspected_segments(&inspected);

    let query = first_json_line(&dataset.join("queries.jsonl"));
    let vector_path = data.join("query.json");
    fs::write(
        &vector_path,
        serde_json::to_string(&query["vector"]).expect("the vector serializes"),
    )
    .expect("the query vector is written");

    let log = data.join("segment-readers.log");
    reset_events(&log).unwrap_or_else(|error| panic!("the hook log is creatable: {error}"));
    let output = crible_with_hook(
        &[
            "search",
            "--data",
            data_arg,
            "--index",
            "synth-small",
            "--vector-file",
            vector_path.to_str().expect("UTF-8 path"),
            "--k",
            "10",
        ],
        &log,
    );
    assert!(
        output.status.success(),
        "crible search failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let events =
        read_events(&log).unwrap_or_else(|error| panic!("the hook log is readable: {error}"));
    let mut opened: Vec<String> = Vec::new();
    for event in &events {
        if let ReaderEvent::Open { segment_id } = event
            && !opened.contains(segment_id)
        {
            opened.push(segment_id.clone());
        }
    }
    assert_eq!(
        opened.len(),
        segments,
        "`crible search` opens every segment of the generation it serves: \
         {events:?}"
    );
    for segment_id in &opened {
        let opens = events
            .iter()
            .filter(|event| matches!(event, ReaderEvent::Open { .. }))
            .filter(|event| event.segment_id() == segment_id)
            .count();
        let seals = events
            .iter()
            .filter(|event| matches!(event, ReaderEvent::Seal { .. }))
            .filter(|event| event.segment_id() == segment_id)
            .count();
        assert_eq!(
            opens, 1,
            "the CLI opens the segment '{segment_id}' once per process, not once \
             per query (ADR-0126, decision 3): {events:?}"
        );
        assert_eq!(
            seals, 1,
            "the CLI seals the segment '{segment_id}' once, at its opening \
             (ADR-0126, decision 2): {events:?}"
        );
    }
    // The opening path is `open` then `seal`, once per segment. The `drop` lines
    // belong to the end of the process, when the readers are released: they are
    // counted but never double, so the process opens and seals each segment once
    // and releases it at most once.
    let opens = events
        .iter()
        .filter(|event| matches!(event, ReaderEvent::Open { .. }))
        .count();
    let seals = events
        .iter()
        .filter(|event| matches!(event, ReaderEvent::Seal { .. }))
        .count();
    let drops = events
        .iter()
        .filter(|event| matches!(event, ReaderEvent::Drop { .. }))
        .count();
    assert_eq!(
        opens, segments,
        "the CLI opens each segment of the generation it serves once: {events:?}"
    );
    assert_eq!(
        seals, segments,
        "the CLI seals each segment it opened once (ADR-0126, decision 2): {events:?}"
    );
    assert!(
        drops <= segments,
        "each reader of the process is released once at most: {events:?}"
    );

    // The search still answers what it answered before the cache: the generation
    // it served and one result of the reference query (`cli.rs` compares the
    // whole answer, this test pins the part the opening path could break).
    let answer: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "the search output is JSON: {error}\n{}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    assert_eq!(
        answer["generation"], 3,
        "the search serves the last attached generation: {answer}"
    );
    let results = answer["results"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer carries its results: {answer}"));
    assert!(
        !results.is_empty(),
        "the search answers at least one passage: {answer}"
    );
}
