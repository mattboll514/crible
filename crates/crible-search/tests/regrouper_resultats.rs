// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `REGROUPER-RESULTATS` — `group_by: "document"`, `passages_per_document` and
//! `dedup: "content"` on a real index.
//!
//! References: `docs/phases/P4-recherche-rapide.md` (`REGROUPER-RESULTATS`,
//! phase P4), `docs/03-recherche.md` ("Regroupement", and step 7 of the pipeline
//! table), `docs/05-api.md` (`group_by`, `passages_per_document`, `dedup`),
//! ADR-0074 (the `k` of the dense branch is `candidates_per_branch`) and
//! ADR-0075 (order of the two steps, over-sampling layer, shape of the answer,
//! candidates under `BINARY`).
//!
//! # What each test pins
//!
//! | Test | Acceptance criterion |
//! |---|---|
//! | `group_by_document_returns_k_distinct_documents` | "on obtient k documents distincts" |
//! | `the_best_passage_of_a_document_is_the_one_kept` | "le meilleur passage de chaque document est conservé" |
//! | `dedup_content_fuses_passages_of_the_same_digest` | "les doublons de contenu sont fusionnés" |
//! | `a_document_with_several_passages_in_the_top_fills_passages_per_document` | "des tests couvrent un document qui a plusieurs passages dans le top" |
//! | `grouped_entries_of_a_document_are_consecutive` | ADR-0075 §3: the entries of one document are consecutive |
//! | `dedup_runs_before_grouping_so_k_documents_survive` | ADR-0075 §1: dedup first, then the grouping fills up again |
//! | `fewer_than_k_documents_when_the_candidates_run_out` | ADR-0075 §1: fewer than `k` documents, without error |
//! | `a_request_that_regroups_nothing_keeps_k_passages` | `group_by` and `dedup` null: no grouping |
//! | `passages_per_document_out_of_bounds_is_refused` | the domain of `docs/05-api.md` (1 to 5) |
//! | `binary_scan_keeps_max_of_n_and_the_over_sampled_candidates` | ADR-0075 §4, ADR-0074 decision 4 |
//!
//! The scores are the dot product of the query with the stored vectors, and
//! the fixture stores them unnormalised, so each sample's score is readable
//! from its single non-zero component: the fixture orders the candidates and
//! the tests can state the expected answer in one line.
//!
//! `dense::search_grouped` is the pipeline entry point under test: candidates,
//! regrouping, then payload. The over-sampling rule itself is pinned by the unit
//! tests of `crible_search::group::candidates_per_branch`.

mod common;

use crible_core::config::{FusionConfig, ScanStrategy, SearchConfig};
use crible_search::dense::exact::Hit;
use crible_search::dense::{DenseQuery, search_grouped};
use crible_search::group::{Grouping, candidates_per_branch};

use common::dense::{DenseFixture, Sample, match_all};

/// Dimensions of the fixture: one component carries the score, the others stay
/// zero, so the expected order is the order of the first component.
const DIMS: u32 = 4;

/// Digest shared by `A#0` and `B#0`: the two passages have the same content.
const SHARED_DIGEST: &str = "blake3:shared-content";

// --- Settings ---

/// Dense settings that always search exactly, so the answer depends on the
/// regrouping alone (`docs/03-recherche.md`, strategy table: `scan: none`).
fn exact_settings() -> SearchConfig {
    SearchConfig {
        scan: ScanStrategy::None,
        rescore_candidates: 1_000,
        exact_threshold: 1_000,
        candidates_per_branch: 100,
        fusion: FusionConfig::Rrf,
    }
}

/// Query of the tests: its norm is 1, so a score is the first component of the
/// stored vector.
fn query() -> Vec<f32> {
    vec![1.0, 0.0, 0.0, 0.0]
}

// --- Fixture ---

/// One passage whose score with [`query`] is `score`.
fn sample(doc_id: &str, ord: u32, score: f32) -> Sample {
    Sample::new(doc_id, ord, vec![score, 0.0, 0.0, 0.0])
}

/// The fixture of every test: two passages of `A`, one of `B` whose content is
/// the one of `A#0`, one of `C`, two of `D`, one of `E`.
///
/// | Key | Score | Digest |
/// |---|---|---|
/// | `A#0` | 4.0 | `SHARED_DIGEST` |
/// | `A#1` | 3.0 | its own |
/// | `B#0` | 2.5 | `SHARED_DIGEST` |
/// | `C#0` | 2.0 | its own |
/// | `D#0` | 1.5 | its own |
/// | `D#1` | 1.2 | its own |
/// | `E#0` | 1.0 | its own |
fn fixture() -> DenseFixture {
    let mut fixture = DenseFixture::create(DIMS);
    let samples = vec![
        sample("A", 0, 4.0).with_passage_hash(SHARED_DIGEST),
        sample("A", 1, 3.0),
        sample("B", 0, 2.5).with_passage_hash(SHARED_DIGEST),
        sample("C", 0, 2.0),
        sample("D", 0, 1.5),
        sample("D", 1, 1.2),
        sample("E", 0, 1.0),
    ];
    fixture.add_generation(&samples);
    fixture
}

/// A fixture whose passages are **interleaved** in the global ranking: the two
/// passages of `A` and of `B` alternate by score.
///
/// | Key | Score |
/// |---|---|
/// | `A#0` | 4.0 |
/// | `B#0` | 3.5 |
/// | `A#1` | 3.0 |
/// | `B#1` | 2.5 |
///
/// The global order would write `A#0, B#0, A#1, B#1`; ADR-0075 decision 3
/// requires the entries of one document to be consecutive, so the grouped answer
/// is `A#0, A#1, B#0, B#1`.
fn interleaved_fixture() -> DenseFixture {
    let mut fixture = DenseFixture::create(DIMS);
    let samples = vec![
        sample("A", 0, 4.0),
        sample("B", 0, 3.5),
        sample("A", 1, 3.0),
        sample("B", 1, 2.5),
    ];
    fixture.add_generation(&samples);
    fixture
}

/// A fixture of `documents` documents with two passages each, whose scores rank
/// document `D0` first, then `D1`, and so on, both passages of a document
/// together.
///
/// It is the fixture of the `BINARY` test: ten documents give twenty passages,
/// so a grouped answer of `k = 10` with `passages_per_document = 2` needs twenty
/// candidates. A scan that kept fewer than the over-sampled count would not
/// reach them.
fn paired_fixture(documents: u32) -> DenseFixture {
    let mut fixture = DenseFixture::create(DIMS);
    let mut samples = Vec::new();
    for index in 0..documents {
        let doc_id = format!("D{index}");
        let first = 200.0 - 2.0 * index as f32;
        samples.push(sample(&doc_id, 0, first));
        samples.push(sample(&doc_id, 1, first - 1.0));
    }
    fixture.add_generation(&samples);
    fixture
}

// --- Running the pipeline ---

/// Runs the grouped pipeline of `k` results with `grouping`, exactly.
fn search(fixture: &DenseFixture, k: usize, grouping: &Grouping) -> Vec<Hit> {
    search_with(fixture, k, grouping, &exact_settings())
}

/// Runs the grouped pipeline of `k` results with `grouping` and `settings`.
fn search_with(
    fixture: &DenseFixture,
    k: usize,
    grouping: &Grouping,
    settings: &SearchConfig,
) -> Vec<Hit> {
    let snapshot = fixture.snapshot();
    let searcher = fixture.searcher();
    let pool = fixture.pool(&match_all());
    search_grouped(
        &snapshot,
        &searcher,
        &pool,
        &DenseQuery {
            vector: &query(),
            k,
            exact: false,
            grouping,
        },
        settings,
    )
    .unwrap_or_else(|error| panic!("the grouped search must succeed: {error}"))
}

/// Keys of a result list, in order.
fn keys(hits: &[Hit]) -> Vec<String> {
    hits.iter().map(|hit| hit.key.clone()).collect()
}

/// Distinct document identifiers of a result list, in the order they appear.
fn documents(hits: &[Hit]) -> Vec<String> {
    let mut documents: Vec<String> = Vec::new();
    for hit in hits {
        if !documents.contains(&hit.doc_id) {
            documents.push(hit.doc_id.clone());
        }
    }
    documents
}

/// The grouping of a request that asks `group_by: "document"`.
fn by_document(passages_per_document: usize) -> Grouping {
    Grouping {
        by_document: true,
        passages_per_document,
        dedup_content: false,
    }
}

/// The grouping of a request that asks `dedup: "content"`.
fn dedup() -> Grouping {
    Grouping {
        dedup_content: true,
        ..Grouping::none()
    }
}

// --- Criteria ---

/// Acceptance criterion 1: a grouped search returns `k` **distinct** documents,
/// and no more, even though the best candidates hold two passages of the same
/// document.
#[test]
fn group_by_document_returns_k_distinct_documents() {
    let fixture = fixture();
    let hits = search(&fixture, 4, &by_document(1));

    assert_eq!(
        documents(&hits),
        vec!["A", "B", "C", "D"],
        "four results must be four distinct documents, best first"
    );
    assert_eq!(
        keys(&hits),
        vec!["A#0", "B#0", "C#0", "D#0"],
        "each document is represented by one passage"
    );
}

/// Acceptance criterion 2: the passage kept for a document is its **best** one.
/// `A#1` scores 3.0 and matches as well as `A#0`, but `A#0` scores 4.0.
#[test]
fn the_best_passage_of_a_document_is_the_one_kept() {
    let fixture = fixture();
    let hits = search(&fixture, 3, &by_document(1));

    let documents = documents(&hits);
    assert_eq!(documents, vec!["A", "B", "C"]);
    assert_eq!(
        hits.first().map(|hit| hit.key.as_str()),
        Some("A#0"),
        "the document A is represented by A#0 (4.0), not by A#1 (3.0)"
    );
    assert!(
        !keys(&hits).contains(&"A#1".to_owned()),
        "with passages_per_document = 1, the second passage of A is dropped"
    );
}

/// Acceptance criterion 3: two passages of the same `passage_hash` are fused,
/// and the better one is kept. `A#0` (4.0) and `B#0` (2.5) share their digest.
#[test]
fn dedup_content_fuses_passages_of_the_same_digest() {
    let fixture = fixture();
    let hits = search(&fixture, 10, &dedup());

    let keys = keys(&hits);
    assert!(
        keys.contains(&"A#0".to_owned()),
        "the best copy of the shared content is kept: {keys:?}"
    );
    assert!(
        !keys.contains(&"B#0".to_owned()),
        "the other copy of the shared content is fused: {keys:?}"
    );
    assert_eq!(
        keys.len(),
        6,
        "seven passages match, and the duplicate removes one: {keys:?}"
    );
    assert_eq!(
        documents(&hits),
        vec!["A", "C", "D", "E"],
        "dedup alone keeps passages, not documents: A keeps both of its own"
    );
}

/// Acceptance criterion 4: a document with several passages in the top is
/// covered. With `passages_per_document = 2`, `A` takes its two passages, and
/// the grouping stops after `k` documents.
#[test]
fn a_document_with_several_passages_in_the_top_fills_passages_per_document() {
    let fixture = fixture();
    let hits = search(&fixture, 3, &by_document(2));

    assert_eq!(
        documents(&hits),
        vec!["A", "B", "C"],
        "k counts documents, not results"
    );
    assert_eq!(
        keys(&hits),
        vec!["A#0", "A#1", "B#0", "C#0"],
        "the extra passage of A is added after the best one, in score order"
    );
    assert_eq!(
        hits.iter().filter(|hit| hit.doc_id == "A").count(),
        2,
        "passages_per_document = 2 adds one passage to the best one"
    );
}

/// Step 7 order and recompletion (ADR-0075, decision 1): deduplication runs
/// **before** the grouping, which then fills up again from the rest of the list.
///
/// `A#0` and `B#0` share their content. Deduplicating first keeps `A#0` and
/// removes `B#0`, so `C` and `D` fill the three documents. Deduplicating after
/// the grouping would keep `A#0`, `B#0` and `C#0`, then fuse `B#0` into `A#0`,
/// and the answer would hold two documents for `k = 3`.
#[test]
fn dedup_runs_before_grouping_so_k_documents_survive() {
    let fixture = fixture();
    let grouping = Grouping {
        by_document: true,
        passages_per_document: 1,
        dedup_content: true,
    };
    let hits = search(&fixture, 3, &grouping);

    assert_eq!(
        documents(&hits),
        vec!["A", "C", "D"],
        "the document whose only passage was fused gives way to the next one: \
         the answer still holds k = 3 distinct documents"
    );
    assert_eq!(keys(&hits), vec!["A#0", "C#0", "D#0"]);
}

/// Without `group_by` and without `dedup`, the answer is the plain top-k of the
/// passages: two passages of the same document may come back.
#[test]
fn a_request_that_regroups_nothing_keeps_k_passages() {
    let fixture = fixture();
    let hits = search(&fixture, 2, &Grouping::none());

    assert_eq!(
        keys(&hits),
        vec!["A#0", "A#1"],
        "no grouping: k passages, not k documents"
    );
}

/// `passages_per_document` accepts 1 to 5 only (`docs/05-api.md`); a higher
/// value is refused with `invalid_request` instead of being clamped silently.
#[test]
fn passages_per_document_out_of_bounds_is_refused() {
    let fixture = fixture();
    let snapshot = fixture.snapshot();
    let searcher = fixture.searcher();
    let pool = fixture.pool(&match_all());
    for passages in [0, Grouping::MAX_PASSAGES_PER_DOCUMENT + 1] {
        let grouping = by_document(passages);
        let outcome = search_grouped(
            &snapshot,
            &searcher,
            &pool,
            &DenseQuery {
                vector: &query(),
                k: 3,
                exact: false,
                grouping: &grouping,
            },
            &exact_settings(),
        );
        let error = outcome
            .err()
            .unwrap_or_else(|| panic!("passages_per_document = {passages} must be refused"));
        assert_eq!(
            error.code(),
            crible_core::error::ErrorCode::InvalidRequest,
            "the request is invalid, not the store"
        );
    }
}

/// The over-sampling of `docs/03-recherche.md`: `clamp(k × 20, 200, 1000)` when
/// the request regroups, `max(k, 100)` otherwise. The dense branch receives
/// this count as its `k`, so the regrouping has candidates to drop.
#[test]
fn candidates_per_branch_follows_the_over_sampling_rule() {
    let grouping = by_document(1);
    for (k, expected) in [
        (1, 200),
        (9, 200),
        (10, 200),
        (11, 220),
        (50, 1_000),
        (100, 1_000),
        (1_000, 1_000),
    ] {
        assert_eq!(
            candidates_per_branch(k, &grouping),
            expected,
            "grouping with k = {k}"
        );
    }
    for (k, expected) in [(1, 100), (10, 100), (100, 100), (150, 150), (1_000, 1_000)] {
        assert_eq!(
            candidates_per_branch(k, &Grouping::none()),
            expected,
            "no regrouping with k = {k}"
        );
    }
}

/// ADR-0075, decision 3: the entries of one document are **consecutive**.
///
/// The fixture interleaves the passages of `A` and of `B` in the global
/// ranking. Keeping the global order would answer `A#0, B#0, A#1, B#1`; the
/// grouping must gather the two passages of a document.
#[test]
fn grouped_entries_of_a_document_are_consecutive() {
    let fixture = interleaved_fixture();
    let hits = search(&fixture, 2, &by_document(2));

    assert_eq!(
        keys(&hits),
        vec!["A#0", "A#1", "B#0", "B#1"],
        "the passages of A come one after the other, then those of B, although \
         the global ranking interleaves them"
    );
    assert_eq!(
        documents(&hits),
        vec!["A", "B"],
        "the documents are ranked by their best passage"
    );
    let mut closed: Vec<String> = Vec::new();
    for window in hits.windows(2) {
        if window[0].doc_id != window[1].doc_id {
            closed.push(window[0].doc_id.clone());
            assert!(
                !closed.contains(&window[1].doc_id),
                "no document may reappear after another one: {:?}",
                keys(&hits)
            );
        }
    }
}

/// ADR-0075, decision 1: a list that runs out before `k` documents gives fewer
/// documents, without error.
///
/// The fixture holds five documents, and the request asks for ten: the answer
/// keeps the five it found. A small pool or a selective filter is a valid
/// request, not a failure.
#[test]
fn fewer_than_k_documents_when_the_candidates_run_out() {
    let fixture = fixture();
    let hits = search(&fixture, 10, &by_document(1));

    assert_eq!(
        documents(&hits),
        vec!["A", "B", "C", "D", "E"],
        "the five documents of the pool come back, in the order of their best passage"
    );
    assert_eq!(hits.len(), 5, "five documents, one entry each");
}

/// ADR-0075, decision 4 and ADR-0074, decision 4: under `BINARY`, the scan keeps
/// `max(N, candidates_per_branch)` candidates, so the rescoring covers the
/// over-sampled list.
///
/// The fixture holds 30 documents of two passages each, the filter keeps all 60,
/// and `N` is 10 while `candidates_per_branch(10, grouping)` is 200. All 60
/// passages are therefore rescored, and the grouped answer is the one of the
/// exact strategy: ten documents and twenty entries. A scan that kept `k = 10`
/// candidates could not return twenty entries.
#[test]
fn binary_scan_keeps_max_of_n_and_the_over_sampled_candidates() {
    let fixture = paired_fixture(30);
    let settings = SearchConfig {
        scan: ScanStrategy::Binary,
        rescore_candidates: 10,
        exact_threshold: 0,
        candidates_per_branch: 100,
        fusion: FusionConfig::Rrf,
    };
    let grouping = by_document(2);
    let requested = candidates_per_branch(10, &grouping);

    let pool = fixture.pool(&match_all());
    let planned = crible_search::dense::plan(&settings, pool.len(), requested, false);
    assert_eq!(
        planned.rescore_candidates,
        u32::try_from(requested).unwrap_or(u32::MAX),
        "under BINARY the plan rescoring follows the over-sampled candidate count, not k"
    );
    assert!(
        planned.rescore_candidates > settings.rescore_candidates,
        "the over-sampling must raise the rescoring above N"
    );

    let binary = search_with(&fixture, 10, &grouping, &settings);
    let exact = search(&fixture, 10, &grouping);
    assert_eq!(
        binary.len(),
        20,
        "ten documents of two passages each need twenty candidates: a scan limited \
         to k = 10 could not reach them"
    );
    assert_eq!(
        keys(&binary),
        keys(&exact),
        "with every passage rescored, the BINARY answer is the exact one"
    );
}
