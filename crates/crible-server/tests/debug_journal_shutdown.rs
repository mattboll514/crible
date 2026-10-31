// SPDX-License-Identifier: Apache-2.0

//! The debug journal is flushed at the shutdown, after the analytics
//! (ADR-0122, decision 7).
//!
//! The sequence of ADR-0099 drains the requests in flight, flushes the
//! analytics, then the journal; the journal comes last because the usage
//! figures must not yield to the diagnostic. This test proves the second half on
//! the real sequence: a search is served, the instance drains, and **without any
//! explicit flush** the journal holds its row and the analytics hold their event.
//! A run that stopped the writer before draining its queue would leave
//! `debug.sqlite` without the row, and the test would fail.

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixture of this file is a helper.
#![allow(clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_core::error::CribleError;
use crible_server::state::AppState;
use crible_server::{Shutdown, lifecycle::run_server};
use crible_store::analytics::{ANALYTICS_FILE_NAME, read_minute_events};
use crible_store::debug_journal::{DEBUG_JOURNAL_FILE_NAME, read_queries};
use serde_json::json;
use tokio::net::TcpListener;

use common::{Journal, READ_SECRET, base64_vector, instance_with_alpha_and_journal, send};

/// Path of the search route of the fixture index.
const SEARCH: &str = "/v1/indexes/alpha/search";

/// Vector of the fixture document, and of the query of this test.
const VECTOR: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

/// Bound of the drainage and of the flush jobs of the shutdown.
///
/// It never delays the test: the instance drains an empty server at once.
const BOUND: Duration = Duration::from_secs(5);

/// One instance serving `router` on an ephemeral port.
struct Served {
    shutdown: Shutdown,
    handle: tokio::task::JoinHandle<Result<crible_server::lifecycle::StopReason, CribleError>>,
}

impl Served {
    /// Triggers the shutdown and returns the reason of the stop.
    async fn join(self) -> crible_server::lifecycle::StopReason {
        self.handle
            .await
            .expect("the server task ends")
            .expect("the server stops on its own drainage")
    }
}

/// Binds an ephemeral port and runs the server of `state` on it.
///
/// The test reaches the routes through the `Router` itself, so no request travels
/// over the listener: it exists because `run_server` serves one.
async fn served(router: Router, state: Arc<AppState>) -> Served {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("an ephemeral port is bindable");
    let shutdown = Shutdown::new();
    let handle = tokio::spawn(run_server(
        listener,
        router,
        state,
        shutdown.clone(),
        BOUND,
        BOUND,
    ));
    Served { shutdown, handle }
}

#[tokio::test]
async fn the_shutdown_writes_the_journal_after_the_analytics() {
    // `sample_rate = 1` records the search without `debug`, which is the way of
    // ADR-0122, decision 1, that a journal must never lose at the shutdown.
    let (instance, state, router) = instance_with_alpha_and_journal(Journal::enabled(1.0)).await;
    let server = served(router.clone(), Arc::clone(&state)).await;
    let (status, body) = send(
        &router,
        Method::POST,
        SEARCH,
        READ_SECRET,
        Some(json!({
            "vector": base64_vector(VECTOR),
            "mode": "dense",
            "k": 10,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the search answers: {body}");

    // Neither file holds anything yet: no periodic flush ran, and nothing forces
    // one before the shutdown.
    let journal = instance.root().join("data").join(DEBUG_JOURNAL_FILE_NAME);
    let analytics = instance.root().join("data").join(ANALYTICS_FILE_NAME);
    assert!(
        read_queries(&journal)
            .expect("the debug journal is readable")
            .is_empty(),
        "no flush of the journal ran before the shutdown"
    );
    assert!(
        read_minute_events(&analytics)
            .expect("the analytics file is readable")
            .is_empty(),
        "no flush of the analytics ran before the shutdown"
    );

    server.shutdown.drain();
    let reason = server.join().await;
    assert_eq!(
        reason,
        crible_server::lifecycle::StopReason::Drained,
        "the drainage ends on its own bound"
    );

    // The analytics went first, and the journal followed: both are written by the
    // shutdown alone (ADR-0122, decision 7).
    let events = read_minute_events(&analytics).expect("the analytics file is readable");
    let searches: Vec<_> = events
        .iter()
        .filter(|row| row.operation == "search" && row.index == common::INDEX)
        .collect();
    assert_eq!(
        searches.len(),
        1,
        "the shutdown flushed the analytics event of the search: {events:?}"
    );
    assert_eq!(searches[0].count, 1, "one search, one event");
    let rows = read_queries(&journal).expect("the debug journal is readable");
    assert_eq!(
        rows.len(),
        1,
        "the shutdown drained the journal queue: {rows:?}"
    );
    assert_eq!(rows[0].status, "ok", "the row of the search: {rows:?}");
    assert_eq!(rows[0].client, "reader", "the row of the search: {rows:?}");
    assert_eq!(rows[0].mode, "dense", "the row of the search: {rows:?}");
    assert!(
        rows[0].total_ms.is_some(),
        "a finished search has a total: {rows:?}"
    );
}
