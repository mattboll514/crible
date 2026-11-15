// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and every helper of this file is a fixture or an assertion.
#![allow(clippy::expect_used, clippy::panic)]
#![cfg(feature = "failpoints")]

//! The `encode` sample of a multi-index search reaches the analytics **once**,
//! on the event of the first target (ADR-0156, decision 4, and decision 6,
//! point 7).
//!
//! The `failpoints` build accepts a test double at the single encoding point of
//! the server (`crate::search::encode_query`): `CRIBLE_TEST_ENCODER` names a
//! directory, and every real call appends one byte to `<dir>/encoded`. The
//! double answers for the `external` model of the fixture, so no ONNX Runtime is
//! loaded and the request encodes without a model.
//!
//! The rows of `minute_events` are read directly: the histogram of a stage
//! counts its samples, so the sum of `latency_encode` of a row is the number of
//! encodings that row carries. No chronometer is read and no duration is
//! compared: the **count** of the samples is the fact of the decision.
//!
//! | Test | What it proves |
//! |---|---|
//! | `the_encode_sample_of_a_multi_index_search_reaches_the_first_target_only` | one encoding for the request, one sample, on the first target; each target closes its own `total`; the events are written under `operation=search_multi`, never under `search` |
//! | `the_route_renders_the_events_of_a_multi_index_hybrid_search_under_its_mode` | `GET /v1/stats?operation=search_multi&mode=hybrid` renders the two events of the targets (ADR-0156, decision 6, point 8) |
//!
//! ```text
//! cargo test -p crible-server --features failpoints --test stats_search_multi_encoding
//! ```

mod common;

use std::sync::OnceLock;

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::http::build_router;
use crible_server::state::AppState;
use crible_store::analytics::{ANALYTICS_FILE_NAME, MinuteEvent, read_minute_events};
use serde_json::{Value, json};

use common::{ADMIN_SECRET, READ_SECRET, index_config, send};

/// Path of the multi-index search route.
const MULTI: &str = "/v1/search";

/// Directory the double counts in, and the slot that serialises the tests of
/// this binary.
///
/// `CRIBLE_TEST_ENCODER` names one directory per process, so the two tests of
/// this binary take the slot before they send a request and keep it until their
/// end; the counter reads how many encodings the requests before the current one
/// made.
struct Counter {
    directory: tempfile::TempDir,
    slot: tokio::sync::Mutex<()>,
}

impl Counter {
    /// Takes the process-wide slot and returns the number of encodings counted
    /// so far.
    async fn acquire() -> (&'static Self, tokio::sync::MutexGuard<'static, ()>, usize) {
        let counter = COUNTER.get_or_init(|| {
            let directory = tempfile::tempdir().expect("a counter directory is creatable");
            // SAFETY: every test of this binary takes the slot of this value
            // before it sends a request, and the variable is set here, before
            // any request runs.
            unsafe { std::env::set_var("CRIBLE_TEST_ENCODER", directory.path()) };
            Self {
                directory,
                slot: tokio::sync::Mutex::new(()),
            }
        });
        let guard = counter.slot.lock().await;
        let counted = counter.counted();
        (counter, guard, counted)
    }

    /// Number of real encodings the double counted in the directory.
    fn counted(&self) -> usize {
        std::fs::read(self.directory.path().join("encoded"))
            .map(|calls| calls.len())
            .unwrap_or(0)
    }
}

/// The counter of the binary, created by the first test that runs.
static COUNTER: OnceLock<Counter> = OnceLock::new();

/// Creates the index `name` on the fixture model, with the lexical branch, so a
/// `hybrid` search runs both branches.
async fn create(router: &Router, name: &str) {
    let mut body = index_config();
    body["name"] = json!(name);
    body["lexical"]["enabled"] = json!(true);
    let (status, answer) = send(
        router,
        Method::POST,
        "/v1/indexes",
        ADMIN_SECRET,
        Some(body),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the index {name} is created: {answer}"
    );
}

/// An instance whose two indexes serve `hybrid`, with its state and its router.
async fn instance() -> (common::Instance, std::sync::Arc<AppState>, Router) {
    let instance = common::Instance::new();
    let state = instance.state();
    let router = build_router(std::sync::Arc::clone(&state));
    create(&router, "alpha").await;
    create(&router, "beta").await;
    (instance, state, router)
}

/// Sends the `hybrid` search of two targets and returns its answer.
async fn search(router: &Router) -> Value {
    let body = json!({
        "query": "contrat",
        "mode": "hybrid",
        "targets": [ { "index": "alpha" }, { "index": "beta" } ],
    });
    let (status, answer) = send(router, Method::POST, MULTI, READ_SECRET, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "the search runs: {answer}");
    answer
}

/// Sum of the samples of a latency histogram: how many events the row carries
/// for that stage.
fn samples(histogram: &[u32]) -> u32 {
    histogram.iter().copied().sum()
}

/// The single event of `index` for the search of the fixture.
fn event<'a>(rows: &'a [MinuteEvent], index: &str) -> &'a MinuteEvent {
    rows.iter()
        .find(|row| row.index == index && row.operation == "search_multi")
        .unwrap_or_else(|| {
            panic!("the analytics hold an event of {index} under `search_multi`: {rows:?}")
        })
}

/// One `hybrid` search of two indexes calls the encoding once, and its single
/// sample is carried by the event of the first target; each target closes its
/// own `total`, and the events are written under `operation=search_multi`, never
/// under `search` (ADR-0156, decision 4, and decision 6, points 7 and 8).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_encode_sample_of_a_multi_index_search_reaches_the_first_target_only() {
    let (counter, _slot, before) = Counter::acquire().await;
    let (instance, state, router) = instance().await;

    let answer = search(&router).await;
    assert_eq!(
        counter.counted(),
        before + 1,
        "the query is encoded once for the two targets: {answer}"
    );

    state
        .flush_analytics()
        .await
        .expect("the events of the search are flushed");

    let path = instance.root().join("data").join(ANALYTICS_FILE_NAME);
    let rows = read_minute_events(&path).expect("the analytics file is readable");
    let (alpha, beta) = (event(&rows, "alpha"), event(&rows, "beta"));
    for row in [alpha, beta] {
        assert_eq!(row.mode, "hybrid", "the row keeps the mode served: {row:?}");
        assert_eq!(row.count, 1, "one event per target: {row:?}");
        assert_eq!(
            samples(&row.latency_total),
            1,
            "each target closes its own total: {row:?}"
        );
    }
    assert_eq!(
        samples(&alpha.latency_encode),
        1,
        "the first target carries the single encoding of the request: {alpha:?}"
    );
    assert_eq!(
        samples(&beta.latency_encode),
        0,
        "the second target carries no encoding sample: {beta:?}"
    );

    // The events are readable under the operation the multi-index route writes.
    let (status, filtered) = send(
        &router,
        Method::GET,
        "/v1/stats?operation=search_multi",
        ADMIN_SECRET,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the statistics answer: {filtered}");
    assert_eq!(
        filtered["totals"]["count"], 2,
        "one event per target is rendered: {filtered}"
    );

    // Nothing is written under `search` by the multi-index route.
    let (status, without) = send(
        &router,
        Method::GET,
        "/v1/stats?operation=search",
        ADMIN_SECRET,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the statistics answer: {without}");
    assert_eq!(
        without["buckets"],
        json!([]),
        "the multi-index route writes no `search` event: {without}"
    );
}

/// The route renders the events of a multi-index `hybrid` search once the filter
/// names its operation and its mode (ADR-0156, decision 6, point 8).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_route_renders_the_events_of_a_multi_index_hybrid_search_under_its_mode() {
    let (_counter, _slot, _before) = Counter::acquire().await;
    let (_instance, state, router) = instance().await;

    let answer = search(&router).await;
    assert_eq!(
        answer["targets"].as_array().map(Vec::len),
        Some(2),
        "both targets ran: {answer}"
    );

    state
        .flush_analytics()
        .await
        .expect("the events of the search are flushed");

    let (status, filtered) = send(
        &router,
        Method::GET,
        "/v1/stats?operation=search_multi&mode=hybrid",
        ADMIN_SECRET,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the statistics answer: {filtered}");
    assert_eq!(
        filtered["totals"]["count"], 2,
        "one event per target is rendered: {filtered}"
    );
    assert!(
        filtered["totals"]["latency_ms"]["encode"].is_object(),
        "the encoding of the request is readable on the first target: {filtered}"
    );
    assert!(
        filtered["totals"]["latency_ms"]["total"].is_object(),
        "each target closes its own total: {filtered}"
    );
}
