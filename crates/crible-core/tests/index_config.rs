// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `IndexConfig`, `Schema`, `FieldType` and `FieldLevel`.
//!
//! References: `docs/04-donnees-et-ingestion.md`, section "Configuration d'un
//! index", the `DEFINIR-CONFIG-INDEX` task of `docs/phases/P0-fondations.md`,
//! and ADR-0054 (bounds of the acceptance settings).

use crible_core::config::{
    FieldLevel, FieldType, FusionConfig, IndexConfig, IndexPatch, ReferencePrecision, Schema,
    apply_patch,
};
use crible_core::error::ErrorCode;

/// The index configuration example of `docs/04-donnees-et-ingestion.md`, verbatim.
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
    "rescore_candidates": 5000,
    "exact_threshold": 5000,
    "candidates_per_branch": 100,
    "fusion": { "method": "rrf" }
  },
  "ingest":     { "acceptance_sample": 200, "acceptance_min_cosine": 0.999 },
  "compaction": { "dead_ratio": 0.10 }
}"#;

fn parse(raw: &str) -> IndexConfig {
    IndexConfig::from_json_str(raw)
        .unwrap_or_else(|error| panic!("the configuration must be valid: {error}"))
}

/// Replaces one top-level member of the documented example.
fn config_with(member: &str, value: &str) -> String {
    let mut value_json: serde_json::Value = serde_json::from_str(DECISIONS_CONFIG)
        .unwrap_or_else(|error| panic!("the example is valid JSON: {error}"));
    value_json[member] = serde_json::from_str(value)
        .unwrap_or_else(|error| panic!("the replacement is a valid JSON value: {error}"));
    serde_json::to_string(&value_json)
        .unwrap_or_else(|error| panic!("the configuration serializes: {error}"))
}

#[test]
fn index_config_doc_example_is_parsed() {
    let config = parse(DECISIONS_CONFIG);

    assert_eq!(config.name, "decisions");
    assert_eq!(config.model, "mdenseon");
    assert_eq!(config.reference_precision, ReferencePrecision::Fp16);
    assert_eq!(config.cluster_by, ["juridiction"]);

    assert!(config.lexical.enabled);
    assert_eq!(config.lexical.language, "fr");
    assert!(!config.lexical.positions);

    assert_eq!(config.search.rescore_candidates, 5000);
    assert_eq!(config.search.exact_threshold, 5000);
    assert_eq!(config.search.candidates_per_branch, 100);
    assert_eq!(config.search.fusion, FusionConfig::Rrf);

    assert_eq!(config.ingest.acceptance_sample, 200);
    assert_eq!(config.ingest.acceptance_min_cosine, 0.999);

    assert_eq!(config.compaction.dead_ratio, 0.1);
}

#[test]
fn index_config_doc_example_declares_typed_fields_at_both_levels() {
    let config = parse(DECISIONS_CONFIG);

    let (juridiction, level) = config
        .schema
        .field("juridiction")
        .expect("juridiction is declared");
    assert_eq!(juridiction.field_type, FieldType::Keyword);
    assert_eq!(level, FieldLevel::Document);

    let (date, _) = config
        .schema
        .field("date_decision")
        .expect("date_decision is declared");
    assert_eq!(date.field_type, FieldType::Date);

    let (partie, level) = config.schema.field("partie").expect("partie is declared");
    assert_eq!(partie.field_type, FieldType::Keyword);
    assert_eq!(level, FieldLevel::Passage);

    assert!(
        config.schema.field("inconnu").is_none(),
        "an undeclared field is not part of the schema"
    );
}

#[test]
fn schema_field_options_default_to_filterable_and_returned() {
    let config = parse(DECISIONS_CONFIG);

    let (partie, _) = config.schema.field("partie").expect("partie is declared");
    assert!(partie.filterable, "filterable defaults to true");
    assert!(partie.returned, "returned defaults to true");

    let (titre, _) = config.schema.field("titre").expect("titre is declared");
    assert!(
        !titre.filterable,
        "the example disables filterable on titre"
    );
    assert!(titre.returned, "returned stays at its default");
}

#[test]
fn field_type_parses_every_documented_type() {
    let schema = Schema::from_json_str(
        r#"{
          "document_fields": {
            "k": { "type": "keyword" },
            "i": { "type": "integer" },
            "f": { "type": "float" },
            "d": { "type": "date" },
            "b": { "type": "bool" }
          },
          "passage_fields": {}
        }"#,
    )
    .expect("the five documented types are valid");

    for (name, expected) in [
        ("k", FieldType::Keyword),
        ("i", FieldType::Integer),
        ("f", FieldType::Float),
        ("d", FieldType::Date),
        ("b", FieldType::Bool),
    ] {
        let (spec, _) = schema.field(name).expect("the field is declared");
        assert_eq!(spec.field_type, expected, "wrong type for field {name}");
    }
}

#[test]
fn field_type_rejects_an_unknown_type() {
    let error = Schema::from_json_str(
        r#"{
          "document_fields": { "corps": { "type": "text" } },
          "passage_fields": {}
        }"#,
    )
    .expect_err("`text` is not a documented field type");

    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    let message = error.message();
    assert!(
        message.contains("text"),
        "the message names the type: {message}"
    );
    assert!(
        message.contains("keyword") && message.contains("date"),
        "the message names the expected types: {message}"
    );
}

#[test]
fn schema_rejects_a_field_declared_at_both_levels() {
    let error = Schema::from_json_str(
        r#"{
          "document_fields": { "partie": { "type": "keyword" } },
          "passage_fields": { "partie": { "type": "keyword" } }
        }"#,
    )
    .expect_err("a field name must be unambiguous");

    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(
        error.message().contains("partie"),
        "the message names the duplicated field: {}",
        error.message()
    );
}

#[test]
fn index_config_rejects_an_unknown_reference_precision() {
    let error = IndexConfig::from_json_str(&config_with("reference_precision", "\"fp8\""))
        .expect_err("only fp16 and fp32 are documented");

    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    let message = error.message();
    assert!(
        message.contains("fp8"),
        "the message names the value: {message}"
    );
    assert!(
        message.contains("fp16") && message.contains("fp32"),
        "the message names the accepted precisions: {message}"
    );
}

#[test]
fn index_config_rejects_an_empty_name() {
    let error =
        IndexConfig::from_json_str(&config_with("name", "\"\"")).expect_err("a name is required");

    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(
        error.message().contains("name"),
        "the message names the setting: {}",
        error.message()
    );
}

#[test]
fn index_config_rejects_malformed_json() {
    let error = IndexConfig::from_json_str("{ \"name\": ").expect_err("truncated JSON is invalid");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
}

/// ADR-0054: the acceptance settings are bounded at creation, `invalid_request`.
#[test]
fn index_config_rejects_an_unbounded_acceptance_setting() {
    for (sample, min_cosine) in [(0_u32, 0.999_f64), (200, 0.0), (200, -0.1), (200, 1.000_1)] {
        let raw = config_with(
            "ingest",
            &format!(
                r#"{{ "acceptance_sample": {sample}, "acceptance_min_cosine": {min_cosine} }}"#
            ),
        );

        let error = IndexConfig::from_json_str(&raw)
            .expect_err("a sample of zero or a threshold outside ]0, 1] is refused (ADR-0054)");

        assert_eq!(error.code(), ErrorCode::InvalidRequest);
    }
}

/// ADR-0054: the bounds themselves are accepted: one passage and a threshold
/// of 1, and the two ends of the open/closed interval.
#[test]
fn index_config_accepts_the_bounds_of_the_acceptance_settings() {
    for (sample, min_cosine) in [(1_u32, 1.0_f64), (200, 0.000_1)] {
        let raw = config_with(
            "ingest",
            &format!(
                r#"{{ "acceptance_sample": {sample}, "acceptance_min_cosine": {min_cosine} }}"#
            ),
        );

        let config = parse(&raw);

        assert_eq!(config.ingest.acceptance_sample, sample);
        assert_eq!(config.ingest.acceptance_min_cosine, min_cosine);
    }
}

/// ADR-0054: a `PATCH` cannot smuggle a value that creation refuses; the
/// rejected patch changes nothing.
#[test]
fn index_patch_rejects_an_unbounded_acceptance_setting() {
    for patch in [
        r#"{ "ingest": { "acceptance_sample": 0 } }"#,
        r#"{ "ingest": { "acceptance_min_cosine": 0 } }"#,
        r#"{ "ingest": { "acceptance_min_cosine": -0.5 } }"#,
        r#"{ "ingest": { "acceptance_min_cosine": 1.5 } }"#,
    ] {
        let original = parse(DECISIONS_CONFIG);
        let mut config = original.clone();
        let patch = IndexPatch::from_json_str(patch)
            .unwrap_or_else(|error| panic!("the patch is well-formed JSON: {error}"));

        let error = apply_patch(&mut config, &patch)
            .expect_err("an unbounded acceptance setting is refused (ADR-0054)");

        assert_eq!(error.code(), ErrorCode::InvalidRequest);
        assert_eq!(
            config, original,
            "a rejected patch must leave the configuration untouched"
        );
    }
}

#[test]
fn fusion_relative_score_accepts_an_alpha() {
    let config = IndexConfig::from_json_str(&config_with(
        "search",
        r#"{ "scan": "binary", "rescore_candidates": 1000, "exact_threshold": 1000,
            "candidates_per_branch": 100,
            "fusion": { "method": "relative_score", "alpha": 0.5 } }"#,
    ))
    .expect("relative_score with alpha is valid");

    assert_eq!(
        config.search.fusion,
        FusionConfig::RelativeScore { alpha: 0.5 }
    );
}

#[test]
fn fusion_relative_score_requires_an_alpha() {
    let error = IndexConfig::from_json_str(&config_with(
        "search",
        r#"{ "scan": "binary", "rescore_candidates": 1000, "exact_threshold": 1000,
            "candidates_per_branch": 100, "fusion": { "method": "relative_score" } }"#,
    ))
    .expect_err("relative_score needs a weight");

    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(
        error.message().contains("alpha"),
        "the message names the missing member: {}",
        error.message()
    );
}

#[test]
fn fusion_rejects_an_unknown_method() {
    let error = IndexConfig::from_json_str(&config_with(
        "search",
        r#"{ "scan": "binary", "rescore_candidates": 1000, "exact_threshold": 1000,
            "candidates_per_branch": 100, "fusion": { "method": "linear" } }"#,
    ))
    .expect_err("only rrf and relative_score are documented");

    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    let message = error.message();
    assert!(
        message.contains("linear"),
        "the message names the value: {message}"
    );
    assert!(
        message.contains("rrf") && message.contains("relative_score"),
        "the message names the accepted methods: {message}"
    );
}
