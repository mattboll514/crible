// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `REDUIRE-COUT-DES-CLES`: reading the keys **after** the sort by score must
//! not change the ranking.
//!
//! References: `docs/phases/P4-recherche-rapide.md` (`REDUIRE-COUT-DES-CLES`,
//! phase P4), ADR-0079 (decisions 2, 3 and 6), ADR-0072 (decision 3: scores and
//! order bit-identical to `EXACT`), ADR-0045 (descending score, then ascending
//! textual key), ADR-0075 (`dedup` then `group_by`, candidates consumed by the
//! regrouping) and INV-06.
//!
//! # Why these tests exist before the change
//!
//! `ExactSearcher::rescore` reads the key of **every** candidate today, sorts
//! the whole list with [`Scored`](crible_search::dense::Scored) and the planner
//! truncates it to `k`. Reading the keys only for the retained candidates is
//! the change of ADR-0079, decision 3. It is wrong in one case: when several
//! candidates share the score of the frontier, only the keys tell them apart,
//! and truncating on `(score, iid)` before reading them selects the ex aequo by
//! internal identifier instead of by text (ADR-0079, decision 6).
//!
//! Every fixture here is built so that the **iid order is the reverse of the
//! key order**: the answer an iid cut would give is written beside the expected
//! one, and asserted different, so the test cannot pass vacuously.
//!
//! The duplicated vectors make the ties exact **bit for bit** — the same stored
//! components, the same kernel, the same score — so the comparison with `EXACT`
//! is an equality and not a tolerance.
//!
//! # What each test pins
//!
//! | Test | Criterion |
//! |---|---|
//! | `ties_at_the_k_boundary_are_broken_by_the_key_not_the_iid` | the criterion "égalités de score à la frontière", ties straddling `k` |
//! | `a_single_tie_inside_k_and_the_others_beyond_matches_exact` | ties outside `k` but one |
//! | `all_candidates_tied_match_exact` | every candidate ex aequo |
//! | `k_above_the_candidate_count_returns_every_candidate` | `k` greater than the number of candidates |
//! | `the_answer_does_not_depend_on_the_insertion_order` | the criterion "ne dépend pas de l'ordre d'itération" |
//! | `group_by_document_breaks_the_document_boundary_ties_by_the_key` | ADR-0075: tied boundary of `k` documents |
//! | `group_by_document_reads_beyond_k_candidates_to_fill_k_documents` | ADR-0075: the grouping consumes more than `k` candidates |
//! | `dedup_content_breaks_the_boundary_ties_by_the_key` | ADR-0075: `dedup` at a tied boundary |
//! | `dedup_content_reads_beyond_k_so_the_answer_holds_k_results` | ADR-0075: `dedup` consumes more than `k` candidates |
//! | `dedup_then_group_by_matches_exact_on_a_tied_boundary` | ADR-0075 §1: `dedup` then `group_by`, tied boundary |
//! | `inv_06_binary_rescoring_of_a_filtered_pool_returns_the_exact_result` | INV-06 on the rescoring path, which no `inv_06_*` test covered |
//!
//! # What the tests do not call
//!
//! They go through [`crible_search::dense::search`] and
//! [`crible_search::dense::search_grouped`], the two entry points of the
//! pipeline, and never through `rescore` directly: the number of candidates a
//! rescoring keeps is a planning decision (ADR-0074, decision 4), and the
//! change of ADR-0079 may carry it in the call. A test bound to the signature
//! of `rescore` would constrain the implementation instead of the behaviour.

mod common;

use crible_core::config::{FusionConfig, ScanStrategy, SearchConfig};
use crible_core::filter::Filter;
use crible_search::dense::exact::Hit;
use crible_search::dense::{DenseQuery, Strategy, candidates, plan, search, search_grouped};
use crible_search::group::Grouping;

use common::dense::{DenseFixture, Sample, match_all};

/// Dimensions of the fixture. One component carries the score, the others stay
/// zero, so a stored vector `[s, 0, 0, 0]` scores exactly `s` against
/// [`query`]: the expected order is readable without computing a cosine.
const DIMS: u32 = 4;

/// Digest shared by the passages a `dedup: content` test must fuse.
const SHARED_DIGEST: &str = "blake3:shared-content";

/// Query of the tests: its norm is 1, so the score of `[s, 0, 0, 0]` is `s`.
fn query() -> Vec<f32> {
    vec![1.0, 0.0, 0.0, 0.0]
}

/// One passage scoring `score` against [`query`].
///
/// The scores are integers below 2 048, so the `fp16` reference stores them
/// exactly and two samples with the same `score` really tie.
fn sample(doc_id: &str, ord: u32, score: f32) -> Sample {
    Sample::new(doc_id, ord, vec![score, 0.0, 0.0, 0.0])
}

// --- Settings ---

/// `BINARY` whatever the pool: the threshold is `0`, so only an empty pool would
/// be planned exact. `N` is far above every fixture pool, so the scan keeps the
/// whole pool and the rescoring scores every passage: the answer then depends on
/// the sort alone, which is what the change of ADR-0079 moves.
fn binary_settings() -> SearchConfig {
    SearchConfig {
        scan: ScanStrategy::Binary,
        rescore_candidates: 1_000,
        exact_threshold: 0,
        candidates_per_branch: 100,
        fusion: FusionConfig::Rrf,
    }
}

/// `EXACT` whatever the pool: `scan: none` is "toujours exact"
/// (`docs/03-recherche.md`, strategy table). It is the oracle of every test.
fn exact_settings() -> SearchConfig {
    SearchConfig {
        scan: ScanStrategy::None,
        rescore_candidates: 1_000,
        exact_threshold: 1_000,
        candidates_per_branch: 100,
        fusion: FusionConfig::Rrf,
    }
}

// --- Running the pipeline ---

/// Runs the dense branch of `k` results with `settings`.
fn run(fixture: &DenseFixture, k: usize, settings: &SearchConfig) -> Vec<Hit> {
    run_with_filter(fixture, &match_all(), k, settings)
}

/// Runs the dense branch of `k` results on the pool of `filter`, with
/// `settings`.
fn run_with_filter(
    fixture: &DenseFixture,
    filter: &Filter,
    k: usize,
    settings: &SearchConfig,
) -> Vec<Hit> {
    let snapshot = fixture.snapshot();
    let searcher = fixture.searcher();
    let pool = fixture.pool(filter);
    search(&snapshot, &searcher, &pool, &query(), k, settings, false)
        .unwrap_or_else(|error| panic!("the dense search must succeed: {error}"))
}

/// Runs the dense branch regrouped by `grouping`, with `settings`.
fn run_grouped(
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
fn keys_of(hits: &[Hit]) -> Vec<String> {
    hits.iter().map(|hit| hit.key.clone()).collect()
}

/// Distinct document identifiers of a result list, in the order they appear.
fn doc_ids_of(hits: &[Hit]) -> Vec<String> {
    let mut documents: Vec<String> = Vec::new();
    for hit in hits {
        if !documents.contains(&hit.doc_id) {
            documents.push(hit.doc_id.clone());
        }
    }
    documents
}

/// Keys and score **bits** of a result list, in order.
///
/// The bits, not the `f32`, so "identical to `EXACT`" is an equality and not a
/// tolerance that could hide a different arithmetic (ADR-0072, decision 3).
fn bits_of(hits: &[Hit]) -> Vec<(String, u32)> {
    hits.iter()
        .map(|hit| (hit.key.clone(), hit.score.to_bits()))
        .collect()
}

// --- Assertions ---

/// Asserts that the whole list carries one and the same score, bit for bit: the
/// fixture must exercise an exact tie, not two close scores.
fn assert_one_score(hits: &[Hit], label: &str) {
    let first = hits
        .first()
        .unwrap_or_else(|| panic!("{label}: the list must not be empty"))
        .score
        .to_bits();
    for hit in hits {
        assert_eq!(
            hit.score.to_bits(),
            first,
            "{label}: {} scores {} bits, the tie must be exact, not approximate",
            hit.key,
            hit.score
        );
    }
}

/// The keys an implementation that truncates on `(score, iid)` **before**
/// reading the keys would return: the trap of ADR-0079, decision 6.
///
/// The candidates are read through [`crible_search::dense::candidates`], the
/// step before the regrouping and the payload, so they still carry their
/// internal identifier; the helper then re-sorts them by descending score and
/// ascending iid, and keeps `k`. It is the non-vacuity proof of a fixture: a
/// test asserts that this answer is **not** the expected one, so the fixture
/// really tells the two cuts apart.
fn iid_cut_keys(fixture: &DenseFixture, k: usize, pool_size: usize) -> Vec<String> {
    let snapshot = fixture.snapshot();
    let searcher = fixture.searcher();
    let pool = fixture.pool(&match_all());
    let mut candidates = candidates(
        &snapshot,
        &searcher,
        &pool,
        &query(),
        pool_size,
        &binary_settings(),
        false,
    )
    .unwrap_or_else(|error| panic!("the candidate step must succeed: {error}"));
    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.iid.get().cmp(&right.iid.get()))
    });
    candidates
        .iter()
        .take(k)
        .map(|candidate| candidate.key.clone())
        .collect()
}

/// Asserts that the binary pipeline gives the keys, the order and the score
/// bits of `EXACT`.
fn assert_bit_identical_to_exact(fixture: &DenseFixture, k: usize, label: &str) -> Vec<Hit> {
    let binary = run(fixture, k, &binary_settings());
    let exact = run(fixture, k, &exact_settings());
    assert_eq!(
        bits_of(&binary),
        bits_of(&exact),
        "{label}: BINARY must return the keys, the order and the score bits of EXACT"
    );
    binary
}

/// Asserts that the grouped binary pipeline gives the keys, the order and the
/// score bits of the grouped exact pipeline.
fn assert_grouped_bit_identical_to_exact(
    fixture: &DenseFixture,
    k: usize,
    grouping: &Grouping,
    label: &str,
) -> Vec<Hit> {
    let binary = run_grouped(fixture, k, grouping, &binary_settings());
    let exact = run_grouped(fixture, k, grouping, &exact_settings());
    assert_eq!(
        bits_of(&binary),
        bits_of(&exact),
        "{label}: the grouped BINARY answer must be the grouped EXACT answer, bit for bit"
    );
    binary
}

// --- The boundary of `k` ---

/// The criterion of ADR-0079, decision 6: more ex aequo than places left at
/// rank `k`, with duplicated vectors.
///
/// `t#6` scores 5.0; the six other passages score 3.0 and tie. `k = 4` leaves
/// three places for six ties, so the frontier falls **inside** the tied group,
/// and the answer must be `t#0`, `t#1`, `t#2` — the three smallest keys. The
/// passages are inserted in the order `t#6, t#5 … t#0`, so the iids run the
/// other way: an implementation that truncates on `(score, iid)` before reading
/// the keys returns `t#5`, `t#4`, `t#3` instead, which the test asserts different.
#[test]
fn ties_at_the_k_boundary_are_broken_by_the_key_not_the_iid() {
    let mut fixture = DenseFixture::create(DIMS);
    let mut samples = vec![sample("t", 6, 5.0)];
    for ord in (0..6).rev() {
        samples.push(sample("t", ord, 3.0));
    }
    fixture.add_generation(&samples);

    let binary = assert_bit_identical_to_exact(&fixture, 4, "ties across the k boundary");
    let keys = keys_of(&binary);
    assert_eq!(
        keys,
        ["t#6", "t#0", "t#1", "t#2"],
        "the best passage, then the three smallest keys of the tied group"
    );
    assert_ne!(
        keys,
        ["t#6", "t#5", "t#4", "t#3"],
        "an iid cut would keep the last inserted passages: the fixture must tell the two apart"
    );
    assert_eq!(
        iid_cut_keys(&fixture, 4, 7),
        ["t#6", "t#5", "t#4", "t#3"],
        "the fixture is adversarial: truncating on (score, iid) before reading the keys \
         really returns another answer"
    );
    assert_one_score(&binary[1..], "the tied group of the answer");
    assert_eq!(
        binary[0].score.to_bits(),
        // `5.0` is exact in f16 and the query has a unit norm.
        5.0_f32.to_bits(),
        "the leader must score the 5.0 of its stored vector"
    );
}

/// A tied group that `k` cuts after its **first** member: one ex aequo inside
/// the answer, four beyond it.
///
/// `u#5` scores 9.0 and the five other passages score 3.0; `k = 2` keeps
/// `u#5` and `u#0`, the smallest key of the tied group. The iid cut would keep
/// `u#4`.
#[test]
fn a_single_tie_inside_k_and_the_others_beyond_matches_exact() {
    let mut fixture = DenseFixture::create(DIMS);
    let mut samples = vec![sample("u", 5, 9.0)];
    for ord in (0..5).rev() {
        samples.push(sample("u", ord, 3.0));
    }
    fixture.add_generation(&samples);

    let binary = assert_bit_identical_to_exact(&fixture, 2, "one tie inside k");
    let keys = keys_of(&binary);
    assert_eq!(
        keys,
        ["u#5", "u#0"],
        "the single place left goes to the smallest key of the tied group"
    );
    assert_ne!(
        keys,
        ["u#5", "u#4"],
        "an iid cut would keep the first inserted tie: the fixture must tell the two apart"
    );
}

/// Every candidate tied, `k` smaller than the pool: the answer is the `k`
/// smallest keys of the whole pool, and every score is the same bit pattern.
#[test]
fn all_candidates_tied_match_exact() {
    let mut fixture = DenseFixture::create(DIMS);
    let mut samples = Vec::new();
    for ord in (0..7).rev() {
        samples.push(sample("v", ord, 2.0));
    }
    fixture.add_generation(&samples);

    let binary = assert_bit_identical_to_exact(&fixture, 3, "every candidate tied");
    assert_eq!(
        keys_of(&binary),
        ["v#0", "v#1", "v#2"],
        "a whole pool of ex aequo is ranked by the ascending textual key"
    );
    assert_one_score(&binary, "a pool where every candidate ties");
    assert_eq!(binary.len(), 3, "k = 3 results out of seven candidates");
}

/// `k` above the number of candidates: every candidate comes back, in the order
/// of the ranking — by descending score, and for a pool where all scores tie, by
/// ascending key.
#[test]
fn k_above_the_candidate_count_returns_every_candidate() {
    // Distinct scores, inserted in the reverse of the key order.
    let mut distinct = DenseFixture::create(DIMS);
    let mut samples = Vec::new();
    for position in 0..5_u32 {
        // `w#4` is inserted first, so the iid order is the reverse of the key
        // order, and the scores decrease as the keys grow.
        samples.push(sample("w", 4 - position, 5.0 - position as f32));
    }
    distinct.add_generation(&samples);

    let binary = assert_bit_identical_to_exact(&distinct, 10, "k above the candidate count");
    assert_eq!(
        keys_of(&binary),
        ["w#4", "w#3", "w#2", "w#1", "w#0"],
        "a k above the pool returns the whole pool, best first"
    );

    // Every candidate tied, inserted in the reverse of the key order.
    let mut tied = DenseFixture::create(DIMS);
    let mut samples = Vec::new();
    for ord in (0..5).rev() {
        samples.push(sample("x", ord, 2.0));
    }
    tied.add_generation(&samples);

    let binary = assert_bit_identical_to_exact(&tied, 10, "k above a pool where all scores tie");
    assert_eq!(
        keys_of(&binary),
        ["x#0", "x#1", "x#2", "x#3", "x#4"],
        "a k above the pool returns every candidate, ties by ascending key"
    );
    assert_one_score(&binary, "k above a pool where all scores tie");
}

/// The criterion "le résultat ne dépend pas de l'ordre d'itération": the same
/// logical index, fed to the engine in three orders, returns the same answer.
///
/// The samples are the ones of [`order_fixture_samples`]: `a` scores 5.0, `b`,
/// `c`, `d` and `e` score 3.0 and tie, `f` scores 1.0; `k = 3` keeps `a` and the
/// two smallest keys of the tied group, `b` and `c`.
///
/// The three layouts are: the ascending order (`a` first, so the iid order is the
/// key order), the reverse order (`f` first, so the iid order is the reverse of
/// the key order), and two generations that split the samples, so the iids and
/// the segments differ from both. The answer must be the same three times, and
/// be the one of `EXACT` each time.
#[test]
fn the_answer_does_not_depend_on_the_insertion_order() {
    let expected = ["a#0", "b#0", "c#0"];

    let mut ascending = DenseFixture::create(DIMS);
    ascending.add_generation(&order_fixture_samples(false));

    let mut reverse = DenseFixture::create(DIMS);
    reverse.add_generation(&order_fixture_samples(true));

    // Two generations: the same six passages, two segments, other iid ranges.
    let mut split = DenseFixture::create(DIMS);
    split.add_generation(&[
        sample("c", 0, 3.0),
        sample("f", 0, 1.0),
        sample("b", 0, 3.0),
    ]);
    split.add_generation(&[
        sample("a", 0, 5.0),
        sample("e", 0, 3.0),
        sample("d", 0, 3.0),
    ]);

    let mut answers: Vec<Vec<(String, u32)>> = Vec::new();
    for (name, fixture) in [
        ("ascending insertion", &ascending),
        ("reverse insertion", &reverse),
        ("two generations", &split),
    ] {
        let binary = assert_bit_identical_to_exact(fixture, 3, name);
        assert_eq!(
            keys_of(&binary),
            expected,
            "{name}: the answer must be the one of the tied boundary, whatever the order"
        );
        answers.push(bits_of(&binary));
    }
    assert_eq!(
        answers[0], answers[1],
        "the insertion order must not change the answer"
    );
    assert_eq!(
        answers[0], answers[2],
        "the segments of two generations must not change the answer"
    );
}

/// The six samples of [`the_answer_does_not_depend_on_the_insertion_order`]:
/// `a` at 5.0, four passages tied at 3.0, `f` at 1.0; inserted in the key order,
/// or in its reverse.
fn order_fixture_samples(reverse: bool) -> Vec<Sample> {
    let mut samples = vec![
        sample("a", 0, 5.0),
        sample("b", 0, 3.0),
        sample("c", 0, 3.0),
        sample("d", 0, 3.0),
        sample("e", 0, 3.0),
        sample("f", 0, 1.0),
    ];
    if reverse {
        samples.reverse();
    }
    samples
}

// --- The same boundary under regrouping (ADR-0075) ---

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

/// `group_by: "document"` at a tied boundary of documents: `k` counts
/// documents, and the tied group is cut inside it.
///
/// `g5` scores 5.0, the five other documents score 3.0 and tie; `k = 3`
/// documents keeps `g5` and the two smallest keys, `g0` and `g1`. The documents
/// are inserted as `g5, g4 … g0`, so an iid cut would keep `g4` and `g3`.
#[test]
fn group_by_document_breaks_the_document_boundary_ties_by_the_key() {
    let mut fixture = DenseFixture::create(DIMS);
    let mut samples = vec![sample("g5", 0, 5.0)];
    for index in (0..5).rev() {
        samples.push(sample(&format!("g{index}"), 0, 3.0));
    }
    fixture.add_generation(&samples);

    let grouping = by_document(1);
    let binary = assert_grouped_bit_identical_to_exact(&fixture, 3, &grouping, "grouped ties");
    assert_eq!(
        doc_ids_of(&binary),
        ["g5", "g0", "g1"],
        "the three documents of the answer, the tied group by ascending key"
    );
    assert_ne!(
        doc_ids_of(&binary),
        ["g5", "g4", "g3"],
        "an iid cut would keep the last inserted documents: the fixture must tell them apart"
    );
    assert_eq!(
        binary.len(),
        3,
        "one passage per document, so three entries"
    );
}

/// `group_by: "document"` consumes **more than `k` candidates**: the documents
/// after `k` in the global ranking are still needed to fill `k` documents.
///
/// The global ranking is `A#0` (5.0), `B#0` (4.8), `A#1` (4.5), `A#2` (4.0),
/// `C#0` (1.0). With `k = 3` documents and one passage per document, `C` is the
/// fifth candidate, so an implementation that reads the keys of the first `k`
/// candidates alone never sees it and answers two documents (ADR-0079,
/// decision 3: the keys of "les candidats que consomme le regroupement").
#[test]
fn group_by_document_reads_beyond_k_candidates_to_fill_k_documents() {
    let mut fixture = DenseFixture::create(DIMS);
    // Inserted in the reverse of the key order, so no cut by iid can pass for
    // the right answer.
    fixture.add_generation(&[
        sample("C", 0, 1.0),
        sample("A", 2, 4.0),
        sample("A", 1, 4.5),
        sample("B", 0, 4.8),
        sample("A", 0, 5.0),
    ]);

    let grouping = by_document(1);
    let binary = assert_grouped_bit_identical_to_exact(&fixture, 3, &grouping, "beyond k");
    assert_eq!(
        keys_of(&binary),
        ["A#0", "B#0", "C#0"],
        "three documents need the fifth candidate of the ranking"
    );
    assert_eq!(
        binary.len(),
        3,
        "the third document must not be lost when the keys are read late"
    );
}

/// `dedup: "content"` at a tied boundary: the two copies of the shared content
/// tie, and only the first of the ranking survives.
///
/// `s#0` scores 5.0, `t#0` and `u#0` score 3.0 and share a digest, `v#0` scores
/// 2.0. `k = 2` keeps `s#0` and `t#0`: the copy kept is the smallest key, so an
/// answer of `u#0` (the first by iid) is wrong.
#[test]
fn dedup_content_breaks_the_boundary_ties_by_the_key() {
    let mut fixture = DenseFixture::create(DIMS);
    fixture.add_generation(&[
        sample("v", 0, 2.0),
        sample("u", 0, 3.0).with_passage_hash(SHARED_DIGEST),
        sample("t", 0, 3.0).with_passage_hash(SHARED_DIGEST),
        sample("s", 0, 5.0),
    ]);

    let binary = assert_grouped_bit_identical_to_exact(&fixture, 2, &dedup(), "dedup ties");
    assert_eq!(
        keys_of(&binary),
        ["s#0", "t#0"],
        "the shared content is kept once, through its smallest key"
    );
    assert_ne!(
        keys_of(&binary),
        ["s#0", "u#0"],
        "an iid cut would keep the other copy: the fixture must tell them apart"
    );
}

/// `dedup: "content"` consumes more than `k` candidates: an implementation that
/// reads the keys of the `k` best alone has no copy to fuse, and returns fewer
/// results than asked.
///
/// `A#0` (5.0) and `B#0` (4.0) share a digest, `C#0` scores 3.0, `D#0` 2.0.
/// With `k = 3`, the deduplication removes `B#0` and `D#0` fills the third
/// place: three results, not two.
#[test]
fn dedup_content_reads_beyond_k_so_the_answer_holds_k_results() {
    let mut fixture = DenseFixture::create(DIMS);
    fixture.add_generation(&[
        sample("D", 0, 2.0),
        sample("C", 0, 3.0),
        sample("B", 0, 4.0).with_passage_hash(SHARED_DIGEST),
        sample("A", 0, 5.0).with_passage_hash(SHARED_DIGEST),
    ]);

    let binary = assert_grouped_bit_identical_to_exact(&fixture, 3, &dedup(), "dedup beyond k");
    assert_eq!(
        keys_of(&binary),
        ["A#0", "C#0", "D#0"],
        "the fused copy lets the fourth candidate in, so the answer still holds k results"
    );
    assert_eq!(
        binary.len(),
        3,
        "a deduplication that stops at k would return two results instead of three"
    );
}

/// `dedup` then `group_by` (ADR-0075, decision 1) on a fixture that ties at both
/// frontiers at once: the two copies of the top score, and two documents tied
/// for the second place.
///
/// `A#0` and `B#0` both score 5.0 and share a digest; `C#0` and `D#0` both score
/// 4.0; `E#0` scores 1.0. `k = 2` documents: the deduplication keeps `A#0` (the
/// smallest key), then the grouping keeps `A` and `C` (the smallest key of the
/// tie). An iid cut would keep `B#0` and `D#0`.
#[test]
fn dedup_then_group_by_matches_exact_on_a_tied_boundary() {
    let mut fixture = DenseFixture::create(DIMS);
    fixture.add_generation(&[
        sample("E", 0, 1.0),
        sample("D", 0, 4.0),
        sample("C", 0, 4.0),
        sample("B", 0, 5.0).with_passage_hash(SHARED_DIGEST),
        sample("A", 0, 5.0).with_passage_hash(SHARED_DIGEST),
    ]);

    let grouping = Grouping {
        by_document: true,
        passages_per_document: 1,
        dedup_content: true,
    };
    let binary = assert_grouped_bit_identical_to_exact(&fixture, 2, &grouping, "dedup then group");
    assert_eq!(
        keys_of(&binary),
        ["A#0", "C#0"],
        "dedup keeps the smallest key of the shared content, then the grouping the smallest \
         key of the tied documents"
    );
    assert_ne!(
        keys_of(&binary)[0],
        "B#0",
        "an iid cut would keep the other copy of the shared content, not the smallest key"
    );
}

// --- INV-06 on the rescoring path ---

/// INV-06: "pool ≤ seuil exact ⇒ résultat identique au calcul exact", which
/// ADR-0072, decision 3 extends to the rescoring whenever the pool is fully
/// rescored: the two strategies score through the same function and rank with
/// the same order.
///
/// Until this test, no `inv_06_*` test covered the `BINARY` path: the two of
/// `planifier_strategie.rs` are planned `EXACT`, and
/// `binary_pool_with_n_covering_the_pool_returns_the_exact_result` carries the
/// proof under another name. `REDUIRE-COUT-DES-CLES` moves exactly this path,
/// so the invariant needs its own test.
///
/// The pool is a real restriction — a filter keeps the seven passages of `t`
/// out of ten, and drops a document that would rank first — and it holds a tie
/// at the boundary of `k`. `N` is far above the pool, so the scan keeps the
/// whole pool and the rescoring scores every passage of it.
#[test]
fn inv_06_binary_rescoring_of_a_filtered_pool_returns_the_exact_result() {
    let mut fixture = DenseFixture::create(DIMS);
    let mut samples = vec![sample("t", 6, 5.0)];
    for ord in (0..6).rev() {
        samples.push(sample("t", ord, 3.0));
    }
    // 9.0 outranks everything, so a filter that let this document through would
    // show in the answer.
    for ord in 0..3 {
        samples.push(sample("other", ord, 9.0));
    }
    fixture.add_generation(&samples);

    let filter = Filter::from_json_str(r#"{"fonds":"t"}"#)
        .unwrap_or_else(|error| panic!("the filter of the test must parse: {error}"));
    let pool = fixture.pool(&filter);
    assert_eq!(
        pool.len(),
        7,
        "the filter must keep the seven passages of t and drop the three of other"
    );
    assert_eq!(
        plan(&binary_settings(), pool.len(), 4, false).strategy,
        Strategy::Binary,
        "the test must run the rescoring path, not the exact one"
    );

    let binary = run_with_filter(&fixture, &filter, 4, &binary_settings());
    let exact = run_with_filter(&fixture, &filter, 4, &exact_settings());
    assert_eq!(
        bits_of(&binary),
        bits_of(&exact),
        "INV-06: the rescoring of a fully rescored pool must return the exact keys, order and \
         score bits"
    );
    assert_eq!(
        keys_of(&binary),
        ["t#6", "t#0", "t#1", "t#2"],
        "the pool is the filtered one, and its tied boundary is cut by the key"
    );
    assert!(
        binary.iter().all(|hit| hit.doc_id == "t"),
        "a passage outside the pool must never be returned"
    );
}
