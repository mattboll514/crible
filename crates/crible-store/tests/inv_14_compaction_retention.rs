// SPDX-License-Identifier: Apache-2.0

//! INV-14 — nothing that cost GPU time is destroyed without a valid copy: the
//! compaction keeps the epoch E until `CURRENT_EPOCH` switches, and a crash
//! during the compaction leaves E intact and usable.
//!
//! References: `docs/08-invariants-et-tests.md` (INV-14),
//! `docs/02-architecture.md` ("Suppressions, pierres tombales et compaction",
//! the six steps of a compaction; "Concurrence"),
//! `docs/adr/0006-segments-pierres-tombales-compaction.md`,
//! `docs/adr/0017-compaction-suspend-les-ecritures.md` and
//! `docs/adr/0043-payload-d-un-resultat-dont-le-document-est-supprime.md`
//! (point 4: an epoch is never deleted while a snapshot holds it), and the
//! `COMPACTER-INDEX` task of `docs/phases/P3-cycle-de-vie.md`.
//!
//! The tests also cover the two acceptance criteria of the task sheet that are
//! about the outcome of a successful compaction: no dead passage remains, and
//! the internal identifiers are contiguous and ordered by (`cluster_by`,
//! `doc_id`, `ord`).
//!
//! # API under test
//!
//! `crible_store::compact` implements this API; the contract below is the one the
//! tests rely on, and it follows the task sheet and the design.
//!
//! ```rust,ignore
//! // crible_store::compact
//!
//! /// Builds epoch E+1 without the dead passages: suspends the writer, reads the
//! /// live passages of the published snapshot sorted by (`index_config.cluster_by`,
//! /// `doc_id`, `ord`), writes the segments (at most 1 M passages each), a new
//! /// `records.sqlite` keeping the `batches` history, verifies counts and
//! /// checksums, renames `CURRENT_EPOCH` to E+1 (the switch, generation G+1),
//! /// publishes the snapshot, then deletes E when no snapshot holds it.
//! ///
//! /// `index_config` is the configuration the index declares, passed by the
//! /// caller the way `attach` takes it, so the compaction reads `cluster_by`
//! /// from the same authority as the rest of the pipeline. Reading `index.json`
//! /// itself would be the other acceptable choice.
//! ///
//! /// An error raised before the `CURRENT_EPOCH` rename leaves E current, whole
//! /// and usable (INV-14). Nothing of E+1 stays reachable: a later `open`
//! /// recovers E, and a later `compact` converges to a complete E+1.
//! pub fn compact(
//!     store: &mut GenerationStore,
//!     index_config: &IndexConfig,
//! ) -> Result<CompactReport, CribleError>;
//!
//! /// Deletes every epoch directory that `CURRENT_EPOCH` does not name and that
//! /// no live `Arc<Snapshot>` holds, then returns the deleted epochs in
//! /// ascending order. A compaction calls it too, but it can never delete the
//! /// epoch a reader still holds.
//! pub fn reclaim_epochs(store: &GenerationStore) -> Result<Vec<Epoch>, CribleError>;
//!
//! pub struct CompactReport {
//!     /// Epoch the compaction started from.
//!     pub from_epoch: Epoch,
//!     /// Epoch that `CURRENT_EPOCH` names when the call returns.
//!     pub to_epoch: Epoch,
//!     /// Generation published when the call returns: G+1.
//!     pub generation: Generation,
//!     /// Passages the new epoch holds.
//!     pub live_passages: u64,
//!     /// Passages of E that the compaction did not copy: the tombstones.
//!     pub removed_passages: u64,
//! }
//!
//! /// Progress of a compaction, for the observer below and for the log.
//! pub enum CompactStep {
//!     /// Step 1: the writer is suspended, the reads continue.
//!     WriterSuspended,
//!     /// Step 3: the segments and the new `records.sqlite` of E+1 are written.
//!     EpochWritten,
//!     /// Step 4: counts and checksums of E+1 are verified.
//!     Verified,
//!     /// Step 5: `CURRENT_EPOCH` names E+1; E is still on disk.
//!     Switched,
//!     /// Step 6: the snapshot of E+1 is published.
//!     Published,
//! }
//!
//! /// Same as [`compact`], and calls `observer` at each step. Test hook: it only
//! /// exists with the `failpoints` cargo feature, never in a release build. It is
//! /// what lets a test hold a compaction at a known step instead of sleeping.
//! #[cfg(feature = "failpoints")]
//! pub fn compact_with_observer<F>(
//!     store: &mut GenerationStore,
//!     index_config: &IndexConfig,
//!     observer: F,
//! ) -> Result<CompactReport, CribleError>
//! where
//!     F: FnMut(CompactStep);
//! ```
//!
//! # Assumed failpoints
//!
//! The `failpoints` feature already used by `inv_04_recovery.rs`, extended to
//! the compaction steps, with the same protocol: `CRIBLE_FAILPOINT=<step>` makes
//! the store panic with a message containing `CRIBLE_FAILPOINT` when it reaches
//! that step, and the unwind rolls back what the transaction holds.
//!
//! | Step | Position |
//! |---|---|
//! | `before_epoch_switch` | E+1 written and verified, `CURRENT_EPOCH` still names E (step 4 → 5) |
//! | `after_epoch_switch` | `CURRENT_EPOCH` names E+1, nothing published and E not deleted (step 5 → 6) |
//!
//! Run the whole file with:
//!
//! ```text
//! cargo test -p crible-store --features failpoints --test inv_14_compaction_retention
//! ```

// The fixtures of this file build an index and call `panic!` outside the
// `#[test]` functions; the workspace lint allows a panic in a test function
// only, as in `common`.
#![allow(clippy::panic)]

mod common;

use std::path::{Path, PathBuf};

use crible_core::config::IndexConfig;
#[cfg(feature = "failpoints")]
use crible_core::error::ErrorCode;
use crible_core::ids::{Epoch, Generation, Iid};
use crible_store::generation::{GenerationStore, Snapshot};
use crible_store::records::{DocumentRecord, ReadPool};
use serde_json::json;

/// Second epoch, the one a compaction of the first epoch creates.
const NEXT_EPOCH: Epoch = Epoch::new(2);

/// Generation of the fixture before any compaction: two committed changes.
const FIXTURE_GENERATION: Generation = Generation::new(2);

/// Directory of an epoch, whatever its number.
fn epoch_dir(index: &Path, epoch: Epoch) -> PathBuf {
    index.join("epochs").join(epoch.get().to_string())
}

/// Path of the reference database of an epoch.
fn records_path(index: &Path, epoch: Epoch) -> PathBuf {
    epoch_dir(index, epoch).join("records.sqlite")
}

/// Configuration of the fixture index, parsed from the JSON the fixture writes.
fn fixture_config(raw: &str) -> IndexConfig {
    IndexConfig::from_json_str(raw)
        .unwrap_or_else(|error| panic!("the fixture configuration must be valid: {error}"))
}

/// The fixture configuration of `common::test_config_with_cluster_by`.
fn clustered_config_json(fields: &[&str]) -> String {
    common::test_config_with_cluster_by("compaction", &common::fixture_schema(), fields)
}

/// One document whose `title` is `title`, at the given generation.
fn titled_document(doc_id: &str, title: &str, generation: u64) -> DocumentRecord {
    DocumentRecord {
        doc_id: doc_id.to_owned(),
        doc_hash: format!("blake3:doc-{doc_id}"),
        fields: json!({ "title": title }),
        updated_generation: Generation::new(generation),
    }
}

/// Live identifiers of a snapshot, sorted.
fn live_iids(snapshot: &Snapshot) -> Vec<u32> {
    let mut iids: Vec<u32> = snapshot.live_iids().map(Iid::get).collect();
    iids.sort_unstable();
    iids
}

/// Creates the fixture index from the JSON configuration `raw`, whose schema is
/// the fixture schema.
fn create_fixture_index(index: &Path, raw: &str) -> GenerationStore {
    let schema = common::fixture_schema();
    crible_store::generation::write_index_config(index, raw)
        .unwrap_or_else(|error| panic!("the fixture index configuration must be written: {error}"));
    GenerationStore::create(index, Epoch::new(common::EPOCH), &schema)
        .unwrap_or_else(|error| panic!("the fixture index must be created: {error}"))
}

/// Builds the fixture index: two generations, six passages, one tombstone.
///
/// Generation 1 writes `seg-a` with four passages whose identifiers do not
/// follow the reading order, so a compaction that only copies them cannot pass
/// the ordering test by accident:
///
/// | iid | key | state at generation 2 |
/// |---|---|---|
/// | 0 | `doc-b#0` | alive |
/// | 1 | `doc-a#0` | **tombstoned at generation 2** |
/// | 2 | `doc-c#0` | alive |
/// | 3 | `doc-a#1` | alive |
///
/// Generation 2 writes `seg-b` (two passages) and tombstones `iid 1`:
/// `iid 4` is `doc-d#0`, `iid 5` is `doc-a#2`.
///
/// `titles` gives the `title` of each document, in `doc-a`, `doc-b`, `doc-c`,
/// `doc-d` order; the default of the ordering tests is the document identifier.
fn populated_index(index: &Path, raw: &str, titles: [&str; 4]) -> GenerationStore {
    let mut store = create_fixture_index(index, raw);

    let first = common::write_segment(
        index,
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
                titled_document("doc-a", titles[0], 1),
                titled_document("doc-b", titles[1], 1),
                titled_document("doc-c", titles[2], 1),
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

    let second = common::write_segment(
        index,
        "seg-b",
        4,
        2,
        &[&[1.0, 1.0, 0.0, 0.0], &[0.5, 0.5, 0.5, 0.5]],
    );
    common::commit(
        &mut store,
        common::change(
            vec![second],
            vec![titled_document("doc-d", titles[3], 2)],
            vec![
                common::passage(4, "doc-d", 0, "seg-b", 2),
                common::passage(5, "doc-a", 2, "seg-b", 2),
            ],
            vec![Iid::new(1)],
        ),
    );
    store
}

/// The fixture index whose documents are titled after their own identifier, the
/// configuration declaring no `cluster_by`.
fn default_fixture(index: &Path, raw: &str) -> GenerationStore {
    populated_index(index, raw, ["doc-a", "doc-b", "doc-c", "doc-d"])
}

/// Reads the passage keys of the first `count` identifiers of an epoch, in
/// ascending `iid` order.
fn keys_of_epoch(index: &Path, epoch: Epoch, count: u32) -> Vec<String> {
    let pool = ReadPool::new(&records_path(index, epoch), 2)
        .unwrap_or_else(|error| panic!("the fixture read pool must open: {error}"));
    (0..count)
        .map(|iid| {
            pool.read_passage_key(Iid::new(iid))
                .unwrap_or_else(|error| panic!("iid {iid} must be readable: {error}"))
                .unwrap_or_else(|| panic!("iid {iid} must exist in epoch {}", epoch.get()))
                .to_string()
        })
        .collect()
}

/// Reads one passage of an epoch through a read-only pool, the way the payload
/// step does.
fn read_passage_of_epoch(
    index: &Path,
    epoch: Epoch,
    iid: u32,
) -> crible_store::records::PassageRecord {
    let pool = ReadPool::new(&records_path(index, epoch), 2).unwrap_or_else(|error| {
        panic!("the read pool of epoch {} must open: {error}", epoch.get())
    });
    pool.read_passage(Iid::new(iid))
        .unwrap_or_else(|error| {
            panic!(
                "iid {iid} of epoch {} must be readable: {error}",
                epoch.get()
            )
        })
        .unwrap_or_else(|| panic!("iid {iid} must exist in epoch {}", epoch.get()))
}

/// Compacts the fixture index, panicking on any error.
fn compact(
    store: &mut GenerationStore,
    config: &IndexConfig,
) -> crible_store::compact::CompactReport {
    crible_store::compact::compact(store, config)
        .unwrap_or_else(|error| panic!("the fixture compaction must succeed: {error}"))
}

/// Runs a compaction that must panic at the armed failpoint, and checks that the
/// panic really comes from the failpoint and not from another bug.
#[cfg(feature = "failpoints")]
fn compact_expecting_failpoint(store: &mut GenerationStore, config: &IndexConfig) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crible_store::compact::compact(store, config)
    }));
    match outcome {
        Ok(Ok(report)) => panic!(
            "CRIBLE_FAILPOINT was armed but the compaction completed epoch {}; \
             the failpoint is not implemented",
            report.to_epoch.get()
        ),
        Ok(Err(error)) => panic!(
            "the compaction returned {error} instead of hitting the armed failpoint; \
             the failpoint must be reachable on the success path"
        ),
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .map(|text| (*text).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<a panic payload that is not a string>".to_owned());
            assert!(
                message.contains("CRIBLE_FAILPOINT"),
                "the compaction panicked, but not at a failpoint: {message}"
            );
        }
    }
}

/// After a successful compaction, no passage of the new epoch is dead, and the
/// report accounts for every passage of the old one.
///
/// Acceptance criterion of `COMPACTER-INDEX`: "après la compaction, il ne reste
/// plus aucun passage mort".
#[test]
fn compaction_leaves_no_dead_passage() {
    // The failpoint variable is process-global: every test of this binary that
    // compacts takes the same guard, armed or not.
    let _serial = common::SerialGuard::acquire();
    let (_temp, index) = common::temp_index();
    let raw = common::test_config("compaction", &common::fixture_schema());
    let config = fixture_config(&raw);
    let mut store = default_fixture(&index, &raw);

    let before = store
        .state()
        .unwrap_or_else(|error| panic!("the state of the fixture index must be readable: {error}"));
    assert_eq!(
        (before.live_passages, before.dead_passages),
        (5, 1),
        "the fixture must start with five live passages and one tombstone"
    );

    let report = compact(&mut store, &config);

    assert_eq!(report.from_epoch, Epoch::new(common::EPOCH));
    assert_eq!(report.to_epoch, NEXT_EPOCH);
    assert_eq!(report.generation, Generation::new(3));
    assert_eq!(
        report.live_passages, 5,
        "the five live passages of generation 2 must be copied"
    );
    assert_eq!(
        report.removed_passages, 1,
        "the tombstoned passage must be the only one left behind"
    );

    let after = store.state().unwrap_or_else(|error| {
        panic!("the state of the compacted index must be readable: {error}")
    });
    assert_eq!(after.epoch, NEXT_EPOCH);
    assert_eq!(after.generation, Generation::new(3));
    assert_eq!(
        after.dead_passages, 0,
        "no row of the new epoch may carry a tombstone: the dead passage is gone, \
         not carried over"
    );
    assert_eq!(after.live_passages, 5);
    assert_eq!(
        after.documents, 4,
        "the compaction copies the document rows"
    );
    assert_eq!(
        common::read_current_epoch(&index),
        NEXT_EPOCH.get(),
        "the compaction must switch CURRENT_EPOCH to the new epoch"
    );
    assert_eq!(
        store.records_path(),
        records_path(&index, NEXT_EPOCH),
        "the store must read and write the new epoch after the switch"
    );
    let total: u64 = after
        .segments
        .iter()
        .map(|segment| u64::from(segment.count))
        .sum();
    assert_eq!(
        total, 5,
        "the segments of the new epoch must hold every live passage exactly once"
    );
    assert!(
        epoch_dir(&index, NEXT_EPOCH)
            .join("manifests")
            .join("3.json")
            .exists(),
        "the compaction publishes generation 3, so its manifest must exist"
    );
}

/// The internal identifiers of the new epoch are dense, start at 0 and follow
/// (`doc_id`, `ord`) when `cluster_by` is empty.
///
/// Acceptance criterion of `COMPACTER-INDEX`: "les iid sont contigus et triés
/// selon `cluster_by`".
#[test]
fn compaction_renumbers_iids_contiguously_in_key_order() {
    let _serial = common::SerialGuard::acquire();
    let (_temp, index) = common::temp_index();
    let raw = common::test_config("compaction", &common::fixture_schema());
    let config = fixture_config(&raw);
    let mut store = default_fixture(&index, &raw);

    compact(&mut store, &config);

    assert_eq!(
        live_iids(&store.snapshot()),
        vec![0, 1, 2, 3, 4],
        "the identifiers of the new epoch must be dense and start at 0"
    );
    assert_eq!(
        keys_of_epoch(&index, NEXT_EPOCH, 5),
        vec!["doc-a#1", "doc-a#2", "doc-b#0", "doc-c#0", "doc-d#0"],
        "the compaction orders the live passages by (cluster_by, doc_id, ord), \
         and the fixture declares no cluster_by"
    );
    assert_eq!(
        read_passage_of_epoch(&index, NEXT_EPOCH, 0).text,
        "doc-a#1",
        "the text of a passage must follow its key into the new epoch"
    );
}

/// `cluster_by` orders the new identifiers before `doc_id` and `ord`.
///
/// Acceptance criterion of `COMPACTER-INDEX`: the passages are read sorted by
/// (`cluster_by`, `doc_id`, `ord`), "pour qu'un filtre sur ce champ parcoure une
/// zone mémoire contiguë" (`docs/03-recherche.md`).
#[test]
fn compaction_orders_iids_by_cluster_then_doc_then_ord() {
    let _serial = common::SerialGuard::acquire();
    let (_temp, index) = common::temp_index();
    let raw = clustered_config_json(&["title"]);
    let config = fixture_config(&raw);
    // `title` is "a" for doc-b and doc-c, "m" for doc-d and "z" for doc-a: the
    // order differs from the (doc_id, ord) order, which is what makes this test
    // decisive.
    let mut store = populated_index(&index, &raw, ["z", "a", "a", "m"]);

    compact(&mut store, &config);

    assert_eq!(
        keys_of_epoch(&index, NEXT_EPOCH, 5),
        vec!["doc-b#0", "doc-c#0", "doc-d#0", "doc-a#1", "doc-a#2"],
        "the new epoch must be ordered by (title, doc_id, ord): title 'a' first \
         (doc-b, doc-c), then 'm' (doc-d), then 'z' (doc-a)"
    );
}

/// A cluster field declared at passage level orders the new identifiers too, and
/// each passage carries its own value.
///
/// The fixture writes `part = ord`, so a compaction that read no passage field
/// would fall back to the (`doc_id`, `ord`) order and fail this test.
#[test]
fn compaction_orders_iids_by_a_passage_cluster_field() {
    let _serial = common::SerialGuard::acquire();
    let (_temp, index) = common::temp_index();
    let raw = clustered_config_json(&["part"]);
    let config = fixture_config(&raw);
    let mut store = default_fixture(&index, &raw);

    compact(&mut store, &config);

    assert_eq!(
        keys_of_epoch(&index, NEXT_EPOCH, 5),
        vec!["doc-b#0", "doc-c#0", "doc-d#0", "doc-a#1", "doc-a#2"],
        "the new epoch must be ordered by (part, doc_id, ord): part 0 first, then \
         part 1, then part 2"
    );
}

/// At the last step before the switch, `CURRENT_EPOCH` still names E, E is
/// whole, and the readers of E still read it.
///
/// INV-14: the compaction keeps E until the switch.
#[cfg(feature = "failpoints")]
#[test]
fn inv_14_epoch_unchanged_and_readable_while_compaction_runs() {
    use crible_store::compact::{CompactStep, compact_with_observer};

    let _serial = common::SerialGuard::acquire();
    let (_temp, index) = common::temp_index();
    let raw = common::test_config("compaction", &common::fixture_schema());
    let config = fixture_config(&raw);
    let mut store = default_fixture(&index, &raw);
    let observed = index.clone();

    let report = compact_with_observer(&mut store, &config, |step| {
        if !matches!(step, CompactStep::Verified) {
            return;
        }
        assert_eq!(
            common::read_current_epoch(&observed),
            common::EPOCH,
            "before the switch, CURRENT_EPOCH must still name the epoch the readers \
             use (INV-14)"
        );
        assert!(
            epoch_dir(&observed, NEXT_EPOCH).exists(),
            "the new epoch must be written before it is verified and switched to"
        );
        let alive = read_passage_of_epoch(&observed, Epoch::new(common::EPOCH), 3);
        assert_eq!(alive.text, "doc-a#1");
        let dead = read_passage_of_epoch(&observed, Epoch::new(common::EPOCH), 1);
        assert_eq!(
            dead.dead_since_gen,
            Some(FIXTURE_GENERATION),
            "E must still hold its tombstones until the switch: they are the evidence \
             the copied passages were the right ones"
        );
    })
    .unwrap_or_else(|error| panic!("the observed compaction must succeed: {error}"));

    assert_eq!(report.to_epoch, NEXT_EPOCH);
    assert_eq!(common::read_current_epoch(&index), NEXT_EPOCH.get());
}

/// A crash before the switch leaves E current, whole and usable, and the next
/// `open` sees the generation it had before the compaction.
///
/// INV-14: "une panne ou un échec en cours de compaction laisse E intacte et
/// utilisable (recherche identique, génération inchangée)".
#[cfg(feature = "failpoints")]
#[test]
fn inv_14_crash_before_epoch_switch_keeps_epoch_usable() {
    let (_temp, index) = common::temp_index();
    let raw = common::test_config("compaction", &common::fixture_schema());
    let config = fixture_config(&raw);
    {
        let mut store = default_fixture(&index, &raw);
        let guard = common::FailpointGuard::arm("before_epoch_switch");
        compact_expecting_failpoint(&mut store, &config);
        drop(guard);
    }

    assert_eq!(
        common::read_current_epoch(&index),
        common::EPOCH,
        "a crash before the switch must leave CURRENT_EPOCH untouched"
    );
    let reopened = common::open_store(&index);
    assert_eq!(reopened.epoch(), Epoch::new(common::EPOCH));
    assert_eq!(
        reopened.generation(),
        FIXTURE_GENERATION,
        "the crashed compaction must not advance the generation"
    );
    common::assert_view(
        &reopened.snapshot(),
        FIXTURE_GENERATION.get(),
        &["seg-a", "seg-b"],
        &[0, 2, 3, 4, 5],
    );
    let dead = read_passage_of_epoch(&index, Epoch::new(common::EPOCH), 1);
    assert_eq!(
        dead.dead_since_gen,
        Some(FIXTURE_GENERATION),
        "E keeps its tombstones, so its live set is the one the readers had"
    );
    assert_eq!(
        read_passage_of_epoch(&index, Epoch::new(common::EPOCH), 3).text,
        "doc-a#1",
        "the payloads of E stay readable after the crash"
    );
    assert!(
        !epoch_dir(&index, Epoch::new(common::EPOCH))
            .join("manifests")
            .join("3.json")
            .exists(),
        "the crashed compaction must not leave a manifest of a generation it never \
         committed"
    );
    assert!(
        !epoch_dir(&index, NEXT_EPOCH).join("CURRENT_EPOCH").exists(),
        "no epoch directory carries its own CURRENT_EPOCH; the index holds the only one"
    );
}

/// A crash right after the switch leaves E+1 current and whole, and E still on
/// disk: the old epoch is only deleted once the new one is published and no
/// reader holds it.
///
/// INV-14.
#[cfg(feature = "failpoints")]
#[test]
fn inv_14_crash_after_epoch_switch_keeps_new_epoch_whole() {
    let (_temp, index) = common::temp_index();
    let raw = common::test_config("compaction", &common::fixture_schema());
    let config = fixture_config(&raw);
    {
        let mut store = default_fixture(&index, &raw);
        let guard = common::FailpointGuard::arm("after_epoch_switch");
        compact_expecting_failpoint(&mut store, &config);
        drop(guard);
    }

    assert_eq!(
        common::read_current_epoch(&index),
        NEXT_EPOCH.get(),
        "the switch is a rename of CURRENT_EPOCH: it survives the crash"
    );
    let reopened = common::open_store(&index);
    assert_eq!(reopened.epoch(), NEXT_EPOCH);
    assert_eq!(reopened.generation(), Generation::new(3));
    let state = reopened.state().unwrap_or_else(|error| {
        panic!("the state of the recovered index must be readable: {error}")
    });
    assert_eq!(state.live_passages, 5);
    assert_eq!(state.dead_passages, 0);
    assert_eq!(
        keys_of_epoch(&index, NEXT_EPOCH, 5),
        vec!["doc-a#1", "doc-a#2", "doc-b#0", "doc-c#0", "doc-d#0"],
        "the new epoch must be complete and ordered after the crash"
    );
    assert!(
        epoch_dir(&index, Epoch::new(common::EPOCH)).exists(),
        "a crash before the publication must leave E on disk: the deletion belongs \
         after the publication and the last reader (INV-14)"
    );
}

/// A compaction with no reader deletes the epoch it replaced in the same call,
/// so the index never keeps more than one epoch on disk.
///
/// INV-14 and ADR-0006 ("deux fois l'espace disque" pendant la compaction): the
/// compaction releases its own snapshot of E before the reclaim, so E goes as
/// soon as the new epoch is published. Only a reader that outlives the
/// compaction keeps an old epoch (see the next test).
#[test]
fn compaction_deletes_the_replaced_epoch_when_no_reader_holds_it() {
    let _serial = common::SerialGuard::acquire();
    let (_temp, index) = common::temp_index();
    let raw = common::test_config("compaction", &common::fixture_schema());
    let config = fixture_config(&raw);
    let mut store = default_fixture(&index, &raw);

    compact(&mut store, &config);

    let epochs = common::directory_entries(&index.join("epochs"));
    assert_eq!(
        epochs,
        vec![NEXT_EPOCH.get().to_string()],
        "the compaction must leave a single epoch directory, the current one: {epochs:?}"
    );
    assert!(
        !epoch_dir(&index, Epoch::new(common::EPOCH)).exists(),
        "the epoch the compaction replaced must be deleted in this call: the \
         compaction's own snapshot of it must not count as a reader"
    );
    assert!(
        epoch_dir(&index, NEXT_EPOCH).exists(),
        "the epoch CURRENT_EPOCH names is never reclaimed"
    );
    assert!(
        crible_store::compact::reclaim_epochs(&store)
            .unwrap_or_else(|error| panic!("the reclaim must not fail: {error}"))
            .is_empty(),
        "nothing is left to reclaim after a compaction without a reader"
    );
    assert_eq!(live_iids(&store.snapshot()), vec![0, 1, 2, 3, 4]);
}

/// An epoch held by a snapshot survives the compaction, and is deleted only
/// after the reader releases it.
///
/// INV-14 and ADR-0043 (point 4): "une époque n'est jamais supprimée tant qu'un
/// instantané la tient".
#[test]
fn inv_14_epoch_held_by_reader_survives_compaction() {
    let _serial = common::SerialGuard::acquire();
    let (_temp, index) = common::temp_index();
    let raw = common::test_config("compaction", &common::fixture_schema());
    let config = fixture_config(&raw);
    let mut store = default_fixture(&index, &raw);

    // A search that started before the compaction: it holds this snapshot for
    // the whole request, and reads its payloads in E.
    let held = store.snapshot();
    assert_eq!(held.generation(), FIXTURE_GENERATION);

    compact(&mut store, &config);

    assert!(
        epoch_dir(&index, Epoch::new(common::EPOCH)).exists(),
        "the compaction must not delete an epoch a live snapshot holds (INV-14, \
         ADR-0043 point 4)"
    );
    assert!(
        crible_store::compact::reclaim_epochs(&store)
            .unwrap_or_else(|error| panic!("the reclaim must not fail: {error}"))
            .is_empty(),
        "nothing is reclaimable while the reader holds E"
    );
    assert_eq!(
        live_iids(&held),
        vec![0, 2, 3, 4, 5],
        "the held snapshot must still see its generation whole"
    );
    assert_eq!(
        read_passage_of_epoch(&index, Epoch::new(common::EPOCH), 3).text,
        "doc-a#1",
        "the payload of a held epoch stays readable"
    );

    drop(held);
    assert_eq!(
        crible_store::compact::reclaim_epochs(&store)
            .unwrap_or_else(|error| panic!("the reclaim must not fail: {error}")),
        vec![Epoch::new(common::EPOCH)],
        "once the last reader is done, the old epoch is reclaimable"
    );
    assert!(
        !epoch_dir(&index, Epoch::new(common::EPOCH)).exists(),
        "the reclaimed epoch directory must be gone"
    );
    assert!(
        epoch_dir(&index, NEXT_EPOCH).exists(),
        "the current epoch is never reclaimed"
    );
    assert_eq!(store.generation(), Generation::new(3));
    assert_eq!(live_iids(&store.snapshot()), vec![0, 1, 2, 3, 4]);
}

/// A store left behind the pointer refuses to compact again, and the epoch the
/// pointer names stays on disk.
///
/// A failed `adopt_epoch` leaves `CURRENT_EPOCH` naming E+1 while the store still
/// holds E (`generation.rs`, `adopt_epoch`). A second compaction on that store
/// would take E+1 as its target and remove the very epoch the pointer names: it
/// must refuse instead, in `internal`, until the store is reopened (INV-04).
#[cfg(feature = "failpoints")]
#[test]
fn a_store_left_behind_the_pointer_refuses_to_compact_again() {
    // The retry runs the store after the guard is dropped, so the serial guard is
    // held for the whole body and the failpoint borrowed from it.
    let serial = common::SerialGuard::acquire();
    let (_temp, index) = common::temp_index();
    let raw = common::test_config("compaction", &common::fixture_schema());
    let config = fixture_config(&raw);
    let mut store = default_fixture(&index, &raw);
    let guard = common::FailpointGuard::arm_holding(&serial, "after_epoch_switch");
    compact_expecting_failpoint(&mut store, &config);
    drop(guard);

    assert_eq!(
        common::read_current_epoch(&index),
        NEXT_EPOCH.get(),
        "the switch happened before the crash"
    );
    assert_eq!(
        store.epoch(),
        Epoch::new(common::EPOCH),
        "nothing was published, so the store still holds E"
    );

    let error = crible_store::compact::compact(&mut store, &config)
        .expect_err("a store behind CURRENT_EPOCH must refuse to compact");

    assert_eq!(error.code(), ErrorCode::Internal, "the message: {error}");
    assert!(
        error.message().contains("reopen"),
        "the refusal must say what to do: {error}"
    );
    assert!(
        epoch_dir(&index, NEXT_EPOCH).exists(),
        "the refused compaction must not remove the epoch CURRENT_EPOCH names"
    );
    assert!(
        crible_store::compact::reclaim_epochs(&store)
            .unwrap_or_else(|error| panic!("the reclaim must not fail: {error}"))
            .is_empty(),
        "the reclaim must never delete the epoch CURRENT_EPOCH names"
    );
    assert!(
        epoch_dir(&index, NEXT_EPOCH).exists(),
        "the epoch CURRENT_EPOCH names survives a reclaim"
    );
}

/// Compacting again after a crash before the switch converges to the same epoch
/// as an uninterrupted compaction.
///
/// The half-written E+1 of the crashed attempt must not block the retry: the
/// index either cleans it at `open` or overwrites it, and the result is a whole
/// epoch with the same content and no leftover temporary directory.
#[cfg(feature = "failpoints")]
#[test]
fn inv_14_recompact_after_crash_before_switch_converges() {
    // The retry compacts without arming anything, so it must stay the only test
    // of the binary that reaches the store while `CRIBLE_FAILPOINT` is read: the
    // serial guard is held for the whole body, and the failpoint borrowed from it.
    let serial = common::SerialGuard::acquire();
    let (_temp, index) = common::temp_index();
    let raw = common::test_config("compaction", &common::fixture_schema());
    let config = fixture_config(&raw);
    {
        let mut store = default_fixture(&index, &raw);
        let guard = common::FailpointGuard::arm_holding(&serial, "before_epoch_switch");
        compact_expecting_failpoint(&mut store, &config);
        drop(guard);
    }

    let mut reopened = common::open_store(&index);
    let report = compact(&mut reopened, &config);

    assert_eq!(report.to_epoch, NEXT_EPOCH);
    assert_eq!(report.generation, Generation::new(3));
    assert_eq!(common::read_current_epoch(&index), NEXT_EPOCH.get());
    let state = reopened.state().unwrap_or_else(|error| {
        panic!("the state of the retried compaction must be readable: {error}")
    });
    assert_eq!((state.live_passages, state.dead_passages), (5, 0));
    assert_eq!(
        keys_of_epoch(&index, NEXT_EPOCH, 5),
        vec!["doc-a#1", "doc-a#2", "doc-b#0", "doc-c#0", "doc-d#0"],
        "the retried compaction must produce the same order as the first attempt"
    );
    let temporaries: Vec<String> = common::directory_entries(&epoch_dir(&index, NEXT_EPOCH))
        .into_iter()
        .filter(|name| name.starts_with("tmp-"))
        .collect();
    assert!(
        temporaries.is_empty(),
        "the retried compaction must leave no temporary directory behind: {temporaries:?}"
    );
}
