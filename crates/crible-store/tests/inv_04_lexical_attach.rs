// SPDX-License-Identifier: Apache-2.0

//! INV-04 — the lexical branch of an index follows its configuration and its
//! committed generation.
//!
//! References: `docs/02-architecture.md` ("Organisation sur disque",
//! `epochs/<epoch>/lexical/`, and "Générations et point de commit", step 4),
//! `docs/03-recherche.md` ("Branche lexicale (tantivy)"), `docs/08-invariants-et-tests.md`
//! (INV-04) and the `INDEXER-TANTIVY-A-L-ATTACHE` task of
//! `docs/phases/P5-lexical-et-hybride.md`, whose two acceptance criteria are:
//!
//! - a crash between step 3 and step 4, then a restart, finds the lexical index
//!   at the committed generation — the crash case, in
//!   `tests/inv_04_crash_recovery.rs`, which needs the `failpoints` feature;
//! - an index whose `lexical.enabled` is `false` creates no lexical directory —
//!   the first test below, which needs no fault injection.
//!
//! The second test is the positive control of the same rule: an index that
//! enables the lexical branch does have one, and it indexes exactly the live
//! iids of its committed generation.
//!
//! The lexical state is read through [`crible_store::lexical::read_state`],
//! which opens the index of the epoch read-only and reports the generation of
//! its last commit and the iids it holds.
//!
//! ```text
//! cargo test -p crible-store --test inv_04_lexical_attach
//! ```

// The fixtures of this file call `panic!` outside the `#[test]` functions, as
// the other integration tests do; see `tests/common/mod.rs`.
#![allow(clippy::panic)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use crible_core::config::IndexConfig;
use crible_core::ids::Generation;
use crible_core::model::ModelConfig;
use crible_store::attach::attach;
use crible_store::generation::GenerationStore;
use crible_store::lexical::read_state;

/// External model of the fixture batch: four dimensions, normalized vectors.
fn fixture_model() -> ModelConfig {
    ModelConfig::from_json_str(
        r#"{ "name": "lexical-attach-external", "kind": "external", "dims": 4, "normalize": true }"#,
    )
    .unwrap_or_else(|error| panic!("the fixture model configuration must be valid: {error}"))
}

/// Attaches the fixture batch of two documents to `store`, with the
/// configuration the index declares.
fn attach_two_documents(store: &mut GenerationStore, index: &Path, batch: &Path) {
    let raw = fs::read_to_string(index.join("index.json")).unwrap_or_else(|error| {
        panic!(
            "the index configuration of {} must be readable: {error}",
            index.display()
        )
    });
    let config = IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("the index configuration must be valid: {error}"));
    let model = fixture_model();
    attach(store, batch, &config, &model, None, None).unwrap_or_else(|error| {
        panic!(
            "the fixture attach must succeed: {}: {}",
            error.code().as_str(),
            error.message()
        )
    });
}

/// Collects every directory named `lexical` under `dir`, recursively.
fn lexical_directories(dir: &Path, found: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        // A directory that does not exist holds no lexical index.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => panic!("{} must be readable: {error}", dir.display()),
    };
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| panic!("an entry must be readable: {error}"));
        let path = entry.path();
        if path.is_dir() {
            if entry.file_name() == "lexical" {
                found.push(path.clone());
            }
            lexical_directories(&path, found);
        }
    }
}

/// Asserts the branch is off all the way down the index: no `lexical/`
/// directory exists at any level, before or after the operation `when` names.
fn assert_no_lexical_directory(index: &Path, when: &str) {
    let mut found = Vec::new();
    lexical_directories(index, &mut found);
    assert!(
        found.is_empty(),
        "{when}: the index disables the lexical branch, so it must create no lexical \
         directory, but these exist: {found:?}"
    );
}

/// `lexical.enabled: false` is the setting of `docs/02` that spares the whole
/// tantivy side: no directory, not at creation, not on an attach, not on the
/// restart that follows.
#[test]
fn inv_04_lexical_disabled_creates_no_lexical_directory() {
    let (temp, index) = common::temp_index();
    let mut store = common::create_store(&index);
    assert_no_lexical_directory(&index, "creation of an index with lexical.enabled = false");

    let batch = common::write_batch(
        &temp.path().join("batches"),
        "lexical-disabled",
        &common::TWO_DOCUMENTS,
    );
    attach_two_documents(&mut store, &index, &batch);
    assert_eq!(
        store.generation().get(),
        1,
        "the attach must commit generation 1"
    );
    assert_no_lexical_directory(&index, "the attach of generation 1");
    drop(store);

    // The restart runs recovery, which must not create what the configuration
    // never asks for either.
    drop(common::open_store(&index));
    assert_no_lexical_directory(&index, "the restart of generation 1");
}

/// The positive control: an index that enables the lexical branch indexes the
/// passages of its committed generation, under their iids, and keeps them
/// across a restart. The text indexed is the encoder input (`docs/03`), which
/// is why the iids must match the live passages of the dense branch one to one.
#[test]
fn inv_04_attach_indexes_the_live_iids_of_the_committed_generation() {
    let (temp, index) = common::temp_index();
    let schema = common::fixture_schema();
    let config_json = common::test_config_with_lexical("test", &schema, true);
    let mut store = common::create_store_with_config(&index, &schema, &config_json);

    let batch = common::write_batch(
        &temp.path().join("batches"),
        "lexical-attach",
        &common::TWO_DOCUMENTS,
    );
    attach_two_documents(&mut store, &index, &batch);
    assert_eq!(
        store.generation().get(),
        1,
        "the attach must commit generation 1"
    );

    let epoch_dir = common::epoch_dir(&index);
    let state = read_state(&epoch_dir)
        .unwrap_or_else(|error| panic!("the lexical index must be readable: {error}"));
    assert_eq!(
        state.generation,
        Some(Generation::new(1)),
        "the last tantivy commit must name the generation it indexed"
    );
    let mut iids: Vec<u32> = state.iids.iter().map(|iid| iid.get()).collect();
    iids.sort_unstable();
    assert_eq!(
        iids,
        vec![0, 1],
        "the lexical index must hold the iids of the two live passages of generation 1"
    );
    drop(store);

    let store = common::open_store(&index);
    assert_eq!(
        store.generation().get(),
        1,
        "the restart must publish generation 1"
    );
    let state = read_state(&epoch_dir)
        .unwrap_or_else(|error| panic!("the lexical index must stay readable: {error}"));
    assert_eq!(
        state.generation,
        Some(Generation::new(1)),
        "the restart must leave a consistent lexical index where the commit left it"
    );
    let mut iids: Vec<u32> = state.iids.iter().map(|iid| iid.get()).collect();
    iids.sort_unstable();
    assert_eq!(
        iids,
        vec![0, 1],
        "the restart must not drop indexed passages"
    );
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

/// `Gt > G` names a state no commit of the epoch produced: an index whose
/// `lexical/` was copied from another epoch, or a restored epoch that kept the
/// lexical directory of a later generation. The restart rebuilds it from
/// `records.sqlite`, so the branch holds generation 1 and the iids `{0, 1}` of
/// that generation, never the third passage tantivy still carries (INV-04,
/// `ADR-0088`, decision 6).
///
/// This test needs no fault injection, so it runs in the default configuration
/// of the workspace: a state that only the recovery can repair is precisely what
/// a plain `cargo test` must cover.
#[test]
fn inv_04_lexical_ahead_of_the_sqlite_state_is_rebuilt_from_it() {
    let temp = tempfile::tempdir().unwrap_or_else(|error| {
        panic!("a temporary directory for the two indexes must be creatable: {error}")
    });
    let ahead = temp.path().join("indexes").join("ahead");
    let rebuilt = temp.path().join("indexes").join("rebuilt");
    let schema = common::fixture_schema();
    let config = common::test_config_with_lexical("test", &schema, true);
    let batches = temp.path().join("batches");
    let first_batch = common::write_batch(&batches, "lexical-first", &common::TWO_DOCUMENTS);
    let second_batch =
        common::write_batch(&batches, "lexical-second", &common::ONE_FURTHER_DOCUMENT);

    // Index A: generation 2, three passages, lexical at generation 2.
    {
        let mut store = common::create_store_with_config(&ahead, &schema, &config);
        attach_two_documents(&mut store, &ahead, &first_batch);
        attach_two_documents(&mut store, &ahead, &second_batch);
        assert_eq!(store.generation().get(), 2);
    }
    // Index B: generation 1, two passages, lexical at generation 1.
    {
        let mut store = common::create_store_with_config(&rebuilt, &schema, &config);
        attach_two_documents(&mut store, &rebuilt, &first_batch);
        assert_eq!(store.generation().get(), 1);
    }

    // The derived lexical index of B is replaced by the one of A: B's tantivy
    // now names generation 2 and holds iid 2, which B's SQLite never wrote.
    let source = common::epoch_dir(&ahead).join("lexical");
    let target = common::epoch_dir(&rebuilt).join("lexical");
    match fs::remove_dir_all(&target) {
        Ok(()) => {}
        // B has no lexical directory to replace: the copy below creates it,
        // which leaves the same inconsistency.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!(
            "the lexical directory {} must be removable: {error}",
            target.display()
        ),
    }
    copy_directory(&source, &target);

    let broken = read_state(&common::epoch_dir(&rebuilt))
        .unwrap_or_else(|error| panic!("the copied lexical index must be readable: {error}"));
    assert_eq!(
        broken.generation,
        Some(Generation::new(2)),
        "the fixture must leave the lexical index ahead of SQLite: {:?}",
        broken.generation
    );

    // Opening the index rebuilds the lexical branch from SQLite, and the
    // snapshot serves it: the rebuild is a repair of a derived state, so it
    // publishes neither a new generation nor a lexical state a query must refuse.
    let store = common::open_store(&rebuilt);
    assert_eq!(
        store.generation().get(),
        1,
        "the rebuild must not create a generation"
    );
    assert_eq!(
        store.snapshot().lexical_status(),
        crible_store::lexical::LexicalStatus::Ready {
            committed: Generation::new(1)
        },
        "the rebuilt index is at the committed generation, so the snapshot exposes its reader"
    );
    assert!(
        store.snapshot().lexical().is_some(),
        "the snapshot of the repaired index must expose its reader"
    );
    let state = read_state(&common::epoch_dir(&rebuilt))
        .unwrap_or_else(|error| panic!("the rebuilt lexical index must be readable: {error}"));
    assert_eq!(
        state.generation,
        Some(Generation::new(1)),
        "the rebuild must commit the generation SQLite holds"
    );
    let mut iids: Vec<u32> = state.iids.iter().map(|iid| iid.get()).collect();
    iids.sort_unstable();
    assert_eq!(
        iids,
        vec![0, 1],
        "the rebuild must hold the two live passages of generation 1, never the third one"
    );
}
