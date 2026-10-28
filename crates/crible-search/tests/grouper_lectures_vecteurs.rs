// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `GROUPER-LECTURES-VECTEURS` — the safety net of the batched, parallel read of
//! the `f16` rows of the rescoring.
//!
//! References: `docs/phases/P4-recherche-rapide.md` (`GROUPER-LECTURES-VECTEURS`,
//! phase P4), ADR-0080 (decisions 2, 3 and 6), ADR-0072 (decision 3: keys, score
//! bits and order identical to `EXACT`), ADR-0083 (only the key read after the
//! sort was delivered) and INV-06.
//!
//! # What the change must not move
//!
//! ADR-0080, decision 2 replaces the per-candidate read of `SegmentReader` by a
//! batched, parallel read of the `N` rows into a transient buffer, **before** the
//! calculation, which then keeps the same `kernels::dot` per vector and the same
//! final sort. The answer must therefore not change at all: same keys, same
//! order, same score **bits** (ADR-0072, decision 3). These tests pin that
//! promise on the current code, so they pass before the change and must still
//! pass after it.
//!
//! # Why a fixture of unequal segments
//!
//! A batched read walks the candidates segment by segment, in chunks. A
//! single-segment fixture cannot show a chunk that straddles a boundary, nor a
//! buffer sized on the whole candidate list while the rows come from five
//! files. The fixture here has five segments of **1, 7, 64, 65 and 1 000**
//! passages: the pools of those sizes are the plausible chunk boundaries of a
//! batched read (one row, a small chunk, a power of two, one row past it, a
//! large pool), and the whole live set walks every boundary of the generation.
//!
//! # What each test pins
//!
//! | Test | Criterion |
//! |---|---|
//! | `rescoring_the_whole_live_set_across_unequal_segments_matches_exact_bit_for_bit` | N ≥ pool over five unequal segments, `k` below and above the pool |
//! | `rescoring_matches_exact_for_pools_of_one_to_a_thousand_candidates` | pools of 1, 7, 64, 65 and 1 000 candidates, N = pool |
//! | `the_result_does_not_depend_on_the_scan_pool_worker_count` | 1, 2 and 8 rayon workers give the same bits, and the bits of `EXACT` |
//! | `the_result_does_not_depend_on_the_candidate_arrival_order` | shuffled candidate lists give the same bits as the scan order |
//! | `inv_06_multi_segment_pool_fully_rescored_returns_the_exact_result` | INV-06 on a filtered pool spread over several segments |
//!
//! # What these tests do not duplicate
//!
//! - the empty pool and the empty generation are covered by
//!   `rescoring_an_empty_candidate_list_returns_no_result` and
//!   `rescoring_an_empty_generation_returns_no_result`
//!   (`crates/crible-search/tests/rescorer_fp16.rs`), on a one-segment fixture;
//! - the `fp32` reference, the malformed query, the tie-break by key and the
//!   single-passage pool are covered by the other tests of `rescorer_fp16.rs`;
//! - the tied boundary of `k` and the `group_by` / `dedup` cases belong to
//!   `crates/crible-search/tests/reduire_cout_des_cles.rs`.
//!
//! The tests below add what none of them has: **several segments of unequal
//! sizes**, the **worker count** of the rayon pool, and the **arrival order** of
//! the candidates.

mod common;

use std::path::Path;
use std::sync::Arc;

use crible_core::config::{FusionConfig, ScanStrategy, SearchConfig};
use crible_core::filter::Filter;
use crible_search::dense::binary::{Candidate, top_k as scan_top_k};
use crible_search::dense::exact::{DenseSearcher, Hit};
use crible_search::dense::{Scored, Strategy, plan, search};
use crible_search::pool::Pool;
use crible_search::scan_pool::ScanPool;
use crible_store::codes::quantize;
use crible_store::generation::Snapshot;

use common::dense::{DenseFixture, Sample, match_all};

/// Dimensions of the fixture: one word of code, two rows of `f16` per page is
/// irrelevant here, the point is the number of candidates, not the bytes.
const DIMS: u32 = 64;

/// Sizes of the segments of the fixture, in identifier order.
///
/// They are deliberately unequal and each of them, except the whole live set, is
/// a pool of the criterion: one candidate, seven, sixty-four, one past
/// sixty-four, and a thousand. A batched read of `N` rows meets every plausible
/// chunk size on one of them.
const SEGMENT_SIZES: [u32; 5] = [1, 7, 64, 65, 1_000];

/// Number of results the tests ask for when `k` is not the pool size.
const K: usize = 10;

/// `BINARY` whatever the pool: the threshold is `0`, so only an empty pool would
/// be planned exact. `N` is above every fixture pool, so the scan keeps the whole
/// pool and the rescoring scores every passage of it: the answer then depends on
/// the scoring and the sort alone, which is what the batched read must not move.
fn binary_settings() -> SearchConfig {
    SearchConfig {
        scan: ScanStrategy::Binary,
        rescore_candidates: 2_000,
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
        rescore_candidates: 2_000,
        exact_threshold: 2_000,
        candidates_per_branch: 100,
        fusion: FusionConfig::Rrf,
    }
}

/// Query of the tests, distinct from every stored vector, so the scores are
/// spread and their order is readable.
fn query() -> Vec<f32> {
    (0..DIMS)
        .map(|component| ((component * 11) % 9) as f32 - 4.0)
        .collect()
}

/// Deterministic vector of the fixture, one per passage.
fn vector(seed: u32) -> Vec<f32> {
    (0..DIMS)
        .map(|component| {
            let value = (u64::from(seed) * 31 + u64::from(component) * 17) % 7;
            value as f32 - 3.0
        })
        .collect()
}

/// Fixture whose segments hold [`SEGMENT_SIZES`] passages, hence the identifier
/// ranges `0..1`, `1..8`, `8..72`, `72..137` and `137..1137`.
///
/// Every passage of a segment belongs to the document `S<index>`, so a `fonds`
/// condition keeps a whole segment or none of it: the pools of the tests are
/// exactly the segment sizes above.
fn fixture() -> DenseFixture {
    let mut fixture = DenseFixture::create(DIMS);
    let mut seed = 0_u32;
    for (index, size) in SEGMENT_SIZES.iter().enumerate() {
        let doc_id = format!("S{index}");
        let samples: Vec<Sample> = (0..*size)
            .map(|ord| {
                let sample = Sample::new(&doc_id, ord, vector(seed));
                seed += 1;
                sample
            })
            .collect();
        fixture.add_generation(&samples);
    }
    fixture
}

/// Checks that the fixture has the segment layout its tests claim.
fn assert_segment_shape(snapshot: &Snapshot) {
    let segments = snapshot.segments();
    assert_eq!(
        segments.len(),
        SEGMENT_SIZES.len(),
        "the fixture must have one segment per declared size"
    );
    let mut start = 0_u32;
    for (index, (segment, size)) in segments.iter().zip(SEGMENT_SIZES).enumerate() {
        assert_eq!(
            segment.iid_start.get(),
            start,
            "segment {index} must start at identifier {start}"
        );
        assert_eq!(
            segment.count, size,
            "segment {index} must hold {size} passages"
        );
        start += size;
    }
}

/// Filter of a JSON expression.
fn filter(raw: &str) -> Filter {
    Filter::from_json_str(raw)
        .unwrap_or_else(|error| panic!("the filter {raw} must parse: {error}"))
}

/// Keys and score **bits** of a result list, in order.
///
/// The bits, not the `f32`, so "identical to `EXACT`" is an equality and not a
/// tolerance that could hide a different arithmetic (ADR-0072, decision 3).
fn bits_of_hits(hits: &[Hit]) -> Vec<(String, u32)> {
    hits.iter()
        .map(|hit| (hit.key.clone(), hit.score.to_bits()))
        .collect()
}

/// Keys and score **bits** of a rescored list, in order.
fn bits_of_scored(scored: &[Scored]) -> Vec<(String, u32)> {
    scored
        .iter()
        .map(|candidate| (candidate.key.clone(), candidate.score.to_bits()))
        .collect()
}

/// Runs the dense branch of `k` results on `pool`, on a dedicated pool of
/// `workers` rayon workers.
///
/// The searcher is opened **inside** [`ScanPool::install`], as the server does
/// (`crates/crible-search/src/scan_pool.rs`): its read pool is then sized on the
/// worker count, which is the state the batched read will meet.
fn run_dense(
    segments_dir: &Path,
    records: &Path,
    snapshot: &Arc<Snapshot>,
    pool: &Pool,
    k: usize,
    settings: &SearchConfig,
    workers: usize,
) -> Vec<Hit> {
    let segments_dir = segments_dir.to_path_buf();
    let records = records.to_path_buf();
    let snapshot = Arc::clone(snapshot);
    let pool = pool.clone();
    let settings = settings.clone();
    ScanPool::new(workers)
        .unwrap_or_else(|error| panic!("a {workers}-worker scan pool must be buildable: {error}"))
        .install(move || {
            let searcher = DenseSearcher::open(&segments_dir, snapshot.segments(), &records)
                .unwrap_or_else(|error| panic!("the searcher must open: {error}"));
            search(&snapshot, &searcher, &pool, &query(), k, &settings, false)
                .unwrap_or_else(|error| panic!("the dense search must succeed: {error}"))
        })
}

/// Asserts that `BINARY` and `EXACT` answer the same keys, in the same order,
/// with the same score bits, and returns the `BINARY` answer.
fn assert_bit_identical_to_exact(
    fixture: &DenseFixture,
    pool: &Pool,
    k: usize,
    workers: usize,
    label: &str,
) -> Vec<Hit> {
    assert_eq!(
        plan(&binary_settings(), pool.len(), k, false).strategy,
        Strategy::Binary,
        "{label}: the test must run the rescoring path, not the exact one"
    );
    let binary = run_dense(
        &fixture.segments_dir(),
        &fixture.records_path(),
        &fixture.snapshot(),
        pool,
        k,
        &binary_settings(),
        workers,
    );
    let exact = run_dense(
        &fixture.segments_dir(),
        &fixture.records_path(),
        &fixture.snapshot(),
        pool,
        k,
        &exact_settings(),
        workers,
    );
    assert_eq!(
        bits_of_hits(&binary),
        bits_of_hits(&exact),
        "{label}: BINARY must return the keys, the order and the score bits of EXACT"
    );
    binary
}

/// Same candidate list, in a deterministic other order, drawn from `seed`.
///
/// The generator is a fixed linear congruential sequence, so the permutation is
/// reproducible and the test depends neither on the clock nor on a hash seed. It
/// stands for the order in which a batched, parallel read finishes its chunks.
fn shuffled(candidates: &[Candidate], seed: u64) -> Vec<Candidate> {
    let mut positions: Vec<usize> = (0..candidates.len()).collect();
    let mut state = seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    for index in (1..positions.len()).rev() {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let pick = usize::try_from(state >> 33).unwrap_or(0) % (index + 1);
        positions.swap(index, pick);
    }
    positions
        .into_iter()
        .map(|position| candidates[position])
        .collect()
}

// --- Criterion 1: N ≥ pool over unequal segments ---

/// The whole live set, spread over five segments of 1 to 1 000 passages, is
/// rescored to the answer of `EXACT`, bit for bit, when `N` covers the pool.
///
/// Two `k` are exercised: `10`, the shape of a real request, where the answer
/// keeps a prefix of the pool and the rescoring reads the keys of the frontier
/// only; and the pool size, where every passage is returned and every row of
/// every segment is read.
#[test]
fn rescoring_the_whole_live_set_across_unequal_segments_matches_exact_bit_for_bit() {
    let fixture = fixture();
    let snapshot = fixture.snapshot();
    assert_segment_shape(&snapshot);
    let pool = fixture.pool(&match_all());
    let pool_size = usize::try_from(pool.len()).unwrap_or(usize::MAX);
    let expected: usize = SEGMENT_SIZES.iter().map(|size| *size as usize).sum();
    assert_eq!(
        pool_size, expected,
        "the unfiltered pool must be the whole live set"
    );

    let best = assert_bit_identical_to_exact(&fixture, &pool, K, 1, "the whole live set, k = 10");
    assert_eq!(best.len(), K, "k = 10 keeps ten results of a 1 137 pool");

    let all = assert_bit_identical_to_exact(
        &fixture,
        &pool,
        pool_size,
        1,
        "the whole live set, k = pool",
    );
    assert_eq!(
        all.len(),
        pool_size,
        "N ≥ pool means the rescoring keeps the whole pool"
    );
}

/// Pools of 1, 7, 64, 65 and 1 000 candidates, each inside one segment, are
/// rescored to the answer of `EXACT`, bit for bit.
///
/// Each size is a plausible chunk of a batched read: one row, a small chunk, a
/// power of two, one row past it, and a large pool that spans several chunks.
/// `N` equals the pool exactly, which is the condition of the criterion, and `k`
/// is above the small pools so that every candidate is returned.
#[test]
fn rescoring_matches_exact_for_pools_of_one_to_a_thousand_candidates() {
    let fixture = fixture();
    let snapshot = fixture.snapshot();
    assert_segment_shape(&snapshot);

    for (index, size) in SEGMENT_SIZES.iter().enumerate() {
        let raw = format!("{{\"fonds\": \"S{index}\"}}");
        let pool = fixture.pool(&filter(&raw));
        assert_eq!(
            pool.len(),
            *size,
            "the filter of {raw} must keep the {size} passages of its segment"
        );
        let answered = assert_bit_identical_to_exact(
            &fixture,
            &pool,
            K.max(*size as usize),
            1,
            &format!("the pool of {size} candidates of S{index}"),
        );
        assert_eq!(
            answered.len(),
            *size as usize,
            "a pool of {size} candidates must come back whole"
        );
    }
}

// --- Criterion 2: worker count and arrival order ---

/// The answer does not depend on the number of rayon workers, and equals the one
/// of `EXACT`.
///
/// A batched, parallel read splits the candidates between the workers of the
/// pool installed by the caller; the chunk boundaries move with the worker count.
/// The answer must not, because the ranking is built after every score
/// (ADR-0080, decision 2). One worker is the reference, and the answer of
/// `EXACT` is the oracle at each count.
#[test]
fn the_result_does_not_depend_on_the_scan_pool_worker_count() {
    let fixture = fixture();
    let snapshot = fixture.snapshot();
    assert_segment_shape(&snapshot);
    let pools = [
        ("the whole live set", match_all(), 1_137_usize),
        ("a pool of 64 candidates", filter("{\"fonds\": \"S2\"}"), 64),
        ("a pool of 7 candidates", filter("{\"fonds\": \"S1\"}"), 7),
    ];

    for (name, filter, expected) in pools {
        let pool = fixture.pool(&filter);
        assert_eq!(
            usize::try_from(pool.len()).unwrap_or(usize::MAX),
            expected,
            "{name}: the fixture must hold {expected} live passages in it"
        );

        let reference = assert_bit_identical_to_exact(&fixture, &pool, K, 1, name);
        for workers in [2_usize, 8] {
            let answer = run_dense(
                &fixture.segments_dir(),
                &fixture.records_path(),
                &snapshot,
                &pool,
                K,
                &binary_settings(),
                workers,
            );
            assert_eq!(
                bits_of_hits(&answer),
                bits_of_hits(&reference),
                "{name}: {workers} workers must give the bits of one worker, and of EXACT"
            );
        }
    }
}

/// The answer does not depend on the order in which the candidates reach the
/// rescoring.
///
/// This test calls `DenseSearcher::rescore` directly, because the arrival order
/// is an internal input of the batched read that no pipeline entry point exposes:
/// the scan builds its list in Hamming order, and a batched, parallel read hands
/// its rows back in the order its chunks finished. Four fixed permutations stand
/// for those orders; each must produce the same keys, order and score bits as the
/// scan order, and as `EXACT`.
#[test]
fn the_result_does_not_depend_on_the_candidate_arrival_order() {
    let fixture = fixture();
    let snapshot = fixture.snapshot();
    assert_segment_shape(&snapshot);
    let pool = fixture.pool(&match_all());
    let pool_size = usize::try_from(pool.len()).unwrap_or(usize::MAX);

    let searcher = fixture.searcher();
    let query = query();
    let code = quantize(&query);
    let candidates = scan_top_k(&snapshot, &pool, &code, pool_size)
        .unwrap_or_else(|error| panic!("the binary scan must run: {error}"));
    assert_eq!(
        candidates.len(),
        pool_size,
        "the scan must keep the whole pool, so every segment contributes candidates"
    );

    let from_scan = searcher
        .rescore(&query, &candidates, pool_size)
        .unwrap_or_else(|error| panic!("the rescoring must run: {error}"));
    let exact = searcher
        .top_k(&query, &pool, pool_size)
        .unwrap_or_else(|error| panic!("the exact search must run: {error}"));
    assert_eq!(
        bits_of_scored(&from_scan),
        bits_of_hits(&exact),
        "the rescoring in scan order must be the answer of EXACT"
    );

    let scan_order: Vec<u32> = candidates
        .iter()
        .map(|candidate| candidate.iid.get())
        .collect();
    for seed in [1_u64, 7, 64, 65, 1_000] {
        let permuted = shuffled(&candidates, seed);
        let permuted_order: Vec<u32> = permuted
            .iter()
            .map(|candidate| candidate.iid.get())
            .collect();
        assert_ne!(
            permuted_order, scan_order,
            "seed {seed}: the permutation must really move the candidates, or the test is vacuous"
        );
        let answer = searcher
            .rescore(&query, &permuted, pool_size)
            .unwrap_or_else(|error| panic!("the rescoring of seed {seed} must run: {error}"));
        assert_eq!(
            bits_of_scored(&answer),
            bits_of_scored(&from_scan),
            "seed {seed}: the arrival order must not move a key, a score bit or a rank"
        );
    }
}

// --- Criterion 3 and INV-06: a filtered pool across several segments ---

/// INV-06: "pool ≤ seuil exact ⇒ résultat identique au calcul exact", extended by
/// ADR-0072, decision 3 to a fully rescored pool.
///
/// The seven `inv_06_*` tests of the crate do not cover this state: the two of
/// `planifier_strategie.rs` are planned `EXACT`, `dense_exact.rs` and
/// `inv_06_dense_exact_synth.rs` exercise the exact strategy, and
/// `inv_06_binary_rescoring_of_a_filtered_pool_returns_the_exact_result`
/// (`crates/crible-search/tests/reduire_cout_des_cles.rs:694`) rescans a filtered
/// pool that lives in **one** segment. A batched read that walks the candidates
/// segment by segment is exercised by none of them: this test is that gap.
///
/// The pool skips two whole segments — one of them holds a thousand passages — and
/// spans three files. `N` is above it, so every candidate is rescored.
#[test]
fn inv_06_multi_segment_pool_fully_rescored_returns_the_exact_result() {
    let fixture = fixture();
    let snapshot = fixture.snapshot();
    assert_segment_shape(&snapshot);

    let filter = filter("{\"fonds\": {\"in\": [\"S0\", \"S1\", \"S3\", \"S4\"]}}");
    let pool = fixture.pool(&filter);
    let pool_size = usize::try_from(pool.len()).unwrap_or(usize::MAX);
    assert_eq!(
        pool_size,
        1 + 7 + 65 + 1_000,
        "the pool must skip the segment of 64 passages and walk the other four"
    );

    let binary = assert_bit_identical_to_exact(&fixture, &pool, K, 1, "a pool over four segments");
    assert_eq!(binary.len(), K, "the answer keeps the ten best of the pool");
    assert!(
        binary.iter().all(|hit| hit.doc_id != "S2"),
        "a passage outside the pool must never be returned"
    );

    let whole =
        assert_bit_identical_to_exact(&fixture, &pool, pool_size, 1, "the same pool, k = pool");
    assert_eq!(
        whole.len(),
        pool_size,
        "N ≥ pool means every passage of the filtered pool is returned"
    );
}
