// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the checks of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! Write routes of the documents and the role matrix of
//! `ROUTES-DOCUMENTS-ET-LOTS` (`docs/phases/P6-serveur-et-api.md`, ADR-0107).
//!
//! | Test | Acceptance criterion |
//! |---|---|
//! | `a_replayed_batch_id_writes_nothing_and_returns_the_stored_report` | replay keeps the report and the generation |
//! | `a_replayed_batch_id_with_another_content_fails_in_batch_conflict` | `batch_id` reuse with other content → `batch_conflict` |
//! | `deletions_remove_the_document_of_the_batch` | `deletions` tombstone the document |
//! | `a_patch_of_an_unknown_document_fails_not_found` | business error carried by the job |
//! | `a_patch_replaces_a_document_field` | patch updates the fields, report in memory |
//! | `the_attach_journal_carries_the_durations_once` | one duration line per attach, no content |
//! | `a_read_key_is_forbidden_on_the_write_routes` | role matrix, `read` |
//! | `a_write_key_is_forbidden_on_the_admin_routes` | role matrix, `write` |
//! | `the_admin_routes_refuse_a_non_empty_body` | no body on `compact`/`exports`/`snapshots` |
//!
//! ```text
//! cargo test -p crible-server --test routes_documents
//! ```

mod common;

use axum::Router;
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use common::{
    ADMIN_SECRET, DOC_ID, INDEX, READ_SECRET, WRITE_SECRET, deletion_batch, instance_with_alpha,
    online_batch, post_documents, send, terminal_job,
};

/// Asserts that the job succeeded and returns its report.
fn succeeded(job: &Value) -> &Value {
    assert_eq!(job["state"], "succeeded", "the job succeeds: {job}");
    assert!(
        job["error"].is_null(),
        "a succeeded job carries no error: {job}"
    );
    &job["result"]["report"]
}

/// Asserts that the job failed with `code` and returns its error.
fn failed_with(job: &Value, code: &str) -> Value {
    assert_eq!(job["state"], "failed", "the job fails: {job}");
    let error = job["error"].clone();
    assert_eq!(error["code"], code, "wrong error code: {job}");
    error
}

/// Reads the document of the fixture and returns its body, or `None` when the
/// route answers `404`.
async fn read_document(router: &Router) -> Option<Value> {
    let path = format!("/v1/indexes/{INDEX}/documents/{DOC_ID}");
    let (status, body) = send(router, Method::GET, &path, READ_SECRET, None).await;
    match status {
        StatusCode::OK => Some(body),
        StatusCode::NOT_FOUND => None,
        other => panic!("unexpected status {other}: {body}"),
    }
}

/// Replaying a `batch_id` with the same content writes nothing: the second job
/// succeeds and returns the report of the first one, at the same generation
/// (ADR-0107, decision 3; INV-10).
#[tokio::test]
async fn a_replayed_batch_id_writes_nothing_and_returns_the_stored_report() {
    let (_instance, state, router) = instance_with_alpha().await;
    let body = online_batch("batch-replay", "le contrat fait la loi", "1240");

    let (status, accepted) = post_documents(&router, body.clone()).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the first batch is applied: {accepted}"
    );
    let first = terminal_job(&router, &state, &accepted).await;
    let first_report = succeeded(&first).clone();

    let (status, accepted) = post_documents(&router, body).await;
    assert_eq!(status, StatusCode::OK, "the replay is accepted: {accepted}");
    let second = terminal_job(&router, &state, &accepted).await;
    let second_report = succeeded(&second);

    assert_eq!(
        second_report, &first_report,
        "a replay returns the report of the original attach (INV-10)"
    );
    assert_eq!(
        second["generation"], first["generation"],
        "a replay publishes no generation"
    );

    let document = read_document(&router)
        .await
        .expect("the document of the first batch is visible");
    assert_eq!(
        document["generation"], first["generation"],
        "the replay wrote nothing: the snapshot stays at the first generation"
    );
}

/// Reusing a `batch_id` for different content is a conflict, carried by the job
/// and never by the route (ADR-0107, decision 3).
#[tokio::test]
async fn a_replayed_batch_id_with_another_content_fails_in_batch_conflict() {
    let (_instance, state, router) = instance_with_alpha().await;
    let (status, accepted) = post_documents(
        &router,
        online_batch("batch-conflict", "le contrat fait la loi", "1240"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    let first = terminal_job(&router, &state, &accepted).await;
    succeeded(&first);

    let (status, accepted) = post_documents(
        &router,
        online_batch("batch-conflict", "un autre texte", "1241"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the body has a valid shape, so the route queues it: {accepted}"
    );
    let second = terminal_job(&router, &state, &accepted).await;
    failed_with(&second, "batch_conflict");
}

/// The `deletions` of an online batch remove the document: the next read is
/// `404 not_found` (ADR-0107, decision 1).
#[tokio::test]
async fn deletions_remove_the_document_of_the_batch() {
    let (_instance, state, router) = instance_with_alpha().await;
    let (status, accepted) = post_documents(
        &router,
        online_batch("batch-before-deletion", "le contrat fait la loi", "1240"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    let job = terminal_job(&router, &state, &accepted).await;
    succeeded(&job);
    assert!(
        read_document(&router).await.is_some(),
        "the document is visible before the deletion"
    );

    let (status, accepted) = post_documents(&router, deletion_batch("batch-deletion")).await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    assert!(
        accepted["acceptance"].is_null(),
        "a body that carries 'deletions' only has no acceptance to report (ADR-0107): {accepted}"
    );
    let job = terminal_job(&router, &state, &accepted).await;
    let report = succeeded(&job);
    assert_eq!(
        report["deleted"], 1,
        "the report counts the tombstoned document: {report}"
    );
    assert!(
        read_document(&router).await.is_none(),
        "a deleted document is not found at the generation of the snapshot"
    );
}

/// A patch of a document the index does not hold fails the job in `not_found`,
/// and applies nothing (ADR-0060, ADR-0107).
#[tokio::test]
async fn a_patch_of_an_unknown_document_fails_not_found() {
    let (_instance, state, router) = instance_with_alpha().await;
    let path = format!("/v1/indexes/{INDEX}/documents?wait=true");
    let (status, accepted) = send(
        &router,
        Method::PATCH,
        &path,
        WRITE_SECRET,
        Some(json!({
            "documents": [{ "doc_id": "missing", "fields": { "numero": "1" } }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the patch is queued: {accepted}");
    assert_eq!(accepted["kind"], "patch_documents", "{accepted}");
    let job = terminal_job(&router, &state, &accepted).await;
    let error = failed_with(&job, "not_found");
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|message| message.contains("missing")),
        "the error names the document: {error}"
    );
}

/// A patch replaces the fields of a document and reports `{updated, unchanged,
/// generation}` from memory (ADR-0060, ADR-0103).
#[tokio::test]
async fn a_patch_replaces_a_document_field() {
    let (_instance, state, router) = instance_with_alpha().await;
    let (status, accepted) = post_documents(
        &router,
        online_batch("batch-before-patch", "le contrat fait la loi", "1240"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    let job = terminal_job(&router, &state, &accepted).await;
    succeeded(&job);

    let path = format!("/v1/indexes/{INDEX}/documents?wait=true");
    let (status, accepted) = send(
        &router,
        Method::PATCH,
        &path,
        WRITE_SECRET,
        Some(json!({
            "documents": [{ "doc_id": DOC_ID, "fields": { "numero": "9999" } }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the patch is queued: {accepted}");
    let job = terminal_job(&router, &state, &accepted).await;
    let report = succeeded(&job);
    assert_eq!(report["updated"], 1, "one document is updated: {report}");
    assert_eq!(report["unchanged"], 0, "no document is unchanged: {report}");

    let document = read_document(&router)
        .await
        .expect("the patched document is visible");
    assert_eq!(
        document["fields"]["numero"], "9999",
        "the patch replaced the field: {document}"
    );
    assert_eq!(
        document["passages"][0]["fields"]["texte"], "contrat",
        "the patch left the passage fields untouched: {document}"
    );
}

/// The `read` role reaches neither the write routes nor the admin ones
/// (ADR-0095, ADR-0107, decision 8).
#[tokio::test]
async fn a_read_key_is_forbidden_on_the_write_routes() {
    let (instance, state, router) = instance_with_alpha().await;
    let paths: Vec<(Method, String, Option<Value>)> = vec![
        (
            Method::POST,
            format!("/v1/indexes/{INDEX}/documents"),
            Some(online_batch("batch-read", "le contrat fait la loi", "1240")),
        ),
        (
            Method::PATCH,
            format!("/v1/indexes/{INDEX}/documents"),
            Some(json!({ "documents": [] })),
        ),
        (
            Method::POST,
            format!("/v1/indexes/{INDEX}/batches"),
            Some(json!({ "directory": "deposited" })),
        ),
    ];
    for (method, path, body) in paths {
        let (status, answer) = send(&router, method.clone(), &path, READ_SECRET, body).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{method} {path} is write only: {answer}"
        );
        assert_eq!(answer["error"]["code"], "forbidden", "{answer}");
    }
    // No job was queued by a forbidden request (ADR-0107, decision 3).
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
        jobs["jobs"].as_array().is_some_and(Vec::is_empty),
        "a forbidden request queues no job: {jobs}"
    );
    drop(state);
    drop(instance);
}

/// The `write` role reaches neither `compact`, `exports` nor `snapshots`
/// (ADR-0107, decision 7).
#[tokio::test]
async fn a_write_key_is_forbidden_on_the_admin_routes() {
    let (_instance, _state, router) = instance_with_alpha().await;
    for suffix in ["compact", "exports", "snapshots"] {
        let path = format!("/v1/indexes/{INDEX}/{suffix}");
        let (status, answer) = send(&router, Method::POST, &path, WRITE_SECRET, None).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{path} is admin only: {answer}"
        );
        assert_eq!(answer["error"]["code"], "forbidden", "{answer}");
    }
}

/// The three admin routes take no body: a non-empty one is `400
/// invalid_request`, whatever its content (ADR-0107, decision 7).
#[tokio::test]
async fn the_admin_routes_refuse_a_non_empty_body() {
    let (_instance, _state, router) = instance_with_alpha().await;
    for suffix in ["compact", "exports", "snapshots"] {
        let path = format!("/v1/indexes/{INDEX}/{suffix}");
        let (status, answer) = send(
            &router,
            Method::POST,
            &path,
            ADMIN_SECRET,
            Some(json!({ "destination": "anywhere" })),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{path} must refuse a body: {answer}"
        );
        assert_eq!(answer["error"]["code"], "invalid_request", "{answer}");
    }
}

/// Each admin route publishes its report as a job result, and that result
/// carries no duration and no absolute path of the server (ADR-0042, ADR-0101,
/// ADR-0107, decision 7).
#[tokio::test]
async fn each_admin_route_publishes_its_report() {
    let (instance, state, router) = instance_with_alpha().await;
    let (status, accepted) = post_documents(
        &router,
        online_batch("batch-before-maintenance", "le contrat fait la loi", "1240"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    let job = terminal_job(&router, &state, &accepted).await;
    succeeded(&job);

    let compact_path = format!("/v1/indexes/{INDEX}/compact?wait=true");
    let (_, accepted) = send(&router, Method::POST, &compact_path, ADMIN_SECRET, None).await;
    let compact = terminal_job(&router, &state, &accepted).await;
    let report = succeeded(&compact);
    assert_eq!(report["from_epoch"], 1, "{report}");
    assert_eq!(report["to_epoch"], 2, "{report}");

    let exports_path = format!("/v1/indexes/{INDEX}/exports?wait=true");
    let (_, accepted) = send(&router, Method::POST, &exports_path, ADMIN_SECRET, None).await;
    let export = terminal_job(&router, &state, &accepted).await;
    let report = succeeded(&export);
    let directory = report["directory"]
        .as_str()
        .unwrap_or_else(|| panic!("the export reports its directory: {report}"));
    assert_eq!(
        directory,
        format!("outbox/{INDEX}-g{}", report["generation"]),
        "the export is named after the index and the generation: {report}"
    );
    assert!(
        instance
            .outbox()
            .join(format!("{INDEX}-g{}", report["generation"]))
            .is_dir(),
        "the export exists in the outbox: {report}"
    );

    let snapshots_path = format!("/v1/indexes/{INDEX}/snapshots?wait=true");
    let (_, accepted) = send(&router, Method::POST, &snapshots_path, ADMIN_SECRET, None).await;
    let snapshot = terminal_job(&router, &state, &accepted).await;
    let report = succeeded(&snapshot);
    assert_eq!(report["index"], INDEX, "{report}");
    assert_eq!(
        report["epoch"], 2,
        "the snapshot copies the compacted epoch: {report}"
    );
    assert!(
        report["bytes"].as_u64().is_some_and(|bytes| bytes > 0),
        "the snapshot reports its size: {report}"
    );

    for (name, job) in [
        ("compact", compact),
        ("export", export),
        ("snapshot", snapshot),
    ] {
        let result = job["result"].to_string();
        assert!(
            !result.contains("_ms"),
            "the result of {name} carries no duration (ADR-0042): {result}"
        );
        let root = instance.root().to_string_lossy().into_owned();
        assert!(
            !result.contains(&root),
            "the result of {name} carries no absolute path of the server: {result}"
        );
    }
}

/// The shape of an online body is refused before the job is queued: a mode other
/// than `upsert`, a `doc_id` both written and deleted, an unknown member
/// (ADR-0107, decision 3).
#[tokio::test]
async fn the_shape_of_an_online_body_is_refused_before_the_job() {
    let (_instance, _state, router) = instance_with_alpha().await;
    let replace_all = {
        let mut body = online_batch("batch-shape", "le contrat fait la loi", "1240");
        body["mode"] = json!("replace_all");
        body
    };
    let overlap = json!({
        "batch_id": "batch-shape",
        "documents": [{
            "doc_id": DOC_ID,
            "fields": {},
            "passages": [{
                "text": "le contrat fait la loi",
                "fields": {},
                "vector": "AAAAAAAAAAAAAAAAAAAAAA=="
            }]
        }],
        "deletions": [DOC_ID]
    });
    let unknown_member = {
        let mut body = online_batch("batch-shape", "le contrat fait la loi", "1240");
        body["replace"] = json!(true);
        body
    };
    let empty = json!({ "batch_id": "batch-shape", "mode": "upsert" });
    let empty_arrays = json!({
        "batch_id": "batch-shape",
        "mode": "upsert",
        "documents": [],
        "deletions": []
    });
    for (case, body) in [
        ("replace_all", replace_all),
        ("a document also deleted", overlap),
        ("an unknown member", unknown_member),
        ("no documents and no deletions", empty),
        ("empty documents and empty deletions", empty_arrays),
    ] {
        let (status, answer) = post_documents(&router, body).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{case} is refused by the route: {answer}"
        );
        assert_eq!(
            answer["error"]["code"], "invalid_request",
            "{case}: {answer}"
        );
        if case.contains("no documents") || case.contains("empty documents") {
            let message = answer["error"]["message"]
                .as_str()
                .unwrap_or_else(|| panic!("the refusal carries a message: {answer}"));
            assert!(
                message.contains("documents") && message.contains("deletions"),
                "{case}: the message names both members: {answer}"
            );
        }
    }

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
        jobs["jobs"].as_array().is_some_and(Vec::is_empty),
        "a refused shape queues no job: {jobs}"
    );
}

/// ADR-0107, decision 4: the attach journal line names the job, the kind, the
/// index and one duration per step, once, and never the content of the batch.
#[tokio::test]
async fn the_attach_journal_carries_the_durations_once() {
    let journal = common::journal();
    let (_instance, state, router) = instance_with_alpha().await;
    let (status, accepted) = post_documents(
        &router,
        online_batch("batch-journal", "le secret du client 4242", "998877"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    let job = terminal_job(&router, &state, &accepted).await;
    succeeded(&job);
    let id = job["id"]
        .as_str()
        .expect("the job carries its id")
        .to_owned();
    let text = journal.contents();
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| line.contains(id.as_str()) && line.contains("attach durations"))
        .collect();
    assert_eq!(
        lines.len(),
        1,
        "the attach job logs its durations exactly once: {text}"
    );
    let line = lines[0];
    let fields: Vec<String> = vec![
        "\"kind\":\"online_batch\"".to_owned(),
        format!("\"index\":\"{INDEX}\""),
        "\"validation_ms\":".to_owned(),
        "\"segment_ms\":".to_owned(),
        "\"transaction_ms\":".to_owned(),
        "\"lexical_ms\":".to_owned(),
        "\"manifest_ms\":".to_owned(),
        "\"snapshot_ms\":".to_owned(),
    ];
    for field in &fields {
        assert!(
            line.contains(field.as_str()),
            "the duration line carries '{field}': {line}"
        );
    }
    assert!(
        !line.contains("998877") && !line.contains("4242"),
        "the duration line carries no field value and no text (INV-13): {line}"
    );
}

/// `?text=true` adds the text of each passage; `GET` of an unknown document is
/// `404 not_found` (ADR-0107, decision 6).
#[tokio::test]
async fn the_read_route_renders_the_text_on_request_and_not_found_otherwise() {
    let (_instance, state, router) = instance_with_alpha().await;
    let (status, accepted) = post_documents(
        &router,
        online_batch("batch-read", "le contrat fait la loi", "1240"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    let job = terminal_job(&router, &state, &accepted).await;
    succeeded(&job);

    let path = format!("/v1/indexes/{INDEX}/documents/{DOC_ID}?text=true");
    let (status, document) = send(&router, Method::GET, &path, READ_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "the document is read: {document}");
    assert_eq!(
        document["passages"][0]["text"], "le contrat fait la loi",
        "?text=true adds the text of the passage: {document}"
    );

    let path = format!("/v1/indexes/{INDEX}/documents/UNKNOWN");
    let (status, answer) = send(&router, Method::GET, &path, READ_SECRET, None).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "an unknown document: {answer}"
    );
    assert_eq!(answer["error"]["code"], "not_found", "{answer}");
}

/// A body above the entry limit is `413 payload_too_large`, before any content
/// is validated (ADR-0107, decision 3). Passages and deletions count together,
/// so the test uses deletions only.
#[tokio::test]
async fn a_body_above_the_entry_limit_is_payload_too_large() {
    let (_instance, _state, router) = instance_with_alpha().await;
    let deletions: Vec<String> = (0..=10_000).map(|n| format!("DOC-{n}")).collect();
    let (status, answer) = post_documents(
        &router,
        json!({ "batch_id": "batch-too-large", "deletions": deletions }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "10 001 entries are above the limit: {answer}"
    );
    assert_eq!(answer["error"]["code"], "payload_too_large", "{answer}");
}
