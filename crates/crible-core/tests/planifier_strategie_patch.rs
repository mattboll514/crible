// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `PLANIFIER-STRATEGIE` — the library-level `PATCH` of the settings the dense
//! strategy reads: `search.exact_threshold` and `search.scan`.
//!
//! References: `docs/phases/P4-recherche-rapide.md` (`PLANIFIER-STRATEGIE`,
//! phase P4), `docs/04-donnees-et-ingestion.md` ("Ce qui est figé, ce qui est
//! réglable" and the `search` object of the configuration), `docs/03-recherche.md`
//! ("Réglages de précision": `search.scan` is `binary` or `none`).
//!
//! # What `docs/04` settles, and what it does not
//!
//! `docs/04` puts `search.*` in the "réglable ensuite (`PATCH`)" column, so the
//! two settings must be accepted by a patch and must change the configuration.
//! It gives a **domain** only for `ingest.*` (`acceptance_sample ≥ 1`,
//! `acceptance_min_cosine ∈ ]0, 1]`, ADR-0054): `exact_threshold` has no
//! documented bound, so these tests pin only the values a *type* settles —
//! `u32`, so a negative number, a fraction and a value above the `u32` range are
//! refused, and a value inside the range is accepted, `0` included. Whether `0`
//! should be refused is a design question, not something this file decides.
//!
//! The two values of `search.scan` are settled by `docs/03`: `binary` and `none`.
//! Anything else is refused with `invalid_request` (400), like every invalid
//! value of the configuration.
//!
//! # What each test pins
//!
//! | Test | Criterion |
//! |---|---|
//! | `patch_accepts_the_two_values_of_search_scan` | `binary` and `none` are accepted |
//! | `patch_applies_exact_threshold_and_scan` | both settings change, and nothing else does |
//! | `patch_accepts_an_exact_threshold_of_zero` | `0` is inside the `u32` range `docs/04` leaves unbounded |
//! | `patch_refuses_an_unknown_scan_value` | `invalid_request`, message names `scan` |
//! | `patch_refuses_a_scan_that_is_not_a_string` | the type of `scan` is checked |
//! | `patch_refuses_a_negative_exact_threshold` | the type of `exact_threshold` is checked |
//! | `patch_refuses_a_fractional_exact_threshold` | idem |
//! | `patch_refuses_an_exact_threshold_above_the_u32_range` | idem |
//! | `a_refused_search_patch_leaves_the_configuration_unchanged` | a rejected patch mutates nothing |
//! | `patch_keeps_the_search_settings_it_does_not_name` | absent keys keep their value |

use crible_core::config::{IndexConfig, IndexPatch, ScanStrategy, apply_patch};
use crible_core::error::{CribleError, ErrorCode};

/// The index configuration of `docs/04-donnees-et-ingestion.md`, unchanged:
/// every setting that a search patch must leave alone comes from here.
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

/// The index configuration example of `docs/04`.
fn config() -> IndexConfig {
    IndexConfig::from_json_str(DECISIONS_CONFIG)
        .unwrap_or_else(|error| panic!("the example of docs/04 is valid: {error}"))
}

/// Parses a patch and applies it, whichever stage rejects it.
fn apply(config: &mut IndexConfig, raw: &str) -> Result<(), CribleError> {
    match IndexPatch::from_json_str(raw) {
        Ok(patch) => apply_patch(config, &patch),
        Err(error) => Err(error),
    }
}

/// Applies a patch that must be accepted.
fn apply_ok(config: &mut IndexConfig, raw: &str) {
    apply(config, raw).unwrap_or_else(|error| panic!("the patch {raw} must apply: {error}"));
}

/// Applies a patch that must be refused with `invalid_request`.
fn apply_invalid(config: &mut IndexConfig, raw: &str) -> CribleError {
    let error = apply(config, raw)
        .err()
        .unwrap_or_else(|| panic!("the patch {raw} must be refused"));
    assert_eq!(
        error.code(),
        ErrorCode::InvalidRequest,
        "the patch {raw} must be an invalid_request, got {}: {}",
        error.code().as_str(),
        error.message()
    );
    error
}

/// `docs/03-recherche.md`, "Réglages de précision": `search.scan` is `binary`
/// (the default) or `none` ("toujours exact"). Both must be accepted by a patch.
#[test]
fn patch_accepts_the_two_values_of_search_scan() {
    let mut config = config();

    apply_ok(&mut config, r#"{ "search": { "scan": "none" } }"#);
    assert_eq!(
        config.search.scan,
        ScanStrategy::None,
        "the patch must set scan to none"
    );

    apply_ok(&mut config, r#"{ "search": { "scan": "binary" } }"#);
    assert_eq!(
        config.search.scan,
        ScanStrategy::Binary,
        "the patch must set scan back to binary"
    );
}

/// Both settings are in the "réglable ensuite (`PATCH`)" column of `docs/04`:
/// a patch naming them changes them, and changes nothing else.
#[test]
fn patch_applies_exact_threshold_and_scan() {
    let mut config = config();

    apply_ok(
        &mut config,
        r#"{ "search": { "scan": "none", "exact_threshold": 250 } }"#,
    );

    assert_eq!(config.search.scan, ScanStrategy::None);
    assert_eq!(
        config.search.exact_threshold, 250,
        "the patch must set the threshold the strategy compares the pool with"
    );
    assert_eq!(
        config.search.rescore_candidates, 1000,
        "a patch that does not name N must keep the N of the configuration"
    );
    assert_eq!(
        config.search.candidates_per_branch, 100,
        "a patch that does not name it must keep the branch budget"
    );
    assert_eq!(
        config.name, "decisions",
        "a search patch must not touch the frozen settings"
    );
}

/// `docs/04` gives no domain for `search.exact_threshold`; the type is a `u32`,
/// so `0` is inside the accepted range. The test pins the current, documented
/// behaviour: refusing `0` would need a domain in `docs/04`, as `ingest.*` has
/// one (ADR-0054).
#[test]
fn patch_accepts_an_exact_threshold_of_zero() {
    let mut config = config();

    apply_ok(&mut config, r#"{ "search": { "exact_threshold": 0 } }"#);

    assert_eq!(
        config.search.exact_threshold, 0,
        "0 is a u32 and docs/04 sets no lower bound on exact_threshold"
    );
}

/// An unknown scan strategy is an invalid configuration, whatever the task that
/// would read it: `invalid_request` (400), and the message names the setting.
#[test]
fn patch_refuses_an_unknown_scan_value() {
    let mut config = config();

    let error = apply_invalid(&mut config, r#"{ "search": { "scan": "hamming" } }"#);

    assert!(
        error.message().contains("scan"),
        "the message must name the setting: {}",
        error.message()
    );
    assert!(
        error.message().contains("binary") && error.message().contains("none"),
        "the message must give the two accepted values: {}",
        error.message()
    );
}

/// `scan` is a string: a number or a boolean is a malformed request, not a
/// setting to coerce.
#[test]
fn patch_refuses_a_scan_that_is_not_a_string() {
    let mut config = config();

    for raw in [
        r#"{ "search": { "scan": 3 } }"#,
        r#"{ "search": { "scan": true } }"#,
        r#"{ "search": { "scan": null } }"#,
    ] {
        apply_invalid(&mut config, raw);
    }
}

/// `exact_threshold` is a `u32`: a negative number is refused, not wrapped.
#[test]
fn patch_refuses_a_negative_exact_threshold() {
    let mut config = config();

    let error = apply_invalid(&mut config, r#"{ "search": { "exact_threshold": -1 } }"#);

    assert!(
        error.message().contains("exact_threshold"),
        "the message must name the setting: {}",
        error.message()
    );
}

/// A fractional threshold has no meaning for a comparison with a pool size, so
/// it is refused instead of truncated.
#[test]
fn patch_refuses_a_fractional_exact_threshold() {
    let mut config = config();

    apply_invalid(&mut config, r#"{ "search": { "exact_threshold": 1.5 } }"#);
}

/// A value above `u32::MAX` cannot designate a pool size and is refused as a
/// malformed request, not saturated.
#[test]
fn patch_refuses_an_exact_threshold_above_the_u32_range() {
    let mut config = config();

    apply_invalid(
        &mut config,
        r#"{ "search": { "exact_threshold": 4294967296 } }"#,
    );
}

/// A refused patch changes nothing: the configuration keeps every value it had,
/// so the strategy the planner reads is the one before the attempt.
#[test]
fn a_refused_search_patch_leaves_the_configuration_unchanged() {
    let mut config = config();
    let before = config.clone();

    apply_invalid(
        &mut config,
        r#"{ "search": { "scan": "hamming", "exact_threshold": 12 } }"#,
    );

    assert_eq!(
        config, before,
        "a refused patch must leave the configuration untouched"
    );
}

/// `PATCH` is partial: the keys it does not name keep their value, so raising
/// the threshold alone does not silently change the scan or `N`.
#[test]
fn patch_keeps_the_search_settings_it_does_not_name() {
    let mut config = config();
    let fusion = config.search.fusion;

    apply_ok(&mut config, r#"{ "search": { "exact_threshold": 4000 } }"#);

    assert_eq!(config.search.exact_threshold, 4000);
    assert_eq!(
        config.search.scan,
        ScanStrategy::Binary,
        "the scan of the configuration must survive a patch that does not name it"
    );
    assert_eq!(
        config.search.fusion, fusion,
        "the fusion settings must survive a search patch"
    );
}
