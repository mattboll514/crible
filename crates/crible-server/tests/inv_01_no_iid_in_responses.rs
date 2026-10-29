// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and several fixtures and checks of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! INV-01 — no internal id leaves the engine, through the write routes of
//! `ROUTES-DOCUMENTS-ET-LOTS` (`docs/phases/P6-serveur-et-api.md`).
//!
//! The routes under test are the ones of ADR-0107: `POST` and `PATCH` on
//! `/v1/indexes/{index}/documents`, `GET /v1/indexes/{index}/documents/{doc_id}`
//! with and without `?text=true`, `POST /v1/indexes/{index}/batches`,
//! `POST /compact`, `/exports` and `/snapshots`, and the job each of them
//! renders (`GET /v1/jobs/{job_id}`, ADR-0103).
//!
//! Every body rendered by those routes is walked twice:
//!
//! - no key at any depth contains `iid` — the check of
//!   `routes_indexes::the_list_is_sorted_by_name_and_the_detail_exposes_no_internal_id`
//!   and of `inv_01_exported_batch_names_no_iid` in `crible-store`;
//! - no number under a key that `docs/05-api.md` does not document as a counter
//!   equals a live iid. The fixture attaches two passages in a fresh index, so
//!   the only live iids are `0` and `1`.
//!
//! The journal captured by the server during those calls is scanned for the
//! same substring, the way `tests/auth.rs` scans it.
//!
//! | Test | What it proves |
//! |---|---|
//! | `inv_01_write_routes_render_no_iid_key_or_value` | INV-01 on every response and job body of the seven routes |
//! | `inv_01_write_routes_journal_carries_no_iid` | INV-01 on the journal of those calls |
//!
//! The instance serves an `external` model and the online batch carries its
//! vectors in base64: no ONNX Runtime is loaded.
//!
//! ```text
//! cargo test -p crible-server --test inv_01_no_iid_in_responses
//! ```

use std::sync::Arc;

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::state::AppState;
use serde_json::{Value, json};

mod common;

use common::{
    ADMIN_SECRET, DOC_ID, INDEX, Instance, READ_SECRET, REQUEST_ID, WRITE_SECRET, base64_vector,
    instance_with_alpha, journal, send, terminal_job,
};

/// Body of the online batch: two passages of `D1` with their vectors.
fn online_batch() -> Value {
    json!({
        "batch_id": "inv-01-online-1",
        "mode": "upsert",
        "documents": [
            {
                "doc_id": DOC_ID,
                "fields": { "numero": "1240" },
                "passages": [
                    {
                        "text": "le contrat fait la loi des parties",
                        "fields": { "texte": "contrat" },
                        "vector": base64_vector([0.6, 0.8, 0.0, 0.0])
                    },
                    {
                        "text": "la responsabilite du fait des choses",
                        "fields": { "texte": "responsabilite" },
                        "vector": base64_vector([0.0, 0.0, 0.6, 0.8])
                    }
                ]
            }
        ]
    })
}

/// Exercises every route of `ROUTES-DOCUMENTS-ET-LOTS` and returns the labelled
/// bodies it rendered.
async fn exercise(
    router: &Router,
    state: &Arc<AppState>,
    instance: &Instance,
) -> Vec<(String, Value)> {
    let mut seen = Vec::new();

    // 1. Online batch: `202` and an `online_batch` job (ADR-0107, decision 2).
    let path = format!("/v1/indexes/{INDEX}/documents?wait=true");
    let (status, accepted) = send(
        router,
        Method::POST,
        &path,
        WRITE_SECRET,
        Some(online_batch()),
    )
    .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::ACCEPTED,
        "the online batch is accepted: {accepted}"
    );
    assert_eq!(accepted["kind"], "online_batch", "{accepted}");
    seen.push(("online batch acceptance".to_owned(), accepted.clone()));
    let job = terminal_job(router, state, &accepted).await;
    assert_eq!(
        job["state"], "succeeded",
        "the online batch attaches with its provided vectors: {job}"
    );
    seen.push(("online batch job".to_owned(), job));

    // 2. `GET /documents/{doc_id}` without `?text=true`: the passages carry
    // their `ord` and their fields, never their text (ADR-0107, decision 6).
    let path = format!("/v1/indexes/{INDEX}/documents/{DOC_ID}");
    let (status, document) = send(router, Method::GET, &path, READ_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "the document is read: {document}");
    assert_eq!(document["doc_id"], DOC_ID);
    assert!(
        document["generation"].as_u64().is_some(),
        "the document carries the generation of the snapshot: {document}"
    );
    assert!(
        document["doc_hash"]
            .as_str()
            .is_some_and(|hash| hash.starts_with("blake3:")),
        "the document carries its digest: {document}"
    );
    assert_eq!(document["fields"]["numero"], "1240");
    let passages = document["passages"]
        .as_array()
        .unwrap_or_else(|| panic!("the document carries its passages: {document}"));
    assert_eq!(passages.len(), 2, "{document}");
    assert_eq!(passages[0]["ord"], 0);
    assert_eq!(passages[1]["ord"], 1);
    assert_eq!(passages[0]["fields"]["texte"], "contrat");
    assert!(
        passages[0].get("text").is_none(),
        "no text without '?text=true': {document}"
    );
    seen.push(("document without text".to_owned(), document));

    // 3. The same document with `?text=true`.
    let path = format!("/v1/indexes/{INDEX}/documents/{DOC_ID}?text=true");
    let (status, document) = send(router, Method::GET, &path, READ_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "the document is read: {document}");
    let passages = document["passages"]
        .as_array()
        .unwrap_or_else(|| panic!("the document carries its passages: {document}"));
    assert_eq!(
        passages[0]["text"], "le contrat fait la loi des parties",
        "the text is rendered with '?text=true': {document}"
    );
    assert_eq!(
        passages[1]["text"], "la responsabilite du fait des choses",
        "{document}"
    );
    seen.push(("document with text".to_owned(), document));

    // 4. Metadata patch: a `patch_documents` job (ADR-0060, ADR-0107).
    let path = format!("/v1/indexes/{INDEX}/documents?wait=true");
    let (status, accepted) = send(
        router,
        Method::PATCH,
        &path,
        WRITE_SECRET,
        Some(json!({ "documents": [ { "doc_id": DOC_ID, "fields": { "numero": "1250" } } ] })),
    )
    .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::ACCEPTED,
        "the patch is accepted: {accepted}"
    );
    assert_eq!(accepted["kind"], "patch_documents", "{accepted}");
    seen.push(("patch acceptance".to_owned(), accepted.clone()));
    let job = terminal_job(router, state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the patch applies: {job}");
    seen.push(("patch job".to_owned(), job));

    let path = format!("/v1/indexes/{INDEX}/documents/{DOC_ID}");
    let (status, document) = send(router, Method::GET, &path, READ_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "{document}");
    assert_eq!(
        document["fields"]["numero"], "1250",
        "the patch replaced the document field: {document}"
    );
    seen.push(("document after patch".to_owned(), document));

    // 5. Compaction, export and snapshot: `202` with one job each, no body
    // (ADR-0107, decision 7).
    for (suffix, kind) in [
        ("compact", "compact"),
        ("exports", "export"),
        ("snapshots", "snapshot"),
    ] {
        let path = format!("/v1/indexes/{INDEX}/{suffix}?wait=true");
        let (status, accepted) = send(router, Method::POST, &path, ADMIN_SECRET, None).await;
        assert!(
            status == StatusCode::OK || status == StatusCode::ACCEPTED,
            "the {kind} route is accepted: {accepted}"
        );
        assert_eq!(accepted["kind"], kind, "{accepted}");
        seen.push((format!("{kind} acceptance"), accepted.clone()));
        let job = terminal_job(router, state, &accepted).await;
        assert_eq!(job["state"], "succeeded", "the {kind} job runs: {job}");
        seen.push((format!("{kind} job"), job));
    }

    // 6. `POST /batches`. No Parquet fixture is reachable from this crate
    // (`crates/crible-store/tests/common/mod.rs` is a test module of another
    // package), so the deposited-batch route is exercised through its absent
    // directory: the `404 not_found` body is a body of this route and carries no
    // iid either. The attachment itself is proved by the store tests.
    let path = format!("/v1/indexes/{INDEX}/batches");
    let (status, refused) = send(
        router,
        Method::POST,
        &path,
        WRITE_SECRET,
        Some(json!({ "directory": "absent-batch" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{refused}");
    assert_eq!(refused["error"]["code"], "not_found", "{refused}");
    seen.push(("deposited batch refusal".to_owned(), refused));

    // 7. The job list: the same bodies, read back (ADR-0103).
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
        jobs["jobs"].as_array().is_some_and(|jobs| jobs.len() >= 5),
        "the five mutations of this test are listed: {jobs}"
    );
    seen.push(("job list".to_owned(), jobs));

    assert!(
        instance.root().join("data").is_dir(),
        "the instance directory stays in place"
    );
    seen
}

/// Panics when an object key at any depth of `value` contains `iid`, whatever
/// its case.
fn assert_no_iid_key(label: &str, value: &Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                assert!(
                    !key.to_ascii_lowercase().contains("iid"),
                    "the key '{key}' of {label} names an internal id (INV-01): {value}"
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

/// True when `docs/05-api.md` documents `key` as a counter, an `ord`, a
/// generation or a size: the numbers those keys carry are not internal ids.
fn is_documented_numeric(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    if key == "k" {
        // `k` is the size of a search response (`docs/05-api.md`, "Recherche").
        return true;
    }
    [
        "generation",
        "ord",
        "passage",
        "document",
        "segment",
        "epoch",
        "count",
        "added",
        "replaced",
        "unchanged",
        "updated",
        "deleted",
        "ignored",
        "sample",
        "progress",
        "done",
        "total",
        "size",
        "byte",
        "file",
        "part",
        "row",
        "duration",
        "took",
        "version",
        "ratio",
        "score",
        "cosine",
        "median",
        "truncated",
        "pool",
        "depth",
        "attempt",
        "retry",
        "index",
        "limit",
        "quota",
        "_ms",
    ]
    .iter()
    .any(|word| key.contains(word))
}

/// Panics when a number of `value` equals a live iid under a key that the API
/// does not document as a counter.
fn assert_no_iid_value(label: &str, value: &Value, live_iids: &[u64]) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if let Some(number) = child.as_u64() {
                    assert!(
                        is_documented_numeric(key) || !live_iids.contains(&number),
                        "the value of '{key}' in {label} equals the internal id {number} (INV-01)"
                    );
                }
                assert_no_iid_value(label, child, live_iids);
            }
        }
        Value::Array(items) => {
            for item in items {
                assert_no_iid_value(label, item, live_iids);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

/// Runs `exercise` on a `current_thread` runtime.
fn run_exercise() -> (Instance, Vec<(String, Value)>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime is startable");
    runtime.block_on(async {
        let (instance, state, router) = instance_with_alpha().await;
        let seen = exercise(&router, &state, &instance).await;
        (instance, seen)
    })
}

#[test]
fn inv_01_write_routes_render_no_iid_key_or_value() {
    // INV-01: every body of the seven routes, and the body of each job they
    // render, carries no key and no value that is an internal id.
    let (_instance, seen) = run_exercise();
    assert!(!seen.is_empty(), "the exercise rendered bodies");
    // The fixture attaches two passages in a fresh index, whose iids are 0
    // and 1 (dense `u32`, `docs/00-glossaire.md`).
    let live_iids = [0, 1];
    for (label, body) in &seen {
        assert_no_iid_key(label, body);
        assert_no_iid_value(label, body, &live_iids);
    }
}

#[test]
fn inv_01_write_routes_journal_carries_no_iid() {
    // INV-01: the journal of the same calls carries no internal id either; the
    // capture itself is proved by the request identifier it pins (ADR-0098).
    let sink = journal();
    let (_instance, seen) = run_exercise();
    assert!(!seen.is_empty(), "the exercise rendered bodies");
    let text = sink.contents();
    assert!(
        text.contains(REQUEST_ID),
        "the test must capture the journal of its requests: {text:?}"
    );
    assert!(
        !text.to_ascii_lowercase().contains("iid"),
        "the journal names an internal id (INV-01): {text}"
    );
}
