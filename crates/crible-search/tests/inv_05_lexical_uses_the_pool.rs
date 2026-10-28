// SPDX-License-Identifier: Apache-2.0

// Test fixtures may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixtures of this file build an index.
#![allow(clippy::panic)]

//! `INV-05` — the dense and lexical branches use the same pool.
//!
//! References: `docs/08-invariants-et-tests.md` (INV-05, "les candidats des deux
//! branches sont inclus dans le pool"), `docs/03-recherche.md` ("Filtres → pool",
//! "Branche lexicale (tantivy)", "Fusion"), `docs/phases/P5-lexical-et-hybride.md`
//! (`RECHERCHER-BM25-DANS-LE-POOL`) and ADR-0008 (hybrid on the same pool).
//!
//! # What the invariant means here
//!
//! The pool is the set of **live** passages the filter keeps, computed once per
//! query (`docs/03`, step 3). Each branch has its own storage and its own
//! ranking, so each of them could return a passage the filter excluded, or a
//! passage a later generation tombstoned:
//!
//! - the tantivy index is a copy of the texts, and a copy can lag: the
//!   tombstone of a passage is applied to it by step 4 of `docs/02`, and no
//!   order of the engine promises that the two structures agree at the moment a
//!   query runs. `docs/03` makes the live bitmap the authority;
//! - the filter is evaluated on the reference state alone, so nothing but the
//!   collector keeps the lexical branch inside it.
//!
//! The tests therefore build a corpus where the query **does** match passages the
//! filter excludes: without that control, an empty result would prove nothing,
//! since a branch that returns nothing is trivially a subset of the pool.
//!
//! # The API under test
//!
//! ```text
//! pub mod crible_search::lexical;
//!
//! pub fn search(
//!     snapshot: &Snapshot,
//!     pool: &Pool,
//!     query: &str,
//!     k: usize,
//! ) -> Result<Vec<crible_search::dense::Scored>, CribleError>;
//! ```
//!
//! A hit is [`crible_search::dense::Scored`], the type that carries the order of
//! the answer for both branches (ADR-0072, decision 3): its `iid` stays inside the
//! engine, and its `key` is the tie-break the lexical order shares with the
//! dense one (ADR-0045, ADR-0079). The tests below only inspect `iid` and
//! `score`; `lexical_ties_and_explain.rs` checks the key.
//!
//! ```text
//! cargo test -p crible-search --test inv_05_lexical_uses_the_pool
//! ```

mod common;

use common::lexical::{
    Doc, LexicalFixture, copy_tree, doc, match_all, replace_tree, rewrite_payload,
    stored_generation,
};
use crible_core::filter::Filter;
use crible_core::ids::{Generation, Iid};
use crible_search::dense::Scored;
use crible_search::lexical;
use crible_store::lexical::LexicalStatus;

/// Query of the first two tests: three distinct terms once analysed, so the
/// lexical branch has a bag of terms and not a single one.
const QUERY: &str = "article 1240 responsabilite";

/// Query vector of the dense branch, which ranks `civil-1240` first and the two
/// excluded documents next: the two branches must exclude the same passages.
const QUERY_VECTOR: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

/// Filter of the first two tests: the three `civil` documents only.
const CIVIL_FILTER: &str = r#"{ "fonds": { "eq": "civil" } }"#;

/// Corpus of the first two tests.
///
/// Every document holds at least one term of [`QUERY`], so the lexical branch
/// matches all five; `travail-1132` and `travail-1240` are the two the filter
/// excludes, and `travail-1240` holds the three terms.
fn corpus() -> Vec<Doc> {
    vec![
        Doc {
            fonds: "civil".to_owned(),
            ..doc(
                "civil-1240",
                "article 1240 code civil responsabilite",
                [1.0, 0.0, 0.0, 0.0],
            )
        },
        Doc {
            fonds: "civil".to_owned(),
            ..doc(
                "civil-1241",
                "article 1241 code civil garde",
                [0.0, 1.0, 0.0, 0.0],
            )
        },
        Doc {
            fonds: "civil".to_owned(),
            ..doc(
                "civil-1134",
                "contrat article 1134 code civil force",
                [0.0, 0.0, 1.0, 0.0],
            )
        },
        Doc {
            fonds: "travail".to_owned(),
            ..doc(
                "travail-1132",
                "article 1132 contrat responsabilite salarie",
                [0.8, 0.6, 0.0, 0.0],
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
    ]
}

/// Parsed [`CIVIL_FILTER`].
fn civil_filter() -> Filter {
    Filter::from_json_str(CIVIL_FILTER)
        .unwrap_or_else(|error| panic!("the fixture filter must parse: {error}"))
}

/// Sorted iids of a set of hits, for a readable assertion.
fn iids(hits: &[Scored]) -> Vec<u32> {
    let mut iids: Vec<u32> = hits.iter().map(|hit| hit.iid.get()).collect();
    iids.sort_unstable();
    iids
}

/// Acceptance criterion: no result of the lexical branch leaves the pool.
///
/// The filter keeps the three `civil` documents, and the query matches all five:
/// the two `travail` documents are returned when the pool is the whole live set,
/// which proves the restriction of the next assertion is the pool and not the
/// query.
#[test]
fn inv_05_lexical_branch_returns_only_passages_of_the_pool() {
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&corpus());

    let snapshot = fixture.snapshot();
    let pool = crible_search::pool::Pool::evaluate(snapshot.filters(), &civil_filter())
        .unwrap_or_else(|error| panic!("the fixture filter must evaluate: {error}"));
    assert_eq!(
        common::sorted(pool.bitmap()),
        vec![0, 1, 2],
        "the fixture must keep the three `civil` passages and exclude the two others"
    );

    let hits = lexical::search(&snapshot, &pool, QUERY, 10)
        .unwrap_or_else(|error| panic!("the lexical search must run: {}", error.message()));
    assert!(
        !hits.is_empty(),
        "a filtered lexical search that returns nothing proves no subset relation"
    );
    for hit in &hits {
        assert!(
            pool.bitmap().contains(hit.iid.get()),
            "INV-05: the lexical branch returned iid {} at score {}, which the filter excluded",
            hit.iid.get(),
            hit.score
        );
    }

    // Control: the same query, the whole live set as pool. The two excluded
    // documents do match it, so the filter above is what removes them.
    let whole = crible_search::pool::Pool::evaluate(snapshot.filters(), &match_all())
        .unwrap_or_else(|error| panic!("the empty conjunction must evaluate: {error}"));
    let unfiltered = lexical::search(&snapshot, &whole, QUERY, 10).unwrap_or_else(|error| {
        panic!(
            "the unfiltered lexical search must run: {}",
            error.message()
        )
    });
    let returned = iids(&unfiltered);
    assert!(
        returned.contains(&3) && returned.contains(&4),
        "the query must match the two passages the filter excludes, otherwise the subset \
         assertion above proves nothing; the whole index returned {returned:?}"
    );
}

/// INV-05, second half: the same query, run in both branches, over the **same**
/// [`Pool`] value. Every candidate of each branch is in it, and both branches
/// return the excluded passages when the pool is the whole live set.
#[test]
fn inv_05_dense_and_lexical_branches_share_the_same_pool() {
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&corpus());

    let snapshot = fixture.snapshot();
    let pool = crible_search::pool::Pool::evaluate(snapshot.filters(), &civil_filter())
        .unwrap_or_else(|error| panic!("the fixture filter must evaluate: {error}"));
    let searcher = fixture.searcher(&snapshot);

    let dense = searcher
        .top_k_candidates(&QUERY_VECTOR, &pool, 10)
        .unwrap_or_else(|error| panic!("the dense search must run: {}", error.message()));
    let lexical = lexical::search(&snapshot, &pool, QUERY, 10)
        .unwrap_or_else(|error| panic!("the lexical search must run: {}", error.message()));
    assert!(
        !dense.is_empty() && !lexical.is_empty(),
        "both branches must return candidates, otherwise their inclusion in the pool is vacuous"
    );

    for candidate in &dense {
        assert!(
            pool.bitmap().contains(candidate.iid.get()),
            "INV-05: the dense branch returned iid {}, which the filter excluded",
            candidate.iid.get()
        );
    }
    for hit in &lexical {
        assert!(
            pool.bitmap().contains(hit.iid.get()),
            "INV-05: the lexical branch returned iid {}, which the filter excluded",
            hit.iid.get()
        );
    }

    // The two branches really see the same pool: on the whole live set, each of
    // them returns the two documents the filter excludes.
    let whole = crible_search::pool::Pool::evaluate(snapshot.filters(), &match_all())
        .unwrap_or_else(|error| panic!("the empty conjunction must evaluate: {error}"));
    let dense_whole = searcher
        .top_k_candidates(&QUERY_VECTOR, &whole, 10)
        .unwrap_or_else(|error| {
            panic!("the unfiltered dense search must run: {}", error.message())
        });
    let lexical_whole = lexical::search(&snapshot, &whole, QUERY, 10).unwrap_or_else(|error| {
        panic!(
            "the unfiltered lexical search must run: {}",
            error.message()
        )
    });
    let dense_iids: Vec<u32> = dense_whole
        .iter()
        .map(|candidate| candidate.iid.get())
        .collect();
    let lexical_iids = iids(&lexical_whole);
    for iid in [3_u32, 4] {
        assert!(
            dense_iids.contains(&iid),
            "the dense branch must see iid {iid} when the pool holds it, got {dense_iids:?}"
        );
        assert!(
            lexical_iids.contains(&iid),
            "the lexical branch must see iid {iid} when the pool holds it, got {lexical_iids:?}"
        );
    }
}

/// Acceptance criterion: no dead passage appears, even when tantivy has not
/// removed it yet.
///
/// The state is the one a crash between the SQLite commit and step 4, followed
/// by a restore of `lexical/`, leaves on disk: the index still holds the
/// tombstoned passage, and its payload claims to describe the committed
/// generation, so the snapshot has a reader and the branch runs. The live bitmap
/// is the only thing that hides the passage (`docs/03`, "Suppressions : elles
/// sont aussi faites dans tantivy, mais la bitmap des vivants reste
/// l'autorité").
#[test]
fn inv_05_a_dead_passage_never_appears_even_when_tantivy_still_holds_it() {
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&[
        doc(
            "mort",
            "article 1240 responsabilite employeur",
            [1.0, 0.0, 0.0, 0.0],
        ),
        doc(
            "vivant",
            "contrat article 1134 force obligatoire",
            [0.0, 1.0, 0.0, 0.0],
        ),
        doc(
            "autre",
            "vente article 1582 propriete",
            [0.0, 0.0, 1.0, 0.0],
        ),
    ]);

    // The lexical directory of generation 1, before the tombstone of `mort`.
    let saved = fixture.temp().join("lexical-before-the-tombstone");
    copy_tree(&fixture.lexical_dir(), &saved);

    fixture.remove_document("mort");
    assert_eq!(
        stored_generation(&fixture.records_path()),
        2,
        "the removal must commit generation 2"
    );
    fixture.close();

    // Put back the index that still holds the passage, and let its payload name
    // the committed generation: the state a restore of a derived directory
    // leaves, which no commit of the engine produced but which the branch must
    // still survive.
    replace_tree(&saved, &fixture.lexical_dir());
    rewrite_payload(&fixture.lexical_dir().join("meta.json"), "2");

    let state = crible_store::lexical::read_state(&fixture.epoch_dir())
        .unwrap_or_else(|error| panic!("the restored lexical index must be readable: {error}"));
    assert!(
        state.iids.contains(&Iid::new(0)),
        "the fixture must leave the tombstoned passage in tantivy, otherwise the test proves \
         nothing; it holds {:?}",
        state.iids.iter().map(|iid| iid.get()).collect::<Vec<_>>()
    );

    let store = fixture.reopen_read_only();
    let snapshot = store.snapshot();
    assert_eq!(
        snapshot.lexical_status(),
        LexicalStatus::Ready {
            committed: Generation::new(2)
        },
        "the restored payload must certify the index, so the branch really runs against a copy \
         that still holds the tombstoned passage"
    );
    assert!(
        snapshot.lexical().is_some(),
        "the branch must have a reader for this test to prove anything"
    );

    let pool = crible_search::pool::Pool::evaluate(snapshot.filters(), &match_all())
        .unwrap_or_else(|error| panic!("the empty conjunction must evaluate: {error}"));
    assert_eq!(
        common::sorted(pool.bitmap()),
        vec![1, 2],
        "the pool must exclude the tombstoned passage: the live bitmap is the authority"
    );

    // `employeur` is a term only the tombstoned passage holds, and tantivy still
    // holds it: nothing but the pool can keep it out.
    let hits = lexical::search(&snapshot, &pool, "employeur", 10)
        .unwrap_or_else(|error| panic!("the lexical search must run: {}", error.message()));
    assert!(
        hits.is_empty(),
        "the lexical branch returned {} hit(s) for a term only the tombstoned passage holds: {:?}",
        hits.len(),
        iids(&hits)
    );

    // Control: the branch is alive on the same snapshot.
    let alive = lexical::search(&snapshot, &pool, "contrat", 10)
        .unwrap_or_else(|error| panic!("the lexical search must run: {}", error.message()));
    assert_eq!(
        iids(&alive),
        vec![1],
        "the query on a live term must return the live passage, otherwise the empty answer \
         above proves nothing"
    );
}
