// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and several fixtures and checks of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! `POST /v1/indexes/{index}/batches` — the path of a deposited batch stays in
//! the inbox (`docs/phases/P6-serveur-et-api.md`, `ROUTES-DOCUMENTS-ET-LOTS`,
//! criterion "un chemin de lot hors de l'inbox est refusé"; ADR-0107,
//! decision 5, and `docs/05-api.md`, "Lot déposé").
//!
//! Every refusal is `400 invalid_request`, its message names the field
//! `directory`, it never carries an absolute path of the server (ADR-0101), and
//! no job is queued. An absent directory is `404 not_found`.
//!
//! The tests need no Parquet fixture: they pin the validation, which happens
//! before the batch is read. The attachment of a valid batch is proved by the
//! store tests.
//!
//! | Test | Case of ADR-0107, decision 5 |
//! |---|---|
//! | `a_batch_directory_with_a_parent_component_is_refused` | `..` |
//! | `a_batch_directory_that_is_a_dot_is_refused` | `.` |
//! | `a_batch_directory_with_a_separator_is_refused` | `/` and `\` |
//! | `a_batch_directory_with_an_absolute_path_is_refused` | absolute path |
//! | `an_empty_batch_directory_is_refused` | empty string |
//! | `a_batch_directory_named_by_a_symlink_is_refused` | `<nom>` is a symbolic link |
//! | `a_symlinked_file_of_the_batch_is_refused` | symbolic link inside the batch |
//! | `a_symlink_in_a_subdirectory_of_the_batch_is_refused` | link at any depth |
//! | `a_symlink_to_another_batch_of_the_inbox_is_refused` | link to another batch |
//! | `a_special_entry_of_the_batch_is_refused` | entry neither file nor directory |
//! | `an_absent_batch_directory_is_not_found` | absent directory → `404` |
//!
//! ```text
//! cargo test -p crible-server --test batch_path_security
//! ```

use axum::Router;
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

mod common;

use common::{INDEX, Instance, WRITE_SECRET, send};

/// Returns the instance and its router, over the fixture of [`common`].
async fn instance_with_alpha() -> (Instance, Router) {
    let (instance, _state, router) = common::instance_with_alpha().await;
    (instance, router)
}

/// Submits `directory` to the batch route.
async fn submit(router: &Router, directory: &str) -> (StatusCode, Value) {
    let path = format!("/v1/indexes/{INDEX}/batches");
    send(
        router,
        Method::POST,
        &path,
        WRITE_SECRET,
        Some(json!({ "directory": directory })),
    )
    .await
}

/// Asserts that the job board of the index holds no job: a refused submission
/// queues nothing (ADR-0107, decision 5).
async fn assert_no_job(router: &Router) {
    let (status, jobs) = send(
        router,
        Method::GET,
        &format!("/v1/jobs?index={INDEX}"),
        WRITE_SECRET,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{jobs}");
    assert!(
        jobs["jobs"].as_array().is_some_and(Vec::is_empty),
        "a refused batch queues no job: {jobs}"
    );
}

/// Asserts the `400 invalid_request` of a refused `directory`: the code, the
/// field named in the message, no server path in the message, no job queued.
async fn assert_refused(instance: &Instance, router: &Router, directory: &str) {
    let (status, body) = submit(router, directory).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "directory {directory:?} is refused: {body}"
    );
    assert_eq!(
        body["error"]["code"], "invalid_request",
        "directory {directory:?}: {body}"
    );
    let message = body["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the refusal carries a message: {body}"));
    assert!(
        message.contains("directory"),
        "the message names the field 'directory': {message}"
    );
    let root = instance.root().display().to_string();
    assert!(
        !message.contains(&root),
        "the message carries an absolute path of the server: {message}"
    );
    assert!(
        !message.contains(&instance.inbox().display().to_string()),
        "the message carries the inbox path: {message}"
    );
    assert_no_job(router).await;
}

#[tokio::test]
async fn a_batch_directory_with_a_parent_component_is_refused() {
    // ADR-0107, decision 5: `<nom>` is one path component; `..` is refused by
    // `path_component` before the inbox is canonicalised.
    let (instance, router) = instance_with_alpha().await;
    assert_refused(&instance, &router, "..").await;
}

#[tokio::test]
async fn a_batch_directory_that_is_a_dot_is_refused() {
    // ADR-0107, decision 5: `.` names no batch.
    let (instance, router) = instance_with_alpha().await;
    assert_refused(&instance, &router, ".").await;
}

#[tokio::test]
async fn a_batch_directory_with_a_separator_is_refused() {
    // ADR-0107, decision 5: neither `/` nor `\` may appear in `<nom>`.
    let (instance, router) = instance_with_alpha().await;
    for directory in ["legifacile/2026-10-19", "legifacile\\2026-10-19"] {
        assert_refused(&instance, &router, directory).await;
    }
}

#[tokio::test]
async fn a_batch_directory_with_an_absolute_path_is_refused() {
    // ADR-0107, decision 5: an absolute path would leave the inbox, and it is
    // refused before any canonicalisation.
    let (instance, router) = instance_with_alpha().await;
    assert_refused(&instance, &router, "/tmp/legifacile-2026-10-19").await;
}

#[tokio::test]
async fn an_empty_batch_directory_is_refused() {
    // ADR-0107, decision 5: the empty string names no component.
    let (instance, router) = instance_with_alpha().await;
    assert_refused(&instance, &router, "").await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_batch_directory_named_by_a_symlink_is_refused() {
    // ADR-0107, decision 5, rule 2: `<nom>` itself may not be a symbolic link,
    // even one pointing outside the inbox.
    let (instance, router) = instance_with_alpha().await;
    let target = instance.root().join("outside");
    std::fs::create_dir_all(&target).expect("the target directory is creatable");
    std::fs::write(target.join("manifest.json"), b"{}").expect("the manifest is writable");
    std::os::unix::fs::symlink(&target, instance.inbox().join("linked"))
        .expect("the symbolic link is creatable");
    assert_refused(&instance, &router, "linked").await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlinked_file_of_the_batch_is_refused() {
    // ADR-0107, decision 5, rule 3: no symbolic link inside the batch, even one
    // pointing at a file outside the inbox.
    let (instance, router) = instance_with_alpha().await;
    let outside = instance.root().join("outside.json");
    std::fs::write(&outside, b"{}").expect("the target file is writable");
    let batch = instance.inbox().join("batch");
    std::fs::create_dir_all(&batch).expect("the batch directory is creatable");
    std::os::unix::fs::symlink(&outside, batch.join("manifest.json"))
        .expect("the symbolic link is creatable");
    assert_refused(&instance, &router, "batch").await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_in_a_subdirectory_of_the_batch_is_refused() {
    // ADR-0107, decision 5, rule 3: the refusal holds at any depth.
    let (instance, router) = instance_with_alpha().await;
    let outside = instance.root().join("outside.parquet");
    std::fs::write(&outside, b"not a parquet file").expect("the target file is writable");
    let batch = instance.inbox().join("batch");
    let parts = batch.join("parts");
    std::fs::create_dir_all(&parts).expect("the subdirectory is creatable");
    std::fs::write(batch.join("manifest.json"), b"{}").expect("the manifest is writable");
    std::os::unix::fs::symlink(&outside, parts.join("passages.parquet"))
        .expect("the symbolic link is creatable");
    assert_refused(&instance, &router, "batch").await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_to_another_batch_of_the_inbox_is_refused() {
    // ADR-0107, decision 5, rule 3, second clause: a link to another batch of
    // the inbox is refused too, so a batch never borrows another manifest.
    let (instance, router) = instance_with_alpha().await;
    let inbox = instance.inbox();
    let other = inbox.join("other");
    std::fs::create_dir_all(&other).expect("the other batch is creatable");
    std::fs::write(other.join("manifest.json"), b"{}").expect("the manifest is writable");
    let batch = inbox.join("batch");
    std::fs::create_dir_all(&batch).expect("the batch directory is creatable");
    std::fs::write(batch.join("manifest.json"), b"{}").expect("the manifest is writable");
    std::os::unix::fs::symlink(&other, batch.join("borrowed"))
        .expect("the symbolic link is creatable");
    assert_refused(&instance, &router, "batch").await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_special_entry_of_the_batch_is_refused() {
    // ADR-0107, decision 5, rule 4: an entry that is neither a file nor a
    // directory refuses the batch. A Unix socket is such an entry.
    let (instance, router) = instance_with_alpha().await;
    let batch = instance.inbox().join("batch");
    std::fs::create_dir_all(&batch).expect("the batch directory is creatable");
    std::fs::write(batch.join("manifest.json"), b"{}").expect("the manifest is writable");
    let _socket = std::os::unix::net::UnixListener::bind(batch.join("socket"))
        .expect("the socket is creatable");
    assert_refused(&instance, &router, "batch").await;
}

#[tokio::test]
async fn an_absent_batch_directory_is_not_found() {
    // ADR-0107, decision 5: an absent directory is `404 not_found`, not `400`.
    let (_instance, router) = instance_with_alpha().await;
    let (status, body) = submit(&router, "absent-batch").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["code"], "not_found", "{body}");
    let message = body["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the refusal carries a message: {body}"));
    assert!(
        message.contains("directory"),
        "the message names the field 'directory': {message}"
    );
    assert_no_job(&router).await;
}
