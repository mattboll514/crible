// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and several fixtures and checks of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! The routes of the API while the startup is still loading (ADR-0146).
//!
//! The server binds before it loads, and that is decided (ADR-0099). Until the
//! loader publishes the index registry and the state opened with it, every route
//! that reads them renders `503 overloaded` with the fixed message
//! "the server is starting", `details: {}`, `x-request-id` and `Retry-After: 1`
//! (ADR-0146, decisions 1 and 2). `/health` and `/ready` keep the meaning of
//! ADR-0099 (decision 4).
//!
//! What each test pins:
//!
//! | Test | Criterion of the ADR |
//! |---|---|
//! | `the_startup_refuses_the_index_registry_routes` | the five administration routes, `503` instead of `200 []` and `404` |
//! | `the_startup_refuses_the_maintenance_routes` | `compact`, `exports`, `snapshots` |
//! | `the_startup_refuses_the_document_routes` | the two writes and the read of a document |
//! | `the_startup_refuses_the_batch_route` | `batches` |
//! | `the_startup_refuses_the_simple_search` | `503` instead of `404 unknown_index` |
//! | `the_startup_refuses_the_multi_index_search` | `503` instead of `404` naming the index |
//! | `the_startup_refuses_the_job_routes` | `503` instead of `200 {"jobs":[]}` and `404` |
//! | `the_startup_refuses_the_statistics_route` | `503` instead of `500 internal` |
//! | `the_startup_refuses_the_debug_journal_route` | `503` instead of `200 enabled: false` |
//! | `a_loaded_instance_without_index_answers_200_with_empty_lists` | decision 6, the bound of the fix |
//! | `the_probes_keep_their_meaning_during_the_loading` | decision 4 |
//! | `an_invalid_body_is_refused_with_400_before_the_503` | decision 3, the order of ADR-0110 |
//! | `an_explicit_timeout_outside_its_bounds_is_refused_with_400_before_the_503` | decision 3, the bounds of `timeout_ms` come before the published configuration |
//! | `an_out_of_bounds_grouping_is_refused_with_400_before_the_503` | decision 3, the regrouping comes before the published configuration |
//! | `an_explicit_mode_without_its_inputs_is_refused_with_400_before_the_503` | decision 3, the inputs of an explicit mode come before the published configuration |
//! | `a_missing_key_is_refused_with_401_before_the_503` | decision 3, the order of ADR-0100 |
//!
//! The unloaded state is built **without a barrier**: `AppState::new()`, then the
//! authenticator, then the router. It is the same state as during a real loading,
//! without depending on a scheduling (ADR-0146, decision 8).
//!
//! ```text
//! cargo test -p crible-server --test startup_loading
//! ```

use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, RETRY_AFTER};
use axum::http::{HeaderMap, Method, StatusCode};
use crible_server::config::{ApiKey, Config, Role};
use crible_server::http::{REQUEST_ID_HEADER, build_router};
use crible_server::state::AppState;
use crible_server::{Auth, auth::sha256_hex};
use serde_json::{Value, json};
use tower::ServiceExt;

/// Secret of the `admin` key.
const ADMIN_SECRET: &str = "admin-secret-of-the-startup-loading-tests";

/// Secret of the `write` key.
const WRITE_SECRET: &str = "write-secret-of-the-startup-loading-tests";

/// Secret of the `read` key.
const READ_SECRET: &str = "read-secret-of-the-startup-loading-tests";

/// Identifier pinned on every request (ADR-0098).
const REQUEST_ID: &str = "startup-loading-test-1";

/// The fixed message of the `503` of ADR-0146, decision 1.
const STARTING_MESSAGE: &str = "the server is starting";

/// The name every refused route names, so a body that leaked it would be seen.
const INDEX: &str = "alpha";

/// One answer of the router, decoded.
struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

impl Answer {
    /// Whether the body carries `key`, which the wrong pre-startup answers did.
    fn has(&self, key: &str) -> bool {
        self.body.get(key).is_some()
    }
}

/// Builds the state of an instance whose loading has not published anything.
///
/// No barrier is armed: `loaded` stays `None`, which is exactly the state of a
/// real loading between the bind and the publication of the last snapshot
/// (ADR-0146, decision 8).
fn unloaded_router() -> Router {
    let state = Arc::new(AppState::new());
    let auth = Auth::new(&[
        ApiKey {
            name: "administrator".to_owned(),
            role: Role::Admin,
            sha256: sha256_hex(ADMIN_SECRET),
        },
        ApiKey {
            name: "writer".to_owned(),
            role: Role::Write,
            sha256: sha256_hex(WRITE_SECRET),
        },
        ApiKey {
            name: "reader".to_owned(),
            role: Role::Read,
            sha256: sha256_hex(READ_SECRET),
        },
    ])
    .unwrap_or_else(|error| panic!("the keys are valid: {}", error.message()));
    state
        .set_auth(Arc::new(auth))
        .unwrap_or_else(|error| panic!("the authenticator is published: {}", error.message()));
    build_router(state)
}

/// Sends one request, with the pinned identifier and the given key.
async fn send(
    router: &Router,
    method: Method,
    path: &str,
    secret: Option<&str>,
    body: Option<Value>,
) -> Answer {
    let mut request = axum::http::Request::builder()
        .method(method)
        .uri(path)
        .header("x-request-id", REQUEST_ID);
    if let Some(secret) = secret {
        request = request.header(AUTHORIZATION, format!("Bearer {secret}"));
    }
    let payload = match body {
        Some(value) => {
            request = request.header(CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = router
        .clone()
        .oneshot(request.body(payload).expect("the request is valid"))
        .await
        .expect("the router answers");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("the body is readable");
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer {
        status,
        headers,
        body,
    }
}

/// Checks the whole `503` of ADR-0146, decisions 1 and 2.
fn assert_starting(answer: &Answer, route: &str) {
    assert_eq!(
        answer.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{route} must answer 503 before the publication: {}",
        answer.body
    );
    assert_eq!(
        answer.body["error"]["code"], "overloaded",
        "{route} carries the code of a resource to retry: {}",
        answer.body
    );
    assert_eq!(
        answer.body["error"]["message"], STARTING_MESSAGE,
        "{route} carries the fixed message of ADR-0146: {}",
        answer.body
    );
    assert_eq!(
        answer.body["error"]["details"],
        json!({}),
        "{route} adds no key to `details` (ADR-0101): {}",
        answer.body
    );
    assert_eq!(
        answer.body["error"]["request_id"], REQUEST_ID,
        "{route} echoes the request identifier in its body (ADR-0098)"
    );
    assert_eq!(
        answer
            .headers
            .get(REQUEST_ID_HEADER)
            .and_then(|value| value.to_str().ok()),
        Some(REQUEST_ID),
        "{route} carries `x-request-id` (ADR-0098)"
    );
    assert_eq!(
        answer
            .headers
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok()),
        Some("1"),
        "{route} carries `Retry-After: 1`, set by the error renderer (ADR-0103, decision 9)"
    );
    assert_eq!(
        answer.body.as_object().map(serde_json::Map::len),
        Some(1),
        "{route} answers the error envelope alone: {}",
        answer.body
    );
}

/// Configuration of an index that needs no encoder and no lexical branch, as the
/// `POST /v1/indexes` route reads it.
fn index_config(name: &str) -> Value {
    json!({
        "name": name,
        "model": "m",
        "reference_precision": "fp16",
        "schema": {
            "document_fields": { "numero": { "type": "keyword" } },
            "passage_fields": { "texte": { "type": "keyword" } }
        },
        "cluster_by": [],
        "lexical": { "enabled": false, "language": "fr", "positions": false },
        "search": {
            "scan": "none",
            "rescore_candidates": 10,
            "exact_threshold": 10,
            "candidates_per_branch": 10,
            "fusion": { "method": "rrf" }
        },
        "ingest": { "acceptance_sample": 1, "acceptance_min_cosine": 0.9 },
        "compaction": { "dead_ratio": 0.1 }
    })
}

#[tokio::test]
async fn the_startup_refuses_the_index_registry_routes() {
    // ADR-0146, decision 1: the five administration routes. Before the fix, the
    // list answered `200 {"indexes":[]}`, the detail, the patch and the delete a
    // `404 unknown_index`, and the creation a `500 internal`.
    let router = unloaded_router();

    let list = send(&router, Method::GET, "/v1/indexes", Some(READ_SECRET), None).await;
    assert_starting(&list, "GET /v1/indexes");
    assert!(
        !list.has("indexes"),
        "an empty list must no longer answer for a registry that is not loaded: {}",
        list.body
    );

    let create = send(
        &router,
        Method::POST,
        "/v1/indexes",
        Some(ADMIN_SECRET),
        Some(index_config(INDEX)),
    )
    .await;
    assert_starting(&create, "POST /v1/indexes");

    let detail = send(
        &router,
        Method::GET,
        &format!("/v1/indexes/{INDEX}"),
        Some(READ_SECRET),
        None,
    )
    .await;
    assert_starting(&detail, "GET /v1/indexes/{index}");

    let patch = send(
        &router,
        Method::PATCH,
        &format!("/v1/indexes/{INDEX}"),
        Some(ADMIN_SECRET),
        Some(json!({})),
    )
    .await;
    assert_starting(&patch, "PATCH /v1/indexes/{index}");

    let delete = send(
        &router,
        Method::DELETE,
        &format!("/v1/indexes/{INDEX}?confirm={INDEX}"),
        Some(ADMIN_SECRET),
        None,
    )
    .await;
    assert_starting(&delete, "DELETE /v1/indexes/{index}");
}

#[tokio::test]
async fn the_startup_refuses_the_maintenance_routes() {
    // ADR-0146, decision 1: the three maintenance routes of the registry.
    let router = unloaded_router();
    for route in ["compact", "exports", "snapshots"] {
        let answer = send(
            &router,
            Method::POST,
            &format!("/v1/indexes/{INDEX}/{route}"),
            Some(ADMIN_SECRET),
            None,
        )
        .await;
        assert_starting(&answer, route);
    }
}

#[tokio::test]
async fn the_startup_refuses_the_document_routes() {
    // ADR-0146, decision 1: the two writes and the read of a document, which
    // answered `404 unknown_index` before the fix.
    let router = unloaded_router();
    let path = format!("/v1/indexes/{INDEX}/documents");

    let post = send(
        &router,
        Method::POST,
        &path,
        Some(WRITE_SECRET),
        Some(json!({})),
    )
    .await;
    assert_starting(&post, "POST /v1/indexes/{index}/documents");

    let patch = send(
        &router,
        Method::PATCH,
        &path,
        Some(WRITE_SECRET),
        Some(json!({})),
    )
    .await;
    assert_starting(&patch, "PATCH /v1/indexes/{index}/documents");

    let read = send(
        &router,
        Method::GET,
        &format!("/v1/indexes/{INDEX}/documents/doc-1"),
        Some(READ_SECRET),
        None,
    )
    .await;
    assert_starting(&read, "GET /v1/indexes/{index}/documents/{doc_id}");
}

#[tokio::test]
async fn the_startup_refuses_the_batch_route() {
    // ADR-0146, decision 1: the attach of a deposited batch.
    let router = unloaded_router();
    let answer = send(
        &router,
        Method::POST,
        &format!("/v1/indexes/{INDEX}/batches"),
        Some(WRITE_SECRET),
        Some(json!({})),
    )
    .await;
    assert_starting(&answer, "POST /v1/indexes/{index}/batches");
}

#[tokio::test]
async fn the_startup_refuses_the_simple_search() {
    // ADR-0146, decision 1: `POST /v1/indexes/{index}/search` answered
    // `404 unknown_index` before the fix.
    let router = unloaded_router();
    let answer = send(
        &router,
        Method::POST,
        &format!("/v1/indexes/{INDEX}/search"),
        Some(READ_SECRET),
        Some(json!({ "query": "responsabilité du fait des choses" })),
    )
    .await;
    assert_starting(&answer, "POST /v1/indexes/{index}/search");
}

#[tokio::test]
async fn the_startup_refuses_the_multi_index_search() {
    // ADR-0146, decision 1: `POST /v1/search` answered a `404 not_found` naming
    // the index before the fix. The timeout step of the validation reads the
    // published configuration, so it must answer `503` too, never `500`.
    let router = unloaded_router();
    let answer = send(
        &router,
        Method::POST,
        "/v1/search",
        Some(READ_SECRET),
        Some(json!({ "query": "x", "targets": [{ "index": INDEX }] })),
    )
    .await;
    assert_starting(&answer, "POST /v1/search");
}

#[tokio::test]
async fn the_startup_refuses_the_job_routes() {
    // ADR-0146, decision 2: the job table is opened by the loading, so the list
    // answered `200 {"jobs":[]}` and the detail a `404` before the fix.
    let router = unloaded_router();

    let list = send(&router, Method::GET, "/v1/jobs", Some(WRITE_SECRET), None).await;
    assert_starting(&list, "GET /v1/jobs");
    assert!(
        !list.has("jobs"),
        "an empty list must no longer answer for a table that is not open: {}",
        list.body
    );

    let detail = send(
        &router,
        Method::GET,
        "/v1/jobs/job-1",
        Some(WRITE_SECRET),
        None,
    )
    .await;
    assert_starting(&detail, "GET /v1/jobs/{job_id}");
}

#[tokio::test]
async fn the_startup_refuses_the_statistics_route() {
    // ADR-0146, decision 2: the route answered `500 internal` before the fix,
    // which announced a failure that was not one.
    let router = unloaded_router();
    let answer = send(&router, Method::GET, "/v1/stats", Some(ADMIN_SECRET), None).await;
    assert_starting(&answer, "GET /v1/stats");
}

#[tokio::test]
async fn the_startup_refuses_the_debug_journal_route() {
    // ADR-0146, decision 2: `enabled: false` stays reserved to a journal that the
    // published configuration really disables (ADR-0122); before the publication
    // the route answers `503`.
    let router = unloaded_router();
    let answer = send(
        &router,
        Method::GET,
        "/v1/debug/queries",
        Some(ADMIN_SECRET),
        None,
    )
    .await;
    assert_starting(&answer, "GET /v1/debug/queries");
    assert!(
        !answer.has("enabled"),
        "a published configuration decides `enabled`, not the loading: {}",
        answer.body
    );
}

#[tokio::test]
async fn the_probes_keep_their_meaning_during_the_loading() {
    // ADR-0146, decision 4: `/health` answers from the bind and `/ready` says
    // "not ready"; neither takes the `503 overloaded` of the registry.
    let router = unloaded_router();

    let health = send(&router, Method::GET, "/health", None, None).await;
    assert_eq!(health.status, StatusCode::OK);
    assert_eq!(health.body["status"], "ok");

    let ready = send(&router, Method::GET, "/ready", None, None).await;
    assert_eq!(ready.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(ready.body["status"], "not_ready");
    assert!(
        ready.body.get("error").is_none(),
        "`/ready` keeps the body of ADR-0099, never the error format: {}",
        ready.body
    );
}

#[tokio::test]
async fn an_invalid_body_is_refused_with_400_before_the_503() {
    // ADR-0146, decision 3: the validation order of ADR-0110 is preserved. The
    // body is refused before the index is examined, so an unknown field is a
    // `400 invalid_request`, never the `503` of a registry that is not loaded.
    let router = unloaded_router();
    let answer = send(
        &router,
        Method::POST,
        "/v1/search",
        Some(READ_SECRET),
        Some(json!({
            "query": "x",
            "targets": [{ "index": INDEX }],
            "bogus": true
        })),
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::BAD_REQUEST,
        "the shape of the body comes first (ADR-0110, decision 7): {}",
        answer.body
    );
    assert_eq!(answer.body["error"]["code"], "invalid_request");
}

#[tokio::test]
async fn an_explicit_timeout_outside_its_bounds_is_refused_with_400_before_the_503() {
    // ADR-0146, decision 3 with ADR-0110, decision 7: the bounds of an explicit
    // `timeout_ms` belong to the step (a) of the validation. Reading the default
    // of `[limits]` needs the published configuration, so it comes after the
    // bounds: a value out of them stays a `400` while the server starts, instead
    // of a `503` that would hide a permanent fault of the body.
    let router = unloaded_router();
    for timeout_ms in [0, 30_001] {
        let answer = send(
            &router,
            Method::POST,
            "/v1/search",
            Some(READ_SECRET),
            Some(json!({
                "query": "x",
                "targets": [{ "index": INDEX }],
                "timeout_ms": timeout_ms
            })),
        )
        .await;
        assert_eq!(
            answer.status,
            StatusCode::BAD_REQUEST,
            "`timeout_ms: {timeout_ms}` is out of bounds before any configuration is read: {}",
            answer.body
        );
        assert_eq!(answer.body["error"]["code"], "invalid_request");
    }
}

#[tokio::test]
async fn an_out_of_bounds_grouping_is_refused_with_400_before_the_503() {
    // ADR-0146, decision 3 with ADR-0110, decision 7: the domain of
    // `passages_per_document` is checked before the published configuration.
    let router = unloaded_router();
    for passages_per_document in [0, 6] {
        let answer = send(
            &router,
            Method::POST,
            "/v1/search",
            Some(READ_SECRET),
            Some(json!({
                "query": "x",
                "targets": [{ "index": INDEX }],
                "passages_per_document": passages_per_document
            })),
        )
        .await;
        assert_eq!(
            answer.status,
            StatusCode::BAD_REQUEST,
            "`passages_per_document: {passages_per_document}` is refused before the registry is read: {}",
            answer.body
        );
        assert_eq!(answer.body["error"]["code"], "invalid_request");
    }
}

#[tokio::test]
async fn an_explicit_mode_without_its_inputs_is_refused_with_400_before_the_503() {
    // ADR-0146, decision 3 with ADR-0110, decision 7: the entries of an explicit
    // `mode` are checked before the published configuration, so `mode: dense`
    // without `query` or `vector` is a `400`, not the `503` of the startup.
    let router = unloaded_router();
    let answer = send(
        &router,
        Method::POST,
        "/v1/search",
        Some(READ_SECRET),
        Some(json!({
            "targets": [{ "index": INDEX }],
            "mode": "dense"
        })),
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::BAD_REQUEST,
        "an explicit mode without its entries is refused before the registry is read: {}",
        answer.body
    );
    assert_eq!(answer.body["error"]["code"], "invalid_request");
}

#[tokio::test]
async fn a_missing_key_is_refused_with_401_before_the_503() {
    // ADR-0146, decision 3: the authentication passes before the registry, as
    // ADR-0100 decides. A `401` never becomes a `503`.
    let router = unloaded_router();
    for (method, path) in [
        (Method::GET, "/v1/indexes"),
        (Method::GET, "/v1/jobs"),
        (Method::GET, "/v1/stats"),
    ] {
        let answer = send(&router, method.clone(), path, None, None).await;
        assert_eq!(
            answer.status,
            StatusCode::UNAUTHORIZED,
            "{method} {path} asks for a key before reading the registry: {}",
            answer.body
        );
        assert_eq!(answer.body["error"]["code"], "unauthorized");
    }
}

/// A temporary instance, for the loaded and empty case.
struct Instance {
    _directory: tempfile::TempDir,
    config: Config,
}

impl Instance {
    /// Creates the directories and `crible.toml` of an instance with no index.
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("a temporary instance is creatable");
        let root = directory.path();
        for name in ["data", "models", "exchange"] {
            std::fs::create_dir_all(root.join(name)).expect("an instance directory is creatable");
        }
        let path = root.join("crible.toml");
        std::fs::write(&path, config_toml(root)).expect("crible.toml is writable");
        let config = Config::load_with_env(&path, Vec::new()).unwrap_or_else(|error| {
            panic!("the fixture configuration must load: {}", error.message())
        });
        Self {
            _directory: directory,
            config,
        }
    }
}

/// `crible.toml` of the fixture, with one key per role.
fn config_toml(root: &Path) -> String {
    format!(
        r#"
[server]
bind = "127.0.0.1:0"
search_threads = 0
max_concurrent_searches = 0

[paths]
data = "{data}"
models = "{models}"
exchange = "{exchange}"

[[auth.keys]]
name = "administrator"
role = "admin"
sha256 = "{admin}"

[[auth.keys]]
name = "writer"
role = "write"
sha256 = "{writer}"

[[auth.keys]]
name = "reader"
role = "read"
sha256 = "{reader}"

[analytics]
retention_days = 90

[debug_journal]
enabled = false
sample_rate = 0.0
ttl_days = 7
max_rows = 500000
max_bytes = 2147483648

[encoder]
query_precision = "fp32"

[limits]
writer_queue = 16
default_timeout_ms = 2000
wait_timeout_ms = 60000
query_cache_entries = 1024
"#,
        data = root.join("data").display(),
        models = root.join("models").display(),
        exchange = root.join("exchange").display(),
        admin = sha256_hex(ADMIN_SECRET),
        writer = sha256_hex(WRITE_SECRET),
        reader = sha256_hex(READ_SECRET),
    )
}

#[tokio::test]
async fn a_loaded_instance_without_index_answers_200_with_empty_lists() {
    // ADR-0146, decision 6, which bounds the fix: an instance that is loaded and
    // serves no index is ready (ADR-0099), and an empty list is then true.
    let instance = Instance::new();
    let state = Arc::new(AppState::new());
    state
        .load(&instance.config)
        .unwrap_or_else(|error| panic!("an empty instance must load: {}", error.message()));
    state.mark_ready();
    let auth = Auth::new(&instance.config.auth.keys)
        .unwrap_or_else(|error| panic!("the keys are valid: {}", error.message()));
    state
        .set_auth(Arc::new(auth))
        .unwrap_or_else(|error| panic!("the authenticator is published: {}", error.message()));
    let router = build_router(Arc::clone(&state));

    let indexes = send(&router, Method::GET, "/v1/indexes", Some(READ_SECRET), None).await;
    assert_eq!(indexes.status, StatusCode::OK, "{}", indexes.body);
    assert_eq!(indexes.body, json!({ "indexes": [] }));

    let jobs = send(&router, Method::GET, "/v1/jobs", Some(WRITE_SECRET), None).await;
    assert_eq!(jobs.status, StatusCode::OK, "{}", jobs.body);
    assert_eq!(jobs.body, json!({ "jobs": [] }));

    let detail = send(
        &router,
        Method::GET,
        &format!("/v1/indexes/{INDEX}"),
        Some(READ_SECRET),
        None,
    )
    .await;
    assert_eq!(
        detail.status,
        StatusCode::NOT_FOUND,
        "an unknown index after the publication stays a `404`: {}",
        detail.body
    );
    assert_eq!(detail.body["error"]["code"], "unknown_index");
}
