// SPDX-License-Identifier: Apache-2.0

//! Compaction of one index: builds epoch E+1 without the dead passages.
//!
//! References: `docs/02-architecture.md` ("Suppressions, pierres tombales et
//! compaction", the six steps; "Concurrence"), `ADR-0006` (segments, tombstones
//! and occasional compaction), `ADR-0017` (the compaction suspends the writer,
//! the reads continue) and `ADR-0043` (point 4: an epoch is never deleted while
//! a snapshot holds it, and the payload step always reads the `records.sqlite`
//! of its snapshot). Invariants: INV-02 (`docs/08-invariants-et-tests.md`) and
//! INV-14.
//!
//! # The six steps
//!
//! 1. **Suspend the writer.** The caller holds the `&mut GenerationStore`, so no
//!    commit can run; the readers keep their published snapshots and are never
//!    blocked (ADR-0017, INV-02).
//! 2. **Read** the live passages of the published snapshot, sorted by
//!    (`cluster_by`, `doc_id`, `ord`), so that a filter on a clustered field
//!    walks a contiguous range of identifiers (`docs/03-recherche.md`).
//! 3. **Write E+1** in `epochs/tmp-<E+1>/`: segments of at most
//!    [`MAX_SEGMENT_PASSAGES`] passages, renumbered from 0, and a new
//!    `records.sqlite` that keeps the `batches` history (INV-10). The directory
//!    is fsynced and renamed to `epochs/<E+1>/`, so a crash never leaves a
//!    half-written epoch under its final name.
//! 4. **Verify** the counts, the tiling of the segments by `iid_start` and the
//!    checksums of the new vector files. The equivalence of an sample of
//!    queries is INV-09, the task `VERIFIER-EQUIVALENCE-COMPACTION`.
//! 5. **Switch** `CURRENT_EPOCH` to E+1, with the temporary-file protocol: this
//!    rename is the point of the compaction. The generation becomes G+1.
//! 6. **Publish** the snapshot of E+1, then delete E once no snapshot holds it
//!    ([`reclaim_epochs`]).
//!
//! An error before the switch leaves E current, whole and usable (INV-14).
//! Nothing of E+1 is reachable: `CURRENT_EPOCH` still names E, so a later
//! [`GenerationStore::open`] recovers E, and a later [`compact`] removes the
//! abandoned E+1 and converges to a complete one.
//!
//! # Cost
//!
//! The step 4 opens every new segment with
//! [`SegmentReader::open`](crate::segment::SegmentReader::open), which streams
//! the vector file through its blake3 checksum: the compaction reads each new
//! vector file once more than it wrote it. `MES-07` measures the duration of the
//! whole compaction; sampling the verification would be a change of this
//! protocol, for a later decision.
//!
//! # Identifiers of a compacted epoch
//!
//! A segment of a compacted epoch carries the identifier `c<epoch>-<iid_start>`
//! and a `batch_id` of the form `compaction-e<E>-e<E+1>`. That `batch_id` names
//! the compaction that built the segment, not an attach: no row of `batches`
//! carries it, and INV-10's replay check therefore never matches it. The history
//! of `batches` is copied unchanged from E, so a lot applied before the
//! compaction still replays as a no-op afterwards. A reader of `segment.json`
//! (export, snapshot, inspection) must read `batch_id` as this provenance
//! marker, not as a foreign key.
//!
//! # Fault injection
//!
//! With the `failpoints` cargo feature, and only with it, the compaction stops
//! at `CRIBLE_FAILPOINT=before_epoch_switch` (E+1 written and verified,
//! `CURRENT_EPOCH` still naming E) or `CRIBLE_FAILPOINT=after_epoch_switch`
//! (`CURRENT_EPOCH` naming E+1, nothing published, E still on disk). Stopping
//! means panicking with a message containing `CRIBLE_FAILPOINT`, exactly as the
//! commit protocol does. [`compact_with_observer`] is the other test hook: it
//! calls back at each step, so a test holds the compaction at a known point
//! without sleeping.

use crible_core::config::{FieldLevel, FieldType, IndexConfig};
use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::{Epoch, Generation, Iid};
use serde_json::Value;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use crate::filters_index::date_days;
use crate::generation::{
    GenerationStore, IndexState, MANIFESTS_DIR_NAME, RECORDS_FILE_NAME, SEGMENTS_DIR_NAME,
    Snapshot, TEMPORARY_PREFIX, epoch_directory, epochs_directory, generation_after,
    read_current_epoch, stop_at_failpoint, write_current_epoch,
};
use crate::records::{self, DocumentRecord, PassageRecord, RecordStore};
use crate::segment::{
    SegmentMeta, SegmentReader, SegmentSpec, SegmentWriter, io_error, sync_directory,
};

/// Largest number of passages one segment of a compacted epoch holds.
///
/// `docs/02-architecture.md` (compaction, step 3): "des segments d'au plus 1 M
/// passages". At the scale of Legifacile that is nine segments for 8,58 M
/// passages, each covering a contiguous range of identifiers.
pub const MAX_SEGMENT_PASSAGES: u32 = 1_000_000;

/// What one compaction did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactReport {
    /// Epoch the compaction started from.
    pub from_epoch: Epoch,
    /// Epoch `CURRENT_EPOCH` names when the call returns.
    pub to_epoch: Epoch,
    /// Generation published when the call returns: G+1.
    pub generation: Generation,
    /// Passages the new epoch holds.
    pub live_passages: u64,
    /// Passages of E the compaction did not copy: the tombstones.
    pub removed_passages: u64,
}

/// Progress of a compaction, reported to the observer and to the log.
#[derive(Debug, Clone, Copy)]
pub enum CompactStep {
    /// Step 1: the writer is suspended, the reads continue.
    WriterSuspended,
    /// Step 3: the segments and the new `records.sqlite` of E+1 are written.
    EpochWritten,
    /// Step 4: the counts, the tiling of the segments and the checksums are
    /// verified. `CURRENT_EPOCH` still names E.
    Verified,
    /// Step 5: `CURRENT_EPOCH` names E+1. E is still on disk.
    Switched,
    /// Step 6: the snapshot of E+1 is published.
    Published,
}

/// True when the state of an index reaches the `dead_ratio` of its
/// configuration (`docs/02-architecture.md`, compaction).
///
/// An index that holds no dead passage is never compacted, whatever the
/// threshold: a compaction copies the whole epoch, so a ratio of 0 with a
/// threshold of 0 is not a reason to rewrite it. A caller may still compact by
/// hand (`crible compact`), which ignores this predicate.
#[must_use]
pub fn needs_compaction(state: &IndexState, index_config: &IndexConfig) -> bool {
    state.dead_passages > 0 && state.dead_ratio() >= index_config.compaction.dead_ratio
}

/// Compacts the index of `store` into the next epoch.
///
/// The six steps and their failure modes are described in the module comment.
/// `index_config` is the configuration the index declares: the compaction reads
/// `cluster_by` and the precision of the reference vectors from it, the same way
/// an attach does.
pub fn compact(
    store: &mut GenerationStore,
    index_config: &IndexConfig,
) -> Result<CompactReport, CribleError> {
    run(store, index_config, &mut |_step| {})
}

/// Same as [`compact`], and calls `observer` at each step.
///
/// Test hook: it only exists with the `failpoints` cargo feature, never in a
/// release build. The observer runs on the caller's thread, once per step and in
/// the order of the six steps, so a test that blocks in it holds the compaction
/// at that exact step.
#[cfg(feature = "failpoints")]
pub fn compact_with_observer<F>(
    store: &mut GenerationStore,
    index_config: &IndexConfig,
    observer: F,
) -> Result<CompactReport, CribleError>
where
    F: FnMut(CompactStep),
{
    let mut observer = observer;
    run(store, index_config, &mut observer)
}

/// Deletes every epoch directory that `CURRENT_EPOCH` does not name and that no
/// snapshot of this store holds, then returns the deleted epochs in ascending
/// order.
///
/// The current epoch is never deleted. An epoch a reader still holds is never
/// deleted either (INV-14, ADR-0043 point 4): a search that started before a
/// compaction keeps reading the `records.sqlite`, the segments and the payloads
/// of the epoch of its snapshot, and the epoch directory only disappears once
/// that snapshot is dropped.
///
/// A compaction calls this at its last step, but a caller may call it on its own,
/// for instance after a crash that left an old epoch behind: recovery only
/// cleans the current epoch (`docs/02-architecture.md`, "Reprise au démarrage"),
/// so a crash after the switch of a compaction leaves E on disk until the next
/// reclaim.
pub fn reclaim_epochs(store: &GenerationStore) -> Result<Vec<Epoch>, CribleError> {
    let current = store.epoch();
    // The epoch the store holds and the one on disk must agree; if they do not,
    // the pointer is the authority, and the epoch it names is never deleted.
    let on_disk = read_current_epoch(store.index_dir())?;
    let held = store.held_epochs();
    let epochs_dir = epochs_directory(store.index_dir());
    if !epochs_dir.exists() {
        return Ok(Vec::new());
    }

    let mut deleted = Vec::new();
    for entry in fs::read_dir(&epochs_dir)
        .map_err(|error| io_error("list the epochs", &epochs_dir, &error))?
    {
        let entry = entry.map_err(|error| io_error("read an epoch entry", &epochs_dir, &error))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Ok(number) = name.parse::<u32>() else {
            // A `tmp-<epoch>` directory of an interrupted compaction: the
            // compaction removes its own staging directory, not the reclaim.
            continue;
        };
        let epoch = Epoch::new(number);
        if epoch == current || epoch == on_disk || held.contains(&epoch) {
            continue;
        }
        remove_directory(&entry.path())?;
        deleted.push(epoch);
    }
    if !deleted.is_empty() {
        sync_directory(&epochs_dir)?;
    }
    deleted.sort_unstable();
    Ok(deleted)
}

/// Runs a compaction and reports each step to `observer`.
fn run<F>(
    store: &mut GenerationStore,
    index_config: &IndexConfig,
    observer: &mut F,
) -> Result<CompactReport, CribleError>
where
    F: FnMut(CompactStep),
{
    let index_dir = store.index_dir().to_path_buf();
    let from_epoch = store.epoch();
    // The pointer is the authority on which epoch is current. A store left on E
    // while `CURRENT_EPOCH` names E+1 (a failed `adopt_epoch`) would otherwise
    // compute E+1 as its target, remove the very epoch the pointer names, and
    // leave the index pointing at a missing directory if it stopped before the
    // switch. Only a reopen publishes the epoch the pointer names (INV-04).
    let on_disk = read_current_epoch(&index_dir)?;
    if on_disk != from_epoch {
        return Err(internal(format!(
            "CURRENT_EPOCH names epoch {} but this store holds epoch {}; reopen the store before \
             compacting",
            on_disk.get(),
            from_epoch.get()
        )));
    }
    let to_epoch = next_epoch(from_epoch)?;
    let generation = generation_after(store.generation())?;
    let schema = &index_config.schema;
    let cluster = ClusterPlan::from_config(index_config)?;

    // Step 1: the writer is suspended by the exclusive borrow; the readers keep
    // their snapshots of E and are not blocked (ADR-0017, INV-02).
    let state = store.state()?;
    let removed_passages = state.dead_passages;
    observer(CompactStep::WriterSuspended);

    let source = store.snapshot();
    if source.epoch() != from_epoch {
        return Err(internal(format!(
            "the published snapshot belongs to epoch {} but the index is on epoch {}",
            source.epoch().get(),
            from_epoch.get()
        )));
    }
    // Step 2: the live passages of the published snapshot, in the order of the
    // new identifiers. The document rows are read once, here, and the ordering
    // looks their fields up by `doc_id`: reading them through the passages would
    // parse each document once per passage.
    let documents = records::all_documents(store.records().connection())?;
    let batches = records::all_batches(store.records().connection())?;
    let identities =
        records::live_identities(store.records().connection(), cluster.reads_passage_fields())?;
    let order = cluster.order(identities, &documents)?;
    let live_passages = u64::try_from(order.len()).map_err(|_| {
        internal("the epoch holds more live passages than a u64 can count".to_owned())
    })?;
    let next_iid = u32::try_from(order.len()).map_err(|_| {
        internal(
            "the epoch holds more live passages than the internal identifier space, so no \
             compaction can renumber them"
                .to_owned(),
        )
    })?;

    // Step 3: E+1 is built under a temporary name, then renamed into place.
    let target = epoch_directory(&index_dir, to_epoch);
    if target == epoch_directory(&index_dir, from_epoch) {
        return Err(internal(
            "the compaction target is the current epoch, so it would overwrite it".to_owned(),
        ));
    }
    let staging =
        epochs_directory(&index_dir).join(format!("{TEMPORARY_PREFIX}{}", to_epoch.get()));
    remove_directory(&staging)?;
    remove_directory(&target)?;
    write_new_epoch(
        store,
        &staging,
        to_epoch,
        generation,
        next_iid,
        &order,
        &documents,
        &batches,
        &source,
        index_config,
    )?;
    rename_staging_into_place(&staging, &target, &index_dir)?;
    observer(CompactStep::EpochWritten);
    // The copy of E is done, so the compaction releases its own snapshot of E
    // here, before the reclaim of step 6. Keeping it would make its lease count
    // as a live view of E, and `reclaim_epochs` would never delete E in this
    // call: every compaction would leave the previous epoch on disk (INV-14,
    // ADR-0043 point 4). E stays on disk until the switch, so the readers that
    // still hold it are not affected.
    drop(source);
    // Step 4: counts, tiling and checksums of E+1.
    let records = verify_epoch(
        &target,
        to_epoch,
        generation,
        live_passages,
        next_iid,
        &documents,
    )?;
    observer(CompactStep::Verified);
    stop_at_failpoint("before_epoch_switch");

    // Step 5: the switch.
    write_current_epoch(&index_dir, to_epoch)?;
    observer(CompactStep::Switched);
    stop_at_failpoint("after_epoch_switch");

    // Step 6: publish, then reclaim E.
    store.adopt_epoch(to_epoch, records, schema)?;
    observer(CompactStep::Published);
    if let Err(error) = reclaim_epochs(store) {
        // The new epoch is live and whole: a leftover old epoch is a cost on
        // disk, not a corruption, so the compaction does not fail for it.
        tracing::warn!(
            error = %error.message(),
            epoch = from_epoch.get(),
            "cannot reclaim the epoch the compaction replaced"
        );
    }

    Ok(CompactReport {
        from_epoch,
        to_epoch,
        generation,
        live_passages,
        removed_passages,
    })
}

/// Builds the whole epoch directory at `staging`: segments, `records.sqlite`,
/// its rows, and the history of the applied batches.
///
/// The new database is filled in one transaction, whose commit is not the commit
/// point of the index: the epoch is invisible until `CURRENT_EPOCH` names it.
/// The transaction still keeps the epoch either whole or absent if the process
/// dies while it is written.
#[allow(clippy::too_many_arguments)]
fn write_new_epoch(
    store: &GenerationStore,
    staging: &Path,
    epoch: Epoch,
    generation: Generation,
    next_iid: u32,
    order: &[Iid],
    documents: &[DocumentRecord],
    batches: &[records::StoredBatchRow],
    source: &Snapshot,
    index_config: &IndexConfig,
) -> Result<(), CribleError> {
    let segments_dir = staging.join(SEGMENTS_DIR_NAME);
    let manifests_dir = staging.join(MANIFESTS_DIR_NAME);
    for directory in [staging, &segments_dir, &manifests_dir] {
        fs::create_dir_all(directory)
            .map_err(|error| io_error("create a directory of the new epoch", directory, &error))?;
    }

    let mut records = RecordStore::create(&staging.join(RECORDS_FILE_NAME))?;
    records.set_epoch(epoch)?;

    let source_epoch_dir = epoch_directory(store.index_dir(), source.epoch());
    let source_segments_dir = crate::generation::segments_directory(&source_epoch_dir);
    let mut vectors = EpochVectors::new(source_segments_dir, source.segments());
    let batch_id = compaction_batch_id(store.epoch(), epoch);

    {
        let transaction = records.begin()?;
        for row in batches {
            records::insert_batch_row(&transaction, row)?;
        }
        for document in documents {
            records::insert_document(&transaction, document)?;
        }

        let mut writer: Option<SegmentWriter> = None;
        let mut segment_id = String::new();
        let mut in_segment: u32 = 0;
        let mut new_iid: u32 = 0;
        for old_iid in order {
            if writer.is_none() {
                let dims = vectors.dims().ok_or_else(|| {
                    internal(format!(
                        "passage {} is live but the epoch it belongs to holds no segment",
                        old_iid.get()
                    ))
                })?;
                segment_id = compacted_segment_id(epoch, new_iid);
                let spec = SegmentSpec {
                    segment_id: segment_id.clone(),
                    iid_start: Iid::new(new_iid),
                    dims,
                    generation,
                    batch_id: batch_id.clone(),
                    precision: index_config.reference_precision,
                };
                writer = Some(SegmentWriter::create(&segments_dir, spec)?);
            }
            let passage = records::passage_by_iid(store.records().connection(), *old_iid)?
                .ok_or_else(|| {
                    internal(format!(
                        "passage {} is live in the published snapshot but absent from the \
                         reference state of its epoch",
                        old_iid.get()
                    ))
                })?;
            let vector = vectors.vector(*old_iid)?;
            let active = writer.as_mut().ok_or_else(|| {
                internal("the segment of the new epoch was not created".to_owned())
            })?;
            active.append(&vector)?;

            // `docs/02-architecture.md`: the compaction renumbers the identifiers
            // and drops the tombstones. The creation generation and the fields of
            // the passage follow it, unchanged: nothing was re-encoded.
            let record = PassageRecord {
                iid: Iid::new(new_iid),
                doc_id: passage.doc_id,
                ord: passage.ord,
                text: passage.text,
                passage_hash: passage.passage_hash,
                fields: passage.fields,
                segment_id: segment_id.clone(),
                created_generation: passage.created_generation,
                dead_since_gen: None,
            };
            records::insert_passage(&transaction, &record)?;
            new_iid = new_iid.checked_add(1).ok_or_else(|| {
                internal(
                    "the new epoch holds more passages than its identifiers can number".to_owned(),
                )
            })?;
            in_segment += 1;
            if in_segment == MAX_SEGMENT_PASSAGES {
                in_segment = 0;
                close_segment(&transaction, &mut writer)?;
            }
        }
        close_segment(&transaction, &mut writer)?;
        records::advance_meta(&transaction, generation, next_iid)?;
        transaction
            .commit()
            .map_err(|error| records::database_error("commit the new epoch", &error))?;
    }

    rebuild_lexical(staging)?;
    // The connection must be closed before the directory is renamed: SQLite
    // derives the names of `records.sqlite-wal` and `-shm` from the path it was
    // opened with, and closing checkpoints the WAL into the database.
    drop(records);
    sync_file(&staging.join(RECORDS_FILE_NAME))?;
    sync_directory(&segments_dir)?;
    sync_directory(&manifests_dir)?;
    sync_directory(staging)?;
    Ok(())
}

/// Finishes the segment being written, when there is one, and records it in the
/// transaction of the new epoch.
fn close_segment(
    transaction: &rusqlite::Transaction<'_>,
    writer: &mut Option<SegmentWriter>,
) -> Result<(), CribleError> {
    let Some(active) = writer.take() else {
        return Ok(());
    };
    let meta = active.finish()?;
    records::insert_segment(transaction, &meta)
}

/// Renames `staging` to `target`, then fsyncs the `epochs/` directory, so the new
/// epoch survives a crash once it has its final name.
fn rename_staging_into_place(
    staging: &Path,
    target: &Path,
    index_dir: &Path,
) -> Result<(), CribleError> {
    fs::rename(staging, target)
        .map_err(|error| io_error("rename the new epoch into place", staging, &error))?;
    sync_directory(&epochs_directory(index_dir))
}

/// Verifies the new epoch and returns its reference state, open for the
/// publication that follows the switch.
///
/// Step 4 of `docs/02-architecture.md`: the counts, the tiling of the segments by
/// `iid_start` and the checksums. Every segment is opened through
/// [`SegmentReader::open`], which streams its vector file through the blake3
/// checksum the writer declared; a corrupted or truncated file is therefore
/// caught before `CURRENT_EPOCH` names the epoch. A failure is an
/// [`ErrorCode::Internal`] error: the compaction produced a state no commit
/// could have produced.
fn verify_epoch(
    epoch_dir: &Path,
    epoch: Epoch,
    generation: Generation,
    live_passages: u64,
    next_iid: u32,
    documents: &[DocumentRecord],
) -> Result<RecordStore, CribleError> {
    let records = RecordStore::open(&epoch_dir.join(RECORDS_FILE_NAME))?;
    let meta = records.meta()?;
    if meta.epoch != epoch {
        return Err(internal(format!(
            "the new epoch directory is epoch {} but its reference state declares epoch {}",
            epoch.get(),
            meta.epoch.get()
        )));
    }
    if meta.generation != generation {
        return Err(internal(format!(
            "the new epoch declares generation {} but the compaction committed {}",
            meta.generation.get(),
            generation.get()
        )));
    }
    if meta.next_iid != next_iid {
        return Err(internal(format!(
            "the new epoch announces the next iid {} but holds {} passages",
            meta.next_iid, live_passages
        )));
    }

    let counts = records.counts()?;
    if counts.passages != live_passages {
        return Err(internal(format!(
            "the new epoch holds {} passage rows but the compaction copied {}",
            counts.passages, live_passages
        )));
    }
    let expected_documents = u64::try_from(documents.len()).map_err(|_| {
        internal("the document count of the compaction does not fit in u64".to_owned())
    })?;
    if counts.documents != expected_documents {
        return Err(internal(format!(
            "the new epoch holds {} document rows but the compaction copied {}",
            counts.documents, expected_documents
        )));
    }
    let dead = records::tombstoned_passages(records.connection())?;
    if dead != 0 {
        return Err(internal(format!(
            "the new epoch carries {dead} tombstoned passage(s): a compaction copies the live \
             passages only"
        )));
    }

    let segments_dir = crate::generation::segments_directory(epoch_dir);
    let mut next_start: u64 = 0;
    for segment_id in records.segment_ids()? {
        let reader = SegmentReader::open(&segments_dir.join(&segment_id))?;
        let meta = reader.meta();
        if meta.generation != generation {
            return Err(internal(format!(
                "segment {segment_id} declares generation {} but the compaction committed {}",
                meta.generation.get(),
                generation.get()
            )));
        }
        if u64::from(meta.iid_start.get()) != next_start {
            return Err(internal(format!(
                "segment {segment_id} starts at iid {} but the previous segments end at {next_start}",
                meta.iid_start.get()
            )));
        }
        next_start = next_start
            .checked_add(u64::from(meta.count))
            .ok_or_else(|| internal("the segment counts of the new epoch overflow".to_owned()))?;
    }
    if next_start != live_passages {
        return Err(internal(format!(
            "the segments of the new epoch hold {next_start} passages but the reference state \
             holds {live_passages}"
        )));
    }
    Ok(records)
}

/// Placeholder of the rebuild of the lexical index of the new epoch.
///
/// Step 3 of `docs/02-architecture.md` has tantivy rebuilt from the stored texts.
/// Tantivy arrives in P5 (`RECHERCHER-LEXICAL`): the hook does nothing in P3, and
/// P5 fills it here, at the step the design names. The epoch directory is the one
/// the new database and its segments already live in.
fn rebuild_lexical(_epoch_dir: &Path) -> Result<(), CribleError> {
    Ok(())
}

/// The epoch that follows `epoch`, refusing an exhausted counter.
fn next_epoch(epoch: Epoch) -> Result<Epoch, CribleError> {
    epoch.get().checked_add(1).map(Epoch::new).ok_or_else(|| {
        internal(format!(
            "the epoch counter is exhausted: no epoch can follow {}",
            epoch.get()
        ))
    })
}

/// Identifier of a segment of a compacted epoch, derived from the epoch and from
/// the first identifier it covers.
///
/// The name is deterministic, never starts with the `tmp-` prefix the recovery
/// deletes, and names one directory of `segments/` (validated by
/// [`SegmentWriter::create`](crate::segment::SegmentWriter::create)).
fn compacted_segment_id(epoch: Epoch, iid_start: u32) -> String {
    format!("c{}-{iid_start:010}", epoch.get())
}

/// Batch identifier a compacted segment names: the compaction that built it.
///
/// No `batches` row carries it, because a compaction is not an attach. The field
/// is kept so every segment names where it comes from (`segment.json`).
fn compaction_batch_id(from_epoch: Epoch, to_epoch: Epoch) -> String {
    format!("compaction-e{}-e{}", from_epoch.get(), to_epoch.get())
}

/// Reads the vectors of the segments of the source epoch, opening each segment
/// the first time a passage of it is copied.
///
/// Opening a segment verifies its checksum, which reads the whole vector file
/// once: the compaction re-reads the source vectors it copies, and that cost is
/// the one `MES-07` measures.
struct EpochVectors<'a> {
    segments_dir: PathBuf,
    metas: &'a [SegmentMeta],
    readers: Vec<Option<SegmentReader>>,
}

impl<'a> EpochVectors<'a> {
    fn new(segments_dir: PathBuf, metas: &'a [SegmentMeta]) -> Self {
        Self {
            segments_dir,
            metas,
            readers: metas.iter().map(|_| None).collect(),
        }
    }

    /// Dimensionality of the vectors of the epoch, or `None` when it holds no
    /// segment at all.
    fn dims(&self) -> Option<u32> {
        self.metas.first().map(|meta| meta.dims)
    }

    /// Reference vector of `iid`, decoded from the segment that covers it.
    fn vector(&mut self, iid: Iid) -> Result<Vec<f32>, CribleError> {
        let index = self
            .metas
            .iter()
            .rposition(|meta| meta.iid_start.get() <= iid.get())
            .ok_or_else(|| {
                internal(format!(
                    "passage {} is live but no segment of its epoch starts before it",
                    iid.get()
                ))
            })?;
        if self.readers[index].is_none() {
            let path = self.segments_dir.join(&self.metas[index].segment_id);
            self.readers[index] = Some(SegmentReader::open(&path)?);
        }
        let reader = self.readers[index].as_ref().ok_or_else(|| {
            internal(format!(
                "the segment covering passage {} could not be opened",
                iid.get()
            ))
        })?;
        if !reader.contains(iid) {
            return Err(internal(format!(
                "passage {} is live but falls in the gap after segment {}",
                iid.get(),
                reader.meta().segment_id
            )));
        }
        reader.vector(iid)
    }
}

/// Ordering rule of the new identifiers: (`cluster_by`, `doc_id`, `ord`).
struct ClusterPlan<'a> {
    fields: Vec<ClusterField<'a>>,
}

/// One field of `cluster_by`, with the level that carries its value.
struct ClusterField<'a> {
    name: &'a str,
    level: FieldLevel,
    field_type: FieldType,
}

impl<'a> ClusterPlan<'a> {
    /// Reads `cluster_by` from the configuration of the index.
    ///
    /// A name the schema does not declare is an [`ErrorCode::InvalidRequest`]
    /// error: the configuration would order the epoch by a value no row carries.
    fn from_config(index_config: &'a IndexConfig) -> Result<Self, CribleError> {
        let mut fields = Vec::with_capacity(index_config.cluster_by.len());
        for name in &index_config.cluster_by {
            let Some((spec, level)) = index_config.schema.field(name) else {
                return Err(CribleError::new(
                    ErrorCode::InvalidRequest,
                    format!(
                        "cluster_by names the field '{name}', which the schema of the index \
                         does not declare"
                    ),
                ));
            };
            fields.push(ClusterField {
                name: name.as_str(),
                level,
                field_type: spec.field_type,
            });
        }
        Ok(Self { fields })
    }

    /// True when a field of `cluster_by` is carried by the passages.
    ///
    /// The compaction then reads the `fields` column of `passages`; when every
    /// clustered field is a document field, it reads none, and no JSON value is
    /// built per passage.
    fn reads_passage_fields(&self) -> bool {
        self.fields
            .iter()
            .any(|field| field.level == FieldLevel::Passage)
    }

    /// Sorts the live passages and returns their identifiers in the order of the
    /// new epoch.
    ///
    /// `documents` is the `documents` table as the compaction read it, in
    /// ascending `doc_id`: the value of a document-level cluster field is looked
    /// up there, so every document is parsed once for the whole compaction.
    ///
    /// `doc_id` and `ord` break the ties, and the old `iid` breaks the last ones,
    /// so two compactions of the same state produce the same order.
    fn order(
        &self,
        identities: Vec<records::LiveIdentity>,
        documents: &[DocumentRecord],
    ) -> Result<Vec<Iid>, CribleError> {
        let mut keys = Vec::with_capacity(identities.len());
        for identity in identities {
            let cluster = self.keys(&identity, documents)?;
            keys.push(LiveKey {
                iid: identity.iid,
                doc_id: identity.doc_id,
                ord: identity.ord,
                cluster,
            });
        }
        keys.sort_by(|left, right| {
            left.cluster
                .cmp(&right.cluster)
                .then_with(|| left.doc_id.cmp(&right.doc_id))
                .then_with(|| left.ord.cmp(&right.ord))
                .then_with(|| left.iid.cmp(&right.iid))
        });
        Ok(keys.into_iter().map(|key| key.iid).collect())
    }

    /// Value of every `cluster_by` field for one passage.
    fn keys(
        &self,
        identity: &records::LiveIdentity,
        documents: &[DocumentRecord],
    ) -> Result<Vec<ClusterKey>, CribleError> {
        self.fields
            .iter()
            .map(|field| field.value(identity, documents))
            .collect()
    }
}

impl ClusterField<'_> {
    /// Value of this field for one passage, `Absent` when it carries none.
    ///
    /// A document-level field is read from the document row of the same
    /// `doc_id`; a row that is gone gives `Absent` (ADR-0043). A value of another
    /// type than the declared one is an [`ErrorCode::Internal`] error: the attach
    /// that wrote the row validated it against the same schema. The message names
    /// the field and the identifier, never the value (rule 11 of `AGENTS.md`).
    fn value(
        &self,
        identity: &records::LiveIdentity,
        documents: &[DocumentRecord],
    ) -> Result<ClusterKey, CribleError> {
        let raw = match self.level {
            FieldLevel::Document => documents
                .binary_search_by(|document| document.doc_id.as_str().cmp(identity.doc_id.as_str()))
                .ok()
                .and_then(|index| documents[index].fields.get(self.name)),
            FieldLevel::Passage => identity
                .passage_fields
                .as_ref()
                .and_then(|fields| fields.get(self.name)),
        };
        let Some(raw) = raw else {
            return Ok(ClusterKey::Absent);
        };
        let key = match (self.field_type, raw) {
            (_, Value::Null) => Some(ClusterKey::Absent),
            (FieldType::Keyword, Value::String(text)) => Some(ClusterKey::Keyword(text.clone())),
            (FieldType::Integer, value) => value.as_i64().map(ClusterKey::Integer),
            (FieldType::Float, value) => value
                .as_f64()
                .map(|number| ClusterKey::Float(ordered_bits(number))),
            (FieldType::Date, Value::String(text)) => {
                date_days(text).map(|days| ClusterKey::Integer(i64::from(days)))
            }
            (FieldType::Bool, value) => value.as_bool().map(ClusterKey::Bool),
            _ => None,
        };
        key.ok_or_else(|| {
            internal(format!(
                "the value of cluster field '{}' on passage {} is not a {}; the attach that \
                 wrote the row validated it against the same schema",
                self.name,
                identity.iid.get(),
                self.field_type.as_str()
            ))
        })
    }
}

/// One live passage with the values its cluster fields give it.
struct LiveKey {
    iid: Iid,
    doc_id: String,
    ord: u32,
    cluster: Vec<ClusterKey>,
}

/// Value of one `cluster_by` field, in the order the compaction sorts by.
///
/// A missing value sorts first. The variants are one per declared type, and a
/// field has one type in the schema, so only values of the same variant are ever
/// compared. Floats are stored as their ordered bits, which makes the derived
/// `Ord` the total order of `f64` without a manual comparison.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum ClusterKey {
    Absent,
    Bool(bool),
    Integer(i64),
    Float(u64),
    Keyword(String),
}

/// Maps an `f64` to an integer whose unsigned order is the order of the number.
fn ordered_bits(value: f64) -> u64 {
    let bits = value.to_bits();
    if bits & (1 << 63) != 0 {
        !bits
    } else {
        bits | (1 << 63)
    }
}

/// Removes a directory tree, and accepts its absence.
fn remove_directory(path: &Path) -> Result<(), CribleError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error("remove a directory of the index", path, &error)),
    }
}

/// fsyncs a file, so the rename of its directory cannot outlive its bytes.
fn sync_file(path: &Path) -> Result<(), CribleError> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| io_error("fsync the file", path, &error))
}

/// Builds an internal error, the code of a state no commit could have produced.
fn internal(message: String) -> CribleError {
    CribleError::new(ErrorCode::Internal, message)
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_SEGMENT_PASSAGES, compacted_segment_id, needs_compaction, next_epoch, ordered_bits,
    };
    use crate::filters_index::FilterMemory;
    use crate::generation::IndexState;
    use crible_core::config::IndexConfig;
    use crible_core::error::ErrorCode;
    use crible_core::ids::{Epoch, Generation};

    /// Index state holding `live` live and `dead` dead passages.
    fn state(live: u64, dead: u64) -> IndexState {
        IndexState {
            epoch: Epoch::new(1),
            generation: Generation::new(1),
            documents: 1,
            live_passages: live,
            dead_passages: dead,
            segments: Vec::new(),
            filter_memory: FilterMemory {
                live: 0,
                keywords: 0,
                booleans: 0,
                columns: 0,
            },
        }
    }

    /// Index configuration whose `dead_ratio` is `ratio`.
    fn config(ratio: f64) -> IndexConfig {
        let raw = format!(
            r#"{{"name":"i","model":"m","reference_precision":"fp16",
                "schema":{{"document_fields":{{}},"passage_fields":{{}}}},
                "cluster_by":[],
                "lexical":{{"enabled":false,"language":"fr","positions":false}},
                "search":{{"scan":"none","rescore_candidates":10,"exact_threshold":10,
                    "candidates_per_branch":10,"fusion":{{"method":"rrf"}}}},
                "ingest":{{"acceptance_sample":1,"acceptance_min_cosine":0.9}},
                "compaction":{{"dead_ratio":{ratio}}}}}"#
        );
        IndexConfig::from_json_str(&raw)
            .unwrap_or_else(|error| panic!("the test configuration must parse: {error}"))
    }

    /// The `dead_ratio` trigger fires at or above the threshold of the index
    /// configuration, and never on an index that holds no dead passage, even
    /// with a threshold of 0: a compaction would copy the whole epoch.
    #[test]
    fn needs_compaction_follows_the_dead_ratio_of_the_configuration() {
        assert!(
            !needs_compaction(&state(10, 1), &config(0.1)),
            "a ratio below the threshold must not trigger a compaction"
        );
        assert!(
            needs_compaction(&state(10, 10), &config(0.1)),
            "a ratio at the threshold must trigger a compaction"
        );
        assert!(
            needs_compaction(&state(0, 10), &config(1.0)),
            "an epoch of only dead passages must trigger a compaction"
        );
        assert!(
            !needs_compaction(&state(10, 0), &config(0.0)),
            "an index without a dead passage is never compacted"
        );
        assert!(
            !needs_compaction(&state(0, 0), &config(0.0)),
            "an empty index is never compacted"
        );
    }

    /// The epoch counter is refused at its end instead of wrapping around.
    #[test]
    fn next_epoch_refuses_an_exhausted_counter() {
        assert_eq!(
            next_epoch(Epoch::new(1)).unwrap_or_else(|error| panic!("epoch 2 must exist: {error}")),
            Epoch::new(2)
        );
        let error = next_epoch(Epoch::new(u32::MAX)).expect_err("no epoch can follow u32::MAX");
        assert_eq!(error.code(), ErrorCode::Internal);
    }

    /// A compacted segment is named after its epoch and its first identifier, and
    /// the name is not one the recovery would delete.
    #[test]
    fn compacted_segment_ids_are_deterministic_and_safe() {
        assert_eq!(
            compacted_segment_id(Epoch::new(3), 0),
            "c3-0000000000".to_owned()
        );
        assert_eq!(
            compacted_segment_id(Epoch::new(3), MAX_SEGMENT_PASSAGES),
            "c3-0001000000".to_owned()
        );
        assert!(!compacted_segment_id(Epoch::new(1), 0).starts_with("tmp-"));
    }

    /// The ordered bits of `f64` keep the order of the numbers, negative ones
    /// first.
    #[test]
    fn ordered_bits_follow_the_numeric_order() {
        let ordered: Vec<u64> = [-2.5, -0.0, 0.0, 1.0, 3.5]
            .into_iter()
            .map(ordered_bits)
            .collect();
        assert!(
            ordered.windows(2).all(|pair| pair[0] <= pair[1]),
            "the ordered bits must follow the numbers: {ordered:?}"
        );
    }
}
