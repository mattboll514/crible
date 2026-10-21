// SPDX-License-Identifier: Apache-2.0

//! INV-03 — a generation is visible entirely or not at all.
//!
//! References: `docs/08-invariants-et-tests.md` (INV-03),
//! `docs/02-architecture.md` ("Générations et point de commit", "Concurrence"),
//! `docs/adr/0016-point-commit-sqlite.md` (ADR-0016 — the SQLite commit is the
//! only commit point) and the `GERER-GENERATIONS` task of
//! `docs/phases/P1-stockage-et-recherche-exacte.md`.
//!
//! The tests pin the visibility rule: a reader that takes a snapshot keeps a
//! coherent view of the generation it started on while a commit publishes the
//! next one; a reader that takes a new snapshot sees the new generation whole,
//! tombstone included. A refused commit publishes nothing.
//!
//! # Assumed API
//!
//! `crible_store::generation` does not exist yet. These tests are written
//! against the smallest API that the task sheet and `docs/02-architecture.md`
//! allow. `@dev` may adjust it, but must then adjust these tests and say so.
//!
//! ```rust,ignore
//! // crible_store::generation
//!
//! pub struct GenerationStore {
//!     // owns the index directory, the writable `RecordStore` of the epoch,
//!     // and the `ArcSwap<Snapshot>` of the current generation
//! }
//!
//! impl GenerationStore {
//!     /// Creates `index_dir`, `CURRENT_EPOCH`, `epochs/<epoch>/records.sqlite`
//!     /// and the snapshot of generation 0.
//!     pub fn create(index_dir: &Path, epoch: Epoch) -> Result<Self, CribleError>;
//!
//!     /// Reads `CURRENT_EPOCH`, opens the epoch and runs the recovery of
//!     /// INV-04, then publishes the snapshot of the committed generation.
//!     pub fn open(index_dir: &Path) -> Result<Self, CribleError>;
//!
//!     pub fn epoch(&self) -> Epoch;
//!     pub fn generation(&self) -> Generation;
//!
//!     /// `ArcSwap::load_full`: the caller holds `Arc<Snapshot>` for the whole
//!     /// request.
//!     pub fn snapshot(&self) -> Arc<Snapshot>;
//!
//!     /// One generation in one SQLite transaction: `batches`, `segments`,
//!     /// `documents`, `passages`, the tombstones, `meta.generation = G + 1`
//!     /// and `meta.next_iid`. Returns the committed generation. Any error
//!     /// rolls the transaction back and publishes no snapshot.
//!     pub fn commit(&mut self, change: GenerationChange) -> Result<Generation, CribleError>;
//! }
//!
//! pub struct GenerationChange {
//!     pub segments: Vec<SegmentMeta>,   // `crible_store::segment::SegmentMeta`
//!     pub documents: Vec<DocumentRecord>,
//!     pub passages: Vec<PassageRecord>,
//!     pub deletions: Vec<Iid>,          // iids tombstoned at the new generation
//! }
//!
//! pub struct Snapshot {
//!     // `Send + Sync`, held by `Arc`
//! }
//!
//! impl Snapshot {
//!     pub fn generation(&self) -> Generation;
//!     pub fn segments(&self) -> &[SegmentMeta];
//!     /// iids alive in this generation, sorted: tombstones of the generation
//!     /// and of the previous ones are excluded.
//!     pub fn live_iids(&self) -> &[Iid];
//! }
//! ```
//!
//! The tests also assume that `commit` rejects a passage whose `iid` is
//! already used (primary-key conflict), and that the transaction is rolled
//! back on that error, as ADR-0016 requires.

mod common;

use std::sync::Arc;

use crible_core::error::ErrorCode;
use crible_core::ids::Iid;

/// Commits generation 1: segment `seg-a` with two passages of two documents.
fn commit_first_generation(
    store: &mut crible_store::generation::GenerationStore,
    index: &std::path::Path,
) {
    let segment = common::write_segment(
        index,
        "seg-a",
        0,
        1,
        &[&[0.0, 1.0, 0.0, 0.0], &[1.0, 0.0, 0.0, 0.0]],
    );
    common::commit(
        store,
        common::change(
            vec![segment],
            vec![common::document("doc-a", 1), common::document("doc-b", 1)],
            vec![
                common::passage(0, "doc-a", 0, "seg-a", 1),
                common::passage(1, "doc-b", 0, "seg-a", 1),
            ],
            Vec::new(),
        ),
    );
}

/// Commits generation 2: segment `seg-b`, document `doc-c`, and the tombstone
/// of iid 0. A reader on generation 1 must still see `[0, 1]`; a reader on
/// generation 2 must see `[1, 2]`.
fn commit_second_generation(
    store: &mut crible_store::generation::GenerationStore,
    index: &std::path::Path,
) {
    let segment = common::write_segment(index, "seg-b", 2, 2, &[&[0.0, 0.0, 1.0, 0.0]]);
    common::commit(
        store,
        common::change(
            vec![segment],
            vec![common::document("doc-c", 2)],
            vec![common::passage(2, "doc-c", 0, "seg-b", 2)],
            vec![Iid::new(0)],
        ),
    );
}

/// A snapshot taken before a commit keeps describing the old generation in
/// full, after the commit published the next one.
#[test]
fn inv_03_held_snapshot_keeps_previous_generation_after_commit() {
    let (_temp, index) = common::temp_index();
    let mut store = common::create_store(&index);
    assert_eq!(store.epoch().get(), common::EPOCH);
    assert_eq!(store.generation().get(), 0);

    commit_first_generation(&mut store, &index);
    let first = store.snapshot();
    common::assert_view(&first, 1, &["seg-a"], &[0, 1]);

    commit_second_generation(&mut store, &index);

    // The held snapshot is frozen on generation 1: old segment set, iid 0
    // still alive, iid 2 still absent.
    common::assert_view(&first, 1, &["seg-a"], &[0, 1]);

    // A new snapshot sees generation 2 entirely: both segments, the tombstone
    // of iid 0 applied, the new passage visible.
    let second = store.snapshot();
    common::assert_view(&second, 2, &["seg-a", "seg-b"], &[1, 2]);
    assert!(
        !Arc::ptr_eq(&first, &second),
        "a commit must publish a new snapshot, not mutate the one readers hold"
    );
}

/// A reader that holds a snapshot while another thread commits sees the old
/// generation; no sleep, the two synchronise on channels.
#[test]
fn inv_03_reader_holding_snapshot_does_not_see_concurrent_commit() {
    use std::sync::mpsc;

    let (_temp, index) = common::temp_index();
    let mut store = common::create_store(&index);
    commit_first_generation(&mut store, &index);

    let snapshot = store.snapshot();
    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let reader = std::thread::spawn(move || {
        let held = snapshot;
        ready_tx.send(()).unwrap_or_else(|error| {
            panic!("the reader must signal that it holds the snapshot: {error}")
        });
        go_rx.recv().unwrap_or_else(|error| {
            panic!("the reader must be released after the commit: {error}")
        });
        // The commit is over when this runs, yet the held snapshot is intact.
        common::assert_view(&held, 1, &["seg-a"], &[0, 1]);
    });

    ready_rx.recv().unwrap_or_else(|error| {
        panic!("the main thread waits for the reader to be ready: {error}")
    });
    commit_second_generation(&mut store, &index);
    go_tx
        .send(())
        .unwrap_or_else(|error| panic!("the main thread releases the reader: {error}"));
    if let Err(payload) = reader.join() {
        std::panic::resume_unwind(payload);
    }

    common::assert_view(&store.snapshot(), 2, &["seg-a", "seg-b"], &[1, 2]);
}

/// A commit refused by SQLite changes neither the generation nor the snapshot,
/// and leaves no trace after a restart.
#[test]
fn inv_03_failed_commit_leaves_generation_and_snapshot_unchanged() {
    let (_temp, index) = common::temp_index();
    let mut store = common::create_store(&index);
    commit_first_generation(&mut store, &index);
    let before = store.snapshot();

    // A second segment is written and renamed before the commit, as an attach
    // does. Reusing iid 0 makes the transaction fail on the primary key.
    let segment = common::write_segment(&index, "seg-b", 2, 2, &[&[0.0, 0.0, 1.0, 0.0]]);
    let rejected = common::change(
        vec![segment],
        vec![common::document("doc-c", 2)],
        vec![common::passage(0, "doc-c", 0, "seg-b", 2)],
        Vec::new(),
    );
    let outcome = store.commit(rejected);
    assert!(
        outcome.is_err(),
        "a passage reusing an existing iid must be refused by the commit transaction"
    );

    assert!(
        Arc::ptr_eq(&before, &store.snapshot()),
        "a refused commit must not publish a new snapshot"
    );
    common::assert_view(&store.snapshot(), 1, &["seg-a"], &[0, 1]);

    drop(store);
    let reopened = common::open_store(&index);
    common::assert_view(&reopened.snapshot(), 1, &["seg-a"], &[0, 1]);
    // `seg-b` was never referenced by SQLite: recovery removes it.
    common::assert_segment_directories(&index, &["seg-a"]);
}

/// The snapshot and the reference database agree: the live iids and the
/// segments of one generation come from the same committed state.
#[test]
fn inv_03_snapshot_matches_committed_rows() {
    let (_temp, index) = common::temp_index();
    let mut store = common::create_store(&index);
    commit_first_generation(&mut store, &index);
    commit_second_generation(&mut store, &index);

    let connection =
        rusqlite::Connection::open(common::records_path(&index)).unwrap_or_else(|error| {
            panic!("the reference database of the epoch must be readable: {error}")
        });
    let mut live_statement = connection
        .prepare("SELECT iid FROM passages WHERE dead_since_gen IS NULL ORDER BY iid")
        .unwrap_or_else(|error| panic!("the live passages must be queryable: {error}"));
    let mut from_sql: Vec<u32> = live_statement
        .query_map([], |row| row.get::<_, u32>(0))
        .unwrap_or_else(|error| panic!("the live iids must be readable: {error}"))
        .map(|iid| iid.unwrap_or_else(|error| panic!("one live iid must be readable: {error}")))
        .collect();
    live_statement
        .finalize()
        .unwrap_or_else(|error| panic!("the live statement must be finalizable: {error}"));
    from_sql.sort_unstable();

    let snapshot = store.snapshot();
    let mut from_snapshot: Vec<u32> = snapshot.live_iids().iter().map(|iid| iid.get()).collect();
    from_snapshot.sort_unstable();
    assert_eq!(
        from_snapshot, from_sql,
        "a snapshot must expose exactly the passages alive in the generation it names"
    );

    let mut segment_statement = connection
        .prepare("SELECT segment_id FROM segments ORDER BY segment_id")
        .unwrap_or_else(|error| panic!("the segments must be queryable: {error}"));
    let mut segments_from_sql: Vec<String> = segment_statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap_or_else(|error| panic!("the segment ids must be readable: {error}"))
        .map(|id| id.unwrap_or_else(|error| panic!("one segment id must be readable: {error}")))
        .collect();
    segment_statement
        .finalize()
        .unwrap_or_else(|error| panic!("the segment statement must be finalizable: {error}"));
    segments_from_sql.sort_unstable();

    let mut segments_from_snapshot: Vec<String> = snapshot
        .segments()
        .iter()
        .map(|meta| meta.segment_id.clone())
        .collect();
    segments_from_snapshot.sort_unstable();
    assert_eq!(
        segments_from_snapshot, segments_from_sql,
        "a snapshot must reference exactly the segments committed for its generation"
    );
}

/// A change whose rows name another generation is refused before the commit
/// point, and publishes nothing: no visible state may carry a generation number
/// that no commit produced.
#[test]
fn inv_03_commit_refuses_rows_of_another_generation() {
    let (_temp, index) = common::temp_index();
    let mut store = common::create_store(&index);
    commit_first_generation(&mut store, &index);
    let before = store.snapshot();

    // An attach that believes it commits generation 7 prepares this segment,
    // while the store is about to publish generation 2.
    let segment = common::write_segment(&index, "seg-late", 2, 7, &[&[0.0, 0.0, 1.0, 0.0]]);
    let outcome = store.commit(common::change(
        vec![segment],
        Vec::new(),
        vec![common::passage(2, "doc-c", 0, "seg-late", 2)],
        Vec::new(),
    ));
    let error = outcome.expect_err("a segment of another generation must be refused");
    assert_eq!(
        error.code(),
        ErrorCode::Internal,
        "a row of another generation is an internal inconsistency: {}",
        error.message()
    );
    assert!(
        Arc::ptr_eq(&before, &store.snapshot()),
        "a refused commit must not publish a new snapshot"
    );
    common::assert_view(&store.snapshot(), 1, &["seg-a"], &[0, 1]);

    drop(store);
    let reopened = common::open_store(&index);
    common::assert_view(&reopened.snapshot(), 1, &["seg-a"], &[0, 1]);
    common::assert_segment_directories(&index, &["seg-a"]);
}
