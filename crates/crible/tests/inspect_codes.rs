// SPDX-License-Identifier: Apache-2.0

// `clippy::panic` is allowed on the fixture helpers that stop their test, not on
// the whole file: `clippy.toml` covers `#[test]` functions only.

//! `crible inspect` reports the memory held by the binary codes.
//!
//! Acceptance criterion of `QUANTIFIER-BINAIRE`
//! (`docs/phases/P4-recherche-rapide.md`): "`crible inspect` affiche la mémoire
//! occupée par les codes". The codes live in RAM, not in a memory map
//! (`docs/02-architecture.md`, "Concurrence"), so the figure an operator reads
//! is the size of the arrays loaded from every `codes.bin` of the epoch.
//!
//! The test reads the printed segments of the same output and recomputes the
//! figure from them: the number printed must be the sum over segments of
//! `count × words_per_code(dims) × 8` bytes, with `dims` the dimensionality of
//! the index (768 in `synth-small`). Dead passages still hold a code until the
//! compaction, and the segment count is what includes them.
//!
//! The figure counts every code the snapshot holds, whether it was read from a
//! `codes.bin` or built in RAM because the file was missing (`ADR-0069`,
//! decisions 1 and 2): an index restored from a snapshot, which never carries
//! `codes.bin`, must report the same memory as the index it came from.
//! `inspect` opens the index in read mode, so it never writes a missing
//! `codes.bin`: the codes are quantized in RAM and the figure is the same.
//!
//! Assumed output of `crible inspect`, one line after the `bitmaps:` line:
//!
//! ```text
//!   codes: 384000 bytes
//! ```
//!
//! API behind it: `Snapshot::codes_memory(&self) -> u64`, the sum of
//! `crible_store::codes::codes_len(count, dims)` over the segments of the
//! generation, exposed as `IndexState::codes_memory: u64`.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{crible, path, require_synth, synth_root};

/// Dimensionality of the vectors of `synth-small` (`model.json` of the
/// dataset, as `tools/gen_synth.py` writes it).
const DIMS: u64 = 768;

/// Bytes of one binary code: `words_per_code(768) × 8`.
const CODE_BYTES: u64 = 96;

/// External model configuration of `synth-small`, the same one the J1 demo
/// writes.
const MODEL_JSON: &str =
    r#"{ "name": "synthetic-external", "kind": "external", "dims": 768, "normalize": true }"#;

/// Creates the data root of the test and the sibling models volume it refers to.
///
/// The helpers of this file allow `clippy::panic` one by one: `clippy.toml`
/// covers `#[test]` functions only, and a helper that cannot prepare its fixture
/// must stop the test that called it.
#[allow(clippy::panic)]
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

/// Byte count printed by the `codes:` line of `inspect`, and the vector counts of
/// the `    <segment>: <n> vectors` lines of the same output.
///
/// Same reason as [`prepare_data`] for allowing `clippy::panic`: a malformed
/// output must stop the calling test.
#[allow(clippy::panic)]
fn codes_line(output: &str) -> (u64, Vec<u64>) {
    let mut codes = None;
    let mut counts = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("codes: ") {
            let value = value
                .strip_suffix(" bytes")
                .unwrap_or_else(|| panic!("the codes line must end in ' bytes': {line:?}"));
            let parsed = value
                .parse::<u64>()
                .unwrap_or_else(|error| panic!("the codes memory must be a number: {error}"));
            codes = Some(parsed);
        }
        if let Some(rest) = trimmed.split_once(": ")
            && rest.1.ends_with(" vectors")
        {
            let count = rest
                .1
                .trim_end_matches(" vectors")
                .parse::<u64>()
                .unwrap_or_else(|error| panic!("a segment count must be a number: {error}"));
            counts.push(count);
        }
    }
    let codes = codes.unwrap_or_else(|| panic!("inspect must print a 'codes:' line: {output}"));
    (codes, counts)
}

/// Runs `crible` and returns its standard output, failing on a non-zero exit.
fn run(arguments: &[&str]) -> String {
    let output = crible(arguments);
    assert!(
        output.status.success(),
        "crible {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Runs `inspect` on an index holding one attached batch, so the codes of some
/// passages are loaded.
fn inspect_one_batch(data: &Path, models: &Path, dataset: &Path) -> String {
    run(&[
        "index",
        "create",
        "--data",
        path(data),
        "--config",
        path(&dataset.join("index.json")),
    ]);
    run(&[
        "attach",
        "--data",
        path(data),
        "--models",
        path(models),
        "--index",
        "synth-small",
        path(&dataset.join("l1")),
    ]);
    run(&["inspect", "--data", path(data), "--index", "synth-small"])
}

#[test]
fn inspect_reports_codes_memory_matching_written_codes() {
    let dataset = synth_root();
    require_synth(&dataset);
    let (_temp, data, models) = prepare_data();

    let output = inspect_one_batch(&data, &models, &dataset);
    let (codes_memory, counts) = codes_line(&output);

    assert!(
        !counts.is_empty(),
        "the attached batch must leave at least one segment: {output}"
    );
    let expected: u64 = counts.iter().map(|count| count * CODE_BYTES).sum();
    assert!(
        expected > 0,
        "the attached batch must index some passages: {output}"
    );
    assert_eq!(
        codes_memory, expected,
        "inspect must print the RAM held by the codes, {CODE_BYTES} bytes per passage \
         of {DIMS} dimensions (segments: {counts:?})"
    );
}

/// Every `codes.bin` the attached index holds, for the test that removes them.
#[allow(clippy::panic)]
fn codes_files(data: &Path) -> Vec<PathBuf> {
    let epochs = data.join("indexes").join("synth-small").join("epochs");
    let mut paths = Vec::new();
    let entries = fs::read_dir(&epochs)
        .unwrap_or_else(|error| panic!("{} must be listable: {error}", epochs.display()));
    for entry in entries {
        let entry =
            entry.unwrap_or_else(|error| panic!("an epoch entry must be readable: {error}"));
        let segments = entry.path().join("segments");
        if !segments.is_dir() {
            continue;
        }
        let segments = fs::read_dir(&segments)
            .unwrap_or_else(|error| panic!("{} must be listable: {error}", segments.display()));
        for segment in segments {
            let segment =
                segment.unwrap_or_else(|error| panic!("a segment entry must be readable: {error}"));
            let codes = segment.path().join("codes.bin");
            if codes.is_file() {
                paths.push(codes);
            }
        }
    }
    paths
}

/// `inspect` is a read of the store (ADR-0056): it reports the codes memory of
/// an index whose `codes.bin` are missing, without writing them back
/// (`ADR-0069`, decision 2).
#[test]
fn inspect_does_not_write_a_missing_codes_file() {
    let dataset = synth_root();
    require_synth(&dataset);
    let (_temp, data, models) = prepare_data();

    let before = inspect_one_batch(&data, &models, &dataset);
    let (memory, counts) = codes_line(&before);

    let files = codes_files(&data);
    assert!(
        !files.is_empty(),
        "the attached batch must have written at least one codes.bin"
    );
    for file in &files {
        fs::remove_file(file).unwrap_or_else(|error| panic!("{}: {error}", file.display()));
    }

    let after = run(&["inspect", "--data", path(&data), "--index", "synth-small"]);
    let (memory_after, counts_after) = codes_line(&after);
    assert_eq!(
        memory_after, memory,
        "inspect must report the same codes memory whether the codes come from the file \
         or from the vectors"
    );
    assert_eq!(
        counts_after, counts,
        "inspect must report the same segments as before"
    );
    for file in &files {
        assert!(
            !file.exists(),
            "inspect must not rebuild {}: a read writes nothing under the epoch",
            file.display()
        );
    }
}

#[test]
fn inspect_reports_zero_codes_memory_for_an_index_without_segments() {
    let dataset = synth_root();
    require_synth(&dataset);
    let (_temp, data, _models) = prepare_data();

    run(&[
        "index",
        "create",
        "--data",
        path(&data),
        "--config",
        path(&dataset.join("index.json")),
    ]);
    let output = run(&["inspect", "--data", path(&data), "--index", "synth-small"]);
    let (codes_memory, counts) = codes_line(&output);

    assert!(
        counts.is_empty(),
        "a fresh index holds no segment: {output}"
    );
    assert_eq!(
        codes_memory, 0,
        "an index without a segment holds no code, and inspect must say so: {output}"
    );
}
