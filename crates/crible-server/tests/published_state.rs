// SPDX-License-Identifier: Apache-2.0

// Test fixtures may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixture below builds an index and a job table.
#![allow(clippy::panic)]

//! The triplet `(config, snapshot, summary)` of an index is published together
//! (ADR-0105, decision 4).
//!
//! A `PATCH` job replaces the configuration and publishes a snapshot in the
//! same step; `refresh`, after a job that only committed a generation, replaces
//! the snapshot and the summary and keeps the configuration. A request must see
//! the old triplet or the new one, never a configuration beside a summary of
//! another state, so the three parts live under one lock and a reader takes one
//! guard ([`IndexWriter::published`]).
//!
//! Two tests prove the invariant without a duration. The first publishes pairs
//! whose two halves are equal by construction and reads them back; with three
//! separate publications the reader observes a mixture as soon as it
//! interleaves, and the bounded loop is long enough that the interleaving
//! happens. The second runs real jobs and reads the two HTTP routes that serve
//! the triplet, `GET /v1/indexes/{index}` and `GET /v1/indexes`, while a bounded
//! series of refreshes interleaves with them.
//!
//! ```text
//! cargo test -p crible-server --test published_state
//! ```

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_core::config::IndexConfig;
use crible_server::jobs::{IndexSummary, IndexWriter, JobBoard};
use crible_store::generation::{GenerationStore, INITIAL_EPOCH, write_index_config};
use serde_json::{Value, json};

use common::{
    INDEX, READ_SECRET, base64_vector, instance_with_alpha, post_documents, send, terminal_job,
};

/// Configuration of the fixture index, with no field indexed.
const CONFIG: &str = r#"{
    "name": "atomic",
    "model": "m",
    "reference_precision": "fp16",
    "schema": { "document_fields": {}, "passage_fields": {} },
    "cluster_by": [],
    "lexical": { "enabled": false, "language": "fr", "positions": false },
    "search": {
        "scan": "none",
        "rescore_candidates": 10,
        "exact_threshold": 10,
        "candidates_per_branch": 10,
        "fusion": { "method": "rrf" }
    },
    "ingest": { "acceptance_sample": 1, "acceptance_min_cosine": 0.9 },
    "compaction": { "dead_ratio": 0.1 }
}"#;

/// Summary whose `documents` carries the marker of the published pair.
fn summary(marker: u64) -> IndexSummary {
    IndexSummary {
        epoch: 1,
        generation: 0,
        documents: marker,
        live_passages: 0,
        dead_passages: 0,
        segments: 0,
        filter_memory_bytes: 0,
        codes_memory_bytes: 0,
        last_batch: None,
    }
}

#[test]
fn a_reader_never_sees_a_configuration_beside_another_state_summary() {
    let directory = tempfile::tempdir().expect("a temporary directory is creatable");
    let index = directory.path().join("index");
    write_index_config(&index, CONFIG).expect("the configuration is written");
    let base = IndexConfig::from_json_str(CONFIG).expect("the fixture configuration is valid");
    let store = GenerationStore::create(&index, INITIAL_EPOCH, &base.schema)
        .expect("the fixture index is created");
    let board = Arc::new(
        JobBoard::open(&directory.path().join("jobs.sqlite"))
            .expect("the job table of the fixture is open"),
    );
    let writer = Arc::new(
        IndexWriter::new("atomic".to_owned(), store, base.clone(), 1, board)
            .expect("the writer of the fixture starts"),
    );
    let snapshot = writer
        .published()
        .expect("the initial triplet is readable")
        .snapshot;
    // The initial pair must satisfy the invariant too, so the reader never
    // reports the state the index starts in.
    writer
        .publish(base.clone(), Arc::clone(&snapshot), summary(10))
        .expect("the initial triplet is publishable");

    let stop = AtomicBool::new(false);
    let (started, ready) = mpsc::channel::<()>();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            started.send(()).expect("the reader is listening");
            for step in 0..50_000_u64 {
                let marker = 1 + step % 2;
                let mut config = base.clone();
                config.search.rescore_candidates =
                    u32::try_from(marker).expect("the marker fits the configuration field");
                writer
                    .publish(config, Arc::clone(&snapshot), summary(marker))
                    .expect("the triplet is publishable");
            }
            stop.store(true, Ordering::Release);
        });
        ready.recv().expect("the writer has started");
        let mut reads = 0_u64;
        loop {
            let published = writer.published().expect("the triplet is readable");
            assert_eq!(
                u64::from(published.config.search.rescore_candidates),
                published.summary.documents,
                "a reader saw a configuration beside a summary of another state"
            );
            reads += 1;
            if stop.load(Ordering::Acquire) {
                break;
            }
        }
        assert!(reads > 1, "the reader must observe the publications");
    });
}

/// Body of an online lot that attaches the document `DOC-<step>`, one passage.
fn batch(step: u64) -> Value {
    json!({
        "batch_id": format!("batch-{step}"),
        "mode": "upsert",
        "documents": [{
            "doc_id": format!("DOC-{step}"),
            "fields": { "numero": format!("{step}") },
            "passages": [{
                "text": "le contrat fait la loi",
                "fields": { "texte": "contrat" },
                "vector": base64_vector([0.6, 0.8, 0.0, 0.0])
            }]
        }]
    })
}

/// `GET /v1/indexes/{index}`, the body of the detail route.
async fn detail(router: &Router) -> Value {
    let path = format!("/v1/indexes/{INDEX}");
    let (status, body) = send(router, Method::GET, &path, READ_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "the index is served: {body}");
    body
}

/// A refresh keeps the configuration, and both HTTP readers see the new
/// snapshot.
///
/// Each online lot commits one generation and attaches one document, so the
/// published triplet must always satisfy `generation == documents`. A reader
/// that saw the generation of one refresh beside the summary of another would
/// break it. The series is bounded, so the loop ends; it is long enough that the
/// reader interleaves with the refreshes.
#[tokio::test]
async fn a_refresh_from_a_real_job_reaches_the_two_http_readers() {
    const BATCHES: u64 = 4;
    let (_instance, state, router) = instance_with_alpha().await;
    let config = detail(&router).await["config"].clone();
    assert_eq!(
        detail(&router).await["state"]["generation"],
        0,
        "the fixture index starts at generation 0"
    );

    let stop = Arc::new(AtomicBool::new(false));
    let reads = Arc::new(AtomicUsize::new(0));
    let reading = {
        let router = router.clone();
        let stop = Arc::clone(&stop);
        let reads = Arc::clone(&reads);
        let config = config.clone();
        tokio::spawn(async move {
            while !stop.load(Ordering::Acquire) {
                let body = detail(&router).await;
                assert_eq!(
                    body["config"], config,
                    "a refresh keeps the configuration: {body}"
                );
                let generation = body["state"]["generation"]
                    .as_u64()
                    .unwrap_or_else(|| panic!("the detail carries its generation: {body}"));
                let documents = body["state"]["documents"]
                    .as_u64()
                    .unwrap_or_else(|| panic!("the detail carries its counts: {body}"));
                assert_eq!(
                    generation, documents,
                    "a reader saw the generation of one refresh beside the summary \
                     of another: {body}"
                );
                reads.fetch_add(1, Ordering::AcqRel);
                tokio::task::yield_now().await;
            }
        })
    };

    for step in 1..=BATCHES {
        let (status, accepted) = post_documents(&router, batch(step)).await;
        assert_eq!(status, StatusCode::OK, "{accepted}");
        let job = terminal_job(&router, &state, &accepted).await;
        assert_eq!(job["state"], "succeeded", "the lot attaches: {job}");
        assert_eq!(
            job["generation"],
            json!(step),
            "the job committed generation {step}: {job}"
        );
        let body = detail(&router).await;
        assert_eq!(
            body["config"], config,
            "the job refreshed the triplet, it did not publish a configuration: {body}"
        );
        for (field, expected) in [
            ("generation", step),
            ("documents", step),
            ("live_passages", step),
        ] {
            assert_eq!(
                body["state"][field],
                json!(expected),
                "the detail serves the refreshed {field}: {body}"
            );
        }
    }

    stop.store(true, Ordering::Release);
    reading.await.expect("the reader never sees a mixture");
    assert!(
        reads.load(Ordering::Acquire) > 1,
        "the reader observed the refreshes"
    );

    // `list_entry` reads the same triplet under one guard: the two readers never
    // disagree.
    let (status, list) = send(&router, Method::GET, "/v1/indexes", READ_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let entry = &list["indexes"][0];
    let body = detail(&router).await;
    assert_eq!(entry["name"], INDEX, "{list}");
    for (listed, detailed) in [
        ("generation", "generation"),
        ("documents", "documents"),
        ("live_passages", "live_passages"),
    ] {
        assert_eq!(
            entry[listed], body["state"][detailed],
            "the list and the detail serve the same triplet: {list} {body}"
        );
    }
    assert_eq!(entry["generation"], json!(BATCHES), "{list}");
}
