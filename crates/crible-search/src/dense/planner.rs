// SPDX-License-Identifier: Apache-2.0

//! Planning of the dense strategy: `EXACT` or `BINARY`, and the number of
//! candidates the binary scan keeps.
//!
//! References: `docs/phases/P4-recherche-rapide.md` (`PLANIFIER-STRATEGIE`,
//! phase P4), `docs/03-recherche.md` ("Branche dense", strategy table and
//! "Réglages de précision"), `docs/05-api.md` ("Recherche : réponse", the plan
//! of the debug output) and ADR-0074 (threshold domain, materialised default,
//! strategy names and `k` above `N`).
//!
//! # The rule
//!
//! One rule decides everything, and it is total:
//!
//! - `exact: true` of the request forces `EXACT`, "quelle que soit la taille"
//!   of the pool;
//! - `search.scan: none` means "toujours exact", so it forces `EXACT` too;
//! - otherwise a pool of at most `search.exact_threshold` passages, **equality
//!   included**, is searched exactly, and a larger pool is scanned
//!   ([`Strategy::Binary`]).
//!
//! The threshold accepts every `u32` (ADR-0074, decision 1): `0` lets only an
//! empty pool be exact, and `u32::MAX` always is. No additional error exists.
//!
//! # The number of candidates
//!
//! `k` is the number of results this branch returns, not the number the API
//! returns: the pipeline gives the branch `candidates_per_branch` and asks for
//! more when it groups passages by document (`REGROUPER-RESULTATS`). Under
//! `BINARY`, the scan keeps `max(N, k)` candidates and all of them are
//! rescored, so the branch can always return `k` results (ADR-0074,
//! decision 4). [`Plan::rescore_candidates`] carries that effective number: it
//! is what the debug output reports, and it differs from `N` only when `k`
//! exceeds it.
//!
//! [`search_grouped`] is the entry point of the pipeline: it asks for
//! `candidates_per_branch(k, grouping)` candidates, regroups them with
//! [`crate::group::group`] (step 7 of `docs/03-recherche.md`), and reads the
//! payload of the survivors only (step 8). [`search`] is the same branch without
//! over-sampling, for a caller that supplies its own candidate count.
//!
//! Under `EXACT`, nothing is rescored, so the plan carries `N` as configured:
//! the plan then describes the settings the strategy runs with, which is what
//! `docs/05-api.md` reports beside `dense_strategy`.
//!
//! # What a plan is not
//!
//! The plan is the dense part of the search plan of `docs/05-api.md`, and it is
//! not serialised in P4: the debug output is assembled in P7, from these two
//! fields. [`Strategy::as_str`] gives the two published spellings, `"exact"`
//! and `"binary"`, and nothing else (ADR-0074, decision 3).

use crible_core::config::{ScanStrategy, SearchConfig};
use crible_core::error::CribleError;
use crible_store::codes::quantize;
use crible_store::generation::Snapshot;

use super::Scored;
use super::binary;
use super::exact::{ExactSearcher, Hit};
use crate::group::Grouping;
use crate::pool::Pool;

/// Strategy the dense branch runs for one query.
///
/// This is not the `search.scan` setting: `ScanStrategy::None` removes the
/// choice, it is not a third strategy. The plan names the strategy that
/// actually ran, so `search.scan: binary` with `exact: true` plans
/// [`Strategy::Exact`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// Exact dot product over the whole pool.
    Exact,
    /// Binary scan of the pool, then the exact rescoring of its candidates.
    Binary,
}

impl Strategy {
    /// Name of the strategy in the plan of the debug output: `"exact"` or
    /// `"binary"` (`docs/05-api.md`, ADR-0074).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Binary => "binary",
        }
    }
}

/// Dense part of the search plan, as the debug output will report it in P7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    /// Strategy the branch runs.
    pub strategy: Strategy,
    /// Candidates the binary scan keeps and the rescoring scores.
    ///
    /// `N` under `EXACT`, `max(N, k)` under `BINARY` (ADR-0074, decision 4).
    pub rescore_candidates: u32,
}

/// Chooses the strategy of one query and the number of candidates it rescans.
///
/// `pool_len` is the number of live passages the filter kept, `k` the number of
/// results the dense branch returns, and `exact` the request option that forces
/// the exact calculation. An empty pool is exact whatever the threshold, since
/// `0 ≤ exact_threshold` for every `u32`.
#[must_use]
pub fn plan(search: &SearchConfig, pool_len: u32, k: usize, exact: bool) -> Plan {
    if exact || search.scan == ScanStrategy::None || pool_len <= search.exact_threshold {
        return Plan {
            strategy: Strategy::Exact,
            rescore_candidates: search.rescore_candidates,
        };
    }
    Plan {
        strategy: Strategy::Binary,
        // `k` above `u32::MAX` cannot happen (`docs/05-api.md` caps `k` at
        // 1 000), and the saturation keeps the rule total if a direct caller
        // ever passes more.
        rescore_candidates: search
            .rescore_candidates
            .max(u32::try_from(k).unwrap_or(u32::MAX)),
    }
}

/// Runs the dense branch of one query with the planned strategy.
///
/// `EXACT` is [`ExactSearcher::top_k`], unchanged. `BINARY` is the scan of
/// [`binary::top_k`], run on `max(N, k)` candidates, followed by
/// [`ExactSearcher::rescore`], which scores them with the same dot product as
/// the exact strategy (ADR-0072); the `k` best are kept. Both strategies
/// normalise the query through the searcher, so a passage receives the same
/// bits from either of them.
///
/// The query code the scan needs is quantised here, from the normalised query:
/// a caller passes the raw vector, as it does for the exact search. The sign of
/// a component is what the code keeps, and normalising the query does not
/// change it.
///
/// `k == 0` and an empty pool return no result and no error, as the exact
/// strategy does.
pub fn search(
    snapshot: &Snapshot,
    searcher: &ExactSearcher,
    pool: &Pool,
    query: &[f32],
    k: usize,
    search: &SearchConfig,
    exact: bool,
) -> Result<Vec<Hit>, CribleError> {
    let candidates = candidates(snapshot, searcher, pool, query, k, search, exact)?;
    candidates
        .into_iter()
        .map(|candidate| searcher.payload(candidate))
        .collect()
}

/// Runs the dense branch of one query with the planned strategy and returns its
/// `k` best candidates, payload unread.
///
/// This is the candidate step of [`search`], separated so that the regrouping
/// step of `docs/03-recherche.md` (step 7) can run between the ranking and the
/// payload step (step 8): only the candidates [`crate::group::group`] keeps pay for their
/// text and their fields.
pub fn candidates(
    snapshot: &Snapshot,
    searcher: &ExactSearcher,
    pool: &Pool,
    query: &[f32],
    k: usize,
    search: &SearchConfig,
    exact: bool,
) -> Result<Vec<Scored>, CribleError> {
    let plan = plan(search, pool.len(), k, exact);
    if plan.strategy == Strategy::Exact {
        return searcher.top_k_candidates(query, pool, k);
    }

    let normalised = searcher.normalised_query(query, pool.is_empty())?;
    let code = quantize(&normalised);
    let candidates = binary::top_k(
        snapshot,
        pool,
        &code,
        usize::try_from(plan.rescore_candidates).unwrap_or(usize::MAX),
    )?;
    let rescored = searcher.rescore(query, &candidates)?;
    Ok(rescored.into_iter().take(k).collect())
}

/// The dense part of one search request, as the regrouping entry point takes
/// it.
///
/// It carries the query vector, the number of results the API asks for (`k`,
/// which counts documents when the request groups), the `exact` option that
/// forces the exact strategy, and the regrouping options
/// (`docs/05-api.md`, "Recherche : requête").
#[derive(Debug, Clone, Copy)]
pub struct DenseQuery<'a> {
    /// Query vector, raw: both strategies normalise it.
    pub vector: &'a [f32],
    /// Number of results the request asks for; the number of **documents** when
    /// [`DenseQuery::grouping`] groups by document.
    pub k: usize,
    /// `exact: true` forces the exact calculation whatever the pool holds.
    pub exact: bool,
    /// Regrouping options of the request; [`Grouping::none`] regroups nothing.
    pub grouping: &'a Grouping,
}

/// Runs the dense branch of one query as the input of the regrouping step.
///
/// The branch is asked for `candidates_per_branch(k, grouping)` candidates
/// instead of `k` (`docs/03-recherche.md`, "Sur-échantillonnage"), because
/// deduplication and grouping drop some of them. The candidates are then
/// regrouped by [`crate::group::group`], and only the survivors reach the
/// payload step.
///
/// The over-sampled count is also the `k` of [`plan`], so under `BINARY` the
/// scan keeps `max(N, candidates_per_branch)` candidates and all of them are
/// rescored (ADR-0074, decision 4; ADR-0075, decision 4): no second rule
/// decides the rescoring.
///
/// [`Grouping::none`] asks for `max(k, 100)` candidates and keeps the first
/// `k` of them, which is the same answer as [`search`] for the same query.
pub fn search_grouped(
    snapshot: &Snapshot,
    searcher: &ExactSearcher,
    pool: &Pool,
    request: &DenseQuery<'_>,
    search: &SearchConfig,
) -> Result<Vec<Hit>, CribleError> {
    request.grouping.validate()?;
    let requested = crate::group::candidates_per_branch(request.k, request.grouping);
    let candidates = candidates(
        snapshot,
        searcher,
        pool,
        request.vector,
        requested,
        search,
        request.exact,
    )?;
    let grouped = crate::group::group(candidates, request.k, request.grouping, |iid| {
        searcher.passage_hash_of(iid)
    })?;
    grouped
        .into_iter()
        .map(|candidate| searcher.payload(candidate))
        .collect()
}
