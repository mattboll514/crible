// SPDX-License-Identifier: Apache-2.0

//! Dense branch of a search: the cosine between the query and the passages of
//! the pool.
//!
//! `docs/03-recherche.md` ("Branche dense") splits the branch by strategy: the
//! scan of the binary codes above the exact threshold ([`binary`]), the exact
//! dot product at or below it ([`exact`]), and the rescoring of the candidates
//! the scan kept ([`rescore`]). The binary scan has a single traversal, the sparse
//! walk of the pool bitmap with a segment cursor (ADR-0081, ADR-0082). The
//! planner chooses between the two strategies from the search settings, the
//! pool and the request option (ADR-0074), and runs the planned one: [`plan`]
//! gives the choice, [`search`] executes it.
//!
//! Both strategies parallelise the same way (`docs/03-recherche.md`,
//! "Parallélisme"): the pool is cut into contiguous windows of internal
//! identifiers, one per rayon worker, each window keeps its own bounded heap,
//! and the heaps are merged at the end. `windows` is that partitioning, shared
//! by the two so their windows can never diverge.
//!
//! # One calculation of the dot product
//!
//! ADR-0072 makes the score of a passage the result of **one** function,
//! `cosine`, called by the exact strategy and by the rescoring alike. A
//! reference stored as `f16` goes through the kernel of ADR-0070
//! (`crate::kernels::dot`, products in `f32`, accumulation in `f64`, a single
//! rounding to `f32`); a reference stored as `fp32` goes through
//! `fp32_dot`, products and accumulation in `f64`, one rounding to `f32`,
//! which is scalar because no SIMD `fp32 × fp32` kernel exists (ADR-0072,
//! decision 2). Two strategies that score the same passage therefore return the
//! same bits, on the same machine: that is what the acceptance criterion of
//! `RESCORER-FP16` checks when the rescoring keeps the whole pool.

pub mod binary;
pub mod exact;
mod planner;
pub mod rescore;

pub use planner::{DenseQuery, Plan, Strategy, candidates, plan, search, search_grouped};

use crible_core::config::ReferencePrecision;
use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::Iid;
use crible_store::segment::SegmentReader;
use roaring::RoaringBitmap;

use crate::kernels;

/// Cuts the pool into at most `threads` contiguous windows of about
/// `pool_size / threads` **live** passages.
///
/// A window is a range of internal identifiers, not a list of them: only the
/// bounds are kept, one entry per worker. The scan of a window walks the live
/// bitmap from its first identifier and skips the tombstoned ones, so a
/// fragmented pool (one live passage out of two, the normal state after
/// interleaved attaches) costs no extra window, heap or rayon task. This keeps
/// the work count bounded by the number of workers whatever the fragmentation,
/// while a window still covers only live passages. `exact: true` may cover the
/// whole index (`docs/03-recherche.md`), which is why the pool is never
/// materialised as a vector of identifiers.
///
/// The bounds are deterministic: they are read from the sorted live bitmap, and
/// the same pool always yields the same windows.
pub(crate) fn windows(bitmap: &RoaringBitmap, total: u32, threads: usize) -> Vec<(u32, u32)> {
    let total = usize::try_from(total).unwrap_or(usize::MAX);
    if total == 0 {
        return Vec::new();
    }
    let width = total.div_ceil(threads.max(1)).max(1);
    let mut starts = Vec::with_capacity(threads.max(1));
    let mut last = 0_u32;
    for (position, iid) in bitmap.iter().enumerate() {
        if position % width == 0 {
            starts.push(iid);
        }
        last = iid;
    }
    let mut windows: Vec<(u32, u32)> = Vec::with_capacity(starts.len());
    for (position, &from) in starts.iter().enumerate() {
        let to = match starts.get(position + 1) {
            Some(&next) => next - 1,
            None => last,
        };
        windows.push((from, to));
    }
    windows
}

/// One scored passage of the dense branch, ordered best first.
///
/// The order is the one of the answer: descending score, then ascending
/// **textual** key `doc_id#ord`. It is not the `Ord` of
/// [`crible_core::ids::PassageKey`], which compares `(doc_id, ord)` with `ord`
/// as a number; the two diverge from `ord = 10` on, and the text is the order
/// `tools/reference_topk.py` uses. A score is finite here: both strategies
/// refuse a non-finite cosine before a candidate enters a heap or a result, so
/// [`f32::total_cmp`] only provides the total order a heap requires.
///
/// Exactly one type carries that order, so the exact strategy and the
/// rescoring cannot drift apart on a tie: the comparator lives here, next to
/// the score it ranks (ADR-0072, decision 3).
#[derive(Debug, Clone)]
pub struct Scored {
    /// Internal identifier of the passage, kept until the payload step.
    pub iid: Iid,
    /// Textual key `doc_id#ord`, the tie-break.
    pub key: String,
    /// Cosine between the query and the passage.
    pub score: f32,
}

impl PartialEq for Scored {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

impl Eq for Scored {}

impl PartialOrd for Scored {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Scored {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Greater is better: a higher score wins, and at equal score the
        // smaller key wins. A window wraps its heap in `Reverse`, so the top of
        // the heap is the candidate that `pop` removes first.
        self.score
            .total_cmp(&other.score)
            .then_with(|| other.key.cmp(&self.key))
    }
}

/// Cosine between the normalised `query` and the reference vector of `iid`.
///
/// This is the only place the dense branch computes a score (ADR-0072,
/// decision 1). The vector is read from the memory map of the segment, in the
/// precision the segment stores, and handed as is to the matching calculation:
///
/// - `fp16` reference: [`crate::kernels::dot`], the runtime-dispatched kernel of
///   ADR-0070, which widens each component itself;
/// - `fp32` reference: [`fp32_dot`], the scalar calculation of ADR-0072.
///
/// `query` must already be normalised and must hold one component per dimension
/// of the index; both callers validate it before the first score. A segment
/// whose code array and vector file disagree is not detected here: the reader
/// opened in [`exact::DenseSearcher::open`] checked the row length.
pub(crate) fn cosine(reader: &SegmentReader, iid: Iid, query: &[f32]) -> Result<f32, CribleError> {
    match reader.meta().precision {
        ReferencePrecision::Fp16 => {
            let vector = reader.vector_f16(iid)?;
            Ok(kernels::dot(&vector, query))
        }
        ReferencePrecision::Fp32 => {
            let vector = reader.vector(iid)?;
            Ok(fp32_dot(&vector, query))
        }
    }
}

/// Dot product of a stored `fp32` vector and a normalised `f32` query.
///
/// ADR-0072, decision 2: the products and the accumulation are computed in
/// `f64`, and the result is rounded once, to `f32`, at the end. The products of
/// two `f32` values are exact in `f64`, so only the summation orders the result,
/// and it orders it the same way for the exact strategy and for the rescoring.
/// The loop reads `min(stored.len(), query.len())` components, the common-prefix
/// rule of the kernels.
///
/// The function is scalar on purpose: no SIMD `fp32 × fp32` kernel is written
/// until a measurement asks for one, and `fp32` is not the default reference
/// precision (`docs/03-recherche.md`).
fn fp32_dot(stored: &[f32], query: &[f32]) -> f32 {
    let mut sum = 0.0_f64;
    for (stored, query) in stored.iter().zip(query) {
        sum += f64::from(*stored) * f64::from(*query);
    }
    sum as f32
}

/// Divides `query` by its Euclidean norm in `f32`, as the numpy reference does.
///
/// The query comes from the caller, so a non-finite component, or finite
/// components whose squares overflow `f32`, is an invalid request: the norm
/// would be infinite and every component would become zero in silence, scoring
/// the whole pool at zero. A zero vector has no cosine either, so it is refused
/// rather than scoring every passage at zero, which would be meaningless.
///
/// Both strategies normalise through this function, so a passage receives the
/// same query bits from either of them.
pub(crate) fn normalise(query: &[f32]) -> Result<Vec<f32>, CribleError> {
    let mut squares = 0.0_f32;
    for component in query {
        if !component.is_finite() {
            return Err(invalid_request(
                "the query vector holds a non-finite component",
            ));
        }
        squares += component * component;
    }
    let norm = squares.sqrt();
    if !norm.is_finite() {
        return Err(invalid_request(
            "the squared norm of the query vector overflows f32",
        ));
    }
    if norm == 0.0 {
        return Err(invalid_request(
            "the query vector has a zero norm, so no cosine is defined",
        ));
    }
    Ok(query.iter().map(|component| component / norm).collect())
}

/// Error raised when the structures of the epoch contradict each other.
pub(crate) fn internal(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::Internal, message)
}

/// Error raised when the request itself is malformed.
pub(crate) fn invalid_request(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::InvalidRequest, message)
}

#[cfg(test)]
mod tests {
    use roaring::RoaringBitmap;

    use super::{cosine, fp32_dot, windows};

    /// A fragmented pool: every live identifier is isolated, as after
    /// interleaved attaches or a filter that keeps one passage out of two.
    fn fragmented(live: u32) -> RoaringBitmap {
        (0..live).map(|position| position * 2).collect()
    }

    #[test]
    fn windows_are_bounded_by_the_worker_count_on_a_fragmented_pool() {
        let bitmap = fragmented(1_000);
        let total = u32::try_from(bitmap.len()).unwrap_or(u32::MAX);
        let windows = windows(&bitmap, total, 4);

        assert_eq!(
            windows.len(),
            4,
            "four workers must give four windows, not one per run of the bitmap"
        );
        for pair in windows.windows(2) {
            assert!(
                pair[0].1 < pair[1].0,
                "the windows must be ordered and disjoint"
            );
        }
        for (from, to) in &windows {
            let covered = bitmap
                .iter()
                .filter(|&iid| *from <= iid && iid <= *to)
                .count();
            assert_eq!(
                covered, 250,
                "every window must hold about pool_size / workers live passages"
            );
        }
        let mut seen = 0_usize;
        for iid in bitmap.iter() {
            let owners = windows
                .iter()
                .filter(|(from, to)| *from <= iid && iid <= *to)
                .count();
            assert_eq!(owners, 1, "iid {iid} must belong to exactly one window");
            seen += 1;
        }
        assert_eq!(seen, 1_000, "the windows must cover the whole pool");
    }

    #[test]
    fn windows_of_an_empty_or_small_pool() {
        assert!(
            windows(&RoaringBitmap::new(), 0, 4).is_empty(),
            "an empty pool has no window"
        );

        let one: RoaringBitmap = std::iter::once(42).collect();
        assert_eq!(
            windows(&one, 1, 4),
            vec![(42, 42)],
            "a single live passage gives a single window"
        );

        let few = fragmented(3);
        let total = u32::try_from(few.len()).unwrap_or(u32::MAX);
        assert_eq!(
            windows(&few, total, 8).len(),
            3,
            "fewer live passages than workers give one window per passage, never more"
        );
    }

    /// The `fp32` calculation of ADR-0072 accumulates in `f64` and rounds once,
    /// so a large component does not swallow the small ones of the same vector.
    ///
    /// A sequential `f32` accumulator would lose the `1.0` between `1.0e8` and
    /// `-1.0e8` and return `0.0`: the test fails if the summation ever goes back
    /// to `f32`, which is what ADR-0072 replaced.
    #[test]
    fn the_fp32_dot_accumulates_in_f64_and_rounds_once() {
        let stored = [1.0e8_f32, 1.0, -1.0e8];
        let query = [1.0_f32, 1.0, 1.0];
        assert_eq!(
            fp32_dot(&stored, &query),
            1.0,
            "the f64 accumulator keeps the small term the f32 one loses"
        );

        // The products of two f32 values are exact in f64, so the only rounding
        // is the final one: the result is the f32 nearest to the exact sum.
        let stored = [0.1_f32, 0.2, 0.3];
        let query = [1.0_f32, 1.0, 1.0];
        let exact = f64::from(0.1_f32) + f64::from(0.2_f32) + f64::from(0.3_f32);
        assert_eq!(
            fp32_dot(&stored, &query),
            exact as f32,
            "the result must be the exact f64 sum of the stored components, rounded once"
        );

        // Common-prefix rule of the kernels: the shorter slice decides.
        assert_eq!(
            fp32_dot(&stored, &[2.0_f32]),
            0.1_f32 * 2.0,
            "a single component multiplies the first component only"
        );
    }

    /// The `f16` branch of [`cosine`] is [`crate::kernels::dot`], whose
    /// accumulator is `f64` (ADR-0070, ADR-0072 decision 1).
    ///
    /// The stored vector is `[1.0, 6e-8, -1.0]`, which `f16` rounds to its
    /// smallest subnormal `5.96e-8`, and the query is `[1, 1, 1]`. A sequential
    /// `f32` accumulator rounds `1.0 + 5.96e-8` back to `1.0` (half an ulp,
    /// ties to even), then loses that term against `-1.0` and returns `0.0`.
    /// The `f64` accumulation keeps it. Comparing the score with
    /// [`crate::kernels::dot`] pins the choice: if the `f16` branch ever went
    /// back to an `f32` sum, both strategies would change together and INV-06
    /// would still pass at `1e-5`, but this test would fail.
    #[test]
    fn the_fp16_cosine_accumulates_in_f64_like_the_kernel() {
        use crible_core::config::ReferencePrecision;
        use crible_core::ids::{Generation, Iid};
        use crible_store::segment::{SegmentReader, SegmentSpec, SegmentWriter};
        use half::f16;

        let temp = tempfile::tempdir().unwrap_or_else(|error| {
            panic!("a temporary directory for the segment must be creatable: {error}")
        });
        let spec = SegmentSpec {
            segment_id: "segment-1".to_owned(),
            iid_start: Iid::new(0),
            dims: 3,
            generation: Generation::new(1),
            batch_id: "batch-1".to_owned(),
            precision: ReferencePrecision::Fp16,
        };
        let components = [1.0_f32, 6e-8, -1.0];
        let mut writer = SegmentWriter::create(temp.path(), spec)
            .unwrap_or_else(|error| panic!("the segment writer must start: {error}"));
        writer
            .append(&components)
            .unwrap_or_else(|error| panic!("the stored vector must be appended: {error}"));
        writer
            .finish()
            .unwrap_or_else(|error| panic!("the segment must be finished: {error}"));
        let reader = SegmentReader::open(&temp.path().join("segment-1"))
            .unwrap_or_else(|error| panic!("the segment must open: {error}"));

        let query = [1.0_f32, 1.0, 1.0];
        let score = cosine(&reader, Iid::new(0), &query)
            .unwrap_or_else(|error| panic!("the cosine must be computed: {error}"));
        let stored: Vec<f16> = components
            .iter()
            .map(|component| f16::from_f32(*component))
            .collect();

        assert_eq!(
            score.to_bits(),
            crate::kernels::dot(&stored, &query).to_bits(),
            "the f16 branch of cosine must be the kernel of ADR-0070, bit for bit"
        );
        let sequential: f32 = stored
            .iter()
            .zip(query)
            .map(|(component, query)| component.to_f32() * query)
            .sum();
        assert_eq!(
            sequential, 0.0,
            "a sequential f32 sum is the case the f64 accumulator exists to avoid"
        );
        assert_ne!(
            score, 0.0,
            "the small component must survive the accumulation"
        );
    }
}
