// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixtures of this file are helpers.
#![allow(clippy::panic)]

//! ADR-0134 — the step boundaries of the write protocol, as `crible-store`
//! reports them (ADR-0134, "À surveiller").
//!
//! The job routes only expose the **last** published progression, so the order
//! of the steps and the constancy of `total` can only be proved here, with a
//! recording closure handed to the public entry points
//! [`attach_with_progress`](crible_store::attach::attach_with_progress) and
//! [`apply_with_progress`](crible_store::patch::apply_with_progress).
//!
//! | Test | What it pins |
//! |---|---|
//! | `an_attach_that_changes_text_publishes_the_six_steps_in_order` | the attach runs the six steps of `docs/02`, in order, always with the passages of its manifest as `total` |
//! | `an_attach_that_changes_no_text_publishes_no_segment_and_no_lexical` | a batch that adds no passage text runs neither `segment` nor `lexical`, and still reports the other four steps |
//! | `a_patch_publishes_transaction_then_manifest_then_snapshot` | a patch runs exactly `transaction`, `manifest`, `snapshot`, with its document count as `total`, and never `lexical` |
//!
//! ```text
//! cargo test -p crible-store --test progress_sequence
//! ```

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use crible_core::config::{IndexConfig, Schema};
use crible_core::model::ModelConfig;
use crible_store::attach::attach_with_progress;
use crible_store::generation::{GenerationStore, ProtocolStep};
use crible_store::patch::{self, DocumentPatch, PatchSource};
use serde_json::{Map, Value};

/// One reported boundary: the step and the `total` the callback carried.
type Reported = (ProtocolStep, u64);

/// External model of the fixture batch: four dimensions, normalized vectors.
fn fixture_model() -> ModelConfig {
    ModelConfig::from_json_str(
        r#"{ "name": "progress-sequence-external", "kind": "external", "dims": 4, "normalize": true }"#,
    )
    .unwrap_or_else(|error| panic!("the fixture model configuration must be valid: {error}"))
}

/// Reads the configuration the index declares, which the attach takes as an
/// argument.
fn index_config_of(index: &Path) -> IndexConfig {
    let raw = std::fs::read_to_string(index.join("index.json"))
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", index.display()));
    IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("the index configuration must be valid: {error}"))
}

/// Attaches `batch` and returns the sequence its progress callback recorded.
fn attach_recording(store: &mut GenerationStore, index: &Path, batch: &Path) -> Vec<Reported> {
    let config = index_config_of(index);
    let model = fixture_model();
    let mut reported = Vec::new();
    attach_with_progress(
        store,
        batch,
        &config,
        &model,
        None,
        None,
        &mut |step, total| reported.push((step, total)),
    )
    .unwrap_or_else(|error| {
        panic!(
            "the fixture attach must succeed: {}: {}",
            error.code().as_str(),
            error.message()
        )
    });
    reported
}

/// Applies one patch of `doc-a` and returns the sequence its callback recorded.
fn patch_recording(store: &mut GenerationStore, schema: &Schema) -> Vec<Reported> {
    let mut fields = Map::new();
    fields.insert("title".to_owned(), Value::from("patched"));
    let patches = [DocumentPatch {
        doc_id: "doc-a".to_owned(),
        fields,
    }];
    let mut reported = Vec::new();
    patch::apply_with_progress(
        store,
        schema,
        PatchSource::Body,
        &patches,
        &mut |step, total| reported.push((step, total)),
    )
    .unwrap_or_else(|error| {
        panic!(
            "the fixture patch must succeed: {}: {}",
            error.code().as_str(),
            error.message()
        )
    });
    reported
}

/// Asserts that every step of `reported` carries the same `total`, and returns
/// it.
fn assert_one_total(reported: &[Reported]) -> u64 {
    let totals: BTreeMap<u64, usize> = reported.iter().fold(BTreeMap::new(), |mut seen, entry| {
        *seen.entry(entry.1).or_default() += 1;
        seen
    });
    assert_eq!(
        totals.len(),
        1,
        "every step of one job carries the same total, got {reported:?}"
    );
    reported.first().map_or(0, |entry| entry.1)
}

/// The steps of `reported`, without their totals, for the order assertions.
fn steps(reported: &[Reported]) -> Vec<ProtocolStep> {
    reported.iter().map(|entry| entry.0).collect()
}

/// An attach that writes two new passages runs the six steps of `docs/02` in
/// order, and each of them carries the two passages of the batch as `total`.
#[test]
fn an_attach_that_changes_text_publishes_the_six_steps_in_order() {
    let (temp, index) = common::temp_index();
    let schema = common::fixture_schema();
    let config = common::test_config_with_lexical("test", &schema, true);
    let mut store = common::create_store_with_config(&index, &schema, &config);
    let batch = common::write_batch(
        &temp.path().join("batches"),
        "progress-six-steps",
        &common::TWO_DOCUMENTS,
    );

    let reported = attach_recording(&mut store, &index, &batch);
    assert_eq!(
        steps(&reported),
        vec![
            ProtocolStep::Validation,
            ProtocolStep::Segment,
            ProtocolStep::Transaction,
            ProtocolStep::Lexical,
            ProtocolStep::Manifest,
            ProtocolStep::Snapshot,
        ],
        "the attach runs the six steps of the protocol, in order: {reported:?}"
    );
    assert_eq!(
        assert_one_total(&reported),
        2,
        "the total is the number of passages of the manifest: {reported:?}"
    );
}

/// A batch that adds no passage text — the two documents are rewritten with the
/// same digest and fields — runs neither `segment` nor `lexical`, and still
/// reports the other four steps of the protocol, in order.
#[test]
fn an_attach_that_changes_no_text_publishes_no_segment_and_no_lexical() {
    let (temp, index) = common::temp_index();
    let schema = common::fixture_schema();
    let config = common::test_config_with_lexical("test", &schema, true);
    let mut store = common::create_store_with_config(&index, &schema, &config);
    let batches = temp.path().join("batches");
    let first = common::write_batch(&batches, "progress-first", &common::TWO_DOCUMENTS);
    attach_recording(&mut store, &index, &first);

    // A distinct `batch_id` is a distinct batch, not a replay: the attach
    // commits a generation whose passages are identical to the stored ones.
    let same = common::write_batch(&batches, "progress-same-text", &common::TWO_DOCUMENTS);
    let reported = attach_recording(&mut store, &index, &same);
    assert_eq!(
        steps(&reported),
        vec![
            ProtocolStep::Validation,
            ProtocolStep::Transaction,
            ProtocolStep::Manifest,
            ProtocolStep::Snapshot,
        ],
        "a batch that adds no passage text writes no segment and runs no lexical \
         commit: {reported:?}"
    );
    assert_eq!(
        assert_one_total(&reported),
        2,
        "the total is still the number of passages of the manifest: {reported:?}"
    );
}

/// A patch publishes `transaction`, `manifest` and `snapshot` in that order,
/// with the number of documents of its body as `total`, and never publishes
/// `lexical` (ADR-0134, decision 1).
#[test]
fn a_patch_publishes_transaction_then_manifest_then_snapshot() {
    let (temp, index) = common::temp_index();
    let schema = common::fixture_schema();
    let config = common::test_config_with_lexical("test", &schema, true);
    let mut store = common::create_store_with_config(&index, &schema, &config);
    let batch = common::write_batch(
        &temp.path().join("batches"),
        "progress-patch-source",
        &common::TWO_DOCUMENTS,
    );
    attach_recording(&mut store, &index, &batch);

    let reported = patch_recording(&mut store, &schema);
    assert_eq!(
        steps(&reported),
        vec![
            ProtocolStep::Transaction,
            ProtocolStep::Manifest,
            ProtocolStep::Snapshot,
        ],
        "a metadata patch runs the three steps of `docs/02`, in order, and no \
         other: {reported:?}"
    );
    assert_eq!(
        assert_one_total(&reported),
        1,
        "the total is the number of documents of the patch: {reported:?}"
    );
}
