// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixtures of this file are helpers.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

//! `VERROU-ECRIVAIN-PAR-INDEX` (phase P8): one process writes a given index, a
//! concurrent write command is refused at once in `overloaded`, and the lock is
//! free as soon as the process that held it dies.
//!
//! References: the task sheet (`docs/phases/P8-docker-et-deploiement.md`),
//! ADR-0116 (decision 2: `WRITER_LOCK`, taken by `File::try_lock`, exclusive and
//! non-blocking, with the identity check of ADR-0056), ADR-0056 (presence lock
//! and identity check), ADR-0101 (a message never carries a server path),
//! `docs/02-architecture.md` ("Organisation sur disque", "Concurrence"),
//! `docs/05-api.md` (code `overloaded`), `docs/08-invariants-et-tests.md`
//! (INV-04).
//!
//! # What the invariant means here
//!
//! INV-04 says the state after a restart is the last committed generation, and
//! everything else is rebuilt or cleaned. The writer lock leans on the kernel:
//! a lock taken by `File::try_lock` is released when the process dies, however
//! it dies, so a leftover `WRITER_LOCK` file never blocks the next writer. The
//! tests prove that at the binary boundary: a real `crible serve` is started on
//! a real index, killed by `SIGKILL`, and the next write command of the CLI
//! must pass and commit from the last committed generation.
//!
//! The lock file is also not a residue the recovery cleans: it holds no data,
//! it belongs to the index directory (ADR-0116, "Organisation sur disque"), and
//! it never appears in a snapshot, in `snapshot.json` or in an export.
//!
//! The criterion "a `WRITER_LOCK` recreated between the open and the lock does
//! not give two writers" is proved by the unit tests of the lock itself
//! (`crates/crible-store/src/file_lock.rs`).
//!
//! ```text
//! cargo test -p crible --test verrou_ecrivain_par_index
//! ```

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::Value;
use tempfile::TempDir;

/// Name of the index every test serves.
const INDEX: &str = "alpha";

/// Name of the `external` model: no ONNX Runtime is needed to serve it.
const MODEL: &str = "m";

/// Dimensionality of the fixture vectors.
const DIMS: usize = 4;

/// Time a child has to become ready or to exit before the test fails.
const PATIENCE: Duration = Duration::from_secs(20);

/// Name of the writer lock of one index (ADR-0116, decision 2).
const WRITER_LOCK: &str = "WRITER_LOCK";

/// One temporary instance: a data root, a model, an exchange root, one index
/// configuration, one online document and one query vector.
struct Fixture {
    /// Keeps the temporary directory alive for the whole test.
    _temp: TempDir,
    /// Root of the temporary instance.
    root: PathBuf,
    /// `[paths] data`.
    data: PathBuf,
    /// `[paths] models`.
    models: PathBuf,
    /// `[paths] exchange`.
    exchange: PathBuf,
    /// `crible.toml` of the instance.
    config: PathBuf,
    /// `index.json` of the fixture index.
    index_config: PathBuf,
    /// `documents.jsonl` holding one document of one passage.
    documents: PathBuf,
    /// JSON array of four floats, for `crible search --vector-file`.
    vector: PathBuf,
}

impl Fixture {
    /// Writes the directories, the model, the configuration and the inputs.
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("a temporary instance is creatable");
        let root = temp.path().to_path_buf();
        let data = root.join("data");
        let models = root.join("models");
        let exchange = root.join("exchange");
        for directory in [
            &data,
            &models.join(MODEL),
            &exchange.join("inbox"),
            &exchange.join("outbox"),
        ] {
            fs::create_dir_all(directory)
                .unwrap_or_else(|error| panic!("{} is creatable: {error}", directory.display()));
        }
        // An `external` model: the server loads its configuration, never a
        // graph, so this test needs no ONNX Runtime (ADR-0099).
        fs::write(
            models.join(MODEL).join("model.json"),
            format!(r#"{{"name":"{MODEL}","kind":"external","dims":{DIMS},"normalize":true}}"#),
        )
        .expect("the model configuration is written");

        let config = root.join("crible.toml");
        fs::write(&config, configuration(&root)).expect("crible.toml is writable");

        let index_config = root.join("index.json");
        fs::write(&index_config, index_json()).expect("index.json is writable");

        let documents = root.join("documents.jsonl");
        fs::write(
            &documents,
            format!(
                "{{\"doc_id\":\"DOC-1\",\"passages\":[{{\"text\":\"droit contrat\",\
                 \"vector\":\"{}\"}}]}}\n",
                base64_vector(&[0.6, 0.8, 0.0, 0.0])
            ),
        )
        .expect("documents.jsonl is writable");

        let vector = root.join("vector.json");
        fs::write(&vector, "[0.6,0.8,0.0,0.0]\n").expect("the query vector is writable");

        Self {
            _temp: temp,
            root,
            data,
            models,
            exchange,
            config,
            index_config,
            documents,
            vector,
        }
    }

    /// `<data>/indexes/<index>`, the directory the lock lives in.
    fn index_dir(&self) -> PathBuf {
        self.data.join("indexes").join(INDEX)
    }

    /// Path of the writer lock of the fixture index.
    fn writer_lock(&self) -> PathBuf {
        self.index_dir().join(WRITER_LOCK)
    }

    /// Path of the snapshot of the fixture, written by [`snapshot`].
    fn exchange_entries(&self, directory: &str) -> Vec<PathBuf> {
        let listing = fs::read_dir(self.exchange.join(directory))
            .unwrap_or_else(|error| panic!("{directory} is listable: {error}"));
        let mut entries: Vec<PathBuf> = listing
            .map(|entry| {
                entry
                    .unwrap_or_else(|error| panic!("an entry of {directory} is readable: {error}"))
                    .path()
            })
            .collect();
        entries.sort();
        entries
    }
}

/// `crible index create --data <data> --config <index.json>`.
fn create_index(fixture: &Fixture) -> Output {
    run(&[
        "index",
        "create",
        "--data",
        &fixture.data.display().to_string(),
        "--config",
        &fixture.index_config.display().to_string(),
    ])
}

/// `crible ingest` on the fixture document: generation 1, one live passage.
fn ingest(fixture: &Fixture) -> Output {
    run(&[
        "ingest",
        "--data",
        &fixture.data.display().to_string(),
        "--models",
        &fixture.models.display().to_string(),
        "--index",
        INDEX,
        &fixture.documents.display().to_string(),
    ])
}

/// `crible compact --data <data> --index <index>`: a write command.
fn compact(fixture: &Fixture) -> Output {
    run(&[
        "compact",
        "--data",
        &fixture.data.display().to_string(),
        "--index",
        INDEX,
    ])
}

/// `crible patch --data <data> --index <index> <patch>`: another write command.
///
/// The patch carries an empty `fields`, so its content alone never changes the
/// index: whatever the command reports, the lock was taken or not.
fn patch(fixture: &Fixture) -> Output {
    let file = fixture.root.join("patch.jsonl");
    fs::write(&file, "{\"doc_id\":\"DOC-1\",\"fields\":{}}\n").expect("the patch is writable");
    run(&[
        "patch",
        "--data",
        &fixture.data.display().to_string(),
        "--index",
        INDEX,
        &file.display().to_string(),
    ])
}

/// `crible inspect --data <data> --index <index>`: a read command.
fn inspect(fixture: &Fixture) -> Output {
    run(&[
        "inspect",
        "--data",
        &fixture.data.display().to_string(),
        "--index",
        INDEX,
    ])
}

/// `crible search --vector-file`: a read command, with the provided vector.
fn search(fixture: &Fixture) -> Output {
    run(&[
        "search",
        "--data",
        &fixture.data.display().to_string(),
        "--index",
        INDEX,
        "--vector-file",
        &fixture.vector.display().to_string(),
        "--k",
        "10",
    ])
}

/// `crible export`: a read command, written into the outbox of the exchange.
fn export(fixture: &Fixture) -> Output {
    run(&[
        "export",
        "--data",
        &fixture.data.display().to_string(),
        "--models",
        &fixture.models.display().to_string(),
        "--exchange",
        &fixture.exchange.display().to_string(),
        "--index",
        INDEX,
    ])
}

/// `crible snapshot`: a write command, written into the snapshots of the
/// exchange.
fn snapshot(fixture: &Fixture) -> Output {
    run(&[
        "snapshot",
        "--data",
        &fixture.data.display().to_string(),
        "--exchange",
        &fixture.exchange.display().to_string(),
        "--index",
        INDEX,
    ])
}

/// Runs the binary of the test with a closed environment: a `CRIBLE_*` variable
/// of the host must never change the fixture, and the journal of a command must
/// stay silent so `stderr` holds its only report.
fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_crible"))
        .args(arguments)
        .env_clear()
        .env("RUST_LOG", "warn")
        .output()
        .unwrap_or_else(|error| panic!("cannot run crible {arguments:?}: {error}"))
}

/// Reads the generation out of the output of `crible inspect`.
fn generation(output: &Output) -> u64 {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .find(|line| line.trim_start().starts_with("generation:"))
        .unwrap_or_else(|| panic!("crible inspect reports the generation, got {stdout:?}"));
    line.split_once(':')
        .and_then(|(_, value)| value.trim().parse().ok())
        .unwrap_or_else(|| panic!("the generation is a number, got {line:?}"))
}

/// Creates the fixture index and commits its generation 1.
fn prepare_index(fixture: &Fixture) {
    let created = create_index(fixture);
    assert!(
        created.status.success(),
        "crible index create must succeed: {}",
        String::from_utf8_lossy(&created.stderr)
    );
    let ingested = ingest(fixture);
    assert!(
        ingested.status.success(),
        "crible ingest must succeed: {}",
        String::from_utf8_lossy(&ingested.stderr)
    );
}

/// The state of a `WRITER_LOCK` file, as another process sees it.
#[derive(Debug, PartialEq, Eq)]
enum LockState {
    /// No process holds an exclusive lock: the file is free.
    Free,
    /// Another process holds the exclusive lock.
    Held,
}

/// Probes the lock of `path` from this process, without keeping it.
///
/// The file must exist: a missing `WRITER_LOCK` proves nothing about the lock,
/// so the test fails instead of reading it as "free".
fn lock_state(path: &Path) -> LockState {
    let file = fs::File::options()
        .read(true)
        .write(true)
        .open(path)
        .unwrap_or_else(|error| {
            panic!(
                "{} must exist: the writer lock is the file of ADR-0116, decision 2 \
                 (open failed: {error})",
                path.display()
            )
        });
    match file.try_lock() {
        Ok(()) => LockState::Free,
        Err(std::fs::TryLockError::WouldBlock) => LockState::Held,
        Err(std::fs::TryLockError::Error(error)) => {
            panic!("cannot try the lock of {}: {error}", path.display())
        }
    }
}

/// One entry of the recursive inventory of a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    /// Path relative to the root of the inventory, with `/` separators.
    path: String,
    /// Size in bytes: `0` for a directory.
    size: u64,
    /// `blake3:<hex>` of the bytes of a file; `<directory>` for a directory.
    digest: String,
}

/// Recursive inventory of `root`: every path, its size and its blake3, sorted
/// by path, `-wal` and `-shm` excepted (SQLite manages those itself, ADR-0038).
fn inventory(root: &Path) -> Vec<Entry> {
    let mut entries = Vec::new();
    collect(root, root, &mut entries);
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    entries
}

/// Recursive body of [`inventory`].
fn collect(root: &Path, directory: &Path, entries: &mut Vec<Entry>) {
    let listing = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("{} is listable: {error}", directory.display()));
    for entry in listing {
        let entry = entry.unwrap_or_else(|error| {
            panic!("an entry of {} is readable: {error}", directory.display())
        });
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with("-wal") || name.ends_with("-shm") {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or_else(|error| {
                panic!("{} is under {}: {error}", path.display(), root.display())
            })
            .to_string_lossy()
            .replace('\\', "/");
        let metadata = fs::symlink_metadata(&path)
            .unwrap_or_else(|error| panic!("{} is inspectable: {error}", path.display()));
        if metadata.is_dir() {
            entries.push(Entry {
                path: relative,
                size: 0,
                digest: "<directory>".to_owned(),
            });
            collect(root, &path, entries);
            continue;
        }
        let bytes = fs::read(&path)
            .unwrap_or_else(|error| panic!("{} is readable: {error}", path.display()));
        entries.push(Entry {
            path: relative,
            size: metadata.len(),
            digest: format!("blake3:{}", blake3::hash(&bytes).to_hex()),
        });
    }
}

/// Every path under `root` whose file name is `WRITER_LOCK`.
fn writer_lock_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(listing) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in listing {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if entry.file_name().to_string_lossy() == WRITER_LOCK {
                found.push(path);
            } else if path.is_dir() {
                pending.push(path);
            }
        }
    }
    found.sort();
    found
}

/// True when `message` says that another process writes the index.
///
/// The exact wording belongs to the implementation; ADR-0116, decision 2, only
/// mandates the two facts: another process, and that it writes the index.
fn says_another_process_writes(message: &str) -> bool {
    let lower = message.to_lowercase();
    lower.contains("another process") && lower.contains("writ")
}

/// A real `crible serve` child, killed if the test fails before it stops it.
struct Server {
    child: Child,
    /// Address the child bound, read from its journal.
    address: SocketAddr,
    /// Thread draining the journal, so a full pipe never blocks the child.
    _journal: JoinHandle<()>,
}

impl Server {
    /// Starts `crible serve` on `fixture` and waits for the address it bound.
    ///
    /// The configuration binds `127.0.0.1:0` and the address is read from the
    /// `"the server is bound"` line of the journal, so the test never reserves
    /// a port the child cannot keep (the race `serve_cli.rs` documents).
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
        let (sender, receiver) = mpsc::channel::<String>();
        let journal = std::thread::spawn(move || {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(stdout);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let deadline = Instant::now() + PATIENCE;
        let address = loop {
            match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(line) => {
                    if let Some(address) = bound_address(&line) {
                        break address;
                    }
                }
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!(
                        "crible serve must announce the address it bound within {PATIENCE:?}; \
                         nothing was announced"
                    );
                }
            }
        };
        Self {
            child,
            address,
            _journal: journal,
        }
    }

    /// Waits until `/ready` answers `200`: the index is loaded and served, so
    /// the writer lock is held.
    fn wait_ready(&self) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Some(reply) = try_ready(self.address)
                && reply.status == 200
            {
                return;
            }
            if Instant::now() >= deadline {
                panic!("/ready never answered 200 within {PATIENCE:?}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Kills the child with `SIGKILL` and waits for its death.
    ///
    /// `Child::kill` sends `SIGKILL` on Unix: the process runs no destructor and
    /// no shutdown, which is what INV-04 must survive.
    fn kill_hard(&mut self) {
        self.child
            .kill()
            .unwrap_or_else(|error| panic!("the child is killable: {error}"));
        let status = self
            .child
            .wait()
            .unwrap_or_else(|error| panic!("the child is waitable: {error}"));
        assert!(
            !status.success(),
            "a SIGKILL never reports success, got {status}"
        );
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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

/// One reply of a probe, read over a socket.
struct Reply {
    status: u16,
}

/// Sends `GET /ready` and reads the status line, or `None` while the server
/// does not accept yet.
fn try_ready(address: SocketAddr) -> Option<Reply> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(200)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("the timeout is settable");
    let request = "GET /ready HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    stream
        .write_all(request.as_bytes())
        .expect("the request is writable");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).expect("the reply is readable");
    let text = String::from_utf8_lossy(&raw);
    let head = text
        .split_once("\r\n\r\n")
        .map(|(head, _)| head)
        .unwrap_or_else(|| panic!("a reply has a head and a body, got {text:?}"));
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or_else(|| panic!("the status line carries a code, got {head:?}"));
    Some(Reply { status })
}

/// Configuration of the fixture instance (`docs/07-stack-et-deploiement.md`).
fn configuration(root: &Path) -> String {
    let data = root.join("data");
    let models = root.join("models");
    let exchange = root.join("exchange");
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
name = "legifacile-mcp"
role = "read"
sha256 = "1111111111111111111111111111111111111111111111111111111111111111"

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
        data = data.display(),
        models = models.display(),
        exchange = exchange.display(),
    )
}

/// `index.json` of the fixture: an `external` model, no lexical branch, an
/// empty schema.
fn index_json() -> String {
    format!(
        r#"{{
  "name": "{INDEX}",
  "model": "{MODEL}",
  "reference_precision": "fp16",
  "schema": {{ "document_fields": {{}}, "passage_fields": {{}} }},
  "cluster_by": [],
  "lexical": {{ "enabled": false, "language": "fr", "positions": false }},
  "search": {{
    "scan": "none",
    "rescore_candidates": 10,
    "exact_threshold": 10,
    "candidates_per_branch": 10,
    "fusion": {{ "method": "rrf" }}
  }},
  "ingest": {{ "acceptance_sample": 1, "acceptance_min_cosine": 0.9 }},
  "compaction": {{ "dead_ratio": 0.1 }}
}}"#
    )
}

/// Base64 of the little-endian `f32` components of `vector`, the vector of an
/// `external` model (`docs/format-lot-v1.md`).
fn base64_vector(vector: &[f32]) -> String {
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

// --- INV-04: the lock dies with its process ---

#[test]
fn inv_04_writer_lock_is_free_after_the_serving_process_is_killed() {
    // INV-04: "au redémarrage, l'état est la dernière génération commitée".
    // The kernel releases a file lock when the process dies, including on
    // SIGKILL, so a leftover `WRITER_LOCK` file never blocks the next writer
    // (ADR-0116, decision 2). The lock is proven held while the server serves,
    // then free after the kill, and the next write command commits from the
    // last committed generation.
    let fixture = Fixture::new();
    prepare_index(&fixture);
    let committed = generation(&inspect(&fixture));
    assert_eq!(committed, 1, "the fixture commits generation 1");

    let mut server = Server::start(&fixture);
    server.wait_ready();

    assert!(
        fixture.writer_lock().is_file(),
        "the serving process holds {} while it serves the index (ADR-0116, decision 2); \
         the file is missing",
        fixture.writer_lock().display()
    );
    assert_eq!(
        lock_state(&fixture.writer_lock()),
        LockState::Held,
        "the serving process holds the writer lock while it serves the index"
    );

    server.kill_hard();

    // No sleep: the lock is released by the kernel at the death of the process,
    // and `kill_hard` waited for that death.
    assert_eq!(
        lock_state(&fixture.writer_lock()),
        LockState::Free,
        "the writer lock must be free after its holder dies (INV-04, ADR-0116)"
    );

    let recovered = inspect(&fixture);
    assert!(
        recovered.status.success(),
        "the index still opens after the kill: {}",
        String::from_utf8_lossy(&recovered.stderr)
    );
    assert_eq!(
        generation(&recovered),
        committed,
        "the index is at the last committed generation after the kill (INV-04)"
    );

    let compacted = compact(&fixture);
    assert!(
        compacted.status.success(),
        "a write command passes once its holder is dead, got {}\nstderr: {}",
        compacted.status,
        String::from_utf8_lossy(&compacted.stderr)
    );
    let after = inspect(&fixture);
    assert!(
        after.status.success(),
        "the index opens after the compaction: {}",
        String::from_utf8_lossy(&after.stderr)
    );
    assert_eq!(
        generation(&after),
        committed + 1,
        "the write command committed from the last committed generation (INV-04)"
    );

    let stdout = String::from_utf8_lossy(&search(&fixture).stdout).into_owned();
    let answer: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("the search answer is JSON, got {stdout:?}: {error}"));
    assert_eq!(
        answer["results"].as_array().map(Vec::len),
        Some(1),
        "the passage survived the kill and the compaction, got {stdout}"
    );
}

// --- INV-04: the lock file is not a residue, nor part of a copy ---

#[test]
fn inv_04_writer_lock_is_left_in_place_by_the_write_opening() {
    // The lock file lives in the index directory (ADR-0116) and carries no
    // data. A write opening runs the recovery of INV-04, which cleans the
    // leftovers of a crash; it must leave the lock file alone, and the lock
    // file must stay out of a snapshot, of its `snapshot.json` and of an
    // export.
    let fixture = Fixture::new();
    prepare_index(&fixture);

    assert!(
        fixture.writer_lock().is_file(),
        "a write command takes {} for the whole duration of its open index \
         (ADR-0116, decision 2); the file is missing",
        fixture.writer_lock().display()
    );
    assert_eq!(
        lock_state(&fixture.writer_lock()),
        LockState::Free,
        "the lock is released at the end of the command"
    );

    // The recovery of a later write open must not unlink the lock file: a
    // writer that removes it reopens the ABA defect of ADR-0056, because a
    // second writer would lock a new inode while the first holds the old one.
    let held = fixture
        .writer_lock()
        .metadata()
        .expect("the lock is inspectable");
    use std::os::unix::fs::MetadataExt;
    let inode_before = (held.dev(), held.ino());
    let patched = patch(&fixture);
    assert!(
        patched.status.success(),
        "crible patch must succeed: {}",
        String::from_utf8_lossy(&patched.stderr)
    );
    let after = fixture.writer_lock().metadata().unwrap_or_else(|error| {
        panic!(
            "the write opening removed {} as if it were a residue (ADR-0116); \
             the lock file must stay in place: {error}",
            fixture.writer_lock().display()
        )
    });
    assert_eq!(
        (after.dev(), after.ino()),
        inode_before,
        "the write opening replaced the lock file instead of locking it"
    );

    let taken = snapshot(&fixture);
    assert!(
        taken.status.success(),
        "crible snapshot must succeed: {}",
        String::from_utf8_lossy(&taken.stderr)
    );
    let snapshots = fixture.exchange_entries("snapshots");
    assert_eq!(
        snapshots.len(),
        1,
        "one snapshot was written, got {snapshots:?}"
    );
    assert!(
        writer_lock_files(&snapshots[0]).is_empty(),
        "the snapshot must not carry {} (ADR-0116, decision 2), found {:?}",
        WRITER_LOCK,
        writer_lock_files(&snapshots[0])
    );
    let attestation: Value = serde_json::from_slice(
        &fs::read(snapshots[0].join("snapshot.json"))
            .unwrap_or_else(|error| panic!("snapshot.json must be readable: {error}")),
    )
    .expect("snapshot.json is JSON");
    let listed = attestation["files"]
        .as_array()
        .unwrap_or_else(|| panic!("snapshot.json lists its files, got {attestation}"));
    for file in listed {
        let path = file["path"].as_str().unwrap_or_default();
        assert!(
            !path.ends_with(WRITER_LOCK),
            "snapshot.json must not list {WRITER_LOCK}, got {path:?}"
        );
    }

    let exported = export(&fixture);
    assert!(
        exported.status.success(),
        "crible export must succeed: {}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let outbox = fixture.exchange_entries("outbox");
    assert_eq!(outbox.len(), 1, "one batch was exported, got {outbox:?}");
    assert!(
        writer_lock_files(&outbox[0]).is_empty(),
        "the exported batch must not carry {WRITER_LOCK}, found {:?}",
        writer_lock_files(&outbox[0])
    );
    let manifest = fs::read(outbox[0].join("manifest.json"))
        .unwrap_or_else(|error| panic!("the manifest of the batch is readable: {error}"));
    assert!(
        !String::from_utf8_lossy(&manifest).contains(WRITER_LOCK),
        "the manifest of the exported batch must not name {WRITER_LOCK}"
    );

    assert!(
        fixture.writer_lock().is_file(),
        "the lock file belongs to the index directory and survives the copies"
    );
}

// --- The refusal of a concurrent write command ---

#[test]
fn write_command_is_refused_in_overloaded_while_a_server_serves_the_index() {
    // ADR-0116, decision 2: while an instance serves an index, a write command
    // of the CLI is refused at once, without waiting, in `overloaded`
    // (`docs/05-api.md`). The message names the index and says that another
    // process writes it, and never carries a path of the server (ADR-0101).
    // The refused command leaves the index intact.
    let fixture = Fixture::new();
    prepare_index(&fixture);
    let committed = generation(&inspect(&fixture));

    let mut server = Server::start(&fixture);
    server.wait_ready();

    let before = inventory(&fixture.index_dir());
    let started = Instant::now();
    let refused = compact(&fixture);
    let elapsed = started.elapsed();

    assert!(
        !refused.status.success(),
        "crible compact must be refused while the index is served, got {}\nstdout: {}",
        refused.status,
        String::from_utf8_lossy(&refused.stdout)
    );
    assert_eq!(
        refused.status.code(),
        Some(1),
        "a command failure exits 1 (`crates/crible/src/main.rs`), got {}",
        refused.status
    );
    let stderr = String::from_utf8_lossy(&refused.stderr).into_owned();
    assert!(
        stderr.starts_with("overloaded: "),
        "the refusal is `overloaded` (`docs/05-api.md`), got {stderr:?}"
    );
    assert!(
        stderr.contains(INDEX),
        "the message names the index, got {stderr:?}"
    );
    assert!(
        says_another_process_writes(&stderr),
        "the message says that another process writes the index, got {stderr:?}"
    );
    for path in [
        fixture.data.display().to_string(),
        fixture.index_dir().display().to_string(),
        fixture.root.display().to_string(),
    ] {
        assert!(
            !stderr.contains(&path),
            "the message must not carry a server path (ADR-0101), got {stderr:?}"
        );
    }
    assert!(
        elapsed < Duration::from_secs(3),
        "the refusal is immediate, without waiting (ADR-0116, decision 2); it took {elapsed:?}"
    );

    let after = inventory(&fixture.index_dir());
    assert_eq!(
        after, before,
        "a refused write command leaves the index intact: the inventory of paths, \
         sizes and blake3 must be identical, -wal and -shm excepted"
    );
    assert_eq!(
        generation(&inspect(&fixture)),
        committed,
        "the refused command commits no generation"
    );

    server.kill_hard();
}

#[test]
fn patch_is_refused_in_overloaded_while_a_server_serves_the_index() {
    // ADR-0116, decision 2: the rule holds for every command of the CLI that
    // opens an existing index for writing, not only for `compact`. `patch` is
    // the other write path with no model and no compaction, so the refusal does
    // not depend on either.
    let fixture = Fixture::new();
    prepare_index(&fixture);

    let mut server = Server::start(&fixture);
    server.wait_ready();

    let refused = patch(&fixture);
    assert!(
        !refused.status.success(),
        "crible patch must be refused while the index is served, got {}",
        refused.status
    );
    assert_eq!(refused.status.code(), Some(1), "a command failure exits 1");
    let stderr = String::from_utf8_lossy(&refused.stderr).into_owned();
    assert!(
        stderr.starts_with("overloaded: "),
        "the refusal is `overloaded`, got {stderr:?}"
    );
    assert!(
        stderr.contains(INDEX),
        "the message names the index, got {stderr:?}"
    );
    assert!(
        says_another_process_writes(&stderr),
        "the message says that another process writes the index, got {stderr:?}"
    );
    assert!(
        !stderr.contains(&fixture.data.display().to_string()),
        "the message must not carry a server path (ADR-0101), got {stderr:?}"
    );

    server.kill_hard();
}

// --- The read commands pass beside the lock ---

#[test]
fn search_inspect_and_export_pass_beside_a_serving_process() {
    // ADR-0116, decision 2: "les commandes de lecture ne prennent pas le
    // verrou". `search`, `inspect` and `export` must answer beside a server
    // that holds the writer lock, and none of them may touch the index
    // directory (decision 1, `OUVRIR-EN-LECTURE-SANS-ECRIRE`).
    let fixture = Fixture::new();
    prepare_index(&fixture);
    let committed = generation(&inspect(&fixture));

    let mut server = Server::start(&fixture);
    server.wait_ready();
    let before = inventory(&fixture.index_dir());

    let searched = search(&fixture);
    assert!(
        searched.status.success(),
        "crible search passes beside a serving process: {}",
        String::from_utf8_lossy(&searched.stderr)
    );
    let answer: Value = serde_json::from_slice(&searched.stdout)
        .unwrap_or_else(|error| panic!("the search answer is JSON: {error}"));
    assert_eq!(
        answer["generation"].as_u64(),
        Some(committed),
        "the search reads the served generation, got {answer}"
    );
    assert_eq!(
        answer["results"].as_array().map(Vec::len),
        Some(1),
        "the search finds the fixture passage, got {answer}"
    );

    let inspected = inspect(&fixture);
    assert!(
        inspected.status.success(),
        "crible inspect passes beside a serving process: {}",
        String::from_utf8_lossy(&inspected.stderr)
    );
    assert_eq!(
        generation(&inspected),
        committed,
        "inspect reports the served generation"
    );

    let exported = export(&fixture);
    assert!(
        exported.status.success(),
        "crible export passes beside a serving process: {}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let outbox = fixture.exchange_entries("outbox");
    assert_eq!(outbox.len(), 1, "one batch was exported, got {outbox:?}");
    assert!(
        outbox[0].join("manifest.json").is_file(),
        "the exported batch carries its manifest"
    );

    let after = inventory(&fixture.index_dir());
    assert_eq!(
        after, before,
        "the read commands leave the index directory untouched, -wal and -shm \
         excepted (ADR-0116, decision 1)"
    );

    server.kill_hard();
}
