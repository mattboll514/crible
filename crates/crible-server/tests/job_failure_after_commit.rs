// SPDX-License-Identifier: Apache-2.0

// Test fixtures may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixture below builds an index and an epoch.
#![allow(clippy::panic)]

//! A job that fails **after** its commit point still refreshes the published
//! state (INV-03, INV-04; ADR-0126, decision 4).
//!
//! The commit of a generation is the SQLite commit of the epoch, and building
//! the snapshot and writing the manifest happen **after** it
//! (`crates/crible-store/src/attach.rs`, "An error raised after the commit
//! point"): a job can therefore return `internal` while generation `G+1` is
//! already committed. The writer refreshes the published triplet whatever the
//! work returned, so the counts served by `GET /v1/indexes/{index}` follow the
//! committed store; the error of the work stays the error the job reports.
//!
//! The fault is driven from the disk, without a failpoint: the `manifests/`
//! directory of the epoch is replaced by a regular file, so `write_manifest`
//! fails with `internal` after the commit, which no other job step pays.
//!
//! | Test | What it proves |
//! |---|---|
//! | `a_job_failing_after_its_commit_point_still_refreshes_the_counts` | the counts follow the committed store, the served snapshot stays whole |
//!
//! ```text
//! cargo test -p crible-server --test job_failure_after_commit
//! ```

mod common;

use std::path::{Path, PathBuf};

use axum::Router;
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use common::{
    INDEX, Instance, READ_SECRET, base64_vector, instance_with_alpha, post_documents, send,
    terminal_job,
};

/// Body of an online lot that attaches `DOC-<step>`, one passage.
fn batch(step: u64) -> Value {
    json!({
        "batch_id": format!("batch-{step}"),
        "mode": "upsert",
        "documents": [{
            "doc_id": format!("DOC-{step}"),
            "fields": { "numero": format!("{step}") },
            "passages": [{
                "text": format!("le contrat numero {step}"),
                "fields": { "texte": "contrat" },
                "vector": base64_vector([0.6, 0.8, 0.0, 0.0])
            }]
        }]
    })
}

/// `GET /v1/indexes/{index}`, the body of the detail route.
async fn detail(router: &Router) -> Value {
    let path = format!("/v1/indexes/{INDEX}");
    let (status, body) = send(router, Method::GET, &path, READ_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "the index is served: {body}");
    body
}

/// Directory of the single epoch of the fixture index.
fn epoch_directory(instance: &Instance) -> PathBuf {
    let epochs = instance
        .root()
        .join("data")
        .join("indexes")
        .join(INDEX)
        .join("epochs");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&epochs)
        .unwrap_or_else(|error| panic!("the epochs of {INDEX} are readable: {error}"))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("an epoch entry is readable: {error}"))
                .path()
        })
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "the fixture index holds one epoch: {entries:?}"
    );
    entries.remove(0)
}

/// Replaces the `manifests/` directory of `epoch` by a regular file, so the next
/// commit fails **after** its commit point, when it writes the manifest.
fn break_manifests(epoch: &Path) {
    let manifests = epoch.join("manifests");
    assert!(manifests.is_dir(), "the epoch has a manifests directory");
    std::fs::remove_dir_all(&manifests).unwrap_or_else(|error| {
        panic!("the manifests directory is removable: {error}");
    });
    std::fs::write(&manifests, b"not a directory")
        .unwrap_or_else(|error| panic!("the manifests directory is replaceable: {error}"));
}

/// A job that failed after its commit point leaves the counts of the committed
/// store and the snapshot of the last served generation (ADR-0126, decision 4).
#[tokio::test]
async fn a_job_failing_after_its_commit_point_still_refreshes_the_counts() {
    let (instance, state, router) = instance_with_alpha().await;

    let (status, accepted) = post_documents(&router, batch(1)).await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    let job = terminal_job(&router, &state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the first lot attaches: {job}");
    assert_eq!(job["generation"], json!(1), "{job}");

    break_manifests(&epoch_directory(&instance));

    let (status, accepted) = post_documents(&router, batch(2)).await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    let job = terminal_job(&router, &state, &accepted).await;
    assert_eq!(
        job["state"], "failed",
        "the second lot fails after its commit point: {job}"
    );
    assert_eq!(
        job["error"]["code"], "internal",
        "the failure of the manifest is reported: {job}"
    );

    // The refresh ran whatever the work returned: `documents`, `last_batch` and
    // the generation come from the committed store, generation 2 included, while
    // the generation served stays the last one published. An implementation that
    // skipped the refresh on an error would still serve `documents = 1`
    // (ADR-0126, decision 4).
    //
    // `live_passages` and `dead_passages` read the mix of
    // `GenerationStore::state()` and must not be read as a tombstone: the live
    // count comes from the bitmap of the published snapshot, which is still
    // generation 1 and holds one passage, while the total comes from
    // `records.sqlite`, which holds the two passages of the committed history.
    // The difference counts the passage of the generation that is not served
    // yet, not a deletion (`@relecteur`, REUTILISER-LECTEURS-DENSES).
    let body = detail(&router).await;
    for (field, expected) in [
        ("generation", 1),
        ("documents", 2),
        ("live_passages", 1),
        ("dead_passages", 1),
    ] {
        assert_eq!(
            body["state"][field],
            json!(expected),
            "the detail serves the refreshed {field}: {body}"
        );
    }
    assert_eq!(
        body["state"]["last_batch"]["batch_id"],
        json!("batch-2"),
        "the refresh read the committed history: {body}"
    );

    // The published snapshot is the one the index served before the failure:
    // its readers were republished, not emptied, and the second lot is not
    // visible (INV-03).
    let mut search = json!({
        "vector": base64_vector([0.6, 0.8, 0.0, 0.0]),
        "mode": "dense",
        "k": 10,
        "debug": true,
    });
    let path = format!("/v1/indexes/{INDEX}/search");
    let (status, answer) = send(
        &router,
        Method::POST,
        &path,
        READ_SECRET,
        Some(search.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the index still answers: {answer}");
    assert_eq!(
        answer["results"].as_array().map(Vec::len),
        Some(1),
        "the served generation is the first lot alone: {answer}"
    );

    // A second request serves the same state: the refresh left no mixture.
    search["debug"] = json!(false);
    let (status, answer) = send(&router, Method::POST, &path, READ_SECRET, Some(search)).await;
    assert_eq!(status, StatusCode::OK, "the index answers twice: {answer}");
    assert_eq!(
        answer["results"].as_array().map(Vec::len),
        Some(1),
        "{answer}"
    );
}
