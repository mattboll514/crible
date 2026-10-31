// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the checks of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! `POST /v1/indexes/{index}/batches` with a batch an operator deposited in the
//! inbox (`docs/phases/P6-serveur-et-api.md`, `ROUTES-DOCUMENTS-ET-LOTS`,
//! criterion "un lot déposé s'attache"; ADR-0107, decisions 4 and 5).
//!
//! The batch is a real one: the first instance exports its index into its
//! outbox, the test moves the directory into the inbox, and a second instance
//! sharing the same exchange attaches it into an empty index. Nothing is
//! fabricated by the test, so the route is proved on the format the export
//! writes (`docs/format-lot-v1.md`).
//!
//! ```text
//! cargo test -p crible-server --test deposited_batch
//! ```

mod common;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use common::{
    ADMIN_SECRET, DOC_ID, INDEX, Instance, WRITE_SECRET, export_index, index_config,
    instance_with_alpha, online_batch, post_documents, send, terminal_job,
};

/// Creates the fixture index on `router`.
async fn create_index(router: &axum::Router) {
    let (status, body) = send(
        router,
        Method::POST,
        "/v1/indexes",
        ADMIN_SECRET,
        Some(index_config()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "the index is created: {body}");
}

/// A batch exported by one instance, deposited in the inbox, attaches into the
/// index of another instance (ADR-0107, decision 4).
#[tokio::test]
async fn an_exported_batch_attaches_into_another_instance() {
    let journal = common::journal();
    let (first, first_state, first_router) = instance_with_alpha().await;
    let (status, accepted) = post_documents(
        &first_router,
        online_batch("batch-exported", "le contrat fait la loi", "1240"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    let job = terminal_job(&first_router, &first_state, &accepted).await;
    assert_eq!(
        job["state"], "succeeded",
        "the source batch attaches: {job}"
    );

    let exported = export_index(&first_router, &first_state).await;
    assert_eq!(
        exported,
        format!("outbox/{INDEX}-g{}", job["generation"]),
        "the export names its batch after the index and the generation"
    );

    // The operator moves the exported directory into the inbox, under a name of
    // its choice (`docs/05-api.md`, "Lot déposé").
    let deposited = "deposited-1";
    let source = first.exchange().join(&exported);
    let destination = first.inbox().join(deposited);
    std::fs::rename(&source, &destination)
        .unwrap_or_else(|error| panic!("{} is movable: {error}", source.display()));

    let second = Instance::sharing(first.exchange());
    let second_state = second.state();
    let second_router = crible_server::http::build_router(std::sync::Arc::clone(&second_state));
    create_index(&second_router).await;

    let path = format!("/v1/indexes/{INDEX}/batches?wait=true");
    let (status, accepted) = send(
        &second_router,
        Method::POST,
        &path,
        WRITE_SECRET,
        Some(json!({ "directory": deposited })),
    )
    .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::ACCEPTED,
        "the deposited batch is accepted: {accepted}"
    );
    assert_eq!(accepted["kind"], "attach_batch", "{accepted}");
    let job = terminal_job(&second_router, &second_state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the attach succeeds: {job}");
    let report: &Value = &job["result"]["report"];
    assert_eq!(report["added"], 1, "the document came along: {report}");

    // ADR-0107, decision 4: the attach of a deposited batch logs the job, the
    // kind, the index and one duration per step, once, and no batch content.
    let id = job["id"]
        .as_str()
        .expect("the job carries its id")
        .to_owned();
    let text = journal.contents();
    let line = text
        .lines()
        .find(|line| line.contains(id.as_str()) && line.contains("attach durations"))
        .unwrap_or_else(|| panic!("the attach job logs its durations: {text}"));
    for field in [
        "\"kind\":\"attach_batch\"",
        "\"validation_ms\":",
        "\"snapshot_ms\":",
    ] {
        assert!(
            line.contains(field),
            "the duration line carries '{field}': {line}"
        );
    }
    assert!(
        !line.contains("le contrat fait la loi") && !line.contains("1240"),
        "the duration line carries no batch content (INV-13): {line}"
    );

    let (status, document) = send(
        &second_router,
        Method::GET,
        &format!("/v1/indexes/{INDEX}/documents/{DOC_ID}"),
        WRITE_SECRET,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the attached document is read: {document}"
    );
    assert_eq!(document["doc_id"], DOC_ID);
    assert_eq!(
        document["fields"]["numero"], "1240",
        "the fields of the export are kept: {document}"
    );
    assert_eq!(
        document["passages"][0]["fields"]["texte"], "contrat",
        "the passage fields of the export are kept: {document}"
    );
}
