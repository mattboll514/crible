// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and every helper of this module is a fixture or an assertion.
#![allow(clippy::expect_used, clippy::panic)]

//! Fixtures shared by the tests of `ROUTES-DOCUMENTS-ET-LOTS` that need a live
//! instance (`docs/phases/P6-serveur-et-api.md`).
//!
//! The instance is the one of `batch_path_security.rs`: an `external` model
//! `m` of four dimensions, no lexical branch, an `admin`, a `write` and a `read`
//! key. A second instance can share the exchange of the first, which is how the
//! deposited-batch test hands a batch from an export to an attach.

// This module is compiled into several test binaries and no binary uses every
// helper. Marking each unused item separately would only add noise.
#![allow(dead_code)]

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use axum::Router;
use axum::body::Body;
use axum::http::header::AUTHORIZATION;
use axum::http::{Method, StatusCode};
use crible_server::config::Config;
use crible_server::http::build_router;
use crible_server::logging::log_subscriber;
use crible_server::state::AppState;
use crible_server::{Auth, Shutdown};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt;

/// Secret of the `admin` key.
pub const ADMIN_SECRET: &str = "admin-secret-of-the-routes-tests";

/// Secret of the `write` key.
pub const WRITE_SECRET: &str = "write-secret-of-the-routes-tests";

/// Secret of the `read` key.
pub const READ_SECRET: &str = "read-secret-of-the-routes-tests";

/// Identifier pinned on every request of these tests (ADR-0098).
pub const REQUEST_ID: &str = "routes-test-1";

/// Name of the fixture index.
pub const INDEX: &str = "alpha";

/// Dimensionality of the `external` fixture model.
pub const DIMS: usize = 4;

/// External identifier of the document every test of this module writes.
pub const DOC_ID: &str = "DOC-1";

/// A temporary instance: its directory (kept alive for the test), its
/// configuration.
pub struct Instance {
    directory: TempDir,
    config: Config,
}

impl Instance {
    /// Creates an instance whose exchange is its own `<root>/exchange`.
    pub fn new() -> Self {
        let directory = tempfile::tempdir().expect("a temporary instance is creatable");
        let exchange = directory.path().join("exchange");
        Self::with_exchange(directory, &exchange, 0)
    }

    /// Creates an instance whose `[server].max_concurrent_searches` is `max`.
    ///
    /// The search limit tests need one token, so a second search is refused at
    /// once (ADR-0102, decision 2).
    pub fn with_max_concurrent_searches(max: usize) -> Self {
        let directory = tempfile::tempdir().expect("a temporary instance is creatable");
        let exchange = directory.path().join("exchange");
        Self::with_exchange(directory, &exchange, max)
    }

    /// Creates an instance that shares `exchange` with another instance.
    ///
    /// The caller keeps the holder of the exchange alive, which is why the
    /// directory is not owned here.
    pub fn sharing(exchange: &Path) -> Self {
        let directory = tempfile::tempdir().expect("a temporary instance holder is creatable");
        Self::with_exchange(directory, exchange, 0)
    }

    /// Writes the directories, the model and `crible.toml`, then loads the
    /// configuration with an empty environment.
    fn with_exchange(directory: TempDir, exchange: &Path, max_concurrent_searches: usize) -> Self {
        let root = directory.path();
        for name in ["data", "models"] {
            std::fs::create_dir_all(root.join(name)).expect("an instance directory is creatable");
        }
        std::fs::create_dir_all(exchange.join("inbox"))
            .expect("the inbox of the exchange is creatable");
        std::fs::create_dir_all(exchange.join("outbox"))
            .expect("the outbox of the exchange is creatable");
        write_external_model(root);
        let path = root.join("crible.toml");
        std::fs::write(&path, config_toml(root, exchange, max_concurrent_searches))
            .expect("crible.toml is writable");
        let config = Config::load_with_env(&path, Vec::new()).unwrap_or_else(|error| {
            panic!("the fixture configuration must load: {}", error.message())
        });
        Self { directory, config }
    }

    /// Root of the temporary instance.
    pub fn root(&self) -> &Path {
        self.directory.path()
    }

    /// Root of the exchange the instance uses.
    pub fn exchange(&self) -> &Path {
        &self.config.paths.exchange
    }

    /// Inbox of the exchange: `<exchange>/inbox` (`docs/02-architecture.md`).
    pub fn inbox(&self) -> PathBuf {
        self.exchange().join("inbox")
    }

    /// Outbox of the exchange: `<exchange>/outbox`.
    pub fn outbox(&self) -> PathBuf {
        self.exchange().join("outbox")
    }

    /// Loads the state of the instance, publishes its authenticator and its
    /// trigger, then starts the writers.
    pub fn state(&self) -> Arc<AppState> {
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
}

/// `crible.toml` of the fixture, with an `admin`, a `write` and a `read` key.
fn config_toml(root: &Path, exchange: &Path, max_concurrent_searches: usize) -> String {
    format!(
        r#"
[server]
bind = "127.0.0.1:0"
search_threads = 0
max_concurrent_searches = {max_concurrent_searches}

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
        exchange = exchange.display(),
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
pub fn index_config() -> Value {
    json!({
        "name": INDEX,
        "model": "m",
        "reference_precision": "fp16",
        "schema": {
            "document_fields": { "numero": { "type": "keyword", "returned": true } },
            "passage_fields": { "texte": { "type": "keyword", "returned": true } }
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

/// Sends one request through the router and returns its status and decoded
/// body.
pub async fn send(
    router: &Router,
    method: Method,
    path: &str,
    secret: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let (status, _headers, body) = send_with_headers(router, method, path, secret, body).await;
    (status, body)
}

/// Same as [`send`], and returns the response headers as well.
///
/// The concurrency limit of ADR-0102 adds `Retry-After: 1` to its `503`; the
/// search-limit tests read it here.
pub async fn send_with_headers(
    router: &Router,
    method: Method,
    path: &str,
    secret: &str,
    body: Option<Value>,
) -> (StatusCode, axum::http::HeaderMap, Value) {
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

/// Creates the fixture index `alpha` and returns the instance, its state and
/// its router.
pub async fn instance_with_alpha() -> (Instance, Arc<AppState>, Router) {
    instance_with_alpha_and_limit(0).await
}

/// Same as [`instance_with_alpha`], with `[server].max_concurrent_searches`
/// set to `max` for the search-limit tests of ADR-0102.
pub async fn instance_with_alpha_and_limit(max: usize) -> (Instance, Arc<AppState>, Router) {
    let instance = Instance::with_max_concurrent_searches(max);
    let state = instance.state();
    let router = build_router(Arc::clone(&state));
    let (status, body) = send(
        &router,
        Method::POST,
        "/v1/indexes",
        ADMIN_SECRET,
        Some(index_config()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "the index is created: {body}");
    (instance, state, router)
}

/// Exports the index and returns the directory of the batch, relative to the
/// exchange root (`POST /v1/indexes/{index}/exports?wait=true`).
pub async fn export_index(router: &Router, state: &Arc<AppState>) -> String {
    let path = format!("/v1/indexes/{INDEX}/exports?wait=true");
    let (status, accepted) = send(router, Method::POST, &path, ADMIN_SECRET, None).await;
    assert!(
        status == StatusCode::OK || status == StatusCode::ACCEPTED,
        "the export is accepted: {accepted}"
    );
    let job = terminal_job(router, state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the export succeeds: {job}");
    job["result"]["report"]["directory"]
        .as_str()
        .unwrap_or_else(|| panic!("the export reports its directory: {job}"))
        .to_owned()
}

/// Body of an online batch: one document of one passage, with its vector.
///
/// `batch_id` identifies the batch; `text` is the text of the passage and
/// `numero` the value of the document field.
pub fn online_batch(batch_id: &str, text: &str, numero: &str) -> Value {
    json!({
        "batch_id": batch_id,
        "mode": "upsert",
        "documents": [
            {
                "doc_id": DOC_ID,
                "fields": { "numero": numero },
                "passages": [
                    {
                        "text": text,
                        "fields": { "texte": "contrat" },
                        "vector": base64_vector([0.6, 0.8, 0.0, 0.0])
                    }
                ]
            }
        ]
    })
}

/// Body of an online batch that deletes `DOC_ID`.
pub fn deletion_batch(batch_id: &str) -> Value {
    json!({
        "batch_id": batch_id,
        "mode": "upsert",
        "documents": [],
        "deletions": [DOC_ID]
    })
}

/// Base64 of the little-endian `f32` components of `vector`, the vector of an
/// `external` model (`docs/format-lot-v1.md`).
pub fn base64_vector<const N: usize>(vector: [f32; N]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for component in vector {
        bytes.extend_from_slice(&component.to_le_bytes());
    }
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let first = group[0];
        let second = group.get(1).copied().unwrap_or(0);
        let third = group.get(2).copied().unwrap_or(0);
        encoded.push(char::from(ALPHABET[usize::from(first >> 2)]));
        encoded.push(char::from(
            ALPHABET[usize::from(((first & 0b11) << 4) | (second >> 4))],
        ));
        if group.len() > 1 {
            encoded.push(char::from(
                ALPHABET[usize::from(((second & 0b1111) << 2) | (third >> 6))],
            ));
        } else {
            encoded.push('=');
        }
        if group.len() > 2 {
            encoded.push(char::from(ALPHABET[usize::from(third & 0b11_1111)]));
        } else {
            encoded.push('=');
        }
    }
    encoded
}

/// Waits for the job of an accepted mutation and returns the body of
/// `GET /v1/jobs/{job_id}`.
pub async fn terminal_job(router: &Router, state: &Arc<AppState>, accepted: &Value) -> Value {
    let id = accepted["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the acceptance carries the job id: {accepted}"));
    let terminal = matches!(accepted["state"].as_str(), Some("succeeded" | "failed"));
    if !terminal {
        let view = state
            .wait_for_job(id)
            .await
            .unwrap_or_else(|| panic!("the job {id} reaches a terminal state"));
        assert!(
            matches!(view.state.as_str(), "succeeded" | "failed"),
            "the job {id} is terminal after the wait: {}",
            view.state.as_str()
        );
    }
    let path = format!("/v1/jobs/{id}");
    let (status, job) = send(router, Method::GET, &path, WRITE_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "the job is readable: {job}");
    job
}

/// Posts `body` on `POST /v1/indexes/{index}/documents?wait=true` and returns
/// the acceptance.
pub async fn post_documents(router: &Router, body: Value) -> (StatusCode, Value) {
    let path = format!("/v1/indexes/{INDEX}/documents?wait=true");
    send(router, Method::POST, &path, WRITE_SECRET, Some(body)).await
}

/// In-memory writer that captures the journal of a test binary.
#[derive(Clone, Default)]
pub struct Sink(Arc<Mutex<Vec<u8>>>);

impl Sink {
    /// Contents captured so far, lossily decoded.
    pub fn contents(&self) -> String {
        let bytes = self.0.lock().map(|bytes| bytes.clone()).unwrap_or_default();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// One write handle of the [`Sink`].
pub struct SinkWriter(Sink);

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

/// Journal of the test binary, installed once at `info` (ADR-0096).
///
/// The server journalises from its runtime, so the subscriber is global to the
/// binary; every test that reads the journal goes through this function, which
/// keeps a single subscriber whatever the number of readers.
pub fn journal() -> Sink {
    static JOURNAL: OnceLock<Sink> = OnceLock::new();
    JOURNAL
        .get_or_init(|| {
            let sink = Sink::default();
            let subscriber = log_subscriber(sink.clone(), "info").expect("info is a valid filter");
            subscriber
                .try_init()
                .expect("no other test of this binary installs a journal");
            sink
        })
        .clone()
}
