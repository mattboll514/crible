// SPDX-License-Identifier: Apache-2.0

// Test fixtures may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixture below builds an index and an epoch.
#![allow(clippy::panic)]

//! A job that fails **after** its commit point still refreshes the published
//! state, and the refreshed summary describes the snapshot it serves
//! (INV-03, INV-04; ADR-0105, decision 3; ADR-0126, decision 4).
//!
//! The commit of a generation is the SQLite commit of the epoch, and building
//! the snapshot and writing the manifest happen **after** it
//! (`crates/crible-store/src/attach.rs`, "An error raised after the commit
//! point"): a job can therefore return `internal` while generation `G+1` is
//! already committed and never published. The writer refreshes the published
//! triplet whatever the work returned, so the state served by
//! `GET /v1/indexes/{index}` is not left behind the store; the error of the work
//! stays the error the job reports.
//!
//! The counts of that refreshed summary describe the **served** snapshot, not the
//! committed reference state: `documents`, `live_passages` and `dead_passages`
//! are the ones the snapshot recorded when it was built (`docs/05-api.md:128`,
//! "les comptes sont ceux de l'instantané publié"). `last_batch` is the one
//! exception, and deliberately so: it is the last row of `batches`
//! (ADR-0105, decision 3), so it may name the batch of the committed generation
//! the readers do not see yet. That asymmetry is what the first test asserts.
//!
//! The fault is driven from the disk, without a failpoint: the `manifests/`
//! directory of the epoch is replaced by a regular file, so `write_manifest`
//! fails with `internal` after the commit, which no other job step pays.
//!
//! | Test | What it proves |
//! |---|---|
//! | `a_job_failing_after_its_commit_point_still_refreshes_the_counts` | the summary describes the served snapshot, `last_batch` names the committed row |
//! | `a_patch_after_the_failure_agrees_with_the_detail_and_the_search` | a later `PATCH` publishes the committed generation, and the detail, the list and a search agree |
//! | `an_attach_after_the_failure_rebuilds_the_counts_from_the_reference_state` | a commit that follows an unpublished one no longer applies a delta to a stale snapshot |
//! | `a_lexical_patch_after_the_failure_publishes_the_committed_generation` | a lexical `PATCH` after the failure takes the `reconfigure` path and publishes the committed generation |
//! | `the_published_counts_match_the_reference_state_after_each_kind_of_commit` | upsert, replacement, deletion, metadata patch, replay and compaction keep the published counts equal to `count(*)` |
//!
//! ```text
//! cargo test -p crible-server --test job_failure_after_commit
//! ```

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::state::AppState;
use serde_json::{Value, json};

use common::{
    ADMIN_SECRET, INDEX, Instance, READ_SECRET, WRITE_SECRET, base64_vector, instance_with_alpha,
    post_documents, send, terminal_job,
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

/// `GET /v1/indexes`, the entry of the fixture index.
async fn list_entry(router: &Router) -> Value {
    let (status, body) = send(router, Method::GET, "/v1/indexes", READ_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "the indexes are listed: {body}");
    body["indexes"]
        .as_array()
        .and_then(|indexes| {
            indexes
                .iter()
                .find(|entry| entry["name"] == json!(INDEX))
                .cloned()
        })
        .unwrap_or_else(|| panic!("the list holds {INDEX}: {body}"))
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

/// Restores the `manifests/` directory of `epoch`, so the next commit can write
/// its manifest: the cause of the failure is gone, the committed generation stays
/// unpublished until a later commit republishes it.
fn repair_manifests(epoch: &Path) {
    let manifests = epoch.join("manifests");
    std::fs::remove_file(&manifests).unwrap_or_else(|error| {
        panic!("the manifests file is removable: {error}");
    });
    std::fs::create_dir(&manifests)
        .unwrap_or_else(|error| panic!("the manifests directory comes back: {error}"));
}

/// Counts of the reference state of `epoch`: document rows, passage rows and live
/// passages. They are read from `records.sqlite`, the authority the snapshot must
/// agree with (ADR-0105, decision 3).
fn reference_counts(instance: &Instance, epoch: u64) -> (u64, u64, u64) {
    let path = instance
        .root()
        .join("data")
        .join("indexes")
        .join(INDEX)
        .join("epochs")
        .join(epoch.to_string())
        .join("records.sqlite");
    let connection = rusqlite::Connection::open(&path)
        .unwrap_or_else(|error| panic!("{} opens: {error}", path.display()));
    let count = |sql: &str| -> u64 {
        let value: i64 = connection
            .query_row(sql, [], |row| row.get(0))
            .unwrap_or_else(|error| panic!("`{sql}` reads: {error}"));
        u64::try_from(value).unwrap_or_else(|_| panic!("`{sql}` counts a negative number: {value}"))
    };
    let documents = count("SELECT count(*) FROM documents");
    let passages = count("SELECT count(*) FROM passages");
    let live = count("SELECT count(*) FROM passages WHERE dead_since_gen IS NULL");
    (documents, passages, live)
}

/// Checks that the counts of the served snapshot are the ones of the reference
/// state of its epoch.
async fn assert_counts_match_the_reference_state(instance: &Instance, router: &Router, step: &str) {
    let body = detail(router).await;
    let epoch = body["state"]["epoch"]
        .as_u64()
        .unwrap_or_else(|| panic!("{step}: the state names its epoch: {body}"));
    let (documents, passages, live) = reference_counts(instance, epoch);
    let dead = passages
        .checked_sub(live)
        .unwrap_or_else(|| panic!("{step}: {live} live passages are among the {passages} rows"));
    for (field, expected) in [
        ("documents", documents),
        ("live_passages", live),
        ("dead_passages", dead),
    ] {
        assert_eq!(
            body["state"][field],
            json!(expected),
            "{step}: the served snapshot counts the {field} of its reference state: {body}"
        );
    }
}

/// Attaches the lot of `step` and checks that it succeeded.
async fn attach(router: &Router, state: &Arc<AppState>, step: u64) {
    let (status, accepted) = post_documents(router, batch(step)).await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    let job = terminal_job(router, state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the lot attaches: {job}");
    assert_eq!(job["generation"], json!(step), "{job}");
}

/// Runs the failure sequence: the first lot is served, the second one commits
/// and then fails before its snapshot and manifest are published.
///
/// Returns the instance, its state and its router.
async fn instance_with_a_failure_after_the_commit() -> (Instance, Arc<AppState>, Router) {
    let (instance, state, router) = instance_with_alpha().await;
    attach(&router, &state, 1).await;
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
    (instance, state, router)
}

/// A job that failed after its commit point leaves a summary that describes the
/// served snapshot, and `last_batch` still names the last committed row
/// (ADR-0105, decision 3).
#[tokio::test]
async fn a_job_failing_after_its_commit_point_still_refreshes_the_counts() {
    let (_instance, _state, router) = instance_with_a_failure_after_the_commit().await;

    // The refresh ran whatever the work returned: an implementation that skipped
    // it on an error would still serve the summary of the creation, with no
    // batch (ADR-0126, decision 4). But every count is the one of generation 1,
    // the snapshot the readers hold: generation 2 is committed in
    // `records.sqlite` and invisible, so it contributes neither a document nor a
    // dead passage (ADR-0105, decision 3, `docs/05-api.md:128`).
    //
    // `last_batch` is the exception the decision 3 of ADR-0105 writes: it is the
    // last row of `batches`, so it names `batch-2` although generation 2 is not
    // served. That asymmetry is wanted, not a mixture of two generations: the
    // row describes the reference state, every other field the served snapshot.
    let body = detail(&router).await;
    for (field, expected) in [
        ("generation", json!(1)),
        ("documents", json!(1)),
        ("live_passages", json!(1)),
        ("dead_passages", json!(0)),
        ("dead_ratio", json!(0.0)),
    ] {
        assert_eq!(
            body["state"][field], expected,
            "the detail describes the served snapshot in {field}: {body}"
        );
    }
    assert_eq!(
        body["state"]["last_batch"]["batch_id"],
        json!("batch-2"),
        "the last row of batches is reported, whatever generation the readers serve: {body}"
    );
    assert_eq!(
        body["state"]["last_batch"]["generation"],
        json!(2),
        "the row is the one of the committed batch: {body}"
    );

    // The list route reads the same summary: its counts describe the served
    // snapshot too.
    let entry = list_entry(&router).await;
    for (field, expected) in [
        ("generation", json!(1)),
        ("documents", json!(1)),
        ("live_passages", json!(1)),
    ] {
        assert_eq!(
            entry[field], expected,
            "the list describes the served snapshot in {field}: {entry}"
        );
    }

    // The served generation is the first lot alone: the counts and the search
    // agree on one passage (INV-03).
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

/// A `PATCH` that follows the failed job publishes generation 2 with its own
/// counts, and the detail, the list and a search agree on it.
#[tokio::test]
async fn a_patch_after_the_failure_agrees_with_the_detail_and_the_search() {
    let (_instance, state, router) = instance_with_a_failure_after_the_commit().await;

    // A patch that changes no lexical setting runs `patch_config`
    // (ADR-0105, decision 4). Its job rebuilds the snapshot from the reference
    // state, which holds the generation the failed job committed, and publishes
    // configuration, snapshot and counts together.
    let path = format!("/v1/indexes/{INDEX}?wait=true");
    let (status, accepted) = send(
        &router,
        Method::PATCH,
        &path,
        ADMIN_SECRET,
        Some(json!({ "search": { "rescore_candidates": 20 } })),
    )
    .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::ACCEPTED,
        "the patch is accepted: {accepted}"
    );
    assert_eq!(accepted["kind"], "patch_config", "{accepted}");
    let job = terminal_job(&router, &state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the patch applies: {job}");

    // Generation 2 is now served with the counts of the state it was built
    // from: two documents, two passages, no tombstone.
    let body = detail(&router).await;
    for (field, expected) in [
        ("generation", json!(2)),
        ("documents", json!(2)),
        ("live_passages", json!(2)),
        ("dead_passages", json!(0)),
        ("dead_ratio", json!(0.0)),
    ] {
        assert_eq!(
            body["state"][field], expected,
            "the detail of the republished generation: {body}"
        );
    }
    assert_eq!(
        body["state"]["last_batch"]["batch_id"],
        json!("batch-2"),
        "the last row of batches is unchanged: {body}"
    );

    let entry = list_entry(&router).await;
    for (field, expected) in [
        ("generation", json!(2)),
        ("documents", json!(2)),
        ("live_passages", json!(2)),
    ] {
        assert_eq!(
            entry[field], expected,
            "the list agrees with the detail on {field}: {entry}"
        );
    }

    // The search serves the same generation: two passages, as the detail says.
    let search = json!({
        "vector": base64_vector([0.6, 0.8, 0.0, 0.0]),
        "mode": "dense",
        "k": 10,
        "debug": true,
    });
    let path = format!("/v1/indexes/{INDEX}/search");
    let (status, answer) = send(&router, Method::POST, &path, READ_SECRET, Some(search)).await;
    assert_eq!(status, StatusCode::OK, "the index answers: {answer}");
    assert_eq!(
        answer["results"].as_array().map(Vec::len),
        Some(2),
        "the search and the detail agree on the two passages: {answer}"
    );
}

/// Body of an online lot that replaces `doc_id` by `text`, one passage.
fn document(doc_id: &str, text: &str) -> Value {
    json!({
        "doc_id": doc_id,
        "fields": { "numero": doc_id },
        "passages": [{
            "text": text,
            "fields": { "texte": "contrat" },
            "vector": base64_vector([0.6, 0.8, 0.0, 0.0])
        }]
    })
}

/// Posts `body` on the documents route and checks that its job succeeded.
async fn apply(router: &Router, state: &Arc<AppState>, body: Value, step: &str) -> Value {
    let (status, accepted) = post_documents(router, body).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{step}: the batch is accepted: {accepted}"
    );
    let job = terminal_job(router, state, &accepted).await;
    assert_eq!(
        job["state"], "succeeded",
        "{step}: the batch applies: {job}"
    );
    job
}

/// A commit that follows an unpublished one reads the counts from the reference
/// state: the delta of the consecutive branch does not cover the generation the
/// failed manifest left behind (ADR-0105, decision 3).
///
/// Sequence: generation 1 is published, generation 2 commits and fails on its
/// manifest, then the cause is repaired and generation 3 commits. The snapshot is
/// rebuilt from `records.sqlite`, so the counts and the bitmaps describe the same
/// generation and `dead_passages` no longer subtracts a live passage.
#[tokio::test]
async fn an_attach_after_the_failure_rebuilds_the_counts_from_the_reference_state() {
    let (instance, state, router) = instance_with_a_failure_after_the_commit().await;
    // The failure leaves the served snapshot on generation 1; the reference state
    // holds generation 2, so the counts deliberately differ until a later commit
    // republishes the committed state (ADR-0105, decision 3).
    let body = detail(&router).await;
    for (field, expected) in [("generation", json!(1)), ("documents", json!(1))] {
        assert_eq!(
            body["state"][field], expected,
            "the failure serves generation 1 in {field}: {body}"
        );
    }

    repair_manifests(&epoch_directory(&instance));
    attach(&router, &state, 3).await;

    let body = detail(&router).await;
    assert_eq!(
        body["state"]["generation"],
        json!(3),
        "the third lot is served: {body}"
    );
    assert_counts_match_the_reference_state(&instance, &router, "after the third lot").await;

    // The counts and the search agree on the whole committed state (INV-03).
    let search = json!({
        "vector": base64_vector([0.6, 0.8, 0.0, 0.0]),
        "mode": "dense",
        "k": 10,
    });
    let path = format!("/v1/indexes/{INDEX}/search");
    let (status, answer) = send(&router, Method::POST, &path, READ_SECRET, Some(search)).await;
    assert_eq!(status, StatusCode::OK, "the index answers: {answer}");
    assert_eq!(
        answer["results"].as_array().map(Vec::len),
        Some(3),
        "the three committed passages are served: {answer}"
    );
}

/// A lexical rebuild after the failure publishes the generation it rebuilt, not
/// the counts of the snapshot it found: `rebuild_lexical` goes through the
/// non-consecutive branch too (ADR-0105, decision 4).
#[tokio::test]
async fn a_lexical_patch_after_the_failure_publishes_the_committed_generation() {
    let (instance, state, router) = instance_with_a_failure_after_the_commit().await;

    let path = format!("/v1/indexes/{INDEX}?wait=true");
    let (status, accepted) = send(
        &router,
        Method::PATCH,
        &path,
        ADMIN_SECRET,
        Some(json!({ "lexical": { "enabled": true } })),
    )
    .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::ACCEPTED,
        "the patch is accepted: {accepted}"
    );
    assert_eq!(accepted["kind"], "rebuild_lexical", "{accepted}");
    let job = terminal_job(&router, &state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the rebuild applies: {job}");

    let body = detail(&router).await;
    assert_eq!(
        body["state"]["generation"],
        json!(2),
        "the rebuilt generation is the committed one: {body}"
    );
    assert_counts_match_the_reference_state(&instance, &router, "after the rebuild").await;

    // The search serves the two passages the counts describe.
    let search = json!({
        "vector": base64_vector([0.6, 0.8, 0.0, 0.0]),
        "mode": "dense",
        "k": 10,
    });
    let path = format!("/v1/indexes/{INDEX}/search");
    let (status, answer) = send(&router, Method::POST, &path, READ_SECRET, Some(search)).await;
    assert_eq!(status, StatusCode::OK, "the index answers: {answer}");
    assert_eq!(
        answer["results"].as_array().map(Vec::len),
        Some(2),
        "the two committed passages are served: {answer}"
    );
}

/// Every kind of commit leaves the published counts equal to the reference state:
/// upsert, replacement of an existing document, deletion, metadata patch, replay
/// of a batch (INV-10) and compaction.
#[tokio::test]
async fn the_published_counts_match_the_reference_state_after_each_kind_of_commit() {
    let (instance, state, router) = instance_with_alpha().await;

    apply(
        &router,
        &state,
        json!({
            "batch_id": "batch-alpha",
            "mode": "upsert",
            "documents": [
                document("DOC-A", "le contrat alpha"),
                document("DOC-B", "le contrat beta"),
            ],
        }),
        "two new documents",
    )
    .await;
    assert_counts_match_the_reference_state(&instance, &router, "after two new documents").await;

    apply(
        &router,
        &state,
        json!({
            "batch_id": "batch-alpha-2",
            "mode": "upsert",
            "documents": [document("DOC-A", "le contrat alpha, modifie")],
        }),
        "a replaced document",
    )
    .await;
    assert_counts_match_the_reference_state(&instance, &router, "after a replaced document").await;

    apply(
        &router,
        &state,
        json!({
            "batch_id": "batch-delete",
            "documents": [],
            "deletions": ["DOC-B"],
        }),
        "a deletion",
    )
    .await;
    assert_counts_match_the_reference_state(&instance, &router, "after a deletion").await;

    let path = format!("/v1/indexes/{INDEX}/documents?wait=true");
    let (status, accepted) = send(
        &router,
        Method::PATCH,
        &path,
        WRITE_SECRET,
        Some(json!({
            "documents": [{ "doc_id": "DOC-A", "fields": { "numero": "9999" } }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the patch is queued: {accepted}");
    let job = terminal_job(&router, &state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the patch applies: {job}");
    assert_counts_match_the_reference_state(&instance, &router, "after a metadata patch").await;

    // INV-10: the same `batch_id` with the same content writes nothing.
    let before = detail(&router).await;
    apply(
        &router,
        &state,
        json!({
            "batch_id": "batch-alpha-2",
            "mode": "upsert",
            "documents": [document("DOC-A", "le contrat alpha, modifie")],
        }),
        "a replay",
    )
    .await;
    let after = detail(&router).await;
    assert_eq!(
        after["state"]["generation"], before["state"]["generation"],
        "the replay publishes no generation: {after}"
    );
    assert_counts_match_the_reference_state(&instance, &router, "after a replay").await;

    let path = format!("/v1/indexes/{INDEX}/compact?wait=true");
    let (status, accepted) = send(&router, Method::POST, &path, ADMIN_SECRET, None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the compaction is queued: {accepted}"
    );
    let job = terminal_job(&router, &state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the compaction applies: {job}");
    assert_counts_match_the_reference_state(&instance, &router, "after a compaction").await;

    // The compaction removed the two tombstones: one document, one passage.
    let body = detail(&router).await;
    for (field, expected) in [
        ("documents", json!(1)),
        ("live_passages", json!(1)),
        ("dead_passages", json!(0)),
    ] {
        assert_eq!(
            body["state"][field], expected,
            "the compacted generation holds no tombstone in {field}: {body}"
        );
    }
}
