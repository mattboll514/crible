// SPDX-License-Identifier: Apache-2.0

// Test fixtures may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the helpers of this file are free functions.
#![allow(clippy::expect_used, clippy::panic)]

//! The journal of the server, seen from outside the crate (ADR-0096,
//! ADR-0101).
//!
//! This test has a binary of its own on purpose. It captures the journal of the
//! process, and `tracing` freezes the interest of a callsite for the whole
//! process: a sibling test of the same binary that touches
//! `crible_server::error::log_refusal` from a thread with no default dispatcher
//! before this test installs its subscriber caches that callsite as `never`, and
//! no scoped default brings it back. A binary with one test leaves no sibling to
//! do that, so the capture is deterministic (`@relecteur`, NETTOYER-P7).
//!
//! ```text
//! cargo test -p crible-server --test error_journal
//! ```

mod common;

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::Extension;
use axum::http::Method;
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use crible_core::error::{CribleError, ErrorCode};
use crible_server::auth::{Auth, protect, sha256_hex};
use crible_server::config::{ApiKey, Role};
use crible_server::http::{RequestId, with_common_layers};
use crible_server::limits;
use serde_json::{Value, json};

use common::{journal_at_level, send_raw};

/// Key of the `read` role used by the test.
const READ_KEY: &str = "1111111111111111111111111111111111111111111111111111111111111111";

/// Identifier pinned on every request, so the journal line can be read by it
/// (ADR-0098).
const REQUEST_ID: &str = "errors-test-1";

/// Cause of the `internal` the test route renders: it must reach the journal,
/// and never the body (ADR-0101, decision 4).
const INTERNAL_CAUSE: &str = "the blocking task of a search did not finish normally: zz-cause-zz";

/// Journal of this binary: one sink, installed once for the process.
///
/// The capture is global and lives in a `OnceLock`, like `common::journal()`
/// does for the binaries that share it: a thread-local default does not bring
/// back a callsite frozen as `never`, while the global registration rebuilds
/// the interest of every callsite (`@relecteur`, NETTOYER-P7). The level is
/// `trace`, because axum journals a rejection on its own target (`@relecteur`,
/// NETTOYER-P7).
fn journal() -> common::Sink {
    journal_at_level("trace")
}

/// Router of the test: one route per reachable code, plus a panic.
fn router() -> Router {
    let auth = Arc::new(
        Auth::new(&[ApiKey {
            name: "reader".to_owned(),
            role: Role::Read,
            sha256: sha256_hex(READ_KEY),
        }])
        .expect("the declared key holds a valid digest"),
    );
    let read = protect(
        Router::new()
            .route("/echo", post(echo))
            .route("/typed", post(typed)),
        auth,
        Role::Read,
    );
    let open = Router::new()
        .route("/panic", get(panics))
        .route("/internal", get(fails));
    with_common_layers(read.merge(open))
}

/// Echoes its body: an extractor rejection is the only way to fail.
async fn echo(Json(value): Json<Value>) -> Json<Value> {
    Json(value)
}

/// Reads a body of a declared type: a JSON value of the wrong type is refused
/// with a message that quotes it, which is the leak ADR-0096 forbids.
async fn typed(Json(value): Json<u64>) -> Json<Value> {
    Json(json!({ "value": value }))
}

/// Panics on purpose: the panic layer must render a `500` (ADR-0101).
async fn panics() -> &'static str {
    panic!("a handler panicked: zz-boom-zz");
}

/// Renders an `internal` the way a route of the server does: the generic body
/// for the client, the cause for the journal (ADR-0101, decisions 2 and 4).
async fn fails(Extension(identifier): Extension<RequestId>) -> Response {
    let failure = CribleError::new(ErrorCode::Internal, INTERNAL_CAUSE);
    limits::refusal(&failure, &identifier.0)
}

/// Sends one request of the `read` key without a body.
async fn send(router: &Router, method: Method, path: &str) -> Response {
    send_raw(
        router,
        method,
        path,
        READ_KEY,
        REQUEST_ID,
        None,
        Body::empty(),
    )
    .await
}

#[test]
fn the_journal_names_errors_without_any_client_message() {
    // ADR-0101, decision 4: one line per error, with its code, its status and
    // its `request_id`; the message of a `4xx` is never journalised, and the
    // cause of an `internal` is, because it is the diagnostic of that code.
    // ADR-0096: no value the client sent reaches the journal, whatever the
    // target of the line.
    let sink = journal();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime is startable");
    runtime.block_on(async {
        let router = router();
        let refused = send_raw(
            &router,
            Method::POST,
            "/echo",
            READ_KEY,
            REQUEST_ID,
            Some("application/json"),
            Body::from(r#"{"filter_value": "zz-4xx-zz", "#),
        )
        .await;
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
        // A valid JSON body of the wrong type: serde quotes the value it
        // refused, and axum journals that text on its own target at `TRACE`.
        let mistyped = send_raw(
            &router,
            Method::POST,
            "/typed",
            READ_KEY,
            REQUEST_ID,
            Some("application/json"),
            Body::from(r#""zz-typed-value-zz""#),
        )
        .await;
        assert_eq!(mistyped.status(), StatusCode::BAD_REQUEST);
        let unknown = send(&router, Method::GET, "/no-such-route").await;
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
        let panicked = send(&router, Method::GET, "/panic").await;
        assert_eq!(panicked.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let failed = send(&router, Method::GET, "/internal").await;
        assert_eq!(failed.status(), StatusCode::INTERNAL_SERVER_ERROR);
    });

    let text = sink.contents();
    let ours: Vec<&str> = text
        .lines()
        .filter(|line| line.contains("crible_server::error"))
        .collect();
    assert!(!ours.is_empty(), "the lines of the server: {text}");
    for line in &ours {
        assert!(
            !line.contains("zz-4xx-zz"),
            "a message of a 4xx reached the journal: {line}"
        );
    }
    // No filter on the target here: the check covers every line the process
    // wrote, axum's rejection journal included (ADR-0096).
    assert!(
        !text.contains("zz-typed-value-zz"),
        "a value the client sent reached the journal: {text}"
    );
    assert!(
        ours.iter()
            .any(|line| line.contains("\"request_id\":\"errors-test-1\"")),
        "the identifier is journalised: {text}"
    );
    assert!(
        ours.iter()
            .any(|line| line.contains("\"code\":\"invalid_request\"")),
        "the code of a 4xx is journalised: {text}"
    );
    assert!(
        ours.iter()
            .any(|line| line.contains("\"code\":\"internal\"") && line.contains("zz-boom-zz")),
        "the cause of a panic is journalised with the code: {text}"
    );
    assert!(
        ours.iter()
            .any(|line| line.contains("\"code\":\"internal\"") && line.contains("zz-cause-zz")),
        "the cause of an internal of a route is journalised too: {text}"
    );
}
