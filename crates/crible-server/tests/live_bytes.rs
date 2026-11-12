// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixtures of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! `state.sizes.live_bytes` of `GET /v1/indexes/{index}` (ADR-0152).
//!
//! The member is the sum, over the segments of the published generation, of
//! `segment.json` and of the vector file. It is computed from the published
//! state: no walk of the directory and no `stat` on the path of the route.
//!
//! | Test | Criterion of the task |
//! |---|---|
//! | `live_bytes_is_zero_and_present_on_an_empty_index` | `0` for an index without a segment, member always present |
//! | `live_bytes_grows_by_the_files_of_the_attached_segment` | the growth at an attach equals the size of the files of the added segment, and the value equals the sum over the published segments |
//! | `live_bytes_is_identical_after_a_restart` | the same value before and after a restart on the same data root |
//! | `a_temporary_directory_and_a_wal_do_not_change_live_bytes` | no walk: a `tmp-*` and a `-wal` added in the index directory move `disk_bytes`, not `live_bytes` |
//!
//! A restart is a new process: the writer task of the first instance holds its
//! store, and with it the writer lock of the index, until its runtime goes
//! away. Each phase therefore runs in a runtime of its own, dropped before the
//! next one opens the same data root, as `inv_04_delete_job_after_restart`
//! does.
//!
//! ```text
//! cargo test -p crible-server --test live_bytes
//! ```

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::http::build_router;
use crible_server::state::AppState;
use serde_json::{Value, json};

use common::{
    ADMIN_SECRET, DOC_ID, INDEX, Instance, READ_SECRET, base64_vector, index_config,
    instance_with_alpha, post_documents, send, terminal_job,
};

/// The two sizes of the detail body, with the shape of the published state.
struct Detail {
    /// Bytes of the directory on disk, transients included.
    disk_bytes: u64,
    /// Bytes of the files of the published segments.
    live_bytes: u64,
    /// Number of segments of the published generation.
    segments: u64,
    /// Epoch directory holding the published state.
    epoch: u64,
}

/// Reads `GET /v1/indexes/{index}` and returns the sizes of its state.
async fn detail(router: &Router) -> Detail {
    let path = format!("/v1/indexes/{INDEX}");
    let (status, body) = send(router, Method::GET, &path, READ_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "the detail is readable: {body}");
    let sizes = &body["state"]["sizes"];
    Detail {
        disk_bytes: number(sizes, "disk_bytes", &body),
        live_bytes: number(sizes, "live_bytes", &body),
        segments: number(&body["state"], "segments", &body),
        epoch: number(&body["state"], "epoch", &body),
    }
}

/// Reads one unsigned member of the body, naming it when it is absent.
fn number(object: &Value, member: &str, body: &Value) -> u64 {
    object[member].as_u64().unwrap_or_else(|| {
        panic!("`{member}` is an unsigned integer of the body: {body}");
    })
}

/// Attaches one document of one passage through `POST /v1/indexes/{index}/documents`
/// and waits for its job.
async fn attach(router: &Router, state: &Arc<AppState>, batch_id: &str, doc_id: &str) {
    let body = json!({
        "batch_id": batch_id,
        "mode": "upsert",
        "documents": [
            {
                "doc_id": doc_id,
                "fields": { "numero": "1240" },
                "passages": [
                    {
                        "text": "le contrat fait la loi",
                        "fields": { "texte": "contrat" },
                        "vector": base64_vector([0.6, 0.8, 0.0, 0.0])
                    }
                ]
            }
        ]
    });
    let (status, accepted) = post_documents(router, body).await;
    assert_eq!(status, StatusCode::OK, "the batch is accepted: {accepted}");
    let job = terminal_job(router, state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the batch attaches: {job}");
}

/// `<data>/indexes/<name>`, the directory of the fixture index.
fn index_directory(instance: &Instance) -> PathBuf {
    instance.root().join("data").join("indexes").join(INDEX)
}

/// Directories of the published segments of `epoch`, sorted, transients
/// excluded.
fn segment_directories(instance: &Instance, epoch: u64) -> Vec<PathBuf> {
    let directory = index_directory(instance)
        .join("epochs")
        .join(epoch.to_string())
        .join("segments");
    let mut paths: Vec<PathBuf> = fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("{} is readable: {error}", directory.display()))
        .map(|entry| entry.expect("an entry of the segments directory").path())
        .filter(|path| path.is_dir())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_none_or(|name| !name.starts_with("tmp-"))
        })
        .collect();
    paths.sort();
    paths
}

/// Sum of the files `live_bytes` counts in one segment directory: its
/// `segment.json` and its vector file, never `codes.bin`.
fn live_file_bytes(directory: &Path) -> u64 {
    fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("{} is readable: {error}", directory.display()))
        .map(|entry| {
            let entry = entry.expect("an entry of the segment directory");
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "segment.json" || name.starts_with("vectors.") {
                entry
                    .metadata()
                    .unwrap_or_else(|error| panic!("{name} is statable: {error}"))
                    .len()
            } else {
                0
            }
        })
        .sum()
}

/// `live_bytes` is an integer, always present, worth `0` for an index without
/// a segment (ADR-0152, decision 2).
#[tokio::test]
async fn live_bytes_is_zero_and_present_on_an_empty_index() {
    let (_instance, _state, router) = instance_with_alpha().await;
    let path = format!("/v1/indexes/{INDEX}");
    let (status, body) = send(&router, Method::GET, &path, READ_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let sizes = body["state"]["sizes"]
        .as_object()
        .unwrap_or_else(|| panic!("`sizes` is an object: {body}"));
    assert_eq!(
        sizes.get("live_bytes"),
        Some(&json!(0)),
        "the member is always present and worth 0 without a segment: {body}"
    );
    assert!(
        sizes["disk_bytes"].as_u64().is_some_and(|bytes| bytes > 0),
        "the directory of an empty index already holds files: {body}"
    );
    assert_eq!(
        detail(&router).await.segments,
        0,
        "the fixture index references no segment"
    );
}

/// The growth of an attach is exactly the size of the files of the segment the
/// attach added, and the whole value is the sum over the published segments.
#[tokio::test]
async fn live_bytes_grows_by_the_files_of_the_attached_segment() {
    let (instance, state, router) = instance_with_alpha().await;
    assert_eq!(
        detail(&router).await.live_bytes,
        0,
        "the fixture index starts without a segment"
    );

    attach(&router, &state, "batch-one", DOC_ID).await;
    let first = detail(&router).await;
    let directories = segment_directories(&instance, first.epoch);
    assert_eq!(
        directories.len() as u64,
        first.segments,
        "the published generation references every segment of its epoch"
    );
    assert_eq!(
        first.live_bytes,
        directories
            .iter()
            .map(|directory| live_file_bytes(directory))
            .sum::<u64>(),
        "the member is the sum of the files of the published segments"
    );

    attach(&router, &state, "batch-two", "DOC-2").await;
    let second = detail(&router).await;
    let after = segment_directories(&instance, second.epoch);
    let added: Vec<&PathBuf> = after
        .iter()
        .filter(|path| !directories.contains(path))
        .collect();
    assert_eq!(added.len(), 1, "an attach adds exactly one segment");
    let added_bytes: u64 = added
        .iter()
        .map(|directory| live_file_bytes(directory))
        .sum();
    assert!(
        added_bytes > 0,
        "the added segment carries its segment.json and its vector file"
    );
    assert_eq!(
        second.live_bytes - first.live_bytes,
        added_bytes,
        "the growth is the size of the files of the added segment"
    );
    assert_eq!(
        second.live_bytes,
        after
            .iter()
            .map(|directory| live_file_bytes(directory))
            .sum::<u64>(),
        "the value stays the sum over the published segments"
    );
}

/// One process attaches a batch and reads `live_bytes`; the next process, on
/// the same data root, reads the same value (ADR-0152, decision 4).
#[test]
fn live_bytes_is_identical_after_a_restart() {
    let instance = Instance::new();
    let before = attach_in_a_fresh_process(&instance);
    assert!(before > 0, "the attached segment gives the member a value");
    let after = live_bytes_in_a_fresh_process(&instance);
    assert_eq!(
        before, after,
        "the restart changes neither the generation nor the table of segments, \
         and live_bytes reads no file (ADR-0152, decisions 3 and 4)"
    );
}

/// Creates the fixture index in a runtime of its own, attaches one batch,
/// returns `live_bytes`, then drops the runtime as the death of the process
/// would end its writer tasks.
fn attach_in_a_fresh_process(instance: &Instance) -> u64 {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime is creatable");
    let (state, router) = {
        let _entered = runtime.enter();
        let state = instance.state();
        (Arc::clone(&state), build_router(state))
    };
    let value = runtime.block_on(async {
        let (status, body) = send(
            &router,
            Method::POST,
            "/v1/indexes",
            ADMIN_SECRET,
            Some(index_config()),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "the index is created: {body}");
        attach(&router, &state, "batch-one", DOC_ID).await;
        detail(&router).await.live_bytes
    });
    drop(runtime);
    value
}

/// Loads the same data root in a fresh runtime of its own and reads
/// `live_bytes`.
fn live_bytes_in_a_fresh_process(instance: &Instance) -> u64 {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime is creatable");
    let router = {
        let _entered = runtime.enter();
        build_router(instance.state())
    };
    let value = runtime.block_on(async { detail(&router).await.live_bytes });
    drop(runtime);
    value
}

/// A `tmp-*` directory and a `-wal` file added under the index directory move
/// `disk_bytes` and leave `live_bytes` unchanged: the member never walks the
/// directory (ADR-0152, decisions 2 and 3).
///
/// SQLite keeps the WAL of the reference database beside it, under
/// `epochs/<E>/` (`docs/02-architecture.md`, "Organisation sur disque"), so the
/// decoy is written there. It is named `decoy.sqlite-wal` and never
/// `records.sqlite-wal`: the database of the running store is open, and
/// overwriting the WAL of a live database is not a test's business.
#[tokio::test]
async fn a_temporary_directory_and_a_wal_do_not_change_live_bytes() {
    let (instance, state, router) = instance_with_alpha().await;
    attach(&router, &state, "batch-one", DOC_ID).await;
    let before = detail(&router).await;

    let epoch = index_directory(&instance)
        .join("epochs")
        .join(before.epoch.to_string());
    fs::write(epoch.join("decoy.sqlite-wal"), vec![0_u8; 4096])
        .expect("a wal-like file is writable beside the reference database");
    let staging = index_directory(&instance).join("tmp-attach");
    fs::create_dir_all(&staging).expect("a staging directory is creatable");
    fs::write(staging.join("segment.json"), b"{}\n").expect("a staging file is writable");

    let after = detail(&router).await;
    assert_eq!(
        after.live_bytes, before.live_bytes,
        "neither the wal nor the staging directory enters the sum"
    );
    assert_eq!(
        after.segments, before.segments,
        "the published generation did not change"
    );
    assert!(
        after.disk_bytes > before.disk_bytes,
        "disk_bytes walks the directory and counts them: {} > {}",
        after.disk_bytes,
        before.disk_bytes
    );
}
