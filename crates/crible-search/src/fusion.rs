// SPDX-License-Identifier: Apache-2.0

//! Fusion of the dense and lexical rankings of the `hybrid` mode: `rrf` and
//! `relative_score`.
//!
//! References: `docs/03-recherche.md` ("Fusion (mode `hybrid`)" and step 6 of
//! the pipeline), `docs/05-api.md` (`fusion` of the search request, `score_type`
//! and the `per_result` part of the debug output), `docs/phases/P5-lexical-et-
//! hybride.md` (`FUSIONNER-HYBRIDE`), ADR-0008 (both branches on the same pool,
//! two methods, the rank and the score of each branch kept for the debug) and
//! ADR-0079 (the order of the answer: descending score, then ascending key),
//! and ADR-0090 (a constant branch normalises its candidates to 1, an empty
//! branch contributes nothing, and `alpha` is in `[0, 1]`).
//!
//! # What a fusion receives
//!
//! It receives the two lists of the branches, both already ordered best first by
//! the order of the answer ([`crate::dense::Scored`]), and both computed on the
//! same pool (INV-05, ADR-0008). The rank of a candidate is its position in its
//! branch, from 1. The fusion combines the ranks and the scores alone; it reads
//! no payload and no store, so it is pure and testable on known lists.
//!
//! # `rrf`
//!
//! `score = Σ 1 / (60 + rank)`, the rank starting at 1
//! (`docs/03-recherche.md`). A passage absent from a branch has no term there:
//! the sum runs over the branches it appears in. [`RRF_K`] is the constant 60.
//!
//! # `relative_score`
//!
//! Each branch is normalised min-max **on its own candidates**: its best
//! candidate scores 1, its worst 0. The fused score is `α · dense +
//! (1 − α) · lexical`, where a passage absent from a branch counts 0 there.
//!
//! Two degenerate cases have to be defined, and `docs/03` only said "min-max on
//! its own candidates": ADR-0090, decision 1, settles both, and this module
//! follows it.
//!
//! - **an empty branch** contributes nothing: it has no candidate to normalise,
//!   and every passage is absent from it, so its weight counts 0 everywhere;
//! - **a branch whose scores are all equal** has `min = max`, and the textbook
//!   quotient would divide by zero. Its candidates all score **1** (ADR-0090,
//!   decision 1): min-max maps the best candidate of a branch to 1, and when no
//!   candidate is worse than another, none of them is worst. The alternative, 0,
//!   would make a passage found by that branch indistinguishable from one it
//!   never matched, while `rrf` and the "absent = 0" rule of `docs/03` both give
//!   presence an advantage. The rule is written once, in [`Bounds::normalise`],
//!   and covered by `a_branch_of_equal_scores_normalises_its_candidates_to_one`.
//!
//! `alpha` itself is in `[0, 1]`, bounds included, on a finite value; the
//! control lives in `crible_core::config::check_alpha` and refuses everything
//! else in `invalid_request` (ADR-0090, decision 2).
//!
//! The arithmetic is done in `f64` and rounded once, to `f32`, at the end: an
//! `f32` sum of two `1 / (60 + rank)` terms is the same to within a rounding,
//! and a single rounding keeps the fused score reproducible for a known list.
//!
//! # What it returns
//!
//! One [`Fused`] per passage of either list, ordered by the order of the answer
//! (descending fused score, then ascending key, ADR-0079, never the internal
//! identifier). Each carries the rank and the score its branches gave it, which
//! is what the debug output of P7 reports as `dense_rank`, `dense_score`,
//! `lexical_rank` and `lexical_score` (`docs/05-api.md`): nothing here has to be
//! recomputed later, and the regrouping of step 7 keeps them too.
//!
//! ```text
//! cargo test -p crible-search fusion::tests
//! ```

use std::cmp::Ordering;
use std::collections::HashMap;

use crible_core::config::FusionConfig;
use crible_core::ids::Iid;

use crate::dense::{Scored, compare_ranked};
use crate::group::Candidate;

/// Constant of `rrf`: the rank is added to it, so rank 1 contributes
/// `1 / 61` (`docs/03-recherche.md`).
pub const RRF_K: u32 = 60;

/// Rank and score of one result in one branch.
///
/// It is the `dense` or the `lexical` part of the debug output: the rank starts
/// at 1, and the score is the one the branch computed (a cosine or a BM25
/// score), never a fused one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BranchScore {
    /// Rank of the passage in its branch, from 1.
    pub rank: u32,
    /// Score the branch gave the passage.
    pub score: f32,
}

/// One result of the fusion, with what each branch gave it.
///
/// The order of the answer is that of the dense branch and of the lexical one
/// (ADR-0079): descending fused score, then ascending key `doc_id#ord`, which is
/// why [`Fused`] implements [`Ord`] on [`crate::dense::compare_ranked`] and not
/// on the internal identifier.
#[derive(Debug, Clone, PartialEq)]
pub struct Fused {
    /// Internal identifier of the passage, kept until the payload step.
    pub iid: Iid,
    /// Textual key `doc_id#ord`, the tie-break and the document of the
    /// regrouping.
    pub key: String,
    /// Fused score: an RRF sum or an `α`-weighted relative score.
    pub score: f32,
    /// Rank and score of the dense branch, absent when the branch did not
    /// return the passage.
    pub dense: Option<BranchScore>,
    /// Rank and score of the lexical branch, absent when the branch did not
    /// return the passage.
    pub lexical: Option<BranchScore>,
}

impl PartialOrd for Fused {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Fused {
    fn cmp(&self, other: &Self) -> Ordering {
        // Greater is better, as for `Scored`: the fused score wins, then the
        // smaller key. The internal identifier never decides (INV-01,
        // ADR-0079).
        compare_ranked(self.score, &self.key, other.score, &other.key)
    }
}

impl Eq for Fused {}

impl Candidate for Fused {
    fn iid(&self) -> Iid {
        self.iid
    }

    fn key(&self) -> &str {
        &self.key
    }
}

/// Fuses the two branches with the method of `config`.
///
/// `config` is the `fusion` of the request when it names one, and the default
/// of the index otherwise (`docs/05-api.md`). The result holds every passage of
/// either list, ordered by the order of the answer, with the contributions of
/// both branches: the regrouping of step 7 truncates it.
#[must_use]
pub fn fuse(dense: &[Scored], lexical: &[Scored], config: FusionConfig) -> Vec<Fused> {
    match config {
        FusionConfig::Rrf => rrf(dense, lexical),
        FusionConfig::RelativeScore { alpha } => relative_score(dense, lexical, alpha),
    }
}

/// Reciprocal rank fusion: `Σ 1 / (60 + rank)`, rank from 1.
///
/// A passage absent from a branch contributes nothing there
/// (`docs/03-recherche.md`). The result is ordered by the order of the answer.
#[must_use]
pub fn rrf(dense: &[Scored], lexical: &[Scored]) -> Vec<Fused> {
    let mut fused = union(dense, lexical);
    for entry in &mut fused {
        let mut sum = 0.0_f64;
        for contribution in [entry.dense, entry.lexical].into_iter().flatten() {
            sum += 1.0 / (f64::from(RRF_K) + f64::from(contribution.rank));
        }
        entry.score = sum as f32;
    }
    sort(&mut fused);
    fused
}

/// Relative score: min-max on each branch, `α · dense + (1 − α) · lexical`.
///
/// `alpha` is the weight of the dense branch (`docs/03-recherche.md`), in
/// `[0, 1]`, bounds included, on a finite value (ADR-0090, decision 2); the
/// branch it does not weight keeps the rest. A passage absent from a branch
/// counts 0 there, and a branch whose scores are all equal normalises its
/// candidates to 1, without dividing by zero ([`Bounds::normalise`], ADR-0090,
/// decision 1). The result is ordered by the order of the answer.
#[must_use]
pub fn relative_score(dense: &[Scored], lexical: &[Scored], alpha: f64) -> Vec<Fused> {
    let dense_bounds = Bounds::of(dense);
    let lexical_bounds = Bounds::of(lexical);
    let mut fused = union(dense, lexical);
    for entry in &mut fused {
        // A branch with a contribution is not empty, so its bounds exist; `zip`
        // states both facts in one place, and a missing side counts 0.
        let dense_term = dense_bounds
            .zip(entry.dense)
            .map_or(0.0, |(bounds, contribution)| {
                alpha * bounds.normalise(contribution.score)
            });
        let lexical_term = lexical_bounds
            .zip(entry.lexical)
            .map_or(0.0, |(bounds, contribution)| {
                (1.0 - alpha) * bounds.normalise(contribution.score)
            });
        entry.score = (dense_term + lexical_term) as f32;
    }
    sort(&mut fused);
    fused
}

/// Lowest and highest score of one branch, for the min-max normalisation.
///
/// It is built from a non-empty list only: an empty branch has nothing to
/// normalise, and every passage is absent from it (`docs/03-recherche.md`).
#[derive(Debug, Clone, Copy)]
struct Bounds {
    /// Score of the worst candidate of the branch.
    min: f32,
    /// Score of the best candidate of the branch.
    max: f32,
}

impl Bounds {
    /// Bounds of `branch`, or `None` when it returned no candidate.
    fn of(branch: &[Scored]) -> Option<Self> {
        let mut scores = branch.iter().map(|candidate| candidate.score);
        let first = scores.next()?;
        let mut bounds = Self {
            min: first,
            max: first,
        };
        for score in scores {
            if score < bounds.min {
                bounds.min = score;
            }
            if score > bounds.max {
                bounds.max = score;
            }
        }
        Some(bounds)
    }

    /// The score of a candidate of the branch, between 0 and 1.
    ///
    /// The subtraction and the division are done in `f64`, so a candidate gets
    /// the same normalised value whatever the magnitude of the raw scores. When
    /// every candidate of the branch has the same score, `min = max` and the
    /// quotient would divide by zero: the candidates all score 1 (ADR-0090,
    /// decision 1), the value min-max gives the best candidate of a branch,
    /// since none of them is worst.
    fn normalise(self, score: f32) -> f64 {
        let span = f64::from(self.max) - f64::from(self.min);
        if span > 0.0 {
            (f64::from(score) - f64::from(self.min)) / span
        } else {
            1.0
        }
    }
}

/// One [`Fused`] per passage of either branch, without any score yet.
///
/// The two lists are walked in a fixed order, dense first, so the rank of a
/// passage is its position in its own branch and the contributions are assigned
/// once. A passage cannot appear twice in one branch: both branches return at
/// most one candidate per internal identifier.
fn union(dense: &[Scored], lexical: &[Scored]) -> Vec<Fused> {
    let mut entries: Vec<Fused> = Vec::with_capacity(dense.len() + lexical.len());
    let mut slot: HashMap<Iid, usize> = HashMap::with_capacity(dense.len() + lexical.len());
    for (is_dense, branch) in [(true, dense), (false, lexical)] {
        for (position, candidate) in branch.iter().enumerate() {
            let rank = u32::try_from(position + 1).unwrap_or(u32::MAX);
            let contribution = BranchScore {
                rank,
                score: candidate.score,
            };
            match slot.get(&candidate.iid) {
                Some(&index) => {
                    let entry = &mut entries[index];
                    if is_dense {
                        entry.dense = Some(contribution);
                    } else {
                        entry.lexical = Some(contribution);
                    }
                }
                None => {
                    slot.insert(candidate.iid, entries.len());
                    entries.push(Fused {
                        iid: candidate.iid,
                        key: candidate.key.clone(),
                        score: 0.0,
                        dense: is_dense.then_some(contribution),
                        lexical: (!is_dense).then_some(contribution),
                    });
                }
            }
        }
    }
    entries
}

/// Orders the fused list the way the answer is ordered (ADR-0079).
fn sort(fused: &mut [Fused]) {
    fused.sort_unstable_by(|left, right| right.cmp(left));
}

#[cfg(test)]
mod tests {
    use crible_core::config::FusionConfig;
    use crible_core::ids::Iid;

    use super::{Fused, fuse, relative_score, rrf};
    use crate::dense::Scored;

    /// One candidate of a branch, with its internal identifier, its key and its
    /// score.
    fn candidate(iid: u32, key: &str, score: f32) -> Scored {
        Scored {
            iid: Iid::new(iid),
            key: key.to_owned(),
            score,
        }
    }

    /// The keys of a fused list, in order.
    fn keys(fused: &[Fused]) -> Vec<&str> {
        fused.iter().map(|entry| entry.key.as_str()).collect()
    }

    /// Rank 1 of one branch contributes `1 / 61`, and nothing else does: this
    /// pins the constant 60 and the rank starting at 1.
    #[test]
    fn rrf_scores_the_first_rank_of_one_branch_as_one_over_sixty_one() {
        let dense = [candidate(7, "A#0", 0.9)];
        let fused = rrf(&dense, &[]);
        assert_eq!(fused.len(), 1);
        assert_eq!(
            fused[0].score,
            (1.0_f64 / 61.0) as f32,
            "rank 1 must contribute 1 / (60 + 1)"
        );
        assert_eq!(
            fused[0].dense,
            Some(super::BranchScore {
                rank: 1,
                score: 0.9
            }),
            "the rank and the score of the dense branch must be kept"
        );
        assert_eq!(
            fused[0].lexical, None,
            "a passage absent from a branch has no contribution there"
        );
    }

    /// A passage of both branches sums the two terms, and so outranks a passage
    /// of one branch alone.
    #[test]
    fn rrf_adds_the_contributions_of_the_two_branches() {
        let dense = [
            candidate(0, "A#0", 0.9),
            candidate(1, "B#0", 0.8),
            candidate(2, "C#0", 0.7),
        ];
        let lexical = [candidate(1, "B#0", 12.0), candidate(3, "D#0", 9.0)];
        let fused = rrf(&dense, &lexical);

        assert_eq!(
            keys(&fused),
            vec!["B#0", "A#0", "D#0", "C#0"],
            "B is second of the dense branch and first of the lexical one, so it outranks A and \
             D, which are first of one branch only; D is second of the lexical branch, so it \
             precedes C, third of the dense one"
        );
        let b = &fused[0];
        assert_eq!(
            b.score,
            (1.0_f64 / 62.0 + 1.0_f64 / 61.0) as f32,
            "B must sum 1 / (60 + 2) and 1 / (60 + 1)"
        );
        assert_eq!(
            b.dense,
            Some(super::BranchScore {
                rank: 2,
                score: 0.8
            })
        );
        assert_eq!(
            b.lexical,
            Some(super::BranchScore {
                rank: 1,
                score: 12.0
            })
        );
        assert_eq!(fused[1].score, (1.0_f64 / 61.0) as f32);
        assert_eq!(fused[2].score, (1.0_f64 / 62.0) as f32);
        assert_eq!(fused[3].score, (1.0_f64 / 63.0) as f32);
    }

    /// A tie of the fused score is broken by the ascending key, never by the
    /// internal identifier: ADR-0079 is what makes the answer reproducible.
    #[test]
    fn the_fused_order_breaks_a_tie_on_the_ascending_key() {
        // The identifiers are the reverse of the keys on purpose: an
        // implementation that ordered on `iid` would return B before A.
        let dense = [candidate(1, "A#0", 0.5)];
        let lexical = [candidate(0, "B#0", 3.0)];
        let fused = rrf(&dense, &lexical);
        assert_eq!(
            fused[0].score, fused[1].score,
            "both passages are rank 1 of one branch"
        );
        assert_eq!(keys(&fused), vec!["A#0", "B#0"]);
    }

    /// Min-max maps the best candidate of a branch to 1 and its worst to 0, in
    /// each branch separately, then weights the two.
    #[test]
    fn relative_score_normalises_each_branch_before_weighting() {
        let dense = [
            candidate(0, "A#0", 1.0),
            candidate(1, "B#0", 0.5),
            candidate(2, "C#0", 0.0),
        ];
        let lexical = [candidate(1, "B#0", 10.0), candidate(3, "D#0", 5.0)];
        let fused = relative_score(&dense, &lexical, 0.5);

        let scores: Vec<(String, f32)> = fused
            .iter()
            .map(|entry| (entry.key.clone(), entry.score))
            .collect();
        assert_eq!(
            scores,
            vec![
                ("B#0".to_owned(), 0.75_f32),
                ("A#0".to_owned(), 0.5_f32),
                ("C#0".to_owned(), 0.0_f32),
                ("D#0".to_owned(), 0.0_f32),
            ],
            "B is the middle of the dense branch and the best of the lexical one; A is the best \
             of the dense branch only; C is the worst of the dense branch only and D the worst \
             of the lexical one, so both score 0 and their keys order them"
        );
    }

    /// `α` weights the dense branch and `1 − α` the lexical one: `α = 1` keeps
    /// the dense ranking alone, `α = 0` the lexical one alone.
    #[test]
    fn the_alpha_weights_the_two_branches() {
        let dense = [candidate(0, "A#0", 0.9), candidate(1, "B#0", 0.1)];
        let lexical = [candidate(2, "C#0", 8.0)];

        let dense_only = relative_score(&dense, &lexical, 1.0);
        assert_eq!(
            keys(&dense_only),
            vec!["A#0", "B#0", "C#0"],
            "alpha = 1 leaves the dense branches in their order and gives the lexical one nothing"
        );

        let lexical_only = relative_score(&dense, &lexical, 0.0);
        assert_eq!(
            keys(&lexical_only),
            vec!["C#0", "A#0", "B#0"],
            "alpha = 0 leaves the lexical branch alone; the dense passages then tie at 0 and \
             follow their key"
        );
    }

    /// A passage found by one branch only receives 0 from the other, so it is
    /// ranked exactly as the branch it matched scores it.
    #[test]
    fn a_passage_of_one_branch_only_gets_zero_from_the_other() {
        let dense = [candidate(0, "A#0", 1.0), candidate(1, "B#0", 0.0)];
        let lexical = [candidate(2, "C#0", 3.0)];
        let fused = relative_score(&dense, &lexical, 0.5);
        assert_eq!(
            keys(&fused),
            vec!["A#0", "C#0", "B#0"],
            "A is the best of the dense branch, C the only candidate of the lexical branch, and \
             B the worst of the dense branch: A and C tie at 0.5 and their keys order them"
        );
        let by_key = |key: &str| {
            fused
                .iter()
                .find(|entry| entry.key == key)
                .map(|entry| entry.score)
        };
        assert_eq!(by_key("A#0"), Some(0.5), "0.5 * 1 + 0");
        assert_eq!(
            by_key("C#0"),
            Some(0.5),
            "0 + 0.5 * 1, one candidate is its best"
        );
        assert_eq!(
            by_key("B#0"),
            Some(0.0),
            "0.5 * 0 and nothing from the lexical branch"
        );
    }

    /// A branch whose scores are all equal has `min = max`: the quotient would
    /// divide by zero, so its candidates all score 1 (ADR-0090, decision 1).
    /// Nothing else may change: an empty branch still contributes nothing.
    #[test]
    fn a_branch_of_equal_scores_normalises_its_candidates_to_one() {
        let dense = [candidate(0, "A#0", 0.7), candidate(1, "B#0", 0.7)];
        let lexical = [candidate(2, "C#0", 2.0), candidate(3, "D#0", 2.0)];
        let fused = relative_score(&dense, &lexical, 0.5);

        for entry in &fused {
            assert!(
                entry.score.is_finite(),
                "a constant branch must not divide by zero, got {} for {}",
                entry.score,
                entry.key
            );
            assert_eq!(
                entry.score, 0.5,
                "every candidate of a constant branch scores 1, and each passage here is in one \
                 branch only, so it takes that branch's weight"
            );
        }
        assert_eq!(
            keys(&fused),
            vec!["A#0", "B#0", "C#0", "D#0"],
            "the tie the constant branches create is broken by the key"
        );

        let one = relative_score(&[], &[candidate(4, "E#0", 5.0)], 0.5);
        assert_eq!(
            one[0].score, 0.5,
            "a branch of one candidate is its own best: it scores 1 and takes the lexical weight"
        );
    }

    /// An empty branch contributes nothing (ADR-0090, decision 1): the other
    /// branch alone decides, and the answer holds no result when both are empty.
    #[test]
    fn an_empty_branch_contributes_nothing() {
        let dense = [candidate(0, "A#0", 0.2), candidate(1, "B#0", 0.1)];
        let fused = relative_score(&dense, &[], 0.3);
        assert_eq!(keys(&fused), vec!["A#0", "B#0"]);
        assert_eq!(
            fused[0].score,
            (0.3_f64 * 1.0) as f32,
            "the dense weight only, since the lexical branch has nothing to normalise"
        );
        assert!(rrf(&[], &[]).is_empty(), "two empty branches fuse nothing");
        assert!(relative_score(&[], &[], 0.5).is_empty());
    }

    /// The method comes from the configuration of the request or of the index:
    /// [`fuse`] is the only entry point of the pipeline.
    #[test]
    fn fuse_chooses_the_method_of_the_configuration() {
        let dense = [candidate(0, "A#0", 0.9), candidate(1, "B#0", 0.2)];
        let lexical = [candidate(1, "B#0", 6.0), candidate(2, "C#0", 3.0)];

        assert_eq!(
            fuse(&dense, &lexical, FusionConfig::Rrf),
            rrf(&dense, &lexical),
            "rrf is the method of the configuration `rrf`"
        );
        assert_eq!(
            fuse(
                &dense,
                &lexical,
                FusionConfig::RelativeScore { alpha: 0.75 }
            ),
            relative_score(&dense, &lexical, 0.75),
            "relative_score takes its alpha from the configuration"
        );
    }
}
