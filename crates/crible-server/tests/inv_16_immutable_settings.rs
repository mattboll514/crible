// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and several fixtures and checks of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! INV-16 seen from the HTTP API: a frozen setting never changes through a
//! `PATCH`, and the refusal happens before any job is queued.
//!
//! Acceptance criterion of `ROUTES-INDEX` (`docs/phases/P6-serveur-et-api.md`):
//! "un `PATCH` sur un réglage figé renvoie `409` en nommant le champ".
//!
//! The frozen settings are the first column of `docs/04-donnees-et-ingestion.md`
//! (`:44-55`): `name`, `model`, `reference_precision`, and the type and the
//! level of an existing field. ADR-0105, decision 4, fixes the order: the patch
//! is validated **before any queueing**, so a refusal leaves no job behind and
//! `index.json` untouched.
//!
//! The routes of `/v1/indexes` exist since `ROUTES-INDEX`. Every test of this
//! file goes through the production router and the documented HTTP contract,
//! never through a private function, so it pins what a client observes.

use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method, StatusCode};
use crible_server::config::Config;
use crible_server::http::build_router;
use crible_server::state::AppState;
use crible_server::{Auth, Shutdown};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

/// Secret of the `admin` key used by the tests.
const ADMIN_SECRET: &str = "admin-secret-of-the-inv-16-tests";

/// Identifier pinned on every request of this file (ADR-0098).
const REQUEST_ID: &str = "inv-16-test-1";

/// Name of the index every test creates.
const INDEX: &str = "alpha";

/// A temporary instance: its directory (kept alive for the test), its
/// configuration, and the secret of its `admin` key.
struct Instance {
    _directory: TempDir,
    config: Config,
    admin_secret: String,
}

impl Instance {
    /// Creates the instance directories, writes the `external` model `m` and
    /// `crible.toml`, then loads the configuration with an empty environment.
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("a temporary instance is creatable");
        let root = directory.path();
        for name in ["data", "models", "exchange"] {
            std::fs::create_dir_all(root.join(name)).expect("an instance directory is creatable");
        }
        write_external_model(root);
        let path = root.join("crible.toml");
        std::fs::write(&path, config_toml(root)).expect("crible.toml is writable");
        let config = Config::load_with_env(&path, Vec::new()).unwrap_or_else(|error| {
            panic!("the fixture configuration must load: {}", error.message())
        });
        Self {
            _directory: directory,
            config,
            admin_secret: ADMIN_SECRET.to_owned(),
        }
    }

    /// Loads the state of the instance and publishes its trigger.
    fn state(&self) -> (Arc<AppState>, Shutdown) {
        let state = Arc::new(AppState::new());
        state
            .load(&self.config)
            .unwrap_or_else(|error| panic!("the instance must load: {}", error.message()));
        let auth = Arc::new(
            Auth::new(&self.config.auth.keys)
                .unwrap_or_else(|error| panic!("the keys must be valid: {}", error.message())),
        );
        state
            .set_auth(auth)
            .unwrap_or_else(|error| panic!("the authenticator is published: {}", error.message()));
        let shutdown = Shutdown::new();
        state
            .set_shutdown(shutdown.clone())
            .unwrap_or_else(|error| panic!("the trigger is published: {}", error.message()));
        state
            .start_writers()
            .unwrap_or_else(|error| panic!("the writers must start: {}", error.message()));
        (state, shutdown)
    }

    /// Production router of the instance.
    fn router(&self, state: &Arc<AppState>) -> Router {
        build_router(Arc::clone(state))
    }
}

/// `crible.toml` of the fixture, with one `admin` key and the documented keys.
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

[analytics]
retention_days = 90

[debug_journal]
enabled = false
sample_rate = 0.0
ttl_days = 7

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
        admin = crible_server::auth::sha256_hex(ADMIN_SECRET),
    )
}

/// Writes the `external` model directory `m` under `<root>/models`.
///
/// The model is `external`, so the fixture needs no ONNX Runtime, exactly as
/// the other tests of `crates/crible-server/tests` do.
fn write_external_model(root: &Path) {
    let directory = root.join("models").join("m");
    std::fs::create_dir_all(&directory).expect("the model directory is creatable");
    std::fs::write(
        directory.join("model.json"),
        r#"{"name":"m","kind":"external","dims":4,"normalize":true}"#,
    )
    .expect("the model configuration is written");
}

/// Configuration of the index created by every test (`docs/04`, `index.json`).
///
/// The two levels carry a field, so a patch can try to change a type and a
/// level. The lexical branch is disabled: the fixture never builds tantivy.
fn index_config() -> Value {
    json!({
        "name": INDEX,
        "model": "m",
        "reference_precision": "fp16",
        "schema": {
            "document_fields": {
                "numero": { "type": "keyword" },
                "partie": { "type": "keyword" }
            },
            "passage_fields": {
                "texte": { "type": "keyword" }
            }
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

/// A request with an optional JSON body, through the router.
async fn send(
    router: &Router,
    method: Method,
    path: &str,
    secret: &str,
    body: Option<Value>,
) -> (StatusCode, HeaderMap, Value) {
    let mut request = axum::http::Request::builder()
        .method(method)
        .uri(path)
        .header("x-request-id", REQUEST_ID)
        .header(AUTHORIZATION, format!("Bearer {secret}"));
    let payload = match body {
        Some(value) => {
            request = request.header("content-type", "application/json");
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
    (status, headers, body)
}

/// Creates the fixture index through `POST /v1/indexes` (ADR-0105,
/// decision 2), and returns the router with its body.
async fn created(instance: &Instance) -> (Router, Value) {
    let (state, _shutdown) = instance.state();
    let router = instance.router(&state);
    let (status, _headers, body) = send(
        &router,
        Method::POST,
        "/v1/indexes",
        &instance.admin_secret,
        Some(index_config()),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the fixture index is created by the route: {body}"
    );
    (router, body)
}

/// The materialised configuration of the fixture index, as `GET` renders it.
async fn configuration(router: &Router, secret: &str) -> Value {
    let (status, _headers, body) =
        send(router, Method::GET, "/v1/indexes/alpha", secret, None).await;
    assert_eq!(status, StatusCode::OK, "the index is readable: {body}");
    body["config"].clone()
}

/// The retained jobs, as `GET /v1/jobs` renders them.
async fn jobs(router: &Router, secret: &str) -> Vec<Value> {
    let (status, _headers, body) = send(router, Method::GET, "/v1/jobs", secret, None).await;
    assert_eq!(status, StatusCode::OK, "the job list is readable: {body}");
    body["jobs"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| panic!("the job list is an array: {body}"))
}

/// Sends `patch`, then checks the three consequences of INV-16: `409` with
/// `immutable_setting`, no job queued, and the same configuration.
///
/// `named` are the segments the message must contain: the JSON path of the
/// frozen setting (`docs/05-api.md`, ADR-0105 decision 4).
async fn refused(router: &Router, secret: &str, patch: Value, named: &[&str]) {
    let before_config = configuration(router, secret).await;
    let before_jobs = jobs(router, secret).await.len();

    let (status, _headers, body) = send(
        router,
        Method::PATCH,
        "/v1/indexes/alpha",
        secret,
        Some(patch),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a frozen setting is refused with 409: {body}"
    );
    assert_eq!(
        body["error"]["code"], "immutable_setting",
        "the code of INV-16: {body}"
    );
    assert_eq!(
        body["error"]["request_id"], REQUEST_ID,
        "the refusal carries the request identifier (ADR-0101): {body}"
    );
    let message = body["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the refusal carries a message: {body}"));
    for segment in named {
        assert!(
            message.contains(segment),
            "the message names '{segment}' (JSON path of the frozen setting): {message}"
        );
    }

    assert_eq!(
        jobs(router, secret).await.len(),
        before_jobs,
        "no job is created by a refused patch (ADR-0105, decision 4)"
    );
    assert_eq!(
        configuration(router, secret).await,
        before_config,
        "the configuration is unchanged by a refused patch"
    );
}

#[tokio::test]
async fn inv_16_patch_of_name_is_refused_with_409_naming_the_field() {
    // INV-16: `name` is frozen at creation (`docs/04:44-55`).
    let instance = Instance::new();
    let (router, _body) = created(&instance).await;
    refused(
        &router,
        &instance.admin_secret,
        json!({ "name": "beta" }),
        &["name"],
    )
    .await;
}

#[tokio::test]
async fn inv_16_patch_of_model_is_refused_with_409_naming_the_field() {
    // INV-16: `model` is frozen, its fingerprint is copied in `index.json`.
    let instance = Instance::new();
    let (router, _body) = created(&instance).await;
    refused(
        &router,
        &instance.admin_secret,
        json!({ "model": "other" }),
        &["model"],
    )
    .await;
}

#[tokio::test]
async fn inv_16_patch_of_reference_precision_is_refused_with_409_naming_the_field() {
    // INV-16: `reference_precision` is frozen at creation.
    let instance = Instance::new();
    let (router, _body) = created(&instance).await;
    refused(
        &router,
        &instance.admin_secret,
        json!({ "reference_precision": "fp32" }),
        &["reference_precision"],
    )
    .await;
}

#[tokio::test]
async fn inv_16_patch_of_a_field_type_is_refused_with_409_naming_the_field() {
    // INV-16: the type of an existing field is frozen. The message names the
    // JSON path of the setting, not only the field.
    let instance = Instance::new();
    let (router, _body) = created(&instance).await;
    refused(
        &router,
        &instance.admin_secret,
        json!({
            "schema": {
                "document_fields": { "partie": { "type": "integer" } }
            }
        }),
        &["schema", "document_fields", "partie", "type"],
    )
    .await;
}

#[tokio::test]
async fn inv_16_patch_of_a_field_level_is_refused_with_409_naming_the_field() {
    // INV-16: the level of an existing field is frozen. Declaring the document
    // field `numero` at the passage level is the only expressible attempt: the
    // refusal must be `409 immutable_setting`, not the `400` of a same-name
    // collision across levels (ADR-0030), because the field is known and its
    // level is frozen.
    let instance = Instance::new();
    let (router, _body) = created(&instance).await;
    let before_config = configuration(&router, &instance.admin_secret).await;
    let before_jobs = jobs(&router, &instance.admin_secret).await.len();

    let (status, _headers, body) = send(
        &router,
        Method::PATCH,
        "/v1/indexes/alpha",
        &instance.admin_secret,
        Some(json!({
            "schema": {
                "passage_fields": { "numero": { "type": "keyword" } }
            }
        })),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "the level of an existing field is frozen: {body}"
    );
    assert_eq!(body["error"]["code"], "immutable_setting", "{body}");
    let message = body["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the refusal carries a message: {body}"));
    for segment in ["schema", "numero"] {
        assert!(
            message.contains(segment),
            "the message names '{segment}' (JSON path of the frozen setting): {message}"
        );
    }
    assert!(
        message.contains("document_fields") || message.contains("passage_fields"),
        "the message names the level of the field: {message}"
    );
    assert_eq!(
        jobs(&router, &instance.admin_secret).await.len(),
        before_jobs,
        "no job is created by a refused patch"
    );
    assert_eq!(
        configuration(&router, &instance.admin_secret).await,
        before_config,
        "the configuration is unchanged by a refused patch"
    );
}

#[tokio::test]
async fn inv_16_patch_mixing_a_frozen_and_a_mutable_setting_applies_nothing() {
    // INV-16 and ADR-0105, decision 4: the patch is validated as a whole,
    // before any queueing. A frozen setting anywhere in the body refuses it
    // entirely: the mutable setting is not queued and not applied.
    let instance = Instance::new();
    let (router, _body) = created(&instance).await;
    refused(
        &router,
        &instance.admin_secret,
        json!({
            "name": "beta",
            "search": { "rescore_candidates": 20 }
        }),
        &["name"],
    )
    .await;
    let config = configuration(&router, &instance.admin_secret).await;
    assert_eq!(
        config["search"]["rescore_candidates"], 10,
        "the mutable setting of a refused patch is not applied: {config}"
    );
}

#[tokio::test]
async fn inv_16_a_mutable_setting_alone_is_accepted_and_queues_a_job() {
    // Control test: the refusal of INV-16 is specific to frozen settings. The
    // same body without the frozen key is accepted, and its job is queued, so a
    // `409` above does not come from a route that refuses every patch.
    let instance = Instance::new();
    let (router, _body) = created(&instance).await;
    let before_jobs = jobs(&router, &instance.admin_secret).await.len();

    let (status, _headers, body) = send(
        &router,
        Method::PATCH,
        "/v1/indexes/alpha?wait=true",
        &instance.admin_secret,
        Some(json!({ "search": { "rescore_candidates": 20 } })),
    )
    .await;

    assert!(
        status == StatusCode::OK || status == StatusCode::ACCEPTED,
        "a mutable setting is accepted: {body}"
    );
    assert_eq!(
        body["kind"], "patch_config",
        "the job kind of ADR-0105, decision 4: {body}"
    );
    assert_eq!(
        jobs(&router, &instance.admin_secret).await.len(),
        before_jobs + 1,
        "the accepted patch queues exactly one job"
    );
    let config = configuration(&router, &instance.admin_secret).await;
    assert_eq!(
        config["search"]["rescore_candidates"], 20,
        "an accepted patch is applied at the end of its job: {config}"
    );
}
