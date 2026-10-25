// SPDX-License-Identifier: Apache-2.0

//! Snapshot of one epoch, and offline restore of a snapshot into an index.
//!
//! References: `docs/04-donnees-et-ingestion.md` ("Export, instantanés,
//! restauration", "Règles de l'instantané", "Règles de la restauration"),
//! `docs/02-architecture.md` ("Restauration d'un instantané", "Reprise au
//! démarrage"), `docs/05-api.md` (error codes), `ADR-0064` (`snapshot.json`, its
//! content and its refusals, the copied files and the exclusions, the occupied
//! destination and the staging directory) and `ADR-0065` (offline snapshot,
//! epoch E+1 with the generation left at G, target index created or checked, the
//! replaced epoch kept on disk, the compaction write protocol, the command
//! shapes). Invariants: INV-14 (nothing that cost GPU is destroyed without a
//! valid copy) and INV-15 (every on-disk format is explicit and versioned).
//!
//! # What a snapshot is
//!
//! A directory `exchange/snapshots/<index>-e<E>-g<G>/` holding a coherent copy of
//! the epoch E at generation G:
//!
//! ```text
//! index.json                         configuration of the index
//! records.sqlite                     VACUUM INTO of the reference database
//! manifests/<G>.json                 the manifest of the copied generation
//! segments/<id>/segment.json         for every segment of the generation
//! segments/<id>/vectors.f16|f32      its reference vectors
//! lexical/…                          the tantivy index, when it exists (P5)
//! snapshot.json                      the attestation of the copy
//! ```
//!
//! `codes.bin`, `cache/` and every other derived file stay out, and so does
//! `CURRENT_EPOCH`, which the restore writes itself (ADR-0064, decision 4).
//! `snapshot.json` lists every copied file — path, size, blake3 — and is written
//! last: its presence is the signal that the copy ran to its end (ADR-0064,
//! decision 1).
//!
//! # Offline, and what "the writer is suspended" means
//!
//! [`create`] takes `&mut GenerationStore`, exactly as the compaction does
//! (`crate::compact`): no commit can run while the copy holds that borrow, and
//! the readers keep their published snapshots (ADR-0017, ADR-0065 decision 1).
//! A process cannot suspend the writer of another process, so the CLI form of the
//! command is **offline**: the operator stops the server, as `docs/04` states.
//! The future HTTP route `POST /v1/indexes/{index}/snapshots` (`docs/05`) reuses
//! this function inside the server, where the borrow is the real suspension.
//!
//! # Restore
//!
//! [`restore`] is offline too, and does nothing before it has checked the whole
//! snapshot and the target (ADR-0065, decisions 2, 3 and 6):
//!
//! 1. the attestation: `snapshot.json` present, `format_version` known, every
//!    listed file present with its size and its blake3, no unlisted file;
//! 2. the target: the index name comes from `snapshot.json`; an absent index is
//!    created from the snapshot's `index.json`; an existing one must declare the
//!    same `index.json`, and a target at a **higher** generation is accepted (a
//!    deliberate rollback);
//! 3. the epoch: the copy is written into `epochs/tmp-<N>/`, `meta.epoch` and
//!    `manifests/<G>.json` receive `N`, then the directory is fsynced and renamed
//!    to `epochs/<N>/`, as the compaction does;
//! 4. the switch: `CURRENT_EPOCH ← N` is written last, so a failure before it
//!    leaves the target intact (INV-03, INV-04).
//!
//! The epoch is `E+1` for an existing target and the initial epoch of a creation
//! for an absent one. The **generation stays G**: a restore is not a commit and
//! does not increment it (ADR-0065, decision 2). The replaced epoch is never
//! deleted here, even though `crate::compact::reclaim_epochs` would delete it:
//! once a write has followed the snapshot, that epoch holds passages no valid
//! copy carries, so destroying it would break INV-14 (ADR-0065, decision 4). The
//! report names it, and the operator removes it by hand.
//!
//! The iid of a copied epoch are kept, dense as they were: no renumbering, no
//! reconstruction, so the restored index answers the same queries with the same
//! scores (the acceptance criterion of the task). Only `meta.epoch`, the manifest
//! of G and `CURRENT_EPOCH` change; `segment.json` keeps naming the generation
//! and the batch that produced the segment, as for a compaction.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crible_core::config::IndexConfig;
use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::{Epoch, Generation};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::compact::{next_epoch, remove_directory};
use crate::generation::{
    CURRENT_EPOCH_FILE_NAME, EPOCHS_DIR_NAME, GenerationStore, INDEX_CONFIG_FILE_NAME,
    MANIFESTS_DIR_NAME, RECORDS_FILE_NAME, SEGMENTS_DIR_NAME, TEMPORARY_PREFIX, epoch_directory,
    epochs_directory, manifests_directory, read_current_epoch, segments_directory,
    write_current_epoch,
};
use crate::records::RecordStore;
use crate::segment::{io_error, sync_directory, vector_file_name, write_file_synced};

/// Version of the snapshot attestation (`snapshot.json`), from ADR-0064.
pub const SNAPSHOT_FORMAT_VERSION: u32 = 1;

/// Name of the attestation file at the root of a snapshot.
pub const SNAPSHOT_FILE_NAME: &str = "snapshot.json";

/// Sub-directory of the exchange root that holds the snapshots (`docs/02`).
pub(crate) const SNAPSHOTS_DIRECTORY: &str = "snapshots";

/// Sub-directory of the data root that holds the indexes (`docs/02`).
pub(crate) const INDEXES_DIRECTORY: &str = "indexes";

/// Name of the tantivy directory of an epoch (`docs/02`).
///
/// Absent in P3: tantivy arrives in P5, and the snapshot copies it only when the
/// epoch carries it (ADR-0064, decision 4).
pub(crate) const LEXICAL_DIR_NAME: &str = "lexical";

/// Epoch a restoration gives to an index it creates (`crates/crible/src/commands.rs`).
///
/// `ADR-0065`, decision 2: an absent target has no epoch to pass, so it gets the
/// initial epoch of a creation; the `E+1` rule only applies to an index that
/// already has an epoch.
pub const INITIAL_EPOCH: Epoch = Epoch::new(1);

/// Prefix of the staging directory of a snapshot, inside `snapshots/`.
const STAGING_PREFIX: &str = ".tmp-";

/// Distinguishes two snapshots of the same epoch inside one process.
static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// What one snapshot did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotReport {
    /// Name of the index, as its `index.json` declares it.
    pub index: String,
    /// Directory holding the copy: `<exchange>/snapshots/<index>-e<E>-g<G>/`.
    pub directory: PathBuf,
    /// Epoch the copy was taken from.
    pub epoch: Epoch,
    /// Generation the copy describes.
    pub generation: Generation,
    /// Documents the reference state holds.
    pub documents: u64,
    /// Live passages of the copied generation.
    pub passages: u64,
    /// Bytes of the copied files, `snapshot.json` excepted.
    pub bytes: u64,
}

/// What one restore did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreReport {
    /// Name of the restored index, taken from `snapshot.json`.
    pub index: String,
    /// Epoch the restore replaced, or `None` when the index was created.
    pub replaced_epoch: Option<Epoch>,
    /// Generation the restore replaced, or `None` when the index was created.
    pub replaced_generation: Option<Generation>,
    /// Epoch `CURRENT_EPOCH` names when the call returns.
    pub epoch: Epoch,
    /// Generation of the restored state: the snapshot's G.
    pub generation: Generation,
    /// Documents the restored reference state holds.
    pub documents: u64,
    /// Live passages of the restored generation.
    pub passages: u64,
}

/// One copied file, as `snapshot.json` attests it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SnapshotFile {
    /// Path relative to the root of the snapshot, with `/` separators.
    path: String,
    /// Size in bytes.
    size: u64,
    /// blake3 of the file, under the form `blake3:<hex>`.
    blake3: String,
}

/// The attestation a snapshot carries at its root (`snapshot.json`).
///
/// It attests a copy of files, it does not describe a generation: it never
/// replaces `manifests/<G>.json` nor `meta.generation` (ADR-0064, decision 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SnapshotAttestation {
    /// Version of this format (INV-15).
    format_version: u32,
    /// Name of the index the copy was taken from.
    index: String,
    /// Epoch copied from the source index.
    epoch: u32,
    /// Generation the copy describes.
    generation: u64,
    /// Every copied file except `snapshot.json`, sorted by path.
    files: Vec<SnapshotFile>,
}

/// Takes a snapshot of the published epoch into `<exchange>/snapshots/`.
///
/// The store is taken by exclusive reference, so no commit can run while the
/// copy is taken (ADR-0017, ADR-0065 decision 1). The caller is responsible for
/// running this offline in the CLI form.
///
/// # Errors
///
/// - [`ErrorCode::InvalidRequest`] when
///   `<exchange>/snapshots/<index>-e<E>-g<G>/` already exists: a snapshot never
///   overwrites one (ADR-0064, decision 3);
/// - [`ErrorCode::Internal`] when the disk fails, or when the index holds a state
///   no commit could have produced (a segment of the published generation without
///   its directory, a manifest of G that cannot be found).
pub fn create(
    store: &mut GenerationStore,
    index_config: &IndexConfig,
    exchange: &Path,
) -> Result<SnapshotReport, CribleError> {
    let published = store.snapshot();
    let epoch = published.epoch();
    let generation = published.generation();
    let name = index_config.name.as_str();
    let state = store.state()?;

    let snapshots = exchange.join(SNAPSHOTS_DIRECTORY);
    fs::create_dir_all(&snapshots)
        .map_err(|error| io_error("create the snapshots directory", &snapshots, &error))?;
    let directory_name = format!("{name}-e{}-g{}", epoch.get(), generation.get());
    let destination = snapshots.join(&directory_name);
    if destination.exists() {
        return Err(invalid(format!(
            "the snapshot {} already exists; move it away or use another exchange root, because \
             a snapshot never overwrites one (ADR-0064)",
            destination.display()
        )));
    }

    // The copy is built under a staging name of its own and renamed once whole,
    // so a failure leaves no incomplete snapshot under the name an operator
    // would restore, and two simultaneous snapshots never share a directory.
    let staging = staging_path(&snapshots, &directory_name);
    fs::create_dir(&staging).map_err(|error| {
        io_error(
            "create the staging directory of the snapshot (a fresh name is drawn for every \
             snapshot)",
            &staging,
            &error,
        )
    })?;
    let guard = StagingGuard {
        path: staging.clone(),
        armed: true,
    };

    let index_dir = store.index_dir().to_path_buf();
    let epoch_dir = store.epoch_directory();
    let mut files = BTreeMap::new();

    // `VACUUM INTO` yields one coherent file, without `-wal` nor `-shm`, so the
    // checksum of the attestation covers a logical state (ADR-0064, decision 4).
    vacuum_into(
        store.records().connection(),
        &staging.join(RECORDS_FILE_NAME),
    )?;
    files.insert(
        RECORDS_FILE_NAME.to_owned(),
        attest(&staging, RECORDS_FILE_NAME)?,
    );

    copy_file(
        &index_dir.join(INDEX_CONFIG_FILE_NAME),
        &staging.join(INDEX_CONFIG_FILE_NAME),
    )?;
    files.insert(
        INDEX_CONFIG_FILE_NAME.to_owned(),
        attest(&staging, INDEX_CONFIG_FILE_NAME)?,
    );

    // "The manifest" of the task sheet is `manifests/<G>.json`, and only G is
    // copied: the earlier generations are the history of the index (ADR-0064,
    // decision 2).
    let manifest = format!("{MANIFESTS_DIR_NAME}/{}.json", generation.get());
    copy_file(
        &manifests_directory(&epoch_dir).join(format!("{}.json", generation.get())),
        &staging.join(&manifest),
    )?;
    files.insert(manifest.clone(), attest(&staging, &manifest)?);

    for meta in published.segments() {
        let relative_dir = format!("{SEGMENTS_DIR_NAME}/{}", meta.segment_id);
        for file_name in ["segment.json", vector_file_name(meta.precision)] {
            let relative = format!("{relative_dir}/{file_name}");
            copy_file(
                &segments_directory(&epoch_dir)
                    .join(&meta.segment_id)
                    .join(file_name),
                &staging.join(&relative),
            )?;
            files.insert(relative.clone(), attest(&staging, &relative)?);
        }
    }

    let lexical = epoch_dir.join(LEXICAL_DIR_NAME);
    if lexical.is_dir() {
        copy_tree(
            &lexical,
            &staging.join(LEXICAL_DIR_NAME),
            LEXICAL_DIR_NAME,
            &mut files,
        )?;
    }

    let bytes = files.values().map(|file| file.size).sum();
    let attestation = SnapshotAttestation {
        format_version: SNAPSHOT_FORMAT_VERSION,
        index: name.to_owned(),
        epoch: epoch.get(),
        generation: generation.get(),
        files: files.into_values().collect(),
    };
    let text = serde_json::to_string(&attestation).map_err(|error| {
        CribleError::new(
            ErrorCode::Internal,
            format!("cannot serialize the attestation of a snapshot: {error}"),
        )
    })?;
    write_file_synced(&staging.join(SNAPSHOT_FILE_NAME), text.as_bytes())?;

    // Every file of the copy is durable before the directory entry names it, as
    // the export does: a snapshot directory exists only complete.
    sync_tree(&staging)?;
    drop(published);
    fs::rename(&staging, &destination)
        .map_err(|error| io_error("rename the snapshot into place", &staging, &error))?;
    guard.disarm();
    sync_directory(&snapshots)?;

    Ok(SnapshotReport {
        index: name.to_owned(),
        directory: destination,
        epoch,
        generation,
        documents: state.documents,
        passages: state.live_passages,
        bytes,
    })
}

/// Restores the snapshot at `snapshot_directory` into the index of `data`.
///
/// The index name is the one `snapshot.json` declares, so the command has no
/// `--index`: the copy names its own target (ADR-0065, decision 3). Nothing is
/// written before the snapshot and the target have been checked.
///
/// # Errors
///
/// - [`ErrorCode::InvalidRequest`] when the snapshot is absent, incomplete or
///   altered in any of the five ways ADR-0064 decision 1 lists, when an index
///   directory without `CURRENT_EPOCH` is in the way, or when the target's
///   `index.json` differs from the snapshot's;
/// - [`ErrorCode::Internal`] when the disk fails or the restored database cannot
///   be opened.
pub fn restore(data: &Path, snapshot_directory: &Path) -> Result<RestoreReport, CribleError> {
    let attestation = load_attestation(snapshot_directory)?;
    let index_dir = data.join(INDEXES_DIRECTORY).join(&attestation.index);
    let index_json = snapshot_directory.join(INDEX_CONFIG_FILE_NAME);
    let declared = fs::read(&index_json)
        .map_err(|error| invalid(format!("cannot read {}: {error}", index_json.display())))?;
    let declared_text = std::str::from_utf8(&declared).map_err(|_| {
        invalid(format!(
            "{} is not valid UTF-8, so no index can be read from it",
            index_json.display()
        ))
    })?;
    let index_config = IndexConfig::from_json_str(declared_text)?;
    if index_config.name != attestation.index {
        return Err(invalid(format!(
            "the snapshot names the index {} but its index.json declares {}; the copy cannot be \
             installed under a name its configuration does not carry",
            attestation.index, index_config.name
        )));
    }

    let pointer = index_dir.join(CURRENT_EPOCH_FILE_NAME);
    let complete = pointer.exists();
    let partial =
        index_dir.join(EPOCHS_DIR_NAME).exists() || index_dir.join(RECORDS_FILE_NAME).exists();
    if !complete && partial {
        // A creation interrupted before `CURRENT_EPOCH` leaves a partial tree
        // that no opening accepts (ADR-0065, decision 5); a restore neither
        // repairs nor overwrites it.
        return Err(invalid(format!(
            "a partial index tree is present at {}: it has no {} pointer, so no opening accepts \
             it; remove it before restoring (ADR-0065)",
            index_dir.display(),
            CURRENT_EPOCH_FILE_NAME
        )));
    }

    let (replaced_epoch, replaced_generation, new_epoch) = if complete {
        let epoch = read_current_epoch(&index_dir)?;
        check_target_configuration(
            &index_dir,
            &index_dir.join(INDEX_CONFIG_FILE_NAME),
            &declared,
        )?;
        let records =
            RecordStore::open(&epoch_directory(&index_dir, epoch).join(RECORDS_FILE_NAME))?;
        let meta = records.meta()?;
        drop(records);
        (Some(epoch), Some(meta.generation), next_epoch(epoch)?)
    } else {
        (None, None, INITIAL_EPOCH)
    };

    // From here the command writes. Everything it writes lives outside the tree
    // `CURRENT_EPOCH` makes reachable until the last step.
    fs::create_dir_all(&index_dir)
        .map_err(|error| io_error("create the index directory", &index_dir, &error))?;
    if !complete {
        write_file_synced(&index_dir.join(INDEX_CONFIG_FILE_NAME), &declared)?;
    }
    let epochs = epochs_directory(&index_dir);
    fs::create_dir_all(&epochs)
        .map_err(|error| io_error("create the epochs directory", &epochs, &error))?;

    let staging = epochs.join(format!("{TEMPORARY_PREFIX}{}", new_epoch.get()));
    let target_epoch = epoch_directory(&index_dir, new_epoch);
    // ADR-0065, decision 5: a staging directory or an epoch `N` a residue
    // occupies is removed before writing, as the compaction does.
    remove_directory(&staging)?;
    remove_directory(&target_epoch)?;
    fs::create_dir_all(&staging)
        .map_err(|error| io_error("create the staging epoch", &staging, &error))?;
    let guard = StagingGuard {
        path: staging.clone(),
        armed: true,
    };

    for file in &attestation.files {
        if file.path == INDEX_CONFIG_FILE_NAME {
            // `index.json` lives at the root of the index, not inside its epoch.
            continue;
        }
        copy_file(
            &snapshot_directory.join(&file.path),
            &staging.join(&file.path),
        )?;
    }

    // `meta.epoch` must name `CURRENT_EPOCH` (`docs/02`), and the copied database
    // names the source epoch: the restore rewrites it, and only it. Closing the
    // database checkpoints its WAL into the file, so the epoch directory holds a
    // single database again.
    {
        let mut records = RecordStore::open(&staging.join(RECORDS_FILE_NAME))?;
        let meta = records.meta()?;
        if meta.generation != Generation::new(attestation.generation) {
            return Err(invalid(format!(
                "the snapshot declares generation {} but its {} declares generation {}; the copy \
                 does not describe the generation its attestation names",
                attestation.generation,
                RECORDS_FILE_NAME,
                meta.generation.get()
            )));
        }
        records.set_epoch(new_epoch)?;
    }
    sync_file(&staging.join(RECORDS_FILE_NAME))?;

    rewrite_manifest_epoch(
        &staging.join(format!(
            "{MANIFESTS_DIR_NAME}/{}.json",
            attestation.generation
        )),
        new_epoch,
        Generation::new(attestation.generation),
    )?;

    sync_tree(&staging)?;
    fs::rename(&staging, &target_epoch)
        .map_err(|error| io_error("rename the restored epoch into place", &staging, &error))?;
    guard.disarm();
    sync_directory(&epochs)?;

    // The switch, last: a failure before this line leaves the target untouched.
    write_current_epoch(&index_dir, new_epoch)?;

    // Opening runs the recovery of the target and rebuilds the derived manifest
    // of the restored generation, then publishes it (INV-04).
    let store = GenerationStore::open(&index_dir, &index_config.schema)?;
    let state = store.state()?;
    if state.generation != Generation::new(attestation.generation) {
        return Err(CribleError::new(
            ErrorCode::Internal,
            format!(
                "the restored index publishes generation {} although its snapshot declares {}",
                state.generation.get(),
                attestation.generation
            ),
        ));
    }

    Ok(RestoreReport {
        index: attestation.index,
        replaced_epoch,
        replaced_generation,
        epoch: new_epoch,
        generation: state.generation,
        documents: state.documents,
        passages: state.live_passages,
    })
}

/// Reads and validates the attestation of a snapshot, then every file it lists.
///
/// The five refusals of ADR-0064 decision 1: no `snapshot.json`, an unknown
/// `format_version`, a missing listed file, a size or blake3 that differs, and an
/// unlisted file. Every refusal is [`ErrorCode::InvalidRequest`], raised before
/// anything is written.
fn load_attestation(directory: &Path) -> Result<SnapshotAttestation, CribleError> {
    if !directory.is_dir() {
        return Err(invalid(format!(
            "the snapshot {} does not exist or is not a directory",
            directory.display()
        )));
    }
    let path = directory.join(SNAPSHOT_FILE_NAME);
    let bytes = fs::read(&path).map_err(|error| {
        invalid(format!(
            "cannot read {}, the attestation of a snapshot: {error}",
            path.display()
        ))
    })?;
    let attestation: SnapshotAttestation = serde_json::from_slice(&bytes).map_err(|error| {
        invalid(format!(
            "{} is not a valid snapshot attestation: {error}",
            path.display()
        ))
    })?;
    if attestation.format_version != SNAPSHOT_FORMAT_VERSION {
        return Err(invalid(format!(
            "the snapshot declares format_version {} but this build reads format_version {}",
            attestation.format_version, SNAPSHOT_FORMAT_VERSION
        )));
    }
    validate_index_name(&attestation.index)?;
    let generation = Generation::new(attestation.generation);

    let mut listed: BTreeSet<&str> = BTreeSet::new();
    for file in &attestation.files {
        validate_relative_path(&file.path)?;
        if file.path == SNAPSHOT_FILE_NAME {
            return Err(invalid(format!(
                "the snapshot lists {SNAPSHOT_FILE_NAME} among its files, but the attestation \
                 cannot hash itself"
            )));
        }
        if !listed.insert(file.path.as_str()) {
            return Err(invalid(format!("the snapshot lists {} twice", file.path)));
        }
        let path = directory.join(&file.path);
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            invalid(format!(
                "the snapshot is missing {}, which its attestation lists: {error}",
                file.path
            ))
        })?;
        if !metadata.is_file() {
            return Err(invalid(format!(
                "the snapshot holds {} but it is not a regular file",
                file.path
            )));
        }
        if metadata.len() != file.size {
            return Err(invalid(format!(
                "the snapshot's {} is {} bytes long but its attestation declares {}",
                file.path,
                metadata.len(),
                file.size
            )));
        }
        let digest = hash_snapshot_file(&path, &file.path)?;
        if digest != file.blake3 {
            return Err(invalid(format!(
                "the snapshot's {} hashes to {digest} but its attestation declares {}",
                file.path, file.blake3
            )));
        }
    }

    for required in [
        INDEX_CONFIG_FILE_NAME.to_owned(),
        RECORDS_FILE_NAME.to_owned(),
        format!("{MANIFESTS_DIR_NAME}/{}.json", generation.get()),
    ] {
        if !listed.contains(required.as_str()) {
            return Err(invalid(format!(
                "the snapshot does not list {required}, which every snapshot of an epoch carries"
            )));
        }
    }

    let mut unlisted = Vec::new();
    collect_unlisted(directory, "", &mut unlisted)?;
    for path in unlisted {
        if path == SNAPSHOT_FILE_NAME || listed.contains(path.as_str()) {
            continue;
        }
        return Err(invalid(format!(
            "the snapshot holds {path}, which its attestation does not list; a snapshot refuses \
             any file it cannot account for (ADR-0064)"
        )));
    }

    Ok(attestation)
}

/// Refuses a target whose `index.json` differs from the snapshot's.
///
/// The epoch would otherwise be interpreted under another schema or other
/// settings than the ones it was written with, and the queries would change
/// behaviour silently (ADR-0065, decision 3). The message names the divergent
/// setting, never a document value.
fn check_target_configuration(
    index_dir: &Path,
    target_path: &Path,
    snapshot_bytes: &[u8],
) -> Result<(), CribleError> {
    let target = fs::read(target_path)
        .map_err(|error| invalid(format!("cannot read {}: {error}", target_path.display())))?;
    let target_text = std::str::from_utf8(&target).map_err(|_| {
        invalid(format!(
            "{} is not valid UTF-8, so its configuration cannot be compared",
            target_path.display()
        ))
    })?;
    // Parsing validates the configuration, including every unknown key.
    IndexConfig::from_json_str(target_text)?;
    let snapshot_text = std::str::from_utf8(snapshot_bytes).map_err(|_| {
        invalid(
            "the snapshot's index.json is not valid UTF-8, so its configuration cannot be compared"
                .to_owned(),
        )
    })?;
    let target_value: Value = serde_json::from_str(target_text).map_err(|error| {
        invalid(format!(
            "{} is not valid JSON: {error}",
            target_path.display()
        ))
    })?;
    let snapshot_value: Value = serde_json::from_str(snapshot_text).map_err(|error| {
        invalid(format!(
            "the snapshot's index.json is not valid JSON: {error}"
        ))
    })?;
    let Some(difference) = first_difference(&snapshot_value, &target_value, "$") else {
        return Ok(());
    };
    Err(invalid(format!(
        "the index at {} declares a different index.json: {difference} differs from the snapshot; \
         restore the index to its original configuration, or restore under another index name \
         (ADR-0065)",
        index_dir.display()
    )))
}

/// Path of the first value that differs between two canonical configurations,
/// or `None` when they are equal.
fn first_difference(left: &Value, right: &Value, path: &str) -> Option<String> {
    match (left, right) {
        (Value::Object(left), Value::Object(right)) => {
            let mut keys: BTreeSet<&str> = left.keys().map(String::as_str).collect();
            keys.extend(right.keys().map(String::as_str));
            for key in keys {
                let child = format!("{path}.{key}");
                match (left.get(key), right.get(key)) {
                    (Some(_), None) | (None, Some(_)) => return Some(child),
                    (Some(left), Some(right)) => {
                        if let Some(found) = first_difference(left, right, &child) {
                            return Some(found);
                        }
                    }
                    (None, None) => {}
                }
            }
            None
        }
        (Value::Array(left), Value::Array(right)) => {
            for (index, (left, right)) in left.iter().zip(right).enumerate() {
                if let Some(found) = first_difference(left, right, &format!("{path}[{index}]")) {
                    return Some(found);
                }
            }
            if left.len() != right.len() {
                return Some(format!("{path}[]"));
            }
            None
        }
        (left, right) => (left != right).then(|| path.to_owned()),
    }
}

/// Rewrites the epoch of the manifest of the restored generation.
///
/// The copied `manifests/<G>.json` names the source epoch; installed as `N`, it
/// must name `N` (ADR-0065, decision 2). The JSON stays canonical: keys sorted,
/// compact, and the serialization of a parsed value keeps both (ADR-0027).
fn rewrite_manifest_epoch(
    path: &Path,
    epoch: Epoch,
    generation: Generation,
) -> Result<(), CribleError> {
    let bytes =
        fs::read(path).map_err(|error| io_error("read the copied manifest", path, &error))?;
    let mut manifest: Value = serde_json::from_slice(&bytes).map_err(|error| {
        CribleError::new(
            ErrorCode::Internal,
            format!(
                "the copied manifest {} is not valid JSON: {error}",
                path.display()
            ),
        )
    })?;
    let declared = manifest.get("generation").and_then(Value::as_u64);
    if declared != Some(generation.get()) {
        return Err(invalid(format!(
            "the snapshot's manifest names generation {declared:?} but its attestation declares \
             generation {}",
            generation.get()
        )));
    }
    let object = manifest.as_object_mut().ok_or_else(|| {
        CribleError::new(
            ErrorCode::Internal,
            format!(
                "the copied manifest {} is not a JSON object",
                path.display()
            ),
        )
    })?;
    object.insert("epoch".to_owned(), Value::from(epoch.get()));
    let text = serde_json::to_string(&manifest).map_err(|error| {
        CribleError::new(
            ErrorCode::Internal,
            format!("cannot serialize the restored manifest: {error}"),
        )
    })?;
    write_file_synced(path, text.as_bytes())
}

/// Refuses a path that would escape the snapshot directory.
fn validate_relative_path(value: &str) -> Result<(), CribleError> {
    let path = Path::new(value);
    let valid = !value.is_empty()
        && !path.is_absolute()
        && !value.contains('\\')
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    if valid {
        Ok(())
    } else {
        Err(invalid(format!(
            "the snapshot lists the path {value:?}, which is not a relative path inside the \
             snapshot"
        )))
    }
}

/// Refuses an index name that would escape `<data>/indexes/`.
fn validate_index_name(value: &str) -> Result<(), CribleError> {
    validate_relative_path(value)?;
    if value == "." || value == ".." || value.contains('/') || value.contains('\\') {
        return Err(invalid(format!(
            "the snapshot names the index {value:?}, which is not a single path component"
        )));
    }
    Ok(())
}

/// Collects every file of the snapshot tree, relative to its root.
fn collect_unlisted(
    directory: &Path,
    prefix: &str,
    found: &mut Vec<String>,
) -> Result<(), CribleError> {
    let entries = fs::read_dir(directory)
        .map_err(|error| io_error("list the files of the snapshot", directory, &error))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| io_error("read an entry of the snapshot", directory, &error))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(invalid(format!(
                "the snapshot holds a file name that is not UTF-8 under {}",
                directory.display()
            )));
        };
        let relative = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| io_error("read a file of the snapshot", &path, &error))?;
        if metadata.is_dir() {
            collect_unlisted(&path, &relative, found)?;
        } else {
            found.push(relative);
        }
    }
    Ok(())
}

/// Copies a file, creating its parent directories, and fsyncs it.
fn copy_file(source: &Path, destination: &Path) -> Result<u64, CribleError> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| io_error("create a directory of the copy", parent, &error))?;
    }
    let mut input =
        File::open(source).map_err(|error| io_error("open a file to copy", source, &error))?;
    let mut output = File::create(destination)
        .map_err(|error| io_error("create a copied file", destination, &error))?;
    let bytes = io::copy(&mut input, &mut output)
        .map_err(|error| io_error("copy a file", destination, &error))?;
    output
        .sync_all()
        .map_err(|error| io_error("fsync a copied file", destination, &error))?;
    Ok(bytes)
}

/// Copies a directory tree of files, attesting each one.
fn copy_tree(
    source: &Path,
    destination: &Path,
    relative_root: &str,
    files: &mut BTreeMap<String, SnapshotFile>,
) -> Result<(), CribleError> {
    let entries = fs::read_dir(source)
        .map_err(|error| io_error("list a directory to copy", source, &error))?;
    for entry in entries {
        let entry = entry.map_err(|error| io_error("read an entry to copy", source, &error))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(CribleError::new(
                ErrorCode::Internal,
                format!(
                    "the epoch holds a file name that is not UTF-8 under {}",
                    source.display()
                ),
            ));
        };
        let relative = format!("{relative_root}/{name}");
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| io_error("read a file to copy", &path, &error))?;
        if metadata.is_dir() {
            let copied = destination.join(name);
            fs::create_dir_all(&copied)
                .map_err(|error| io_error("create a directory of the copy", &copied, &error))?;
            copy_tree(&path, &copied, &relative, files)?;
        } else if metadata.is_file() {
            let copied = destination.join(name);
            copy_file(&path, &copied)?;
            files.insert(relative.clone(), attest_file(&copied, &relative)?);
        } else {
            return Err(CribleError::new(
                ErrorCode::Internal,
                format!(
                    "the epoch holds {}, which is neither a file nor a directory",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

/// Attests a file of the staging directory, relative to `root`.
fn attest(root: &Path, relative: &str) -> Result<SnapshotFile, CribleError> {
    attest_file(&root.join(relative), relative)
}

/// Attests the copied file at `path` under its path relative to the snapshot.
fn attest_file(path: &Path, relative: &str) -> Result<SnapshotFile, CribleError> {
    let size = fs::metadata(path)
        .map_err(|error| io_error("measure a copied file", path, &error))?
        .len();
    let digest = hash_copied_file(path, relative)?;
    Ok(SnapshotFile {
        path: relative.to_owned(),
        size,
        blake3: digest,
    })
}

/// blake3 of one file the snapshot writes, under the form `blake3:<hex>`.
///
/// A read failure here is a disk failure of the source index, so it is
/// [`ErrorCode::Internal`] (`docs/05-api.md`, ADR-0065 decision 6).
fn hash_copied_file(path: &Path, relative: &str) -> Result<String, CribleError> {
    hash(path, |error| {
        CribleError::new(
            ErrorCode::Internal,
            format!("cannot read the copied file {relative}: {error}"),
        )
    })
}

/// blake3 of one file of a snapshot, under the form `blake3:<hex>`.
///
/// A read failure here means the snapshot itself cannot be read, which the
/// restore reports as [`ErrorCode::InvalidRequest`] (ADR-0064, decision 1).
fn hash_snapshot_file(path: &Path, relative: &str) -> Result<String, CribleError> {
    hash(path, |error| {
        invalid(format!("cannot read {relative}: {error}"))
    })
}

/// Streams a file through blake3, mapping a read failure with `failure`.
fn hash(
    path: &Path,
    failure: impl Fn(std::io::Error) -> CribleError,
) -> Result<String, CribleError> {
    let mut file = File::open(path).map_err(&failure)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(&failure)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("blake3:{}", hasher.finalize().to_hex()))
}

/// Copies the reference database into `target` with `VACUUM INTO`.
fn vacuum_into(connection: &rusqlite::Connection, target: &Path) -> Result<(), CribleError> {
    let path = target.to_str().ok_or_else(|| {
        CribleError::new(
            ErrorCode::Internal,
            format!(
                "{} is not a UTF-8 path, so SQLite cannot copy the reference database into it",
                target.display()
            ),
        )
    })?;
    connection
        .execute("VACUUM INTO ?1", params![path])
        .map_err(|error| {
            CribleError::new(
                ErrorCode::Internal,
                format!(
                    "cannot copy the reference database into {}: {error}",
                    target.display()
                ),
            )
        })?;
    sync_file(target)
}

/// fsyncs one file that another API wrote.
fn sync_file(path: &Path) -> Result<(), CribleError> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| io_error("fsync a file of the epoch", path, &error))
}

/// fsyncs every directory of a tree, bottom-up, then the tree itself.
fn sync_tree(root: &Path) -> Result<(), CribleError> {
    let entries =
        fs::read_dir(root).map_err(|error| io_error("list a directory to fsync", root, &error))?;
    for entry in entries {
        let entry = entry.map_err(|error| io_error("read an entry to fsync", root, &error))?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| io_error("read a file to fsync", &entry.path(), &error))?;
        if metadata.is_dir() {
            sync_tree(&entry.path())?;
        }
    }
    sync_directory(root)
}

/// Draws a staging directory name no other snapshot of the same epoch can hold.
fn staging_path(snapshots: &Path, directory_name: &str) -> PathBuf {
    let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    snapshots.join(format!(
        "{STAGING_PREFIX}{directory_name}-{}-{sequence}-{nanos}",
        std::process::id()
    ))
}

/// Removes the staging directory of a snapshot that did not reach its rename.
struct StagingGuard {
    path: PathBuf,
    armed: bool,
}

impl StagingGuard {
    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

/// Builds an [`ErrorCode::InvalidRequest`] error, the code of a snapshot Crible
/// cannot use (`docs/05-api.md`).
fn invalid(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::InvalidRequest, message)
}

#[cfg(test)]
mod tests {
    use super::{first_difference, validate_index_name, validate_relative_path};
    use serde_json::json;

    #[test]
    fn a_relative_path_inside_the_snapshot_is_accepted() {
        for path in [
            "index.json",
            "records.sqlite",
            "segments/seg-1/vectors.f16",
            "manifests/412.json",
        ] {
            assert!(
                validate_relative_path(path).is_ok(),
                "{path} is a path of a snapshot"
            );
        }
    }

    #[test]
    fn a_path_that_escapes_the_snapshot_is_refused() {
        for path in [
            "",
            "/etc/passwd",
            "../outside",
            "segments/../../outside",
            "./index.json",
            r"segments\\seg-1",
        ] {
            assert!(
                validate_relative_path(path).is_err(),
                "{path:?} must not be read as a file of the snapshot"
            );
        }
    }

    #[test]
    fn an_index_name_is_one_path_component() {
        assert!(validate_index_name("legifacile").is_ok());
        for name in ["", ".", "..", "a/b", "a\\b", "/abs"] {
            assert!(
                validate_index_name(name).is_err(),
                "{name:?} must not name an index outside <data>/indexes/"
            );
        }
    }

    #[test]
    fn the_first_difference_names_the_divergent_setting() {
        let left = json!({"search": {"rescore_candidates": 10}, "name": "i"});
        let right = json!({"search": {"rescore_candidates": 20}, "name": "i"});
        assert_eq!(
            first_difference(&left, &right, "$"),
            Some("$.search.rescore_candidates".to_owned())
        );
        assert_eq!(first_difference(&left, &left, "$"), None);
        assert_eq!(
            first_difference(&left, &json!({"name": "i"}), "$"),
            Some("$.search".to_owned())
        );
    }
}
