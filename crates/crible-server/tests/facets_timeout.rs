// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and every helper of this file is a fixture or an assertion.
#![allow(clippy::expect_used, clippy::panic)]
#![cfg(feature = "failpoints")]

//! The deadline of a facets request answers `504`, never a truncated result
//! (ADR-0154, decision 8).
//!
//! The work is held by the `facets` barrier of the `failpoints` build, written
//! **after** the concurrency token is taken. No duration is compared, so the
//! outcome does not depend on the load of the machine: the test knows the work
//! is still running when the deadline of `timeout_ms` expires, and proves the
//! two halves of the decision:
//!
//! 1. the answer is `504 timeout`, with the message of the facets route;
//! 2. the body of that `504` carries neither `count` nor `fields`: a partial
//!    count, and a fortiori a wrong `other`, is never rendered.
//!
//! The token of that `504` stays taken until the work really ends, as for a
//! search (ADR-0102, decision 3): the second call proves it with a `503`
//! `overloaded`. The return of the token is proved by `limits.rs`, and the
//! whole token behaviour of the search route by `search_timeout.rs`.
//!
//! ```text
//! cargo test -p crible-server --features failpoints --test facets_timeout
//! ```

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use axum::http::header::RETRY_AFTER;
use serde_json::{Value, json};

use common::{READ_SECRET, instance_with_alpha_and_limit, send, send_with_headers};

/// Path of the facets route of the fixture index.
const FACETS: &str = "/v1/indexes/alpha/facets";

/// Waits for the barrier to signal that the facets request holds its token.
fn wait_mark(directory: &Path, name: &str) {
    let mark = directory.join(name);
    for _ in 0..2_000 {
        if mark.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "the facets request never reached the barrier mark {}",
        mark.display()
    );
}

/// Releases the barrier when it drops, so a failing assertion fails the test
/// instead of leaving the work waiting for the release bound.
struct Release {
    directory: PathBuf,
    released: bool,
}

impl Release {
    /// Takes the release of the `facets` scope of `directory`.
    fn new(directory: &Path) -> Self {
        Self {
            directory: directory.to_path_buf(),
            released: false,
        }
    }

    /// Writes the mark that wakes the held work, once.
    fn release(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        fs::write(self.directory.join("go-facets"), b"").expect("the work is releasable");
    }
}

impl Drop for Release {
    fn drop(&mut self) {
        self.release();
    }
}

/// A facets request of the fixture field `numero`, with `timeout_ms`.
fn body(timeout_ms: Option<u64>) -> Value {
    match timeout_ms {
        None => json!({ "fields": ["numero"] }),
        Some(milliseconds) => json!({ "fields": ["numero"], "timeout_ms": milliseconds }),
    }
}

/// Sends a request of the facets route and returns its status and body.
async fn call(router: &Router, body: Value) -> (StatusCode, Value) {
    send(
        router,
        axum::http::Method::POST,
        FACETS,
        READ_SECRET,
        Some(body),
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expired_facets_request_is_a_timeout_with_no_partial_result() {
    // The barrier directory is process-global: this binary owns it.
    let _environment = common::lock_barrier_environment().await;
    let barrier = tempfile::tempdir().expect("a barrier directory is creatable");
    // SAFETY: the test holds the barrier environment slot, so no other test of
    // this process reads or writes the variable while it is set.
    unsafe { std::env::set_var("CRIBLE_TEST_BARRIER", barrier.path()) };
    // The work step of the route is opt-in: this test holds it on purpose.
    fs::write(barrier.path().join("armed-facets"), b"").expect("the work step is armable");

    let (_instance, _state, router) = instance_with_alpha_and_limit(1).await;
    let held = tokio::spawn({
        let router = router.clone();
        async move { call(&router, body(Some(1))).await }
    });
    wait_mark(barrier.path(), "ready-facets");
    let mut release = Release::new(barrier.path());

    // The deadline expires while the work is held: the answer names the route,
    // and it carries no count at all (ADR-0154, decision 8).
    let (status, answer) = held.await.expect("the request task finishes");
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT, "{answer}");
    assert_eq!(answer["error"]["code"], "timeout", "{answer}");
    assert_eq!(
        answer["error"]["message"], "the facets request exceeded its deadline (timeout_ms)",
        "{answer}"
    );
    assert!(
        answer.get("count").is_none(),
        "a `504` never carries a count, even partial: {answer}"
    );
    assert!(
        answer.get("fields").is_none(),
        "a `504` never carries values, even partial: {answer}"
    );

    // The `504` did not give the token back: the work is still held, and the
    // refusal does not wait for it.
    let refused = tokio::time::timeout(
        Duration::from_secs(30),
        send_with_headers(
            &router,
            axum::http::Method::POST,
            FACETS,
            READ_SECRET,
            Some(body(None)),
        ),
    )
    .await
    .expect("the refusal does not wait for a token");
    let (status, headers, refusal) = refused;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{refusal}");
    assert_eq!(refusal["error"]["code"], "overloaded", "{refusal}");
    assert_eq!(
        headers.get(RETRY_AFTER),
        Some(&"1".parse().expect("a static header value")),
        "the refusal says when to retry (ADR-0102)"
    );

    // The work ends when the guard releases the barrier, and the test waits for
    // the mark that says the work has left it: the token of the `504` comes back
    // with the end of the work (ADR-0102, decision 3).
    release.release();
    wait_mark(barrier.path(), "left-facets");
}
