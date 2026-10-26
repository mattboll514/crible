// SPDX-License-Identifier: Apache-2.0

//! Binary scan of the pool: the top-N passages by Hamming distance.
//!
//! References: `docs/phases/P4-recherche-rapide.md` (`SCANNER-POOL-BINAIRE`),
//! `docs/03-recherche.md` ("Branche dense", strategy BINARY and "Parallélisme"),
//! ADR-0004 (bit `i` is 1 when `x_i > 0`), ADR-0069 (the `codes.bin` layout),
//! ADR-0070 (the runtime dispatch of the kernels) and ADR-0071 (the threshold
//! between the dense and the sparse traversal).
//!
//! # What the scan computes
//!
//! A binary code is `ceil(dims / 64)` little-endian words. The distance between
//! the query code and the code of a passage is the Hamming distance, that is
//! the popcount of their exclusive or. [`top_k`] returns the `n` passages of the
//! pool with the smallest distance, sorted by ascending distance and, at equal
//! distance, by ascending internal identifier. That order is total, so the
//! answer never depends on the partitioning of the scan.
//!
//! The codes are read from [`Snapshot::codes_of`], the copy the snapshot holds
//! in RAM (`QUANTIFIER-BINAIRE`), never from a segment reader: a second copy
//! would cost 0.82 GB at the scale of Legifacile.
//!
//! # Dense or sparse traversal
//!
//! `docs/03-recherche.md` describes both traversals but fixes no threshold
//! between them: the `exact_threshold` of the strategy table selects EXACT
//! against BINARY, not one traversal of the binary scan against the other.
//! ADR-0071 fixes that threshold, with no setting:
//!
//! - **dense** when the pool covers at least **half** of the identifier range it
//!   spans, that is `2 × pool_len ≥ last − first + 1`: the scan walks the
//!   contiguous codes and tests the pool bitmap for each identifier of that
//!   range;
//! - **sparse** otherwise: the scan walks the pool bitmap and reads the code of
//!   each live identifier.
//!
//! The decision is taken once per scan, on the whole pool, then applied to every
//! window (ADR-0071). The ratio is a cost heuristic, not a measured optimum:
//! MES-02 is the measure that could replace it, through a new ADR. The choice
//! changes the cost only, never the result; both traversals are compared to a
//! full sort in the tests.
//!
//! # Parallelism and the kernel
//!
//! The pool is cut into contiguous windows of internal identifiers, one per
//! rayon worker (`crate::dense::windows`, the same partitioning as the exact
//! strategy). Each window keeps its own bounded heap and the heaps are merged
//! at the end. The Hamming implementation is resolved **once** per scan, through
//! [`crate::kernels::hamming_function`]: the runtime dispatch would otherwise
//! cost about 40 % of a 12-word Hamming, once per passage (`NOYAUX-SIMD`,
//! `benches/kernels.rs`).
//!
//! # Malformed inputs
//!
//! The query code must carry exactly `ceil(dims / 64)` words: the kernels apply
//! a common-prefix rule for slices of different lengths, so a shorter or longer
//! code would silently compare a truncated or padded distance
//! ([`ErrorCode::InvalidRequest`]). A segment whose code array does not hold
//! `count × words` words is a corruption of the epoch
//! ([`ErrorCode::Internal`]), as is a pool identifier that no segment covers.

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::Iid;
use crible_store::codes::words_per_code;
use crible_store::generation::Snapshot;
use rayon::prelude::*;
use roaring::RoaringBitmap;

use super::windows;
use crate::kernels;
use crate::pool::Pool;

/// One candidate of the binary scan: a passage and its Hamming distance.
///
/// The order is the one of the answer: ascending distance, then ascending
/// internal identifier. A candidate never leaves the engine: the rescoring
/// step (`RESCORER-FP16`) turns it into a scored passage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    /// Internal identifier of the passage.
    pub iid: Iid,
    /// Number of bits that differ between the query code and the passage code.
    pub distance: u64,
}

impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        // Greater is better: a smaller distance wins, and at equal distance a
        // smaller identifier wins. A window wraps its heap in `Reverse`, so the
        // top of the heap is the candidate that `pop` removes first.
        self.distance
            .cmp(&other.distance)
            .reverse()
            .then_with(|| other.iid.get().cmp(&self.iid.get()))
    }
}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Returns the `n` passages of `pool` with the smallest Hamming distance to
/// `query_code`, sorted by ascending distance then ascending internal
/// identifier.
///
/// `query_code` holds the code of the query, quantized as a passage code is
/// (bit `i` is 1 when `x_i > 0`, ADR-0004). An empty pool, or `n == 0`, returns
/// no candidate and no error. An `n` above the pool size returns the whole
/// pool. A generation without a segment has no dimensionality to compare
/// against; its pool is necessarily empty, so the call returns no candidate.
///
/// The scan runs on the rayon pool installed by the caller
/// ([`crate::scan_pool::ScanPool`] in production), never on the tokio threads.
pub fn top_k(
    snapshot: &Snapshot,
    pool: &Pool,
    query_code: &[u64],
    n: usize,
) -> Result<Vec<Candidate>, CribleError> {
    let segments = prepare(snapshot, pool, query_code)?;
    if n == 0 || pool.is_empty() {
        return Ok(Vec::new());
    }
    let traversal = traversal(pool.bitmap(), pool.len());
    scan(
        &segments,
        pool.bitmap(),
        pool.len(),
        query_code,
        n,
        traversal,
    )
}

/// Binary codes of one segment, checked once per scan.
#[derive(Debug)]
struct SegmentCodes<'a> {
    /// Internal identifier of the first code.
    iid_start: u32,
    /// Number of codes.
    count: u32,
    /// Codes of the segment, `count × words` words, in identifier order.
    codes: &'a [u64],
}

impl SegmentCodes<'_> {
    /// First identifier the segment does **not** hold.
    fn end(&self) -> u64 {
        u64::from(self.iid_start) + u64::from(self.count)
    }

    /// Code of `iid`, or `None` when the segment does not hold it.
    fn code(&self, iid: u32, words: usize) -> Option<&[u64]> {
        let offset = usize::try_from(iid.checked_sub(self.iid_start)?).ok()?;
        let begin = offset.checked_mul(words)?;
        let end = begin.checked_add(words)?;
        self.codes.get(begin..end)
    }
}

/// Reads and checks the code arrays of the generation.
///
/// The query length is checked even when the pool is empty, as the exact
/// strategy does: a malformed request is refused whatever the filter keeps.
fn prepare<'a>(
    snapshot: &'a Snapshot,
    pool: &Pool,
    query_code: &[u64],
) -> Result<Vec<SegmentCodes<'a>>, CribleError> {
    let metas = snapshot.segments();
    let Some(first) = metas.first() else {
        if pool.is_empty() {
            return Ok(Vec::new());
        }
        return Err(internal(
            "the generation holds no segment, yet its pool is not empty",
        ));
    };
    let words = words_per_code(first.dims);
    if words == 0 {
        return Err(internal(format!(
            "segment '{}' declares zero dimensions, so no binary code exists",
            first.segment_id
        )));
    }
    if query_code.len() != words {
        return Err(CribleError::new(
            ErrorCode::InvalidRequest,
            format!(
                "the query code holds {} words where the index holds {} dimensions ({} words)",
                query_code.len(),
                first.dims,
                words
            ),
        ));
    }
    let mut segments = Vec::with_capacity(metas.len());
    for meta in metas {
        if meta.dims != first.dims {
            return Err(internal(format!(
                "segment '{}' holds {} dimensions where segment '{}' holds {}",
                meta.segment_id, meta.dims, first.segment_id, first.dims
            )));
        }
        let codes = snapshot.codes_of(&meta.segment_id).ok_or_else(|| {
            internal(format!(
                "the snapshot holds no binary code for segment '{}'",
                meta.segment_id
            ))
        })?;
        let expected = usize::try_from(meta.count)
            .ok()
            .and_then(|count| count.checked_mul(words))
            .ok_or_else(|| {
                internal(format!(
                    "segment '{}' declares more binary codes than this platform can address",
                    meta.segment_id
                ))
            })?;
        if codes.len() != expected {
            return Err(internal(format!(
                "segment '{}' holds {} code words where {} dimensions and {} codes require {}",
                meta.segment_id,
                codes.len(),
                meta.dims,
                meta.count,
                expected
            )));
        }
        segments.push(SegmentCodes {
            iid_start: meta.iid_start.get(),
            count: meta.count,
            codes,
        });
    }
    Ok(segments)
}

/// Traversal of the pool: the contiguous codes, or the pool bitmap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Traversal {
    /// Walk the codes of the identifier range and test the pool bitmap.
    Dense,
    /// Walk the pool bitmap and read the code of each live identifier.
    Sparse,
}

/// Chooses the traversal of one scan.
///
/// The dense traversal pays one bitmap test per identifier of the window,
/// whether live or not, and reads the codes of the live identifiers in memory
/// order. The sparse one pays one bitmap step per live passage, plus one segment
/// lookup and one random code read per passage. Dense is therefore kept while
/// the pool covers at least half of the range it spans; below that, the number
/// of dead identifiers tested would exceed the number of codes read.
///
/// The threshold is the one ADR-0071 fixes, decided once per scan on the whole
/// pool: `2 × pool_len ≥ last − first + 1`. MES-02 is the measure that could
/// replace it, through a new ADR.
fn traversal(pool: &RoaringBitmap, pool_len: u32) -> Traversal {
    let (Some(first), Some(last)) = (pool.min(), pool.max()) else {
        return Traversal::Sparse;
    };
    let span = u64::from(last) - u64::from(first) + 1;
    if u64::from(pool_len) * 2 >= span {
        Traversal::Dense
    } else {
        Traversal::Sparse
    }
}

/// Runs the scan over the windows of the pool, then merges the window heaps.
fn scan(
    segments: &[SegmentCodes<'_>],
    pool: &RoaringBitmap,
    pool_len: u32,
    query_code: &[u64],
    n: usize,
    traversal: Traversal,
) -> Result<Vec<Candidate>, CribleError> {
    let scan = Scan {
        segments,
        words: query_code.len(),
        query_code,
        n,
        traversal,
        hamming: kernels::hamming_function(),
    };
    let ranges = windows(pool, pool_len, rayon::current_num_threads().max(1));
    let heaps: Vec<Vec<Candidate>> = ranges
        .par_iter()
        .map(|&(from, to)| scan.window(pool, from, to))
        .collect::<Result<Vec<_>, CribleError>>()?;

    let mut candidates: Vec<Candidate> = heaps.into_iter().flatten().collect();
    // Best first: `Candidate::cmp` orders a smaller distance as greater.
    candidates.sort_unstable_by(|left, right| right.cmp(left));
    candidates.truncate(n);
    Ok(candidates)
}

/// State shared by every window of one scan.
struct Scan<'a> {
    /// Codes of the generation, sorted by first identifier.
    segments: &'a [SegmentCodes<'a>],
    /// Words per code.
    words: usize,
    /// Code of the query.
    query_code: &'a [u64],
    /// Number of candidates kept per window.
    n: usize,
    /// Traversal chosen for the whole scan.
    traversal: Traversal,
    /// Hamming implementation, resolved once for the scan.
    hamming: fn(&[u64], &[u64]) -> u64,
}

impl<'a> Scan<'a> {
    /// Scans one window `from..=to` and returns its best `n` candidates.
    fn window(
        &self,
        pool: &RoaringBitmap,
        from: u32,
        to: u32,
    ) -> Result<Vec<Candidate>, CribleError> {
        let mut heap = BinaryHeap::with_capacity(
            self.n
                .min(usize::try_from(pool.len()).unwrap_or(usize::MAX)),
        );
        match self.traversal {
            Traversal::Dense => self.dense_window(pool, from, to, &mut heap)?,
            Traversal::Sparse => self.sparse_window(pool, from, to, &mut heap)?,
        }
        Ok(heap
            .into_iter()
            .map(|Reverse(candidate)| candidate)
            .collect())
    }

    /// Walks the contiguous codes of the window and tests the pool bitmap.
    ///
    /// The number of live identifiers met is compared with the rank of the pool
    /// over the same range: a pool identifier that no segment covers would be
    /// skipped silently here, and the sparse traversal would refuse it.
    fn dense_window(
        &self,
        pool: &RoaringBitmap,
        from: u32,
        to: u32,
        heap: &mut BinaryHeap<Reverse<Candidate>>,
    ) -> Result<(), CribleError> {
        let mut live = 0_u64;
        for segment in self.segments {
            let start = u64::from(segment.iid_start);
            let first = start.max(u64::from(from));
            let last = segment.end().min(u64::from(to) + 1);
            if first >= last {
                continue;
            }
            let span = usize::try_from(last - first).unwrap_or(usize::MAX);
            for position in 0..span {
                // `first + position < segment.end() <= 2^32`, so the cast is exact.
                let iid = (first + position as u64) as u32;
                if !pool.contains(iid) {
                    continue;
                }
                live += 1;
                let code = segment.code(iid, self.words).ok_or_else(|| {
                    internal("a live passage has no binary code in the generation")
                })?;
                offer(heap, self.distance(code), iid, self.n);
            }
        }
        let before = from.checked_sub(1).map_or(0, |value| pool.rank(value));
        let expected = pool.rank(to) - before;
        if live != expected {
            return Err(internal(
                "the pool holds an identifier that no segment of the generation covers",
            ));
        }
        Ok(())
    }

    /// Walks the pool bitmap and reads the code of each live identifier.
    fn sparse_window(
        &self,
        pool: &RoaringBitmap,
        from: u32,
        to: u32,
        heap: &mut BinaryHeap<Reverse<Candidate>>,
    ) -> Result<(), CribleError> {
        let mut live = pool.iter();
        live.advance_to(from);
        for iid in live {
            if iid > to {
                break;
            }
            let segment = self.segment_of(iid)?;
            let code = segment
                .code(iid, self.words)
                .ok_or_else(|| internal("a live passage has no binary code in the generation"))?;
            offer(heap, self.distance(code), iid, self.n);
        }
        Ok(())
    }

    /// Distance between the query code and one passage code.
    fn distance(&self, code: &[u64]) -> u64 {
        (self.hamming)(self.query_code, code)
    }

    /// Segment holding `iid`.
    fn segment_of(&self, iid: u32) -> Result<&'a SegmentCodes<'a>, CribleError> {
        let position = self
            .segments
            .partition_point(|segment| segment.iid_start <= iid);
        let segment = position
            .checked_sub(1)
            .and_then(|index| self.segments.get(index))
            .ok_or_else(|| {
                internal("the pool holds an identifier that no segment of the generation covers")
            })?;
        if u64::from(iid) >= segment.end() {
            return Err(internal(
                "the pool holds an identifier that no segment of the generation covers",
            ));
        }
        Ok(segment)
    }
}

/// Offers one candidate to a bounded max-heap of at most `n` candidates.
///
/// A candidate strictly worse than the current worst cannot enter, and is
/// dropped without a push. An equal candidate is offered: it may still win the
/// identifier tie-break, and the push-then-pop keeps exactly the same set either
/// way.
fn offer(heap: &mut BinaryHeap<Reverse<Candidate>>, distance: u64, iid: u32, n: usize) {
    let candidate = Candidate {
        iid: Iid::new(iid),
        distance,
    };
    if heap.len() == n
        && let Some(worst) = heap.peek()
        && candidate.cmp(&worst.0) == Ordering::Less
    {
        return;
    }
    heap.push(Reverse(candidate));
    if heap.len() > n {
        heap.pop();
    }
}

/// Error raised when the structures of the epoch contradict each other.
fn internal(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::Internal, message)
}

#[cfg(test)]
mod tests {
    use super::{Candidate, Scan, SegmentCodes, Traversal, scan, traversal};
    use roaring::RoaringBitmap;

    /// Deterministic generator, so a failing case is reproducible.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0
        }
    }

    /// Dimensions of the unit tests: three words per code.
    const DIMS: u32 = 130;

    /// Three segments of 100 codes each, filled with pseudo-random bits.
    fn segments() -> (Vec<u64>, Vec<u64>, Vec<u64>) {
        let mut generator = Lcg(7);
        let words = 3;
        let mut fill = || (0..300 * words).map(|_| generator.next()).collect();
        (fill(), fill(), fill())
    }

    fn build<'a>(a: &'a [u64], b: &'a [u64], c: &'a [u64]) -> Vec<SegmentCodes<'a>> {
        let words = super::words_per_code(DIMS);
        vec![
            SegmentCodes {
                iid_start: 0,
                count: 100,
                codes: &a[..100 * words],
            },
            SegmentCodes {
                iid_start: 100,
                count: 100,
                codes: &b[..100 * words],
            },
            SegmentCodes {
                iid_start: 200,
                count: 100,
                codes: &c[..100 * words],
            },
        ]
    }

    /// Two segments that do not tile the identifier space: `0..100` and
    /// `150..250`. Nothing covers `100..150`, a shape `Snapshot` does not
    /// forbid even though the store never produces it in normal operation.
    fn build_with_gap<'a>(a: &'a [u64], b: &'a [u64]) -> Vec<SegmentCodes<'a>> {
        let words = super::words_per_code(DIMS);
        vec![
            SegmentCodes {
                iid_start: 0,
                count: 100,
                codes: &a[..100 * words],
            },
            SegmentCodes {
                iid_start: 150,
                count: 100,
                codes: &b[..100 * words],
            },
        ]
    }

    /// Full sort by Hamming distance: the oracle of the two traversals.
    fn brute_force(
        segments: &[SegmentCodes<'_>],
        pool: &RoaringBitmap,
        query_code: &[u64],
        n: usize,
    ) -> Vec<Candidate> {
        let words = query_code.len();
        let mut candidates: Vec<Candidate> = pool
            .iter()
            .filter_map(|iid| {
                let segment = segments.iter().find(|segment| {
                    u64::from(iid) >= u64::from(segment.iid_start) && u64::from(iid) < segment.end()
                })?;
                let code = segment.code(iid, words)?;
                let distance = query_code
                    .iter()
                    .zip(code)
                    .map(|(left, right)| u64::from((left ^ right).count_ones()))
                    .sum();
                Some(Candidate {
                    iid: crible_core::ids::Iid::new(iid),
                    distance,
                })
            })
            .collect();
        candidates.sort_unstable_by(|left, right| right.cmp(left));
        candidates.truncate(n);
        candidates
    }

    /// Pools of every density: full, half, sparse and a single identifier.
    fn pools() -> Vec<RoaringBitmap> {
        let full: RoaringBitmap = (0..300).collect();
        let half: RoaringBitmap = (0..300).filter(|iid| iid % 2 == 0).collect();
        let sparse: RoaringBitmap = (0..300).filter(|iid| iid % 37 == 0).collect();
        let one: RoaringBitmap = std::iter::once(123).collect();
        vec![full, half, sparse, one, RoaringBitmap::new()]
    }

    #[test]
    fn dense_and_sparse_traversals_agree_with_a_full_sort() {
        let (a, b, c) = segments();
        let segments = build(&a, &b, &c);
        let mut generator = Lcg(11);
        let query_code: Vec<u64> = (0..3).map(|_| generator.next()).collect();

        for pool in pools() {
            let pool_len = u32::try_from(pool.len()).unwrap_or(u32::MAX);
            let expected = brute_force(&segments, &pool, &query_code, 10);
            for traversal in [Traversal::Dense, Traversal::Sparse] {
                let got = scan(&segments, &pool, pool_len, &query_code, 10, traversal)
                    .unwrap_or_else(|error| panic!("{traversal:?} must scan: {error}"));
                assert_eq!(
                    got, expected,
                    "{traversal:?} must return the top 10 of a full sort"
                );
            }
        }
    }

    #[test]
    fn a_pool_spanning_a_gap_between_segments_matches_a_full_sort() {
        let (a, b, _) = segments();
        let segments = build_with_gap(&a, &b);
        let query_code = [3_u64, 1, 2];

        // The first pool covers both sides of the gap, so its identifiers are
        // not contiguous; it is dense enough to be walked through the codes.
        let bridging: RoaringBitmap = (0..100).chain(150..250).collect();
        // The second one is sparse on both sides of the same gap.
        let sparse: RoaringBitmap = (0..100)
            .filter(|iid| iid % 10 == 0)
            .chain((150..250).filter(|iid| iid % 10 == 0))
            .collect();

        for (pool, chosen) in [(bridging, Traversal::Dense), (sparse, Traversal::Sparse)] {
            let pool_len = u32::try_from(pool.len()).unwrap_or(u32::MAX);
            assert_eq!(
                traversal(&pool, pool_len),
                chosen,
                "the case must exercise the {chosen:?} traversal"
            );
            let full = brute_force(&segments, &pool, &query_code, 10);
            for traversal in [Traversal::Dense, Traversal::Sparse] {
                let got = scan(&segments, &pool, pool_len, &query_code, 10, traversal)
                    .unwrap_or_else(|error| panic!("{traversal:?} must scan: {error}"));
                assert_eq!(
                    got, full,
                    "{traversal:?} must match the full sort across the gap"
                );
            }
        }
    }

    #[test]
    fn a_pool_identifier_outside_the_segments_is_refused() {
        let (a, b, _) = segments();
        let segments = build_with_gap(&a, &b);
        let query_code = [0_u64; 3];

        // `120` falls in the gap, `300` past the last segment: neither is
        // covered by a code, and the two traversals must refuse both the same
        // way rather than skip one silently.
        for pool in [
            std::iter::once(120_u32).collect::<RoaringBitmap>(),
            std::iter::once(300_u32).collect::<RoaringBitmap>(),
        ] {
            let pool_len = u32::try_from(pool.len()).unwrap_or(u32::MAX);
            for traversal in [Traversal::Dense, Traversal::Sparse] {
                let error = scan(&segments, &pool, pool_len, &query_code, 10, traversal)
                    .err()
                    .unwrap_or_else(|| {
                        panic!("{traversal:?} must refuse an identifier outside the segments")
                    });
                assert_eq!(
                    error.code(),
                    super::ErrorCode::Internal,
                    "{traversal:?} must report a corrupted generation"
                );
            }
        }
    }

    #[test]
    fn n_above_the_pool_returns_the_whole_pool() {
        let (a, b, c) = segments();
        let segments = build(&a, &b, &c);
        let query_code = [1_u64, 2, 3];
        let pool: RoaringBitmap = (0..300).filter(|iid| iid % 17 == 0).collect();
        let pool_len = u32::try_from(pool.len()).unwrap_or(u32::MAX);

        for traversal in [Traversal::Dense, Traversal::Sparse] {
            let none = scan(&segments, &pool, pool_len, &query_code, 0, traversal)
                .unwrap_or_else(|error| panic!("{traversal:?} must scan: {error}"));
            assert!(
                none.is_empty(),
                "{traversal:?} must return no candidate when n is zero"
            );
            let got = scan(&segments, &pool, pool_len, &query_code, 1_000, traversal)
                .unwrap_or_else(|error| panic!("{traversal:?} must scan: {error}"));
            assert_eq!(
                got.len(),
                usize::try_from(pool.len()).unwrap_or(usize::MAX),
                "{traversal:?} must return the whole pool when n exceeds it"
            );
            assert_eq!(
                got,
                brute_force(&segments, &pool, &query_code, 1_000),
                "{traversal:?} must match the full sort"
            );
        }
    }

    #[test]
    fn the_traversal_follows_the_density_of_the_pool() {
        let full: RoaringBitmap = (0..1_000).collect();
        assert_eq!(
            traversal(&full, 1_000),
            Traversal::Dense,
            "a full pool is walked through its contiguous codes"
        );

        let half: RoaringBitmap = (0..1_000).filter(|iid| iid % 2 == 0).collect();
        assert_eq!(
            traversal(&half, u32::try_from(half.len()).unwrap_or(u32::MAX)),
            Traversal::Dense,
            "half is the boundary of the dense traversal"
        );

        let sparse: RoaringBitmap = (0..1_000).filter(|iid| iid % 3 == 0).collect();
        assert_eq!(
            traversal(&sparse, u32::try_from(sparse.len()).unwrap_or(u32::MAX)),
            Traversal::Sparse,
            "a pool below half of its range is walked through its bitmap"
        );

        assert_eq!(
            traversal(&RoaringBitmap::new(), 0),
            Traversal::Sparse,
            "an empty pool has no range to walk"
        );
    }

    #[test]
    fn a_window_of_the_scan_keeps_at_most_n_candidates() {
        let (a, b, c) = segments();
        let segments = build(&a, &b, &c);
        let scanner = Scan {
            segments: &segments,
            words: 3,
            query_code: &[0, 0, 0],
            n: 3,
            traversal: Traversal::Sparse,
            hamming: crate::kernels::hamming_function(),
        };
        let pool: RoaringBitmap = (0..300).collect();
        let got = scanner
            .window(&pool, 0, 299)
            .unwrap_or_else(|error| panic!("the window must scan: {error}"));
        assert_eq!(got.len(), 3, "a window keeps at most n candidates");
    }
}
