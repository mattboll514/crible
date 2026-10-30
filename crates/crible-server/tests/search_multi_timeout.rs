// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and every helper of this file is a fixture or an assertion.
#![allow(clippy::expect_used, clippy::panic)]
#![cfg(feature = "failpoints")]

//! The deadline of a multi-index search covers the whole request, across every
//! target (ADR-0102, decision 3; ADR-0110, decision 8).
//!
//! The search is held by the `search-multi` barrier of the `failpoints` build,
//! written **after** the single token is taken. The test knows the token is held
//! when the barrier is ready, and proves that the deadline expires while the work
//! is held and that the route answers `504 timeout`.
//!
//! ```text
//! cargo test -p crible-server --features failpoints --test search_multi_timeout
//! ```

mod common;

use std::path::Path;
use std::time::Duration;

use axum::Router;
use axum::http::{Method, StatusCode};
use serde_json::json;

use common::{READ_SECRET, base64_vector, instance_with_alpha_and_limit, send};

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
fn body(timeout_ms: u64) -> serde_json::Value {
    json!({
        "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
        "mode": "dense",
        "timeout_ms": timeout_ms,
        "targets": [ { "index": "alpha" } ],
    })
}

/// Starts one multi-index search in a background task.
fn spawn_search(
    router: &Router,
    timeout_ms: u64,
) -> tokio::task::JoinHandle<(StatusCode, serde_json::Value)> {
    let router = router.clone();
    tokio::spawn(async move {
        send(
            &router,
            Method::POST,
            MULTI,
            READ_SECRET,
            Some(body(timeout_ms)),
        )
        .await
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_multi_index_search_past_its_deadline_answers_504() {
    let barrier = tempfile::tempdir().expect("a barrier directory is creatable");
    // SAFETY: this integration test binary sets the variable before spawning any
    // request, and no other test of this process reads or writes it.
    unsafe { std::env::set_var("CRIBLE_TEST_BARRIER", barrier.path()) };

    let (_instance, _state, router) = instance_with_alpha_and_limit(1).await;

    let first = spawn_search(&router, 1);
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
