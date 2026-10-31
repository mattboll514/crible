// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixtures and the scans of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! INV-13 — the analytics never hold a query text, a filter value, a hash or a
//! result identifier (`docs/08-invariants-et-tests.md:21`; `docs/06-observabilite.md`,
//! "Ce qu'on ne stocke jamais"; ADR-0117, decision 5).
//!
//! The test drives the real routes of the server: it sends 100 searches, each
//! with three markers it alone knows — the text of the query, the value of the
//! filter, and the external id of the document the answer returns. It then
//! forces a flush, and searches every marker in the raw bytes of
//! `analytics.sqlite` and of its `-wal` and `-shm` when they exist, and in the
//! JSON lines the server journalises at `info` (ADR-0096).
//!
//! The searches are `lexical` on purpose: `docs/05-api.md` needs no encoder and
//! no vector for that mode, so the fixture uses an `external` model and ONNX
//! Runtime is never loaded. The coverage of the request surface is the one the
//! invariant needs: the simple route, the multi-index route (`POST /v1/search`,
//! one event per target), and refusals — a `400` is an operation the server
//! tried, and it must not leak either (ADR-0118, decision 6).
//!
//! The test cannot pass on an empty run: it asserts the aggregates count every
//! search it sent, that the returned ids are the ones it indexed, that the
//! analytics file starts with the SQLite header, and that the journal holds the
//! line the server writes when it loads its indexes.
//!
//! | Test | What it proves |
//! |---|---|
//! | `inv_13_the_analytics_and_the_journal_hold_no_query_text_no_filter_value_and_no_result_id` | 100 searches, simple, multi-index and refused: no marker anywhere |
//! | `inv_13_a_refused_search_leaks_neither_its_query_text_nor_its_filter_value` | the `400` path alone leaks nothing, and still counts |
//! | `the_marker_of_a_line_written_by_the_test_is_found_in_the_captured_journal` | positive control: the journal capture is not empty, so the absence is real |
//!
//! ```text
//! cargo test -p crible-server --test inv_13_analytics_without_content
//! ```

mod common;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::http::build_router;
use crible_server::state::AppState;
use crible_store::analytics::{ANALYTICS_FILE_NAME, MinuteEvent, read_minute_events};
use serde_json::{Value, json};

use common::{
    ADMIN_SECRET, Instance, READ_SECRET, WRITE_SECRET, base64_vector, journal, send, terminal_job,
};

/// First index of the fixture.
const ALPHA: &str = "alpha";

/// Second index of the fixture, the other target of the multi-index searches.
const BETA: &str = "beta";

/// Search route of the fixture index.
const SEARCH_ALPHA: &str = "/v1/indexes/alpha/search";

/// Multi-index route.
const MULTI: &str = "/v1/search";

/// Vector of every passage of the fixture: the lexical branch is the only one
/// that runs, so the value is never compared.
const VECTOR: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

/// Number of searches of the simple route.
const SINGLE: usize = 40;

/// Number of searches of the multi-index route.
const MULTI_REQUESTS: usize = 30;

/// Number of refused searches.
const REFUSED: usize = 30;

/// Total of the searches of the main test: the 100 of the deliverable.
const TOTAL: usize = SINGLE + MULTI_REQUESTS + REFUSED;

/// Number of refused searches of the refusal test.
const REFUSED_ALONE: usize = 12;

/// A marker of an analytics row is `u64`, and an id of a result is a `u64` of
/// the same kind: the marker must be a text that only this run can produce.
///
/// The markers are random: the seed of one generator mixes the wall clock, the
/// process id and a process-wide counter, and each draw is the finaliser of
/// SplitMix64. `std` alone provides the three, so no dependency is added. A
/// marker already present in the file for another reason cannot mask a leak, and
/// a run that leaks names the markers it used in its failure message.
struct Markers {
    state: u64,
}

/// Finaliser of SplitMix64: a bijective mix, so two nearby seeds give two
/// unrelated markers.
fn mix(value: u64) -> u64 {
    let mut z = value;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

impl Markers {
    /// The generator of one test, at a seed no other run and no other generator
    /// of this binary can repeat.
    fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};

        // Two tests may start in the same nanosecond: the counter separates
        // them, and the process id separates two `cargo test` processes.
        static GENERATORS: AtomicU64 = AtomicU64::new(0);
        let generator = GENERATORS.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos() as u64);
        let seed = mix(nanos
            ^ (u64::from(std::process::id()) << 32)
            ^ generator.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        Self { state: seed }
    }

    /// Next marker, `m13` followed by sixteen hexadecimal digits.
    ///
    /// Nineteen ASCII characters carrying 64 bits: a marker found by accident in
    /// a few tens of kilobytes is not a hazard, and the lexical tokenizer keeps
    /// it as one token, so a query text matches the passage it names.
    fn next(&mut self) -> String {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        format!("m13{:016x}", mix(self.state))
    }
}

/// The three markers of one search: its query text, its filter value, and the
/// external id of the document its answer returns.
struct MarkedRequest {
    query: String,
    filter: String,
    doc_id: String,
}

/// An instance whose two indexes `alpha` and `beta` have the lexical branch.
///
/// The fixture model is `external`, so no ONNX Runtime is loaded; `lexical`
/// needs neither an encoder nor a vector (`docs/05-api.md`).
async fn instance() -> (Instance, Arc<AppState>, Router) {
    let instance = Instance::new();
    let state = instance.state();
    let router = build_router(Arc::clone(&state));
    create(&router, ALPHA).await;
    create(&router, BETA).await;
    (instance, state, router)
}

/// Creates the fixture index `name`, with the lexical branch enabled.
async fn create(router: &Router, name: &str) {
    let mut body = common::index_config();
    body["name"] = json!(name);
    body["lexical"]["enabled"] = json!(true);
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

/// One online-batch document: its external id, its filter value, its text.
fn document(doc_id: &str, numero: &str, text: &str) -> Value {
    json!({
        "doc_id": doc_id,
        "fields": { "numero": numero },
        "passages": [{
            "text": text,
            "fields": { "texte": "contrat" },
            "vector": base64_vector(VECTOR),
        }],
    })
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

/// Body of a `lexical` search of one index.
fn single_body(query: &str, filter: &str) -> Value {
    json!({
        "query": query,
        "mode": "lexical",
        "k": 5,
        "filter": { "numero": { "eq": filter } },
    })
}

/// Body of a `lexical` search of the two indexes, one filter per target.
fn multi_body(query: &str, filter: &str) -> Value {
    json!({
        "query": query,
        "mode": "lexical",
        "k": 5,
        "targets": [
            { "index": ALPHA, "filter": { "numero": { "eq": filter } } },
            { "index": BETA, "filter": { "numero": { "eq": filter } } },
        ],
    })
}

/// Body of a search refused for its filter: the operator `regex` is unknown to
/// the grammar of `docs/05-api.md`, and the value it carries is a marker.
fn refused_body(query: &str, filter: &str) -> Value {
    json!({
        "query": query,
        "mode": "lexical",
        "k": 5,
        "filter": { "numero": { "regex": filter } },
    })
}

/// Path of the analytics file of the instance.
fn analytics_path(instance: &Instance) -> PathBuf {
    instance.root().join("data").join(ANALYTICS_FILE_NAME)
}

/// Raw bytes of the analytics file, and of the write-ahead log and the shared
/// memory SQLite may keep beside it when they exist.
///
/// The three files are scanned: a value written in any column of any type would
/// appear in one of them, and a marker that reached the `-wal` and not yet the
/// main file is as much a leak (INV-13).
fn analytics_bytes(path: &Path) -> Vec<(String, Vec<u8>)> {
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

/// Asserts that neither the analytics files nor the journal hold `marker`.
///
/// `kind` names what the marker is, so a failure says which half of INV-13 is
/// broken: the file of the aggregates, or the journal of the server.
fn assert_absent(files: &[(String, Vec<u8>)], journal_text: &str, marker: &str, kind: &str) {
    for (name, bytes) in files {
        assert!(
            !contains(bytes, marker.as_bytes()),
            "the analytics file {name} holds the {kind} {marker:?} (INV-13)"
        );
    }
    assert!(
        !journal_text.contains(marker),
        "the journal holds the {kind} {marker:?} (INV-13)"
    );
}

/// Sum of the `count` of the rows of one key.
fn count(rows: &[MinuteEvent], index: &str, operation: &str, status: &str) -> u64 {
    rows.iter()
        .filter(|row| row.index == index && row.operation == operation && row.status == status)
        .map(|row| row.count)
        .sum()
}

/// One hundred searches leave no marker in the analytics file nor in the
/// journal, and the aggregates count every one of them.
///
/// INV-13, and the deliverable of `TESTER-ANALYTICS-SANS-CONTENU`
/// (`docs/phases/P7-observabilite.md:48`): 100 requests, each with a unique
/// random marker in its query text and another in a filter value, the simple
/// route and the multi-index one, plus refusals. The flush is forced, then every
/// marker is searched in the raw bytes of `analytics.sqlite`, of its `-wal` and
/// of its `-shm`, and in the `info` journal.
#[tokio::test]
async fn inv_13_the_analytics_and_the_journal_hold_no_query_text_no_filter_value_and_no_result_id()
{
    // The journal is taken before the fixture: the sink captures every line the
    // server writes from here, including the ones of the load.
    let journal = journal();
    let (instance, state, router) = instance().await;

    let mut markers = Markers::new();
    let requests: Vec<MarkedRequest> = (0..TOTAL)
        .map(|_| MarkedRequest {
            query: markers.next(),
            filter: markers.next(),
            doc_id: markers.next(),
        })
        .collect();
    // The documents of `beta`: one per multi-index request, with an id of its
    // own, so a multi-index answer returns an id of each index.
    let beta_docs: Vec<String> = (0..MULTI_REQUESTS).map(|_| markers.next()).collect();

    // The markers are random, so the test must not assume they differ: it
    // checks it. A repeated marker would weaken the scan, because one value
    // would stand for two pieces of content.
    let all_markers: HashSet<&str> = requests
        .iter()
        .flat_map(|request| {
            [
                request.query.as_str(),
                request.filter.as_str(),
                request.doc_id.as_str(),
            ]
        })
        .chain(beta_docs.iter().map(String::as_str))
        .collect();
    assert_eq!(
        all_markers.len(),
        3 * TOTAL + MULTI_REQUESTS,
        "every marker of the run is unique"
    );

    // Every request owns one document of `alpha`, whose text is the query of
    // that request and whose field `numero` is its filter value: a search that
    // finds it returns the id marker of that document.
    attach(
        &router,
        &state,
        ALPHA,
        "inv-13-alpha",
        requests
            .iter()
            .map(|request| document(&request.doc_id, &request.filter, &request.query))
            .collect(),
    )
    .await;
    attach(
        &router,
        &state,
        BETA,
        "inv-13-beta",
        requests[SINGLE..SINGLE + MULTI_REQUESTS]
            .iter()
            .zip(&beta_docs)
            .map(|(request, doc_id)| document(doc_id, &request.filter, &request.query))
            .collect(),
    )
    .await;

    // The ids the answers actually returned: they are markers too, and
    // INV-13 forbids the analytics from holding them.
    let mut returned: Vec<String> = Vec::new();

    for (position, request) in requests.iter().enumerate() {
        if position < SINGLE {
            let (status, answer) = send(
                &router,
                Method::POST,
                SEARCH_ALPHA,
                READ_SECRET,
                Some(single_body(&request.query, &request.filter)),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "the search answers: {answer}");
            assert_eq!(
                answer["results"][0]["doc_id"], request.doc_id,
                "the search returns the document its markers name: {answer}"
            );
            returned.push(request.doc_id.clone());
        } else if position < SINGLE + MULTI_REQUESTS {
            let (status, answer) = send(
                &router,
                Method::POST,
                MULTI,
                READ_SECRET,
                Some(multi_body(&request.query, &request.filter)),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::OK,
                "the multi-index search answers: {answer}"
            );
            let results = answer["results"]
                .as_array()
                .unwrap_or_else(|| panic!("the multi-index answer holds results: {answer}"));
            assert_eq!(
                results.len(),
                2,
                "one result per target, and each is a marker: {answer}"
            );
            let beta_doc = &beta_docs[position - SINGLE];
            for doc_id in [&request.doc_id, beta_doc] {
                assert!(
                    results
                        .iter()
                        .any(|result| result["doc_id"] == doc_id.as_str()),
                    "the multi-index answer returns {doc_id}: {answer}"
                );
                returned.push(doc_id.clone());
            }
        } else {
            let (status, answer) = send(
                &router,
                Method::POST,
                SEARCH_ALPHA,
                READ_SECRET,
                Some(refused_body(&request.query, &request.filter)),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "the unknown operator is refused: {answer}"
            );
            assert_eq!(answer["error"]["code"], "invalid_filter", "{answer}");
        }
    }

    assert_eq!(
        returned.len(),
        SINGLE + 2 * MULTI_REQUESTS,
        "every answered search returned the id markers of its documents"
    );

    state
        .flush_analytics()
        .await
        .expect("the events of the 100 searches are flushed");

    // The aggregates count the searches, so the scan below cannot pass on an
    // empty file. The refused ones are operations the server tried: they count
    // under the code of their refusal (ADR-0118, decision 6).
    let path = analytics_path(&instance);
    let rows = read_minute_events(&path).expect("the analytics file is readable");
    assert_eq!(
        count(&rows, ALPHA, "search", "ok"),
        SINGLE as u64,
        "the simple searches are counted: {rows:?}"
    );
    assert_eq!(
        count(&rows, ALPHA, "search", "invalid_filter"),
        REFUSED as u64,
        "the refused searches are counted: {rows:?}"
    );
    assert_eq!(
        count(&rows, ALPHA, "search_multi", "ok"),
        MULTI_REQUESTS as u64,
        "the multi-index searches are counted on their first target: {rows:?}"
    );
    assert_eq!(
        count(&rows, BETA, "search_multi", "ok"),
        MULTI_REQUESTS as u64,
        "one event per target of a multi-index search: {rows:?}"
    );

    let files = analytics_bytes(&path);
    assert!(
        !files.is_empty(),
        "the analytics file is readable at {}",
        path.display()
    );
    let (first_name, first_bytes) = &files[0];
    assert!(
        first_bytes.starts_with(b"SQLite format 3\0"),
        "the bytes read from {first_name} are a SQLite database"
    );
    // The schema of the table may live in the main file or in its write-ahead
    // log, whichever SQLite checkpointed: one of the files read holds it.
    assert!(
        files
            .iter()
            .any(|(_, bytes)| contains(bytes, b"minute_events")),
        "the bytes read from the analytics files hold the aggregates table: {:?}",
        files
            .iter()
            .map(|(name, bytes)| (name.clone(), bytes.len()))
            .collect::<Vec<_>>()
    );

    let journal_text = journal.contents();
    assert!(
        journal_text.contains("models and indexes loaded"),
        "the journal of the test holds the lines of the server: {journal_text:?}"
    );

    // The scan is not vacuous: the name of an index is not client content and
    // the aggregates hold it (`docs/06-observabilite.md`: a field name of the
    // schema is permitted), so the bytes really are the rows of the searches.
    assert!(
        files
            .iter()
            .any(|(_, bytes)| contains(bytes, ALPHA.as_bytes())),
        "the analytics bytes hold the name of the index, so the scan reads the rows"
    );
    assert!(
        journal_text.contains(ALPHA),
        "the journal of the test holds the name of the index"
    );
    // The canonical form of the filters of the answered searches is written by
    // the writer itself (`numero:eq`): finding it proves the scan reaches the
    // strings the analytics store, not only the identifiers of the schema.
    assert!(
        files.iter().any(|(_, bytes)| contains(bytes, b"numero:eq")),
        "the analytics bytes hold the shape of the filters of the searches"
    );

    for (request, position) in requests.iter().zip(0..) {
        assert_absent(&files, &journal_text, &request.query, "query text");
        assert_absent(&files, &journal_text, &request.filter, "filter value");
        assert_absent(&files, &journal_text, &request.doc_id, "result id");
        if (SINGLE..SINGLE + MULTI_REQUESTS).contains(&position) {
            let beta_doc = &beta_docs[position - SINGLE];
            assert_absent(&files, &journal_text, beta_doc, "result id");
        }
    }
}

/// A refused search leaves no trace of its content, and still counts.
///
/// INV-13 on the refusal path of `search_inner`: the error is rendered and
/// journalised with its code, its status and its request identifier only
/// (ADR-0101, decision 4), and the analytics count the operation without any
/// field of the request — neither the query text nor the value of the filter
/// that was refused.
#[tokio::test]
async fn inv_13_a_refused_search_leaks_neither_its_query_text_nor_its_filter_value() {
    let journal = journal();
    let instance = Instance::new();
    let state = instance.state();
    let router = build_router(Arc::clone(&state));
    create(&router, ALPHA).await;

    let mut markers = Markers::new();
    let requests: Vec<(String, String)> = (0..REFUSED_ALONE)
        .map(|_| (markers.next(), markers.next()))
        .collect();

    for (query, filter) in &requests {
        let (status, answer) = send(
            &router,
            Method::POST,
            SEARCH_ALPHA,
            READ_SECRET,
            Some(refused_body(query, filter)),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "the unknown operator is refused: {answer}"
        );
        assert_eq!(answer["error"]["code"], "invalid_filter", "{answer}");
    }

    state
        .flush_analytics()
        .await
        .expect("the refused searches are flushed");

    let path = analytics_path(&instance);
    let rows = read_minute_events(&path).expect("the analytics file is readable");
    assert_eq!(
        count(&rows, ALPHA, "search", "invalid_filter"),
        REFUSED_ALONE as u64,
        "a refusal is an operation the server tried: {rows:?}"
    );

    let files = analytics_bytes(&path);
    assert!(
        !files.is_empty(),
        "the analytics file is readable at {}",
        path.display()
    );
    let journal_text = journal.contents();
    assert!(
        !journal_text.is_empty(),
        "the journal of the test holds the lines of the server"
    );

    for (query, filter) in &requests {
        assert_absent(&files, &journal_text, query, "query text");
        assert_absent(&files, &journal_text, filter, "filter value");
    }
}

/// Positive control: a line of the test carrying a marker reaches the captured
/// journal, so the absence proved by the other tests is a real absence.
///
/// Without this control, a subscriber that captured nothing would let
/// INV-13 pass on an empty journal. The marker is a fixed literal, distinct from
/// the generated ones, so it cannot mask a leak of the other tests.
#[tokio::test]
async fn the_marker_of_a_line_written_by_the_test_is_found_in_the_captured_journal() {
    let journal = journal();
    let marker = "positive-control-of-the-journal-m13";
    tracing::info!(probe = %marker, "a probe of the capture");
    assert!(
        journal.contents().contains(marker),
        "the journal of the test captures the lines it writes"
    );
}
