// SPDX-License-Identifier: Apache-2.0

//! Validation of `DocId`: a non-empty UTF-8 identifier of at most 512 bytes.
//!
//! References: `docs/02-architecture.md` ("Identifiants") and the
//! `DEFINIR-TYPES-DE-BASE` task of `docs/phases/P0-fondations.md`.

mod common;

use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::DocId;
use proptest::prelude::*;

fn new(raw: &str) -> Result<DocId, CribleError> {
    DocId::new(raw)
}

#[test]
fn doc_id_new_accepts_a_plain_identifier() {
    let doc_id = new("LEGIARTI000006419280").expect("a plain ASCII id is valid");
    assert_eq!(doc_id.as_str(), "LEGIARTI000006419280");
}

#[test]
fn doc_id_new_accepts_a_non_ascii_identifier() {
    let doc_id = new("décision-日本-🦀").expect("UTF-8 ids are valid");
    assert_eq!(doc_id.as_str(), "décision-日本-🦀");
}

#[test]
fn doc_id_new_rejects_an_empty_identifier() {
    let error = new("").expect_err("an empty id is invalid");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
}

#[test]
fn doc_id_new_accepts_exactly_512_bytes() {
    let raw = "a".repeat(DocId::MAX_BYTES);
    assert_eq!(raw.len(), 512);
    let doc_id = new(&raw).expect("512 bytes is the documented limit");
    assert_eq!(doc_id.as_str(), raw);
}

#[test]
fn doc_id_new_rejects_513_bytes() {
    let raw = "a".repeat(DocId::MAX_BYTES + 1);
    assert_eq!(raw.len(), 513);
    let error = new(&raw).expect_err("513 bytes exceeds the documented limit");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
}

#[test]
fn doc_id_limit_counts_utf8_bytes_not_characters() {
    // 256 two-byte characters = 512 bytes: accepted.
    let accepted = "é".repeat(256);
    assert_eq!(accepted.chars().count(), 256);
    assert_eq!(accepted.len(), 512);
    new(&accepted).expect("the limit is expressed in bytes");

    // 257 two-byte characters = 514 bytes: rejected, even though the character
    // count stays far below 512.
    let rejected = "é".repeat(257);
    assert_eq!(rejected.len(), 514);
    let error = new(&rejected).expect_err("the limit is expressed in bytes");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);

    // 128 four-byte characters = 512 bytes: accepted.
    let accepted = "🦀".repeat(128);
    assert_eq!(accepted.len(), 512);
    new(&accepted).expect("the limit is expressed in bytes");

    // 129 four-byte characters = 516 bytes: rejected.
    let rejected = "🦀".repeat(129);
    assert_eq!(rejected.len(), 516);
    let error = new(&rejected).expect_err("the limit is expressed in bytes");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
}

#[test]
fn doc_id_new_keeps_the_separator_character() {
    // `#` is the separator of `PassageKey`. The documentation restricts `doc_id`
    // only by emptiness and byte length, so it must be accepted here.
    let doc_id = new("2024#12").expect("`#` is not a forbidden character");
    assert_eq!(doc_id.as_str(), "2024#12");
}

#[test]
fn doc_id_acceptance_matches_the_documented_rule() {
    let alphabet = prop_oneof![
        Just('a'),
        Just('Z'),
        Just('0'),
        Just('-'),
        Just('#'),
        Just(' '),
        Just('é'),
        Just('日'),
    ];
    let strategy = proptest::collection::vec(alphabet, 0..=600)
        .prop_map(|chars| chars.into_iter().collect::<String>());

    common::check_property(strategy, |raw| {
        let expected = !raw.is_empty() && raw.len() <= DocId::MAX_BYTES;
        match DocId::new(raw.clone()) {
            Ok(doc_id) => {
                prop_assert!(
                    expected,
                    "accepted {:?} ({} bytes, {} characters)",
                    raw,
                    raw.len(),
                    raw.chars().count()
                );
                prop_assert_eq!(doc_id.as_str(), raw.as_str());
            }
            Err(error) => {
                prop_assert!(
                    !expected,
                    "rejected {:?} ({} bytes, {} characters)",
                    raw,
                    raw.len(),
                    raw.chars().count()
                );
                prop_assert_eq!(error.code(), ErrorCode::InvalidRequest);
            }
        }
        Ok(())
    });
}
