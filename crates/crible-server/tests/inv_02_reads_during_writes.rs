// SPDX-License-Identifier: Apache-2.0

// The tests of this file hold a write job with the test barrier of the
// `failpoints` build (`src/barriers.rs`, complement of 2026-10-29 to ADR-0068,
// decision 2), which exists only with the server's `failpoints` feature, so a
// plain `cargo test` compiles no test here.
#![cfg(feature = "failpoints")]
// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the checks of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! INV-02 — a read is never blocked by a write, proved at the HTTP level
//! (`docs/08-invariants-et-tests.md:10`; ADR-0113, decision 3).
//!
//! Each test submits one write job without waiting for it, lets the
//! `failpoints` barrier hold that job **inside its write**, then sends a
//! `POST /v1/indexes/{index}/search` and checks that it answers `200` with the
//! **previous** generation and the previous results — without the job being
//! released. Releasing the barrier lets the job finish, and a second search must
//! then see the generation the job published. Nothing is timed: no latency
//! threshold is asserted here. MES-16 measures the percentiles of ADR-0113,
//! decision 1 under load.
//!
//! | Test | Hold point |
//! |---|---|
//! | `inv_02_a_search_completes_while_an_attach_is_held_in_its_write` | attach, before the SQLite `COMMIT` |
//! | `inv_02_a_search_completes_while_an_attach_is_held_after_its_sqlite_commit` | attach, after the SQLite `COMMIT`, before the publication |
//! | `inv_02_a_hybrid_search_completes_while_an_attach_is_held_in_its_write` | same point, lexical reader |
//! | `inv_02_a_search_completes_while_a_compaction_is_held` | compaction, after the epoch switch |
//!
//! The compaction barrier is `"compaction"` (`src/maintenance.rs`); the two
//! attach barriers are `"attach-before-commit"` and
//! `"attach-after-sqlite-commit"`, delivered by `crible_store::generation`'s
//! `CommitStep` observer of the attach write (`attach_with_observer`,
//! `ingest_documents_with_observer`) and wired in
//! `crates/crible-server/src/attach_job.rs` through `barriers::hold_attach`.
//! Both the attach and the compaction barriers only exist with the `failpoints`
//! feature.
//!
//! ```text
//! cargo test -p crible-server --features failpoints --test inv_02_reads_during_writes
//! ```

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::state::AppState;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use common::{
    ADMIN_SECRET, DOC_ID, INDEX, Instance, READ_SECRET, WRITE_SECRET, base64_vector,
    instance_with_alpha, send, terminal_job,
};

/// Environment variable of the test barrier (`src/barriers.rs`).
const BARRIER_ENV: &str = "CRIBLE_TEST_BARRIER";

/// Barrier of the attach write, just before the SQLite `COMMIT`.
const ATTACH_BEFORE_COMMIT: &str = "attach-before-commit";

/// Barrier of the attach write, just after the SQLite `COMMIT` and before the
/// publication of the snapshot.
const ATTACH_AFTER_SQLITE_COMMIT: &str = "attach-after-sqlite-commit";

/// Barrier of the compaction, after the epoch switch and before the
/// publication (`src/maintenance.rs`).
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

/// Serialises the access to the process-global `CRIBLE_TEST_BARRIER`.
///
/// The variable names **one** directory per process, and the jobs of a test read
/// it, so this binary runs one test at a time: each test takes the slot
/// [`lock_environment`] returns **before** it builds its instance and its
/// fixture, and holds it until its end. The guard is a `tokio` one, so it is
/// held across the `await`s of a test without blocking a runtime thread.
static ENVIRONMENT: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// The process-wide slot of a test, taken before its instance exists.
///
/// A test that held the slot only around [`Barrier::armed`] would let its own
/// fixture attach run while a neighbouring test has `CRIBLE_TEST_BARRIER` set:
/// that attach would read the variable, reach the held step and write the
/// `ready-<scope>` of the neighbour, which its `wait_ready` would take for its
/// own job.
struct Slot {
    _serial: tokio::sync::MutexGuard<'static, ()>,
}

/// Takes the process-wide slot of the test barrier.
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
    /// Arms the barrier directory, in the slot `slot` of the test.
    ///
    /// `released` names the write steps this test does **not** want to hold;
    /// they are released up front. An attach passes `attach-before-commit`
    /// before it reaches `attach-after-sqlite-commit`, so the test of the second
    /// point must release the first one, or the job waits 30 s at a step the
    /// test never releases and misses the point it armed.
    ///
    /// It is called **after** the fixture is written, because every attach
    /// submitted while the barrier is armed stops at the held step.
    fn armed(slot: Slot, released: &[&str]) -> Self {
        let directory = tempfile::tempdir().expect("a temporary barrier directory is creatable");
        // SAFETY: `slot` is the process-wide slot of [`ENVIRONMENT`], taken at
        // the start of the test, before its instance exists; no other test of
        // this binary runs, so no job reads the variable while it is set, and
        // `Drop` removes it before the slot is released.
        unsafe { std::env::set_var(BARRIER_ENV, directory.path()) };
        // The attach steps are opt-in (`src/barriers.rs`): a test that arms
        // them writes `armed-attach`, so an attach of one of the other server
        // tests is never held.
        std::fs::write(directory.path().join("armed-attach"), b"")
            .expect("the attach steps are armable");
        // The search route, the multi-index route and the deposited-batch job
        // have barriers of their own (`src/search.rs`, `src/search_multi.rs`,
        // `src/batches.rs`). This file proves INV-02 on the **writes**, so those
        // are released up front: a search that waited on an unrelated barrier
        // would answer `504` instead of proving the invariant.
        for scope in ["search", "search-multi", "batch"].iter().chain(released) {
            let go = directory.path().join(format!("go-{scope}"));
            std::fs::write(&go, b"").expect("an unrelated barrier is releasable");
        }
        Self {
            directory,
            _slot: slot,
        }
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
                "{}",
                missing_barrier(scope, &ready)
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

/// Message of a barrier that the job never reached.
///
/// The hold point exists (`src/barriers.rs`, `attach_job.rs`): the message names
/// the scope and the path, and leaves the cause to the diagnosis of the run.
fn missing_barrier(scope: &str, path: &Path) -> String {
    format!(
        "the job never reached the barrier '{scope}' at {}",
        path.display()
    )
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
    let path = SEARCH;
    let (status, body) = send(
        router,
        Method::POST,
        path,
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
        "the search completes while the write is held (INV-02): {body}"
    );
    body
}

/// Sends a hybrid search and asserts it answers `200` (INV-02, lexical reader).
async fn search_hybrid(router: &Router) -> Value {
    let (status, body) = send(
        router,
        Method::POST,
        SEARCH,
        READ_SECRET,
        Some(json!({
            "query": "contrat",
            "vector": base64_vector(VECTOR),
            "mode": "hybrid",
            "k": 10,
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the hybrid search completes while the write is held (INV-02): {body}"
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

/// Fixture instance whose index `alpha` has the lexical branch enabled.
///
/// The hybrid test needs a lexical reader; the other tests keep the fixture of
/// `common`, which never builds tantivy.
async fn instance_with_lexical_alpha() -> (Instance, Arc<AppState>, Router) {
    let instance = Instance::new();
    let state = instance.state();
    let router = crible_server::http::build_router(Arc::clone(&state));
    let mut config = common::index_config();
    config["lexical"]["enabled"] = json!(true);
    let (status, body) = send(
        &router,
        Method::POST,
        "/v1/indexes",
        ADMIN_SECRET,
        Some(config),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "the index is created: {body}");
    (instance, state, router)
}

/// A search sent while an attach is held before its SQLite `COMMIT` answers
/// `200` with the previous generation, and the release publishes the next one.
///
/// INV-02, and `docs/02-architecture.md` ("Concurrence"): one writer per index,
/// readers never blocked, each request takes an `Arc<Snapshot>`.
#[tokio::test]
async fn inv_02_a_search_completes_while_an_attach_is_held_in_its_write() {
    let slot = lock_environment().await;
    let (_instance, state, router) = instance_with_alpha().await;
    let published = attach(&router, &state, upsert("batch-before", BEFORE, "1240")).await;

    let barrier = Barrier::armed(slot, &[ATTACH_AFTER_SQLITE_COMMIT]);
    let accepted = submit(&router, upsert("batch-held", AFTER, "1241")).await;
    barrier.wait_ready(ATTACH_BEFORE_COMMIT).await;

    // The transaction of the attach is open and not committed: the search must
    // still see the generation before it, and the passages of that generation.
    let held = search(&router).await;
    assert_eq!(
        generation(&held),
        published,
        "a search during a held attach reads the previous generation: {held}"
    );
    assert_eq!(
        first_text(&held),
        BEFORE,
        "a search during a held attach reads the passages of the previous generation: {held}"
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

/// Same proof with the hold **after** the SQLite `COMMIT`: the generation is
/// committed but not published, and a search keeps answering the published one.
///
/// INV-02, and INV-03: a reader sees the previous generation entirely, never the
/// committed-but-unpublished one (`docs/02-architecture.md`, "Concurrence";
/// ADR-0043).
#[tokio::test]
async fn inv_02_a_search_completes_while_an_attach_is_held_after_its_sqlite_commit() {
    let slot = lock_environment().await;
    let (_instance, state, router) = instance_with_alpha().await;
    let published = attach(&router, &state, upsert("batch-before", BEFORE, "1240")).await;

    let barrier = Barrier::armed(slot, &[ATTACH_BEFORE_COMMIT]);
    let accepted = submit(&router, upsert("batch-held", AFTER, "1241")).await;
    barrier.wait_ready(ATTACH_AFTER_SQLITE_COMMIT).await;

    // The transaction is committed and the snapshot is not published: the
    // search must answer the published generation, not the committed one.
    let held = search(&router).await;
    assert_eq!(
        generation(&held),
        published,
        "a search between the COMMIT and the publication reads the published \
         generation: {held}"
    );
    assert_eq!(
        first_text(&held),
        BEFORE,
        "the passages of the committed generation are not visible yet: {held}"
    );

    barrier.release(ATTACH_AFTER_SQLITE_COMMIT);
    let job = terminal_job(&router, &state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the attach ends: {job}");

    let after = search(&router).await;
    assert_eq!(
        generation(&after),
        published + 1,
        "the publication is what makes the new generation visible: {after}"
    );
    assert_eq!(
        first_text(&after),
        AFTER,
        "a search after the release sees the new passages: {after}"
    );
}

/// A `hybrid` search holds the lexical reader of the published snapshot, and it
/// completes while the attach is held before its `COMMIT`.
///
/// INV-02 on the second branch: the attach owns the tantivy writer, so a
/// lexical reader that waited on it would be a blocked read.
#[tokio::test]
async fn inv_02_a_hybrid_search_completes_while_an_attach_is_held_in_its_write() {
    let slot = lock_environment().await;
    let (_instance, state, router) = instance_with_lexical_alpha().await;
    let published = attach(&router, &state, upsert("batch-before", BEFORE, "1240")).await;

    let barrier = Barrier::armed(slot, &[ATTACH_AFTER_SQLITE_COMMIT]);
    let accepted = submit(&router, upsert("batch-held", AFTER, "1241")).await;
    barrier.wait_ready(ATTACH_BEFORE_COMMIT).await;

    let held = search_hybrid(&router).await;
    assert_eq!(
        generation(&held),
        published,
        "a hybrid search during a held attach reads the previous generation: {held}"
    );
    assert_eq!(
        first_text(&held),
        BEFORE,
        "the lexical reader of the held snapshot keeps answering: {held}"
    );

    barrier.release(ATTACH_BEFORE_COMMIT);
    let job = terminal_job(&router, &state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the attach ends: {job}");

    let after = search_hybrid(&router).await;
    assert_eq!(
        generation(&after),
        published + 1,
        "a hybrid search after the release sees the new generation: {after}"
    );
    assert_eq!(
        first_text(&after),
        AFTER,
        "a hybrid search after the release sees the new passages: {after}"
    );
}

/// A search sent while a compaction is held after its epoch switch answers
/// `200` with the generation of the published snapshot, and the release makes
/// the compaction visible.
///
/// INV-02, and ADR-0017: "les lectures continuent normalement sur l'ancienne
/// époque, jusqu'à la bascule"; INV-09 on the results, which a compaction keeps.
#[tokio::test]
async fn inv_02_a_search_completes_while_a_compaction_is_held() {
    let slot = lock_environment().await;
    let (_instance, state, router) = instance_with_alpha().await;
    let published = attach(&router, &state, upsert("batch-before", BEFORE, "1240")).await;

    let barrier = Barrier::armed(slot, &[]);
    let path = format!("/v1/indexes/{INDEX}/compact?wait=false");
    let (status, accepted) = send(&router, Method::POST, &path, ADMIN_SECRET, None).await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "the compaction is queued: {accepted}"
    );
    barrier.wait_ready(COMPACTION).await;

    // `CURRENT_EPOCH` names E+1 and its generation is G+1, while the writer
    // still publishes the snapshot of E: the search must answer G.
    let held = search(&router).await;
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
