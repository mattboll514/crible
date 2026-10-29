// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the helpers of this file are fixtures or assertions.
#![allow(clippy::expect_used, clippy::panic)]

//! Local proof "API = CLI" on the Legifacile sample (ADR-0109, decision 3 (a)).
//!
//! The test is `#[ignore]`d and named exactly
//! `api_and_cli_return_the_same_results_on_the_legifacile_sample`: the name is
//! the contract the fifth `--skip` of the `rust-onnx` job filters, and the data
//! derives from production (ADR-0032, ADR-0057, ADR-0062, ADR-0063, ADR-0085,
//! ADR-0109).
//!
//! For every labelled question of `data/legifacile-sample/queries.jsonl` and for
//! the three modes `dense`, `lexical` and `hybrid` at `k = 10`, it compares the
//! body of `POST /v1/indexes/{index}/search` with the JSON of `crible search`:
//! same keys, same order, same payload, scores **bit for bit**, same `generation`
//! and same `truncated_query`. `took_ms` and `request_id` stay out
//! (`docs/05:263-265`, ADR-0109).
//!
//! The instance is built in one temporary directory from the batches of the
//! sample, through `crible index create` and `crible attach`: the two paths under
//! comparison read the same index and the same model. The real model and ONNX
//! Runtime are required.
//!
//! ```text
//! ORT_DYLIB_PATH=/path/to/libonnxruntime.dylib \
//!   cargo test -p crible --test api_and_cli_on_sample -- --ignored
//! ```

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::config::Config;
use crible_server::http::build_router;
use crible_server::state::AppState;
use crible_server::{Auth, Shutdown};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::common::{
    LEGIFACILE_STRUCTURES, SamplePaths, crible_ok, legifacile_sample_paths, path, read_json_lines,
    repository_root, text,
};

/// Name of the ignored test, the contract of the fifth `--skip` (ADR-0109).
const TEST_NAME: &str = "api_and_cli_return_the_same_results_on_the_legifacile_sample";

/// Secret of the `read` key of the fixture.
const READ_SECRET: &str = "sample-read";

/// Number of results asked of every query (ADR-0109, decision 3).
const K: usize = 10;

/// One temporary instance holding the six structures of the sample.
struct Fixture {
    directory: tempfile::TempDir,
    data: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("a temporary instance is creatable");
        let data = directory.path().join("data");
        fs::create_dir_all(&data).expect("the data root is creatable");
        fs::create_dir_all(directory.path().join("exchange"))
            .expect("the exchange root is creatable");
        Self { directory, data }
    }

    fn config_path(&self) -> PathBuf {
        self.directory.path().join("crible.toml")
    }

    fn data_arg(&self) -> &str {
        path(&self.data)
    }
}

/// `crible.toml` of the sample instance, with one `read` key.
fn config_toml(root: &Path, data: &Path, models: &Path) -> String {
    format!(
        r#"
[server]
bind = "127.0.0.1:0"
search_threads = 0
max_concurrent_searches = 2

[paths]
data = "{data}"
models = "{models}"
exchange = "{exchange}"

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
        data = data.display(),
        models = models.display(),
        exchange = root.join("exchange").display(),
        read = crible_server::auth::sha256_hex(READ_SECRET),
    )
}

/// Creates and fills the six indexes of the sample through the CLI.
fn build(fixture: &Fixture, sample: &SamplePaths) {
    let models = path(&sample.models);
    for structure in LEGIFACILE_STRUCTURES {
        crible_ok(&[
            "index",
            "create",
            "--data",
            fixture.data_arg(),
            "--config",
            path(&sample.config(structure)),
        ]);
        crible_ok(&[
            "attach",
            "--data",
            fixture.data_arg(),
            "--models",
            models,
            "--index",
            structure,
            path(&sample.batch(structure)),
        ]);
    }
}

/// Starts the router over the built instance.
fn router(fixture: &Fixture, sample: &SamplePaths) -> Router {
    fs::write(
        fixture.config_path(),
        config_toml(fixture.directory.path(), &fixture.data, &sample.models),
    )
    .expect("crible.toml is writable");
    let config = Config::load_with_env(&fixture.config_path(), Vec::new())
        .unwrap_or_else(|error| panic!("the sample configuration must load: {}", error.message()));
    let state = Arc::new(AppState::new());
    state
        .load(&config)
        .unwrap_or_else(|error| panic!("the sample instance must load: {}", error.message()));
    let auth = Arc::new(
        Auth::new(&config.auth.keys)
            .unwrap_or_else(|error| panic!("the key must be valid: {}", error.message())),
    );
    state
        .set_auth(auth)
        .expect("the authenticator is published once");
    state
        .set_shutdown(Shutdown::new())
        .expect("the shutdown trigger is published once");
    build_router(state)
}

/// Sends one search through the router and returns the decoded body.
async fn search(router: &Router, index: &str, question: &str, mode: &str) -> Value {
    let request = axum::http::Request::builder()
        .method(Method::POST)
        .uri(format!("/v1/indexes/{index}/search"))
        .header("authorization", format!("Bearer {READ_SECRET}"))
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            json!({ "query": question, "mode": mode, "k": K }).to_string(),
        ))
        .expect("the request is valid");
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("the router answers");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 22)
        .await
        .expect("the body is readable");
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    assert_eq!(status, StatusCode::OK, "the search answers: {body}");
    body
}

/// Runs `crible search` and parses its JSON answer.
fn cli_search(
    fixture: &Fixture,
    sample: &SamplePaths,
    index: &str,
    question: &str,
    mode: &str,
) -> Value {
    let output = crate::common::crible(&[
        "search",
        "--data",
        fixture.data_arg(),
        "--index",
        index,
        "--models",
        path(&sample.models),
        "--query",
        question,
        "--mode",
        mode,
        "--k",
        "10",
        "--format",
        "json",
    ]);
    assert!(
        output.status.success(),
        "crible search on {index} ({mode}) failed: {}",
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs data/legifacile-sample, data/models/mdenseon and ONNX Runtime (ADR-0109)"]
async fn api_and_cli_return_the_same_results_on_the_legifacile_sample() {
    let sample = legifacile_sample_paths(&repository_root(), TEST_NAME)
        .unwrap_or_else(|message| panic!("{message}"));
    let fixture = Fixture::new();
    build(&fixture, &sample);
    let router = router(&fixture, &sample);

    let rows = read_json_lines(&sample.directory.join("queries.jsonl"));
    assert_eq!(rows.len(), 300, "the sample holds 300 labelled questions");
    let mut compared_queries = 0_usize;
    for (position, row) in rows.iter().enumerate() {
        let structure = text(row, "structure");
        let question = text(row, "question");
        let label = text(row, "id");
        // Progress line: the run lasts tens of minutes and CI logs must show it
        // is alive.
        eprintln!("[{}/{}] {label} ({structure})", position + 1, rows.len());
        for mode in ["dense", "lexical", "hybrid"] {
            let api = search(&router, &structure, &question, mode).await;
            let cli = cli_search(&fixture, &sample, &structure, &question, mode);
            assert_eq!(
                compared(&api),
                compared(&cli),
                "{label} ({structure}, {mode}) must return the same answer through the API and \
                 the CLI"
            );
            compared_queries += 1;
        }
    }
    assert_eq!(
        compared_queries,
        rows.len() * 3,
        "every question is compared in the three modes"
    );
}
