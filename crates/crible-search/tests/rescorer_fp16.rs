// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `RESCORER-FP16`: the exact score of the `N` candidates of the binary scan.
//!
//! References: `docs/phases/P4-recherche-rapide.md` (`RESCORER-FP16`, phase P4),
//! `docs/03-recherche.md` ("Branche dense", strategy BINARY), ADR-0072 (one
//! calculation of the dot product for the exact strategy and the rescoring) and
//! ADR-0070 (the `f16` kernel).
//!
//! # API under test
//!
//! ```text
//! crible_search::dense::exact::DenseSearcher::rescore(
//!     &self, query: &[f32], candidates: &[binary::Candidate])
//!     -> Result<Vec<Scored>, CribleError>;
//!
//! crible_search::dense::Scored {
//!     pub iid: Iid,
//!     pub key: String,   // `doc_id#ord`, the tie-break
//!     pub score: f32,    // the score of `cosine`, the shared calculation
//! }
//! ```
//!
//! The candidates come from `crible_search::dense::binary::top_k`, the scan of
//! `SCANNER-POOL-BINAIRE`.
//!
//! # What each test pins
//!
//! | Test | Criterion |
//! |---|---|
//! | `rescoring_the_whole_pool_matches_exact_bit_for_bit` | N ≥ pool, fp16: same keys, same order, bit-identical scores, on a full, a filtered, a single-passage and an empty pool |
//! | `rescoring_the_whole_pool_of_an_fp32_reference_matches_exact_bit_for_bit` | the same on an `fp32` reference (ADR-0072, decision 2) |
//! | `rescoring_scores_the_candidates_the_scan_kept` | N < pool: the rescoring returns the score `EXACT` gives each of those passages, best first |
//! | `rescoring_breaks_ties_by_the_ascending_textual_key` | "départage stable par clé `doc_id#ord`": `doc#10` sorts before `doc#2`, as text, and not by iid |
//! | `rescoring_rejects_a_malformed_query` | the query is validated before an empty candidate list short-circuits |
//! | `rescoring_an_empty_candidate_list_returns_no_result` | a pool the filter emptied |
//! | `rescoring_an_empty_generation_returns_no_result` | the generation of `crible index create`, with no segment |

mod common;

use crible_core::config::ReferencePrecision;
use crible_core::error::ErrorCode;
use crible_core::filter::Filter;
use crible_search::dense::Scored;
use crible_search::dense::binary;
use crible_search::dense::exact::Hit;
use crible_store::codes::quantize;
use serde_json::json;

use common::dense::{DenseFixture, Sample, match_all};

/// Dimensions of the fixture index.
const DIMS: u32 = 8;

/// Builds a filter from its JSON form, panicking with the JSON on a failure.
fn filter(raw: &str) -> Filter {
    Filter::from_json_str(raw)
        .unwrap_or_else(|error| panic!("the filter {raw} must parse: {error}"))
}

/// Query of the tests, with one component per dimension.
///
/// It is deliberately not normalised: the search normalises it, and both
/// strategies must do so through the same function (ADR-0072, decision 1).
fn query() -> Vec<f32> {
    vec![1.0, 2.0, 3.0, 0.5, -1.0, 0.25, -2.0, 0.75]
}

/// Keys and score **bits** of a result list, in order.
///
/// The bits, not the `f32`, so the comparison of the criterion is an equality
/// and not a tolerance that could hide a different arithmetic.
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

/// A vector of two axes at `angle`, normalised, so the fixture scores are
/// distinct and their order is strict.
fn axis(angle: f32) -> Vec<f32> {
    let mut vector = vec![0.0; DIMS as usize];
    vector[0] = angle.cos();
    vector[1] = angle.sin();
    vector
}

/// Twelve passages in two generations of six, one document per passage, with a
/// document field that separates the two generations.
///
/// The vectors differ by their angle, so the scores are distinct except where a
/// test builds a tie on purpose.
fn fixture(precision: ReferencePrecision) -> DenseFixture {
    let mut fixture = DenseFixture::create_with_precision(DIMS, precision);
    let first: Vec<Sample> = (0..6_u32)
        .map(|position| {
            Sample::new(
                &format!("a-{position}"),
                position,
                axis(0.1 * f32::from(u8::try_from(position).unwrap_or(0))),
            )
            .with_doc_fields(json!({ "fonds": "JORF" }))
        })
        .collect();
    fixture.add_generation(&first);
    let second: Vec<Sample> = (0..6_u32)
        .map(|position| {
            Sample::new(
                &format!("b-{position}"),
                position,
                axis(0.4 + 0.1 * f32::from(u8::try_from(position).unwrap_or(0))),
            )
            .with_doc_fields(json!({ "fonds": "LEGI" }))
        })
        .collect();
    fixture.add_generation(&second);
    fixture
}

/// Runs both strategies on `filter` and returns what each of them answers.
///
/// `n` is the number of candidates the scan keeps; pass the size of the pool to
/// exercise the criterion "N ≥ taille du pool".
fn both_strategies(
    fixture: &DenseFixture,
    query: &[f32],
    filter: &Filter,
    n: usize,
) -> (Vec<Hit>, Vec<Scored>, usize) {
    let pool = fixture.pool(filter);
    let snapshot = fixture.snapshot();
    let code = quantize(query);
    let candidates = binary::top_k(&snapshot, &pool, &code, n)
        .unwrap_or_else(|error| panic!("the binary scan must run: {error}"));
    let searcher = fixture.searcher();
    let pool_size = usize::try_from(pool.len()).unwrap_or(usize::MAX);
    let exact = searcher
        .top_k(query, &pool, pool_size)
        .unwrap_or_else(|error| panic!("the exact search must run: {error}"));
    let rescored = searcher
        .rescore(query, &candidates, pool_size)
        .unwrap_or_else(|error| panic!("the rescoring must run: {error}"));
    (exact, rescored, pool_size)
}

/// The four pools of the criterion: full, filtered, a single passage, empty.
fn pools() -> Vec<(&'static str, Filter)> {
    vec![
        ("full", match_all()),
        ("filtered", filter(r#"{"fonds":"JORF"}"#)),
        ("single passage", filter(r#"{"fonds":"LEGI","rang":5}"#)),
        ("empty", filter(r#"{"fonds":"absent"}"#)),
    ]
}

#[test]
fn rescoring_the_whole_pool_matches_exact_bit_for_bit() {
    let fixture = fixture(ReferencePrecision::Fp16);
    let query = query();

    for (name, filter) in pools() {
        let pool = fixture.pool(&filter);
        let pool_size = usize::try_from(pool.len()).unwrap_or(usize::MAX);
        let (exact, rescored, seen) = both_strategies(&fixture, &query, &filter, pool_size);
        assert_eq!(
            seen, pool_size,
            "{name} pool: the exact search must cover the pool"
        );
        assert_eq!(
            rescored.len(),
            pool_size,
            "{name} pool: N ≥ pool means the rescoring keeps the whole pool"
        );
        assert_eq!(
            bits_of_scored(&rescored),
            bits_of_hits(&exact),
            "{name} pool: the rescoring must return the keys and the score bits of EXACT"
        );
    }
}

#[test]
fn rescoring_the_whole_pool_of_an_fp32_reference_matches_exact_bit_for_bit() {
    let fixture = fixture(ReferencePrecision::Fp32);
    let query = query();

    for (name, filter) in pools() {
        let pool = fixture.pool(&filter);
        let pool_size = usize::try_from(pool.len()).unwrap_or(usize::MAX);
        let (exact, rescored, _) = both_strategies(&fixture, &query, &filter, pool_size);
        assert_eq!(
            rescored.len(),
            pool_size,
            "{name} pool: the rescoring keeps the whole pool"
        );
        assert_eq!(
            bits_of_scored(&rescored),
            bits_of_hits(&exact),
            "{name} pool: the fp32 calculation of ADR-0072 is shared by both strategies"
        );
    }
}

#[test]
fn rescoring_scores_the_candidates_the_scan_kept() {
    let fixture = fixture(ReferencePrecision::Fp16);
    let query = query();
    let all = match_all();

    // The score of every passage, as the exact strategy computes it.
    let pool = fixture.pool(&all);
    let pool_size = usize::try_from(pool.len()).unwrap_or(usize::MAX);
    let (exact, _, _) = both_strategies(&fixture, &query, &all, pool_size);
    let scores: std::collections::HashMap<String, u32> = exact
        .iter()
        .map(|hit| (hit.key.clone(), hit.score.to_bits()))
        .collect();

    // The three candidates the scan kept, scored by the rescoring.
    let (_, rescored, _) = both_strategies(&fixture, &query, &all, 3);
    assert_eq!(rescored.len(), 3, "N < pool keeps N candidates");
    for candidate in &rescored {
        let expected = scores.get(&candidate.key).unwrap_or_else(|| {
            panic!(
                "the rescoring returned {} which EXACT does not rank",
                candidate.key
            )
        });
        assert_eq!(
            candidate.score.to_bits(),
            *expected,
            "{}: the rescoring must give a candidate the score EXACT computes for it",
            candidate.key
        );
    }
    for pair in rescored.windows(2) {
        assert!(
            pair[0].score >= pair[1].score,
            "the rescoring must return the candidates best first: {} then {}",
            pair[0].key,
            pair[1].key
        );
    }
}

#[test]
fn rescoring_breaks_ties_by_the_ascending_textual_key() {
    // One document, two passages of the same vector: their scores are equal. The
    // fixture hands out iids in the sample order, `doc#2` first, so an iid
    // tie-break would return `doc#2` first; the textual key order puts `doc#10`
    // first, and it is the order of the reference (`tools/reference_topk.py`)
    // and of the exact strategy.
    let mut fixture = DenseFixture::create(DIMS);
    let vector = axis(0.0);
    fixture.add_generation(&[
        Sample::new("doc", 2, vector.clone()),
        Sample::new("doc", 10, vector),
    ]);

    let query = query();
    let all = match_all();
    let pool = fixture.pool(&all);
    let pool_size = usize::try_from(pool.len()).unwrap_or(usize::MAX);
    let (exact, rescored, _) = both_strategies(&fixture, &query, &all, pool_size);
    let keys: Vec<&str> = rescored
        .iter()
        .map(|candidate| candidate.key.as_str())
        .collect();
    assert_eq!(
        keys,
        ["doc#10", "doc#2"],
        "an equal score must be broken by the ascending textual key"
    );
    assert_eq!(
        bits_of_scored(&rescored),
        bits_of_hits(&exact),
        "the tie-break of the rescoring must be the one of EXACT"
    );
    assert_eq!(
        rescored[0].score.to_bits(),
        rescored[1].score.to_bits(),
        "the test must exercise an exact tie, not two close scores"
    );
}

#[test]
fn rescoring_rejects_a_malformed_query() {
    let fixture = fixture(ReferencePrecision::Fp16);
    let searcher = fixture.searcher();
    let all = match_all();
    let pool = fixture.pool(&all);
    let snapshot = fixture.snapshot();
    let candidates = binary::top_k(&snapshot, &pool, &quantize(&query()), 3)
        .unwrap_or_else(|error| panic!("the binary scan must run: {error}"));
    let empty: Vec<binary::Candidate> = Vec::new();

    let wrong_dims = vec![1.0_f32; DIMS as usize - 1];
    let zero = vec![0.0_f32; DIMS as usize];
    let mut not_finite = query();
    not_finite[3] = f32::NAN;

    for (name, malformed) in [
        ("wrong dimensionality", wrong_dims),
        ("zero norm", zero),
        ("non-finite component", not_finite),
    ] {
        // With candidates and without: the request is validated first, so an
        // empty candidate list cannot turn a malformed query into an empty
        // answer.
        for (case, list) in [("candidates", &candidates), ("no candidate", &empty)] {
            let error = searcher
                .rescore(&malformed, list, list.len())
                .expect_err("a malformed query must be refused");
            assert_eq!(
                error.code(),
                ErrorCode::InvalidRequest,
                "{name}, {case}: wrong error code: {}",
                error.message()
            );
        }
    }
}

#[test]
fn rescoring_an_empty_candidate_list_returns_no_result() {
    let fixture = fixture(ReferencePrecision::Fp16);
    let empty = filter(r#"{"fonds":"absent"}"#);
    let (exact, rescored, pool_size) = both_strategies(&fixture, &query(), &empty, 10);
    assert_eq!(pool_size, 0, "the test filter must select no passage");
    assert!(
        exact.is_empty() && rescored.is_empty(),
        "an empty pool gives no result from either strategy, and no error"
    );
}

#[test]
fn rescoring_an_empty_generation_returns_no_result() {
    let fixture = DenseFixture::create(DIMS);
    let searcher = fixture.searcher();
    let rescored = searcher
        .rescore(&query(), &[], 10)
        .unwrap_or_else(|error| panic!("an empty generation must accept any query: {error}"));
    assert!(
        rescored.is_empty(),
        "a generation without a segment holds no passage to score"
    );
}
