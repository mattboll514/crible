// SPDX-License-Identifier: Apache-2.0

//! INV-02 — a read is never blocked by a write: a read that runs during a
//! compaction completes without waiting for the compaction to end.
//!
//! References: `docs/08-invariants-et-tests.md` (INV-02),
//! `docs/02-architecture.md` ("Concurrence": one writer per index, readers never
//! blocked, each request takes an `Arc<Snapshot>`; "Suppressions, pierres
//! tombales et compaction", step 1),
//! `docs/adr/0017-compaction-suspend-les-ecritures.md` ("les lectures continuent
//! normalement sur l'ancienne époque, jusqu'à la bascule") and the
//! `COMPACTER-INDEX` task of `docs/phases/P3-cycle-de-vie.md`.
//!
//! The `p99 × 3` part of INV-02 is a load measurement: it belongs to P7 and is
//! not tested here. What these tests pin is the sequencing: while a compaction
//! is held at a known step, a read started before it finishes, and it sees the
//! generation and the payloads of the epoch the compaction is about to replace.
//! The hold comes from the test observer of `crible_store::compact`, so no test
//! waits on a clock.
//!
//! # Assumed API
//!
//! Same assumed API as `inv_14_compaction_retention.rs`, whose module comment
//! gives the full signatures, plus the test hook of this file:
//!
//! ```rust,ignore
//! // crible_store::compact, behind the `failpoints` cargo feature only
//! pub fn compact_with_observer<F>(
//!     store: &mut GenerationStore,
//!     index_config: &IndexConfig,
//!     observer: F,
//! ) -> Result<CompactReport, CribleError>
//! where
//!     F: FnMut(CompactStep);
//!
//! pub enum CompactStep {
//!     WriterSuspended,
//!     EpochWritten,
//!     /// Step 4 done: E+1 is verified, CURRENT_EPOCH still names E.
//!     Verified,
//!     Switched,
//!     Published,
//! }
//! ```
//!
//! The observer runs synchronously inside `compact`, on the caller's thread,
//! once per step and in the order of `docs/02-architecture.md`. A test that
//! blocks in the observer therefore holds the compaction at that exact step.
//!
//! Run this file with:
//!
//! ```text
//! cargo test -p crible-store --features failpoints --test inv_02_compaction_reads
//! ```

#![cfg(feature = "failpoints")]
// The fixtures of this file build an index and call `panic!` outside the
// `#[test]` functions; the workspace lint allows a panic in a test function
// only, as in `common`.
#![allow(clippy::panic)]

mod common;

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use crible_core::config::IndexConfig;
use crible_core::ids::{Epoch, Generation, Iid};
use crible_store::compact::{CompactStep, compact_with_observer};
use crible_store::generation::{GenerationStore, Snapshot};
use crible_store::records::ReadPool;
use serde_json::json;

/// Longest wait of a test for the other side of a rendezvous.
///
/// It is a deadlock watchdog, never a synchronisation delay: a test that has to
/// wait this long has a real bug, and fails with a message that says so.
const DEADLOCK_GUARD: Duration = Duration::from_secs(30);

/// Path of the reference database of an epoch.
fn records_path(index: &std::path::Path, epoch: Epoch) -> std::path::PathBuf {
    index
        .join("epochs")
        .join(epoch.get().to_string())
        .join("records.sqlite")
}

/// Builds the fixture: two generations, four passages, one tombstone.
///
/// Generation 1 writes `seg-a` with `iid 0 = doc-b#0`, `1 = doc-a#0`,
/// `2 = doc-a#1`; generation 2 writes `seg-b` with `iid 3 = doc-c#0` and
/// tombstones `iid 0`. Generation 2 therefore has three live passages,
/// `[1, 2, 3]`, in epoch 1 only.
fn fixture(index: &std::path::Path) -> (GenerationStore, IndexConfig) {
    let schema = common::fixture_schema();
    let raw = common::test_config("compaction-reads", &schema);
    let config = IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("the fixture configuration must be valid: {error}"));
    crible_store::generation::write_index_config(index, &raw)
        .unwrap_or_else(|error| panic!("the fixture index configuration must be written: {error}"));
    let mut store = GenerationStore::create(index, Epoch::new(common::EPOCH), &schema)
        .unwrap_or_else(|error| panic!("the fixture index must be created: {error}"));

    let first = common::write_segment(
        index,
        "seg-a",
        0,
        1,
        &[
            &[0.0, 1.0, 0.0, 0.0],
            &[1.0, 0.0, 0.0, 0.0],
            &[0.0, 0.0, 1.0, 0.0],
        ],
    );
    common::commit(
        &mut store,
        common::change(
            vec![first],
            vec![common::document("doc-a", 1), common::document("doc-b", 1)],
            vec![
                common::passage(0, "doc-b", 0, "seg-a", 1),
                common::passage(1, "doc-a", 0, "seg-a", 1),
                common::passage(2, "doc-a", 1, "seg-a", 1),
            ],
            Vec::new(),
        ),
    );

    let second = common::write_segment(index, "seg-b", 3, 2, &[&[0.5, 0.5, 0.5, 0.5]]);
    common::commit(
        &mut store,
        common::change(
            vec![second],
            vec![common::document("doc-c", 2)],
            vec![common::passage(3, "doc-c", 0, "seg-b", 2)],
            vec![Iid::new(0)],
        ),
    );
    (store, config)
}

/// What a "search" of the fixture read while the compaction was held.
struct ReadOutcome {
    generation: Generation,
    keys: Vec<String>,
    texts: Vec<String>,
}

/// Waits for the compaction to reach the observed step, then reads the held
/// snapshot and the payloads of epoch 1 the way a search does, and sends the
/// result back.
///
/// The thread only touches what a real request touches: an `Arc<Snapshot>` taken
/// before the compaction, and a read-only pool on the `records.sqlite` of that
/// snapshot's epoch (ADR-0043 point 4). It never borrows the store, so the
/// compaction owning `&mut GenerationStore` cannot block it.
fn search_during_compaction(
    paused: Receiver<()>,
    done: Sender<ReadOutcome>,
    snapshot: std::sync::Arc<Snapshot>,
    pool: ReadPool,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        paused.recv_timeout(DEADLOCK_GUARD).unwrap_or_else(|error| {
            panic!("the compaction never reached the observed step: {error}")
        });
        let mut iids: Vec<u32> = snapshot.live_iids().map(Iid::get).collect();
        iids.sort_unstable();
        let keys: Vec<String> = iids
            .iter()
            .map(|iid| {
                pool.read_passage_key(Iid::new(*iid))
                    .unwrap_or_else(|error| panic!("iid {iid} must be readable: {error}"))
                    .unwrap_or_else(|| panic!("iid {iid} must exist in the held epoch"))
                    .to_string()
            })
            .collect();
        let texts: Vec<String> = iids
            .iter()
            .map(|iid| {
                pool.read_passage(Iid::new(*iid))
                    .unwrap_or_else(|error| {
                        panic!("the payload of iid {iid} must be readable: {error}")
                    })
                    .unwrap_or_else(|| panic!("iid {iid} must exist in the held epoch"))
                    .text
            })
            .collect();
        done.send(ReadOutcome {
            generation: snapshot.generation(),
            keys,
            texts,
        })
        .unwrap_or_else(|error| panic!("the compaction stopped listening for the read: {error}"));
    })
}

/// A read that starts before the compaction and runs while it is held at its
/// last step completes before the compaction ends, and sees generation 2 whole.
///
/// INV-02, and ADR-0017: "les lectures continuent normalement sur l'ancienne
/// époque, jusqu'à la bascule".
#[test]
fn inv_02_reads_during_compaction_complete_without_waiting() {
    // The failpoint variable is process-global: every test of this binary that
    // compacts takes the same guard, armed or not.
    let _serial = common::SerialGuard::acquire();
    let (_temp, index) = common::temp_index();
    let (mut store, config) = fixture(&index);

    let (paused_tx, paused_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let reader = search_during_compaction(
        paused_rx,
        done_tx,
        store.snapshot(),
        ReadPool::new(&records_path(&index, Epoch::new(common::EPOCH)), 2)
            .unwrap_or_else(|error| panic!("the held epoch read pool must open: {error}")),
    );

    let report = compact_with_observer(&mut store, &config, |step| {
        if !matches!(step, CompactStep::Verified) {
            return;
        }
        paused_tx.send(()).unwrap_or_else(|error| {
            panic!("the reader vanished before the compaction paused: {error}")
        });
        // The compaction is held here, before the switch: the read must come
        // back on its own, not because the compaction released a lock.
        let outcome = match done_rx.recv_timeout(DEADLOCK_GUARD) {
            Ok(outcome) => outcome,
            Err(RecvTimeoutError::Timeout) => panic!(
                "the read did not complete while the compaction was held at \
                 CompactStep::Verified: the compaction blocks the readers (INV-02)"
            ),
            Err(RecvTimeoutError::Disconnected) => panic!(
                "the reader panicked instead of completing its read during the \
                 compaction (INV-02)"
            ),
        };
        assert_eq!(
            outcome.generation,
            Generation::new(2),
            "a read started before the compaction must see the generation it began \
             on, not a half-switched one"
        );
        assert_eq!(
            outcome.keys,
            vec!["doc-a#0", "doc-a#1", "doc-c#0"],
            "the held snapshot must still see its live passages, in iid order"
        );
        assert_eq!(outcome.texts, vec!["doc-a#0", "doc-a#1", "doc-c#0"]);
    })
    .unwrap_or_else(|error| panic!("the observed compaction must succeed: {error}"));

    reader
        .join()
        .unwrap_or_else(|_| panic!("the reader thread must finish"));
    assert_eq!(report.to_epoch, Epoch::new(2));
    assert_eq!(store.epoch(), Epoch::new(2));
    assert_eq!(store.generation(), Generation::new(3));
}

/// The payload read of a search, on the `records.sqlite` of the epoch its
/// snapshot holds, works while the compaction writes and verifies the new
/// epoch.
///
/// INV-02: the payload step never waits on the writer, which ADR-0043 point 4
/// pins to "le `records.sqlite` de l'époque du snapshot".
#[test]
fn inv_02_payload_reads_on_held_epoch_during_compaction() {
    let _serial = common::SerialGuard::acquire();
    let (_temp, index) = common::temp_index();
    let (mut store, config) = fixture(&index);

    let (paused_tx, paused_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let pool = ReadPool::new(&records_path(&index, Epoch::new(common::EPOCH)), 2)
        .unwrap_or_else(|error| panic!("the held epoch read pool must open: {error}"));
    let reader = thread::spawn(move || {
        paused_rx
            .recv_timeout(DEADLOCK_GUARD)
            .unwrap_or_else(|error| {
                panic!("the compaction never reached the observed step: {error}")
            });
        let document = pool
            .read_document("doc-a")
            .unwrap_or_else(|error| panic!("the payload of doc-a must be readable: {error}"))
            .unwrap_or_else(|| panic!("doc-a must exist in the held epoch"));
        done_tx.send(document.fields).unwrap_or_else(|error| {
            panic!("the compaction stopped listening for the read: {error}")
        });
    });

    compact_with_observer(&mut store, &config, |step| {
        if !matches!(step, CompactStep::Verified) {
            return;
        }
        paused_tx.send(()).unwrap_or_else(|error| {
            panic!("the reader vanished before the compaction paused: {error}")
        });
        let fields = match done_rx.recv_timeout(DEADLOCK_GUARD) {
            Ok(fields) => fields,
            Err(RecvTimeoutError::Timeout) => panic!(
                "the payload read did not complete while the compaction was held at \
                 CompactStep::Verified: the compaction locks the readers out of the \
                 held epoch (INV-02)"
            ),
            Err(RecvTimeoutError::Disconnected) => panic!(
                "the reader panicked instead of completing its payload read during \
                 the compaction (INV-02)"
            ),
        };
        assert_eq!(
            fields,
            json!({ "title": "doc-a" }),
            "the payload must come from the held epoch, unchanged"
        );
    })
    .unwrap_or_else(|error| panic!("the observed compaction must succeed: {error}"));

    reader
        .join()
        .unwrap_or_else(|_| panic!("the reader thread must finish"));
    assert_eq!(store.epoch(), Epoch::new(2));
}
