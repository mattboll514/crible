// SPDX-License-Identifier: Apache-2.0

//! INV-04 — a read-only open never repairs the lexical branch (`ADR-0087`).
//!
//! References: `docs/02-architecture.md` ("Organisation sur disque", "Reprise au
//! démarrage", "Concurrence"), `docs/03-recherche.md` ("Branche lexicale"),
//! `docs/08-invariants-et-tests.md` (INV-02, INV-03, INV-04), `ADR-0056`
//! (decision 4), `ADR-0069` (decision 1) and `ADR-0087` (decisions 1 and 2).
//!
//! The task `INDEXER-TANTIVY-A-L-ATTACHE` compares the payload of the last
//! tantivy commit (`Gt`) with the generation SQLite holds (`G`) **on a write open
//! only**. A read-only open — `crible search`, `crible inspect` — opens no
//! `IndexWriter`, commits nothing, removes nothing under `lexical/`, and gives
//! the snapshot no lexical reader when `Gt ≠ G`; the next write open repairs.
//!
//! What this file asserts, and how:
//!
//! - the lexical directory is byte for byte what it was before the read-only
//!   open, payload included: no commit, no rebuild, no writer lock file;
//! - the snapshot of the read-only open exposes no lexical reader
//!   (`Snapshot::lexical` returns `None`), while the dense branch serves the
//!   committed generation;
//! - a write open then catches the index up: the same snapshot has a reader, and
//!   the payload names `G`;
//! - the same holds when the payload is **ahead** of the committed generation
//!   (`Gt > G`): the snapshot names `Ahead`, serves dense, and `lexical/` stays
//!   byte for byte what it was (`ADR-0088`, decision 6);
//! - step 4 never extends an index the published snapshot did not certify: a
//!   generation committed on a store whose snapshot was `Behind` replays the
//!   generations the index missed before committing the new one (`ADR-0088`,
//!   decision 2 — the gravest risk the ADR names).
//!
//! ```text
//! cargo test -p crible-store --test inv_04_lexical_read_only
//! ```

// The fixtures of this file call `panic!` outside the `#[test]` functions, as
// the other integration tests do; see `tests/common/mod.rs`.
#![allow(clippy::panic)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use crible_core::ids::Generation;
use crible_store::generation::GenerationStore;
use crible_store::lexical::{LexicalStatus, read_state};

/// Lexical directory of an epoch, under `<index>/epochs/<E>/`.
const LEXICAL_DIR: &str = "lexical";

/// One file of a directory tree: its path relative to the root, and the blake3
/// of its bytes.
type FileDigest = (String, String);

/// Every file under `root`, recursively, as `(relative path, blake3)` pairs,
/// sorted by path so two listings compare directly.
fn tree(root: &Path) -> Vec<FileDigest> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files);
    files.sort();
    files
}

/// Recursive body of [`tree`].
fn collect_files(root: &Path, dir: &Path, files: &mut Vec<FileDigest>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        // A directory that does not exist holds no file: an absent `lexical/`
        // compares equal to itself.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => panic!("{} must be readable: {error}", dir.display()),
    };
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| panic!("an entry must be readable: {error}"));
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, files);
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or_else(|error| {
                panic!(
                    "{} must live under {}: {error}",
                    path.display(),
                    root.display()
                )
            })
            .to_string_lossy()
            .into_owned();
        let bytes = fs::read(&path)
            .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
        files.push((
            relative,
            format!("blake3:{}", blake3::hash(&bytes).to_hex()),
        ));
    }
}

/// Copies the directory `from` into `to`, recursively.
fn copy_directory(from: &Path, to: &Path) {
    fs::create_dir_all(to)
        .unwrap_or_else(|error| panic!("{} must be creatable: {error}", to.display()));
    for entry in fs::read_dir(from)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", from.display()))
    {
        let entry = entry.unwrap_or_else(|error| panic!("an entry must be readable: {error}"));
        let source = entry.path();
        let target = to.join(entry.file_name());
        if source.is_dir() {
            copy_directory(&source, &target);
        } else {
            fs::copy(&source, &target).unwrap_or_else(|error| {
                panic!(
                    "{} must be copied to {}: {error}",
                    source.display(),
                    target.display()
                )
            });
        }
    }
}

/// Builds an index at generation 2 whose lexical index is still at generation 1.
///
/// The fixture commits generation 1 with two passages, saves its lexical
/// directory, commits generation 2 with a third passage, then puts the saved
/// directory back: `Gt` is 1 while `G` is 2, exactly what an `attach` killed
/// between `COMMIT` and step 4 leaves on disk. The two commits use the same
/// schema and the same analysis chain, so the restored directory is the one the
/// index really produced, not a hand-made one.
fn behind_fixture() -> (tempfile::TempDir, PathBuf) {
    let (temp, index) = common::temp_index();
    let schema = common::fixture_schema();
    let config = common::test_config_with_lexical("test", &schema, true);
    let mut store = common::create_store_with_config(&index, &schema, &config);

    let first = common::write_segment(
        &index,
        "seg-a",
        0,
        1,
        &[&[1.0, 0.0, 0.0, 0.0], &[0.0, 1.0, 0.0, 0.0]],
    );
    common::commit(
        &mut store,
        common::change(
            vec![first],
            vec![common::document("doc-a", 1), common::document("doc-b", 1)],
            vec![
                common::passage(0, "doc-a", 0, "seg-a", 1),
                common::passage(1, "doc-b", 0, "seg-a", 1),
            ],
            Vec::new(),
        ),
    );

    let epoch_dir = common::epoch_dir(&index);
    let saved = temp.path().join("lexical-at-generation-1");
    copy_directory(&epoch_dir.join(LEXICAL_DIR), &saved);

    let second = common::write_segment(&index, "seg-b", 2, 2, &[&[0.0, 0.0, 1.0, 0.0]]);
    common::commit(
        &mut store,
        common::change(
            vec![second],
            vec![common::document("doc-c", 2)],
            vec![common::passage(2, "doc-c", 0, "seg-b", 2)],
            Vec::new(),
        ),
    );
    drop(store);

    fs::remove_dir_all(epoch_dir.join(LEXICAL_DIR))
        .unwrap_or_else(|error| panic!("the lexical directory must be removable: {error}"));
    copy_directory(&saved, &epoch_dir.join(LEXICAL_DIR));
    (temp, index)
}

/// The criterion of the fiche: a read-only open with `Gt ≠ G` writes nothing,
/// opens no `IndexWriter`, and exposes no lexical reader; a write open repairs.
#[test]
fn inv_04_read_only_open_with_a_stale_lexical_index_repairs_nothing() {
    let (_temp, index) = behind_fixture();
    let schema = common::fixture_schema();
    let epoch_dir = common::epoch_dir(&index);

    let before = tree(&epoch_dir.join(LEXICAL_DIR));
    let stale = read_state(&epoch_dir).unwrap_or_else(|error| {
        panic!("the stale lexical index of the fixture must be readable: {error}")
    });
    assert_eq!(
        stale.generation,
        Some(Generation::new(1)),
        "the fixture must leave the lexical index at generation 1, behind SQLite"
    );
    assert!(
        !before.is_empty(),
        "the fixture must leave a lexical index on disk, otherwise the test proves nothing"
    );

    let store = GenerationStore::open_read_only(&index, &schema).unwrap_or_else(|error| {
        panic!(
            "a read-only open of an index whose lexical index is behind must succeed: {}: {}",
            error.code().as_str(),
            error.message()
        )
    });
    assert_eq!(
        store.generation(),
        Generation::new(2),
        "the read-only open must publish the generation SQLite committed"
    );
    common::assert_view(&store.snapshot(), 2, &["seg-a", "seg-b"], &[0, 1, 2]);
    assert!(
        store.snapshot().lexical().is_none(),
        "the snapshot of a read-only open must expose no lexical reader when Gt ≠ G \
         (ADR-0087): a reader of generation 1 under a generation-2 snapshot would mix \
         two generations"
    );
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::Behind {
            committed: Some(Generation::new(1))
        },
        "the snapshot must name the state it observed: the marker 2 is ahead of the payload 1, \
         so a generation after the commit created a passage (`ADR-0088`, decision 5)"
    );
    drop(store);

    assert_eq!(
        tree(&epoch_dir.join(LEXICAL_DIR)),
        before,
        "a read-only open must not commit, rebuild or delete anything under lexical/, and \
         must not leave the tantivy writer lock behind (ADR-0056, ADR-0069, ADR-0087)"
    );
    let unchanged = read_state(&epoch_dir).unwrap_or_else(|error| {
        panic!("the lexical index must still be readable after the read-only open: {error}")
    });
    assert_eq!(
        unchanged, stale,
        "the payload and the iids of the lexical index must be exactly what they were before \
         the read-only open: no commit ran"
    );

    let store = GenerationStore::open(&index, &schema).unwrap_or_else(|error| {
        panic!(
            "the write open must repair the lexical index: {}: {}",
            error.code().as_str(),
            error.message()
        )
    });
    assert_eq!(
        store.generation(),
        Generation::new(2),
        "the repair must not create a generation: it is a derived state of generation 2"
    );
    common::assert_view(&store.snapshot(), 2, &["seg-a", "seg-b"], &[0, 1, 2]);
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::Ready {
            committed: Generation::new(2)
        },
        "the caught-up index is at the committed generation, so the state is Ready"
    );
    assert!(
        store.snapshot().lexical().is_some(),
        "the write open catches the lexical index up before publishing, so the snapshot has \
         a reader once it returns (INV-04, ADR-0087)"
    );

    let repaired = read_state(&epoch_dir)
        .unwrap_or_else(|error| panic!("the repaired lexical index must be readable: {error}"));
    assert_eq!(
        repaired.generation,
        Some(Generation::new(2)),
        "the repair must leave the payload of the last commit at the committed generation"
    );
    let mut iids: Vec<u32> = repaired.iids.iter().map(|iid| iid.get()).collect();
    iids.sort_unstable();
    assert_eq!(
        iids,
        vec![0, 1, 2],
        "the repaired lexical index must hold exactly the live iids of generation 2"
    );
}

/// Step 4 must never extend an index the published snapshot did not certify.
///
/// The fixture is an index whose `lexical/` is one generation behind
/// (`Gt = 1`, `G = 2`, `M = 2`). A read-only open publishes
/// [`LexicalStatus::Behind`] with no reader; a caller that then commits on that
/// same store — the public API allows it — must not let step 4 add only the new
/// delta to the stale index and commit `G+1`: the passage created at generation
/// 2 would stay unindexed while the payload claimed otherwise (INV-04, INV-05;
/// `ADR-0088` names this the gravest risk of the marker).
#[test]
fn inv_04_an_attach_after_a_read_only_open_replays_the_missed_generations() {
    let (temp, index) = behind_fixture();
    let schema = common::fixture_schema();
    let epoch_dir = common::epoch_dir(&index);

    let mut store = GenerationStore::open_read_only(&index, &schema).unwrap_or_else(|error| {
        panic!(
            "the read-only open of the fixture must succeed: {}: {}",
            error.code().as_str(),
            error.message()
        )
    });
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::Behind {
            committed: Some(Generation::new(1))
        },
        "the fixture must publish the state step 4 has to repair"
    );
    assert!(store.snapshot().lexical().is_none());

    // A fourth document, distinct from the three the fixture holds, so the
    // attach creates one passage and commits generation 3.
    let fourth = common::FixtureDocument {
        doc_id: "doc-d",
        text: "doc-d#0",
        vector: [0.0, 0.0, 0.0, 1.0],
    };
    let batch = common::write_batch(
        &temp.path().join("batches"),
        "read-only-then-attach",
        &[fourth],
    );
    let raw = fs::read_to_string(index.join("index.json"))
        .unwrap_or_else(|error| panic!("the index configuration must be readable: {error}"));
    let config = crible_core::config::IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("the index configuration must be valid: {error}"));
    let model = crible_core::model::ModelConfig::from_json_str(
        r#"{ "name": "read-only-attach-external", "kind": "external", "dims": 4, "normalize": true }"#,
    )
    .unwrap_or_else(|error| panic!("the model configuration must be valid: {error}"));
    crible_store::attach::attach(&mut store, &batch, &config, &model, None, None).unwrap_or_else(
        |error| {
            panic!(
                "the attach after a read-only open must succeed: {}: {}",
                error.code().as_str(),
                error.message()
            )
        },
    );

    assert_eq!(
        store.generation(),
        Generation::new(3),
        "the attach must commit generation 3"
    );
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::Ready {
            committed: Generation::new(3)
        },
        "step 4 must replay the generations the stale index missed, then commit the new one, so \
         the state it publishes is certified"
    );
    assert!(
        store.snapshot().lexical().is_some(),
        "the snapshot of the replayed index must expose its reader"
    );
    let mut iids: Vec<u32> = store.snapshot().live_iids().map(|iid| iid.get()).collect();
    iids.sort_unstable();
    assert_eq!(
        iids,
        vec![0, 1, 2, 3],
        "the dense branch must serve four passages"
    );

    let state = read_state(&epoch_dir)
        .unwrap_or_else(|error| panic!("the replayed lexical index must be readable: {error}"));
    assert_eq!(
        state.generation,
        Some(Generation::new(3)),
        "the last tantivy commit must name the generation it indexed"
    );
    let mut indexed: Vec<u32> = state.iids.iter().map(|iid| iid.get()).collect();
    indexed.sort_unstable();
    assert_eq!(
        indexed,
        vec![0, 1, 2, 3],
        "the passage created at generation 2 must be replayed: adding only the delta of \
         generation 3 would leave it unindexed while the payload names generation 3"
    );
}

/// Builds two indexes of the same fixtures, then copies the lexical directory of
/// the more advanced one into the other: the second index is at generation 1
/// while its tantivy commit names generation 2 (`Gt > G`, `ADR-0088`,
/// decision 6).
///
/// The two indexes share the schema and the analysis chain, so the copied
/// directory is one a Crible index really produced, not a hand-made one.
fn ahead_fixture() -> (tempfile::TempDir, PathBuf) {
    let (temp, ahead) = common::temp_index();
    let schema = common::fixture_schema();
    let config = common::test_config_with_lexical("test", &schema, true);

    // The advanced index: generation 2, three passages, lexical at generation 2.
    let mut store = common::create_store_with_config(&ahead, &schema, &config);
    let first = common::write_segment(
        &ahead,
        "seg-a",
        0,
        1,
        &[&[1.0, 0.0, 0.0, 0.0], &[0.0, 1.0, 0.0, 0.0]],
    );
    common::commit(
        &mut store,
        common::change(
            vec![first],
            vec![common::document("doc-a", 1), common::document("doc-b", 1)],
            vec![
                common::passage(0, "doc-a", 0, "seg-a", 1),
                common::passage(1, "doc-b", 0, "seg-a", 1),
            ],
            Vec::new(),
        ),
    );
    let second = common::write_segment(&ahead, "seg-b", 2, 2, &[&[0.0, 0.0, 1.0, 0.0]]);
    common::commit(
        &mut store,
        common::change(
            vec![second],
            vec![common::document("doc-c", 2)],
            vec![common::passage(2, "doc-c", 0, "seg-b", 2)],
            Vec::new(),
        ),
    );
    drop(store);

    // The index that stays at generation 1, with two passages.
    let index = temp.path().join("indexes").join("behind");
    let mut store = common::create_store_with_config(&index, &schema, &config);
    let first = common::write_segment(
        &index,
        "seg-a",
        0,
        1,
        &[&[1.0, 0.0, 0.0, 0.0], &[0.0, 1.0, 0.0, 0.0]],
    );
    common::commit(
        &mut store,
        common::change(
            vec![first],
            vec![common::document("doc-a", 1), common::document("doc-b", 1)],
            vec![
                common::passage(0, "doc-a", 0, "seg-a", 1),
                common::passage(1, "doc-b", 0, "seg-a", 1),
            ],
            Vec::new(),
        ),
    );
    drop(store);

    let target = common::epoch_dir(&index).join(LEXICAL_DIR);
    match fs::remove_dir_all(&target) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!(
            "the lexical directory {} must be removable: {error}",
            target.display()
        ),
    }
    copy_directory(&common::epoch_dir(&ahead).join(LEXICAL_DIR), &target);
    (temp, index)
}

/// `Gt > G` seen by a **read-only** open: a state no commit of the epoch
/// produced. The snapshot must name it, expose no lexical reader, and keep
/// serving the dense branch of the committed generation; the derived directory
/// must stay byte for byte what it was, because a read-only open repairs nothing
/// (`ADR-0088`, decisions 6 and 7; `ADR-0087`).
#[test]
fn inv_04_lexical_ahead_read_only_serves_dense_without_a_reader() {
    let (_temp, index) = ahead_fixture();
    let schema = common::fixture_schema();
    let epoch_dir = common::epoch_dir(&index);
    let before = tree(&epoch_dir.join(LEXICAL_DIR));
    assert!(
        !before.is_empty(),
        "the fixture must leave a lexical index on disk, otherwise the test proves nothing"
    );
    assert_eq!(
        read_state(&epoch_dir)
            .unwrap_or_else(|error| panic!("the copied lexical index must be readable: {error}"))
            .generation,
        Some(Generation::new(2)),
        "the fixture must leave the lexical index ahead of SQLite"
    );

    let store = GenerationStore::open_read_only(&index, &schema).unwrap_or_else(|error| {
        panic!(
            "a read-only open whose lexical index is ahead must succeed: {}: {}",
            error.code().as_str(),
            error.message()
        )
    });
    assert_eq!(
        store.generation(),
        Generation::new(1),
        "the read-only open must publish the generation SQLite committed"
    );
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::Ahead {
            committed: Generation::new(2)
        },
        "a payload ahead of the committed generation is the `ahead` state: the branch must name \
         it rather than assert a delay"
    );
    assert!(
        store.snapshot().lexical().is_none(),
        "a reader of generation 2 under a generation-1 snapshot would mix two generations"
    );
    let mut iids: Vec<u32> = store.snapshot().live_iids().map(|iid| iid.get()).collect();
    iids.sort_unstable();
    assert_eq!(
        iids,
        vec![0, 1],
        "the dense branch must serve the two live passages of generation 1, never the third \
         passage tantivy still holds"
    );
    drop(store);

    assert_eq!(
        tree(&epoch_dir.join(LEXICAL_DIR)),
        before,
        "a read-only open must not commit, rebuild or delete anything under lexical/, and must \
         not leave the tantivy writer lock behind (ADR-0056, ADR-0069, ADR-0087)"
    );
}
