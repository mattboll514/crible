// SPDX-License-Identifier: Apache-2.0

// Test fixtures may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixtures of this file build an index.
#![allow(clippy::panic)]

//! ADR-0148 — the canonical identifier token in the lexical branch, seen from
//! the search side.
//!
//! References: `docs/03-recherche.md` ("Branche lexicale", "Requête"),
//! `docs/08-invariants-et-tests.md` (INV-04, INV-05), ADR-0106, decision 4 (a
//! read-only open refuses another chain in `overloaded`) and ADR-0148
//! (decisions 1 to 5: the token is added, forms A and B, position, query side).
//!
//! | Test | Criterion of the fiche |
//! |---|---|
//! | `a_number_identifier_ranks_its_document_first_under_v1` | `10-13.625` met en tête le document qui le porte |
//! | `the_cited_and_the_stored_form_of_an_article_rank_identically` | `L. 1132-1` et `L1132-1` donnent le même rang |
//! | `a_neighbouring_article_is_not_ranked_first_by_its_neighbour` | contrôle négatif `l1132-11` |
//! | `inv_04_a_read_only_open_with_another_identifiers_refuses_the_branch_in_overloaded` | lecture seule, chaîne différente ⇒ `overloaded` |
//!
//! The corpus is deliberately small and the texts are short: the ranking is
//! decided by the canonical term, which only one passage carries, and not by the
//! statistics of a large index.
//!
//! ```text
//! cargo test -p crible-search --test lexical_identifiers
//! ```

mod common;

use std::fs;

use common::lexical::{Doc, LexicalFixture, doc, match_all, refusal};
use crible_core::error::ErrorCode;
use crible_search::dense::Scored;
use crible_search::lexical;
use crible_store::lexical::LexicalStatus;
use serde_json::Value;

/// Corpus of the ranking tests. One passage per document, at `ord = 0`, so the
/// key of a document is `<doc_id>#0`.
///
/// `cass-10-13-626` shares two of the three pieces of the pourvoi under test,
/// so the query is not a single-document match: the canonical term is what puts
/// the right decision first.
fn corpus() -> Vec<Doc> {
    vec![
        doc(
            "cass-10-13-625",
            "pourvoi 10-13.625 chambre sociale",
            [1.0, 0.0, 0.0, 0.0],
        ),
        doc(
            "cass-10-13-626",
            "pourvoi 10-13.626 chambre sociale",
            [0.0, 1.0, 0.0, 0.0],
        ),
        doc(
            "article-l1132-1",
            "article L. 1132-1 du code du travail",
            [0.0, 0.0, 1.0, 0.0],
        ),
        doc(
            "article-l1132-11",
            "article L. 1132-11 du code du travail",
            [0.0, 0.0, 0.0, 1.0],
        ),
    ]
}

/// Keys of the hits, in rank order.
fn keys(hits: &[Scored]) -> Vec<&str> {
    hits.iter().map(|hit| hit.key.as_str()).collect()
}

/// Runs the lexical branch of `fixture` on the whole live set.
fn search(fixture: &LexicalFixture, query: &str) -> Vec<Scored> {
    let snapshot = fixture.snapshot();
    let pool = crible_search::pool::Pool::evaluate(snapshot.filters(), &match_all())
        .unwrap_or_else(|error| panic!("the empty conjunction must evaluate: {error}"));
    lexical::search(&snapshot, &pool, query, 10).unwrap_or_else(|error| {
        panic!(
            "the lexical search of {query:?} must run, got {}: {}",
            error.code().as_str(),
            error.message()
        )
    })
}

#[test]
fn a_number_identifier_ranks_its_document_first_under_v1() {
    // ADR-0148, decisions 1 and 3: the pourvoi keeps a canonical token of its
    // own, so the passage that carries it outranks the one that only shares two
    // of its groups.
    let mut fixture = LexicalFixture::create_with_identifiers(Some("v1"));
    fixture.add_generation(&corpus());

    let hits = search(&fixture, "10-13.625");
    assert!(!hits.is_empty(), "the pourvoi of the corpus must be found");
    assert_eq!(
        hits[0].key,
        "cass-10-13-625#0",
        "the decision that carries 10-13.625 must be ranked first, got {:?}",
        keys(&hits)
    );
}

#[test]
fn the_cited_and_the_stored_form_of_an_article_rank_identically() {
    // ADR-0148, decisions 2 and 3: `L. 1132-1` and `L1132-1` both analyse to the
    // canonical token `l1132-1`, so the index and the query cannot diverge on it,
    // and both forms rank the article first.
    //
    // Corrected by `@dev`: the test required the two forms to return the same
    // *list*. Decision 1 forbids it — the pieces of 1.0 stay — and their piece
    // sets differ: the cited form yields `1132` and `1`, the stored form the one
    // piece `l1132`. The cited form therefore also matches the neighbouring
    // article, which shares `1132`, and the stored form does not. What the ADR
    // fixes is the ranking of the article, which the two forms share.
    let mut fixture = LexicalFixture::create_with_identifiers(Some("v1"));
    fixture.add_generation(&corpus());

    for query in ["L. 1132-1", "L1132-1"] {
        let hits = search(&fixture, query);
        assert!(
            !hits.is_empty(),
            "the article of the corpus must be found by {query:?}"
        );
        assert_eq!(
            hits[0].key,
            "article-l1132-1#0",
            "{query:?} must rank the article that carries L1132-1 first, got {:?}",
            keys(&hits)
        );
    }
}

#[test]
fn a_neighbouring_article_is_not_ranked_first_by_its_neighbour() {
    // ADR-0148, decision 4, negative control: the pieces of `l1132-11` match
    // neither the pieces nor the canonical token of `l1132-1`, so the article
    // L1132-1 must not be returned, let alone ranked first.
    let mut fixture = LexicalFixture::create_with_identifiers(Some("v1"));
    fixture.add_generation(&corpus());

    let hits = search(&fixture, "l1132-11");
    assert!(
        !hits.is_empty(),
        "the negative control must still find its own article"
    );
    assert_eq!(
        hits[0].key,
        "article-l1132-11#0",
        "l1132-11 must rank the article that carries it first, got {:?}",
        keys(&hits)
    );
    assert!(
        !keys(&hits).contains(&"article-l1132-1#0"),
        "the negative control must not return the article L1132-1, got {:?}",
        keys(&hits)
    );
}

#[test]
fn inv_04_a_read_only_open_with_another_identifiers_refuses_the_branch_in_overloaded() {
    // ADR-0148, decisions 6 and 7, with ADR-0106, decision 4: the setting enters
    // the analysis fingerprint. An index built with the key absent (`"none"`)
    // and served by a configuration that names `"v1"` is another chain; a
    // read-only open repairs nothing and the lexical branch refuses in
    // `overloaded`, while the dense branch keeps its snapshot.
    let mut fixture = LexicalFixture::create_with_identifiers(None);
    fixture.add_generation(&corpus());
    fixture.close();

    let path = fixture.index_dir().join("index.json");
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
    let mut value: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("{} must hold JSON: {error}", path.display()));
    value["lexical"]["identifiers"] = Value::String("v1".to_owned());
    fs::write(
        &path,
        serde_json::to_string(&value)
            .unwrap_or_else(|error| panic!("the configuration must serialize: {error}")),
    )
    .unwrap_or_else(|error| panic!("{} must be writable: {error}", path.display()));

    let store = fixture.reopen_read_only();
    let snapshot = store.snapshot();
    assert_eq!(
        snapshot.lexical_status(),
        LexicalStatus::OtherChain,
        "the read-only open must observe another analysis chain"
    );
    assert!(
        snapshot.lexical().is_none(),
        "a refused branch publishes no reader"
    );

    let pool = crible_search::pool::Pool::evaluate(snapshot.filters(), &match_all())
        .unwrap_or_else(|error| panic!("the empty conjunction must evaluate: {error}"));
    let error = match lexical::search(&snapshot, &pool, "10-13.625", 10) {
        Ok(hits) => panic!(
            "the branch must refuse in overloaded, got {} hit(s)",
            hits.len()
        ),
        Err(error) => error,
    };
    assert_eq!(error.code(), ErrorCode::Overloaded, "{}", error.message());
    let message = refusal(error);
    assert!(
        message.contains("another analysis chain"),
        "the refusal must name the chain (ADR-0106, decision 4), got {message:?}"
    );
    assert!(
        !message.contains("10-13.625"),
        "the refusal must carry no query text (INV-01), got {message:?}"
    );
}
