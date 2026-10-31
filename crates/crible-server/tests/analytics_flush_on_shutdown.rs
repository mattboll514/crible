// SPDX-License-Identifier: Apache-2.0

//! The analytics are flushed at the shutdown (ADR-0118, decision 3).
//!
//! `docs/07-stack-et-deploiement.md:121` and ADR-0099, "Arrêt", drain the
//! requests in flight and then flush the analytics that exist. This test proves
//! the second half on the real sequence: a search is sent, the instance drains,
//! and the row of the search is in `analytics.sqlite` **without** any explicit
//! flush. A run that stopped the writer before draining its queue would leave the
//! file without the row, and the test would fail.

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
use serde_json::json;
use tokio::net::TcpListener;

use common::{INDEX, Instance, READ_SECRET, base64_vector, instance_with_alpha, send};

/// Path of the search route of the fixture index.
const SEARCH: &str = "/v1/indexes/alpha/search";

/// Vector of the fixture document, and of the query of this test.
const VECTOR: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

/// Bound of the drainage and of the write jobs of the shutdown.
///
/// It never delays the test: the instance drains an empty server at once.
const BOUND: Duration = Duration::from_secs(5);

/// Sends one dense search through `router`.
async fn search(router: &Router) {
    let (status, body) = send(
        router,
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
}

/// Reads the analytics file of the instance.
fn analytics_path(instance: &Instance) -> std::path::PathBuf {
    instance.root().join("data").join(ANALYTICS_FILE_NAME)
}

#[tokio::test]
async fn the_shutdown_writes_the_search_it_served() {
    let (instance, state, router) = instance_with_alpha().await;
    let server = served(router.clone(), Arc::clone(&state)).await;
    search(&router).await;

    // The event is in the bounded queue, not yet in the file: no periodic flush
    // has run, and nothing forces one before the shutdown.
    assert!(
        read_minute_events(&analytics_path(&instance))
            .expect("the analytics file is readable")
            .is_empty(),
        "no flush ran before the shutdown"
    );

    server.shutdown.drain();
    let reason = server.join().await;
    assert_eq!(
        reason,
        crible_server::lifecycle::StopReason::Drained,
        "the drainage ends on its own bound"
    );

    let rows =
        read_minute_events(&analytics_path(&instance)).expect("the analytics file is readable");
    let search = rows
        .iter()
        .find(|row| row.index == INDEX && row.operation == "search")
        .unwrap_or_else(|| panic!("the shutdown flushed the search it served: {rows:?}"));
    assert_eq!(search.count, 1, "one search, one event");
    assert_eq!(search.status, "ok", "the search answered `200`");
    assert_eq!(search.client, "reader", "the client is the name of the key");
    assert_eq!(search.mode, "dense", "the mode of the request is recorded");
    assert_eq!(
        search.dense_strategy, "exact",
        "the strategy the dense branch ran is recorded"
    );
    assert_eq!(
        search.pool.iter().sum::<u32>(),
        1,
        "the pool of the fixture holds one passage, one sample in the log2 histogram"
    );
}

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
/// The tests reach the routes through the `Router` itself, so no request travels
/// over the listener: it exists because `run_server` serves one, and it accepts
/// nothing during the test.
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
