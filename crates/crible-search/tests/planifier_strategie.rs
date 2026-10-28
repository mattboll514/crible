// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `PLANIFIER-STRATEGIE` — choosing `EXACT` or `BINARY`, and forcing the exact
//! search on request.
//!
//! References: `docs/phases/P4-recherche-rapide.md` (`PLANIFIER-STRATEGIE`,
//! phase P4), `docs/03-recherche.md` ("Branche dense", strategy table and
//! "Réglages de précision"), `docs/04-donnees-et-ingestion.md` ("Ce qui est
//! figé, ce qui est réglable"), `docs/05-api.md` ("plan": `dense_strategy`),
//! `docs/08-invariants-et-tests.md` (INV-06) and ADR-0072 (one calculation of
//! the dot product, so a binary plan whose `N` covers the pool returns the
//! exact result bit for bit).
//!
//! # API under test
//!
//! The task sheet asks for a strategy selection, an `exact` option, the chosen
//! strategy exposed in the plan, and the library-level `PATCH`. The crate has
//! the two strategies ([`crible_search::dense::exact::DenseSearcher::top_k`] and
//! [`crible_search::dense::binary::top_k`] followed by
//! [`crible_search::dense::exact::DenseSearcher::rescore`]), and
//! `dense::planner` chooses between them from the settings, the pool and the
//! request option (ADR-0074). The names below follow the vocabulary of
//! `docs/03-recherche.md` (EXACT, BINARY) and of the `plan` object of
//! `docs/05-api.md` (`dense_strategy`):
//!
//! ```text
//! #[derive(Debug, Clone, Copy, PartialEq, Eq)]
//! crible_search::dense::Strategy                      // the chosen strategy
//!     Exact | Binary
//! impl Strategy {
//!     /// `"exact"` or `"binary"`: the spelling of `plan.dense_strategy`
//!     /// (`docs/05-api.md`).
//!     pub fn as_str(self) -> &'static str;
//! }
//!
//! /// The dense part of the search plan the debug output exposes.
//! #[derive(Debug, Clone, Copy, PartialEq, Eq)]
//! crible_search::dense::Plan {
//!     pub strategy: Strategy,
//!     pub rescore_candidates: u32,   // N, the two fields of the plan of docs/05
//! }
//!
//! /// Chooses the strategy of one query.
//! ///
//! /// EXACT when the pool holds at most `search.exact_threshold` passages,
//! /// when `search.scan` is `ScanStrategy::None`, or when `exact` is true;
//! /// BINARY otherwise (`docs/03-recherche.md`, strategy table). Under BINARY,
//! /// the plan carries `max(N, k)` (ADR-0074, decision 4).
//! crible_search::dense::plan(
//!     search: &crible_core::config::SearchConfig,
//!     pool_len: u32,
//!     k: usize,
//!     exact: bool,
//! ) -> Plan;
//!
//! /// Runs the dense branch of one query with the planned strategy.
//! crible_search::dense::search(
//!     snapshot: &crible_store::generation::Snapshot,
//!     searcher: &crible_search::dense::exact::DenseSearcher,
//!     pool: &crible_search::pool::Pool,
//!     query: &[f32],
//!     k: usize,
//!     search: &crible_core::config::SearchConfig,
//!     exact: bool,
//! ) -> Result<Vec<crible_search::dense::exact::Hit>, CribleError>;
//! ```
//!
//! `dense::search` runs EXACT on a plan that says `Exact`, and the scan of
//! `binary::top_k` (`N = search.rescore_candidates`) followed by
//! `DenseSearcher::rescore` on a plan that says `Binary`, then keeps the `k`
//! best. The query code the scan needs is quantized inside, from the query the
//! searcher normalises, so a caller passes the raw vector only.
//!
//! `k` is the number of results the *branch* returns, which is
//! `candidates_per_branch` in the pipeline of `docs/03-recherche.md` (sur-sampled
//! when grouping, `REGROUPER-RESULTATS`), not the number of results the API
//! returns. A `k` above `rescore_candidates` is covered by ADR-0074, decision 4:
//! the scan keeps `max(N, k)` candidates and all of them are rescored, which the
//! last test of this file pins (`k` above `N` still returns `k` results, and the
//! plan says `max(N, k)`).
//!
//! The setting is [`crible_core::config::ScanStrategy`]: `Binary` or `None`
//! ("toujours exact"). The plan's [`Strategy`] is not that setting: `None` is
//! not a strategy, it is the setting that removes the choice.
//!
//! # What each test pins
//!
//! | Test | Criterion |
//! |---|---|
//! | `inv_06_pool_below_the_exact_threshold_is_planned_exact` | INV-06: pool ≤ threshold ⇒ EXACT |
//! | `a_pool_equal_to_the_exact_threshold_is_planned_exact` | the bound of `docs/03` is `≤`, not `<` |
//! | `a_pool_above_the_exact_threshold_is_planned_binary` | above the threshold, BINARY |
//! | `scan_none_plans_exact_whatever_the_pool` | `search.scan = none` = "toujours exact" |
//! | `exact_requested_plans_exact_above_the_threshold` | "`exact: true` ⇒ EXACT quelle que soit la taille" |
//! | `the_plan_names_the_strategy_as_docs_05_does` | the plan is readable by the debug output (P7) |
//! | `the_plan_carries_the_rescore_candidates_of_the_configuration` | N is in the plan |
//! | `the_selection_is_a_total_function_of_the_setting_and_the_pool` | proptest: every pool length, both settings, both values of `exact` |
//! | `inv_06_pool_below_the_threshold_returns_the_exact_result` | INV-06 on a real index: same keys, same order, bit-identical scores |
//! | `inv_06_exact_requested_above_the_threshold_returns_the_exact_result` | `exact: true` gives the exact result whatever the pool |
//! | `an_empty_pool_is_planned_exact_and_returns_no_result` | a filter that empties the pool is not an error |
//! | `a_zero_k_short_circuits_the_binary_plan` | `k == 0` returns no result under BINARY, without scanning, and still validates the query |
//! | `binary_pool_with_n_covering_the_pool_returns_the_exact_result` | the BINARY path is really scan → rescore (ADR-0072, N ≥ pool) |
//! | `binary_pool_with_n_below_the_pool_returns_the_best_rescored_candidates` | the BINARY path rescues the `N` candidates of the scan |
//! | `patching_the_exact_threshold_moves_the_planned_strategy` | the library `PATCH` changes the selection |
//! | `patching_the_scan_to_none_plans_exact_whatever_the_pool` | the library `PATCH` of `search.scan` changes it |
//! | `a_binary_plan_keeps_max_of_n_and_k_candidates_and_returns_k_results` | ADR-0074, decision 4: `k` above `N` |

mod common;

use crible_core::config::{
    FusionConfig, IndexConfig, IndexPatch, ScanStrategy, SearchConfig, apply_patch,
};
use crible_core::error::ErrorCode;
use crible_core::filter::Filter;
use crible_search::dense::binary;
use crible_search::dense::exact::Hit;
use crible_search::dense::{Plan, Strategy, plan, search};
use crible_store::codes::quantize;
use proptest::prelude::any;
use proptest::prop_assert_eq;
use proptest::test_runner::{Config as ProptestConfig, RngSeed, TestRunner};

use common::dense::{DenseFixture, Sample, match_all};

/// Dimensions of the fixture: three words per code, like a 384-dimension model.
const DIMS: u32 = 130;

/// Number of passages of the fixture.
const SAMPLES: u32 = 50;

/// Seed of the property run: fixed, so a failure reproduces.
const PROPTEST_SEED: u64 = 0x504C_414E_4946_4945;

/// Number of cases of the property run.
const CASES: u32 = 256;

// --- Settings ---

/// The dense settings of `docs/04` with the values the test chooses.
fn settings(scan: ScanStrategy, rescore_candidates: u32, exact_threshold: u32) -> SearchConfig {
    SearchConfig {
        scan,
        rescore_candidates,
        exact_threshold,
        candidates_per_branch: 10,
        fusion: FusionConfig::Rrf,
    }
}

// --- Fixture ---

/// Deterministic vector whose signs vary with the passage index.
fn vector(seed: u32) -> Vec<f32> {
    (0..DIMS)
        .map(|component| {
            let value = (u64::from(seed) * 31 + u64::from(component) * 17) % 7;
            value as f32 - 3.0
        })
        .collect()
}

/// Fixture of [`SAMPLES`] passages, one per document `D0` … `D49`, in one
/// generation, so a pool can be the whole index or a filtered part of it.
fn fixture() -> DenseFixture {
    let mut fixture = DenseFixture::create(DIMS);
    let samples: Vec<Sample> = (0..SAMPLES)
        .map(|index| Sample::new(&format!("D{index}"), 0, vector(index)))
        .collect();
    fixture.add_generation(&samples);
    fixture
}

/// Query of the tests, with one component per dimension.
fn query() -> Vec<f32> {
    (0..DIMS)
        .map(|component| ((component * 13) % 5) as f32 - 2.0)
        .collect()
}

/// Filter of a JSON expression, validated against the fixture schema.
fn filter(json: &str) -> Filter {
    Filter::from_json_str(json)
        .unwrap_or_else(|error| panic!("the filter {json} must parse: {error}"))
}

/// Keys and score **bits** of a result list, in order.
///
/// The bits, so "identical to the exact search" is an equality and not a
/// tolerance that could hide a different path (ADR-0072).
fn bits_of(hits: &[Hit]) -> Vec<(String, u32)> {
    hits.iter()
        .map(|hit| (hit.key.clone(), hit.score.to_bits()))
        .collect()
}

// --- Selection: the strategy of the plan ---

/// INV-06: a pool at or below `exact_threshold` is searched exactly. Here the
/// threshold is the one of `docs/04` (1 000) and the pool is one passage below
/// it, with the binary scan enabled: the setting does not remove the choice,
/// the size decides it.
#[test]
fn inv_06_pool_below_the_exact_threshold_is_planned_exact() {
    let settings = settings(ScanStrategy::Binary, 1_000, 1_000);
    let plan = plan(&settings, 999, 10, false);

    assert_eq!(
        plan.strategy,
        Strategy::Exact,
        "a pool of 999 passages under a threshold of 1000 must be searched exactly"
    );
    assert_eq!(
        plan.strategy.as_str(),
        "exact",
        "the plan names the strategy the debug output reports"
    );
}

/// `docs/03-recherche.md` writes the condition `taille du pool ≤
/// exact_threshold`: the threshold is inclusive, so a pool exactly at it is
/// exact. Reading it as `<` would make `exact_threshold` mean one less than what
/// the documentation promises.
#[test]
fn a_pool_equal_to_the_exact_threshold_is_planned_exact() {
    let settings = settings(ScanStrategy::Binary, 1_000, 1_000);
    let plan = plan(&settings, 1_000, 10, false);

    assert_eq!(
        plan.strategy,
        Strategy::Exact,
        "the bound of the strategy table is `≤`, so a pool of exactly 1000 passages is exact"
    );
}

/// One passage above the threshold, the scan of Hamming followed by the
/// rescoring is chosen.
#[test]
fn a_pool_above_the_exact_threshold_is_planned_binary() {
    let settings = settings(ScanStrategy::Binary, 1_000, 1_000);
    let plan = plan(&settings, 1_001, 10, false);

    assert_eq!(
        plan.strategy,
        Strategy::Binary,
        "a pool of 1001 passages above a threshold of 1000 must be scanned"
    );
    assert_eq!(
        plan.strategy.as_str(),
        "binary",
        "the plan names the strategy the debug output reports"
    );
}

/// `search.scan = none` means "toujours exact" (`docs/03-recherche.md`): the
/// threshold is not consulted at all, so even a pool of millions of passages is
/// searched exactly.
#[test]
fn scan_none_plans_exact_whatever_the_pool() {
    let settings = settings(ScanStrategy::None, 1_000, 0);
    let plan = plan(&settings, 1_000_000, 10, false);

    assert_eq!(
        plan.strategy,
        Strategy::Exact,
        "scan = none means always exact, whatever the threshold and the pool"
    );
}

/// `exact: true` forces EXACT "quelle que soit sa taille" (`docs/03-recherche`,
/// strategy table), including when the scan is enabled and the threshold is
/// small: this is the reference mode of the evaluation and of the debug.
#[test]
fn exact_requested_plans_exact_above_the_threshold() {
    let settings = settings(ScanStrategy::Binary, 5, 10);
    let plan = plan(&settings, 50, 10, true);

    assert_eq!(
        plan.strategy,
        Strategy::Exact,
        "exact: true must force EXACT on a pool above the threshold"
    );
}

/// The plan is the object the debug output of P7 will report
/// (`docs/05-api.md`, `"plan": { …, "dense_strategy": … }`), so its strategy
/// must have exactly the two spellings the documentation shows.
#[test]
fn the_plan_names_the_strategy_as_docs_05_does() {
    assert_eq!(Strategy::Exact.as_str(), "exact");
    assert_eq!(Strategy::Binary.as_str(), "binary");
}

/// The plan carries `N` beside the strategy: the debug output reports
/// `rescore_candidates` (`docs/05-api.md`), and a BINARY plan without the `N`
/// it kept cannot be interpreted.
#[test]
fn the_plan_carries_the_rescore_candidates_of_the_configuration() {
    let settings = settings(ScanStrategy::Binary, 1_234, 10);
    let plan = plan(&settings, 50, 10, false);

    assert_eq!(
        plan.rescore_candidates, 1_234,
        "the plan must report the N of the configuration it plans with"
    );
}

/// The selection is a total function of three inputs and has no other branch:
/// with the threshold `t`, every pool of `0 … t` is exact, every pool above `t`
/// is binary, `scan = none` removes the choice, and `exact` overrides
/// everything.
///
/// The property pins the *whole* rule of `docs/03`, not the four cases the
/// sheet spells out: an implementation that reads the bound as `<`, that
/// ignores `scan`, or that lets `exact` be overridden would fail here.
#[test]
fn the_selection_is_a_total_function_of_the_setting_and_the_pool() {
    let config = ProptestConfig {
        cases: CASES,
        rng_seed: RngSeed::Fixed(PROPTEST_SEED),
        failure_persistence: None,
        ..ProptestConfig::default()
    };
    let mut runner = TestRunner::new(config);

    runner
        .run(
            &(0_u32..5_000, 0_u32..5_000, any::<bool>(), any::<bool>()),
            |(pool_len, threshold, exact, scan_enabled)| {
                let scan = if scan_enabled {
                    ScanStrategy::Binary
                } else {
                    ScanStrategy::None
                };
                let settings = settings(scan, 1_000, threshold);
                let plan: Plan = plan(&settings, pool_len, 10, exact);

                let expected = if exact || !scan_enabled || pool_len <= threshold {
                    Strategy::Exact
                } else {
                    Strategy::Binary
                };
                prop_assert_eq!(
                    plan.strategy,
                    expected,
                    "pool {} / threshold {} / exact {} / scan {:?}",
                    pool_len,
                    threshold,
                    exact,
                    scan
                );
                Ok(())
            },
        )
        .unwrap_or_else(|failure| panic!("the selection property failed: {failure}"));
}

// --- Results: the planned strategy decides the answer ---

/// INV-06 on a real index: a pool under the threshold is served by EXACT, so
/// its answer is *bit for bit* the one of the exact search — same keys, same
/// order, same score bits (ADR-0072: both go through `dense::cosine`).
#[test]
fn inv_06_pool_below_the_threshold_returns_the_exact_result() {
    let fixture = fixture();
    let snapshot = fixture.snapshot();
    let searcher = fixture.searcher();
    let pool = fixture.pool(&match_all());
    // 50 passages, threshold 100: the pool is under it, so EXACT is planned
    // even though the scan is enabled.
    let settings = settings(ScanStrategy::Binary, 1_000, 100);
    let query = query();

    assert_eq!(
        plan(&settings, pool.len(), 10, false).strategy,
        Strategy::Exact,
        "the fixture must plan EXACT for the comparison to prove INV-06"
    );

    let expected = searcher
        .top_k(&query, &pool, 10)
        .unwrap_or_else(|error| panic!("the exact search must succeed: {error}"));
    let got = search(&snapshot, &searcher, &pool, &query, 10, &settings, false)
        .unwrap_or_else(|error| panic!("the planned search must succeed: {error}"));

    assert_eq!(
        bits_of(&got),
        bits_of(&expected),
        "a pool under the threshold must return the exact keys, in the exact order, with the \
         exact score bits"
    );
}

/// `exact: true` on a pool above the threshold returns the exact answer, not
/// the `N` candidates of the scan: the option is the reference mode of the
/// evaluation.
#[test]
fn inv_06_exact_requested_above_the_threshold_returns_the_exact_result() {
    let fixture = fixture();
    let snapshot = fixture.snapshot();
    let searcher = fixture.searcher();
    let pool = fixture.pool(&match_all());
    // 50 passages, threshold 10 and N = 5: a binary plan would keep 5
    // candidates, so an exact answer proves the option was honoured.
    let settings = settings(ScanStrategy::Binary, 5, 10);
    let query = query();

    assert_eq!(
        plan(&settings, pool.len(), 10, false).strategy,
        Strategy::Binary,
        "the fixture must plan BINARY without the option, so the test is not vacuous"
    );

    let expected = searcher
        .top_k(&query, &pool, 10)
        .unwrap_or_else(|error| panic!("the exact search must succeed: {error}"));
    let got = search(&snapshot, &searcher, &pool, &query, 10, &settings, true)
        .unwrap_or_else(|error| panic!("the forced exact search must succeed: {error}"));

    assert_eq!(
        bits_of(&got),
        bits_of(&expected),
        "exact: true must return the exact keys, order and score bits whatever the pool"
    );
}

/// A pool the filter emptied is planned EXACT (0 ≤ threshold) and returns no
/// result and no error, as the exact search does.
#[test]
fn an_empty_pool_is_planned_exact_and_returns_no_result() {
    let fixture = fixture();
    let snapshot = fixture.snapshot();
    let searcher = fixture.searcher();
    let pool = fixture.pool(&filter("{\"fonds\": \"nowhere\"}"));
    let settings = settings(ScanStrategy::Binary, 5, 10);

    assert!(
        pool.is_empty(),
        "the fixture filter must empty the pool for the test to prove anything"
    );
    assert_eq!(
        plan(&settings, pool.len(), 10, false).strategy,
        Strategy::Exact,
        "an empty pool is under every threshold"
    );

    let got = search(&snapshot, &searcher, &pool, &query(), 10, &settings, false)
        .unwrap_or_else(|error| panic!("an empty pool must not be an error: {error}"));
    assert!(got.is_empty(), "an empty pool must return no result");
}

/// `k == 0` short-circuits a BINARY plan as it does EXACT: the plan still names
/// BINARY (the pool is above the threshold), yet the search returns no result
/// without scanning the pool and rescoring `N` candidates nothing would consume.
/// The query is validated all the same, as the exact strategy validates it
/// before its own `k == 0` check.
#[test]
fn a_zero_k_short_circuits_the_binary_plan() {
    let fixture = fixture();
    let snapshot = fixture.snapshot();
    let searcher = fixture.searcher();
    let pool = fixture.pool(&match_all());
    let config = settings(ScanStrategy::Binary, 10, 10);

    assert_eq!(
        plan(&config, pool.len(), 0, false).strategy,
        Strategy::Binary,
        "the pool is above the threshold, so the plan must be BINARY"
    );
    let got = search(&snapshot, &searcher, &pool, &query(), 0, &config, false)
        .unwrap_or_else(|error| panic!("k = 0 must not be an error: {error}"));
    assert!(got.is_empty(), "k = 0 returns no result");

    let malformed = vec![1.0_f32; 3];
    let error = search(&snapshot, &searcher, &pool, &malformed, 0, &config, false)
        .err()
        .unwrap_or_else(|| panic!("a malformed query must be refused even at k = 0"));
    assert_eq!(
        error.code(),
        ErrorCode::InvalidRequest,
        "the short-circuit keeps the query validation of the exact strategy"
    );
}

/// The BINARY path really is scan → rescore: when `N` covers the pool, the
/// scan keeps every passage and the rescoring scores them all, so the answer is
/// the exact one, bit for bit (ADR-0072, decision 3, and the criterion of
/// `RESCORER-FP16`).
///
/// This is what distinguishes a wired-up BINARY strategy from an EXACT one that
/// ignored the plan: the plan says BINARY and the answer still equals EXACT.
#[test]
fn binary_pool_with_n_covering_the_pool_returns_the_exact_result() {
    let fixture = fixture();
    let snapshot = fixture.snapshot();
    let searcher = fixture.searcher();
    let pool = fixture.pool(&match_all());
    // 50 passages, threshold 10, N = 1000: BINARY, with every passage kept.
    let settings = settings(ScanStrategy::Binary, 1_000, 10);
    let query = query();

    assert_eq!(
        plan(&settings, pool.len(), 10, false).strategy,
        Strategy::Binary,
        "the fixture must plan BINARY for this test to prove the binary path"
    );

    let expected = searcher
        .top_k(&query, &pool, 10)
        .unwrap_or_else(|error| panic!("the exact search must succeed: {error}"));
    let got = search(&snapshot, &searcher, &pool, &query, 10, &settings, false)
        .unwrap_or_else(|error| panic!("the binary search must succeed: {error}"));

    assert_eq!(
        bits_of(&got),
        bits_of(&expected),
        "a binary plan whose N covers the pool must return the exact answer, bit for bit"
    );
}

/// When `N` is below the pool, the answer is the rescoring of the `N`
/// candidates the scan kept, and nothing else: the oracle is recomputed here
/// from `binary::top_k` and `DenseSearcher::rescore`, the two functions the
/// planned BINARY strategy must call.
#[test]
fn binary_pool_with_n_below_the_pool_returns_the_best_rescored_candidates() {
    let fixture = fixture();
    let snapshot = fixture.snapshot();
    let searcher = fixture.searcher();
    let pool = fixture.pool(&match_all());
    let settings = settings(ScanStrategy::Binary, 5, 10);
    let query = query();

    let candidates = binary::top_k(&snapshot, &pool, &quantize(&query), 5)
        .unwrap_or_else(|error| panic!("the scan must succeed: {error}"));
    assert_eq!(
        candidates.len(),
        5,
        "the scan must keep the N candidates the test asked for"
    );
    let rescored = searcher
        .rescore(&query, &candidates, 5)
        .unwrap_or_else(|error| panic!("the rescoring must succeed: {error}"));
    let expected: Vec<(String, u32)> = rescored
        .iter()
        .take(5)
        .map(|candidate| (candidate.key.clone(), candidate.score.to_bits()))
        .collect();

    let got = search(&snapshot, &searcher, &pool, &query, 5, &settings, false)
        .unwrap_or_else(|error| panic!("the binary search must succeed: {error}"));

    assert_eq!(
        bits_of(&got),
        expected,
        "a binary plan must return the rescored candidates of the scan, best first"
    );
}

// --- The library-level PATCH ---

/// Configuration of `docs/04`, with the values the tests of the PATCH need.
const PATCH_CONFIG: &str = r#"{
  "name": "planifier-strategie",
  "model": "m",
  "reference_precision": "fp16",
  "schema": {
    "document_fields": { "fonds": { "type": "keyword" } },
    "passage_fields": { "rang": { "type": "integer" } }
  },
  "cluster_by": [],
  "lexical": { "enabled": false, "language": "fr", "positions": false },
  "search": {
    "scan": "binary",
    "rescore_candidates": 1000,
    "exact_threshold": 100,
    "candidates_per_branch": 100,
    "fusion": { "method": "rrf" }
  },
  "ingest": { "acceptance_sample": 1, "acceptance_min_cosine": 0.9 },
  "compaction": { "dead_ratio": 0.1 }
}"#;

/// Parses [`PATCH_CONFIG`], the index the PATCH tests start from.
fn patch_config() -> IndexConfig {
    IndexConfig::from_json_str(PATCH_CONFIG)
        .unwrap_or_else(|error| panic!("the configuration of the test must parse: {error}"))
}

/// Applies a patch, failing on a refusal.
fn patch(config: &mut IndexConfig, raw: &str) {
    let patch = IndexPatch::from_json_str(raw)
        .unwrap_or_else(|error| panic!("the patch {raw} must parse: {error}"));
    apply_patch(config, &patch)
        .unwrap_or_else(|error| panic!("the patch {raw} must apply: {error}"));
}

/// `PATCH { "search": { "exact_threshold": … } }` changes the strategy the
/// library plans: the threshold is a `search.*` setting, réglable by `PATCH`
/// (`docs/04`, "Ce qui est figé, ce qui est réglable").
#[test]
fn patching_the_exact_threshold_moves_the_planned_strategy() {
    let mut config = patch_config();
    let pool_len = 150;

    assert_eq!(
        plan(&config.search, pool_len, 10, false).strategy,
        Strategy::Binary,
        "the configuration of the test must start above its threshold"
    );

    patch(&mut config, r#"{ "search": { "exact_threshold": 200 } }"#);

    assert_eq!(
        config.search.exact_threshold, 200,
        "the patch must change the threshold"
    );
    assert_eq!(
        plan(&config.search, pool_len, 10, false).strategy,
        Strategy::Exact,
        "a pool under the patched threshold must now be searched exactly"
    );
    assert_eq!(
        plan(&config.search, 250, 10, false).strategy,
        Strategy::Binary,
        "a pool above the patched threshold must still be scanned"
    );
}

/// `PATCH { "search": { "scan": "none" } }` removes the choice: every pool is
/// searched exactly, as the setting is documented ("none = toujours exact").
#[test]
fn patching_the_scan_to_none_plans_exact_whatever_the_pool() {
    let mut config = patch_config();

    assert_eq!(
        plan(&config.search, 10_000_000, 10, false).strategy,
        Strategy::Binary,
        "the configuration of the test must start with the binary scan"
    );

    patch(&mut config, r#"{ "search": { "scan": "none" } }"#);

    assert_eq!(config.search.scan, ScanStrategy::None);
    assert_eq!(
        plan(&config.search, 10_000_000, 10, false).strategy,
        Strategy::Exact,
        "scan = none must plan EXACT whatever the pool"
    );
}

// --- The effective number of candidates: `k` above `N` ---

/// ADR-0074, decision 4: under BINARY, the scan keeps `max(N, k)` candidates and
/// all of them are rescored, so a `k` above `N` still returns `k` results, and
/// no hidden fallback to EXACT happens.
///
/// The oracle is recomputed here: the scan keeps the `max(N, k)` candidates and
/// the rescoring scores them all. A plan that stopped at `N` would return 5
/// results instead of 12, and a fallback to EXACT would return the 12 best
/// passages of the pool rather than the 12 best of the scanned candidates.
#[test]
fn a_binary_plan_keeps_max_of_n_and_k_candidates_and_returns_k_results() {
    let fixture = fixture();
    let snapshot = fixture.snapshot();
    let searcher = fixture.searcher();
    let pool = fixture.pool(&match_all());
    // 50 passages, threshold 10, N = 5: BINARY whatever the k of the request.
    let settings = settings(ScanStrategy::Binary, 5, 10);
    let query = query();
    let k = 12;

    let plan = plan(&settings, pool.len(), k, false);
    assert_eq!(
        plan.strategy,
        Strategy::Binary,
        "the fixture must plan BINARY for this test to prove the effective candidate count"
    );
    assert_eq!(
        plan.rescore_candidates, 12,
        "the plan must report max(N, k) = 12 effective candidates rescorred, not N = 5"
    );

    let candidates = binary::top_k(&snapshot, &pool, &quantize(&query), 12)
        .unwrap_or_else(|error| panic!("the scan must succeed: {error}"));
    assert_eq!(
        candidates.len(),
        12,
        "the scan must keep the max(N, k) candidates the plan reports"
    );
    let rescored = searcher
        .rescore(&query, &candidates, k)
        .unwrap_or_else(|error| panic!("the rescoring must succeed: {error}"));
    let expected: Vec<(String, u32)> = rescored
        .iter()
        .take(k)
        .map(|candidate| (candidate.key.clone(), candidate.score.to_bits()))
        .collect();

    let got = search(&snapshot, &searcher, &pool, &query, k, &settings, false)
        .unwrap_or_else(|error| panic!("the binary search must succeed: {error}"));

    assert_eq!(
        got.len(),
        k,
        "a BINARY plan with k above N must still return k results, never N"
    );
    assert_eq!(
        bits_of(&got),
        expected,
        "the answer must be the rescoring of the max(N, k) candidates, best first"
    );
}
