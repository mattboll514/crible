// SPDX-License-Identifier: Apache-2.0

// Test fixtures may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixture below builds an index and a job table.
#![allow(clippy::panic)]

//! The triplet `(config, snapshot, summary)` of an index is published together
//! (ADR-0105, decision 4).
//!
//! A `PATCH` job replaces the configuration and publishes a snapshot in the
//! same step. A request must see the old triplet or the new one, never a
//! configuration beside a summary of another state, so the three parts live
//! under one lock and a reader takes one guard
//! ([`IndexWriter::published`]).
//!
//! The test proves the invariant without a duration: one thread publishes pairs
//! whose two halves are equal by construction, the other reads the pair and
//! checks that the halves match. With three separate publications the reader
//! observes a mixture as soon as it interleaves, and the loop is long enough
//! that the interleaving happens.
//!
//! ```text
//! cargo test -p crible-server --test published_state
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use crible_core::config::IndexConfig;
use crible_server::jobs::{IndexSummary, IndexWriter, JobBoard};
use crible_store::generation::{GenerationStore, INITIAL_EPOCH, write_index_config};

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
