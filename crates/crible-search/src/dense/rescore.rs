// SPDX-License-Identifier: Apache-2.0

//! Rescoring of the candidates of the binary scan: their exact cosine.
//!
//! References: `docs/phases/P4-recherche-rapide.md` (`RESCORER-FP16`, phase P4),
//! `docs/03-recherche.md` ("Branche dense", strategy BINARY: "scan de Hamming
//! sur le pool → top-N → rescoring fp16 → tri"), ADR-0072 (one calculation of
//! the dot product for the exact strategy and for the rescoring) and ADR-0070
//! (the `f16` kernel).
//!
//! # What the rescoring computes
//!
//! The binary scan ranks the pool by Hamming distance, which is cheap and
//! approximate; the rescoring takes the `N` passages it kept and computes their
//! exact cosine, then sorts them. The score comes from
//! `cosine`, the one function the dense branch uses (ADR-0072):
//! the same components, the same kernel, the same rounding as the exact
//! strategy.
//!
//! The vectors are read from the memory map of the segment that holds each
//! passage, in the precision the segment stores. The binary scan's copy of the
//! codes is not read here: the rescoring needs the reference vectors, not their
//! signs.
//!
//! # Identity with the exact strategy
//!
//! The acceptance criterion of the task sheet is: when `N` is at least the size
//! of the pool, the rescoring returns **the same keys, in the same order, with
//! bit-identical scores** as `EXACT` (ADR-0072, decision 3).
//!
//! This module meets it by construction, in three places:
//!
//! - the candidate list then covers the whole pool, because the scan returns
//!   the whole pool when `N` exceeds it;
//! - both strategies score through the same function, and normalise the query
//!   through the searcher's own validation, so a passage receives the same bits
//!   from either of them;
//! - both rank with [`Scored`], the single carrier of the order "descending
//!   score, then ascending textual key", so a tie cannot be broken one way by a
//!   heap and another way by a sort.
//!
//! # Keys after the score (ADR-0079, decision 3, ADR-0083)
//!
//! A key lives in `records.sqlite`, one row per candidate, and MES-03 attributes
//! 69 to 93 % of the hot cost of the rescoring to those random page reads
//! (`docs/mesures/MES-03-latence-rescoring.md`). The loop therefore scores first
//! and reads the **key** afterwards, and only where the ranking needs one:
//!
//! 1. the scores alone are sorted by descending score, then ascending
//!    identifier: the identifier only breaks the ties of this intermediate
//!    ranking, where the keys are not known yet, and it never leaves the engine
//!    (rule 11 of `AGENTS.md`);
//! 2. the keys are then read **by group of equal score**, and the whole group is
//!    ordered by ascending key before the next group is touched. The cut falls
//!    after a group, so a tie cannot be resolved by the identifier: a group that
//!    straddles the `limit` of the caller is entirely read and entirely sorted,
//!    and the caller's truncation keeps the smallest keys of it, as `EXACT`
//!    would (ADR-0045, ADR-0072 decision 3).
//!
//! Reading stops as soon as a whole group reaches `limit`. The caller passes the
//! number of candidates it will consume, which is `k` for a plain search and the
//! over-sampled count of `crate::group::candidates_per_branch` for a regrouped
//! one, so the regrouping still reads the keys it consumes beyond `k`
//! (ADR-0075). `N` key reads become about `k` plus the ties of the boundary.
//!
//! **Nesting rule.** As with [`ExactSearcher::top_k`], a caller must not hold a
//! connection of the searcher's read pool when it calls this method: the key
//! reads would wait for it, and the pool would run out of connections. The
//! rescoring holds none itself, and every vector read is finished before the
//! first key read.
//!
//! # Parallelism and cost
//!
//! The loop is sequential. `N` is bounded by `search.rescore_candidates` (5 000
//! by default, ADR-0078), so it is a few thousand products plus the random reads
//! of the retained vectors, while the scan it follows has already walked
//! millions of codes. MES-03 measures that step; a parallel loop is added only
//! if the measurement asks for one, and it would not change the answer, since
//! the ranking is total and built after the loop.

use std::ops::Range;

use crible_core::error::CribleError;
use crible_core::ids::Iid;

use super::binary::Candidate;
use super::exact::ExactSearcher;
use super::{Scored, cosine, internal};

impl ExactSearcher {
    /// Scores the `candidates` of the binary scan and returns them best first,
    /// by descending cosine and, at equal score, by ascending key `doc_id#ord`.
    ///
    /// `candidates` is the result of one scan of the pool
    /// ([`binary::top_k`](super::binary::top_k)): distinct identifiers, taken
    /// from the generation this searcher opened. The Hamming distance they
    /// carry is ignored: it ranked the candidates, it does not score them.
    ///
    /// `limit` is the number of candidates the caller consumes: the return value
    /// holds at least `limit` candidates when the list has them, and may hold
    /// more, because the group of equal score that straddles `limit` is returned
    /// whole (see the module documentation). The caller truncates it to `limit`,
    /// and the keys of that group are ordered, so its truncation keeps the
    /// smallest keys. A `limit` of `0` returns no result and reads no key.
    ///
    /// The query is validated and normalised exactly as the exact strategy does
    /// it, before an empty candidate list short-circuits: a malformed query is
    /// refused whatever the scan kept, and a query with a zero norm has no
    /// cosine to compute. An empty list returns no result and no error, which is
    /// the state of a pool the filter emptied or of a generation without a
    /// segment.
    ///
    /// A stored vector that produces a non-finite cosine stops the rescoring
    /// with an [`ErrorCode::Internal`](crible_core::error::ErrorCode::Internal)
    /// error, as in the exact strategy: the vectors are written normalised and
    /// their checksums are verified when a segment opens, so a non-finite score
    /// means the stored data contradicts the format, not a bad request.
    ///
    /// A candidate whose passage row is absent is an `internal` error, here and
    /// in the exact strategy: a passage of the pool always has a row. The
    /// rescoring only meets that case for a candidate it reads the key of, which
    /// is the economy the exact strategy already practises with its heap
    /// (`exact.rs`, `score_slice`). A document row may be absent, which is a
    /// normal state of the database and is handled by the payload step
    /// (ADR-0043).
    pub fn rescore(
        &self,
        query: &[f32],
        candidates: &[Candidate],
        limit: usize,
    ) -> Result<Vec<Scored>, CribleError> {
        let empty = candidates.is_empty();
        let query = self.normalised_query(query, empty)?;
        if empty || limit == 0 {
            return Ok(Vec::new());
        }

        // The candidates are visited in the order the scan returned them, that
        // is by ascending Hamming distance: visiting them by identifier was
        // measured as a loss at both N (ADR-0083, decision 2) and is gone. The
        // visit order reaches nothing but the order of this vector, since the
        // ranking below is total.
        let mut scored: Vec<(Iid, f32)> = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let iid = candidate.iid;
            let score = cosine(self.reader(iid)?, iid, &query)?;
            if !score.is_finite() {
                return Err(internal(
                    "a stored vector produces a non-finite cosine, so the ranking is undefined",
                ));
            }
            scored.push((iid, score));
        }

        // Intermediate order, before any key is read: descending score, then
        // ascending identifier. The identifier only breaks ties here, and only
        // so that the order is total; the final order compares the keys.
        scored.sort_unstable_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| left.0.cmp(&right.0))
        });

        let runs = equal_score_runs(&scored);
        let wanted = runs_to_read(&runs, limit);
        let mut kept: Vec<Scored> = Vec::with_capacity(limit.min(scored.len()));
        for run in runs.iter().take(wanted) {
            let mut group: Vec<Scored> = Vec::with_capacity(run.len());
            for (iid, score) in &scored[run.clone()] {
                group.push(Scored {
                    iid: *iid,
                    key: self.key_of(*iid)?,
                    score: *score,
                });
            }
            group.sort_unstable_by(|left, right| right.cmp(left));
            kept.extend(group);
        }
        Ok(kept)
    }
}

/// Cuts a list of scored candidates, ordered by descending score, into the runs
/// of candidates that share the same score.
///
/// The runs are contiguous and cover the whole list, in order. They are what the
/// rescoring reads the keys by: the whole run is needed once its score reaches
/// the caller's `limit`, because only the keys order its members.
///
/// The comparison is [`f32::total_cmp`], the one [`Scored`] ranks with, so two
/// scores are in the same run exactly when the final order considers them equal
/// (`-0.0` and `0.0` are two different runs, as they are two different rankings).
fn equal_score_runs(scored: &[(Iid, f32)]) -> Vec<Range<usize>> {
    let mut runs: Vec<Range<usize>> = Vec::new();
    let mut start = 0_usize;
    for position in 1..scored.len() {
        if scored[position].1.total_cmp(&scored[start].1) != std::cmp::Ordering::Equal {
            runs.push(start..position);
            start = position;
        }
    }
    if !scored.is_empty() {
        runs.push(start..scored.len());
    }
    runs
}

/// Number of leading runs of `runs` to read so that the caller's `limit` is
/// covered.
///
/// The answer is the smallest count whose runs hold `limit` candidates, capped
/// at the number of runs, so a `limit` above the list reads everything and a
/// `limit` of `0` reads nothing. A run that straddles the limit is counted whole:
/// its keys are what decides which of its members the caller keeps.
fn runs_to_read(runs: &[Range<usize>], limit: usize) -> usize {
    if limit == 0 {
        return 0;
    }
    let mut covered = 0_usize;
    for (position, run) in runs.iter().enumerate() {
        covered += run.len();
        if covered >= limit {
            return position + 1;
        }
    }
    runs.len()
}

#[cfg(test)]
mod tests {
    use crible_core::ids::Iid;

    use super::super::Scored;
    use super::{equal_score_runs, runs_to_read};

    /// The order the rescoring sorts with is total, and a tie is broken by the
    /// textual key, not by the internal identifier.
    ///
    /// `doc#10` sorts **before** `doc#2` as text, while the `Ord` of
    /// [`crible_core::ids::PassageKey`] would order them the other way: the test
    /// pins the textual order that `tools/reference_topk.py` and the exact
    /// strategy use (ADR-0072, decision 3).
    #[test]
    fn a_tie_is_broken_by_the_ascending_textual_key() {
        let mut scored = [
            Scored {
                iid: Iid::new(2),
                key: "doc#2".to_owned(),
                score: 0.5,
            },
            Scored {
                iid: Iid::new(10),
                key: "doc#10".to_owned(),
                score: 0.5,
            },
            Scored {
                iid: Iid::new(4),
                key: "doc#0".to_owned(),
                score: 0.75,
            },
        ];
        scored.sort_unstable_by(|left, right| right.cmp(left));

        let keys: Vec<&str> = scored
            .iter()
            .map(|candidate| candidate.key.as_str())
            .collect();
        assert_eq!(
            keys,
            vec!["doc#0", "doc#10", "doc#2"],
            "descending score first, then the ascending textual key"
        );
    }

    /// One run per score value, in order, covering the whole list.
    #[test]
    fn equal_scores_are_grouped_into_runs() {
        let scored = [
            (Iid::new(4), 5.0_f32),
            (Iid::new(1), 3.0),
            (Iid::new(7), 3.0),
            (Iid::new(2), 3.0),
            (Iid::new(0), 1.5),
            (Iid::new(9), 1.5),
        ];
        let runs = equal_score_runs(&scored);
        assert_eq!(
            runs,
            vec![0..1, 1..4, 4..6],
            "the runs must follow the score and cover the whole list"
        );
        for run in &runs {
            let score = scored[run.start].1;
            assert!(
                scored[run.clone()].iter().all(|entry| entry.1 == score),
                "a run must hold one score only"
            );
        }
    }

    /// The boundaries of the grouping: an empty list, a list where every score
    /// differs, and a list where every score ties.
    #[test]
    fn equal_score_runs_of_degenerate_lists() {
        assert!(equal_score_runs(&[]).is_empty(), "an empty list has no run");

        let distinct = [
            (Iid::new(0), 4.0_f32),
            (Iid::new(1), 3.0),
            (Iid::new(2), 2.0),
        ];
        assert_eq!(
            equal_score_runs(&distinct),
            vec![0..1, 1..2, 2..3],
            "distinct scores give one run per candidate"
        );

        let tied = [
            (Iid::new(0), 2.0_f32),
            (Iid::new(1), 2.0),
            (Iid::new(2), 2.0),
        ];
        assert_eq!(
            equal_score_runs(&tied),
            vec![0..3],
            "a list of ex aequo is a single run"
        );

        // `-0.0` and `0.0` are distinct for `total_cmp`, so they are distinct for
        // the final order too: the grouping must not merge them.
        let signed = [(Iid::new(0), 0.0_f32), (Iid::new(1), -0.0)];
        assert_eq!(
            equal_score_runs(&signed),
            vec![0..1, 1..2],
            "the grouping must use the comparison of the ranking, not the `==` of f32"
        );
    }

    /// The reading stops after the run that reaches `limit`, never inside it.
    #[test]
    fn runs_to_read_stops_after_the_run_that_reaches_the_limit() {
        let runs = vec![0..1, 1..4, 4..6];
        assert_eq!(runs_to_read(&runs, 0), 0, "a zero limit reads nothing");
        assert_eq!(
            runs_to_read(&runs, 1),
            1,
            "a limit met by the first run reads one run"
        );
        assert_eq!(
            runs_to_read(&runs, 2),
            2,
            "a limit inside the second run reads that run whole"
        );
        assert_eq!(
            runs_to_read(&runs, 3),
            2,
            "a limit inside the second run reads that run whole"
        );
        assert_eq!(
            runs_to_read(&runs, 4),
            2,
            "a limit exactly at the end of a run does not read the next one"
        );
        assert_eq!(
            runs_to_read(&runs, 6),
            3,
            "a limit above the list reads every run"
        );
        assert_eq!(
            runs_to_read(&runs, 99),
            3,
            "a limit above the list reads every run, and reads no more"
        );
        assert_eq!(runs_to_read(&[], 3), 0, "an empty list has nothing to read");
    }
}
