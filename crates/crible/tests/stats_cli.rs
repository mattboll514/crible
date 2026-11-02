// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the helpers of this file are fixtures or assertions.
#![allow(clippy::expect_used, clippy::panic)]

//! `crible stats` (ADR-0124, decisions 13, 14 and 15).
//!
//! | Test | What it proves |
//! |---|---|
//! | `the_json_of_the_cli_is_the_body_of_the_route_without_analytics_dropped` | the two paths agree on the same traffic, `analytics_dropped` absent from the CLI |
//! | `the_table_format_is_the_default_and_renders_the_sections` | `--format table` is the default, and every section is printed |
//! | `an_absent_file_is_a_message_and_not_a_failure` | absent `analytics.sqlite`: clear message, exit code 0 |
//! | `a_parameter_outside_its_domain_is_refused_as_the_route_does` | same parameters, same refusal |
//! | `the_command_writes_nothing_in_the_analytics` | read-only: the bytes and the row count do not change |
//!
//! The traffic is written straight into `analytics.sqlite` before the state
//! starts: the route only reads the file, so a known set of rows reaches both
//! paths without driving the search routes. The fixture never loads a model.
//!
//! ```text
//! cargo test -p crible --test stats_cli
//! ```

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::http::Method;
use crible_server::config::Config;
use crible_server::http::build_router;
use crible_server::state::AppState;
use crible_server::{Auth, Shutdown};
use crible_store::analytics::{
    ANALYTICS_FILE_NAME, AnalyticsStore, DayIndex, MinuteEvent, ShapeCount, read_minute_events,
};
use serde_json::Value;
use tower::ServiceExt;

use crate::common::{crible, path};

/// Secret of the `admin` key of the fixture.
const ADMIN_SECRET: &str = "stats-cli-admin";

/// A temporary instance: data, models, exchange and `crible.toml`.
struct Fixture {
    directory: tempfile::TempDir,
    data: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("a temporary instance is creatable");
        let root = directory.path();
        for name in ["data", "models", "exchange"] {
            std::fs::create_dir_all(root.join(name)).expect("a fixture directory is creatable");
        }
        std::fs::write(root.join("crible.toml"), config_toml(root))
            .expect("crible.toml is written");
        Self {
            data: root.join("data"),
            directory,
        }
    }

    /// Path of the analytics file of the instance.
    fn analytics(&self) -> PathBuf {
        self.data.join(ANALYTICS_FILE_NAME)
    }

    /// Writes the traffic the two paths read: three successful searches, one of
    /// them without a result, and one refusal, with one filter form and one
    /// daily row.
    fn seed(&self) {
        let store = AnalyticsStore::open_or_create(&self.analytics())
            .expect("the analytics file is creatable");
        let mut ok = MinuteEvent::empty("alpha", "search", "ok");
        ok.client = "reader".to_owned();
        ok.mode = "dense".to_owned();
        ok.dense_strategy = "exact".to_owned();
        ok.count = 3;
        ok.zero_results = 1;
        for stage in 0..9 {
            ok.add_latency(stage, 3);
        }
        let mut refused = MinuteEvent::empty("alpha", "search", "invalid_filter");
        refused.client = "reader".to_owned();
        refused.mode = "dense".to_owned();
        refused.count = 1;
        for stage in 0..9 {
            refused.add_latency(stage, 1);
        }
        let shapes = vec![ShapeCount {
            index: "alpha".to_owned(),
            shape: "fonds:eq".to_owned(),
            count: 2,
        }];
        let day = DayIndex {
            index: "alpha".to_owned(),
            documents: 7,
            live_passages: 21,
            dead_passages: 2,
            disk_bytes: 4096,
            generation: 5,
            batches: 9,
            ..DayIndex::default()
        };
        store
            .write(&[ok, refused], &shapes, &[day])
            .expect("the analytics of the fixture are written");
    }

    /// The state of the instance, its writers started.
    fn state(&self) -> Arc<AppState> {
        let config = Config::load_with_env(&self.directory.path().join("crible.toml"), Vec::new())
            .unwrap_or_else(|error| {
                panic!("the fixture configuration must load: {}", error.message())
            });
        let state = Arc::new(AppState::new());
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

/// `crible.toml` of the fixture, with the `admin` key of the route.
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

/// Sends the statistics request `query` through the router.
async fn route(router: &Router, query: &str) -> Value {
    let request = axum::http::Request::builder()
        .method(Method::GET)
        .uri(format!("/v1/stats{query}"))
        .header("authorization", format!("Bearer {ADMIN_SECRET}"))
        .body(axum::body::Body::empty())
        .expect("the request is valid");
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("the router answers");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::OK,
        "the route answers the statistics"
    );
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("the body is readable");
    serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        panic!(
            "the route answer must be JSON: {error}\n{}",
            String::from_utf8_lossy(&bytes)
        )
    })
}

/// Runs `crible stats` and asserts it succeeded, returning its stdout.
fn cli(fixture: &Fixture, extra: &[&str]) -> String {
    let data = path(&fixture.data);
    let mut arguments = vec!["stats", "--data", data];
    arguments.extend_from_slice(extra);
    let output = crible(&arguments);
    assert!(
        output.status.success(),
        "crible {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("the output is UTF-8")
}

/// `--format json` prints exactly the body of the route, less
/// `analytics_dropped` (ADR-0124, decisions 14 and 15).
#[tokio::test]
async fn the_json_of_the_cli_is_the_body_of_the_route_without_analytics_dropped() {
    let fixture = Fixture::new();
    fixture.seed();
    let state = fixture.state();
    let router = build_router(Arc::clone(&state));

    for (query, arguments) in [
        ("", &["--format", "json"][..]),
        (
            "?group_by=minute",
            &["--format", "json", "--group-by", "minute"][..],
        ),
        (
            "?index=alpha",
            &["--format", "json", "--index", "alpha"][..],
        ),
        (
            "?operation=search",
            &["--format", "json", "--operation", "search"][..],
        ),
    ] {
        let mut expected = route(&router, query).await;
        let dropped = expected
            .as_object_mut()
            .and_then(|body| body.remove("analytics_dropped"));
        assert!(
            dropped.is_some(),
            "the route renders analytics_dropped: {expected}"
        );
        let raw = cli(&fixture, arguments);
        let printed: Value = serde_json::from_str(&raw).unwrap_or_else(|error| {
            panic!("the CLI answer must be JSON: {error}\n{raw}");
        });
        assert_eq!(
            printed, expected,
            "the CLI renders the body of the route, less analytics_dropped ({query})"
        );
    }
}

/// `table` is the default format, and it prints the window, the buckets, the
/// totals, the two rankings and the daily state (ADR-0124, decision 14).
#[tokio::test]
async fn the_table_format_is_the_default_and_renders_the_sections() {
    let fixture = Fixture::new();
    fixture.seed();
    let printed = cli(&fixture, &[]);
    for expected in [
        "window ",
        "buckets: hour",
        "buckets\n",
        "start",
        "errors",
        "zero_results",
        "totals",
        "invalid_filter=1",
        "latency (ms)",
        "p50_ms",
        "top indexes",
        "alpha",
        "top filter shapes",
        "fonds:eq",
        "indexes",
        "generation",
        "batches",
    ] {
        assert!(
            printed.contains(expected),
            "the table holds '{expected}':\n{printed}"
        );
    }
    // The percentiles are rendered per stage, not only for the total: the
    // `totals` line of the latency table names each step the fixture sampled
    // (ADR-0124, decision 4).
    for stage in ["validate", "encode", "filter", "dense", "fusion", "total"] {
        assert!(
            printed
                .lines()
                .any(|line| line.starts_with("totals") && line.contains(stage)),
            "the latency table renders the stage '{stage}':\n{printed}"
        );
    }
    // The percentiles live in the `latency (ms)` table only: the buckets table
    // carries the state of the window, never a repeated `total`
    // (`@relecteur`, NETTOYER-P7).
    let header = printed
        .lines()
        .find(|line| line.starts_with("start") && line.contains("zero_results"))
        .unwrap_or_else(|| panic!("the buckets table has its header:\n{printed}"));
    assert!(
        !header.contains("p50_ms"),
        "the buckets table leaves the percentiles to the latency table: {header}"
    );
    assert!(
        !printed.contains("analytics_dropped"),
        "the CLI does not render a per-process counter:\n{printed}"
    );
}

/// A missing `analytics.sqlite` is a message, not a failure: the command names
/// the file and the data directory and exits with code `0`
/// (ADR-0124, decision 13).
#[tokio::test]
async fn an_absent_file_is_a_message_and_not_a_failure() {
    let fixture = Fixture::new();
    assert!(!fixture.analytics().exists(), "the fixture starts empty");
    let data = path(&fixture.data);
    let output = crible(&["stats", "--data", data]);
    assert!(
        output.status.success(),
        "an absent file is not a failure: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let printed = String::from_utf8(output.stdout).expect("the output is UTF-8");
    assert!(printed.contains(ANALYTICS_FILE_NAME), "{printed}");
    assert!(printed.contains(data), "{printed}");
    assert!(
        output.stderr.is_empty(),
        "the message is on stdout: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The command never writes in the analytics: the bytes of the file and the
/// number of rows are the same before and after (ADR-0124, decision 13).
#[tokio::test]
async fn the_command_writes_nothing_in_the_analytics() {
    let fixture = Fixture::new();
    fixture.seed();
    let before = std::fs::read(fixture.analytics()).expect("the analytics file is readable");
    let rows = read_minute_events(&fixture.analytics())
        .expect("the analytics are readable")
        .len();

    cli(&fixture, &[]);
    cli(&fixture, &["--format", "json"]);

    let after = std::fs::read(fixture.analytics()).expect("the analytics file is readable");
    assert_eq!(before, after, "the command writes nothing in the file");
    assert_eq!(
        read_minute_events(&fixture.analytics())
            .expect("the analytics are readable")
            .len(),
        rows,
        "the command adds no row"
    );
}

/// The command checks its parameters with the code of the route, before it
/// looks at the file: the same query is refused with the same message
/// (ADR-0124, decisions 2 and 14).
#[tokio::test]
async fn a_parameter_outside_its_domain_is_refused_as_the_route_does() {
    let fixture = Fixture::new();
    let state = fixture.state();
    let router = build_router(Arc::clone(&state));
    let request = axum::http::Request::builder()
        .method(Method::GET)
        .uri("/v1/stats?group_by=week")
        .header("authorization", format!("Bearer {ADMIN_SECRET}"))
        .body(axum::body::Body::empty())
        .expect("the request is valid");
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("the router answers");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::BAD_REQUEST,
        "the route refuses the width"
    );
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("the body is readable");
    let answer: Value = serde_json::from_slice(&bytes).expect("the refusal is JSON");
    let expected = answer["error"]["message"]
        .as_str()
        .expect("the refusal carries a message")
        .to_owned();

    let data = path(&fixture.data);
    let output = crible(&["stats", "--data", data, "--group-by", "week"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a refused parameter is a failure"
    );
    let stderr = String::from_utf8(output.stderr).expect("the error is UTF-8");
    assert_eq!(
        stderr,
        format!("invalid_request: {expected}\n"),
        "the CLI refuses with the words of the route"
    );
}
