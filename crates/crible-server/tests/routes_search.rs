// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and every helper of this file is a fixture or an assertion.
#![allow(clippy::expect_used, clippy::panic)]

//! `POST /v1/indexes/{index}/search` (`ROUTE-RECHERCHE`, phase P6).
//!
//! | Test | Acceptance criterion |
//! |---|---|
//! | `the_three_roles_reach_the_search_route` | no `403` is reachable (ADR-0095) |
//! | `a_conforming_answer_carries_generation_pool_size_and_truncated_query` | answer of `docs/05` |
//! | `an_unknown_index_is_a_404_unknown_index` | `404 unknown_index` |
//! | `an_unknown_field_names_its_json_path_at_every_level` | ADR-0109, decision 1 |
//! | `debug_and_explain_are_refused_in_p6` | ADR-0109, decision 1 |
//! | `timeout_ms_is_read_from_the_body_and_bounded` | ADR-0102, decision 3 |
//! | `a_query_on_an_external_model_is_refused_naming_the_model` | `docs/05:235` |
//! | `a_vector_of_the_wrong_length_is_refused` | `docs/05:235` |
//! | `a_lexical_mode_on_a_disabled_index_is_refused` | ADR-0089, decision 3 |
//! | `the_return_selection_bounds_the_payload` | `docs/05:243`, ADR-0043 |
//! | `a_return_field_declared_not_returned_is_refused` | ADR-0109, decision 5 |
//! | `a_filter_outside_its_grammar_is_an_invalid_filter_with_its_path` | `details.path`, ADR-0110, decision 2 |
//! | `the_regrouping_options_are_bounded_on_the_route` | `docs/05:266`, ADR-0075 |
//! | `a_hybrid_search_on_an_external_model_is_served_with_query_and_vector` | ADR-0109, decision 4 |
//! | `a_hybrid_search_on_an_external_model_needs_a_vector` | ADR-0109, decision 4 |
//! | `k_is_bounded_by_the_documented_domain` | `docs/05:242` |
//! | `the_fusion_domain_is_checked` | ADR-0090 |
//!
//! ```text
//! cargo test -p crible-server --test routes_search
//! ```

mod common;

use std::sync::Arc;

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::http::build_router;
use crible_server::state::AppState;
use serde_json::{Value, json};

use common::{
    ADMIN_SECRET, DOC_ID, INDEX, Instance, READ_SECRET, WRITE_SECRET, base64_vector, index_config,
    instance_with_alpha, post_documents, send, terminal_job,
};

/// Path of the search route of the fixture index.
const SEARCH: &str = "/v1/indexes/alpha/search";

/// Posts `body` on the search route of `index` with `secret`.
async fn search_in(router: &Router, index: &str, secret: &str, body: Value) -> (StatusCode, Value) {
    let path = format!("/v1/indexes/{index}/search");
    send(router, Method::POST, &path, secret, Some(body)).await
}

/// Posts `body` on the search route of the fixture index with `secret`.
async fn search(router: &Router, secret: &str, body: Value) -> (StatusCode, Value) {
    search_in(router, INDEX, secret, body).await
}

/// Creates an instance whose only index is described by `config`.
///
/// The tests of ADR-0109 need an index the shared fixture does not serve: one
/// with the lexical branch enabled (`hybrid` on an `external` model), or one
/// whose schema declares a `returned: false` field.
async fn instance_with_index(config: Value) -> (Instance, Arc<AppState>, Router) {
    let instance = Instance::new();
    let state = instance.state();
    let router = build_router(Arc::clone(&state));
    let (status, answer) = send(
        &router,
        Method::POST,
        "/v1/indexes",
        ADMIN_SECRET,
        Some(config),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{answer}");
    (instance, state, router)
}

/// Configuration of an index named `name`, with the lexical branch on or off.
fn config(name: &str, lexical: bool) -> Value {
    let mut config = index_config();
    config["name"] = json!(name);
    config["lexical"]["enabled"] = json!(lexical);
    config
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

/// Writes two documents of the fixture index and waits for their attach.
async fn fill(router: &Router, state: &std::sync::Arc<crible_server::state::AppState>) {
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

/// A conforming dense search for the first document of the fixture.
fn dense_body() -> Value {
    json!({
        "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
        "mode": "dense",
        "k": 10,
    })
}

/// The three roles reach the route: `read` is the mount role, and `write` and
/// `admin` include it (ADR-0095, ADR-0109).
#[tokio::test]
async fn the_three_roles_reach_the_search_route() {
    let (_instance, state, router) = instance_with_alpha().await;
    fill(&router, &state).await;
    for secret in [READ_SECRET, WRITE_SECRET, ADMIN_SECRET] {
        let (status, body) = search(&router, secret, dense_body()).await;
        assert_eq!(status, StatusCode::OK, "the role reaches the route: {body}");
        assert_eq!(body["request_id"], common::REQUEST_ID, "{body}");
    }
}

/// The response carries `generation`, `pool_size`, `took_ms` and
/// `truncated_query` (`docs/05-api.md`, "Recherche : réponse"), and the results
/// follow the pipeline order.
#[tokio::test]
async fn a_conforming_answer_carries_generation_pool_size_and_truncated_query() {
    let (_instance, state, router) = instance_with_alpha().await;
    fill(&router, &state).await;
    let (status, body) = search(&router, READ_SECRET, dense_body()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, index) = send(&router, Method::GET, "/v1/indexes/alpha", READ_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "{index}");
    assert_eq!(body["generation"], index["state"]["generation"], "{body}");
    assert_eq!(body["pool_size"], 2, "{body}");
    assert_eq!(body["truncated_query"], false, "{body}");
    assert!(body["took_ms"].is_number(), "{body}");
    let results = body["results"].as_array().expect("results is an array");
    assert_eq!(results.len(), 2, "both passages are in the pool: {body}");
    assert_eq!(results[0]["key"], format!("{DOC_ID}#0"), "{body}");
    assert_eq!(results[0]["doc_id"], DOC_ID, "{body}");
    assert_eq!(results[0]["passage"], 0, "{body}");
    assert_eq!(results[0]["score_type"], "cosine", "{body}");
    assert_eq!(results[0]["fields"]["numero"], "1240", "{body}");
    assert_eq!(results[0]["fields"]["texte"], "contrat", "{body}");
    assert_eq!(results[0]["text"], "le contrat fait la loi", "{body}");
    assert!(
        results[1]["score"].as_f64().expect("a score")
            <= results[0]["score"].as_f64().expect("a score"),
        "the results are best first: {body}"
    );
}

/// An index the instance does not serve is `404 unknown_index`.
#[tokio::test]
async fn an_unknown_index_is_a_404_unknown_index() {
    let (_instance, _state, router) = instance_with_alpha().await;
    let (status, body) = send(
        &router,
        Method::POST,
        "/v1/indexes/unknown/search",
        READ_SECRET,
        Some(dense_body()),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["code"], "unknown_index", "{body}");
}

/// A field outside the fourteen of `docs/05-api.md` is refused at every level
/// and the message names its JSON path (ADR-0109, decision 1).
#[tokio::test]
async fn an_unknown_field_names_its_json_path_at_every_level() {
    let (_instance, _state, router) = instance_with_alpha().await;
    for (body, needle) in [
        (json!({ "filtre": {} }), "$.filtre"),
        (json!({ "return": { "field": "all" } }), "$.return.field"),
        (
            json!({ "fusion": { "method": "rrf", "alpah": 0.3 } }),
            "$.fusion.alpah",
        ),
    ] {
        let (status, answer) = search(&router, READ_SECRET, body).await;
        refused(status, &answer, needle);
    }
}

/// `debug` and `explain_ids` are not silently ignored in P6: they are refused
/// with a message that says they open in P7 (ADR-0109, decision 1).
#[tokio::test]
async fn debug_and_explain_are_refused_in_p6() {
    let (_instance, _state, router) = instance_with_alpha().await;
    for body in [json!({ "debug": true }), json!({ "explain_ids": [] })] {
        let (status, answer) = search(&router, READ_SECRET, body).await;
        refused(status, &answer, "not supported yet");
    }
}

/// `timeout_ms` is read from the body, in the documented domain; the default is
/// `[limits].default_timeout_ms` and applies when the field is absent
/// (ADR-0102, decision 3).
#[tokio::test]
async fn timeout_ms_is_read_from_the_body_and_bounded() {
    let (_instance, _state, router) = instance_with_alpha().await;
    for body in [
        json!({ "timeout_ms": 0 }),
        json!({ "timeout_ms": 30_001 }),
        json!({ "timeout_ms": -1 }),
        json!({ "timeout_ms": "10" }),
    ] {
        let (status, answer) = search(&router, READ_SECRET, body).await;
        refused(status, &answer, "timeout_ms");
    }
    // The upper bound is included and the default of the fixture is 2 000 ms.
    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({ "vector": base64_vector([1.0, 0.0, 0.0, 0.0]), "timeout_ms": 30_000 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
}

/// A text query on an `external` index names the model: it has no encoder, so
/// the client must provide a `vector` (`docs/04`, `docs/05:235`).
#[tokio::test]
async fn a_query_on_an_external_model_is_refused_naming_the_model() {
    let (_instance, _state, router) = instance_with_alpha().await;
    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({ "query": "le contrat", "mode": "dense" }),
    )
    .await;
    refused(status, &answer, "external");
    refused(status, &answer, "'m'");
}

/// A vector of the wrong length, or one that is not base64, is refused before
/// any search runs (`docs/05:235`).
#[tokio::test]
async fn a_vector_of_the_wrong_length_is_refused() {
    let (_instance, _state, router) = instance_with_alpha().await;
    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({ "vector": base64_vector([1.0, 0.0, 0.0]), "mode": "dense" }),
    )
    .await;
    refused(status, &answer, "components");
    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({ "vector": "not base64", "mode": "dense" }),
    )
    .await;
    refused(status, &answer, "base64");
}

/// `lexical` and `hybrid` are refused on an index whose `lexical.enabled` is
/// false, before any work (ADR-0089, decision 3); `dense` keeps serving it.
#[tokio::test]
async fn a_lexical_mode_on_a_disabled_index_is_refused() {
    let (_instance, _state, router) = instance_with_alpha().await;
    for mode in ["lexical", "hybrid"] {
        let (status, answer) = search(
            &router,
            READ_SECRET,
            json!({ "query": "contrat", "mode": mode }),
        )
        .await;
        refused(status, &answer, "lexical branch is disabled");
    }
}

/// `return.fields` selects the payload and `return.text` removes the text;
/// an empty selection renders no `fields` member (`docs/05:243`, ADR-0043).
#[tokio::test]
async fn the_return_selection_bounds_the_payload() {
    let (_instance, state, router) = instance_with_alpha().await;
    fill(&router, &state).await;
    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({
            "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
            "mode": "dense",
            "return": { "fields": ["numero"], "text": false },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let first = &answer["results"][0];
    assert_eq!(first["fields"], json!({ "numero": "1240" }), "{answer}");
    assert!(first.get("text").is_none(), "{answer}");

    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({
            "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
            "mode": "dense",
            "return": { "fields": [] },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert!(answer["results"][0].get("fields").is_none(), "{answer}");

    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({
            "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
            "mode": "dense",
            "return": { "fields": ["unknown"] },
        }),
    )
    .await;
    refused(status, &answer, "$.return.fields[0]");
}

/// `k` is bounded by `docs/05:242`.
#[tokio::test]
async fn k_is_bounded_by_the_documented_domain() {
    let (_instance, _state, router) = instance_with_alpha().await;
    for k in [0, 1_001] {
        let (status, answer) = search(
            &router,
            READ_SECRET,
            json!({ "vector": base64_vector([1.0, 0.0, 0.0, 0.0]), "k": k }),
        )
        .await;
        refused(status, &answer, "k must be between 1 and 1000");
    }
}

/// The `fusion` object follows ADR-0090: `rrf` takes no `alpha`, and
/// `relative_score` needs one inside `[0, 1]`.
#[tokio::test]
async fn the_fusion_domain_is_checked() {
    let (_instance, _state, router) = instance_with_alpha().await;
    for (fusion, needle) in [
        (json!({ "method": "rrf", "alpha": 0.5 }), "$.fusion.alpha"),
        (
            json!({ "method": "relative_score", "alpha": 2.0 }),
            "$.fusion.alpha",
        ),
        (json!({ "method": "relative_score" }), "$.fusion.alpha"),
        (json!({ "method": "unknown" }), "$.fusion.method"),
    ] {
        let (status, answer) = search(
            &router,
            READ_SECRET,
            json!({
                "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
                "fusion": fusion,
            }),
        )
        .await;
        refused(status, &answer, needle);
    }
}

/// An empty body is refused: a search needs `query` or `vector`.
#[tokio::test]
async fn an_empty_body_is_refused() {
    let (_instance, _state, router) = instance_with_alpha().await;
    let (status, answer) = search(&router, READ_SECRET, json!({})).await;
    refused(status, &answer, "query");
}

/// A clause outside the filter grammar is `invalid_filter`, and its JSON path is
/// in the message and in `details.path` (`docs/05`, ADR-0109, ADR-0110).
#[tokio::test]
async fn a_filter_outside_its_grammar_is_an_invalid_filter_with_its_path() {
    let (_instance, _state, router) = instance_with_alpha().await;
    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({
            "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
            "mode": "dense",
            "filter": { "numero": { "regex": "1240" } },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    assert_eq!(answer["error"]["code"], "invalid_filter", "{answer}");
    let message = answer["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the refusal carries a message: {answer}"));
    assert!(message.contains("unknown operator 'regex'"), "{message}");
    assert!(message.contains("$.filter.numero"), "{message}");
    assert_eq!(
        answer["error"]["details"]["path"], "$.filter.numero",
        "the details carry the full path of the filter: {answer}"
    );
}

/// `group_by`, `dedup` and `passages_per_document` are bounded on the route,
/// and their accepted values reach the pipeline (`docs/05:266`, ADR-0075).
#[tokio::test]
async fn the_regrouping_options_are_bounded_on_the_route() {
    let (_instance, state, router) = instance_with_alpha().await;
    fill(&router, &state).await;
    for (body, needle) in [
        (
            json!({ "vector": base64_vector([1.0, 0.0, 0.0, 0.0]), "group_by": "article" }),
            "group_by",
        ),
        (
            json!({ "vector": base64_vector([1.0, 0.0, 0.0, 0.0]), "dedup": "text" }),
            "dedup",
        ),
        (
            json!({ "vector": base64_vector([1.0, 0.0, 0.0, 0.0]), "passages_per_document": 6 }),
            "passages_per_document",
        ),
        (
            json!({ "vector": base64_vector([1.0, 0.0, 0.0, 0.0]), "passages_per_document": 0 }),
            "passages_per_document",
        ),
    ] {
        let (status, answer) = search(&router, READ_SECRET, body).await;
        refused(status, &answer, needle);
    }

    // The accepted values run through the pipeline: two documents, one passage
    // each, in a flat list.
    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({
            "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
            "mode": "dense",
            "group_by": "document",
            "dedup": "content",
            "passages_per_document": 2,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let results = answer["results"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer holds results: {answer}"));
    assert_eq!(results.len(), 2, "one passage per document: {answer}");
}

/// `hybrid` on an `external` index is served when the request carries both the
/// query text and the vector (ADR-0109, decision 4).
#[tokio::test]
async fn a_hybrid_search_on_an_external_model_is_served_with_query_and_vector() {
    let (_instance, state, router) = instance_with_index(config(INDEX, true)).await;
    fill(&router, &state).await;
    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({
            "query": "contrat",
            "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
            "mode": "hybrid",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(
        answer["results"][0]["score_type"], "rrf",
        "the two branches are fused: {answer}"
    );
}

/// `hybrid` with the query alone on an `external` index is `400`, and the
/// message names the model (ADR-0109, decision 4).
#[tokio::test]
async fn a_hybrid_search_on_an_external_model_needs_a_vector() {
    let (_instance, _state, router) = instance_with_index(config(INDEX, true)).await;
    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({ "query": "contrat", "mode": "hybrid" }),
    )
    .await;
    refused(status, &answer, "external");
    refused(status, &answer, "'m'");
}

/// A name of `return.fields` declared `returned: false` is `400`, and the
/// message names the field, its path and the declaration (ADR-0109,
/// decision 5).
#[tokio::test]
async fn a_return_field_declared_not_returned_is_refused() {
    let mut gamma = config("gamma", false);
    gamma["schema"] = json!({
        "document_fields": {
            "numero": { "type": "keyword", "returned": true },
            "interne": { "type": "keyword", "returned": false },
        },
        "passage_fields": { "texte": { "type": "keyword", "returned": true } },
    });
    let (_instance, _state, router) = instance_with_index(gamma).await;
    let (status, answer) = search_in(
        &router,
        "gamma",
        READ_SECRET,
        json!({
            "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
            "mode": "dense",
            "return": { "fields": ["interne"] },
        }),
    )
    .await;
    refused(status, &answer, "interne");
    let message = answer["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the refusal carries a message: {answer}"));
    assert!(message.contains("$.return.fields[0]"), "{message}");
    assert!(message.contains("returned: false"), "{message}");
}

/// The fixture index is `alpha`; this keeps `INDEX` used by the path helper and
/// documents the convention.
#[test]
fn the_route_path_names_the_fixture_index() {
    assert_eq!(SEARCH, format!("/v1/indexes/{INDEX}/search"));
}
