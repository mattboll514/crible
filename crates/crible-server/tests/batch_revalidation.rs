// SPDX-License-Identifier: Apache-2.0

// The only test of this file holds a job with the test barrier of the
// `failpoints` build (complement of 2026-10-29 to ADR-0068, decision 2), so the
// whole binary exists only with the server's `failpoints` feature.
#![cfg(feature = "failpoints")]
// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the checks of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! A symbolic link deposited **after** the `202` of `POST /batches` fails the
//! job in `invalid_request` and attaches nothing (complement of 2026-10-29 to
//! ADR-0107, decision 5).
//!
//! The route validates the batch directory before it queues the job, and the
//! operator can change the inbox while the job waits in the writer queue. The
//! job therefore revalidates the directory with the same function just before it
//! reads it. The barrier of `crates/crible-server/src/barriers.rs` holds the job
//! between the `202` and that revalidation, which makes the race deterministic.
//!
//! ```text
//! cargo test -p crible-server --features failpoints --test batch_revalidation
//! ```

mod common;

use axum::http::{Method, StatusCode};
use serde_json::json;

use common::{
    INDEX, READ_SECRET, WRITE_SECRET, export_index, instance_with_alpha, online_batch,
    post_documents, send, terminal_job, wait_for,
};

/// Environment variable of the barrier (`src/barriers.rs`).
const BARRIER_ENV: &str = "CRIBLE_TEST_BARRIER";

/// Reads the generation of the index.
async fn generation(router: &axum::Router) -> u64 {
    let (status, body) = send(
        router,
        Method::GET,
        &format!("/v1/indexes/{INDEX}"),
        READ_SECRET,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the index is served: {body}");
    body["state"]["generation"]
        .as_u64()
        .unwrap_or_else(|| panic!("the index detail carries its generation: {body}"))
}

/// Submits a deposited batch without waiting for its job.
async fn submit(router: &axum::Router, directory: &str) -> serde_json::Value {
    let path = format!("/v1/indexes/{INDEX}/batches?wait=false");
    let (status, accepted) = send(
        router,
        Method::POST,
        &path,
        WRITE_SECRET,
        Some(json!({ "directory": directory })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    accepted
}

/// The job revalidates the batch directory it reads, after the `202`.
#[tokio::test]
async fn a_link_added_after_the_accepted_response_fails_the_job() {
    let barrier = tempfile::tempdir().expect("a temporary barrier directory");
    // SAFETY: this binary holds a single test, so no other thread reads or writes
    // the environment while the barrier is armed.
    unsafe { std::env::set_var(BARRIER_ENV, barrier.path()) };

    let (instance, state, router) = instance_with_alpha().await;
    let (status, accepted) = post_documents(
        &router,
        online_batch("batch-revalidated", "le contrat fait la loi", "1240"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    terminal_job(&router, &state, &accepted).await;

    let exported = export_index(&router, &state).await;
    let deposited = "held-batch";
    std::fs::rename(
        instance.exchange().join(&exported),
        instance.inbox().join(deposited),
    )
    .expect("the exported batch moves into the inbox");
    let before = generation(&router).await;

    let accepted = submit(&router, deposited).await;
    let ready = barrier.path().join("ready-batch");
    wait_for("the batch revalidation", &ready).await;

    // The operator adds a link inside the batch once the response is sent: the
    // directory that was checked is no longer the directory the job would read.
    let link = instance.inbox().join(deposited).join("late-link");
    std::os::unix::fs::symlink("/etc/hosts", &link).expect("the link is creatable");
    std::fs::write(barrier.path().join("go-batch"), b"").expect("the barrier is releasable");

    let job = terminal_job(&router, &state, &accepted).await;
    assert_eq!(job["state"], "failed", "the job refuses the batch: {job}");
    assert_eq!(
        job["error"]["code"], "invalid_request",
        "the job names the batch contract: {job}"
    );
    let message = job["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the refusal carries a message: {job}"));
    assert!(
        message.contains("directory"),
        "the refusal names the field: {job}"
    );
    assert_eq!(
        generation(&router).await,
        before,
        "a refused job attaches nothing"
    );

    // SAFETY: as above, the single test of this binary owns the environment.
    unsafe { std::env::remove_var(BARRIER_ENV) };
}
