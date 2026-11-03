// SPDX-License-Identifier: Apache-2.0

// The tests of this file hold the debug journal writer with the test barrier of
// the `failpoints` build (`src/barriers.rs`, complement of 2026-10-29 to
// ADR-0068, decision 2), which exists only with the server's `failpoints`
// feature, so a plain `cargo test` compiles no test here.
#![cfg(feature = "failpoints")]
// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the checks of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! INV-17 — writing the debug journal never blocks a request, proved at the HTTP
//! level (`docs/08-invariants-et-tests.md:25`; ADR-0122, decision 11).
//!
//! The test repeats the protocol ADR-0118, decision 7 fixed for the analytics,
//! on the second writer INV-17 covers. The debug journal writer is held **at the
//! head of its loop, before it receives anything** (`debug-journal-flush`
//! barrier). While it is held, the bounded channel of ADR-0122, decision 6
//! fills; the tests then prove three things:
//!
//! 1. every search still answers `200`, with the same body as the search sent
//!    before the channel was full (only `took_ms` and the `debug` member, both
//!    durations, are left out: `docs/05-api.md`, "Recherche : réponse");
//! 2. every query the channel refused is counted in `debug_journal_dropped`,
//!    and nothing else is;
//! 3. once the barrier is released and a flush is forced, the table `queries`
//!    holds **exactly** the queries the channel accepted: a loss announced by
//!    the counter never appears as a row, and a row never appears that the
//!    counter did not announce.
//!
//! No latency is measured and no threshold is asserted: the latency of a shared
//! runner is not reproducible (ADR-0113, `:94`). The tests prove that a search
//! **completes** and answers the same thing while the writing is held. MES-16
//! measures the percentiles of ADR-0113, decision 1 under load (ADR-0122,
//! decision 12).
//!
//! | Test | What it proves |
//! |---|---|
//! | `inv_17_searches_answer_while_the_debug_journal_writer_is_held` | `200` and identical body during a full channel; `debug_journal_dropped` grows by exactly the overflow |
//! | `inv_17_accepted_queries_are_all_written_after_the_release` | the table holds exactly the accepted rows, with their payload members and no `iid` |
//! | `inv_17_holding_the_debug_journal_does_not_hold_the_analytics` | the two flush barriers are distinct, so one writer held never stops the other |
//! | `inv_17_debug_journal_cap_never_blocks` | a trim at `max_rows` holds the writer inside its transaction, and every search still answers like an empty journal (ADR-0127, decision 3) |
//!
//! The barrier has its own scope, `debug-journal-flush`, and follows the
//! protocol of `src/barriers.rs`: `<dir>/ready-debug-journal-flush` then
//! `<dir>/go-debug-journal-flush`, an opt-in mark
//! `<dir>/armed-debug-journal-flush` (so a test that arms another scope never
//! holds the journal writer), and no barrier armed means a normal run. The
//! scope is **distinct** from `analytics-flush`: an armed barrier for one
//! writer must not retain the other (ADR-0122, decision 11).
//!
//! The cap of ADR-0127 is proved by a second scope, `debug-journal-trim`: the
//! blocking task of a flush inserts its rows, then trims the oldest ones at the
//! cap **before** its `COMMIT`, and writes `<dir>/ready-debug-journal-trim`
//! there. That point runs on the blocking pool, so holding it never holds the
//! async runtime; the searches of the test answer while it is held, which is
//! INV-17 under a real trim rather than a full channel. The mark
//! `<dir>/armed-debug-journal-trim` arms it alone, so the head-of-loop barrier
//! above is not armed by this test.
//!
//! ```text
//! cargo test -p crible-server --features failpoints --test inv_17_debug_journal_never_blocks
//! ```

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::debug_journal;
use crible_server::state::AppState;
use crible_store::debug_journal::{DEBUG_JOURNAL_FILE_NAME, DebugQuery, read_queries};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use common::{
    DOC_ID, INDEX, Instance, Journal, READ_SECRET, base64_vector, instance_with_alpha_and_journal,
    post_documents, send, terminal_job,
};

/// Environment variable of the test barrier (`src/barriers.rs`).
const BARRIER_ENV: &str = "CRIBLE_TEST_BARRIER";

/// Barrier of the debug journal writer, at the head of its loop, before it
/// receives anything (ADR-0122, decision 11).
const DEBUG_JOURNAL_FLUSH: &str = "debug-journal-flush";

/// Mark that arms the debug journal barrier.
///
/// The journal writer runs in every server test whose instance enabled the
/// journal, so the hold point is opt-in: without this mark a test that armed
/// another scope runs the writer at full speed, like the attach steps of
/// `src/barriers.rs`.
const DEBUG_JOURNAL_FLUSH_ARMED: &str = "armed-debug-journal-flush";

/// Barrier inside the blocking task of a flush, once the rows are inserted and
/// before the trim deletes the oldest ones at the cap (ADR-0127, decision 3).
///
/// It runs on the blocking pool, so the hold point is a blocking wait and
/// never stops the async runtime the searches need.
const DEBUG_JOURNAL_TRIM: &str = "debug-journal-trim";

/// Mark that arms the trim barrier.
///
/// The scope is opt-in like the flush one: a test of this binary that arms
/// another point runs a trimming flush at full speed.
const DEBUG_JOURNAL_TRIM_ARMED: &str = "armed-debug-journal-trim";

/// Path of the search route of the fixture index.
const SEARCH: &str = "/v1/indexes/alpha/search";

/// Path of the journal route.
const QUERIES: &str = "/v1/debug/queries";

/// Number of searches sent **beyond** the capacity of the channel.
///
/// Three is enough to prove the counter: a test that overflowed by one could
/// pass on a writer that lost one query by accident, and a larger overflow
/// makes no other difference. The exact number is asserted, so a second loss
/// anywhere else fails the test.
const OVERFLOW: u64 = 3;

/// Rows the fixture keeps once the cap is reached (ADR-0127, decision 1).
///
/// Three is the smallest cap that still shows the oldest row leaving while two
/// newer ones stay, and `max_rows >= 1` is the validation of the same decision.
const MAX_ROWS: usize = 3;

/// Searches sent while the trim is held.
///
/// The channel is empty at that point, so every one is accepted: the number
/// only needs to show that the route answers repeatedly during the trim.
const DURING_TRIM: usize = 8;

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
/// Unlike the analytics, the journal **holds** the filter by design (ADR-0121,
/// decision 4): this marker is what proves the payload carries the filter as it
/// was received, rather than a canonical form or nothing.
const FILTER_MARKER: &str = "marqueur-de-valeur-de-filtre-2f1c7d";

/// Serialises the access to the process-global `CRIBLE_TEST_BARRIER` and to the
/// process-global `debug_journal_dropped`.
///
/// The variable names **one** directory per process, and `debug_journal_dropped`
/// is a counter per process (ADR-0122, decision 6), so this binary runs one test
/// at a time: each test takes the slot [`lock_environment`] returns **before**
/// it builds its instance and holds it until its end. The guard is a `tokio` one,
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
    /// Arms the debug journal writer barrier, in the slot `slot` of the test.
    ///
    /// It is called **after** the fixture is written: the writer of the fixture
    /// must not be held before the test has a stable starting point.
    fn armed(slot: Slot) -> Self {
        Self::with_marks(slot, &[DEBUG_JOURNAL_FLUSH_ARMED])
    }

    /// Arms the trim barrier alone, in the slot `slot` of the test.
    ///
    /// The head-of-loop barrier is left unarmed, so the writer receives and
    /// drains normally and only stops once its rows are inserted and the trim
    /// runs (ADR-0127, decision 3).
    fn armed_trim(slot: Slot) -> Self {
        Self::with_marks(slot, &[DEBUG_JOURNAL_TRIM_ARMED])
    }

    /// Arms the marks of `marks` in one barrier directory.
    fn with_marks(slot: Slot, marks: &[&str]) -> Self {
        let directory = tempfile::tempdir().expect("a temporary barrier directory is creatable");
        // SAFETY: `slot` is the process-wide slot of [`ENVIRONMENT`], taken at
        // the start of the test; no other test of this binary runs, so no job
        // reads the variable while it is set, and `Drop` removes it before the
        // slot is released.
        unsafe { std::env::set_var(BARRIER_ENV, directory.path()) };
        for mark in marks {
            // The hold points are opt-in (`src/barriers.rs`): a writer that only
            // finds the directory of a test holding another scope runs at full
            // speed.
            std::fs::write(directory.path().join(mark), b"")
                .expect("the debug journal barrier is armable");
        }
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

    /// Waits for the debug journal writer to reach the barrier.
    async fn wait_ready(&self, scope: &str) {
        let ready = self.ready(scope);
        let deadline = Instant::now() + TIMEOUT;
        while !ready.exists() {
            assert!(
                Instant::now() < deadline,
                "the debug journal writer never reached the barrier '{scope}' at {}",
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
    let (status, accepted) = post_documents(router, body).await;
    assert_eq!(status, StatusCode::OK, "the batch is accepted: {accepted}");
    let job = terminal_job(router, state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the batch attaches: {job}");
}

/// A fixture whose journal channel is **empty** and whose writer is **idle**.
///
/// The journal is enabled with `sample_rate = 0`: only a search that asks
/// `debug: true` is recorded (ADR-0122, decision 1), and the fixture sends none.
/// The fixture write is followed by a forced flush anyway, so a row the
/// implementation might deposit on the creation or the attach does not occupy a
/// slot: the capacity the test fills is the whole capacity.
async fn instance_with_an_empty_channel() -> (Instance, Arc<AppState>, Router) {
    let journal = Journal::enabled(0.0);
    let (instance, state, router) = instance_with_alpha_and_journal(journal).await;
    attach(&router, &state, upsert("batch-fixture", TEXT)).await;
    state
        .flush_debug_journal()
        .await
        .expect("the journal of the fixture is flushed");
    (instance, state, router)
}

/// Sends one dense search that asks `debug`, and asserts it answers `200`
/// (INV-17).
///
/// The search uses a provided vector, so the fixture `external` model needs no
/// encoder: the route does its own work and nothing else. Its filter matches the
/// only document of the index, so every search of the burst answers the same
/// body, and that filter is the one the payload must carry verbatim.
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
            "debug": true,
            "filter": { "numero": { "eq": FILTER_MARKER } },
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the search completes while the debug journal writer is held (INV-17): {body}"
    );
    body
}

/// The body of an answer without its durations.
///
/// `took_ms` and the `debug` member (`plan`, `timings_ms`, `per_result`) are the
/// only parts of `docs/05-api.md` that differ between two identical searches;
/// the protocol of ADR-0122, decision 11 names the results, the order, the
/// scores and the `generation`, never a duration.
fn without_durations(body: &Value) -> Value {
    let mut body = body.clone();
    if let Some(object) = body.as_object_mut() {
        object.remove("took_ms");
        object.remove("debug");
    }
    body
}

/// Asserts that an answer carries the same body as the reference.
fn assert_same_answer(reference: &Value, answer: &Value, step: usize) {
    assert_eq!(
        without_durations(answer),
        without_durations(reference),
        "the debug journal does not change an answer; search {step} differs (INV-17)"
    );
}

/// Sends one reference search, then the rest of the capacity of the channel,
/// then [`OVERFLOW`] refused ones, and asserts that every answer is `200` and
/// identical to the reference.
///
/// The reference is the first of the accepted queries; the writer being held,
/// the channel accepts `QUEUE_CAPACITY` - 1 searches after it and refuses the
/// rest.
async fn saturate(router: &Router) -> Value {
    let capacity = debug_journal::QUEUE_CAPACITY;
    let reference = search(router).await;
    let mut step = 1;
    for _ in 1..capacity {
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
            .flush_debug_journal()
            .await
            .expect("the flush that wakes the writer succeeds");
    });
    barrier.wait_ready(DEBUG_JOURNAL_FLUSH).await;
    wake
}

/// Waits for the end of the flush that woke the writer.
///
/// Called **after** the release, so the task can never be waited for while the
/// writer is held.
async fn join_flush(wake: tokio::task::JoinHandle<()>) {
    wake.await.expect("the flush task ends");
}

/// Spawns a forced flush and waits until the blocking task is held at the trim.
///
/// The flush takes one row above `max_rows` into its transaction, inserts it,
/// then trims and reaches the barrier (ADR-0127, decision 3). The task is
/// spawned, never awaited before the release: a flush that is not held would
/// finish, and the test would fail on its missing mark rather than hang.
async fn spawn_and_wait_trim(
    state: &Arc<AppState>,
    barrier: &Barrier,
) -> tokio::task::JoinHandle<()> {
    let flushing = Arc::clone(state);
    let wake = tokio::spawn(async move {
        flushing
            .flush_debug_journal()
            .await
            .expect("the flush that reaches the trim succeeds");
    });
    barrier.wait_ready(DEBUG_JOURNAL_TRIM).await;
    wake
}

/// Every search answers `200` with the same body while the writer is held, and
/// the queries the full channel refused are counted, exactly.
///
/// INV-17, protocol steps 3 and 4 of ADR-0122, decision 11: the response does
/// not depend on the journal, and `debug_journal_dropped` grows by exactly the
/// overflow — nor more (no query lost elsewhere), nor less (no silent refusal).
#[tokio::test]
async fn inv_17_searches_answer_while_the_debug_journal_writer_is_held() {
    let slot = lock_environment().await;
    let (_instance, state, router) = instance_with_an_empty_channel().await;

    let baseline = debug_journal::dropped();
    let barrier = Barrier::armed(slot);
    let wake = hold_writer(&state, &barrier).await;

    // The reference search and every other one must answer `200` with the same
    // body, whether the channel accepts or refuses their query.
    let _reference = saturate(&router).await;

    assert_eq!(
        debug_journal::dropped() - baseline,
        OVERFLOW,
        "the counter counts exactly the refused queries, no more and no less (INV-17)"
    );

    barrier.release(DEBUG_JOURNAL_FLUSH);
    join_flush(wake).await;
}

/// Once the writer is released and a flush forced, the table holds exactly the
/// queries the channel accepted, and none of the refused ones.
///
/// INV-17, protocol step 4 of ADR-0122, decision 11: the counter and the rows
/// check each other. Every accepted `debug` search writes one row of the fixture
/// index, with the payload members of ADR-0121, decision 4, and no internal id
/// (INV-01); no row carries the API key.
#[tokio::test]
async fn inv_17_accepted_queries_are_all_written_after_the_release() {
    let slot = lock_environment().await;
    let (instance, state, router) = instance_with_an_empty_channel().await;

    let baseline = debug_journal::dropped();
    let barrier = Barrier::armed(slot);
    let wake = hold_writer(&state, &barrier).await;
    let _reference = saturate(&router).await;
    assert_eq!(
        debug_journal::dropped() - baseline,
        OVERFLOW,
        "the channel is full and the overflow is counted (INV-17)"
    );

    // The release lets the writer leave the hold point; the forced flush drains
    // the channel to empty and writes it (ADR-0122, decision 7).
    barrier.release(DEBUG_JOURNAL_FLUSH);
    join_flush(wake).await;
    state
        .flush_debug_journal()
        .await
        .expect("the accepted queries of the burst are flushed");

    let path = instance.root().join("data").join(DEBUG_JOURNAL_FILE_NAME);
    let rows = read_queries(&path).expect("the debug journal file is readable");
    assert_eq!(
        rows.len(),
        debug_journal::QUEUE_CAPACITY,
        "the table holds the accepted queries, not the refused ones (INV-17)"
    );
    for row in &rows {
        assert_row(row);
    }

    // ADR-0122, decision 11: not one credential. The API key, in clear, is
    // scanned in the raw file and in its write-ahead log, so a key that reached
    // a column or the payload would fail here (ADR-0100).
    let files = journal_bytes(&path);
    assert!(
        !files.is_empty(),
        "the debug journal file is readable at {}",
        path.display()
    );
    for (name, bytes) in files {
        assert!(
            !contains(&bytes, READ_SECRET.as_bytes()),
            "the debug journal file {name} holds the API key (ADR-0100, INV-17)"
        );
    }
}

/// Asserts that one row carries the columns and the payload members of
/// ADR-0121, decisions 3 and 4, and no internal id.
fn assert_row(row: &DebugQuery) {
    assert_eq!(
        row.index.as_deref(),
        Some(INDEX),
        "the row names the index of a simple search: {row:?}"
    );
    assert_eq!(row.status, "ok", "every search answered `200`: {row:?}");
    assert_eq!(
        row.client, "reader",
        "the client is the name of the key: {row:?}"
    );
    assert_eq!(
        row.mode, "dense",
        "the row carries the mode of the request: {row:?}"
    );
    assert!(
        row.total_ms.is_some(),
        "a finished search carries its `total_ms`: {row:?}"
    );
    assert!(
        !row.request_id.is_empty(),
        "the row carries a request id: {row:?}"
    );

    let payload: Value =
        serde_json::from_str(&row.payload).expect("the payload of the row is JSON text");
    let object = payload.as_object().expect("the payload is a JSON object");
    for member in ["request", "plan", "timings_ms", "capture", "generation"] {
        assert!(
            object.contains_key(member),
            "the payload of a `debug` search carries `{member}` (ADR-0121, decision 4): {payload}"
        );
    }
    // The filter travels **as received**, canonicalising it back to its fields
    // and operators is the analytics' job, not the journal's.
    assert_eq!(
        payload["request"]["filter"],
        json!({ "numero": { "eq": FILTER_MARKER } }),
        "the payload keeps the filter as it was received (ADR-0121, decision 4): {payload}"
    );
    // The capture names the external keys of `doc_id#ord`, never an internal id
    // (ADR-0119, decision 8; INV-01).
    let dense = payload["capture"]["dense"]
        .as_array()
        .unwrap_or_else(|| panic!("a dense search has a dense capture: {payload}"));
    assert!(
        !dense.is_empty(),
        "the capture of the fixture holds a hit: {payload}"
    );
    for entry in dense {
        let key = entry["key"]
            .as_str()
            .expect("a capture entry carries its key");
        assert!(
            key.starts_with(&format!("{DOC_ID}#")),
            "the capture names the external key: {key}"
        );
    }
    common::assert_no_iid_key("the payload of a journal row", &payload);
    common::assert_no_iid_key("a capture entry", &payload["capture"]);
}

/// The raw bytes of the debug journal file and of its write-ahead log, when it
/// exists.
fn journal_bytes(path: &Path) -> Vec<(String, Vec<u8>)> {
    let wal = path.with_file_name(format!("{DEBUG_JOURNAL_FILE_NAME}-wal"));
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

/// Holding the debug journal writer does not hold the analytics writer, and the
/// reverse holds too: the two barriers are distinct scopes (ADR-0122,
/// decision 11).
///
/// Only `armed-debug-journal-flush` is written here. If the two writers shared a
/// hold point, the analytics flush would wait for a release that never comes;
/// the bounded `timeout` turns that deadlock into a failure instead of a hung
/// test.
#[tokio::test]
async fn inv_17_holding_the_debug_journal_does_not_hold_the_analytics() {
    let slot = lock_environment().await;
    let (_instance, state, router) = instance_with_an_empty_channel().await;

    let baseline = debug_journal::dropped();
    let barrier = Barrier::armed(slot);
    let wake = hold_writer(&state, &barrier).await;
    let _reference = saturate(&router).await;
    assert_eq!(
        debug_journal::dropped() - baseline,
        OVERFLOW,
        "the channel is full and the overflow is counted (INV-17)"
    );

    let flushed = tokio::time::timeout(TIMEOUT, state.flush_analytics()).await;
    let flushed = flushed.unwrap_or_else(|_| {
        panic!("the analytics writer is held by the debug journal barrier: the scopes are shared")
    });
    flushed.expect("the analytics of the burst are flushed while the journal is held");

    // The route that reads the journal answers while its writer is held: a held
    // writer never turns into a `5xx` (ADR-0122, decision 9).
    let (status, body) = send(&router, Method::GET, QUERIES, common::ADMIN_SECRET, None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the journal route answers while the writer is held: {body}"
    );

    barrier.release(DEBUG_JOURNAL_FLUSH);
    join_flush(wake).await;
}

/// A trim at `max_rows` never blocks nor extends a search, and the answers stay
/// those of an empty journal (INV-17; ADR-0127, decisions 3 and 4).
///
/// The fixture sets `max_rows = `[`MAX_ROWS`] and `sample_rate = 0`, so only the
/// searches that ask `debug` are recorded. The journal is filled to the cap, one
/// row per search, then one more row takes it **above** the cap: that flush
/// inserts, trims, and holds at `debug-journal-trim`, inside its transaction.
/// While it is held, the searches must answer `200` with the body of the empty
/// journal, and the bounded channel must accept every row. After the release the
/// file is capped by the trim, and a second flush keeps it capped.
///
/// No latency is measured (ADR-0113): the test proves that a search
/// **completes** and answers the same thing, never a duration.
#[tokio::test]
async fn inv_17_debug_journal_cap_never_blocks() {
    let slot = lock_environment().await;
    let (instance, state, router) =
        instance_with_alpha_and_journal(Journal::with_max_rows(0.0, MAX_ROWS)).await;
    attach(&router, &state, upsert("batch-fixture", TEXT)).await;
    state
        .flush_debug_journal()
        .await
        .expect("the empty journal of the fixture is flushed");

    // The reference is the answer of an **empty** journal: no row is written
    // yet, so nothing in it comes from a capped or a trimmed file.
    let reference = search(&router).await;
    state
        .flush_debug_journal()
        .await
        .expect("the reference row is written");

    // Fill the journal to the cap, one row per search. A journal exactly at
    // `max_rows` is not above it, so no trim runs yet (ADR-0127, decision 3).
    for step in 1..MAX_ROWS {
        let answer = search(&router).await;
        assert_same_answer(&reference, &answer, step);
    }
    state
        .flush_debug_journal()
        .await
        .expect("the journal is filled to the cap");
    let path = instance.root().join("data").join(DEBUG_JOURNAL_FILE_NAME);
    let first = read_queries(&path).expect("the debug journal file is readable");
    assert_eq!(
        first.len(),
        MAX_ROWS,
        "the fixture is exactly at the cap, so no trim ran yet: {first:?}"
    );
    let oldest = first[0].id;

    // One more row takes the journal **above** the cap: this flush inserts it,
    // then trims the oldest row, and stops at the barrier (ADR-0127,
    // decision 3).
    let extra = search(&router).await;
    assert_same_answer(&reference, &extra, MAX_ROWS);
    let baseline = debug_journal::dropped();
    let barrier = Barrier::armed_trim(slot);
    let wake = spawn_and_wait_trim(&state, &barrier).await;

    // The trim holds the journal connection inside its transaction. No search
    // waits on it: each answers `200` with the body of the empty journal, and
    // the empty channel accepts every row (INV-17).
    for step in 0..DURING_TRIM {
        let answer = search(&router).await;
        assert_same_answer(&reference, &answer, step);
    }
    assert_eq!(
        debug_journal::dropped() - baseline,
        0,
        "the trim refuses no row: the channel is empty (INV-17)"
    );

    barrier.release(DEBUG_JOURNAL_TRIM);
    join_flush(wake).await;

    // The trim really ran: the file is capped and the oldest row is gone, so
    // the searches above were answered while a trim was under way.
    let capped = read_queries(&path).expect("the debug journal file is readable");
    assert_eq!(
        capped.len(),
        MAX_ROWS,
        "the trim keeps `max_rows` rows (ADR-0127, decision 3): {capped:?}"
    );
    assert!(
        capped.iter().all(|row| row.id > oldest),
        "the oldest row was trimmed first (ADR-0127, decision 3): {capped:?}"
    );

    // Sustained traffic keeps the cap: the rows accepted during the trim are
    // written by the next flush, which trims again.
    state
        .flush_debug_journal()
        .await
        .expect("the rows accepted during the trim are flushed");
    let after = read_queries(&path).expect("the debug journal file is readable");
    assert_eq!(
        after.len(),
        MAX_ROWS,
        "the cap holds across flushes: {after:?}"
    );
    assert!(
        after.iter().all(|row| row.id > capped[0].id),
        "the next flush trims the oldest survivor: {after:?}"
    );
}
