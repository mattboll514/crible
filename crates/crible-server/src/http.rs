// SPDX-License-Identifier: Apache-2.0

//! Router of the server: the two probes and the request identifier.
//!
//! References: ADR-0098 (`x-request-id`), ADR-0099 (`/health`, `/ready`) and
//! `docs/05-api.md` (conventions). The `/v1` routes arrive with the tasks that
//! create them and mount on this router; nothing here anticipates their shape.
//!
//! `/health` answers `200` from the bind, whatever the loader is doing, and
//! `/ready` answers `503` until the models are loaded and every index has
//! published its snapshot. Neither probe asks for authentication
//! (`docs/05-api.md`, role "—"), and neither body carries an index name, a
//! field name, a path or a detailed count (ADR-0099): the body only tells
//! "alive" from "ready".
//!
//! Every response, including a `404` rendered before any route, carries
//! `x-request-id`. The value is the caller's header when it holds at most 128
//! bytes of `[A-Za-z0-9._:-]`; otherwise a UUID v4 is generated and the refused
//! value is never journalised (ADR-0098).

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::{Instrument, Level, error, info_span};
use uuid::Uuid;

use crate::AppState;

/// Header carrying the request identifier on every response (ADR-0098).
pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// Longest request identifier accepted from the caller, in bytes (ADR-0098).
const MAX_REQUEST_ID_BYTES: usize = 128;

/// Identifier of the request being served, for the error body and the logs.
///
/// `AUTHENTIFIER-CLES` and `FORMAT-ERREURS-ET-LIMITES` read it; the probes of
/// this task do not need it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestId(pub String);

/// Builds the router of an instance.
///
/// The state is the one created at the bind: the router answers `503` on
/// `/ready` until [`AppState::mark_ready`] is called.
pub fn build_router(state: Arc<AppState>) -> Router {
    with_common_layers(
        Router::new()
            .route("/health", get(health))
            .route("/ready", get(ready)),
    )
    .with_state(state)
}

/// Applies the layers every router of the server shares.
///
/// The trace layer and the request identifier are mounted here so a route
/// added by another task, or a test router of the middleware, carries them the
/// same way: a `401` rendered before a route still names its request (ADR-0098),
/// and no log line records the path with its query string (ADR-0096).
pub fn with_common_layers<S>(router: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router
        .layer(
            TraceLayer::new_for_http()
                // The path only: `docs/06` forbids a query text and a filter
                // value in a log at any level, and `RUST_LOG` makes `debug`
                // reachable. The default span of `TraceLayer` records the whole
                // URI, query string included.
                .make_span_with(|request: &Request<Body>| {
                    info_span!(
                        "http",
                        method = %request.method(),
                        path = %request.uri().path()
                    )
                })
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(middleware::from_fn(assign_request_id))
}

/// `/health`: the process is alive.
///
/// Fixed body, independent of the indexes and of the models: it answers during
/// a long recovery, which is the reason the server binds before loading
/// (ADR-0099).
async fn health() -> (StatusCode, Json<Value>) {
    (StatusCode::OK, Json(json!({ "status": "ok" })))
}

/// `/ready`: models loaded, every index published, no recovery in progress.
///
/// `503` with `{"status":"not_ready"}` until the loader has finished, `200` with
/// `{"status":"ready"}` after. The two bodies tell "alive" from "ready" and
/// reveal nothing else (ADR-0099).
async fn ready(State(state): State<Arc<AppState>>) -> (StatusCode, Json<Value>) {
    if state.is_ready() {
        (StatusCode::OK, Json(json!({ "status": "ready" })))
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "status": "not_ready" })),
        )
    }
}

/// Assigns the request identifier and returns it on the response.
///
/// The layer is the outermost one: a `404`, a `405` or a rejection rendered
/// before any route still carries the header (ADR-0098). The span carries the
/// identifier so every line logged during the request names it.
async fn assign_request_id(mut request: Request, next: Next) -> Response {
    let identifier = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(accepted_request_id)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    request
        .extensions_mut()
        .insert(RequestId(identifier.clone()));
    let span = info_span!("request", request_id = %identifier);
    let mut response = next.run(request).instrument(span).await;
    match HeaderValue::from_str(&identifier) {
        Ok(value) => {
            response
                .headers_mut()
                .insert(HeaderName::from_static(REQUEST_ID_HEADER), value);
        }
        Err(_) => {
            // Unreachable: the identifier is either a validated ASCII value or
            // a UUID. Logged rather than ignored, because a response without
            // the header would break the trace with no diagnostic.
            error!("the request identifier cannot be sent back as a header value");
        }
    }
    response
}

/// Returns the caller's identifier when it is acceptable, otherwise `None`.
///
/// ADR-0098: at most 128 bytes, and only `[A-Za-z0-9._:-]`. A refused value is
/// dropped without a trace: its length and its content are never journalised.
fn accepted_request_id(value: &HeaderValue) -> Option<String> {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_REQUEST_ID_BYTES {
        return None;
    }
    let accepted = bytes.iter().all(|byte| {
        matches!(
            byte,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b':' | b'-'
        )
    });
    if !accepted {
        return None;
    }
    std::str::from_utf8(bytes).ok().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{REQUEST_ID_HEADER, build_router};
    use crate::state::AppState;
    use axum::Router;
    use axum::body::Body;
    use axum::extract::Request;
    use axum::http::StatusCode;
    use axum::response::Response;
    use std::sync::Arc;
    use tower::ServiceExt;

    /// Sends one `GET`, with an optional caller identifier, through the router.
    async fn get(router: &Router, path: &str, identifier: Option<&str>) -> Response {
        let mut request = Request::builder().uri(path);
        if let Some(identifier) = identifier {
            request = request.header(REQUEST_ID_HEADER, identifier);
        }
        let request = request
            .body(Body::empty())
            .expect("the request is well formed");
        router
            .clone()
            .oneshot(request)
            .await
            .expect("the router always answers")
    }

    fn identifier_of(response: &Response) -> String {
        let value = response
            .headers()
            .get(REQUEST_ID_HEADER)
            .unwrap_or_else(|| panic!("every response carries {REQUEST_ID_HEADER}"));
        value.to_str().expect("the identifier is ASCII").to_owned()
    }

    /// Whether `value` has the canonical form of a UUID v4.
    fn is_uuid_v4(value: &str) -> bool {
        let bytes = value.as_bytes();
        bytes.len() == 36
            && bytes.iter().enumerate().all(|(index, byte)| match index {
                8 | 13 | 18 | 23 => *byte == b'-',
                14 => *byte == b'4',
                _ => byte.is_ascii_digit() || (b'a'..=b'f').contains(byte),
            })
    }

    #[tokio::test]
    async fn a_valid_incoming_identifier_is_echoed() {
        // First state of ADR-0098: reprise of the caller's header.
        let router = build_router(Arc::new(AppState::new()));
        let response = get(&router, "/health", Some("mcp-42:retry.1_abc-DEF")).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(identifier_of(&response), "mcp-42:retry.1_abc-DEF");
    }

    #[tokio::test]
    async fn an_absent_identifier_is_generated() {
        let router = build_router(Arc::new(AppState::new()));
        let response = get(&router, "/health", None).await;
        let identifier = identifier_of(&response);
        assert!(is_uuid_v4(&identifier), "got {identifier}");
    }

    #[tokio::test]
    async fn an_identifier_longer_than_128_bytes_is_replaced() {
        let router = build_router(Arc::new(AppState::new()));
        let too_long = "a".repeat(129);
        let response = get(&router, "/health", Some(&too_long)).await;
        let identifier = identifier_of(&response);
        assert_ne!(identifier, too_long);
        assert!(is_uuid_v4(&identifier), "got {identifier}");
    }

    #[tokio::test]
    async fn an_identifier_with_a_forbidden_character_is_replaced() {
        // ADR-0098: a space or any character outside `[A-Za-z0-9._:-]` makes
        // the value refused, never journalised. A value with a control
        // character never reaches the router: `HeaderValue` refuses to build it.
        let router = build_router(Arc::new(AppState::new()));
        for refused in ["two words", "a/b", "a+b"] {
            let response = get(&router, "/health", Some(refused)).await;
            let identifier = identifier_of(&response);
            assert_ne!(identifier, refused);
            assert!(is_uuid_v4(&identifier), "got {identifier}");
        }
    }

    #[tokio::test]
    async fn a_response_before_any_route_carries_the_identifier() {
        // ADR-0098: a `404` rendered by the fallback still carries the header,
        // because the layer is the outermost one.
        let router = build_router(Arc::new(AppState::new()));
        let response = get(&router, "/no-such-route", None).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(is_uuid_v4(&identifier_of(&response)));
    }

    #[tokio::test]
    async fn the_probes_tell_alive_from_ready() {
        // `docs/06` and ADR-0099: two bodies, two meanings. The state is not
        // ready until the loader publishes it.
        let state = Arc::new(AppState::new());
        let router = build_router(Arc::clone(&state));

        let health = get(&router, "/health", None).await;
        assert_eq!(health.status(), StatusCode::OK);
        let ready = get(&router, "/ready", None).await;
        assert_eq!(ready.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            body_json(ready).await["status"],
            "not_ready",
            "the body of a not-ready instance names its state"
        );

        state.mark_ready();
        let ready = get(&router, "/ready", None).await;
        assert_eq!(ready.status(), StatusCode::OK);
        assert_eq!(body_json(ready).await["status"], "ready");
        assert_eq!(
            body_json(get(&router, "/health", None).await).await["status"],
            "ok",
            "the two bodies stay distinct"
        );
    }

    #[tokio::test]
    async fn the_probes_need_no_authentication() {
        // `docs/05-api.md` gives the two probes the role "—": no `Authorization`
        // header is sent here, and both answer on their own status.
        let state = Arc::new(AppState::new());
        state.mark_ready();
        let router = build_router(state);
        assert_ne!(
            get(&router, "/health", None).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_ne!(
            get(&router, "/ready", None).await.status(),
            StatusCode::UNAUTHORIZED
        );
    }

    /// Decodes the JSON body of a response.
    async fn body_json(response: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("the body is readable");
        serde_json::from_slice(&bytes).expect("the body of a probe is JSON")
    }

    #[test]
    fn the_identifier_header_is_the_documented_one() {
        assert_eq!(REQUEST_ID_HEADER, "x-request-id");
    }

    #[test]
    fn only_the_documented_characters_are_accepted() {
        for value in ["abc", "A-Z.a_z:0"] {
            let header = value.parse().expect("the test value is a header");
            assert_eq!(
                super::accepted_request_id(&header).as_deref(),
                Some(value),
                "ADR-0098 accepts {value}"
            );
        }
        for value in ["", " ", "a b", "+"] {
            let header = value.parse().expect("the test value is a header");
            assert!(
                super::accepted_request_id(&header).is_none(),
                "ADR-0098 refuses {value:?}"
            );
        }
    }
}
