// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `CREER-ET-RESTAURER-INSTANTANE` (phase P3): `snapshot::create` and
//! `snapshot::restore`.
//!
//! References: the task sheet (`docs/phases/P3-cycle-de-vie.md:121-130`),
//! `docs/04-donnees-et-ingestion.md` ("Règles de l'instantané", "Règles de la
//! restauration"), `docs/02-architecture.md` ("Restauration d'un instantané"),
//! `ADR-0064` (format and content of a snapshot, its five refusals) and
//! `ADR-0065` (offline snapshot, epoch of a restore, target index, retention of
//! the replaced epoch, write protocol).
//!
//! | Test | Criterion / decision |
//! |---|---|
//! | `snapshot_then_restore_keeps_the_generation_and_the_rows` | a restore gives back G and the same rows (task criterion 1; ADR-0065 §2) |
//! | `restore_rolls_back_and_keeps_the_replaced_epoch` | a higher-generation target is accepted, the replaced epoch stays (ADR-0065 §3, §4) |
//! | `restore_creates_an_absent_index_at_the_initial_epoch` | the absent target gets the initial epoch, generation G (ADR-0065 §2, §3) |
//! | `restore_refuses_a_divergent_index_json` | a different `index.json` is refused, the setting named (ADR-0065 §3) |
//! | `restore_refuses_an_absent_attestation` | no `snapshot.json` (ADR-0064 §1) |
//! | `restore_refuses_an_unknown_format_version` | unknown `format_version` (ADR-0064 §1, INV-15) |
//! | `restore_refuses_a_missing_listed_file` | a missing listed file (task criterion 2; ADR-0064 §1) |
//! | `restore_refuses_a_size_mismatch` | a size that differs (ADR-0064 §1) |
//! | `restore_refuses_a_blake3_mismatch` | a blake3 that differs (ADR-0064 §1) |
//! | `restore_refuses_an_unlisted_file` | an unlisted file (ADR-0064 §1) |
//! | `restore_refuses_a_path_outside_the_snapshot` | a path escaping the snapshot (ADR-0064 §1) |
//! | `a_second_snapshot_of_the_same_epoch_is_refused` | an occupied destination (ADR-0064 §3) |
//! | `restore_removes_a_stale_staging_and_target_epoch` | a residue `tmp-N` or `N` is removed first (ADR-0065 §5) |
//! | `a_failure_during_the_write_leaves_the_current_epoch_intact` | a failure before `CURRENT_EPOCH` leaves the target intact (ADR-0065 §5) |
//! | `an_index_without_current_epoch_is_refused_at_open` | a partial tree is refused at open (ADR-0065 §5) |
//! | `the_attestation_is_deterministic` | two snapshots of one epoch are byte-identical (ADR-0064 §1) |

use std::fs;
use std::path::{Path, PathBuf};

use crible_core::error::ErrorCode;
use crible_core::ids::{Epoch, Generation, Iid};
use crible_store::generation::{GenerationStore, INDEX_CONFIG_FILE_NAME};
use crible_store::snapshot::{self, RestoreReport, SNAPSHOT_FILE_NAME, SnapshotReport};
use serde_json::{Value, json};

mod common;

/// Name of the index the fixtures create.
const INDEX: &str = "invariants";

/// One temporary instance: the data root, the index, the exchange root. The
/// index starts at generation 1; [`Instance::advance`] commits generation 2.
struct Instance {
    temp: tempfile::TempDir,
    data: PathBuf,
    index: PathBuf,
    exchange: PathBuf,
}

impl Instance {
    /// Creates the index at generation 1: two documents and three passages.
    fn new() -> Self {
        let (temp, index) = common::temp_index();
        let data = index
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| panic!("the fixture index must live under <data>/indexes/"));
        let exchange = temp.path().join("exchange");
        let mut store = create_fixture(&index);
        let first = common::write_segment(
            &index,
            "seg-1",
            0,
            1,
            &[
                &[1.0_f32, 0.0, 0.0, 0.0][..],
                &[0.0, 1.0, 0.0, 0.0],
                &[0.0, 0.0, 1.0, 0.0],
            ],
        );
        common::commit(
            &mut store,
            common::change(
                vec![first],
                vec![common::document("D1", 1), common::document("D2", 1)],
                vec![
                    common::passage(0, "D1", 0, "seg-1", 1),
                    common::passage(1, "D1", 1, "seg-1", 1),
                    common::passage(2, "D2", 0, "seg-1", 1),
                ],
                Vec::new(),
            ),
        );
        Self {
            temp,
            data,
            index,
            exchange,
        }
    }

    /// Commits generation 2: a third document, its segment and its passage.
    fn advance(&self) {
        let mut store = common::open_store(&self.index);
        let segment =
            common::write_segment(&self.index, "seg-2", 3, 2, &[&[0.0_f32, 0.0, 0.0, 1.0][..]]);
        common::commit(
            &mut store,
            common::change(
                vec![segment],
                vec![common::document("D3", 2)],
                vec![common::passage(3, "D3", 0, "seg-2", 2)],
                Vec::new(),
            ),
        );
    }

    /// Takes the snapshot of the fixture index and returns its report.
    fn snapshot(&self) -> SnapshotReport {
        let mut store = common::open_store(&self.index);
        let config = crible_core::config::IndexConfig::from_json_str(&common::test_config(
            INDEX,
            &common::fixture_schema(),
        ))
        .unwrap_or_else(|error| panic!("the fixture configuration must be valid: {error}"));
        snapshot::create(&mut store, &config, &self.exchange)
            .unwrap_or_else(|error| panic!("the snapshot must be taken: {error}"))
    }

    /// Directory of the snapshot of the first generation.
    fn snapshot_directory(&self) -> PathBuf {
        self.exchange
            .join("snapshots")
            .join(format!("{INDEX}-e1-g1"))
    }

    /// Bytes of the `CURRENT_EPOCH` pointer, the state a refusal must not touch.
    fn pointer(&self) -> String {
        fs::read_to_string(self.index.join("CURRENT_EPOCH"))
            .unwrap_or_else(|error| panic!("the pointer must be readable: {error}"))
    }

    /// Restores the first-generation snapshot into this instance.
    fn restore(&self, directory: &Path) -> Result<RestoreReport, crible_core::error::CribleError> {
        snapshot::restore(&self.data, directory)
    }
}

/// Creates the fixture index under the name its directory carries.
///
/// `common::create_store` writes the shared configuration, whose `name` is
/// `test`; the CLI refuses an index directory and a configuration that disagree,
/// and the restore does too, so this fixture writes the matching name.
fn create_fixture(index: &Path) -> GenerationStore {
    let schema = common::fixture_schema();
    crible_store::generation::write_index_config(index, &common::test_config(INDEX, &schema))
        .unwrap_or_else(|error| panic!("the fixture configuration must be written: {error}"));
    GenerationStore::create(index, Epoch::new(1), &schema)
        .unwrap_or_else(|error| panic!("the fixture index must be created: {error}"))
}

/// Reads the attestation of a snapshot as a mutable JSON value.
fn attestation(directory: &Path) -> Value {
    let text = fs::read_to_string(directory.join(SNAPSHOT_FILE_NAME))
        .unwrap_or_else(|error| panic!("snapshot.json must be readable: {error}"));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("snapshot.json must be valid JSON: {error}"))
}

/// Writes the attestation of a snapshot back.
fn write_attestation(directory: &Path, value: &Value) {
    fs::write(
        directory.join(SNAPSHOT_FILE_NAME),
        serde_json::to_vec(value)
            .unwrap_or_else(|error| panic!("the value must serialize: {error}")),
    )
    .unwrap_or_else(|error| panic!("snapshot.json must be written: {error}"));
}

/// Rewrites the attestation entry of `relative` from the file on disk, as a
/// snapshot whose content was altered consistently would carry it.
fn re_attest(directory: &Path, relative: &str) {
    let path = directory.join(relative);
    let size = fs::metadata(&path)
        .unwrap_or_else(|error| panic!("the file must be measurable: {error}"))
        .len();
    let hasher = blake3::hash(
        &fs::read(&path).unwrap_or_else(|error| panic!("the file must be readable: {error}")),
    );
    let digest = format!("blake3:{}", hasher.to_hex());
    let mut value = attestation(directory);
    let files = value["files"]
        .as_array_mut()
        .unwrap_or_else(|| panic!("the attestation lists files"));
    let entry = files
        .iter_mut()
        .find(|file| file["path"] == json!(relative))
        .unwrap_or_else(|| panic!("the attestation lists {relative}"));
    entry["size"] = json!(size);
    entry["blake3"] = json!(digest);
    write_attestation(directory, &value);
}

/// Asserts that a refusal carries `invalid_request` and that the target index
/// was not touched.
fn assert_refused(instance: &Instance, directory: &Path, needle: &str) {
    let pointer = instance.pointer();
    let error = instance
        .restore(directory)
        .err()
        .unwrap_or_else(|| panic!("the restore must be refused"));
    assert_eq!(
        error.code(),
        ErrorCode::InvalidRequest,
        "a snapshot Crible cannot use is refused in invalid_request: {}",
        error.message()
    );
    assert!(
        error.message().contains(needle),
        "the refusal must name {needle:?}: {}",
        error.message()
    );
    assert_eq!(
        instance.pointer(),
        pointer,
        "a refused restore must not move CURRENT_EPOCH"
    );
}

/// Restores a snapshot into another data root.
fn restore_into(data: &Path, directory: &Path) -> RestoreReport {
    snapshot::restore(data, directory)
        .unwrap_or_else(|error| panic!("the restore must succeed: {error}"))
}

#[test]
fn snapshot_then_restore_keeps_the_generation_and_the_rows() {
    // The task criterion: a restore gives back the generation and the same
    // rows. The snapshot is taken at G1, the index then moves to G2, so the
    // restore is a real rollback of the visible state.
    let instance = Instance::new();
    let report = instance.snapshot();
    assert_eq!(
        (report.epoch.get(), report.generation.get()),
        (1, 1),
        "the snapshot describes the epoch and the generation it copied"
    );
    assert_eq!(
        (report.documents, report.passages),
        (2, 3),
        "the report counts the live rows of the copied generation"
    );
    assert_eq!(report.directory, instance.snapshot_directory());
    assert!(report.bytes > 0, "the report counts the copied bytes");

    instance.advance();
    assert_eq!(
        common::open_store(&instance.index).generation(),
        Generation::new(2),
        "the index moved past the snapshot before the restore"
    );

    let restored = instance.snapshot_directory();
    let report = instance
        .restore(&restored)
        .unwrap_or_else(|error| panic!("the restore must succeed: {error}"));
    assert_eq!(
        (report.epoch.get(), report.generation.get()),
        (2, 1),
        "the snapshot is installed as epoch E+1, with the generation left at G"
    );
    assert_eq!(
        report.replaced_generation,
        Some(Generation::new(2)),
        "the report names the generation the restore replaced"
    );
    assert_eq!(
        (report.documents, report.passages),
        (2, 3),
        "the restored state holds the live rows of the snapshot"
    );

    let store = common::open_store(&instance.index);
    assert_eq!(
        store.epoch(),
        Epoch::new(2),
        "CURRENT_EPOCH names the restored epoch"
    );
    assert_eq!(
        store.generation(),
        Generation::new(1),
        "a restore does not increment the generation"
    );
    let live: Vec<Iid> = store.snapshot().live_iids().collect();
    assert_eq!(
        live,
        vec![Iid::new(0), Iid::new(1), Iid::new(2)],
        "the iid are copied unchanged, so the pool is the one of the snapshot"
    );
    let segments: Vec<String> = store
        .snapshot()
        .segments()
        .iter()
        .map(|meta| meta.segment_id.clone())
        .collect();
    assert_eq!(
        segments,
        vec!["seg-1"],
        "the restored generation references the segments of the snapshot, not the ones added after it"
    );
    let state = store
        .state()
        .unwrap_or_else(|error| panic!("the restored state must be readable: {error}"));
    assert_eq!(state.documents, 2);
    assert_eq!(state.live_passages, 3);
    assert_eq!(state.dead_passages, 0, "generation 1 held no tombstone");
}

#[test]
fn restore_rolls_back_and_keeps_the_replaced_epoch() {
    // ADR-0065 decisions 3 and 4: a target at a higher generation is accepted
    // (deliberate rollback), and the replaced epoch stays on disk because it may
    // hold passages no valid copy carries (INV-14).
    let instance = Instance::new();
    instance.snapshot();
    instance.advance();
    let directory = instance.snapshot_directory();
    let report = instance
        .restore(&directory)
        .unwrap_or_else(|error| panic!("the rollback must succeed: {error}"));
    assert_eq!(report.replaced_epoch, Some(Epoch::new(1)));
    assert_eq!(report.replaced_generation, Some(Generation::new(2)));
    assert!(
        instance.index.join("epochs").join("1").is_dir(),
        "the replaced epoch is kept, never reclaimed by the restore"
    );
    assert!(
        instance.index.join("epochs").join("2").is_dir(),
        "the restored epoch has its own directory"
    );
    // The kept epoch is the one the rollback replaced: it still holds the rows
    // added after the snapshot.
    let kept = rusqlite::Connection::open(
        instance
            .index
            .join("epochs")
            .join("1")
            .join("records.sqlite"),
    )
    .unwrap_or_else(|error| panic!("the replaced epoch must still open: {error}"));
    let documents: i64 = kept
        .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
        .unwrap_or_else(|error| panic!("the replaced epoch must be readable: {error}"));
    assert_eq!(
        documents, 3,
        "the kept epoch is the state that followed the snapshot"
    );
}

#[test]
fn restore_creates_an_absent_index_at_the_initial_epoch() {
    // ADR-0065 decision 2: an absent target has no epoch to pass, so it gets the
    // initial epoch of a creation and the generation of the snapshot.
    let instance = Instance::new();
    instance.snapshot();
    let directory = instance.snapshot_directory();

    let fresh = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("a temporary data root must be creatable: {error}"));
    let data = fresh.path().join("data");
    let report = restore_into(&data, &directory);
    assert_eq!(report.replaced_epoch, None, "nothing was replaced");
    assert_eq!(
        (report.epoch.get(), report.generation.get()),
        (1, 1),
        "the created index gets epoch 1 and the snapshot's generation"
    );
    let index = data.join("indexes").join(INDEX);
    let store = common::open_store(&index);
    assert_eq!(store.epoch(), Epoch::new(1));
    assert_eq!(store.generation(), Generation::new(1));
    assert_eq!(
        store.snapshot().live_iids().collect::<Vec<_>>(),
        vec![Iid::new(0), Iid::new(1), Iid::new(2)],
        "the created index answers from the copied iid"
    );
}

#[test]
fn restore_refuses_a_divergent_index_json() {
    // ADR-0065 decision 3: the target must declare the same `index.json`, and
    // the refusal names the divergent setting.
    let instance = Instance::new();
    instance.snapshot();
    let directory = instance.snapshot_directory();
    let path = instance.index.join(INDEX_CONFIG_FILE_NAME);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("the target configuration must be readable: {error}"));
    let mut value: Value = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("the target configuration must be valid: {error}"));
    value["search"]["rescore_candidates"] = json!(20);
    fs::write(
        &path,
        serde_json::to_vec(&value)
            .unwrap_or_else(|error| panic!("the configuration must serialize: {error}")),
    )
    .unwrap_or_else(|error| panic!("the target configuration must be written: {error}"));

    assert_refused(&instance, &directory, "$.search.rescore_candidates");
}

#[test]
fn restore_refuses_a_snapshot_that_names_another_index_in_its_index_json() {
    // The name of `snapshot.json` and the name its `index.json` declares must
    // agree, or the restore would install an index under a directory its own
    // configuration denies.
    let instance = Instance::new();
    instance.snapshot();
    let directory = instance.snapshot_directory();
    let path = directory.join(INDEX_CONFIG_FILE_NAME);
    let mut value: Value = serde_json::from_slice(
        &fs::read(&path)
            .unwrap_or_else(|error| panic!("the configuration must be readable: {error}")),
    )
    .unwrap_or_else(|error| panic!("the configuration must be valid: {error}"));
    value["name"] = json!("another-index");
    fs::write(
        &path,
        serde_json::to_vec(&value)
            .unwrap_or_else(|error| panic!("the configuration must serialize: {error}")),
    )
    .unwrap_or_else(|error| panic!("the configuration must be written: {error}"));
    re_attest(&directory, INDEX_CONFIG_FILE_NAME);
    assert_refused(&instance, &directory, "another-index");
}

#[test]
fn restore_refuses_an_absent_attestation() {
    // ADR-0064 decision 1: a snapshot without `snapshot.json` is refused.
    let instance = Instance::new();
    instance.snapshot();
    let directory = instance.snapshot_directory();
    fs::remove_file(directory.join(SNAPSHOT_FILE_NAME))
        .unwrap_or_else(|error| panic!("snapshot.json must be removable: {error}"));
    assert_refused(&instance, &directory, SNAPSHOT_FILE_NAME);
}

#[test]
fn restore_refuses_an_unknown_format_version() {
    // ADR-0064 decision 1 and INV-15: a version this build does not read is
    // refused, never guessed.
    let instance = Instance::new();
    instance.snapshot();
    let directory = instance.snapshot_directory();
    let mut value = attestation(&directory);
    value["format_version"] = json!(2);
    write_attestation(&directory, &value);
    assert_refused(&instance, &directory, "format_version");
}

#[test]
fn restore_refuses_a_missing_listed_file() {
    // The task criterion 2: a snapshot missing one of its files is refused.
    let instance = Instance::new();
    instance.snapshot();
    let directory = instance.snapshot_directory();
    fs::remove_file(directory.join("segments/seg-1/segment.json"))
        .unwrap_or_else(|error| panic!("the segment manifest must be removable: {error}"));
    assert_refused(&instance, &directory, "segments/seg-1/segment.json");
}

#[test]
fn restore_refuses_a_size_mismatch() {
    // ADR-0064 decision 1: a listed file whose size differs is refused, so a
    // truncated database never reaches the index.
    let instance = Instance::new();
    instance.snapshot();
    let directory = instance.snapshot_directory();
    let path = directory.join(INDEX_CONFIG_FILE_NAME);
    let mut bytes = fs::read(&path)
        .unwrap_or_else(|error| panic!("the configuration must be readable: {error}"));
    bytes.push(b'\n');
    fs::write(&path, bytes)
        .unwrap_or_else(|error| panic!("the configuration must be written: {error}"));
    assert_refused(&instance, &directory, "bytes long");
}

#[test]
fn restore_refuses_a_blake3_mismatch() {
    // ADR-0064 decision 1: a file altered without changing its size is refused
    // by the checksum, which is the case a size check alone would miss.
    let instance = Instance::new();
    instance.snapshot();
    let directory = instance.snapshot_directory();
    let path = directory.join(INDEX_CONFIG_FILE_NAME);
    let mut bytes = fs::read(&path)
        .unwrap_or_else(|error| panic!("the configuration must be readable: {error}"));
    let last = bytes
        .last_mut()
        .unwrap_or_else(|| panic!("the configuration must not be empty"));
    *last = last.wrapping_add(1);
    fs::write(&path, bytes)
        .unwrap_or_else(|error| panic!("the configuration must be written: {error}"));
    assert_refused(&instance, &directory, "hashes to");
}

#[test]
fn restore_refuses_an_unlisted_file() {
    // ADR-0064 decision 1: an unlisted file makes the snapshot unusable, so a
    // hand-added note or a `.DS_Store` cannot be ignored silently.
    let instance = Instance::new();
    instance.snapshot();
    let directory = instance.snapshot_directory();
    fs::write(directory.join("notes.txt"), b"a note added by hand")
        .unwrap_or_else(|error| panic!("the note must be written: {error}"));
    assert_refused(&instance, &directory, "notes.txt");
}

#[test]
fn restore_refuses_a_path_outside_the_snapshot() {
    // A forged attestation must never make the restore read outside its own
    // directory, even with a file the attestation lists correctly.
    let instance = Instance::new();
    instance.snapshot();
    let directory = instance.snapshot_directory();
    let mut value = attestation(&directory);
    value["files"]
        .as_array_mut()
        .unwrap_or_else(|| panic!("the attestation lists files"))
        .push(json!({ "path": "../outside", "size": 0, "blake3": "blake3:00" }));
    write_attestation(&directory, &value);
    assert_refused(&instance, &directory, "../outside");
}

#[test]
fn a_second_snapshot_of_the_same_epoch_is_refused() {
    // ADR-0064 decision 3: an occupied destination is refused, and the first
    // snapshot stays exactly as it was.
    let instance = Instance::new();
    let first = instance.snapshot();
    let attestation_before = attestation(&first.directory);

    let mut store = common::open_store(&instance.index);
    let config = crible_core::config::IndexConfig::from_json_str(&common::test_config(
        INDEX,
        &common::fixture_schema(),
    ))
    .unwrap_or_else(|error| panic!("the fixture configuration must be valid: {error}"));
    let error = snapshot::create(&mut store, &config, &instance.exchange)
        .err()
        .unwrap_or_else(|| panic!("a second snapshot of the same epoch must be refused"));
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(
        error
            .message()
            .contains(&first.directory.display().to_string()),
        "the refusal names the occupied directory: {}",
        error.message()
    );
    assert_eq!(
        attestation(&first.directory),
        attestation_before,
        "the refused snapshot must not touch the copy already in place"
    );
}

#[test]
fn restore_removes_a_stale_staging_and_target_epoch() {
    // ADR-0065 decision 5: a staging directory or an epoch N a residue occupies
    // is removed before the write, as the compaction does.
    let instance = Instance::new();
    instance.snapshot();
    let directory = instance.snapshot_directory();
    let epochs = instance.index.join("epochs");
    for leftover in ["tmp-2", "2"] {
        let path = epochs.join(leftover);
        fs::create_dir_all(&path)
            .unwrap_or_else(|error| panic!("the residue must be creatable: {error}"));
        fs::write(path.join("junk"), b"a residue of an interrupted operation")
            .unwrap_or_else(|error| panic!("the residue must be written: {error}"));
    }

    let report = instance
        .restore(&directory)
        .unwrap_or_else(|error| panic!("the restore must succeed over a residue: {error}"));
    assert_eq!(report.epoch, Epoch::new(2));
    assert!(
        !epochs.join("tmp-2").exists(),
        "the staging directory of the restore is gone"
    );
    assert!(
        !epochs.join("2").join("junk").exists(),
        "the residue epoch was replaced by the restored one"
    );
    let store = common::open_store(&instance.index);
    assert_eq!(store.generation(), Generation::new(1));
}

#[test]
fn a_failure_during_the_write_leaves_the_current_epoch_intact() {
    // ADR-0065 decision 5 and INV-04: a failure before `CURRENT_EPOCH` leaves the
    // target on its current epoch, whole and usable. The failure is forced by a
    // snapshot whose attested database is not a database: the attestation is
    // consistent, so validation passes, and the restore dies while writing.
    let instance = Instance::new();
    instance.snapshot();
    instance.advance();
    let directory = instance.snapshot_directory();
    let path = directory.join("records.sqlite");
    fs::write(&path, b"not a SQLite database")
        .unwrap_or_else(|error| panic!("the database must be writable: {error}"));
    re_attest(&directory, "records.sqlite");

    let pointer = instance.pointer();
    let error = instance
        .restore(&directory)
        .err()
        .unwrap_or_else(|| panic!("a restored database that cannot open must fail"));
    assert!(
        !error.message().is_empty(),
        "the failure carries a message for the operator"
    );
    assert_eq!(
        instance.pointer(),
        pointer,
        "CURRENT_EPOCH still names the epoch the target was on"
    );
    assert!(
        !instance.index.join("epochs").join("2").exists(),
        "no half-written epoch is reachable under its final name"
    );
    let mut names: Vec<String> = fs::read_dir(instance.index.join("epochs"))
        .unwrap_or_else(|error| panic!("the epochs must be listable: {error}"))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("an epoch entry must be readable: {error}"))
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec!["1".to_owned()],
        "the failed restore removed its staging directory and left the current epoch alone"
    );
    let store = common::open_store(&instance.index);
    assert_eq!(
        store.generation(),
        Generation::new(2),
        "the target still publishes the state it had before the failed restore"
    );
}

#[test]
fn an_index_without_current_epoch_is_refused_at_open() {
    // ADR-0065 decision 5: a directory a restore (or a creation) left without
    // `CURRENT_EPOCH` is refused by the opening, and the restore refuses it too
    // instead of overwriting it.
    let instance = Instance::new();
    instance.snapshot();
    let directory = instance.snapshot_directory();

    let fresh = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("a temporary data root must be creatable: {error}"));
    let data = fresh.path().join("data");
    let index = data.join("indexes").join(INDEX);
    fs::create_dir_all(index.join("epochs").join("1"))
        .unwrap_or_else(|error| panic!("the partial tree must be creatable: {error}"));
    fs::copy(
        directory.join(INDEX_CONFIG_FILE_NAME),
        index.join(INDEX_CONFIG_FILE_NAME),
    )
    .unwrap_or_else(|error| panic!("the configuration must be copied: {error}"));

    let error = GenerationStore::open(&index, &common::fixture_schema())
        .err()
        .unwrap_or_else(|| panic!("an index without CURRENT_EPOCH must not open"));
    assert_eq!(
        error.code(),
        ErrorCode::InvalidRequest,
        "a partial tree is an invalid request, not a corrupt state: {}",
        error.message()
    );

    let error = snapshot::restore(&data, &directory)
        .err()
        .unwrap_or_else(|| panic!("a restore must not overwrite a partial tree"));
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(
        !index.join("CURRENT_EPOCH").exists(),
        "the refused restore writes no pointer"
    );
}

#[test]
fn the_attestation_is_deterministic() {
    // ADR-0064 decision 1: no timestamp, a list sorted by path, so two snapshots
    // of one epoch are byte-identical and two copies are comparable.
    let instance = Instance::new();
    let first = instance.snapshot();
    let second_exchange = instance.temp.path().join("exchange-2");
    let mut store = common::open_store(&instance.index);
    let config = crible_core::config::IndexConfig::from_json_str(&common::test_config(
        INDEX,
        &common::fixture_schema(),
    ))
    .unwrap_or_else(|error| panic!("the fixture configuration must be valid: {error}"));
    let second = snapshot::create(&mut store, &config, &second_exchange)
        .unwrap_or_else(|error| panic!("the second snapshot must be taken: {error}"));
    assert_eq!(
        fs::read(first.directory.join(SNAPSHOT_FILE_NAME))
            .unwrap_or_else(|error| panic!("the attestation must be readable: {error}")),
        fs::read(second.directory.join(SNAPSHOT_FILE_NAME))
            .unwrap_or_else(|error| panic!("the attestation must be readable: {error}")),
        "two snapshots of one epoch produce the same attestation"
    );

    let value = attestation(&first.directory);
    let paths: Vec<&str> = value["files"]
        .as_array()
        .unwrap_or_else(|| panic!("the attestation lists files"))
        .iter()
        .filter_map(|file| file["path"].as_str())
        .collect();
    let mut sorted = paths.clone();
    sorted.sort_unstable();
    assert_eq!(paths, sorted, "the files are sorted by path");
    assert!(
        !paths.iter().any(|path| path.ends_with("codes.bin")),
        "the derived codes are excluded: {paths:?}"
    );
    assert!(
        !paths.iter().any(|path| path == &"CURRENT_EPOCH"),
        "the pointer of the source index is excluded: {paths:?}"
    );
    assert!(
        paths.contains(&"manifests/1.json"),
        "the manifest of the copied generation is in the snapshot: {paths:?}"
    );
    assert!(
        !paths.contains(&"manifests/2.json"),
        "the manifests of later generations are not: {paths:?}"
    );
}
