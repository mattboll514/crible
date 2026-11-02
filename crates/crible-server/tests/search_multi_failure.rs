// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and every helper of this file is a fixture or an assertion.
#![allow(clippy::expect_used, clippy::panic)]

//! A multi-index search that fails **after** the targets were resolved records
//! one event per resolved target (ADR-0124, decision 10; `docs/06`, decision on
//! the events).
//!
//! | Test | What it proves |
//! |---|---|
//! | `a_failure_on_the_first_target_records_one_event_per_target` | three targets, the first one fails before any answer: three events, the same code |
//! | `a_failure_on_a_later_target_keeps_the_earlier_events` | the targets that answered keep `ok` and their steps |
//! | `a_refused_vector_records_one_event_per_target` | a refusal taken at the single encoding/vector step spreads over the resolved targets |
//! | `a_search_refused_at_the_token_records_one_event_per_target` | `503 overloaded` before any target answers: one event per target |
//!
//! The two mid-loop failures exist **only with the `failpoints` feature**: a
//! served generation opens no reader, so removing the epoch of an index from the
//! disk fails no request any more (ADR-0126, decisions 1 and 4). The fault of
//! ADR-0126, decision 6 is raised inside the search of one target, after its
//! filter ran and before its branches, which is where the opening of the readers
//! used to fail.
//!
//! ```text
//! cargo test -p crible-server --test search_multi_failure
//! cargo test -p crible-server --features failpoints --test search_multi_failure
//! ```

mod common;

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::http::build_router;
use crible_server::state::AppState;
use crible_store::analytics::{ANALYTICS_FILE_NAME, MinuteEvent, read_minute_events};
use serde_json::{Value, json};

#[cfg(feature = "failpoints")]
use common::lock_barrier_environment;
use common::{
    ADMIN_SECRET, Instance, READ_SECRET, WRITE_SECRET, base64_vector, index_config, send,
    terminal_job,
};

/// Path of the multi-index route.
const MULTI: &str = "/v1/search";

/// The three targets of the fixture, in the order of the requests.
const TARGETS: [&str; 3] = ["alpha", "beta", "gamma"];

/// A dense multi-index search over [`TARGETS`] with a long deadline.
fn body() -> Value {
    json!({
        "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
        "mode": "dense",
        "timeout_ms": 30_000,
        "targets": [ { "index": "alpha" }, { "index": "beta" }, { "index": "gamma" } ],
    })
}

/// Creates the index `name` on the fixture model, without lexical branch.
async fn create_index(router: &Router, name: &str) {
    let mut body = index_config();
    body["name"] = json!(name);
    let (status, answer) = send(
        router,
        Method::POST,
        "/v1/indexes",
        ADMIN_SECRET,
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "the index {name}: {answer}");
}

/// Attaches one document to `index` and waits for the job.
async fn attach_one(router: &Router, state: &Arc<AppState>, index: &str) {
    let path = format!("/v1/indexes/{index}/documents?wait=true");
    let document = json!({
        "doc_id": "DOC-1",
        "fields": { "numero": "1240" },
        "passages": [{
            "text": "contrat",
            "fields": { "texte": "contrat" },
            "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
        }],
    });
    let body = json!({
        "batch_id": format!("{index}-1"),
        "mode": "upsert",
        "documents": [document],
    });
    let (status, accepted) = send(router, Method::POST, &path, WRITE_SECRET, Some(body)).await;
    assert!(
        status == StatusCode::OK || status == StatusCode::ACCEPTED,
        "the batch of {index} is accepted: {accepted}"
    );
    let job = terminal_job(router, state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the batch of {index}: {job}");
}

/// An instance that serves [`TARGETS`], with `max_concurrent_searches`.
async fn instance_with_targets(
    max_concurrent_searches: usize,
) -> (Instance, Arc<AppState>, Router) {
    let instance = Instance::with_max_concurrent_searches(max_concurrent_searches);
    let state = instance.state();
    let router = build_router(Arc::clone(&state));
    create_index(&router, "alpha").await;
    for name in ["beta", "gamma"] {
        create_index(&router, name).await;
        attach_one(&router, &state, name).await;
    }
    attach_one(&router, &state, "alpha").await;
    (instance, state, router)
}

/// An instance that serves [`TARGETS`], with the search of `faulty` failing.
///
/// ADR-0126, decision 6: the readers of a served generation are cached, so no
/// per-query read of the disk fails any more. The fault is armed through the
/// `failpoints` build of the server, and raised inside the search of one target,
/// after its filter ran — the point where the opening of the readers used to
/// fail.
#[cfg(feature = "failpoints")]
async fn instance_with_fault(faulty: &str) -> (Armed, Instance, Arc<AppState>, Router) {
    let armed = Armed::arm(faulty).await;
    let (instance, state, router) = instance_with_targets(0).await;
    (armed, instance, state, router)
}

/// The process-global barrier variable of one test, cleared by `Drop`.
///
/// The variable is set for the whole process, so an assertion that panics before
/// the end of the test must not leave it naming a directory that `Drop` of the
/// `TempDir` then removes: the other tests of the binary would read a stale
/// value. The guard clears it first, as `dense_reader_cache` and
/// `inv_02_dense_readers` do.
#[cfg(feature = "failpoints")]
struct Armed {
    /// Directory the variable names, kept alive until the guard drops.
    _barrier: tempfile::TempDir,
    /// Serialises the tests of this binary that arm the variable.
    _guard: tokio::sync::MutexGuard<'static, ()>,
}

#[cfg(feature = "failpoints")]
impl Armed {
    /// Sets the variable on a fresh directory and names `index` as the failing
    /// target.
    async fn arm(index: &str) -> Self {
        let guard = lock_barrier_environment().await;
        let barrier = tempfile::tempdir().expect("a barrier directory is creatable");
        // SAFETY: the guard is held, so no other test of this binary arms or reads
        // the variable until the guard is released, after `Drop` removed it.
        unsafe { std::env::set_var("CRIBLE_TEST_BARRIER", barrier.path()) };
        std::fs::write(barrier.path().join("fault-search-index"), index)
            .expect("the fault of one index is armable");
        Self {
            _barrier: barrier,
            _guard: guard,
        }
    }
}

#[cfg(feature = "failpoints")]
impl Drop for Armed {
    fn drop(&mut self) {
        // SAFETY: the guard is still held here, so no test of this binary reads
        // the variable while it is removed.
        unsafe { std::env::remove_var("CRIBLE_TEST_BARRIER") };
    }
}

/// The `search_multi` rows of the analytics file, after a forced flush.
async fn flushed_rows(instance: &Instance, state: &Arc<AppState>) -> Vec<MinuteEvent> {
    state
        .flush_analytics()
        .await
        .expect("the analytics are flushed");
    let path = instance.root().join("data").join(ANALYTICS_FILE_NAME);
    read_minute_events(&path)
        .expect("the analytics file is readable")
        .into_iter()
        .filter(|row| row.operation == "search_multi")
        .collect()
}

/// Number of samples of a latency histogram.
fn samples(histogram: &[u32]) -> u64 {
    histogram.iter().map(|bucket| u64::from(*bucket)).sum()
}

/// Asserts that `row` carries no step and no pool size: nothing is invented for
/// work that did not run.
fn assert_empty_steps(row: &MinuteEvent) {
    for (step, histogram) in [
        ("filter", &row.latency_filter),
        ("dense", &row.latency_dense),
        ("lexical", &row.latency_lexical),
        ("fusion", &row.latency_fusion),
        ("group", &row.latency_group),
        ("fetch", &row.latency_fetch),
    ] {
        assert_eq!(
            samples(histogram),
            0,
            "the step {step} stays empty: {row:?}"
        );
    }
    // The pool histogram carries its single sample of zero: no size above it is
    // invented for work that did not run (ADR-0124, decision 10).
    assert!(
        row.pool
            .get(1..)
            .is_some_and(|above| { above.iter().all(|bucket| *bucket == 0) }),
        "no pool size is invented: {row:?}"
    );
}

/// A failure of the **first** target leaves no earlier event to keep: every
/// resolved target still records one event, all carrying the code of the
/// answer, with empty steps (ADR-0124, decision 10).
#[cfg(feature = "failpoints")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failure_on_the_first_target_records_one_event_per_target() {
    let (_armed, instance, state, router) = instance_with_fault("alpha").await;

    let (status, answer) = send(&router, Method::POST, MULTI, READ_SECRET, Some(body())).await;
    assert!(
        status.is_server_error(),
        "the faulty target fails the request: {answer}"
    );

    let rows = flushed_rows(&instance, &state).await;
    assert_eq!(
        rows.len(),
        TARGETS.len(),
        "one event per resolved target, never a single one: {rows:?}"
    );
    for (row, expected) in rows.iter().zip(TARGETS) {
        assert_eq!(row.index, expected, "{rows:?}");
        assert_eq!(
            row.status, "internal",
            "the code is the one of the answer: {rows:?}"
        );
        assert_eq!(row.mode, "dense", "the mode was resolved: {rows:?}");
        assert_eq!(row.count, 1, "{rows:?}");
        assert_empty_steps(row);
    }
}

/// A failure in the middle of the loop keeps the event of the targets that
/// answered — their `ok` and their own steps — and gives the code of the answer
/// to the faulty target and to those that never ran (ADR-0124, decision 10).
#[cfg(feature = "failpoints")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failure_on_a_later_target_keeps_the_earlier_events() {
    let (_armed, instance, state, router) = instance_with_fault("beta").await;

    let (status, answer) = send(&router, Method::POST, MULTI, READ_SECRET, Some(body())).await;
    assert!(
        status.is_server_error(),
        "the faulty target fails the request: {answer}"
    );

    let rows = flushed_rows(&instance, &state).await;
    assert_eq!(
        rows.len(),
        TARGETS.len(),
        "one event per resolved target: {rows:?}"
    );
    assert_eq!(rows[0].index, "alpha", "{rows:?}");
    assert_eq!(
        rows[0].status, "ok",
        "the target that answered keeps its status: {rows:?}"
    );
    assert!(
        samples(&rows[0].latency_dense) > 0,
        "the target that answered keeps its steps: {rows:?}"
    );
    for (row, expected) in rows.iter().zip(TARGETS).skip(1) {
        assert_eq!(row.index, expected, "{rows:?}");
        assert_eq!(row.status, "internal", "{rows:?}");
        assert_empty_steps(row);
    }
}

/// A refusal taken at the single vector/encoding step happens after the
/// targets were resolved: each of them reports it (ADR-0124, decision 10). The
/// first carries the validation, the others no step at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_vector_records_one_event_per_target() {
    let (instance, state, router) = instance_with_targets(0).await;

    let mut body = body();
    body["vector"] = json!(base64_vector([1.0, 0.0]));
    let (status, answer) = send(&router, Method::POST, MULTI, READ_SECRET, Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    assert_eq!(answer["error"]["code"], "invalid_request", "{answer}");

    let rows = flushed_rows(&instance, &state).await;
    assert_eq!(
        rows.len(),
        TARGETS.len(),
        "one event per resolved target: {rows:?}"
    );
    for (row, expected) in rows.iter().zip(TARGETS) {
        assert_eq!(row.index, expected, "{rows:?}");
        assert_eq!(row.status, "invalid_request", "{rows:?}");
        assert_eq!(row.mode, "dense", "the mode was resolved: {rows:?}");
        assert_empty_steps(row);
    }
}

/// A search refused at the concurrency token never reaches one target: all of
/// them carry `overloaded`, with empty steps (ADR-0124, decision 10).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_search_refused_at_the_token_records_one_event_per_target() {
    let (instance, state, router) = instance_with_targets(1).await;
    let limit = state
        .search_limit()
        .expect("the loaded instance exposes its search limit");

    // The closure signals that the token is held, then blocks until released.
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let held = tokio::spawn(async move {
        limit
            .run_blocking(Duration::from_secs(30), move || {
                started_tx.send(()).expect("the test waits for the token");
                release_rx
                    .recv_timeout(Duration::from_secs(30))
                    .expect("the test releases the token");
            })
            .await
    });
    started_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the token is held before the search runs");

    let (status, answer) = send(&router, Method::POST, MULTI, READ_SECRET, Some(body())).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{answer}");
    assert_eq!(answer["error"]["code"], "overloaded", "{answer}");

    release_tx.send(()).expect("the held token is released");
    held.await
        .expect("the holding task finishes")
        .expect("the held token returns");

    let rows = flushed_rows(&instance, &state).await;
    assert_eq!(
        rows.len(),
        TARGETS.len(),
        "one event per resolved target: {rows:?}"
    );
    for (row, expected) in rows.iter().zip(TARGETS) {
        assert_eq!(row.index, expected, "{rows:?}");
        assert_eq!(row.status, "overloaded", "{rows:?}");
        assert_eq!(row.mode, "dense", "the mode was resolved: {rows:?}");
        assert_empty_steps(row);
    }
}
