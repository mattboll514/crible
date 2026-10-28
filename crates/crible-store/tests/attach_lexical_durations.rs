// SPDX-License-Identifier: Apache-2.0

//! `docs/04` — the lexical step of an attach is measured on its own (`ADR-0087`).
//!
//! References: `docs/02-architecture.md` ("Générations et point de commit",
//! step 4), `docs/04-donnees-et-ingestion.md` (measured steps: `validation`,
//! `segment`, `transaction`, `lexical`, `manifest`, `snapshot`), `ADR-0042` (the
//! durations are call telemetry, never persisted), `ADR-0087` (decision 5) and
//! `ADR-0088` (the branch runs only when a generation touches a passage).
//!
//! The attribution is proved by a **contrast**, not by a stopwatch: the same
//! field carries a duration when step 4 commits tantivy, and exactly zero when
//! the branch is disabled, while the steps around it stay measured. A field that
//! covered the window after the commit point would be non-zero in both cases.
//!
//! | Test | What it pins |
//! |---|---|
//! | `the_lexical_step_of_an_attach_is_measured` | branch enabled: the tantivy commit and its reader reload are reported |
//! | `the_lexical_step_is_zero_when_the_branch_is_disabled` | no tantivy commit: zero, while `snapshot` and `manifest` stay measured |
//!
//! ```text
//! cargo test -p crible-store --test attach_lexical_durations
//! ```

// The fixtures of this file call `panic!` outside the `#[test]` functions, as
// the other integration tests do; see `tests/common/mod.rs`.
#![allow(clippy::panic)]

mod common;

use std::fs;
use std::path::Path;

use crible_core::config::IndexConfig;
use crible_core::model::ModelConfig;
use crible_store::attach::{AttachDurations, attach};
use crible_store::generation::GenerationStore;

/// External model of the fixture batch: four dimensions, normalized vectors.
fn fixture_model() -> ModelConfig {
    ModelConfig::from_json_str(
        r#"{ "name": "lexical-durations-external", "kind": "external", "dims": 4, "normalize": true }"#,
    )
    .unwrap_or_else(|error| panic!("the fixture model configuration must be valid: {error}"))
}

/// Reads the configuration the index declares, which `attach` takes as an
/// argument.
fn index_config_of(index: &Path) -> IndexConfig {
    let raw = fs::read_to_string(index.join("index.json")).unwrap_or_else(|error| {
        panic!(
            "the index configuration of {} must be readable: {error}",
            index.display()
        )
    });
    IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("the index configuration must be valid: {error}"))
}

/// Attaches a batch and returns the durations of the call, which the report
/// carries as telemetry (ADR-0042).
fn attach_batch(store: &mut GenerationStore, index: &Path, batch: &Path) -> AttachDurations {
    attach(
        store,
        batch,
        &index_config_of(index),
        &fixture_model(),
        None,
        None,
    )
    .unwrap_or_else(|error| {
        panic!(
            "the fixture attach must succeed: {}: {}",
            error.code().as_str(),
            error.message()
        )
    })
    .durations
    .unwrap_or_else(|| {
        panic!("the caller of the attach must receive the durations of its own call")
    })
}

/// Prints every duration of a report, for the failure messages.
fn format_durations(durations: AttachDurations) -> String {
    format!(
        "validation={:?} segment={:?} transaction={:?} lexical={:?} manifest={:?} snapshot={:?}",
        durations.validation,
        durations.segment,
        durations.transaction,
        durations.lexical,
        durations.manifest,
        durations.snapshot
    )
}

/// Branch enabled, an attach that indexes two passages: the field carries the
/// tantivy commit and its reader reload, and it is reported beside the other
/// steps of the same call.
///
/// The assertion is not a stopwatch: a tantivy commit writes and syncs segment
/// files and `meta.json`, which cannot fit in the microsecond the report
/// truncates to. The zero case below pins what the field does not cover.
#[test]
fn the_lexical_step_of_an_attach_is_measured() {
    let (temp, index) = common::temp_index();
    let schema = common::fixture_schema();
    let config = common::test_config_with_lexical("test", &schema, true);
    let mut store = common::create_store_with_config(&index, &schema, &config);
    let batch = common::write_batch(
        &temp.path().join("batches"),
        "lexical-durations-enabled",
        &common::TWO_DOCUMENTS,
    );

    let durations = attach_batch(&mut store, &index, &batch);
    assert!(
        !durations.lexical.is_zero(),
        "the attach indexes two passages, so step 4 commits tantivy and its duration must be \
         reported, got {}",
        format_durations(durations)
    );
    assert!(
        !durations.snapshot.is_zero(),
        "the snapshot step of the same attach must stay measured, got {}",
        format_durations(durations)
    );
}

/// Branch disabled: no tantivy commit happens at all, so the field is zero while
/// the steps around it stay measured. A field that covered the window after the
/// commit point would be non-zero here.
#[test]
fn the_lexical_step_is_zero_when_the_branch_is_disabled() {
    let (temp, index) = common::temp_index();
    let mut store = common::create_store(&index);
    let batch = common::write_batch(
        &temp.path().join("batches"),
        "lexical-disabled-durations",
        &common::TWO_DOCUMENTS,
    );

    let durations = attach_batch(&mut store, &index, &batch);
    assert!(
        durations.lexical.is_zero(),
        "a configuration that disables the lexical branch runs no tantivy commit, so its \
         duration must be exactly zero, got {:?}",
        durations.lexical
    );
    assert!(
        !durations.snapshot.is_zero() && !durations.manifest.is_zero(),
        "the other steps must stay measured, otherwise the zero above proves nothing, got {}",
        format_durations(durations)
    );
}
