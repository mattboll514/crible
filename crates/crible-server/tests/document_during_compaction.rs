// SPDX-License-Identifier: Apache-2.0

// The only test of this file holds a compaction with the observer of
// `crible_store::compact::compact_with_observer`, which exists only with the
// `failpoints` feature, so the whole binary needs the server's feature too
// (complement of 2026-10-29 to ADR-0068, decision 2).
#![cfg(feature = "failpoints")]
// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the checks of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! `GET /documents/{doc_id}` reads the epoch of its **published snapshot**, not
//! the epoch `CURRENT_EPOCH` names, and renders the generation of that snapshot
//! (ADR-0107, decision 6).
//!
//! The compaction is held at its `Switched` step: the on-disk epoch is E+1 and
//! its generation is G+1, while the writer still publishes the snapshot of E at
//! generation G. The read must answer G, without an internal id.
//!
//! ```text
//! cargo test -p crible-server --features failpoints --test document_during_compaction
//! ```

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use common::{
    ADMIN_SECRET, DOC_ID, INDEX, READ_SECRET, instance_with_alpha, online_batch, post_documents,
    send, terminal_job,
};

/// Environment variable of the barrier (`src/barriers.rs`).
const BARRIER_ENV: &str = "CRIBLE_TEST_BARRIER";

/// Waits for `path` to exist, or panics after ten seconds.
///
/// The wait is asynchronous on purpose: the test runs on the current-thread
/// runtime, so a blocking wait would keep the runtime from polling the writer
/// task that starts the compaction.
async fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "the compaction reached the barrier: {}",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Asserts that no key of `value` at any depth contains `iid` (INV-01).
fn assert_no_iid_key(label: &str, value: &Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                assert!(
                    !key.to_lowercase().contains("iid"),
                    "{label} carries the key '{key}' (INV-01)"
                );
                assert_no_iid_key(label, child);
            }
        }
        Value::Array(items) => {
            for item in items {
                assert_no_iid_key(label, item);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

/// Reads the fixture document and returns its body.
async fn read_document(router: &axum::Router) -> Value {
    let path = format!("/v1/indexes/{INDEX}/documents/{DOC_ID}");
    let (status, body) = send(router, Method::GET, &path, READ_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "the document is served: {body}");
    body
}

/// The read during a compaction serves the published snapshot.
#[tokio::test]
async fn a_read_during_a_compaction_serves_the_epoch_of_the_snapshot() {
    let barrier = tempfile::tempdir().expect("a temporary barrier directory");
    // SAFETY: this binary holds a single test, so no other thread reads or writes
    // the environment while the barrier is armed.
    unsafe { std::env::set_var(BARRIER_ENV, barrier.path()) };

    let (_instance, state, router) = instance_with_alpha().await;
    let (status, accepted) = post_documents(
        &router,
        online_batch(
            "batch-read-during-compaction",
            "le contrat fait la loi",
            "1240",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    let job = terminal_job(&router, &state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the batch attaches: {job}");
    let published = job["generation"].as_u64().expect("a generation");

    let path = format!("/v1/indexes/{INDEX}/compact?wait=false");
    let (status, accepted) = send(&router, Method::POST, &path, ADMIN_SECRET, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    wait_for(&barrier.path().join("ready-compaction")).await;

    // `CURRENT_EPOCH` names E+1, whose generation is G+1: reading the on-disk
    // current epoch would answer G+1, not G.
    let document = read_document(&router).await;
    assert_eq!(
        document["generation"], published,
        "the read renders the generation of the published snapshot: {document}"
    );
    assert_eq!(
        document["doc_id"], DOC_ID,
        "the document of the snapshot is served: {document}"
    );
    assert_no_iid_key("the document read during a compaction", &document);

    std::fs::write(barrier.path().join("go-compaction"), b"").expect("the barrier is releasable");
    let job = terminal_job(&router, &state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the compaction ends: {job}");
    assert_eq!(
        job["result"]["report"]["generation"],
        json!(published + 1),
        "the compaction publishes G+1: {job}"
    );

    let after = read_document(&router).await;
    assert_eq!(
        after["generation"],
        json!(published + 1),
        "the read follows the new snapshot: {after}"
    );
    assert_no_iid_key("the document read after a compaction", &after);

    // SAFETY: as above, the single test of this binary owns the environment.
    unsafe { std::env::remove_var(BARRIER_ENV) };
}
