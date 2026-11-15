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
//! | `a_value_outside_the_mode_domain_is_a_400_naming_the_parameter` | closed `mode` domain, message of ADR-0156, decision 1 |
//! | `the_mode_filter_restricts_the_buckets_and_the_totals` | `mode` restricts `count`, `errors` and `zero_results_rate` (ADR-0156, decision 2) |
//! | `the_mode_filter_restricts_the_top_indexes` | `mode` restricts `top_indexes` (ADR-0156, decision 2) |
//! | `two_modes_of_one_window_have_distinct_percentiles_and_their_counts_add_up` | two modes read apart, and their counts add up to the answer without `mode` (ADR-0156, decision 6, points 1 and 2) |
//! | `a_row_without_a_mode_is_reachable_without_a_mode_only` | the rows written before the mode was resolved are excluded by an explicit `mode` (ADR-0156, decision 1) |
//! | `the_members_without_a_mode_column_are_the_same_with_and_without_a_mode` | `top_filter_shapes`, `indexes`, `analytics_dropped` and the echoes do not follow the filter (ADR-0156, decision 2) |
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
    mode_row(index, "search", status, "dense", count, zero_results, 3)
}

/// A row of `index` carrying `count` events of `mode`, with one sample of every
/// stage in histogram `bucket`.
///
/// The bucket is what a mode renders as its percentiles, so two modes seeded in
/// two buckets read two distinct values (ADR-0117, decision 4). An empty `mode`
/// is the row of an operation whose mode was never resolved (ADR-0156,
/// decision 1).
fn mode_row(
    index: &str,
    operation: &str,
    status: &str,
    mode: &str,
    count: u64,
    zero_results: u64,
    bucket: usize,
) -> MinuteEvent {
    let mut row = MinuteEvent::empty(index, operation, status);
    row.client = "reader".to_owned();
    row.mode = mode.to_owned();
    row.dense_strategy = if mode.is_empty() || mode == "lexical" {
        String::new()
    } else {
        "exact".to_owned()
    };
    row.count = count;
    row.zero_results = zero_results;
    // Bucket 3 is the first bucket above 0.2 ms: the exact value never matters,
    // only that the stage carries a sample.
    for stage in 0..9 {
        row.add_latency(stage, bucket);
    }
    row
}

/// The filter form and the daily row of every fixture.
fn shapes_and_day() -> (Vec<ShapeCount>, DayIndex) {
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
    (shapes, day)
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
    let (shapes, day) = shapes_and_day();
    store
        .write(&minute, &shapes, &[day])
        .expect("the analytics of the fixture are written");
}

/// Writes two modes in one window: `alpha` served four `dense` events, `beta`
/// five `hybrid` ones.
///
/// The two modes sample two different histogram buckets, so their percentiles
/// are distinct, and no event carries an empty mode: the counts of the two
/// modes add up to the count of the answer without `mode` (ADR-0156,
/// decision 6, points 1 and 2).
fn seed_two_modes(instance: &Instance) {
    let store = AnalyticsStore::open_or_create(&analytics_path(instance))
        .expect("the analytics file is creatable");
    let minute = vec![
        mode_row("alpha", "search", "ok", "dense", 3, 1, 3),
        mode_row("alpha", "search", "invalid_filter", "dense", 1, 0, 3),
        mode_row("beta", "search_multi", "ok", "hybrid", 5, 0, 10),
    ];
    let (shapes, day) = shapes_and_day();
    store
        .write(&minute, &shapes, &[day])
        .expect("the analytics of the two modes are written");
}

/// Writes a window holding a `dense` event and one event whose mode is empty.
///
/// The empty mode is what a request refused before the mode was resolved
/// leaves (ADR-0156, decision 1): an explicit `mode` excludes it, and the
/// answer without `mode` counts it.
fn seed_without_mode(instance: &Instance) {
    let store = AnalyticsStore::open_or_create(&analytics_path(instance))
        .expect("the analytics file is creatable");
    let minute = vec![
        mode_row("alpha", "search", "ok", "dense", 1, 0, 3),
        mode_row("alpha", "search", "invalid_request", "", 2, 0, 10),
    ];
    let (shapes, day) = shapes_and_day();
    store
        .write(&minute, &shapes, &[day])
        .expect("the analytics without a mode are written");
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

/// An instance whose analytics hold two modes in one window, with its router.
fn seeded_with_two_modes() -> (Instance, Arc<AppState>, Router) {
    let instance = Instance::new();
    seed_two_modes(&instance);
    let state = instance.state();
    let router = build_router(Arc::clone(&state));
    (instance, state, router)
}

/// An instance whose analytics hold a row without a mode, with its router.
fn seeded_without_mode() -> (Instance, Arc<AppState>, Router) {
    let instance = Instance::new();
    seed_without_mode(&instance);
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

    // ADR-0156, decision 1: the message of an unknown parameter enumerates the
    // six names of the route, `mode` among them, in the order of the list.
    let (status, answer) = stats(&router, ADMIN_SECRET, "?inconnu=1").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    assert_eq!(
        answer["error"]["message"],
        "'inconnu' is not a parameter of this route; the parameters are `from`, `to`, `index`, \
         `operation`, `mode` and `group_by`",
        "{answer}"
    );
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
        Operation::Facets,
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

/// The three percentiles of `stage`, read from a `latency_ms` object.
fn percentiles(latency: &Value, stage: &str) -> (f64, f64, f64) {
    let stage = &latency[stage];
    let number = |name: &str| {
        stage[name]
            .as_f64()
            .unwrap_or_else(|| panic!("the percentiles carry '{name}': {latency}"))
    };
    (number("p50"), number("p95"), number("p99"))
}

/// `mode` is a closed domain, and a value outside it is `400 invalid_request`
/// with the message of ADR-0156, decision 1, which names the parameter and the
/// value it refused.
///
/// An empty value is a presence outside the domain, not an absence: the ADR
/// makes absence the lack of the parameter, and a closed domain admits no
/// fourth spelling (ADR-0156, decision 1).
#[tokio::test]
async fn a_value_outside_the_mode_domain_is_a_400_naming_the_parameter() {
    let (_instance, _state, router) = seeded_with_two_modes();
    for query in [
        "?mode=semantic",
        "?mode=Dense",
        "?mode=lexical,hybrid",
        "?mode=",
    ] {
        let (status, answer) = stats(&router, ADMIN_SECRET, query).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}: {answer}");
        assert_eq!(answer["error"]["code"], "invalid_request", "{query}");
        let message = answer["error"]["message"]
            .as_str()
            .unwrap_or_else(|| panic!("the refusal carries a message: {query}: {answer}"));
        for expected in ["'mode'", "dense", "lexical", "hybrid"] {
            assert!(
                message.contains(expected),
                "the refusal of '{query}' names '{expected}': {message}"
            );
        }
    }

    // The message itself is the one ADR-0156, decision 1, fixes, on the shape of
    // the refusals of `group_by` and `operation` (ADR-0124, decision 2).
    let (_, answer) = stats(&router, ADMIN_SECRET, "?mode=semantic").await;
    assert_eq!(
        answer["error"]["message"], "'mode' must be 'dense', 'lexical' or 'hybrid': 'semantic'",
        "{answer}"
    );
}

/// An explicit `mode` restricts `buckets` and `totals`, so `count`, `errors`,
/// `zero_results_rate` and every `latency_ms` count that mode only
/// (ADR-0156, decision 2).
#[tokio::test]
async fn the_mode_filter_restricts_the_buckets_and_the_totals() {
    let (_instance, _state, router) = seeded_with_two_modes();

    let (status, dense) = stats(&router, ADMIN_SECRET, "?mode=dense").await;
    assert_eq!(status, StatusCode::OK, "{dense}");
    let buckets = dense["buckets"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer holds buckets: {dense}"));
    assert_eq!(buckets.len(), 1, "one hour holds the fixture: {dense}");
    assert_eq!(buckets[0]["count"], 4, "the four dense events: {dense}");
    assert_eq!(
        buckets[0]["errors"]["invalid_filter"], 1,
        "the dense refusal is counted: {dense}"
    );
    assert_eq!(dense["totals"]["count"], 4, "{dense}");
    assert_eq!(dense["totals"]["errors"]["invalid_filter"], 1, "{dense}");
    assert_eq!(
        dense["totals"]["zero_results_rate"],
        1.0 / 3.0,
        "one of the three successful dense searches returned nothing: {dense}"
    );
    assert!(
        dense["totals"]["latency_ms"]["total"]["p50"].is_number(),
        "the percentiles of the mode are rendered: {dense}"
    );

    let (status, hybrid) = stats(&router, ADMIN_SECRET, "?mode=hybrid").await;
    assert_eq!(status, StatusCode::OK, "{hybrid}");
    assert_eq!(hybrid["buckets"][0]["count"], 5, "{hybrid}");
    assert_eq!(hybrid["totals"]["count"], 5, "{hybrid}");
    assert!(
        hybrid["totals"].get("errors").is_none(),
        "no hybrid event failed, so the member is omitted: {hybrid}"
    );
    assert_eq!(
        hybrid["totals"]["zero_results_rate"], 0.0,
        "no hybrid search returned nothing: {hybrid}"
    );

    // A mode no event carries renders an empty window, never a `404` and never
    // an invented count (ADR-0124, decision 3).
    let (status, lexical) = stats(&router, ADMIN_SECRET, "?mode=lexical").await;
    assert_eq!(status, StatusCode::OK, "{lexical}");
    assert_eq!(lexical["buckets"], json!([]), "{lexical}");
    assert!(
        lexical.get("totals").is_none(),
        "an empty window has no totals: {lexical}"
    );
    assert_eq!(lexical["top_indexes"], json!([]), "{lexical}");
}

/// An explicit `mode` restricts `top_indexes`, which reads `minute_events`
/// (ADR-0156, decision 2).
#[tokio::test]
async fn the_mode_filter_restricts_the_top_indexes() {
    let (_instance, _state, router) = seeded_with_two_modes();

    let (status, dense) = stats(&router, ADMIN_SECRET, "?mode=dense").await;
    assert_eq!(status, StatusCode::OK, "{dense}");
    assert_eq!(
        dense["top_indexes"].as_array().map(Vec::len),
        Some(1),
        "{dense}"
    );
    assert_eq!(dense["top_indexes"][0]["index"], "alpha", "{dense}");
    assert_eq!(dense["top_indexes"][0]["count"], 4, "{dense}");

    let (status, hybrid) = stats(&router, ADMIN_SECRET, "?mode=hybrid").await;
    assert_eq!(status, StatusCode::OK, "{hybrid}");
    assert_eq!(
        hybrid["top_indexes"].as_array().map(Vec::len),
        Some(1),
        "{hybrid}"
    );
    assert_eq!(hybrid["top_indexes"][0]["index"], "beta", "{hybrid}");
    assert_eq!(hybrid["top_indexes"][0]["count"], 5, "{hybrid}");
}

/// Two modes of one window read two distinct sets of percentiles, and the
/// answer without `mode` aggregates both: their counts add up to its count, and
/// its rate mixes their zero results (ADR-0156, decision 1, and decision 6,
/// points 1 and 2).
///
/// The proof that the answer without `mode` is the one of before the parameter
/// is `the_answer_has_the_members_of_the_route`, which pins it on the fixture of
/// ADR-0124; this test proves that the two restricted answers are its parts.
#[tokio::test]
async fn two_modes_of_one_window_have_distinct_percentiles_and_their_counts_add_up() {
    let (_instance, _state, router) = seeded_with_two_modes();

    let (status, all) = stats(&router, ADMIN_SECRET, "").await;
    assert_eq!(status, StatusCode::OK, "{all}");
    let (status, dense) = stats(&router, ADMIN_SECRET, "?mode=dense").await;
    assert_eq!(status, StatusCode::OK, "{dense}");
    let (status, hybrid) = stats(&router, ADMIN_SECRET, "?mode=hybrid").await;
    assert_eq!(status, StatusCode::OK, "{hybrid}");

    // The fixture samples the two modes in two histogram buckets, so a
    // percentile that ignored the mode would render the same value for both.
    let (dense_latency, hybrid_latency) = (
        &dense["totals"]["latency_ms"],
        &hybrid["totals"]["latency_ms"],
    );
    let (dense_p, hybrid_p) = (
        percentiles(dense_latency, "total"),
        percentiles(hybrid_latency, "total"),
    );
    assert!(
        dense_p.0 < hybrid_p.0 && dense_p.1 < hybrid_p.1 && dense_p.2 < hybrid_p.2,
        "each mode renders its own percentiles: dense {dense_p:?}, hybrid {hybrid_p:?}"
    );

    // The counts of the modes add up to the count of the answer without `mode`.
    let sum = dense["totals"]["count"].as_u64().unwrap_or_default()
        + hybrid["totals"]["count"].as_u64().unwrap_or_default();
    assert_eq!(
        all["totals"]["count"].as_u64(),
        Some(sum),
        "the modes partition the window: {all}"
    );

    // The aggregate is neither mode: its rate mixes the zero result of the
    // dense events with the five hybrid results.
    assert_eq!(
        all["totals"]["zero_results_rate"],
        1.0 / 8.0,
        "one of the eight successful searches of the window returned nothing: {all}"
    );
    assert_eq!(
        all["totals"]["errors"]["invalid_filter"], 1,
        "the refusal of the window comes from the dense mode: {all}"
    );
}

/// A row whose mode is empty — the row of a request refused before the mode was
/// resolved — is excluded by any explicit `mode` and counted without one
/// (ADR-0156, decision 1).
#[tokio::test]
async fn a_row_without_a_mode_is_reachable_without_a_mode_only() {
    let (_instance, _state, router) = seeded_without_mode();

    let (status, all) = stats(&router, ADMIN_SECRET, "").await;
    assert_eq!(status, StatusCode::OK, "{all}");
    assert_eq!(
        all["totals"]["count"], 3,
        "the window holds the dense event and the two without a mode: {all}"
    );

    for mode in ["dense", "lexical", "hybrid"] {
        let query = format!("?mode={mode}");
        let (status, answer) = stats(&router, ADMIN_SECRET, &query).await;
        assert_eq!(status, StatusCode::OK, "{query}: {answer}");
        let counted = answer["totals"]["count"].as_u64().unwrap_or(0);
        assert!(
            counted < 3,
            "'{query}' excludes the rows without a mode: {answer}"
        );
        if mode == "dense" {
            assert_eq!(
                counted, 1,
                "'{query}' keeps the single dense event: {answer}"
            );
            assert!(
                answer["totals"].get("errors").is_none(),
                "the refusal without a mode belongs to no mode: {answer}"
            );
        }
    }
}

/// `top_filter_shapes`, `indexes`, `analytics_dropped` and the echoes describe
/// something else than a mode, and the filter does not touch them
/// (ADR-0156, decision 2).
#[tokio::test]
async fn the_members_without_a_mode_column_are_the_same_with_and_without_a_mode() {
    let (_instance, _state, router) = seeded_with_two_modes();

    let (status, all) = stats(&router, ADMIN_SECRET, "").await;
    assert_eq!(status, StatusCode::OK, "{all}");
    for query in ["?mode=dense", "?mode=hybrid", "?mode=lexical"] {
        let (status, answer) = stats(&router, ADMIN_SECRET, query).await;
        assert_eq!(status, StatusCode::OK, "{query}: {answer}");
        for member in ["top_filter_shapes", "indexes", "from", "to", "group_by"] {
            assert_eq!(
                answer[member], all[member],
                "'{member}' does not follow the mode ({query}): {answer}"
            );
        }
        assert_eq!(
            answer["analytics_dropped"], all["analytics_dropped"],
            "the counter of the process does not follow the mode ({query}): {answer}"
        );
    }
    // The rankings are not empty, so the comparison above is not vacuous.
    assert_eq!(all["top_filter_shapes"][0]["shape"], "fonds:eq", "{all}");
    assert_eq!(all["indexes"][0]["generation"], 5, "{all}");
}
