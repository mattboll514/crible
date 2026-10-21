// SPDX-License-Identifier: Apache-2.0

//! INV-04 — at restart, the state is the last committed generation; everything
//! else is rebuilt or cleaned.
//!
//! References: `docs/08-invariants-et-tests.md` (INV-04 and the "Pannes" row),
//! `docs/02-architecture.md` ("Organisation sur disque", "Générations et point
//! de commit", "Concurrence"), `docs/adr/0016-point-commit-sqlite.md`
//! (ADR-0016 — the SQLite commit is the only commit point) and the
//! `GERER-GENERATIONS` task of `docs/phases/P1-stockage-et-recherche-exacte.md`.
//!
//! A crash is simulated by panicking inside `commit` at an injected step, then
//! dropping the store and reopening the index the way a restart does. The
//! panic unwinds through the SQLite transaction, which rolls it back exactly
//! as a process death would.
//!
//! # Assumed API
//!
//! Same API as `inv_03_generation_visibility.rs`, whose module comment gives
//! the full signatures. In short:
//!
//! ```rust,ignore
//! GenerationStore::create(index_dir: &Path, epoch: Epoch) -> Result<Self, CribleError>
//! GenerationStore::open(index_dir: &Path) -> Result<Self, CribleError>  // runs recovery
//! GenerationStore::generation(&self) -> Generation
//! GenerationStore::snapshot(&self) -> Arc<Snapshot>
//! GenerationStore::commit(&mut self, change: GenerationChange) -> Result<Generation, CribleError>
//! ```
//!
//! # Assumed failpoints
//!
//! Cargo feature `failpoints` (declared in `crates/crible-store/Cargo.toml`,
//! never enabled in a release build). When `CRIBLE_FAILPOINT` names a step,
//! `commit` panics with a message containing `CRIBLE_FAILPOINT` when it reaches
//! that step. The steps follow the protocol of `docs/02-architecture.md`,
//! section "Générations et point de commit":
//!
//! | Step | Position |
//! |---|---|
//! | `before_sqlite_commit` | inside the transaction, rows written, before `COMMIT` (step 3) |
//! | `after_sqlite_commit` | right after `COMMIT` returns, before the manifest (steps 4-5) |
//! | `after_manifest` | after `manifests/<G>.json` is fsynced and renamed, before publication (step 5) |
//! | `after_publish` | after the `ArcSwap` publication, before `commit` returns (step 6) |
//!
//! The two tests that do not need a failpoint (orphan cleanup, manifest
//! rewrite) run without the feature. The tests that need it are compiled only
//! with the feature; run them with:
//!
//! ```text
//! cargo test -p crible-store --features failpoints --test inv_04_recovery
//! ```

mod common;

use std::fs;

#[cfg(feature = "failpoints")]
use std::path::Path;

use crible_core::ids::Generation;

/// Failpoint step, and the generation a restart must observe after a crash
/// there. Either the old generation or the new one, never a mix.
#[cfg(feature = "failpoints")]
const CRASH_STEPS: [(&str, u64); 4] = [
    ("before_sqlite_commit", 0),
    ("after_sqlite_commit", 1),
    ("after_manifest", 1),
    ("after_publish", 1),
];

/// Arms `step`, commits generation 1 and lets it crash, then drops the store.
///
/// The segment is written and renamed before the commit, so the crash happens
/// after the segment write for every step of the table.
#[cfg(feature = "failpoints")]
fn crash_while_committing_first_generation(index: &Path, step: &str) {
    let mut store = common::create_store(index);
    let segment = common::write_segment(
        index,
        "seg-a",
        0,
        1,
        &[&[0.0, 1.0, 0.0, 0.0], &[1.0, 0.0, 0.0, 0.0]],
    );
    let change = common::change(
        vec![segment],
        vec![common::document("doc-a", 1), common::document("doc-b", 1)],
        vec![
            common::passage(0, "doc-a", 0, "seg-a", 1),
            common::passage(1, "doc-b", 0, "seg-a", 1),
        ],
        Vec::new(),
    );
    let guard = common::FailpointGuard::arm(step);
    common::commit_expecting_failpoint(&mut store, change);
    drop(guard);
    drop(store);
}

/// A crash before the SQLite commit leaves the previous generation, and the
/// segment written before the commit is an orphan, removed on restart.
#[cfg(feature = "failpoints")]
#[test]
fn inv_04_crash_before_sqlite_commit_keeps_previous_generation() {
    let (_temp, index) = common::temp_index();
    crash_while_committing_first_generation(&index, "before_sqlite_commit");

    let reopened = common::open_store(&index);
    common::assert_view(&reopened.snapshot(), 0, &[], &[]);
    // Nothing of the aborted generation is visible, not even its segment.
    common::assert_segment_directories(&index, &[]);
    assert!(
        common::temporary_segment_directories(&index).is_empty(),
        "no tmp-* directory survives the restart"
    );
    common::assert_manifest_describes(&index, 0, &[]);
}

/// A crash right after the SQLite commit keeps the whole new generation, and
/// the manifest that was not written yet is rebuilt from SQLite.
#[cfg(feature = "failpoints")]
#[test]
fn inv_04_crash_after_sqlite_commit_keeps_new_generation() {
    let (_temp, index) = common::temp_index();
    crash_while_committing_first_generation(&index, "after_sqlite_commit");

    let reopened = common::open_store(&index);
    common::assert_view(&reopened.snapshot(), 1, &["seg-a"], &[0, 1]);
    common::assert_segment_directories(&index, &["seg-a"]);
    // The manifest of generation 1 did not exist at crash time.
    common::assert_manifest_describes(&index, 1, &["seg-a"]);
}

/// A crash after the manifest but before the snapshot publication publishes
/// the committed generation at restart.
#[cfg(feature = "failpoints")]
#[test]
fn inv_04_crash_after_manifest_publishes_new_generation_at_restart() {
    let (_temp, index) = common::temp_index();
    crash_while_committing_first_generation(&index, "after_manifest");

    let reopened = common::open_store(&index);
    common::assert_view(&reopened.snapshot(), 1, &["seg-a"], &[0, 1]);
    common::assert_manifest_describes(&index, 1, &["seg-a"]);
}

/// A crash at any step of the protocol gives the old or the new generation in
/// full, never a mix, and leaves no orphan nor `tmp-*` behind.
#[cfg(feature = "failpoints")]
#[test]
fn inv_04_crash_at_every_step_yields_old_or_new_generation() {
    for (step, expected_generation) in CRASH_STEPS {
        let (_temp, index) = common::temp_index();
        crash_while_committing_first_generation(&index, step);

        let reopened = common::open_store(&index);
        assert_eq!(
            reopened.generation().get(),
            expected_generation,
            "after a crash at {step}, the restart must expose the committed generation"
        );

        let segments: &[&str] = if expected_generation == 1 {
            &["seg-a"]
        } else {
            &[]
        };
        let live_iids: &[u32] = if expected_generation == 1 {
            &[0, 1]
        } else {
            &[]
        };
        common::assert_view(
            &reopened.snapshot(),
            expected_generation,
            segments,
            live_iids,
        );
        common::assert_segment_directories(&index, segments);
        common::assert_manifest_describes(&index, expected_generation, segments);
        assert!(
            common::temporary_segment_directories(&index).is_empty(),
            "after a crash at {step}, no tmp-* directory survives the restart"
        );
    }
}

/// Restart removes every `tmp-*` directory and every segment SQLite does not
/// reference, and keeps the referenced one.
#[test]
fn inv_04_recovery_removes_temporary_and_orphan_segments() {
    // The failpoint variable is process-global: every committing test of
    // this binary takes the same guard, armed or not.
    let _serial = common::SerialGuard::acquire();
    let (_temp, index) = common::temp_index();
    {
        let mut store = common::create_store(&index);
        let segment = common::write_segment(
            &index,
            "seg-a",
            0,
            1,
            &[&[0.0, 1.0, 0.0, 0.0], &[1.0, 0.0, 0.0, 0.0]],
        );
        common::commit(
            &mut store,
            common::change(
                vec![segment],
                vec![common::document("doc-a", 1)],
                vec![common::passage(0, "doc-a", 0, "seg-a", 1)],
                Vec::new(),
            ),
        );
    }

    // A directory left by a crash between `SegmentWriter::create` and
    // `SegmentWriter::finish`, before the rename.
    let leftover = common::segments_dir(&index).join("tmp-seg-crashed");
    fs::create_dir_all(&leftover)
        .unwrap_or_else(|error| panic!("the tmp-* fixture directory must be creatable: {error}"));
    fs::write(leftover.join("vectors.f16"), [0_u8; 8])
        .unwrap_or_else(|error| panic!("the tmp-* fixture file must be writable: {error}"));

    // A segment written and renamed, but never referenced by SQLite because
    // the commit never happened.
    common::write_segment(&index, "seg-orphan", 2, 2, &[&[0.0, 0.0, 1.0, 0.0]]);

    let reopened = common::open_store(&index);
    common::assert_view(&reopened.snapshot(), 1, &["seg-a"], &[0]);
    common::assert_segment_directories(&index, &["seg-a"]);
}

/// A missing manifest is rewritten from SQLite at restart, and describes the
/// committed generation with its segments.
#[test]
fn inv_04_recovery_rewrites_missing_manifest_from_sqlite() {
    // The failpoint variable is process-global: every committing test of
    // this binary takes the same guard, armed or not.
    let _serial = common::SerialGuard::acquire();
    let (_temp, index) = common::temp_index();
    {
        let mut store = common::create_store(&index);
        let segment = common::write_segment(
            &index,
            "seg-a",
            0,
            1,
            &[&[0.0, 1.0, 0.0, 0.0], &[1.0, 0.0, 0.0, 0.0]],
        );
        common::commit(
            &mut store,
            common::change(
                vec![segment],
                vec![common::document("doc-a", 1)],
                vec![common::passage(0, "doc-a", 0, "seg-a", 1)],
                Vec::new(),
            ),
        );
    }

    let path = common::manifest_path(&index, Generation::new(1));
    assert!(
        path.exists(),
        "the commit must write the manifest of the generation it publishes"
    );
    fs::remove_file(&path)
        .unwrap_or_else(|error| panic!("the manifest fixture must be removable: {error}"));
    assert!(
        !path.exists(),
        "the manifest must really be missing on restart"
    );

    let reopened = common::open_store(&index);
    common::assert_view(&reopened.snapshot(), 1, &["seg-a"], &[0]);
    common::assert_manifest_describes(&index, 1, &["seg-a"]);
}

/// Recovery is idempotent: a second restart changes nothing.
#[test]
fn inv_04_recovery_is_idempotent() {
    // The failpoint variable is process-global: every committing test of
    // this binary takes the same guard, armed or not.
    let _serial = common::SerialGuard::acquire();
    let (_temp, index) = common::temp_index();
    {
        let mut store = common::create_store(&index);
        let segment = common::write_segment(&index, "seg-a", 0, 1, &[&[0.0, 1.0, 0.0, 0.0]]);
        common::commit(
            &mut store,
            common::change(
                vec![segment],
                vec![common::document("doc-a", 1)],
                vec![common::passage(0, "doc-a", 0, "seg-a", 1)],
                Vec::new(),
            ),
        );
    }

    let first = common::open_store(&index);
    common::assert_view(&first.snapshot(), 1, &["seg-a"], &[0]);
    drop(first);
    let manifests_after_first_restart = common::directory_entries(&common::manifests_dir(&index));

    let second = common::open_store(&index);
    common::assert_view(&second.snapshot(), 1, &["seg-a"], &[0]);
    common::assert_segment_directories(&index, &["seg-a"]);
    // One manifest per generation, no accumulation: a second restart writes
    // exactly the manifests the first one left, and never a duplicate.
    assert_eq!(
        common::directory_entries(&common::manifests_dir(&index)),
        manifests_after_first_restart,
        "restarting twice must not add or drop a manifest"
    );
}
