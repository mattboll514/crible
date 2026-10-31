// SPDX-License-Identifier: Apache-2.0

// Test fixtures may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the helpers of this file are free functions.
#![allow(clippy::expect_used, clippy::panic)]

//! `jobs.sqlite`: schema, states, restart and retention (ADR-0103).
//!
//! The acceptance criteria of `FILE-D-ECRITURE-ET-JOBS` that belong to the store
//! are proved here: a finished job survives a restart, the transient states of a
//! crash become `failed` at open, an unknown `format_version` is refused
//! (INV-15), and the retention removes a job finished more than 30 days ago
//! without touching one still running.

use crible_core::error::ErrorCode;
use crible_store::jobs::{
    FORMAT_VERSION, INTERRUPTED_MESSAGE, JOBS_FILE_NAME, JobKind, JobState, JobStore,
    RETENTION_DAYS,
};
use rusqlite::Connection;
use tempfile::TempDir;

/// A temporary `<data>` directory and the path of its job table.
fn fixture() -> (TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().expect("a temporary directory is creatable");
    let path = directory.path().join(JOBS_FILE_NAME);
    (directory, path)
}

/// Runs one job from `queued` to `succeeded`.
fn succeed(store: &JobStore, id: &str) {
    store
        .mark_running(id, Some(r#"{"step":"commit","done":1,"total":1}"#))
        .expect("a queued job starts");
    store
        .finish_succeeded(id, Some(7), Some("null"), Some(r#"{"report":"kept"}"#))
        .expect("a running job succeeds");
}

#[test]
fn a_finished_job_is_read_after_a_reopen() {
    // Acceptance criterion: a finished job remains readable after a restart.
    let (_directory, path) = fixture();
    {
        let store = JobStore::open_or_create(&path).expect("the job table opens");
        let created = store
            .insert(
                "job_00000000000000000000000000000001",
                JobKind::AttachBatch,
                "alpha",
            )
            .expect("the job is inserted");
        assert!(!created.is_empty(), "SQLite writes created_at");
        assert_eq!(
            store
                .job("job_00000000000000000000000000000001")
                .expect("read")
                .map(|job| job.state),
            Some(JobState::Queued)
        );
        succeed(&store, "job_00000000000000000000000000000001");
    }
    let store = JobStore::open_or_create(&path).expect("the job table reopens");
    let job = store
        .job("job_00000000000000000000000000000001")
        .expect("read")
        .expect("the finished job survived the restart");
    assert_eq!(job.state, JobState::Succeeded);
    assert_eq!(job.generation, Some(7));
    assert_eq!(job.result.as_deref(), Some(r#"{"report":"kept"}"#));
    assert!(job.started_at.is_some(), "the start is kept");
    assert!(job.finished_at.is_some(), "the end is kept");
}

#[test]
fn an_unknown_format_version_is_refused() {
    // INV-15 and ADR-0103, decision 1: a version this build does not know stops
    // the startup instead of being read as if it were this one.
    let (_directory, path) = fixture();
    let connection = Connection::open(&path).expect("the file is creatable");
    connection
        .execute_batch("CREATE TABLE meta (format_version INTEGER NOT NULL);")
        .expect("the meta table is creatable");
    connection
        .execute(
            "INSERT INTO meta (format_version) VALUES (?1)",
            [FORMAT_VERSION + 1],
        )
        .expect("the version is writable");
    drop(connection);

    let failure = JobStore::open_or_create(&path)
        .err()
        .expect("an unknown version is refused");
    assert_eq!(failure.code(), ErrorCode::InvalidRequest);
    assert!(
        failure.message().contains("format_version"),
        "the message names the version: {}",
        failure.message()
    );

    let (_other_directory, other) = fixture();
    let connection = Connection::open(&other).expect("the file is creatable");
    connection
        .execute_batch("CREATE TABLE unrelated (value INTEGER);")
        .expect("a foreign table is creatable");
    drop(connection);
    let failure = JobStore::open_or_create(&other)
        .err()
        .expect("a database without meta is refused");
    assert_eq!(failure.code(), ErrorCode::InvalidRequest);
    assert!(failure.message().contains("format_version"));
}

#[test]
fn transient_states_are_failed_at_open_and_never_replayed() {
    // ADR-0103, decision 5: a job left `queued` or `running` by a crash is
    // `failed` at the next startup, with the fixed message, and is not replayed.
    let (_directory, path) = fixture();
    {
        let store = JobStore::open_or_create(&path).expect("the job table opens");
        store
            .insert(
                "job_00000000000000000000000000000002",
                JobKind::Compact,
                "alpha",
            )
            .expect("the queued job is inserted");
        store
            .insert(
                "job_00000000000000000000000000000003",
                JobKind::Export,
                "alpha",
            )
            .expect("the running job is inserted");
        store
            .mark_running("job_00000000000000000000000000000003", None)
            .expect("the second job starts");
    }
    let store = JobStore::open_or_create(&path).expect("the job table reopens");
    let changed = store.fail_interrupted().expect("the recovery runs");
    assert_eq!(changed, 2, "the queued and the running job are corrected");
    for id in [
        "job_00000000000000000000000000000002",
        "job_00000000000000000000000000000003",
    ] {
        let job = store.job(id).expect("read").expect("the job is kept");
        assert_eq!(job.state, JobState::Failed, "{id}");
        assert!(job.finished_at.is_some(), "{id} names its end");
        let error: serde_json::Value = serde_json::from_str(
            job.error
                .as_deref()
                .expect("the failed job carries an error"),
        )
        .expect("the error is JSON");
        assert_eq!(error["code"], "internal");
        assert_eq!(error["message"], INTERRUPTED_MESSAGE);
        assert_eq!(error["details"], serde_json::json!({}));
        assert!(
            error.get("request_id").is_none(),
            "a job error has no request_id (ADR-0103)"
        );
    }
}

#[test]
fn jobs_finished_more_than_thirty_days_ago_are_purged() {
    // ADR-0103, decision 6: the retention is fixed at 30 days, removes the job
    // from the table, and never touches one still queued.
    let (_directory, path) = fixture();
    {
        let store = JobStore::open_or_create(&path).expect("the job table opens");
        for (id, kind) in [
            ("job_00000000000000000000000000000004", JobKind::Compact),
            ("job_00000000000000000000000000000005", JobKind::Export),
        ] {
            store
                .insert(id, kind, "alpha")
                .expect("the job is inserted");
            succeed(&store, id);
        }
        store
            .insert(
                "job_00000000000000000000000000000006",
                JobKind::Snapshot,
                "alpha",
            )
            .expect("a queued job is inserted");
    }
    // Backdate the first job only, with the clock of `batches.applied_at`.
    let old = format!("-{} days", RETENTION_DAYS + 1);
    let connection = Connection::open(&path).expect("the file is readable");
    connection
        .execute(
            "UPDATE jobs SET finished_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?1)
             WHERE id = ?2",
            rusqlite::params![old, "job_00000000000000000000000000000004"],
        )
        .expect("the job is backdated");
    drop(connection);

    let store = JobStore::open_or_create(&path).expect("the job table reopens");
    let purged = store.purge().expect("the retention runs");
    assert_eq!(
        purged,
        vec!["job_00000000000000000000000000000004".to_owned()]
    );
    assert!(
        store
            .job("job_00000000000000000000000000000004")
            .expect("read")
            .is_none(),
        "the expired job is gone"
    );
    assert!(
        store
            .job("job_00000000000000000000000000000005")
            .expect("read")
            .is_some(),
        "a recent finished job is kept"
    );
    assert!(
        store
            .job("job_00000000000000000000000000000006")
            .expect("read")
            .is_some(),
        "a queued job is never purged"
    );
}

#[test]
fn the_list_is_filtered_and_ordered() {
    // ADR-0103, decision 7: filters by index and state, ordered by created_at
    // descending then id. The test writes the two timestamps itself so the
    // order does not depend on the second the rows are inserted in.
    let (_directory, path) = fixture();
    let store = JobStore::open_or_create(&path).expect("the job table opens");
    for (id, kind, index) in [
        (
            "job_00000000000000000000000000000007",
            JobKind::Compact,
            "alpha",
        ),
        (
            "job_00000000000000000000000000000008",
            JobKind::Export,
            "beta",
        ),
    ] {
        store.insert(id, kind, index).expect("the job is inserted");
        succeed(&store, id);
    }
    let connection = Connection::open(&path).expect("the file is readable");
    connection
        .execute(
            "UPDATE jobs SET created_at = ?1 WHERE id = ?2",
            rusqlite::params![
                "2026-01-01T00:00:00Z",
                "job_00000000000000000000000000000007"
            ],
        )
        .expect("the older job is dated");
    connection
        .execute(
            "UPDATE jobs SET created_at = ?1 WHERE id = ?2",
            rusqlite::params![
                "2026-02-01T00:00:00Z",
                "job_00000000000000000000000000000008"
            ],
        )
        .expect("the newer job is dated");
    drop(connection);

    let all = store.list(None, None).expect("the list is readable");
    assert_eq!(
        all.iter().map(|job| job.id.as_str()).collect::<Vec<_>>(),
        [
            "job_00000000000000000000000000000008",
            "job_00000000000000000000000000000007"
        ],
        "the newest is first"
    );
    let alpha = store
        .list(Some("alpha"), None)
        .expect("the filtered list is readable");
    assert_eq!(alpha.len(), 1);
    assert_eq!(alpha[0].index, "alpha");
    let succeeded = store
        .list(Some("beta"), Some(JobState::Succeeded))
        .expect("the doubly filtered list is readable");
    assert_eq!(succeeded.len(), 1);
    assert!(
        store
            .list(None, Some(JobState::Failed))
            .expect("the filtered list is readable")
            .is_empty(),
        "no job of this fixture failed"
    );
}

#[test]
fn the_timestamps_follow_the_format_of_batches() {
    // ADR-0103, decision 2: `created_at` is RFC 3339 UTC, written by SQLite, so
    // the code owns no clock and no format.
    let (_directory, path) = fixture();
    let store = JobStore::open_or_create(&path).expect("the job table opens");
    let created = store
        .insert(
            "job_00000000000000000000000000000009",
            JobKind::DeleteIndex,
            "alpha",
        )
        .expect("the job is inserted");
    assert_eq!(created.len(), 20, "YYYY-MM-DDTHH:MM:SSZ: {created}");
    assert!(created.ends_with('Z'), "{created}");
    assert_eq!(&created[4..5], "-");
    assert_eq!(&created[10..11], "T");
}
