// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the helpers of this file are fixtures or assertions.
#![allow(clippy::expect_used, clippy::panic)]

//! Continuous proof "API = CLI" of `ROUTE-RECHERCHE` (ADR-0109, decision 3 (b)).
//!
//! A small `external` index is built in one temporary instance, without the
//! Legifacile sample and without ONNX Runtime: the model is `external`, so a
//! dense search takes its `vector` in the request and a lexical search scores
//! the text with BM25 alone (`docs/04`, `docs/05:235`). The test then compares,
//! for `dense` and `lexical`, the body of `POST /v1/indexes/{index}/search` with
//! the JSON printed by `crible search`:
//!
//! - same keys, same order, same payload;
//! - scores equal **bit for bit** (the same `f32` serialized by the same code);
//! - same `pool_size`, `generation` and `truncated_query`.
//!
//! `took_ms` is left out, because the two durations measure different spans, and
//! `request_id` does not exist in the CLI (`docs/05:263-265`, ADR-0109).
//!
//! `hybrid` is not compared here: it needs an encoder for the dense branch,
//! which an `external` index does not have. The local test (a) covers it on the
//! Legifacile sample, with the real model.
//!
//! ```text
//! cargo test -p crible --test api_matches_cli
//! ```

mod common;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::config::Config;
use crible_server::http::build_router;
use crible_server::state::AppState;
use crible_server::{Auth, Shutdown};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::common::crible;

/// Secret of the `admin` key of the fixture.
const ADMIN_SECRET: &str = "api-cli-admin";
/// Secret of the `write` key of the fixture.
const WRITE_SECRET: &str = "api-cli-write";
/// Secret of the `read` key of the fixture.
const READ_SECRET: &str = "api-cli-read";
/// Name of the fixture index.
const INDEX: &str = "api-cli";

/// A temporary instance: data, models, exchange and `crible.toml`.
struct Fixture {
    directory: tempfile::TempDir,
    data: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("a temporary instance is creatable");
        let root = directory.path();
        for name in ["data", "models", "exchange"] {
            fs::create_dir_all(root.join(name)).expect("a fixture directory is creatable");
        }
        let model = root.join("models").join("m");
        fs::create_dir_all(&model).expect("the model directory is creatable");
        fs::write(
            model.join("model.json"),
            r#"{"name":"m","kind":"external","dims":4,"normalize":true}"#,
        )
        .expect("the model configuration is written");
        fs::write(root.join("crible.toml"), config_toml(root)).expect("crible.toml is writable");
        Self {
            data: root.join("data"),
            directory,
        }
    }

    fn config(&self) -> Config {
        Config::load_with_env(&self.directory.path().join("crible.toml"), Vec::new())
            .unwrap_or_else(|error| {
                panic!("the fixture configuration must load: {}", error.message())
            })
    }

    fn state(&self) -> Arc<AppState> {
        let state = Arc::new(AppState::new());
        let config = self.config();
        state
            .load(&config)
            .unwrap_or_else(|error| panic!("the instance must load: {}", error.message()));
        let auth = Arc::new(
            Auth::new(&config.auth.keys)
                .unwrap_or_else(|error| panic!("the keys must be valid: {}", error.message())),
        );
        state
            .set_auth(auth)
            .expect("the authenticator is published once");
        state
            .set_shutdown(Shutdown::new())
            .expect("the shutdown trigger is published once");
        state.start_writers().expect("the writers start");
        state
    }
}

/// `crible.toml` of the fixture, with the three roles of the test.
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
writer_queue = 16
default_timeout_ms = 2000
wait_timeout_ms = 60000
query_cache_entries = 1024
"#,
        data = root.join("data").display(),
        models = root.join("models").display(),
        exchange = root.join("exchange").display(),
        admin = crible_server::auth::sha256_hex(ADMIN_SECRET),
        write = crible_server::auth::sha256_hex(WRITE_SECRET),
        read = crible_server::auth::sha256_hex(READ_SECRET),
    )
}

/// Configuration of the fixture index: `external` model, lexical branch alive.
fn index_config() -> Value {
    json!({
        "name": INDEX,
        "model": "m",
        "reference_precision": "fp16",
        "schema": {
            "document_fields": { "numero": { "type": "keyword", "returned": true } },
            "passage_fields": { "texte": { "type": "keyword", "returned": true } }
        },
        "cluster_by": [],
        "lexical": { "enabled": true, "language": "fr", "positions": false },
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

/// Base64 of little-endian `f32` components (the `vector` of `docs/05`).
fn base64_vector(vector: [f32; 4]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bytes = Vec::with_capacity(16);
    for component in vector {
        bytes.extend_from_slice(&component.to_le_bytes());
    }
    let mut encoded = String::with_capacity(24);
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

/// Sends one request through the router and returns its status and body.
async fn send(
    router: &Router,
    method: Method,
    path: &str,
    secret: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = axum::http::Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {secret}"));
    let payload = match body {
        Some(value) => {
            request = request.header("content-type", "application/json");
            axum::body::Body::from(value.to_string())
        }
        None => axum::body::Body::empty(),
    };
    let response = router
        .clone()
        .oneshot(request.body(payload).expect("the request is valid"))
        .await
        .expect("the router answers");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("the body is readable");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// Writes the documents of the fixture index through the write route.
async fn fill(router: &Router, data: &Path) {
    for (batch, doc_id, numero, text, vector) in [
        (
            "batch-1",
            "DOC-1",
            "1240",
            "le contrat fait la loi",
            [1.0, 0.0, 0.0, 0.0],
        ),
        (
            "batch-2",
            "DOC-2",
            "1241",
            "le delit est puni",
            [0.0, 1.0, 0.0, 0.0],
        ),
        (
            "batch-3",
            "DOC-3",
            "1242",
            "le contrat est nul",
            [0.707_106_77, 0.707_106_77, 0.0, 0.0],
        ),
    ] {
        let body = json!({
            "batch_id": batch,
            "mode": "upsert",
            "documents": [{
                "doc_id": doc_id,
                "fields": { "numero": numero },
                "passages": [{
                    "text": text,
                    "fields": { "texte": "contrat" },
                    "vector": base64_vector(vector),
                }]
            }]
        });
        let path = format!("/v1/indexes/{INDEX}/documents?wait=true");
        let (status, accepted) = send(router, Method::POST, &path, WRITE_SECRET, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "the batch is applied: {accepted}");
        assert_eq!(accepted["state"], "succeeded", "{accepted}");
    }
    assert!(data.is_dir());
}

/// Runs `crible search` and parses its JSON answer.
fn cli_search(fixture: &Fixture, extra: &[&str]) -> Value {
    let data = fixture.data.to_str().expect("the data path is UTF-8");
    let mut arguments = vec![
        "search", "--data", data, "--index", INDEX, "--format", "json",
    ];
    arguments.extend_from_slice(extra);
    let output = crible(&arguments);
    assert!(
        output.status.success(),
        "crible {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "the CLI answer must be JSON: {error}\n{}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

/// What the two paths must agree on (ADR-0109, decision 3).
fn compared(answer: &Value) -> Value {
    json!({
        "results": answer["results"],
        "pool_size": answer["pool_size"],
        "generation": answer["generation"],
        "truncated_query": answer["truncated_query"],
    })
}

/// `dense` and `lexical` return the same answer through the route and the CLI.
#[tokio::test]
async fn api_and_cli_return_the_same_results_on_a_small_external_index() {
    let fixture = Fixture::new();
    let state = fixture.state();
    let router = build_router(Arc::clone(&state));
    let (status, created) = send(
        &router,
        Method::POST,
        "/v1/indexes",
        ADMIN_SECRET,
        Some(index_config()),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the index is created: {created}"
    );
    fill(&router, &fixture.data).await;

    // `dense`: the vector travels in the body and in the `--vector-file`.
    let vector = [1.0_f32, 0.0, 0.0, 0.0];
    let vector_path = fixture.data.join("query.json");
    fs::write(
        &vector_path,
        serde_json::to_string(&vector).expect("the vector serializes"),
    )
    .expect("the query vector is written");
    let dense_path = format!("/v1/indexes/{INDEX}/search");
    let (status, api_dense) = send(
        &router,
        Method::POST,
        &dense_path,
        READ_SECRET,
        Some(json!({ "vector": base64_vector(vector), "mode": "dense", "k": 10 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{api_dense}");
    let cli_dense = cli_search(
        &fixture,
        &[
            "--vector-file",
            vector_path.to_str().expect("UTF-8 path"),
            "--mode",
            "dense",
            "--k",
            "10",
        ],
    );
    assert_eq!(
        compared(&api_dense),
        compared(&cli_dense),
        "the API and the CLI must return the same dense answer"
    );

    // `lexical`: the route scores the text as is; the CLI does the same.
    let (status, api_lexical) = send(
        &router,
        Method::POST,
        &dense_path,
        READ_SECRET,
        Some(json!({ "query": "contrat", "mode": "lexical", "k": 10 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{api_lexical}");
    let cli_lexical = cli_search(
        &fixture,
        &["--query", "contrat", "--mode", "lexical", "--k", "10"],
    );
    assert_eq!(
        compared(&api_lexical),
        compared(&cli_lexical),
        "the API and the CLI must return the same lexical answer"
    );

    // A non-vacuous comparison: the lexical branch really matched two passages.
    assert_eq!(
        api_lexical["results"]
            .as_array()
            .expect("results is an array")
            .len(),
        2,
        "{api_lexical}"
    );
}
