// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and several fixtures and checks of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! Write jobs of an index, seen from outside the server.
//!
//! Acceptance criteria of `FILE-D-ECRITURE-ET-JOBS`
//! (`docs/phases/P6-serveur-et-api.md`), under ADR-0103, ADR-0104 and ADR-0067:
//!
//! - the jobs of one index run in arrival order;
//! - a finished job is readable after a restart, and a job a restart
//!   interrupted is `failed`;
//! - the shutdown drains the accepted jobs, refuses a later submission with
//!   `503`, and the bound of the drainage covers the jobs;
//! - a `read` key is `403` on `GET /v1/jobs` and `GET /v1/jobs/{job_id}`;
//! - `?wait=true` renders `202` at the deadline and `200` when the job finished;
//! - a full queue renders `503`.
//!
//! The write routes do not exist yet (`ROUTES-INDEX`,
//! `ROUTES-DOCUMENTS-ET-LOTS`): the submissions go through
//! [`AppState::submit_job`], and the `503` of a full queue is proved on a test
//! router that calls it, as ADR-0095 allows. Everything runs without ONNX
//! Runtime: the instance serves an `external` index.

use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::header::{AUTHORIZATION, RETRY_AFTER};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use crible_core::config::IndexConfig;
use crible_core::error::ErrorCode;
use crible_server::config::{Config, Role};
use crible_server::http::build_router;
use crible_server::jobs::{JobOutcome, SHUTTING_DOWN_MESSAGE};
use crible_server::limits::refusal;
use crible_server::logging::log_subscriber;
use crible_server::state::AppState;
use crible_server::{Auth, Shutdown};
use crible_store::generation::{GenerationStore, INITIAL_EPOCH, write_index_config};
use crible_store::jobs::{INTERRUPTED_MESSAGE, JOBS_FILE_NAME, JobKind, JobState, JobStore};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt;

/// Secret of the `write` key used by the tests.
const WRITE_SECRET: &str = "write-secret-of-the-jobs-tests";

/// Secret of the `read` key used by the tests.
const READ_SECRET: &str = "read-secret-of-the-jobs-tests";

/// Identifier pinned on the responses of the test router (ADR-0098).
const REQUEST_ID: &str = "jobs-test-1";

/// Identifier of the job the restart test writes before the startup.
const INTERRUPTED_ID: &str = "job_0000000000000000000000000000000a";

/// A temporary instance: its directory (kept alive for the test), its
/// configuration, and the secrets its keys hash.
struct Instance {
    _directory: TempDir,
    config: Config,
    write_secret: String,
    read_secret: String,
}

impl Instance {
    /// Creates the instance directories, writes `crible.toml` with the two
    /// queues of the job tests, and loads it with an empty environment.
    fn new(writer_queue: usize, wait_timeout_ms: u64) -> Self {
        let directory = tempfile::tempdir().expect("a temporary instance is creatable");
        let root = directory.path();
        for name in ["data", "models", "exchange"] {
            std::fs::create_dir_all(root.join(name)).expect("an instance directory is creatable");
        }
        write_external_model(root);
        create_index(root);
        let path = root.join("crible.toml");
        std::fs::write(&path, config_toml(root, writer_queue, wait_timeout_ms))
            .expect("crible.toml is writable");
        let config = Config::load_with_env(&path, Vec::new()).unwrap_or_else(|error| {
            panic!("the fixture configuration must load: {}", error.message())
        });
        Self {
            _directory: directory,
            config,
            write_secret: WRITE_SECRET.to_owned(),
            read_secret: READ_SECRET.to_owned(),
        }
    }

    /// Root directory of the instance.
    fn root(&self) -> &Path {
        self._directory.path()
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

/// Configuration of an index that needs no encoder and no lexical branch.
fn index_json(name: &str) -> String {
    format!(
        r#"{{"name":"{name}","model":"m","reference_precision":"fp16",
            "schema":{{"document_fields":{{}},"passage_fields":{{}}}},"cluster_by":[],
            "lexical":{{"enabled":false,"language":"fr","positions":false}},
            "search":{{"scan":"none","rescore_candidates":10,"exact_threshold":10,
                      "candidates_per_branch":10,"fusion":{{"method":"rrf"}}}},
            "ingest":{{"acceptance_sample":1,"acceptance_min_cosine":0.9}},
            "compaction":{{"dead_ratio":0.1}}}}"#
    )
}

/// The `crible.toml` of the fixture, with the two keys of the tests.
fn config_toml(root: &Path, writer_queue: usize, wait_timeout_ms: u64) -> String {
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
name = "writer"
role = "write"
sha256 = "{write}"

[[auth.keys]]
name = "reader"
role = "read"
sha256 = "{read}"

[analytics]
retention_days = 90

[debug_journal]
enabled = false
sample_rate = 0.0
ttl_days = 7

[encoder]
query_precision = "fp32"

[limits]
writer_queue = {writer_queue}
default_timeout_ms = 2000
wait_timeout_ms = {wait_timeout_ms}
query_cache_entries = 1024
"#,
        data = root.join("data").display(),
        models = root.join("models").display(),
        exchange = root.join("exchange").display(),
        write = crible_server::auth::sha256_hex(WRITE_SECRET),
        read = crible_server::auth::sha256_hex(READ_SECRET),
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

/// Creates the index `alpha` of `root`.
fn create_index(root: &Path) {
    let raw = index_json("alpha");
    let index = IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("the fixture index must be valid: {}", error.message()));
    let directory = root.join("data").join("indexes").join("alpha");
    write_index_config(&directory, &raw)
        .unwrap_or_else(|error| panic!("the index configuration is written: {}", error.message()));
    GenerationStore::create(&directory, INITIAL_EPOCH, &index.schema)
        .unwrap_or_else(|error| panic!("the index is created: {}", error.message()));
}

/// Work that appends `marker` to `seen` and does nothing to the store.
fn appending(seen: Arc<Mutex<Vec<String>>>, marker: &'static str) -> crible_server::jobs::JobWork {
    Box::new(move |_store, _progress, _writer| {
        seen.lock()
            .map_err(|_| {
                crible_core::error::CribleError::new(ErrorCode::Internal, "the log is poisoned")
            })?
            .push(marker.to_owned());
        Ok(JobOutcome::default())
    })
}

/// A request with `Connection: close`, through the router.
async fn send(
    router: &Router,
    method: Method,
    path: &str,
    secret: Option<&str>,
) -> (StatusCode, HeaderMap, Value) {
    let mut request = axum::http::Request::builder()
        .method(method)
        .uri(path)
        .header("x-request-id", REQUEST_ID);
    if let Some(secret) = secret {
        request = request.header(AUTHORIZATION, format!("Bearer {secret}"));
    }
    let response = router
        .clone()
        .oneshot(request.body(Body::empty()).expect("the request is valid"))
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

#[tokio::test]
async fn jobs_of_one_index_run_in_arrival_order() {
    // Acceptance criterion: one writer per index, a bounded queue, arrival
    // order (ADR-0103, decision 8).
    let instance = Instance::new(16, 60_000);
    let (state, _shutdown) = instance.state();
    let seen = Arc::new(Mutex::new(Vec::new()));
    for marker in ["first", "second", "third"] {
        state
            .submit_job(
                "alpha",
                JobKind::Compact,
                appending(Arc::clone(&seen), marker),
            )
            .await
            .expect("the job is accepted");
    }
    state.jobs().expect("the board is open").drained().await;
    let order = seen.lock().expect("the log is readable").clone();
    assert_eq!(order, ["first", "second", "third"], "the queue is FIFO");
}

#[tokio::test]
async fn a_finished_job_is_read_after_a_restart() {
    // Acceptance criterion: a finished job remains readable after a restart.
    let instance = Instance::new(16, 60_000);
    let (state, _shutdown) = instance.state();
    let job = state
        .submit_job(
            "alpha",
            JobKind::Compact,
            Box::new(|_store, _progress, _writer| {
                Ok(JobOutcome {
                    generation: Some(7),
                    result: Some(json!({"report": "kept"})),
                })
            }),
        )
        .await
        .expect("the job is accepted");
    state.jobs().expect("the board is open").drained().await;
    let finished = state.job(&job.id).expect("the job is kept");
    assert_eq!(finished.state, JobState::Succeeded);
    drop(state);

    let restarted = Arc::new(AppState::new());
    restarted
        .load(&instance.config)
        .unwrap_or_else(|error| panic!("the instance restarts: {}", error.message()));
    let read = restarted.job(&job.id).expect("the finished job survived");
    assert_eq!(read.state, JobState::Succeeded);
    assert_eq!(read.generation, Some(7));
    assert_eq!(read.result, Some(json!({"report": "kept"})));
}

#[tokio::test]
async fn an_interrupted_job_is_failed_at_the_next_startup() {
    // ADR-0103, decision 5: no job is replayed, and the job is visible as
    // `failed` through the board, not only in the table.
    let instance = Instance::new(16, 60_000);
    let jobs = instance.root().join("data").join(JOBS_FILE_NAME);
    {
        let store = JobStore::open_or_create(&jobs).expect("the job table is creatable");
        store
            .insert(INTERRUPTED_ID, JobKind::Export, "alpha")
            .expect("the job is inserted");
        store
            .mark_running(INTERRUPTED_ID, None)
            .expect("the job starts");
    }
    let (state, _shutdown) = instance.state();
    let job = state.job(INTERRUPTED_ID).expect("the job is kept");
    assert_eq!(job.state, JobState::Failed);
    let error = job.error.expect("the failed job carries an error");
    assert_eq!(error["code"], "internal");
    assert_eq!(error["message"], INTERRUPTED_MESSAGE);
}

#[tokio::test]
async fn a_read_key_is_forbidden_on_both_job_routes() {
    // Acceptance criterion: the two job reads are `write` only (ADR-0095).
    let instance = Instance::new(16, 60_000);
    let (state, _shutdown) = instance.state();
    let router = instance.router(&state);
    for path in ["/v1/jobs", "/v1/jobs/job_0000000000000000000000000000000a"] {
        let (status, _headers, body) =
            send(&router, Method::GET, path, Some(&instance.read_secret)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
        assert_eq!(body["error"]["code"], "forbidden", "{path}");
    }
    let (status, _headers, _body) = send(
        &router,
        Method::GET,
        "/v1/jobs",
        Some(&instance.write_secret),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "a write key reads the list");
}

#[tokio::test]
async fn wait_true_renders_202_at_the_deadline_with_the_job_still_running() {
    // ADR-0067: the deadline is not an error; the answer is the job as it is.
    let instance = Instance::new(16, 50);
    let (state, _shutdown) = instance.state();
    let (release, blocked) = mpsc::channel::<()>();
    let job = state
        .submit_job(
            "alpha",
            JobKind::Compact,
            Box::new(move |_store, _progress, _writer| {
                let _ = blocked.recv_timeout(Duration::from_secs(5));
                Ok(JobOutcome::default())
            }),
        )
        .await
        .expect("the job is accepted");
    let answer = crible_server::accepted(&state, job.clone(), true).await;
    assert_eq!(answer.status(), StatusCode::ACCEPTED);
    let bytes = axum::body::to_bytes(answer.into_body(), 1 << 20)
        .await
        .expect("the body is readable");
    let body: Value = serde_json::from_slice(&bytes).expect("the body is JSON");
    assert_eq!(body["id"], job.id);
    assert!(
        body["state"] == "running" || body["state"] == "queued",
        "the job is not terminal at the deadline: {}",
        body["state"]
    );
    let _ = release.send(());
    state.jobs().expect("the board is open").drained().await;
}

#[tokio::test]
async fn wait_true_renders_200_when_the_job_finished_before_the_deadline() {
    // ADR-0067: a job that finishes in the delay is rendered terminal, in 200.
    let instance = Instance::new(16, 60_000);
    let (state, _shutdown) = instance.state();
    let job = state
        .submit_job(
            "alpha",
            JobKind::Snapshot,
            Box::new(|_store, _progress, _writer| Ok(JobOutcome::default())),
        )
        .await
        .expect("the job is accepted");
    let answer = crible_server::accepted(&state, job, true).await;
    assert_eq!(answer.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(answer.into_body(), 1 << 20)
        .await
        .expect("the body is readable");
    let body: Value = serde_json::from_slice(&bytes).expect("the body is JSON");
    assert_eq!(body["state"], "succeeded");
}

#[tokio::test]
async fn wait_true_renders_202_as_soon_as_the_drainage_begins() {
    // ADR-0104, decision 5: the wait does not run to its deadline; the first
    // phase of the drainage renders the job in `202`, as it is at that instant.
    let instance = Instance::new(16, 60_000);
    let (state, shutdown) = instance.state();
    let (release, blocked) = mpsc::channel::<()>();
    let job = state
        .submit_job(
            "alpha",
            JobKind::Compact,
            Box::new(move |_store, _progress, _writer| {
                let _ = blocked.recv_timeout(Duration::from_secs(5));
                Ok(JobOutcome::default())
            }),
        )
        .await
        .expect("the job is accepted");
    let id = job.id.clone();
    let waiter = Arc::clone(&state);
    let waiting = tokio::spawn(async move { crible_server::accepted(&waiter, job, true).await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    shutdown.drain();
    let answer = tokio::time::timeout(Duration::from_secs(2), waiting)
        .await
        .expect("the wait ends as soon as the drainage begins")
        .expect("the wait task joins");
    assert_eq!(answer.status(), StatusCode::ACCEPTED);
    let bytes = axum::body::to_bytes(answer.into_body(), 1 << 20)
        .await
        .expect("the body is readable");
    let body: Value = serde_json::from_slice(&bytes).expect("the body is JSON");
    assert_eq!(body["id"], id);
    assert!(
        body["state"] == "running" || body["state"] == "queued",
        "the job is not terminal: {}",
        body["state"]
    );
    let _ = release.send(());
    state.jobs().expect("the board is open").drained().await;
}

#[tokio::test]
async fn the_drain_wait_armed_before_the_last_job_is_woken_by_it() {
    // The wait is registered before the last job finishes: `notify_waiters`
    // keeps no permit, so `drained` must arm its future to see the transition
    // (ADR-0104). Without that, the stop would run to its bound and log jobs
    // that were never abandoned.
    //
    // This test keeps the nominal path. The other window, a notification sent
    // between the last count of `pending` and the `await` of the wait, is proved
    // by the unit test
    // `jobs::tests::a_notification_between_the_last_count_and_the_wait_is_not_lost`,
    // the only place that can stop the wait inside that window (`@relecteur`,
    // FILE-D-ECRITURE-ET-JOBS).
    let instance = Instance::new(16, 60_000);
    let (state, _shutdown) = instance.state();
    let (release, blocked) = mpsc::channel::<()>();
    state
        .submit_job(
            "alpha",
            JobKind::Compact,
            Box::new(move |_store, _progress, _writer| {
                let _ = blocked.recv_timeout(Duration::from_secs(5));
                Ok(JobOutcome::default())
            }),
        )
        .await
        .expect("the job is accepted");
    let board = Arc::clone(state.jobs().expect("the board is open"));
    let waiting = tokio::spawn(async move { board.drained().await });
    // The spawned wait is polled up to its `await` before the job is released:
    // yielding hands the current-thread runtime to the waiter, without a sleep.
    tokio::task::yield_now().await;
    let _ = release.send(());
    tokio::time::timeout(Duration::from_secs(2), waiting)
        .await
        .expect("the armed wait is woken by the last job")
        .expect("the wait task joins");
    assert!(
        state
            .jobs()
            .expect("the board is open")
            .unfinished()
            .is_empty(),
        "the job finished, it was not abandoned"
    );
}

#[tokio::test]
async fn a_full_queue_renders_503_with_retry_after() {
    // Acceptance criterion: a full queue is `503`. The route that submits does
    // not exist yet, so a test router calls the internal API (ADR-0095).
    let instance = Instance::new(1, 60_000);
    let (state, _shutdown) = instance.state();
    let (release, blocked) = mpsc::channel::<()>();
    let running = state
        .submit_job(
            "alpha",
            JobKind::Compact,
            Box::new(move |_store, _progress, _writer| {
                let _ = blocked.recv_timeout(Duration::from_secs(5));
                Ok(JobOutcome::default())
            }),
        )
        .await
        .expect("the running job is accepted");
    // The writer task takes the first job out of the queue: the slot the second
    // submission needs is free only from that moment.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while state.job(&running.id).map(|job| job.state) != Some(JobState::Running) {
        assert!(
            std::time::Instant::now() < deadline,
            "the writer never started the first job"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let waiting = state
        .submit_job(
            "alpha",
            JobKind::Export,
            Box::new(|_store, _progress, _writer| Ok(JobOutcome::default())),
        )
        .await
        .expect("the waiting job fills the queue");

    let auth =
        Arc::new(Auth::new(&instance.config.auth.keys).expect("the keys of the fixture are valid"));
    let router = crible_server::http::with_common_layers(crible_server::protect(
        Router::new().route("/v1/test/jobs", post(submit)),
        auth,
        Role::Write,
    ))
    .with_state(Arc::clone(&state));

    let response = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/v1/test/jobs")
                .header(AUTHORIZATION, format!("Bearer {}", instance.write_secret))
                .body(Body::empty())
                .expect("the request is valid"),
        )
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok()),
        Some("1"),
        "a refusal is retried after one second (ADR-0103)"
    );
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("the body is readable");
    let body: Value = serde_json::from_slice(&bytes).expect("the body is JSON");
    assert_eq!(body["error"]["code"], "overloaded");
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("write queue of index 'alpha' is full")),
        "the message names the queue: {body}"
    );

    let _ = release.send(());
    state.jobs().expect("the board is open").drained().await;
    assert_eq!(
        state
            .list_jobs(Some("alpha"), Some(JobState::Succeeded))
            .len(),
        2,
        "the refused job is not visible"
    );
    assert_eq!(
        state
            .job(&running.id)
            .expect("the running job is kept")
            .state,
        JobState::Succeeded
    );
    // ADR-0103, decision 9: the place in the queue is reserved before the row is
    // written, so the refused job left no line in the table either.
    let store = JobStore::open_or_create(&instance.root().join("data").join(JOBS_FILE_NAME))
        .expect("the job table is readable");
    let mut rows: Vec<String> = store
        .list(None, None)
        .expect("the job table is readable")
        .into_iter()
        .map(|row| row.id)
        .collect();
    rows.sort();
    let mut accepted = vec![running.id.clone(), waiting.id.clone()];
    accepted.sort();
    assert_eq!(
        rows, accepted,
        "the refused job wrote no row in jobs.sqlite"
    );
}

/// Test route: submits one job and renders it as accepted, without `?wait=true`.
async fn submit(axum::extract::State(state): axum::extract::State<Arc<AppState>>) -> Response {
    match state
        .submit_job(
            "alpha",
            JobKind::Compact,
            Box::new(|_store, _progress, _writer| Ok(JobOutcome::default())),
        )
        .await
    {
        Ok(job) => crible_server::accepted(&state, job, false)
            .await
            .into_response(),
        Err(failure) => refusal(&failure, REQUEST_ID),
    }
}

#[tokio::test]
async fn the_list_filters_and_the_detail_answers_404_for_an_unknown_job() {
    // ADR-0103, decision 7: `index` and `state` filters, a `400` for a state
    // outside the four, and a `404` for a job the table does not hold.
    let instance = Instance::new(16, 60_000);
    let (state, _shutdown) = instance.state();
    let running = state
        .submit_job(
            "alpha",
            JobKind::Compact,
            Box::new(|_store, _progress, _writer| Ok(JobOutcome::default())),
        )
        .await
        .expect("the job is accepted");
    state.jobs().expect("the board is open").drained().await;
    let router = instance.router(&state);

    let (status, _headers, body) = send(
        &router,
        Method::GET,
        "/v1/jobs?index=alpha&state=succeeded",
        Some(&instance.write_secret),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["jobs"].as_array().map(Vec::len), Some(1));
    assert_eq!(body["jobs"][0]["id"], running.id);

    let (status, _headers, body) = send(
        &router,
        Method::GET,
        "/v1/jobs?index=beta",
        Some(&instance.write_secret),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["jobs"].as_array().map(Vec::len), Some(0));

    let (status, _headers, body) = send(
        &router,
        Method::GET,
        "/v1/jobs?state=pending",
        Some(&instance.write_secret),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_request");

    let (status, _headers, body) = send(
        &router,
        Method::GET,
        "/v1/jobs/job_0000000000000000000000000000000b",
        Some(&instance.write_secret),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");

    let (status, _headers, body) = send(
        &router,
        Method::GET,
        &format!("/v1/jobs/{}", running.id),
        Some(&instance.write_secret),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["state"], "succeeded");
}

#[tokio::test]
async fn the_shutdown_drains_the_accepted_jobs_and_refuses_the_later_ones() {
    // Acceptance criterion: no accepted job is lost at the stop, and the bound
    // of the drainage covers them (ADR-0104).
    let instance = Instance::new(16, 60_000);
    let (state, shutdown) = instance.state();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the fixture binds");
    let router = instance.router(&state);
    let served = tokio::spawn(crible_server::run_server(
        listener,
        router,
        Arc::clone(&state),
        shutdown.clone(),
        Duration::from_secs(5),
        Duration::from_secs(5),
    ));
    let done = Arc::new(AtomicUsize::new(0));
    for _ in 0..2 {
        let done = Arc::clone(&done);
        state
            .submit_job(
                "alpha",
                JobKind::Compact,
                Box::new(move |_store, _progress, _writer| {
                    std::thread::sleep(Duration::from_millis(50));
                    done.fetch_add(1, Ordering::AcqRel);
                    Ok(JobOutcome::default())
                }),
            )
            .await
            .expect("the job is accepted before the signal");
    }
    tokio::time::sleep(Duration::from_millis(10)).await;
    shutdown.drain();
    let refused = state
        .submit_job(
            "alpha",
            JobKind::Export,
            Box::new(|_store, _progress, _writer| Ok(JobOutcome::default())),
        )
        .await
        .expect_err("a submission during the drainage is refused");
    assert_eq!(refused.code(), ErrorCode::Overloaded);
    assert_eq!(refused.message(), SHUTTING_DOWN_MESSAGE);
    let reason = served
        .await
        .expect("the server task joins")
        .expect("the server stops without an error");
    assert_eq!(
        reason,
        crible_server::StopReason::Drained,
        "the accepted jobs finished within the bound"
    );
    assert_eq!(done.load(Ordering::Acquire), 2, "no accepted job is lost");
    assert!(
        state
            .list_jobs(Some("alpha"), None)
            .iter()
            .all(|job| job.state == JobState::Succeeded),
        "every accepted job is terminal"
    );
}

/// Journal sink of the abandoned-job test.
#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl io::Write for Sink {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("the journal sink is poisoned"))?
            .extend_from_slice(buffer);
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

/// One write handle of the [`Sink`].
struct SinkWriter(Sink);

impl io::Write for SinkWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// Installs the journal once for the whole test binary.
fn journal() -> &'static Sink {
    static SINK: OnceLock<Sink> = OnceLock::new();
    SINK.get_or_init(|| {
        let sink = Sink::default();
        let subscriber = log_subscriber(sink.clone(), "crible_server=warn")
            .expect("the filter of the fixture is valid");
        subscriber
            .try_init()
            .expect("the journal is installed once per test binary");
        sink
    })
}

#[tokio::test]
async fn the_shutdown_bound_abandons_the_jobs_still_unfinished_and_logs_them() {
    // ADR-0104: at the bound, the jobs still unfinished are abandoned, and the
    // line names their identifier, kind and index.
    let sink = journal().clone();
    let instance = Instance::new(16, 60_000);
    let (state, shutdown) = instance.state();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the fixture binds");
    let router = instance.router(&state);
    let served = tokio::spawn(crible_server::run_server(
        listener,
        router,
        Arc::clone(&state),
        shutdown.clone(),
        Duration::from_millis(100),
        Duration::from_millis(100),
    ));
    let (release, blocked) = mpsc::channel::<()>();
    let job = state
        .submit_job(
            "alpha",
            JobKind::Compact,
            Box::new(move |_store, _progress, _writer| {
                let _ = blocked.recv_timeout(Duration::from_secs(5));
                Ok(JobOutcome::default())
            }),
        )
        .await
        .expect("the job is accepted before the signal");
    tokio::time::sleep(Duration::from_millis(10)).await;
    shutdown.drain();
    let reason = served
        .await
        .expect("the server task joins")
        .expect("the server stops without an error");
    assert_eq!(
        reason,
        crible_server::StopReason::BoundReached,
        "the job bound is reached"
    );
    let _ = release.send(());
    state.jobs().expect("the board is open").drained().await;

    let bytes = sink.0.lock().expect("the journal is readable").clone();
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("abandoned write job"),
        "the journal records the abandoned job: {text}"
    );
    assert!(text.contains(&job.id), "the line names the job: {text}");
    assert!(text.contains("\"kind\":\"compact\""), "the kind: {text}");
    assert!(text.contains("\"index\":\"alpha\""), "the index: {text}");
}
