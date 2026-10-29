// SPDX-License-Identifier: Apache-2.0

//! The pipeline of one search request: pool, branches, fusion, regrouping and
//! payload, for the three modes `dense`, `lexical` and `hybrid`.
//!
//! References: the pipeline table of `docs/03-recherche.md` (steps 3 to 8),
//! `docs/03` ("Fusion (mode `hybrid`)", "Regroupement"), `docs/05-api.md`
//! ("Recherche : requête" and "Recherche : réponse"), `docs/phases/P5-lexical-
//! et-hybride.md` (`FUSIONNER-HYBRIDE`), ADR-0008 (both branches on the same
//! pool, run in parallel), ADR-0075 (regrouping after the fusion, shared
//! over-sampling) and ADR-0079 (order of the answer).
//!
//! # What a mode runs
//!
//! | Mode | Steps |
//! |---|---|
//! | `dense` | pool → dense branch → regrouping → payload |
//! | `lexical` | pool → lexical branch → regrouping → payload |
//! | `hybrid` | pool → dense **and** lexical branches → fusion → regrouping → payload |
//!
//! The two branches of `hybrid` run **in parallel**, on the **same** [`Pool`]
//! (INV-05, ADR-0008): the pool is computed once, and no branch may widen it.
//! Each branch is asked for `candidates_per_branch(k, grouping)` candidates
//! (ADR-0075, decision 2), so the fusion has the over-sampled list the grouping
//! needs after it has dropped duplicates.
//!
//! # Refusals are never degraded
//!
//! A lexical branch that cannot run refuses with the error of
//! [`crate::lexical::search`]: `overloaded` (503) with the message of the state
//! observed (ADR-0088, ADR-0089) or `invalid_request` (400) when the
//! configuration disables the branch (ADR-0089, decision 3). `hybrid` returns
//! that error and serves **nothing**: it never falls back to the dense branch,
//! because a response reduced to the dense ranking would carry a `score_type`
//! and an order the client did not ask for (ADR-0087, decision 2). The `dense`
//! mode keeps serving the same snapshot, since it never asks for the lexical
//! reader.
//!
//! # Score and score type
//!
//! [`ScoreType`] names the score a result carries (`docs/03-recherche.md`):
//! `cosine` in `dense`, `bm25` in `lexical`, `rrf` or `relative` in `hybrid`. A
//! score never compares across queries.
//!
//! Each [`RankedHit`] also carries the rank and the score its branches gave it
//! ([`crate::fusion::BranchScore`]) — for `dense` and `lexical` too, where one
//! branch is enough — because the debug output of P7 reports them as
//! `dense_rank` and `lexical_rank` (`docs/05-api.md`) and recomputing them after
//! the regrouping would be a second traversal of the branch ranking.
//!
//! # The payload step
//!
//! Only the survivors of the regrouping pay for their text and their fields
//! (step 8), through `DenseSearcher::payload`: the reader of `records.sqlite`
//! serves the three modes, since a lexical result is a passage row too.

use crible_core::config::{FusionConfig, SearchConfig};
use crible_core::error::{CribleError, ErrorCode};
use crible_store::generation::Snapshot;

use crate::dense::exact::{DenseSearcher, Hit};
use crate::dense::{Scored, candidates, grouped_candidates, ranks};
use crate::fusion::{self, BranchScore, Fused};
use crate::group::{Candidate, Grouping, candidates_per_branch, group};
use crate::lexical;
use crate::pool::Pool;

/// Branches a search request runs (`mode` of `docs/05-api.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The dense branch alone.
    Dense,
    /// The lexical branch alone.
    Lexical,
    /// Both branches, fused.
    Hybrid,
}

impl Mode {
    /// Mode of a request that does not name one: `hybrid` when the index enables
    /// the lexical branch, `dense` otherwise (`docs/05-api.md`, ADR-0089,
    /// decision 3).
    ///
    /// The rule reads the configuration alone, and not the state of the epoch: an
    /// index whose lexical branch is enabled but unavailable runs `hybrid` and
    /// refuses, rather than degrading to `dense` (ADR-0087, decision 2).
    #[must_use]
    pub const fn default_for(lexical_enabled: bool) -> Self {
        if lexical_enabled {
            Self::Hybrid
        } else {
            Self::Dense
        }
    }

    /// Name of the mode in the plan of the debug output: `"dense"`,
    /// `"lexical"` or `"hybrid"`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dense => "dense",
            Self::Lexical => "lexical",
            Self::Hybrid => "hybrid",
        }
    }

    /// Whether the mode runs the dense branch.
    #[must_use]
    pub const fn uses_dense(self) -> bool {
        matches!(self, Self::Dense | Self::Hybrid)
    }

    /// Whether the mode runs the lexical branch.
    #[must_use]
    pub const fn uses_lexical(self) -> bool {
        matches!(self, Self::Lexical | Self::Hybrid)
    }
}

/// Type of the score a result carries (`score_type` of `docs/03-recherche.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoreType {
    /// Cosine of the dense branch.
    Cosine,
    /// BM25 of the lexical branch.
    Bm25,
    /// Reciprocal rank fusion of `hybrid`.
    Rrf,
    /// Relative score of `hybrid`.
    Relative,
}

impl ScoreType {
    /// Name of the score type in the answer: `"cosine"`, `"bm25"`, `"rrf"` or
    /// `"relative"` (`docs/03-recherche.md`, `docs/05-api.md`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cosine => "cosine",
            Self::Bm25 => "bm25",
            Self::Rrf => "rrf",
            Self::Relative => "relative",
        }
    }
}

/// One search request, as the pipeline takes it.
///
/// `vector` is required by a mode that runs the dense branch, `text` by a mode
/// that runs the lexical one: `hybrid` needs both. A missing input is an
/// `invalid_request`, raised before any branch runs.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    /// Branches to run (`mode`).
    pub mode: Mode,
    /// Query vector, raw: the dense branch normalises it.
    pub vector: Option<&'a [f32]>,
    /// Query text, as the producer wrote it: the lexical branch analyses it.
    pub text: Option<&'a str>,
    /// Number of results; the number of **documents** when [`Request::grouping`]
    /// groups by document.
    pub k: usize,
    /// `exact: true` forces the exact dense calculation.
    pub exact: bool,
    /// Regrouping options; [`Grouping::none`] regroups nothing.
    pub grouping: &'a Grouping,
    /// Fusion of the two branches in `hybrid`; nothing in the two other modes.
    pub fusion: FusionConfig,
}

/// One result of a search, with the payload and the branches that found it.
#[derive(Debug, Clone)]
pub struct RankedHit {
    /// Payload of the passage (step 8); its `score` is the score of
    /// [`ScoreType`], fused in `hybrid`.
    pub hit: Hit,
    /// Rank and score of the dense branch, absent when the branch did not
    /// return the passage.
    pub dense: Option<BranchScore>,
    /// Rank and score of the lexical branch, absent when the branch did not
    /// return the passage.
    pub lexical: Option<BranchScore>,
}

/// Answer of one search request.
#[derive(Debug, Clone)]
pub struct Answer {
    /// Type of the score every result carries.
    pub score_type: ScoreType,
    /// Results, best first, regrouping applied.
    pub results: Vec<RankedHit>,
}

/// Runs the pipeline of `request` on `pool`.
///
/// The pool is computed by the caller, once per request, and shared by the
/// branches (step 3, INV-05). The result follows the order of the answer
/// (ADR-0079); `k` counts documents when the request groups (ADR-0075).
///
/// A lexical branch that cannot run refuses, and `hybrid` refuses with it
/// instead of serving the dense branch alone (ADR-0087, ADR-0088, ADR-0089).
pub fn search(
    snapshot: &Snapshot,
    searcher: &DenseSearcher,
    pool: &Pool,
    request: &Request<'_>,
    search: &SearchConfig,
) -> Result<Answer, CribleError> {
    request.grouping.validate()?;
    // The over-sampling of ADR-0075 covers both branches and the fusion: each of
    // them must be able to fill the regrouped answer.
    let requested = candidates_per_branch(request.k, request.grouping);
    let results = match request.mode {
        Mode::Dense => dense(snapshot, searcher, pool, request, search)?,
        Mode::Lexical => lexical(snapshot, searcher, pool, request, requested)?,
        Mode::Hybrid => hybrid(snapshot, searcher, pool, request, search, requested)?,
    };
    Ok(Answer {
        score_type: score_type(request.mode, request.fusion),
        results,
    })
}

/// Type of the score the mode produces, from the fusion of the request.
const fn score_type(mode: Mode, fusion: FusionConfig) -> ScoreType {
    match mode {
        Mode::Dense => ScoreType::Cosine,
        Mode::Lexical => ScoreType::Bm25,
        Mode::Hybrid => match fusion {
            FusionConfig::Rrf => ScoreType::Rrf,
            FusionConfig::RelativeScore { .. } => ScoreType::Relative,
        },
    }
}

/// Runs the dense branch alone.
///
/// The over-sampling, the branch and the regrouping are those of
/// [`grouped_candidates`], the same composition [`crate::dense::search_grouped`]
/// runs, so the two cannot drift apart.
fn dense(
    snapshot: &Snapshot,
    searcher: &DenseSearcher,
    pool: &Pool,
    request: &Request<'_>,
    search: &SearchConfig,
) -> Result<Vec<RankedHit>, CribleError> {
    let vector = vector(request)?;
    let query = crate::dense::DenseQuery {
        vector,
        k: request.k,
        exact: request.exact,
        grouping: request.grouping,
    };
    let (ranks, kept) = grouped_candidates(snapshot, searcher, pool, &query, search)?;
    kept.into_iter()
        .map(|candidate| {
            let dense = ranks.get(&candidate.iid()).copied();
            Ok(RankedHit {
                hit: searcher.payload(candidate)?,
                dense,
                lexical: None,
            })
        })
        .collect()
}

/// Runs the lexical branch alone.
fn lexical(
    snapshot: &Snapshot,
    searcher: &DenseSearcher,
    pool: &Pool,
    request: &Request<'_>,
    requested: usize,
) -> Result<Vec<RankedHit>, CribleError> {
    let text = text(request)?;
    let branch = lexical::search(snapshot, pool, text, requested)?;
    let ranks = ranks(&branch);
    let kept = group(branch, request.k, request.grouping, |iid| {
        searcher.passage_hash_of(iid)
    })?;
    kept.into_iter()
        .map(|candidate| {
            let lexical = ranks.get(&candidate.iid()).copied();
            Ok(RankedHit {
                hit: searcher.payload(candidate)?,
                dense: None,
                lexical,
            })
        })
        .collect()
}

/// Runs both branches in parallel on the same pool, then fuses and regroups.
fn hybrid(
    snapshot: &Snapshot,
    searcher: &DenseSearcher,
    pool: &Pool,
    request: &Request<'_>,
    search: &SearchConfig,
    requested: usize,
) -> Result<Vec<RankedHit>, CribleError> {
    // Both inputs are checked before any branch runs: a malformed request pays
    // for no work.
    let vector = vector(request)?;
    let text = text(request)?;
    let (dense_branch, lexical_branch) = rayon::join(
        || {
            candidates(
                snapshot,
                searcher,
                pool,
                vector,
                requested,
                search,
                request.exact,
            )
        },
        || lexical::search(snapshot, pool, text, requested),
    );
    // The Dense branch's error is a malformed query vector, which the request
    // alone explains; the lexical branch's refusal is the state of the epoch
    // (ADR-0088, ADR-0089). Both are returned as they are: `hybrid` never
    // degrades to the dense ranking (ADR-0087, decision 2).
    let fused = fusion::fuse(&dense_branch?, &lexical_branch?, request.fusion);
    let kept = group(fused, request.k, request.grouping, |iid| {
        searcher.passage_hash_of(iid)
    })?;
    kept.into_iter()
        .map(|entry| {
            let Fused {
                iid,
                key,
                score,
                dense,
                lexical,
            } = entry;
            Ok(RankedHit {
                hit: searcher.payload(Scored { iid, key, score })?,
                dense,
                lexical,
            })
        })
        .collect()
}

/// Query vector the mode needs, or the `invalid_request` that names the mode.
fn vector<'a>(request: &Request<'a>) -> Result<&'a [f32], CribleError> {
    request.vector.ok_or_else(|| {
        invalid_request(format!(
            "mode {} runs the dense branch, so it needs a query vector",
            request.mode.as_str()
        ))
    })
}

/// Query text the mode needs, or the `invalid_request` that names the mode.
fn text<'a>(request: &Request<'a>) -> Result<&'a str, CribleError> {
    request.text.ok_or_else(|| {
        invalid_request(format!(
            "mode {} runs the lexical branch, so it needs a query text",
            request.mode.as_str()
        ))
    })
}

/// Error raised when the request itself is malformed.
fn invalid_request(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::InvalidRequest, message)
}
