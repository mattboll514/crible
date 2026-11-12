// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! The derived table `iid → ordinal de document` equals a full rebuild after
//! every generation (ADR-0154, decisions 5 and 11).
//!
//! References: ADR-0154 (decision 5, the table and its two paths), ADR-0066
//! (a generation derives its structures from the delta of its commit),
//! `docs/03-recherche.md` ("Filtres → pool") and INV-07, whose filters have the
//! same proof: after each generation, the structures `after_commit` derived
//! equal a reconstruction from `records.sqlite`.
//!
//! Only the **count** of distinct documents is compared, never the ordinals
//! themselves: an ordinal is an internal label, and its value among the
//! ordinals of one state is arbitrary. Two tables that label the same documents
//! differently must still count the same pool.
//!
//! | Test | Criterion or invariant |
//! |---|---|
//! | `the_document_ordinals_equal_a_full_rebuild_after_every_generation_and_a_compaction` | ADR-0154, decisions 5 and 11: three generations, a removed document, a re-attach of the same `doc_id` and a compaction, each compared with `DocumentOrdinals::from_records` on the whole live pool and on a restricted one |
//! | `a_pool_passage_without_an_ordinal_is_an_internal_error` | decision 5: the table never undercounts silently; a passage the table does not know is `Internal` |
//!
//! ```text
//! cargo test -p crible-store --test document_ordinals
//! ```

mod common;

use crible_core::config::IndexConfig;
use crible_core::error::ErrorCode;
use crible_core::ids::{Generation, Iid};
use crible_store::document_ordinals::DocumentOrdinals;
use crible_store::generation::{GenerationStore, Snapshot};
use roaring::RoaringBitmap;
use rusqlite::{Connection, OpenFlags};

/// Rebuilds the table of the current reference state, the way an open does.
fn rebuild(store: &GenerationStore) -> DocumentOrdinals {
    let path = store.records_path();
    let connection = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap_or_else(|error| {
            panic!(
                "the reference state {} must open read-only: {error}",
                path.display()
            )
        });
    DocumentOrdinals::from_records(&connection)
        .unwrap_or_else(|error| panic!("the document ordinals must rebuild: {error}"))
}

/// Bitmap of the live passages of `snapshot`: the pool of an empty filter.
fn live_pool(snapshot: &Snapshot) -> RoaringBitmap {
    let mut pool = RoaringBitmap::new();
    for iid in snapshot.live_iids() {
        pool.insert(iid.get());
    }
    pool
}

/// Bitmap of every second live iid: the shape a filter yields, a subset of the
/// live set that still spans several documents.
fn restricted_pool(pool: &RoaringBitmap) -> RoaringBitmap {
    pool.iter().step_by(2).collect()
}

/// Counts the documents of `pool`, panicking on any refusal.
fn count(table: &DocumentOrdinals, pool: &RoaringBitmap) -> u64 {
    table
        .count_documents(pool, || Ok(()))
        .unwrap_or_else(|error| panic!("the document count must succeed: {error}"))
}

/// Asserts that the table of the published snapshot equals a full rebuild, on
/// the whole live pool and on a restricted one.
fn assert_equals_a_rebuild(store: &GenerationStore, generation: u64) -> DocumentOrdinals {
    let snapshot = store.snapshot();
    assert_eq!(
        snapshot.generation(),
        Generation::new(generation),
        "the snapshot must be the one of generation {generation}"
    );
    let held = snapshot.documents_ordinals().clone();
    let rebuilt = rebuild(store);
    assert_eq!(
        held.documents(),
        rebuilt.documents(),
        "the table of generation {generation} must count the documents of a rebuild"
    );
    let pool = live_pool(&snapshot);
    assert_eq!(
        count(&held, &pool),
        count(&rebuilt, &pool),
        "the whole live pool of generation {generation} must count the same documents"
    );
    let restricted = restricted_pool(&pool);
    assert!(
        restrict_is_strict(&restricted, &pool),
        "the restricted pool of generation {generation} must be a strict subset"
    );
    assert_eq!(
        count(&held, &restricted),
        count(&rebuilt, &restricted),
        "a pool under a filter must count the same documents"
    );
    // The ordinals themselves are arbitrary: an ordinal is an internal label,
    // so the test compares the coverage of the pool, not the labels.
    for iid in pool.iter() {
        assert!(
            snapshot
                .documents_ordinals()
                .ordinal(Iid::new(iid))
                .is_some(),
            "every live passage of generation {generation} must have an ordinal"
        );
    }
    held
}

/// Whether `restricted` holds fewer passages than `pool`.
fn restrict_is_strict(restricted: &RoaringBitmap, pool: &RoaringBitmap) -> bool {
    restricted.len() < pool.len() && !restricted.is_empty()
}

#[test]
fn the_document_ordinals_equal_a_full_rebuild_after_every_generation_and_a_compaction() {
    let (_temp, index) = common::temp_index();
    let raw = common::test_config("ordinals", &common::fixture_schema());
    let config = IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("the fixture configuration must be valid: {error}"));
    let mut store = common::create_store_with_config(&index, &common::fixture_schema(), &raw);

    // Generation 1: three documents, one of them with two passages. The
    // identifiers do not follow the reading order, so a table that assumed one
    // would not pass the count by accident.
    let first = common::write_segment(
        &index,
        "seg-a",
        0,
        1,
        &[
            &[0.0, 1.0, 0.0, 0.0],
            &[1.0, 0.0, 0.0, 0.0],
            &[0.0, 0.0, 1.0, 0.0],
            &[0.0, 0.0, 0.0, 1.0],
        ],
    );
    common::commit(
        &mut store,
        common::change(
            vec![first],
            vec![
                common::document("doc-a", 1),
                common::document("doc-b", 1),
                common::document("doc-c", 1),
            ],
            vec![
                common::passage(0, "doc-b", 0, "seg-a", 1),
                common::passage(1, "doc-a", 0, "seg-a", 1),
                common::passage(2, "doc-c", 0, "seg-a", 1),
                common::passage(3, "doc-a", 1, "seg-a", 1),
            ],
            Vec::new(),
        ),
    );
    let table = assert_equals_a_rebuild(&store, 1);
    assert_eq!(table.documents(), 3, "generation 1 holds three documents");

    // Generation 2: a new document and a further passage of an existing one.
    let second = common::write_segment(
        &index,
        "seg-b",
        4,
        2,
        &[&[1.0, 1.0, 0.0, 0.0], &[0.5, 0.5, 0.5, 0.5]],
    );
    common::commit(
        &mut store,
        common::change(
            vec![second],
            vec![common::document("doc-d", 2)],
            vec![
                common::passage(4, "doc-d", 0, "seg-b", 2),
                common::passage(5, "doc-a", 2, "seg-b", 2),
            ],
            Vec::new(),
        ),
    );
    let table = assert_equals_a_rebuild(&store, 2);
    assert_eq!(table.documents(), 4, "generation 2 adds one document");

    // Generation 3: a removed document (its passages are tombstoned, the epoch
    // keeps their rows: ADR-0041) and a new one.
    let third = common::write_segment(&index, "seg-c", 6, 3, &[&[0.0, 1.0, 1.0, 0.0]]);
    common::commit(
        &mut store,
        common::change(
            vec![third],
            vec![common::document("doc-e", 3)],
            vec![common::passage(6, "doc-e", 0, "seg-c", 3)],
            vec![Iid::new(2)],
        ),
    );
    let table = assert_equals_a_rebuild(&store, 3);
    assert_eq!(
        table.documents(),
        5,
        "a tombstone keeps the ordinal of the removed document, as a rebuild does"
    );

    // Generation 4: the same `doc_id` is attached again after its removal. The
    // commit path finds its first passage and reuses the ordinal, so the count
    // does not grow: a new ordinal here would be a document counted twice.
    let fourth = common::write_segment(&index, "seg-d", 7, 4, &[&[0.0, 0.0, 1.0, 1.0]]);
    common::commit(
        &mut store,
        common::change(
            vec![fourth],
            vec![common::document("doc-c", 4)],
            vec![common::passage(7, "doc-c", 0, "seg-d", 4)],
            Vec::new(),
        ),
    );
    let table = assert_equals_a_rebuild(&store, 4);
    assert_eq!(
        table.documents(),
        5,
        "a re-attached `doc_id` must reuse its ordinal"
    );

    // A compaction builds a new epoch from the live passages only, so the rows
    // of the removed document disappear: both the table and a rebuild must
    // shrink to the five documents that still hold a passage.
    let report = crible_store::compact::compact(&mut store, &config)
        .unwrap_or_else(|error| panic!("the fixture index must compact: {error}"));
    assert_eq!(
        report.generation,
        Generation::new(5),
        "a compaction publishes generation 5"
    );
    let table = assert_equals_a_rebuild(&store, 5);
    assert_eq!(
        table.documents(),
        5,
        "the compacted epoch still holds five documents"
    );
}

#[test]
fn a_pool_passage_without_an_ordinal_is_an_internal_error() {
    // A table that does not cover the pool must refuse the count instead of
    // undercounting it: a facet answer with a false `count.documents` would be
    // read as exact (ADR-0154, decision 5).
    let mut pool = RoaringBitmap::new();
    pool.insert(0);
    let failure = DocumentOrdinals::default()
        .count_documents(&pool, || Ok(()))
        .expect_err("a pool outside the table is refused");
    assert_eq!(failure.code(), ErrorCode::Internal, "{}", failure.message());
}
