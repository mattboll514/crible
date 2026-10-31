// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and several fixtures and checks of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! Lifecycle of the server: readiness, drainage, startup failures.
//!
//! Acceptance criteria of `DEMARRER-SERVEUR-AXUM`
//! (`docs/phases/P6-serveur-et-api.md`):
//!
//! - `/ready` renders `503` until every index has published its snapshot, then
//!   `200` (ADR-0099);
//! - a stop test shows that a request in flight ends before the exit, within
//!   the drainage bound (ADR-0099).
//!
//! Everything here runs **without ONNX Runtime**: the instance serves an
//! `external` index or no index at all, as ADR-0099 requires of the ordinary
//! CI. The startup failure of a missing model file is proved on the hash check,
//! which runs before ONNX Runtime is opened.

use std::fs;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use axum::Router;
use axum::routing::get;
use crible_core::config::IndexConfig;
use crible_core::error::ErrorCode;
use crible_server::Shutdown;
use crible_server::config::Config;
use crible_server::http::build_router;
use crible_server::lifecycle::{StopReason, run_server, serve_bound};
use crible_server::state::AppState;
use crible_store::generation::{GenerationStore, INITIAL_EPOCH, write_index_config};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Configuration of an index that needs no encoder and no lexical branch.
///
/// The lexical branch is disabled, so the store never builds a tantivy index:
/// the startup stays cheap and touches no French analyzer.
fn index_json(name: &str, model: &str) -> String {
    format!(
        r#"{{"name":"{name}","model":"{model}","reference_precision":"fp16",
            "schema":{{"document_fields":{{}},"passage_fields":{{}}}},"cluster_by":[],
            "lexical":{{"enabled":false,"language":"fr","positions":false}},
            "search":{{"scan":"none","rescore_candidates":10,"exact_threshold":10,
                      "candidates_per_branch":10,"fusion":{{"method":"rrf"}}}},
            "ingest":{{"acceptance_sample":1,"acceptance_min_cosine":0.9}},
            "compaction":{{"dead_ratio":0.1}}}}"#
    )
}

/// The `crible.toml` of an instance rooted at `root`, bound to `bind`.
///
/// It carries every key of `docs/07`: the loader refuses a missing one.
fn config_toml(root: &Path, bind: &str) -> String {
    format!(
        r#"
[server]
bind = "{bind}"
search_threads = 0
max_concurrent_searches = 0

[paths]
data = "{data}"
models = "{models}"
exchange = "{exchange}"

[[auth.keys]]
name = "legifacile-mcp"
role = "read"
sha256 = "1111111111111111111111111111111111111111111111111111111111111111"

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
    )
}

/// A temporary instance: its directory (kept alive for the test) and its
/// configuration as the server reads it.
struct Instance {
    _directory: TempDir,
    config: Config,
}

impl Instance {
    /// Creates the instance directories, writes `crible.toml`, and loads it
    /// with an empty environment: a `CRIBLE_*` variable of the host must not
    /// change what the test asserts.
    fn new(build: impl FnOnce(&Path)) -> Self {
        let directory = tempfile::tempdir().expect("a temporary instance is creatable");
        let root = directory.path();
        for name in ["data", "models", "exchange"] {
            fs::create_dir_all(root.join(name)).expect("an instance directory is creatable");
        }
        build(root);
        let path = root.join("crible.toml");
        fs::write(&path, config_toml(root, "127.0.0.1:0")).expect("crible.toml is writable");
        let config = Config::load_with_env(&path, Vec::new()).unwrap_or_else(|error| {
            panic!("the fixture configuration must load: {}", error.message())
        });
        Self {
            _directory: directory,
            config,
        }
    }

    /// Root directory of the instance.
    fn root(&self) -> &Path {
        self._directory.path()
    }
}

/// Writes the external model directory `m` under `<root>/models`.
fn write_external_model(root: &Path) {
    let directory = root.join("models").join("m");
    fs::create_dir_all(&directory).expect("the model directory is creatable");
    fs::write(
        directory.join("model.json"),
        r#"{"name":"m","kind":"external","dims":4,"normalize":true}"#,
    )
    .expect("the model configuration is written");
}

/// Creates a named pipe, whose reader blocks until a writer opens it.
#[cfg(unix)]
fn fifo(path: &Path) {
    let status = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("mkfifo must run");
    assert!(status.success(), "mkfifo must create {}", path.display());
}

/// Creates the index `alpha` of `root`, with `model` in its `index.json`.
fn create_index(root: &Path, model: &str) {
    let raw = index_json("alpha", model);
    let index = IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("the fixture index must be valid: {}", error.message()));
    let directory = root.join("data").join("indexes").join("alpha");
    write_index_config(&directory, &raw)
        .unwrap_or_else(|error| panic!("the index configuration is written: {}", error.message()));
    GenerationStore::create(&directory, INITIAL_EPOCH, &index.schema)
        .unwrap_or_else(|error| panic!("the index is created: {}", error.message()));
}

/// An instance serving one `external` index.
fn instance_with_external_index() -> Instance {
    Instance::new(|root| {
        write_external_model(root);
        create_index(root, "m");
    })
}

/// One HTTP/1.1 reply, as the probes are read over a socket.
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }

    fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|error| panic!("the body must be JSON, got {:?}: {error}", self.body))
    }
}

/// Sends one `GET` with `Connection: close` and reads the whole reply.
async fn http_get(address: std::net::SocketAddr, path: &str) -> Reply {
    try_http_get(address, path)
        .await
        .unwrap_or_else(|| panic!("the server at {address} must accept"))
}

/// Same as [`http_get`], or `None` while the server does not accept yet.
async fn try_http_get(address: std::net::SocketAddr, path: &str) -> Option<Reply> {
    let mut stream = TcpStream::connect(address).await.ok()?;
    let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("the request is writable");
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .await
        .expect("the reply is readable");
    Some(parse_reply(&raw))
}

/// Waits until `path` answers `expected`, and returns the reply.
async fn wait_for(address: std::net::SocketAddr, path: &str, expected: u16) -> Reply {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(reply) = try_http_get(address, path).await
            && reply.status == expected
        {
            return reply;
        }
        assert!(
            Instant::now() < deadline,
            "{path} never answered {expected} within 10 s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn parse_reply(raw: &[u8]) -> Reply {
    let text = String::from_utf8_lossy(raw);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("a reply has a head and a body, got {text:?}"));
    let mut lines = head.lines();
    let status_line = lines.next().expect("a reply has a status line");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or_else(|| panic!("the status line carries a code, got {status_line:?}"));
    let headers = lines
        .filter_map(|line| {
            line.split_once(':')
                .map(|(key, value)| (key.trim().to_ascii_lowercase(), value.trim().to_owned()))
        })
        .collect();
    Reply {
        status,
        headers,
        body: body.to_owned(),
    }
}

/// Binds an ephemeral port and returns the listener and its address.
async fn bind_ephemeral() -> (TcpListener, std::net::SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("an ephemeral port is bindable");
    let address = listener.local_addr().expect("the bound address is known");
    (listener, address)
}

/// Joins the server task and returns its stop reason.
async fn stopped(
    server: tokio::task::JoinHandle<Result<StopReason, crible_core::error::CribleError>>,
) -> StopReason {
    let joined = server.await.expect("the server task joins");
    joined.unwrap_or_else(|error| panic!("the server must stop cleanly: {}", error.message()))
}

#[test]
fn ready_stays_503_until_the_indexes_are_published() {
    // Criterion of `DEMARRER-SERVEUR-AXUM` and ADR-0099: the instance is not
    // ready while its indexes have not published, even once the loader is
    // over; it becomes ready when its state says so.
    let instance = instance_with_external_index();
    let state = Arc::new(AppState::new());
    state
        .load(&instance.config)
        .unwrap_or_else(|error| panic!("the fixture must load: {}", error.message()));
    assert!(
        !state.is_ready(),
        "opening the index does not publish the readiness by itself"
    );
    assert!(
        state
            .loaded()
            .is_some_and(|loaded| loaded.indexes().is_ok_and(|indexes| indexes.len() == 1)),
        "the index of the fixture is open"
    );
    state.mark_ready();
    assert!(state.is_ready());
}

#[tokio::test]
async fn the_probe_bodies_distinguish_alive_from_ready() {
    // ADR-0099: `/health` says the process is alive, `/ready` says the service
    // can answer. The router is exercised before and after publication.
    let state = Arc::new(AppState::new());
    let router = build_router(Arc::clone(&state));
    let (listener, address) = bind_ephemeral().await;
    let shutdown = Shutdown::new();
    let server = tokio::spawn(run_server(
        listener,
        router,
        Arc::clone(&state),
        shutdown.clone(),
        Duration::from_secs(5),
        Duration::from_secs(5),
    ));

    let health = http_get(address, "/health").await;
    assert_eq!(health.status, 200, "/health answers from the bind");
    assert_eq!(health.json()["status"], "ok");

    let ready = http_get(address, "/ready").await;
    assert_eq!(ready.status, 503, "not published yet");
    assert_eq!(ready.json()["status"], "not_ready");
    assert!(
        ready.header("x-request-id").is_some(),
        "a 503 still carries the request identifier (ADR-0098)"
    );

    state.mark_ready();
    let ready = http_get(address, "/ready").await;
    assert_eq!(ready.status, 200, "published");
    assert_eq!(ready.json()["status"], "ready");
    assert_ne!(
        health.body, ready.body,
        "the two probes tell two states apart"
    );

    shutdown.drain();
    assert_eq!(stopped(server).await, StopReason::Drained);
}

#[cfg(unix)]
#[tokio::test]
async fn ready_goes_from_503_to_200_on_a_loaded_index() {
    // The criterion on the router: `serve_bound` binds and serves before it
    // loads, so `/ready` answers 503 until the loader has opened the external
    // index, then 200. The model file is a named pipe, which blocks the loader,
    // so the 503 is observed deterministically; the test then writes the
    // configuration and the transition happens for real.
    let instance = Instance::new(|root| {
        let directory = root.join("models").join("m");
        fs::create_dir_all(&directory).expect("the model directory is creatable");
        fifo(&directory.join("model.json"));
        create_index(root, "m");
    });
    let (listener, address) = bind_ephemeral().await;
    let server = tokio::spawn(serve_bound(listener, instance.config.clone()));

    let health = wait_for(address, "/health", 200).await;
    assert_eq!(health.json()["status"], "ok");
    let ready = http_get(address, "/ready").await;
    assert_eq!(ready.status, 503, "the loader is still blocked");
    assert_eq!(ready.json()["status"], "not_ready");

    let model = instance.root().join("models").join("m").join("model.json");
    tokio::task::spawn_blocking(move || {
        fs::write(
            model,
            r#"{"name":"m","kind":"external","dims":4,"normalize":true}"#,
        )
    })
    .await
    .expect("the writer task joins")
    .expect("the model configuration is written");

    let ready = wait_for(address, "/ready", 200).await;
    assert_eq!(ready.json()["status"], "ready");
    server.abort();
}

#[tokio::test]
async fn an_instance_without_index_is_ready() {
    // ADR-0099: "une instance sans index est prête".
    let instance = Instance::new(|_| {});
    let state = Arc::new(AppState::new());
    state
        .load(&instance.config)
        .unwrap_or_else(|error| panic!("an empty instance must load: {}", error.message()));
    state.mark_ready();
    assert!(state.is_ready());
    assert!(
        state
            .loaded()
            .is_some_and(|loaded| loaded.indexes().is_ok_and(|indexes| indexes.is_empty())),
        "no index is opened"
    );
}

#[test]
fn the_startup_purges_the_stale_online_batches() {
    // ADR-0099 and `docs/02-architecture.md:140`: the write open removes the
    // `online-*` leftovers of `<data>/tmp` whose presence lock is free, and
    // keeps a lot whose lock a writer holds (ADR-0056, decision 4).
    use crible_store::online::TEMPORARY_DIRECTORY;
    use std::fs::File;

    let instance = Instance::new(|root| {
        let temporary = root.join("data").join(TEMPORARY_DIRECTORY);
        fs::create_dir_all(temporary.join("online-dead")).expect("the leftover is creatable");
        fs::write(temporary.join("online-dead.lock"), b"").expect("its lock is writable");
        fs::create_dir_all(temporary.join("online-live")).expect("the live lot is creatable");
    });
    let temporary = instance.root().join("data").join(TEMPORARY_DIRECTORY);
    let held = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(temporary.join("online-live.lock"))
        .expect("the lock of the live lot is creatable");
    held.try_lock().expect("the test takes the lock of the lot");

    AppState::new()
        .load(&instance.config)
        .unwrap_or_else(|error| panic!("the fixture must load: {}", error.message()));

    assert!(
        !temporary.join("online-dead").exists(),
        "a leftover whose lock is free is purged"
    );
    assert!(
        !temporary.join("online-dead.lock").exists(),
        "its presence lock goes with it"
    );
    assert!(
        temporary.join("online-live").exists(),
        "a lot whose lock is held is kept"
    );
    assert!(
        temporary.join("online-live.lock").exists(),
        "its presence lock is kept too"
    );

    held.unlock().expect("the test releases the lock");
}

#[test]
fn an_absent_data_root_is_refused() {
    // A typo in `[paths].data` must not start an instance that serves nothing.
    let instance = Instance::new(|root| {
        fs::remove_dir_all(root.join("data")).expect("the data root is removable");
    });
    let error = AppState::new()
        .load(&instance.config)
        .expect_err("an absent data root is refused");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(error.message().contains("data"), "got {}", error.message());
}

#[test]
fn an_index_directory_without_configuration_stops_the_startup() {
    let instance = Instance::new(|root| {
        fs::create_dir_all(root.join("data").join("indexes").join("broken"))
            .expect("the directory is creatable");
    });
    let error = AppState::new()
        .load(&instance.config)
        .expect_err("a directory that is no index stops the startup");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(
        error.message().contains("index.json"),
        "got {}",
        error.message()
    );
}

#[test]
fn a_model_name_that_leaves_the_models_root_stops_the_startup() {
    // `index.json` is data of the index, not a trusted input: a `model` of `..`
    // or an absolute path would read a configuration outside `[paths].models`.
    for model in ["../escaped", "/etc/passwd", ".."] {
        let instance = Instance::new(|root| create_index(root, model));
        let error = AppState::new()
            .load(&instance.config)
            .expect_err("a model name outside the models root stops the startup");
        assert_eq!(error.code(), ErrorCode::InvalidRequest);
        let message = error.message();
        assert!(
            message.contains("single path component"),
            "the guard is named: {message}"
        );
    }
}

#[test]
fn a_missing_model_file_stops_the_startup() {
    // ADR-0099: a model the index requires and that is absent fails the
    // startup, naming the model and the file. The hash check runs before ONNX
    // Runtime is opened, so this proof needs no ONNX Runtime.
    let instance = Instance::new(|root| {
        let directory = root.join("models").join("onnx");
        fs::create_dir_all(&directory).expect("the model directory is creatable");
        fs::write(
            directory.join("model.json"),
            r#"{"name":"onnx","kind":"onnx","hf_id":"huggingface/model","revision":"abc",
                "dims":4,"pooling":"cls","normalize":true,"max_tokens":512,
                "prompts":{"query":"query: ","document":"document: "},
                "files":{"fp32":{"path":"model.onnx",
                                 "blake3":"0000000000000000000000000000000000000000000000000000000000000000"},
                         "tokenizer":{"path":"tokenizer.json",
                                      "blake3":"0000000000000000000000000000000000000000000000000000000000000000"}}}"#,
        )
        .expect("the model configuration is written");
        create_index(root, "onnx");
    });
    let error = AppState::new()
        .load(&instance.config)
        .expect_err("a missing model file stops the startup");
    let message = error.message();
    assert!(message.contains("onnx"), "the model is named: {message}");
    assert!(
        message.contains("model.onnx"),
        "the file is named: {message}"
    );
}

/// Set by the `/slow` route when a request reaches it.
static SLOW_STARTED: OnceLock<tokio::sync::Notify> = OnceLock::new();

/// A route that answers after 300 ms, so a request can be in flight.
async fn slow() -> &'static str {
    if let Some(started) = SLOW_STARTED.get() {
        started.notify_one();
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    "finished"
}

#[tokio::test]
async fn a_request_in_flight_ends_before_the_exit() {
    // Criterion of `DEMARRER-SERVEUR-AXUM` (ADR-0095, ADR-0099): the stop
    // drains the requests in flight instead of cutting them.
    let (listener, address) = bind_ephemeral().await;
    let state = Arc::new(AppState::new());
    state.mark_ready();
    let router = build_router(Arc::clone(&state)).merge(Router::new().route("/slow", get(slow)));
    let shutdown = Shutdown::new();
    let server = tokio::spawn(run_server(
        listener,
        router,
        Arc::clone(&state),
        shutdown.clone(),
        Duration::from_secs(5),
        Duration::from_secs(5),
    ));

    let started = SLOW_STARTED.get_or_init(tokio::sync::Notify::new);
    let request = tokio::spawn(async move { http_get(address, "/slow").await });
    started.notified().await;

    shutdown.drain();
    let reply = request.await.expect("the request task joins");
    assert_eq!(reply.status, 200, "the request in flight completes");
    assert_eq!(reply.body, "finished");
    assert_eq!(
        stopped(server).await,
        StopReason::Drained,
        "the server stopped after the request ended"
    );
}

/// Set by the `/hold` route when a request reaches it.
static HOLD_STARTED: OnceLock<tokio::sync::Notify> = OnceLock::new();

/// A route that holds a connection for 300 ms, long enough for the drain to
/// start while it is in flight.
async fn hold() -> &'static str {
    if let Some(started) = HOLD_STARTED.get() {
        started.notify_one();
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    "held"
}

#[tokio::test]
async fn the_drain_withdraws_readiness() {
    // ADR-0099, "Arrêt", step 1: the drainage withdraws `/ready`, so it answers
    // `503`. The withdrawal is owned by the beginning of the drainage in
    // `run_server`, so a test that drains goes through it too, not only a
    // signal. The probe body of a withdrawn instance is the one checked by
    // `the_probe_bodies_distinguish_alive_from_ready`.
    let (listener, address) = bind_ephemeral().await;
    let state = Arc::new(AppState::new());
    state.mark_ready();
    let router = build_router(Arc::clone(&state)).merge(Router::new().route("/hold", get(hold)));
    let shutdown = Shutdown::new();
    let server = tokio::spawn(run_server(
        listener,
        router,
        Arc::clone(&state),
        shutdown.clone(),
        Duration::from_secs(5),
        Duration::from_secs(5),
    ));

    let started = HOLD_STARTED.get_or_init(tokio::sync::Notify::new);
    let request = tokio::spawn(async move { http_get(address, "/hold").await });
    started.notified().await;
    assert!(state.is_ready(), "the instance is ready before the stop");

    shutdown.drain();
    let deadline = Instant::now() + Duration::from_secs(2);
    while state.is_ready() {
        assert!(
            Instant::now() < deadline,
            "the drainage must withdraw the readiness"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let reply = request.await.expect("the request task joins");
    assert_eq!(reply.status, 200, "the request in flight completes");
    assert_eq!(reply.body, "held");
    assert_eq!(stopped(server).await, StopReason::Drained);
}

/// Set by the `/stuck` route when a request reaches it.
static STUCK_STARTED: OnceLock<tokio::sync::Notify> = OnceLock::new();
/// A route that never answers within the test, so the bound is reached.
async fn stuck() -> &'static str {
    if let Some(started) = STUCK_STARTED.get() {
        started.notify_one();
    }
    tokio::time::sleep(Duration::from_secs(30)).await;
    "never"
}

#[tokio::test]
async fn the_drainage_is_bounded() {
    // ADR-0099: the drainage waits at most `default_timeout_ms` + 1 000 ms,
    // then the server logs the cut requests and exits. A client that holds a
    // request open must not hold the process.
    let (listener, address) = bind_ephemeral().await;
    let state = Arc::new(AppState::new());
    state.mark_ready();
    let router = build_router(Arc::clone(&state)).merge(Router::new().route("/stuck", get(stuck)));
    let shutdown = Shutdown::new();
    let bound = Duration::from_millis(150);
    let server = tokio::spawn(run_server(
        listener,
        router,
        Arc::clone(&state),
        shutdown.clone(),
        bound,
        Duration::from_secs(5),
    ));

    let started = STUCK_STARTED.get_or_init(tokio::sync::Notify::new);
    let request = tokio::spawn(async move { http_get(address, "/stuck").await });
    started.notified().await;

    let stop = std::time::Instant::now();
    shutdown.drain();
    let reason = stopped(server).await;
    let elapsed = stop.elapsed();
    assert_eq!(reason, StopReason::BoundReached);
    assert!(
        elapsed >= bound,
        "the bound is awaited before cutting, got {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "the bound ends the drainage instead of waiting for the request, got {elapsed:?}"
    );
    request.abort();
}

#[tokio::test]
async fn a_second_signal_ends_the_drainage_at_once() {
    // ADR-0099: a second signal exits immediately with a non-zero code; the
    // caller turns `StopReason::Forced` into an error.
    let (listener, _address) = bind_ephemeral().await;
    let state = Arc::new(AppState::new());
    state.mark_ready();
    let router = build_router(Arc::clone(&state));
    let shutdown = Shutdown::new();
    let server = tokio::spawn(run_server(
        listener,
        router,
        Arc::clone(&state),
        shutdown.clone(),
        Duration::from_secs(30),
        Duration::from_secs(5),
    ));
    shutdown.force();
    assert_eq!(
        stopped(server).await,
        StopReason::Forced,
        "a forced stop does not wait for the bound"
    );
}
