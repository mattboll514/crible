// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the checks of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! An index whose reopen fails after a failed compaction stops writing, keeps
//! serving reads, fails every later job in `internal`, and logs the error once
//! (ADR-0108, decision 2).
//!
//! The failure is driven from the disk: the test makes `CURRENT_EPOCH` name an
//! epoch that does not exist, so the next compaction is refused by the guard of
//! ADR-0108, decision 1, and the reopen that follows fails. That path needs no
//! failpoint: a build without the feature exercises it too.
//!
//! ```text
//! cargo test -p crible-server --test broken_index
//! ```

mod common;

use axum::http::{Method, StatusCode};
use serde_json::Value;

use common::{
    DOC_ID, INDEX, READ_SECRET, assert_no_iid_key, instance_with_alpha, online_batch,
    post_documents, send, terminal_job,
};

/// Submits a valid online batch and returns its accepted body.
async fn submit_batch(router: &axum::Router) -> Value {
    let (status, accepted) = post_documents(
        router,
        online_batch("batch-broken", "le contrat fait la loi", "1240"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    accepted
}

/// An index that cannot reopen still serves reads and refuses every write.
#[tokio::test]
async fn an_index_that_cannot_reopen_serves_reads_and_refuses_writes() {
    let journal = common::journal();
    let (instance, state, router) = instance_with_alpha().await;
    let accepted = submit_batch(&router).await;
    let job = terminal_job(&router, &state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the batch attaches: {job}");
    let published = job["generation"].clone();

    // `CURRENT_EPOCH` names an epoch the disk does not hold: the next compaction
    // is refused by the guard of ADR-0108, decision 1, and the reopen after that
    // failure fails too.
    let pointer = instance
        .root()
        .join("data")
        .join("indexes")
        .join(INDEX)
        .join("CURRENT_EPOCH");
    std::fs::write(&pointer, b"99\n").expect("the pointer is writable");

    let path = format!("/v1/indexes/{INDEX}/compact?wait=true");
    let (status, accepted) = send(&router, Method::POST, &path, common::ADMIN_SECRET, None).await;
    assert!(
        status == StatusCode::OK || status == StatusCode::ACCEPTED,
        "the compaction is accepted before its job runs: {accepted}"
    );
    let job = terminal_job(&router, &state, &accepted).await;
    assert_eq!(job["state"], "failed", "the compaction fails: {job}");
    assert_eq!(
        job["error"]["code"], "internal",
        "the guard refuses the moved pointer: {job}"
    );

    // First effect: the reads keep the last snapshot, at its generation.
    let document_path = format!("/v1/indexes/{INDEX}/documents/{DOC_ID}");
    let (status, document) = send(&router, Method::GET, &document_path, READ_SECRET, None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the last snapshot still serves the document: {document}"
    );
    assert_eq!(
        document["generation"], published,
        "the read keeps the generation of the last snapshot: {document}"
    );
    assert_no_iid_key("the document read on a broken index", &document);

    // Second effect: every later job fails in `internal` with the broken-index
    // message, and none of them touches the disk.
    for _ in 0..3 {
        let accepted = submit_batch(&router).await;
        let job = terminal_job(&router, &state, &accepted).await;
        assert_eq!(job["state"], "failed", "the write is refused: {job}");
        assert_eq!(job["error"]["code"], "internal", "{job}");
    }

    // Third effect: the journal carries the broken-index error once, not once
    // per job and not once per refused write (ADR-0108, decision 2). The cause
    // of the failed reopen is a `warn`, so the single `error` line of the broken
    // index is the one of `mark_broken`.
    let text = journal.contents();
    let broken = text
        .lines()
        .filter(|line| {
            line.contains("\"level\":\"ERROR\"")
                && line.contains("cannot reopen after a failed compaction")
        })
        .count();
    assert_eq!(broken, 1, "the broken-index error is logged once: {text}");
    let cause = text
        .lines()
        .filter(|line| {
            line.contains("\"level\":\"WARN\"")
                && line.contains("could not reopen after a failed compaction")
        })
        .count();
    assert_eq!(
        cause, 1,
        "the cause of the failed reopen is logged once: {text}"
    );

    // The read route still answers after the refused writes.
    let (status, _) = send(&router, Method::GET, &document_path, READ_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "the reads keep being served");
}
