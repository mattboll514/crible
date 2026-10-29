// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and every helper of this file is a fixture or an assertion.
#![allow(clippy::expect_used, clippy::panic)]
#![cfg(feature = "failpoints")]

//! `504 timeout` of a search keeps its concurrency token until the computation
//! really ends (ADR-0102, decision 3; ADR-0109, decision 2).
//!
//! The search is held by the `search` barrier of the `failpoints` build, written
//! **after** the token is taken. The test therefore knows the token is held when
//! the barrier is ready, and proves the two halves of the decision:
//!
//! 1. the deadline expires while the work is held, and the route answers
//!    `504 timeout`;
//! 2. after that `504`, a second search still finds no token (`503 overloaded`),
//!    so the token did not come back at the deadline;
//! 3. when the barrier is released, the token comes back and the next search
//!    answers `200`.
//!
//! ```text
//! cargo test -p crible-server --features failpoints --test search_timeout
//! ```

mod common;

use std::path::Path;
use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use serde_json::{Value, json};

use common::{READ_SECRET, base64_vector, instance_with_alpha_and_limit, send, send_with_headers};

/// Path of the search route of the fixture index.
const SEARCH: &str = "/v1/indexes/alpha/search";

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

/// A dense search with `timeout_ms` milliseconds.
fn body(timeout_ms: u64) -> Value {
    json!({
        "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
        "mode": "dense",
        "timeout_ms": timeout_ms,
    })
}

/// Starts one search in a background task.
fn spawn_search(router: &Router, timeout_ms: u64) -> tokio::task::JoinHandle<(StatusCode, Value)> {
    let router = router.clone();
    tokio::spawn(async move {
        send(
            &router,
            axum::http::Method::POST,
            SEARCH,
            READ_SECRET,
            Some(body(timeout_ms)),
        )
        .await
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expired_search_keeps_its_token_until_the_computation_really_ends() {
    let barrier = tempfile::tempdir().expect("a barrier directory is creatable");
    // SAFETY: this integration test binary sets the variable before spawning any
    // request, and no other test of this process reads or writes it.
    unsafe { std::env::set_var("CRIBLE_TEST_BARRIER", barrier.path()) };

    let (_instance, _state, router) = instance_with_alpha_and_limit(1).await;

    let first = spawn_search(&router, 1);
    wait_ready(barrier.path(), "search");

    // The deadline expires while the work is held: `504 timeout`.
    let (status, first_body) = first.await.expect("the request task finishes");
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT, "{first_body}");
    assert_eq!(first_body["error"]["code"], "timeout", "{first_body}");

    // The `504` did not give the token back: the work is still held.
    let (status, headers, overloaded) = send_with_headers(
        &router,
        axum::http::Method::POST,
        SEARCH,
        READ_SECRET,
        Some(body(30_000)),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{overloaded}");
    assert_eq!(overloaded["error"]["code"], "overloaded", "{overloaded}");
    assert_eq!(
        headers
            .get("retry-after")
            .and_then(|value| value.to_str().ok()),
        Some("1"),
        "the overloaded answer carries Retry-After: 1"
    );

    // Release the work: the token comes back when the computation ends.
    std::fs::write(barrier.path().join("go-search"), b"").expect("the barrier is released");
    let mut last = Value::Null;
    let mut status = StatusCode::SERVICE_UNAVAILABLE;
    for _ in 0..400 {
        let (candidate, answer) = send(
            &router,
            axum::http::Method::POST,
            SEARCH,
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
    assert_eq!(last["pool_size"], 0, "{last}");

    // SAFETY: the test owns the variable until its end, as for `set_var`.
    unsafe { std::env::remove_var("CRIBLE_TEST_BARRIER") };
}
