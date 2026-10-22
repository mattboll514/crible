// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! INV-16 — a frozen setting cannot be changed through the API.
//!
//! References: `docs/08-invariants-et-tests.md` (INV-16),
//! `docs/04-donnees-et-ingestion.md` ("Ce qui est figé, ce qui est réglable")
//! and the `DEFINIR-CONFIG-INDEX` task of `docs/phases/P0-fondations.md`.

mod common;

use crible_core::config::{IndexConfig, IndexPatch, apply_patch};
use crible_core::error::{CribleError, ErrorCode};
use proptest::sample::select;
use serde_json::{Value, json};

/// The index configuration example of `docs/04-donnees-et-ingestion.md`.
const DECISIONS_CONFIG: &str = r#"{
  "name": "decisions",
  "model": "mdenseon",
  "reference_precision": "fp16",
  "schema": {
    "document_fields": {
      "juridiction":   { "type": "keyword" },
      "formation":     { "type": "keyword" },
      "date_decision": { "type": "date" },
      "numero":        { "type": "keyword" },
      "titre":         { "type": "keyword", "filterable": false }
    },
    "passage_fields": {
      "partie": { "type": "keyword" }
    }
  },
  "cluster_by": ["juridiction"],
  "lexical":    { "enabled": true, "language": "fr", "positions": false },
  "search": {
    "scan": "binary",
    "rescore_candidates": 1000,
    "exact_threshold": 1000,
    "candidates_per_branch": 100,
    "fusion": { "method": "rrf" }
  },
  "ingest":     { "acceptance_sample": 200, "acceptance_min_cosine": 0.999 },
  "compaction": { "dead_ratio": 0.10 }
}"#;

fn config() -> IndexConfig {
    IndexConfig::from_json_str(DECISIONS_CONFIG)
        .unwrap_or_else(|error| panic!("the example is valid: {error}"))
}

/// Parses a patch and applies it, whichever stage rejects it.
fn apply(config: &mut IndexConfig, raw: &str) -> Result<(), CribleError> {
    match IndexPatch::from_json_str(raw) {
        Ok(patch) => apply_patch(config, &patch),
        Err(error) => Err(error),
    }
}

fn assert_immutable(raw: &str, needles: &[&str]) -> CribleError {
    let mut config = config();
    let error = match apply(&mut config, raw) {
        Ok(()) => panic!("the patch must be rejected: {raw}"),
        Err(error) => error,
    };

    assert_eq!(
        error.code(),
        ErrorCode::ImmutableSetting,
        "wrong error code for {raw}: {}",
        error.message()
    );
    assert_eq!(error.code().http_status(), 409);
    for needle in needles {
        assert!(
            error.message().contains(needle),
            "message {:?} must contain {needle:?} for {raw}",
            error.message()
        );
    }
    error
}

/// Asserts that a patch is refused as an invalid request, leaves the
/// configuration untouched, and names every needle.
fn assert_invalid(raw: &str, needles: &[&str]) -> CribleError {
    let mut config = config();
    let before = config.clone();
    let error = match apply(&mut config, raw) {
        Ok(()) => panic!("the patch must be rejected: {raw}"),
        Err(error) => error,
    };

    assert_eq!(
        error.code(),
        ErrorCode::InvalidRequest,
        "wrong error code for {raw}: {}",
        error.message()
    );
    for needle in needles {
        assert!(
            error.message().contains(needle),
            "message {:?} must contain {needle:?} for {raw}",
            error.message()
        );
    }
    assert_eq!(
        config, before,
        "no member of a rejected patch may be applied"
    );
    error
}

#[test]
fn inv_16_renaming_the_index_is_rejected() {
    assert_immutable(r#"{ "name": "decisions-v2" }"#, &["name"]);
}

#[test]
fn inv_16_changing_the_model_is_rejected() {
    assert_immutable(r#"{ "model": "other-model" }"#, &["model"]);
}

#[test]
fn inv_16_changing_the_reference_precision_is_rejected() {
    assert_immutable(
        r#"{ "reference_precision": "fp32" }"#,
        &["reference_precision"],
    );
}

#[test]
fn inv_16_changing_the_type_of_an_existing_field_is_rejected() {
    assert_immutable(
        r#"{ "schema": { "document_fields": { "titre": { "type": "integer" } } } }"#,
        &["titre", "type"],
    );
}

#[test]
fn apply_patch_rejects_a_field_added_at_both_levels() {
    // ADR-0030 and `docs/04`: a field name designates one level. Adding
    // `titre` at passage level while it is a document field is a 400, not a
    // frozen-setting 409.
    assert_invalid(
        r#"{ "schema": { "passage_fields": { "titre": { "type": "keyword" } } } }"#,
        &["titre", "document_fields", "passage_fields"],
    );
}

#[test]
fn apply_patch_rejects_a_passage_field_added_at_document_level() {
    assert_invalid(
        r#"{ "schema": { "document_fields": { "partie": { "type": "keyword" } } } }"#,
        &["partie", "document_fields", "passage_fields"],
    );
}

#[test]
fn inv_16_a_rejected_patch_changes_nothing() {
    let mut config = config();
    let before = config.clone();

    let error = apply(
        &mut config,
        r#"{ "search": { "rescore_candidates": 2000 }, "name": "decisions-v2" }"#,
    )
    .expect_err("a patch with a frozen setting is rejected as a whole");

    assert_eq!(error.code(), ErrorCode::ImmutableSetting);
    assert_eq!(
        config, before,
        "no member of a rejected patch may be applied"
    );
}

#[test]
fn inv_16_every_immutable_top_level_setting_is_rejected() {
    let immutable: [(&str, Value); 3] = [
        ("name", json!("decisions-v2")),
        ("model", json!("other-model")),
        ("reference_precision", json!("fp32")),
    ];

    common::check_property(select(immutable.to_vec()), |(key, value)| {
        let mut body = serde_json::Map::new();
        // A mutable setting rides along: the frozen one must still win.
        body.insert("search".to_string(), json!({ "rescore_candidates": 42 }));
        body.insert(key.to_string(), value);

        let mut config = config();
        let error = match apply(&mut config, &Value::Object(body).to_string()) {
            Ok(()) => {
                return Err(proptest::test_runner::TestCaseError::fail(format!(
                    "patching {key} must be rejected"
                )));
            }
            Err(error) => error,
        };
        if error.code() != ErrorCode::ImmutableSetting {
            return Err(proptest::test_runner::TestCaseError::fail(format!(
                "patching {key} must give immutable_setting, got {:?}",
                error.code()
            )));
        }
        if !error.message().contains(key) {
            return Err(proptest::test_runner::TestCaseError::fail(format!(
                "the message must name {key}: {}",
                error.message()
            )));
        }
        if config.search.rescore_candidates != 1000 {
            return Err(proptest::test_runner::TestCaseError::fail(
                "a rejected patch must not change a mutable setting",
            ));
        }
        Ok(())
    });
}

#[test]
fn apply_patch_updates_search_settings() {
    let mut config = config();
    apply(
        &mut config,
        r#"{ "search": { "rescore_candidates": 2000, "exact_threshold": 5000,
                         "fusion": { "method": "relative_score", "alpha": 0.3 } } }"#,
    )
    .expect("search settings are mutable");

    assert_eq!(config.search.rescore_candidates, 2000);
    assert_eq!(config.search.exact_threshold, 5000);
    assert_eq!(
        config.search.fusion,
        crible_core::config::FusionConfig::RelativeScore { alpha: 0.3 }
    );
    assert_eq!(
        config.search.candidates_per_branch, 100,
        "a partial patch keeps the untouched members"
    );
}

#[test]
fn apply_patch_updates_lexical_settings() {
    let mut config = config();
    apply(
        &mut config,
        r#"{ "lexical": { "enabled": false, "language": "en", "positions": true } }"#,
    )
    .expect("lexical settings are mutable");

    assert!(!config.lexical.enabled);
    assert_eq!(config.lexical.language, "en");
    assert!(config.lexical.positions);
}

#[test]
fn apply_patch_updates_cluster_by() {
    let mut config = config();
    apply(
        &mut config,
        r#"{ "cluster_by": ["juridiction", "formation"] }"#,
    )
    .expect("cluster_by is mutable");

    assert_eq!(config.cluster_by, ["juridiction", "formation"]);
}

#[test]
fn apply_patch_updates_ingest_and_compaction() {
    let mut config = config();
    apply(
        &mut config,
        r#"{ "ingest": { "acceptance_sample": 500 }, "compaction": { "dead_ratio": 0.2 } }"#,
    )
    .expect("ingest and compaction settings are mutable");

    assert_eq!(config.ingest.acceptance_sample, 500);
    assert_eq!(
        config.ingest.acceptance_min_cosine, 0.999,
        "a partial patch keeps the untouched members"
    );
    assert_eq!(config.compaction.dead_ratio, 0.2);
}

#[test]
fn apply_patch_updates_filterable_and_returned() {
    let mut config = config();
    apply(
        &mut config,
        r#"{ "schema": { "document_fields": { "titre": { "filterable": true, "returned": false } } } }"#,
    )
    .expect("filterable and returned are mutable");

    let (titre, _) = config.schema.field("titre").expect("titre is declared");
    assert!(titre.filterable);
    assert!(!titre.returned);
}

#[test]
fn apply_patch_adds_a_nullable_field() {
    let mut config = config();
    apply(
        &mut config,
        r#"{ "schema": { "document_fields": { "source": { "type": "keyword" } } } }"#,
    )
    .expect("a new field may be added");

    let (source, level) = config.schema.field("source").expect("source is declared");
    assert_eq!(level, crible_core::config::FieldLevel::Document);
    assert_eq!(source.field_type, crible_core::config::FieldType::Keyword);
    assert!(source.filterable, "filterable defaults to true");
    assert!(source.returned, "returned defaults to true");
}

#[test]
fn apply_patch_rejects_a_new_field_without_a_type() {
    let mut config = config();
    let error = apply(
        &mut config,
        r#"{ "schema": { "document_fields": { "source": {} } } }"#,
    )
    .expect_err("a new field needs a type");

    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(
        error.message().contains("source") && error.message().contains("type"),
        "the message names the field and the missing member: {}",
        error.message()
    );
}

#[test]
fn apply_patch_rejects_an_unknown_setting() {
    let mut config = config();
    let error = apply(&mut config, r#"{ "colour": "red" }"#)
        .expect_err("an unknown setting is not silently ignored");

    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(
        error.message().contains("colour"),
        "the message names the unknown setting: {}",
        error.message()
    );
}

#[test]
fn apply_patch_rejects_an_unknown_nested_setting() {
    // ADR-0031: the rule applies at every level, and the message gives the
    // full JSON path so a typo is easy to locate.
    assert_invalid(
        r#"{ "search": { "rescore_candidate": 1 } }"#,
        &["rescore_candidate", "$.search.rescore_candidate"],
    );
}

#[test]
fn apply_patch_rejects_malformed_json() {
    let mut config = config();
    let error = apply(&mut config, "{ \"search\": ").expect_err("truncated JSON is invalid");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
}
