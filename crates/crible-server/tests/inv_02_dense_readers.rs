// SPDX-License-Identifier: Apache-2.0

// The tests of this file hold a write job with the test barrier of the
// `failpoints` build (`src/barriers.rs`, complement of 2026-10-29 to ADR-0068,
// decision 2) and read the segment reader hook of the same build
// (`crible_store::testhooks`, ADR-0126, decision 6), so a plain `cargo test`
// compiles no test here.
#![cfg(feature = "failpoints")]
// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the checks of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! INV-02 with the dense reader cache — a read is never blocked by a write, and
//! a served generation is never served without its readers
//! (`docs/08-invariants-et-tests.md:10`; ADR-0113, decision 1; ADR-0126,
//! decision 6).
//!
//! The criterion is the one of `docs/08`, not a p99 against a p99: during each
//! writing phase, **every** dense search completes in less than three times the
//! p99 of the rest phase (ADR-0113, decision 1). The rest phase and the writing
//! phase use the same load, `CLIENTS` closed-loop clients (ADR-0113,
//! decision 5), so the two sides are comparable.
//!
//! Each test also proves the second half of decision 6 of ADR-0126: a search of
//! a generation already published **opens no segment reader**. The proof rests
//! on the observable hook of `crible_store::testhooks`, never on a reading of
//! the code: the number of `open` events does not move while the searches run,
//! and yet they return a scored result — a dense score cannot be computed
//! without the reference vectors, so the readers were already open.
//!
//! | Test | Hold point | What it proves |
//! |---|---|---|
//! | `inv_02_dense_searches_stay_under_the_criterion_while_an_attach_is_held` | attach, before the SQLite `COMMIT` | per-request criterion, previous generation, no reader opened |
//! | `inv_02_dense_searches_stay_under_the_criterion_while_a_compaction_is_held` | compaction, after the epoch switch | same, on the epoch the compaction replaces |
//!
//! ```text
//! cargo test -p crible-server --features failpoints --test inv_02_dense_readers
//! ```

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::state::AppState;
use crible_store::testhooks::{ReaderEvent, SEGMENT_READERS_ENV, read_events, reset_events};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use common::{
    ADMIN_SECRET, DOC_ID, INDEX, READ_SECRET, WRITE_SECRET, base64_vector, instance_with_alpha,
    send, terminal_job,
};

/// Environment variable of the test barrier (`src/barriers.rs`).
const BARRIER_ENV: &str = "CRIBLE_TEST_BARRIER";

/// Barrier of the attach write, just before the SQLite `COMMIT`.
const ATTACH_BEFORE_COMMIT: &str = "attach-before-commit";

/// Barrier of the compaction, after the epoch switch and before the publication
/// (`src/maintenance.rs`).
const COMPACTION: &str = "compaction";

/// Path of the search route of the fixture index.
const SEARCH: &str = "/v1/indexes/alpha/search";

/// Longest wait of a test for a barrier to be reached.
///
/// It is a deadlock watchdog, never a synchronisation delay: a missing barrier
/// only costs this wait, and the test then fails with the name of the point it
/// expected.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Granularity of the polling loop, in milliseconds.
const POLL: Duration = Duration::from_millis(5);

/// Vector of the fixture document, and of every query of this file.
const VECTOR: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

/// Text of the document before the held write.
const BEFORE: &str = "le contrat fait la loi";

/// Text of the document the held write publishes.
const AFTER: &str = "le contrat est puni";

/// Number of closed-loop clients of the load (ADR-0113, decision 5).
///
/// It stays below the number of cores of the CI runners, so the criterion
/// measures the effect of the write, never a saturated processor.
const CLIENTS: usize = 4;

/// Rounds of each client in the rest phase: 200 samples, so the p99 is the
/// 198th of them (nearest rank, ADR-0113, decision 5).
const REST_ROUNDS: usize = 50;

/// Rounds of each client in a writing phase: 40 samples, enough for the
/// criterion to see one blocked read.
const HELD_ROUNDS: usize = 10;

/// Serialises the access to the process-global `CRIBLE_TEST_BARRIER` and to the
/// process-global `CRIBLE_TEST_SEGMENT_READERS`.
///
/// Both variables name **one** directory per process, and the jobs and the
/// readers of a test read them, so this binary runs one test at a time: each
/// test takes the slot [`lock_environment`] returns **before** it builds its
/// instance and holds it until its end. The guard is a `tokio` one, so it is
/// held across the `await`s of a test without blocking a runtime thread.
static ENVIRONMENT: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// The process-wide slot of a test, taken before its instance exists.
///
/// It also arms the segment reader hook: the log file lives in the temporary
/// directory of the slot, so a test never reads the events of a neighbour.
struct Slot {
    directory: tempfile::TempDir,
    _serial: tokio::sync::MutexGuard<'static, ()>,
}

/// Takes the process-wide slot of the test barrier and of the reader hook.
async fn lock_environment() -> Slot {
    // The order matters: the guard is taken first, so no other test of this
    // binary writes the variables or the log while they are armed.
    let serial = ENVIRONMENT.lock().await;
    let directory = tempfile::tempdir().expect("a temporary directory is creatable");
    // SAFETY: the guard is held, and no other test of this binary runs: no
    // reader reads the variable between the write and the end of the test, and
    // `Drop` removes it before the guard is released.
    unsafe { std::env::set_var(SEGMENT_READERS_ENV, log_path(directory.path())) };
    Slot {
        directory,
        _serial: serial,
    }
}

/// Path of the log of the segment reader hook, in `directory`.
fn log_path(directory: &Path) -> PathBuf {
    directory.join("segment-readers.log")
}

impl Slot {
    /// Every event the segment reader hook recorded since the last reset.
    fn events(&self) -> Vec<ReaderEvent> {
        let path = log_path(self.directory.path());
        read_events(&path).unwrap_or_else(|error| {
            panic!(
                "the segment reader log {} is readable: {error}",
                path.display()
            )
        })
    }

    /// Forgets the events recorded so far.
    fn reset(&self) {
        let path = log_path(self.directory.path());
        reset_events(&path).unwrap_or_else(|error| {
            panic!(
                "the segment reader log {} is resettable: {error}",
                path.display()
            )
        });
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        // SAFETY: the guard is still held here, so no reader of another test
        // reads the variable while it is removed.
        unsafe { std::env::remove_var(SEGMENT_READERS_ENV) };
    }
}

/// A barrier directory armed for one test, released on drop.
struct Barrier {
    directory: tempfile::TempDir,
    slot: Slot,
}

impl Barrier {
    /// Arms the barrier directory, in the slot `slot` of the test.
    ///
    /// `released` names the write steps this test does **not** want to hold;
    /// they are released up front, as `inv_02_reads_during_writes.rs` explains.
    ///
    /// It is called **after** the fixture is written, because every attach
    /// submitted while the barrier is armed stops at the held step.
    fn armed(slot: Slot, released: &[&str]) -> Self {
        let directory = tempfile::tempdir().expect("a temporary barrier directory is creatable");
        // SAFETY: `slot` holds the process-wide guard of [`ENVIRONMENT`], taken
        // at the start of the test: no other test of this binary runs, so no
        // job reads the variable while it is set, and `Drop` removes it before
        // the guard is released.
        unsafe { std::env::set_var(BARRIER_ENV, directory.path()) };
        // The attach steps are opt-in (`src/barriers.rs`): a test that arms
        // them writes `armed-attach`, so an attach of one of the other server
        // tests is never held.
        std::fs::write(directory.path().join("armed-attach"), b"")
            .expect("the attach steps are armable");
        for scope in ["batch"].iter().chain(released) {
            let go = directory.path().join(format!("go-{scope}"));
            std::fs::write(&go, b"").expect("an unrelated barrier is releasable");
        }
        Self { directory, slot }
    }

    /// `<dir>/ready-<scope>`: the job reached the barrier.
    fn ready(&self, scope: &str) -> PathBuf {
        self.directory.path().join(format!("ready-{scope}"))
    }

    /// `<dir>/go-<scope>`: the job may go on.
    fn release(&self, scope: &str) {
        let go = self.directory.path().join(format!("go-{scope}"));
        std::fs::write(&go, b"")
            .unwrap_or_else(|error| panic!("the barrier {} is releasable: {error}", go.display()));
    }

    /// Waits for the job to reach the barrier of `scope`.
    async fn wait_ready(&self, scope: &str) {
        let ready = self.ready(scope);
        let deadline = Instant::now() + TIMEOUT;
        while !ready.exists() {
            assert!(
                Instant::now() < deadline,
                "the job never reached the barrier '{scope}' at {}",
                ready.display()
            );
            tokio::time::sleep(POLL).await;
        }
    }

    /// Every event the segment reader hook recorded since the last reset.
    fn events(&self) -> Vec<ReaderEvent> {
        self.slot.events()
    }

    /// Number of `open` events recorded since the last reset.
    fn opens(&self) -> usize {
        self.events()
            .iter()
            .filter(|event| matches!(event, ReaderEvent::Open { .. }))
            .count()
    }
}

impl Drop for Barrier {
    fn drop(&mut self) {
        // SAFETY: see `Barrier::armed`; the slot of the test is still held here.
        unsafe { std::env::remove_var(BARRIER_ENV) };
    }
}

/// Body of an online batch that upserts the fixture document.
fn upsert(batch_id: &str, text: &str, numero: &str) -> Value {
    json!({
        "batch_id": batch_id,
        "mode": "upsert",
        "documents": [{
            "doc_id": DOC_ID,
            "fields": { "numero": numero },
            "passages": [{
                "text": text,
                "fields": { "texte": "contrat" },
                "vector": base64_vector(VECTOR),
            }]
        }]
    })
}

/// Attaches one online batch and returns the generation it published.
async fn attach(router: &Router, state: &Arc<AppState>, body: Value) -> u64 {
    let (status, accepted) = common::post_documents(router, body).await;
    assert_eq!(status, StatusCode::OK, "the batch is accepted: {accepted}");
    let job = terminal_job(router, state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the batch attaches: {job}");
    job["generation"]
        .as_u64()
        .unwrap_or_else(|| panic!("the attach publishes a generation: {job}"))
}

/// Submits one online batch without waiting for its job.
async fn submit(router: &Router, body: Value) -> Value {
    let path = format!("/v1/indexes/{INDEX}/documents?wait=false");
    let (status, accepted) = send(router, Method::POST, &path, WRITE_SECRET, Some(body)).await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "the online batch is queued: {accepted}"
    );
    accepted
}

/// Sends a dense search and asserts it answers `200` (INV-02).
async fn search(router: &Router) -> Value {
    let (status, body) = send(
        router,
        Method::POST,
        SEARCH,
        READ_SECRET,
        Some(json!({
            "vector": base64_vector(VECTOR),
            "mode": "dense",
            "k": 10,
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the dense search completes while the write is held (INV-02): {body}"
    );
    body
}

/// The generation of an answer.
fn generation(answer: &Value) -> u64 {
    answer["generation"]
        .as_u64()
        .unwrap_or_else(|| panic!("the answer carries its generation: {answer}"))
}

/// The text of the first result of an answer.
fn first_text(answer: &Value) -> &str {
    answer["results"]
        .as_array()
        .and_then(|results| results.first())
        .and_then(|first| first["text"].as_str())
        .unwrap_or_else(|| panic!("the answer carries one result with its text: {answer}"))
}

/// Asserts that an answer really scored a passage.
///
/// A dense score cannot be computed without the reference vectors of the
/// segment, so a scored result proves the search read them. Combined with a
/// constant number of `open` events, it proves they came from the cache of the
/// published generation (ADR-0126, decisions 1 and 6).
fn assert_scored(answer: &Value) {
    let score = answer["results"]
        .as_array()
        .and_then(|results| results.first())
        .and_then(|first| first["score"].as_f64())
        .unwrap_or_else(|| {
            panic!("a dense answer carries the score of its first result: {answer}")
        });
    assert!(
        score > 0.99,
        "the query vector is the vector of the fixture passage, so the cosine is \
         1 and the search really read the reference vectors: {answer}"
    );
}

/// Sends `rounds` dense searches from each of [`CLIENTS`] clients, in closed
/// loop, and returns their wall-clock latencies and the last answer.
///
/// The latency is measured at the wall clock of the client (ADR-0113,
/// decision 5): it covers the whole request, the wait for a concurrency token
/// included.
async fn concurrent_searches(router: &Router, rounds: usize) -> (Vec<Duration>, Value) {
    let mut tasks = Vec::with_capacity(CLIENTS);
    for _ in 0..CLIENTS {
        let router = router.clone();
        tasks.push(tokio::spawn(async move {
            let mut samples = Vec::with_capacity(rounds);
            let mut last = Value::Null;
            for _ in 0..rounds {
                let started = Instant::now();
                last = search(&router).await;
                samples.push(started.elapsed());
            }
            (samples, last)
        }));
    }
    let mut samples = Vec::with_capacity(CLIENTS * rounds);
    let mut last = Value::Null;
    for task in tasks {
        let (mut taken, answer) = task
            .await
            .unwrap_or_else(|error| panic!("a search client ended without panicking: {error}"));
        samples.append(&mut taken);
        last = answer;
    }
    (samples, last)
}

/// The p99 of `samples`, at the nearest rank (ADR-0113, decision 5; ADR-0077).
fn p99(samples: &mut [Duration]) -> Duration {
    samples.sort_unstable();
    let count = samples.len();
    assert!(count > 0, "the rest phase collects at least one sample");
    let rank = ((count as f64) * 0.99).ceil() as usize;
    samples[rank.saturating_sub(1).min(count - 1)]
}

/// Asserts that every latency of the writing phase is under `reference`.
fn assert_all_under(samples: &[Duration], reference: Duration, phase: &str) {
    for sample in samples {
        assert!(
            *sample < reference,
            "a dense search took {sample:?} during the {phase}, which is over \
             three times the p99 of the rest phase ({reference:?}): the write \
             blocks a read (INV-02; ADR-0113, decision 1)"
        );
    }
}

/// Dense searches run under the INV-02 criterion while an attach is held in its
/// write, on the previous generation, and open no reader.
///
/// INV-02 (ADR-0113, decision 1) and ADR-0126, decision 6: no reader is opened
/// on the path of a search of a generation already served.
#[tokio::test]
async fn inv_02_dense_searches_stay_under_the_criterion_while_an_attach_is_held() {
    let slot = lock_environment().await;
    let (_instance, state, router) = instance_with_alpha().await;
    let published = attach(&router, &state, upsert("batch-before", BEFORE, "1240")).await;

    // Rest phase: the reference of the criterion, on the same load.
    slot.reset();
    let (mut rest, answer) = concurrent_searches(&router, REST_ROUNDS).await;
    assert_eq!(
        generation(&answer),
        published,
        "the rest phase reads the published generation: {answer}"
    );
    assert_scored(&answer);
    assert!(
        slot.events().is_empty(),
        "a search of the rest phase opened a segment reader: {:?} (ADR-0126, decision 1)",
        slot.events()
    );
    let reference = p99(&mut rest) * 3;

    let barrier = Barrier::armed(slot, &["attach-after-sqlite-commit"]);
    let accepted = submit(&router, upsert("batch-held", AFTER, "1241")).await;
    barrier.wait_ready(ATTACH_BEFORE_COMMIT).await;
    let opened_before = barrier.opens();

    // The transaction of the attach is open and not committed: every search
    // must stay under the criterion, see the previous generation and score with
    // the readers that generation already holds.
    let (during, held) = concurrent_searches(&router, HELD_ROUNDS).await;
    assert_all_under(&during, reference, "attache retenue");
    assert_eq!(
        generation(&held),
        published,
        "a search during a held attach reads the previous generation: {held}"
    );
    assert_eq!(
        first_text(&held),
        BEFORE,
        "a search during a held attach reads the passages of the previous \
         generation: {held}"
    );
    assert_scored(&held);
    assert_eq!(
        barrier.opens(),
        opened_before,
        "a search of a generation already served opened a segment reader \
         (ADR-0126, decision 6): {:?}",
        barrier.events()
    );

    barrier.release(ATTACH_BEFORE_COMMIT);
    let job = terminal_job(&router, &state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the attach ends: {job}");
    assert_eq!(
        job["generation"],
        json!(published + 1),
        "the released attach publishes the next generation: {job}"
    );

    let after = search(&router).await;
    assert_eq!(
        generation(&after),
        published + 1,
        "a search after the release sees the new generation: {after}"
    );
    assert_eq!(
        first_text(&after),
        AFTER,
        "a search after the release sees the new passages: {after}"
    );
}

/// Same proof while a compaction is held after its epoch switch: the reads keep
/// the epoch the compaction replaces, complete under the criterion, and open no
/// reader.
///
/// INV-02, ADR-0017 ("les lectures continuent normalement sur l'ancienne
/// époque, jusqu'à la bascule") and ADR-0126, decision 6.
#[tokio::test]
async fn inv_02_dense_searches_stay_under_the_criterion_while_a_compaction_is_held() {
    let slot = lock_environment().await;
    let (_instance, state, router) = instance_with_alpha().await;
    let published = attach(&router, &state, upsert("batch-before", BEFORE, "1240")).await;

    slot.reset();
    let (mut rest, answer) = concurrent_searches(&router, REST_ROUNDS).await;
    assert_eq!(
        generation(&answer),
        published,
        "the rest phase reads the published generation: {answer}"
    );
    assert!(slot.events().is_empty(), "the rest phase opens no reader");
    let reference = p99(&mut rest) * 3;

    let barrier = Barrier::armed(slot, &[]);
    let path = format!("/v1/indexes/{INDEX}/compact?wait=false");
    let (status, accepted) = send(&router, Method::POST, &path, ADMIN_SECRET, None).await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "the compaction is queued: {accepted}"
    );
    barrier.wait_ready(COMPACTION).await;
    let opened_before = barrier.opens();

    // `CURRENT_EPOCH` names E+1 and its generation is G+1, while the writer
    // still publishes the snapshot of E: the searches must answer G, with the
    // readers of E.
    let (during, held) = concurrent_searches(&router, HELD_ROUNDS).await;
    assert_all_under(&during, reference, "compaction retenue");
    assert_eq!(
        generation(&held),
        published,
        "a search during a held compaction reads the published generation: {held}"
    );
    assert_eq!(
        first_text(&held),
        BEFORE,
        "a search during a held compaction reads the passages of the old epoch: {held}"
    );
    assert_scored(&held);
    assert_eq!(
        barrier.opens(),
        opened_before,
        "a search of a generation already served opened a segment reader \
         (ADR-0126, decision 6): {:?}",
        barrier.events()
    );

    barrier.release(COMPACTION);
    let job = terminal_job(&router, &state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the compaction ends: {job}");
    assert_eq!(
        job["generation"],
        json!(published + 1),
        "the released compaction publishes the next generation: {job}"
    );

    let after = search(&router).await;
    assert_eq!(
        generation(&after),
        published + 1,
        "a search after the compaction sees the new generation: {after}"
    );
    assert_eq!(
        first_text(&after),
        BEFORE,
        "a compaction keeps the results of the generation it replaces (INV-09): {after}"
    );
}
