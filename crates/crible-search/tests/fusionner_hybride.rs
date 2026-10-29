// SPDX-License-Identifier: Apache-2.0

// Test fixtures may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixtures of this file build an index.
#![allow(clippy::panic)]

//! `FUSIONNER-HYBRIDE` (phase P5): the `hybrid` mode of the pipeline, its two
//! fusions, and the fact that the regrouping runs after the fusion.
//!
//! References: `docs/phases/P5-lexical-et-hybride.md` (`FUSIONNER-HYBRIDE`),
//! `docs/03-recherche.md` ("Fusion (mode `hybrid`)", "Regroupement", and the
//! pipeline table), `docs/05-api.md` (`mode`, `fusion`, `score_type`),
//! `docs/08-invariants-et-tests.md` (INV-05) and ADR-0008, ADR-0075, ADR-0079.
//!
//! # The API under test
//!
//! ```text
//! pub mod crible_search::pipeline;
//!
//! pub fn search(
//!     snapshot: &Snapshot,
//!     searcher: &crible_search::dense::exact::DenseSearcher,
//!     pool: &crible_search::pool::Pool,
//!     request: &pipeline::Request<'_>,
//!     search: &crible_core::config::SearchConfig,
//! ) -> Result<pipeline::Answer, CribleError>;
//! ```
//!
//! | Test | Criterion / invariant |
//! |---|---|
//! | `a_document_of_one_branch_only_still_appears` | "un document présent dans une seule branche est traité correctement" |
//! | `grouping_runs_after_the_fusion` | "le regroupement s'applique bien après la fusion" |
//! | `dedup_content_runs_after_the_fusion` | "la déduplication de contenu s'applique bien après la fusion" |
//! | `relative_score_weights_the_dense_ranking_and_changes_the_order` | "les deux fusions : `rrf` et `relative_score`" |
//! | `the_two_branches_share_the_pool` | INV-05 |
//! | `each_mode_reports_the_score_type_of_its_score` | "le `score_type`" |
//! | `a_mode_without_its_input_is_refused` | "un mode qui manque son entrée est refusé en `invalid_request`" |
//!
//! The fusion itself is pinned on known lists by the unit tests of
//! `crates/crible-search/src/fusion.rs`; these tests pin what only a real index
//! and the two branches together can show.
//!
//! ```text
//! cargo test -p crible-search --test fusionner_hybride
//! ```

mod common;

use common::lexical::{Doc, LexicalFixture, doc, match_all};
use crible_core::config::{FusionConfig, ScanStrategy, SearchConfig};
use crible_core::error::CribleError;
use crible_core::filter::Filter;
use crible_search::dense::exact::DenseSearcher;
use crible_search::group::Grouping;
use crible_search::pipeline::{self, Answer, Mode, Request, ScoreType};
use crible_search::pool::Pool;
use crible_store::generation::Snapshot;

/// Query text of every test: three terms once analysed, so the lexical branch
/// has a bag of terms and not a single one.
const QUERY: &str = "article 1240 responsabilite";

/// Query vector of every test: the first component, so the dense branch ranks
/// the documents by their first component.
const QUERY_VECTOR: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

/// Dense settings of the tests: the exact strategy, whatever the pool, and the
/// over-sampling of ADR-0075 left to the pipeline.
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

/// Request of one test, with both inputs the `hybrid` mode needs.
fn request<'a>(mode: Mode, k: usize, grouping: &'a Grouping, fusion: FusionConfig) -> Request<'a> {
    Request {
        mode,
        vector: Some(&QUERY_VECTOR),
        text: Some(QUERY),
        k,
        exact: false,
        grouping,
        fusion,
    }
}

/// Pool of `filter` on `snapshot`.
fn pool_of(snapshot: &Snapshot, filter: &Filter) -> Pool {
    Pool::evaluate(snapshot.filters(), filter)
        .unwrap_or_else(|error| panic!("the fixture filter must evaluate: {error}"))
}

/// Runs the pipeline of `request` on the whole live set.
fn search(
    fixture: &LexicalFixture,
    snapshot: &Snapshot,
    pool: &Pool,
    request: &Request<'_>,
) -> Result<Answer, CribleError> {
    let searcher = fixture.searcher(snapshot);
    pipeline::search(snapshot, &searcher, pool, request, &settings())
}

/// Document identifiers of an answer, in order.
fn documents(answer: &Answer) -> Vec<&str> {
    answer
        .results
        .iter()
        .map(|result| result.hit.doc_id.as_str())
        .collect()
}

/// Corpus of [`a_document_of_one_branch_only_still_appears`].
///
/// 100 passages share one vector, so the dense branch holds exactly 100
/// candidates (`max(k, 100)` of ADR-0075) and none of them matches the text: a
/// 101st document matches the text only and sits at cosine 0, so the dense
/// branch turns it away. Each direction of "one branch only" is then visible in
/// the answer of `k = 10`.
fn one_hundred_fillers_and_one_lexical_only() -> Vec<Doc> {
    let mut docs: Vec<Doc> = (0..100)
        .map(|position| {
            doc(
                &format!("filler-{position:03}"),
                "contrat force",
                [1.0, 0.0, 0.0, 0.0],
            )
        })
        .collect();
    docs.push(doc(
        "lexical-only",
        "article 1240 responsabilite",
        [0.0, 1.0, 0.0, 0.0],
    ));
    docs
}

/// A document the dense branch returned and the lexical one did not is treated
/// as present in one branch: its fused score is the dense term alone, and the
/// same goes the other way round.
///
/// A document of a single branch is not dropped and not given a score it did not
/// earn: `docs/03-recherche.md` makes an absent branch contribute nothing.
#[test]
fn a_document_of_one_branch_only_still_appears() {
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&one_hundred_fillers_and_one_lexical_only());
    let snapshot = fixture.snapshot();
    let pool = pool_of(&snapshot, &match_all());
    assert_eq!(pool.len(), 101, "the fixture must hold 101 live passages");

    let grouping = no_grouping();
    let answer = search(
        &fixture,
        &snapshot,
        &pool,
        &request(Mode::Hybrid, 10, &grouping, FusionConfig::Rrf),
    )
    .unwrap_or_else(|error| panic!("a hybrid search must run: {}", error.message()));

    assert_eq!(answer.score_type, ScoreType::Rrf);
    let documents = documents(&answer);
    assert_eq!(
        documents.len(),
        10,
        "k results, no more: the fusion does not replace the truncation of the answer"
    );

    // The first filler is rank 1 of the dense branch and rank 1 of the lexical
    // one does not hold it, so its score is exactly 1 / 61.
    let filler = &answer.results[0];
    assert_eq!(filler.hit.doc_id, "filler-000");
    assert_eq!(
        filler.hit.score,
        (1.0_f64 / 61.0) as f32,
        "a document of the dense branch only scores its dense term"
    );
    assert!(
        filler.lexical.is_none(),
        "the lexical branch did not return a filler, so it has no contribution there"
    );
    assert_eq!(
        filler.dense.map(|branch| branch.rank),
        Some(1),
        "the rank of the dense branch must be kept"
    );

    // The document only the lexical branch found ties the first filler and
    // follows it on its key: it is ranked, not dropped.
    let lexical_only = answer
        .results
        .iter()
        .find(|result| result.hit.doc_id == "lexical-only")
        .unwrap_or_else(|| panic!("a document of the lexical branch only must be in the answer"));
    assert!(
        lexical_only.dense.is_none(),
        "the dense branch turned that passage away, so its dense contribution is absent"
    );
    assert_eq!(
        lexical_only.lexical.map(|branch| branch.rank),
        Some(1),
        "it is the best candidate of the lexical branch"
    );
    assert_eq!(
        lexical_only.hit.score,
        (1.0_f64 / 61.0) as f32,
        "a document of the lexical branch only scores its lexical term"
    );
    assert_eq!(
        documents,
        vec![
            "filler-000",
            "lexical-only",
            "filler-001",
            "filler-002",
            "filler-003",
            "filler-004",
            "filler-005",
            "filler-006",
            "filler-007",
            "filler-008",
        ],
        "the fillers follow the order of the dense ranking, cut by the lexical passage on the \
         tie of rank 1"
    );
}

/// Corpus of [`grouping_runs_after_the_fusion`]: the dense branch ranks `A`
/// first, and the lexical branch returns `C` alone.
fn dense_first_but_lexical_second() -> Vec<Doc> {
    vec![
        doc("A", "contrat force", [1.0, 0.0, 0.0, 0.0]),
        doc("B", "garde salarie", [0.5, 0.5, 0.0, 0.0]),
        doc("C", "article 1240 responsabilite", [0.0, 1.0, 0.0, 0.0]),
    ]
}

/// `dedup` and `group_by` run **after** the fusion (step 7 after step 6,
/// ADR-0075): the document the grouping keeps is the first of the fused
/// ranking, not the first of the dense one.
///
/// The two rankings disagree on purpose: `C` is last of the dense branch and
/// alone in the lexical one, so it fuses above `A`, which is first of the dense
/// branch only. A grouping applied before the fusion would return `A`.
#[test]
fn grouping_runs_after_the_fusion() {
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&dense_first_but_lexical_second());
    let snapshot = fixture.snapshot();
    let pool = pool_of(&snapshot, &match_all());

    let grouping = Grouping {
        by_document: true,
        passages_per_document: 1,
        dedup_content: false,
    };
    let grouped = search(
        &fixture,
        &snapshot,
        &pool,
        &request(Mode::Hybrid, 1, &grouping, FusionConfig::Rrf),
    )
    .unwrap_or_else(|error| panic!("a grouped hybrid search must run: {}", error.message()));
    assert_eq!(
        documents(&grouped),
        vec!["C"],
        "the grouped answer keeps the first document of the fused ranking"
    );
    assert_eq!(
        grouped.results[0].lexical.map(|branch| branch.rank),
        Some(1),
        "the fused entry keeps the contribution of the lexical branch through the grouping"
    );
    assert_eq!(
        grouped.results[0].hit.score,
        (1.0_f64 / 61.0 + 1.0_f64 / 63.0) as f32,
        "C is rank 1 of the lexical branch and rank 3 of the dense one, so it sums both terms"
    );

    // The dense mode on the same snapshot keeps its own order, which is the one
    // a grouping applied before the fusion would have kept.
    let dense_order = search(
        &fixture,
        &snapshot,
        &pool,
        &request(Mode::Dense, 3, &no_grouping(), FusionConfig::Rrf),
    )
    .unwrap_or_else(|error| panic!("the dense mode must serve: {}", error.message()));
    assert_eq!(documents(&dense_order), vec!["A", "B", "C"]);

    let two = Grouping {
        by_document: true,
        passages_per_document: 1,
        dedup_content: false,
    };
    let grouped = search(
        &fixture,
        &snapshot,
        &pool,
        &request(Mode::Hybrid, 2, &two, FusionConfig::Rrf),
    )
    .unwrap_or_else(|error| panic!("a grouped hybrid search must run: {}", error.message()));
    assert_eq!(
        documents(&grouped),
        vec!["C", "A"],
        "the grouping fills up from the fused ranking: C then A"
    );
}

/// `dedup: "content"` also runs **after** the fusion (step 7 after step 6,
/// ADR-0075): the copy it keeps is the first of the fused ranking, which here is
/// the one only the lexical branch found.
///
/// The two passages share the same `passage_hash`, so a deduplication applied
/// before the fusion, branch by branch, would keep both: `dup-dense` is alone in
/// the dense branch and `dup-lexical` alone in the lexical one. Fusing first
/// ranks `dup-lexical` above `dup-dense`, because it sums the two branch terms,
/// and the deduplication then keeps it alone.
#[test]
fn dedup_content_runs_after_the_fusion() {
    let mut fixture = LexicalFixture::create(true);
    let shared = "blake3:shared-content".to_owned();
    fixture.add_generation(&[
        Doc {
            passage_hash: shared.clone(),
            ..doc("dup-dense", "contrat force", [1.0, 0.0, 0.0, 0.0])
        },
        Doc {
            passage_hash: shared,
            ..doc(
                "dup-lexical",
                "article 1240 responsabilite",
                [0.0, 1.0, 0.0, 0.0],
            )
        },
    ]);
    let snapshot = fixture.snapshot();
    let pool = pool_of(&snapshot, &match_all());

    // The fusion alone keeps both copies, in the fused order.
    let both = search(
        &fixture,
        &snapshot,
        &pool,
        &request(Mode::Hybrid, 3, &no_grouping(), FusionConfig::Rrf),
    )
    .unwrap_or_else(|error| panic!("a hybrid search must run: {}", error.message()));
    assert_eq!(documents(&both), vec!["dup-lexical", "dup-dense"]);

    let dedup = Grouping {
        by_document: false,
        passages_per_document: 1,
        dedup_content: true,
    };
    let deduplicated = search(
        &fixture,
        &snapshot,
        &pool,
        &request(Mode::Hybrid, 3, &dedup, FusionConfig::Rrf),
    )
    .unwrap_or_else(|error| panic!("a deduplicated hybrid search must run: {}", error.message()));
    assert_eq!(
        documents(&deduplicated),
        vec!["dup-lexical"],
        "the deduplication keeps the copy the fused ranking put first"
    );
}

/// `relative_score` normalises each branch on its own candidates, so the dense
/// ranking can win a document that `rrf` gave to the lexical one.
///
/// With `α = 0.5`, `A` is the best of the dense branch (1) and absent from the
/// lexical one (0), so it scores 0.5; `C` is the best of the lexical branch (1)
/// and the worst of the dense one (0), so it also scores 0.5. The tie is broken
/// on the key, which puts `A` first, whereas `rrf` put `C` first.
#[test]
fn relative_score_weights_the_dense_ranking_and_changes_the_order() {
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&dense_first_but_lexical_second());
    let snapshot = fixture.snapshot();
    let pool = pool_of(&snapshot, &match_all());

    let answer = search(
        &fixture,
        &snapshot,
        &pool,
        &request(
            Mode::Hybrid,
            3,
            &no_grouping(),
            FusionConfig::RelativeScore { alpha: 0.5 },
        ),
    )
    .unwrap_or_else(|error| panic!("a relative hybrid search must run: {}", error.message()));

    assert_eq!(
        answer.score_type,
        ScoreType::Relative,
        "relative_score reports the score type `relative` of docs/03-recherche.md"
    );
    assert_eq!(
        answer.score_type.as_str(),
        "relative",
        "the answer publishes the score type under its documented spelling"
    );
    assert_eq!(
        documents(&answer),
        vec!["A", "C", "B"],
        "A and C tie at 0.5 and their keys order them; B is the middle of the dense branch only"
    );
    assert_eq!(answer.results[0].hit.score, 0.5, "0.5 * 1 + 0");
    assert_eq!(answer.results[1].hit.score, 0.5, "0 + 0.5 * 1");
    assert_eq!(
        answer.results[2].hit.score, 0.25,
        "0.5 * 0.5, the middle of the dense branch"
    );
}

/// Corpus of [`the_two_branches_share_the_pool`]: `travail-1240` matches both
/// branches and outranks `civil-1240` in both of them.
fn one_match_outside_the_filter() -> Vec<Doc> {
    vec![
        Doc {
            fonds: "civil".to_owned(),
            ..doc(
                "civil-1240",
                "article 1240 responsabilite",
                [0.5, 0.5, 0.0, 0.0],
            )
        },
        Doc {
            fonds: "travail".to_owned(),
            ..doc(
                "travail-1240",
                "article 1240 responsabilite salarie",
                [1.0, 0.0, 0.0, 0.0],
            )
        },
    ]
}

/// INV-05 — the two branches of `hybrid` receive the same pool, and neither of
/// them may return a passage the filter excluded.
///
/// The filter keeps the `civil` document alone, while `travail-1240` matches the
/// query text and wins the dense ranking too: a branch that filtered on its own
/// would leak it into the answer.
#[test]
fn the_two_branches_share_the_pool() {
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&one_match_outside_the_filter());
    let snapshot = fixture.snapshot();

    let unfiltered = pool_of(&snapshot, &match_all());
    let top = search(
        &fixture,
        &snapshot,
        &unfiltered,
        &request(Mode::Hybrid, 2, &no_grouping(), FusionConfig::Rrf),
    )
    .unwrap_or_else(|error| panic!("a hybrid search must run: {}", error.message()));
    let unfiltered_documents = documents(&top);
    assert!(
        unfiltered_documents.contains(&"travail-1240")
            && unfiltered_documents.contains(&"civil-1240"),
        "without a filter, both documents match the query and the vector, so both branches \
         return both of them, got {unfiltered_documents:?}"
    );
    let filter = Filter::from_json_str(r#"{ "fonds": { "eq": "civil" } }"#)
        .unwrap_or_else(|error| panic!("the fixture filter must parse: {error}"));
    let filtered = pool_of(&snapshot, &filter);
    assert_eq!(filtered.len(), 1, "the filter must keep one passage");
    let answer = search(
        &fixture,
        &snapshot,
        &filtered,
        &request(Mode::Hybrid, 2, &no_grouping(), FusionConfig::Rrf),
    )
    .unwrap_or_else(|error| panic!("a filtered hybrid search must run: {}", error.message()));
    assert_eq!(
        documents(&answer),
        vec!["civil-1240"],
        "the excluded document matches the query text and the vector, and must still be absent"
    );
    assert!(
        answer
            .results
            .iter()
            .all(|result| result.hit.doc_id == "civil-1240"),
        "no result of the answer may fall outside the pool (INV-05)"
    );

    let empty = Filter::from_json_str(r#"{ "fonds": { "eq": "absent" } }"#)
        .unwrap_or_else(|error| panic!("the fixture filter must parse: {error}"));
    let empty_pool = pool_of(&snapshot, &empty);
    let answer = search(
        &fixture,
        &snapshot,
        &empty_pool,
        &request(Mode::Hybrid, 2, &no_grouping(), FusionConfig::Rrf),
    )
    .unwrap_or_else(|error| {
        panic!(
            "an empty pool must answer without error: {}",
            error.message()
        )
    });
    assert!(
        answer.results.is_empty(),
        "an empty pool has no result, in both branches"
    );
}

/// Each mode reports the score type of the score it produces, and the score of a
/// result is the score of that type: `cosine` for `dense`, `bm25` for `lexical`
/// (`docs/03-recherche.md`).
#[test]
fn each_mode_reports_the_score_type_of_its_score() {
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&dense_first_but_lexical_second());
    let snapshot = fixture.snapshot();
    let pool = pool_of(&snapshot, &match_all());

    let dense = search(
        &fixture,
        &snapshot,
        &pool,
        &request(Mode::Dense, 3, &no_grouping(), FusionConfig::Rrf),
    )
    .unwrap_or_else(|error| panic!("the dense mode must serve: {}", error.message()));
    assert_eq!(dense.score_type, ScoreType::Cosine);
    assert_eq!(dense.results[0].hit.score, 1.0, "the cosine of A is 1");
    assert!(
        dense.results.iter().all(|result| result.lexical.is_none()),
        "the dense mode runs no lexical branch"
    );

    let lexical = search(
        &fixture,
        &snapshot,
        &pool,
        &request(Mode::Lexical, 3, &no_grouping(), FusionConfig::Rrf),
    )
    .unwrap_or_else(|error| panic!("the lexical mode must serve: {}", error.message()));
    assert_eq!(lexical.score_type, ScoreType::Bm25);
    assert_eq!(
        documents(&lexical),
        vec!["C"],
        "C is the only passage the query text matches"
    );
    assert!(
        lexical.results[0].hit.score > 0.0,
        "a BM25 score of a match is positive"
    );
    assert!(
        lexical.results.iter().all(|result| result.dense.is_none()),
        "the lexical mode runs no dense branch"
    );

    let hybrid = search(
        &fixture,
        &snapshot,
        &pool,
        &request(Mode::Hybrid, 3, &no_grouping(), FusionConfig::Rrf),
    )
    .unwrap_or_else(|error| panic!("the hybrid mode must serve: {}", error.message()));
    assert_eq!(hybrid.score_type, ScoreType::Rrf);
    assert_eq!(
        hybrid.score_type.as_str(),
        "rrf",
        "the answer publishes the score type under its documented spelling"
    );
    assert_eq!(
        hybrid.results[0].hit.score,
        (1.0_f64 / 61.0 + 1.0_f64 / 63.0) as f32,
        "the score of a fused result is the fused score, not a cosine"
    );
}

/// The `dense` mode needs a vector and the `lexical` mode a text: a request that
/// runs the dense branch without a vector, or the lexical one without a text, is
/// refused before any branch runs (`invalid_request`).
#[test]
fn a_mode_without_its_input_is_refused() {
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&dense_first_but_lexical_second());
    let snapshot = fixture.snapshot();
    let pool = pool_of(&snapshot, &match_all());
    let searcher: DenseSearcher = fixture.searcher(&snapshot);

    let grouping = no_grouping();
    let no_vector = Request {
        vector: None,
        ..request(Mode::Hybrid, 1, &grouping, FusionConfig::Rrf)
    };
    let error = match pipeline::search(&snapshot, &searcher, &pool, &no_vector, &settings()) {
        Ok(answer) => panic!(
            "a hybrid search without a vector must be refused, it returned {} result(s)",
            answer.results.len()
        ),
        Err(error) => error,
    };
    assert_eq!(error.code(), crible_core::error::ErrorCode::InvalidRequest);
    assert!(
        error.message().contains("mode hybrid") && error.message().contains("query vector"),
        "the message must name the mode and the input it misses, got {error:?}"
    );

    let no_text = Request {
        text: None,
        ..request(Mode::Lexical, 1, &grouping, FusionConfig::Rrf)
    };
    let error = match pipeline::search(&snapshot, &searcher, &pool, &no_text, &settings()) {
        Ok(answer) => panic!(
            "a lexical search without a text must be refused, it returned {} result(s)",
            answer.results.len()
        ),
        Err(error) => error,
    };
    assert_eq!(error.code(), crible_core::error::ErrorCode::InvalidRequest);
    assert!(
        error.message().contains("mode lexical") && error.message().contains("query text"),
        "the message must name the mode and the input it misses, got {error:?}"
    );
}
