// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and every helper of this file is a fixture or an assertion.
#![allow(clippy::expect_used, clippy::panic)]

//! `explain_ids` on both search routes (`EXPLIQUER-DOCUMENTS`, phase P7;
//! ADR-0123).
//!
//! | Test | Acceptance criterion |
//! |---|---|
//! | `a_document_and_a_passage_key_are_explained_on_the_simple_route` | decisions 1 and 5: shape, ranks, omission of a missing member |
//! | `an_id_out_of_the_filter_names_its_leaf_and_leaves_the_branches_empty` | decision 7: `excluded_by` by its JSON path |
//! | `explain_ids_changes_neither_the_results_nor_the_rest_of_the_body` | decision 8: no effect on `results` |
//! | `the_multi_route_renders_one_flat_list_in_the_request_order` | decision 2: `{ "index", "id" }`, flat list |
//! | `an_explain_id_outside_the_targets_is_refused_before_any_search` | decision 2: `$.explain_ids[i].index` |
//! | `explain_ids_is_bounded_deduplicated_and_typed_on_both_routes` | decision 3: bound, duplicates, malformed id |
//! | `the_explanations_of_both_routes_carry_no_internal_id` | decision 9: no `iid`, `inv_01` extended to `explain` |
//!
//! The instance serves an `external` model of four dimensions, so no ONNX
//! Runtime is loaded; the requests carry their vectors in base64. The fixture
//! index scans `none` with `exact_threshold: 10`, so a pool of two passages
//! always takes the exact strategy and `binary_rank` is absent (decision 5).
//!
//! ```text
//! cargo test -p crible-server --test routes_explain
//! ```

mod common;

use std::sync::Arc;

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::state::AppState;
use serde_json::{Value, json};

use common::{
    ADMIN_SECRET, DOC_ID, INDEX, READ_SECRET, assert_no_iid_key, base64_vector, index_config,
    instance_with_alpha, post_documents, send, terminal_job,
};

/// Path of the multi-index route.
const MULTI: &str = "/v1/search";

/// Second index of the multi-index fixture.
const BETA: &str = "beta";

/// Document of `beta`, the second target of the multi-index test.
const BETA_DOC: &str = "DOC-B";

/// Request vector of every search: norm one, so a score is a cosine.
const VECTOR: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

/// Posts `body` on the search route of `index` with `secret`.
async fn search_in(router: &Router, index: &str, secret: &str, body: Value) -> (StatusCode, Value) {
    let path = format!("/v1/indexes/{index}/search");
    send(router, Method::POST, &path, secret, Some(body)).await
}

/// Posts `body` on the search route of the fixture index.
async fn search(router: &Router, body: Value) -> (StatusCode, Value) {
    search_in(router, INDEX, READ_SECRET, body).await
}

/// A conforming dense search of the fixture index.
fn dense_body() -> Value {
    json!({
        "vector": base64_vector(VECTOR),
        "mode": "dense",
        "k": 10,
    })
}

/// Asserts a refusal and returns its message.
fn refused(status: StatusCode, body: &Value, needle: &str) {
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "the request is refused: {body}"
    );
    assert_eq!(body["error"]["code"], "invalid_request", "{body}");
    let message = body["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the error carries a message: {body}"));
    assert!(
        message.contains(needle),
        "the message must contain {needle:?}: {message}"
    );
}

/// Writes one document of `index` and waits for its attach.
async fn attach_one(router: &Router, state: &Arc<AppState>, index: &str, body: Value) {
    let path = format!("/v1/indexes/{index}/documents?wait=true");
    let (status, accepted) = send(
        router,
        Method::POST,
        &path,
        common::WRITE_SECRET,
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the batch is accepted: {accepted}");
    let job = terminal_job(router, state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the batch succeeds: {job}");
}

/// Fills the fixture index with two documents of one passage each.
async fn fill(router: &Router, state: &Arc<AppState>) {
    for (batch_id, doc_id, text, numero, vector) in [
        (
            "batch-a",
            DOC_ID,
            "le contrat fait la loi",
            "1240",
            [1.0, 0.0, 0.0, 0.0],
        ),
        (
            "batch-b",
            "DOC-2",
            "le delit est puni",
            "1241",
            [0.0, 1.0, 0.0, 0.0],
        ),
    ] {
        let body = json!({
            "batch_id": batch_id,
            "mode": "upsert",
            "documents": [{
                "doc_id": doc_id,
                "fields": { "numero": numero },
                "passages": [{
                    "text": text,
                    "fields": { "texte": "contrat" },
                    "vector": base64_vector(vector),
                }]
            }]
        });
        let (status, accepted) = post_documents(router, body).await;
        assert_eq!(status, StatusCode::OK, "the batch is accepted: {accepted}");
        let job = terminal_job(router, state, &accepted).await;
        assert_eq!(job["state"], "succeeded", "the batch succeeds: {job}");
    }
}

/// Creates the index `BETA` of the multi-index fixture and fills it with one
/// document.
async fn create_beta(router: &Router, state: &Arc<AppState>) {
    let mut config = index_config();
    config["name"] = json!(BETA);
    let (status, created) = send(
        router,
        Method::POST,
        "/v1/indexes",
        ADMIN_SECRET,
        Some(config),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "beta is created: {created}");
    attach_one(
        router,
        state,
        BETA,
        json!({
            "batch_id": "batch-b",
            "mode": "upsert",
            "documents": [{
                "doc_id": BETA_DOC,
                "fields": { "numero": "2000" },
                "passages": [{
                    "text": "la loi du contrat",
                    "fields": { "texte": "contrat" },
                    "vector": base64_vector([0.6, 0.8, 0.0, 0.0]),
                }]
            }]
        }),
    )
    .await;
}

/// Returns the entry of `explain` at `position`.
fn entry(body: &Value, position: usize) -> &Value {
    assert!(
        body["explain"]
            .as_array()
            .is_some_and(|items| items.len() > position),
        "the answer carries the entry {position}: {body}"
    );
    &body["explain"][position]
}

/// The simple route explains a document and a passage key with the shape of
/// ADR-0123, decision 1; a member without a value is omitted.
#[tokio::test]
async fn a_document_and_a_passage_key_are_explained_on_the_simple_route() {
    let (_instance, state, router) = instance_with_alpha().await;
    fill(&router, &state).await;
    let mut body = dense_body();
    body["explain_ids"] = json!([DOC_ID, format!("{DOC_ID}#0"), "never"]);
    let (status, answer) = search(&router, body).await;
    assert_eq!(status, StatusCode::OK, "the search answers: {answer}");

    let explain = answer["explain"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer carries an explain list: {answer}"));
    assert_eq!(explain.len(), 3, "one entry per id, in order: {answer}");

    let document = &explain[0];
    assert_eq!(document["id"], DOC_ID, "{answer}");
    assert_eq!(document["doc_id"], DOC_ID, "{answer}");
    assert_eq!(document["exists"], true, "{answer}");
    assert_eq!(document["alive"], true, "{answer}");
    assert_eq!(document["in_pool"], true, "{answer}");
    assert!(
        document.get("excluded_by").is_none(),
        "an id in the pool: {answer}"
    );
    let dense = &document["dense"];
    assert_eq!(dense["key"], format!("{DOC_ID}#0"), "{answer}");
    assert_eq!(dense["exact_rank"], 1, "the vector of the query: {answer}");
    assert!(
        dense.get("binary_rank").is_none() && dense.get("in_binary_candidates").is_none(),
        "the exact strategy has no binary selection (ADR-0123, decision 5): {answer}"
    );
    assert_eq!(document["fused"]["rank"], 1, "{answer}");
    assert!(
        document.get("grouped_under").is_none(),
        "a document is not absorbed by a regrouping: {answer}"
    );

    let passage = &explain[1];
    assert_eq!(passage["id"], format!("{DOC_ID}#0"), "{answer}");
    assert_eq!(passage["dense"]["key"], format!("{DOC_ID}#0"), "{answer}");
    assert!(
        passage.get("grouped_under").is_none(),
        "the result itself is rendered: {answer}"
    );

    let unknown = &explain[2];
    assert_eq!(unknown["id"], "never", "{answer}");
    assert_eq!(
        unknown["exists"], false,
        "an unknown id is not an error: {answer}"
    );
    assert!(
        unknown.get("alive").is_none(),
        "`alive` needs `exists`: {answer}"
    );
    assert_eq!(unknown["in_pool"], false, "{answer}");
    assert!(
        unknown.get("dense").is_none()
            && unknown.get("lexical").is_none()
            && unknown.get("fused").is_none(),
        "a document out of the state has no branch: {answer}"
    );
}

/// A document the filter excludes names each failing leaf by its full JSON
/// path and carries no branch (ADR-0123, decision 7).
#[tokio::test]
async fn an_id_out_of_the_filter_names_its_leaf_and_leaves_the_branches_empty() {
    let (_instance, state, router) = instance_with_alpha().await;
    fill(&router, &state).await;
    let (status, answer) = search(
        &router,
        json!({
            "vector": base64_vector(VECTOR),
            "mode": "dense",
            "filter": { "numero": { "eq": "9999" } },
            "explain_ids": [DOC_ID],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the search answers: {answer}");
    let document = entry(&answer, 0);
    assert_eq!(document["in_pool"], false, "{answer}");
    assert_eq!(
        document["excluded_by"],
        json!(["$.filter.numero"]),
        "the leaf is named by its JSON path: {answer}"
    );
    assert!(
        document.get("dense").is_none() && document.get("fused").is_none(),
        "an id out of the pool has no branch: {answer}"
    );
}

/// `explain_ids` changes neither the results nor the rest of the answer, and
/// its absence keeps the `explain` member away (ADR-0123, decision 8).
#[tokio::test]
async fn explain_ids_changes_neither_the_results_nor_the_rest_of_the_body() {
    let (_instance, state, router) = instance_with_alpha().await;
    fill(&router, &state).await;
    let (status, plain) = search(&router, dense_body()).await;
    assert_eq!(status, StatusCode::OK, "{plain}");
    let mut explained = dense_body();
    explained["explain_ids"] = json!([DOC_ID, "DOC-2"]);
    let (status, with_explain) = search(&router, explained).await;
    assert_eq!(status, StatusCode::OK, "{with_explain}");

    assert!(plain.get("explain").is_none(), "no `explain_ids`: {plain}");
    assert!(with_explain.get("explain").is_some(), "{with_explain}");
    assert_eq!(
        with_explain["results"], plain["results"],
        "the results are unchanged"
    );
    for member in ["generation", "pool_size", "truncated_query"] {
        assert_eq!(with_explain[member], plain[member], "{member} is unchanged");
    }
}

/// The multi-index route renders one flat list, in the order of the request,
/// each entry naming its index (ADR-0123, decision 2).
#[tokio::test]
async fn the_multi_route_renders_one_flat_list_in_the_request_order() {
    let (_instance, state, router) = instance_with_alpha().await;
    fill(&router, &state).await;
    create_beta(&router, &state).await;
    let (status, answer) = send(
        &router,
        Method::POST,
        MULTI,
        READ_SECRET,
        Some(json!({
            "vector": base64_vector(VECTOR),
            "mode": "dense",
            "k": 10,
            "targets": [{ "index": INDEX }, { "index": BETA }],
            "explain_ids": [
                { "index": INDEX, "id": DOC_ID },
                { "index": BETA, "id": BETA_DOC },
                { "index": INDEX, "id": format!("{DOC_ID}#0") },
            ],
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the multi-index search answers: {answer}"
    );
    let explain = answer["explain"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer carries an explain list: {answer}"));
    assert_eq!(explain.len(), 3, "one entry per id, in order: {answer}");
    let indices: Vec<&str> = explain
        .iter()
        .map(|item| item["index"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(indices, vec![INDEX, BETA, INDEX], "{answer}");
    assert_eq!(explain[0]["id"], DOC_ID, "{answer}");
    assert_eq!(explain[1]["id"], BETA_DOC, "{answer}");
    assert_eq!(explain[1]["doc_id"], BETA_DOC, "{answer}");
    assert_eq!(explain[2]["id"], format!("{DOC_ID}#0"), "{answer}");
    for item in explain {
        assert_eq!(
            item["in_pool"], true,
            "each id is in its own pool: {answer}"
        );
    }
}

/// An entry that names an index outside `targets` is refused with its JSON
/// path, before any index is read (ADR-0123, decision 2).
#[tokio::test]
async fn an_explain_id_outside_the_targets_is_refused_before_any_search() {
    let (_instance, _state, router) = instance_with_alpha().await;
    let (status, answer) = send(
        &router,
        Method::POST,
        MULTI,
        READ_SECRET,
        Some(json!({
            "targets": [{ "index": INDEX }],
            "explain_ids": [{ "index": "gamma", "id": "A" }],
        })),
    )
    .await;
    refused(status, &answer, "$.explain_ids[0].index");
}

/// The bound of 100, the merge of duplicates and the typing of an id are the
/// ones of ADR-0123, decision 3, on both routes.
#[tokio::test]
async fn explain_ids_is_bounded_deduplicated_and_typed_on_both_routes() {
    let (_instance, _state, router) = instance_with_alpha().await;
    // A duplicate is merged keeping the first occurrence.
    let (status, answer) = search(
        &router,
        json!({
            "vector": base64_vector(VECTOR),
            "explain_ids": [DOC_ID, DOC_ID, format!("{DOC_ID}#0")],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(
        answer["explain"].as_array().map(Vec::len),
        Some(2),
        "the duplicate is merged: {answer}"
    );

    // An id is a key when it ends with `#` and a decimal `ord`; every other id
    // is a `doc_id`, `#` included (ADR-0123, complement of 2026-11-01). An empty
    // id, a key whose document part is empty, and an entry which is not a string
    // are refused.
    for body in [
        json!({ "explain_ids": [""] }),
        json!({ "explain_ids": ["#3"] }),
        json!({ "explain_ids": [1] }),
    ] {
        let (status, answer) = search(&router, body).await;
        refused(status, &answer, "$.explain_ids[0]");
    }
    let (status, answer) = search(
        &router,
        json!({ "vector": base64_vector(VECTOR), "explain_ids": ["A#x"] }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "A#x is a doc_id, not a malformed key: {answer}"
    );
    assert_eq!(
        answer["explain"][0]["id"], "A#x",
        "the id is rendered as requested: {answer}"
    );
    assert_eq!(
        answer["explain"][0]["exists"], false,
        "an unknown doc_id is not an error: {answer}"
    );

    // Above the bound, after the merge of the duplicates.
    let too_many: Vec<String> = (0..=100).map(|position| format!("D{position}")).collect();
    let (status, answer) = search(&router, json!({ "explain_ids": too_many })).await;
    refused(status, &answer, "$.explain_ids");

    // The multi-index form is an object `{ index, id }`, checked with the body.
    let (status, answer) = send(
        &router,
        Method::POST,
        MULTI,
        READ_SECRET,
        Some(json!({
            "targets": [{ "index": INDEX }],
            "explain_ids": ["A"],
        })),
    )
    .await;
    refused(status, &answer, "$.explain_ids[0]");
    let (status, answer) = send(
        &router,
        Method::POST,
        MULTI,
        READ_SECRET,
        Some(json!({
            "targets": [{ "index": INDEX }],
            "explain_ids": [{ "index": INDEX }],
        })),
    )
    .await;
    refused(status, &answer, "$.explain_ids[0].id");
}

/// INV-01 extended to `explain` (ADR-0123, decision 9): no key at any depth of
/// the explanation carries an internal identifier, on either route.
#[tokio::test]
async fn the_explanations_of_both_routes_carry_no_internal_id() {
    let (_instance, state, router) = instance_with_alpha().await;
    fill(&router, &state).await;
    let mut body = dense_body();
    body["explain_ids"] = json!([DOC_ID, format!("{DOC_ID}#0"), "never"]);
    let (status, answer) = search(&router, body).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert!(answer.get("explain").is_some(), "{answer}");
    assert_no_iid_key("simple explain", &answer);

    let (status, answer) = send(
        &router,
        Method::POST,
        MULTI,
        READ_SECRET,
        Some(json!({
            "vector": base64_vector(VECTOR),
            "mode": "dense",
            "targets": [{ "index": INDEX }],
            "explain_ids": [
                { "index": INDEX, "id": DOC_ID },
                { "index": INDEX, "id": format!("{DOC_ID}#0") },
            ],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert!(answer.get("explain").is_some(), "{answer}");
    assert_no_iid_key("multi-index explain", &answer);
}
