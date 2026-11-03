// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixtures of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! `crible eval` in API mode (`EVALUER-PAR-API`, phase P8).
//!
//! References: the task sheet (`docs/phases/P8-docker-et-deploiement.md:124-146`
//! and its note of 2026-11-02), ADR-0131 (the interface: `--url`, the simple
//! search route, `CRIBLE_EVAL_API_KEY`, plain HTTP, named refusals, fixed
//! deadlines, `format_version: 2`, `source`, `latency_basis`), ADR-0125 (the
//! local mode this one must stay comparable with) and `docs/08`
//! (`INV-13`).
//!
//! | Test | Criterion / decision |
//! |---|---|
//! | `inv_13_eval_api_mode_never_writes_query_text_or_key` | no query text and no key in stdout, stderr, server journal, JSON or Markdown (INV-13, decisions 4 and 16) |
//! | `eval_api_and_local_reports_agree_on_fidelity_and_quality` | same fidelity and quality figures, server at `exact_threshold = 0` against a local `--exact-threshold 0` (acceptance criterion 1, decision 10) |
//! | `eval_api_stops_when_the_generation_moves_during_the_campaign` | an answer of another generation stops the campaign naming both (decision 10) |
//! | `eval_api_report_names_its_source_and_client_latency` | `format_version: 2`, `source.mode`/`source.url`, `latency_basis`, `total` only in API (criterion 2, decision 7) |
//! | `eval_api_names_a_refused_connection_without_panicking` | a refused connection is a named error, exit 1, no panic, no partial report (criterion 4, decision 5) |
//! | `eval_api_returns_the_401_of_a_wrong_key_without_panicking` | a non-2xx answer is a named error, exit 1, no panic (criterion 4, decision 5) |
//! | `eval_api_refuses_https_before_any_io` | `https://` refused in `invalid_request`, naming the scheme and TLS (decision 3) |
//! | `eval_api_refuses_data_and_url_together` | `--data` and `--url` are exclusive (decision 6) |
//! | `eval_api_refuses_exact_threshold_with_url` | `--exact-threshold` is refused in API mode (decision 6) |
//!
//! Every test drives the `crible` binary, and every API test starts a real
//! `crible serve` child on a port the kernel picks (`127.0.0.1:0`), waits for the
//! `"the server is bound"` line of its journal and `/ready`, and stops it on
//! SIGTERM, even when the test fails (the `Drop` of [`Server`]).
//!
//! The fixture index is `external`: the query vectors travel in the game, so
//! neither the server nor `eval` needs ONNX Runtime. The 305-passage fixture
//! makes the binary strategy lose the best five passages (the same fixture as
//! `eval_cli.rs`): its fidelity is `0.5`, not `1`, so an equality between the two
//! modes is not the equality of two vacuous figures.
//!
//! ```text
//! cargo test -p crible --test eval_api_cli
//! ```

mod common;

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crible_core::config::{ReferencePrecision, Schema};
use crible_core::ids::{Epoch, Generation, Iid};
use crible_store::generation::{GenerationChange, GenerationStore};
use crible_store::records::{DocumentRecord, PassageRecord};
use crible_store::segment::{SegmentMeta, SegmentSpec, SegmentWriter};
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::common::path;

/// Name of the fixture index.
const INDEX: &str = "fixture";

/// Name of the `external` model: no ONNX Runtime is needed to serve it.
const MODEL: &str = "m";

/// Dimensions of the fixture vectors.
const DIMS: u32 = 4;

/// Secret of the `read` key of the fixture, as `crible eval` receives it.
///
/// Long and improbable on purpose: a shorter string could collide with a digest,
/// a `job_id` or a request id of a report.
const READ_KEY: &str = "CLE-EVAL-API-4b8e2f6a9d1c0357e8a2f4b6c8d0e2f4";

/// A key the instance does not know, for the 401 of decision 5.
const WRONG_KEY: &str = "CLE-FAUSSE-EVAL-API-71d0c5a3e9b2f4d6a8c0e2b4d6f8a0c2";

/// Text of the query the confidentiality test sends, and looks for everywhere.
///
/// Long, uppercase and hyphenated: no hexadecimal digest, no `job_id` and no
/// request id can contain it by accident.
const QUERY_MARKER: &str = "MARQUEUR-REQUETE-API-7f3c9a1e5b2d8046-c1f9";

/// Schema of the fixture index: no field is needed by the queries.
const SCHEMA: &str = r#"{ "document_fields": {}, "passage_fields": {} }"#;

/// Time a child has to become ready, answer or exit before the test fails.
const PATIENCE: Duration = Duration::from_secs(20);

/// Vector of the query used by the fidelity fixture.
const FIDELITY_QUERY: [f32; 4] = [1.0, 0.1, 0.0, 0.0];

/// One temporary instance: data, models, exchange, `crible.toml` and one index.
struct Fixture {
    /// Keeps the temporary directory alive for the whole test.
    _temp: TempDir,
    /// Root of the temporary instance.
    root: PathBuf,
    /// `[paths] data`.
    data: PathBuf,
    /// `crible.toml` of the instance.
    config: PathBuf,
}

impl Fixture {
    /// Builds `entries` as generation 1 of the fixture index and writes the
    /// instance configuration around it.
    ///
    /// `rescore_candidates` and `exact_threshold` are the *preconditions of the
    /// instance* of ADR-0131, decision 9: in API mode the command cannot change
    /// them, so the local mode is given the same values on its command line.
    fn new(
        entries: &[(String, [f32; 4])],
        lexical: bool,
        rescore_candidates: u32,
        exact_threshold: u32,
    ) -> Self {
        let temp = tempfile::tempdir().expect("a temporary instance is creatable");
        let root = temp.path().to_path_buf();
        let data = root.join("data");
        let models = root.join("models");
        let exchange = root.join("exchange");
        for directory in [&data, &models.join(MODEL), &exchange] {
            fs::create_dir_all(directory)
                .unwrap_or_else(|error| panic!("{} is creatable: {error}", directory.display()));
        }
        // An `external` model: the server loads its configuration, never a graph,
        // so this test needs no ONNX Runtime (ADR-0099).
        fs::write(
            models.join(MODEL).join("model.json"),
            format!(r#"{{"name":"{MODEL}","kind":"external","dims":{DIMS},"normalize":true}}"#),
        )
        .expect("the model configuration is written");

        let config = root.join("crible.toml");
        fs::write(&config, configuration(&root)).expect("crible.toml is writable");

        let index_config = root.join("index.json");
        fs::write(
            &index_config,
            index_json(lexical, rescore_candidates, exact_threshold),
        )
        .expect("index.json is writable");

        let fixture = Self {
            _temp: temp,
            root,
            data,
            config,
        };

        let created = run_crible(
            &[
                "index",
                "create",
                "--data",
                path(&fixture.data),
                "--config",
                path(&index_config),
            ],
            None,
        );
        assert!(
            created.status.success(),
            "crible index create must succeed: {}",
            String::from_utf8_lossy(&created.stderr)
        );
        fixture.commit(entries);
        fixture
    }

    /// Directory of the fixture index.
    fn index_dir(&self) -> PathBuf {
        self.data.join("indexes").join(INDEX)
    }

    /// Writes the single segment of generation 1 and commits it.
    fn commit(&self, entries: &[(String, [f32; 4])]) {
        let directory = self.index_dir();
        let mut store = GenerationStore::open(&directory, &schema())
            .unwrap_or_else(|error| panic!("the fixture index must open: {error}"));
        let vectors: Vec<[f32; 4]> = entries.iter().map(|(_, vector)| *vector).collect();
        let segment = write_segment(&directory, &vectors);
        store
            .commit(GenerationChange {
                segments: vec![segment],
                documents: entries
                    .iter()
                    .enumerate()
                    .map(|(position, (doc_id, _))| DocumentRecord {
                        doc_id: doc_id.clone(),
                        doc_hash: format!("blake3:doc-{position}"),
                        fields: json!({}),
                        updated_generation: Generation::new(1),
                    })
                    .collect(),
                passages: entries
                    .iter()
                    .enumerate()
                    .map(|(position, (doc_id, _))| PassageRecord {
                        iid: Iid::new(u32::try_from(position).unwrap_or_default()),
                        doc_id: doc_id.clone(),
                        ord: 0,
                        text: format!("passage {position}"),
                        passage_hash: format!("blake3:passage-{position}"),
                        fields: json!({}),
                        segment_id: "seg-1".to_owned(),
                        created_generation: Generation::new(1),
                        dead_since_gen: None,
                    })
                    .collect(),
                deletions: Vec::new(),
            })
            .unwrap_or_else(|error| panic!("the fixture generation must commit: {error}"));
    }

    /// Writes a game of queries under the temporary root and returns its path.
    fn game(&self, name: &str, lines: &[Value]) -> PathBuf {
        let file = self.root.join(name);
        let body: String = lines.iter().map(|line| format!("{line}\n")).collect();
        fs::write(&file, body).expect("the game is writable");
        file
    }

    /// Creates and returns a report directory under the temporary root.
    fn out(&self, name: &str) -> PathBuf {
        let directory = self.root.join(name);
        fs::create_dir_all(&directory)
            .unwrap_or_else(|error| panic!("{} is creatable: {error}", directory.display()));
        directory
    }
}

/// Configuration of the fixture index.
fn index_json(lexical: bool, rescore_candidates: u32, exact_threshold: u32) -> String {
    format!(
        r#"{{"name":"{INDEX}","model":"{MODEL}","reference_precision":"fp16","schema":{SCHEMA},
            "cluster_by":[],"lexical":{{"enabled":{lexical},"language":"fr","positions":false}},
            "search":{{"scan":"binary","rescore_candidates":{rescore_candidates},
                      "exact_threshold":{exact_threshold},"candidates_per_branch":100,
                      "fusion":{{"method":"rrf"}}}},
            "ingest":{{"acceptance_sample":1,"acceptance_min_cosine":0.9}},
            "compaction":{{"dead_ratio":0.1}}}}"#
    )
}

/// `crible.toml` of the fixture, with one `read` key and a kernel-chosen port.
fn configuration(root: &Path) -> String {
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
name = "eval-reader"
role = "read"
sha256 = "{read}"

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
        read = crible_server::auth::sha256_hex(READ_KEY),
    )
}

/// Schema of the fixture index, parsed.
fn schema() -> Schema {
    Schema::from_json_str(SCHEMA)
        .unwrap_or_else(|error| panic!("the fixture schema must be valid: {error}"))
}

/// Writes the only segment of the fixture index and returns its metadata.
fn write_segment(index: &Path, vectors: &[[f32; 4]]) -> SegmentMeta {
    let segments = index
        .join("epochs")
        .join(Epoch::new(1).get().to_string())
        .join("segments");
    fs::create_dir_all(&segments)
        .unwrap_or_else(|error| panic!("the segments directory must be creatable: {error}"));
    let mut writer = SegmentWriter::create(
        &segments,
        SegmentSpec {
            segment_id: "seg-1".to_owned(),
            iid_start: Iid::new(0),
            dims: DIMS,
            generation: Generation::new(1),
            batch_id: "batch-eval-api".to_owned(),
            precision: ReferencePrecision::Fp16,
        },
    )
    .unwrap_or_else(|error| panic!("the segment writer must start: {error}"));
    for vector in vectors {
        writer
            .append(vector)
            .unwrap_or_else(|error| panic!("the fixture vector must be appended: {error}"));
    }
    writer
        .finish()
        .unwrap_or_else(|error| panic!("the fixture segment must be finished: {error}"))
}

/// Three documents, one passage each: enough for a report, cheap for a server.
fn small_entries() -> Vec<(String, [f32; 4])> {
    vec![
        ("A".to_owned(), [1.0, 0.0, 0.0, 0.0]),
        ("B".to_owned(), [0.9, 0.1, 0.0, 0.0]),
        ("C".to_owned(), [0.5, 0.5, 0.0, 0.0]),
    ]
}

/// 305 passages whose binary scan drops the best cosine (the fixture of
/// `eval_cli.rs`): the exact top-10 holds the five `W` passages and the best
/// five `O` ones, the binary top-10, under `N = 100`, holds ten `O` passages.
///
/// The fidelity is therefore `5/10 = 0,5` in both modes, and an equality of the
/// two reports is a real equality.
fn fidelity_entries() -> Vec<(String, [f32; 4])> {
    let mut entries = Vec::new();
    for position in 0..100 {
        let offset = position as f32 * 0.0001;
        entries.push((format!("O{position}"), [0.05 + offset, 1.0, 0.0, 0.0]));
    }
    for position in 0..5 {
        entries.push((format!("W{position}"), [1.0, -0.05, 0.02, 0.0]));
    }
    for position in 0..200 {
        entries.push((format!("F{position}"), [-1.0, 0.5, 0.0, 0.0]));
    }
    entries
}

/// The two queries of the fidelity fixture.
fn fidelity_game(fixture: &Fixture) -> PathBuf {
    fixture.game(
        "queries.jsonl",
        &[
            json!({ "id": "q1", "vector": FIDELITY_QUERY, "expected": ["W0"] }),
            json!({ "id": "q2", "vector": FIDELITY_QUERY, "expected": ["O99"] }),
        ],
    )
}

/// Runs the `crible` binary with a closed environment, optionally carrying the
/// `read` key of the fixture.
///
/// The environment is closed so a `CRIBLE_*` variable of the host can neither
/// change a fixture nor be read as a key.
fn run_crible(arguments: &[&str], api_key: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_crible"));
    command.args(arguments).env_clear().env("RUST_LOG", "warn");
    if let Some(key) = api_key {
        command.env("CRIBLE_EVAL_API_KEY", key);
    }
    command
        .output()
        .unwrap_or_else(|error| panic!("cannot run crible {arguments:?}: {error}"))
}

/// Runs `crible eval` in local mode on `game`, into `out`.
fn local_eval(fixture: &Fixture, game: &Path, out: &Path, extra: &[&str]) -> Output {
    let mut arguments: Vec<String> = vec![
        "eval".to_owned(),
        "--data".to_owned(),
        path(&fixture.data).to_owned(),
        "--index".to_owned(),
        INDEX.to_owned(),
        "--queries".to_owned(),
        path(game).to_owned(),
        "--out".to_owned(),
        path(out).to_owned(),
    ];
    arguments.extend(extra.iter().map(|argument| (*argument).to_owned()));
    let borrowed: Vec<&str> = arguments.iter().map(String::as_str).collect();
    run_crible(&borrowed, None)
}

/// Runs `crible eval` in API mode on `game`, into `out`.
fn api_eval(game: &Path, out: &Path, url: &str, key: Option<&str>, extra: &[&str]) -> Output {
    let mut arguments: Vec<String> = vec![
        "eval".to_owned(),
        "--index".to_owned(),
        INDEX.to_owned(),
        "--queries".to_owned(),
        path(game).to_owned(),
        "--out".to_owned(),
        path(out).to_owned(),
        "--url".to_owned(),
        url.to_owned(),
    ];
    arguments.extend(extra.iter().map(|argument| (*argument).to_owned()));
    let borrowed: Vec<&str> = arguments.iter().map(String::as_str).collect();
    run_crible(&borrowed, key)
}

/// The two report paths the command prints, in order.
fn reports(output: &Output) -> (PathBuf, PathBuf) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut json_path = None;
    let mut markdown_path = None;
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("json: ") {
            json_path = Some(PathBuf::from(rest));
        }
        if let Some(rest) = line.strip_prefix("markdown: ") {
            markdown_path = Some(PathBuf::from(rest));
        }
    }
    (
        json_path.unwrap_or_else(|| panic!("eval prints its JSON path: {stdout}")),
        markdown_path.unwrap_or_else(|| panic!("eval prints its Markdown path: {stdout}")),
    )
}

/// Reads a JSON report.
fn read_report(path: &Path) -> Value {
    let raw = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("{} must be valid JSON: {error}", path.display()))
}

/// Files written into `directory`, sorted; an absent directory holds none.
fn written_files(directory: &Path) -> Vec<PathBuf> {
    let Ok(listing) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = listing
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("an entry is readable: {error}"))
                .path()
        })
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    files
}

/// Asserts a failed command wrote no report.
fn assert_no_partial_report(out: &Path) {
    let written = written_files(out);
    assert!(
        written.is_empty(),
        "a failed campaign writes no partial report (ADR-0131, decision 5), got {written:?}"
    );
}

/// Asserts the failure of `output` is a named error with exit code 1 and no
/// panic, and returns its stderr.
fn named_failure(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        !output.status.success(),
        "the command must fail, got success with stderr {stderr:?}"
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "a named error exits 1 (ADR-0131, decision 5), got {} with stderr {stderr:?}",
        output.status
    );
    assert!(
        !stderr.contains("panicked"),
        "a failure is an error, never a panic, got {stderr:?}"
    );
    stderr
}

/// A real `crible serve` child, stopped on SIGTERM and killed if the test fails
/// before that.
struct Server {
    /// The child, taken by [`Server::stop`] so `Drop` does not kill it twice.
    child: Option<Child>,
    /// Its stderr, read only when the startup fails.
    stderr: Option<ChildStderr>,
    /// Address the child bound, read from its journal.
    address: SocketAddr,
    /// Lines of the journal, filled as the child writes them.
    journal: Arc<Mutex<Vec<String>>>,
    /// Thread reading the stdout of the child.
    reader: Option<JoinHandle<()>>,
    /// Thread moving the remaining journal lines into `journal`.
    drain: Option<JoinHandle<()>>,
}

impl Server {
    /// Starts `crible serve` on `fixture` and waits for the address it bound.
    ///
    /// The configuration binds `127.0.0.1:0`, so the kernel picks a port the
    /// child keeps: the test never reserves an address it cannot hand over. The
    /// address is read from the `"the server is bound"` line of the journal
    /// (ADR-0099).
    fn start(fixture: &Fixture) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_crible"))
            .arg("serve")
            .arg("--config")
            .arg(&fixture.config)
            .env_clear()
            .env("RUST_LOG", "info")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("crible serve must start: {error}"));
        let stdout = child
            .stdout
            .take()
            .expect("the journal of the child is captured");
        let mut stderr = child
            .stderr
            .take()
            .expect("the errors of the child are captured");

        let (sender, receiver) = mpsc::channel::<String>();
        let reader = thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });

        let deadline = Instant::now() + PATIENCE;
        let mut lines = Vec::new();
        let address = loop {
            match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(line) => {
                    let bound = bound_address(&line);
                    lines.push(line);
                    if let Some(address) = bound {
                        break address;
                    }
                }
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let mut errors = String::new();
                    let _ = stderr.read_to_string(&mut errors);
                    panic!(
                        "crible serve must announce the address it bound within {PATIENCE:?}; \
                         journal {lines:?}, stderr {errors:?}"
                    );
                }
            }
        };

        let journal = Arc::new(Mutex::new(lines));
        let buffer = Arc::clone(&journal);
        let drain = thread::spawn(move || {
            while let Ok(line) = receiver.recv() {
                buffer
                    .lock()
                    .expect("the journal buffer is not poisoned")
                    .push(line);
            }
        });

        Self {
            child: Some(child),
            stderr: Some(stderr),
            address,
            journal,
            reader: Some(reader),
            drain: Some(drain),
        }
    }

    /// The address the child bound.
    fn address(&self) -> SocketAddr {
        self.address
    }

    /// Waits until `/ready` answers 200: the index is loaded and served.
    fn wait_ready(&self) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if ready(self.address) {
                return;
            }
            if Instant::now() >= deadline {
                panic!("/ready never answered 200 within {PATIENCE:?}");
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// The journal of the child, read so far.
    fn journal(&self) -> String {
        self.journal
            .lock()
            .expect("the journal buffer is not poisoned")
            .join("\n")
    }

    /// Stops the child on SIGTERM and waits for its exit, bounded by [`PATIENCE`].
    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let signalled = Command::new("kill")
                .arg("-TERM")
                .arg(child.id().to_string())
                .status()
                .unwrap_or_else(|error| panic!("kill -TERM must run: {error}"));
            assert!(signalled.success(), "kill -TERM must reach the child");
            let deadline = Instant::now() + PATIENCE;
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() >= deadline => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                    Ok(None) => thread::sleep(Duration::from_millis(20)),
                    Err(error) => panic!("cannot poll crible serve: {error}"),
                }
            }
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(drain) = self.drain.take() {
            let _ = drain.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(drain) = self.drain.take() {
            let _ = drain.join();
        }
        let _ = self.stderr.take();
    }
}

/// Address carried by the `"the server is bound"` journal line, if `line` is it.
fn bound_address(line: &str) -> Option<SocketAddr> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value["fields"]["message"] != "the server is bound" {
        return None;
    }
    value["fields"]["address"].as_str()?.parse().ok()
}

/// Sends `GET /ready` and returns true when it answers 200.
fn ready(address: SocketAddr) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(200)) else {
        return false;
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("the timeout is settable");
    let request = "GET /ready HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut raw = Vec::new();
    if stream.read_to_end(&mut raw).is_err() {
        return false;
    }
    let text = String::from_utf8_lossy(&raw);
    let Some((head, _)) = text.split_once("\r\n\r\n") else {
        return false;
    };
    head.lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        == Some(200)
}

/// The URL of a child, as `--url` takes it.
fn url_of(server: &Server) -> String {
    format!("http://{}", server.address())
}

/// A tiny HTTP/1.1 instance that answers two generations of the fixture index.
///
/// `crible eval` runs in its own process and prints nothing between two
/// requests, so a real `crible serve` cannot be moved at a known point of the
/// campaign without a race. This server instead answers the campaign in order:
/// the state route and the first search pin generation 1, and every later
/// search answers generation 2. The switch is therefore deterministic, with no
/// sleep and no retry.
struct TwoGenerationInstance {
    /// Address the listener bound.
    address: SocketAddr,
    /// Number of search answers served so far.
    searches: Arc<Mutex<u64>>,
    /// Set by `Drop` to stop the accept loop.
    stop: Arc<AtomicBool>,
    /// Thread serving the connections, joined by `Drop`.
    thread: Option<JoinHandle<()>>,
}

impl TwoGenerationInstance {
    /// Binds an ephemeral port and starts serving on a thread.
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .unwrap_or_else(|error| panic!("the test instance must bind: {error}"));
        let address = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("the test instance must know its address: {error}"));
        let searches = Arc::new(Mutex::new(0_u64));
        let stop = Arc::new(AtomicBool::new(false));
        let counter = Arc::clone(&searches);
        let flag = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            for stream in listener.incoming() {
                if flag.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                answer_one(stream, &counter);
            }
        });
        Self {
            address,
            searches,
            stop,
            thread: Some(thread),
        }
    }

    /// The base, as `--url` takes it.
    fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// Number of search answers served so far.
    fn searches(&self) -> u64 {
        *self
            .searches
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Drop for TwoGenerationInstance {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wakes the blocked `accept`: the loop checks the flag and exits.
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Reads one HTTP/1.1 request and answers it with one JSON body.
///
/// The state route always names generation 1; the first search answer names
/// generation 1 too, and every later one generation 2.
fn answer_one(stream: TcpStream, searches: &Arc<Mutex<u64>>) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.is_empty() {
        return;
    }
    let mut content_length = 0_usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            return;
        }
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0_u8; content_length];
    if reader.read_exact(&mut body).is_err() {
        return;
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    let payload = if method == "GET" && path.starts_with("/v1/indexes/") {
        json!({
            "state": { "generation": 1, "live_passages": 3 },
            "config": { "search": { "exact_threshold": 10, "rescore_candidates": 100 } }
        })
        .to_string()
    } else if method == "POST" && path.ends_with("/search") {
        let mut counter = searches
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *counter += 1;
        let generation = if *counter == 1 { 1 } else { 2 };
        json!({
            "generation": generation,
            "pool_size": 3,
            "results": [{ "key": "A#0", "doc_id": "A" }]
        })
        .to_string()
    } else {
        "{}".to_owned()
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{payload}",
        payload.len()
    );
    let _ = writer.write_all(response.as_bytes());
    let _ = writer.flush();
}

// --- INV-13: neither the query text nor the key leaves a trace ---

#[test]
fn inv_13_eval_api_mode_never_writes_query_text_or_key() {
    let _serial = common::SerialGuard::acquire();
    // INV-13: "les analytics ne contiennent ni texte de requête…", and
    // ADR-0131, decisions 4 and 16, extend it to the journal and to both
    // reports. A lexical query carries text, which the API mode sends in the
    // body of the request: that text must appear nowhere the test can read, and
    // neither must the `read` key.
    let fixture = Fixture::new(&small_entries(), true, 5_000, 0);
    let game = fixture.game(
        "queries.jsonl",
        &[json!({ "id": "q-marqueur", "query": QUERY_MARKER, "expected": ["A"] })],
    );
    let out = fixture.out("out-api");

    let server = Server::start(&fixture);
    server.wait_ready();
    let output = api_eval(
        &game,
        &out,
        &url_of(&server),
        Some(READ_KEY),
        &["--modes", "lexical"],
    );
    assert!(
        output.status.success(),
        "the API campaign must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let (json_path, markdown_path) = reports(&output);
    let json_text = fs::read_to_string(&json_path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", json_path.display()));
    let markdown_text = fs::read_to_string(&markdown_path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", markdown_path.display()));
    let journal = server.journal();

    // The campaign really ran the marked query: the report names its id, so an
    // empty campaign cannot pass this test.
    let report = read_report(&json_path);
    assert_eq!(
        report["modes"]["lexical"]["queries"][0]["id"], "q-marqueur",
        "the marked query must be in the report: {report}"
    );

    let written = format!(
        "{}\n{}\n{}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        json_text,
        markdown_text,
        journal
    );
    assert!(
        !written.contains(QUERY_MARKER),
        "no query text may reach stdout, stderr, the server journal or a report (INV-13)"
    );
    assert!(
        !written.contains(READ_KEY),
        "no key may reach stdout, stderr, the server journal or a report (ADR-0032, INV-13)"
    );
}

// --- The same figures in both modes ---

#[test]
fn eval_api_and_local_reports_agree_on_fidelity_and_quality() {
    let _serial = common::SerialGuard::acquire();
    // Acceptance criterion 1 and ADR-0131, decision 10: the server is configured
    // `exact_threshold = 0` and the local campaign runs `--exact-threshold 0`,
    // so both take the binary strategy on the same 305-passage fixture.
    //
    // The local campaign runs first and stops: the server then takes the writer
    // lock of the index (`VERROU-ECRIVAIN-PAR-INDEX`), so no two processes write
    // the same index.
    let fixture = Fixture::new(&fidelity_entries(), false, 100, 0);
    let game = fidelity_game(&fixture);

    let local_out = fixture.out("out-local");
    let local = local_eval(
        &fixture,
        &game,
        &local_out,
        &["--exact-threshold", "0", "--rescore-candidates", "100"],
    );
    assert!(
        local.status.success(),
        "the local campaign must succeed: {}",
        String::from_utf8_lossy(&local.stderr)
    );
    let (local_json, _) = reports(&local);
    let local_report = read_report(&local_json);

    let server = Server::start(&fixture);
    server.wait_ready();
    let api_out = fixture.out("out-api");
    let api = api_eval(&game, &api_out, &url_of(&server), Some(READ_KEY), &[]);
    assert!(
        api.status.success(),
        "the API campaign must succeed: {}",
        String::from_utf8_lossy(&api.stderr)
    );
    let (api_json, _) = reports(&api);
    let api_report = read_report(&api_json);

    let local_mode = &local_report["modes"]["dense"];
    let api_mode = &api_report["modes"]["dense"];

    // Non-vacuous premises: the fixture really separates the binary strategy
    // from `exact`, and both modes really measured the two queries.
    assert_eq!(
        local_mode["fidelity"]["n"], 2,
        "the local campaign must measure both queries: {local_report}"
    );
    assert_eq!(
        local_mode["fidelity"]["recall_at_10"].as_f64(),
        Some(0.5),
        "the binary strategy must lose the five best passages: {local_report}"
    );
    assert_eq!(
        local_mode["quality"]["recall_at_k"].as_f64(),
        Some(0.5),
        "the grouped run must miss one expected document: {local_report}"
    );

    assert_eq!(
        api_mode["fidelity"], local_mode["fidelity"],
        "the API and the local reports must carry the same fidelity figures \
         (criterion 1, decision 10)\nlocal: {local_report}\napi: {api_report}"
    );
    assert_eq!(
        api_mode["quality"], local_mode["quality"],
        "the API and the local reports must carry the same quality figures \
         (criterion 1, decision 10)\nlocal: {local_report}\napi: {api_report}"
    );
}

// --- The report names its source and its latency basis ---

#[test]
fn eval_api_report_names_its_source_and_client_latency() {
    let _serial = common::SerialGuard::acquire();
    // Criterion 2 and ADR-0131, decision 7: `format_version: 2`, a `source`
    // member, `latency_basis` per mode, and in API mode a `latency` that carries
    // `total` only.
    let fixture = Fixture::new(&small_entries(), false, 5_000, 0);
    let game = fixture.game(
        "queries.jsonl",
        &[json!({ "id": "q1", "vector": [1.0, 0.0, 0.0, 0.0], "expected": ["A"] })],
    );

    let local_out = fixture.out("out-local");
    let local = local_eval(&fixture, &game, &local_out, &["--exact-threshold", "0"]);
    assert!(
        local.status.success(),
        "the local campaign must succeed: {}",
        String::from_utf8_lossy(&local.stderr)
    );
    let (local_json, _) = reports(&local);
    let local_report = read_report(&local_json);

    let server = Server::start(&fixture);
    server.wait_ready();
    let api_out = fixture.out("out-api");
    let url = url_of(&server);
    let api = api_eval(&game, &api_out, &url, Some(READ_KEY), &[]);
    assert!(
        api.status.success(),
        "the API campaign must succeed: {}",
        String::from_utf8_lossy(&api.stderr)
    );
    let (api_json, _) = reports(&api);
    let api_report = read_report(&api_json);

    assert_eq!(
        local_report["format_version"], 2,
        "the local report passes to format_version 2 too: {local_report}"
    );
    assert_eq!(
        local_report["source"]["mode"], "local",
        "the local report names its mode: {local_report}"
    );
    assert_eq!(
        local_report["modes"]["dense"]["latency"]["latency_basis"], "server_stages",
        "the local latency is measured on the server stages: {local_report}"
    );

    assert_eq!(
        api_report["format_version"], 2,
        "the API report carries format_version 2: {api_report}"
    );
    assert_eq!(
        api_report["source"]["mode"], "api",
        "the API report names its mode: {api_report}"
    );
    let source_url = api_report["source"]["url"]
        .as_str()
        .unwrap_or_else(|| panic!("the API report carries source.url: {api_report}"));
    assert!(
        source_url.starts_with("http://"),
        "source.url is the base the command received, got {source_url:?}"
    );
    assert!(
        source_url.contains(&server.address().to_string()),
        "source.url names the instance, got {source_url:?}"
    );
    assert!(
        !source_url.contains(READ_KEY),
        "source.url never carries the key, got {source_url:?}"
    );

    let api_latency = api_report["modes"]["dense"]["latency"]
        .as_object()
        .unwrap_or_else(|| panic!("the API report carries a latency object: {api_report}"));
    assert_eq!(
        api_latency["latency_basis"], "client_round_trip",
        "the API latency is the client round trip: {api_report}"
    );
    let mut api_keys: Vec<&str> = api_latency.keys().map(String::as_str).collect();
    api_keys.sort_unstable();
    assert_eq!(
        api_keys,
        vec!["latency_basis", "total"],
        "the API latency carries `total` only (decision 7): {api_report}"
    );
    for percentile in ["p50", "p95", "p99"] {
        assert!(
            api_latency["total"][percentile].is_number(),
            "the API total carries {percentile}: {api_report}"
        );
    }

    let local_latency = local_report["modes"]["dense"]["latency"]
        .as_object()
        .unwrap_or_else(|| panic!("the local report carries a latency object: {local_report}"));
    assert!(
        local_latency.contains_key("validate"),
        "the local report keeps the per-stage latencies: {local_report}"
    );
    assert!(
        local_latency.len() > 2,
        "the local latency keeps its stages beside `latency_basis` and `total`: {local_report}"
    );
}

// --- The generation is pinned for the whole campaign ---

#[test]
fn eval_api_stops_when_the_generation_moves_during_the_campaign() {
    let _serial = common::SerialGuard::acquire();
    // ADR-0131, decision 10: the generation is pinned by the first state read,
    // and an answer of another generation stops the campaign naming both. The
    // test instance answers generation 1 to the first search and generation 2
    // to every later one, in the order of the campaign: the switch is
    // deterministic, with no sleep and no retry.
    let fixture = Fixture::new(&small_entries(), false, 5_000, 0);
    let game = fixture.game(
        "queries.jsonl",
        &[
            json!({ "id": "q1", "vector": [1.0, 0.0, 0.0, 0.0] }),
            json!({ "id": "q2", "vector": [0.9, 0.1, 0.0, 0.0] }),
        ],
    );
    let out = fixture.out("out-api");
    let instance = TwoGenerationInstance::start();

    let output = api_eval(&game, &out, &instance.url(), Some(READ_KEY), &[]);
    let stderr = named_failure(&output);
    assert!(
        stderr.contains("internal"),
        "a campaign that spans two generations is `internal` (decision 10), got {stderr:?}"
    );
    assert!(
        stderr.contains("pinned generation 1")
            && stderr.contains("answered generation 2")
            && stderr.contains("the index moved during the campaign"),
        "the stop names both generations and the cause (decision 10), got {stderr:?}"
    );
    assert!(
        !stderr.contains(READ_KEY),
        "the failure never carries the key, got {stderr:?}"
    );
    assert!(
        instance.searches() >= 2,
        "the campaign sent at least two searches, so generation 2 was a real answer, got {}",
        instance.searches()
    );
    assert_no_partial_report(&out);
}

// --- A refused connection is a named error ---

#[test]
fn eval_api_names_a_refused_connection_without_panicking() {
    let _serial = common::SerialGuard::acquire();
    // Criterion 4 and ADR-0131, decision 5: a transport error is `internal`,
    // exits 1, names the URL without the key, and writes no partial report.
    //
    // The address is a port the fixture server bound and then released. Another
    // process could take it in the window; the assertion on the message (the
    // URL and the cause) would then fail honestly rather than pass.
    let fixture = Fixture::new(&small_entries(), false, 5_000, 0);
    let game = fixture.game(
        "queries.jsonl",
        &[json!({ "id": "q1", "vector": [1.0, 0.0, 0.0, 0.0] })],
    );
    let out = fixture.out("out-api");

    let mut server = Server::start(&fixture);
    server.wait_ready();
    let url = url_of(&server);
    server.stop();

    let output = api_eval(&game, &out, &url, Some(READ_KEY), &[]);
    let stderr = named_failure(&output);
    assert!(
        stderr.contains("internal"),
        "a transport error is `internal` (decision 5), got {stderr:?}"
    );
    assert!(
        stderr.contains(&server.address().to_string()),
        "the failure names the URL, without the key, got {stderr:?}"
    );
    assert!(
        stderr.contains("query 'q1'"),
        "the failure names the id of the query that could not run (decision 5), got {stderr:?}"
    );
    assert!(
        !stderr.contains(READ_KEY),
        "the failure never carries the key, got {stderr:?}"
    );
    assert_no_partial_report(&out);
}

// --- A non-2xx answer is a named error ---

#[test]
fn eval_api_returns_the_401_of_a_wrong_key_without_panicking() {
    let _serial = common::SerialGuard::acquire();
    // Criterion 4 and ADR-0131, decision 5: the status code and the message of
    // the server body are resumed, with a non-zero exit code and no panic.
    let fixture = Fixture::new(&small_entries(), false, 5_000, 0);
    let game = fixture.game(
        "queries.jsonl",
        &[json!({ "id": "q1", "vector": [1.0, 0.0, 0.0, 0.0] })],
    );
    let out = fixture.out("out-api");

    let server = Server::start(&fixture);
    server.wait_ready();
    let output = api_eval(&game, &out, &url_of(&server), Some(WRONG_KEY), &[]);

    let stderr = named_failure(&output);
    assert!(
        stderr.contains("unauthorized"),
        "the 401 of the server is resumed by its code, got {stderr:?}"
    );
    assert!(
        stderr.contains("query 'q1'"),
        "the failure names the id of the query that could not run (decision 5), got {stderr:?}"
    );
    assert!(
        !stderr.contains(WRONG_KEY),
        "the failure never echoes the key, got {stderr:?}"
    );
    assert_no_partial_report(&out);
}

// --- The key comes from the environment only ---

#[test]
fn eval_api_refuses_a_missing_or_empty_key_before_any_io() {
    let _serial = common::SerialGuard::acquire();
    // ADR-0131, decision 4: "une variable absente ou vide rend une erreur claire
    // qui nomme `CRIBLE_EVAL_API_KEY`, sans valeur". The key has no option and
    // no file, and the port nothing listens on proves the refusal comes before
    // the first connection.
    let fixture = Fixture::new(&small_entries(), false, 5_000, 0);
    let game = fixture.game(
        "queries.jsonl",
        &[json!({ "id": "q1", "vector": [1.0, 0.0, 0.0, 0.0] })],
    );
    let out = fixture.out("out-api");

    for (case, key) in [("absent", None), ("empty", Some(""))] {
        let output = api_eval(&game, &out, "http://127.0.0.1:1", key, &[]);
        let stderr = named_failure(&output);
        assert!(
            stderr.contains("invalid_request"),
            "a missing or empty key is `invalid_request` ({case}), got {stderr:?}"
        );
        assert!(
            stderr.contains("CRIBLE_EVAL_API_KEY"),
            "the refusal names the variable ({case}), got {stderr:?}"
        );
        assert!(
            !stderr.contains("connection refused"),
            "the refusal happens before any I/O ({case}), got {stderr:?}"
        );
        assert_no_partial_report(&out);
    }
}

// --- A baseline of the previous format is still read ---

#[test]
fn eval_api_baseline_reads_format_version_1_and_names_uncomparable_latencies() {
    let _serial = common::SerialGuard::acquire();
    // ADR-0131, decision 7: `--baseline` keeps reading a `format_version: 1`
    // report, whose latencies are server stages. The API campaign does not
    // subtract them from its client round trips: it records
    // `latency_not_compared` instead of inventing a latency comparison.
    let fixture = Fixture::new(&small_entries(), false, 5_000, 0);
    let game = fixture.game(
        "queries.jsonl",
        &[json!({ "id": "q1", "vector": [1.0, 0.0, 0.0, 0.0] })],
    );
    let out = fixture.out("out-api");
    let baseline = fixture.root.join("baseline-v1.json");
    let reference = json!({
        "format_version": 1,
        "k": 10,
        "modes": {
            "dense": {
                "quality": { "n": 1 },
                "latency": {
                    "validate": { "p50": 1.0, "p95": 1.0, "p99": 1.0 },
                },
            },
        },
    });
    fs::write(
        &baseline,
        serde_json::to_string_pretty(&reference).expect("the reference is JSON"),
    )
    .expect("the reference is written");

    let server = Server::start(&fixture);
    server.wait_ready();
    let output = api_eval(
        &game,
        &out,
        &url_of(&server),
        Some(READ_KEY),
        &["--baseline", path(&baseline)],
    );
    assert!(
        output.status.success(),
        "a version 1 baseline is read: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let (json_path, _) = reports(&output);
    let report = read_report(&json_path);
    assert_eq!(report["format_version"], 2);
    assert_eq!(report["baseline"]["regression"], false);
    let mode = &report["baseline"]["modes"]["dense"];
    assert!(
        mode.get("latency").is_none(),
        "two bases are never subtracted: {report}"
    );
    assert_eq!(
        mode["latency_not_compared"]["current_basis"], "client_round_trip",
        "the report names the basis of the campaign: {report}"
    );
    assert!(
        mode["latency_not_compared"]
            .get("reference_basis")
            .is_none(),
        "a version 1 report carries no basis: {report}"
    );
}

// --- Refusals that happen before any I/O ---

#[test]
fn eval_api_refuses_https_before_any_io() {
    let _serial = common::SerialGuard::acquire();
    // ADR-0131, decision 3: `https://` is refused in `invalid_request`, with a
    // message that names the scheme and says TLS would need an ADR. The port is
    // one nothing listens on: a message about a refused connection would prove
    // the client tried to connect first, which the decision forbids.
    let fixture = Fixture::new(&small_entries(), false, 5_000, 0);
    let game = fixture.game(
        "queries.jsonl",
        &[json!({ "id": "q1", "vector": [1.0, 0.0, 0.0, 0.0] })],
    );
    let out = fixture.out("out-api");

    let output = api_eval(&game, &out, "https://127.0.0.1:1", Some(READ_KEY), &[]);
    let stderr = named_failure(&output);
    assert!(
        stderr.contains("invalid_request"),
        "an `https://` URL is `invalid_request` (decision 3), got {stderr:?}"
    );
    assert!(
        stderr.contains("https"),
        "the refusal names the scheme, got {stderr:?}"
    );
    assert!(
        stderr.to_lowercase().contains("tls"),
        "the refusal says TLS would need an ADR, got {stderr:?}"
    );
    assert!(
        !stderr.contains("connection refused"),
        "the refusal happens before any I/O, got {stderr:?}"
    );
    assert_no_partial_report(&out);
}

#[test]
fn eval_api_refuses_data_and_url_together() {
    let _serial = common::SerialGuard::acquire();
    // ADR-0131, decision 6: `--data` and `--url` are exclusive; the refusal
    // names both options.
    let fixture = Fixture::new(&small_entries(), false, 5_000, 0);
    let game = fixture.game(
        "queries.jsonl",
        &[json!({ "id": "q1", "vector": [1.0, 0.0, 0.0, 0.0] })],
    );
    let out = fixture.out("out-api");

    let output = run_crible(
        &[
            "eval",
            "--data",
            path(&fixture.data),
            "--index",
            INDEX,
            "--queries",
            path(&game),
            "--out",
            path(&out),
            "--url",
            "http://127.0.0.1:1",
        ],
        Some(READ_KEY),
    );
    let stderr = named_failure(&output);
    assert!(
        stderr.contains("invalid_request"),
        "the exclusivity of `--data` and `--url` is `invalid_request`, got {stderr:?}"
    );
    assert!(
        stderr.contains("--data") && stderr.contains("--url"),
        "the refusal names both options (decision 6), got {stderr:?}"
    );
    assert_no_partial_report(&out);
}

#[test]
fn eval_api_refuses_exact_threshold_with_url() {
    let _serial = common::SerialGuard::acquire();
    // ADR-0131, decisions 6 and 9: `--exact-threshold` is a precondition of the
    // instance in API mode; the refusal names the option.
    let fixture = Fixture::new(&small_entries(), false, 5_000, 0);
    let game = fixture.game(
        "queries.jsonl",
        &[json!({ "id": "q1", "vector": [1.0, 0.0, 0.0, 0.0] })],
    );
    let out = fixture.out("out-api");

    let output = api_eval(
        &game,
        &out,
        "http://127.0.0.1:1",
        Some(READ_KEY),
        &["--exact-threshold", "0"],
    );
    let stderr = named_failure(&output);
    assert!(
        stderr.contains("invalid_request"),
        "the refusal of `--exact-threshold` is `invalid_request`, got {stderr:?}"
    );
    assert!(
        stderr.contains("--exact-threshold"),
        "the refusal names the option (decision 6), got {stderr:?}"
    );
    assert_no_partial_report(&out);
}
