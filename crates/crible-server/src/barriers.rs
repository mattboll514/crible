// SPDX-License-Identifier: Apache-2.0

//! Test barriers of the `failpoints` build.
//!
//! Some invariants need a job to be **held** at a precise point while a test
//! changes the instance and reads it from another request: the job-time
//! revalidation of a deposited batch (`ADR-0107`, decision 5), the read of a
//! document during a compaction (`ADR-0107`, decision 6) and the search sent
//! during an attach held in its write (INV-02; `ADR-0113`, decision 3). A crash
//! failpoint panics, so it cannot hold anything; these barriers are the
//! suspension counterpart, and, like
//! `crible_store::compact::compact_with_observer`, they exist **only** with the
//! `failpoints` cargo feature, never in a normal build and never in a release
//! build.
//!
//! Protocol, entirely driven by one environment variable:
//!
//! - `CRIBLE_TEST_BARRIER` names a directory the test owns;
//! - the job writes `<dir>/ready-<scope>` and waits for `<dir>/go-<scope>`;
//! - the attach steps need one more mark: a test that arms
//!   `attach-before-commit` or `attach-after-sqlite-commit` also writes
//!   `<dir>/armed-attach`, so an attach started by a test that only arms the
//!   `batch`, `search` or `compaction` scope is never held;
//! - the pre-token step of the search route needs `<dir>/armed-search-start`,
//!   so a search of a test that holds a write job is never held there;
//! - the work step of the search route needs `<dir>/armed-search`, and the one
//!   of the multi-index route `<dir>/armed-search-multi`, for the same reason:
//!   every server test sends searches, so a test that arms only another scope
//!   must not stop them on their token;
//! - the head of the loop of the analytics writer needs
//!   `<dir>/armed-analytics-flush`, so a test that holds a write job or a search
//!   never holds the analytics (ADR-0118, decision 7); that barrier is async, so
//!   it waits on the async runtime and never blocks the requests of the test;
//! - without the variable the call returns at once, so a build with the feature
//!   but no test armed behaves exactly like a normal one.
//!
//! The wait is bounded: a test that dies before releasing the barrier delays the
//! job by at most [`RELEASE_TIMEOUT`], never for ever.

use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

/// Environment variable naming the barrier directory of a test.
pub(crate) const BARRIER_ENV: &str = "CRIBLE_TEST_BARRIER";

/// How long a job waits for its release before going on without it.
///
/// The bound keeps a crashed test from wedging the writer of the index; 30 s is
/// far above the few milliseconds a test needs to create a link or read a
/// document, and far below the timeout of any test runner.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(30);

/// Granularity of the polling loop, in milliseconds.
const POLL_MILLIS: u64 = 5;

/// Name of the mark a test writes to arm the attach steps.
///
/// The attach runs in every write job, so it would otherwise be held by any
/// test that armed another scope of the same directory.
const ATTACH_ARMED: &str = "armed-attach";

/// Delivers one step of an attach write to the barriers a test armed.
///
/// `attach-before-commit` holds the attach with an open transaction, before the
/// SQLite `COMMIT`; `attach-after-sqlite-commit` holds it once the generation is
/// committed, before its snapshot is published. A search sent at either point
/// must answer the previous, published generation (INV-02; ADR-0113,
/// decision 3). The observer is
/// [`CommitStep`](crible_store::generation::CommitStep).
pub(crate) fn hold_attach(step: crible_store::generation::CommitStep) {
    match step {
        crible_store::generation::CommitStep::BeforeSqliteCommit => {
            wait_attach("attach-before-commit")
        }
        crible_store::generation::CommitStep::AfterSqliteCommit => {
            wait_attach("attach-after-sqlite-commit");
        }
    }
}

/// Holds the calling attach until the test releases `scope`, if the attach
/// steps are armed.
///
/// A write of a test that only arms another scope runs at full speed: the
/// attach steps are opt-in through [`ATTACH_ARMED`].
fn wait_attach(scope: &str) {
    wait_armed(ATTACH_ARMED, scope);
}

/// Holds the calling job at `scope`, if the mark `armed` is present.
///
/// The variable names one directory per process, and several tests of a binary
/// may arm different steps of it: a step is held only when the test wrote its
/// own mark, so an unrelated scope never delays a job.
fn wait_armed(armed: &str, scope: &str) {
    let Ok(directory) = std::env::var(BARRIER_ENV) else {
        return;
    };
    if !Path::new(&directory).join(armed).exists() {
        return;
    }
    hold(&directory, scope);
}

/// Name of the mark a test writes to arm the pre-token search step.
///
/// The search route is reached by tests that only hold a write job, so the step
/// is opt-in: without this mark a search of an armed directory runs at full
/// speed, like for the attach steps.
const SEARCH_START_ARMED: &str = "armed-search-start";

/// Holds the calling search before it takes its concurrency token.
///
/// The route reads the instant of `took_ms` before this point, so a test proves
/// that the duration covers the wait (ADR-0109, decision 2).
pub(crate) fn wait_search_start() {
    let Ok(directory) = std::env::var(BARRIER_ENV) else {
        return;
    };
    if !Path::new(&directory).join(SEARCH_START_ARMED).exists() {
        return;
    }
    hold(&directory, "search-start");
}

/// Name of the mark a test writes to arm the work step of the search route.
///
/// Every server test sends searches, and the analytics tests arm a directory of
/// their own; without this mark their searches would stop here for
/// [`RELEASE_TIMEOUT`] and answer `504` (INV-17, ADR-0118, decision 7).
const SEARCH_ARMED: &str = "armed-search";

/// Holds the calling search inside its blocking task, before the computation.
///
/// A test proves here that a `504` keeps its concurrency token until the
/// computation really ends (ADR-0102, decision 3). The step is opt-in through
/// [`SEARCH_ARMED`].
pub(crate) fn wait_search() {
    wait_armed(SEARCH_ARMED, "search");
}

/// Name of the mark a test writes to arm the work step of the multi-index route.
const SEARCH_MULTI_ARMED: &str = "armed-search-multi";

/// Holds the calling multi-index search inside its blocking task.
///
/// A test proves here the single token of the whole request (ADR-0110,
/// decision 8; ADR-0102, decision 2). The step is opt-in through
/// [`SEARCH_MULTI_ARMED`].
pub(crate) fn wait_search_multi() {
    wait_armed(SEARCH_MULTI_ARMED, "search-multi");
}

/// Name of the mark a test writes to arm the analytics writer's hold point.
///
/// The analytics writer runs in every server test, so the hold point is opt-in:
/// without this mark a test that armed another scope runs the writer at full
/// speed, like the attach steps above.
const ANALYTICS_FLUSH_ARMED: &str = "armed-analytics-flush";

/// Holds the analytics writer at the head of its loop, before it receives
/// anything (ADR-0118, decision 7).
///
/// It is **async**: the writer runs on the async runtime, and a blocking sleep
/// here would stop every task of a current-thread test runtime, the route that
/// must answer while the writer is held included.
pub(crate) async fn wait_analytics_flush() {
    let Ok(directory) = std::env::var(BARRIER_ENV) else {
        return;
    };
    let directory = Path::new(&directory);
    if !directory.join(ANALYTICS_FLUSH_ARMED).exists() {
        return;
    }
    let ready = directory.join("ready-analytics-flush");
    let go = directory.join("go-analytics-flush");
    if fs::write(&ready, b"").is_err() {
        return;
    }
    let deadline = Instant::now() + RELEASE_TIMEOUT;
    while !go.exists() {
        if Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(POLL_MILLIS)).await;
    }
}

/// Name of the mark a test writes to arm the debug journal writer's hold point.
///
/// The journal writer runs in every server test whose instance enabled the
/// journal, so the hold point is opt-in (ADR-0122, decision 11).
const DEBUG_JOURNAL_FLUSH_ARMED: &str = "armed-debug-journal-flush";

/// Holds the debug journal writer at the head of its loop, before it receives
/// anything (ADR-0122, decision 11).
///
/// It is **async**, like [`wait_analytics_flush`]: the writer runs on the async
/// runtime, and the route that must answer while it is held runs there too. The
/// scope `debug-journal-flush` is distinct from `analytics-flush`, so an armed
/// barrier for one writer never retains the other.
pub(crate) async fn wait_debug_journal_flush() {
    let Ok(directory) = std::env::var(BARRIER_ENV) else {
        return;
    };
    let directory = Path::new(&directory);
    if !directory.join(DEBUG_JOURNAL_FLUSH_ARMED).exists() {
        return;
    }
    let ready = directory.join("ready-debug-journal-flush");
    let go = directory.join("go-debug-journal-flush");
    if fs::write(&ready, b"").is_err() {
        return;
    }
    let deadline = Instant::now() + RELEASE_TIMEOUT;
    while !go.exists() {
        if Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(POLL_MILLIS)).await;
    }
}

/// Holds the calling job until the test writes `<dir>/go-<scope>`.
///
/// `scope` names the barrier: a test that arms two of them in one binary uses
/// two different names.
pub(crate) fn wait(scope: &str) {
    let Ok(directory) = std::env::var(BARRIER_ENV) else {
        return;
    };
    hold(&directory, scope);
}

/// Writes `<dir>/ready-<scope>` and waits for `<dir>/go-<scope>`.
fn hold(directory: &str, scope: &str) {
    let directory = Path::new(directory);
    let ready = directory.join(format!("ready-{scope}"));
    let go = directory.join(format!("go-{scope}"));
    if fs::write(&ready, b"").is_err() {
        return;
    }
    let deadline = Instant::now() + RELEASE_TIMEOUT;
    while !go.exists() {
        if Instant::now() >= deadline {
            return;
        }
        thread::sleep(Duration::from_millis(POLL_MILLIS));
    }
}
