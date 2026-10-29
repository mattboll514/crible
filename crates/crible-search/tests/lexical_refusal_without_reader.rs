// SPDX-License-Identifier: Apache-2.0

// Test fixtures may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixtures of this file build an index.
#![allow(clippy::panic)]

//! Acceptance criterion of `RECHERCHER-BM25-DANS-LE-POOL`: a lexical search on a
//! snapshot without a lexical reader refuses in `overloaded` (503) with the
//! message of the state observed, and `dense` on the same snapshot succeeds.
//!
//! References: `docs/phases/P5-lexical-et-hybride.md`
//! (`RECHERCHER-BM25-DANS-LE-POOL`), `docs/05-api.md` (`overloaded`,
//! `invalid_request`), `docs/03-recherche.md` ("Branche lexicale"), ADR-0087 (a
//! read-only open repairs nothing, and refuses the branch), ADR-0088, decisions 5
//! and 7 (the four messages, one per state observed) and ADR-0089 (the two
//! messages of the states without a tantivy commit, `Missing`, and a branch the
//! configuration disables).
//!
//! # The four messages of ADR-0088, decision 5
//!
//! | State | Message |
//! |---|---|
//! | `Behind { committed: Some(Gt) }` | `lexical index behind generation Gt of G; open the index for writing to repair it` |
//! | `Ahead { committed: Gt }` | `lexical index ahead of generation Gt of G; open the index for writing to repair it` |
//! | `NotCertified { committed: Some(Gt) }` | `lexical index state at generation Gt is not certified for generation G; open the index for writing to repair it` |
//! | `Unreadable` | `lexical index is unreadable at generation G; open the index for writing to repair it` |
//!
//! `Gt` and `G` are the decimal generations, the only numbers these messages
//! carry: an internal identifier never leaves the engine (INV-01), and the tests
//! compare the whole string, so an identifier that crept into it would fail
//! them.
//!
//! # Two states ADR-0088 does not name, and the disabled branch
//!
//! Two states have no `Gt` to name, while the four messages above all do. ADR-0089,
//! decision 1, gives them their own text, still in `overloaded`:
//!
//! | State | Message |
//! |---|---|
//! | `Behind { committed: None }` | `no lexical index is committed in this epoch although passages exist (generation G); open the index for writing to rebuild it` |
//! | `NotCertified { committed: None }` | `lexical index state is not certified for generation G; open the index for writing to repair it` |
//!
//! `Disabled` — an index whose configuration disables the branch — has no reader
//! either. ADR-0089, decision 3, refuses it in `invalid_request` (400), with
//! « lexical branch is disabled for this index »: the property is permanent and
//! known when the configuration is validated, so retrying cannot succeed. Its
//! test therefore asserts another code, and `dense` keeps serving.
//!
//! ```text
//! cargo test -p crible-search --test lexical_refusal_without_reader
//! ```

mod common;

use std::fs;
use std::path::PathBuf;

use common::lexical::{
    LexicalFixture, break_payload, copy_tree, doc, drop_marker_column, match_all, replace_tree,
};
use crible_core::error::ErrorCode;
use crible_core::ids::Generation;
use crible_search::lexical;
use crible_search::pool::Pool;
use crible_store::generation::{GenerationStore, Snapshot};
use crible_store::lexical::LexicalStatus;
use serde_json::{Map, Value};

/// Query of every refusal test: `articl`, `1240` and `responsabil` once
/// analysed, three terms the fixture corpus holds.
const QUERY: &str = "article 1240 responsabilite";

/// Vector of the dense branch, which must keep being served on the same
/// snapshot.
const QUERY_VECTOR: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

/// The two documents of generation 1.
fn first_generation() -> Vec<common::lexical::Doc> {
    vec![
        doc(
            "civil-1240",
            "article 1240 code civil responsabilite",
            [1.0, 0.0, 0.0, 0.0],
        ),
        doc(
            "civil-1241",
            "article 1241 code civil garde",
            [0.0, 1.0, 0.0, 0.0],
        ),
    ]
}

/// The document of generation 2.
fn second_generation() -> Vec<common::lexical::Doc> {
    vec![doc(
        "civil-1134",
        "contrat article 1134 code civil force",
        [0.0, 0.0, 1.0, 0.0],
    )]
}

/// Fixture at generation 1, lexical branch enabled.
fn at_generation_one() -> LexicalFixture {
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&first_generation());
    fixture
}

/// Pool of the whole live set on `snapshot`.
fn whole_pool(snapshot: &Snapshot) -> Pool {
    Pool::evaluate(snapshot.filters(), &match_all())
        .unwrap_or_else(|error| panic!("the empty conjunction must evaluate: {error}"))
}

/// Asserts that a lexical search refuses in `overloaded` (503) with exactly
/// `expected`, and returns the message for the caller's own assertions.
fn assert_refused(snapshot: &Snapshot, pool: &Pool, expected: &str) -> String {
    let error = match lexical::search(snapshot, pool, QUERY, 10) {
        Ok(hits) => panic!(
            "a lexical search without a reader must refuse, it returned {} hit(s) instead",
            hits.len()
        ),
        Err(error) => error,
    };
    assert_eq!(
        error.code(),
        ErrorCode::Overloaded,
        "a lexical search without a reader refuses in overloaded (503), got {}: {}",
        error.code().as_str(),
        error.message()
    );
    assert_eq!(
        error.message(),
        expected,
        "the refusal must carry the message of the state observed (ADR-0088, decision 5)"
    );
    error.message().to_owned()
}

/// Asserts that the dense branch still serves the same snapshot.
///
/// ADR-0087, decision 2: the refusal of the lexical branch never degrades
/// `hybrid` into the dense mode, and the dense mode keeps serving the generation
/// the snapshot names.
fn assert_dense_is_served(fixture: &LexicalFixture, snapshot: &Snapshot, pool: &Pool) {
    let searcher = fixture.searcher(snapshot);
    let hits = searcher
        .top_k(&QUERY_VECTOR, pool, 5)
        .unwrap_or_else(|error| panic!("the dense branch must still serve: {}", error.message()));
    assert!(
        !hits.is_empty(),
        "the dense branch must return a result on the same snapshot whose lexical branch refuses"
    );
}

/// `M > Gt`: the derived index described an older generation, and a generation
/// after it created a passage. This is the state a crash between the SQLite
/// commit and step 4 leaves, and the one message ADR-0087:38 already stated.
#[test]
fn lexical_search_refuses_behind_in_overloaded() {
    let mut fixture = at_generation_one();
    let saved = fixture.temp().join("lexical-at-generation-1");
    copy_tree(&fixture.lexical_dir(), &saved);
    fixture.add_generation(&second_generation());
    fixture.close();
    replace_tree(&saved, &fixture.lexical_dir());

    let store = fixture.reopen_read_only();
    let snapshot = store.snapshot();
    assert_eq!(
        snapshot.lexical_status(),
        LexicalStatus::Behind {
            committed: Some(Generation::new(1))
        },
        "the fixture must leave the lexical index one generation behind"
    );
    assert!(snapshot.lexical().is_none());

    let pool = whole_pool(&snapshot);
    let message = assert_refused(
        &snapshot,
        &pool,
        "lexical index behind generation 1 of 2; open the index for writing to repair it",
    );
    assert!(
        !message.contains("civil-1240") && !message.contains("iid"),
        "the message must name the state and the generations only (INV-01), got {message:?}"
    );
    assert_dense_is_served(&fixture, &snapshot, &pool);
}

/// `Gt > G`: no commit of the epoch produced that state — a restored epoch
/// without its derived directory, or a directory copied by hand.
#[test]
fn lexical_search_refuses_ahead_in_overloaded() {
    // The advanced index: two generations, so its lexical index names 2.
    let mut advanced = at_generation_one();
    advanced.add_generation(&second_generation());
    advanced.close();
    let advanced_lexical: PathBuf = advanced.lexical_dir();

    // The index that stays at generation 1, whose payload is then replaced by
    // the one of the advanced index: `Gt = 2` under `G = 1`.
    let mut fixture = at_generation_one();
    fixture.close();
    replace_tree(&advanced_lexical, &fixture.lexical_dir());

    let store = fixture.reopen_read_only();
    let snapshot = store.snapshot();
    assert_eq!(
        snapshot.lexical_status(),
        LexicalStatus::Ahead {
            committed: Generation::new(2)
        },
        "the fixture must leave the lexical index ahead of the committed generation"
    );
    assert!(snapshot.lexical().is_none());

    let pool = whole_pool(&snapshot);
    assert_refused(
        &snapshot,
        &pool,
        "lexical index ahead of generation 2 of 1; open the index for writing to repair it",
    );
    assert_dense_is_served(&fixture, &snapshot, &pool);
}

/// The marker column is absent and `Gt < G`: an epoch written before P5, opened
/// for reading before any write open completed its column (`ADR-0088`,
/// decision 4). The narrow rule of ADR-0087 applies, and the message says the
/// state is not certified rather than asserting a delay.
#[test]
fn lexical_search_refuses_not_certified_in_overloaded() {
    let mut fixture = at_generation_one();
    let mut fields = Map::new();
    fields.insert("fonds".to_owned(), Value::String("autre".to_owned()));
    let generation = fixture.patch_document("civil-1241", fields);
    assert_eq!(
        generation,
        Generation::new(2),
        "the patch must commit generation 2 and run no step 4"
    );
    fixture.close();
    drop_marker_column(&fixture.records_path());

    let store = fixture.reopen_read_only();
    let snapshot = store.snapshot();
    assert_eq!(
        snapshot.lexical_status(),
        LexicalStatus::NotCertified {
            committed: Some(Generation::new(1))
        },
        "without the marker, `Gt = G - 1` is not certified"
    );
    assert!(snapshot.lexical().is_none());

    let pool = whole_pool(&snapshot);
    assert_refused(
        &snapshot,
        &pool,
        "lexical index state at generation 1 is not certified for generation 2; open the index \
         for writing to repair it",
    );
    assert_dense_is_served(&fixture, &snapshot, &pool);
}

/// The payload this build cannot read: `Gt` is undecidable, so nothing can be
/// caught up and the branch must refuse rather than serve a state it cannot
/// name.
#[test]
fn lexical_search_refuses_unreadable_payload_in_overloaded() {
    let mut fixture = at_generation_one();
    fixture.close();
    break_payload(&fixture.lexical_dir().join("meta.json"));

    let store = fixture.reopen_read_only();
    let snapshot = store.snapshot();
    assert_eq!(snapshot.lexical_status(), LexicalStatus::Unreadable);
    assert!(snapshot.lexical().is_none());

    let pool = whole_pool(&snapshot);
    assert_refused(
        &snapshot,
        &pool,
        "lexical index is unreadable at generation 1; open the index for writing to repair it",
    );
    assert_dense_is_served(&fixture, &snapshot, &pool);
}

/// A `lexical/` tantivy does not open at all — here a `meta.json` that is not
/// JSON — is the same undecidable state as a payload this build cannot read
/// (`ADR-0088`, decisions 6 and 7), so the same message.
#[test]
fn lexical_search_refuses_an_unopenable_directory_in_overloaded() {
    let mut fixture = at_generation_one();
    fixture.close();
    let meta = fixture.lexical_dir().join("meta.json");
    fs::write(&meta, b"{ this is not tantivy's meta.json")
        .unwrap_or_else(|error| panic!("{} must be writable: {error}", meta.display()));

    let store = fixture.reopen_read_only();
    let snapshot = store.snapshot();
    assert_eq!(snapshot.lexical_status(), LexicalStatus::Unreadable);

    let pool = whole_pool(&snapshot);
    assert_refused(
        &snapshot,
        &pool,
        "lexical index is unreadable at generation 1; open the index for writing to repair it",
    );
    assert_dense_is_served(&fixture, &snapshot, &pool);
}

/// `Behind { committed: None }`: the epoch holds passages and its `lexical/` was
/// never created — a branch enabled after the passages were committed, or an
/// epoch restored without its derived directory.
///
/// ADR-0089, decision 1: the message says **rebuild**, because the next write
/// open rebuilds the index of the epoch instead of catching it up.
#[test]
fn lexical_search_refuses_behind_without_any_commit_in_overloaded() {
    let mut fixture = LexicalFixture::create(false);
    fixture.add_generation(&first_generation());
    fixture.close();
    fixture.enable_lexical();
    assert!(
        !fixture.lexical_dir().exists(),
        "the fixture must hold no lexical index, otherwise it is not the state under test"
    );

    let store = fixture.reopen_read_only();
    let snapshot = store.snapshot();
    assert_eq!(
        snapshot.lexical_status(),
        LexicalStatus::Behind { committed: None },
        "passages exist and no tantivy commit describes them"
    );
    assert!(snapshot.lexical().is_none());

    let pool = whole_pool(&snapshot);
    assert_refused(
        &snapshot,
        &pool,
        "no lexical index is committed in this epoch although passages exist (generation 1); \
         open the index for writing to rebuild it",
    );
    assert_dense_is_served(&fixture, &snapshot, &pool);
}

/// `NotCertified { committed: None }`: an epoch written before P5 that holds
/// passages and no lexical index. The absence of an index says nothing, so the
/// state is not `Missing` (`ADR-0088`, decision 4).
///
/// ADR-0089, decision 1: without a marker the write open completes `M` before it
/// decides, so the message promises no catch-up either.
#[test]
fn lexical_search_refuses_not_certified_without_any_commit_in_overloaded() {
    let mut fixture = LexicalFixture::create(false);
    fixture.add_generation(&first_generation());
    fixture.close();
    fixture.enable_lexical();
    drop_marker_column(&fixture.records_path());

    let store = fixture.reopen_read_only();
    let snapshot = store.snapshot();
    assert_eq!(
        snapshot.lexical_status(),
        LexicalStatus::NotCertified { committed: None },
        "without the marker, an absent lexical index certifies nothing"
    );
    assert!(snapshot.lexical().is_none());

    let pool = whole_pool(&snapshot);
    assert_refused(
        &snapshot,
        &pool,
        "lexical index state is not certified for generation 1; open the index for writing to \
         repair it",
    );
    assert_dense_is_served(&fixture, &snapshot, &pool);
}

/// `Missing` is not a refusal: the epoch never carried a passage, so the branch
/// has nothing to serve and answers an empty list, as the dense branch answers
/// an empty pool.
#[test]
fn lexical_search_on_an_epoch_without_passages_returns_no_result() {
    let fixture = LexicalFixture::create(true);
    let snapshot = fixture.snapshot();
    assert_eq!(
        snapshot.lexical_status(),
        LexicalStatus::Missing,
        "a fresh index with the branch enabled has no passage to describe"
    );

    let pool = whole_pool(&snapshot);
    assert!(pool.is_empty(), "a fresh index has an empty pool");
    let hits = lexical::search(&snapshot, &pool, QUERY, 10).unwrap_or_else(|error| {
        panic!(
            "an empty branch returns no result, it does not refuse: {}",
            error.message()
        )
    });
    assert!(hits.is_empty(), "a fresh index has no passage to return");
}

/// An index whose configuration disables the branch has no lexical reader
/// either, and a `lexical` search cannot be served.
///
/// ADR-0089, decision 3: the refusal is `invalid_request` (400), not `overloaded`
/// (503). `lexical.enabled` is a permanent property of the index, known when its
/// configuration was validated, so `Retry-After` would make a client retry a
/// request that cannot succeed. The message names no identifier (INV-01), and the
/// dense mode keeps serving the same snapshot.
#[test]
fn lexical_search_on_a_disabled_branch_is_refused() {
    let mut fixture = LexicalFixture::create(false);
    fixture.add_generation(&first_generation());
    let snapshot = fixture.snapshot();
    assert_eq!(
        snapshot.lexical_status(),
        LexicalStatus::Disabled,
        "the configuration of the fixture disables the branch"
    );
    assert!(snapshot.lexical().is_none());

    let pool = whole_pool(&snapshot);
    let error = match lexical::search(&snapshot, &pool, QUERY, 10) {
        Ok(hits) => panic!(
            "a lexical search on a disabled branch must refuse, it returned {} hit(s) instead",
            hits.len()
        ),
        Err(error) => error,
    };
    assert_eq!(
        error.code(),
        ErrorCode::InvalidRequest,
        "a disabled branch is a request the index cannot carry out, got {}: {}",
        error.code().as_str(),
        error.message()
    );
    assert_eq!(
        error.message(),
        "lexical branch is disabled for this index",
        "the refusal must carry the message of ADR-0089, decision 3"
    );
    assert_dense_is_served(&fixture, &snapshot, &pool);
}

/// The refusal happens before any search work: an empty pool does not turn a
/// missing reader into an empty answer.
///
/// A snapshot without reader must refuse whatever the query is, otherwise a
/// caller would read an empty list as "nothing matched" instead of "the branch
/// cannot run" (`docs/05-api.md`, `overloaded`).
#[test]
fn lexical_search_refuses_even_on_an_empty_pool() {
    let mut fixture = at_generation_one();
    fixture.close();
    break_payload(&fixture.lexical_dir().join("meta.json"));

    let store: GenerationStore = fixture.reopen_read_only();
    let snapshot = store.snapshot();
    let filter = crible_core::filter::Filter::from_json_str(r#"{ "fonds": { "eq": "absent" } }"#)
        .unwrap_or_else(|error| panic!("the fixture filter must parse: {error}"));
    let pool = Pool::evaluate(snapshot.filters(), &filter)
        .unwrap_or_else(|error| panic!("the fixture filter must evaluate: {error}"));
    assert!(pool.is_empty(), "the fixture filter must keep nothing");

    assert_refused(
        &snapshot,
        &pool,
        "lexical index is unreadable at generation 1; open the index for writing to repair it",
    );
}
