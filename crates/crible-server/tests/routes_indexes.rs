// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and several fixtures and checks of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! The five administration routes of `/v1/indexes` (ADR-0105), seen from the
//! production router: creation at runtime, the two reads, the patch and the
//! deletion, plus the roles of ADR-0095 and the startup purge of `deleted-*`.
//!
//! What each test pins:
//!
//! | Test | Criterion of the fiche |
//! |---|---|
//! | `an_index_created_by_the_route_is_served_without_a_restart` | creation, `201` with the detail body, the index is served |
//! | `a_creation_with_a_taken_name_is_refused_with_400` | name already taken, in the registry then on disk |
//! | `a_creation_with_an_absent_model_leaves_no_directory` | the model is read before any write |
//! | `the_list_is_sorted_by_name_and_the_detail_exposes_no_internal_id` | envelope, order, `segments` is a count (INV-01) |
//! | `a_lexical_language_patch_queues_a_rebuild_lexical_job` | the `kind` of the `PATCH` |
//! | `a_search_patch_is_applied_at_the_end_of_its_job` | `search.*` takes effect after the job |
//! | `two_patches_queued_before_the_first_job_ran_compose` | the patches run in the order of their arrival |
//! | `a_failed_quarantine_leaves_the_index_registered_and_served` | a failed delete leaves the index intact |
//! | `a_delete_without_or_with_a_wrong_confirm_is_refused` | `DELETE` requires `confirm` |
//! | `a_detail_read_between_the_rename_and_the_deregistration_is_a_404` | a read that interleaves a delete answers `404`, never `500` |
//! | `a_delete_removes_the_index_and_keeps_its_job_history` | delete protocol, `404`, folder gone, history kept |
//! | `the_startup_purges_the_quarantined_directories` | `deleted-*` purged at startup |
//! | `the_write_key_is_forbidden_on_the_administration_routes` | `admin` only for `POST`, `PATCH`, `DELETE` |
//! | `the_read_key_reads_the_two_get_routes` | `read` for the two `GET` |
//!
//! ```text
//! cargo test -p crible-server --test routes_indexes
//! ```

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc;

use axum::Router;
use axum::body::Body;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method, StatusCode};
use crible_server::config::Config;
use crible_server::http::build_router;
use crible_server::jobs::JobOutcome;
use crible_server::state::AppState;
use crible_server::{Auth, Shutdown};
use crible_store::jobs::JobKind;
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

/// Secret of the `admin` key.
const ADMIN_SECRET: &str = "admin-secret-of-the-routes-indexes-tests";

/// Secret of the `write` key.
const WRITE_SECRET: &str = "write-secret-of-the-routes-indexes-tests";

/// Secret of the `read` key.
const READ_SECRET: &str = "read-secret-of-the-routes-indexes-tests";

/// Identifier pinned on every request of this file (ADR-0098).
const REQUEST_ID: &str = "routes-indexes-test-1";

/// Name of the fixture index.
const INDEX: &str = "alpha";

/// A temporary instance: its directory (kept alive for the test), its
/// configuration.
struct Instance {
    _directory: TempDir,
    config: Config,
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
        }
    }

    /// Root of `<data>`.
    fn data(&self) -> &Path {
        &self.config.paths.data
    }

    /// Loads the state of the instance, publishes its authenticator and its
    /// trigger, then starts the writers.
    fn state(&self) -> Arc<AppState> {
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
            .set_shutdown(shutdown)
            .unwrap_or_else(|error| panic!("the trigger is published: {}", error.message()));
        state
            .start_writers()
            .unwrap_or_else(|error| panic!("the writers must start: {}", error.message()));
        state
    }

    /// Production router of the instance.
    fn router(&self, state: &Arc<AppState>) -> Router {
        build_router(Arc::clone(state))
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
        writer = crible_server::auth::sha256_hex(WRITE_SECRET),
        reader = crible_server::auth::sha256_hex(READ_SECRET),
    )
}

/// Writes the `external` model directory `m` under `<root>/models`.
fn write_external_model(root: &Path) {
    let directory = root.join("models").join("m");
    std::fs::create_dir_all(&directory).expect("the model directory is creatable");
    std::fs::write(
        directory.join("model.json"),
        r#"{"name":"m","kind":"external","dims":4,"normalize":true}"#,
    )
    .expect("the model configuration is written");
}

/// Configuration of the fixture index (`docs/04`, `index.json`).
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

/// Sends one request through the router and returns the decoded body.
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

/// Creates `name` through the route and checks the `201`.
async fn create(router: &Router, name: &str) -> Value {
    let (status, _headers, body) = send(
        router,
        Method::POST,
        "/v1/indexes",
        ADMIN_SECRET,
        Some(index_config(name)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "the index is created: {body}");
    body
}

/// Creates the fixture index `alpha` and returns the instance and its router.
async fn instance_with_alpha() -> (Instance, Router) {
    let instance = Instance::new();
    let state = instance.state();
    let router = instance.router(&state);
    create(&router, INDEX).await;
    (instance, router)
}

#[tokio::test]
async fn an_index_created_by_the_route_is_served_without_a_restart() {
    // ADR-0105, decisions 1 and 2: the index is created at runtime, served by
    // the same process, and the `201` carries the body of the detail route.
    let instance = Instance::new();
    let state = instance.state();
    let router = instance.router(&state);

    assert!(
        send(&router, Method::GET, "/v1/indexes", ADMIN_SECRET, None)
            .await
            .2["indexes"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "the instance starts with no index"
    );

    let body = create(&router, INDEX).await;
    assert_eq!(body["name"], INDEX);
    assert_eq!(body["config"]["name"], INDEX);
    assert_eq!(body["state"]["documents"], 0);
    assert_eq!(body["state"]["generation"], 0);
    assert_eq!(body["state"]["segments"], 0);
    assert_eq!(
        body["state"]["last_batch"],
        Value::Null,
        "a fresh index attached no batch"
    );
    assert!(
        instance.data().join("indexes").join(INDEX).is_dir(),
        "the index is written on disk"
    );

    let (status, _headers, list) =
        send(&router, Method::GET, "/v1/indexes", READ_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "the list serves the new index");
    assert_eq!(list["indexes"][0]["name"], INDEX);
    let (status, _headers, detail) =
        send(&router, Method::GET, "/v1/indexes/alpha", READ_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "the detail serves the new index");
    assert_eq!(detail["name"], INDEX);
}

#[tokio::test]
async fn a_creation_with_a_taken_name_is_refused_with_400() {
    // ADR-0105, decision 2: a name taken, in the registry or on disk, is
    // `400 invalid_request` with the message of the CLI.
    let (_instance, router) = instance_with_alpha().await;

    let (status, _headers, answer) = send(
        &router,
        Method::POST,
        "/v1/indexes",
        ADMIN_SECRET,
        Some(index_config(INDEX)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a taken name: {answer}");
    assert_eq!(answer["error"]["code"], "invalid_request");
    let message = answer["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the refusal carries a message: {answer}"));
    assert!(
        message.contains("index already exists"),
        "the message of the CLI: {message}"
    );

    // A second name is accepted: the refusal above is specific to the name.
    let (status, _headers, answer) = send(
        &router,
        Method::POST,
        "/v1/indexes",
        ADMIN_SECRET,
        Some(index_config("gamma")),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "another name is accepted: {answer}"
    );
}

#[tokio::test]
async fn a_creation_with_an_absent_model_leaves_no_directory() {
    // ADR-0105, decision 2: the model is read before any write, and the message
    // names it. A failure after the beginning of the write quarantines the
    // directory; here there is nothing to quarantine, and nothing is created.
    let instance = Instance::new();
    let state = instance.state();
    let router = instance.router(&state);
    let mut config = index_config("ghost");
    config["model"] = json!("absent");

    let (status, _headers, body) = send(
        &router,
        Method::POST,
        "/v1/indexes",
        ADMIN_SECRET,
        Some(config),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "invalid_request");
    let message = body["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the refusal carries a message: {body}"));
    assert!(
        message.contains("absent"),
        "the message names the model: {message}"
    );
    assert!(
        !instance.data().join("indexes").join("ghost").exists(),
        "a refused creation leaves no directory"
    );
}

#[tokio::test]
async fn the_list_is_sorted_by_name_and_the_detail_exposes_no_internal_id() {
    // ADR-0105, decision 3: the list is sorted by name, the envelope is the one
    // of ADR-0103, and the detail exposes a count of segments, never their iid
    // bounds (INV-01).
    let instance = Instance::new();
    let state = instance.state();
    let router = instance.router(&state);
    create(&router, "beta").await;
    create(&router, "alpha").await;

    let (status, _headers, list) =
        send(&router, Method::GET, "/v1/indexes", READ_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let names: Vec<&str> = list["indexes"]
        .as_array()
        .unwrap_or_else(|| panic!("the list is an array: {list}"))
        .iter()
        .filter_map(|entry| entry["name"].as_str())
        .collect();
    assert_eq!(names, ["alpha", "beta"], "the list is sorted by name");
    assert_eq!(list["indexes"][0]["model"], "m");
    assert_eq!(list["indexes"][0]["generation"], 0);
    assert_eq!(list["indexes"][0]["documents"], 0);
    assert_eq!(list["indexes"][0]["live_passages"], 0);
    assert_eq!(
        list["indexes"][0]
            .as_object()
            .map(|entry| entry.len())
            .unwrap_or_default(),
        5,
        "the entry of the list is exactly the five fields of ADR-0105: {list}"
    );

    let (status, _headers, detail) =
        send(&router, Method::GET, "/v1/indexes/alpha", READ_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert!(
        detail["state"]["segments"].is_number(),
        "segments is a count: {detail}"
    );
    assert!(
        !detail.to_string().contains("iid"),
        "no internal id leaves the engine (INV-01): {detail}"
    );
    let state = detail["state"]
        .as_object()
        .unwrap_or_else(|| panic!("state is an object: {detail}"));
    let sizes = detail["state"]["sizes"]
        .as_object()
        .unwrap_or_else(|| panic!("sizes is an object: {detail}"));
    assert!(sizes["disk_bytes"].as_u64().is_some_and(|bytes| bytes > 0));
    assert!(state["dead_ratio"].is_number());
}

#[tokio::test]
async fn a_lexical_language_patch_queues_a_rebuild_lexical_job() {
    // ADR-0105, decision 4: enabling the branch and changing its chain queues
    // `rebuild_lexical`, not `patch_config`.
    let (_instance, router) = instance_with_alpha().await;
    let (status, _headers, body) = send(
        &router,
        Method::PATCH,
        "/v1/indexes/alpha",
        ADMIN_SECRET,
        Some(json!({ "lexical": { "enabled": true, "language": "en" } })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["kind"], "rebuild_lexical", "{body}");
    assert_eq!(body["index"], INDEX);
    assert_eq!(body["state"], "queued");
}

#[tokio::test]
async fn a_search_patch_is_applied_at_the_end_of_its_job() {
    // ADR-0105, decision 4: `search.*` is applied at the end of the job, and the
    // configuration read afterwards is the new one.
    let (_instance, router) = instance_with_alpha().await;
    let (status, _headers, body) = send(
        &router,
        Method::PATCH,
        "/v1/indexes/alpha?wait=true",
        ADMIN_SECRET,
        Some(json!({ "search": { "rescore_candidates": 20 } })),
    )
    .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::ACCEPTED,
        "{body}"
    );
    assert_eq!(body["kind"], "patch_config", "{body}");
    assert_eq!(
        body["result"],
        Value::Null,
        "the result of the patch job is null"
    );
    let (status, _headers, detail) =
        send(&router, Method::GET, "/v1/indexes/alpha", READ_SECRET, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        detail["config"]["search"]["rescore_candidates"], 20,
        "the patch is in index.json and in the served configuration: {detail}"
    );
    let raw = std::fs::read_to_string(_instance.data().join("indexes/alpha/index.json"))
        .expect("index.json is readable");
    assert!(
        raw.contains("\"rescore_candidates\":20"),
        "the job wrote index.json atomically: {raw}"
    );
}

#[tokio::test]
async fn a_delete_without_or_with_a_wrong_confirm_is_refused() {
    // ADR-0105, decision 5: `confirm` is required and must repeat the name; the
    // message names the parameter.
    let (_instance, router) = instance_with_alpha().await;
    for path in ["/v1/indexes/alpha", "/v1/indexes/alpha?confirm=beta"] {
        let (status, _headers, body) =
            send(&router, Method::DELETE, path, ADMIN_SECRET, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "on {path}: {body}");
        assert_eq!(body["error"]["code"], "invalid_request");
        let message = body["error"]["message"]
            .as_str()
            .unwrap_or_else(|| panic!("the refusal carries a message: {body}"));
        assert!(
            message.contains("confirm"),
            "the message names 'confirm': {message}"
        );
    }
    let (status, _headers, _body) =
        send(&router, Method::GET, "/v1/indexes/alpha", READ_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "a refused delete removes nothing");
}

#[tokio::test]
async fn a_detail_read_between_the_rename_and_the_deregistration_is_a_404() {
    // ADR-0105, decision 6: `delete_index` renames the index directory first,
    // then removes the index from the registry. A `GET /v1/indexes/{index}` that
    // interleaves finds the index still registered and its directory gone: it
    // answers `404 unknown_index`, never `500 internal` (`@relecteur`,
    // ROUTES-INDEX). The intermediate state is reproduced exactly, so the test
    // never races the job.
    let (instance, router) = instance_with_alpha().await;
    let tmp = instance.data().join("tmp");
    std::fs::create_dir_all(&tmp).expect("the quarantine directory is creatable");
    std::fs::rename(
        instance.data().join("indexes").join(INDEX),
        tmp.join("deleted-alpha-job_test"),
    )
    .expect("the index directory moves into the quarantine");

    let (status, _headers, body) =
        send(&router, Method::GET, "/v1/indexes/alpha", READ_SECRET, None).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a renamed directory is an unknown index, not an internal failure: {body}"
    );
    assert_eq!(body["error"]["code"], "unknown_index", "{body}");
}

#[tokio::test]
async fn a_delete_removes_the_index_and_keeps_its_job_history() {
    // ADR-0105, decision 6: the registry loses the index, the directory is
    // quarantined then removed, and the job history stays.
    let (instance, router) = instance_with_alpha().await;
    let (status, _headers, body) = send(
        &router,
        Method::DELETE,
        "/v1/indexes/alpha?confirm=alpha&wait=true",
        ADMIN_SECRET,
        None,
    )
    .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::ACCEPTED,
        "{body}"
    );
    assert_eq!(body["kind"], "delete_index", "{body}");

    let (status, _headers, answer) =
        send(&router, Method::GET, "/v1/indexes/alpha", READ_SECRET, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{answer}");
    assert_eq!(answer["error"]["code"], "unknown_index");
    assert!(
        !instance.data().join("indexes").join(INDEX).exists(),
        "the index directory is removed"
    );
    let tmp = instance.data().join("tmp");
    let leftovers: Vec<String> = std::fs::read_dir(&tmp)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter_map(|entry| entry.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    assert!(
        leftovers.iter().all(|name| !name.starts_with("deleted-")),
        "the quarantine is removed: {leftovers:?}"
    );

    let (status, _headers, jobs) = send(
        &router,
        Method::GET,
        "/v1/jobs?index=alpha",
        ADMIN_SECRET,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{jobs}");
    assert!(
        jobs["jobs"]
            .as_array()
            .is_some_and(|jobs| jobs.iter().any(|job| job["kind"] == "delete_index")),
        "the history of the deleted index stays in jobs.sqlite: {jobs}"
    );

    // ADR-0105, decision 6: the same name is accepted once the directory is
    // renamed, and refused while the index is still in the registry.
    let body = create(&router, INDEX).await;
    assert_eq!(body["name"], INDEX);
}

#[tokio::test]
async fn the_startup_purges_the_quarantined_directories() {
    // ADR-0105, decision 6, and ADR-0056: a crash after the rename leaves a
    // `deleted-*` directory under `<data>/tmp`, purged by the next startup.
    let instance = Instance::new();
    let leftover = instance.data().join("tmp").join("deleted-ghost-job_abc");
    std::fs::create_dir_all(&leftover).expect("the quarantine directory is creatable");
    std::fs::write(leftover.join("CURRENT_EPOCH"), "1\n").expect("the file is writable");

    let _state = instance.state();
    assert!(
        !leftover.exists(),
        "the startup purges the quarantined directories of <data>/tmp"
    );
}

#[tokio::test]
async fn the_write_key_is_forbidden_on_the_administration_routes() {
    // ADR-0105, decision 7, and ADR-0095: the three mutations are `admin`; a
    // `write` key is authenticated and refused with `403`, without
    // `WWW-Authenticate`.
    let (_instance, router) = instance_with_alpha().await;
    let requests = [
        (
            Method::POST,
            "/v1/indexes".to_owned(),
            Some(index_config("beta")),
        ),
        (
            Method::PATCH,
            "/v1/indexes/alpha".to_owned(),
            Some(json!({ "search": { "rescore_candidates": 20 } })),
        ),
        (
            Method::DELETE,
            "/v1/indexes/alpha?confirm=alpha".to_owned(),
            None,
        ),
    ];
    for (method, path, body) in requests {
        let (status, headers, answer) =
            send(&router, method.clone(), &path, WRITE_SECRET, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {answer}");
        assert_eq!(answer["error"]["code"], "forbidden");
        assert!(
            !headers.contains_key("www-authenticate"),
            "a 403 carries no WWW-Authenticate (ADR-0100)"
        );
    }
    let (status, _headers, _body) =
        send(&router, Method::GET, "/v1/indexes/alpha", READ_SECRET, None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the refusals did not touch the index"
    );
}

#[tokio::test]
async fn the_read_key_reads_the_two_get_routes() {
    // ADR-0105, decision 7: both reads are open to `read`.
    let (_instance, router) = instance_with_alpha().await;
    let (status, _headers, list) =
        send(&router, Method::GET, "/v1/indexes", READ_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["indexes"][0]["name"], INDEX);
    let (status, _headers, detail) =
        send(&router, Method::GET, "/v1/indexes/alpha", READ_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["config"]["model"], "m");
}

#[tokio::test]
async fn two_patches_queued_before_the_first_job_ran_compose() {
    // ADR-0105, decision 4: the jobs run in the order of their arrival, and each
    // patch applies to the configuration published when it starts. Two patches
    // accepted before the first job ran must both take effect; applying the
    // second to the state the route validated against would erase the first.
    let instance = Instance::new();
    let state = instance.state();
    let router = instance.router(&state);
    create(&router, INDEX).await;

    // Hold the writer so both patches are queued before either runs, without
    // relying on a duration.
    let (release, blocked) = mpsc::channel::<()>();
    state
        .submit_job(
            INDEX,
            JobKind::PatchConfig,
            Box::new(move |_store, _progress, _writer| {
                let _ = blocked.recv();
                Ok(JobOutcome::default())
            }),
        )
        .await
        .expect("the blocker job is queued");

    let (status, _headers, first) = send(
        &router,
        Method::PATCH,
        "/v1/indexes/alpha",
        ADMIN_SECRET,
        Some(json!({ "search": { "rescore_candidates": 20 } })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "the first patch is queued: {first}"
    );
    let (status, _headers, second) = send(
        &router,
        Method::PATCH,
        "/v1/indexes/alpha",
        ADMIN_SECRET,
        Some(json!({ "search": { "candidates_per_branch": 40 } })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "the second patch is queued: {second}"
    );

    release.send(()).expect("the blocker is released");
    let id = second["id"].as_str().expect("the job carries its id");
    let outcome = state
        .wait_for_job(id)
        .await
        .expect("the second patch reaches a terminal state");
    assert_eq!(
        outcome.state.as_str(),
        "succeeded",
        "the second patch succeeds"
    );

    let (status, _headers, detail) =
        send(&router, Method::GET, "/v1/indexes/alpha", READ_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(
        detail["config"]["search"]["rescore_candidates"], 20,
        "the first patch is not lost: {detail}"
    );
    assert_eq!(
        detail["config"]["search"]["candidates_per_branch"], 40,
        "the second patch is applied: {detail}"
    );
}

#[tokio::test]
async fn a_failed_quarantine_leaves_the_index_registered_and_served() {
    // ADR-0105, decision 6: the rename comes before the deregistration, so a
    // rename that fails fails the job and leaves the index intact, registered
    // and served. Here `<data>/tmp` is a file, so the quarantine cannot be
    // created.
    let (instance, router) = instance_with_alpha().await;
    let tmp = instance.data().join("tmp");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::write(&tmp, b"not a directory").expect("the obstruction is written");

    let (status, _headers, body) = send(
        &router,
        Method::DELETE,
        "/v1/indexes/alpha?confirm=alpha&wait=true",
        ADMIN_SECRET,
        None,
    )
    .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::ACCEPTED,
        "{body}"
    );
    assert_eq!(body["state"], "failed", "the delete job fails: {body}");

    let (status, _headers, detail) =
        send(&router, Method::GET, "/v1/indexes/alpha", READ_SECRET, None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the index stays served after the failure: {detail}"
    );
    assert!(
        instance.data().join("indexes").join(INDEX).is_dir(),
        "the directory stays in place"
    );
    let (status, _headers, answer) = send(
        &router,
        Method::POST,
        "/v1/indexes",
        ADMIN_SECRET,
        Some(index_config(INDEX)),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "the name stays taken while the index is served: {answer}"
    );
}
