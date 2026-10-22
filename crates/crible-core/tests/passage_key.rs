// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! Analysis and display of `PassageKey`, the `doc_id#ord` passage key.
//!
//! References: `docs/02-architecture.md` ("Identifiants") and the
//! `DEFINIR-TYPES-DE-BASE` task of `docs/phases/P0-fondations.md`.

mod common;

use std::str::FromStr;

use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::{DocId, PassageKey};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;

fn doc_id(raw: &str) -> DocId {
    DocId::new(raw).unwrap_or_else(|error| panic!("test id {raw:?} is invalid: {error}"))
}

fn parse(raw: &str) -> Result<PassageKey, CribleError> {
    PassageKey::from_str(raw)
}

#[test]
fn passage_key_display_joins_doc_id_and_ord_with_hash() {
    let key = PassageKey::new(doc_id("LEGIARTI000006419280"), 12);
    assert_eq!(key.to_string(), "LEGIARTI000006419280#12");
}

#[test]
fn passage_key_exposes_its_parts() {
    let key = PassageKey::new(doc_id("D1"), 7);
    assert_eq!(key.doc_id().as_str(), "D1");
    assert_eq!(key.ord(), 7);
}

#[test]
fn passage_key_parses_a_well_formed_key() {
    let key = parse("LEGIARTI000006419280#3").expect("a well formed key parses");
    assert_eq!(key.doc_id().as_str(), "LEGIARTI000006419280");
    assert_eq!(key.ord(), 3);
}

#[test]
fn passage_key_parses_ord_zero() {
    // Ordinals start at 0 (`docs/02-architecture.md`).
    let key = parse("D1#0").expect("ord 0 is valid");
    assert_eq!(key.ord(), 0);
}

#[test]
fn passage_key_parses_the_largest_ord() {
    let key = parse("D1#4294967295").expect("u32::MAX is a valid ord");
    assert_eq!(key.ord(), u32::MAX);
}

#[test]
fn passage_key_splits_on_the_last_hash_separator() {
    // A `doc_id` may contain `#`; only the last separator carries the ordinal,
    // otherwise the display/parse round trip could not hold for such ids.
    let key = parse("2024#12#3").expect("a doc id containing `#` is valid");
    assert_eq!(key.doc_id().as_str(), "2024#12");
    assert_eq!(key.ord(), 3);
}

#[test]
fn passage_key_rejects_an_empty_string() {
    let error = parse("").expect_err("an empty key is invalid");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
}

#[test]
fn passage_key_rejects_a_missing_separator() {
    let error = parse("D1").expect_err("a key needs a separator");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
}

#[test]
fn passage_key_rejects_an_empty_doc_id() {
    let error = parse("#3").expect_err("a doc id cannot be empty");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
}

#[test]
fn passage_key_rejects_an_empty_ord() {
    let error = parse("D1#").expect_err("an ordinal is required");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
}

#[test]
fn passage_key_rejects_a_non_numeric_ord() {
    let error = parse("D1#abc").expect_err("an ordinal is numeric");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
}

#[test]
fn passage_key_rejects_a_negative_ord() {
    let error = parse("D1#-1").expect_err("ordinals are unsigned");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
}

#[test]
fn passage_key_rejects_an_ord_above_u32_max() {
    let error = parse("D1#4294967296").expect_err("ordinals are u32");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
}

#[test]
fn passage_key_rejects_a_doc_id_above_512_bytes() {
    let raw = format!("{}#0", "a".repeat(513));
    let error = parse(&raw).expect_err("the doc id limit still applies");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
}

#[test]
fn passage_key_round_trips_through_display_and_parse() {
    let alphabet = prop_oneof![
        Just('D'),
        Just('1'),
        Just('#'),
        Just('-'),
        Just('é'),
        Just(' '),
    ];
    let doc_ids = proptest::collection::vec(alphabet, 0..=40)
        .prop_map(|chars| chars.into_iter().collect::<String>());

    common::check_property((doc_ids, any::<u32>()), |(raw, ord)| {
        let Ok(doc_id) = DocId::new(raw) else {
            // Invalid ids are covered by the `DocId` acceptance test.
            return Ok(());
        };
        let key = PassageKey::new(doc_id, ord);
        let displayed = key.to_string();

        let parsed = PassageKey::from_str(&displayed).map_err(|error| {
            TestCaseError::fail(format!("{displayed:?} did not parse: {error}"))
        })?;

        prop_assert_eq!(&parsed, &key);
        prop_assert_eq!(parsed.doc_id().as_str(), key.doc_id().as_str());
        prop_assert_eq!(parsed.ord(), ord);
        Ok(())
    });
}
