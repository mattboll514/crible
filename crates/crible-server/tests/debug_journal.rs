// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and every helper of this file is a fixture or an assertion.
#![allow(clippy::expect_used, clippy::panic)]

//! `JOURNAL-DE-DEBUG` (phase P7; ADR-0121, ADR-0122).
//!
//! The acceptance criteria of the card that this file proves, and the decisions
//! behind them:
//!
//! | Test | Criterion or decision |
//! |---|---|
//! | `a_disabled_journal_writes_no_file` | "désactivé par défaut : aucun fichier écrit" (ADR-0121, decision 1) |
//! | `the_queries_route_answers_an_empty_list_when_the_journal_is_disabled` | `200` with `enabled: false` and `queries: []` (ADR-0122, decision 9) |
//! | `read_and_write_keys_receive_403_on_the_queries_route` | "un rôle `read` reçoit `403`" (ADR-0095; ADR-0122, decision 9) |
//! | `sample_rate_zero_records_only_the_search_that_asks_debug` | the two ways of ADR-0122, decision 1 |
//! | `sample_rate_one_records_a_search_without_debug` | `sample_rate` alone, and no `debug` member (ADR-0122, decisions 1 and 3) |
//! | `the_payload_holds_the_query_text_and_the_filter_as_received` | payload members (ADR-0121, decision 4) |
//! | `a_multi_index_search_writes_one_row_with_targets` | one row, `index` `NULL`, `targets[]` (ADR-0122, decision 4) |
//! | `a_refused_body_is_recorded_with_its_code` | an error after the key is recorded (ADR-0122, decision 5) |
//! | `the_route_filters_by_index_total_ms_and_dates` | the filters of ADR-0122, decision 9 |
//! | `the_route_paginates_by_before_id` | keyset pagination on `id` (ADR-0122, decision 9) |
//! | `an_invalid_parameter_is_a_400_naming_it` | `400 invalid_request` naming the parameter (ADR-0122, decision 9) |
//! | `the_route_renders_the_payload_as_an_object` | the payload is an object, `created_at` is RFC 3339 (ADR-0121, decision 3) |
//! | `the_purge_removes_the_entries_older_than_the_ttl` | "la purge supprime bien les entrées expirées" (ADR-0122, decision 8) |
//!
//! INV-17 itself is proved by `tests/inv_17_debug_journal_never_blocks.rs`, under
//! the `failpoints` feature (ADR-0122, decision 11): this file never arms a
//! barrier, so it runs with a plain `cargo test`.
//!
//! The assertions on `dropped` never name an absolute value: the counter is
//! process-wide (ADR-0122, decision 6) and the tests of one binary share it, so
//! only its type and its presence are checked here, and its exact growth is
//! proved by the `inv_17_*` tests, which run one at a time.
//!
//! ```text
//! cargo test -p crible-server --test debug_journal
//! ```

mod common;

use std::sync::Arc;

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::http::build_router;
use crible_server::state::AppState;
use crible_store::debug_journal::{
    DEBUG_JOURNAL_FILE_NAME, DebugJournalStore, DebugQuery, DebugQueryRow, JournalCap, read_queries,
};
use serde_json::{Value, json};

use common::{
    ADMIN_SECRET, DOC_ID, INDEX, Instance, Journal, READ_SECRET, REQUEST_ID, WRITE_SECRET,
    base64_vector, instance_with_alpha, instance_with_alpha_and_journal, send, terminal_job,
};

/// Path of the search route of the fixture index.
const SEARCH: &str = "/v1/indexes/alpha/search";

/// Path of the multi-index search route.
const MULTI: &str = "/v1/search";

/// Path of the journal route.
const QUERIES: &str = "/v1/debug/queries";

/// Vector of the fixture document, and of every query of this file.
const VECTOR: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

/// Text of the fixture passage.
const TEXT: &str = "le contrat fait la loi";

/// Value of the document field `numero`, and of the filter of the searches.
const FILTER_MARKER: &str = "marqueur-de-valeur-de-filtre-2f1c7d";

/// Path of the journal file of `instance` (`docs/02:60-62`).
fn journal_path(instance: &Instance) -> std::path::PathBuf {
    instance.root().join("data").join(DEBUG_JOURNAL_FILE_NAME)
}

/// Every row of the journal file of `instance`, in `id` order.
fn rows(instance: &Instance) -> Vec<DebugQuery> {
    read_queries(&journal_path(instance)).expect("the debug journal file is readable")
}

/// Used size of the journal file, read on a second connection with the pragmas
/// of ADR-0127, decision 2: `(page_count - freelist_count) × page_size`.
///
/// This is the independent measurement the route is compared to; it shares no
/// code with the store, so a wrong formula on either side shows up.
fn used_bytes(instance: &Instance) -> u64 {
    let connection =
        rusqlite::Connection::open(journal_path(instance)).expect("the journal file opens");
    let pragma = |name: &str| -> i64 {
        connection
            .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
            .unwrap_or_else(|error| panic!("PRAGMA {name} is readable: {error}"))
    };
    let used = pragma("page_count") - pragma("freelist_count");
    let page_size = pragma("page_size");
    u64::try_from(used).expect("the used page count is positive")
        * u64::try_from(page_size).expect("the page size is positive")
}

/// Flushes the journal and reads its rows.
async fn flushed_rows(instance: &Instance, state: &Arc<AppState>) -> Vec<DebugQuery> {
    state
        .flush_debug_journal()
        .await
        .expect("the debug journal is flushed");
    rows(instance)
}

/// The payload of one row, parsed.
fn payload_of(row: &DebugQuery) -> Value {
    serde_json::from_str(&row.payload).expect("the payload of a row is JSON text")
}

/// Sends one dense search with `debug` when `debug` is `true`, and returns the
/// answer.
async fn dense_search(router: &Router, debug: bool) -> (StatusCode, Value) {
    let mut body = json!({
        "vector": base64_vector(VECTOR),
        "mode": "dense",
        "k": 10,
        "filter": { "numero": { "eq": FILTER_MARKER } },
    });
    if debug {
        body["debug"] = json!(true);
    }
    send(router, Method::POST, SEARCH, READ_SECRET, Some(body)).await
}

/// Sends one dense search with `debug` and asserts it answers `200`.
async fn debug_search(router: &Router) -> Value {
    let (status, body) = dense_search(router, true).await;
    assert_eq!(status, StatusCode::OK, "the search answers: {body}");
    body
}

/// Body of an online batch that upserts `doc_id`, with one passage.
fn batch(batch_id: &str, doc_id: &str, numero: &str, text: &str) -> Value {
    json!({
        "batch_id": batch_id,
        "mode": "upsert",
        "documents": [{
            "doc_id": doc_id,
            "fields": { "numero": numero },
            "passages": [{
                "text": text,
                "fields": { "texte": "contrat" },
                "vector": base64_vector(VECTOR),
            }]
        }]
    })
}

/// Attaches `body` to `index` and waits for the job.
async fn attach(router: &Router, state: &Arc<AppState>, index: &str, body: Value) {
    let path = format!("/v1/indexes/{index}/documents?wait=true");
    let (status, accepted) = send(router, Method::POST, &path, WRITE_SECRET, Some(body)).await;
    assert!(
        status == StatusCode::OK || status == StatusCode::ACCEPTED,
        "the batch of {index} is accepted: {accepted}"
    );
    let job = terminal_job(router, state, &accepted).await;
    assert_eq!(job["state"], "succeeded", "the batch attaches: {job}");
}

/// Creates the index `body` names.
async fn create_index(router: &Router, body: Value) {
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

/// An instance with the `[debug_journal]` section of `journal`, one index
/// `alpha` and one document.
async fn instance_with_alpha_journal(journal: Journal) -> (Instance, Arc<AppState>, Router) {
    let (instance, state, router) = instance_with_alpha_and_journal(journal).await;
    let body = batch("batch-fixture", DOC_ID, FILTER_MARKER, TEXT);
    let path = format!("/v1/indexes/{INDEX}/documents?wait=true");
    let (status, accepted) = send(&router, Method::POST, &path, WRITE_SECRET, Some(body)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the fixture batch is accepted: {accepted}"
    );
    let job = terminal_job(&router, &state, &accepted).await;
    assert_eq!(
        job["state"], "succeeded",
        "the fixture batch attaches: {job}"
    );
    (instance, state, router)
}

/// An instance with two indexes `alpha` and `beta`, one document each, and the
/// `[debug_journal]` section of `journal`.
async fn instance_with_two_indexes(journal: Journal) -> (Instance, Arc<AppState>, Router) {
    let instance = Instance::with_journal(journal);
    let state = instance.state();
    let router = build_router(Arc::clone(&state));
    for name in ["alpha", "beta"] {
        let mut body = common::index_config();
        body["name"] = json!(name);
        create_index(&router, body).await;
    }
    for (name, doc_id) in [("alpha", "ALPHA-1"), ("beta", "BETA-1")] {
        let body = batch(&format!("batch-{name}"), doc_id, FILTER_MARKER, TEXT);
        attach(&router, &state, name, body).await;
    }
    (instance, state, router)
}

/// `GET /v1/debug/queries` with `query`, and the given key.
async fn queries(router: &Router, query: &str, secret: &str) -> (StatusCode, Value) {
    let path = if query.is_empty() {
        QUERIES.to_owned()
    } else {
        format!("{QUERIES}?{query}")
    };
    send(router, Method::GET, &path, secret, None).await
}

/// `GET /v1/debug/queries` with the `admin` key, asserting `200`.
async fn admin_queries(router: &Router, query: &str) -> Value {
    let (status, body) = queries(router, query, ADMIN_SECRET).await;
    assert_eq!(status, StatusCode::OK, "the journal route answers: {body}");
    body
}

/// The `id`s of the rows of an answer, in its order.
fn ids(body: &Value) -> Vec<i64> {
    body["queries"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer holds a list of queries: {body}"))
        .iter()
        .map(|row| {
            row["id"]
                .as_i64()
                .unwrap_or_else(|| panic!("a row carries its id: {row}"))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Activation: disabled by default, and the two ways of ADR-0122, decision 1.
// ---------------------------------------------------------------------------

/// The journal is disabled by default, and a disabled journal writes **no**
/// file, even when searches ask `debug: true` (ADR-0121, decision 1).
///
/// The file is neither opened nor created: a `debug` search with the journal
/// off is a normal search, and a forced flush has nothing to write.
#[tokio::test]
async fn a_disabled_journal_writes_no_file() {
    let (instance, state, router) = instance_with_alpha().await;

    for _ in 0..2 {
        debug_search(&router).await;
    }
    state
        .flush_debug_journal()
        .await
        .expect("a flush of a disabled journal is a no-op");

    let path = journal_path(&instance);
    assert!(
        !path.exists(),
        "a disabled journal writes no file: {} exists (ADR-0121, decision 1)",
        path.display()
    );
    let wal = path.with_file_name(format!("{DEBUG_JOURNAL_FILE_NAME}-wal"));
    assert!(
        !wal.exists(),
        "a disabled journal opens no write-ahead log: {} exists",
        wal.display()
    );
}

/// With the journal disabled, the route answers `200` with `enabled: false` and
/// an empty list, without opening the file (ADR-0122, decision 9).
///
/// `dropped` is checked for its presence and its type only: the counter is
/// process-wide, and another test of this binary may have incremented it.
#[tokio::test]
async fn the_queries_route_answers_an_empty_list_when_the_journal_is_disabled() {
    let (_instance, _state, router) = instance_with_alpha().await;
    let body = admin_queries(&router, "").await;
    assert_eq!(body["enabled"], false, "{body}");
    assert_eq!(body["queries"], json!([]), "{body}");
    assert!(
        body["dropped"].is_u64(),
        "`dropped` is the counter of the process (ADR-0122, decision 6): {body}"
    );
}

/// A `read` and a `write` key receive `403 forbidden` on the journal route; an
/// `admin` key reads it (ADR-0095; ADR-0122, decision 9).
///
/// The check runs with the journal disabled, which is the state of an instance
/// that never enabled it: the role is read before the handler, so no file is
/// opened to refuse a key.
#[tokio::test]
async fn read_and_write_keys_receive_403_on_the_queries_route() {
    let (_instance, _state, router) = instance_with_alpha().await;
    for (role, secret) in [("read", READ_SECRET), ("write", WRITE_SECRET)] {
        let (status, body) = queries(&router, "", secret).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a {role} key does not read the journal (ADR-0095): {body}"
        );
        assert_eq!(body["error"]["code"], "forbidden", "{body}");
    }
    let (status, body) = queries(&router, "", ADMIN_SECRET).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an admin key reads the journal: {body}"
    );
}

/// With the journal enabled and `sample_rate = 0`, a search that asks `debug`
/// writes one row and a search that does not ask it writes none
/// (ADR-0122, decision 1).
///
/// With `sample_rate = 0` at a journal that is on, the "always recorded when
/// `debug`" way is the only way that records: a `debug` search that were sent
/// through the draw would be lost, which is exactly what decision 1 forbids.
#[tokio::test]
async fn sample_rate_zero_records_only_the_search_that_asks_debug() {
    let (instance, state, router) = instance_with_alpha_journal(Journal::enabled(0.0)).await;

    debug_search(&router).await;
    let rows = flushed_rows(&instance, &state).await;
    assert_eq!(
        rows.len(),
        1,
        "a `debug` search is recorded at `sample_rate = 0` (ADR-0122, decision 1)"
    );
    let row = &rows[0];
    assert_eq!(row.index.as_deref(), Some(INDEX), "{row:?}");
    assert_eq!(
        row.client, "reader",
        "the client is the name of the key: {row:?}"
    );
    assert_eq!(row.status, "ok", "{row:?}");
    assert_eq!(row.mode, "dense", "{row:?}");
    assert_eq!(
        row.request_id, REQUEST_ID,
        "the row carries the request id: {row:?}"
    );
    assert!(
        row.total_ms.is_some(),
        "a finished search has a total: {row:?}"
    );

    let (status, answer) = dense_search(&router, false).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert!(
        answer.get("debug").is_none(),
        "a search without `debug` renders no `debug` member: {answer}"
    );
    let rows = flushed_rows(&instance, &state).await;
    assert_eq!(
        rows.len(),
        1,
        "a search without `debug` is not recorded at `sample_rate = 0` (ADR-0122, decision 1)"
    );
}

/// With the journal enabled and `sample_rate = 1`, a search that does not ask
/// `debug` is recorded, and its answer still carries no `debug` member
/// (ADR-0122, decisions 1 and 3).
///
/// The capture and the plan are built for the journal, and the answer stays the
/// one of a search that asked nothing: `debug` never travels outside the
/// request that asked it (ADR-0119, decision 10).
#[tokio::test]
async fn sample_rate_one_records_a_search_without_debug() {
    let (instance, state, router) = instance_with_alpha_journal(Journal::enabled(1.0)).await;

    let (status, answer) = dense_search(&router, false).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert!(
        answer.get("debug").is_none(),
        "a sampled search renders no `debug` member (ADR-0119, decision 10): {answer}"
    );

    let rows = flushed_rows(&instance, &state).await;
    assert_eq!(
        rows.len(),
        1,
        "a search is sampled at `sample_rate = 1` (ADR-0122, decision 1)"
    );
    let payload = payload_of(&rows[0]);
    assert!(
        payload["capture"].is_object(),
        "a sampled search builds its capture for the journal (ADR-0122, decision 3): {payload}"
    );
}

// ---------------------------------------------------------------------------
// Payload: the closed list of ADR-0121, decision 4.
// ---------------------------------------------------------------------------

/// An instance whose index `alpha` has the lexical branch on, one document, and
/// the `[debug_journal]` section of `journal`.
async fn instance_with_lexical_journal(journal: Journal) -> (Instance, Arc<AppState>, Router) {
    let instance = Instance::with_journal(journal);
    let state = instance.state();
    let router = build_router(Arc::clone(&state));
    let mut config = common::index_config();
    config["lexical"]["enabled"] = json!(true);
    create_index(&router, config).await;
    let body = batch("batch-fixture", DOC_ID, FILTER_MARKER, TEXT);
    attach(&router, &state, INDEX, body).await;
    (instance, state, router)
}

/// The payload holds the query text and the filter **as received**, plus the
/// plan, the timings, the capture and the generation — and no internal id
/// (ADR-0121, decision 4; INV-01).
///
/// The search is lexical, so the text travels to the journal exactly as the
/// analytics never would keep it: the journal exists to find one query again
/// (`docs/06-observabilite.md`, "Pourquoi").
#[tokio::test]
async fn the_payload_holds_the_query_text_and_the_filter_as_received() {
    let (instance, state, router) = instance_with_lexical_journal(Journal::enabled(0.0)).await;
    let filter = json!({ "numero": { "eq": FILTER_MARKER } });
    let (status, answer) = send(
        &router,
        Method::POST,
        SEARCH,
        READ_SECRET,
        Some(json!({
            "query": "contrat",
            "mode": "lexical",
            "k": 10,
            "debug": true,
            "filter": filter.clone(),
            // ADR-0123, decision 8: `explain_ids` enters neither the payload nor
            // the capture, and the journal's member list stays closed.
            "explain_ids": [DOC_ID],
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the lexical search answers: {answer}"
    );

    let rows = flushed_rows(&instance, &state).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    let payload = payload_of(&rows[0]);
    assert_eq!(
        payload["request"]["query"], "contrat",
        "the payload keeps the text of the query (ADR-0121, decision 4): {payload}"
    );
    assert_eq!(
        payload["request"]["filter"], filter,
        "the payload keeps the filter as it was received: {payload}"
    );
    assert_eq!(payload["request"]["mode"], "lexical", "{payload}");
    assert_eq!(payload["request"]["k"], 10, "{payload}");
    for member in ["plan", "timings_ms", "capture", "generation"] {
        assert!(
            payload.get(member).is_some(),
            "the payload of a `debug` search carries `{member}`: {payload}"
        );
    }
    assert!(
        payload.get("explain").is_none() && payload["request"].get("explain_ids").is_none(),
        "`explain_ids` enters neither the payload nor its request (ADR-0123, decision 8): {payload}"
    );
    assert_eq!(
        payload["generation"], answer["generation"],
        "the generation of the payload is the one of the answer: {payload}"
    );
    let lexical = payload["capture"]["lexical"]
        .as_array()
        .unwrap_or_else(|| panic!("a lexical search has a lexical capture: {payload}"));
    assert!(
        !lexical.is_empty(),
        "the capture holds the fixture hit: {payload}"
    );
    for entry in lexical {
        let key = entry["key"]
            .as_str()
            .expect("a capture entry carries its key");
        assert!(
            key.starts_with(&format!("{DOC_ID}#")),
            "the capture names the external key: {key}"
        );
        assert!(
            entry["rank"].is_u64() && entry["score"].is_number(),
            "a capture entry carries its rank and its score (ADR-0119, decision 8): {entry}"
        );
    }
    common::assert_no_iid_key("the payload of a journal row", &payload);
}

/// A multi-index search writes **one** row, with `index` at `NULL` and one
/// entry per target in `payload.targets[]` (ADR-0122, decision 4).
///
/// The text and the filter of the request are written once, not once per target;
/// the filter of each target, its generation and its capture live in
/// `targets[]`, in the order of the request.
#[tokio::test]
async fn a_multi_index_search_writes_one_row_with_targets() {
    let (instance, state, router) = instance_with_two_indexes(Journal::enabled(0.0)).await;
    let (status, answer) = send(
        &router,
        Method::POST,
        MULTI,
        READ_SECRET,
        Some(json!({
            "vector": base64_vector(VECTOR),
            "mode": "dense",
            "k": 10,
            "debug": true,
            "targets": [
                { "index": "alpha", "filter": { "numero": { "eq": FILTER_MARKER } } },
                { "index": "beta", "filter": {} },
            ],
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the multi-index search answers: {answer}"
    );

    let rows = flushed_rows(&instance, &state).await;
    assert_eq!(
        rows.len(),
        1,
        "a multi-index search writes one row, not one per target (ADR-0122, decision 4): {rows:?}"
    );
    let row = &rows[0];
    assert_eq!(
        row.index, None,
        "the column says which route wrote the row, and a multi-index row has no index: {row:?}"
    );
    assert_eq!(row.status, "ok", "{row:?}");
    let payload = payload_of(row);
    assert!(
        payload["request"].get("filter").is_none(),
        "`request.filter` is absent in multi-index; each target carries its own: {payload}"
    );
    let targets = payload["targets"]
        .as_array()
        .unwrap_or_else(|| panic!("the payload carries `targets[]`: {payload}"));
    assert_eq!(targets.len(), 2, "{payload}");
    assert_eq!(targets[0]["index"], "alpha", "{payload}");
    assert_eq!(targets[1]["index"], "beta", "{payload}");
    assert_eq!(
        targets[0]["filter"],
        json!({ "numero": { "eq": FILTER_MARKER } }),
        "each target carries its own filter as received: {payload}"
    );
    assert_eq!(targets[1]["filter"], json!({}), "{payload}");
    for target in targets {
        for member in ["index", "generation", "capture"] {
            assert!(
                target.get(member).is_some(),
                "a target carries `{member}`: {target}"
            );
        }
    }
    common::assert_no_iid_key("the payload of a multi-index row", &payload);

    // The `index` filter of the route retains a multi-index row by membership in
    // its targets, and never on the `NULL` column (ADR-0122, decision 9).
    for name in ["alpha", "beta"] {
        let body = admin_queries(&router, &format!("index={name}")).await;
        assert_eq!(
            ids(&body).len(),
            1,
            "a multi-index row is retained when `{name}` is among its targets: {body}"
        );
    }
    let body = admin_queries(&router, "index=gamma").await;
    assert!(
        ids(&body).is_empty(),
        "a multi-index row is not retained for an index it does not target: {body}"
    );
}

/// An error raised after the key was resolved is recorded with its code, and
/// its payload holds only the code (ADR-0122, decision 5).
///
/// `sample_rate = 1` is what records a request whose body fails to parse: the
/// request cannot say `debug`, so only the draw can keep it. The code is the one
/// of `docs/05-api.md`, never the message.
#[tokio::test]
async fn a_refused_body_is_recorded_with_its_code() {
    let (instance, state, router) = instance_with_alpha_journal(Journal::enabled(1.0)).await;
    let (status, answer) = send(
        &router,
        Method::POST,
        SEARCH,
        READ_SECRET,
        Some(json!({ "debug": true, "mode": "dense", "inconnu": 1 })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    assert_eq!(answer["error"]["code"], "invalid_request", "{answer}");

    let rows = flushed_rows(&instance, &state).await;
    assert_eq!(
        rows.len(),
        1,
        "a refused request is a request tried (ADR-0122, decision 5): {rows:?}"
    );
    let row = &rows[0];
    assert_eq!(row.status, "invalid_request", "{row:?}");
    let payload = payload_of(row);
    assert_eq!(
        payload["error"],
        json!({ "code": "invalid_request" }),
        "`error` holds the code alone (ADR-0121, decision 4): {payload}"
    );
    common::assert_no_iid_key("the payload of a failed row", &payload);
}

// ---------------------------------------------------------------------------
// Route: filters, pagination and refusals of ADR-0122, decision 9.
// ---------------------------------------------------------------------------

/// The route filters by `index`, `min_total_ms`, `from` and `to`
/// (ADR-0122, decision 9).
///
/// The two rows are one search per index, so `index` proves the column filter;
/// a real `total_ms` is milliseconds of a fixture search, so the two bounds of
/// `min_total_ms` are `0` (kept) and an hour (excluded). The dates are the same
/// proof at the edge of the range: `from` before and `to` before the write.
#[tokio::test]
async fn the_route_filters_by_index_total_ms_and_dates() {
    let (instance, state, router) = instance_with_two_indexes(Journal::enabled(0.0)).await;
    for name in ["alpha", "beta"] {
        let path = format!("/v1/indexes/{name}/search");
        let (status, answer) = send(
            &router,
            Method::POST,
            &path,
            READ_SECRET,
            Some(json!({
                "vector": base64_vector(VECTOR),
                "mode": "dense",
                "k": 10,
                "debug": true,
            })),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the search of {name} answers: {answer}"
        );
    }
    let written = flushed_rows(&instance, &state).await;
    assert_eq!(written.len(), 2, "{written:?}");

    let alpha = admin_queries(&router, "index=alpha").await;
    assert_eq!(ids(&alpha).len(), 1, "{alpha}");
    assert_eq!(alpha["queries"][0]["index"], "alpha", "{alpha}");
    let beta = admin_queries(&router, "index=beta").await;
    assert_eq!(ids(&beta).len(), 1, "{beta}");
    assert_eq!(beta["queries"][0]["index"], "beta", "{beta}");

    let kept = admin_queries(&router, "min_total_ms=0").await;
    assert_eq!(
        ids(&kept).len(),
        2,
        "`min_total_ms` keeps a row whose total is above the bound: {kept}"
    );
    let excluded = admin_queries(&router, "min_total_ms=3600000").await;
    assert!(
        ids(&excluded).is_empty(),
        "`min_total_ms` excludes a row whose total is below the bound: {excluded}"
    );

    let inside = admin_queries(&router, "from=1970-01-01T00:00:00Z").await;
    assert_eq!(
        ids(&inside).len(),
        2,
        "`from` before the write keeps both rows: {inside}"
    );
    let before = admin_queries(&router, "to=1970-01-01T00:00:00Z").await;
    assert!(
        ids(&before).is_empty(),
        "`to` before the write excludes both rows: {before}"
    );
}

/// The route pages by keyset on `id`, never by `OFFSET`: the page is the `limit`
/// most recent rows, and `before_id` continues from the last `id` received
/// (ADR-0122, decision 9).
///
/// Three rows of the same flush may share `created_at`, so the total order
/// `created_at DESC, id DESC` rests on `id` here: the ids of a page strictly
/// decrease, and no row is repeated between two pages.
#[tokio::test]
async fn the_route_paginates_by_before_id() {
    let (instance, state, router) = instance_with_alpha_journal(Journal::enabled(0.0)).await;
    for _ in 0..3 {
        debug_search(&router).await;
    }
    let written = flushed_rows(&instance, &state).await;
    assert_eq!(written.len(), 3, "{written:?}");

    let first = admin_queries(&router, "limit=2").await;
    let first_ids = ids(&first);
    assert_eq!(first_ids.len(), 2, "`limit` bounds the page: {first}");
    assert!(
        first_ids[0] > first_ids[1],
        "the order is `created_at DESC, id DESC`: {first_ids:?}"
    );
    let newest = written.iter().map(|row| row.id).max().expect("a row");
    assert_eq!(
        first_ids[0], newest,
        "the page starts at the newest row: {first}"
    );

    let rest = admin_queries(&router, &format!("limit=2&before_id={}", first_ids[1])).await;
    let rest_ids = ids(&rest);
    assert_eq!(
        rest_ids.len(),
        1,
        "the second page holds the last row: {rest}"
    );
    assert!(
        rest_ids[0] < first_ids[1],
        "`before_id` keeps the rows of a strictly lower id: {rest_ids:?}"
    );
    assert!(
        !first_ids.contains(&rest_ids[0]),
        "no row is repeated between two pages: {first_ids:?} and {rest_ids:?}"
    );
}

/// A parameter outside its domain, an unparsable value, or an unknown parameter
/// renders `400 invalid_request`, and the message names the parameter
/// (ADR-0122, decision 9; ADR-0109).
#[tokio::test]
async fn an_invalid_parameter_is_a_400_naming_it() {
    // The journal is enabled, so the refusals cannot be confused with the
    // "disabled journal" answer of ADR-0122, decision 9: the parameters of a
    // route that reads the file are checked before it reads anything.
    let (_instance, _state, router) = instance_with_alpha_and_journal(Journal::enabled(0.0)).await;
    let cases = [
        ("limit=0", "limit"),
        ("limit=1001", "limit"),
        ("limit=abc", "limit"),
        ("before_id=-1", "before_id"),
        ("min_total_ms=-1", "min_total_ms"),
        ("min_total_ms=abc", "min_total_ms"),
        ("from=pas-une-date", "from"),
        ("to=2026-13-45T00:00:00Z", "to"),
        ("inconnu=1", "inconnu"),
    ];
    for (query, parameter) in cases {
        let (status, body) = queries(&router, query, ADMIN_SECRET).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "`{query}` is refused: {body}"
        );
        assert_eq!(
            body["error"]["code"], "invalid_request",
            "`{query}`: {body}"
        );
        let message = body["error"]["message"]
            .as_str()
            .unwrap_or_else(|| panic!("`{query}` is refused with a message: {body}"));
        assert!(
            message.contains(parameter),
            "the message of `{query}` names `{parameter}`: {message}"
        );
    }
}

// ---------------------------------------------------------------------------
// Purge (ADR-0122, decision 8).
// ---------------------------------------------------------------------------

/// A row of the journal, with the columns of ADR-0121, decision 3.
///
/// `created_at` is empty at deposit: SQLite fills it at the insertion. The
/// purge test sets it through the store hook
/// `DebugJournalStore::insert_dated_for_tests`, the only way to age a row
/// without a clock in the code (ADR-0121, decision 3).
#[cfg(feature = "failpoints")]
fn row(request_id: &str, payload: &str) -> DebugQueryRow {
    DebugQueryRow {
        created_at: String::new(),
        request_id: request_id.to_owned(),
        index: Some(INDEX.to_owned()),
        client: "reader".to_owned(),
        mode: "dense".to_owned(),
        status: "ok".to_owned(),
        total_ms: Some(1.0),
        payload: payload.to_owned(),
    }
}

/// The purge removes the entries older than `ttl_days`, and keeps the younger
/// ones (ADR-0122, decision 8): "la purge supprime bien les entrées expirées".
///
/// A row of the current write is younger than any positive `ttl_days`, and a row
/// dated in 2000 is older: the purge of `ttl_days = 1` removes exactly the one,
/// the file keeps the other, and the route no longer returns the expired row.
#[cfg(feature = "failpoints")]
#[tokio::test]
async fn the_purge_removes_the_entries_older_than_the_ttl() {
    let journal = Journal {
        enabled: true,
        sample_rate: 0.0,
        ttl_days: 1,
        ..Journal::default()
    };
    let (instance, state, router) = instance_with_alpha_journal(journal).await;
    debug_search(&router).await;
    let written = flushed_rows(&instance, &state).await;
    assert_eq!(written.len(), 1, "{written:?}");
    let fresh_id = written[0].id;

    let path = journal_path(&instance);
    let store = DebugJournalStore::open_or_create(&path).expect("the journal file opens");
    let expired = store
        .insert_dated_for_tests(
            &row("expired-request", r#"{"request":{"query":"expiree"}}"#),
            "2000-01-01T00:00:00.000Z",
        )
        .expect("the aged row is inserted");
    assert_eq!(
        rows(&instance).len(),
        2,
        "the aged row is in the file before the purge"
    );

    let purged = store.purge(journal.ttl_days).expect("the purge runs");
    assert_eq!(purged, 1, "the purge removes the expired row alone");

    let rows = rows(&instance);
    assert_eq!(
        rows.len(),
        1,
        "the purge kept the fresh row alone: {rows:?}"
    );
    assert_eq!(
        rows[0].id, fresh_id,
        "the fresh row is the one kept: {rows:?}"
    );
    assert!(
        rows.iter().all(|row| row.id != expired),
        "the expired row is gone: {rows:?}"
    );

    let body = admin_queries(&router, "").await;
    assert_eq!(
        ids(&body),
        vec![fresh_id],
        "the route no longer returns the expired row: {body}"
    );
}

/// The `payload` of every row of the route is a JSON **object**, never a string:
/// a client reads it, it does not parse it (ADR-0122, decision 9).
#[tokio::test]
async fn the_route_renders_the_payload_as_an_object() {
    let (instance, state, router) = instance_with_alpha_journal(Journal::enabled(0.0)).await;
    debug_search(&router).await;
    let written = flushed_rows(&instance, &state).await;
    assert_eq!(written.len(), 1, "{written:?}");

    let body = admin_queries(&router, "").await;
    let entry = &body["queries"][0];
    assert!(
        entry["payload"].is_object(),
        "the payload is rendered as an object (ADR-0122, decision 9): {entry}"
    );
    assert_eq!(entry["client"], "reader", "{entry}");
    assert_eq!(entry["mode"], "dense", "{entry}");
    assert_eq!(entry["request_id"], REQUEST_ID, "{entry}");
    assert!(
        entry["created_at"]
            .as_str()
            .is_some_and(|date| date.ends_with('Z') && date.contains('T')),
        "`created_at` is RFC 3339 in UTC, read by SQLite (ADR-0121, decision 3): {entry}"
    );
}

// ---------------------------------------------------------------------------
// Plafond de taille (ADR-0127).
// ---------------------------------------------------------------------------

/// A row of the journal, as a test writes it before an instance starts.
fn stored_row(request_id: &str) -> DebugQueryRow {
    DebugQueryRow {
        created_at: String::new(),
        request_id: request_id.to_owned(),
        index: Some(INDEX.to_owned()),
        client: "reader".to_owned(),
        mode: "dense".to_owned(),
        status: "ok".to_owned(),
        total_ms: Some(1.0),
        payload: "{}".to_owned(),
    }
}

/// The route renders the four members of the cap, with their documented values
/// (ADR-0127, decision 6).
///
/// The journal keeps two rows, and the third search takes it above the cap:
/// `capped` is true, `rows` is the row count of the last measurement, `bytes` is
/// the used size, and `trimmed` counts the row the cap deleted.
#[tokio::test]
async fn the_route_renders_the_cap_counters() {
    let (instance, state, router) =
        instance_with_alpha_journal(Journal::with_max_rows(0.0, 2)).await;
    let path = journal_path(&instance);
    let mut written = Vec::new();
    for _ in 0..3 {
        debug_search(&router).await;
        written.extend(flushed_rows(&instance, &state).await);
    }
    let kept = read_queries(&path).expect("the debug journal file is readable");
    assert_eq!(kept.len(), 2, "the cap keeps two rows: {kept:?}");

    let body = admin_queries(&router, "").await;
    assert_eq!(body["enabled"], true, "{body}");
    assert_eq!(body["capped"], true, "{body}");
    assert_eq!(body["rows"], 2, "{body}");
    assert_eq!(body["trimmed"], 1, "{body}");
    assert_eq!(
        body["bytes"].as_u64(),
        Some(used_bytes(&instance)),
        "the route renders the used size of the file, `(page_count - freelist_count) × page_size`: {body}"
    );
    let mut expected = kept.iter().map(|row| row.id).collect::<Vec<_>>();
    expected.sort_unstable_by(|left, right| right.cmp(left));
    assert_eq!(
        ids(&body),
        expected,
        "the route serves the rows the cap kept, newest first: {body}"
    );
    assert!(
        body["dropped"].as_u64().is_some(),
        "the loss counter stays a number: {body}"
    );
}

/// The route always carries the seven members of the closed object
/// (ADR-0111, ADR-0127, decision 6), and the four counters are zero when the
/// journal is disabled.
#[tokio::test]
async fn the_schema_of_the_route_is_closed_and_the_counters_are_zero() {
    let (_instance, _state, router) = instance_with_alpha().await;
    let body = admin_queries(&router, "").await;
    let mut members = body
        .as_object()
        .expect("the answer is an object")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    members.sort();
    assert_eq!(
        members,
        vec![
            "bytes", "capped", "dropped", "enabled", "queries", "rows", "trimmed"
        ],
        "the object is closed and carries the four members of the cap: {body}"
    );
    assert_eq!(body["enabled"], false, "{body}");
    assert_eq!(body["capped"], false, "{body}");
    assert_eq!(body["rows"], 0, "{body}");
    assert_eq!(body["bytes"], 0, "{body}");
    assert_eq!(body["trimmed"], 0, "{body}");
}

/// The cap is checked once at the open, so a file left above it comes back
/// under its bounds before the instance serves (ADR-0127, decision 4).
///
/// The file is written before the state is loaded, as a crash or a lower
/// configuration would leave it: four rows, and a cap of two.
#[tokio::test]
async fn the_open_control_trims_a_file_left_above_the_cap() {
    let instance = Instance::with_journal(Journal::with_max_rows(0.0, 2));
    let path = instance.root().join("data").join(DEBUG_JOURNAL_FILE_NAME);
    {
        let store = DebugJournalStore::open_or_create(&path).expect("the journal opens");
        for position in 0..4 {
            store
                .flush(
                    &[stored_row(&format!("left-{position}"))],
                    JournalCap {
                        max_rows: usize::MAX,
                        max_bytes: usize::MAX,
                    },
                )
                .expect("the row is written");
        }
        assert_eq!(read_queries(&path).expect("the journal reads").len(), 4);
    }

    let state = instance.state();
    let router = build_router(Arc::clone(&state));
    let rows = read_queries(&path).expect("the journal reads");
    assert_eq!(
        rows.len(),
        2,
        "the open control keeps the newest rows: {rows:?}"
    );
    assert_eq!(
        rows.iter()
            .map(|row| row.request_id.clone())
            .collect::<Vec<_>>(),
        vec!["left-2", "left-3"],
        "the oldest rows left at the open: {rows:?}"
    );

    let body = admin_queries(&router, "").await;
    assert_eq!(body["capped"], true, "{body}");
    assert_eq!(body["rows"], 2, "{body}");
    assert_eq!(body["trimmed"], 2, "{body}");
}

/// The `warn` of the cap carries numbers only: no query text, no filter value
/// and no key (ADR-0122, decision 10; ADR-0127, decision 6).
#[tokio::test]
async fn the_cap_warning_carries_numbers_only() {
    let sink = common::journal();
    let (instance, state, router) =
        instance_with_alpha_journal(Journal::with_max_rows(0.0, 1)).await;
    for _ in 0..2 {
        debug_search(&router).await;
    }
    state
        .flush_debug_journal()
        .await
        .expect("the flush trims at the cap");
    assert_eq!(
        read_queries(&journal_path(&instance))
            .expect("the journal reads")
            .len(),
        1,
        "the cap is reached"
    );

    let logged = sink.contents();
    let cap_lines = logged
        .lines()
        .filter(|line| line.contains("at its cap"))
        .collect::<Vec<_>>();
    assert!(
        !cap_lines.is_empty(),
        "the cap warning is journalised once: {logged}"
    );
    for line in cap_lines {
        assert!(
            line.contains("\"rows\":"),
            "the warn counts the rows: {line}"
        );
        assert!(
            line.contains("\"bytes\":"),
            "the warn counts the bytes: {line}"
        );
        assert!(
            line.contains("\"file_bytes\":"),
            "the warn gives the physical size: {line}"
        );
        assert!(
            line.contains("\"trimmed\":"),
            "the warn counts the trimmed rows: {line}"
        );
        assert!(
            !line.contains(FILTER_MARKER),
            "the warn carries no filter value: {line}"
        );
        assert!(
            !line.contains("contrat"),
            "the warn carries no query text: {line}"
        );
        assert!(
            !line.contains("request") && !line.contains("payload"),
            "the warn carries no payload: {line}"
        );
    }
}
