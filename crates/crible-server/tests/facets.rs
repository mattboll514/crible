// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and every helper of this file is a fixture or an assertion.
#![allow(clippy::expect_used, clippy::panic)]

//! `POST /v1/indexes/{index}/facets` — the facets route of ADR-0154, and the
//! invariants it cites (INV-01, INV-03, INV-13).
//!
//! The ticket has no fiche in `docs/phases/`: the specification is ADR-0154,
//! decisions 1 to 11 and its section "À surveiller", with the "Facettes"
//! section of `docs/05-api.md`.
//!
//! | Test | What it proves |
//! |---|---|
//! | `inv_01_facets_render_no_internal_id_in_the_body_the_journal_and_the_analytics` | INV-01 |
//! | `inv_03_facets_answer_one_snapshot_during_fifty_concurrent_attaches` | INV-03, one snapshot per answer |
//! | `inv_13_facets_hold_no_field_value_nor_filter_value_in_the_journal_nor_in_the_analytics` | INV-13, and the operation name `facets` |
//! | `the_facet_counts_equal_the_search_pool_and_the_live_passages_of_the_index` | criterion 1 |
//! | `the_value_counts_sum_to_the_pool_passages_carrying_a_value` | criterion 2 |
//! | `the_values_are_ordered_by_count_then_bytes_and_two_identical_calls_agree` | criterion 3 |
//! | `the_limit_domain_is_one_to_one_hundred_and_fields_carry_one_to_ten_distinct_names` | criterion 4 |
//! | `a_field_of_another_type_an_unknown_field_or_a_non_filterable_field_is_invalid_request` | criterion 5 |
//! | `a_refused_filter_an_unknown_index_and_a_missing_key_keep_the_codes_of_the_api` | criterion 6 |
//! | `an_empty_pool_answers_two_hundred_with_zero_counts_and_an_empty_values_list` | criterion 7 |
//! | `the_startup_refuses_the_facets_route_with_the_fixed_overloaded_message` | criterion 11, ADR-0146 |
//! | `count_documents_is_exact_across_the_passages_of_a_document_and_after_an_attach` | criterion 12 |
//!
//! Criterion 10 is covered by the markers of `docs/05-api.md`: the components
//! `FacetsRequest` and `FacetsResponse` are validated by `openapi_examples.rs`,
//! and the row of the table is compared, in both directions, with the paths of
//! the generated specification by `openapi_routes.rs`. No test is added here for
//! it; both existing tests fail until the route exists, for the right reason.
//!
//! ```text
//! cargo test -p crible-server --test facets
//! ```
//!
//! Criterion 12 assumes `count.documents` is served: if MES-18 applies the
//! repli of ADR-0154, decision 11, that member is omitted and the test fails on
//! purpose — it is the test to adapt when the repli is decided.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::http::header::{AUTHORIZATION, RETRY_AFTER};
use axum::http::{HeaderMap, Method, StatusCode};
use crible_server::Auth;
use crible_server::auth::sha256_hex;
use crible_server::config::{ApiKey, Role};
use crible_server::http::{REQUEST_ID_HEADER, build_router};
use crible_server::state::AppState;
use crible_store::analytics::{ANALYTICS_FILE_NAME, MinuteEvent, read_minute_events};
use serde_json::{Value, json};
use tower::ServiceExt;

use common::{
    ADMIN_SECRET, INDEX, Instance, READ_SECRET, WRITE_SECRET, assert_no_iid_key, base64_vector,
    index_config, instance_with_alpha, journal, send, terminal_job,
};

/// The vector of every passage of this file: no branch is scored by a facet.
const VECTOR: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

/// Number of attaches of the INV-03 test: "une cinquantaine de fois".
const ATTACHES: u64 = 50;

// --- fixtures -------------------------------------------------------------

/// Creates an instance whose only index is described by `config`.
async fn instance_with(config: Value) -> (Instance, Arc<AppState>, Router) {
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
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the index is created: {answer}"
    );
    (instance, state, router)
}

/// The fixture index `alpha` with the lexical branch enabled.
///
/// Criterion 1 compares the facet counts with the `pool_size` of a `lexical`
/// search, which the fixture of `common` cannot serve: its lexical branch is
/// disabled (ADR-0089).
async fn instance_with_alpha_and_lexical() -> (Instance, Arc<AppState>, Router) {
    let mut config = index_config();
    config["name"] = json!(INDEX);
    config["lexical"]["enabled"] = json!(true);
    instance_with(config).await
}

/// One passage of an online batch, with its vector and its optional `texte`.
fn passage(text: &str, texte: Option<&str>) -> Value {
    let mut passage = json!({ "text": text, "vector": base64_vector(VECTOR) });
    if let Some(texte) = texte {
        passage["fields"] = json!({ "texte": texte });
    }
    passage
}

/// One document of an online batch, with its optional document field `numero`.
fn document(doc_id: &str, numero: Option<&str>, passages: Vec<Value>) -> Value {
    let mut document = json!({ "doc_id": doc_id, "passages": passages });
    if let Some(numero) = numero {
        document["fields"] = json!({ "numero": numero });
    }
    document
}

/// Attaches `documents` and `deletions` to `index` and returns the job.
async fn attach(
    router: &Router,
    state: &Arc<AppState>,
    index: &str,
    batch_id: &str,
    documents: Vec<Value>,
    deletions: Value,
) -> Value {
    let path = format!("/v1/indexes/{index}/documents?wait=true");
    let body = json!({
        "batch_id": batch_id,
        "mode": "upsert",
        "documents": documents,
        "deletions": deletions,
    });
    let (status, accepted) = send(router, Method::POST, &path, WRITE_SECRET, Some(body)).await;
    assert!(
        status == StatusCode::OK || status == StatusCode::ACCEPTED,
        "the batch {batch_id} is accepted: {accepted}"
    );
    let job = terminal_job(router, state, &accepted).await;
    assert_eq!(
        job["state"], "succeeded",
        "the batch {batch_id} attaches: {job}"
    );
    job
}

// --- the route ------------------------------------------------------------

/// Posts `body` on the facets route of `index` with `secret`.
async fn facets_in(router: &Router, index: &str, secret: &str, body: Value) -> (StatusCode, Value) {
    let path = format!("/v1/indexes/{index}/facets");
    send(router, Method::POST, &path, secret, Some(body)).await
}

/// Posts `body` on the facets route of the fixture index `alpha`.
async fn facets(router: &Router, body: Value) -> (StatusCode, Value) {
    facets_in(router, INDEX, READ_SECRET, body).await
}

/// Sends a facet request of the fixture index and asserts `200`.
async fn facets_ok(router: &Router, body: Value) -> Value {
    let (status, answer) = facets(router, body).await;
    assert_eq!(status, StatusCode::OK, "the facets call answers: {answer}");
    answer
}

/// Asserts a `400 invalid_request` with `details: {}` and returns its message.
fn invalid_request(answer: &Value) -> String {
    assert_eq!(
        answer["error"]["code"], "invalid_request",
        "the refusal is an invalid request: {answer}"
    );
    assert_eq!(
        answer["error"]["details"],
        json!({}),
        "a refused request adds no key to `details` (ADR-0101): {answer}"
    );
    answer["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the refusal carries a message: {answer}"))
        .to_owned()
}

// --- accessors of the answer ---------------------------------------------

/// The `{ values, other }` object of `field`.
fn field_entry<'a>(answer: &'a Value, field: &str) -> &'a Value {
    answer["fields"]
        .get(field)
        .unwrap_or_else(|| panic!("the answer carries the field `{field}`: {answer}"))
}

/// The `(value, count)` pairs of `field`, in the order of the answer.
fn field_values(answer: &Value, field: &str) -> Vec<(String, u64)> {
    field_entry(answer, field)["values"]
        .as_array()
        .unwrap_or_else(|| panic!("the field `{field}` carries its values: {answer}"))
        .iter()
        .map(|value| {
            (
                value["value"]
                    .as_str()
                    .unwrap_or_else(|| panic!("a value is a string: {value}"))
                    .to_owned(),
                value["count"]
                    .as_u64()
                    .unwrap_or_else(|| panic!("a count is an unsigned integer: {value}")),
            )
        })
        .collect()
}

/// The `other` counter of `field`.
fn other(answer: &Value, field: &str) -> u64 {
    field_entry(answer, field)["other"]
        .as_u64()
        .unwrap_or_else(|| panic!("the field `{field}` carries `other`: {answer}"))
}

/// The unsigned member `path` of `answer`.
fn unsigned(answer: &Value, path: &str) -> u64 {
    answer["count"][path]
        .as_u64()
        .unwrap_or_else(|| panic!("the answer carries `count.{path}`: {answer}"))
}

// --- INV-01 ---------------------------------------------------------------

/// INV-01 — no internal id leaves the engine through the facets route.
///
/// The response is walked: no key contains `iid`, and no number equals a live
/// iid under a key the API does not document as a counter. The fixture attaches
/// two passages in a fresh index, so the only live iids are `0` and `1`.
/// The journal of the same call and the raw bytes of `analytics.sqlite` (with
/// its `-wal` and `-shm`) are scanned for the same substring.
#[tokio::test]
async fn inv_01_facets_render_no_internal_id_in_the_body_the_journal_and_the_analytics() {
    let sink = journal();
    let (instance, state, router) = instance_with_alpha().await;
    attach(
        &router,
        &state,
        INDEX,
        "inv-01-facets",
        vec![document(
            "D1",
            Some("1240"),
            vec![
                passage("le contrat", Some("contrat")),
                passage("le delit", Some("autre")),
            ],
        )],
        json!([]),
    )
    .await;

    let answer = facets_ok(&router, json!({ "fields": ["numero", "texte"] })).await;
    assert_eq!(unsigned(&answer, "passages"), 2, "{answer}");
    assert_no_iid_key("the facets answer", &answer);
    assert_no_iid_number("the facets answer", &answer, &[0, 1]);

    state
        .flush_analytics()
        .await
        .expect("the facet events are flushed");

    let analytics = analytics_bytes(&analytics_path(&instance));
    assert!(
        analytics
            .iter()
            .any(|(_, bytes)| contains(bytes, b"facets")),
        "the analytics files hold the row of the call, so the scan is not vacuous"
    );
    let journal_text = sink.contents();
    assert!(
        journal_text.contains("models and indexes loaded"),
        "the capture holds the journal of the server: {journal_text:?}"
    );
    assert!(
        !journal_text.to_ascii_lowercase().contains("iid"),
        "the journal names an internal id (INV-01): {journal_text}"
    );
    for (name, bytes) in &analytics {
        assert!(
            !contains(&bytes.to_ascii_lowercase(), b"iid"),
            "the analytics file {name} names an internal id (INV-01)"
        );
    }
}

/// Panics when a number of `value` equals a live iid under a key the facets
/// answer does not document as a counter.
fn assert_no_iid_number(label: &str, value: &Value, live_iids: &[u64]) {
    const COUNTERS: [&str; 5] = ["generation", "passages", "documents", "count", "other"];
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if let Some(number) = child.as_u64() {
                    assert!(
                        COUNTERS.contains(&key.as_str()) || !live_iids.contains(&number),
                        "the value of '{key}' in {label} equals the internal id {number} (INV-01)"
                    );
                }
                assert_no_iid_number(label, child, live_iids);
            }
        }
        Value::Array(items) => {
            for item in items {
                assert_no_iid_number(label, item, live_iids);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

// --- INV-03 ---------------------------------------------------------------

/// INV-03 — one facets answer comes from a single published snapshot, even
/// while an attach runs.
///
/// The fixture attaches fifty documents one generation at a time; the writer
/// signals each completed attach on a channel, and the test answers every signal
/// with one facets call, so a call overlaps the next attach. The fixture is
/// built so that generation `g` has exactly `g` live passages, `g` documents and
/// `g` distinct values of `numero`, each count `1`: an answer that mixed two
/// generations would break one of those equalities. Nothing sleeps; the
/// synchronisation is the channel. The test asserts how many answers it checked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inv_03_facets_answer_one_snapshot_during_fifty_concurrent_attaches() {
    let (_instance, state, router) = instance_with_alpha().await;
    let body = json!({ "fields": ["numero"], "limit": 100 });

    let (sender, mut receiver) = tokio::sync::mpsc::channel::<()>(ATTACHES as usize);
    let writer_router = router.clone();
    let writer_state = Arc::clone(&state);
    let writer = tokio::spawn(async move {
        for generation in 1..=ATTACHES {
            attach(
                &writer_router,
                &writer_state,
                INDEX,
                &format!("inv-03-{generation}"),
                vec![document(
                    &format!("D{generation}"),
                    Some(&format!("g{generation}")),
                    vec![passage("le contrat", Some("contrat"))],
                )],
                json!([]),
            )
            .await;
            sender
                .send(())
                .await
                .expect("the reader of the test is still listening");
        }
    });

    let mut answers = 0_u64;
    while receiver.recv().await.is_some() {
        let answer = facets_ok(&router, body.clone()).await;
        answers += 1;
        let generation = answer["generation"]
            .as_u64()
            .unwrap_or_else(|| panic!("the answer carries its generation: {answer}"));
        assert!(
            generation <= ATTACHES,
            "a facet answer names a published generation: {answer}"
        );
        assert_eq!(
            unsigned(&answer, "passages"),
            generation,
            "the pool of the answer is the one of its generation, never a mix (INV-03): {answer}"
        );
        assert_eq!(
            unsigned(&answer, "documents"),
            generation,
            "the document count of the answer is the one of its generation (INV-03): {answer}"
        );
        let values = field_values(&answer, "numero");
        assert_eq!(
            values.len() as u64,
            generation,
            "the values of the answer are the ones of its generation, never a mix (INV-03): \
             {answer}"
        );
        assert!(
            values.iter().all(|(_, count)| *count == 1),
            "one passage per value in the fixture: {answer}"
        );
        assert_eq!(
            other(&answer, "numero"),
            0,
            "every passage of the pool carries a value in the fixture: {answer}"
        );
    }
    writer.await.expect("the writer task ends");
    assert_eq!(
        answers, ATTACHES,
        "the loop checked one facets answer per attach"
    );

    let last = facets_ok(&router, body).await;
    assert_eq!(last["generation"], json!(ATTACHES), "{last}");
    assert_eq!(unsigned(&last, "passages"), ATTACHES, "{last}");
    assert_eq!(
        field_values(&last, "numero").len() as u64,
        ATTACHES,
        "{last}"
    );
}

// --- INV-13 ---------------------------------------------------------------

/// Marks the filter value and the field value of the call.
///
/// The markers are fixed literals: no other test of this binary writes them, and
/// a failure names the file that holds one.
const FILTER_MARKER: &str = "facets-inv13-filter-marker";
const FIELD_MARKER: &str = "facets-inv13-field-marker";

/// INV-13 — the analytics and the journal hold neither the value of a filter nor
/// the value of a field.
///
/// A facet call carries a marker in its filter and returns the same marker in
/// `values`, so both halves are exercised: a value that only the answer may
/// carry, and a filter value that no file may carry. The operation appears in
/// `analytics.sqlite` under the closed name `facets`, counted once for the served
/// call and once under the code of the refused one (ADR-0118, decision 6;
/// ADR-0154, decision 9).
#[tokio::test]
async fn inv_13_facets_hold_no_field_value_nor_filter_value_in_the_journal_nor_in_the_analytics() {
    let sink = journal();
    let (instance, state, router) = instance_with_alpha().await;
    attach(
        &router,
        &state,
        INDEX,
        "inv-13-facets",
        vec![document(
            "D1",
            Some(FILTER_MARKER),
            vec![passage("le contrat", Some(FIELD_MARKER))],
        )],
        json!([]),
    )
    .await;

    let answer = facets_ok(
        &router,
        json!({
            "fields": ["numero", "texte"],
            "filter": { "numero": { "eq": FILTER_MARKER } },
        }),
    )
    .await;
    assert_eq!(
        field_values(&answer, "numero"),
        vec![(FILTER_MARKER.to_owned(), 1)],
        "the value of the filter is rendered in `values`, and nowhere else: {answer}"
    );
    assert_eq!(
        field_values(&answer, "texte"),
        vec![(FIELD_MARKER.to_owned(), 1)],
        "the value of the field is rendered in `values`, and nowhere else: {answer}"
    );

    let (status, refused) = facets(&router, json!({ "fields": ["inexistant"] })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(refused["error"]["code"], "invalid_request", "{refused}");

    state
        .flush_analytics()
        .await
        .expect("the facet events are flushed");

    let path = analytics_path(&instance);
    let rows = read_minute_events(&path).expect("the analytics file is readable");
    assert_eq!(
        count(&rows, INDEX, "facets", "ok"),
        1,
        "the served call is counted under the operation `facets`: {rows:?}"
    );
    assert_eq!(
        count(&rows, INDEX, "facets", "invalid_request"),
        1,
        "the refused call is counted under its code: {rows:?}"
    );

    let analytics = analytics_bytes(&path);
    assert!(
        analytics
            .iter()
            .any(|(_, bytes)| contains(bytes, b"facets")),
        "the analytics bytes hold the operation name, so the scan is not vacuous"
    );
    let journal_text = sink.contents();
    assert!(
        journal_text.contains("models and indexes loaded"),
        "the capture holds the journal of the server: {journal_text:?}"
    );
    for (name, bytes) in &analytics {
        for (kind, marker) in [
            ("filter value", FILTER_MARKER),
            ("field value", FIELD_MARKER),
        ] {
            assert!(
                !contains(bytes, marker.as_bytes()),
                "the analytics file {name} holds the {kind} {marker:?} (INV-13)"
            );
        }
    }
    for (kind, marker) in [
        ("filter value", FILTER_MARKER),
        ("field value", FIELD_MARKER),
    ] {
        assert!(
            !journal_text.contains(marker),
            "the journal holds the {kind} {marker:?} (INV-13)"
        );
    }
}

// --- criterion 1 ----------------------------------------------------------

/// Criterion 1 — `count.passages` is the pool of a search with the same filter,
/// and `live_passages` without filter.
///
/// The `pool_size` of a `lexical` search with `k: 1` is compared on the same
/// generation: the filter is a document field, which every passage of its
/// document inherits (`docs/03-recherche.md`).
#[tokio::test]
async fn the_facet_counts_equal_the_search_pool_and_the_live_passages_of_the_index() {
    let (_instance, state, router) = instance_with_alpha_and_lexical().await;
    attach(
        &router,
        &state,
        INDEX,
        "criterion-1",
        vec![
            document(
                "D1",
                Some("1240"),
                vec![
                    passage("le contrat", Some("contrat")),
                    passage("le delit", Some("autre")),
                ],
            ),
            document(
                "D2",
                Some("1241"),
                vec![passage("le contrat", Some("contrat"))],
            ),
        ],
        json!([]),
    )
    .await;

    // Without filter: the size of the live pool of the index.
    let (status, detail) = send(
        &router,
        Method::GET,
        &format!("/v1/indexes/{INDEX}"),
        READ_SECRET,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    let live = detail["state"]["live_passages"]
        .as_u64()
        .unwrap_or_else(|| panic!("the detail carries `live_passages`: {detail}"));
    let all = facets_ok(&router, json!({ "fields": ["numero"] })).await;
    assert_eq!(
        unsigned(&all, "passages"),
        live,
        "without filter, `count.passages` is `live_passages` (ADR-0154, decision 5): {all}"
    );

    // With a filter: the `pool_size` of a `lexical` search with `k: 1`, on the
    // same generation.
    let filtered = json!({ "numero": { "eq": "1240" } });
    let answer = facets_ok(
        &router,
        json!({ "fields": ["numero"], "filter": filtered.clone() }),
    )
    .await;
    let (status, search) = send(
        &router,
        Method::POST,
        &format!("/v1/indexes/{INDEX}/search"),
        READ_SECRET,
        Some(json!({
            "query": "contrat",
            "mode": "lexical",
            "k": 1,
            "filter": filtered,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the lexical search runs: {search}");
    assert_eq!(
        unsigned(&answer, "passages"),
        search["pool_size"]
            .as_u64()
            .unwrap_or_else(|| panic!("the search carries `pool_size`: {search}")),
        "`count.passages` is the pool of a search with the same filter (INV-07): {answer} vs \
         {search}"
    );
    assert_eq!(
        answer["generation"], search["generation"],
        "both answers come from the same generation: {answer} vs {search}"
    );
    assert_eq!(unsigned(&answer, "passages"), 2, "{answer}");
}

// --- criterion 2 ----------------------------------------------------------

/// Criterion 2 — `Σ count + other` is the number of pool passages carrying a
/// value, and `other` is `0` when everything fits in `limit`.
///
/// The fixture has one passage without `numero` and one without `texte`, so the
/// sum is strictly below `count.passages` for both fields: a passage without a
/// value enters neither `values` nor `other` (ADR-0154, decision 4).
#[tokio::test]
async fn the_value_counts_sum_to_the_pool_passages_carrying_a_value() {
    let (_instance, state, router) = instance_with_alpha().await;
    attach(
        &router,
        &state,
        INDEX,
        "criterion-2",
        vec![
            document(
                "D1",
                Some("1240"),
                vec![
                    passage("le contrat", Some("contrat")),
                    passage("le delit", Some("autre")),
                ],
            ),
            document(
                "D2",
                Some("1241"),
                vec![passage("le contrat", Some("contrat"))],
            ),
            document("D3", None, vec![passage("sans champ", None)]),
        ],
        json!([]),
    )
    .await;

    let answer = facets_ok(&router, json!({ "fields": ["numero", "texte"] })).await;
    let passages = unsigned(&answer, "passages");
    assert_eq!(passages, 4, "{answer}");
    let mut requested: Vec<&str> = answer["fields"]
        .as_object()
        .unwrap_or_else(|| panic!("the answer carries its fields: {answer}"))
        .keys()
        .map(String::as_str)
        .collect();
    requested.sort_unstable();
    assert_eq!(
        requested,
        vec!["numero", "texte"],
        "`fields` carries exactly the fields requested: {answer}"
    );

    // `limit` 20 by default: everything fits, so `other` is 0.
    assert_eq!(
        field_values(&answer, "numero"),
        vec![("1240".to_owned(), 2), ("1241".to_owned(), 1)],
        "{answer}"
    );
    assert_eq!(other(&answer, "numero"), 0, "{answer}");
    assert_eq!(
        field_values(&answer, "texte"),
        vec![("contrat".to_owned(), 2), ("autre".to_owned(), 1)],
        "{answer}"
    );
    assert_eq!(other(&answer, "texte"), 0, "{answer}");

    // `Σ count + other` is the number of pool passages carrying a value: 3 of
    // the 4 passages, for each field.
    for field in ["numero", "texte"] {
        let sum: u64 = field_values(&answer, field)
            .iter()
            .map(|(_, count)| count)
            .sum();
        assert_eq!(
            sum + other(&answer, field),
            3,
            "the passages of the pool carrying `{field}`: {answer}"
        );
        assert!(
            sum + other(&answer, field) < passages,
            "a passage without a value enters neither `values` nor `other`: {answer}"
        );
    }

    // `limit: 1` truncates the list, and the missing count lands in `other`.
    let truncated = facets_ok(
        &router,
        json!({ "fields": ["numero", "texte"], "limit": 1 }),
    )
    .await;
    assert_eq!(
        field_values(&truncated, "numero"),
        vec![("1240".to_owned(), 2)],
        "{truncated}"
    );
    assert_eq!(other(&truncated, "numero"), 1, "{truncated}");
    assert_eq!(
        field_values(&truncated, "texte"),
        vec![("contrat".to_owned(), 2)],
        "{truncated}"
    );
    assert_eq!(other(&truncated, "texte"), 1, "{truncated}");
    for field in ["numero", "texte"] {
        let sum: u64 = field_values(&truncated, field)
            .iter()
            .map(|(_, count)| count)
            .sum();
        assert_eq!(
            sum + other(&truncated, field),
            3,
            "`other` keeps the truncated part, so the sum is unchanged: {truncated}"
        );
    }
}

// --- criterion 3 ----------------------------------------------------------

/// Criterion 3 — `count` descending, then `value` ascending in UTF-8 bytes, and
/// two identical calls render the same answer.
///
/// The values `"10"` and `"9"` both count once: the byte order puts `"10"` before
/// `"9"`, where a numeric order would not. The two calls are compared member by
/// member, `request_id` aside: that member is the identifier of one request
/// (ADR-0098), not a content of the facet.
#[tokio::test]
async fn the_values_are_ordered_by_count_then_bytes_and_two_identical_calls_agree() {
    let (_instance, state, router) = instance_with_alpha().await;
    attach(
        &router,
        &state,
        INDEX,
        "criterion-3",
        vec![
            document(
                "D1",
                Some("1240"),
                vec![
                    passage("le contrat", Some("contrat")),
                    passage("le delit", Some("autre")),
                ],
            ),
            document(
                "D2",
                Some("10"),
                vec![passage("le contrat", Some("contrat"))],
            ),
            document(
                "D3",
                Some("9"),
                vec![passage("le contrat", Some("contrat"))],
            ),
        ],
        json!([]),
    )
    .await;

    let body = json!({ "fields": ["numero"] });
    let first = facets_ok(&router, body.clone()).await;
    assert_eq!(
        field_values(&first, "numero"),
        vec![
            ("1240".to_owned(), 2),
            ("10".to_owned(), 1),
            ("9".to_owned(), 1),
        ],
        "`count` descending, then `value` ascending in bytes: {first}"
    );

    let second = facets_ok(&router, body).await;
    assert_eq!(
        without_request_id(&first),
        without_request_id(&second),
        "two identical calls render the same answer: {first} vs {second}"
    );
}

/// The answer without its per-request identifier.
fn without_request_id(answer: &Value) -> Value {
    let mut answer = answer.clone();
    if let Some(object) = answer.as_object_mut() {
        object.remove("request_id");
    }
    answer
}

// --- criterion 4 ----------------------------------------------------------

/// Criterion 4 — the domains of `limit` and of `fields`, and the closed body.
///
/// The index declares eleven `keyword` fields, so a refusal of eleven names
/// cannot be the refusal of an unknown field: ten of them are accepted by the
/// control below.
#[tokio::test]
async fn the_limit_domain_is_one_to_one_hundred_and_fields_carry_one_to_ten_distinct_names() {
    let mut schema = serde_json::Map::new();
    for number in 1..=11 {
        schema.insert(format!("f{number}"), json!({ "type": "keyword" }));
    }
    let mut config = index_config();
    config["name"] = json!(INDEX);
    config["schema"]["document_fields"] = Value::Object(schema);
    let (_instance, _state, router) = instance_with(config).await;

    for limit in [1_u64, 20, 100] {
        let answer = facets_ok(&router, json!({ "fields": ["f1"], "limit": limit })).await;
        assert_eq!(unsigned(&answer, "passages"), 0, "{answer}");
    }
    for limit in [0_u64, 101] {
        let (status, answer) = facets(&router, json!({ "fields": ["f1"], "limit": limit })).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "`limit: {limit}` is outside 1 to 100: {answer}"
        );
        invalid_request(&answer);
    }

    // `fields`: absent, empty, duplicated, non-textual, and eleven names.
    let all_eleven: Vec<String> = (1..=11).map(|number| format!("f{number}")).collect();
    let ten: Vec<String> = all_eleven[..10].to_vec();
    for (label, body) in [
        ("absent", json!({ "limit": 20 })),
        ("empty", json!({ "fields": [] })),
        ("duplicated", json!({ "fields": ["f1", "f1"] })),
        ("not an array", json!({ "fields": "f1" })),
        ("not text", json!({ "fields": [1] })),
        ("eleven names", json!({ "fields": all_eleven })),
    ] {
        let (status, answer) = facets(&router, body).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "`fields` {label} is refused: {answer}"
        );
        invalid_request(&answer);
    }

    // Positive control: the same ten names are accepted, so the refusal above is
    // the one of the count of names.
    let accepted = facets_ok(&router, json!({ "fields": ten })).await;
    assert_eq!(
        accepted["fields"]
            .as_object()
            .map(serde_json::Map::len)
            .unwrap_or_default(),
        10,
        "ten distinct names are accepted: {accepted}"
    );

    // The body is closed: a member of the search body is an unknown key.
    for unknown in ["query", "vector", "mode", "k", "return"] {
        let (status, answer) = facets(&router, json!({ "fields": ["f1"], unknown: "x" })).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "the member `{unknown}` is an unknown key of the closed body: {answer}"
        );
        invalid_request(&answer);
    }
    // And a documented member is accepted, so the refusal above is the one of the
    // key and not of the body.
    facets_ok(
        &router,
        json!({ "fields": ["f1"], "filter": { "f1": { "eq": "x" } }, "timeout_ms": 2_000 }),
    )
    .await;
}

// --- criterion 5 ----------------------------------------------------------

/// Criterion 5 — a field of another type, an unknown field and a non-filterable
/// field are `400 invalid_request` with `details: {}`.
///
/// The message names the field and, when its type is known, its type (ADR-0154,
/// decision 3). The index declares one field per documented type, so each
/// refusal is the one of the type and not the one of an unknown name.
#[tokio::test]
async fn a_field_of_another_type_an_unknown_field_or_a_non_filterable_field_is_invalid_request() {
    let mut config = index_config();
    config["name"] = json!(INDEX);
    config["schema"]["document_fields"] = json!({
        "fonds": { "type": "keyword" },
        "annee": { "type": "integer" },
        "poids": { "type": "float" },
        "jour": { "type": "date" },
        "publie": { "type": "bool" },
        "titre": { "type": "keyword", "filterable": false },
    });
    let (_instance, _state, router) = instance_with(config).await;

    for (field, kind) in [
        ("annee", "integer"),
        ("poids", "float"),
        ("jour", "date"),
        ("publie", "bool"),
    ] {
        let (status, answer) = facets(&router, json!({ "fields": [field] })).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "`{field}` is a `{kind}` field, outside the facets of 1.1: {answer}"
        );
        let message = invalid_request(&answer);
        assert!(
            message.contains(field),
            "the message names the field `{field}`: {message}"
        );
        assert!(
            message.contains(kind),
            "the message names the type `{kind}` of `{field}`: {message}"
        );
    }

    for (label, field) in [
        ("an unknown field", "inexistant"),
        ("a non-filterable field", "titre"),
    ] {
        let (status, answer) = facets(&router, json!({ "fields": [field] })).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{label} `{field}` is refused: {answer}"
        );
        let message = invalid_request(&answer);
        assert!(
            message.contains(field),
            "the message names the faulty field `{field}`: {message}"
        );
    }

    // Positive control: the `keyword` filterable field of the same schema is
    // accepted, so the refusals above are the ones of the field.
    facets_ok(&router, json!({ "fields": ["fonds"] })).await;
}

// --- criterion 6 ----------------------------------------------------------

/// Criterion 6 — the codes of the API on the facets route: `invalid_filter`
/// with its path, `unknown_index`, and `unauthorized` without a key.
#[tokio::test]
async fn a_refused_filter_an_unknown_index_and_a_missing_key_keep_the_codes_of_the_api() {
    let (_instance, _state, router) = instance_with_alpha().await;

    let (status, answer) = facets(
        &router,
        json!({
            "fields": ["numero"],
            "filter": { "numero": { "regex": "1240" } },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    assert_eq!(
        answer["error"]["code"], "invalid_filter",
        "a refused grammar is a filter error, never an invalid request: {answer}"
    );
    assert_eq!(
        answer["error"]["details"],
        json!({ "path": "$.filter.numero" }),
        "the details carry the path of the faulty filter and nothing else: {answer}"
    );

    let (status, answer) = facets_in(
        &router,
        "absent",
        READ_SECRET,
        json!({ "fields": ["numero"] }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{answer}");
    assert_eq!(answer["error"]["code"], "unknown_index", "{answer}");

    // No `Authorization` header at all: the refusal is `401 unauthorized`
    // (ADR-0100), and it comes before any reading of the registry.
    let request = axum::http::Request::builder()
        .method(Method::POST)
        .uri(format!("/v1/indexes/{INDEX}/facets"))
        .header("x-request-id", common::REQUEST_ID)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            json!({ "fields": ["numero"] }).to_string(),
        ))
        .expect("the request is valid");
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("the router answers");
    let status = response.status();
    let headers: HeaderMap = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("the body is readable");
    let answer: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{answer}");
    assert_eq!(answer["error"]["code"], "unauthorized", "{answer}");
    assert!(
        headers.contains_key(axum::http::header::WWW_AUTHENTICATE),
        "a `401` carries `WWW-Authenticate: Bearer` (ADR-0100)"
    );
    assert!(
        headers.get(AUTHORIZATION).is_none(),
        "the answer carries no credential back"
    );
}

// --- criterion 7 ----------------------------------------------------------

/// Criterion 7 — an empty pool is `200`, with zero counts, `values: []` and
/// `other: 0`; it is never an error (ADR-0154, decision 4).
#[tokio::test]
async fn an_empty_pool_answers_two_hundred_with_zero_counts_and_an_empty_values_list() {
    let (_instance, state, router) = instance_with_alpha().await;
    attach(
        &router,
        &state,
        INDEX,
        "criterion-7",
        vec![document(
            "D1",
            Some("1240"),
            vec![passage("le contrat", Some("contrat"))],
        )],
        json!([]),
    )
    .await;

    let answer = facets_ok(
        &router,
        json!({
            "fields": ["numero", "texte"],
            "filter": { "numero": { "eq": "aucun-document" } },
        }),
    )
    .await;
    assert_eq!(unsigned(&answer, "passages"), 0, "{answer}");
    assert_eq!(unsigned(&answer, "documents"), 0, "{answer}");
    for field in ["numero", "texte"] {
        assert_eq!(field_values(&answer, field), vec![], "{answer}");
        assert_eq!(other(&answer, field), 0, "{answer}");
    }
}

// --- criterion 11 ---------------------------------------------------------

/// Fixed message of the `503` of ADR-0146, decision 1.
const STARTING_MESSAGE: &str = "the server is starting";

/// Identifier pinned on the request of the startup test (ADR-0098).
const STARTUP_REQUEST_ID: &str = "facets-startup-1";

/// Secret of the `read` key of the startup test.
const STARTUP_READ_SECRET: &str = "read-secret-of-the-facets-startup-test";

/// A router whose loading has published neither the registry nor the state.
///
/// No barrier is armed, as in `startup_loading.rs`: `AppState::new()` then the
/// authenticator is exactly the state of a real loading between the bind and the
/// publication (ADR-0146, decision 8).
fn unloaded_router() -> Router {
    let state = Arc::new(AppState::new());
    let auth = Auth::new(&[ApiKey {
        name: "reader".to_owned(),
        role: Role::Read,
        sha256: sha256_hex(STARTUP_READ_SECRET),
    }])
    .unwrap_or_else(|error| panic!("the key is valid: {}", error.message()));
    state
        .set_auth(Arc::new(auth))
        .unwrap_or_else(|error| panic!("the authenticator is published: {}", error.message()));
    build_router(state)
}

/// Criterion 11 — before the registry is published, the route answers
/// `503 overloaded` with the fixed message, never `404` or `405`.
#[tokio::test]
async fn the_startup_refuses_the_facets_route_with_the_fixed_overloaded_message() {
    let router = unloaded_router();
    let request = axum::http::Request::builder()
        .method(Method::POST)
        .uri(format!("/v1/indexes/{INDEX}/facets"))
        .header("x-request-id", STARTUP_REQUEST_ID)
        .header(AUTHORIZATION, format!("Bearer {STARTUP_READ_SECRET}"))
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            json!({ "fields": ["numero"] }).to_string(),
        ))
        .expect("the request is valid");
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("the router answers");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("the body is readable");
    let answer: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    assert_ne!(
        status,
        StatusCode::NOT_FOUND,
        "the route exists before the publication: {answer}"
    );
    assert_ne!(
        status,
        StatusCode::METHOD_NOT_ALLOWED,
        "the route mounts `POST` before the publication: {answer}"
    );
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "the route refuses while the registry is not loaded: {answer}"
    );
    assert_eq!(answer["error"]["code"], "overloaded", "{answer}");
    assert_eq!(
        answer["error"]["message"], STARTING_MESSAGE,
        "the message is the fixed one of ADR-0146: {answer}"
    );
    assert_eq!(
        answer["error"]["details"],
        json!({}),
        "no value is added to `details` (ADR-0101): {answer}"
    );
    assert_eq!(
        answer["error"]["request_id"], STARTUP_REQUEST_ID,
        "the body echoes the request identifier (ADR-0098): {answer}"
    );
    assert_eq!(
        headers.get(REQUEST_ID_HEADER).and_then(|v| v.to_str().ok()),
        Some(STARTUP_REQUEST_ID),
        "the answer carries `x-request-id` (ADR-0098)"
    );
    assert_eq!(
        headers.get(RETRY_AFTER).and_then(|v| v.to_str().ok()),
        Some("1"),
        "the `503` of ADR-0146 carries `Retry-After: 1`"
    );
    assert!(
        answer.get("generation").is_none() && answer.get("count").is_none(),
        "no facet answer is rendered from an unloaded registry: {answer}"
    );
}

// --- criterion 12 ---------------------------------------------------------

/// Criterion 12 — `count.documents` is exact across the passages of one
/// document, for a document field and a passage field, and after an attach that
/// removes and adds documents.
///
/// The first batch: `D1` has three passages, `D2` one and `D3` two. The second
/// batch deletes `D3` and upserts `D1` with a single passage, and adds `D4` with
/// two: the count of documents follows the pool, not the count of passages, and
/// it is not the count of lines of the `documents` table of the index.
#[tokio::test]
async fn count_documents_is_exact_across_the_passages_of_a_document_and_after_an_attach() {
    let (_instance, state, router) = instance_with_alpha().await;
    attach(
        &router,
        &state,
        INDEX,
        "criterion-12-first",
        vec![
            document(
                "D1",
                Some("1240"),
                vec![
                    passage("le contrat", Some("contrat")),
                    passage("le contrat encore", Some("contrat")),
                    passage("le delit", Some("autre")),
                ],
            ),
            document(
                "D2",
                Some("1241"),
                vec![passage("le contrat", Some("contrat"))],
            ),
            document(
                "D3",
                Some("1242"),
                vec![
                    passage("le delit", Some("autre")),
                    passage("la peine", Some("autre")),
                ],
            ),
        ],
        json!([]),
    )
    .await;

    let all = facets_ok(&router, json!({ "fields": ["numero", "texte"] })).await;
    assert_eq!(unsigned(&all, "passages"), 6, "{all}");
    assert_eq!(
        unsigned(&all, "documents"),
        3,
        "three documents carry the six passages: {all}"
    );

    // A document field: every passage of `D1` carries `numero` `1240`.
    let by_document_field = facets_ok(
        &router,
        json!({
            "fields": ["numero"],
            "filter": { "numero": { "eq": "1240" } },
        }),
    )
    .await;
    assert_eq!(
        unsigned(&by_document_field, "passages"),
        3,
        "{by_document_field}"
    );
    assert_eq!(
        unsigned(&by_document_field, "documents"),
        1,
        "the three passages of `D1` are one document: {by_document_field}"
    );

    // A passage field: `contrat` covers `D1` twice and `D2` once, so three
    // passages and two documents.
    let by_passage_field = facets_ok(
        &router,
        json!({
            "fields": ["texte"],
            "filter": { "texte": { "eq": "contrat" } },
        }),
    )
    .await;
    assert_eq!(
        unsigned(&by_passage_field, "passages"),
        3,
        "{by_passage_field}"
    );
    assert_eq!(
        unsigned(&by_passage_field, "documents"),
        2,
        "the passages of `D1` and `D2` are two documents: {by_passage_field}"
    );

    // One attach removes `D3` and adds `D4`, and `D1` keeps a single passage.
    attach(
        &router,
        &state,
        INDEX,
        "criterion-12-second",
        vec![
            document(
                "D1",
                Some("1240"),
                vec![passage("le contrat", Some("contrat"))],
            ),
            document(
                "D4",
                Some("1243"),
                vec![
                    passage("le delit", Some("autre")),
                    passage("la peine", Some("autre")),
                ],
            ),
        ],
        json!(["D3"]),
    )
    .await;

    let after = facets_ok(&router, json!({ "fields": ["numero"] })).await;
    assert_eq!(
        unsigned(&after, "passages"),
        4,
        "`D1`, `D2` and `D4` carry four live passages: {after}"
    );
    assert_eq!(
        unsigned(&after, "documents"),
        3,
        "`D3` is gone, `D4` is added, and `D1` keeps one document: {after}"
    );
    assert_eq!(
        field_values(&after, "numero"),
        vec![
            ("1243".to_owned(), 2),
            ("1240".to_owned(), 1),
            ("1241".to_owned(), 1)
        ],
        "`count` descending, then the value in bytes: {after}"
    );
}

// --- analytics helpers ----------------------------------------------------

/// Path of the analytics file of the instance.
fn analytics_path(instance: &Instance) -> PathBuf {
    instance.root().join("data").join(ANALYTICS_FILE_NAME)
}

/// Raw bytes of the analytics file, and of the write-ahead log and the shared
/// memory SQLite may keep beside it when they exist.
fn analytics_bytes(path: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    for suffix in ["", "-wal", "-shm"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        let file = PathBuf::from(name);
        match std::fs::read(&file) {
            Ok(bytes) => files.push((file.display().to_string(), bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("the analytics file {} is readable: {error}", file.display()),
        }
    }
    files
}

/// Whether `haystack` holds `needle`, byte for byte.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// Sum of the `count` of the rows of one key.
fn count(rows: &[MinuteEvent], index: &str, operation: &str, status: &str) -> u64 {
    rows.iter()
        .filter(|row| row.index == index && row.operation == operation && row.status == status)
        .map(|row| row.count)
        .sum()
}
