// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and every helper of this file is a fixture or an assertion.
#![allow(clippy::expect_used, clippy::panic)]

//! `POST /v1/search` (`RECHERCHE-MULTI-INDEX`, phase P6; ADR-0110).
//!
//! | Test | Acceptance criterion |
//! |---|---|
//! | `the_three_roles_reach_the_multi_index_route` | no `403` is reachable (ADR-0095) |
//! | `a_dense_fusion_equals_the_global_sort_of_the_per_index_top_k` | `:124`, order of ADR-0110 |
//! | `a_lexical_search_scores_rrf_on_the_ranks_of_each_index` | RRF in `lexical` |
//! | `a_hybrid_search_scores_rrf_on_the_ranks_of_each_index` | RRF in `hybrid` |
//! | `group_by_keeps_documents_consecutive_and_k_documents` | unit of the fusion |
//! | `group_by_under_rrf_scores_every_entry_with_the_document` | ADR-0110, decision 9 |
//! | `an_unknown_field_names_its_json_path_at_every_level` | ADR-0110, decision 2 |
//! | `the_validation_order_is_400_then_404_then_422` | ADR-0110, decision 7 |
//! | `an_incompatible_model_is_refused_in_dense_and_hybrid_only` | ADR-0110, decision 3 |
//! | `the_targets_list_is_checked_before_any_search_runs` | ADR-0110, decision 5 |
//! | `a_filter_is_checked_against_the_schema_of_its_target` | ADR-0110, decisions 2 and 7 |
//! | `return_fields_serve_heterogeneous_schemas` | ADR-0110, decision 6 |
//! | `a_target_without_lexical_is_refused_but_does_not_change_the_default` | ADR-0089, decision 4 |
//! | `the_answer_carries_the_index_of_each_result_and_one_summary_per_target` | ADR-0110, decision 10 |
//!
//! The instance serves two `external` models of four dimensions, so no ONNX
//! Runtime is loaded; the requests carry their vectors in base64. The `504` of
//! the deadline, the `503` of the concurrency limit and the count of the real
//! encoding calls live in `search_multi_timeout.rs`, `search_multi_overloaded.rs`
//! and `search_multi_encoding.rs`.
//!
//! ```text
//! cargo test -p crible-server --test routes_search_multi
//! ```

mod common;

use std::sync::Arc;

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::http::build_router;
use crible_server::state::AppState;
use serde_json::{Map, Value, json};

use common::{
    ADMIN_SECRET, Instance, READ_SECRET, REQUEST_ID, WRITE_SECRET, base64_vector, index_config,
    send, terminal_job,
};

/// Path of the multi-index route.
const MULTI: &str = "/v1/search";

/// Posts `body` on the multi-index route with `secret`.
async fn search(router: &Router, secret: &str, body: Value) -> (StatusCode, Value) {
    send(router, Method::POST, MULTI, secret, Some(body)).await
}

/// Posts a search on one index, the reference path of the per-index top-k.
async fn simple(router: &Router, index: &str, body: Value) -> Value {
    let path = format!("/v1/indexes/{index}/search");
    let (status, answer) = send(router, Method::POST, &path, READ_SECRET, Some(body)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the search of {index} runs: {answer}"
    );
    answer
}

/// Writes the `external` model `n`, whose fingerprint differs from `m`.
fn write_model(instance: &Instance, name: &str, normalize: bool) {
    let directory = instance.root().join("models").join(name);
    std::fs::create_dir_all(&directory).expect("the model directory is creatable");
    std::fs::write(
        directory.join("model.json"),
        format!(r#"{{"name":"{name}","kind":"external","dims":4,"normalize":{normalize}}}"#),
    )
    .expect("the model configuration is written");
}

/// Creates an instance with the two external models `m` and `n`.
fn instance() -> (Instance, Arc<AppState>, Router) {
    let instance = Instance::new();
    write_model(&instance, "n", false);
    let state = instance.state();
    let router = build_router(Arc::clone(&state));
    (instance, state, router)
}

/// Body of an index creation, with one document field and one passage field.
fn index_body(
    name: &str,
    model: &str,
    lexical: bool,
    document_fields: Value,
    passage_fields: Value,
) -> Value {
    let mut body = index_config();
    body["name"] = json!(name);
    body["model"] = json!(model);
    body["lexical"]["enabled"] = json!(lexical);
    body["schema"] = json!({
        "document_fields": document_fields,
        "passage_fields": passage_fields,
    });
    body
}

/// Creates the index `body` names.
async fn create(router: &Router, body: Value) {
    let name = body["name"].clone();
    let (status, answer) = send(
        router,
        Method::POST,
        "/v1/indexes",
        ADMIN_SECRET,
        Some(body),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the index {name} is created: {answer}"
    );
}

/// Creates an index with one `keyword` field at each level.
async fn create_simple(
    router: &Router,
    name: &str,
    model: &str,
    lexical: bool,
    document_field: &str,
    passage_field: &str,
) {
    create(
        router,
        index_body(
            name,
            model,
            lexical,
            json!({ document_field: { "type": "keyword", "returned": true } }),
            json!({ passage_field: { "type": "keyword", "returned": true } }),
        ),
    )
    .await;
}

/// One online-batch document.
fn document(doc_id: &str, field: &str, value: &str, passages: Vec<Value>) -> Value {
    let mut fields = Map::new();
    fields.insert(field.to_owned(), json!(value));
    json!({ "doc_id": doc_id, "fields": fields, "passages": passages })
}

/// One online-batch passage.
fn passage(text: &str, field: &str, value: &str, vector: [f32; 4]) -> Value {
    let mut fields = Map::new();
    fields.insert(field.to_owned(), json!(value));
    json!({ "text": text, "fields": fields, "vector": base64_vector(vector) })
}

/// Attaches `documents` to `index` and waits for the job.
async fn attach(
    router: &Router,
    state: &Arc<AppState>,
    index: &str,
    batch_id: &str,
    documents: Vec<Value>,
) {
    let path = format!("/v1/indexes/{index}/documents?wait=true");
    let body = json!({ "batch_id": batch_id, "mode": "upsert", "documents": documents });
    let (status, accepted) = send(router, Method::POST, &path, WRITE_SECRET, Some(body)).await;
    assert!(
        status == StatusCode::OK || status == StatusCode::ACCEPTED,
        "the batch of {index} is accepted: {accepted}"
    );
    let job = terminal_job(router, state, &accepted).await;
    assert_eq!(
        job["state"], "succeeded",
        "the batch of {index} attaches: {job}"
    );
}

/// Refuses a body with `code` and returns its message.
fn refused(status: StatusCode, body: &Value, code: &str) -> String {
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "the request is refused: {body}"
    );
    assert_eq!(body["error"]["code"], code, "{body}");
    body["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the refusal carries a message: {body}"))
        .to_owned()
}

/// The `(index, key)` pairs of an answer, in its order.
fn pairs(answer: &Value) -> Vec<(String, String)> {
    answer["results"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer holds results: {answer}"))
        .iter()
        .map(|result| {
            (
                result["index"]
                    .as_str()
                    .unwrap_or_else(|| panic!("a result names its index: {result}"))
                    .to_owned(),
                result["key"]
                    .as_str()
                    .unwrap_or_else(|| panic!("a result carries its key: {result}"))
                    .to_owned(),
            )
        })
        .collect()
}

/// Panics when a key at any depth of `value` contains `iid` (INV-01).
fn assert_no_iid(label: &str, value: &Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                assert!(
                    !key.to_ascii_lowercase().contains("iid"),
                    "the key '{key}' of {label} names an internal id (INV-01): {value}"
                );
                assert_no_iid(label, child);
            }
        }
        Value::Array(items) => {
            for item in items {
                assert_no_iid(label, item);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

/// The three roles reach the route: `read` is the mount role, `write` and
/// `admin` include it (ADR-0095).
#[tokio::test]
async fn the_three_roles_reach_the_multi_index_route() {
    let (_instance, state, router) = instance();
    create_simple(&router, "alpha", "m", false, "numero", "texte").await;
    create_simple(&router, "beta", "m", false, "code", "partie").await;
    attach(
        &router,
        &state,
        "alpha",
        "a-1",
        vec![document(
            "A-1",
            "numero",
            "1240",
            vec![passage("contrat", "texte", "contrat", [1.0, 0.0, 0.0, 0.0])],
        )],
    )
    .await;
    attach(
        &router,
        &state,
        "beta",
        "b-1",
        vec![document(
            "B-1",
            "code",
            "B1",
            vec![passage("contrat", "partie", "moyens", [0.0, 1.0, 0.0, 0.0])],
        )],
    )
    .await;
    let body = json!({
        "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
        "mode": "dense",
        "targets": [ { "index": "alpha" }, { "index": "beta" } ],
    });
    for secret in [READ_SECRET, WRITE_SECRET, ADMIN_SECRET] {
        let (status, answer) = search(&router, secret, body.clone()).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the role reaches the route: {answer}"
        );
        assert_eq!(answer["request_id"], REQUEST_ID, "{answer}");
    }
}

/// In `dense`, the fusion equals the global sort of the per-index top-k, with
/// the order of ADR-0110 (score, then index, then key).
#[tokio::test]
async fn a_dense_fusion_equals_the_global_sort_of_the_per_index_top_k() {
    let (_instance, state, router) = instance();
    create_simple(&router, "alpha", "m", false, "numero", "texte").await;
    create_simple(&router, "beta", "m", false, "code", "partie").await;
    // Two unit vectors give exact cosines against `[1, 0, 0, 0]`; `A-2` and
    // `B-2` share the same vector, so their scores tie and the index name then
    // the key decide (ADR-0110, decision 9).
    attach(
        &router,
        &state,
        "alpha",
        "a-1",
        vec![
            document(
                "A-1",
                "numero",
                "1",
                vec![passage("un", "texte", "un", [1.0, 0.0, 0.0, 0.0])],
            ),
            document(
                "A-2",
                "numero",
                "2",
                vec![passage("deux", "texte", "deux", [0.8, 0.6, 0.0, 0.0])],
            ),
            document(
                "A-3",
                "numero",
                "3",
                vec![passage("trois", "texte", "trois", [0.0, 1.0, 0.0, 0.0])],
            ),
        ],
    )
    .await;
    attach(
        &router,
        &state,
        "beta",
        "b-1",
        vec![
            document(
                "B-1",
                "code",
                "B1",
                vec![passage("un", "partie", "un", [0.9, 0.435_889_9, 0.0, 0.0])],
            ),
            document(
                "B-2",
                "code",
                "B2",
                vec![passage("deux", "partie", "deux", [0.8, 0.6, 0.0, 0.0])],
            ),
            document(
                "B-3",
                "code",
                "B3",
                vec![passage("trois", "partie", "trois", [0.6, 0.8, 0.0, 0.0])],
            ),
        ],
    )
    .await;

    let vector = base64_vector([1.0, 0.0, 0.0, 0.0]);
    let per_index = json!({ "vector": vector, "mode": "dense", "k": 3 });
    let alpha = simple(&router, "alpha", per_index.clone()).await;
    let beta = simple(&router, "beta", per_index).await;

    // The reference: the global sort of the two top-k, by the documented order.
    let mut expected: Vec<(f64, String, String, f64)> = Vec::new();
    for (index, answer) in [("alpha", &alpha), ("beta", &beta)] {
        for result in answer["results"]
            .as_array()
            .unwrap_or_else(|| panic!("the answer holds results: {answer}"))
        {
            expected.push((
                result["score"]
                    .as_f64()
                    .unwrap_or_else(|| panic!("a score is a number: {result}")),
                index.to_owned(),
                result["key"]
                    .as_str()
                    .unwrap_or_else(|| panic!("a key is a string: {result}"))
                    .to_owned(),
                result["score"]
                    .as_f64()
                    .unwrap_or_else(|| panic!("a score is a number: {result}")),
            ));
        }
    }
    expected.sort_by(|left, right| {
        right
            .0
            .total_cmp(&left.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    expected.truncate(3);

    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({
            "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
            "mode": "dense",
            "k": 3,
            "targets": [ { "index": "alpha" }, { "index": "beta" } ],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["truncated_query"], false, "{answer}");
    assert!(answer["took_ms"].is_number(), "{answer}");
    assert_no_iid("the multi-index answer", &answer);

    let found = pairs(&answer);
    let wanted: Vec<(String, String)> = expected
        .iter()
        .map(|(_, index, key, _)| (index.clone(), key.clone()))
        .collect();
    assert_eq!(found, wanted, "the fusion equals the global sort: {answer}");

    for (result, (expected_score, _, _, _)) in answer["results"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer holds results: {answer}"))
        .iter()
        .zip(&expected)
    {
        let score = result["score"]
            .as_f64()
            .unwrap_or_else(|| panic!("a score is a number: {result}"));
        assert!(
            (score - expected_score).abs() < 1e-6,
            "the score of a fused result is the one of its index: {result} vs {expected_score}"
        );
        assert_eq!(result["score_type"], "cosine", "{answer}");
    }
    // The tie between `A-2` and `B-2` is broken by the index name.
    let keys = pairs(&answer);
    assert!(
        keys.iter()
            .any(|(index, key)| index == "alpha" && key == "A-2#0"),
        "the alpha tie wins over the beta one: {answer}"
    );
}

/// A lexical search scores the units by RRF on the rank of each index.
#[tokio::test]
async fn a_lexical_search_scores_rrf_on_the_ranks_of_each_index() {
    let (_instance, state, router) = instance();
    create_simple(&router, "lexa", "m", true, "numero", "texte").await;
    create_simple(&router, "lexb", "m", true, "code", "partie").await;
    attach(
        &router,
        &state,
        "lexa",
        "l-a",
        vec![
            document(
                "LA-1",
                "numero",
                "1",
                vec![passage(
                    "le contrat fait la loi",
                    "texte",
                    "contrat",
                    [1.0, 0.0, 0.0, 0.0],
                )],
            ),
            document(
                "LA-2",
                "numero",
                "2",
                vec![passage(
                    "contrat de vente",
                    "texte",
                    "vente",
                    [0.8, 0.6, 0.0, 0.0],
                )],
            ),
        ],
    )
    .await;
    attach(
        &router,
        &state,
        "lexb",
        "l-b",
        vec![
            document(
                "LB-1",
                "code",
                "B1",
                vec![passage(
                    "contrat de bail",
                    "partie",
                    "bail",
                    [0.0, 1.0, 0.0, 0.0],
                )],
            ),
            document(
                "LB-2",
                "code",
                "B2",
                vec![passage(
                    "un long contrat de travail",
                    "partie",
                    "travail",
                    [0.0, 0.0, 1.0, 0.0],
                )],
            ),
        ],
    )
    .await;

    let body = json!({ "query": "contrat", "mode": "lexical", "k": 4 });
    let lexa = simple(&router, "lexa", body.clone()).await;
    let lexb = simple(&router, "lexb", body).await;

    // The reference ranks come from the simple route, index by index.
    let mut expected: Vec<(f64, String, String)> = Vec::new();
    for (index, answer) in [("lexa", &lexa), ("lexb", &lexb)] {
        for (position, result) in answer["results"]
            .as_array()
            .unwrap_or_else(|| panic!("the answer holds results: {answer}"))
            .iter()
            .enumerate()
        {
            let rank = f64::from(u32::try_from(position + 1).unwrap_or(u32::MAX));
            expected.push((
                1.0 / (60.0 + rank),
                index.to_owned(),
                result["key"]
                    .as_str()
                    .unwrap_or_else(|| panic!("a key is a string: {result}"))
                    .to_owned(),
            ));
        }
    }
    expected.sort_by(|left, right| {
        right
            .0
            .total_cmp(&left.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    expected.truncate(4);

    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({
            "query": "contrat",
            "mode": "lexical",
            "k": 4,
            "targets": [ { "index": "lexa" }, { "index": "lexb" } ],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let found = pairs(&answer);
    let wanted: Vec<(String, String)> = expected
        .iter()
        .map(|(_, index, key)| (index.clone(), key.clone()))
        .collect();
    assert_eq!(found, wanted, "the RRF order follows the ranks: {answer}");
    for (result, (score, _, _)) in answer["results"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer holds results: {answer}"))
        .iter()
        .zip(&expected)
    {
        assert_eq!(result["score_type"], "rrf", "{answer}");
        let found = result["score"]
            .as_f64()
            .unwrap_or_else(|| panic!("a score is a number: {result}"));
        assert!(
            (found - score).abs() < 1e-6,
            "the score is 1 / (60 + rank): {result} vs {score}"
        );
    }
}

/// A hybrid search also scores the units by RRF on the rank of each index.
#[tokio::test]
async fn a_hybrid_search_scores_rrf_on_the_ranks_of_each_index() {
    let (_instance, state, router) = instance();
    create_simple(&router, "lexa", "m", true, "numero", "texte").await;
    create_simple(&router, "lexb", "m", true, "code", "partie").await;
    attach(
        &router,
        &state,
        "lexa",
        "h-a",
        vec![document(
            "HA-1",
            "numero",
            "1",
            vec![passage(
                "le contrat fait la loi",
                "texte",
                "contrat",
                [1.0, 0.0, 0.0, 0.0],
            )],
        )],
    )
    .await;
    attach(
        &router,
        &state,
        "lexb",
        "h-b",
        vec![document(
            "HB-1",
            "code",
            "B1",
            vec![passage(
                "contrat de bail",
                "partie",
                "bail",
                [0.0, 1.0, 0.0, 0.0],
            )],
        )],
    )
    .await;

    let vector = base64_vector([1.0, 0.0, 0.0, 0.0]);
    let per_index = json!({
        "query": "contrat",
        "vector": vector,
        "mode": "hybrid",
        "k": 3,
    });
    let lexa = simple(&router, "lexa", per_index.clone()).await;
    let lexb = simple(&router, "lexb", per_index).await;

    let mut expected: Vec<(f64, String, String)> = Vec::new();
    for (index, answer) in [("lexa", &lexa), ("lexb", &lexb)] {
        for (position, result) in answer["results"]
            .as_array()
            .unwrap_or_else(|| panic!("the answer holds results: {answer}"))
            .iter()
            .enumerate()
        {
            let rank = f64::from(u32::try_from(position + 1).unwrap_or(u32::MAX));
            expected.push((
                1.0 / (60.0 + rank),
                index.to_owned(),
                result["key"]
                    .as_str()
                    .unwrap_or_else(|| panic!("a key is a string: {result}"))
                    .to_owned(),
            ));
        }
    }
    expected.sort_by(|left, right| {
        right
            .0
            .total_cmp(&left.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });

    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({
            "query": "contrat",
            "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
            "mode": "hybrid",
            "k": 3,
            "targets": [ { "index": "lexa" }, { "index": "lexb" } ],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(
        pairs(&answer),
        expected
            .iter()
            .map(|(_, index, key)| (index.clone(), key.clone()))
            .collect::<Vec<_>>(),
        "the hybrid RRF follows the ranks: {answer}"
    );
    for (result, (score, _, _)) in answer["results"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer holds results: {answer}"))
        .iter()
        .zip(&expected)
    {
        assert_eq!(result["score_type"], "rrf", "{answer}");
        let found = result["score"]
            .as_f64()
            .unwrap_or_else(|| panic!("a score is a number: {result}"));
        assert!((found - score).abs() < 1e-6, "{result} vs {score}");
    }
}

/// `group_by: "document"` keeps the entries of a document consecutive, counts
/// `k` distinct documents, and ranks a document by its best passage.
#[tokio::test]
async fn group_by_keeps_documents_consecutive_and_k_documents() {
    let (_instance, state, router) = instance();
    create_simple(&router, "alpha", "m", false, "numero", "texte").await;
    create_simple(&router, "beta", "m", false, "code", "partie").await;
    attach(
        &router,
        &state,
        "alpha",
        "g-a",
        vec![document(
            "GA",
            "numero",
            "1",
            vec![
                passage("premier", "texte", "premier", [1.0, 0.0, 0.0, 0.0]),
                passage("second", "texte", "second", [0.9, 0.435_889_9, 0.0, 0.0]),
            ],
        )],
    )
    .await;
    attach(
        &router,
        &state,
        "beta",
        "g-b",
        vec![document(
            "GB",
            "code",
            "B1",
            vec![passage("autre", "partie", "autre", [0.8, 0.6, 0.0, 0.0])],
        )],
    )
    .await;

    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({
            "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
            "mode": "dense",
            "k": 2,
            "group_by": "document",
            "passages_per_document": 2,
            "targets": [ { "index": "alpha" }, { "index": "beta" } ],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let results = answer["results"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer holds results: {answer}"));
    assert_eq!(
        results.len(),
        3,
        "two documents, one with two passages: {answer}"
    );
    assert_eq!(results[0]["index"], "alpha", "{answer}");
    assert_eq!(results[0]["doc_id"], "GA", "{answer}");
    assert_eq!(
        results[1]["index"], "alpha",
        "the entries stay consecutive: {answer}"
    );
    assert_eq!(results[1]["doc_id"], "GA", "{answer}");
    assert_eq!(results[2]["index"], "beta", "{answer}");
    assert_eq!(results[2]["doc_id"], "GB", "{answer}");
    let generation = results
        .iter()
        .filter_map(|result| result["doc_id"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(generation.len(), 2, "k counts documents: {answer}");
}

/// A field outside the body is refused at every level and names its path.
#[tokio::test]
async fn an_unknown_field_names_its_json_path_at_every_level() {
    let (_instance, _state, router) = instance();
    create_simple(&router, "alpha", "m", false, "numero", "texte").await;
    for (body, needle) in [
        (
            json!({ "targets": [ { "index": "alpha", "filtre": {} } ] }),
            "$.targets[0].filtre",
        ),
        (json!({ "filtre": {} }), "$.filtre"),
        (
            json!({ "targets": [ { "index": "alpha" } ], "return": { "field": "all" } }),
            "$.return.field",
        ),
        (
            json!({ "targets": [ { "index": "alpha" } ], "fusion": { "method": "rrf", "alpah": 0.3 } }),
            "$.fusion.alpah",
        ),
    ] {
        let (status, answer) = search(&router, READ_SECRET, body).await;
        let message = refused(status, &answer, "invalid_request");
        assert!(
            message.contains(needle),
            "the message names {needle:?}: {message}"
        );
    }
    // A clause outside the filter grammar keeps `invalid_filter`, with its path
    // rooted at the target (ADR-0110, decision 2).
    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({
            "mode": "dense",
            "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
            "targets": [ { "index": "alpha", "filter": { "$xor": [] } } ],
        }),
    )
    .await;
    let message = refused(status, &answer, "invalid_filter");
    assert!(
        message.contains("$.targets[0].filter"),
        "the filter path is rooted at the target: {message}"
    );
    assert_eq!(
        answer["error"]["details"]["path"], "$.targets[0].filter.$xor",
        "details.path carries the full path of the clause: {answer}"
    );
}

/// In `dense` and `hybrid`, two different fingerprints are `422`; `lexical`
/// never compares the model (ADR-0110, decision 3).
#[tokio::test]
async fn an_incompatible_model_is_refused_in_dense_and_hybrid_only() {
    let (_instance, _state, router) = instance();
    create_simple(&router, "alpha", "m", true, "numero", "texte").await;
    create_simple(&router, "beta", "n", true, "code", "partie").await;
    let vector = base64_vector([1.0, 0.0, 0.0, 0.0]);
    for body in [
        json!({
            "mode": "dense",
            "vector": vector,
            "targets": [ { "index": "alpha" }, { "index": "beta" } ],
        }),
        json!({
            "mode": "hybrid",
            "query": "contrat",
            "vector": vector,
            "targets": [ { "index": "alpha" }, { "index": "beta" } ],
        }),
    ] {
        let (status, answer) = search(&router, READ_SECRET, body).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
        assert_eq!(answer["error"]["code"], "incompatible_model", "{answer}");
        let message = answer["error"]["message"]
            .as_str()
            .unwrap_or_else(|| panic!("the refusal carries a message: {answer}"));
        assert!(
            message.contains("alpha") && message.contains("beta"),
            "the message names the two indexes: {message}"
        );
    }
    // `lexical` compares no vector, so two models are not a fault there.
    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({
            "query": "contrat",
            "mode": "lexical",
            "targets": [ { "index": "alpha" }, { "index": "beta" } ],
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "lexical ignores the models: {answer}"
    );
}

/// `targets` absent, empty, duplicated or unknown is refused before any search
/// runs (ADR-0110, decision 5).
#[tokio::test]
async fn the_targets_list_is_checked_before_any_search_runs() {
    let (_instance, _state, router) = instance();
    create_simple(&router, "alpha", "m", false, "numero", "texte").await;
    let vector = base64_vector([1.0, 0.0, 0.0, 0.0]);
    let absent = json!({ "mode": "dense", "vector": vector });
    let (status, answer) = search(&router, READ_SECRET, absent).await;
    let message = refused(status, &answer, "invalid_request");
    assert!(message.contains("$.targets"), "{message}");

    let empty = json!({ "mode": "dense", "vector": vector, "targets": [] });
    let (status, answer) = search(&router, READ_SECRET, empty).await;
    let message = refused(status, &answer, "invalid_request");
    assert!(message.contains("$.targets"), "{message}");

    let duplicate = json!({
        "mode": "dense",
        "vector": vector,
        "targets": [ { "index": "alpha" }, { "index": "alpha" } ],
    });
    let (status, answer) = search(&router, READ_SECRET, duplicate).await;
    let message = refused(status, &answer, "invalid_request");
    assert!(
        message.contains("$.targets[1].index"),
        "the message names the repeated target: {message}"
    );

    let unknown = json!({
        "mode": "dense",
        "vector": vector,
        "targets": [ { "index": "alpha" }, { "index": "absent" } ],
    });
    let (status, answer) = search(&router, READ_SECRET, unknown).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{answer}");
    assert_eq!(answer["error"]["code"], "unknown_index", "{answer}");
    assert!(
        answer.get("results").is_none(),
        "no partial result is served: {answer}"
    );
}

/// The first fault is rendered in the order of ADR-0110, decision 7: `400`
/// before `404`, and `404` before `422`.
#[tokio::test]
async fn the_validation_order_is_400_then_404_then_422() {
    let (_instance, _state, router) = instance();
    create_simple(&router, "alpha", "m", false, "numero", "texte").await;
    create_simple(&router, "beta", "n", false, "code", "partie").await;
    let vector = base64_vector([1.0, 0.0, 0.0, 0.0]);

    // The duplicate is a `400` of the body, and it wins over the unknown index
    // the same request repeats.
    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({
            "mode": "dense",
            "vector": vector,
            "targets": [ { "index": "absent" }, { "index": "absent" } ],
        }),
    )
    .await;
    let message = refused(status, &answer, "invalid_request");
    assert!(
        message.contains("$.targets[1].index"),
        "the duplicate is the first fault: {message}"
    );

    // `alpha` and `beta` have different models (`422`), but `absent` does not
    // exist: the `404` of the target list comes first.
    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({
            "mode": "dense",
            "vector": vector,
            "targets": [
                { "index": "alpha" },
                { "index": "beta" },
                { "index": "absent" },
            ],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{answer}");
    assert_eq!(answer["error"]["code"], "unknown_index", "{answer}");
    let message = answer["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the refusal carries a message: {answer}"));
    assert!(message.contains("absent"), "{message}");
}

/// Under RRF, every entry of a regrouped document carries the score of its
/// document (ADR-0110, decision 9).
#[tokio::test]
async fn group_by_under_rrf_scores_every_entry_with_the_document() {
    let (_instance, state, router) = instance();
    create_simple(&router, "lexg", "m", true, "numero", "texte").await;
    attach(
        &router,
        &state,
        "lexg",
        "g-rrf",
        vec![
            document(
                "G1",
                "numero",
                "1",
                vec![
                    passage(
                        "le contrat fait la loi",
                        "texte",
                        "un",
                        [1.0, 0.0, 0.0, 0.0],
                    ),
                    passage("contrat de vente", "texte", "deux", [0.0, 1.0, 0.0, 0.0]),
                ],
            ),
            document(
                "G2",
                "numero",
                "2",
                vec![passage(
                    "un contrat de bail",
                    "texte",
                    "trois",
                    [0.0, 0.0, 1.0, 0.0],
                )],
            ),
        ],
    )
    .await;

    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({
            "query": "contrat",
            "mode": "lexical",
            "k": 2,
            "group_by": "document",
            "passages_per_document": 2,
            "targets": [ { "index": "lexg" } ],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let results = answer["results"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer holds results: {answer}"));
    assert_eq!(
        results.len(),
        3,
        "two documents, one with two passages: {answer}"
    );

    // The entries of one document stay consecutive: one run per document, in
    // the order of the answer.
    let mut runs: Vec<(String, Vec<f64>)> = Vec::new();
    for result in results {
        let doc_id = result["doc_id"]
            .as_str()
            .unwrap_or_else(|| panic!("a result carries its document: {result}"))
            .to_owned();
        let score = result["score"]
            .as_f64()
            .unwrap_or_else(|| panic!("a score is a number: {result}"));
        match runs.last_mut() {
            Some((last, scores)) if *last == doc_id => scores.push(score),
            _ => runs.push((doc_id, vec![score])),
        }
    }
    assert_eq!(runs.len(), 2, "k counts documents: {answer}");
    for (position, (_, scores)) in runs.iter().enumerate() {
        let rank = f64::from(u32::try_from(position + 1).unwrap_or(u32::MAX));
        let expected = 1.0 / (60.0 + rank);
        for score in scores {
            assert!(
                (score - expected).abs() < 1e-6,
                "every entry carries the document score {expected}: {answer}"
            );
        }
    }
    assert!(
        runs.iter().any(|(_, scores)| scores.len() == 2),
        "the grouped document keeps its two entries: {answer}"
    );
}

/// Each target applies its own filter; an invalid one is `invalid_filter` and
/// names its path.
#[tokio::test]
async fn a_filter_is_checked_against_the_schema_of_its_target() {
    let (_instance, state, router) = instance();
    create_simple(&router, "alpha", "m", false, "numero", "texte").await;
    create_simple(&router, "beta", "m", false, "code", "partie").await;
    attach(
        &router,
        &state,
        "alpha",
        "f-a",
        vec![
            document(
                "A-1",
                "numero",
                "1240",
                vec![passage("un", "texte", "un", [1.0, 0.0, 0.0, 0.0])],
            ),
            document(
                "A-2",
                "numero",
                "1241",
                vec![passage(
                    "deux",
                    "texte",
                    "deux",
                    [0.9, 0.435_889_9, 0.0, 0.0],
                )],
            ),
        ],
    )
    .await;
    attach(
        &router,
        &state,
        "beta",
        "f-b",
        vec![
            document(
                "B-1",
                "code",
                "B1",
                vec![passage("un", "partie", "un", [1.0, 0.0, 0.0, 0.0])],
            ),
            document(
                "B-2",
                "code",
                "B2",
                vec![passage(
                    "deux",
                    "partie",
                    "deux",
                    [0.9, 0.435_889_9, 0.0, 0.0],
                )],
            ),
        ],
    )
    .await;

    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({
            "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
            "mode": "dense",
            "targets": [
                { "index": "alpha", "filter": { "numero": { "eq": "1240" } } },
                { "index": "beta", "filter": { "code": { "eq": "B2" } } },
            ],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let found = answer["results"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer holds results: {answer}"))
        .iter()
        .map(|result| {
            (
                result["index"]
                    .as_str()
                    .unwrap_or_else(|| panic!("a result names its index: {result}"))
                    .to_owned(),
                result["doc_id"]
                    .as_str()
                    .unwrap_or_else(|| panic!("a result carries its document: {result}"))
                    .to_owned(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        found,
        vec![
            ("alpha".to_owned(), "A-1".to_owned()),
            ("beta".to_owned(), "B-2".to_owned())
        ],
        "each filter kept its own document: {answer}"
    );

    // `filterable: false` and an unknown field are `invalid_filter`, with the
    // path of the target.
    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({
            "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
            "mode": "dense",
            "targets": [
                { "index": "alpha" },
                { "index": "beta", "filter": { "inconnu": { "eq": "x" } } },
            ],
        }),
    )
    .await;
    let message = refused(status, &answer, "invalid_filter");
    assert!(
        message.contains("$.targets[1].filter.inconnu"),
        "the filter path names its target: {message}"
    );
    assert_eq!(
        answer["error"]["details"]["path"], "$.targets[1].filter.inconnu",
        "details.path carries the full path of the clause: {answer}"
    );
}

/// `return.fields` serves heterogeneous schemas, and refuses a name no target
/// declares or one declared `returned: false` (ADR-0110, decision 6).
#[tokio::test]
async fn return_fields_serve_heterogeneous_schemas() {
    let (_instance, state, router) = instance();
    create_simple(&router, "alpha", "m", false, "numero", "texte").await;
    create_simple(&router, "beta", "m", false, "code", "partie").await;
    create(
        &router,
        index_body(
            "gamma",
            "m",
            false,
            json!({
                "numero": { "type": "keyword", "returned": true },
                "interne": { "type": "keyword", "returned": false },
            }),
            json!({ "note": { "type": "keyword", "returned": true } }),
        ),
    )
    .await;
    attach(
        &router,
        &state,
        "alpha",
        "r-a",
        vec![document(
            "A-1",
            "numero",
            "1240",
            vec![passage("un", "texte", "un", [1.0, 0.0, 0.0, 0.0])],
        )],
    )
    .await;
    attach(
        &router,
        &state,
        "beta",
        "r-b",
        vec![document(
            "B-1",
            "code",
            "B1",
            vec![passage("deux", "partie", "deux", [1.0, 0.0, 0.0, 0.0])],
        )],
    )
    .await;

    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({
            "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
            "mode": "dense",
            "return": { "fields": ["numero", "code"] },
            "targets": [ { "index": "alpha" }, { "index": "beta" } ],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    for result in answer["results"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer holds results: {answer}"))
    {
        match result["index"].as_str() {
            Some("alpha") => {
                assert_eq!(result["fields"], json!({ "numero": "1240" }), "{answer}");
            }
            Some("beta") => {
                assert_eq!(result["fields"], json!({ "code": "B1" }), "{answer}");
            }
            other => panic!("a result names a target: {other:?} in {answer}"),
        }
    }

    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({
            "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
            "mode": "dense",
            "return": { "fields": ["absent"] },
            "targets": [ { "index": "alpha" }, { "index": "beta" } ],
        }),
    )
    .await;
    let message = refused(status, &answer, "invalid_request");
    assert!(message.contains("$.return.fields[0]"), "{message}");

    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({
            "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
            "mode": "dense",
            "return": { "fields": ["interne"] },
            "targets": [ { "index": "gamma" } ],
        }),
    )
    .await;
    let message = refused(status, &answer, "invalid_request");
    for needle in ["$.return.fields[0]", "interne", "gamma", "returned: false"] {
        assert!(
            message.contains(needle),
            "the message names {needle:?}: {message}"
        );
    }
}

/// A target without the lexical branch refuses `lexical` and `hybrid` in full,
/// and never changes the default mode to one it would refuse (ADR-0089,
/// decision 4; ADR-0110, decision 4).
#[tokio::test]
async fn a_target_without_lexical_is_refused_but_does_not_change_the_default() {
    let (_instance, state, router) = instance();
    create_simple(&router, "alpha", "m", true, "numero", "texte").await;
    create_simple(&router, "beta", "m", false, "code", "partie").await;
    attach(
        &router,
        &state,
        "beta",
        "d-b",
        vec![document(
            "B-1",
            "code",
            "B1",
            vec![passage("un", "partie", "un", [1.0, 0.0, 0.0, 0.0])],
        )],
    )
    .await;
    // `hybrid` carries a vector so that the external-model check passes and the
    // missing branch of `beta` is the reason named.
    for (mode, vector) in [
        ("lexical", Value::Null),
        ("hybrid", json!(base64_vector([1.0, 0.0, 0.0, 0.0]))),
    ] {
        let mut body = json!({
            "query": "contrat",
            "mode": mode,
            "targets": [ { "index": "alpha" }, { "index": "beta" } ],
        });
        if !vector.is_null() {
            body["vector"] = vector;
        }
        let (status, answer) = search(&router, READ_SECRET, body).await;
        let message = refused(status, &answer, "invalid_request");
        assert!(
            message.contains("lexical branch is disabled") && message.contains("beta"),
            "the message names the target without lexical: {message}"
        );
    }
    // The default is `dense` as soon as one target lacks the branch, and that
    // mode never refuses this request.
    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({
            "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
            "targets": [ { "index": "alpha" }, { "index": "beta" } ],
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the default mode is dense: {answer}"
    );
    assert_eq!(answer["results"][0]["score_type"], "cosine", "{answer}");
}

/// The answer carries `index` in each result and one `targets[]` summary per
/// target, in the order of the request (ADR-0110, decision 10).
#[tokio::test]
async fn the_answer_carries_the_index_of_each_result_and_one_summary_per_target() {
    let (_instance, state, router) = instance();
    create_simple(&router, "alpha", "m", false, "numero", "texte").await;
    create_simple(&router, "beta", "m", false, "code", "partie").await;
    attach(
        &router,
        &state,
        "alpha",
        "s-a",
        vec![document(
            "A-1",
            "numero",
            "1",
            vec![passage("un", "texte", "un", [1.0, 0.0, 0.0, 0.0])],
        )],
    )
    .await;
    attach(
        &router,
        &state,
        "beta",
        "s-b",
        vec![document(
            "B-1",
            "code",
            "B1",
            vec![passage(
                "deux",
                "partie",
                "deux",
                [0.707_106_77, 0.707_106_77, 0.0, 0.0],
            )],
        )],
    )
    .await;

    let (status, answer) = search(
        &router,
        READ_SECRET,
        json!({
            "vector": base64_vector([1.0, 0.0, 0.0, 0.0]),
            "mode": "dense",
            "targets": [ { "index": "beta" }, { "index": "alpha" } ],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let summaries = answer["targets"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer holds targets: {answer}"));
    assert_eq!(summaries.len(), 2, "{answer}");
    assert_eq!(
        summaries[0]["index"], "beta",
        "the order is the request one: {answer}"
    );
    assert_eq!(summaries[1]["index"], "alpha", "{answer}");
    for summary in summaries {
        assert!(summary["generation"].is_number(), "{answer}");
        assert_eq!(summary["pool_size"], 1, "{answer}");
    }
    assert!(
        answer.get("generation").is_none(),
        "no scalar generation: {answer}"
    );
    assert!(
        answer.get("pool_size").is_none(),
        "no scalar pool_size: {answer}"
    );
    for result in answer["results"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer holds results: {answer}"))
    {
        assert!(result["index"].is_string(), "{answer}");
    }
    assert_eq!(answer["truncated_query"], false, "{answer}");
}
