// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixtures of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! The `Bearer` middleware seen from outside the crate.
//!
//! Acceptance criteria of `AUTHENTIFIER-CLES`
//! (`docs/phases/P6-serveur-et-api.md`): the key in clear and its digest never
//! appear in the logs, whatever `RUST_LOG` says (ADR-0096, ADR-0100 decision 9),
//! and a refused request still carries its request identifier (ADR-0098).
//!
//! The test router is the one ADR-0095 asks for: the `/v1` routes and their
//! `403` tests arrive with the tasks that create them.

use std::io;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::http::header::AUTHORIZATION;
use axum::response::Response;
use axum::routing::get;
use crible_server::auth::{Auth, Client, protect, sha256_hex, warn_if_empty};
use crible_server::config::{ApiKey, Role};
use crible_server::http::{REQUEST_ID_HEADER, with_common_layers};
use crible_server::logging::log_subscriber;
use serde_json::Value;
use tower::ServiceExt;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt;

/// Key of the `read` role used by the tests.
const READ_KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Key unknown to the instance: no route knows it, so every one answers `401`
/// with the body of ADR-0100, the `admin` route included; the `403` cases use
/// the `read` key on `admin` instead.
const UNKNOWN_KEY: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

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

/// Router of the test: one route per role level, plus an open probe.
fn router() -> Router {
    let auth = Arc::new(
        Auth::new(&[ApiKey {
            name: "mcp-client".to_owned(),
            role: Role::Read,
            sha256: sha256_hex(READ_KEY),
        }])
        .expect("the declared key holds a valid digest"),
    );
    let read = protect(
        Router::new().route("/read", get(level)),
        auth.clone(),
        Role::Read,
    );
    let admin = protect(Router::new().route("/admin", get(level)), auth, Role::Admin);
    let open = Router::new().route("/health", get(|| async { "ok" }));
    with_common_layers(read.merge(admin).merge(open))
}

/// Answers with the name and the role the middleware attached, when it did.
async fn level(request: Request) -> Response {
    match request.extensions().get::<Client>() {
        Some(client) => Response::new(Body::from(format!(
            "{}:{}",
            client.name,
            client.role.as_str()
        ))),
        None => Response::new(Body::from("no client")),
    }
}

/// Sends one `GET` carrying `authorization`, if any.
async fn send(router: &Router, path: &str, authorization: Option<&str>) -> Response {
    let mut request = Request::builder()
        .uri(path)
        .header(REQUEST_ID_HEADER, "auth-test");
    if let Some(value) = authorization {
        request = request.header(AUTHORIZATION, value);
    }
    router
        .clone()
        .oneshot(
            request
                .body(Body::empty())
                .expect("the request is well formed"),
        )
        .await
        .expect("the router answers")
}

/// Decodes the JSON body of a response.
async fn json_body(response: Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body is readable");
    serde_json::from_slice(&bytes).expect("the body is JSON")
}

#[test]
fn an_empty_key_list_is_announced_once() {
    // ADR-0100, decision 4: the startup accepts an empty `[[auth.keys]]` and
    // says so once, instead of letting every authenticated route fail silently.
    let sink = Sink::default();
    let subscriber = log_subscriber(sink.clone(), "info").expect("info is a valid filter");
    tracing::subscriber::with_default(subscriber, || {
        warn_if_empty(&[]);
        warn_if_empty(&[ApiKey {
            name: "mcp-client".to_owned(),
            role: Role::Read,
            sha256: sha256_hex(READ_KEY),
        }]);
    });
    let text = sink.contents();
    assert_eq!(
        text.lines().count(),
        1,
        "one line for an empty list: {text}"
    );
    assert!(text.contains("[[auth.keys]]"), "got {text}");
    assert!(text.contains("401"), "got {text}");
}

#[test]
fn the_key_and_its_digest_never_reach_the_logs() {
    // ADR-0100, decision 9: at `trace`, the highest level `RUST_LOG` can ask
    // for, no line holds the header, the key or the digest; the name of the key
    // may appear in `client`.
    let sink = Sink::default();
    let subscriber = log_subscriber(sink.clone(), "trace").expect("trace is a valid filter");
    // A global subscriber, as `init_logging` installs it in the server: a
    // thread-local default would not see the events of the runtime, and the
    // property must hold whatever thread emits the line.
    subscriber
        .try_init()
        .expect("no other test of this binary installs a journal");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime is startable");
    let (accepted, refused, forbidden) = runtime.block_on(async {
        let router = router();
        let accepted = send(&router, "/read", Some(&format!("Bearer {READ_KEY}"))).await;
        let refused = send(&router, "/read", Some(&format!("Bearer {UNKNOWN_KEY}"))).await;
        let forbidden = send(&router, "/admin", Some(&format!("Bearer {READ_KEY}"))).await;
        (accepted.status(), refused.status(), forbidden.status())
    });
    assert_eq!(accepted, StatusCode::OK);
    assert_eq!(refused, StatusCode::UNAUTHORIZED);
    assert_eq!(forbidden, StatusCode::FORBIDDEN);

    let text = sink.contents();
    assert!(
        text.contains("mcp-client"),
        "the test must capture the journal of an authenticated request: {text:?}"
    );
    for secret in [READ_KEY, UNKNOWN_KEY] {
        assert!(
            !text.contains(secret),
            "a key in clear reached the logs: {text}"
        );
        assert!(
            !text.contains(&sha256_hex(secret)),
            "a digest reached the logs: {text}"
        );
    }
    assert!(
        !text.contains("Bearer"),
        "the Authorization header reached the logs: {text}"
    );
}

#[tokio::test]
async fn an_open_route_needs_no_key_and_a_refusal_names_its_request() {
    // ADR-0099: a route without a role stays open. ADR-0098: a `401` carries
    // the request identifier of the response in its body as well.
    let router = router();
    assert_eq!(
        send(&router, "/health", None).await.status(),
        StatusCode::OK
    );

    let response = send(&router, "/read", None).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response
            .headers()
            .get(REQUEST_ID_HEADER)
            .and_then(|value| value.to_str().ok()),
        Some("auth-test")
    );
    let body = json_body(response).await;
    assert_eq!(body["error"]["code"], "unauthorized");
    assert_eq!(body["error"]["request_id"], "auth-test");
}

#[tokio::test]
async fn the_read_key_reaches_its_level_and_not_above() {
    let router = router();
    let response = send(&router, "/read", Some(&format!("Bearer {READ_KEY}"))).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body is readable");
    assert_eq!(String::from_utf8_lossy(&bytes), "mcp-client:read");

    let response = send(&router, "/admin", Some(&format!("Bearer {READ_KEY}"))).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(response.headers().get("www-authenticate").is_none());
    assert_eq!(json_body(response).await["error"]["code"], "forbidden");
}
