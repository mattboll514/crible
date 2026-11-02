// SPDX-License-Identifier: Apache-2.0

// The tests of this file hold the analytics writer with the test barrier of the
// `failpoints` build (`src/barriers.rs`, complement of 2026-10-29 to ADR-0068,
// decision 2), which exists only with the server's `failpoints` feature, so a
// plain `cargo test` compiles no test here.
#![cfg(feature = "failpoints")]
// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the checks of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! INV-17 — writing the analytics never blocks a request, proved at the HTTP
//! level (`docs/08-invariants-et-tests.md:25`; ADR-0118, decision 7).
//!
//! The analytics writer is held **at the head of its loop, before it receives
//! anything** (`analytics-flush` barrier). While it is held, the bounded channel
//! of ADR-0118, decision 2 fills; the tests then prove three things:
//!
//! 1. every search still answers `200`, with the same body as a search sent
//!    before the channel was full (only `took_ms`, a duration, is left out:
//!    `docs/05-api.md`, "Recherche : réponse");
//! 2. every event the channel refused is counted in `analytics_dropped`, and
//!    nothing else is;
//! 3. once the barrier is released and a flush is forced, the aggregates of
//!    `analytics.sqlite` count **exactly** the events the channel accepted: a
//!    loss announced by the counter never appears in the aggregates, and the
//!    aggregates never lose an event the counter did not announce.
//!
//! No latency is measured and no threshold is asserted: the latency of a shared
//! runner is not reproducible (ADR-0113, `:94`). The tests prove that a search
//! **completes** and answers the same thing while the writing is held. MES-16
//! measures the percentiles of ADR-0113, decision 1 under load.
//!
//! | Test | What it proves |
//! |---|---|
//! | `inv_17_searches_answer_while_the_analytics_writer_is_held` | `200` and identical body during a full channel; `analytics_dropped` grows by exactly the overflow |
//! | `inv_17_accepted_events_are_all_written_after_the_release` | the aggregates equal the accepted events, and not one more |
//! | `inv_17_an_operation_event_keeps_its_answer_while_the_channel_is_full` | an `OpEvent` produced while the channel is full does not change the response |
//!
//! The barrier has its own scope, `analytics-flush`, and follows the protocol of
//! `src/barriers.rs`: `<dir>/ready-analytics-flush` then `<dir>/go-analytics-flush`,
//! an opt-in mark `<dir>/armed-analytics-flush` (so a test that arms another
//! scope never holds the analytics writer), and no barrier armed means a normal
//! run.
//!
//! ```text
//! cargo test -p crible-server --features failpoints --test inv_17_analytics_never_blocks
//! ```

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::analytics::{self, QUEUE_CAPACITY};
use crible_server::state::AppState;
use crible_store::analytics::{ANALYTICS_FILE_NAME, MinuteEvent, read_minute_events};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use common::{
    DOC_ID, INDEX, Instance, READ_SECRET, base64_vector, instance_with_alpha, send, terminal_job,
};

/// Environment variable of the test barrier (`src/barriers.rs`).
const BARRIER_ENV: &str = "CRIBLE_TEST_BARRIER";

/// Barrier of the analytics writer, at the head of its loop, before it receives
/// anything (ADR-0118, decision 7).
const ANALYTICS_FLUSH: &str = "analytics-flush";

/// Mark that arms the analytics barrier.
///
/// The analytics writer runs in every server test, so the hold point is opt-in:
/// without this mark a test that armed another scope runs the writer at full
/// speed, like the attach steps of `src/barriers.rs`.
const ANALYTICS_FLUSH_ARMED: &str = "armed-analytics-flush";

/// Path of the search route of the fixture index.
const SEARCH: &str = "/v1/indexes/alpha/search";

/// Number of searches sent **beyond** the capacity of the channel.
///
/// Three is enough to prove the counter: a test that overflowed by one could
/// pass on a writer that lost one event by accident, and a larger overflow makes
/// no other difference. The exact number is asserted, so a second loss anywhere
/// else fails the test.
const OVERFLOW: u64 = 3;

/// Longest wait of a test for the barrier to be reached.
///
/// It is a deadlock watchdog, never a synchronisation delay: a missing barrier
/// only costs this wait, and the test then fails with the name of the point it
/// expected.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Granularity of the polling loop, in milliseconds.
const POLL: Duration = Duration::from_millis(5);

/// Vector of the fixture document, and of every query of this file.
const VECTOR: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

/// Text of the fixture passage.
const TEXT: &str = "le contrat fait la loi";

/// Value of the document field `numero`, and of the filter of every search.
///
/// It is long and unique on purpose: the test scans the raw bytes of the file for
/// it, so a filter value that leaked into `analytics.sqlite` would be found
/// (ADR-0118, decision 7; ADR-0117, decision 5).
const FILTER_MARKER: &str = "marqueur-de-valeur-de-filtre-2f1c7d";

/// Serialises the access to the process-global `CRIBLE_TEST_BARRIER` and to the
/// process-global `analytics_dropped`.
///
/// The variable names **one** directory per process, and `analytics_dropped` is
/// a counter per process (ADR-0118, decision 2), so this binary runs one test at
/// a time: each test takes the slot [`lock_environment`] returns **before** it
/// builds its instance and holds it until its end. The guard is a `tokio` one,
/// so it is held across the `await`s of a test without blocking a runtime
/// thread.
static ENVIRONMENT: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// The process-wide slot of a test, taken before its instance exists.
struct Slot {
    _serial: tokio::sync::MutexGuard<'static, ()>,
}

/// Takes the process-wide slot of the test barrier and of the drop counter.
async fn lock_environment() -> Slot {
    Slot {
        _serial: ENVIRONMENT.lock().await,
    }
}

/// A barrier directory armed for one test, released on drop.
struct Barrier {
    directory: tempfile::TempDir,
    _slot: Slot,
}

impl Barrier {
    /// Arms the analytics barrier, in the slot `slot` of the test.
    ///
    /// It is called **after** the fixture is written: the writer of the fixture
    /// must not be held before the test has a stable starting point.
    fn armed(slot: Slot) -> Self {
        let directory = tempfile::tempdir().expect("a temporary barrier directory is creatable");
        // SAFETY: `slot` is the process-wide slot of [`ENVIRONMENT`], taken at
        // the start of the test; no other test of this binary runs, so no job
        // reads the variable while it is set, and `Drop` removes it before the
        // slot is released.
        unsafe { std::env::set_var(BARRIER_ENV, directory.path()) };
        // The hold point is opt-in (`src/barriers.rs`): a writer that only finds
        // the directory of a test holding another scope runs at full speed.
        std::fs::write(directory.path().join(ANALYTICS_FLUSH_ARMED), b"")
            .expect("the analytics barrier is armable");
        Self {
            directory,
            _slot: slot,
        }
    }

    /// `<dir>/ready-<scope>`: the writer reached the barrier.
    fn ready(&self, scope: &str) -> PathBuf {
        self.directory.path().join(format!("ready-{scope}"))
    }

    /// `<dir>/go-<scope>`: the writer may go on.
    fn release(&self, scope: &str) {
        let go = self.directory.path().join(format!("go-{scope}"));
        std::fs::write(&go, b"")
            .unwrap_or_else(|error| panic!("the barrier {} is releasable: {error}", go.display()));
    }

    /// Waits for the analytics writer to reach the barrier.
    async fn wait_ready(&self, scope: &str) {
        let ready = self.ready(scope);
        let deadline = Instant::now() + TIMEOUT;
        while !ready.exists() {
            assert!(
                Instant::now() < deadline,
                "the analytics writer never reached the barrier '{scope}' at {}",
                ready.display()
            );
            tokio::time::sleep(POLL).await;
        }
    }
}

impl Drop for Barrier {
    fn drop(&mut self) {
        // SAFETY: see `Barrier::armed`; the slot of the test is still held here.
        unsafe { std::env::remove_var(BARRIER_ENV) };
    }
}

/// Body of an online batch that upserts the fixture document.
fn upsert(batch_id: &str, text: &str) -> Value {
    json!({
        "batch_id": batch_id,
        "mode": "upsert",
        "documents": [{
            "doc_id": DOC_ID,
            "fields": { "numero": FILTER_MARKER },
            "passages": [{
                "text": text,
                "fields": { "texte": "contrat" },
                "vector": base64_vector(VECTOR),
            }]
        }]
    })
}

/// Attaches one online batch of the fixture document.
async fn attach(router: &Router, state: &Arc<AppState>, body: Value) {
    let (status, accepted) = common::post_documents(router, body).await;
    assert_eq!(status, StatusCode::OK, "the batch is accepted: {accepted}");
    let job = terminal_job(router, state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the batch attaches: {job}");
}

/// A fixture whose analytics channel is **empty** and whose writer is **idle**.
///
/// The fixture write is followed by a forced flush, so the events it produced
/// (`index_admin`, `attach_batch`) are written and no longer occupy a slot of
/// the channel: the capacity the test fills is the whole capacity.
async fn instance_with_an_empty_channel() -> (Instance, Arc<AppState>, Router) {
    let (instance, state, router) = instance_with_alpha().await;
    attach(&router, &state, upsert("batch-fixture", TEXT)).await;
    state
        .flush_analytics()
        .await
        .expect("the events of the fixture are flushed");
    (instance, state, router)
}

/// Sends one dense search and asserts it answers `200` (INV-17).
///
/// The search uses a provided vector, so the fixture `external` model needs no
/// encoder: the route does its own work and nothing else. Its filter matches the
/// only document of the index, so every search of the burst answers the same
/// body, and the value of that filter is a marker the file must never hold.
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
            "filter": { "numero": { "eq": FILTER_MARKER } },
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the search completes while the analytics writer is held (INV-17): {body}"
    );
    body
}

/// The body of an answer without its duration.
///
/// `took_ms` is the only field of `docs/05-api.md` that differs between two
/// identical searches; the protocol of ADR-0118, decision 7 names the keys, the
/// order, the scores and the `generation`, never the duration.
fn without_took_ms(body: &Value) -> Value {
    let mut body = body.clone();
    if let Some(object) = body.as_object_mut() {
        object.remove("took_ms");
    }
    body
}

/// Asserts that an answer carries the same body as the reference.
fn assert_same_answer(reference: &Value, answer: &Value, step: usize) {
    assert_eq!(
        without_took_ms(answer),
        without_took_ms(reference),
        "the analytics do not change an answer; search {step} differs (INV-17)"
    );
}

/// Sends one reference search, then the rest of the [`QUEUE_CAPACITY`] accepted
/// searches, then [`OVERFLOW`] refused ones, and asserts that every answer is
/// `200` and identical to the reference.
///
/// The reference is the first of the accepted events; the writer being held, the
/// channel accepts [`QUEUE_CAPACITY`] - 1 searches after it and refuses the rest.
async fn saturate(router: &Router) -> Value {
    let reference = search(router).await;
    let mut step = 1;
    for _ in 1..QUEUE_CAPACITY {
        let answer = search(router).await;
        assert_same_answer(&reference, &answer, step);
        step += 1;
    }
    for _ in 0..OVERFLOW {
        let answer = search(router).await;
        assert_same_answer(&reference, &answer, step);
        step += 1;
    }
    reference
}

/// Arms the barrier, wakes the writer with a flush command, and waits for it to
/// be held before receiving anything.
///
/// The command is **spawned**, never awaited: depending on where the
/// implementation checks the barrier, the flush may complete before the hold
/// point or be served after the release. The test proves nothing on its
/// completion, only on the barrier.
async fn hold_writer(state: &Arc<AppState>, barrier: &Barrier) -> tokio::task::JoinHandle<()> {
    let waking = Arc::clone(state);
    let wake = tokio::spawn(async move {
        waking
            .flush_analytics()
            .await
            .expect("the flush that wakes the writer succeeds");
    });
    barrier.wait_ready(ANALYTICS_FLUSH).await;
    wake
}

/// Waits for the end of the flush that woke the writer.
///
/// Called **after** the release, so the task can never be waited for while the
/// writer is held.
async fn join_flush(wake: tokio::task::JoinHandle<()>) {
    wake.await.expect("the flush task ends");
}

/// Every search answers `200` with the same body while the writer is held, and
/// the events the full channel refused are counted, exactly.
///
/// INV-17, protocol steps 3 and 4 of ADR-0118, decision 7: the response does not
/// depend on the analytics, and `analytics_dropped` grows by exactly the
/// overflow — nor more (no event lost elsewhere), nor less (no silent refusal).
#[tokio::test]
async fn inv_17_searches_answer_while_the_analytics_writer_is_held() {
    let slot = lock_environment().await;
    let (_instance, state, router) = instance_with_an_empty_channel().await;

    let baseline = analytics::dropped();
    let barrier = Barrier::armed(slot);
    let wake = hold_writer(&state, &barrier).await;

    // The reference search and every other one must answer `200` with the same
    // body, whether the channel accepts or refuses their event.
    let _reference = saturate(&router).await;

    assert_eq!(
        analytics::dropped() - baseline,
        OVERFLOW,
        "the counter counts exactly the refused events, no more and no less (INV-17)"
    );

    barrier.release(ANALYTICS_FLUSH);
    join_flush(wake).await;
}

/// Once the writer is released and a flush forced, the aggregates count exactly
/// the events the channel accepted, and none of the refused ones.
///
/// INV-17, protocol step 4 of ADR-0118, decision 7: the counter and the
/// aggregates check each other. The minute row of the searches does not exist
/// twice (one minute, one row) and its `count` equals the capacity of the
/// channel, while the `count` of the whole accepted traffic is the sum of the
/// latency histograms.
#[tokio::test]
async fn inv_17_accepted_events_are_all_written_after_the_release() {
    let slot = lock_environment().await;
    let (instance, state, router) = instance_with_an_empty_channel().await;

    let baseline = analytics::dropped();
    let barrier = Barrier::armed(slot);
    let wake = hold_writer(&state, &barrier).await;
    let _reference = saturate(&router).await;
    assert_eq!(
        analytics::dropped() - baseline,
        OVERFLOW,
        "the channel is full and the overflow is counted (INV-17)"
    );

    // The release lets the writer leave the hold point; the forced flush drains
    // the channel to empty and writes it (ADR-0118, decision 3).
    barrier.release(ANALYTICS_FLUSH);
    join_flush(wake).await;
    state
        .flush_analytics()
        .await
        .expect("the accepted events of the burst are flushed");

    let path = instance.root().join("data").join(ANALYTICS_FILE_NAME);
    let rows = read_minute_events(&path).expect("the analytics file is readable");
    let searches: Vec<&MinuteEvent> = rows
        .iter()
        .filter(|row| row.index == INDEX && row.operation == "search")
        .collect();
    assert_eq!(
        searches.len(),
        1,
        "one minute row per index and operation: {searches:?}"
    );
    let row = searches[0];
    assert_eq!(
        row.count, QUEUE_CAPACITY as u64,
        "the aggregates count the accepted events, not the refused ones (INV-17): {row:?}"
    );
    assert_eq!(row.status, "ok", "every search answered `200`: {row:?}");
    assert_eq!(
        row.client, "reader",
        "the client is the name of the key: {row:?}"
    );
    assert_eq!(
        total(&row.latency_total),
        QUEUE_CAPACITY as u64,
        "every accepted search produced one `total` sample (ADR-0117, decision 2): {row:?}"
    );

    // ADR-0118, decision 7: not one byte of content. The text of the passage and
    // the value of the filter of every search are scanned in the raw file, and in
    // its write-ahead log when SQLite kept one, so a marker that reached a column
    // of any type would fail here.
    let files = analytics_bytes(&path);
    assert!(
        !files.is_empty(),
        "the analytics file is readable at {}",
        path.display()
    );
    for (name, bytes) in files {
        for marker in [TEXT, FILTER_MARKER] {
            assert!(
                !contains(&bytes, marker.as_bytes()),
                "the analytics file {name} holds the content marker {marker:?} (INV-13, INV-17)"
            );
        }
    }
}

/// The raw bytes of the analytics file and of its write-ahead log, when it
/// exists: the two files SQLite may hold content in.
fn analytics_bytes(path: &Path) -> Vec<(String, Vec<u8>)> {
    let wal = path.with_file_name(format!("{ANALYTICS_FILE_NAME}-wal"));
    let mut files = Vec::new();
    for file in [path.to_path_buf(), wal] {
        if let Ok(bytes) = std::fs::read(&file) {
            files.push((file.display().to_string(), bytes));
        }
    }
    files
}

/// Whether `haystack` holds `needle`, byte for byte.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// An operation event produced while the channel is full does not change the
/// answer of its route either.
///
/// `GET /v1/indexes/{index}` is `index_admin`, one of the operations of the
/// closed list of ADR-0118, decision 6: its `OpEvent` goes through the same
/// `try_send`, is refused for the same reason, and increments the same counter.
/// The response stays the one of an accepted request: a full analytics channel
/// is never an error.
#[tokio::test]
async fn inv_17_an_operation_event_keeps_its_answer_while_the_channel_is_full() {
    let slot = lock_environment().await;
    let (_instance, state, router) = instance_with_an_empty_channel().await;

    let baseline = analytics::dropped();
    let barrier = Barrier::armed(slot);
    let wake = hold_writer(&state, &barrier).await;
    let _reference = saturate(&router).await;
    assert_eq!(
        analytics::dropped() - baseline,
        OVERFLOW,
        "the channel is full and the overflow is counted (INV-17)"
    );

    let path = format!("/v1/indexes/{INDEX}");
    let (status, body) = send(&router, Method::GET, &path, READ_SECRET, None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an operation of a full analytics channel answers normally: {body}"
    );
    assert_eq!(body["name"], INDEX, "the answer is the index: {body}");
    assert_eq!(
        analytics::dropped() - baseline,
        OVERFLOW + 1,
        "the refused operation event is counted like a refused search (INV-17)"
    );

    barrier.release(ANALYTICS_FLUSH);
    join_flush(wake).await;
}

/// Sum of the buckets of a histogram, in `u64` (`docs/06-observabilite.md`,
/// "Histogrammes"; ADR-0117, decision 1).
fn total(buckets: &[u32]) -> u64 {
    buckets.iter().map(|bucket| u64::from(*bucket)).sum()
}
