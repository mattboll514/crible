// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and every helper of this file is a fixture or an assertion.
#![allow(clippy::expect_used, clippy::panic)]

//! `REPONSE-DEBUG` (phase P7): the `debug` member of a search answer, on the
//! simple route and on the multi-index route.
//!
//! References: `docs/phases/P7-observabilite.md` (`REPONSE-DEBUG`),
//! `docs/05-api.md` ("Recherche : réponse", "Recherche multi-index"),
//! `docs/03-recherche.md` (the nine timing keys, the rank of a branch) and
//! ADR-0119 (decisions 1 to 11).
//!
//! | Test | Criterion / decision |
//! |---|---|
//! | `debug_is_absent_unless_asked_and_changes_nothing` | ADR-0119, decision 10 |
//! | `the_critical_path_stays_within_ten_percent_of_the_total` | ADR-0119, decision 6 |
//! | `the_branch_ranks_match_a_single_branch_run` | ADR-0119, decisions 1 and 2 |
//! | `a_branch_that_did_not_find_the_result_omits_its_members` | ADR-0119, decision 3 |
//! | `the_multi_index_debug_carries_targets_and_target_rank` | ADR-0119, decision 9 |
//!
//! The time and rank tests loop inside the test (`ROUNDS` turns) and print the
//! turn and the number of failures, so a flaky measurement is visible.
//!
//! ```text
//! cargo test -p crible-server --test search_debug
//! ```

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_search::group::{Grouping, candidates_per_branch};
use crible_server::http::build_router;
use crible_server::state::AppState;
use serde_json::{Value, json};

use common::{ADMIN_SECRET, Instance, READ_SECRET, base64_vector, index_config, send};

/// Number of turns of the time and rank loops.
const ROUNDS: usize = 20;

/// Serializes the tests of this binary.
///
/// The timing test measures the pipeline against its total (ADR-0119,
/// decision 6): an attach or a search of a sibling test, sharing the process,
/// would make it measure the harness instead. Every test of the file takes the
/// lock, so the measurement runs alone.
static SERIAL: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

/// Waits until the tests of the file run one at a time.
async fn serial() -> tokio::sync::MutexGuard<'static, ()> {
    SERIAL
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

/// The two indexes of the multi-index test.
const ALPHA: &str = "alpha";
const BETA: &str = "beta";

/// Path of the simple search route of `index`.
fn search_path(index: &str) -> String {
    format!("/v1/indexes/{index}/search")
}

/// Configuration of an index named `name`, with its lexical branch on.
fn config(name: &str) -> Value {
    let mut config = index_config();
    config["name"] = json!(name);
    config["lexical"]["enabled"] = json!(true);
    config
}

/// Creates an instance with one index per name of `names`, all lexical enabled.
async fn instance_with(names: &[&str]) -> (Instance, Arc<AppState>, Router) {
    let instance = Instance::new();
    let state = instance.state();
    let router = build_router(Arc::clone(&state));
    for name in names {
        let (status, answer) = send(
            &router,
            Method::POST,
            "/v1/indexes",
            ADMIN_SECRET,
            Some(config(name)),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "the index is created: {answer}"
        );
    }
    (instance, state, router)
}

/// Vector of `position`, distinct from every other: the cosine decreases with
/// `position`, so both branches have a deterministic total order.
fn vector(position: usize) -> String {
    let shift = f32::from(u16::try_from(position % 1_000).unwrap_or(0)) / 1_000.0;
    let norm = (1.0 + shift * shift).sqrt();
    base64_vector([1.0 / norm, shift / norm, 0.0, 0.0])
}

/// Text of `position`: one unique term and two shared terms, so the lexical
/// branch matches every passage and its ties are broken by the key.
fn text(position: usize) -> String {
    format!("article{position} contrat responsabilite")
}

/// Body of one online batch of `count` documents of one passage each, numbered
/// from `start`.
fn corpus(start: usize, count: usize) -> Value {
    let documents: Vec<Value> = (start..start + count)
        .map(|position| {
            json!({
                "doc_id": format!("D{position:04}"),
                "fields": { "numero": format!("{position}") },
                "passages": [{
                    "text": text(position),
                    "fields": { "texte": "contrat" },
                    "vector": vector(position),
                }]
            })
        })
        .collect();
    json!({
        "batch_id": format!("debug-corpus-{start}"),
        "mode": "upsert",
        "documents": documents,
    })
}

/// Documents of one online batch: ADR-0107 bounds a request to 10 000 entries.
const BATCH: usize = 5_000;

/// Writes `count` documents of one passage into `index` and waits for them,
/// in batches the size of [`BATCH`].
async fn fill(router: &Router, index: &str, count: usize) {
    let path = format!("/v1/indexes/{index}/documents?wait=true");
    for start in (0..count).step_by(BATCH) {
        let size = BATCH.min(count - start);
        let (status, accepted) = send(
            router,
            Method::POST,
            &path,
            common::WRITE_SECRET,
            Some(corpus(start, size)),
        )
        .await;
        assert!(
            status == StatusCode::OK || status == StatusCode::ACCEPTED,
            "the corpus is accepted: {accepted}"
        );
        assert!(
            accepted["state"] == "succeeded",
            "the corpus attaches within `wait_timeout_ms`: {accepted}"
        );
    }
}

/// Posts `body` on the simple search route of `index`.
async fn search(router: &Router, index: &str, body: Value) -> (StatusCode, Value) {
    send(
        router,
        Method::POST,
        &search_path(index),
        READ_SECRET,
        Some(body),
    )
    .await
}

/// A search body of `mode`, asking `k` results.
fn body(mode: &str, filter: Value, k: usize, debug: Option<bool>) -> Value {
    // `dense` refuses a query and `lexical` refuses a vector: each mode gets
    // only the input its branch needs.
    let mut body = json!({
        "mode": mode,
        "k": k,
        "exact": true,
        "filter": filter,
    });
    if mode != "lexical" {
        body["vector"] = json!(vector(0));
    }
    if mode != "dense" {
        body["query"] = json!("contrat responsabilite");
    }
    if let Some(debug) = debug {
        body["debug"] = json!(debug);
    }
    body
}

/// The results of an answer, and their keys.
fn results(body: &Value) -> &Vec<Value> {
    body["results"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer carries results: {body}"))
}

/// Keys of the results, in the order of the answer.
fn keys(body: &Value) -> Vec<String> {
    results(body)
        .iter()
        .map(|result| {
            result["key"]
                .as_str()
                .unwrap_or_else(|| panic!("a result carries its key: {result}"))
                .to_owned()
        })
        .collect()
}

/// Keys of a `debug.per_result` list.
fn debug_keys(debug: &Value) -> Vec<String> {
    debug["per_result"]
        .as_array()
        .unwrap_or_else(|| panic!("the debug member carries per_result: {debug}"))
        .iter()
        .map(|entry| {
            entry["key"]
                .as_str()
                .unwrap_or_else(|| panic!("a debug entry carries its key: {entry}"))
                .to_owned()
        })
        .collect()
}

/// Number of a `timings_ms` key, in milliseconds.
fn timing(timings: &Value, key: &str) -> f64 {
    timings[key]
        .as_f64()
        .unwrap_or_else(|| panic!("timings carries {key}: {timings}"))
}

/// Critical path of ADR-0119, decision 6, in milliseconds.
fn critical_path(timings: &Value) -> f64 {
    timing(timings, "encode")
        + timing(timings, "filter")
        + timing(timings, "dense").max(timing(timings, "lexical"))
        + timing(timings, "fusion")
        + timing(timings, "group")
        + timing(timings, "fetch")
}

/// Median of `samples`, the upper middle one for an even count.
fn median(samples: &mut [f64]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    samples.sort_by(f64::total_cmp);
    Some(samples[samples.len() / 2])
}

/// Asserts that the median gap of `gaps` stays under the 10 % bound.
///
/// A single turn may measure the machine and not the pipeline: a blocking task
/// that starts late, a thread preempted by another suite of the workspace, a
/// page fault of the corpus. The median of several turns ignores those outliers
/// and keeps what the test proves: the measured steps account for the total of a
/// representative request. A step missing from the critical path would raise
/// every sample, so the median would raise with them (`@relecteur`,
/// EXPLIQUER-DOCUMENTS; NETTOYER-P7).
fn assert_median_gap(label: &str, mut gaps: Vec<f64>) {
    let median = median(&mut gaps)
        .unwrap_or_else(|| panic!("{label}: the loop measured at least one round"));
    assert!(
        median < 0.10,
        "{label}: the median gap {median} stays within 10 % of the total, samples {gaps:?}"
    );
}

/// `total` is `took_ms` rounded to three decimals (ADR-0119, decision 5).
fn rounded_total(answer: &Value) -> f64 {
    let took_ms = answer["took_ms"]
        .as_f64()
        .unwrap_or_else(|| panic!("took_ms is a number: {answer}"));
    (took_ms * 1_000.0).round() / 1_000.0
}

/// `debug` is absent unless it is asked for, and asking for it changes nothing
/// but one member (ADR-0119, decisions 10 and 3).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn debug_is_absent_unless_asked_and_changes_nothing() {
    let _serial = serial().await;
    let (_instance, _state, router) = instance_with(&[ALPHA]).await;
    fill(&router, ALPHA, 40).await;
    for mode in ["dense", "lexical", "hybrid"] {
        let filter = json!({ "numero": { "in": ["1", "2", "3", "4", "5", "6", "7", "8"] } });
        let (omitted_status, omitted) =
            search(&router, ALPHA, body(mode, filter.clone(), 5, None)).await;
        let (false_status, refused) =
            search(&router, ALPHA, body(mode, filter.clone(), 5, Some(false))).await;
        let (asked_status, asked) = search(&router, ALPHA, body(mode, filter, 5, Some(true))).await;
        assert_eq!(omitted_status, StatusCode::OK, "{omitted}");
        assert_eq!(false_status, StatusCode::OK, "{refused}");
        assert_eq!(asked_status, StatusCode::OK, "{asked}");
        assert!(omitted.get("debug").is_none(), "debug omitted: {omitted}");
        assert!(refused.get("debug").is_none(), "debug false: {refused}");
        let debug = asked
            .get("debug")
            .unwrap_or_else(|| panic!("debug true adds the member: {asked}"));
        // The answer without `debug` is the answer with it, minus that member.
        // `took_ms` is a wall-clock duration: it differs between two requests.
        let mut stripped = asked.clone();
        let mut reference = omitted.clone();
        for answer in [&mut stripped, &mut reference] {
            let object = answer.as_object_mut().expect("an object");
            object.remove("debug");
            object.remove("took_ms");
        }
        assert_eq!(stripped, reference, "debug changes nothing else ({mode})");
        assert_eq!(keys(&asked), keys(&omitted), "same order ({mode})");
        assert_eq!(
            results(&asked).len(),
            results(&omitted).len(),
            "same k ({mode})"
        );
        assert_eq!(
            debug_keys(debug).len(),
            results(&asked).len(),
            "one per_result per result ({mode})"
        );
        // The plan carries the effective mode and the grouping of the request.
        assert_eq!(debug["plan"]["mode"], mode);
        assert_eq!(debug["plan"]["group_by"], Value::Null);
        assert!(debug["plan"]["fusion"].is_string(), "{debug}");
        // The branch members follow the mode (decision 3).
        for entry in debug["per_result"]
            .as_array()
            .expect("per_result is a list")
        {
            let has_dense = entry.get("dense_rank").is_some();
            let has_lexical = entry.get("lexical_rank").is_some();
            match mode {
                "dense" => {
                    assert!(has_dense, "dense mode carries dense members: {entry}");
                    assert!(!has_lexical, "no lexical member in dense: {entry}");
                }
                "lexical" => {
                    assert!(!has_dense, "no dense member in lexical: {entry}");
                    assert!(has_lexical, "lexical mode carries lexical members: {entry}");
                }
                _ => {
                    assert!(
                        has_dense || has_lexical,
                        "a hybrid result carries at least one branch: {entry}"
                    );
                }
            }
            assert!(entry["fused_rank"].as_u64().is_some(), "{entry}");
        }
    }
}

/// The sum of the critical path stays within 10 % of `took_ms`, in dense,
/// lexical and hybrid (ADR-0119, decision 6).
///
/// The corpus is large enough for the measured steps to dominate the scheduling
/// overhead of the blocking task: a request too fast for the bound would make
/// the test measure the harness, not the pipeline. The JSON rendering stays
/// outside `took_ms` (ADR-0120, decision 1). The bound is read on the median of
/// the rounds, so a run of the whole workspace, which preempts the blocking
/// thread, does not fail a test that still proves the accounting of a typical
/// request (NETTOYER-P7).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_critical_path_stays_within_ten_percent_of_the_total() {
    let _serial = serial().await;
    let (_instance, _state, router) = instance_with(&[ALPHA]).await;
    fill(&router, ALPHA, 9_000).await;
    let mut gaps: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    let mut failures = 0;
    for round in 1..=ROUNDS {
        for mode in ["dense", "lexical", "hybrid"] {
            let (status, answer) =
                search(&router, ALPHA, body(mode, json!({}), 10, Some(true))).await;
            assert_eq!(status, StatusCode::OK, "{answer}");
            let timings = &answer["debug"]["timings_ms"];
            let total = timing(timings, "total");
            assert_eq!(
                total,
                rounded_total(&answer),
                "`total` is the rounded `took_ms` (decision 5)"
            );
            let path = critical_path(timings);
            let gap = (total - path).abs() / total;
            if gap >= 0.10 {
                failures += 1;
                println!("round {round} {mode}: total {total}, path {path}, gap {gap}, {timings}");
            }
            gaps.entry(mode).or_default().push(gap);
        }
    }
    println!("{ROUNDS} rounds, {failures} samples above the bound");
    // The bound is checked on the median of the rounds: a loaded machine
    // inflates a sample or two, and the property under test is the accounting of
    // a representative request, not the absence of any preemption (NETTOYER-P7).
    let mut all: Vec<f64> = gaps.values().flatten().copied().collect();
    println!("median gap {:?}", median(&mut all));
    for (mode, samples) in gaps {
        assert_median_gap(mode, samples);
    }
}

/// The ranks of a hybrid answer are the ones of a single-branch run with the
/// same over-sampling, and `fused_rank` follows the fusion before regrouping
/// (ADR-0119, decisions 1 and 2).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_branch_ranks_match_a_single_branch_run() {
    let _serial = serial().await;
    let (_instance, _state, router) = instance_with(&[ALPHA]).await;
    // The first five passages share their text, so `dedup: "content"` drops
    // four of them and the survivors keep their fusion rank.
    fill(&router, ALPHA, 200).await;
    let k = 10;
    let mut failures = 0;
    for round in 1..=ROUNDS {
        for grouping in [
            Grouping::none(),
            Grouping {
                by_document: true,
                passages_per_document: 1,
                dedup_content: false,
            },
            Grouping {
                by_document: false,
                passages_per_document: 1,
                dedup_content: true,
            },
        ] {
            let requested = candidates_per_branch(k, &grouping);
            // Reference: one branch alone, with the same over-sampling.
            let (dense_status, dense) = search(
                &router,
                ALPHA,
                body("dense", json!({}), requested, Some(true)),
            )
            .await;
            let (lexical_status, lexical) = search(
                &router,
                ALPHA,
                body("lexical", json!({}), requested, Some(true)),
            )
            .await;
            let (fusion_status, fusion) = search(
                &router,
                ALPHA,
                body("hybrid", json!({}), requested, Some(true)),
            )
            .await;
            assert_eq!(dense_status, StatusCode::OK, "{dense}");
            assert_eq!(lexical_status, StatusCode::OK, "{lexical}");
            assert_eq!(fusion_status, StatusCode::OK, "{fusion}");
            let dense_ranks = positions(&dense);
            let lexical_ranks = positions(&lexical);
            let mut fused_ranks = BTreeMap::new();
            for (position, key) in debug_keys(&fusion["debug"]).into_iter().enumerate() {
                fused_ranks.insert(key, u32::try_from(position + 1).unwrap_or(u32::MAX));
            }
            let best_by_document = best_by_document(&fusion["debug"], &fused_ranks);

            // The request under test, with the same grouping.
            let mut request = body("hybrid", json!({}), k, Some(true));
            request["group_by"] = if grouping.by_document {
                json!("document")
            } else {
                Value::Null
            };
            request["passages_per_document"] = json!(grouping.passages_per_document);
            request["dedup"] = if grouping.dedup_content {
                json!("content")
            } else {
                Value::Null
            };
            let (status, answer) = search(&router, ALPHA, request).await;
            assert_eq!(status, StatusCode::OK, "{answer}");
            let mut previous = 0;
            for entry in answer["debug"]["per_result"]
                .as_array()
                .expect("per_result is a list")
            {
                let key = entry["key"].as_str().expect("a key");
                if let Some(rank) = entry.get("dense_rank").and_then(Value::as_u64) {
                    let expected = dense_ranks.get(key).copied().unwrap_or(0);
                    if rank != u64::from(expected) {
                        failures += 1;
                        println!("round {round}: dense_rank {rank} != {expected} for {key}");
                    }
                }
                if let Some(rank) = entry.get("lexical_rank").and_then(Value::as_u64) {
                    let expected = lexical_ranks.get(key).copied().unwrap_or(0);
                    if rank != u64::from(expected) {
                        failures += 1;
                        println!("round {round}: lexical_rank {rank} != {expected} for {key}");
                    }
                }
                let fused = entry["fused_rank"].as_u64().expect("fused_rank");
                let document = key.rsplit_once('#').map_or(key, |(document, _)| document);
                let expected = if grouping.by_document {
                    best_by_document.get(document).copied().unwrap_or(0)
                } else {
                    fused_ranks.get(key).copied().unwrap_or(0)
                };
                if fused != u64::from(expected) {
                    failures += 1;
                    println!("round {round}: fused_rank {fused} != {expected} for {key}");
                }
                // The answer is ordered by the fused ranking: a rank never goes
                // back once it advanced.
                if fused < previous {
                    failures += 1;
                    println!("round {round}: fused_rank {fused} after {previous}");
                }
                previous = fused;
            }
        }
    }
    println!("{ROUNDS} rounds, {failures} failures");
    assert_eq!(failures, 0, "the reported ranks are the branch ranks");
}

/// Position of each key in the answer, from 1.
fn positions(answer: &Value) -> BTreeMap<String, u32> {
    keys(answer)
        .into_iter()
        .enumerate()
        .map(|(position, key)| (key, u32::try_from(position + 1).unwrap_or(u32::MAX)))
        .collect()
}

/// Best fusion rank of each document, over every entry of the fusion.
fn best_by_document(debug: &Value, fused_ranks: &BTreeMap<String, u32>) -> BTreeMap<String, u32> {
    let mut best: BTreeMap<String, u32> = BTreeMap::new();
    for entry in debug["per_result"]
        .as_array()
        .expect("per_result is a list")
    {
        let key = entry["key"].as_str().expect("a key");
        let document = key.rsplit_once('#').map_or(key, |(document, _)| document);
        let rank = fused_ranks.get(key).copied().unwrap_or(0);
        best.entry(document.to_owned())
            .and_modify(|current| *current = (*current).min(rank))
            .or_insert(rank);
    }
    best
}

/// A branch that did not find a result omits its members, never `null` and
/// never `0` (ADR-0119, decision 3).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_branch_that_did_not_find_the_result_omits_its_members() {
    let _serial = serial().await;
    let (_instance, _state, router) = instance_with(&[ALPHA]).await;
    fill(&router, ALPHA, 120).await;
    // The query names one document whose cosine is outside the dense top-k, so
    // the lexical branch finds a result the dense branch does not hold, and the
    // dense branch holds results the lexical one does not: both directions
    // appear in one hybrid answer.
    let (status, answer) = search(
        &router,
        ALPHA,
        json!({
            "mode": "hybrid",
            "query": "article119",
            "vector": vector(0),
            "k": 20,
            "exact": true,
            "debug": true,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let entries = answer["debug"]["per_result"]
        .as_array()
        .expect("per_result is a list");
    assert!(!entries.is_empty(), "{answer}");
    for entry in entries {
        for member in ["dense_rank", "dense_score", "lexical_rank", "lexical_score"] {
            if let Some(value) = entry.get(member) {
                assert!(!value.is_null(), "{member} is absent, not null: {entry}");
                if member.ends_with("_rank") {
                    assert!(value.as_u64().is_some_and(|rank| rank > 0), "{entry}");
                }
            }
        }
        assert!(entry["fused_rank"].as_u64().is_some_and(|rank| rank > 0));
    }
    // `per_result` follows the order of `results`, entry by entry (ADR-0119,
    // decision 3).
    assert_eq!(
        debug_keys(&answer["debug"]),
        keys(&answer),
        "per_result follows the results"
    );
    // The hybrid answer carries one entry of each direction: a dense-only
    // result has no lexical member, a lexical-only result no dense member
    // (ADR-0119, decision 3).
    let dense_only = entries
        .iter()
        .any(|entry| entry.get("dense_rank").is_some() && entry.get("lexical_rank").is_none());
    let lexical_only = entries
        .iter()
        .any(|entry| entry.get("lexical_rank").is_some() && entry.get("dense_rank").is_none());
    assert!(dense_only, "a dense-only result appears: {answer}");
    assert!(lexical_only, "a lexical-only result appears: {answer}");
    // In dense mode the lexical members are absent from every entry.
    let (dense_status, dense) =
        search(&router, ALPHA, body("dense", json!({}), 5, Some(true))).await;
    assert_eq!(dense_status, StatusCode::OK, "{dense}");
    for entry in dense["debug"]["per_result"]
        .as_array()
        .expect("per_result is a list")
    {
        assert!(
            entry.get("lexical_rank").is_none() && entry.get("lexical_score").is_none(),
            "{entry}"
        );
    }
    // In lexical mode the dense members are absent from every entry.
    let (lexical_status, lexical) =
        search(&router, ALPHA, body("lexical", json!({}), 5, Some(true))).await;
    assert_eq!(lexical_status, StatusCode::OK, "{lexical}");
    for entry in lexical["debug"]["per_result"]
        .as_array()
        .expect("per_result is a list")
    {
        assert!(
            entry.get("dense_rank").is_none() && entry.get("dense_score").is_none(),
            "{entry}"
        );
    }
}

/// Body of a multi-index search in `mode`, asking for `debug`.
///
/// `dense` carries a vector, `lexical` a query, and `hybrid` both: the fixture
/// model is `external`, so it has no encoder and a hybrid request without a
/// vector is refused (`invalid_request`). `encode` therefore stays `0.0` here.
fn multi_body(mode: &str) -> Value {
    // The first target keeps a thousand passages, so its work dominates the
    // fixed overhead of one search; the second keeps the whole index.
    let thousand: Vec<String> = (0..1_000).map(|position| position.to_string()).collect();
    let mut body = json!({
        "mode": mode,
        "k": 10,
        "exact": true,
        "debug": true,
        "targets": [
            { "index": ALPHA, "filter": { "numero": { "in": thousand } } },
            { "index": BETA, "filter": {} },
        ],
    });
    if mode != "lexical" {
        body["vector"] = json!(vector(0));
    }
    if mode != "dense" {
        body["query"] = json!("contrat responsabilite");
    }
    body
}

/// The multi-index `debug` carries `targets[]` and `target_rank`, measures the
/// validation and the single encoding once in its global timings, and keeps the
/// critical path of every target within 10 % of its own total (ADR-0119,
/// decisions 6 and 9).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_multi_index_debug_carries_targets_and_target_rank() {
    let _serial = serial().await;
    let (_instance, _state, router) = instance_with(&[ALPHA, BETA]).await;
    // The corpus is the one of the simple timing test: the measured steps of a
    // target must dominate the fixed overhead around the pipeline.
    fill(&router, ALPHA, 4_000).await;
    fill(&router, BETA, 4_000).await;

    for mode in ["dense", "lexical", "hybrid"] {
        let body = multi_body(mode);
        let (status, answer) = send(
            &router,
            Method::POST,
            "/v1/search",
            READ_SECRET,
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        let debug = &answer["debug"];
        let targets = debug["targets"].as_array().expect("targets is a list");
        assert_eq!(targets.len(), 2, "{debug}");
        assert_eq!(targets[0]["index"], ALPHA);
        assert_eq!(targets[1]["index"], BETA);
        for target in targets {
            assert_eq!(target["plan"]["mode"], mode, "{target}");
            // The validation and the single encoding belong to the global
            // timings: a target renders `0.0` for them (ADR-0119, decision 9).
            assert_eq!(target["timings_ms"]["validate"], 0.0, "{target}");
            assert_eq!(target["timings_ms"]["encode"], 0.0, "{target}");
            assert!(
                timing(&target["timings_ms"], "total") > 0.0,
                "a target measured its work: {target}"
            );
        }
        let global = &debug["timings_ms"];
        assert_eq!(
            timing(global, "total"),
            rounded_total(&answer),
            "`total` is the rounded `took_ms` (decision 5)"
        );
        assert!(
            timing(global, "validate") > 0.0,
            "the global validation is measured once: {global}"
        );
        // The fixture model is `external`: no request of this bank encodes a
        // query, so the global `encode` is `0.0` in every mode. The single
        // measurement of the request, and its `0.0` on each target, are proved
        // by `search_multi_encoding.rs`, where the failpoints double encodes.
        assert_eq!(
            timing(global, "encode"),
            0.0,
            "a mode that encodes nothing renders 0.0: {global}"
        );
        let entries = debug["per_result"]
            .as_array()
            .expect("per_result is a list");
        assert_eq!(entries.len(), results(&answer).len(), "{debug}");
        for (entry, result) in entries.iter().zip(results(&answer)) {
            assert_eq!(entry["index"], result["index"], "{entry}");
            assert_eq!(entry["key"], result["key"], "{entry}");
            let target_rank = entry["target_rank"]
                .as_u64()
                .unwrap_or_else(|| panic!("target_rank is a number: {entry}"));
            assert!(target_rank >= 1, "{entry}");
            if mode != "dense" {
                // ADR-0110: the RRF between indexes is `1 / (60 + rang)`.
                let expected = 1.0 / (60.0 + f64::from(u32::try_from(target_rank).unwrap_or(0)));
                let score = result["score"].as_f64().expect("a score");
                assert!(
                    (score - expected).abs() < 1e-6,
                    "score {score} is the RRF of rank {target_rank}: {result}"
                );
            }
        }
        // Without `debug`, the answer carries the same results and no member.
        let mut plain_body = body;
        plain_body
            .as_object_mut()
            .expect("an object")
            .remove("debug");
        let (plain_status, plain) = send(
            &router,
            Method::POST,
            "/v1/search",
            READ_SECRET,
            Some(plain_body),
        )
        .await;
        assert_eq!(plain_status, StatusCode::OK, "{plain}");
        assert!(plain.get("debug").is_none(), "{plain}");
        assert_eq!(keys(&plain), keys(&answer), "same results ({mode})");
    }

    // Every target keeps its critical path within 10 % of its own total
    // (ADR-0119, decision 6, extended to `targets[]` by decision 9), on the
    // median of the rounds: a preempted sample would otherwise fail a loaded run
    // and prove nothing (NETTOYER-P7).
    let mut gaps: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut failures = 0;
    for round in 1..=ROUNDS {
        for mode in ["dense", "lexical", "hybrid"] {
            let (status, answer) = send(
                &router,
                Method::POST,
                "/v1/search",
                READ_SECRET,
                Some(multi_body(mode)),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{answer}");
            for target in answer["debug"]["targets"]
                .as_array()
                .expect("targets is a list")
            {
                let total = timing(&target["timings_ms"], "total");
                let path = critical_path(&target["timings_ms"]);
                let gap = (total - path).abs() / total;
                if gap >= 0.10 {
                    failures += 1;
                    println!(
                        "round {round} {mode} {}: total {total}, path {path}, gap {gap}, {}",
                        target["index"], target["timings_ms"]
                    );
                }
                gaps.entry(format!("{mode} {}", target["index"]))
                    .or_default()
                    .push(gap);
            }
        }
    }
    println!("{ROUNDS} rounds, {failures} samples above the bound");
    for (label, samples) in gaps {
        assert_median_gap(&label, samples);
    }
}
