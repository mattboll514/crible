// SPDX-License-Identifier: Apache-2.0

//! Limits of the API: body size, concurrent searches and the request deadline.
//!
//! References: `docs/05-api.md` ("Limites par défaut"), ADR-0102 (body bound,
//! semaphore of the searches, `timeout_ms`), ADR-0038 (`overloaded` for a
//! transient contention, `timeout` for a request deadline) and ADR-0067
//! (`timeout_ms` never applies to a write).
//!
//! `FORMAT-ERREURS-ET-LIMITES` delivers the mechanism; the search routes of
//! `ROUTE-RECHERCHE` and `RECHERCHE-MULTI-INDEX` call it. The router of the
//! tests proves it before those routes exist (ADR-0095).
//!
//! Three things are fixed here:
//!
//! - [`MAX_BODY_BYTES`]: one body bound for the whole instance, a constant of
//!   the code and not a setting, because the list of `docs/07` is closed
//!   (ADR-0097, ADR-0102 decision 1);
//! - [`timeout`]: the bound of `timeout_ms` in the body of a search, from 1 to
//!   30 000 ms, and its default from `[limits].default_timeout_ms`;
//! - [`SearchLimit`]: a semaphore taken **without waiting**. A caller that
//!   finds no token is refused at once with `503 overloaded` and
//!   `Retry-After: 1`, so the answer says the truth instead of queueing the
//!   request (ADR-0102, decision 2).

use std::sync::Arc;
use std::time::Duration;

use axum::response::Response;
use crible_core::error::{CribleError, ErrorCode};
use tokio::sync::Semaphore;

use crate::error;

/// Largest body of any request, in bytes: 32 Mio (ADR-0102, decision 1).
///
/// The same bound applies to the online batch and to the metadata patch of
/// `docs/05-api.md`, whose "32 Mo" are read as Mio.
pub const MAX_BODY_BYTES: usize = 33_554_432;

/// Shortest `timeout_ms` of a search, in milliseconds (ADR-0102, decision 3).
pub const MIN_TIMEOUT_MS: u64 = 1;

/// Longest `timeout_ms` of a search, in milliseconds (ADR-0102, decision 3).
pub const MAX_TIMEOUT_MS: u64 = 30_000;

/// Value of `Retry-After`, in seconds, of the concurrency limit (ADR-0102).
pub const RETRY_AFTER_SECONDS: u64 = 1;

/// Message of the `503` of the concurrency limit.
pub const OVERLOADED_MESSAGE: &str = "too many concurrent searches";

/// Message of the `504` of an expired deadline.
pub const TIMEOUT_MESSAGE: &str = "the search exceeded its deadline (timeout_ms)";

/// Reads the `timeout_ms` of a search, or refuses it.
///
/// The value comes from the body of the search and defaults to
/// `[limits].default_timeout_ms`; outside [`MIN_TIMEOUT_MS`] and
/// [`MAX_TIMEOUT_MS`], the request is `invalid_request` (ADR-0102, decision 3).
/// The search routes call this before running any work; the writes never call
/// it, because `?wait=true` answers `202` at the deadline of `wait_timeout_ms`
/// and never `504` (ADR-0067).
///
/// # Errors
///
/// [`ErrorCode::InvalidRequest`] when the value is outside the bounds.
pub fn timeout(value: Option<u64>, default_ms: u64) -> Result<Duration, CribleError> {
    let milliseconds = value.unwrap_or(default_ms);
    if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&milliseconds) {
        return Err(error::invalid(format!(
            "'timeout_ms' must be between {MIN_TIMEOUT_MS} and {MAX_TIMEOUT_MS} milliseconds"
        )));
    }
    Ok(Duration::from_millis(milliseconds))
}

/// Semaphore of the concurrent searches of an instance (ADR-0102, decision 2).
///
/// One instance holds one `SearchLimit`; the search routes take a token for the
/// whole run of a search, including after its deadline: a `504` does not give
/// the token back, because the rayon computation is not cancelled in v1 and
/// the limit must stay true.
pub struct SearchLimit {
    permits: Arc<Semaphore>,
    total: usize,
}

impl SearchLimit {
    /// Builds the limit of `[server].max_concurrent_searches` searches.
    ///
    /// Zero means twice the number of cores, as `docs/07` declares it.
    #[must_use]
    pub fn new(max_concurrent_searches: usize) -> Self {
        let total = resolved(max_concurrent_searches);
        Self {
            permits: Arc::new(Semaphore::new(total)),
            total,
        }
    }

    /// Number of searches the limit allows at once.
    #[must_use]
    pub fn permits(&self) -> usize {
        self.total
    }

    /// Number of tokens free right now.
    #[must_use]
    pub fn available(&self) -> usize {
        self.permits.available_permits()
    }

    /// Runs `work` on a blocking thread, under one token and one deadline.
    ///
    /// The token is taken **without waiting**: none free means
    /// [`ErrorCode::Overloaded`] at once (ADR-0102, decision 2). The work is
    /// moved into the blocking task with the token, so an expired deadline
    /// answers [`ErrorCode::Timeout`] while the work keeps running and keeps
    /// the token until it really ends (ADR-0102, decision 3). The work of a
    /// search is CPU-bound: the caller installs its rayon computation inside
    /// the closure instead of running it on a tokio thread.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Overloaded`] when no token is free,
    /// [`ErrorCode::Timeout`] at the deadline, [`ErrorCode::Internal`] when the
    /// blocking task does not finish normally.
    pub async fn run_blocking<F, T>(&self, deadline: Duration, work: F) -> Result<T, CribleError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let token = Arc::clone(&self.permits)
            .try_acquire_owned()
            .map_err(|_| CribleError::new(ErrorCode::Overloaded, OVERLOADED_MESSAGE))?;
        // The token travels into the blocking task: dropping the handle below,
        // at the deadline, detaches the task without stopping it, and the token
        // comes back only when `work` returns.
        let running = tokio::task::spawn_blocking(move || {
            let _token = token;
            work()
        });
        match tokio::time::timeout(deadline, running).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_join)) => Err(error::internal(
                "the blocking task of a search did not finish normally",
            )),
            Err(_elapsed) => Err(CribleError::new(ErrorCode::Timeout, TIMEOUT_MESSAGE)),
        }
    }
}

/// Renders the refusal of a search, in the format of `docs/05-api.md`.
///
/// The `503 overloaded` of the concurrency limit carries `Retry-After: 1`
/// through [`error::response`], the single place that adds it to every
/// producer of `overloaded` (ADR-0103, decision 9).
#[must_use]
pub fn refusal(failure: &CribleError, request_id: &str) -> Response {
    error::rendered(failure, request_id)
}

/// Count of tokens of a limit: the setting, or twice the cores for zero.
fn resolved(max_concurrent_searches: usize) -> usize {
    if max_concurrent_searches == 0 {
        2 * std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
    } else {
        max_concurrent_searches
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_BODY_BYTES, MAX_TIMEOUT_MS, MIN_TIMEOUT_MS, OVERLOADED_MESSAGE, SearchLimit,
        TIMEOUT_MESSAGE, resolved, timeout,
    };
    use crible_core::error::ErrorCode;
    use std::sync::{Arc, Barrier};
    use std::time::Duration;

    #[test]
    fn the_body_bound_is_the_fixed_32_mib_of_adr_0102() {
        // ADR-0102, decision 1: 32 Mio, in bytes, and no setting.
        assert_eq!(MAX_BODY_BYTES, 32 * 1024 * 1024);
        assert_eq!(MAX_BODY_BYTES, 33_554_432);
    }

    #[test]
    fn a_timeout_outside_the_bounds_is_an_invalid_request() {
        // ADR-0102, decision 3: 1 to 30 000 ms, and `[limits].default_timeout_ms`
        // when the body names none.
        assert_eq!(
            timeout(Some(1), 2_000).ok(),
            Some(Duration::from_millis(1)),
            "the lower bound is included"
        );
        assert_eq!(
            timeout(Some(MAX_TIMEOUT_MS), 2_000).ok(),
            Some(Duration::from_millis(30_000)),
            "the upper bound is included"
        );
        assert_eq!(
            timeout(None, 2_000).ok(),
            Some(Duration::from_millis(2_000)),
            "the default comes from the configuration"
        );
        for refused in [0, MAX_TIMEOUT_MS + 1, u64::MAX] {
            let failure = timeout(Some(refused), 2_000).expect_err("the value is refused");
            assert_eq!(failure.code(), ErrorCode::InvalidRequest, "{refused}");
            assert!(failure.message().contains("timeout_ms"), "{refused}");
        }
        assert_eq!(
            timeout(None, 0).err().map(|failure| failure.code()),
            Some(ErrorCode::InvalidRequest),
            "a default outside the bounds is refused as well"
        );
    }

    #[test]
    fn zero_concurrent_searches_means_twice_the_cores() {
        // `docs/07-stack-et-deploiement.md`: `0` = 2 × cores.
        let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        assert_eq!(resolved(0), 2 * cores);
        assert_eq!(resolved(7), 7, "a declared count is kept as is");
        assert_eq!(SearchLimit::new(3).permits(), 3);
        assert_eq!(SearchLimit::new(3).available(), 3);
    }

    #[tokio::test]
    async fn a_search_without_a_token_is_refused_without_waiting() {
        // ADR-0102, decision 2: the semaphore is taken without waiting.
        let limit = SearchLimit::new(1);
        let token = limit
            .permits
            .clone()
            .try_acquire_owned()
            .expect("the only token is free");
        let failure = limit
            .run_blocking(Duration::from_secs(30), || ())
            .await
            .expect_err("no token is free");
        assert_eq!(failure.code(), ErrorCode::Overloaded);
        assert_eq!(failure.message(), OVERLOADED_MESSAGE);
        drop(token);
        assert_eq!(limit.available(), 1, "the token comes back");
    }

    #[tokio::test]
    async fn an_expired_deadline_keeps_its_token_until_the_work_ends() {
        // ADR-0102, decision 3: the work is not cancelled, and its token stays
        // taken until it really ends. The work waits for a rendezvous of the
        // test rather than for a duration: no scheduling delay can make it end
        // early, so the deadline is the only possible outcome of the first
        // call and the token can only come back when the test releases it.
        let limit = SearchLimit::new(1);
        let gate = Arc::new(Barrier::new(2));
        let worker = Arc::clone(&gate);
        let failure = limit
            .run_blocking(Duration::from_millis(1), move || {
                worker.wait();
            })
            .await
            .expect_err("the deadline expires before the work ends");
        assert_eq!(failure.code(), ErrorCode::Timeout);
        assert_eq!(failure.message(), TIMEOUT_MESSAGE);
        let failure = limit
            .run_blocking(Duration::from_secs(1), || ())
            .await
            .expect_err("the token is still taken");
        assert_eq!(failure.code(), ErrorCode::Overloaded);

        // The work ends, and the token comes back: awaiting one proves it, with
        // no sleep and no margin.
        gate.wait();
        let token = limit
            .permits
            .clone()
            .acquire_owned()
            .await
            .expect("the semaphore of the limit stays open");
        assert_eq!(
            limit.available(),
            0,
            "the token of the expired work is the one just taken"
        );
        drop(token);
        assert_eq!(
            limit.available(),
            1,
            "the token comes back when the work really ends"
        );
    }

    #[test]
    fn the_bounds_of_the_deadline_are_the_documented_ones() {
        // `docs/05-api.md`: 1 ms at least, 30 000 ms at most.
        assert_eq!(MIN_TIMEOUT_MS, 1);
        assert_eq!(MAX_TIMEOUT_MS, 30_000);
    }
}
