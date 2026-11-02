// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and every helper of this file is a fixture or an assertion.
#![allow(clippy::expect_used, clippy::panic)]
#![cfg(feature = "failpoints")]

//! A `504 timeout` records the steps it really measured (ADR-0122, decision 5).
//!
//! The search is held by the `search` barrier of the `failpoints` build, so the
//! deadline expires after the validation and before any branch: the row of the
//! `504` must carry `timings_ms` with `validate`, and must omit the steps that
//! were never measured, which are not rendered `0.0`. The plan and the capture
//! are absent, and `total_ms` stays `NULL` (ADR-0121, decision 3).
//!
//! ```text
//! cargo test -p crible-server --features failpoints --test debug_journal_timeout
//! ```

mod common;

use std::path::Path;
use std::time::Duration;

use axum::http::StatusCode;
use crible_store::debug_journal::{DEBUG_JOURNAL_FILE_NAME, read_queries};
use serde_json::{Value, json};

use common::{Journal, READ_SECRET, base64_vector, instance_with_alpha_and_journal, send};

/// Path of the search route of the fixture index.
const SEARCH: &str = "/v1/indexes/alpha/search";

/// Waits for the barrier to write the mark `name` of `directory`.
///
/// The loop is bounded: a test that waits for a mark its job never writes fails
/// with the path it wanted, instead of hanging the suite.
fn wait_mark(directory: &Path, name: &str) {
    let mark = directory.join(name);
    for _ in 0..2_000 {
        if mark.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "the search never reached the barrier mark at {}",
        mark.display()
    );
}

/// A dense search with a one-millisecond deadline, asking for `debug` so the row
/// is written even with `sample_rate = 0` (ADR-0122, decision 1).
fn body() -> Value {
    json!({
        "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
        "mode": "dense",
        "timeout_ms": 1,
        "debug": true,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_timeout_records_the_steps_it_measured() {
    // The binary of this file owns the process-global barrier directory.
    let _environment = common::lock_barrier_environment().await;
    let barrier = tempfile::tempdir().expect("a barrier directory is creatable");
    // SAFETY: the test holds the barrier environment slot, so no other test of
    // this process reads or writes the variable while it is set.
    unsafe { std::env::set_var("CRIBLE_TEST_BARRIER", barrier.path()) };
    // The work step of the route is opt-in: this test holds it on purpose.
    std::fs::write(barrier.path().join("armed-search"), b"").expect("the work step is armable");

    let (instance, state, router) = instance_with_alpha_and_journal(Journal::enabled(0.0)).await;
    let search_router = router.clone();
    let held = tokio::spawn(async move {
        send(
            &search_router,
            axum::http::Method::POST,
            SEARCH,
            READ_SECRET,
            Some(body()),
        )
        .await
    });
    wait_mark(barrier.path(), "ready-search");

    let (status, answer) = held.await.expect("the request task finishes");
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT, "{answer}");
    assert_eq!(answer["error"]["code"], "timeout", "{answer}");
    assert!(
        answer.get("debug").is_none(),
        "a `504` never carries `debug` (ADR-0119, decision 10): {answer}"
    );

    state
        .flush_debug_journal()
        .await
        .expect("the debug journal is flushed");

    let rows = read_queries(&instance.root().join("data").join(DEBUG_JOURNAL_FILE_NAME))
        .expect("the debug journal file is readable");
    assert_eq!(rows.len(), 1, "one row for the `504`: {rows:?}");
    let row = &rows[0];
    assert_eq!(
        row.status, "timeout",
        "the row names the code of the answer"
    );
    assert!(
        row.total_ms.is_none(),
        "a `504` keeps `total_ms` NULL (ADR-0121, decision 3): {row:?}"
    );
    let payload: Value =
        serde_json::from_str(&row.payload).expect("the payload of a row is JSON text");
    assert_eq!(payload["error"]["code"], "timeout", "{payload}");
    let timings = payload["timings_ms"]
        .as_object()
        .unwrap_or_else(|| panic!("`timings_ms` is an object: {payload}"));
    assert!(
        timings.contains_key("validate"),
        "the measured validation is recorded (ADR-0122, decision 5): {payload}"
    );
    for absent in [
        "encode", "filter", "dense", "lexical", "fusion", "group", "fetch", "total",
    ] {
        assert!(
            !timings.contains_key(absent),
            "an unmeasured step is omitted, never `0.0` ({absent}): {payload}"
        );
    }
    assert!(
        payload.get("plan").is_none(),
        "a `504` records no plan: {payload}"
    );
    assert!(
        payload.get("capture").is_none(),
        "a `504` records no capture: {payload}"
    );

    // The held search is released, and the test outlives the wake-up of its
    // barrier: it waits for the mark the job writes once it has left the hold
    // point, so the directory stays alive until the work has left it, whatever
    // the load of the machine (NETTOYER-P7).
    std::fs::write(barrier.path().join("go-search"), b"").expect("the barrier is released");
    wait_mark(barrier.path(), "left-search");

    // SAFETY: the test owns the variable until its end, as for `set_var`.
    unsafe { std::env::remove_var("CRIBLE_TEST_BARRIER") };
}
