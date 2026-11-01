// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `EXPLIQUER-DOCUMENTS`: the explanation of `explain_ids` (ADR-0123).
//!
//! References: the fiche `EXPLIQUER-DOCUMENTS` of
//! `docs/phases/P7-observabilite.md`, ADR-0123 (closed shape, bounds, ranks,
//! `grouped_under`) and `docs/05-api.md` ("Recherche : réponse", `explain`).
//!
//! | Test | Case of the fiche |
//! |---|---|
//! | `a_passage_excluded_by_the_filter_names_every_failing_leaf` | exclu par le filtre |
//! | `a_document_in_the_pool_names_no_leaf_and_its_best_passage` | document dans le pool |
//! | `a_document_out_of_the_pool_lists_the_union_of_the_leaves_that_fail` | `$or`, complément C2 |
//! | `a_passage_key_describes_its_passage_and_never_a_sibling` | clé de passage, complément C1/C2 |
//! | `an_unknown_or_removed_document_has_no_alive_member` | mort, inconnu |
//! | `a_passage_outside_the_binary_candidates_keeps_its_rank` | hors des N candidats binaires |
//! | `a_lexical_only_explanation_carries_no_dense_object` | rang lexical seul |
//! | `the_lexical_ranks_match_the_ranking_of_the_whole_pool` | comptage des rangs, complément C5 |
//! | `a_passage_absorbed_by_dedup_names_the_result_that_kept_it` | absorbé par un regroupement |
//! | `under_dedup_grouped_under_follows_the_content_before_the_document` | `grouped_under`, complément C4 |
//! | `a_synthetic_fixture_explains_dense_rank_70_and_lexical_rank_2` | le cas « rang 70 / rang 2 » (ADR-0123, décision 11) |
//! | `an_id_that_ends_with_a_decimal_is_a_key_and_any_other_is_a_doc_id` | lecture d'un id, complément C1 |
//!
//! ```text
//! cargo test -p crible-search --test expliquer_documents
//! ```

mod common;

use std::sync::Arc;

use crible_core::config::{FusionConfig, ScanStrategy, SearchConfig};
use crible_core::filter::Filter;
use crible_core::ids::{DocId, Iid};
use crible_search::dense::exact::DenseSearcher;
use crible_search::explain::{ExplainEntry, ExplainId, ExplainInput, MAX_IDS, explain, survivors};
use crible_search::group::{Grouping, candidates_per_branch};
use crible_search::lexical;
use crible_search::pipeline::{self, Mode, Request};
use crible_search::pool::Pool;
use crible_store::generation::Snapshot;

use common::dense::{DenseFixture, Sample, match_all};
use common::lexical::{Doc, LexicalFixture, doc};

/// Query of the dense fixtures: its norm is one, so a vector `[s, 0, 0, 0]`
/// scores exactly `s`.
fn query() -> Vec<f32> {
    vec![1.0, 0.0, 0.0, 0.0]
}

/// `EXACT` whatever the pool: the oracle of the tests that do not exercise the
/// binary selection.
fn exact_settings() -> SearchConfig {
    SearchConfig {
        scan: ScanStrategy::None,
        rescore_candidates: 1_000,
        exact_threshold: 1_000,
        candidates_per_branch: 100,
        fusion: FusionConfig::Rrf,
    }
}

/// Parses a filter the way the route does, with its full JSON path.
fn filter(raw: serde_json::Value) -> Filter {
    Filter::from_json_str_at(&raw.to_string(), "$.filter")
        .unwrap_or_else(|error| panic!("the fixture filter must parse: {error}"))
}

/// One explanation run of the dense fixture.
struct DenseRun {
    snapshot: Arc<Snapshot>,
    searcher: DenseSearcher,
    pool: Pool,
}

impl DenseRun {
    /// Opens the fixture, evaluates `filter` and prepares the searcher.
    fn open(fixture: &DenseFixture, filter: &Filter) -> Self {
        let snapshot = fixture.snapshot();
        let searcher = fixture.searcher();
        let pool = fixture.pool(filter);
        Self {
            snapshot,
            searcher,
            pool,
        }
    }

    /// Explains `ids` in the dense mode.
    fn explain(
        &self,
        filter: &Filter,
        settings: &SearchConfig,
        ids: &[ExplainId],
    ) -> Vec<ExplainEntry> {
        let settings_ref = settings;
        let grouping = Grouping::none();
        let requested = candidates_per_branch(10, &grouping);
        let answer = pipeline::search_captured(
            &self.snapshot,
            &self.searcher,
            &self.pool,
            &Request {
                mode: Mode::Dense,
                vector: Some(&query()),
                text: None,
                k: 10,
                exact: false,
                grouping: &grouping,
                fusion: FusionConfig::Rrf,
            },
            settings_ref,
            None,
            true,
        )
        .unwrap_or_else(|error| panic!("the pipeline must run: {error}"));
        let kept = survivors(&self.searcher, &answer.results, false)
            .unwrap_or_else(|error| panic!("the survivors must build: {error}"));
        explain(
            &self.snapshot,
            &ExplainInput {
                pool: &self.pool,
                searcher: &self.searcher,
                filter,
                mode: Mode::Dense,
                vector: Some(&query()),
                text: None,
                search: settings,
                requested,
                exact: false,
                strategy: answer.strategy,
                pre_group_ranks: &answer.pre_group_ranks,
                survivors: &kept,
            },
            ids,
        )
        .unwrap_or_else(|error| panic!("the explanation must succeed: {error}"))
    }
}

#[test]
fn a_passage_excluded_by_the_filter_names_every_failing_leaf() {
    // ADR-0123, decision 7: `excluded_by` lists the leaves that fail, each by its
    // full JSON path, in the order of the filter.
    let mut fixture = DenseFixture::create(4);
    fixture.add_generation(&[
        Sample::new("A", 0, query()),
        Sample::new("B", 0, vec![0.5, 0.0, 0.0, 0.0]),
    ]);
    // `fonds` is `doc_id` by default: B fails it, A satisfies it. A fails `rang`
    // only, B fails both.
    let filter = filter(serde_json::json!({ "fonds": "A", "rang": 5 }));
    let run = DenseRun::open(&fixture, &filter);
    let ids = vec![
        ExplainId::Doc(DocId::new("A").expect("A is a doc_id")),
        ExplainId::Doc(DocId::new("B").expect("B is a doc_id")),
    ];
    let entries = run.explain(&filter, &exact_settings(), &ids);
    assert_eq!(
        entries.len(),
        2,
        "one entry per id, in the order of the request"
    );
    assert_eq!(entries[0].id, "A");
    assert!(entries[0].exists, "A has a `documents` row");
    assert_eq!(
        entries[0].excluded_by,
        vec!["$.filter.rang".to_owned()],
        "A is in the pool only if `rang` is not applied: the pool is the fixture's"
    );
    // The fixture's pool uses the very filter, so A is out too; the assertion
    // above states the leaf, the one below the membership.
    assert!(!entries[0].in_pool, "A fails `rang`");
    assert_eq!(
        entries[1].excluded_by,
        vec!["$.filter.fonds".to_owned(), "$.filter.rang".to_owned()],
        "B fails both leaves, in the order of the filter"
    );
    assert!(!entries[1].in_pool);
    assert!(
        entries[1].dense.is_none(),
        "a passage out of the pool has no branch"
    );
}

#[test]
fn a_document_in_the_pool_names_no_leaf_and_its_best_passage() {
    // ADR-0123, decision 7: an id in the pool reports no leaf. `excluded_by` is
    // absent when it is empty (ADR-0119, decision 3).
    let mut fixture = DenseFixture::create(4);
    fixture.add_generation(&[
        Sample::new("A", 0, query()),
        Sample::new("A", 1, vec![0.9, 0.0, 0.0, 0.0]),
        Sample::new("B", 0, vec![0.5, 0.0, 0.0, 0.0]),
    ]);
    let filter = filter(serde_json::json!({ "rang": 0 }));
    let run = DenseRun::open(&fixture, &filter);
    let ids = vec![ExplainId::Doc(DocId::new("A").expect("A is a doc_id"))];
    let entries = run.explain(&filter, &exact_settings(), &ids);
    assert!(entries[0].exists && entries[0].alive == Some(true));
    assert!(entries[0].in_pool, "passage A#0 satisfies `rang = 0`");
    assert!(
        entries[0].excluded_by.is_empty(),
        "a document in the pool has no failing leaf: {:?}",
        entries[0].excluded_by
    );
    let dense = entries[0]
        .dense
        .as_ref()
        .expect("the document is in the pool");
    assert_eq!(dense.key, "A#0", "the branch names the best-ranked passage");
    assert_eq!(dense.exact_rank, 1);
}

#[test]
fn a_document_out_of_the_pool_lists_the_union_of_the_leaves_that_fail() {
    // ADR-0123, complement of 2026-11-01 (C2): a `doc_id` out of the pool names
    // every leaf that fails for at least one of its living passages, in the order
    // of the filter. Each passage here satisfies one branch of the `$or`, so no
    // leaf fails for every passage, and the union still names them all.
    let mut fixture = DenseFixture::create(4);
    fixture.add_generation(&[
        Sample::new("B", 0, query()),
        Sample::new("B", 1, vec![0.5, 0.0, 0.0, 0.0]),
    ]);
    let filter = filter(serde_json::json!({
        "$and": [
            { "$or": [{ "rang": 0 }, { "rang": 1 }] },
            { "fonds": "A" }
        ]
    }));
    let run = DenseRun::open(&fixture, &filter);
    let ids = vec![ExplainId::Doc(DocId::new("B").expect("a doc_id"))];
    let entries = run.explain(&filter, &exact_settings(), &ids);
    assert_eq!(
        entries[0].excluded_by,
        vec![
            "$.filter.$and[0].$or[0].rang".to_owned(),
            "$.filter.$and[0].$or[1].rang".to_owned(),
            "$.filter.$and[1].fonds".to_owned(),
        ],
        "the union, in the order of the filter"
    );
    assert!(!entries[0].in_pool);
}

#[test]
fn a_passage_key_describes_its_passage_and_never_a_sibling() {
    // ADR-0123, complement of 2026-11-01 (C1 and C2): for a key, every member
    // describes the passage itself. `A#1` is out of the pool although its sibling
    // `A#0` is in it, and its fused rank is its own, never the sibling's.
    let mut fixture = DenseFixture::create(4);
    fixture.add_generation(&[
        Sample::new("A", 0, query()),
        Sample::new("A", 1, vec![0.9, 0.0, 0.0, 0.0]),
    ]);
    let keys = vec![
        ExplainId::Passage("A#0".parse().expect("A#0 is a key")),
        ExplainId::Passage("A#1".parse().expect("A#1 is a key")),
    ];
    let all = match_all();
    let run = DenseRun::open(&fixture, &all);
    let entries = run.explain(&all, &exact_settings(), &keys);
    assert!(entries[0].in_pool && entries[1].in_pool);
    assert_eq!(
        entries[0].dense.as_ref().map(|dense| dense.exact_rank),
        Some(1)
    );
    assert_eq!(
        entries[1].dense.as_ref().map(|dense| dense.exact_rank),
        Some(2)
    );
    assert_eq!(entries[0].fused.map(|fused| fused.rank), Some(1));
    assert_eq!(
        entries[1].fused.map(|fused| fused.rank),
        Some(2),
        "the rank of A#1 itself, not the rank of its sibling"
    );

    let filter = filter(serde_json::json!({ "rang": 0 }));
    let run = DenseRun::open(&fixture, &filter);
    let entries = run.explain(&filter, &exact_settings(), &keys);
    assert!(entries[0].in_pool, "A#0 satisfies the filter");
    assert!(
        !entries[1].in_pool,
        "A#1 fails it although its sibling is in the pool"
    );
    assert_eq!(
        entries[1].excluded_by,
        vec!["$.filter.rang".to_owned()],
        "the leaf that excludes the passage itself"
    );
    assert!(
        entries[1].dense.is_none()
            && entries[1].lexical.is_none()
            && entries[1].fused.is_none()
            && entries[1].grouped_under.is_none(),
        "the passage itself is out of the pool"
    );
}

#[test]
fn an_unknown_or_removed_document_has_no_alive_member() {
    // ADR-0041 and ADR-0123, decision 1: `exists` reads the `documents` row and
    // `alive` is absent when `exists` is false. `exists: true, alive: false` is
    // not reachable and is therefore not a case of this test: an attach removes
    // the `documents` row with the tombstones (ADR-0041), and a batch requires at
    // least one passage per document, so a document without a living passage has
    // no row either.
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&[
        doc("A", "alpha", [1.0, 0.0, 0.0, 0.0]),
        doc("B", "beta", [0.5, 0.0, 0.0, 0.0]),
    ]);
    fixture.remove_document("B");
    fixture.close();
    let store = fixture.reopen_read_only();
    let snapshot = store.snapshot();
    let searcher = fixture.searcher(&snapshot);
    let pool = Pool::evaluate(snapshot.filters(), &match_all())
        .unwrap_or_else(|error| panic!("the fixture pool must evaluate: {error}"));
    let ids = vec![
        ExplainId::Doc(DocId::new("never").expect("a doc_id")),
        ExplainId::Doc(DocId::new("A").expect("a doc_id")),
        ExplainId::Doc(DocId::new("B").expect("a doc_id")),
    ];
    let entries = explain(
        &snapshot,
        &ExplainInput {
            pool: &pool,
            searcher: &searcher,
            filter: &match_all(),
            mode: Mode::Lexical,
            vector: None,
            text: Some("alpha"),
            search: &exact_settings(),
            requested: 100,
            exact: false,
            strategy: None,
            pre_group_ranks: &[],
            survivors: &[],
        },
        &ids,
    )
    .unwrap_or_else(|error| panic!("the explanation must succeed: {error}"));
    assert!(!entries[0].exists, "an unknown document does not exist");
    assert!(
        entries[0].alive.is_none(),
        "`alive` is absent when `exists` is false"
    );
    assert!(entries[0].dense.is_none() && entries[0].lexical.is_none());
    assert!(entries[0].fused.is_none() && entries[0].grouped_under.is_none());
    assert!(entries[1].exists && entries[1].alive == Some(true));
    assert!(
        !entries[2].exists,
        "a removed document has no documents row"
    );
    assert!(entries[2].alive.is_none());
    assert!(!entries[2].in_pool);
    assert!(
        entries[2].excluded_by.is_empty(),
        "a dead document is not excluded by the filter"
    );
}

#[test]
fn a_passage_outside_the_binary_candidates_keeps_its_rank() {
    // ADR-0123, decision 5: `binary_rank` is rendered whatever the strategy and
    // `in_binary_candidates` is false when the rank exceeds the effective number
    // of candidates, `max(N, candidates_per_branch)`.
    let mut samples: Vec<Sample> = (0..100)
        .map(|position| Sample::new(&format!("D{position:03}"), 0, vec![1.0, 0.0, 0.0, 0.0]))
        .collect();
    // The query code has bit 0 set; this passage has it clear, so it takes the
    // largest Hamming distance and the last rank of the binary order.
    samples.push(Sample::new("Z", 0, vec![0.0, 1.0, 0.0, 0.0]));
    let mut fixture = DenseFixture::create(4);
    fixture.add_generation(&samples);
    let filter = match_all();
    let settings = SearchConfig {
        scan: ScanStrategy::Binary,
        rescore_candidates: 10,
        exact_threshold: 0,
        candidates_per_branch: 100,
        fusion: FusionConfig::Rrf,
    };
    let run = DenseRun::open(&fixture, &filter);
    let ids = vec![
        ExplainId::Doc(DocId::new("Z").expect("a doc_id")),
        ExplainId::Doc(DocId::new("D000").expect("a doc_id")),
    ];
    let entries = run.explain(&filter, &settings, &ids);
    let far = entries[0].dense.as_ref().expect("Z is in the pool");
    assert_eq!(far.exact_rank, 101, "every other passage scores above Z");
    assert_eq!(
        far.binary_rank,
        Some(101),
        "the distance of Z is the largest"
    );
    assert_eq!(
        far.in_binary_candidates,
        Some(false),
        "101 exceeds max(N, candidates_per_branch) = 100"
    );
    let near = entries[1].dense.as_ref().expect("D000 is in the pool");
    assert_eq!(near.binary_rank, Some(1), "the first identifier of the tie");
    assert_eq!(near.in_binary_candidates, Some(true));
    assert!(near.exact_rank <= 100);
}

#[test]
fn a_lexical_only_explanation_carries_no_dense_object() {
    // ADR-0123, decision 1: a branch that did not run has no object; `fused`
    // stays with the rank of the single branch (ADR-0119, decision 2).
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&[
        doc("A", "alpha alpha", [1.0, 0.0, 0.0, 0.0]),
        doc("B", "alpha", [0.5, 0.0, 0.0, 0.0]),
        doc("C", "gamma", [0.2, 0.0, 0.0, 0.0]),
    ]);
    fixture.close();
    let store = fixture.reopen_read_only();
    let snapshot = store.snapshot();
    let searcher = fixture.searcher(&snapshot);
    let filter = match_all();
    let pool = Pool::evaluate(snapshot.filters(), &filter)
        .unwrap_or_else(|error| panic!("the fixture pool must evaluate: {error}"));
    let grouping = Grouping::none();
    let answer = pipeline::search_captured(
        &snapshot,
        &searcher,
        &pool,
        &Request {
            mode: Mode::Lexical,
            vector: None,
            text: Some("alpha"),
            k: 10,
            exact: false,
            grouping: &grouping,
            fusion: FusionConfig::Rrf,
        },
        &exact_settings(),
        None,
        true,
    )
    .unwrap_or_else(|error| panic!("the pipeline must run: {error}"));
    let kept = survivors(&searcher, &answer.results, false)
        .unwrap_or_else(|error| panic!("the survivors must build: {error}"));
    let ids = vec![ExplainId::Doc(DocId::new("B").expect("a doc_id"))];
    let entries = explain(
        &snapshot,
        &ExplainInput {
            pool: &pool,
            searcher: &searcher,
            filter: &filter,
            mode: Mode::Lexical,
            vector: None,
            text: Some("alpha"),
            search: &exact_settings(),
            requested: candidates_per_branch(10, &grouping),
            exact: false,
            strategy: None,
            pre_group_ranks: &answer.pre_group_ranks,
            survivors: &kept,
        },
        &ids,
    )
    .unwrap_or_else(|error| panic!("the explanation must succeed: {error}"));
    let entry = &entries[0];
    assert!(entry.dense.is_none(), "the dense branch never ran");
    let lexical = entry.lexical.as_ref().expect("the branch scored B");
    assert_eq!(lexical.rank, 2, "`alpha alpha` outscores `alpha`");
    assert_eq!(lexical.key, "B#0");
    assert_eq!(lexical.terms.len(), 1, "one analysed term of the query");
    assert_eq!(lexical.terms[0].term, "alpha");
    assert_eq!(lexical.terms[0].tf, 1, "B holds the term once");
    let sum: f32 = lexical.terms.iter().map(|term| term.score).sum();
    assert!(
        (sum - lexical.score).abs() <= 1e-4,
        "the terms sum to the score: {sum} vs {}",
        lexical.score
    );
    let fused = entry.fused.expect("the answer's fused list holds B");
    assert_eq!(fused.rank, 2, "the single branch is the fused list");
}

#[test]
fn the_lexical_ranks_match_the_ranking_of_the_whole_pool() {
    // ADR-0123, complement of 2026-11-01 (C5): the rank is counted, never
    // materialised. The count must still equal the position `lexical::search`
    // gives over the whole pool, ties on the key included.
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&[
        doc("A", "alpha", [1.0, 0.0, 0.0, 0.0]),
        doc("B", "beta", [0.9, 0.0, 0.0, 0.0]),
        doc("C", "alpha beta", [0.8, 0.0, 0.0, 0.0]),
        doc("D", "alpha alpha", [0.7, 0.0, 0.0, 0.0]),
        doc("E", "alpha", [0.6, 0.0, 0.0, 0.0]),
    ]);
    fixture.close();
    let store = fixture.reopen_read_only();
    let snapshot = store.snapshot();
    let pool = Pool::evaluate(snapshot.filters(), &match_all())
        .unwrap_or_else(|error| panic!("the fixture pool must evaluate: {error}"));
    let whole = pool.len();
    let ranking = lexical::search(&snapshot, &pool, "alpha beta", whole as usize)
        .unwrap_or_else(|error| panic!("the lexical search must run: {error}"));
    let targets: Vec<Iid> = pool.bitmap().iter().map(Iid::new).collect();
    let ranks = lexical::ranks(&snapshot, &pool, "alpha beta", &targets)
        .unwrap_or_else(|error| panic!("the ranks must compute: {error}"));
    assert_eq!(
        ranks.len(),
        ranking.len(),
        "every scored passage of the pool has a rank"
    );
    for (position, scored) in ranking.iter().enumerate() {
        let (rank, score) = ranks
            .get(&scored.iid)
            .copied()
            .unwrap_or_else(|| panic!("{} must have a rank", scored.key));
        assert_eq!(
            rank as usize,
            position + 1,
            "the rank of {} is its position in the pool ranking",
            scored.key
        );
        assert_eq!(
            score, scored.score,
            "the counted rank carries the score of the ranking"
        );
    }
    // The rank counts the whole pool, not only the targets: a second-ranked
    // passage keeps rank 2 when it is the only one asked for.
    let single = lexical::ranks(&snapshot, &pool, "alpha beta", &[ranking[1].iid])
        .unwrap_or_else(|error| panic!("the rank must compute: {error}"));
    assert_eq!(
        single.get(&ranking[1].iid).map(|(rank, _)| *rank),
        Some(2),
        "the passages above a lone target still count"
    );
}

#[test]
fn a_passage_absorbed_by_dedup_names_the_result_that_kept_it() {
    // ADR-0123, decision 5: `grouped_under` names the key rendered in `results`
    // that absorbed the passage by `dedup: "content"`.
    let mut first = doc("A", "alpha", [1.0, 0.0, 0.0, 0.0]);
    let mut second = doc("B", "alpha", [0.5, 0.0, 0.0, 0.0]);
    let shared = "blake3:shared-content".to_owned();
    first.passage_hash = shared.clone();
    second.passage_hash = shared;
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&[first, second]);
    fixture.close();
    let store = fixture.reopen_read_only();
    let snapshot = store.snapshot();
    let searcher = fixture.searcher(&snapshot);
    let filter = match_all();
    let pool = Pool::evaluate(snapshot.filters(), &filter)
        .unwrap_or_else(|error| panic!("the fixture pool must evaluate: {error}"));
    let grouping = Grouping {
        dedup_content: true,
        ..Grouping::none()
    };
    let answer = pipeline::search_captured(
        &snapshot,
        &searcher,
        &pool,
        &Request {
            mode: Mode::Lexical,
            vector: None,
            text: Some("alpha"),
            k: 10,
            exact: false,
            grouping: &grouping,
            fusion: FusionConfig::Rrf,
        },
        &exact_settings(),
        None,
        true,
    )
    .unwrap_or_else(|error| panic!("the pipeline must run: {error}"));
    let kept = survivors(&searcher, &answer.results, true)
        .unwrap_or_else(|error| panic!("the survivors must build: {error}"));
    let ids = vec![ExplainId::Passage(
        "B#0".parse().expect("B#0 is a passage key"),
    )];
    let entries = explain(
        &snapshot,
        &ExplainInput {
            pool: &pool,
            searcher: &searcher,
            filter: &filter,
            mode: Mode::Lexical,
            vector: None,
            text: Some("alpha"),
            search: &exact_settings(),
            requested: candidates_per_branch(10, &grouping),
            exact: false,
            strategy: None,
            pre_group_ranks: &answer.pre_group_ranks,
            survivors: &kept,
        },
        &ids,
    )
    .unwrap_or_else(|error| panic!("the explanation must succeed: {error}"));
    assert_eq!(entries[0].grouped_under.as_deref(), Some("A#0"));
}

#[test]
fn under_dedup_grouped_under_follows_the_content_before_the_document() {
    // ADR-0123, complement of 2026-11-01 (C4): under `dedup: "content"`, the
    // rendered passage of the same digest comes first. `B#0` shares its digest
    // with `A#0`, and the rendered sibling `B#1` must not win.
    let mut fixture = DenseFixture::create(4);
    fixture.add_generation(&[
        Sample::new("A", 0, query()).with_passage_hash("blake3:shared"),
        Sample::new("B", 0, vec![0.5, 0.0, 0.0, 0.0]).with_passage_hash("blake3:shared"),
        Sample::new("B", 1, vec![0.4, 0.0, 0.0, 0.0]).with_passage_hash("blake3:other"),
    ]);
    let filter = match_all();
    let snapshot = fixture.snapshot();
    let searcher = fixture.searcher();
    let pool = fixture.pool(&filter);
    let grouping = Grouping {
        dedup_content: true,
        ..Grouping::none()
    };
    let answer = pipeline::search_captured(
        &snapshot,
        &searcher,
        &pool,
        &Request {
            mode: Mode::Dense,
            vector: Some(&query()),
            text: None,
            k: 10,
            exact: false,
            grouping: &grouping,
            fusion: FusionConfig::Rrf,
        },
        &exact_settings(),
        None,
        true,
    )
    .unwrap_or_else(|error| panic!("the pipeline must run: {error}"));
    let kept = survivors(&searcher, &answer.results, true)
        .unwrap_or_else(|error| panic!("the survivors must build: {error}"));
    let keys: Vec<&str> = kept.iter().map(|survivor| survivor.key.as_str()).collect();
    assert_eq!(
        keys,
        vec!["A#0", "B#1"],
        "the digest of A#0 absorbs B#0, and B#1 survives"
    );
    let ids = vec![ExplainId::Passage(
        "B#0".parse().expect("B#0 is a passage key"),
    )];
    let entries = explain(
        &snapshot,
        &ExplainInput {
            pool: &pool,
            searcher: &searcher,
            filter: &filter,
            mode: Mode::Dense,
            vector: Some(&query()),
            text: None,
            search: &exact_settings(),
            requested: candidates_per_branch(10, &grouping),
            exact: false,
            strategy: answer.strategy,
            pre_group_ranks: &answer.pre_group_ranks,
            survivors: &kept,
        },
        &ids,
    )
    .unwrap_or_else(|error| panic!("the explanation must succeed: {error}"));
    assert_eq!(
        entries[0].grouped_under.as_deref(),
        Some("A#0"),
        "the passage of the same digest absorbs it, before the sibling of its document"
    );
}

#[test]
fn a_synthetic_fixture_explains_dense_rank_70_and_lexical_rank_2() {
    // ADR-0123, decision 11: the case of the fiche, proved without the model and
    // without `data/legifacile-sample`. Sixty-nine passages score above the
    // target dense-wise, and one passage outscores it on the lexical side only.
    let mut docs: Vec<Doc> = Vec::with_capacity(70);
    // `A` is the lexical winner: a short text with the term twice. Its key sorts
    // before `X`, so the tie on the dense score puts it above the target.
    docs.push(doc("A", "alpha alpha", [100.0, 0.0, 0.0, 0.0]));
    // Sixty-eight neutral passages, scores 99 down to 32.
    for (position, score) in (32..=99).rev().enumerate() {
        docs.push(doc(
            &format!("N{position:02}"),
            "rien",
            [score as f32, 0.0, 0.0, 0.0],
        ));
    }
    // The target: the lowest dense score of the pool, and a long text so one
    // occurrence of the term scores below `Y`.
    let long = format!("alpha{}", " neutre".repeat(200));
    docs.push(doc("X", &long, [31.0, 0.0, 0.0, 0.0]));
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&docs);
    fixture.close();
    let store = fixture.reopen_read_only();
    let snapshot = store.snapshot();
    let searcher = fixture.searcher(&snapshot);
    let filter = match_all();
    let pool = Pool::evaluate(snapshot.filters(), &filter)
        .unwrap_or_else(|error| panic!("the fixture pool must evaluate: {error}"));
    let grouping = Grouping::none();
    let answer = pipeline::search_captured(
        &snapshot,
        &searcher,
        &pool,
        &Request {
            mode: Mode::Hybrid,
            vector: Some(&query()),
            text: Some("alpha"),
            k: 10,
            exact: false,
            grouping: &grouping,
            fusion: FusionConfig::Rrf,
        },
        &exact_settings(),
        None,
        true,
    )
    .unwrap_or_else(|error| panic!("the pipeline must run: {error}"));
    let kept = survivors(&searcher, &answer.results, false)
        .unwrap_or_else(|error| panic!("the survivors must build: {error}"));
    let ids = vec![ExplainId::Doc(DocId::new("X").expect("a doc_id"))];
    let entries = explain(
        &snapshot,
        &ExplainInput {
            pool: &pool,
            searcher: &searcher,
            filter: &filter,
            mode: Mode::Hybrid,
            vector: Some(&query()),
            text: Some("alpha"),
            search: &exact_settings(),
            requested: candidates_per_branch(10, &grouping),
            exact: false,
            strategy: answer.strategy,
            pre_group_ranks: &answer.pre_group_ranks,
            survivors: &kept,
        },
        &ids,
    )
    .unwrap_or_else(|error| panic!("the explanation must succeed: {error}"));
    let entry = &entries[0];
    let dense = entry.dense.as_ref().expect("the dense branch ran");
    assert_eq!(dense.exact_rank, 70, "sixty-nine passages score above X");
    let lexical = entry.lexical.as_ref().expect("the lexical branch scored X");
    assert_eq!(lexical.rank, 2, "`A` outscores X on the lexical side");
    assert_eq!(lexical.key, "X#0");
}

#[test]
fn the_bound_and_the_dedup_of_ids_are_the_ones_of_the_adr() {
    // ADR-0123, decision 3: the bound is a constant, and two forms of the same
    // document are two ids.
    assert_eq!(MAX_IDS, 100);
    let doc_id = ExplainId::parse_at("A", "$.explain_ids[0]")
        .unwrap_or_else(|error| panic!("A is a doc_id: {error}"));
    let key = ExplainId::parse_at("A#0", "$.explain_ids[1]")
        .unwrap_or_else(|error| panic!("A#0 is a key: {error}"));
    assert_ne!(doc_id, key);
    // ADR-0123, complement of 2026-11-01 (C1): an id is a key when it ends with
    // `#` and a decimal `ord`; every other id is a `doc_id`, `#` included.
    let hash_doc = ExplainId::parse_at("A#x", "$.explain_ids[0]")
        .unwrap_or_else(|error| panic!("A#x is a doc_id: {error}"));
    assert_eq!(
        hash_doc,
        ExplainId::Doc(DocId::new("A#x").expect("A#x is a legal doc_id"))
    );
    let nested = ExplainId::parse_at("A#3#0", "$.explain_ids[1]")
        .unwrap_or_else(|error| panic!("A#3#0 is a key: {error}"));
    assert_eq!(nested.doc_id().as_str(), "A#3", "the split is the last `#`");
    assert_eq!(
        nested,
        ExplainId::Passage(
            "A#3#0"
                .parse()
                .unwrap_or_else(|error| panic!("A#3#0 is a key: {error}"))
        )
    );
    // An empty id and a key whose document part is empty are refused.
    for raw in ["", "#3"] {
        let failure = ExplainId::parse_at(raw, "$.explain_ids[0]")
            .expect_err("an empty id or an empty document part is refused");
        assert_eq!(
            failure.code(),
            crible_core::error::ErrorCode::InvalidRequest
        );
        assert!(
            failure.message().contains("$.explain_ids[0]"),
            "{}",
            failure.message()
        );
    }
    // The pool of a fixture without a lexical branch is still explained: a branch
    // that did not run has no object.
    let mut fixture = DenseFixture::create(4);
    fixture.add_generation(&[Sample::new("A", 0, query())]);
    let run = DenseRun::open(&fixture, &match_all());
    let entries = run.explain(
        &match_all(),
        &exact_settings(),
        &[ExplainId::Doc(DocId::new("A").expect("a doc_id"))],
    );
    assert!(entries[0].lexical.is_none());
    assert_eq!(
        entries[0].dense.as_ref().map(|dense| dense.exact_rank),
        Some(1)
    );
}

#[test]
fn an_id_that_ends_with_a_decimal_is_a_key_and_any_other_is_a_doc_id() {
    // ADR-0123, complement of 2026-11-01 (C1): a document whose `doc_id` ends
    // with `#<integer>` is explained passage by passage; a `doc_id` that holds
    // `#` without that shape is explained as a document.
    let mut fixture = DenseFixture::create(4);
    fixture.add_generation(&[
        Sample::new("A#3", 0, query()),
        Sample::new("A#x", 0, vec![0.5, 0.0, 0.0, 0.0]),
    ]);
    let run = DenseRun::open(&fixture, &match_all());
    let ids = vec![
        ExplainId::parse_at("A#3#0", "$.explain_ids[0]")
            .unwrap_or_else(|error| panic!("A#3#0 is a key: {error}")),
        ExplainId::parse_at("A#x", "$.explain_ids[1]")
            .unwrap_or_else(|error| panic!("A#x is a doc_id: {error}")),
        ExplainId::parse_at("A#3", "$.explain_ids[2]")
            .unwrap_or_else(|error| panic!("A#3 is a key: {error}")),
    ];
    let entries = run.explain(&match_all(), &exact_settings(), &ids);
    assert!(entries[0].exists, "the document A#3 exists");
    assert_eq!(entries[0].doc_id, "A#3");
    assert_eq!(entries[0].id, "A#3#0", "the key is rendered as requested");
    assert_eq!(
        entries[0].dense.as_ref().map(|dense| dense.key.as_str()),
        Some("A#3#0")
    );
    assert!(entries[1].exists, "the document A#x exists");
    assert_eq!(entries[1].id, "A#x");
    assert_eq!(entries[1].doc_id, "A#x");
    assert!(
        !entries[2].exists,
        "A#3 is read as the key of passage 3 of the document A, which does not exist"
    );
    assert_eq!(entries[2].doc_id, "A");
}
