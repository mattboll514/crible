// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixtures and assertions of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! Progression of the write jobs, as `GET /v1/jobs/{job_id}` renders it
//! (ADR-0134; `docs/05-api.md`, "Jobs").
//!
//! | Test | What it proves |
//! |---|---|
//! | `an_attach_job_shows_its_step_and_total_while_it_runs` | a running attach publishes a step of the protocol and the passages of its manifest, held at a test barrier |
//! | `a_succeeded_attach_batch_ends_at_snapshot_and_the_table_carries_it` | a successful `attach_batch` ends at `snapshot` with `done == total`, in the route and in the table |
//! | `a_failed_attach_keeps_the_last_published_progress` | a job failing after the commit point keeps the `manifest` step it published |
//! | `a_succeeded_online_batch_counts_passages` | an `online_batch` ends with the attach progression, in passages |
//! | `a_replayed_batch_publishes_no_progress` | a replayed batch runs no step, so its succeeded job keeps `progress: null` |
//! | `a_patch_counts_documents` | a `patch_documents` counts the documents of its body, not their passages |
//! | `the_other_kinds_keep_progress_null` | a kind outside the three keeps `progress: null`, member present |
//! | `a_snapshot_job_publishes_copy_then_attest_then_rename_in_bytes` | a `snapshot` ends on `rename` with `done = total = bytes` and the table carries it (ADR-0153 §1 to §4) |
//! | `a_snapshot_refused_before_it_copies_keeps_progress_null` | a snapshot refused before its first publication keeps `progress: null` (ADR-0153 §5) |
//!
//! The mid-flight test uses the attach barrier of the `failpoints` build
//! (`src/barriers.rs`), which is a deterministic stop, never a sleep.
//!
//! ```text
//! cargo test -p crible-server --test job_progress
//! ```

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::jobs::JobOutcome;
use crible_store::jobs::{JOBS_FILE_NAME, JobKind, JobStore};
use serde_json::{Value, json};

#[cfg(feature = "failpoints")]
use common::AttachBarrier;
use common::{
    ADMIN_SECRET, DOC_ID, INDEX, Instance, WRITE_SECRET, base64_vector, export_index, index_config,
    instance_with_alpha, online_batch, post_documents, send, terminal_job,
};

/// Vector of the document every fixture of this file writes.
const VECTOR: [f32; 4] = [0.6, 0.8, 0.0, 0.0];

/// `GET /v1/jobs/{id}`.
async fn job(router: &Router, id: &str) -> Value {
    let path = format!("/v1/jobs/{id}");
    let (status, body) = send(router, Method::GET, &path, WRITE_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "the job is readable: {body}");
    body
}

/// Identifier of an accepted job.
fn id_of(accepted: &Value) -> String {
    accepted["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the acceptance carries the job id: {accepted}"))
        .to_owned()
}

/// Progression of `id` as the table holds it.
///
/// The board holds the only writer connection, but the file runs in WAL
/// (`crates/crible-store/src/jobs.rs`), so a reader opens it while the server
/// holds it.
fn stored_progress(instance: &Instance, id: &str) -> Option<Value> {
    let path = instance.root().join("data").join(JOBS_FILE_NAME);
    let store = JobStore::open_or_create(&path)
        .unwrap_or_else(|error| panic!("{} opens: {}", path.display(), error.message()));
    let record = store
        .job(id)
        .unwrap_or_else(|error| panic!("the job {id} is readable: {}", error.message()))
        .unwrap_or_else(|| panic!("the job {id} is in the table"));
    record.progress.map(|text| {
        serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("the progress of {id} is JSON: {error}"))
    })
}

/// Body of an online batch whose single document holds two passages.
fn two_passage_batch(batch_id: &str) -> Value {
    json!({
        "batch_id": batch_id,
        "mode": "upsert",
        "documents": [{
            "doc_id": DOC_ID,
            "fields": { "numero": "1240" },
            "passages": [
                {
                    "text": "le contrat fait la loi",
                    "fields": { "texte": "contrat" },
                    "vector": base64_vector(VECTOR),
                },
                {
                    "text": "la loi punit le contrat",
                    "fields": { "texte": "contrat" },
                    "vector": base64_vector([0.8, 0.6, 0.0, 0.0]),
                }
            ]
        }]
    })
}

/// `PATCH /v1/indexes/{index}/documents?wait=true`.
async fn patch_documents(router: &Router, body: Value) -> (StatusCode, Value) {
    let path = format!("/v1/indexes/{INDEX}/documents?wait=true");
    send(router, Method::PATCH, &path, WRITE_SECRET, Some(body)).await
}

/// A running attach publishes a step and the passages of its manifest.
///
/// The job is held **before the SQLite commit** of its generation by the
/// barrier of the `failpoints` build: the observation is deterministic, and the
/// step read is the `transaction` one the protocol published at the beginning of
/// that step.
#[cfg(feature = "failpoints")]
#[tokio::test]
async fn an_attach_job_shows_its_step_and_total_while_it_runs() {
    let _slot = common::lock_barrier_environment().await;
    let (_instance, state, router) = instance_with_alpha().await;
    let barrier = AttachBarrier::arm();

    let path = format!("/v1/indexes/{INDEX}/documents?wait=false");
    let (status, accepted) = send(
        &router,
        Method::POST,
        &path,
        WRITE_SECRET,
        Some(online_batch(
            "progress-held",
            "le contrat fait la loi",
            "1240",
        )),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "the online batch is queued: {accepted}"
    );
    let id = id_of(&accepted);

    barrier.wait_before_commit().await;

    let running = job(&router, &id).await;
    assert_eq!(running["state"], "running", "the job is held: {running}");
    let progress = running
        .get("progress")
        .unwrap_or_else(|| panic!("the job carries a progress member: {running}"));
    assert_eq!(
        progress["step"], "transaction",
        "the attach publishes the step it runs before the commit: {running}"
    );
    assert_eq!(
        progress["total"], 1,
        "the total is the passages of the manifest: {running}"
    );
    assert_eq!(
        progress["done"], 0,
        "done advances at the end of a successful job, not per passage: {running}"
    );

    barrier.release_before_commit();
    let finished = terminal_job(&router, &state, &accepted).await;
    assert_eq!(
        finished["state"], "succeeded",
        "the attach succeeds: {finished}"
    );
    assert_eq!(
        finished["progress"],
        json!({ "step": "snapshot", "done": 1, "total": 1 }),
        "a successful job ends at its last step and at `total`: {finished}"
    );
}

/// A successful `attach_batch` ends at `snapshot` with `done == total`, and the
/// final row of the table carries that progression.
#[tokio::test]
async fn a_succeeded_attach_batch_ends_at_snapshot_and_the_table_carries_it() {
    let _slot = common::lock_barrier_environment().await;
    let (first, first_state, first_router) = instance_with_alpha().await;
    let (status, accepted) = post_documents(
        &first_router,
        online_batch("progress-exported", "le contrat fait la loi", "1240"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the source batch attaches: {accepted}"
    );
    let source = terminal_job(&first_router, &first_state, &accepted).await;
    assert_eq!(
        source["state"], "succeeded",
        "the source attaches: {source}"
    );
    let exported = export_index(&first_router, &first_state).await;

    let deposited = "progress-deposited";
    let from = first.exchange().join(&exported);
    let to = first.inbox().join(deposited);
    std::fs::rename(&from, &to)
        .unwrap_or_else(|error| panic!("{} is movable: {error}", from.display()));

    let second = Instance::sharing(first.exchange());
    let second_state = second.state();
    let second_router = crible_server::http::build_router(Arc::clone(&second_state));
    let (status, body) = send(
        &second_router,
        Method::POST,
        "/v1/indexes",
        ADMIN_SECRET,
        Some(index_config()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "the index is created: {body}");

    let path = format!("/v1/indexes/{INDEX}/batches?wait=false");
    let (status, accepted) = send(
        &second_router,
        Method::POST,
        &path,
        WRITE_SECRET,
        Some(json!({ "directory": deposited })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "the deposited batch is accepted: {accepted}"
    );
    assert_eq!(accepted["kind"], "attach_batch", "{accepted}");
    let id = id_of(&accepted);
    let finished = terminal_job(&second_router, &second_state, &accepted).await;
    assert_eq!(
        finished["state"], "succeeded",
        "the attach succeeds: {finished}"
    );
    assert_eq!(
        finished["progress"],
        json!({ "step": "snapshot", "done": 1, "total": 1 }),
        "the route renders the last step of a successful job: {finished}"
    );
    assert_eq!(
        stored_progress(&second, &id).as_ref(),
        Some(&json!({ "step": "snapshot", "done": 1, "total": 1 })),
        "the final row of the table carries the same progression"
    );
}

/// A job that fails after its commit point keeps the last step it published.
#[tokio::test]
async fn a_failed_attach_keeps_the_last_published_progress() {
    let _slot = common::lock_barrier_environment().await;
    let (instance, state, router) = instance_with_alpha().await;
    // The manifest directory replaced by a regular file makes `write_manifest`
    // fail **after** the commit of the generation, so the job reaches the
    // `manifest` step and then fails (the fault of
    // `tests/job_failure_after_commit.rs`).
    let manifests = single_epoch(&instance).join("manifests");
    std::fs::remove_dir_all(&manifests)
        .unwrap_or_else(|error| panic!("{} is removable: {error}", manifests.display()));
    std::fs::write(&manifests, b"not a directory")
        .unwrap_or_else(|error| panic!("{} is replaceable: {error}", manifests.display()));

    let (status, accepted) = post_documents(
        &router,
        online_batch("progress-failed", "le contrat fait la loi", "1240"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the batch is accepted: {accepted}");
    let id = id_of(&accepted);
    let finished = terminal_job(&router, &state, &accepted).await;
    assert_eq!(finished["state"], "failed", "the attach fails: {finished}");
    assert_eq!(
        finished["progress"],
        json!({ "step": "manifest", "done": 0, "total": 1 }),
        "a failed job keeps the last progression it published: {finished}"
    );
    assert_eq!(
        stored_progress(&instance, &id).as_ref(),
        Some(&json!({ "step": "manifest", "done": 0, "total": 1 })),
        "the failed row carries the last published progression"
    );
}

/// The directory of the single epoch of the fixture index.
fn single_epoch(instance: &Instance) -> PathBuf {
    let epochs = instance
        .root()
        .join("data")
        .join("indexes")
        .join(INDEX)
        .join("epochs");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&epochs)
        .unwrap_or_else(|error| panic!("{} is readable: {error}", epochs.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("an epoch entry is readable: {error}"))
                .path()
        })
        .collect();
    assert_eq!(entries.len(), 1, "the fixture holds one epoch: {entries:?}");
    entries.remove(0)
}

/// An `online_batch` ends with the progress of its attach, in passages.
#[tokio::test]
async fn a_succeeded_online_batch_counts_passages() {
    let _slot = common::lock_barrier_environment().await;
    let (_instance, state, router) = instance_with_alpha().await;
    let (status, accepted) = post_documents(&router, two_passage_batch("progress-online")).await;
    assert_eq!(status, StatusCode::OK, "the batch is accepted: {accepted}");
    assert_eq!(accepted["kind"], "online_batch", "{accepted}");
    let finished = terminal_job(&router, &state, &accepted).await;
    assert_eq!(
        finished["state"], "succeeded",
        "the batch attaches: {finished}"
    );
    assert_eq!(
        finished["progress"],
        json!({ "step": "snapshot", "done": 2, "total": 2 }),
        "an online batch counts passages, like its manifest: {finished}"
    );
}

/// A replay of an already attached batch writes nothing, so it runs no step:
/// the job succeeds with `progress: null`, member present.
///
/// ADR-0134, decision 1 skips every step of the protocol for a replay, so
/// `complete()` has nothing to advance. A client must therefore not read
/// `progress: null` on a succeeded `attach_batch` or `online_batch` as a fault,
/// nor expect `done == total` on every successful job.
#[tokio::test]
async fn a_replayed_batch_publishes_no_progress() {
    let _slot = common::lock_barrier_environment().await;
    let (_instance, state, router) = instance_with_alpha().await;
    let body = online_batch("progress-replayed", "le contrat fait la loi", "1240");
    let (status, first) = post_documents(&router, body.clone()).await;
    assert_eq!(status, StatusCode::OK, "the batch attaches: {first}");
    let first = terminal_job(&router, &state, &first).await;
    assert_eq!(first["state"], "succeeded", "the first attach: {first}");

    let (status, replayed) = post_documents(&router, body).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the same batch is accepted again: {replayed}"
    );
    let replayed = terminal_job(&router, &state, &replayed).await;
    assert_eq!(
        replayed["state"], "succeeded",
        "a replay is not an error (INV-10): {replayed}"
    );
    assert!(
        replayed.get("progress").is_some(),
        "the member is present: {replayed}"
    );
    assert_eq!(
        replayed["progress"],
        Value::Null,
        "a replay runs no step and publishes nothing: {replayed}"
    );
}

/// A `patch_documents` counts the documents of its body, not their passages.
#[tokio::test]
async fn a_patch_counts_documents() {
    let _slot = common::lock_barrier_environment().await;
    let (_instance, state, router) = instance_with_alpha().await;
    let (status, accepted) =
        post_documents(&router, two_passage_batch("progress-patch-source")).await;
    assert_eq!(status, StatusCode::OK, "the source attaches: {accepted}");
    let source = terminal_job(&router, &state, &accepted).await;
    assert_eq!(source["state"], "succeeded", "{source}");

    let (status, accepted) = patch_documents(
        &router,
        json!({ "documents": [{ "doc_id": DOC_ID, "fields": { "numero": "999" } }] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the patch is accepted: {accepted}");
    assert_eq!(accepted["kind"], "patch_documents", "{accepted}");
    let finished = terminal_job(&router, &state, &accepted).await;
    assert_eq!(
        finished["state"], "succeeded",
        "the patch applies: {finished}"
    );
    assert_eq!(
        finished["progress"],
        json!({ "step": "snapshot", "done": 1, "total": 1 }),
        "one document patched, whatever its number of passages: {finished}"
    );
}

/// A kind outside the three keeps `progress: null`, the member present.
#[tokio::test]
async fn the_other_kinds_keep_progress_null() {
    let _slot = common::lock_barrier_environment().await;
    let (_instance, state, router) = instance_with_alpha().await;
    let accepted = state
        .submit_job(
            INDEX,
            JobKind::Export,
            Box::new(|_store, _progress, _writer| Ok(JobOutcome::default())),
        )
        .await
        .expect("the export job is accepted");
    state.jobs().expect("the board is open").drained().await;
    assert!(
        accepted.progress.is_none(),
        "the acceptance carries no progression"
    );
    let finished = job(&router, &accepted.id).await;
    assert_eq!(finished["state"], "succeeded", "{finished}");
    assert!(
        finished.get("progress").is_some(),
        "the member is present: {finished}"
    );
    assert_eq!(
        finished["progress"],
        Value::Null,
        "a kind with no unit keeps progress null: {finished}"
    );
}

/// A `snapshot` job publishes its copy in bytes and ends at `rename` with
/// `done = total = bytes` of its report (ADR-0153, decisions 1 to 4).
#[tokio::test]
async fn a_snapshot_job_publishes_copy_then_attest_then_rename_in_bytes() {
    let _slot = common::lock_barrier_environment().await;
    let (instance, state, router) = instance_with_alpha().await;
    let path = format!("/v1/indexes/{INDEX}/snapshots?wait=true");
    let (status, accepted) = send(&router, Method::POST, &path, ADMIN_SECRET, None).await;
    assert!(
        status.is_success(),
        "the snapshot is accepted: {status} {accepted}"
    );
    let finished = terminal_job(&router, &state, &accepted).await;
    assert_eq!(finished["state"], "succeeded", "{finished}");

    let bytes = finished["result"]["report"]["bytes"]
        .as_u64()
        .unwrap_or_else(|| panic!("the report counts the copied bytes: {finished}"));
    assert!(bytes > 0, "{finished}");
    let progress = &finished["progress"];
    assert_eq!(
        progress["step"], "rename",
        "a successful snapshot ends on the rename step: {finished}"
    );
    assert_eq!(
        progress["done"], bytes,
        "the job counts bytes, never passages: {finished}"
    );
    assert_eq!(
        progress["total"], bytes,
        "the exact total is the sum of the copied bytes: {finished}"
    );
    let id = id_of(&accepted);
    assert_eq!(
        stored_progress(&instance, &id),
        Some(json!({ "step": "rename", "done": bytes, "total": bytes })),
        "the terminal transition writes the exact progression to the table"
    );
}

/// A snapshot refused before it copies anything keeps `progress: null`
/// (ADR-0153, decision 5): the occupied destination is checked before the first
/// publication.
#[tokio::test]
async fn a_snapshot_refused_before_it_copies_keeps_progress_null() {
    let _slot = common::lock_barrier_environment().await;
    let (_instance, state, router) = instance_with_alpha().await;
    let path = format!("/v1/indexes/{INDEX}/snapshots?wait=true");

    let (_, first) = send(&router, Method::POST, &path, ADMIN_SECRET, None).await;
    let first = terminal_job(&router, &state, &first).await;
    assert_eq!(first["state"], "succeeded", "{first}");

    let (_, second) = send(&router, Method::POST, &path, ADMIN_SECRET, None).await;
    let second = terminal_job(&router, &state, &second).await;
    assert_eq!(second["state"], "failed", "{second}");
    assert_eq!(second["error"]["code"], "invalid_request", "{second}");
    assert!(
        second.get("progress").is_some(),
        "the member is always present: {second}"
    );
    assert_eq!(
        second["progress"],
        Value::Null,
        "a failure before the first publication leaves no progression: {second}"
    );
}
