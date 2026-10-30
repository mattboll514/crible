// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and every helper of this file is a fixture or an assertion.
#![allow(clippy::expect_used, clippy::panic)]

//! A multi-index search beyond `[server].max_concurrent_searches` is refused at
//! once with `503 overloaded` and `Retry-After: 1` (ADR-0102, decision 2;
//! ADR-0110, decision 8).
//!
//! The route takes **one** token for the whole request, whatever the number of
//! targets. The only token of the fixture instance is held here by the limit
//! itself, so the test needs no `failpoints` build: it acquires the token through
//! `SearchLimit::run_blocking` and keeps it until the test releases the blocking
//! closure. The second search finds no token and is refused without waiting.
//!
//! ```text
//! cargo test -p crible-server --test search_multi_overloaded
//! ```

mod common;

use std::sync::mpsc;
use std::time::Duration;

use axum::http::{Method, StatusCode};
use serde_json::json;

use common::{READ_SECRET, base64_vector, instance_with_alpha_and_limit, send_with_headers};

/// Path of the multi-index route.
const MULTI: &str = "/v1/search";

/// A dense search of the fixture index with a long deadline.
fn body() -> serde_json::Value {
    json!({
        "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
        "mode": "dense",
        "timeout_ms": 30_000,
        "targets": [ { "index": "alpha" } ],
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_multi_index_search_beyond_the_limit_is_refused_at_once() {
    let (_instance, state, router) = instance_with_alpha_and_limit(1).await;
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

    let (status, headers, answer) =
        send_with_headers(&router, Method::POST, MULTI, READ_SECRET, Some(body())).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{answer}");
    assert_eq!(answer["error"]["code"], "overloaded", "{answer}");
    assert_eq!(
        headers
            .get("retry-after")
            .and_then(|value| value.to_str().ok()),
        Some("1"),
        "the overloaded answer carries Retry-After: 1"
    );

    release_tx.send(()).expect("the held token is released");
    let outcome = held.await.expect("the holding task finishes");
    assert!(outcome.is_ok(), "the held token returns: {outcome:?}");
    assert_eq!(
        state
            .search_limit()
            .expect("the loaded instance exposes its search limit")
            .available(),
        1,
        "the token comes back when the held work ends"
    );
}
