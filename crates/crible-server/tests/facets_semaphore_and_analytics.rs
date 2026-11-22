// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and every helper of this file is a fixture or an assertion.
#![allow(clippy::expect_used, clippy::panic)]

//! The two rules of ADR-0154 the facets route shares with the searches: the
//! concurrency token (decision 8) and the limits of its observability
//! (decision 9).
//!
//! | Test | What it proves |
//! |---|---|
//! | `a_facet_call_without_a_token_is_refused_like_a_search` | one semaphore for both routes, same refusal, same `Retry-After` |
//! | `a_facet_call_writes_no_filter_shape_and_no_journal_row` | no `filter_shapes` row, no debug journal row |
//!
//! ```text
//! cargo test -p crible-server --test facets_semaphore_and_analytics
//! ```

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::state::AppState;
use crible_store::debug_journal::{DEBUG_JOURNAL_FILE_NAME, read_queries};
use serde_json::{Value, json};

use common::{
    ADMIN_SECRET, INDEX, Instance, READ_SECRET, WRITE_SECRET, base64_vector,
    instance_with_alpha_and_journal, instance_with_alpha_and_limit, send, send_with_headers,
    terminal_job,
};

/// Vector of every passage and of every dense search of this file.
const VECTOR: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

/// The facets body of this file.
fn facets_body() -> Value {
    json!({ "fields": ["numero"] })
}

/// The dense search body of this file, with a filter and `debug`.
fn search_body() -> Value {
    json!({
        "vector": base64_vector(VECTOR),
        "mode": "dense",
        "k": 1,
        "filter": { "numero": { "eq": "1240" } },
        "debug": true,
        "timeout_ms": 30_000,
    })
}

/// Posts `body` on the facets route of `index` and returns the answer.
async fn facets(router: &Router, index: &str, body: Value) -> (StatusCode, Value) {
    let path = format!("/v1/indexes/{index}/facets");
    send(router, Method::POST, &path, READ_SECRET, Some(body)).await
}

/// Attaches one document `D1` with the value `numero` `1240`.
async fn attach_fixture(router: &Router, state: &Arc<AppState>) {
    let path = format!("/v1/indexes/{INDEX}/documents?wait=true");
    let body = json!({
        "batch_id": "facets-semaphore-and-analytics",
        "mode": "upsert",
        "documents": [{
            "doc_id": "D1",
            "fields": { "numero": "1240" },
            "passages": [{
                "text": "le contrat",
                "fields": { "texte": "contrat" },
                "vector": base64_vector(VECTOR),
            }],
        }],
        "deletions": [],
    });
    let (status, accepted) = send(router, Method::POST, &path, WRITE_SECRET, Some(body)).await;
    assert!(
        status == StatusCode::OK || status == StatusCode::ACCEPTED,
        "the batch is accepted: {accepted}"
    );
    let job = terminal_job(router, state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the batch attaches: {job}");
}

/// Reads `GET /v1/stats` without a window.
async fn stats(router: &Router) -> Value {
    let (status, answer) = send(router, Method::GET, "/v1/stats", ADMIN_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "the statistics answer: {answer}");
    answer
}

/// Path of the debug journal of `instance`.
fn journal_path(instance: &Instance) -> PathBuf {
    instance.root().join("data").join(DEBUG_JOURNAL_FILE_NAME)
}

// --- decision 8 -----------------------------------------------------------

/// Decision 8 — the facets route takes a token of the **same** semaphore as the
/// searches, without waiting.
///
/// The only token of the fixture is held by the limit itself, so the test needs
/// no `failpoints` build: the closure signals that the token is held, then waits
/// for the release. A facet call and a search then find no token, and both are
/// refused at once with the same body and `Retry-After: 1` (ADR-0102,
/// decision 2). Once released, the token comes back and the facet call answers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_facet_call_without_a_token_is_refused_like_a_search() {
    let (_instance, state, router) = instance_with_alpha_and_limit(1).await;
    let limit = state
        .search_limit()
        .expect("the loaded instance exposes its search limit");
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
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
        .expect("the token is held before the calls run");

    let (facets_status, facets_headers, facets_answer) = send_with_headers(
        &router,
        Method::POST,
        &format!("/v1/indexes/{INDEX}/facets"),
        READ_SECRET,
        Some(facets_body()),
    )
    .await;
    let (search_status, search_headers, search_answer) = send_with_headers(
        &router,
        Method::POST,
        &format!("/v1/indexes/{INDEX}/search"),
        READ_SECRET,
        Some(search_body()),
    )
    .await;

    assert_eq!(
        facets_status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{facets_answer}"
    );
    assert_eq!(
        search_status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{search_answer}"
    );
    assert_eq!(
        facets_answer, search_answer,
        "a facet call without a token is refused exactly like a search (ADR-0154, decision 8)"
    );
    assert_eq!(
        facets_answer["error"]["code"], "overloaded",
        "{facets_answer}"
    );
    for (route, headers) in [("facets", &facets_headers), ("search", &search_headers)] {
        assert_eq!(
            headers
                .get("retry-after")
                .and_then(|value| value.to_str().ok()),
            Some("1"),
            "the `{route}` refusal carries Retry-After: 1"
        );
    }

    release_tx.send(()).expect("the held token is released");
    held.await
        .expect("the holding task finishes")
        .expect("the held token returns");
    assert_eq!(
        state
            .search_limit()
            .expect("the loaded instance exposes its search limit")
            .available(),
        1,
        "the token comes back when the held work ends"
    );

    let (status, answer) = facets(&router, INDEX, facets_body()).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the facet call runs once the token is free: {answer}"
    );
    assert_eq!(answer["count"]["passages"], 0, "{answer}");
}

// --- decision 9 -----------------------------------------------------------

/// Decision 9 — a facet call writes no `filter_shapes` row and no debug journal
/// row, even when the journal records every search.
///
/// The fixture enables the journal at the sample rate `1.0`, so a search is
/// always recorded: the first search writes one shape and one row, and the facet
/// calls that follow write neither. The counts are read from `GET /v1/stats`
/// and from the journal file itself, so the check is not vacuous.
#[tokio::test]
async fn a_facet_call_writes_no_filter_shape_and_no_journal_row() {
    let (instance, state, router) =
        instance_with_alpha_and_journal(common::Journal::enabled(1.0)).await;
    attach_fixture(&router, &state).await;

    let (status, answer) = facets(
        &router,
        INDEX,
        json!({
            "fields": ["numero"],
            "filter": { "numero": { "eq": "1240" } },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    state
        .flush_analytics()
        .await
        .expect("the analytics of the facet call are flushed");

    let before = stats(&router).await;
    assert_eq!(
        before["top_filter_shapes"],
        json!([]),
        "a facet call writes no `filter_shapes` row (ADR-0154, decision 9): {before}"
    );

    // A search writes one shape and one journal row, so the checks below are
    // not vacuous.
    let (status, search) = send(
        &router,
        Method::POST,
        &format!("/v1/indexes/{INDEX}/search"),
        READ_SECRET,
        Some(search_body()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the search runs: {search}");
    state
        .flush_analytics()
        .await
        .expect("the analytics of the search are flushed");
    state
        .flush_debug_journal()
        .await
        .expect("the journal of the search is flushed");

    let after_search = stats(&router).await;
    let shapes = after_search["top_filter_shapes"]
        .as_array()
        .unwrap_or_else(|| panic!("the search writes a shape: {after_search}"));
    assert_eq!(shapes.len(), 1, "{after_search}");
    assert_eq!(shapes[0]["shape"], "numero:eq", "{after_search}");
    assert_eq!(shapes[0]["count"], 1, "{after_search}");
    let rows = read_queries(&journal_path(&instance)).expect("the debug journal is readable");
    assert_eq!(rows.len(), 1, "the search wrote one row: {rows:?}");

    // A second facet call, with another filter value, adds neither.
    let (status, answer) = facets(
        &router,
        INDEX,
        json!({
            "fields": ["numero", "texte"],
            "filter": { "numero": { "eq": "absent" } },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    state
        .flush_analytics()
        .await
        .expect("the analytics of the second facet call are flushed");
    state
        .flush_debug_journal()
        .await
        .expect("the journal is flushed again");

    let after_facets = stats(&router).await;
    let shapes = after_facets["top_filter_shapes"]
        .as_array()
        .unwrap_or_else(|| panic!("the search shape is still there: {after_facets}"));
    assert_eq!(
        shapes[0]["count"], 1,
        "a facet call adds no `filter_shapes` row (ADR-0154, decision 9): {after_facets}"
    );
    let rows = read_queries(&journal_path(&instance)).expect("the debug journal is readable");
    assert_eq!(
        rows.len(),
        1,
        "a facet call feeds no debug journal row (ADR-0154, decision 9): {rows:?}"
    );
}
