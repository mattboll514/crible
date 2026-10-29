// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and every helper of this file is a fixture or an assertion.
#![allow(clippy::expect_used, clippy::panic)]
#![cfg(feature = "failpoints")]

//! A search beyond `[server].max_concurrent_searches` is refused at once with
//! `503 overloaded` and `Retry-After: 1` (ADR-0102, decision 2).
//!
//! The only token of the fixture instance is held by a search stopped at the
//! `search` barrier of the `failpoints` build. The second search finds no token
//! and is refused without waiting; the answer carries `Retry-After: 1`.
//!
//! ```text
//! cargo test -p crible-server --features failpoints --test search_overloaded
//! ```

mod common;

use std::path::Path;
use std::time::Duration;

use axum::http::StatusCode;
use serde_json::{Value, json};

use common::{READ_SECRET, base64_vector, instance_with_alpha_and_limit, send, send_with_headers};

/// Path of the search route of the fixture index.
const SEARCH: &str = "/v1/indexes/alpha/search";

/// Waits for the barrier to signal that the first search holds the token.
fn wait_ready(directory: &Path) {
    let ready = directory.join("ready-search");
    for _ in 0..2_000 {
        if ready.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "the first search never reached the barrier at {}",
        ready.display()
    );
}

/// A dense search with a long deadline.
fn body() -> Value {
    json!({
        "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
        "mode": "dense",
        "timeout_ms": 30_000,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_search_beyond_the_limit_is_refused_at_once_with_retry_after() {
    let barrier = tempfile::tempdir().expect("a barrier directory is creatable");
    // SAFETY: this integration test binary sets the variable before spawning any
    // request, and no other test of this process reads or writes it.
    unsafe { std::env::set_var("CRIBLE_TEST_BARRIER", barrier.path()) };

    let (_instance, _state, router) = instance_with_alpha_and_limit(1).await;

    let held = {
        let router = router.clone();
        tokio::spawn(async move {
            send(
                &router,
                axum::http::Method::POST,
                SEARCH,
                READ_SECRET,
                Some(body()),
            )
            .await
        })
    };
    wait_ready(barrier.path());

    let (status, headers, answer) = send_with_headers(
        &router,
        axum::http::Method::POST,
        SEARCH,
        READ_SECRET,
        Some(body()),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{answer}");
    assert_eq!(answer["error"]["code"], "overloaded", "{answer}");
    assert_eq!(
        headers
            .get("retry-after")
            .and_then(|value| value.to_str().ok()),
        Some("1"),
        "the overloaded answer carries Retry-After: 1"
    );

    std::fs::write(barrier.path().join("go-search"), b"").expect("the barrier is released");
    let (status, held_body) = held.await.expect("the held search finishes");
    assert_eq!(status, StatusCode::OK, "{held_body}");

    // SAFETY: the test owns the variable until its end, as for `set_var`.
    unsafe { std::env::remove_var("CRIBLE_TEST_BARRIER") };
}
