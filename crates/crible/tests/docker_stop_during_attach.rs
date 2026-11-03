// SPDX-License-Identifier: Apache-2.0

//! A `docker stop` during an attach: nothing is lost (INV-03, INV-04).
//!
//! `docs/phases/P8-docker-et-deploiement.md`, `VERIFIER-ARRET-PROPRE`. The test
//! starts the image of `docker/README.md`, submits an attach big enough to run
//! for several seconds, waits until the job is `running`, serves searches, then
//! sends `docker stop -t 65` — above `[limits].wait_timeout_ms`, as the README
//! requires, so the stop is the clean one ADR-0099 describes.
//!
//! After the stop, the container is started again and the test checks:
//!
//! - the deliverable of the sheet: the analytics served before the stop are on
//!   disk, written by the shutdown itself (ADR-0118, decision 3). The copy of
//!   `/data` taken **before** the stop holds none of them — they are still in
//!   the in-memory aggregator — so the 10 s tick rules itself out;
//! - **INV-03**: the published generation is the one before the attach or the
//!   one after it, whole. Its counts match it, and the documents of the batch
//!   are all visible with `G+1` and none with `G`;
//! - **INV-04**: the index holds no `tmp-*` entry left by the interrupted work,
//!   and the job is either finished or resubmitted successfully.
//!
//! `inv_04_...` repeats the sequence with a `[limits].wait_timeout_ms` of one
//! second: the stop then lands **inside** the attach instead of waiting for it,
//! which is the interrupted path of ADR-0104. The job comes back `failed` with
//! the fixed message of `docs/05-api.md`, the generation is unchanged, and the
//! same batch directory is replayed successfully.
//!
//! # Prerequisites
//!
//! The test needs Docker and the image, built once (about four minutes):
//!
//! ```text
//! docker buildx build --platform linux/arm64 --progress=plain --load \
//!   -t crible:dev -f docker/Dockerfile .
//! ```
//!
//! The image name is read from `CRIBLE_DOCKER_IMAGE`, `crible:dev` by default.
//! Run both tests, one at a time because each one drives its own containers:
//!
//! ```text
//! CRIBLE_DOCKER_IMAGE=crible:dev cargo test -p crible \
//!   --test docker_stop_during_attach -- --include-ignored --test-threads=1
//! ```
//!
//! `#[ignore]` is the only allowed skip (ADR-0057): the CI has no Docker, so the
//! two tests stay local, and a run without the image fails with the command
//! above rather than passing silently.
//!
//! On a failure, the journal of the container is in the message: `docker logs`
//! of the last run.

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixture of this file is a helper.
#![allow(clippy::expect_used, clippy::panic)]

mod common;

use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

use arrow::array::{FixedSizeBinaryArray, StringArray, UInt32Array};
use arrow::datatypes::{DataType, Field, Schema as ArrowSchema, SchemaRef};
use crible_store::analytics::{ANALYTICS_FILE_NAME, MinuteEvent, read_minute_events};
use half::f16;
use serde_json::{Value, json};
use tempfile::TempDir;

/// Passages of the batch: the attach must outlast the stop several times.
///
/// Measured on the development machine (arm64, named `/data` volume): about
/// 3.5 s for a batch of this size, against the tens of milliseconds a
/// `docker stop` needs to deliver `SIGTERM`. The test refuses to pass when the
/// attach is already over, so a smaller batch would show up as a failure, not
/// as a weaker proof.
const DOCUMENTS: usize = 200_000;

/// Dimensions of the `external` model of the fixture; no ONNX Runtime.
const DIMENSIONS: usize = 16;

/// Searches served during the attach: their analytics rows must survive.
const SEARCHES: u64 = 3;

/// Name of the index of the fixture.
const INDEX: &str = "alpha";

/// Name of the `external` model of the fixture.
const MODEL: &str = "ext";

/// Name of the batch directory under `<exchange>/inbox`.
const BATCH: &str = "stop-during-attach";

/// Secret of the `write` key of the fixture, never in a versioned file.
const SECRET: &str = "docker-stop-during-attach-write-key";

/// Time the instance has to answer `/ready`, model and index included.
const READY_PATIENCE: Duration = Duration::from_secs(120);

/// Time the attach has to reach `running`, and the replay to finish.
const JOB_PATIENCE: Duration = Duration::from_secs(60);

/// Interval between two polls of `/ready` or of the job.
const POLL: Duration = Duration::from_millis(20);

/// Stop delay of `docker stop`, above `[limits].wait_timeout_ms` of the README.
const STOP_TIMEOUT_S: &str = "65";

/// Fixed `created_at` of the batch: the test never reads the clock.
const CREATED_AT: &str = "2026-01-01T00:00:00Z";

/// Number of laboratories of the process, for a name unique per run.
static LABORATORIES: AtomicUsize = AtomicUsize::new(0);

/// `inv_03_a_docker_stop_during_an_attach_leaves_one_complete_generation`.
#[test]
#[ignore = "requires Docker and the image of docker/README.md (CRIBLE_DOCKER_IMAGE, `crible:dev`) ; build it with `docker buildx build --platform linux/arm64 --progress=plain --load -t crible:dev -f docker/Dockerfile .`"]
fn inv_03_a_docker_stop_during_an_attach_leaves_one_complete_generation() {
    // The README configuration: the job bound is the documented 60 s.
    let lab = Lab::new(60_000);

    // Before the attach the index is empty: generation 0, no document.
    let before = lab.index_state();
    assert_eq!(before["generation"], 0, "a fresh index is at generation 0");
    assert_eq!(before["documents"], 0, "a fresh index holds no document");
    assert_eq!(before["live_passages"], 0, "a fresh index holds no passage");

    let (status, job) = lab.submit_attach(false);
    assert_eq!(status, 202, "the attach is queued: {job}");
    let id = job_id(&job);
    lab.wait_running(&id);

    // Searches served while the attach runs: the shutdown must keep them.
    for _ in 0..SEARCHES {
        lab.search();
    }

    // The aggregator of ADR-0118 keeps the events in memory until its 10 s tick
    // (`crates/crible-server/src/analytics.rs:73`, `FLUSH_INTERVAL`). Copying
    // `/data` right here is what tells the flush of the shutdown from a periodic
    // one: nothing of the three searches is on disk yet.
    let before_stop = lab.copy_data("before-stop");
    let pending = searches_in(&before_stop);
    assert_eq!(
        pending, 0,
        "the {SEARCHES} search events are still in the aggregator, not on disk: \
         a periodic flush already wrote {pending} of them"
    );

    // The stop must land inside the attach. A terminal job here would let the
    // test pass without ever interrupting anything, which is the one way this
    // proof could silently weaken.
    let during = lab.job(&id);
    assert_eq!(
        during["state"],
        "running",
        "the attach must still be running when the stop lands, it is {during}: \
         enlarge the batch of the test\n{}",
        lab.journal()
    );

    lab.stop();

    // INV-04, first half: the analytics are on disk before any restart, and the
    // only flush of this run was the one of the stop, the first copy being empty.
    let after_stop = lab.copy_data("after-stop");
    let served = searches_in(&after_stop);
    assert_eq!(
        served, SEARCHES,
        "the shutdown flushed the analytics it served: {served} of {SEARCHES} \
         search events on disk"
    );
    // INV-04, second half, first observation: what the interrupted work left.
    let index_after_stop = after_stop.join("indexes").join(INDEX);
    let left_by_the_stop = temporary_entries(&index_after_stop);

    // The state after the restart. INV-03: generation G or G+1, whole.
    lab.start_again();
    let after = lab.index_state();
    let generation = after["generation"]
        .as_u64()
        .unwrap_or_else(|| panic!("the state carries a generation: {after}"));
    assert!(
        generation <= 1,
        "the attach publishes at most one generation, got {generation}"
    );
    if generation == 0 {
        assert_eq!(after["documents"], 0, "generation 0 holds no document");
        assert_eq!(after["live_passages"], 0, "generation 0 holds no passage");
        assert_eq!(
            lab.document("doc-0000000"),
            404,
            "generation 0 exposes no document of the batch"
        );
    } else {
        assert_eq!(
            after["documents"], DOCUMENTS,
            "generation 1 holds every document of the batch: {after}"
        );
        assert_eq!(
            after["live_passages"], DOCUMENTS,
            "generation 1 holds every passage of the batch: {after}"
        );
        assert_eq!(
            lab.document("doc-0000000"),
            200,
            "generation 1 exposes the first document"
        );
        assert_eq!(
            lab.document("doc-0199999"),
            200,
            "generation 1 exposes the last document: no half-applied commit"
        );
    }

    // The job is finished, or the client resubmits it (ADR-0103).
    let interrupted = lab.job(&id);
    let state = interrupted["state"]
        .as_str()
        .unwrap_or_else(|| panic!("the job carries a state: {interrupted}"))
        .to_owned();
    assert!(
        state == "succeeded" || state == "failed",
        "the stop leaves the job terminal, got '{state}': {interrupted}"
    );
    if state == "failed" {
        assert_eq!(
            interrupted["error"]["code"], "internal",
            "an interrupted job fails in `internal`: {interrupted}"
        );
        assert_eq!(
            interrupted["error"]["message"], "the job was interrupted by a server restart",
            "the message of `docs/05-api.md`: {interrupted}"
        );
    }

    // Replayable: the same batch directory, resubmitted, is applied or ignored.
    let replay = lab.replay();
    assert_eq!(
        replay["state"], "succeeded",
        "the replay succeeded: {replay}"
    );
    let applied = lab.index_state();
    assert_eq!(applied["generation"], 1, "the batch is applied: {applied}");
    assert_eq!(applied["documents"], DOCUMENTS, "every document: {applied}");
    assert_eq!(
        applied["live_passages"], DOCUMENTS,
        "every passage: {applied}"
    );
    assert_eq!(
        lab.document("doc-0199999"),
        200,
        "the last document of the batch is searchable after the replay"
    );

    // INV-04, second half: the restart cleans everything the stop left. The
    // entries observed on the stopped container are named one by one: the proof
    // is not only "no `tmp-*` now", it is "none of *those* survived".
    let after_restart = lab.copy_data("after-restart");
    let leftovers = temporary_entries(&after_restart.join("indexes").join(INDEX));
    let survived: Vec<&String> = left_by_the_stop
        .iter()
        .filter(|entry| leftovers.contains(entry))
        .collect();
    assert!(
        survived.is_empty(),
        "the restart cleans every `tmp-*` entry the stop left: {survived:?} survived \
         (left by the stop {left_by_the_stop:?}, after the restart {leftovers:?})"
    );
}

/// `inv_04_a_docker_stop_during_an_attach_leaves_the_last_committed_generation`.
///
/// `[limits].wait_timeout_ms` is one second, far below the attach: the stop
/// lands inside it (ADR-0104), the job is abandoned, and the next start must
/// leave the last committed generation — the empty one — and clean the rest.
#[test]
#[ignore = "requires Docker and the image of docker/README.md (CRIBLE_DOCKER_IMAGE, `crible:dev`) ; build it with `docker buildx build --platform linux/arm64 --progress=plain --load -t crible:dev -f docker/Dockerfile .`"]
fn inv_04_a_docker_stop_during_an_attach_leaves_the_last_committed_generation() {
    let lab = Lab::new(1_000);

    let before = lab.index_state();
    assert_eq!(before["generation"], 0, "a fresh index is at generation 0");

    let (status, job) = lab.submit_attach(false);
    assert_eq!(status, 202, "the attach is queued: {job}");
    let id = job_id(&job);
    lab.wait_running(&id);

    lab.stop();

    // What the interrupted attach left on the stopped container, before any
    // restart. The protocol builds a segment in `segments/tmp-<segment_id>`
    // before renaming it (`crates/crible-store/src/segment.rs:206`), but only
    // during the write of the segment itself, a fraction of the attach: with a
    // one-second drain the stop lands in the validation, which reads the batch
    // and plans the semantic table before any write
    // (`crates/crible-store/src/attach.rs:561-599`, the segment is written only
    // at `:600`). Presence is therefore not asserted — observed empty on every
    // run of this machine — and only the survival of what is there would be a
    // defect.
    let after_stop = lab.copy_data("after-stop");
    let left_by_the_stop = temporary_entries(&after_stop.join("indexes").join(INDEX));

    // The restart marks the abandoned job `failed` (ADR-0103, ADR-0104).
    lab.start_again();

    let interrupted = lab.job(&id);
    assert_eq!(
        interrupted["state"], "failed",
        "a job cut by the stop fails at the next start: {interrupted}"
    );
    assert_eq!(
        interrupted["error"]["code"], "internal",
        "an interrupted job fails in `internal`: {interrupted}"
    );
    assert_eq!(
        interrupted["error"]["message"], "the job was interrupted by a server restart",
        "the fixed message of `docs/05-api.md`: {interrupted}"
    );

    // INV-04: the state is the last committed generation, and only that one.
    let after = lab.index_state();
    assert_eq!(
        after["generation"], 0,
        "the interrupted attach commits nothing: {after}"
    );
    assert_eq!(after["documents"], 0, "generation 0 holds no document");
    assert_eq!(after["live_passages"], 0, "generation 0 holds no passage");
    assert_eq!(
        lab.document("doc-0000000"),
        404,
        "nothing of the uncommitted batch is served"
    );
    assert_eq!(
        lab.document("doc-0199999"),
        404,
        "nothing of the uncommitted batch is served, the last document either"
    );

    // INV-04: everything the interrupted attach left is rebuilt or cleaned. The
    // proof rests on the entries seen on the stopped container: none may survive.
    let after_restart = lab.copy_data("after-restart");
    let index = after_restart.join("indexes").join(INDEX);
    let leftovers = temporary_entries(&index);
    let survived: Vec<&String> = left_by_the_stop
        .iter()
        .filter(|entry| leftovers.contains(entry))
        .collect();
    assert!(
        survived.is_empty(),
        "the restart cleans every `tmp-*` entry the stop left: {survived:?} survived \
         (left by the stop {left_by_the_stop:?}, after the restart {leftovers:?})"
    );
    let current_epoch = fs::read_to_string(index.join("CURRENT_EPOCH"))
        .expect("the index names its current epoch after the restart")
        .trim()
        .to_owned();
    assert_eq!(
        current_epoch, "1",
        "the interrupted attach opens no new epoch"
    );

    // Replayable: the client resubmits the same batch, and it goes through.
    let replay = lab.replay();
    assert_eq!(
        replay["state"], "succeeded",
        "the replay succeeded: {replay}"
    );
    let applied = lab.index_state();
    assert_eq!(
        applied["generation"], 1,
        "the replay commits G+1: {applied}"
    );
    assert_eq!(
        applied["documents"], DOCUMENTS,
        "the replayed batch holds every document: {applied}"
    );
    assert_eq!(
        applied["live_passages"], DOCUMENTS,
        "the replayed batch holds every passage: {applied}"
    );
    assert_eq!(
        lab.document("doc-0199999"),
        200,
        "the last document of the replayed batch is served"
    );
}

// --- The laboratory: one volume, one container, one exchange directory ---

/// One instance of the image, with its own named volume and container names.
///
/// The names carry the process id and a counter, so two runs of the test never
/// collide. [`Drop`] removes the container and the volume, even when the test
/// panics, so a failed run leaves nothing behind.
struct Lab {
    container: String,
    volume: String,
    root: TempDir,
    /// Host port Docker published; `docker start` may publish another one.
    port: Cell<u16>,
    /// `[limits].wait_timeout_ms` of the fixture configuration.
    wait_timeout_ms: u64,
}

impl Lab {
    /// Creates the volume, the index and the batch, then starts the instance.
    ///
    /// The [`Drop`] guard exists from the first line: a failure on the way
    /// removes the volume it may have created, so a failed run of the test
    /// leaves nothing behind.
    fn new(wait_timeout_ms: u64) -> Self {
        let root = tempfile::tempdir().expect("a temporary instance is creatable");
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            LABORATORIES.fetch_add(1, Ordering::SeqCst)
        );
        let lab = Self {
            container: format!("crible-stop-{suffix}"),
            volume: format!("crible-stop-data-{suffix}"),
            root,
            port: Cell::new(0),
            wait_timeout_ms,
        };
        lab.prepare();
        lab.start_container();
        lab.port.set(host_port(&lab.container));
        lab.wait_ready();
        lab
    }

    /// Writes the configuration, the model, the index and the batch, and
    /// creates the volume with its empty index.
    fn prepare(&self) {
        let root = self.root.path();
        let exchange = root.join("exchange");
        let models = root.join("models").join(MODEL);
        fs::create_dir_all(exchange.join("inbox")).expect("the exchange is creatable");
        fs::create_dir_all(exchange.join("seed")).expect("the seed directory is creatable");
        fs::create_dir_all(&models).expect("the model directory is creatable");

        // The container runs as uid 1000: the directories it reads must be
        // traversable, and `tempfile` creates them `0700`.
        for directory in [
            &exchange,
            &exchange.join("inbox"),
            &exchange.join("seed"),
            &models,
        ] {
            make_traversable(directory);
        }

        fs::write(
            models.join("model.json"),
            format!(
                "{{\"name\":\"{MODEL}\",\"kind\":\"external\",\"dims\":{DIMENSIONS},\"normalize\":true}}\n"
            ),
        )
        .expect("the model configuration is written");
        fs::write(exchange.join("seed").join("index.json"), index_config())
            .expect("the index configuration is written");
        fs::write(root.join("crible.toml"), config(self.wait_timeout_ms))
            .expect("crible.toml is written");

        let batch = exchange.join("inbox").join(BATCH);
        write_batch(&batch);
        make_traversable(&batch);

        docker_ok(&["volume", "create", &self.volume]);
        docker_ok(&[
            "run",
            "--rm",
            "-u",
            "0",
            "-v",
            &format!("{}:/data", self.volume),
            "--entrypoint",
            "sh",
            &image(),
            "-c",
            "mkdir -p /data/indexes && chown -R 1000:1000 /data",
        ]);
        docker_ok(&[
            "run",
            "--rm",
            "-u",
            "1000:1000",
            "-v",
            &format!("{}:/data", self.volume),
            "-v",
            &format!("{}:/exchange", exchange.display()),
            &image(),
            "index",
            "create",
            "--data",
            "/data",
            "--config",
            "/exchange/seed/index.json",
        ]);
    }

    /// Runs the instance in the background, on a random host port.
    fn start_container(&self) {
        let root = self.root.path();
        docker_ok(&[
            "run",
            "-d",
            "--name",
            &self.container,
            "-p",
            "127.0.0.1::8080",
            "--stop-timeout",
            STOP_TIMEOUT_S,
            "-v",
            &format!("{}:/data", self.volume),
            "-v",
            &format!("{}:/models:ro", root.join("models").display()),
            "-v",
            &format!("{}:/exchange", root.join("exchange").display()),
            "-v",
            &format!(
                "{}:/etc/crible/crible.toml:ro",
                root.join("crible.toml").display()
            ),
            &image(),
        ]);
    }

    /// Waits until `/ready` answers `200`: models loaded, index open.
    fn wait_ready(&self) {
        let deadline = Instant::now() + READY_PATIENCE;
        loop {
            if let Some((status, _)) = self.try_request("GET", "/ready", None)
                && status == 200
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the instance is not ready within {READY_PATIENCE:?} \
                 (wait_timeout_ms = {}):\n{}",
                self.wait_timeout_ms,
                self.journal()
            );
            sleep(POLL);
        }
    }

    /// Serves one dense search; fails with the body when the answer is not `200`.
    fn search(&self) {
        let body = json!({
            "vector": crible_store::online::encode_base64(&unit_vector()),
            "mode": "dense",
            "k": 1,
        })
        .to_string();
        let (status, answer) =
            self.request("POST", &format!("/v1/indexes/{INDEX}/search"), Some(&body));
        assert_eq!(
            status, 200,
            "a search during the attach answers `200`: {answer}"
        );
    }

    /// `state` of `GET /v1/indexes/{index}`.
    fn index_state(&self) -> Value {
        let (status, body) = self.request("GET", &format!("/v1/indexes/{INDEX}"), None);
        assert_eq!(status, 200, "the index answers its state: {body}");
        let document: Value = serde_json::from_str(&body).expect("the index body is valid JSON");
        document
            .get("state")
            .cloned()
            .unwrap_or_else(|| panic!("the index body carries its state: {body}"))
    }

    /// Status of `GET /v1/indexes/{index}/documents/{doc_id}`.
    fn document(&self, doc_id: &str) -> u16 {
        let (status, body) = self.request(
            "GET",
            &format!("/v1/indexes/{INDEX}/documents/{doc_id}"),
            None,
        );
        assert!(
            status == 200 || status == 404,
            "the document route answers `200` or `404`, got {status}: {body}"
        );
        status
    }

    /// Submits the batch directory, with or without `?wait=true`.
    fn submit_attach(&self, wait: bool) -> (u16, Value) {
        let path = if wait {
            format!("/v1/indexes/{INDEX}/batches?wait=true")
        } else {
            format!("/v1/indexes/{INDEX}/batches")
        };
        let body = json!({ "directory": BATCH }).to_string();
        let (status, body) = self.request("POST", &path, Some(&body));
        let job: Value = serde_json::from_str(&body)
            .unwrap_or_else(|error| panic!("the job body is valid JSON: {error}: {body}"));
        (status, job)
    }

    /// `GET /v1/jobs/{job_id}`.
    fn job(&self, id: &str) -> Value {
        let (status, body) = self.request("GET", &format!("/v1/jobs/{id}"), None);
        assert_eq!(status, 200, "the job is readable after the restart: {body}");
        serde_json::from_str(&body).unwrap_or_else(|error| panic!("the job body: {error}: {body}"))
    }

    /// Polls the job until it is terminal, and returns it.
    fn wait_terminal(&self, id: &str) -> Value {
        let deadline = Instant::now() + JOB_PATIENCE;
        loop {
            let job = self.job(id);
            let state = job["state"].as_str().unwrap_or("");
            if state == "succeeded" || state == "failed" {
                return job;
            }
            assert!(
                Instant::now() < deadline,
                "the job is not terminal within {JOB_PATIENCE:?}: {job}"
            );
            sleep(POLL);
        }
    }

    /// Resubmits the batch directory and waits for the job to end.
    ///
    /// `?wait=true` is bounded by `[limits].wait_timeout_ms` (ADR-0067): the
    /// fixture of `inv_04` waits one second, less than the attach, so the
    /// answer is a `202` with the job still running and the test polls it.
    fn replay(&self) -> Value {
        let (status, job) = self.submit_attach(true);
        assert!(
            status == 200 || status == 202,
            "the replay is accepted or finishes within the wait: {job}"
        );
        if job["state"] == "succeeded" || job["state"] == "failed" {
            return job;
        }
        self.wait_terminal(&job_id(&job))
    }

    /// Polls the job until it is `running`: the state the stop must find.
    ///
    /// A queued job is fine; a terminal one is a failure: the attach ended
    /// before the stop, so the test would prove nothing.
    ///
    /// The `progress` member is never published by this build — no job calls
    /// `JobProgress::set`, so the proof rests on the `running` state alone,
    /// which is the one `docs/phases/P8-docker-et-deploiement.md` names.
    fn wait_running(&self, id: &str) {
        let deadline = Instant::now() + JOB_PATIENCE;
        loop {
            let job = self.job(id);
            let state = job["state"]
                .as_str()
                .unwrap_or_else(|| panic!("the job carries a state: {job}"))
                .to_owned();
            if state == "running" {
                return;
            }
            assert!(
                state == "queued",
                "the attach must still be running when the stop lands, it is \
                 '{state}': enlarge the batch of the test\n{}",
                self.journal()
            );
            assert!(
                Instant::now() < deadline,
                "the attach is not running within {JOB_PATIENCE:?}: {job}"
            );
            sleep(POLL);
        }
    }

    /// `docker stop -t 65`, the clean stop of `docker/README.md`.
    fn stop(&self) {
        docker_ok(&["stop", "-t", STOP_TIMEOUT_S, &self.container]);
    }

    /// Starts the container again and waits for `/ready`.
    ///
    /// Docker may publish another host port for a random mapping on `start`,
    /// so the port is read again before any request.
    fn start_again(&self) {
        docker_ok(&["start", &self.container]);
        self.port.set(host_port(&self.container));
        self.wait_ready();
    }

    /// Copies `/data` of the container to `<root>/<label>`, and returns it.
    ///
    /// It works on a stopped container, which is how the analytics are read
    /// before the restart.
    fn copy_data(&self, label: &str) -> PathBuf {
        let destination = self.root.path().join(label);
        fs::create_dir_all(&destination).expect("the copy destination is creatable");
        docker_ok(&[
            "cp",
            &format!("{}:/data/.", self.container),
            path_str(&destination),
        ]);
        destination
    }

    /// The journal of the container, for the message of a failure.
    fn journal(&self) -> String {
        let output = Command::new("docker")
            .args(["logs", "--tail", "60", &self.container])
            .stdin(Stdio::null())
            .output();
        match output {
            Ok(output) => format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
            Err(error) => format!("the journal is unreadable: {error}"),
        }
    }

    /// Sends one request to the instance and returns the status and the body.
    fn request(&self, method: &str, path: &str, body: Option<&str>) -> (u16, String) {
        self.try_request_with(method, path, body, Duration::from_secs(120))
            .unwrap_or_else(|| {
                panic!(
                    "the instance must answer {method} {path} on port {}",
                    self.port.get()
                )
            })
    }

    /// Same as [`Lab::request`], `None` when the connection fails.
    fn try_request(&self, method: &str, path: &str, body: Option<&str>) -> Option<(u16, String)> {
        self.try_request_with(method, path, body, Duration::from_secs(5))
    }

    fn try_request_with(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
        timeout: Duration,
    ) -> Option<(u16, String)> {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, self.port.get()));
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5)).ok()?;
        stream.set_read_timeout(Some(timeout)).ok()?;
        stream.set_write_timeout(Some(timeout)).ok()?;
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\
             Authorization: Bearer {SECRET}\r\nConnection: close\r\n"
        );
        if let Some(body) = body {
            head.push_str("Content-Type: application/json\r\n");
            head.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        head.push_str("\r\n");
        stream.write_all(head.as_bytes()).ok()?;
        if let Some(body) = body {
            stream.write_all(body.as_bytes()).ok()?;
        }
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).ok()?;
        Some(parse_response(&raw))
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "-f", &self.container])
            .stdin(Stdio::null())
            .output();
        // Docker removes the container asynchronously: the volume stays busy
        // for a moment. A bounded retry frees it, so a run leaves neither a
        // container nor a volume behind.
        for _ in 0..25 {
            let removed = Command::new("docker")
                .args(["volume", "rm", &self.volume])
                .stdin(Stdio::null())
                .output()
                .is_ok_and(|output| output.status.success());
            if removed {
                return;
            }
            sleep(Duration::from_millis(200));
        }
    }
}

// --- The fixture: configuration, batch, vectors ---

/// Image of the test: `CRIBLE_DOCKER_IMAGE`, `crible:dev` by default.
fn image() -> String {
    std::env::var("CRIBLE_DOCKER_IMAGE").unwrap_or_else(|_| "crible:dev".to_owned())
}

/// Runs `docker`, and fails with the command to produce its prerequisite.
fn docker_output(args: &[&str]) -> Output {
    Command::new("docker")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "`docker {}` must run: {error}\n\
                 the test needs Docker and the image '{}', built with:\n  \
                 docker buildx build --platform linux/arm64 --progress=plain --load \
                 -t crible:dev -f docker/Dockerfile .",
                args.join(" "),
                image()
            )
        })
}

/// Runs `docker` and returns its standard output.
fn docker_text(args: &[&str]) -> String {
    let output = docker_output(args);
    assert!(
        output.status.success(),
        "`docker {}` must succeed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Runs `docker` and fails when it returns a non-zero status.
///
/// The message carries the build command of the image (ADR-0057: a test whose
/// prerequisite is missing fails, and says how to produce it).
fn docker_ok(args: &[&str]) -> Output {
    let output = docker_output(args);
    assert!(
        output.status.success(),
        "`docker {}` must succeed: {}{}\n\
         the test needs the image '{}', built with:\n  \
         docker buildx build --platform linux/arm64 --progress=plain --load \
         -t crible:dev -f docker/Dockerfile .",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        image()
    );
    output
}

/// Host port Docker published for `8080/tcp` of the container.
fn host_port(container: &str) -> u16 {
    let text = docker_text(&["port", container, "8080/tcp"]);
    text.trim()
        .rsplit(':')
        .next()
        .unwrap_or_else(|| panic!("`docker port` names a port: {text:?}"))
        .parse()
        .unwrap_or_else(|error| panic!("`docker port` prints a port: {text:?}: {error}"))
}

/// `<path>` as an `&str`, for the arguments of `docker`.
fn path_str(path: &Path) -> &str {
    path.to_str()
        .unwrap_or_else(|| panic!("{} must be valid UTF-8", path.display()))
}

/// Makes `directory` traversable and readable by uid 1000.
#[cfg(unix)]
fn make_traversable(directory: &Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(directory, fs::Permissions::from_mode(0o755))
        .unwrap_or_else(|error| panic!("{} must be traversable: {error}", directory.display()));
}

/// Windows has no mode bit, and the image is a Linux one.
#[cfg(not(unix))]
fn make_traversable(_directory: &Path) {}

/// The configuration of `docker/crible.example.toml`, with the paths of the
/// container and one `write` key, so the test can submit a batch.
fn config(wait_timeout_ms: u64) -> String {
    format!(
        r#"
[server]
bind = "0.0.0.0:8080"
search_threads = 0
max_concurrent_searches = 0

[paths]
data = "/data"
models = "/models"
exchange = "/exchange"

[[auth.keys]]
name = "tests"
role = "write"
sha256 = "{digest}"

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
wait_timeout_ms = {wait_timeout_ms}
query_cache_entries = 1024
"#,
        digest = crible_server::auth::sha256_hex(SECRET),
    )
}

/// Configuration of an index at an `external` model, without a lexical branch.
///
/// The same shape as the fixture of `crates/crible/tests/serve_cli.rs`: the
/// loader reads it without ONNX Runtime, and the attach of a batch of vectors
/// needs no encoder.
fn index_config() -> String {
    format!(
        r#"{{"name":"{INDEX}","model":"{MODEL}","reference_precision":"fp16",
 "schema":{{"document_fields":{{}},"passage_fields":{{}}}},"cluster_by":[],
 "lexical":{{"enabled":false,"language":"fr","positions":false}},
 "search":{{"scan":"none","rescore_candidates":10,"exact_threshold":10,
           "candidates_per_branch":10,"fusion":{{"method":"rrf"}}}},
 "ingest":{{"acceptance_sample":1,"acceptance_min_cosine":0.9}},
 "compaction":{{"dead_ratio":0.1}}}}
"#
    )
}

/// Schema of `documents.parquet` for an index without document fields.
fn documents_schema() -> SchemaRef {
    Arc::new(ArrowSchema::new(vec![
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("doc_hash", DataType::Utf8, false),
    ]))
}

/// Schema of `passages.parquet` for an index without passage fields.
fn passages_schema() -> SchemaRef {
    Arc::new(ArrowSchema::new(vec![
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("ord", DataType::UInt32, false),
        Field::new("text", DataType::Utf8, false),
        Field::new(
            "vector",
            DataType::FixedSizeBinary(DIMENSIONS as i32 * 2),
            false,
        ),
    ]))
}

/// `[1, 0, …, 0]` in fp32 little-endian: the query, and every passage vector.
///
/// It is normalized (`|norm - 1| = 0`), which the batch reader checks, and
/// deterministic: the test never draws a random number.
fn unit_vector() -> Vec<u8> {
    let mut bytes = vec![0_u8; DIMENSIONS * 4];
    bytes[0..4].copy_from_slice(&1.0_f32.to_le_bytes());
    bytes
}

/// `[1, 0, …, 0]` in fp16 little-endian, the stored vector of every passage.
fn unit_vector_fp16() -> Vec<u8> {
    let mut bytes = vec![0_u8; DIMENSIONS * 2];
    bytes[0..2].copy_from_slice(&f16::from_f32(1.0).to_bits().to_le_bytes());
    bytes
}

/// Writes a batch of [`DOCUMENTS`] documents, one passage each, and its manifest.
///
/// The format is the one of `docs/format-lot-v1.md`: the checksums of the two
/// Parquet files are blake3, the counts match the rows, and the vectors are
/// declared normalized. The Parquet writer is the shared helper of the test
/// binaries of this crate.
fn write_batch(directory: &Path) {
    fs::create_dir_all(directory).expect("the batch directory is creatable");
    let ids: Vec<String> = (0..DOCUMENTS).map(|n| format!("doc-{n:07}")).collect();

    let documents: Vec<String> = ids.clone();
    let hashes: Vec<String> = ids.iter().map(|id| format!("blake3:{id}")).collect();
    common::write_parquet(
        &directory.join("documents.parquet"),
        &documents_schema(),
        vec![
            Arc::new(
                ids.iter()
                    .map(|id| Some(id.as_str()))
                    .collect::<StringArray>(),
            ),
            Arc::new(
                hashes
                    .iter()
                    .map(|hash| Some(hash.as_str()))
                    .collect::<StringArray>(),
            ),
        ],
    );

    let passages: Vec<String> = ids.iter().map(|id| format!("passage of {id}")).collect();
    let ords = UInt32Array::from(vec![0_u32; DOCUMENTS]);
    let vectors = FixedSizeBinaryArray::try_from_iter((0..DOCUMENTS).map(|_| unit_vector_fp16()))
        .expect("the passage vectors share one width");
    common::write_parquet(
        &directory.join("passages.parquet"),
        &passages_schema(),
        vec![
            Arc::new(
                documents
                    .iter()
                    .map(|id| Some(id.as_str()))
                    .collect::<StringArray>(),
            ),
            Arc::new(ords),
            Arc::new(
                passages
                    .iter()
                    .map(|text| Some(text.as_str()))
                    .collect::<StringArray>(),
            ),
            Arc::new(vectors),
        ],
    );

    let mut files = BTreeMap::new();
    for name in ["documents.parquet", "passages.parquet"] {
        let bytes = fs::read(directory.join(name))
            .unwrap_or_else(|error| panic!("the batch file {name} is readable: {error}"));
        files.insert(
            name.to_owned(),
            format!("blake3:{}", blake3::hash(&bytes).to_hex()),
        );
    }
    let manifest = json!({
        "format": "crible-batch",
        "format_version": 1,
        "batch_id": BATCH,
        "index": INDEX,
        "mode": "upsert",
        "created_at": CREATED_AT,
        "producer": { "name": "verifier-arret-propre", "version": "1", "host": "docker-stop" },
        "model": { "dims": DIMENSIONS, "normalize": true, "vector_dtype": "fp16" },
        "chunker": { "name": "one-passage", "version": "1", "params": {} },
        "template": { "name": "plain", "version": "1" },
        "counts": { "documents": DOCUMENTS, "passages": DOCUMENTS, "deletions": 0 },
        "files": files,
    });
    fs::write(
        directory.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).expect("the manifest is serializable"),
    )
    .expect("manifest.json is written");
}

/// Id of the job of a submitted batch.
fn job_id(job: &Value) -> String {
    job["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the job carries an id: {job}"))
        .to_owned()
}

/// Sum of the search events the analytics file of a copied `/data` holds.
fn searches_in(data: &Path) -> u64 {
    let path = data.join(ANALYTICS_FILE_NAME);
    // Before the first flush of ADR-0118 the file may not exist yet, and then no
    // event is on disk. After the stop the file exists, and the caller asserts
    // its content: a zero here would fail loudly.
    if !path.exists() {
        return 0;
    }
    let rows: Vec<MinuteEvent> = read_minute_events(&path)
        .unwrap_or_else(|error| panic!("{} is readable: {error}", path.display()));
    rows.iter()
        .filter(|row| row.index == INDEX && row.operation == "search")
        .map(|row| row.count)
        .sum()
}

/// Every entry of `root` whose name starts with `tmp-`, as a path relative to
/// `root`, so two copies of the same `/data` compare equal.
///
/// The rule is the recovery at startup of `docs/02-architecture.md:144-148`: an
/// opening in write mode removes `segments/tmp-*`, an unreferenced segment
/// directory, the residual `tmp-*` of the index directory and of `manifests/`,
/// then the `online-*` and `deleted-*` directories of `<data>/tmp/`. The
/// `return` of `docs/04-donnees-et-ingestion.md:253` covers only the
/// `tmp/online-*` directories, not these.
///
/// Two limits, deliberate here: the sweep also covers `epochs/`, which the
/// recovery does **not** clean (`docs/02-architecture.md:148`: a
/// `epochs/tmp-*` left by an interrupted compaction stays, inert), and it skips
/// `<data>/tmp/` entirely, whose names do not carry the prefix. Neither matters
/// for an attach, which writes no epoch directory; a compaction test would have
/// to bound the sweep.
fn temporary_entries(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("tmp-") {
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(path.as_path())
                    .display()
                    .to_string();
                found.push(relative);
            }
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Status and body of one HTTP/1.1 answer, `Connection: close` asked for.
fn parse_response(raw: &[u8]) -> (u16, String) {
    let separator = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap_or_else(|| {
            panic!(
                "the answer carries a header block: {:?}",
                String::from_utf8_lossy(raw)
            )
        });
    let head = String::from_utf8_lossy(&raw[..separator]).into_owned();
    let body = &raw[separator + 4..];

    let mut lines = head.lines();
    let status_line = lines
        .next()
        .unwrap_or_else(|| panic!("the answer starts with a status line: {head}"));
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .unwrap_or_else(|| panic!("the status line carries a code: {status_line}"))
        .parse()
        .unwrap_or_else(|error| panic!("the status line carries a code: {status_line}: {error}"));

    let chunked = lines.any(|line| {
        line.to_ascii_lowercase().starts_with("transfer-encoding:")
            && line.to_ascii_lowercase().contains("chunked")
    });
    let body = if chunked {
        dechunk(body)
    } else {
        body.to_vec()
    };
    (
        status,
        String::from_utf8(body).unwrap_or_else(|error| panic!("the body is UTF-8: {error}")),
    )
}

/// Decodes the chunked body of an HTTP/1.1 answer.
fn dechunk(mut body: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::new();
    loop {
        let end = body
            .windows(2)
            .position(|window| window == b"\r\n")
            .unwrap_or_else(|| {
                panic!(
                    "a chunk carries its size: {:?}",
                    String::from_utf8_lossy(body)
                )
            });
        let size = std::str::from_utf8(&body[..end])
            .unwrap_or_else(|error| panic!("a chunk size is ASCII: {error}"));
        let size = usize::from_str_radix(size.split(';').next().unwrap_or(size).trim(), 16)
            .unwrap_or_else(|error| panic!("a chunk size is hexadecimal: {size:?}: {error}"));
        body = &body[end + 2..];
        if size == 0 {
            return decoded;
        }
        decoded.extend_from_slice(&body[..size]);
        body = &body[size + 2..];
    }
}
