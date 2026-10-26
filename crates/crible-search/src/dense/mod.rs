// SPDX-License-Identifier: Apache-2.0

//! Dense branch of a search: the cosine between the query and the passages of
//! the pool.
//!
//! `docs/03-recherche.md` ("Branche dense") splits the branch by strategy: the
//! scan of the binary codes above the exact threshold ([`binary`]), and the
//! exact dot product below it ([`exact`]). The binary scan chooses its dense or
//! sparse traversal at the threshold of ADR-0071. The planner of
//! `PLANIFIER-STRATEGIE` chooses between the two in P4; until then, nothing here
//! chooses a strategy: the caller decides and passes the pool to search.
//!
//! Both strategies parallelise the same way (`docs/03-recherche.md`,
//! "Parallélisme"): the pool is cut into contiguous windows of internal
//! identifiers, one per rayon worker, each window keeps its own bounded heap,
//! and the heaps are merged at the end. `windows` is that partitioning, shared
//! by the two so their windows can never diverge.

pub mod binary;
pub mod exact;

use roaring::RoaringBitmap;

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

#[cfg(test)]
mod tests {
    use roaring::RoaringBitmap;

    use super::windows;

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
}
