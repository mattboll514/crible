// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and every helper of this file is a fixture or an assertion.
#![allow(clippy::expect_used, clippy::panic)]

//! `GET /v1/stats` (`ROUTE-STATS`, phase P7; ADR-0124).
//!
//! | Test | Acceptance criterion |
//! |---|---|
//! | `only_the_admin_role_reaches_the_route` | `admin` only, `403` for `write` and `read` (ADR-0124, decision 16) |
//! | `the_answer_has_the_members_of_the_route` | shape of the answer, `analytics_dropped` from the process (decisions 1 and 14) |
//! | `the_members_without_a_value_are_omitted` | absent `errors`, `zero_results_rate` and `latency_ms` (decision 1) |
//! | `a_parameter_outside_its_domain_is_a_400_naming_it` | closed list, `group_by` and `operation` domains (decision 2) |
//! | `an_index_without_an_event_is_an_empty_200` | `200` with no bucket and no totals (decision 3) |
//! | `a_bounded_window_echoes_its_bounds` | `from` and `to` echoed (decision 2) |
//!
//! The analytics are written **before** the state starts its writers: the route
//! only reads `analytics.sqlite`, so a known traffic reaches it without driving
//! the search routes. The fixture writes in the current minute and the current
//! day, which is what the store attaches a row to (ADR-0118, decision 3).
//!
//! ```text
//! cargo test -p crible-server --test routes_stats
//! ```

mod common;

use std::sync::Arc;

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::analytics::Operation;
use crible_server::http::build_router;
use crible_server::state::AppState;
use crible_store::analytics::{
    ANALYTICS_FILE_NAME, AnalyticsStore, DayIndex, MinuteEvent, OPERATIONS, ShapeCount,
};
use serde_json::{Value, json};

use common::{ADMIN_SECRET, Instance, READ_SECRET, WRITE_SECRET, send};

/// Path of the statistics route.
const STATS: &str = "/v1/stats";

/// Sends `query` on the statistics route with `secret`, `query` starting with
/// the `?` or empty.
async fn stats(router: &Router, secret: &str, query: &str) -> (StatusCode, Value) {
    let path = format!("{STATS}{query}");
    send(router, Method::GET, &path, secret, None).await
}

/// Path of the analytics file of `instance`.
fn analytics_path(instance: &Instance) -> std::path::PathBuf {
    instance.root().join("data").join(ANALYTICS_FILE_NAME)
}

/// A `search` row of the fixture, with one sample per stage.
fn search_row(index: &str, status: &str, count: u64, zero_results: u64) -> MinuteEvent {
    let mut row = MinuteEvent::empty(index, "search", status);
    row.client = "reader".to_owned();
    row.mode = "dense".to_owned();
    row.dense_strategy = "exact".to_owned();
    row.count = count;
    row.zero_results = zero_results;
    // Bucket 3 is the first bucket above 0.2 ms: the exact value never matters,
    // only that the stage carries a sample.
    for stage in 0..9 {
        row.add_latency(stage, 3);
    }
    row
}

/// Writes the analytics of the fixture: two `alpha` rows, one refusal, one
/// filter form and one daily row.
fn seed_analytics(instance: &Instance) {
    let store = AnalyticsStore::open_or_create(&analytics_path(instance))
        .expect("the analytics file is creatable");
    let minute = vec![
        search_row("alpha", "ok", 3, 1),
        search_row("alpha", "invalid_filter", 1, 0),
    ];
    let shapes = vec![ShapeCount {
        index: "alpha".to_owned(),
        shape: "fonds:eq".to_owned(),
        count: 2,
    }];
    let day = DayIndex {
        index: "alpha".to_owned(),
        documents: 7,
        live_passages: 21,
        dead_passages: 2,
        disk_bytes: 4096,
        generation: 5,
        batches: 9,
        ..DayIndex::default()
    };
    store
        .write(&minute, &shapes, &[day])
        .expect("the analytics of the fixture are written");
}

/// An instance whose analytics already hold the fixture traffic, with its
/// router.
fn seeded() -> (Instance, Arc<AppState>, Router) {
    let instance = Instance::new();
    seed_analytics(&instance);
    let state = instance.state();
    let router = build_router(Arc::clone(&state));
    (instance, state, router)
}

/// An instance whose only row is a successful operation with no search
/// dimension: no error, no successful search, no stage.
fn without_search() -> (Instance, Arc<AppState>, Router) {
    let instance = Instance::new();
    let store = AnalyticsStore::open_or_create(&analytics_path(&instance))
        .expect("the analytics file is creatable");
    let mut row = MinuteEvent::empty("alpha", "compact", "ok");
    row.count = 1;
    store
        .write(&[row], &[], &[])
        .expect("the operation is written");
    drop(store);
    let state = instance.state();
    let router = build_router(Arc::clone(&state));
    (instance, state, router)
}

/// The route is `admin` only; `write` and `read` are refused with `403`
/// (ADR-0095; ADR-0124, decision 16).
#[tokio::test]
async fn only_the_admin_role_reaches_the_route() {
    let (_instance, _state, router) = seeded();
    let (status, answer) = stats(&router, ADMIN_SECRET, "").await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    for secret in [WRITE_SECRET, READ_SECRET] {
        let (status, answer) = stats(&router, secret, "").await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{answer}");
        assert_eq!(answer["error"]["code"], "forbidden", "{answer}");
    }
}

/// The answer carries the members of ADR-0124, decision 1, with
/// `analytics_dropped` read from the process (decision 14).
#[tokio::test]
async fn the_answer_has_the_members_of_the_route() {
    let (_instance, _state, router) = seeded();
    let (status, answer) = stats(&router, ADMIN_SECRET, "").await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    assert_eq!(answer["group_by"], "hour", "the default width: {answer}");
    assert!(
        answer["from"].is_string() && answer["to"].is_string(),
        "the window defaults to the retention of the file: {answer}"
    );

    let buckets = answer["buckets"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer holds buckets: {answer}"));
    assert_eq!(buckets.len(), 1, "one hour holds the fixture: {answer}");
    assert_eq!(buckets[0]["count"], 4, "{answer}");
    assert_eq!(
        buckets[0]["errors"]["invalid_filter"], 1,
        "the refusal is counted under its code: {answer}"
    );
    assert_eq!(
        buckets[0]["zero_results_rate"], 0.3333333333333333,
        "one of the three successful searches returned nothing: {answer}"
    );
    assert!(
        buckets[0]["latency_ms"]["total"]["p50"].is_number(),
        "the percentiles are rendered: {answer}"
    );

    assert_eq!(answer["totals"]["count"], 4, "{answer}");
    assert_eq!(answer["totals"]["errors"]["invalid_filter"], 1, "{answer}");

    assert_eq!(answer["top_indexes"][0]["index"], "alpha", "{answer}");
    assert_eq!(answer["top_indexes"][0]["count"], 4, "{answer}");
    assert_eq!(
        answer["top_filter_shapes"][0]["shape"], "fonds:eq",
        "{answer}"
    );
    assert_eq!(answer["top_filter_shapes"][0]["count"], 2, "{answer}");

    let indexes = answer["indexes"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer holds the daily state: {answer}"));
    assert_eq!(indexes.len(), 1, "{answer}");
    assert_eq!(indexes[0]["index"], "alpha", "{answer}");
    assert_eq!(indexes[0]["generation"], 5, "{answer}");
    assert_eq!(indexes[0]["live_passages"], 21, "{answer}");

    // No event was dropped in this process: the member is present, and it comes
    // from the process rather than from the file (decision 14).
    assert_eq!(answer["analytics_dropped"], 0, "{answer}");
}

/// A member without a value is omitted, never rendered as an empty object or a
/// null (ADR-0124, decision 1).
#[tokio::test]
async fn the_members_without_a_value_are_omitted() {
    let (_instance, _state, router) = without_search();

    let (status, answer) = stats(&router, ADMIN_SECRET, "").await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let totals = &answer["totals"];
    assert_eq!(totals["count"], 1, "{answer}");
    for member in ["errors", "zero_results_rate", "latency_ms"] {
        assert!(
            totals.get(member).is_none(),
            "the member '{member}' is omitted: {answer}"
        );
    }
    let bucket = &answer["buckets"][0];
    for member in ["errors", "zero_results_rate", "latency_ms"] {
        assert!(
            bucket.get(member).is_none(),
            "the member '{member}' of a bucket is omitted: {answer}"
        );
    }
}

/// The list of parameters is closed, and `group_by` and `operation` have a
/// closed domain; the refusal names the parameter (ADR-0124, decision 2).
#[tokio::test]
async fn a_parameter_outside_its_domain_is_a_400_naming_it() {
    let (_instance, _state, router) = seeded();
    for (query, named) in [
        ("?group_by=week", "group_by"),
        ("?operation=bidon", "operation"),
        ("?index=../alpha", "index"),
        ("?inconnu=1", "inconnu"),
        ("?from=hier", "from"),
        ("?from=2026-01-02T00:00:00Z&to=2026-01-01T00:00:00Z", "from"),
    ] {
        let (status, answer) = stats(&router, ADMIN_SECRET, query).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}: {answer}");
        assert_eq!(answer["error"]["code"], "invalid_request", "{query}");
        let message = answer["error"]["message"]
            .as_str()
            .unwrap_or_else(|| panic!("the refusal carries a message: {answer}"));
        assert!(
            message.contains(named),
            "the refusal of '{query}' names '{named}': {message}"
        );
    }
}

/// An index present in the file but without an event in the window answers
/// `200` with no bucket and no totals (ADR-0124, decision 3).
#[tokio::test]
async fn an_index_without_an_event_is_an_empty_200() {
    let (_instance, _state, router) = seeded();
    let (status, answer) = stats(&router, ADMIN_SECRET, "?index=absent").await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["group_by"], "hour", "{answer}");
    assert_eq!(answer["buckets"], json!([]), "{answer}");
    assert!(
        answer.get("totals").is_none(),
        "an empty window has no totals: {answer}"
    );
    assert_eq!(answer["top_indexes"], json!([]), "{answer}");
    assert_eq!(answer["indexes"], json!([]), "{answer}");
}

/// The closed list of `crible-store` is the one the server writes on the wire: a
/// rename in `Operation::as_str` fails here instead of turning a valid
/// `operation` into a `400` (ADR-0124, decision 2).
#[test]
fn the_closed_operation_list_is_the_server_enum() {
    let server = [
        Operation::Search,
        Operation::SearchMulti,
        Operation::GetDocument,
        Operation::OnlineBatch,
        Operation::PatchDocuments,
        Operation::AttachBatch,
        Operation::Compact,
        Operation::Export,
        Operation::Snapshot,
        Operation::IndexAdmin,
    ];
    assert_eq!(
        server.len(),
        OPERATIONS.len(),
        "the two lists name the same number of operations"
    );
    for operation in server {
        assert!(
            OPERATIONS.contains(&operation.as_str()),
            "the store list misses the server name '{}'",
            operation.as_str()
        );
    }
    for name in OPERATIONS {
        assert!(
            server.iter().any(|operation| operation.as_str() == name),
            "the server enum misses the stored name '{name}'"
        );
    }
}

/// A bounded window echoes the two bounds it applied, both included
/// (ADR-0124, decisions 2 and 3).
#[tokio::test]
async fn a_bounded_window_echoes_its_bounds() {
    let (_instance, _state, router) = seeded();
    let query = "?from=2026-01-01T00:00:00Z&to=2026-12-31T23:59:00Z";
    let (status, answer) = stats(&router, ADMIN_SECRET, query).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["from"], "2026-01-01T00:00:00Z", "{answer}");
    assert_eq!(answer["to"], "2026-12-31T23:59:00Z", "{answer}");
    assert_eq!(answer["buckets"][0]["count"], 4, "{answer}");
}
