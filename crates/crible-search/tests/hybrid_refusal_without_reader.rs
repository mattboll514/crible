// SPDX-License-Identifier: Apache-2.0

// Test fixtures may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixtures of this file build an index.
#![allow(clippy::panic)]

//! `hybrid` on a snapshot whose lexical branch refuses: the refusal is the same
//! as in `lexical`, and `dense` keeps serving the same snapshot.
//!
//! References: `docs/phases/P5-lexical-et-hybride.md` (`FUSIONNER-HYBRIDE`, and
//! the refusal criteria of `RECHERCHER-BM25-DANS-LE-POOL`), `docs/05-api.md`
//! (`overloaded`, `invalid_request`), `docs/03-recherche.md` ("Branche
//! lexicale"), ADR-0087 (a read-only open repairs nothing, and never degrades
//! `hybrid` into `dense`), ADR-0088 (the four messages of the states with a
//! tantivy commit) and ADR-0089 (the two messages of the states without a
//! commit, `Missing`, and a branch the configuration disables).
//!
//! # Why this file exists
//!
//! The task `RECHERCHER-BM25-DANS-LE-POOL` tested the refusals in the `lexical`
//! mode alone, while its criteria hold for "`lexical` or `hybride`". The same
//! states are built here again — only derived state is moved or rewritten, never
//! a vector or a passage row — and the assertions are made on the **pipeline**
//! of the `hybrid` mode:
//!
//! | State | Code | Message |
//! |---|---|---|
//! | `Behind { committed: Some(Gt) }` | `overloaded` | `lexical index behind generation Gt of G; …` |
//! | `Ahead { committed: Gt }` | `overloaded` | `lexical index ahead of generation Gt of G; …` |
//! | `NotCertified { committed: Some(Gt) }` | `overloaded` | `lexical index state at generation Gt is not certified for generation G; …` |
//! | `Unreadable` | `overloaded` | `lexical index is unreadable at generation G; …` |
//! | `Behind { committed: None }` | `overloaded` | `no lexical index is committed in this epoch although passages exist (generation G); …` |
//! | `NotCertified { committed: None }` | `overloaded` | `lexical index state is not certified for generation G; …` |
//! | `Disabled` | `invalid_request` | `lexical branch is disabled for this index` |
//! | `Missing` | — | no error, an empty answer |
//!
//! No message names a passage or an internal identifier (INV-01).
//!
//! ```text
//! cargo test -p crible-search --test hybrid_refusal_without_reader
//! ```

mod common;

use std::fs;
use std::path::PathBuf;

use common::lexical::{
    LexicalFixture, break_payload, copy_tree, doc, drop_marker_column, match_all, replace_tree,
};
use crible_core::config::{FusionConfig, ScanStrategy, SearchConfig};
use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::Generation;
use crible_search::dense::exact::DenseSearcher;
use crible_search::group::Grouping;
use crible_search::pipeline::{self, Answer, Mode, Request, ScoreType};
use crible_search::pool::Pool;
use crible_store::generation::{GenerationStore, Snapshot};
use crible_store::lexical::LexicalStatus;
use serde_json::{Map, Value};

/// Query of every refusal test: three terms the fixture corpus holds.
const QUERY: &str = "article 1240 responsabilite";

/// Vector of the dense branch, which must keep being served on the same
/// snapshot.
const QUERY_VECTOR: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

/// Dense settings of the tests: the exact strategy, whatever the pool.
fn settings() -> SearchConfig {
    SearchConfig {
        scan: ScanStrategy::None,
        rescore_candidates: 10,
        exact_threshold: 0,
        candidates_per_branch: 10,
        fusion: FusionConfig::Rrf,
    }
}

/// Regrouping options of a request that regroups nothing.
fn no_grouping() -> Grouping {
    Grouping::none()
}

/// Request of the mode under test, with both inputs `hybrid` needs.
fn request(mode: Mode, grouping: &Grouping) -> Request<'_> {
    Request {
        mode,
        vector: Some(&QUERY_VECTOR),
        text: Some(QUERY),
        k: 5,
        exact: false,
        grouping,
        fusion: FusionConfig::Rrf,
    }
}

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

/// Runs the pipeline of `mode` on `pool`.
fn run(
    fixture: &LexicalFixture,
    snapshot: &Snapshot,
    pool: &Pool,
    mode: Mode,
) -> Result<Answer, CribleError> {
    let searcher: DenseSearcher = fixture.searcher(snapshot);
    pipeline::search(
        snapshot,
        &searcher,
        pool,
        &request(mode, &no_grouping()),
        &settings(),
    )
}

/// Asserts that a `hybrid` search refuses with `code` and exactly `message`, and
/// returns the message for the caller's own assertions.
fn assert_hybrid_refused(
    fixture: &LexicalFixture,
    snapshot: &Snapshot,
    pool: &Pool,
    code: ErrorCode,
    message: &str,
) -> String {
    let error = match run(fixture, snapshot, pool, Mode::Hybrid) {
        Ok(answer) => panic!(
            "a hybrid search whose lexical branch cannot run must refuse, it returned {} \
             result(s) instead",
            answer.results.len()
        ),
        Err(error) => error,
    };
    assert_eq!(
        error.code(),
        code,
        "a hybrid search whose lexical branch cannot run refuses in {}, got {}: {}",
        code.as_str(),
        error.code().as_str(),
        error.message()
    );
    assert_eq!(
        error.message(),
        message,
        "the refusal must carry the message of the state observed (ADR-0088, ADR-0089)"
    );
    error.message().to_owned()
}

/// Asserts that the `dense` mode still serves the same snapshot, without any
/// degradation: it never asks for the lexical reader (ADR-0087, decision 2).
fn assert_dense_is_served(fixture: &LexicalFixture, snapshot: &Snapshot, pool: &Pool) {
    let answer = run(fixture, snapshot, pool, Mode::Dense).unwrap_or_else(|error| {
        panic!(
            "the dense mode must keep serving the same snapshot: {}",
            error.message()
        )
    });
    assert_eq!(answer.score_type, ScoreType::Cosine);
    assert!(
        !answer.results.is_empty(),
        "the dense branch must return a result on the snapshot whose lexical branch refuses"
    );
}

/// `M > Gt`: the derived index describes an older generation, and a generation
/// after it created a passage.
#[test]
fn hybrid_refuses_behind_in_overloaded() {
    let mut fixture = at_generation_one();
    let saved = fixture.temp().join("lexical-at-generation-1");
    copy_tree(&fixture.lexical_dir(), &saved);
    fixture.add_generation(&second_generation());
    fixture.close();
    replace_tree(&saved, &fixture.lexical_dir());

    let store: GenerationStore = fixture.reopen_read_only();
    let snapshot = store.snapshot();
    assert_eq!(
        snapshot.lexical_status(),
        LexicalStatus::Behind {
            committed: Some(Generation::new(1))
        },
        "the fixture must leave the lexical index one generation behind"
    );

    let pool = whole_pool(&snapshot);
    let message = assert_hybrid_refused(
        &fixture,
        &snapshot,
        &pool,
        ErrorCode::Overloaded,
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
fn hybrid_refuses_ahead_in_overloaded() {
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

    let pool = whole_pool(&snapshot);
    assert_hybrid_refused(
        &fixture,
        &snapshot,
        &pool,
        ErrorCode::Overloaded,
        "lexical index ahead of generation 2 of 1; open the index for writing to repair it",
    );
    assert_dense_is_served(&fixture, &snapshot, &pool);
}

/// The marker column is absent and `Gt < G`: an epoch written before P5, opened
/// for reading before any write open completed its column (ADR-0088,
/// decision 4).
#[test]
fn hybrid_refuses_not_certified_in_overloaded() {
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

    let pool = whole_pool(&snapshot);
    assert_hybrid_refused(
        &fixture,
        &snapshot,
        &pool,
        ErrorCode::Overloaded,
        "lexical index state at generation 1 is not certified for generation 2; open the index \
         for writing to repair it",
    );
    assert_dense_is_served(&fixture, &snapshot, &pool);
}

/// The payload this build cannot read: `Gt` is undecidable, so nothing can be
/// caught up and the branch must refuse rather than serve a state it cannot
/// name.
#[test]
fn hybrid_refuses_unreadable_payload_in_overloaded() {
    let mut fixture = at_generation_one();
    fixture.close();
    break_payload(&fixture.lexical_dir().join("meta.json"));

    let store = fixture.reopen_read_only();
    let snapshot = store.snapshot();
    assert_eq!(snapshot.lexical_status(), LexicalStatus::Unreadable);

    let pool = whole_pool(&snapshot);
    assert_hybrid_refused(
        &fixture,
        &snapshot,
        &pool,
        ErrorCode::Overloaded,
        "lexical index is unreadable at generation 1; open the index for writing to repair it",
    );
    assert_dense_is_served(&fixture, &snapshot, &pool);
}

/// A `lexical/` tantivy does not open at all — here a `meta.json` that is not
/// JSON — is the same undecidable state as a payload this build cannot read
/// (ADR-0088, decisions 6 and 7), so the same message.
#[test]
fn hybrid_refuses_an_unopenable_directory_in_overloaded() {
    let mut fixture = at_generation_one();
    fixture.close();
    let meta = fixture.lexical_dir().join("meta.json");
    fs::write(&meta, b"{ this is not tantivy's meta.json")
        .unwrap_or_else(|error| panic!("{} must be writable: {error}", meta.display()));

    let store = fixture.reopen_read_only();
    let snapshot = store.snapshot();
    assert_eq!(snapshot.lexical_status(), LexicalStatus::Unreadable);

    let pool = whole_pool(&snapshot);
    assert_hybrid_refused(
        &fixture,
        &snapshot,
        &pool,
        ErrorCode::Overloaded,
        "lexical index is unreadable at generation 1; open the index for writing to repair it",
    );
    assert_dense_is_served(&fixture, &snapshot, &pool);
}

/// `Behind { committed: None }`: the epoch holds passages and its `lexical/` was
/// never created — a branch enabled after the passages were committed, or an
/// epoch restored without its derived directory.
#[test]
fn hybrid_refuses_behind_without_any_commit_in_overloaded() {
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

    let pool = whole_pool(&snapshot);
    assert_hybrid_refused(
        &fixture,
        &snapshot,
        &pool,
        ErrorCode::Overloaded,
        "no lexical index is committed in this epoch although passages exist (generation 1); \
         open the index for writing to rebuild it",
    );
    assert_dense_is_served(&fixture, &snapshot, &pool);
}

/// `NotCertified { committed: None }`: an epoch written before P5 that holds
/// passages and no lexical index. The absence of an index says nothing, so the
/// state is not `Missing` (`ADR-0088`, decision 4).
#[test]
fn hybrid_refuses_not_certified_without_any_commit_in_overloaded() {
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

    let pool = whole_pool(&snapshot);
    assert_hybrid_refused(
        &fixture,
        &snapshot,
        &pool,
        ErrorCode::Overloaded,
        "lexical index state is not certified for generation 1; open the index for writing to \
         repair it",
    );
    assert_dense_is_served(&fixture, &snapshot, &pool);
}

/// `Missing` is not a refusal: the epoch never carried a passage, so both
/// branches have nothing to serve and `hybrid` answers an empty list
/// (ADR-0089, decision 2).
#[test]
fn hybrid_on_an_epoch_without_passages_returns_no_result() {
    let fixture = LexicalFixture::create(true);
    let snapshot = fixture.snapshot();
    assert_eq!(
        snapshot.lexical_status(),
        LexicalStatus::Missing,
        "a fresh index with the branch enabled has no passage to describe"
    );

    let pool = whole_pool(&snapshot);
    let answer = run(&fixture, &snapshot, &pool, Mode::Hybrid).unwrap_or_else(|error| {
        panic!(
            "an epoch without passages answers an empty list, it does not refuse: {}",
            error.message()
        )
    });
    assert!(answer.results.is_empty(), "a fresh index has no result");
    assert_eq!(
        answer.score_type,
        ScoreType::Rrf,
        "the fusion of two empty branches is still an rrf answer"
    );
}

/// An index whose configuration disables the branch cannot serve a `hybrid`
/// search: the refusal is `invalid_request` (400), not `overloaded` (503), and
/// `dense` keeps serving (ADR-0089, decision 3).
#[test]
fn hybrid_on_a_disabled_branch_is_refused() {
    let mut fixture = LexicalFixture::create(false);
    fixture.add_generation(&first_generation());
    let snapshot = fixture.snapshot();
    assert_eq!(
        snapshot.lexical_status(),
        LexicalStatus::Disabled,
        "the configuration of the fixture disables the branch"
    );

    let pool = whole_pool(&snapshot);
    let message = assert_hybrid_refused(
        &fixture,
        &snapshot,
        &pool,
        ErrorCode::InvalidRequest,
        "lexical branch is disabled for this index",
    );
    assert!(
        !message.contains("civil-1240"),
        "the message names no document (INV-01), got {message:?}"
    );
    assert_dense_is_served(&fixture, &snapshot, &pool);
}

/// The refusal happens before any search work: an empty pool does not turn a
/// missing reader into an empty answer. A caller must never read "nothing
/// matched" where the branch could not run.
#[test]
fn hybrid_refuses_even_on_an_empty_pool() {
    let mut fixture = at_generation_one();
    fixture.close();
    break_payload(&fixture.lexical_dir().join("meta.json"));

    let store = fixture.reopen_read_only();
    let snapshot = store.snapshot();
    let filter = crible_core::filter::Filter::from_json_str(r#"{ "fonds": { "eq": "absent" } }"#)
        .unwrap_or_else(|error| panic!("the fixture filter must parse: {error}"));
    let pool = Pool::evaluate(snapshot.filters(), &filter)
        .unwrap_or_else(|error| panic!("the fixture filter must evaluate: {error}"));
    assert!(pool.is_empty(), "the fixture filter must keep nothing");

    assert_hybrid_refused(
        &fixture,
        &snapshot,
        &pool,
        ErrorCode::Overloaded,
        "lexical index is unreadable at generation 1; open the index for writing to repair it",
    );
}
