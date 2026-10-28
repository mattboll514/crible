// SPDX-License-Identifier: Apache-2.0

//! Dedicated rayon pool for the scans of a search.
//!
//! References: `docs/02-architecture.md` ("Concurrence": the scans run on a
//! dedicated rayon pool, never on the tokio threads) and
//! `docs/07-stack-et-deploiement.md` (the `rayon` row).
//!
//! The dense branch parallelises its scan with `rayon::par_iter`, which uses the
//! pool installed on the current thread. A server that called the search from a
//! tokio worker would therefore run the scan on the async runtime's threads,
//! and a CPU-bound scan would starve the reactor the way
//! `docs/02-architecture.md` forbids. [`ScanPool`] is that dedicated pool: the
//! caller builds one for the process and runs every scan and every searcher
//! opening inside [`ScanPool::install`].
//!
//! The type is deliberately thin: it owns the `rayon::ThreadPool` and offers
//! only its installation. The CLI of P1 builds one per command; the server of
//! P6 will build one at start-up, from its `server.search_threads` setting, and
//! keep it for every request. Nothing else in the search crate needs to know
//! how the pool is sized.
//!
//! The pool is also what [`DenseSearcher::open`](crate::dense::exact::DenseSearcher::open)
//! must see: it sizes its read-only connection pool on the worker count of the
//! current rayon pool, so opening the searcher inside `install` gives it the
//! right capacity. Opening it outside and searching inside stays correct, but
//! serialises the key reads on one connection.

use crible_core::error::{CribleError, ErrorCode};
use rayon::ThreadPool;

/// Dedicated rayon pool of the scans, separate from the async runtime.
#[derive(Debug)]
pub struct ScanPool {
    pool: ThreadPool,
}

impl ScanPool {
    /// Builds a pool of `threads` workers.
    ///
    /// `0` means one worker per available core, the convention of
    /// `server.search_threads` in `docs/07`. A pool that cannot be built is an
    /// [`ErrorCode::Internal`] error: the process cannot honour the separation
    /// `docs/02-architecture.md` requires.
    pub fn new(threads: usize) -> Result<Self, CribleError> {
        let threads = if threads == 0 {
            std::thread::available_parallelism()
                .map(std::num::NonZeroUsize::get)
                .unwrap_or(1)
        } else {
            threads
        };
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .map_err(|error| {
                CribleError::new(
                    ErrorCode::Internal,
                    format!("cannot build the dedicated scan pool: {error}"),
                )
            })?;
        Ok(Self { pool })
    }

    /// Number of workers of the pool.
    #[must_use]
    pub fn threads(&self) -> usize {
        self.pool.current_num_threads()
    }

    /// Runs `operation` on the pool, so every `rayon` scan it starts runs on
    /// these workers and not on the caller's.
    pub fn install<T, F>(&self, operation: F) -> T
    where
        T: Send,
        F: FnOnce() -> T + Send,
    {
        self.pool.install(operation)
    }
}

#[cfg(test)]
mod tests {
    use super::ScanPool;

    #[test]
    fn a_scan_pool_uses_the_requested_worker_count() {
        let pool = ScanPool::new(2).expect("a two-worker pool must be buildable");
        assert_eq!(pool.threads(), 2);
        let inside = pool.install(rayon::current_num_threads);
        assert_eq!(inside, 2, "the scan must run on the dedicated pool");
    }

    #[test]
    fn zero_means_one_worker_per_available_core() {
        let pool = ScanPool::new(0).expect("the default pool must be buildable");
        assert!(
            pool.threads() >= 1,
            "the default pool must hold at least one worker"
        );
        assert_eq!(pool.install(rayon::current_num_threads), pool.threads());
    }
}
