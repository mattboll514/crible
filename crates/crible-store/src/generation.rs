// SPDX-License-Identifier: Apache-2.0

//! Generations of one index: the commit protocol and the recovery at startup.
//!
//! References: `docs/02-architecture.md` ("Organisation sur disque",
//! "Générations et point de commit", "Concurrence"), `ADR-0016` (the SQLite
//! commit is the only commit point), `docs/08-invariants-et-tests.md`
//! (INV-03, INV-04).
//!
//! # Layout
//!
//! ```text
//! <index>/CURRENT_EPOCH                             current epoch, written tmp then renamed
//! <index>/epochs/<epoch>/records.sqlite             reference state (WAL)
//! <index>/epochs/<epoch>/segments/<segment_id>/     immutable vectors, see `segment`
//! <index>/epochs/<epoch>/manifests/<generation>.json derived view of one generation
//! ```
//!
//! # Commit protocol
//!
//! [`GenerationStore::commit`] applies one generation change in the order of
//! `docs/02-architecture.md`:
//!
//! 1. one SQLite transaction writes the segments, the documents, the passages,
//!    the tombstones, `meta.generation = G+1` and `meta.next_iid`;
//! 2. **`COMMIT` — the commit point (ADR-0016)**;
//! 3. `manifests/<G+1>.json` is written, fsynced and renamed;
//! 4. a new [`Snapshot`] is built and published through an [`ArcSwap`].
//!
//! The segments themselves are written before the call, by the attach that
//! prepares them: a segment stays invisible until step 1 references it, which
//! is what makes an abandoned one an orphan that recovery removes.
//!
//! # Recovery
//!
//! [`GenerationStore::open`] restores the last committed generation and nothing
//! else (INV-04): it reads `CURRENT_EPOCH`, opens SQLite, reads `meta`, removes
//! every `tmp-*` entry and every segment directory absent from the `segments`
//! table, rewrites the manifest of the current generation when it is missing,
//! then publishes the snapshot.
//!
//! # Fault injection
//!
//! With the `failpoints` cargo feature, and only with it, `commit` stops at the
//! step named by the `CRIBLE_FAILPOINT` environment variable
//! (`before_sqlite_commit`, `after_sqlite_commit`, `after_manifest`,
//! `after_publish`). Stopping means panicking with a message containing
//! `CRIBLE_FAILPOINT`: unwinding through the transaction rolls it back exactly
//! as a process death would. That panic is the expected outcome of the
//! `INV-03` / `INV-04` tests; nothing else in this module panics.
//!
//! # What this module does not do yet
//!
//! The snapshot carries the metadata of its segments, not their vectors: it
//! opens no segment, so it never pays the checksum read of
//! `SegmentReader::open`. Loading the vectors and the bitmaps into the snapshot
//! belongs to `CONSTRUIRE-BITMAPS` and to the dense search tasks.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arc_swap::ArcSwap;
use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::{Epoch, Generation, Iid};
use serde_json::{Value, json};

use crate::records::{self, DocumentRecord, PassageRecord, RecordStore};
use crate::segment::{self, SegmentMeta, io_error, sync_directory, write_file_synced};

/// Version of the derived manifest format (INV-15).
pub const MANIFEST_FORMAT_VERSION: u32 = 1;

/// Name of the file holding the current epoch of an index.
pub const CURRENT_EPOCH_FILE_NAME: &str = "CURRENT_EPOCH";

/// Name of the directory holding the epochs of an index.
const EPOCHS_DIR_NAME: &str = "epochs";

/// Name of the reference database inside an epoch directory.
const RECORDS_FILE_NAME: &str = "records.sqlite";

/// Name of the segments directory inside an epoch directory.
const SEGMENTS_DIR_NAME: &str = "segments";

/// Name of the derived manifests directory inside an epoch directory.
const MANIFESTS_DIR_NAME: &str = "manifests";

/// Prefix of a directory or file built before its rename into place.
///
/// The same prefix as the one [`SegmentWriter`](crate::segment::SegmentWriter)
/// uses: recovery removes every `tmp-*` entry of `segments/` and `manifests/`.
const TEMPORARY_PREFIX: &str = "tmp-";

/// Environment variable naming the step at which `commit` must stop.
#[cfg(feature = "failpoints")]
const FAILPOINT_ENV: &str = "CRIBLE_FAILPOINT";

/// One generation change, applied in a single transaction and published whole.
#[derive(Debug, Clone, PartialEq)]
pub struct GenerationChange {
    /// Segments the new generation makes visible, already written and renamed.
    pub segments: Vec<SegmentMeta>,
    /// Document rows the change writes.
    pub documents: Vec<DocumentRecord>,
    /// Passage rows the change adds.
    pub passages: Vec<PassageRecord>,
    /// Internal identifiers tombstoned at the new generation.
    pub deletions: Vec<Iid>,
}

/// Immutable view of one committed generation, held by a reader for the whole
/// request (INV-02, INV-03).
///
/// A snapshot is never mutated: a commit builds a new one and publishes it, so
/// a reader that took a snapshot keeps the generation it started on, even while
/// another thread commits.
#[derive(Debug)]
pub struct Snapshot {
    generation: Generation,
    segments: Vec<SegmentMeta>,
    live_iids: Vec<Iid>,
}

impl Snapshot {
    /// Generation this snapshot describes.
    #[must_use]
    pub fn generation(&self) -> Generation {
        self.generation
    }

    /// Segments the generation references, in ascending `iid_start` order, then
    /// by identifier.
    #[must_use]
    pub fn segments(&self) -> &[SegmentMeta] {
        &self.segments
    }

    /// Internal identifiers alive in this generation, in ascending order:
    /// passages tombstoned at this generation and at the previous ones are
    /// excluded (INV-01 keeps them inside the engine).
    #[must_use]
    pub fn live_iids(&self) -> &[Iid] {
        &self.live_iids
    }
}

/// Stops the process at `step` when `CRIBLE_FAILPOINT=<step>` is set.
///
/// Simulating a crash means panicking: the unwinding drops the SQLite
/// transaction, which rolls it back exactly as a process death would, and the
/// store is then reopened the way a restart does. That panic is the expected
/// outcome of the `INV-03` / `INV-04` tests and cannot be reached in a build
/// without the `failpoints` feature.
#[cfg(feature = "failpoints")]
fn stop_at_failpoint(step: &str) {
    let armed = std::env::var(FAILPOINT_ENV).unwrap_or_default();
    if armed == step {
        panic!("CRIBLE_FAILPOINT={step}: simulated crash at this step of the commit protocol");
    }
}

/// Reaches the step `step` of the commit protocol.
///
/// Expands to nothing at all without the `failpoints` feature: the normal build
/// carries neither the environment read nor a call.
#[cfg(feature = "failpoints")]
macro_rules! failpoint {
    ($step:literal) => {
        stop_at_failpoint($step)
    };
}

/// Reaches the step `step` of the commit protocol, which does nothing in a
/// normal build.
#[cfg(not(feature = "failpoints"))]
macro_rules! failpoint {
    ($step:literal) => {};
}

/// Owner of one index: the writable reference database of the epoch and the
/// published snapshot of the current generation.
///
/// One writer per index (`docs/02-architecture.md`, "Concurrence"): the type
/// holds the only writable connection of `records.sqlite`, so no second writer
/// can bypass the commit point of ADR-0016.
pub struct GenerationStore {
    index_dir: PathBuf,
    epoch: Epoch,
    records: RecordStore,
    snapshot: ArcSwap<Snapshot>,
}

impl GenerationStore {
    /// Creates the index at `index_dir`: the epoch directory, its `segments/`
    /// and `manifests/` directories, `records.sqlite` with its schema, the
    /// manifest of generation 0, then `CURRENT_EPOCH`.
    ///
    /// `CURRENT_EPOCH` is written last and atomically, and every directory it
    /// makes reachable is fsynced before it: `segments/`, the epoch directory
    /// and `epochs/`, in that order, then the index directory by
    /// `write_current_epoch`. A creation that returns has therefore persisted
    /// the whole chain, so a crash afterwards leaves a usable index; a crash
    /// before leaves no `CURRENT_EPOCH`, which [`GenerationStore::open`] refuses
    /// with [`ErrorCode::InvalidRequest`] instead of reading a half-created
    /// epoch.
    pub fn create(index_dir: &Path, epoch: Epoch) -> Result<Self, CribleError> {
        let epoch_dir = epoch_directory(index_dir, epoch);
        let segments = segments_directory(&epoch_dir);
        let manifests = manifests_directory(&epoch_dir);
        for directory in [&segments, &manifests] {
            fs::create_dir_all(directory)
                .map_err(|error| io_error("create a directory of the epoch", directory, &error))?;
        }

        let mut records = RecordStore::create(&epoch_dir.join(RECORDS_FILE_NAME))?;
        records.set_epoch(epoch)?;

        let snapshot = build_snapshot(&records, &epoch_dir)?;
        write_manifest(&epoch_dir, epoch, &snapshot)?;

        // fsync the directories bottom-up before the pointer that makes them
        // reachable: POSIX allows a rename to survive while a directory entry
        // above it does not, and `open` must never find `CURRENT_EPOCH` without
        // its epoch.
        let epochs_dir = epoch_dir.parent().ok_or_else(|| {
            CribleError::new(
                ErrorCode::Internal,
                format!("{} has no parent directory", epoch_dir.display()),
            )
        })?;
        sync_directory(&segments)?;
        sync_directory(&epoch_dir)?;
        sync_directory(epochs_dir)?;
        write_current_epoch(index_dir, epoch)?;

        Ok(Self {
            index_dir: index_dir.to_path_buf(),
            epoch,
            records,
            snapshot: ArcSwap::from(Arc::new(snapshot)),
        })
    }

    /// Opens an existing index and runs the recovery of INV-04, then publishes
    /// the snapshot of the committed generation.
    ///
    /// Recovery removes every `tmp-*` entry and every segment directory SQLite
    /// does not reference, and rewrites the manifest of the current generation
    /// when it is missing. It is idempotent: a second open changes nothing.
    ///
    /// A directory that holds no `CURRENT_EPOCH` is not an index of this build
    /// and is refused with [`ErrorCode::InvalidRequest`], before anything is
    /// removed from it.
    pub fn open(index_dir: &Path) -> Result<Self, CribleError> {
        let epoch = read_current_epoch(index_dir)?;
        let epoch_dir = epoch_directory(index_dir, epoch);
        let records = RecordStore::open(&epoch_dir.join(RECORDS_FILE_NAME))?;

        let declared = records.meta()?.epoch;
        if declared != epoch {
            return Err(CribleError::new(
                ErrorCode::Internal,
                format!(
                    "{} names epoch {} but {} declares epoch {}",
                    index_dir.join(CURRENT_EPOCH_FILE_NAME).display(),
                    epoch.get(),
                    epoch_dir.join(RECORDS_FILE_NAME).display(),
                    declared.get()
                ),
            ));
        }

        recover(&records, index_dir, &epoch_dir)?;
        let snapshot = build_snapshot(&records, &epoch_dir)?;
        ensure_manifest(&epoch_dir, epoch, &snapshot)?;

        Ok(Self {
            index_dir: index_dir.to_path_buf(),
            epoch,
            records,
            snapshot: ArcSwap::from(Arc::new(snapshot)),
        })
    }

    /// Epoch of the index, as `CURRENT_EPOCH` names it.
    #[must_use]
    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// Generation currently published.
    #[must_use]
    pub fn generation(&self) -> Generation {
        self.snapshot.load().generation()
    }

    /// Takes the snapshot of the published generation.
    ///
    /// The caller holds the returned `Arc` for the whole request, so a commit
    /// that happens meanwhile cannot change the view it reads (INV-02, INV-03).
    #[must_use]
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshot.load_full()
    }

    /// Applies `change` as one generation: transaction, commit point, manifest,
    /// publication, in that order.
    ///
    /// Every segment of the change must declare the new generation G+1, and
    /// every passage must have been created at G+1: rows that name another
    /// generation are refused with [`ErrorCode::Internal`], because they would
    /// write a visible state no commit produced (INV-03).
    ///
    /// Any error raised before the SQLite commit rolls the transaction back and
    /// publishes nothing, so the previous generation stays visible in full
    /// (INV-03). An error raised **after** the commit point (building the
    /// snapshot, writing the manifest) means the generation is committed but not
    /// published: the next [`GenerationStore::open`] rewrites the manifest and
    /// publishes it (ADR-0016, INV-04). A caller must therefore not read such an
    /// error as "nothing was written".
    pub fn commit(&mut self, change: GenerationChange) -> Result<Generation, CribleError> {
        let GenerationChange {
            segments,
            documents,
            passages,
            deletions,
        } = change;
        let epoch_dir = epoch_directory(&self.index_dir, self.epoch);

        let transaction = self.records.begin()?;
        let meta = records::read_meta(&transaction)?;
        let generation =
            Generation::new(meta.generation.get().checked_add(1).ok_or_else(|| {
                CribleError::new(
                    ErrorCode::Internal,
                    "the generation counter is exhausted, no generation can follow it",
                )
            })?);

        let mut next_iid = meta.next_iid;
        for segment in &segments {
            check_generation_matches(
                "segment",
                &segment.segment_id,
                segment.generation,
                generation,
            )?;
            records::insert_segment(&transaction, segment)?;
        }
        for document in &documents {
            records::insert_document(&transaction, document)?;
        }
        for passage in &passages {
            check_generation_matches(
                "passage",
                &passage.doc_id,
                passage.created_generation,
                generation,
            )?;
            records::insert_passage(&transaction, passage)?;
            next_iid = next_iid.max(passage.iid.get().checked_add(1).ok_or_else(|| {
                CribleError::new(
                    ErrorCode::Internal,
                    format!(
                        "iid {} leaves no free identifier after it",
                        passage.iid.get()
                    ),
                )
            })?);
        }
        for iid in &deletions {
            records::tombstone_passage(&transaction, *iid, generation)?;
        }
        records::advance_meta(&transaction, generation, next_iid)?;

        failpoint!("before_sqlite_commit");
        transaction
            .commit()
            .map_err(|error| records::database_error("commit the generation", &error))?;
        failpoint!("after_sqlite_commit");

        let snapshot = build_snapshot(&self.records, &epoch_dir)?;
        write_manifest(&epoch_dir, self.epoch, &snapshot)?;
        failpoint!("after_manifest");

        self.snapshot.store(Arc::new(snapshot));
        failpoint!("after_publish");
        Ok(generation)
    }
}

/// Directory of one epoch: `<index>/epochs/<epoch>`.
fn epoch_directory(index_dir: &Path, epoch: Epoch) -> PathBuf {
    index_dir
        .join(EPOCHS_DIR_NAME)
        .join(epoch.get().to_string())
}

/// Directory of the segments of an epoch.
fn segments_directory(epoch_dir: &Path) -> PathBuf {
    epoch_dir.join(SEGMENTS_DIR_NAME)
}

/// Directory of the derived manifests of an epoch.
fn manifests_directory(epoch_dir: &Path) -> PathBuf {
    epoch_dir.join(MANIFESTS_DIR_NAME)
}

/// Path of the derived manifest of one generation.
fn manifest_path(epoch_dir: &Path, generation: Generation) -> PathBuf {
    manifests_directory(epoch_dir).join(format!("{}.json", generation.get()))
}

/// Reads `CURRENT_EPOCH`, the decimal number of the current epoch.
///
/// The file is plain text, one decimal number, as
/// `docs/02-architecture.md` describes the layout: the version of the on-disk
/// layout is carried by the `epochs/<epoch>/` structure and by the format
/// versions of the files inside it, not by this pointer, which has to stay
/// readable by an operator with `cat`.
fn read_current_epoch(index_dir: &Path) -> Result<Epoch, CribleError> {
    let path = index_dir.join(CURRENT_EPOCH_FILE_NAME);
    let raw = fs::read_to_string(&path).map_err(|error| {
        CribleError::new(
            ErrorCode::InvalidRequest,
            format!(
                "{} is not a Crible index: cannot read {}: {error}",
                index_dir.display(),
                path.display()
            ),
        )
    })?;
    let trimmed = raw.trim();
    trimmed.parse::<u32>().map(Epoch::new).map_err(|error| {
        CribleError::new(
            ErrorCode::InvalidRequest,
            format!(
                "{} must hold the decimal number of the current epoch, found {trimmed:?}: \
                 {error}",
                path.display()
            ),
        )
    })
}

/// Writes `CURRENT_EPOCH` atomically: temporary file, fsync, rename, fsync of
/// the index directory.
fn write_current_epoch(index_dir: &Path, epoch: Epoch) -> Result<(), CribleError> {
    let bytes = format!("{}\n", epoch.get());
    write_atomically(&index_dir.join(CURRENT_EPOCH_FILE_NAME), bytes.as_bytes())
}

/// Writes a file atomically: `tmp-<name>` in the same directory, fsync, rename,
/// then fsync of the directory, so a crash leaves either the old bytes or the
/// new ones, never a truncated file.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), CribleError> {
    let parent = path.parent().ok_or_else(|| {
        CribleError::new(
            ErrorCode::Internal,
            format!(
                "{} has no parent directory, so no atomic write",
                path.display()
            ),
        )
    })?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            CribleError::new(
                ErrorCode::Internal,
                format!(
                    "{} has no UTF-8 file name, so no atomic write",
                    path.display()
                ),
            )
        })?;
    let temporary = parent.join(format!("{TEMPORARY_PREFIX}{file_name}"));
    write_file_synced(&temporary, bytes)?;
    fs::rename(&temporary, path)
        .map_err(|error| io_error("rename the temporary file into place", &temporary, &error))?;
    sync_directory(parent)
}

/// Removes what an interrupted commit or creation left behind (INV-04).
///
/// Every `tmp-*` entry is removed, including a residual `tmp-<segment_id>` that
/// would make [`SegmentWriter::create`](crate::segment::SegmentWriter) fail on
/// the next attempt with the same identifier, and a residual
/// `tmp-CURRENT_EPOCH` that an interrupted creation left in `index_dir`. Every
/// segment directory absent from the `segments` table is removed too: SQLite
/// never referenced it, so it belongs to a generation that was never committed
/// (ADR-0016).
///
/// The caller has read `CURRENT_EPOCH` before calling this, so a directory that
/// is not an index is never touched: a missing or unreadable pointer file is
/// refused earlier, by [`read_current_epoch`].
fn recover(records: &RecordStore, index_dir: &Path, epoch_dir: &Path) -> Result<(), CribleError> {
    let segments = segments_directory(epoch_dir);
    let manifests = manifests_directory(epoch_dir);
    for directory in [&segments, &manifests] {
        fs::create_dir_all(directory)
            .map_err(|error| io_error("create a directory of the epoch", directory, &error))?;
    }
    remove_temporary_entries(&manifests)?;
    remove_temporary_entries(index_dir)?;

    let referenced = records.segment_ids()?;
    let mut removed = false;
    for entry in fs::read_dir(&segments)
        .map_err(|error| io_error("list the segments of the epoch", &segments, &error))?
    {
        let entry = entry.map_err(|error| io_error("read a segment entry", &segments, &error))?;
        let path = entry.path();
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(CribleError::new(
                ErrorCode::Internal,
                format!(
                    "{} has no UTF-8 file name, so recovery cannot tell whether \
                     SQLite references it",
                    path.display()
                ),
            ));
        };
        let temporary = name.starts_with(TEMPORARY_PREFIX);
        if !temporary && referenced.iter().any(|id| id == name) {
            continue;
        }
        remove_entry(&path)?;
        removed = true;
    }
    if removed {
        sync_directory(&segments)?;
    }
    Ok(())
}

/// Removes every `tmp-*` entry of `dir`.
fn remove_temporary_entries(dir: &Path) -> Result<(), CribleError> {
    let mut removed = false;
    for entry in fs::read_dir(dir).map_err(|error| io_error("list the directory", dir, &error))? {
        let entry = entry.map_err(|error| io_error("read a directory entry", dir, &error))?;
        let name = entry.file_name();
        let temporary = name
            .to_str()
            .is_some_and(|name| name.starts_with(TEMPORARY_PREFIX));
        if temporary {
            remove_entry(&entry.path())?;
            removed = true;
        }
    }
    if removed {
        sync_directory(dir)?;
    }
    Ok(())
}

/// Removes one file or one directory tree.
fn remove_entry(path: &Path) -> Result<(), CribleError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| io_error("inspect a leftover entry", path, &error))?;
    let outcome = if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    outcome.map_err(|error| io_error("remove a leftover entry", path, &error))
}

/// Rejects a row whose generation contradicts the one being committed.
///
/// A segment becomes visible with the generation it declares, and a passage is
/// created by the generation it declares: both must be the generation this
/// commit publishes (G+1), otherwise the visible state would hold a generation
/// number that no commit ever produced, and a snapshot would mix two of them
/// (INV-03). A caller that prepares a change for another generation is out of
/// step with the store, which is an internal error, not a bad request.
fn check_generation_matches(
    kind: &str,
    identifier: &str,
    declared: Generation,
    committed: Generation,
) -> Result<(), CribleError> {
    if declared == committed {
        return Ok(());
    }
    Err(CribleError::new(
        ErrorCode::Internal,
        format!(
            "{kind} {identifier} declares generation {} but this commit publishes {}",
            declared.get(),
            committed.get()
        ),
    ))
}

/// Builds the snapshot of the committed generation from the reference state.
///
/// The generation and the live iids come from SQLite, the only authority on
/// what is visible. The segment metadata is read from the segments table and
/// from `segment.json` on disk, which carries the format version and the
/// precision, then sorted by `iid_start`: a snapshot is read in iid order, and
/// a restart produces the manifest in the same order as the commit that wrote
/// it. No vector file is opened here: the snapshot of this task holds metadata,
/// not mmaps.
fn build_snapshot(records: &RecordStore, epoch_dir: &Path) -> Result<Snapshot, CribleError> {
    let generation = records.meta()?.generation;
    let segments_dir = segments_directory(epoch_dir);
    let mut segments = Vec::new();
    for segment_id in records.segment_ids()? {
        segments.push(segment::read_meta(&segments_dir.join(&segment_id))?);
    }
    segments.sort_by(|left, right| {
        left.iid_start
            .get()
            .cmp(&right.iid_start.get())
            .then_with(|| left.segment_id.cmp(&right.segment_id))
    });
    Ok(Snapshot {
        generation,
        segments,
        live_iids: records.live_iids()?,
    })
}

/// Writes the derived manifest of `snapshot`, atomically.
fn write_manifest(epoch_dir: &Path, epoch: Epoch, snapshot: &Snapshot) -> Result<(), CribleError> {
    let path = manifest_path(epoch_dir, snapshot.generation());
    let bytes = manifest_json(epoch, snapshot)?;
    write_atomically(&path, bytes.as_bytes())
}

/// Writes the manifest of `snapshot` when it is missing, as recovery does.
fn ensure_manifest(epoch_dir: &Path, epoch: Epoch, snapshot: &Snapshot) -> Result<(), CribleError> {
    if manifest_path(epoch_dir, snapshot.generation()).exists() {
        return Ok(());
    }
    write_manifest(epoch_dir, epoch, snapshot)
}

/// Serializes the derived view of one generation as canonical JSON.
///
/// The manifest is a derived view, never a source of truth (ADR-0016): it names
/// its generation, its epoch and its segments, with the same fields as
/// `segment.json`, so an operator can read one generation with `cat`. It
/// carries the version of its own format (INV-15).
fn manifest_json(epoch: Epoch, snapshot: &Snapshot) -> Result<String, CribleError> {
    let segments: Vec<Value> = snapshot.segments().iter().map(segment_value).collect();
    let manifest = json!({
        "epoch": epoch.get(),
        "format_version": MANIFEST_FORMAT_VERSION,
        "generation": snapshot.generation().get(),
        "segments": segments,
    });
    serde_json::to_string(&manifest).map_err(|error| {
        CribleError::new(
            ErrorCode::Internal,
            format!("cannot serialize the manifest of a generation: {error}"),
        )
    })
}

/// Derived view of one segment inside a manifest, with the fields of its
/// `segment.json`.
fn segment_value(meta: &SegmentMeta) -> Value {
    json!({
        "batch_id": meta.batch_id,
        "checksums": meta.checksums,
        "count": meta.count,
        "dims": meta.dims,
        "format_version": meta.format_version,
        "generation": meta.generation.get(),
        "iid_start": meta.iid_start.get(),
        "segment_id": meta.segment_id,
    })
}

#[cfg(test)]
mod tests {
    use super::{CURRENT_EPOCH_FILE_NAME, GenerationStore, read_current_epoch};
    use crible_core::error::ErrorCode;
    use crible_core::ids::Epoch;
    use std::fs;

    /// Creation writes the pointer file, readable as a plain decimal number, and
    /// the manifest of generation 0, so a fresh index is inspectable.
    #[test]
    fn create_writes_the_current_epoch_and_the_manifest_of_generation_zero() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let index = temp.path().join("index");
        let store = GenerationStore::create(&index, Epoch::new(3)).expect("the index is created");

        assert_eq!(store.epoch(), Epoch::new(3));
        assert_eq!(store.generation().get(), 0);
        assert_eq!(
            read_current_epoch(&index).expect("the pointer file is readable"),
            Epoch::new(3)
        );
        assert!(
            index
                .join("epochs")
                .join("3")
                .join("manifests")
                .join("0.json")
                .is_file(),
            "creation must leave the manifest of generation 0"
        );
    }

    /// A directory without `CURRENT_EPOCH` is not an index: opening it is a bad
    /// request, and recovery does not touch it.
    #[test]
    fn read_current_epoch_refuses_a_directory_that_is_not_an_index() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let error = read_current_epoch(temp.path())
            .expect_err("a directory without CURRENT_EPOCH is not an index");
        assert_eq!(
            error.code(),
            ErrorCode::InvalidRequest,
            "wrong error code: {}",
            error.message()
        );

        let path = temp.path().join(CURRENT_EPOCH_FILE_NAME);
        fs::write(&path, "not a number").expect("the pointer file is written");
        let error =
            read_current_epoch(temp.path()).expect_err("a pointer file that is not a number");
        assert_eq!(
            error.code(),
            ErrorCode::InvalidRequest,
            "wrong error code: {}",
            error.message()
        );
    }
}
