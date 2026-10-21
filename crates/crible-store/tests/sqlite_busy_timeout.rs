// SPDX-License-Identifier: Apache-2.0

//! Acceptance tests of the SQLite lock wait of `crible_store::records`
//! (ADR-0038): a second writer waits for the busy timeout before it fails, a
//! short contention is survived, and the remaining contention is reported as
//! `overloaded`, not `internal`.
//!
//! A writer holds the lock with a raw `rusqlite` connection: [`RecordStore`]
//! commits every write on its own, so it has no public way to keep a write
//! transaction open. The holder is a real second connection to the same file, so
//! SQLite sees the same lock contention as a concurrent `crible` process.
//!
//! Synchronization is explicit: the holder takes the lock *before* the waiter is
//! spawned, and the waiter announces itself through a channel before it writes.
//! No test relies on a sleep to win a race.
//!
//! References: `docs/adr/0038-delai-d-attente-sqlite-busy-timeout.md`,
//! `docs/02-architecture.md` ("Concurrence") and INV-02.

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crible_core::error::ErrorCode;
use crible_core::ids::Generation;
use crible_store::records::{DocumentRecord, RecordStore};
use rusqlite::Connection;
use serde_json::json;

/// The busy timeout of ADR-0038, restated here because it is a private constant
/// of `crible-store`: the test must not read it through a new public API.
const BUSY_TIMEOUT: Duration = Duration::from_millis(1000);

/// Upper bound of every channel wait, so a regression fails the test instead of
/// hanging the suite.
const CHANNEL_TIMEOUT: Duration = Duration::from_secs(30);

fn document(doc_id: &str) -> DocumentRecord {
    DocumentRecord {
        doc_id: doc_id.to_owned(),
        doc_hash: format!("blake3-of-{doc_id}"),
        fields: json!({ "titre": "T" }),
        updated_generation: Generation::new(1),
    }
}

/// Creates `records.sqlite` at `path` and returns its path.
fn new_database(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let path = dir.path().join("records.sqlite");
    let mut store = RecordStore::create(&path)
        .unwrap_or_else(|error| panic!("cannot create {path:?}: {error}"));
    store
        .write_document(&document("doc-a"))
        .unwrap_or_else(|error| panic!("cannot write the first document: {error}"));
    drop(store);
    path
}

/// Holds the write lock of `path` until the returned connection is dropped.
fn hold_write_lock(path: &std::path::Path) -> Connection {
    let connection = Connection::open(path)
        .unwrap_or_else(|error| panic!("cannot open {path:?} for the holder: {error}"));
    // `BEGIN IMMEDIATE` takes the write lock now, instead of on the first
    // statement, so the waiter is guaranteed to meet a held lock.
    connection
        .execute_batch("BEGIN IMMEDIATE")
        .unwrap_or_else(|error| panic!("the holder cannot take the write lock: {error}"));
    connection
}

#[test]
fn a_second_writer_waits_for_the_busy_timeout_then_fails_overloaded() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = new_database(&dir);

    // Opened before the holder takes the lock: the failure below comes from the
    // write transaction, the path this test is about.
    let mut store = RecordStore::open(&path).expect("the second writer opens the database");
    let holder = hold_write_lock(&path);

    let (done_tx, done_rx) = mpsc::channel();
    thread::scope(|scope| {
        scope.spawn(|| {
            let started = Instant::now();
            let outcome = store.write_document(&document("doc-b"));
            done_tx
                .send((outcome, started.elapsed()))
                .expect("the second writer reports its result");
        });

        let (outcome, elapsed) = done_rx
            .recv_timeout(CHANNEL_TIMEOUT)
            .expect("the second writer finishes once the timeout expires");

        let error = outcome.expect_err("the write must fail while the lock is held");
        assert_eq!(
            error.code(),
            ErrorCode::Overloaded,
            "a lock contention must be retryable, not internal: {error}"
        );
        // Lower bound only: SQLite waits at least the timeout, and a generous
        // upper bound would make the test fail on a loaded machine.
        assert!(
            elapsed >= BUSY_TIMEOUT,
            "the write must wait the busy timeout of {BUSY_TIMEOUT:?}, waited {elapsed:?}"
        );

        drop(holder);
    });

    // The failed write is not part of the database.
    let reader = RecordStore::open(&path).expect("the database reopens");
    assert!(
        reader
            .read_document("doc-b")
            .expect("the document is read")
            .is_none(),
        "a write that failed on the busy timeout must not be committed"
    );
}

#[test]
fn a_short_contention_is_survived() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = new_database(&dir);

    let mut store = RecordStore::open(&path).expect("the second writer opens the database");
    let holder = hold_write_lock(&path);

    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    thread::scope(|scope| {
        scope.spawn(|| {
            started_tx
                .send(())
                .expect("the second writer announces its write");
            let started = Instant::now();
            let outcome = store.write_document(&document("doc-b"));
            done_tx
                .send((outcome, started.elapsed()))
                .expect("the second writer reports its result");
        });

        started_rx
            .recv_timeout(CHANNEL_TIMEOUT)
            .expect("the second writer starts its write");
        // The lock is released well inside the busy timeout, so the write must
        // succeed. The delay is the contention this test measures, not a race:
        // the write can only start after the holder took the lock.
        thread::sleep(Duration::from_millis(100));
        holder
            .execute_batch("COMMIT")
            .expect("the holder releases the write lock");

        let (outcome, elapsed) = done_rx
            .recv_timeout(CHANNEL_TIMEOUT)
            .expect("the second writer finishes once the lock is free");
        outcome.expect("a write that waited less than the busy timeout must succeed");
        assert!(
            elapsed >= Duration::from_millis(50),
            "the write must have waited for the lock, waited {elapsed:?}"
        );
    });
}
