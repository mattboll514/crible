// SPDX-License-Identifier: Apache-2.0

// Tests may panic; `allow-panic-in-tests` covers `#[test]` functions only, and
// the helpers of this file are fixtures and assertions.
#![allow(clippy::expect_used, clippy::panic)]

//! INV-04 — after a restart, the state is the last committed generation of each
//! index, and everything else is rebuilt or cleaned up.
//!
//! A `delete_index` job left non terminal by the death of the process is part of
//! "everything else": nothing is replayed, the row passes to `failed` at the
//! startup (ADR-0103, decision 5), and `failed` is terminal. ADR-0150, decision 3,
//! rule 5, builds on it: after a restart no refusal of `POST /v1/indexes` carries
//! a stale `details.job_id`, because the job table holds no non-terminal
//! `delete_index` any more.
//!
//! What the state lets a test observe is therefore the pair: the job is `failed`
//! at the startup, and the refusal of the name is the one of ADR-0105,
//! decision 2, with `details: {}` — or a `201` when the death came after the
//! rename, since the registry is rebuilt from the disk and the quarantine is
//! purged (ADR-0056).
//!
//! | Test | What it proves |
//! |---|---|
//! | `inv_04_a_restarted_instance_presents_no_stale_delete_job_id` | an interrupted `delete_index` is `failed`, its index is still served, and the refusal names no job |
//! | `inv_04_a_restart_after_the_rename_accepts_the_name_without_a_job_id` | the death after the rename leaves no directory; the name is free and the quarantine is purged |
//!
//! A restart is a new process: the writer task of the first instance holds its
//! store, and with it the writer lock of the index (ADR-0116, decision 2), until
//! its runtime goes away. Each phase therefore runs in a runtime of its own,
//! dropped before the next one opens the same data root.
//!
//! ```text
//! cargo test -p crible-server --test inv_04_delete_job_after_restart
//! ```

use std::sync::Arc;

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::http::build_router;
use crible_server::jobs::new_job_id;
use crible_server::state::AppState;
use crible_store::jobs::{JOBS_FILE_NAME, JobKind, JobStore};
use serde_json::json;

mod common;

use common::{ADMIN_SECRET, INDEX, Instance, READ_SECRET, WRITE_SECRET, index_config, send};

/// Creates the fixture index through the route, in a runtime of its own, then
/// drops that runtime, as the death of the process would end its writer tasks.
fn first_process(instance: &Instance) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime is creatable");
    let router = {
        let _entered = runtime.enter();
        build_router(instance.state())
    };
    runtime.block_on(async {
        let (status, body) = send(
            &router,
            Method::POST,
            "/v1/indexes",
            ADMIN_SECRET,
            Some(index_config()),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "the fixture index is created: {body}"
        );
    });
    drop(runtime);
}

/// Loads the same data root again, in a fresh runtime of its own.
fn restarted(instance: &Instance) -> (tokio::runtime::Runtime, Arc<AppState>, Router) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime is creatable");
    let (state, router) = {
        let _entered = runtime.enter();
        let state = instance.state();
        (Arc::clone(&state), build_router(state))
    };
    (runtime, state, router)
}

/// Path of the job table of the instance, at the root of `<data>` (ADR-0103).
fn job_table(instance: &Instance) -> std::path::PathBuf {
    instance.root().join("data").join(JOBS_FILE_NAME)
}

/// Writes a `delete_index` job of the fixture index that the death of the
/// process left non terminal, as the running job would have left it.
///
/// The row is `queued`, the state a deletion has before the writer of its index
/// picks it up: `JobStore::insert` is the call the board makes when a mutation
/// is accepted with no free writer.
fn interrupted_delete(instance: &Instance) -> String {
    let id = new_job_id();
    let jobs = JobStore::open_or_create(&job_table(instance))
        .expect("the job table of the instance opens");
    jobs.insert(&id, JobKind::DeleteIndex, INDEX)
        .expect("the interrupted deletion is written");
    id
}

#[test]
fn inv_04_a_restarted_instance_presents_no_stale_delete_job_id() {
    // INV-04: the restart rebuilds the state from the last committed generation
    // of the index, which is intact, and the interrupted deletion is `failed`.
    // ADR-0150, decision 3, rule 5: no refusal names the identifier of that job.
    let instance = Instance::new();
    first_process(&instance);
    let id = interrupted_delete(&instance);

    let (runtime, _state, router) = restarted(&instance);
    runtime.block_on(async {
        let (status, job) = send(
            &router,
            Method::GET,
            &format!("/v1/jobs/{id}"),
            WRITE_SECRET,
            None,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the job survives the restart: {job}"
        );
        assert_eq!(
            job["state"], "failed",
            "a `queued` job is `failed` at the next startup, never replayed \
             (ADR-0103, decision 5): {job}"
        );
        assert_eq!(job["kind"], "delete_index", "{job}");

        let (status, detail) = send(
            &router,
            Method::GET,
            &format!("/v1/indexes/{INDEX}"),
            READ_SECRET,
            None,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the death came before the rename, so the index is intact and served: {detail}"
        );

        let (status, answer) = send(
            &router,
            Method::POST,
            "/v1/indexes",
            ADMIN_SECRET,
            Some(index_config()),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "the name is still taken by the index: {answer}"
        );
        assert_eq!(answer["error"]["code"], "invalid_request", "{answer}");
        assert_eq!(
            answer["error"]["details"],
            json!({}),
            "no stale `job_id` is presented after a restart (INV-04; ADR-0150, \
             decision 3, rule 5): {answer}"
        );
        assert!(
            !answer.to_string().contains(&id),
            "the refusal names no job of the table: {answer}"
        );
    });
}

#[test]
fn inv_04_a_restart_after_the_rename_accepts_the_name_without_a_job_id() {
    // INV-04 and ADR-0150, decision 3, rule 5, second half: when the death came
    // after the rename of ADR-0150, decision 4, the registry is rebuilt from a
    // disk that no longer holds the directory, and the quarantine is purged at
    // the startup (ADR-0056). The name is free, and the `201` carries no job.
    let instance = Instance::new();
    first_process(&instance);
    let id = interrupted_delete(&instance);
    let data = instance.root().join("data");
    let quarantine = data.join("tmp").join(format!("deleted-{INDEX}-{id}"));
    std::fs::create_dir_all(data.join("tmp")).expect("the quarantine directory is creatable");
    std::fs::rename(data.join("indexes").join(INDEX), &quarantine)
        .expect("the index directory moves into the quarantine");

    let (runtime, _state, router) = restarted(&instance);
    runtime.block_on(async {
        assert!(
            !quarantine.exists(),
            "the startup purges the quarantined directories of <data>/tmp"
        );

        let (status, body) = send(
            &router,
            Method::POST,
            "/v1/indexes",
            ADMIN_SECRET,
            Some(index_config()),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "the name is free: the deletion is terminal and no directory is left: {body}"
        );
        assert_eq!(body["name"], INDEX, "{body}");
        assert_eq!(
            body["state"]["generation"], 0,
            "the accepted creation is a fresh index: {body}"
        );
        assert!(
            !body.to_string().contains(&id),
            "the `201` names no job (INV-04): {body}"
        );

        let (status, jobs) = send(
            &router,
            Method::GET,
            &format!("/v1/jobs?index={INDEX}"),
            WRITE_SECRET,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{jobs}");
        assert!(
            jobs["jobs"].as_array().is_some_and(|jobs| jobs
                .iter()
                .any(|job| job["id"] == id.as_str() && job["state"] == "failed")),
            "the history of the interrupted deletion stays in jobs.sqlite, failed: {jobs}"
        );
    });
}
