// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixtures of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! `crible serve`: the command, the probes and the stop on a signal.
//!
//! Acceptance criteria of `DEMARRER-SERVEUR-AXUM`
//! (`docs/phases/P6-serveur-et-api.md`) proved at the binary level: the
//! instance answers `/health` and `/ready` once started, the configuration path
//! defaults to `/etc/crible/crible.toml`, and SIGTERM and SIGINT each stop the
//! process cleanly (ADR-0099).
//!
//! The instance serves no index, so ONNX Runtime is never needed (ADR-0099:
//! "une instance sans index est prête").
//!
//! The child process is started and stopped by the test itself, and killed if
//! the test fails before its stop.

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use tempfile::TempDir;

/// Time a child has to answer or to exit before the test fails.
const PATIENCE: Duration = Duration::from_secs(10);

/// A configuration file and the port it binds, in a temporary directory.
struct Instance {
    directory: TempDir,
    port: u16,
}

impl Instance {
    /// Writes a complete `crible.toml`, with an empty instance (no index).
    fn new() -> Self {
        Self::with_timeout(2000)
    }

    /// Same as [`Instance::new`], with `[limits].default_timeout_ms`.
    fn with_timeout(timeout_ms: u64) -> Self {
        let directory = tempfile::tempdir().expect("a temporary instance is creatable");
        for name in ["data", "models", "exchange"] {
            fs::create_dir_all(directory.path().join(name))
                .expect("an instance directory is creatable");
        }
        let port = free_port();
        fs::write(
            directory.path().join("crible.toml"),
            config(directory.path(), port, timeout_ms),
        )
        .expect("crible.toml is writable");
        Self { directory, port }
    }

    fn config_path(&self) -> PathBuf {
        self.directory.path().join("crible.toml")
    }

    fn address(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.port))
    }

    /// Adds an index whose model configuration is a named pipe: the loader
    /// opens it and blocks until a writer arrives, so the startup stays in
    /// progress as long as the test needs.
    #[cfg(unix)]
    fn with_blocked_model(self) -> Self {
        let model = self.directory.path().join("models").join("m");
        fs::create_dir_all(&model).expect("the model directory is creatable");
        fifo(&model.join("model.json"));
        let index = self
            .directory
            .path()
            .join("data")
            .join("indexes")
            .join("alpha");
        fs::create_dir_all(&index).expect("the index directory is creatable");
        fs::write(index.join("index.json"), index_json("alpha"))
            .expect("the index configuration is written");
        self
    }
}

/// Configuration of an index that needs no encoder and no lexical branch.
///
/// The loader reads it before the model, so it parses without ONNX Runtime.
#[cfg(unix)]
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

/// Creates a named pipe, whose reader blocks until a writer opens it.
#[cfg(unix)]
fn fifo(path: &Path) {
    let status = Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("mkfifo must run");
    assert!(status.success(), "mkfifo must create {}", path.display());
}

/// The configuration of `docs/07`, with the paths of `root`, `port` and
/// `[limits].default_timeout_ms`.
fn config(root: &Path, port: u16, timeout_ms: u64) -> String {
    format!(
        r#"
[server]
bind = "127.0.0.1:{port}"
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
default_timeout_ms = {timeout_ms}
wait_timeout_ms = 60000
query_cache_entries = 1024
"#,
        data = root.join("data").display(),
        models = root.join("models").display(),
        exchange = root.join("exchange").display(),
    )
}

/// Reserves an ephemeral port and releases it.
///
/// The port is reused by the child at once: a race exists in principle, and no
/// test elsewhere binds it in the meantime.
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("an ephemeral port is bindable");
    listener
        .local_addr()
        .expect("the bound address is known")
        .port()
}

/// A child server, killed if the test fails before it stops it.
struct Server {
    child: Child,
}

impl Server {
    /// Starts `crible serve` on `instance`.
    fn start(instance: &Instance) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_crible"))
            .arg("serve")
            .arg("--config")
            .arg(instance.config_path())
            // A `CRIBLE_*` variable of the host would be either a valid
            // override of the fixture or an unknown one that stops the start.
            .env_clear()
            .env("RUST_LOG", "info")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("crible serve must start: {error}"));
        Self { child }
    }

    /// Waits until the process exits, with a bound.
    fn wait_for_exit(&mut self) -> std::process::ExitStatus {
        let deadline = Instant::now() + PATIENCE;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return status,
                Ok(None) if Instant::now() >= deadline => {
                    let _ = self.child.kill();
                    panic!("crible serve did not exit within {PATIENCE:?}");
                }
                Ok(None) => sleep(Duration::from_millis(50)),
                Err(error) => panic!("cannot poll crible serve: {error}"),
            }
        }
    }

    /// Reads the journal captured on stdout.
    fn journal(&mut self) -> String {
        let mut stdout = self
            .child
            .stdout
            .take()
            .expect("the journal of the child is captured");
        let mut text = String::new();
        stdout
            .read_to_string(&mut text)
            .expect("the journal of the child is readable");
        text
    }

    /// Sends `signal` to the child.
    fn signal(&mut self, signal: &str) {
        let status = Command::new("kill")
            .arg(format!("-{signal}"))
            .arg(self.child.id().to_string())
            .status()
            .unwrap_or_else(|error| panic!("kill -{signal} must run: {error}"));
        assert!(status.success(), "kill -{signal} must reach the child");
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// One reply of a probe, read over a socket.
struct Reply {
    status: u16,
    body: String,
}

/// Sends one `GET` and reads the reply, or returns `None` while the server
/// does not accept yet.
fn try_get(address: SocketAddr, path: &str) -> Option<Reply> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(200)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("the timeout is settable");
    let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .expect("the request is writable");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).expect("the reply is readable");
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("a reply has a head and a body, got {text:?}"));
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or_else(|| panic!("the status line carries a code, got {head:?}"));
    Some(Reply {
        status,
        body: body.to_owned(),
    })
}

/// Waits until `path` answers `expected`, and returns the reply.
fn wait_for(address: SocketAddr, path: &str, expected: u16) -> Reply {
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Some(reply) = try_get(address, path)
            && reply.status == expected
        {
            return reply;
        }
        if Instant::now() >= deadline {
            panic!("{path} never answered {expected} within {PATIENCE:?}");
        }
        sleep(Duration::from_millis(50));
    }
}

#[test]
fn serve_help_names_the_default_configuration_path() {
    // ADR-0097: `crible serve [--config <chemin>]`, default
    // `/etc/crible/crible.toml`.
    let output = Command::new(env!("CARGO_BIN_EXE_crible"))
        .args(["serve", "--help"])
        .output()
        .expect("crible serve --help must run");
    assert!(output.status.success(), "the help of serve must succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("/etc/crible/crible.toml"),
        "the help must name the default path, got {stdout}"
    );
    assert!(
        stdout.contains("--config"),
        "the help must name the option, got {stdout}"
    );
}

#[test]
fn serve_refuses_an_absent_configuration_file() {
    // ADR-0097: the server never starts on implicit default values.
    let directory = tempfile::tempdir().expect("a temporary directory is creatable");
    let absent = directory.path().join("absent.toml");
    let output = Command::new(env!("CARGO_BIN_EXE_crible"))
        .arg("serve")
        .arg("--config")
        .arg(&absent)
        .env_clear()
        .output()
        .expect("crible serve must run");
    assert!(
        !output.status.success(),
        "an absent configuration file must fail the startup"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid_request"),
        "the failure carries its code, got {stderr}"
    );
    assert!(
        stderr.contains("absent.toml"),
        "the failure names the file, got {stderr}"
    );
}

#[test]
fn serve_answers_the_probes_and_stops_on_sigterm() {
    // ADR-0099: `/health` and `/ready` answer from the bind and after the
    // startup; SIGTERM stops the process cleanly.
    let instance = Instance::new();
    let mut server = Server::start(&instance);

    let health = wait_for(instance.address(), "/health", 200);
    assert!(health.body.contains("\"ok\""), "got {}", health.body);
    let ready = wait_for(instance.address(), "/ready", 200);
    assert!(ready.body.contains("\"ready\""), "got {}", ready.body);

    server.signal("TERM");
    let status = server.wait_for_exit();
    assert!(status.success(), "a graceful stop exits zero, got {status}");

    let journal = server.journal();
    assert!(!journal.trim().is_empty(), "the server logs on stdout");
    for line in journal.lines() {
        let value: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|error| {
            panic!("a journal line is a JSON object, got {line:?}: {error}")
        });
        assert!(
            value.get("level").is_some(),
            "a journal line carries its level, got {line:?}"
        );
    }
}

#[test]
fn serve_refuses_a_crible_variable_that_the_key_refuses_naming_it() {
    // ADR-0097: a `CRIBLE_*` value the key cannot accept fails the startup with
    // a non-zero code, and the message names the variable, not only the key.
    // The sample rate parses as a number; the bound of `[debug_journal]`
    // rejects it, which is a refusal of the key.
    let instance = Instance::new();
    let output = Command::new(env!("CARGO_BIN_EXE_crible"))
        .arg("serve")
        .arg("--config")
        .arg(instance.config_path())
        .env_clear()
        .env("CRIBLE_DEBUG_JOURNAL__SAMPLE_RATE", "2.0")
        .output()
        .expect("crible serve must run");
    assert!(
        !output.status.success(),
        "an out-of-range override must fail the startup"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid_request"),
        "the failure carries its code, got {stderr}"
    );
    assert!(
        stderr.contains("CRIBLE_DEBUG_JOURNAL__SAMPLE_RATE"),
        "the failure names the variable, got {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn serve_stops_on_sigterm_while_the_loader_waits() {
    // The loader is a blocking task that cannot be cancelled. A graceful stop
    // must not wait for it: the drain ends and the process exits (ADR-0099).
    let instance = Instance::new().with_blocked_model();
    let mut server = Server::start(&instance);

    wait_for(instance.address(), "/health", 200);
    let ready = try_get(instance.address(), "/ready").expect("the server accepts");
    assert_eq!(
        ready.status, 503,
        "the loader is still waiting on the model"
    );

    server.signal("TERM");
    let status = server.wait_for_exit();
    assert!(
        status.success(),
        "a graceful stop exits zero even with a loader in progress, got {status}"
    );
}

#[test]
fn serve_exits_at_once_on_a_second_signal_during_the_drain() {
    // ADR-0099: the second signal exits with a non-zero code at once. A client
    // that sends half a request holds the drainage open; the bound is 61 s, so
    // only the second signal can end the process within the test.
    let instance = Instance::with_timeout(60_000);
    let mut server = Server::start(&instance);
    wait_for(instance.address(), "/ready", 200);

    let mut held = TcpStream::connect(instance.address()).expect("the server accepts");
    held.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\n")
        .expect("the partial request is writable");

    server.signal("TERM");
    sleep(Duration::from_millis(200));
    let second = Instant::now();
    server.signal("TERM");
    let status = server.wait_for_exit();
    let elapsed = second.elapsed();
    assert!(
        !status.success(),
        "a forced stop exits non-zero, got {status}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "the forced stop is immediate, got {elapsed:?}"
    );
    drop(held);
}

#[test]
fn serve_stops_on_sigint() {
    // ADR-0099: SIGINT triggers the same sequence as SIGTERM.
    let instance = Instance::new();
    let mut server = Server::start(&instance);
    wait_for(instance.address(), "/ready", 200);

    server.signal("INT");
    let status = server.wait_for_exit();
    assert!(status.success(), "SIGINT stops cleanly too, got {status}");
}
