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
    let Ok(directory) = std::env::var(BARRIER_ENV) else {
        return;
    };
    if !Path::new(&directory).join(ATTACH_ARMED).exists() {
        return;
    }
    hold(&directory, scope);
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
