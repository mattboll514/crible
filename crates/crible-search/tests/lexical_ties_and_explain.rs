// SPDX-License-Identifier: Apache-2.0

// Test fixtures may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixtures of this file build an index.
#![allow(clippy::panic)]

//! Order of the lexical branch and its `explain`, the two points ADR-0089 leaves
//! to `RECHERCHER-BM25-DANS-LE-POOL` besides the refusals.
//!
//! References: `docs/phases/P5-lexical-et-hybride.md`
//! (`RECHERCHER-BM25-DANS-LE-POOL`, "l'explication tantivy disponible pour
//! `explain`"), `docs/03-recherche.md` ("Branche lexicale"), `docs/06-observabilite.md`
//! (`explain`), ADR-0045 and ADR-0079 (descending score, then ascending key
//! `doc_id#ord`, never the identifier) and ADR-0089 ("Rappels").
//!
//! # Why a key decides a tie
//!
//! The identifier of a passage is an internal, epoch-local number: it says
//! nothing about the answer, and it never leaves the engine (INV-01). Two
//! passages with the same score are therefore ordered on their key
//! `doc_id#ord`, as text, exactly as the dense branch orders them. The tests
//! below build a corpus where that order **differs from the identifier order**,
//! so a branch that still sorted on the identifier would fail them:
//!
//! | Document | Identifier | Key |
//! |---|---|---|
//! | `zebre` | 0 | `zebre#0` |
//! | `mike` | 1 | `mike#0` |
//! | `alpha` | 2 | `alpha#0` |
//!
//! The three texts are identical, so the three scores are: the answer of `k = 2`
//! is `alpha#0` then `mike#0`, and only the keys can say so. That is also the case
//! ADR-0089 warns about: a tie at the boundary of the top-k, whose keys have to be
//! read to decide.
//!
//! ```text
//! cargo test -p crible-search --test lexical_ties_and_explain
//! ```

mod common;

use common::lexical::{Doc, LexicalFixture, doc, match_all};
use crible_core::error::ErrorCode;
use crible_core::filter::Filter;
use crible_search::lexical;
use crible_search::pool::Pool;

/// Query of every test: `articl`, `1240` and `responsabil` once analysed.
const QUERY: &str = "article 1240 responsabilite";

/// Text of the three documents of the tie test. Identical on purpose: the BM25
/// score of a passage depends on its text and on the statistics of the index, so
/// identical texts in one segment score the same.
const SAME_TEXT: &str = "article 1240 code civil responsabilite";

/// Pool of the whole live set on `snapshot`.
fn whole_pool(snapshot: &crible_store::generation::Snapshot) -> Pool {
    Pool::evaluate(snapshot.filters(), &match_all())
        .unwrap_or_else(|error| panic!("the empty conjunction must evaluate: {error}"))
}

/// Pool of the `fonds` value `value`: the same filter the `INV-05` tests use.
fn fonds_pool(snapshot: &crible_store::generation::Snapshot, value: &str) -> Pool {
    let filter = Filter::from_json_str(&format!(r#"{{ "fonds": {{ "eq": "{value}" }} }}"#))
        .unwrap_or_else(|error| panic!("the fixture filter must parse: {error}"));
    Pool::evaluate(snapshot.filters(), &filter)
        .unwrap_or_else(|error| panic!("the fixture filter must evaluate: {error}"))
}

/// A tie is broken on the ascending key `doc_id#ord`, not on the internal
/// identifier.
///
/// `zebre` gets identifier 0, `mike` 1 and `alpha` 2, and the three texts are
/// identical. With `k = 2` the boundary is a tie of the three scores: the answer
/// must be the two smallest keys, which the branch can only know by reading the
/// keys of the tie and not by trusting the identifiers.
#[test]
fn lexical_ties_are_broken_on_the_ascending_key() {
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&[
        doc("zebre", SAME_TEXT, [1.0, 0.0, 0.0, 0.0]),
        doc("mike", SAME_TEXT, [0.0, 1.0, 0.0, 0.0]),
        doc("alpha", SAME_TEXT, [0.0, 0.0, 1.0, 0.0]),
    ]);
    let snapshot = fixture.snapshot();
    let pool = whole_pool(&snapshot);
    assert_eq!(
        common::sorted(pool.bitmap()),
        vec![0, 1, 2],
        "the fixture must put all three passages in the pool"
    );

    let hits = lexical::search(&snapshot, &pool, QUERY, 2)
        .unwrap_or_else(|error| panic!("the lexical search must run: {}", error.message()));
    let keys: Vec<&str> = hits.iter().map(|hit| hit.key.as_str()).collect();
    assert_eq!(
        keys,
        vec!["alpha#0", "mike#0"],
        "a tie at the boundary of the top-k must be broken on the ascending key, not on the \
         identifier (ADR-0045, ADR-0079)"
    );
    assert_eq!(
        hits[0].score.to_bits(),
        hits[1].score.to_bits(),
        "the fixture must tie the scores, otherwise this test proves nothing"
    );

    // The whole tie, in order: the key orders all three, whatever k is.
    let all = lexical::search(&snapshot, &pool, QUERY, 10)
        .unwrap_or_else(|error| panic!("the lexical search must run: {}", error.message()));
    let keys: Vec<&str> = all.iter().map(|hit| hit.key.as_str()).collect();
    assert_eq!(
        keys,
        vec!["alpha#0", "mike#0", "zebre#0"],
        "the full answer must be ordered on the key"
    );
    let iids: Vec<u32> = all.iter().map(|hit| hit.iid.get()).collect();
    assert_eq!(
        iids,
        vec![2, 1, 0],
        "the fixture must make the key order differ from the identifier order, otherwise the \
         test above could pass with an identifier sort"
    );
}

/// `explain` renders tantivy's explanation of a passage the filter excludes and
/// the top-k does not reach.
///
/// `travail-1240` matches the query with all three terms, but the filter keeps
/// `civil` only and `k = 1` keeps a single result. The explanation must still be
/// rendered, and its value must be the BM25 score the same passage receives when
/// the pool is the whole live set: `explain` reads the index, not the pool nor the
/// ranking.
#[test]
fn explain_renders_a_passage_outside_the_pool_and_the_top_k() {
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&[
        Doc {
            fonds: "civil".to_owned(),
            ..doc(
                "civil-1240",
                "article 1240 code civil responsabilite",
                [1.0, 0.0, 0.0, 0.0],
            )
        },
        Doc {
            fonds: "travail".to_owned(),
            ..doc(
                "travail-1240",
                "article 1240 responsabilite employeur",
                [0.9, 0.1, 0.0, 0.0],
            )
        },
    ]);
    let snapshot = fixture.snapshot();

    let filtered = fonds_pool(&snapshot, "civil");
    assert_eq!(
        common::sorted(filtered.bitmap()),
        vec![0],
        "the filter must keep the `civil` passage only"
    );
    let hits = lexical::search(&snapshot, &filtered, QUERY, 1).unwrap_or_else(|error| {
        panic!("the filtered lexical search must run: {}", error.message())
    });
    assert_eq!(hits.len(), 1, "k = 1 keeps one result");
    assert_eq!(
        hits[0].key, "civil-1240#0",
        "the excluded passage must be outside the answer"
    );

    let explanation = lexical::explain(&snapshot, "travail-1240#0", QUERY)
        .unwrap_or_else(|error| panic!("the explanation must be rendered: {}", error.message()));
    let Some(explanation) = explanation else {
        panic!("the explained passage is indexed and matched: the branch must explain it");
    };
    assert!(
        explanation.value() > 0.0,
        "the explained passage holds the three terms, its score must be positive, got {}",
        explanation.value()
    );

    // The value is the BM25 of the branch itself: on the whole live set, the same
    // passage is in the answer with that score. A tolerance covers the two
    // summation orders of an f32 score, as the reference test does.
    let whole = whole_pool(&snapshot);
    let all = lexical::search(&snapshot, &whole, QUERY, 10)
        .unwrap_or_else(|error| panic!("the lexical search must run: {}", error.message()));
    let scored = all
        .iter()
        .find(|hit| hit.key == "travail-1240#0")
        .unwrap_or_else(|| panic!("the unfiltered answer must hold travail-1240#0"));
    let gap = (explanation.value() - scored.score).abs();
    assert!(
        gap <= 1e-5,
        "the explanation scored {} where the branch scored {}, a gap of {gap:e}",
        explanation.value(),
        scored.score
    );
}

/// A key the commit read does not hold has nothing to explain, and that is not an
/// error: an unknown document and an unknown position of a known document both
/// answer `None`.
#[test]
fn explain_answers_none_for_a_key_the_commit_does_not_hold() {
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&[doc(
        "civil-1240",
        "article 1240 code civil responsabilite",
        [1.0, 0.0, 0.0, 0.0],
    )]);
    let snapshot = fixture.snapshot();

    for key in ["inconnu#0", "civil-1240#7"] {
        let explanation = lexical::explain(&snapshot, key, QUERY).unwrap_or_else(|error| {
            panic!(
                "an absent key is not an error, got {}: {}",
                error.code().as_str(),
                error.message()
            )
        });
        assert!(
            explanation.is_none(),
            "the commit read holds no passage {key}: nothing to explain, got {explanation:?}"
        );
    }
}

/// A key that is not of the form `doc_id#ord` is an invalid request, not an empty
/// explanation: the caller asked for something that designates nothing.
#[test]
fn explain_refuses_a_key_that_is_not_a_passage_key() {
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&[doc(
        "civil-1240",
        "article 1240 code civil responsabilite",
        [1.0, 0.0, 0.0, 0.0],
    )]);
    let snapshot = fixture.snapshot();

    let error = match lexical::explain(&snapshot, "civil-1240", QUERY) {
        Ok(explanation) => panic!("'civil-1240' is not a passage key, got {explanation:?}"),
        Err(error) => error,
    };
    assert_eq!(
        error.code(),
        ErrorCode::InvalidRequest,
        "a key without '#ord' is an invalid request, got {}: {}",
        error.code().as_str(),
        error.message()
    );
}
