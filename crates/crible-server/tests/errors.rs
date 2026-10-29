// SPDX-License-Identifier: Apache-2.0

// Test fixtures may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the helpers of this file are free functions.
#![allow(clippy::expect_used, clippy::panic)]

//! Errors and limits seen from outside the crate.
//!
//! Acceptance criteria of `FORMAT-ERREURS-ET-LIMITES`
//! (`docs/phases/P6-serveur-et-api.md`): one test per reachable error code. The
//! bodies follow ADR-0101, the body bound, the concurrency limit and the
//! deadline follow ADR-0102.
//!
//! The `/v1` routes do not exist yet: the concurrency limit and the deadline
//! are proved on a test router, as ADR-0095 asks, and the search routes will
//! call the same [`SearchLimit`] and [`limits::refusal`].

use std::io;
use std::sync::{Arc, Barrier, Mutex};
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::{Extension, Request, State};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, RETRY_AFTER, WWW_AUTHENTICATE};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use crible_core::error::{CribleError, ErrorCode};
use crible_server::auth::{Auth, protect, sha256_hex};
use crible_server::config::{ApiKey, Role};
use crible_server::http::{REQUEST_ID_HEADER, RequestId, with_common_layers};
use crible_server::limits::{self, MAX_BODY_BYTES, SearchLimit};
use crible_server::logging::log_subscriber;
use serde_json::{Value, json};
use tower::ServiceExt;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt;

/// Key of the `read` role used by the tests.
const READ_KEY: &str = "1111111111111111111111111111111111111111111111111111111111111111";

/// Key of the `admin` role used by the tests.
const ADMIN_KEY: &str = "2222222222222222222222222222222222222222222222222222222222222222";

/// Identifier pinned on every request, so the body and the header can be
/// compared exactly (ADR-0098).
const REQUEST_ID: &str = "errors-test-1";

/// Default of `[limits].default_timeout_ms` used by the test route.
const DEFAULT_TIMEOUT_MS: u64 = 2_000;

/// Message of the `500` of ADR-0101, decision 2, as `docs/05-api.md` writes it.
const GENERIC_INTERNAL: &str = "internal error; see the server log for this request_id";

/// Cause of the `internal` the test route renders: it must reach the journal,
/// and never the body (ADR-0101, decision 4).
const INTERNAL_CAUSE: &str = "the blocking task of a search did not finish normally: zz-cause-zz";

/// State of the test router: the limit of the searches and the rendezvous the
/// work of a search waits for.
///
/// The rendezvous replaces a duration: the work blocks until the test releases
/// it, so the token of an expired search is held for exactly as long as the
/// assertions need, whatever the load of the machine (ADR-0102, decision 3).
struct TestState {
    limit: SearchLimit,
    gate: Barrier,
}

impl TestState {
    /// Builds a state whose searches allow `max_concurrent_searches` at once.
    fn new(max_concurrent_searches: usize) -> Self {
        Self {
            limit: SearchLimit::new(max_concurrent_searches),
            gate: Barrier::new(2),
        }
    }

    /// Releases the search that waits for the rendezvous.
    ///
    /// Called exactly once, after a request whose work waits: both parties then
    /// pass, the work returns, and its token comes back with it.
    fn release(&self) {
        self.gate.wait();
    }
}

/// In-memory writer that captures the journal of a test.
#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Sink {
    fn contents(&self) -> String {
        let bytes = self.0.lock().map(|bytes| bytes.clone()).unwrap_or_default();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// One write handle of the [`Sink`].
struct SinkWriter(Sink);

impl io::Write for SinkWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let shared = &self.0.0;
        let mut bytes = shared
            .lock()
            .map_err(|_| io::Error::other("the journal sink is poisoned"))?;
        bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Sink {
    type Writer = SinkWriter;

    fn make_writer(&'a self) -> Self::Writer {
        SinkWriter(self.clone())
    }
}

/// Router of the test: one route per reachable code, plus the search mechanism.
fn router(state: Arc<TestState>) -> Router {
    let auth = Arc::new(
        Auth::new(&[
            ApiKey {
                name: "reader".to_owned(),
                role: Role::Read,
                sha256: sha256_hex(READ_KEY),
            },
            ApiKey {
                name: "administrator".to_owned(),
                role: Role::Admin,
                sha256: sha256_hex(ADMIN_KEY),
            },
        ])
        .expect("the declared keys hold a valid digest"),
    );
    let read = protect(
        Router::new()
            .route("/echo", post(echo))
            .route("/typed", post(typed)),
        Arc::clone(&auth),
        Role::Read,
    );
    let admin = protect(Router::new().route("/admin", get(level)), auth, Role::Admin);
    let open = Router::new()
        .route("/health", get(health))
        .route("/panic", get(panics))
        .route("/internal", get(fails))
        .route("/unavailable", get(unavailable))
        .route("/search", post(search));
    with_common_layers(read.merge(admin).merge(open)).with_state(state)
}

/// Answers `200`, for the open probe.
async fn health() -> &'static str {
    "ok"
}

/// Answers `200`, for the route that only `admin` reaches.
async fn level() -> &'static str {
    "level"
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

/// Answers a bare `503`, without the format: the last resort of the formatting
/// layer must render `overloaded`, never `internal` (ADR-0101, decision 6).
async fn unavailable() -> StatusCode {
    StatusCode::SERVICE_UNAVAILABLE
}

/// The shape `ROUTE-RECHERCHE` calls: one deadline, one token, one refusal.
async fn search(
    State(state): State<Arc<TestState>>,
    Extension(identifier): Extension<RequestId>,
    Json(body): Json<Value>,
) -> Response {
    let deadline = match limits::timeout(
        body.get("timeout_ms").and_then(Value::as_u64),
        DEFAULT_TIMEOUT_MS,
    ) {
        Ok(deadline) => deadline,
        Err(failure) => return limits::refusal(&failure, &identifier.0),
    };
    // `wait` makes the work block on the rendezvous of the test until it is
    // released: the token then stays taken for the whole test, without a
    // duration (ADR-0102, decision 3).
    let waiting = Arc::clone(&state);
    let waits = body.get("wait").and_then(Value::as_bool).unwrap_or(false);
    let work = move || {
        if waits {
            waiting.release();
        }
    };
    match state.limit.run_blocking(deadline, work).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(failure) => limits::refusal(&failure, &identifier.0),
    }
}

/// Sends one request through the router.
async fn call(router: &Router, request: Request) -> Response {
    router
        .clone()
        .oneshot(request)
        .await
        .expect("the router answers")
}

/// Sends one request without a body.
async fn send(router: &Router, method: Method, path: &str, key: Option<&str>) -> Response {
    send_with(router, method, path, key, None, Body::empty()).await
}

/// Sends one request, with an optional key, an optional content type and a body.
async fn send_with(
    router: &Router,
    method: Method,
    path: &str,
    key: Option<&str>,
    content_type: Option<&str>,
    body: Body,
) -> Response {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(REQUEST_ID_HEADER, REQUEST_ID);
    if let Some(key) = key {
        builder = builder.header(AUTHORIZATION, format!("Bearer {key}"));
    }
    if let Some(content_type) = content_type {
        builder = builder.header(CONTENT_TYPE, content_type);
    }
    call(
        router,
        builder
            .body(body)
            .expect("the request of the test is well formed"),
    )
    .await
}

/// Sends one JSON body to `path`.
async fn send_json(
    router: &Router,
    path: &str,
    key: Option<&str>,
    content_type: Option<&str>,
    body: &str,
) -> Response {
    send_with(
        router,
        Method::POST,
        path,
        key,
        content_type,
        Body::from(body.to_owned()),
    )
    .await
}

/// Decodes the JSON body of a response.
async fn json_body(response: Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body is readable");
    serde_json::from_slice(&bytes).expect("the body is JSON")
}

/// Asserts the shape of `docs/05-api.md` and returns the `error` object.
///
/// Every error response carries the header of ADR-0098 and the same identifier
/// in its body, a non-empty message, an object of details and exactly the four
/// keys of the format.
async fn error_body(response: Response, status: StatusCode, code: &str) -> Value {
    assert_eq!(response.status(), status, "status of a {code}");
    let identifier = response
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .expect("every response carries x-request-id");
    let body = json_body(response).await;
    let object = body
        .get("error")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("the body names an error: {body}"));
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["code", "details", "message", "request_id"],
        "the four keys of the format: {body}"
    );
    assert_eq!(object["code"], code);
    assert!(
        object["message"]
            .as_str()
            .is_some_and(|text| !text.is_empty()),
        "the message is a non-empty text: {body}"
    );
    assert!(
        object["details"].is_object(),
        "details is an object: {body}"
    );
    assert_eq!(
        object["request_id"], identifier,
        "the body names the identifier of the response"
    );
    Value::Object(object.clone())
}

#[tokio::test]
async fn an_absent_key_is_an_unauthorized() {
    // `docs/05-api.md`: a missing key is `401`, with `WWW-Authenticate: Bearer`.
    let router = router(Arc::new(TestState::new(2)));
    let response = send(&router, Method::POST, "/echo", None).await;
    assert_eq!(
        response.headers().get(WWW_AUTHENTICATE),
        Some(&"Bearer".parse().expect("a static header value"))
    );
    let error = error_body(response, StatusCode::UNAUTHORIZED, "unauthorized").await;
    assert_eq!(error["details"], json!({}), "the default details");
}

#[tokio::test]
async fn a_role_below_the_route_is_a_forbidden() {
    // ADR-0100, decision 8: `403`, and no `WWW-Authenticate`.
    let router = router(Arc::new(TestState::new(2)));
    let response = send(&router, Method::GET, "/admin", Some(READ_KEY)).await;
    assert!(response.headers().get(WWW_AUTHENTICATE).is_none());
    error_body(response, StatusCode::FORBIDDEN, "forbidden").await;
}

#[tokio::test]
async fn an_unknown_route_names_its_method_and_its_path() {
    // ADR-0101, decision 5.
    let router = router(Arc::new(TestState::new(2)));
    let response = send(&router, Method::GET, "/no-such-route", None).await;
    let error = error_body(response, StatusCode::NOT_FOUND, "not_found").await;
    assert_eq!(error["message"], "no route for GET /no-such-route");
}

#[tokio::test]
async fn a_method_not_allowed_on_a_known_route_is_a_not_found() {
    // ADR-0101, decision 5: no `405` ever leaves the server.
    let router = router(Arc::new(TestState::new(2)));
    let response = send(&router, Method::DELETE, "/health", None).await;
    assert_ne!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    let error = error_body(response, StatusCode::NOT_FOUND, "not_found").await;
    assert_eq!(error["message"], "no route for DELETE /health");
}

#[tokio::test]
async fn a_malformed_json_body_is_an_invalid_request() {
    // ADR-0101, decision 6: axum's `400` takes the format and the code.
    let router = router(Arc::new(TestState::new(2)));
    let response = send_json(
        &router,
        "/echo",
        Some(READ_KEY),
        Some("application/json"),
        r#"{"a": "#,
    )
    .await;
    error_body(response, StatusCode::BAD_REQUEST, "invalid_request").await;
}

#[tokio::test]
async fn a_body_without_the_json_content_type_is_an_invalid_request() {
    // ADR-0101, decision 6: `415` never leaves the server; a wrong type and an
    // absent one share the `400 invalid_request`.
    let router = router(Arc::new(TestState::new(2)));
    for content_type in [Some("text/plain"), None] {
        let response = send_json(&router, "/echo", Some(READ_KEY), content_type, "{}").await;
        error_body(response, StatusCode::BAD_REQUEST, "invalid_request").await;
    }
}

#[tokio::test]
async fn a_timeout_ms_outside_the_bounds_is_an_invalid_request() {
    // ADR-0102, decision 3, through the mechanism the search routes call.
    let router = router(Arc::new(TestState::new(2)));
    for refused in [0, 30_001] {
        let body = json!({ "timeout_ms": refused, "work_ms": 0 }).to_string();
        let response = send_json(
            &router,
            "/search",
            Some(READ_KEY),
            Some("application/json"),
            &body,
        )
        .await;
        let error = error_body(response, StatusCode::BAD_REQUEST, "invalid_request").await;
        assert!(
            error["message"]
                .as_str()
                .is_some_and(|text| text.contains("timeout_ms")),
            "the message names the field: {error}"
        );
    }
}

#[tokio::test]
async fn a_body_above_32_mib_is_a_payload_too_large() {
    // ADR-0102, decision 1: one bound for the whole instance, in bytes. The
    // declared length alone is enough: the layer refuses before reading. The
    // path without a declared length is proved in `http.rs`, where the bound is
    // a parameter of the layer so the test does not send 32 MiB.
    let router = router(Arc::new(TestState::new(2)));
    let request = Request::builder()
        .method(Method::POST)
        .uri("/echo")
        .header(REQUEST_ID_HEADER, REQUEST_ID)
        .header(AUTHORIZATION, format!("Bearer {READ_KEY}"))
        .header(CONTENT_TYPE, "application/json")
        .header("content-length", (MAX_BODY_BYTES + 1).to_string())
        .body(Body::empty())
        .expect("the request of the test is well formed");
    let response = call(&router, request).await;
    let error = error_body(response, StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large").await;
    assert_eq!(error["details"], json!({}), "the default details");
}

#[tokio::test]
async fn a_search_over_its_deadline_is_a_timeout_and_keeps_its_token() {
    // ADR-0102, decisions 2 and 3: `504` at the deadline, the work is not
    // cancelled, and its token stays taken until it really ends; a search that
    // finds no token is refused at once with `Retry-After: 1`.
    //
    // The work waits for a rendezvous of the test, which is only given after
    // the assertions below: no duration is compared, so the outcome does not
    // depend on the load of the machine.
    let state = Arc::new(TestState::new(1));
    let router = router(Arc::clone(&state));
    let response = send_json(
        &router,
        "/search",
        Some(READ_KEY),
        Some("application/json"),
        r#"{"timeout_ms": 10, "wait": true}"#,
    )
    .await;
    error_body(response, StatusCode::GATEWAY_TIMEOUT, "timeout").await;

    // The token of that search is still taken, and the refusal does not wait
    // for it: an implementation that queued would answer only after the
    // rendezvous, so the guard below would expire instead of the refusal
    // arriving.
    let refused = tokio::time::timeout(
        Duration::from_secs(30),
        send_json(
            &router,
            "/search",
            Some(READ_KEY),
            Some("application/json"),
            "{}",
        ),
    )
    .await
    .expect("the refusal does not wait for a token");
    assert_eq!(
        refused.headers().get(RETRY_AFTER),
        Some(&"1".parse().expect("a static header value")),
        "the refusal says when to retry (ADR-0102)"
    );
    error_body(refused, StatusCode::SERVICE_UNAVAILABLE, "overloaded").await;
    // The limit covers the search routes only: a probe answers during the
    // overload (ADR-0102, decision 2, and ADR-0099).
    assert_eq!(
        send(&router, Method::GET, "/health", None).await.status(),
        StatusCode::OK,
        "a probe is never refused by the search limit"
    );

    // The work ends, and the token comes back with it. `limits.rs` proves the
    // return of the token itself, without a sleep.
    state.release();
}

#[tokio::test]
async fn a_panicking_handler_is_a_generic_internal() {
    // ADR-0101, decision 7: the client receives the fixed generic message, and
    // the cause never reaches the body.
    let router = router(Arc::new(TestState::new(2)));
    let response = send(&router, Method::GET, "/panic", None).await;
    let error = error_body(response, StatusCode::INTERNAL_SERVER_ERROR, "internal").await;
    assert_eq!(error["message"], GENERIC_INTERNAL);
    assert_eq!(error["details"], json!({}));
}

#[tokio::test]
async fn a_bare_503_of_a_layer_is_an_overloaded() {
    // ADR-0101, decision 6, last resort: a `503` that no route rendered takes
    // the format and the code of the table, `overloaded`, so an overload never
    // becomes an `internal`. `/ready` is the other `503` of the server, and it
    // keeps its own body (ADR-0099).
    let router = router(Arc::new(TestState::new(2)));
    let response = send(&router, Method::GET, "/unavailable", None).await;
    assert_eq!(
        response.headers().get(RETRY_AFTER),
        Some(&"1".parse().expect("a static header value")),
        "every overloaded carries Retry-After: 1, whatever produced it (ADR-0103)"
    );
    let error = error_body(response, StatusCode::SERVICE_UNAVAILABLE, "overloaded").await;
    assert_eq!(error["details"], json!({}), "the default details");
}

#[tokio::test]
async fn an_internal_of_a_route_keeps_its_cause_out_of_the_body() {
    // ADR-0101, decisions 2 and 4: the client reads the generic message and the
    // identifier, the journal reads the cause of the failure.
    let router = router(Arc::new(TestState::new(2)));
    let response = send(&router, Method::GET, "/internal", None).await;
    let error = error_body(response, StatusCode::INTERNAL_SERVER_ERROR, "internal").await;
    assert_eq!(error["message"], GENERIC_INTERNAL);
    assert!(
        !error.to_string().contains("zz-cause-zz"),
        "the cause of an internal never reaches the body: {error}"
    );
}

#[test]
fn the_journal_names_errors_without_any_client_message() {
    // ADR-0101, decision 4: one line per error, with its code, its status and
    // its `request_id`; the message of a `4xx` is never journalised, and the
    // cause of an `internal` is, because it is the diagnostic of that code.
    // ADR-0096: no value the client sent reaches the journal, whatever the
    // target of the line.
    let sink = Sink::default();
    let subscriber = log_subscriber(sink.clone(), "trace").expect("trace is a valid filter");
    // A global subscriber, as `init_logging` installs it in the server: a
    // thread-local default would not see the events of a request served on
    // another thread, and `tracing` caches the interest of a callsite for the
    // whole process until a global subscriber is set.
    subscriber
        .try_init()
        .expect("no other test of this binary installs a journal");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime is startable");
    runtime.block_on(async {
        let router = router(Arc::new(TestState::new(2)));
        let refused = send_json(
            &router,
            "/echo",
            Some(READ_KEY),
            Some("application/json"),
            r#"{"filter_value": "zz-4xx-zz", "#,
        )
        .await;
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
        // A valid JSON body of the wrong type: serde quotes the value it
        // refused, and axum journals that text on its own target at `TRACE`.
        let mistyped = send_json(
            &router,
            "/typed",
            Some(READ_KEY),
            Some("application/json"),
            r#""zz-typed-value-zz""#,
        )
        .await;
        assert_eq!(mistyped.status(), StatusCode::BAD_REQUEST);
        let unknown = send(&router, Method::GET, "/no-such-route", None).await;
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
        let panicked = send(&router, Method::GET, "/panic", None).await;
        assert_eq!(panicked.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let failed = send(&router, Method::GET, "/internal", None).await;
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
