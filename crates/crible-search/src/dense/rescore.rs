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
//! The scan that precedes the rescoring has already walked millions of codes;
//! `N` is bounded by `search.rescore_candidates` (5 000 by default, ADR-0078).
//! What the rescoring pays is therefore not the few thousand products but the
//! **page faults** of the reference vectors: MES-03 attributes 85,0 ms at
//! N = 1000 and 177,5 ms at N = 5000 to the dot product on pages that are not
//! resident, against 0,36 ms and 2,04 ms on resident pages, at about one page
//! per candidate (`docs/mesures/MES-03-latence-rescoring.md`).
//!
//! The access to the vectors is therefore **grouped and parallel** (ADR-0080,
//! decision 2): the `N` rows of `f16` components are read into a transient
//! buffer, by chunks, on the rayon pool of the caller (the
//! [`ScanPool`](crate::scan_pool::ScanPool) in production, never a pool of its
//! own), **before** any score is computed. The point is not to read fewer pages,
//! which grouping cannot do, but to keep several page faults in flight at once:
//! the workers of the pool each block on a different row instead of the single
//! in-order loop paying them one after the other.
//!
//! The buffer holds at most `N × dims × 2` bytes (7,7 MB at N = 5000 and 768
//! dimensions), is allocated per request and dropped by the time the ranking
//! begins: it is not cached between requests, and it is not part of the memory
//! an index owns (MES-09). The calculation itself is unchanged and stays
//! sequential: every row of the buffer goes through the **same**
//! `kernels::dot`, one vector at a time, and the ranking is the same total order
//! built afterwards, so the answer keeps the bits of the exact strategy
//! (ADR-0072, decision 3).
//!
//! An `fp32` reference has no grouped path: its rows are wider, its calculation
//! is already scalar, and ADR-0080 only decides the `f16` access. Those segments
//! keep the per-candidate read of [`cosine`].

use std::ops::Range;

use crible_core::config::ReferencePrecision;
use crible_core::error::CribleError;
use crible_core::ids::Iid;
use half::f16;
use rayon::prelude::*;

use super::binary::Candidate;
use super::exact::ExactSearcher;
use super::{Scored, cosine, internal};
use crate::kernels;

/// Number of rows a worker reads before claiming the next chunk.
///
/// The depth ADR-0080 assumes is that of the pool, about 8 faults in flight:
/// each worker blocks on the row it reads, so the width of a chunk does not add
/// concurrency by itself. A chunk of 8 rows keeps the tasks small enough that
/// every worker of the default pool claims one at once and reaches its first
/// page fault without waiting for the queue to drain, while amortising the
/// per-task cost of rayon (about 100 ns) over 8 rows. A smaller chunk adds
/// scheduling for no extra depth; a much larger one leaves the tail of the
/// candidate list to a single worker. The value is a constant, not a setting:
/// ADR-0080 asks the measurement to decide whether the grouped read is kept, not
/// to expose its chunk width.
const VECTOR_READ_BATCH: usize = 8;

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
        // ranking below is total. Every vector read happens here, before the
        // first key read (nesting rule, ADR-0079), and the transient buffer of
        // the grouped read is dropped before the ranking begins.
        //
        // Every segment of a generation carries the same reference precision
        // (an index-level setting), so the segment of the first candidate
        // decides the read path for the whole list. A segment that disagreed
        // would be refused by `vector_f16_into` as an internal contradiction.
        let first = self.reader(candidates[0].iid)?.meta();
        let dims = first.dims as usize;
        let precision = first.precision;
        let mut scored: Vec<(Iid, f32)> = match precision {
            ReferencePrecision::Fp16 => self.score_f16(&query, candidates, dims)?,
            ReferencePrecision::Fp32 => {
                let mut scored: Vec<(Iid, f32)> = Vec::with_capacity(candidates.len());
                for candidate in candidates {
                    let iid = candidate.iid;
                    scored.push((iid, finite(cosine(self.reader(iid)?, iid, &query)?)?));
                }
                scored
            }
        };
        debug_assert_eq!(
            scored.len(),
            candidates.len(),
            "the ranking must score every candidate"
        );

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

    /// Scores the `candidates` of an `f16` reference, reading their rows
    /// grouped and in parallel first, then computing one dot product per row.
    ///
    /// The rows are read by `read_f16_rows` on the rayon pool of the caller,
    /// into a buffer local to this method: it holds the `N` rows at once, is
    /// filled before the first score, and is dropped before the ranking starts.
    /// The scoring then walks the candidates in their arrival order, one row at
    /// a time, through `kernels::dot`: the arithmetic, its order and its bits are
    /// the ones [`cosine`] gives for an `f16` reference (ADR-0072, decision 3).
    fn score_f16(
        &self,
        query: &[f32],
        candidates: &[Candidate],
        dims: usize,
    ) -> Result<Vec<(Iid, f32)>, CribleError> {
        let rows = read_f16_rows(candidates, dims, |iid, out| {
            self.reader(iid)?.vector_f16_into(iid, out)
        })?;
        let mut scored: Vec<(Iid, f32)> = Vec::with_capacity(candidates.len());
        for (candidate, row) in candidates.iter().zip(rows.chunks_exact(dims)) {
            scored.push((candidate.iid, finite(kernels::dot(row, query))?));
        }
        Ok(scored)
    }
}

/// Reads the `f16` rows of `candidates` into one transient buffer, in parallel.
///
/// `read` fills one row from its identifier; it is called once per candidate,
/// never twice, and may run on any worker of the rayon pool installed by the
/// caller. The candidates are cut into chunks of [`VECTOR_READ_BATCH`] rows, one
/// task per chunk, and each chunk fills its own slice of the buffer: the rows do
/// not move, so the scoring that follows reads them in the arrival order of the
/// candidates, whatever order the workers ran in.
///
/// The buffer holds `candidates.len() × dims` components, which is the bound
/// ADR-0080 gives (7,7 MB at N = 5000 and 768 dimensions). It is allocated
/// here and owned by the caller's result: nothing is cached between requests.
///
/// The first error of any worker stops the read and is returned; a partial
/// buffer has no use, since a candidate without its vector cannot be scored.
fn read_f16_rows<F>(candidates: &[Candidate], dims: usize, read: F) -> Result<Vec<f16>, CribleError>
where
    F: Fn(Iid, &mut [f16]) -> Result<(), CribleError> + Sync,
{
    if dims == 0 {
        return Err(internal(
            "a generation of zero dimensions holds no f16 row to read",
        ));
    }
    let total = candidates.len().checked_mul(dims).ok_or_else(|| {
        internal("the buffer of the grouped vector read overflows the address space")
    })?;
    let width = VECTOR_READ_BATCH
        .checked_mul(dims)
        .ok_or_else(|| internal("a chunk of the grouped vector read overflows its buffer"))?;
    let mut buffer: Vec<f16> = vec![f16::ZERO; total];
    let chunks: Vec<&[Candidate]> = candidates.chunks(VECTOR_READ_BATCH).collect();
    chunks
        .par_iter()
        .zip(buffer.par_chunks_mut(width))
        .try_for_each(|(chunk, slots)| -> Result<(), CribleError> {
            for (position, candidate) in chunk.iter().enumerate() {
                let start = position * dims;
                let end = start + dims;
                let row = slots.get_mut(start..end).ok_or_else(|| {
                    internal("a chunk of the grouped vector read falls outside its buffer")
                })?;
                read(candidate.iid, row)?;
            }
            Ok(())
        })?;
    Ok(buffer)
}

/// Refuses a cosine that is not a number.
///
/// The vectors are written normalised and their checksums are verified when a
/// segment opens, so a non-finite score means the stored data contradicts the
/// format, not a bad request: it stops the rescoring instead of letting
/// [`f32::total_cmp`] rank a NaN above every number.
fn finite(score: f32) -> Result<f32, CribleError> {
    if score.is_finite() {
        return Ok(score);
    }
    Err(internal(
        "a stored vector produces a non-finite cosine, so the ranking is undefined",
    ))
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
    use crible_core::error::ErrorCode;
    use crible_core::ids::Iid;
    use half::f16;

    use super::super::Scored;
    use super::super::binary::Candidate;
    use super::{VECTOR_READ_BATCH, equal_score_runs, internal, read_f16_rows, runs_to_read};

    /// Candidates for the identifiers `0..count`, in arrival order.
    fn candidates(count: u32) -> Vec<Candidate> {
        (0..count)
            .map(|iid| Candidate {
                iid: Iid::new(iid),
                distance: u64::from(iid),
            })
            .collect()
    }

    /// Row the tests expect for `iid`: one component per dimension, derived from
    /// the identifier so a misplaced row is visible.
    fn expected_row(iid: Iid, dims: usize) -> Vec<f16> {
        (0..dims)
            .map(|component| f16::from_f32((iid.get() * 10) as f32 + component as f32))
            .collect()
    }

    /// A grouped read of a list that does not fill its last chunk.
    ///
    /// Eleven candidates are one chunk of eight and a tail of three: the tail
    /// must fill the last three rows of the same buffer, and no row may shift.
    #[test]
    fn a_grouped_read_fills_an_incomplete_last_chunk() {
        let dims = 3;
        let candidates = candidates(2 * VECTOR_READ_BATCH as u32 - 5);
        assert_eq!(
            candidates.len(),
            11,
            "the fixture must leave a tail of three"
        );
        let rows = read_f16_rows(&candidates, dims, |iid, out| {
            out.copy_from_slice(&expected_row(iid, dims));
            Ok(())
        })
        .expect("the grouped read must succeed");

        assert_eq!(
            rows.len(),
            candidates.len() * dims,
            "the buffer must hold one row per candidate and no more"
        );
        for (position, candidate) in candidates.iter().enumerate() {
            let row = &rows[position * dims..(position + 1) * dims];
            assert_eq!(
                row,
                expected_row(candidate.iid, dims).as_slice(),
                "row {position} must be the row of iid {}",
                candidate.iid.get()
            );
        }
    }

    /// A list shorter than one chunk is a single task, filled in place.
    #[test]
    fn a_grouped_read_of_a_single_chunk_keeps_the_arrival_order() {
        let dims = 2;
        let candidates = candidates(VECTOR_READ_BATCH as u32 - 5);
        let rows = read_f16_rows(&candidates, dims, |iid, out| {
            out.copy_from_slice(&expected_row(iid, dims));
            Ok(())
        })
        .expect("the grouped read must succeed");

        assert_eq!(rows.len(), candidates.len() * dims);
        let flattened: Vec<u32> = rows
            .iter()
            .map(|component| component.to_f32() as u32)
            .collect();
        let expected: Vec<u32> = candidates
            .iter()
            .flat_map(|candidate| expected_row(candidate.iid, dims))
            .map(|component| component.to_f32() as u32)
            .collect();
        assert_eq!(
            flattened, expected,
            "a single chunk must keep the rows in the arrival order of the candidates"
        );
    }

    /// Candidates of several segments are read from the segment that holds each
    /// one, and the buffer keeps them in arrival order even when the chunks
    /// straddle a boundary.
    ///
    /// The fixture takes the identifier ranges of the five unequal segments of
    /// the criterion (1, 7, 64, 65 and 1 000 passages), with gaps between them so
    /// that the list walks four of them in a few chunks.
    #[test]
    fn a_grouped_read_serves_candidates_of_several_segments() {
        let dims = 1;
        let starts = [0_u32, 1, 8, 72, 137];
        let candidates: Vec<Candidate> = starts
            .iter()
            .flat_map(|&start| {
                (start..start + 3).map(|iid| Candidate {
                    iid: Iid::new(iid),
                    distance: u64::from(iid),
                })
            })
            .collect();
        let segment_of = |iid: Iid| starts.iter().filter(|&&start| start <= iid.get()).count();

        let rows = read_f16_rows(&candidates, dims, |iid, out| {
            // The "segment" writes its own index, so a row read from the wrong
            // segment is visible.
            out[0] = f16::from_f32(segment_of(iid) as f32);
            Ok(())
        })
        .expect("the grouped read must succeed");

        let served: Vec<u32> = rows.iter().map(|row| row.to_f32() as u32).collect();
        let expected: Vec<u32> = candidates
            .iter()
            .map(|candidate| segment_of(candidate.iid) as u32)
            .collect();
        assert_eq!(
            served, expected,
            "every row must come from the segment that holds its candidate"
        );
        assert!(
            expected.windows(2).any(|pair| pair[0] != pair[1]),
            "the fixture must really span several segments"
        );
    }

    /// The buffer is exactly `candidates × dims` components: empty for an empty
    /// list, one row for one candidate.
    #[test]
    fn the_grouped_buffer_holds_exactly_one_row_per_candidate() {
        let dims = 768;
        assert!(
            read_f16_rows(&[], dims, |_, _| Ok(()))
                .expect("an empty read must succeed")
                .is_empty(),
            "an empty candidate list allocates nothing"
        );

        let one = candidates(1);
        let rows = read_f16_rows(&one, dims, |iid, out| {
            assert_eq!(out.len(), dims, "a row holds one component per dimension");
            out.fill(f16::from_f32(iid.get() as f32));
            Ok(())
        })
        .expect("the grouped read must succeed");
        assert_eq!(rows.len(), dims, "one candidate is one row");
    }

    /// A read that fails stops the grouped read and returns its error, rather
    /// than a buffer with a missing row.
    #[test]
    fn a_grouped_read_propagates_the_error_of_a_row() {
        let candidates = candidates(2 * VECTOR_READ_BATCH as u32);
        let result = read_f16_rows(&candidates, 2, |iid, _| {
            if iid.get() == 11 {
                return Err(internal("the row of iid 11 is unreadable"));
            }
            Ok(())
        });
        let error = result.expect_err("the grouped read must fail");
        assert_eq!(error.code(), ErrorCode::Internal);
    }

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
