// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and every helper of this file is a fixture or an assertion.
#![allow(clippy::expect_used, clippy::panic)]
#![cfg(feature = "failpoints")]

//! The deadline of a multi-index search covers the whole request, across every
//! target (ADR-0102, decision 3; ADR-0110, decision 8), and a `504` records one
//! event per resolved target with empty steps (ADR-0124, decision 10).
//!
//! The search is held by the `search-multi` barrier of the `failpoints` build,
//! written **after** the single token is taken. The test knows the token is held
//! when the barrier is ready, and proves that the deadline expires while the work
//! is held and that the route answers `504 timeout`.
//!
//! | Test | What it proves |
//! |---|---|
//! | `a_multi_index_search_past_its_deadline_answers_504` | one token for the whole request, `504 timeout` |
//! | `a_multi_index_deadline_records_one_event_per_target` | two targets, two events, empty steps, no pool |
//!
//! ```text
//! cargo test -p crible-server --features failpoints --test search_multi_timeout
//! ```

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::http::build_router;
use crible_server::state::AppState;
use crible_store::analytics::{ANALYTICS_FILE_NAME, MinuteEvent, read_minute_events};
use serde_json::{Value, json};

use common::{
    ADMIN_SECRET, Instance, READ_SECRET, WRITE_SECRET, base64_vector, index_config,
    instance_with_alpha_and_limit, lock_barrier_environment, send, terminal_job,
};

/// Path of the multi-index route.
const MULTI: &str = "/v1/search";

/// Waits for the barrier to signal that the search holds its token.
fn wait_ready(directory: &Path, scope: &str) {
    let ready = directory.join(format!("ready-{scope}"));
    for _ in 0..2_000 {
        if ready.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "the search never reached the barrier at {}",
        ready.display()
    );
}

/// A dense multi-index search with `timeout_ms` milliseconds.
fn body(timeout_ms: u64) -> Value {
    json!({
        "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
        "mode": "dense",
        "timeout_ms": timeout_ms,
        "targets": [ { "index": "alpha" } ],
    })
}

/// The same search over the two targets of [`two_indexes`].
fn two_target_body(timeout_ms: u64) -> Value {
    let mut body = body(timeout_ms);
    body["targets"] = json!([ { "index": "alpha" }, { "index": "beta" } ]);
    body
}

/// Starts one multi-index search in a background task.
fn spawn_search(router: &Router, body: Value) -> tokio::task::JoinHandle<(StatusCode, Value)> {
    let router = router.clone();
    tokio::spawn(async move { send(&router, Method::POST, MULTI, READ_SECRET, Some(body)).await })
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

/// An instance that serves the two targets `alpha` and `beta`.
async fn two_indexes() -> (Instance, Arc<AppState>, Router) {
    let instance = Instance::new();
    let state = instance.state();
    let router = build_router(Arc::clone(&state));
    for name in ["alpha", "beta"] {
        create_index(&router, name).await;
        attach_one(&router, &state, name).await;
    }
    (instance, state, router)
}

/// Number of samples of a latency histogram.
fn samples(histogram: &[u32]) -> u64 {
    histogram.iter().map(|bucket| u64::from(*bucket)).sum()
}

/// The multi-index rows of the analytics file, after a forced flush.
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_multi_index_search_past_its_deadline_answers_504() {
    // One barrier directory per process: the tests of this binary take turns.
    let _guard = lock_barrier_environment().await;
    let barrier = tempfile::tempdir().expect("a barrier directory is creatable");
    // SAFETY: this integration test binary sets the variable before spawning any
    // request, and no other test of this process reads or writes it.
    unsafe { std::env::set_var("CRIBLE_TEST_BARRIER", barrier.path()) };
    // The work step of the multi-index route is opt-in: this test holds it.
    std::fs::write(barrier.path().join("armed-search-multi"), b"")
        .expect("the work step is armable");

    let (_instance, _state, router) = instance_with_alpha_and_limit(1).await;

    let first = spawn_search(&router, body(1));
    wait_ready(barrier.path(), "search-multi");

    let (status, first_body) = first.await.expect("the request task finishes");
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT, "{first_body}");
    assert_eq!(first_body["error"]["code"], "timeout", "{first_body}");

    // Release the work so the blocking thread ends before the process does.
    std::fs::write(barrier.path().join("go-search-multi"), b"").expect("the barrier is released");

    // The token comes back only when the computation really ends: the next
    // search answers `200` once the released work returned (ADR-0102, decision 3).
    let mut last = serde_json::Value::Null;
    let mut status = StatusCode::SERVICE_UNAVAILABLE;
    for _ in 0..400 {
        let (candidate, answer) = send(
            &router,
            Method::POST,
            MULTI,
            READ_SECRET,
            Some(body(30_000)),
        )
        .await;
        status = candidate;
        last = answer;
        if status == StatusCode::OK {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(status, StatusCode::OK, "the token comes back: {last}");

    // SAFETY: the test owns the variable until its end, as for `set_var`.
    unsafe { std::env::remove_var("CRIBLE_TEST_BARRIER") };
}

/// A `504` of a two-target request records **one event per resolved target**,
/// each with the deadline as its status, the resolved `mode`, and empty steps:
/// no duration and no pool size is invented for work that did not happen
/// (ADR-0124, decision 10; ADR-0117, decision 5).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_multi_index_deadline_records_one_event_per_target() {
    let _guard = lock_barrier_environment().await;
    let barrier = tempfile::tempdir().expect("a barrier directory is creatable");
    // SAFETY: this test holds `lock_barrier_environment`, which serialises every
    // test of the process that arms the variable.
    unsafe { std::env::set_var("CRIBLE_TEST_BARRIER", barrier.path()) };
    std::fs::write(barrier.path().join("armed-search-multi"), b"")
        .expect("the work step is armable");

    let (instance, state, router) = two_indexes().await;

    let pending = spawn_search(&router, two_target_body(1));
    wait_ready(barrier.path(), "search-multi");
    let (status, answer) = pending.await.expect("the request task finishes");
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT, "{answer}");
    assert_eq!(answer["error"]["code"], "timeout", "{answer}");

    std::fs::write(barrier.path().join("go-search-multi"), b"").expect("the barrier is released");

    let rows = flushed_rows(&instance, &state).await;
    assert_eq!(
        rows.len(),
        2,
        "one event per resolved target, never a consolidated row: {rows:?}"
    );
    for (row, expected) in rows.iter().zip(["alpha", "beta"]) {
        assert_eq!(row.index, expected, "{rows:?}");
        assert_eq!(row.status, "timeout", "{rows:?}");
        assert_eq!(row.mode, "dense", "the mode was resolved: {rows:?}");
        assert_eq!(row.count, 1, "{rows:?}");
        // The event carries no pool size for work that did not run: the sample
        // is zero, never a fabricated count (ADR-0124, decision 10).
        assert!(
            row.pool
                .get(1..)
                .is_some_and(|above| above.iter().all(|bucket| *bucket == 0)),
            "no pool size is invented: {rows:?}"
        );
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
                "the step {step} stays empty: {rows:?}"
            );
        }
    }

    // SAFETY: the test owns the variable until its end, as for `set_var`.
    unsafe { std::env::remove_var("CRIBLE_TEST_BARRIER") };
}
