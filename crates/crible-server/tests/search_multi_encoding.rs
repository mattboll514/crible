// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and every helper of this file is a fixture or an assertion.
#![allow(clippy::expect_used, clippy::panic)]
#![cfg(feature = "failpoints")]

//! A multi-index search encodes its query **once**, whatever the number of
//! targets (ADR-0110, decisions 8 and 11).
//!
//! The `failpoints` build accepts a test double at the single encoding point of
//! the server (`crate::search::encode_query`): `CRIBLE_TEST_ENCODER` names a
//! directory, and every real call appends one byte to `<dir>/encoded`. The
//! length of that file is therefore the number of calls, counted where the
//! server really encodes, not where it reads the query cache (ADR-0051).
//!
//! The two targets use the `external` model of the fixture, so no ONNX Runtime
//! is loaded; the double answers before the route refuses an `external` model
//! for a text query.
//!
//! ```text
//! cargo test -p crible-server --features failpoints --test search_multi_encoding
//! ```

mod common;

use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::http::build_router;
use serde_json::json;

use common::{ADMIN_SECRET, READ_SECRET, index_config, send};

/// Path of the multi-index route.
const MULTI: &str = "/v1/search";

/// Creates the index `name`, on the fixture model and without the lexical
/// branch, so the default mode of the request is `dense`.
async fn create(router: &Router, name: &str) {
    let mut body = index_config();
    body["name"] = json!(name);
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

/// Number of real calls the double counted in `directory`.
fn counted(directory: &Path) -> usize {
    std::fs::read(directory.join("encoded"))
        .expect("the double counted at least one call")
        .len()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_multi_index_search_encodes_its_query_once() {
    let counter = tempfile::tempdir().expect("a counter directory is creatable");
    // SAFETY: this integration test binary sets the variable before spawning any
    // request, and no other test of this process reads or writes it.
    unsafe { std::env::set_var("CRIBLE_TEST_ENCODER", counter.path()) };

    let instance = common::Instance::new();
    let state = instance.state();
    let router = build_router(Arc::clone(&state));
    create(&router, "alpha").await;
    create(&router, "beta").await;

    let body = json!({
        "query": "contrat",
        "mode": "dense",
        "targets": [ { "index": "alpha" }, { "index": "beta" } ],
    });
    let (status, answer) = send(&router, Method::POST, MULTI, READ_SECRET, Some(body)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the multi-index search runs: {answer}"
    );
    assert_eq!(
        answer["targets"]
            .as_array()
            .expect("the answer holds targets")
            .len(),
        2,
        "both targets ran: {answer}"
    );
    assert_eq!(
        counted(counter.path()),
        1,
        "the query is encoded once for the two targets: {answer}"
    );

    // SAFETY: the test owns the variable until its end, as for `set_var`.
    unsafe { std::env::remove_var("CRIBLE_TEST_ENCODER") };
}
