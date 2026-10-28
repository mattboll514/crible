// SPDX-License-Identifier: Apache-2.0

//! INV-04 — an index is up to date when `M ≤ Gt ≤ G`, and an unreadable lexical
//! state is repaired on a write open only (`ADR-0088`).
//!
//! References: `docs/02-architecture.md` ("Reprise au démarrage"),
//! `docs/03-recherche.md` ("Branche lexicale (tantivy)"),
//! `docs/04-donnees-et-ingestion.md` ("Patch de métadonnées"),
//! `docs/05-api.md` (`overloaded`), `docs/08-invariants-et-tests.md` (INV-04),
//! `ADR-0056` (a read does not modify the disk), `ADR-0060` (the patch contract),
//! `ADR-0087` (recovery on a write open) and `ADR-0088` (marker, narrow rule,
//! unreadable states).
//!
//! `M` is `meta.passage_change_generation`, the last generation that created or
//! killed a passage; `Gt` is the payload of the last tantivy commit; `G` is the
//! generation `records.sqlite` holds. What each test pins:
//!
//! | Test | Criterion of the fiche |
//! |---|---|
//! | `inv_04_a_patch_keeps_the_lexical_branch_of_a_read_only_open` | patch then read-only open: the reader is present and serves `G` |
//! | `inv_04_a_patch_creates_no_tantivy_commit_at_the_next_write_open` | patch then write open: no tantivy commit, payload unchanged |
//! | `inv_04_an_unreadable_payload_is_reported_read_only_and_rebuilt_for_writing` | non-decimal payload: read-only serves `dense` without a reader, write rebuilds |
//! | `inv_04_a_lexical_directory_tantivy_cannot_open_is_rebuilt_for_writing` | unreadable `lexical/`: same two behaviours |
//! | `inv_04_an_epoch_without_the_marker_uses_the_narrow_rule` | absent column: strict `Gt = G` read-only, computed and written on a write open |
//! | `inv_04_an_uncertified_epoch_without_a_lexical_index_is_not_missing` | absent column and no index: `NotCertified`, never `Missing` |
//! | `inv_04_the_marker_advances_only_when_a_generation_touches_a_passage` | ADR-0088, decision 3, the risk noted in "À surveiller" |
//!
//! ```text
//! cargo test -p crible-store --test inv_04_lexical_marker
//! ```

// The fixtures of this file call `panic!` outside the `#[test]` functions, as
// the other integration tests do; see `tests/common/mod.rs`.
#![allow(clippy::panic)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use crible_core::config::IndexConfig;
use crible_core::ids::{Generation, Iid};
use crible_core::model::ModelConfig;
use crible_store::attach::attach;
use crible_store::generation::GenerationStore;
use crible_store::lexical::{LexicalStatus, read_state};
use crible_store::patch::{self, DocumentPatch, PatchSource};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};

/// Directory of the tantivy index inside an epoch directory.
const LEXICAL_DIR: &str = "lexical";

/// External model of the fixture batch: four dimensions, normalized vectors.
fn fixture_model() -> ModelConfig {
    ModelConfig::from_json_str(
        r#"{ "name": "lexical-marker-external", "kind": "external", "dims": 4, "normalize": true }"#,
    )
    .unwrap_or_else(|error| panic!("the fixture model configuration must be valid: {error}"))
}

/// Reads the configuration the index declares, which `attach` takes as an
/// argument.
fn index_config_of(index: &Path) -> IndexConfig {
    let raw = fs::read_to_string(index.join("index.json")).unwrap_or_else(|error| {
        panic!(
            "the index configuration of {} must be readable: {error}",
            index.display()
        )
    });
    IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("the index configuration must be valid: {error}"))
}

/// Attaches `batch` to `store`, with the configuration the index declares.
fn attach_batch(store: &mut GenerationStore, index: &Path, batch: &Path) {
    let config = index_config_of(index);
    let model = fixture_model();
    attach(store, batch, &config, &model, None, None).unwrap_or_else(|error| {
        panic!(
            "the fixture attach of {} must succeed: {}: {}",
            batch.display(),
            error.code().as_str(),
            error.message()
        )
    });
}

/// Creates an index whose lexical branch is enabled and attaches the two
/// documents of the fixture batch: generation 1, two live passages, `M = Gt = 1`.
fn index_at_generation_one() -> (tempfile::TempDir, PathBuf) {
    let (temp, index) = common::temp_index();
    let schema = common::fixture_schema();
    let config = common::test_config_with_lexical("test", &schema, true);
    let mut store = common::create_store_with_config(&index, &schema, &config);
    let batch = common::write_batch(
        &temp.path().join("batches"),
        "lexical-marker",
        &common::TWO_DOCUMENTS,
    );
    attach_batch(&mut store, &index, &batch);
    (temp, index)
}

/// Applies a `title` patch to `doc-a`, which changes a document field only: the
/// generation moves to `G+1`, no text does.
fn patch_doc_a(store: &mut GenerationStore, schema: &crible_core::config::Schema) {
    let Value::Object(fields) = json!({ "title": "Code civil, article 1240" }) else {
        panic!("the fields of the fixture patch must be a JSON object");
    };
    let report = patch::apply(
        store,
        schema,
        PatchSource::File,
        &[DocumentPatch {
            doc_id: "doc-a".to_owned(),
            fields,
        }],
    )
    .unwrap_or_else(|error| {
        panic!(
            "the fixture patch must succeed: {}: {}",
            error.code().as_str(),
            error.message()
        )
    });
    assert_eq!(
        report.updated, 1,
        "the fixture patch must change one document"
    );
}

/// Reads `meta.passage_change_generation` of the epoch, `None` when the column
/// is absent or holds `NULL`.
fn marker(index: &Path) -> Option<u64> {
    let connection = Connection::open_with_flags(
        common::records_path(index),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap_or_else(|error| panic!("records.sqlite must open read-only: {error}"));
    let read = connection.query_row("SELECT * FROM meta WHERE id = 1", [], |row| {
        match row.get::<_, Option<i64>>("passage_change_generation") {
            Ok(value) => Ok(Some(value)),
            // The column is absent: the epoch predates P5 and is not certified.
            Err(rusqlite::Error::InvalidColumnName(_)) => Ok(None),
            Err(error) => Err(error),
        }
    });
    let marker = read
        .unwrap_or_else(|error| panic!("meta.passage_change_generation must be readable: {error}"));
    marker.flatten().map(|value| {
        u64::try_from(value).unwrap_or_else(|_| panic!("a generation is never negative"))
    })
}

/// Every file under `root`, recursively, as `(relative path, bytes)`, sorted by
/// path, so two listings of the same directory compare directly.
fn tree(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files);
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

/// Recursive body of [`tree`]: an absent directory holds no file.
fn collect_files(root: &Path, dir: &Path, files: &mut Vec<(String, Vec<u8>)>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
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
        files.push((relative, bytes));
    }
}

/// Rewrites the payload of the last tantivy commit, leaving every other member
/// of `meta.json` as tantivy wrote it.
fn break_payload(index: &Path) {
    let meta = common::epoch_dir(index).join(LEXICAL_DIR).join("meta.json");
    let raw = fs::read_to_string(&meta)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", meta.display()));
    let mut value: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("{} must hold JSON: {error}", meta.display()));
    value["payload"] = json!("not-a-generation");
    fs::write(
        &meta,
        serde_json::to_string(&value)
            .unwrap_or_else(|error| panic!("the tampered meta.json must serialize: {error}")),
    )
    .unwrap_or_else(|error| panic!("{} must be writable: {error}", meta.display()));
}

/// Opens an index for writing, panicking on failure: the write open of a
/// derived state this build can repair must never fail.
fn write_open(index: &Path) -> GenerationStore {
    GenerationStore::open(index, &common::fixture_schema()).unwrap_or_else(|error| {
        panic!(
            "a write open must repair the lexical state of {}: {}: {}",
            index.display(),
            error.code().as_str(),
            error.message()
        )
    })
}

/// Opens an index read-only, panicking on failure: a derived file must never
/// make the reference state unreadable (INV-02, ADR-0088 decision 7).
fn read_only_open(index: &Path) -> GenerationStore {
    GenerationStore::open_read_only(index, &common::fixture_schema()).unwrap_or_else(|error| {
        panic!(
            "a read-only open of {} must succeed without the lexical branch: {}: {}",
            index.display(),
            error.code().as_str(),
            error.message()
        )
    })
}

/// Asserts the snapshot serves exactly `iids` as its live passages.
fn assert_live(store: &GenerationStore, iids: &[u32]) {
    let mut actual: Vec<u32> = store.snapshot().live_iids().map(|iid| iid.get()).collect();
    actual.sort_unstable();
    assert_eq!(
        actual, iids,
        "the dense branch must serve exactly the live passages of the generation"
    );
}

/// Asserts the lexical index holds exactly `iids` at `generation`.
fn assert_lexical_state(index: &Path, generation: u64, iids: &[u32]) {
    let epoch_dir = common::epoch_dir(index);
    let state = read_state(&epoch_dir)
        .unwrap_or_else(|error| panic!("the lexical index must be readable: {error}"));
    assert_eq!(
        state.generation,
        Some(Generation::new(generation)),
        "the last tantivy commit must name the generation it indexed"
    );
    let mut actual: Vec<u32> = state.iids.iter().map(|iid| iid.get()).collect();
    actual.sort_unstable();
    assert_eq!(
        actual, iids,
        "the lexical index must hold exactly the live iids of the generation"
    );
}

/// The criterion of the fiche: a metadata patch changes no text, so the lexical
/// index of the generation it produced is still up to date, and a **read-only**
/// open must serve it. Before `ADR-0088`, `Gt = G-1` made the snapshot reader
/// absent and the lexical branch refuse an index that was perfectly sound.
#[test]
fn inv_04_a_patch_keeps_the_lexical_branch_of_a_read_only_open() {
    let (_temp, index) = index_at_generation_one();
    let schema = common::fixture_schema();
    let epoch_dir = common::epoch_dir(&index);
    assert_eq!(
        marker(&index),
        Some(1),
        "the attach creates a passage: M = 1"
    );
    {
        let mut store = common::open_store(&index);
        patch_doc_a(&mut store, &schema);
    }
    assert_eq!(
        marker(&index),
        Some(1),
        "a patch changes no passage, so it must leave the marker where the last passage \
         change put it (ADR-0088, decision 3)"
    );
    assert_eq!(
        read_state(&epoch_dir)
            .unwrap_or_else(|error| panic!("the lexical index must be readable: {error}"))
            .generation,
        Some(Generation::new(1)),
        "a patch produces no tantivy commit, so the payload stays at the older generation"
    );

    let store = read_only_open(&index);
    assert_eq!(
        store.generation(),
        Generation::new(2),
        "the read-only open must publish the generation SQLite committed"
    );
    assert_live(&store, &[0, 1]);
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::Ready {
            committed: Generation::new(1)
        },
        "M = 1 ≤ Gt = 1 ≤ G = 2: the lexical index describes generation 2 as well as \
         generation 1 (ADR-0088, decision 2)"
    );
    assert!(
        store.snapshot().lexical().is_some(),
        "the snapshot of the read-only open must expose the reader of the up-to-date \
         lexical index: it serves G with the commit Gt"
    );
}

/// The second half of the same criterion: a patch produces no tantivy commit, so
/// the write open that follows must not create one either — no new segment, no
/// new payload, only the snapshot.
#[test]
fn inv_04_a_patch_creates_no_tantivy_commit_at_the_next_write_open() {
    let (_temp, index) = index_at_generation_one();
    let schema = common::fixture_schema();
    let lexical_dir = common::epoch_dir(&index).join(LEXICAL_DIR);
    {
        let mut store = common::open_store(&index);
        patch_doc_a(&mut store, &schema);
    }
    let before = tree(&lexical_dir);
    assert!(
        !before.is_empty(),
        "the fixture must hold a lexical index, otherwise the test proves nothing"
    );

    let store = write_open(&index);
    drop(store);

    assert_eq!(
        tree(&lexical_dir),
        before,
        "no generation of `]Gt, G]` touched a passage, so the repair must replay nothing, \
         commit nothing and delete nothing under lexical/ (ADR-0088, decision 2)"
    );
    assert_eq!(
        marker(&index),
        Some(1),
        "the write open must not move the marker either"
    );
    assert_lexical_state(&index, 1, &[0, 1]);

    let store = read_only_open(&index);
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::Ready {
            committed: Generation::new(1)
        },
        "the state the write open left is the one the read-only open must observe"
    );
}

/// A payload this build cannot read makes `Gt` undecidable: a write open rebuilds
/// the derived directory instead of refusing to start, and a read-only open
/// serves the reference state without the lexical branch (`ADR-0088`,
/// decisions 6 and 7).
#[test]
fn inv_04_an_unreadable_payload_is_reported_read_only_and_rebuilt_for_writing() {
    let (_temp, index) = index_at_generation_one();
    break_payload(&index);
    let lexical_dir = common::epoch_dir(&index).join(LEXICAL_DIR);
    let tampered = tree(&lexical_dir);

    let store = read_only_open(&index);
    assert_eq!(
        store.generation(),
        Generation::new(1),
        "the read-only open must publish the generation SQLite committed"
    );
    assert_live(&store, &[0, 1]);
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::Unreadable,
        "a payload that is not a generation is an unreadable state, not a delay"
    );
    assert!(
        store.snapshot().lexical().is_none(),
        "an unreadable state opens no reader"
    );
    drop(store);
    assert_eq!(
        tree(&lexical_dir),
        tampered,
        "a read-only open must not repair the lexical index: nothing is written under it \
         (ADR-0056, ADR-0087)"
    );

    let store = write_open(&index);
    assert_eq!(
        store.generation(),
        Generation::new(1),
        "the rebuild is a derived state: it must not create a generation"
    );
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::Ready {
            committed: Generation::new(1)
        },
        "the write open must rebuild and publish a reader over the rebuilt index"
    );
    assert!(
        store.snapshot().lexical().is_some(),
        "the snapshot of the repaired index must expose its reader"
    );
    assert_lexical_state(&index, 1, &[0, 1]);
}

/// A `lexical/` tantivy cannot open — here a `meta.json` that is not JSON — is
/// the same undecidable state as an unreadable payload: rebuilt for writing,
/// reported without a reader for reading.
#[test]
fn inv_04_a_lexical_directory_tantivy_cannot_open_is_rebuilt_for_writing() {
    let (_temp, index) = index_at_generation_one();
    let meta = common::epoch_dir(&index)
        .join(LEXICAL_DIR)
        .join("meta.json");
    fs::write(&meta, b"{ this is not tantivy's meta.json")
        .unwrap_or_else(|error| panic!("{} must be writable: {error}", meta.display()));

    let store = read_only_open(&index);
    assert_eq!(store.snapshot().lexical_status(), LexicalStatus::Unreadable);
    assert!(store.snapshot().lexical().is_none());
    assert_live(&store, &[0, 1]);
    drop(store);

    let store = write_open(&index);
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::Ready {
            committed: Generation::new(1)
        },
        "the write open must replace the directory it cannot read instead of refusing"
    );
    assert_lexical_state(&index, 1, &[0, 1]);
}

/// An epoch whose marker column is absent is not certified for anything but
/// `Gt = G`: a read-only open applies the narrow rule of `ADR-0088`, decision 4,
/// and the next write open adds the column, computes the marker once and repairs.
#[test]
fn inv_04_an_epoch_without_the_marker_uses_the_narrow_rule() {
    let (_temp, index) = index_at_generation_one();
    let schema = common::fixture_schema();
    {
        let mut store = common::open_store(&index);
        patch_doc_a(&mut store, &schema);
    }
    assert_eq!(
        store_generation(&index),
        2,
        "the patch must commit generation 2"
    );
    // The column is dropped after the patch: the fixture is an epoch written
    // before P5, never opened for writing by this build since.
    {
        let connection = Connection::open(common::records_path(&index))
            .unwrap_or_else(|error| panic!("records.sqlite must open: {error}"));
        connection
            .execute("ALTER TABLE meta DROP COLUMN passage_change_generation", [])
            .unwrap_or_else(|error| panic!("the marker column must be removable: {error}"));
    }
    assert_eq!(marker(&index), None, "the fixture must drop the column");

    let store = read_only_open(&index);
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::NotCertified {
            committed: Some(Generation::new(1))
        },
        "without the marker, `Gt = G - 1` is not certified: the criterion is never widened"
    );
    assert!(store.snapshot().lexical().is_none());
    drop(store);
    assert_eq!(
        marker(&index),
        None,
        "a read-only open must never add the column nor write the marker"
    );

    let store = write_open(&index);
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::Ready {
            committed: Generation::new(1)
        },
        "the completed marker, 1, is at most the payload 1, so the index is up to date"
    );
    assert!(store.snapshot().lexical().is_some());
    drop(store);
    assert_eq!(
        marker(&index),
        Some(1),
        "the write open must compute the marker once: the last generation that created a \
         passage is 1"
    );
    assert_eq!(
        store_generation(&index),
        2,
        "completing the marker must create no generation"
    );
    assert_lexical_state(&index, 1, &[0, 1]);
}

/// The risk `ADR-0088` names in "À surveiller": a marker left behind a passage
/// change certifies an index that missed it. It must advance on an attach and on
/// a tombstone, and only then.
#[test]
fn inv_04_the_marker_advances_only_when_a_generation_touches_a_passage() {
    let (_temp, index) = index_at_generation_one();
    let schema = common::fixture_schema();
    let mut store = common::open_store(&index);
    assert_eq!(
        marker(&index),
        Some(1),
        "the attach creates two passages at 1"
    );

    patch_doc_a(&mut store, &schema);
    assert_eq!(marker(&index), Some(1), "a patch leaves the marker alone");

    // A generation that only tombstones a passage: step 4 runs and the marker
    // must follow, or the repaired index would pass for up to date.
    common::commit(
        &mut store,
        common::change(Vec::new(), Vec::new(), Vec::new(), vec![Iid::new(0)]),
    );
    assert_eq!(
        marker(&index),
        Some(3),
        "a tombstone changes the set of live passages, so it must move the marker"
    );
    assert_eq!(store.generation(), Generation::new(3));
    assert_lexical_state(&index, 3, &[1]);
    drop(store);

    let store = read_only_open(&index);
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::Ready {
            committed: Generation::new(3)
        }
    );
    assert!(store.snapshot().lexical().is_some());
}

/// Generation `records.sqlite` holds, read through a fresh open of the database.
fn store_generation(index: &Path) -> u64 {
    let connection = Connection::open_with_flags(
        common::records_path(index),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap_or_else(|error| panic!("records.sqlite must open read-only: {error}"));
    let generation: i64 = connection
        .query_row("SELECT generation FROM meta WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap_or_else(|error| panic!("meta.generation must be readable: {error}"));
    u64::try_from(generation).unwrap_or_else(|_| panic!("a generation is never negative"))
}

/// An epoch written before P5 whose `lexical/` is absent is **not** `Missing`:
/// `Missing` says the epoch never carried a passage, and only the marker proves
/// that. Without the column, the absence of an index certifies nothing, so the
/// read-only open must report [`LexicalStatus::NotCertified`] and the write open
/// must complete the column, find the index behind the marker and rebuild it
/// (`ADR-0088`, decisions 4 and 6).
#[test]
fn inv_04_an_uncertified_epoch_without_a_lexical_index_is_not_missing() {
    let (_temp, index) = index_at_generation_one();
    let lexical_dir = common::epoch_dir(&index).join(LEXICAL_DIR);
    {
        let connection = Connection::open(common::records_path(&index))
            .unwrap_or_else(|error| panic!("records.sqlite must open: {error}"));
        connection
            .execute("ALTER TABLE meta DROP COLUMN passage_change_generation", [])
            .unwrap_or_else(|error| panic!("the marker column must be removable: {error}"));
    }
    fs::remove_dir_all(&lexical_dir)
        .unwrap_or_else(|error| panic!("the lexical directory must be removable: {error}"));
    assert_eq!(marker(&index), None, "the fixture must drop the column");

    let store = read_only_open(&index);
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::NotCertified { committed: None },
        "without the marker, an absent lexical index says nothing about the passages the epoch \
         holds: it is not `Missing`, which asserts that no passage was ever created"
    );
    assert!(store.snapshot().lexical().is_none());
    assert_live(&store, &[0, 1]);
    drop(store);
    assert!(
        !lexical_dir.exists(),
        "a read-only open must not create the lexical directory it observed absent"
    );

    let store = write_open(&index);
    assert_eq!(
        marker(&index),
        Some(1),
        "the write open must complete the marker from the passages the epoch holds"
    );
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::Ready {
            committed: Generation::new(1)
        },
        "with the marker at 1 and no committed index, the epoch is behind, so the write open \
         rebuilds it and publishes a reader"
    );
    assert!(store.snapshot().lexical().is_some());
    assert_eq!(
        store_generation(&index),
        1,
        "completing the marker and rebuilding the index create no generation"
    );
    assert_lexical_state(&index, 1, &[0, 1]);
}
