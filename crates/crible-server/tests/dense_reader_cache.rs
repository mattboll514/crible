// SPDX-License-Identifier: Apache-2.0

// The tests of this file read the segment reader hook of the `failpoints` build
// (`crible_store::testhooks`, ADR-0126, decision 6), and the retention test
// holds the compaction with the test barrier of the same build
// (`src/barriers.rs`), so a plain `cargo test` compiles no test here.
#![cfg(feature = "failpoints")]
// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the checks of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! The dense reader cache of ADR-0126, proved on the observable hook.
//!
//! ADR-0126, decisions 1, 4, 5 and 6, give four rules, each with a test:
//!
//! | Rule (ADR-0126) | Test |
//! |---|---|
//! | a search of a served generation opens no reader (decision 6) | `a_served_generation_opens_no_dense_reader_per_search` |
//! | an attach opens only the reader of its new segment, sealed once (decisions 1 and 2) | `an_attach_opens_only_the_reader_of_its_new_segment_and_checks_its_seal_once` |
//! | a patch of metadata opens no reader (decision 1) | `a_metadata_patch_opens_no_dense_reader` |
//! | a generation is served only once its readers are open (decision 4) | `startup_opens_every_segment_before_the_first_search` |
//! | no reader of a retired generation outlives its retention (decision 5) | `readers_of_a_retired_generation_are_freed_after_the_compaction` |
//!
//! The hook is `crible_store::testhooks`: with the `failpoints` feature, and
//! only with it, `SegmentReader::open` appends `open <segment_id>` and
//! `seal <segment_id>`, and the reader appends `drop <segment_id>` when it is
//! released. Nothing else in the store changes. A test proves the absence of an
//! opening by counting those events, never by asserting that the code does not
//! call `open`.
//!
//! ```text
//! cargo test -p crible-server --features failpoints --test dense_reader_cache
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
    ADMIN_SECRET, DOC_ID, INDEX, READ_SECRET, base64_vector, instance_with_alpha, send,
    terminal_job,
};

/// Environment variable of the test barrier (`src/barriers.rs`).
const BARRIER_ENV: &str = "CRIBLE_TEST_BARRIER";

/// Barrier of the compaction, after the epoch switch and before the publication
/// (`src/maintenance.rs`).
const COMPACTION: &str = "compaction";

/// Path of the search route of the fixture index.
const SEARCH: &str = "/v1/indexes/alpha/search";

/// Longest wait of a test for a barrier to be reached.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Granularity of the polling loop, in milliseconds.
const POLL: Duration = Duration::from_millis(5);

/// Vector of the fixture document, and of every query of this file.
const VECTOR: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

/// Text of the passage of the fixture document.
const TEXT: &str = "le contrat fait la loi";

/// Text the second attach of the file writes.
///
/// It must differ from [`TEXT`]: an attach of the same `doc_hash` and the same
/// fields writes nothing and creates no segment (`crible-store`, `attach`), so
/// the new segment the test counts would not exist.
const TEXT_UPDATED: &str = "le contrat est puni";

/// Number of searches a served generation is asked for.
///
/// Twenty is far above one: a searcher opened per query would show twenty
/// openings, and a searcher kept would show none.
const SEARCHES: usize = 20;

/// Serialises the access to the process-global barrier and reader hook.
///
/// Both variables name **one** directory per process, so this binary runs one
/// test at a time: each test takes the slot [`lock_environment`] returns
/// **before** it builds its instance and holds it until its end.
static ENVIRONMENT: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// The process-wide slot of a test, taken before its instance exists.
struct Slot {
    directory: tempfile::TempDir,
    _serial: tokio::sync::MutexGuard<'static, ()>,
}

/// Takes the process-wide slot of the test barrier and of the reader hook.
async fn lock_environment() -> Slot {
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
    /// It is called **after** the fixture is written, because every job
    /// submitted while the barrier is armed stops at its held step.
    fn armed(slot: Slot) -> Self {
        let directory = tempfile::tempdir().expect("a temporary barrier directory is creatable");
        // SAFETY: `slot` holds the process-wide guard of [`ENVIRONMENT`]: no
        // other test of this binary runs, and `Drop` removes the variable
        // before the guard is released.
        unsafe { std::env::set_var(BARRIER_ENV, directory.path()) };
        let go = directory.path().join("go-batch");
        std::fs::write(&go, b"").expect("an unrelated barrier is releasable");
        Self { directory, slot }
    }

    /// `<dir>/go-<scope>`: the job may go on.
    fn release(&self, scope: &str) {
        let go = self.directory.path().join(format!("go-{scope}"));
        std::fs::write(&go, b"")
            .unwrap_or_else(|error| panic!("the barrier {} is releasable: {error}", go.display()));
    }

    /// Waits for the job to reach the barrier of `scope`.
    async fn wait_ready(&self, scope: &str) {
        let ready = self.directory.path().join(format!("ready-{scope}"));
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
            "fields": { "numero": "1240" },
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

/// Sends a dense search, asserts it answers `200` and returns it.
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
    assert_eq!(status, StatusCode::OK, "the dense search answers: {body}");
    body
}

/// The generation of an answer.
fn generation(answer: &Value) -> u64 {
    answer["generation"]
        .as_u64()
        .unwrap_or_else(|| panic!("the answer carries its generation: {answer}"))
}

/// Asserts that an answer really scored a passage.
///
/// A dense score cannot be computed without the reference vectors of the
/// segment, so a scored result proves the search read them, from the cache of
/// the published generation when no `open` event accompanies it (ADR-0126,
/// decisions 1 and 6).
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

/// Number of segments the published generation of the fixture index holds.
///
/// It is the count the detail route renders (`docs/05-api.md`), read on the
/// published triplet: the internal identifiers themselves never leave the
/// engine (INV-01).
fn published_segments(state: &Arc<AppState>) -> usize {
    let loaded = state
        .loaded()
        .unwrap_or_else(|| panic!("the instance has loaded its indexes"));
    let handle = loaded
        .index(INDEX)
        .unwrap_or_else(|error| panic!("the index registry is readable: {error}"))
        .unwrap_or_else(|| panic!("the index '{INDEX}' is served"));
    handle
        .published()
        .unwrap_or_else(|error| panic!("the published state is readable: {error}"))
        .summary
        .segments
}

/// The distinct segments that an `open` event names, in arrival order.
fn opened_ids(events: &[ReaderEvent]) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for event in events {
        if !matches!(event, ReaderEvent::Open { .. }) {
            continue;
        }
        let id = event.segment_id().to_owned();
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

/// How many `open` events name `segment_id`.
fn opens(events: &[ReaderEvent], segment_id: &str) -> usize {
    events
        .iter()
        .filter(|event| event.segment_id() == segment_id)
        .filter(|event| matches!(event, ReaderEvent::Open { .. }))
        .count()
}

/// How many `seal` events name `segment_id`.
fn seals(events: &[ReaderEvent], segment_id: &str) -> usize {
    events
        .iter()
        .filter(|event| event.segment_id() == segment_id)
        .filter(|event| matches!(event, ReaderEvent::Seal { .. }))
        .count()
}

/// How many events of any kind name `segment_id`.
fn count_all(events: &[ReaderEvent], segment_id: &str) -> usize {
    events
        .iter()
        .filter(|event| event.segment_id() == segment_id)
        .count()
}

/// Readers of `segment_id` that were opened and not yet released.
fn live(events: &[ReaderEvent], segment_id: &str) -> i64 {
    let mut live = 0i64;
    for event in events.iter().filter(|e| e.segment_id() == segment_id) {
        match event {
            ReaderEvent::Open { .. } => live += 1,
            ReaderEvent::Drop { .. } => live -= 1,
            ReaderEvent::Seal { .. } => {}
        }
    }
    live
}

/// Twenty searches of a served generation open no segment reader.
///
/// ADR-0126, decision 6, and the acceptance criterion of the task: "aucune
/// ouverture de lecteur dense sur le chemin d'une requête d'une génération déjà
/// servie".
#[tokio::test]
async fn a_served_generation_opens_no_dense_reader_per_search() {
    let slot = lock_environment().await;
    let (_instance, state, router) = instance_with_alpha().await;
    let published = attach(&router, &state, upsert("batch-a", TEXT)).await;

    slot.reset();
    for step in 0..SEARCHES {
        let answer = search(&router).await;
        assert_eq!(
            generation(&answer),
            published,
            "search {step} reads the published generation: {answer}"
        );
        assert_scored(&answer);
    }
    assert!(
        slot.events().is_empty(),
        "a search of a served generation opened a segment reader (ADR-0126, \
         decision 6): {:?}",
        slot.events()
    );
}

/// An attach opens only the reader of its new segment, and verifies its seal
/// once.
///
/// ADR-0126, decisions 1 and 2: the segments already served stay in the cache,
/// and the new one is opened and sealed once, at the fill.
#[tokio::test]
async fn an_attach_opens_only_the_reader_of_its_new_segment_and_checks_its_seal_once() {
    let slot = lock_environment().await;
    let (_instance, state, router) = instance_with_alpha().await;
    let first = attach(&router, &state, upsert("batch-a", TEXT)).await;
    let served = opened_ids(&slot.events());
    assert_eq!(
        served.len(),
        1,
        "the first attach serves one segment with its reader: {:?}",
        slot.events()
    );

    slot.reset();
    let second = attach(&router, &state, upsert("batch-b", TEXT_UPDATED)).await;
    assert_eq!(
        second,
        first + 1,
        "the second attach publishes the next generation"
    );

    let events = slot.events();
    let opened = opened_ids(&events);
    assert_eq!(
        opened.len(),
        1,
        "an attach opens the reader of its new segment, and nothing else \
         (ADR-0126, decision 1): {events:?}"
    );
    let new_segment = &opened[0];
    assert!(
        !served.contains(new_segment),
        "the attach reopened the segment '{new_segment}' that was already \
         served, instead of the segment of its own batch: {events:?}"
    );
    assert_eq!(
        opens(&events, new_segment),
        1,
        "the new segment is opened once, not once per generation (ADR-0126, \
         decision 1): {events:?}"
    );
    assert_eq!(
        seals(&events, new_segment),
        1,
        "the new segment is sealed once, at its opening (ADR-0126, decision 2): \
         {events:?}"
    );
    for segment_id in &served {
        assert_eq!(
            count_all(&events, segment_id),
            0,
            "the attach touched the reader of the segment '{segment_id}' that \
             was already served: {events:?}"
        );
    }
}

/// A patch of metadata opens no segment reader.
///
/// ADR-0126, decision 1: "un patch ne crée aucun segment" (`docs/02:137`), so
/// the cache keeps the same entries and republishes the same readers.
#[tokio::test]
async fn a_metadata_patch_opens_no_dense_reader() {
    let slot = lock_environment().await;
    let (_instance, state, router) = instance_with_alpha().await;
    let published = attach(&router, &state, upsert("batch-a", TEXT)).await;

    slot.reset();
    let (status, accepted) = send(
        &router,
        Method::PATCH,
        "/v1/indexes/alpha?wait=false",
        ADMIN_SECRET,
        Some(json!({ "search": { "rescore_candidates": 20 } })),
    )
    .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::ACCEPTED,
        "the patch is accepted: {accepted}"
    );
    let job = terminal_job(&router, &state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the patch ends: {job}");
    assert_eq!(job["kind"], "patch_config", "the patch job kind: {job}");

    assert!(
        slot.events().is_empty(),
        "a patch of metadata opened a segment reader (ADR-0126, decision 1): \
         {:?}",
        slot.events()
    );

    let answer = search(&router).await;
    assert_eq!(
        generation(&answer),
        published,
        "a patch of metadata publishes no generation: {answer}"
    );
    assert_scored(&answer);
    assert!(
        slot.events().is_empty(),
        "the search after a patch reopened a reader instead of reusing the \
         cached one: {:?}",
        slot.events()
    );
}

/// Every segment of the published generation is open before the first search.
///
/// ADR-0126, decision 4: "une génération servie est une génération publiée avec
/// ses lecteurs". Neither the creation of the index nor the fixture attach is
/// followed by a search here, so the events read below are the trace of the
/// publications alone.
#[tokio::test]
async fn startup_opens_every_segment_before_the_first_search() {
    let slot = lock_environment().await;
    let (_instance, state, router) = instance_with_alpha().await;
    attach(&router, &state, upsert("batch-a", TEXT)).await;
    // A different text commits a second segment: an attach of the same content
    // writes nothing and creates none, so the fill would only ever see one.
    attach(&router, &state, upsert("batch-b", TEXT_UPDATED)).await;

    let events = slot.events();
    let segments = published_segments(&state);
    let opened = opened_ids(&events);
    assert!(
        segments >= 2,
        "the fixture published at least two segments, so a partial fill is \
         visible: {segments}"
    );
    assert_eq!(
        opened.len(),
        segments,
        "every segment of the published generation is opened at its publication, \
         before any search runs (ADR-0126, decision 4): {events:?}"
    );
    for segment_id in &opened {
        assert_eq!(
            opens(&events, segment_id),
            1,
            "the segment '{segment_id}' is opened exactly once before the first \
             search: {events:?}"
        );
        assert_eq!(
            seals(&events, segment_id),
            1,
            "the segment '{segment_id}' is sealed exactly once, at that opening \
             (ADR-0126, decision 2): {events:?}"
        );
    }

    let before = events.len();
    let answer = search(&router).await;
    assert_scored(&answer);
    assert_eq!(
        slot.events().len(),
        before,
        "the first search opened a reader that the publication should have \
         opened already (ADR-0126, decision 6): {:?}",
        slot.events()
    );
}

/// No reader of the generation a compaction retires outlives it.
///
/// ADR-0126, decision 5, and the third acceptance criterion: "aucun lecteur
/// d'une génération retirée n'est gardé au-delà de sa rétention". The retired
/// segment is the one the fixture attach published; the compaction creates a new
/// epoch, publishes its readers and drops the retired ones. A reader held by a
/// request would be dropped by the reference count once that request ended, and
/// the search below runs while the compaction is held, before the publication.
#[tokio::test]
async fn readers_of_a_retired_generation_are_freed_after_the_compaction() {
    let slot = lock_environment().await;
    let (_instance, state, router) = instance_with_alpha().await;
    let published = attach(&router, &state, upsert("batch-a", TEXT)).await;
    let retired = opened_ids(&slot.events());
    assert_eq!(
        retired.len(),
        1,
        "the fixture attach serves one segment: {:?}",
        slot.events()
    );

    let barrier = Barrier::armed(slot);
    let path = format!("/v1/indexes/{INDEX}/compact?wait=false");
    let (status, accepted) = send(&router, Method::POST, &path, ADMIN_SECRET, None).await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "the compaction is queued: {accepted}"
    );
    barrier.wait_ready(COMPACTION).await;

    // A request of the epoch the compaction replaces, ended before the
    // publication: its `Arc<Snapshot>` and its readers are released at the end
    // of the answer.
    let held = search(&router).await;
    assert_eq!(
        generation(&held),
        published,
        "a search during the held compaction reads the published generation: {held}"
    );
    assert_scored(&held);

    barrier.release(COMPACTION);
    let job = terminal_job(&router, &state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the compaction ends: {job}");

    let events = barrier.events();
    for segment_id in &retired {
        assert_eq!(
            live(&events, segment_id),
            0,
            "the reader of the retired segment '{segment_id}' is still alive \
             after the compaction published its epoch (ADR-0126, decision 5): \
             {events:?}"
        );
    }

    let segments = published_segments(&state);
    let current: Vec<String> = opened_ids(&events)
        .into_iter()
        .filter(|segment_id| !retired.contains(segment_id))
        .collect();
    assert_eq!(
        current.len(),
        segments,
        "the published generation serves its own segments: {events:?}"
    );
    for segment_id in &current {
        assert!(
            live(&events, segment_id) >= 1,
            "the segment '{segment_id}' of the published generation has no live \
             reader (ADR-0126, decision 4): {events:?}"
        );
    }

    let before = events.len();
    let answer = search(&router).await;
    assert_eq!(
        generation(&answer),
        job["generation"]
            .as_u64()
            .unwrap_or_else(|| panic!("the compaction publishes a generation: {job}")),
        "the search after the compaction reads the new generation: {answer}"
    );
    assert_scored(&answer);
    assert_eq!(
        barrier.events().len(),
        before,
        "the search after the compaction reopened a reader instead of using the \
         one published with the generation: {:?}",
        barrier.events()
    );
}
