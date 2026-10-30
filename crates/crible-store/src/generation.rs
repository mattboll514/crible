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
//! <index>/index.json                                configuration of the index (schema, frozen)
//! <index>/CURRENT_EPOCH                             current epoch, written tmp then renamed
//! <index>/epochs/<epoch>/records.sqlite             reference state (WAL)
//! <index>/epochs/<epoch>/segments/<segment_id>/     immutable vectors, see `segment`
//! <index>/epochs/<epoch>/manifests/<generation>.json derived view of one generation
//! ```
//!
//! # Commit protocol
//!
//! [`GenerationStore::commit`] applies one generation change in the order of
//! `docs/02-architecture.md` ("Générations et point de commit"). Steps 1 and 2 —
//! validate the batch, write the segment — belong to the attach, which runs them
//! before this call. The store then runs:
//!
//! 3. one SQLite transaction writes the segments, the documents, the passages,
//!    the tombstones, the batch row, `meta.generation = G+1`,
//!    `meta.next_iid` and — when the transaction creates a live passage or
//!    tombstones one — `meta.passage_change_generation = G+1`, then
//!    **`COMMIT` — the commit point (ADR-0016)**;
//! 4. the lexical branch receives the passages the transaction created, loses
//!    the ones it tombstoned, commits with `G+1` as its payload and reloads its
//!    reader ([`crate::lexical`]);
//! 5. `manifests/<G+1>.json` is written, fsynced and renamed;
//! 6. a new [`Snapshot`] is built and published through an [`ArcSwap`].
//!
//! Step 4 only runs when the transaction created or tombstoned a passage and the
//! configuration enables the lexical branch: a metadata patch, and an attach
//! that only rewrites document rows, leave tantivy untouched (`docs/02`), and
//! the snapshot keeps the reader of the previous generation, which the marker
//! says is still up to date (`ADR-0088`, decision 2).
//!
//! The segments themselves are written before the call, by the attach that
//! prepares them: a segment stays invisible until step 3 references it, which is
//! what makes an abandoned one an orphan that recovery removes.
//!
//! `GenerationStore::commit_with_rows` runs the same protocol and lets the
//! caller append its own rows (the attach writes the `batches` row there) to
//! step 3, still inside the single commit point. It also returns the time spent
//! in each of its post-write steps, which the attach reports (`docs/04`).
//!
//! # Recovery
//!
//! [`GenerationStore::open`] restores the last committed generation and nothing
//! else (INV-04): it reads `CURRENT_EPOCH`, opens SQLite, reads `meta`, removes
//! every `tmp-*` entry and every segment directory absent from the `segments`
//! table, rebuilds the `codes.bin` a segment is missing (`ADR-0069`, decision 1),
//! completes `meta.passage_change_generation` when the epoch predates it
//! (`ADR-0088`, decision 4), repairs the lexical index against the committed
//! generation (`docs/02`, "Reprise au démarrage"; ADR-0087, ADR-0088), rewrites
//! the manifest of the current generation when it is missing, then publishes the
//! snapshot. The schema completion, the lexical repair and the `codes.bin`
//! rebuild are the **write**-open steps: a read-only open writes nothing under
//! the epoch, opens no `IndexWriter` and takes a lexical reader only when the
//! state is up to date (`M ≤ Gt ≤ G`); otherwise the snapshot names the state
//! found and the lexical branch of a query refuses (`ADR-0087`, `ADR-0088`).
//!
//! # Fault injection
//!
//! With the `failpoints` cargo feature, and only with it, `commit` stops at the
//! step named by the `CRIBLE_FAILPOINT` environment variable
//! (`before_sqlite_commit`, `after_sqlite_commit`, `after_lexical_commit`,
//! `after_manifest`, `after_publish`). `after_lexical_commit` is reached only
//! when step 4 runs, so a configuration that disables the lexical branch never
//! stops there. Stopping means panicking with a message containing
//! `CRIBLE_FAILPOINT`: unwinding through the transaction rolls it back exactly
//! as a process death would. That panic is the expected outcome of the
//! `INV-03` / `INV-04` tests; nothing else in this module panics.
//!
//! # What the snapshot carries
//!
//! The snapshot carries the metadata of its segments, their **binary codes in
//! RAM** and the filter structures of its generation ([`FilterIndex`])
//! (`docs/02-architecture.md`, "Concurrence"; ADR-0043). While `codes.bin` is
//! present — the normal state, because the writer open rebuilds it — the codes
//! are read from that file and no vector file is opened, so the snapshot pays no
//! checksum read of `SegmentReader::open`. A missing file has one fallback, in
//! `crate::codes::load_codes`: the reference vectors are read through a
//! `SegmentReader`, checksum included, and quantized in RAM. A read-only open
//! takes that fallback for an index no write open has rebuilt yet (ADR-0069,
//! decision 2), and writes nothing. The arrays are shared between successive
//! snapshots of the same epoch by segment identifier: a segment is immutable, so
//! a commit reuses the codes of every segment it keeps, and a shared array is
//! used only after its length is checked against the segment metadata.
//!
//! The filter structures are built from `records.sqlite` at creation and at
//! startup, then derived generation by generation in copy on write by
//! `FilterIndex::after_commit` (ADR-0018, ADR-0019). The schema that describes
//! which field is indexed, and how, therefore reaches the store when it is
//! created or opened: the reference state holds the values, not the schema.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use crible_core::config::{IndexConfig, LexicalConfig, Schema};
use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::{Epoch, Generation, Iid};
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde_json::{Value, json};

use crate::codes;
use crate::filters_index::{CommitDelta, CreatedPassage, FilterIndex, FilterMemory};
use crate::lexical::{self, LexicalIndex, LexicalObservation, LexicalStatus};
use crate::records::{self, DocumentRecord, PassageRecord, RecordStore};
use crate::segment::{self, SegmentMeta, io_error, sync_directory, write_file_synced};

/// Version of the derived manifest format (INV-15).
pub const MANIFEST_FORMAT_VERSION: u32 = 1;

/// Name of the file holding the current epoch of an index.
pub const CURRENT_EPOCH_FILE_NAME: &str = "CURRENT_EPOCH";

/// Name of the index configuration inside an index directory.
///
/// `docs/02-architecture.md` ("Organisation sur disque") places `index.json`
/// next to `CURRENT_EPOCH`. The store reads its schema from it, so the filter
/// structures can never be built from a schema the index does not declare.
pub const INDEX_CONFIG_FILE_NAME: &str = "index.json";

/// Sub-directory of the data root that holds the indexes (`docs/02`).
pub const INDEXES_DIRECTORY: &str = "indexes";

/// Epoch a new index starts on: `crible index create` and a restoration to an
/// absent target both give it this one.
///
/// `ADR-0065`, decision 2: an absent target has no epoch to pass, so it gets the
/// initial epoch of a creation; the `E+1` rule only applies to an index that
/// already has an epoch. No on-disk format depends on the value: `CURRENT_EPOCH`,
/// `meta.epoch` and the manifests carry whatever epoch the commit writes, so this
/// is the convention of the commands, not a format constant.
pub const INITIAL_EPOCH: Epoch = Epoch::new(1);

/// Writes the index configuration `index.json` atomically in `index_dir`.
///
/// The directory is created when it does not exist. The caller writes the
/// configuration before [`GenerationStore::create`], which refuses an index
/// without one. The write is a temporary file, fsync, rename and fsync of the
/// directory (the protocol of `docs/02-architecture.md` for `CURRENT_EPOCH`),
/// so a crash leaves either no configuration or the whole one, never a
/// truncated `index.json` that a retry would refuse.
///
/// `config_json` is parsed with [`IndexConfig::from_json_str`] before anything
/// is written: a malformed configuration is refused with
/// [`ErrorCode::InvalidRequest`] and leaves the directory untouched, rather than
/// landing in an `index.json` that every later `open` and `create` would have to
/// reject. The stored form is the one of
/// [`IndexConfig::materialised_json`]: a setting that has a default is written
/// out, so a creation cannot leave an `index.json` whose effective threshold
/// depends on a later `PATCH` (ADR-0074, decision 2).
///
/// A directory holding any part of an index — `CURRENT_EPOCH`, an `epochs/`
/// directory or a `records.sqlite` — is refused with the same code, naming what
/// is present. Writing a different schema over an epoch that already holds rows
/// would change the filter structures of a populated `records.sqlite`, which
/// `RecordStore::create` reopens without emptying. A residual `index.json` alone
/// is not such a state: a creation that stopped before its first epoch left one,
/// and nothing else exists, so replacing it loses nothing.
pub fn write_index_config(index_dir: &Path, config_json: &str) -> Result<(), CribleError> {
    let stored = IndexConfig::materialised_json(config_json)?;
    if let Some(present) = occupied_index_state(index_dir) {
        return Err(existing_index_error(index_dir, &present));
    }
    fs::create_dir_all(index_dir)
        .map_err(|error| io_error("create the index directory", index_dir, &error))?;
    write_atomically(&index_dir.join(INDEX_CONFIG_FILE_NAME), stored.as_bytes())
}

/// Rewrites the configuration of an **existing** index atomically.
///
/// The `PATCH` job of ADR-0105 writes the new configuration here after the API
/// validated it against the current one. Unlike [`write_index_config`], this
/// function does not refuse a directory that already holds an index: replacing
/// `index.json` is the whole point. The text is parsed and materialised first
/// (ADR-0074), then written with the same temporary-file, fsync, rename and
/// directory-fsync protocol, so a crash leaves either the old configuration or
/// the whole new one.
///
/// # Errors
///
/// [`ErrorCode::InvalidRequest`] when the text is not a valid configuration, and
/// the errors of the atomic write otherwise.
pub fn rewrite_index_config(index_dir: &Path, config_json: &str) -> Result<(), CribleError> {
    let stored = IndexConfig::materialised_json(config_json)?;
    write_atomically(&index_dir.join(INDEX_CONFIG_FILE_NAME), stored.as_bytes())
}

/// Part of an index that already occupies an index directory.
#[derive(Debug, Clone, PartialEq, Eq)]
enum OccupiedIndex {
    /// A complete index: `CURRENT_EPOCH` names its current epoch.
    Complete(PathBuf),
    /// A tree a creation did not finish: an `epochs/` directory or a
    /// `records.sqlite` without the pointer that makes it usable.
    Partial(PathBuf),
}

/// What already occupies `index_dir`, if it holds any part of an index.
///
/// The states are checked in the order a creation writes them, except for the
/// `records.sqlite` at the index root: a creation never writes one there, since
/// the reference database lives inside its epoch directory. It is the trace of
/// a tree moved or interrupted by hand, and it is reported last, once neither
/// the pointer nor `epochs/` explains the directory.
///
/// A residual `index.json` is deliberately absent from this list: it is the only
/// entry a creation may replace.
fn occupied_index_state(index_dir: &Path) -> Option<OccupiedIndex> {
    let pointer = index_dir.join(CURRENT_EPOCH_FILE_NAME);
    if pointer.exists() {
        return Some(OccupiedIndex::Complete(pointer));
    }
    let epochs = index_dir.join(EPOCHS_DIR_NAME);
    if epochs.exists() {
        return Some(OccupiedIndex::Partial(epochs));
    }
    let records = index_dir.join(RECORDS_FILE_NAME);
    if records.exists() {
        return Some(OccupiedIndex::Partial(records));
    }
    None
}

/// Error returned when a creation targets an index that already exists.
///
/// A complete index and a partial tree are two different situations, so the
/// message distinguishes them: the first one is usable and names its commands,
/// the second one is the trace of a creation that stopped, and only there does
/// removing the directory make sense.
fn existing_index_error(index_dir: &Path, occupied: &OccupiedIndex) -> CribleError {
    let message = match occupied {
        OccupiedIndex::Complete(pointer) => format!(
            "an index already exists at {}: {} is present, and creating an index never \
             overwrites one; use `crible attach`, `crible search` or `crible inspect` on it, \
             or create another index under another name",
            index_dir.display(),
            pointer.display()
        ),
        OccupiedIndex::Partial(present) => format!(
            "a partial index tree is present at {}: {} exists, but the index has no {} pointer, \
             so no creation can reopen it, and creating an index never overwrites one; \
             remove it to create a new index",
            index_dir.display(),
            present.display(),
            CURRENT_EPOCH_FILE_NAME
        ),
    };
    CribleError::new(ErrorCode::InvalidRequest, message)
}

/// Name of the directory holding the epochs of an index.
pub(crate) const EPOCHS_DIR_NAME: &str = "epochs";

/// Name of the reference database inside an epoch directory.
pub(crate) const RECORDS_FILE_NAME: &str = "records.sqlite";

/// Name of the segments directory inside an epoch directory.
pub(crate) const SEGMENTS_DIR_NAME: &str = "segments";

/// Name of the derived manifests directory inside an epoch directory.
pub(crate) const MANIFESTS_DIR_NAME: &str = "manifests";

/// Prefix of a directory or file built before its rename into place.
///
/// The same prefix as the one [`SegmentWriter`](crate::segment::SegmentWriter)
/// uses: recovery removes every `tmp-*` entry of `segments/` and `manifests/`.
pub(crate) const TEMPORARY_PREFIX: &str = "tmp-";

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
///
/// The snapshot holds the binary codes of its segments in RAM, as
/// `docs/02-architecture.md` ("Concurrence") and ADR-0043 require. Each array is
/// behind an [`Arc`], so a commit copies no code: it clones the arrays of the
/// segments it keeps from the previous snapshot and loads only the new ones.
pub struct Snapshot {
    generation: Generation,
    segments: Vec<SegmentMeta>,
    /// Binary codes of `segments`, in the same order.
    codes: Vec<Arc<Vec<u64>>>,
    filters: FilterIndex,
    /// Reader of the lexical branch of the epoch, reloaded after the commit that
    /// published this generation (`docs/02`, step 4), or `None` when the index
    /// disables the branch, holds no lexical index yet, or was opened on a
    /// lexical state that is not up to date (ADR-0087, ADR-0088).
    lexical: Option<Arc<LexicalIndex>>,
    /// State the lexical index of the epoch was observed in when the snapshot
    /// was built (`ADR-0088`, decision 5).
    lexical_status: LexicalStatus,
    /// Keep-alive of the epoch the snapshot reads: its strong count is what the
    /// reclaim of [`crate::compact::reclaim_epochs`] looks at (INV-14).
    lease: Arc<EpochLease>,
}

impl std::fmt::Debug for Snapshot {
    /// Summarizes the code arrays: the largest field of a snapshot must not be
    /// printed whole by a log line or an error message.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Snapshot")
            .field("generation", &self.generation)
            .field("segments", &self.segments)
            .field("codes", &format_args!("{} bytes", self.codes_memory()))
            .field("filters", &self.filters)
            .field("lexical", &self.lexical.is_some())
            .field("lexical_status", &self.lexical_status)
            .field("lease", &self.lease)
            .finish()
    }
}

impl Snapshot {
    /// Generation this snapshot describes.
    #[must_use]
    pub fn generation(&self) -> Generation {
        self.generation
    }

    /// Epoch holding the reference state and the payloads of this snapshot.
    ///
    /// The payload step reads `records.sqlite` in this epoch (ADR-0043, point
    /// 4), and the compaction never deletes an epoch a live snapshot holds
    /// (INV-14).
    #[must_use]
    pub fn epoch(&self) -> Epoch {
        self.lease.epoch
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
    ///
    /// The identifiers are read from the bitmap of the live passages, which is
    /// the only copy the snapshot holds: materialising them again as a slice
    /// would store every live iid twice.
    pub fn live_iids(&self) -> impl Iterator<Item = Iid> + '_ {
        self.filters.live().iter().map(Iid::new)
    }

    /// Filter structures of the generation: the bitmaps per value, the present
    /// bitmaps, the value columns and the bitmap of the live passages.
    #[must_use]
    pub fn filters(&self) -> &FilterIndex {
        &self.filters
    }

    /// Reader of the lexical branch of the generation, or `None` when the index
    /// has no consistent lexical index.
    ///
    /// The reader was reloaded after the tantivy commit of this generation, so a
    /// request that holds the snapshot searches the state of its generation. It
    /// is absent when the configuration disables the branch, and when the
    /// lexical index is not up to date for the generation: an index behind, an
    /// index ahead, an epoch whose marker is not certified, and an unreadable
    /// `lexical/` are the states of [`Snapshot::lexical_status`] (`docs/02`,
    /// "Reprise au démarrage"; ADR-0087, ADR-0088). The lexical branch of a
    /// query refuses in `overloaded` naming the state it found
    /// (`RECHERCHER-BM25-DANS-LE-POOL`); this task exposes the state only.
    #[must_use]
    pub fn lexical(&self) -> Option<&LexicalIndex> {
        self.lexical.as_deref()
    }

    /// State the lexical index of the epoch was observed in when this snapshot
    /// was built (`ADR-0088`, decision 5).
    ///
    /// [`LexicalStatus::Ready`] is the only state in which [`Snapshot::lexical`]
    /// returns a reader. Every other state says why it does not, so the lexical
    /// branch of a query can name it to the client instead of asserting a delay
    /// that may not exist: behind, ahead, not certified, unreadable. This task
    /// publishes the state; the refusal is `RECHERCHER-BM25-DANS-LE-POOL`.
    #[must_use]
    pub fn lexical_status(&self) -> LexicalStatus {
        self.lexical_status
    }

    /// Reader of the lexical branch for the next snapshot to share, if any.
    fn shared_lexical(&self) -> Option<&Arc<LexicalIndex>> {
        self.lexical.as_ref()
    }

    /// Estimated heap size of the filter structures, which `crible inspect`
    /// reports.
    ///
    /// The measure covers the bitmaps, the value columns and the `keyword` values
    /// themselves. It leaves out the schema the structures were built from, which
    /// every generation shares and which does not grow with the passage count,
    /// and the fixed overhead of the maps (see [`FilterMemory`]). `MES-13` is the
    /// measure that settles what the report must count.
    #[must_use]
    pub fn filter_memory(&self) -> FilterMemory {
        self.filters.memory()
    }

    /// Bytes of RAM the binary codes of the generation hold: the sum of
    /// `count × ceil(dims / 64) × 8` over its segments.
    ///
    /// Every code of the generation is counted, whether it was read from a
    /// `codes.bin` or quantized in RAM at open because the file was missing
    /// (`ADR-0069`, decisions 1 and 2), and a tombstoned passage keeps its code
    /// until the compaction that drops it (`docs/02-architecture.md`,
    /// "Suppressions, pierres tombales et compaction"). `crible inspect` reports
    /// this figure.
    #[must_use]
    pub fn codes_memory(&self) -> u64 {
        self.segments
            .iter()
            .map(|meta| codes::codes_len(meta.count, meta.dims))
            .sum()
    }

    /// Binary codes of the segment `segment_id`, in RAM, one code after another
    /// as `codes.bin` holds them, or `None` when the generation holds no such
    /// segment.
    ///
    /// This is what the binary scan reads: the snapshot owns the array, so no
    /// query touches the disk for it.
    #[must_use]
    pub fn codes_of(&self, segment_id: &str) -> Option<&[u64]> {
        self.shared_codes(segment_id).map(|codes| codes.as_slice())
    }

    /// Code array of `segment_id` for the next snapshot to share, if any.
    fn shared_codes(&self, segment_id: &str) -> Option<&Arc<Vec<u64>>> {
        self.segments
            .iter()
            .position(|meta| meta.segment_id == segment_id)
            .and_then(|index| self.codes.get(index))
    }
}

/// Stops the process at `step` when `CRIBLE_FAILPOINT=<step>` is set.
///
/// Simulating a crash means panicking: the unwinding drops the SQLite
/// transaction, which rolls it back exactly as a process death would, and the
/// store is then reopened the way a restart does. That panic is the expected
/// outcome of the `INV-03` / `INV-04` tests and cannot be reached in a build
/// without the `failpoints` feature.
///
/// This is the one `panic!` the workspace keeps outside the tests, so the lint
/// `clippy::panic` of `Cargo.toml` is silenced here and nowhere else: an error
/// return would not simulate a process death, and the two invariant tests that
/// read `CRIBLE_FAILPOINT` expect the panic.
#[cfg(feature = "failpoints")]
#[allow(clippy::panic)]
pub(crate) fn stop_at_failpoint(step: &str) {
    let armed = std::env::var(FAILPOINT_ENV).unwrap_or_default();
    if armed == step {
        panic!("CRIBLE_FAILPOINT={step}: simulated crash at this step of the commit protocol");
    }
}

/// Does nothing: a normal build carries neither the environment read nor a call.
#[cfg(not(feature = "failpoints"))]
pub(crate) fn stop_at_failpoint(_step: &str) {}

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

/// A committed generation and the time spent in the steps that follow its
/// writes: the attach turns them into the stage durations of its report.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CommittedGeneration {
    /// Generation the change produced.
    pub(crate) generation: Generation,
    /// Time spent in each step of the commit protocol.
    pub(crate) timings: CommitTimings,
}

/// Step of the commit protocol of [`GenerationStore::commit_with_rows`] that an
/// observer is called at.
///
/// The two steps sit around the SQLite `COMMIT`, which is the only commit point
/// of a generation (ADR-0016). A job held at the first step has written no
/// visible state; a job held at the second one has committed a generation that
/// no reader sees until the snapshot is published (INV-02, INV-03).
///
/// The type is compiled in every build, like
/// [`CompactStep`](crate::compact::CompactStep): a production caller hands
/// [`GenerationStore::commit_with_rows`] a no-op observer, which the compiler
/// removes, and only the attach of the `failpoints` build
/// (`attach_with_observer`) hands a real one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitStep {
    /// Just before the `COMMIT` of the generation, no state visible yet.
    BeforeSqliteCommit,
    /// Just after the `COMMIT`, before the lexical index, the snapshot and the
    /// manifest are written: the generation is committed but not published.
    AfterSqliteCommit,
}

/// Time spent in the steps of [`GenerationStore::commit_with_rows`], measured
/// around the work this module owns.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CommitTimings {
    /// Filling the transaction and reaching the commit point.
    pub(crate) transaction: Duration,
    /// Step 4: the tantivy commit of the generation and its reader reload. Zero
    /// when the step does not run — a configuration that disables the lexical
    /// branch, or a delta that changes no passage text.
    pub(crate) lexical: Duration,
    /// Writing `manifests/<generation>.json`.
    pub(crate) manifest: Duration,
    /// Building the snapshot (step 6; its publication is immediate and not
    /// measured).
    pub(crate) snapshot: Duration,
}

/// The generation that follows `generation`, refusing an exhausted counter.
///
/// The attach computes its planned generation with this before the transaction,
/// and [`GenerationStore::commit_with_rows`] recomputes it inside; both must
/// agree, which the attach checks.
pub(crate) fn generation_after(generation: Generation) -> Result<Generation, CribleError> {
    let next = generation.get().checked_add(1).ok_or_else(|| {
        CribleError::new(
            ErrorCode::Internal,
            "the generation counter is exhausted, no generation can follow it",
        )
    })?;
    Ok(Generation::new(next))
}

/// Owner of one index: the writable reference database of the epoch and the
/// published snapshot of the current generation.
///
/// One writer per index (`docs/02-architecture.md`, "Concurrence"): the type
/// holds the only writable connection of `records.sqlite`, so no second writer
/// can bypass the commit point of ADR-0016.
pub struct GenerationStore {
    index_dir: PathBuf,
    records: RecordStore,
    snapshot: ArcSwap<Snapshot>,
    /// Lexical settings of the index, read from its `index.json`: the
    /// configuration is the authority on whether the branch exists
    /// (`docs/03-recherche.md`).
    lexical: LexicalConfig,
    /// Whether the store was opened for writing.
    ///
    /// Only a write open runs the reconstructions of derived directories —
    /// `codes.bin`, the completion of `meta.passage_change_generation` and the
    /// repair of `lexical/` — and only a write open may take the tantivy writer
    /// lock (`ADR-0069`, `ADR-0087`). The flag is what lets
    /// [`rebuild_lexical`] refuse a store a read command opened.
    writable: bool,
    lease: Arc<EpochLease>,
    retained: Mutex<Vec<Weak<EpochLease>>>,
}

/// Marks one epoch as held by the store and by every snapshot of it.
///
/// A snapshot clones the `Arc`, so the strong count of the lease is the number
/// of live views of that epoch in this process. The store keeps the strong
/// reference of its current epoch, and only a [`Weak`] of every epoch it left
/// behind: an epoch no snapshot holds any more can therefore be deleted
/// (INV-14, ADR-0043 point 4).
///
/// The epoch number lives here and nowhere else on the store: the store reads it
/// through its lease, so a publication cannot leave the two out of step.
#[derive(Debug)]
pub(crate) struct EpochLease {
    epoch: Epoch,
}

impl GenerationStore {
    /// Creates the index at `index_dir`: the epoch directory, its `segments/`
    /// and `manifests/` directories, `records.sqlite` with its schema, the
    /// manifest of generation 0, then `CURRENT_EPOCH`.
    ///
    /// `schema` is the schema of the index configuration (`index.json`,
    /// `docs/02-architecture.md`), which says which fields the filter
    /// structures index and how. A fresh index has no row, so its structures
    /// are the empty ones of that schema. `index.json` must already be present
    /// in `index_dir` — written by [`write_index_config`] — and its schema is
    /// the authority: a different `schema` is refused with
    /// [`ErrorCode::ImmutableSetting`], because the schema is frozen and a
    /// mismatch would build structures of another index. An index without a
    /// declared schema is refused with [`ErrorCode::InvalidRequest`], so no
    /// index exists whose filter structures no configuration describes.
    ///
    /// **An existing index is never overwritten.** When `CURRENT_EPOCH` is
    /// already present, creation stops with [`ErrorCode::InvalidRequest`]
    /// before writing anything: rewriting the pointer and `meta.epoch` would
    /// make a different epoch current while its rows stay in place, which no
    /// commit produced and no recovery expects. `invalid_request` is the code
    /// of `docs/05-api.md` for a request the store cannot carry out; the code
    /// `immutable_setting` is reserved for a frozen setting, which this is not.
    ///
    /// `CURRENT_EPOCH` is written last and atomically, and every directory it
    /// makes reachable is fsynced before it: `segments/`, the epoch directory
    /// and `epochs/`, in that order, then the index directory by
    /// `write_current_epoch`. A creation that returns has therefore persisted
    /// the whole chain, so a crash afterwards leaves a usable index; a crash
    /// before leaves no `CURRENT_EPOCH`, which [`GenerationStore::open`] refuses
    /// with [`ErrorCode::InvalidRequest`] instead of reading a half-created
    /// epoch.
    pub fn create(index_dir: &Path, epoch: Epoch, schema: &Schema) -> Result<Self, CribleError> {
        if let Some(present) = occupied_index_state(index_dir) {
            return Err(existing_index_error(index_dir, &present));
        }
        let config = declared_config(index_dir)?;
        check_declared_schema(&config, schema, index_dir)?;

        let epoch_dir = epoch_directory(index_dir, epoch);
        let segments = segments_directory(&epoch_dir);
        let manifests = manifests_directory(&epoch_dir);
        for directory in [&segments, &manifests] {
            fs::create_dir_all(directory)
                .map_err(|error| io_error("create a directory of the epoch", directory, &error))?;
        }

        let mut records = RecordStore::create(&epoch_dir.join(RECORDS_FILE_NAME))?;
        records.set_epoch(epoch)?;

        let lease = Arc::new(EpochLease { epoch });
        // A fresh index is at generation 0 and holds no passage, so its lexical
        // index has nothing to describe: `lexical/` appears with the first
        // generation that creates a passage (`docs/02`, step 4). A configuration
        // that disables the branch never creates it.
        let status = if config.lexical.enabled {
            // Generation 0 carries no passage: nothing to search, nothing to
            // build (`ADR-0088`, decision 2).
            LexicalStatus::Missing
        } else {
            LexicalStatus::Disabled
        };
        let snapshot = build_snapshot(
            &records,
            &epoch_dir,
            schema,
            PublishedLexical::without_reader(status),
            Arc::clone(&lease),
        )?;
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
            records,
            snapshot: ArcSwap::from(Arc::new(snapshot)),
            lexical: config.lexical,
            writable: true,
            lease,
            retained: Mutex::new(Vec::new()),
        })
    }

    /// Opens an existing index for a **write** and runs the recovery of INV-04,
    /// then publishes the snapshot of the committed generation.
    ///
    /// Recovery removes every `tmp-*` entry and every segment directory SQLite
    /// does not reference, rewrites the manifest of the current generation
    /// when it is missing, and rebuilds every `codes.bin` a segment is missing
    /// (`ADR-0069`, decision 1). It is idempotent: a second open changes
    /// nothing. A caller that only reads uses
    /// [`GenerationStore::open_read_only`], which does the same recovery without
    /// writing a `codes.bin`.
    ///
    /// `schema` is the schema of the index configuration, as for
    /// [`GenerationStore::create`]: the filter structures of the committed
    /// generation are rebuilt from `records.sqlite`, and the schema is what
    /// tells them which field to index and how. It is cross-checked against the
    /// schema `index.json` declares, so opening an index with another schema is
    /// refused with [`ErrorCode::ImmutableSetting`] instead of silently
    /// building different filter structures. An index directory without
    /// `index.json` is refused with [`ErrorCode::InvalidRequest`].
    ///
    /// A directory that holds no `CURRENT_EPOCH` is not an index of this build
    /// and is refused with [`ErrorCode::InvalidRequest`], before anything is
    /// removed from it.
    pub fn open(index_dir: &Path, schema: &Schema) -> Result<Self, CribleError> {
        Self::open_index(index_dir, schema, true)
    }

    /// Opens an existing index for a **read**: same recovery as
    /// [`GenerationStore::open`], but no derived **segment** file is written
    /// under the epoch.
    ///
    /// The only difference is the reconstruction of `codes.bin`
    /// (`ADR-0069`, decision 1), which only a write open runs: here a missing
    /// file stays missing, and `crate::codes::load_codes` quantizes the codes
    /// from the reference vectors into RAM. A read therefore never writes a
    /// segment file of the epoch it reads (`ADR-0056`; `ADR-0069`, decision 2),
    /// which is what `crible search` and `crible inspect` require. Recovery still
    /// removes the `tmp-*` entries and the unreferenced segments, and still
    /// rewrites a **missing manifest**, as it does for the write open (the
    /// manifest is a derived file of the generation, not of a segment): that
    /// behaviour is not changed here.
    pub fn open_read_only(index_dir: &Path, schema: &Schema) -> Result<Self, CribleError> {
        Self::open_index(index_dir, schema, false)
    }

    /// Shared body of [`GenerationStore::open`] and
    /// [`GenerationStore::open_read_only`]: `for_write` is the only difference
    /// between the two. It governs every reconstruction of a derived file the
    /// recovery runs — `codes.bin`, the `passage_change_generation` marker and
    /// the lexical index — so it names the mode of the open, not one of them.
    fn open_index(index_dir: &Path, schema: &Schema, for_write: bool) -> Result<Self, CribleError> {
        let epoch = read_current_epoch(index_dir)?;
        let config = declared_config(index_dir)?;
        check_declared_schema(&config, schema, index_dir)?;
        let epoch_dir = epoch_directory(index_dir, epoch);
        let mut records = RecordStore::open(&epoch_dir.join(RECORDS_FILE_NAME))?;

        let mut meta = records.meta()?;
        if meta.epoch != epoch {
            return Err(CribleError::new(
                ErrorCode::Internal,
                format!(
                    "{} names epoch {} but {} declares epoch {}",
                    index_dir.join(CURRENT_EPOCH_FILE_NAME).display(),
                    epoch.get(),
                    epoch_dir.join(RECORDS_FILE_NAME).display(),
                    meta.epoch.get()
                ),
            ));
        }

        recover(&records, index_dir, &epoch_dir)?;
        let observed = if for_write {
            rebuild_missing_codes(&records, &epoch_dir)?;
            // The marker an epoch written before P5 lacks is completed here,
            // before the recovery compares it with the lexical index: a
            // read-only open never writes it (ADR-0088, decision 4).
            records.ensure_passage_change_generation()?;
            meta = records.meta()?;
            // The lexical branch is a derived directory like `codes.bin`: it is
            // caught up with the committed generation on a **write** open only
            // (`docs/02`, "Reprise au démarrage"; ADR-0069, ADR-0087). A
            // read-only open writes nothing under `lexical/`, so it cannot take
            // the writer lock a live server holds nor delete files under its
            // mappings.
            recover_lexical(&records, &epoch_dir, &config.lexical, meta)?
        } else {
            lexical::observe(
                &epoch_dir,
                &config.lexical,
                meta.generation,
                meta.passage_change_generation,
            )?
        };
        let lease = Arc::new(EpochLease { epoch });
        let snapshot = build_snapshot(
            &records,
            &epoch_dir,
            schema,
            PublishedLexical::from_observation(observed),
            Arc::clone(&lease),
        )?;
        ensure_manifest(&epoch_dir, epoch, &snapshot)?;

        Ok(Self {
            index_dir: index_dir.to_path_buf(),
            records,
            snapshot: ArcSwap::from(Arc::new(snapshot)),
            lexical: config.lexical,
            writable: for_write,
            lease,
            retained: Mutex::new(Vec::new()),
        })
    }

    /// Epoch of the index, as `CURRENT_EPOCH` names it.
    #[must_use]
    pub fn epoch(&self) -> Epoch {
        self.lease.epoch
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
    /// every document must name G+1 as the generation that last wrote it, and
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
        self.commit_with_rows(|transaction, generation, next_iid| {
            let next_iid = write_generation_change(transaction, &change, generation, next_iid)?;
            Ok((next_iid, delta_of(change)))
        })
        .map(|committed| committed.generation)
    }

    /// Opens the transaction of a generation, lets `write` fill it, commits it
    /// once, then writes the manifest and publishes the new snapshot.
    ///
    /// The commit protocol is the one of [`GenerationStore::commit`]: the
    /// callback writes every row of the generation inside the single
    /// transaction, then `advance_meta` and the `COMMIT` end it (ADR-0016). A
    /// callback that returns an error, or panics, drops the transaction, which
    /// rolls it back, so nothing of the generation becomes visible (INV-03).
    ///
    /// The callback receives the transaction, the generation it must stamp on
    /// its rows (G+1), and the first free `iid`; it returns the first free `iid`
    /// after its rows, and the delta it wrote, which
    /// [`build_snapshot_after_commit`] derives the new filter structures from
    /// (ADR-0066).
    ///
    /// This is how an attach writes the `batches` row, the segments, the
    /// documents, the passages, the tombstones and the document removals in one
    /// transaction, which [`GenerationChange`] alone cannot express. The
    /// returned [`CommittedGeneration`] also carries the time spent in the
    /// transaction, in the manifest and in the snapshot.
    pub(crate) fn commit_with_rows<F>(
        &mut self,
        write: F,
    ) -> Result<CommittedGeneration, CribleError>
    where
        F: FnOnce(&Transaction<'_>, Generation, u32) -> Result<(u32, CommitDelta), CribleError>,
    {
        self.commit_with_rows_observed(write, &mut |_step| {})
    }

    /// Same as [`GenerationStore::commit_with_rows`], and calls `observer` at
    /// each [`CommitStep`] of the protocol.
    ///
    /// Only the attach of the `failpoints` build hands a real observer
    /// (`attach_with_observer`, `ingest_documents_with_observer`): it holds the
    /// job at a step while a test reads the index (INV-02). The observer runs on
    /// the caller's thread, so a test that blocks in it holds the commit
    /// protocol at that exact step. A production caller goes through
    /// [`GenerationStore::commit_with_rows`], which passes a no-op closure the
    /// compiler removes.
    pub(crate) fn commit_with_rows_observed<F, O>(
        &mut self,
        write: F,
        observer: &mut O,
    ) -> Result<CommittedGeneration, CribleError>
    where
        F: FnOnce(&Transaction<'_>, Generation, u32) -> Result<(u32, CommitDelta), CribleError>,
        O: FnMut(CommitStep) + ?Sized,
    {
        // ADR-0108, decision 1: before any commit, the pointer is the authority
        // on which epoch is current. A store left on E while `CURRENT_EPOCH`
        // names E+1 (a failed `adopt_epoch`) would write generations no reader
        // reopens, so the commit is refused without writing anything.
        let on_disk = read_current_epoch(&self.index_dir).map_err(|failure| {
            CribleError::new(
                ErrorCode::Internal,
                format!(
                    "cannot read CURRENT_EPOCH of {}: {}",
                    self.index_dir.display(),
                    failure.message()
                ),
            )
        })?;
        if on_disk != self.lease.epoch {
            tracing::error!(
                index = %self.index_dir.display(),
                pointer = on_disk.get(),
                epoch = self.lease.epoch.get(),
                "CURRENT_EPOCH names another epoch than the store; refusing to commit (ADR-0108)"
            );
            return Err(CribleError::new(
                ErrorCode::Internal,
                format!(
                    "CURRENT_EPOCH names epoch {} but this store holds epoch {}; a commit there \
                     would be lost, reopen the store first (ADR-0108)",
                    on_disk.get(),
                    self.lease.epoch.get()
                ),
            ));
        }
        let epoch_dir = epoch_directory(&self.index_dir, self.lease.epoch);

        let transaction_started = Instant::now();
        let transaction = self.records.begin()?;
        let meta = records::read_meta(&transaction)?;
        let generation = generation_after(meta.generation)?;

        let (next_iid, delta) = write(&transaction, generation, meta.next_iid)?;
        records::advance_meta(&transaction, generation, next_iid)?;
        // ADR-0088, decision 3: a generation that creates a live passage or
        // tombstones one moves the marker of the lexical branch in the same
        // transaction, so the commit point and the marker are atomic. A
        // metadata patch writes neither and leaves the marker where it was.
        let passage_change = changes_passage_text(&delta);
        if passage_change {
            records::mark_passage_change(&transaction, generation)?;
        }
        let marker = if passage_change {
            Some(generation)
        } else {
            meta.passage_change_generation
        };

        failpoint!("before_sqlite_commit");
        observer(CommitStep::BeforeSqliteCommit);
        transaction
            .commit()
            .map_err(|error| records::database_error("commit the generation", &error))?;
        let transaction_time = transaction_started.elapsed();
        failpoint!("after_sqlite_commit");
        observer(CommitStep::AfterSqliteCommit);

        let previous = self.snapshot.load_full();
        // Step 4 is measured on its own: the tantivy commit and its reader
        // reload are as expensive as the snapshot they precede, and a measure
        // must attribute them separately (ADR-0087). The step reports zero when
        // it does no work at all — a patch, whose delta touches no text — and
        // its real cost when it catches up a generation this process never
        // published (ADR-0088, review point 8).
        let lexical_started = Instant::now();
        let lexical = index_lexical(
            &self.records,
            &epoch_dir,
            &self.lexical,
            &previous,
            generation,
            marker,
            &delta,
        )?;
        let lexical_time = if lexical.worked {
            lexical_started.elapsed()
        } else {
            Duration::ZERO
        };

        let snapshot_started = Instant::now();
        let snapshot = build_snapshot_after_commit(
            &self.records,
            &epoch_dir,
            &previous,
            generation,
            &delta,
            PublishedLexical::from_step(lexical),
            Arc::clone(&self.lease),
        )?;
        let snapshot_time = snapshot_started.elapsed();

        let manifest_started = Instant::now();
        write_manifest(&epoch_dir, self.lease.epoch, &snapshot)?;
        let manifest_time = manifest_started.elapsed();
        failpoint!("after_manifest");

        self.snapshot.store(Arc::new(snapshot));
        failpoint!("after_publish");
        Ok(CommittedGeneration {
            generation,
            timings: CommitTimings {
                transaction: transaction_time,
                lexical: lexical_time,
                manifest: manifest_time,
                snapshot: snapshot_time,
            },
        })
    }

    /// Runs `read` on the writable connection of the index, outside any
    /// transaction.
    ///
    /// The store is the only writer of its index, so a read here sees a state
    /// no other writer can change until the next call that takes `&mut self`.
    /// The attach uses it to decide its plan before the segment and the
    /// transaction (`docs/02-architecture.md`, "Attacher un lot").
    pub(crate) fn with_connection<T>(
        &self,
        read: impl FnOnce(&Connection) -> Result<T, CribleError>,
    ) -> Result<T, CribleError> {
        read(self.records.connection())
    }

    /// Reads the `batches` row of `batch_id`, or `None` when it has never been
    /// applied (INV-10).
    pub(crate) fn applied_batch(
        &self,
        batch_id: &str,
    ) -> Result<Option<records::StoredBatch>, CribleError> {
        self.records.batch(batch_id)
    }

    /// Directory of the current epoch: `<index>/epochs/<epoch>`.
    #[must_use]
    pub fn epoch_directory(&self) -> PathBuf {
        epoch_directory(&self.index_dir, self.lease.epoch)
    }

    /// Path of the reference database of the current epoch.
    ///
    /// The search opens its read-only payload connections on this file, at one
    /// fixed path (ADR-0043), so it never recomputes an epoch directory per
    /// query.
    #[must_use]
    pub fn records_path(&self) -> PathBuf {
        self.epoch_directory().join(RECORDS_FILE_NAME)
    }

    /// Directory of the segments of the current epoch.
    #[must_use]
    pub fn segments_directory(&self) -> PathBuf {
        segments_directory(&self.epoch_directory())
    }

    /// Root directory of the index: `<index>`.
    ///
    /// The server keeps it to reopen the writer after a failed compaction
    /// (ADR-0108) and to let a route read the reference database of the epoch a
    /// snapshot holds (ADR-0107).
    #[must_use]
    pub fn index_dir(&self) -> &Path {
        &self.index_dir
    }

    /// Borrows the reference state of the current epoch.
    ///
    /// The compaction reads the live passages through this connection, which is
    /// the single writer of the index, so no commit can change the rows under
    /// it.
    pub(crate) fn records(&self) -> &RecordStore {
        &self.records
    }

    /// Epochs an earlier snapshot of this store still holds.
    ///
    /// The store only keeps a [`Weak`] reference of every epoch it left behind,
    /// so an epoch disappears from this list as soon as its last snapshot is
    /// dropped: [`crate::compact::reclaim_epochs`] can then delete it (INV-14,
    /// ADR-0043 point 4).
    pub(crate) fn held_epochs(&self) -> Vec<Epoch> {
        let mut retained = self.retained.lock().unwrap_or_else(PoisonError::into_inner);
        retained.retain(|lease| lease.strong_count() > 0);
        retained
            .iter()
            .filter_map(Weak::upgrade)
            .map(|lease| lease.epoch)
            .collect()
    }

    /// Publishes `epoch` as the current epoch of the index, after the switch of
    /// `CURRENT_EPOCH` by a compaction.
    ///
    /// The caller passes the reference state it has just opened at the new
    /// epoch, whose `meta` carries the generation the compaction committed. The
    /// snapshot is rebuilt from that state, its manifest is written, then the
    /// store switches to it and releases its strong reference on the previous
    /// epoch, which only the snapshots the readers still hold keep alive
    /// (INV-14).
    ///
    /// An error before the publication leaves the store on its previous epoch,
    /// although `CURRENT_EPOCH` already names the new one: the next
    /// [`GenerationStore::open`] publishes the committed epoch (INV-04).
    pub(crate) fn adopt_epoch(
        &mut self,
        epoch: Epoch,
        records: RecordStore,
        schema: &Schema,
    ) -> Result<(), CribleError> {
        let epoch_dir = epoch_directory(&self.index_dir, epoch);
        let lease = Arc::new(EpochLease { epoch });
        // The compaction writes the lexical index of the new epoch itself
        // (`RECONSTRUIRE-LEXICAL`); the reader is opened from what the epoch
        // holds, so a snapshot never publishes a branch the disk contradicts.
        let meta = records.meta()?;
        let observed = lexical::observe(
            &epoch_dir,
            &self.lexical,
            meta.generation,
            meta.passage_change_generation,
        )?;
        let snapshot = build_snapshot(
            &records,
            &epoch_dir,
            schema,
            PublishedLexical::from_observation(observed),
            Arc::clone(&lease),
        )?;
        write_manifest(&epoch_dir, epoch, &snapshot)?;

        let previous = std::mem::replace(&mut self.lease, lease);
        self.retained
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::downgrade(&previous));
        drop(previous);

        self.records = records;
        self.snapshot.store(Arc::new(snapshot));
        Ok(())
    }

    /// State of the published generation, as `crible inspect` and the index
    /// state of the API (`docs/05-api.md`) report it.
    ///
    /// The counts come from `records.sqlite`, the reference state, while the
    /// filter memory comes from the published snapshot. The store is the only
    /// writer of its index, so the two describe the same generation.
    pub fn state(&self) -> Result<IndexState, CribleError> {
        let snapshot = self.snapshot();
        let counts = self.records.counts()?;
        let live_passages = snapshot.filters().live().len();
        let dead_passages = counts.passages.checked_sub(live_passages).ok_or_else(|| {
            CribleError::new(
                ErrorCode::Internal,
                "the live bitmap holds more passages than the reference state",
            )
        })?;
        Ok(IndexState {
            epoch: self.lease.epoch,
            generation: snapshot.generation(),
            documents: counts.documents,
            live_passages,
            dead_passages,
            segments: snapshot.segments().to_vec(),
            filter_memory: snapshot.filter_memory(),
            codes_memory: snapshot.codes_memory(),
        })
    }

    /// The last batch the reference state applied, or `None` when the epoch
    /// holds no row of `batches`.
    ///
    /// ADR-0105, decision 3: `GET /v1/indexes/{index}` reports that row as
    /// `last_batch`. The "last" row is the one with the greatest `applied_at`,
    /// then the greatest `batch_id`; the compaction copies the rows verbatim, so
    /// the answer survives an epoch change.
    ///
    /// # Errors
    ///
    /// The errors of the SQLite query.
    pub fn last_batch(&self) -> Result<Option<LastBatch>, CribleError> {
        self.records
            .connection()
            .query_row(
                "SELECT batch_id, applied_at, generation FROM batches \
                 ORDER BY applied_at DESC, batch_id DESC LIMIT 1",
                [],
                |row| {
                    let generation: i64 = row.get(2)?;
                    Ok(LastBatch {
                        batch_id: row.get(0)?,
                        applied_at: row.get(1)?,
                        generation: u64::try_from(generation)
                            .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(2, generation))?,
                    })
                },
            )
            .optional()
            .map_err(|error| records::database_error("read the last applied batch", &error))
    }

    /// Builds the snapshot a new configuration calls for, **without** publishing
    /// it.
    ///
    /// This is the second half of the `PATCH` job of ADR-0105, decision 4: the
    /// caller has already written `index.json` and validated the patch, so this
    /// function rebuilds what derives from the configuration and returns the
    /// snapshot describing the result. The caller publishes that snapshot
    /// together with the configuration, under one lock (decision 4, "publie
    /// ensemble"), and then calls [`GenerationStore::adopt`] so the next job of
    /// the writer builds on it.
    ///
    /// - the lexical branch is rebuilt when the analysis settings
    ///   (`lexical.enabled`, `lexical.language`, `lexical.positions`) changed and
    ///   the branch is enabled after the patch;
    /// - the filter structures are rebuilt from `records.sqlite` with the new
    ///   schema, which covers `filterable`, `returned` and an added field;
    /// - segments, codes and the generation are unchanged.
    ///
    /// The store must be open for writing, because the lexical rebuild writes
    /// under the epoch (`ADR-0069`, `ADR-0087`).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] on a read-only store, and the errors of the
    /// rebuild, the observation and the snapshot otherwise.
    pub fn reconfigure(&mut self, config: &IndexConfig) -> Result<Arc<Snapshot>, CribleError> {
        if !self.writable {
            return Err(CribleError::new(
                ErrorCode::Internal,
                "applying a configuration rebuilds derived files under the epoch, so it needs a \
                 store opened for writing (ADR-0105)",
            ));
        }
        let lexical_changed = self.lexical != config.lexical;
        self.lexical = config.lexical.clone();
        let meta = self.records.meta()?;
        let epoch_dir = epoch_directory(&self.index_dir, self.lease.epoch);
        let never_carried_a_passage =
            matches!(meta.passage_change_generation.map(Generation::get), Some(0));
        if lexical_changed && config.lexical.enabled && !never_carried_a_passage {
            drop(rebuild_lexical_index(
                &self.records,
                &epoch_dir,
                &config.lexical,
                meta.generation,
            )?);
        }
        let observed = lexical::observe(
            &epoch_dir,
            &config.lexical,
            meta.generation,
            meta.passage_change_generation,
        )?;
        let snapshot = build_snapshot(
            &self.records,
            &epoch_dir,
            &config.schema,
            PublishedLexical::from_observation(observed),
            Arc::clone(&self.lease),
        )?;
        Ok(Arc::new(snapshot))
    }

    /// Publishes a snapshot [`GenerationStore::reconfigure`] built, once its
    /// caller has published it beside the configuration (ADR-0105, decision 4).
    ///
    /// The next job of the writer reads the store snapshot as its base, so a
    /// rebuilt filter structure must become the store's own before the job
    /// returns. Nothing else reads it: the routes of an index read the triplet
    /// the server publishes, never the store.
    pub fn adopt(&self, snapshot: Arc<Snapshot>) {
        self.snapshot.store(snapshot);
    }
}

/// One row of `batches`, as the index state of ADR-0105 reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastBatch {
    /// Identifier of the batch.
    pub batch_id: String,
    /// Time SQLite wrote when the batch was applied, RFC 3339 UTC.
    pub applied_at: String,
    /// Generation the batch published.
    pub generation: u64,
}

/// State of one published generation, for inspection.
///
/// Every count is taken from the reference state; the segments and the filter
/// memory describe the snapshot the store published. The dead ratio is a simple
/// `dead / (live + dead)` and is not stored.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexState {
    /// Epoch directory holding the reference state.
    pub epoch: Epoch,
    /// Published generation.
    pub generation: Generation,
    /// Documents the reference state holds.
    pub documents: u64,
    /// Passages alive at this generation.
    pub live_passages: u64,
    /// Passages tombstoned at this generation or an earlier one.
    pub dead_passages: u64,
    /// Segments the generation references, in `iid_start` order.
    pub segments: Vec<SegmentMeta>,
    /// Estimated heap size of the filter structures.
    pub filter_memory: FilterMemory,
    /// Bytes of RAM the binary codes of the generation hold, over every segment
    /// of the generation (`ADR-0069`).
    pub codes_memory: u64,
}

impl IndexState {
    /// Ratio of dead passages among all the passages of the epoch, or 0 when
    /// the epoch holds none.
    #[must_use]
    pub fn dead_ratio(&self) -> f64 {
        let total = self.live_passages.saturating_add(self.dead_passages);
        if total == 0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        let ratio = self.dead_passages as f64 / total as f64;
        ratio
    }
}

/// Reads the configuration `index.json` declares.
///
/// An index always carries `index.json`: [`GenerationStore::create`] refuses an
/// index directory without it, and [`write_index_config`] is the only way the
/// store writes one. A directory without it is therefore not a complete index,
/// and its caller's schema must not become the only authority, or the filter
/// structures could be built from a schema no index declares. The lexical branch
/// is derived from the same file: the store takes `lexical.enabled`, `language`
/// and `positions` from it, not from a caller.
fn declared_config(index_dir: &Path) -> Result<IndexConfig, CribleError> {
    let path = index_dir.join(INDEX_CONFIG_FILE_NAME);
    let raw = fs::read_to_string(&path).map_err(|error| {
        CribleError::new(
            ErrorCode::InvalidRequest,
            format!(
                "cannot read the index configuration {}: {error}; a Crible index always \
                 declares its schema in {INDEX_CONFIG_FILE_NAME}",
                path.display()
            ),
        )
    })?;
    let config = IndexConfig::from_json_str(&raw).map_err(|error| {
        CribleError::new(
            error.code(),
            format!("{}: {}", path.display(), error.message()),
        )
    })?;
    Ok(config)
}

/// Refuses a `schema` that differs from the one `index.json` declares.
///
/// The schema is frozen (INV-16) and decides which filter structures exist. A
/// store built with another schema would silently build structures no index
/// declares, so the two are cross-checked at creation and at every open.
fn check_declared_schema(
    config: &IndexConfig,
    schema: &Schema,
    index_dir: &Path,
) -> Result<(), CribleError> {
    if &config.schema == schema {
        return Ok(());
    }
    Err(CribleError::new(
        ErrorCode::ImmutableSetting,
        format!(
            "the schema passed to the store differs from the schema declared by {}; the \
             schema of an index is frozen, and its filter structures are built from the \
             declared one",
            index_dir.join(INDEX_CONFIG_FILE_NAME).display()
        ),
    ))
}

/// Runs the lexical recovery of `docs/02-architecture.md` ("Reprise au
/// démarrage") for the generation `records.sqlite` holds, then reports the state
/// it leaves the index in.
///
/// It is a **write**-open step, like every reconstruction of a derived file
/// (ADR-0069, ADR-0087): the caller runs it only when it opened the index for
/// writing, so no read-only command takes the tantivy writer lock or removes
/// files under a live server's mappings.
///
/// `Gt` is the payload of the last tantivy commit of the epoch, `M` the
/// `meta.passage_change_generation` marker, `G` the generation the reference
/// state committed (ADR-0088, decision 2):
///
/// - `M ≤ Gt ≤ G`: the index is up to date, so nothing is replayed — the case of
///   a metadata patch, whose generation changes no text;
/// - `Gt < G` with `M > Gt`: the passages created after `Gt` are indexed and
///   those tombstoned after it are removed, then one commit carries `G`;
/// - `Gt > G`: no commit produces that state — a restored epoch without its
///   derived directory, or a directory copied by hand — so the whole lexical
///   index is rebuilt from `records.sqlite`;
/// - `M > G`, a payload that is not a generation, a `meta.json`, a segment or a
///   directory tantivy does not open: the state is undecidable, so the index is
///   rebuilt as for `Gt > G`, with the case and the original message logged
///   (ADR-0088, decisions 5 and 6).
///
/// A configuration that disables the branch does nothing at all: a `lexical/`
/// directory an earlier configuration left behind is never read, never written
/// and never removed. An epoch that never carried a passage is not caught up
/// either: `lexical/` appears with the first passage (`docs/02`, step 4).
fn recover_lexical(
    records: &RecordStore,
    epoch_dir: &Path,
    config: &LexicalConfig,
    meta: records::Meta,
) -> Result<LexicalObservation, CribleError> {
    let observe = |meta: records::Meta| {
        lexical::observe(
            epoch_dir,
            config,
            meta.generation,
            meta.passage_change_generation,
        )
    };
    let observed = observe(meta)?;
    match observed.status {
        // Up to date, nothing to search, or no branch at all.
        LexicalStatus::Ready { .. } | LexicalStatus::Missing | LexicalStatus::Disabled => {
            Ok(observed)
        }
        LexicalStatus::Behind { committed } => {
            let since = committed.unwrap_or_else(|| Generation::new(0));
            catch_up_lexical(records, epoch_dir, config, meta.generation, since)?;
            observe(meta)
        }
        LexicalStatus::Ahead { committed } => {
            let case = format!(
                "the payload {} is ahead of the committed generation {}",
                committed.get(),
                meta.generation.get()
            );
            rebuild(records, epoch_dir, config, meta.generation, &case)?;
            observe(meta)
        }
        LexicalStatus::NotCertified { committed } => {
            // Cannot happen once the marker is completed on the write open: the
            // arm exists so a state this build does not expect is repaired
            // rather than published.
            let payload = committed.map_or_else(
                || "none".to_owned(),
                |committed| committed.get().to_string(),
            );
            let case = format!(
                "the payload {payload} is not certified for the committed generation {}",
                meta.generation.get()
            );
            rebuild(records, epoch_dir, config, meta.generation, &case)?;
            observe(meta)
        }
        LexicalStatus::Unreadable => {
            rebuild(
                records,
                epoch_dir,
                config,
                meta.generation,
                "the lexical index is unreadable",
            )?;
            observe(meta)
        }
        LexicalStatus::OtherChain => {
            rebuild(
                records,
                epoch_dir,
                config,
                meta.generation,
                "the lexical index was built with another analysis chain",
            )?;
            observe(meta)
        }
    }
}

/// Rebuilds the whole lexical index of an epoch from `records.sqlite`.
///
/// The directory is derived, so it is removed before the rebuild rather than
/// appended to: a lexical index ahead of the reference state holds documents
/// no live passage backs, and an unreadable one holds nothing this build can
/// trust. The rebuild is the repair of the restart (`ADR-0088`, decisions 2 and
/// 6); a rebuild decided outside it is [`rebuild_lexical`] or the compaction.
///
/// The commit carries `generation` as its payload, so it is at least the
/// marker `meta.passage_change_generation` of the epoch it describes
/// (`ADR-0088`, decision 8): the state is then [`LexicalStatus::Ready`], and a
/// reader opened from what the epoch holds serves it.
pub(crate) fn rebuild_lexical_index(
    records: &RecordStore,
    epoch_dir: &Path,
    config: &LexicalConfig,
    generation: Generation,
) -> Result<LexicalIndex, CribleError> {
    lexical::remove_index(epoch_dir)?;
    let mut writer = lexical::writer(epoch_dir, config)?;
    records::for_each_live_passage(records.connection(), |iid, text| writer.add(iid, text))?;
    writer.commit(generation)?;
    writer.reader()
}

/// Logs the case that makes the restart rebuild the lexical index, then rebuilds
/// it (`ADR-0088`, decisions 5 and 6).
///
/// The log names the epoch through its directory and the case. For an unreadable
/// state, [`lexical::observe`] has already logged the message tantivy produced,
/// so the two lines together say where, why and what the failure said.
fn rebuild(
    records: &RecordStore,
    epoch_dir: &Path,
    config: &LexicalConfig,
    generation: Generation,
    case: &str,
) -> Result<LexicalIndex, CribleError> {
    tracing::warn!(
        lexical_index = %lexical::index_directory(epoch_dir).display(),
        case,
        "rebuilding the lexical index from records.sqlite; no generation, no manifest and no \
         visible state change (ADR-0088)"
    );
    rebuild_lexical_index(records, epoch_dir, config, generation)
}

/// Outcome of [`rebuild_lexical`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RebuildOutcome {
    /// Generation the reference state holds; the job publishes no new one.
    pub generation: Generation,
    /// Whether the job rebuilt the lexical index.
    ///
    /// `false` for an epoch whose `passage_change_generation` marker is `0`:
    /// it never carried a passage, so `lexical/` stays absent (`docs/02`,
    /// step 4).
    pub rebuilt: bool,
}

/// The job of `RECONSTRUIRE-LEXICAL`: rebuilds the lexical index of the
/// published epoch from the texts `records.sqlite` stores, whatever state the
/// index was in, and leaves the generations untouched.
///
/// A change of `lexical.language` or `lexical.positions` changes no text, so the
/// generation does not move. The analysis chain is recorded in the payload of
/// the commit, and a write open compares it with the configuration and rebuilds
/// before serving (`ADR-0106`, decisions 4 and 5); the server also queues this
/// job when a `PATCH` changes one of those two settings (`docs/04`, "Ce qui est
/// figé, ce qui est réglable"), and `crible rebuild-lexical` runs it on demand.
///
/// The store must be open for **writing**: the rebuild removes `lexical/` and
/// commits a new tantivy index, which is a write under the epoch (`ADR-0069`,
/// `ADR-0087`). The payload of the commit is the committed generation, so the
/// criterion `M ≤ Gt ≤ G` holds afterwards (`ADR-0088`). The snapshot the store
/// publishes next carries a reader opened **after** the rebuild, so a query
/// started from then on reads the new index. A snapshot taken before the call
/// keeps reading the files it opened: the removal leaves them reachable to that
/// reader alone, and only under POSIX.
///
/// It creates no generation, no epoch and no manifest: the reference state is
/// the authority, and the rebuild only restores what derives from it
/// (`docs/02`, "Reprise au démarrage"). An epoch that never carried a passage has
/// nothing to rebuild, and gets no `lexical/` directory either (`docs/02`, step
/// 4); an index that disables the branch is refused rather than served an index
/// nothing would read. The returned [`RebuildOutcome`] also tells whether the
/// job rebuilt the index.
pub fn rebuild_lexical(store: &mut GenerationStore) -> Result<RebuildOutcome, CribleError> {
    if !store.writable {
        return Err(CribleError::new(
            ErrorCode::Internal,
            "the rebuild of the lexical index removes and commits files under the epoch, so it \
             needs a store opened for writing (`GenerationStore::open`), not one a read command \
             opened (ADR-0069, ADR-0087)"
                .to_owned(),
        ));
    }
    let config = store.lexical.clone();
    if !config.enabled {
        return Err(CribleError::new(
            ErrorCode::InvalidRequest,
            "the index configuration disables the lexical branch (`lexical.enabled`), so there \
             is no lexical index to rebuild; enable the branch before rebuilding it"
                .to_owned(),
        ));
    }
    let meta = store.records.meta()?;
    // `0` is the marker of an epoch that never created or killed a passage
    // (`ADR-0088`, decision 3): a reader of it renders nothing, and `docs/02`
    // makes `lexical/` appear with the first passage.
    if matches!(meta.passage_change_generation.map(Generation::get), Some(0)) {
        return Ok(RebuildOutcome {
            generation: meta.generation,
            rebuilt: false,
        });
    }

    let epoch_dir = epoch_directory(&store.index_dir, store.lease.epoch);
    // The rebuild opens a reader over the commit it just wrote; the job
    // re-observes the state instead of asserting it, so the snapshot publishes
    // what the disk and the marker certify (`ADR-0088`, decision 5).
    drop(rebuild_lexical_index(
        &store.records,
        &epoch_dir,
        &config,
        meta.generation,
    )?);
    let observed = lexical::observe(
        &epoch_dir,
        &config,
        meta.generation,
        meta.passage_change_generation,
    )?;
    publish_lexical_reader(
        store,
        PublishedLexical::from_observation(observed),
        meta.generation,
    );
    Ok(RebuildOutcome {
        generation: meta.generation,
        rebuilt: true,
    })
}

/// Replaces the lexical branch of the published snapshot by `published`, keeping
/// every other structure of the snapshot.
///
/// The rebuild of [`rebuild_lexical`] changes the derived index alone: the
/// segments, the codes and the filter structures describe the same epoch and the
/// same `generation`, so the snapshot is rebuilt by sharing them rather than by
/// reading them again (`docs/02`, "Concurrence"; `ADR-0043`).
fn publish_lexical_reader(
    store: &GenerationStore,
    published: PublishedLexical,
    generation: Generation,
) {
    let previous = store.snapshot();
    let snapshot = Snapshot {
        generation,
        segments: previous.segments.clone(),
        codes: previous.codes.clone(),
        filters: previous.filters.clone(),
        lexical: published.reader,
        lexical_status: published.status,
        lease: Arc::clone(&previous.lease),
    };
    store.snapshot.store(Arc::new(snapshot));
}

/// Replays into the lexical index of `epoch_dir` the generations after `since`,
/// then commits `generation` and returns a reader over it.
/// `since` must be behind `generation`: the caller either saw a payload behind
/// the reference state, or holds no lexical commit at all. Only the passages
/// SQLite still holds alive are indexed, and the tombstones applied after
/// `since` remove what the index may already carry.
fn catch_up_lexical(
    records: &RecordStore,
    epoch_dir: &Path,
    config: &LexicalConfig,
    generation: Generation,
    since: Generation,
) -> Result<LexicalIndex, CribleError> {
    let mut writer = lexical::writer(epoch_dir, config)?;
    records::for_each_passage_created_after(records.connection(), since, |iid, text| {
        writer.add(iid, text)
    })?;
    for iid in records::iids_dead_after(records.connection(), since)? {
        writer.delete(iid);
    }
    writer.commit(generation)?;
    writer.reader()
}

/// The lexical branch a snapshot publishes: the reader of the generation, when
/// the state allows one, and the state itself (`ADR-0088`, decision 5).
///
/// The two travel together because a reader is only ever published with the
/// state that certifies it: a snapshot that exposed a reader under any other
/// status would serve a lexical index of another generation (INV-02, INV-03).
struct PublishedLexical {
    /// Reader over the commit, absent unless `status` is
    /// [`LexicalStatus::Ready`].
    reader: Option<Arc<LexicalIndex>>,
    /// State of the lexical index of the epoch.
    status: LexicalStatus,
}

impl PublishedLexical {
    /// Publishes the reader an observation opened, if any.
    fn from_observation(observation: LexicalObservation) -> Self {
        Self {
            reader: observation.index.map(Arc::new),
            status: observation.status,
        }
    }

    /// Publishes no reader under `status`.
    fn without_reader(status: LexicalStatus) -> Self {
        Self {
            reader: None,
            status,
        }
    }

    /// Publishes what step 4 returned.
    fn from_step(step: LexicalStep) -> Self {
        Self {
            reader: step.index,
            status: step.status,
        }
    }
}

/// The outcome of step 4 for one generation: the lexical branch the snapshot
/// publishes, the state it was observed in, and whether the step did any work.
struct LexicalStep {
    /// Reader of the lexical branch, or `None` when the snapshot has none.
    index: Option<Arc<LexicalIndex>>,
    /// State of the lexical index once the step returned.
    status: LexicalStatus,
    /// Whether the step did work: a tantivy commit, a rebuild, or the catch-up
    /// of a generation this process never published. The attach measures the
    /// duration of the step only when it is true, so the `lexical` field of
    /// `AttachDurations` reports zero for a metadata patch, which does no work
    /// at all (`ADR-0087`, `ADR-0088`).
    worked: bool,
}

impl LexicalStep {
    /// A step that did no work and publishes `index` with `status`.
    fn reused(index: Option<Arc<LexicalIndex>>, status: LexicalStatus) -> Self {
        Self {
            index,
            status,
            worked: false,
        }
    }
}

/// Step 4 of `docs/02-architecture.md`: the passages a generation created and
/// the ones it tombstoned reach tantivy, then a reader over the commit becomes
/// the lexical branch of the snapshot (`docs/03`, "Branche lexicale").
///
/// The step does nothing when the configuration disables the branch, and when
/// the generation changes no passage text: a metadata patch, and an attach that
/// only rewrites document rows, walk steps 3, 5 and 6, and `docs/02` excludes
/// step 4 for both. The snapshot then keeps the reader and the state of the
/// previous generation: the lexical index is still up to date for the new
/// generation, because `M ≤ Gt` and nothing after `Gt` changed a text
/// (ADR-0088, decision 2).
///
/// The commit of the new generation has already happened in SQLite, so an error
/// here leaves the reference state one generation ahead of the lexical index:
/// the next [`GenerationStore::open`] catches the lexical index up
/// ([`recover_lexical`]), which is the `M > Gt` branch of the restart.
fn index_lexical(
    records: &RecordStore,
    epoch_dir: &Path,
    config: &LexicalConfig,
    previous: &Snapshot,
    generation: Generation,
    marker: Option<Generation>,
    delta: &CommitDelta,
) -> Result<LexicalStep, CribleError> {
    if !config.enabled {
        return Ok(LexicalStep::reused(None, LexicalStatus::Disabled));
    }
    // The incremental branch replays `delta` alone, so it is sound only when the
    // published snapshot was itself certified: `Ready` means the epoch's lexical
    // index was up to date at the previous generation, and `Missing` that the
    // epoch never carried a passage, so an empty index plus `delta` is complete.
    // Any other state — an index behind, ahead, not certified or unreadable, as
    // a read-only open or the epoch a compaction adopted publishes — must go
    // through `observe` and its replay: adding `delta` to an index that already
    // missed generations, then committing `G+1`, would certify an incomplete
    // index as up to date (ADR-0088, decisions 2 and 6; INV-04, INV-05).
    let certified = matches!(
        previous.lexical_status(),
        LexicalStatus::Ready { .. } | LexicalStatus::Missing
    );
    let consecutive =
        certified && previous.generation().get().checked_add(1) == Some(generation.get());
    if consecutive && !changes_passage_text(delta) {
        return Ok(LexicalStep::reused(
            previous.shared_lexical().cloned(),
            previous.lexical_status(),
        ));
    }
    if consecutive {
        let mut writer = lexical::writer(epoch_dir, config)?;
        for passage in delta.created.iter().filter(|passage| !passage.dead) {
            writer.add(passage.iid, &passage.text)?;
        }
        for iid in &delta.tombstones {
            writer.delete(*iid);
        }
        writer.commit(generation)?;
        let reader = writer.reader()?;
        // Step 4 is complete: tantivy committed `generation` and a reader
        // reloaded the committed state. The snapshot is not published yet
        // (`after_publish` is step 6).
        failpoint!("after_lexical_commit");
        return Ok(LexicalStep {
            index: Some(Arc::new(reader)),
            status: LexicalStatus::Ready {
                committed: generation,
            },
            worked: true,
        });
    }

    // A generation this process committed but never published (a manifest that
    // could not be written, a crash at a later step): `delta` leaves its rows
    // out, as it does for the filter structures, so the state of the epoch is
    // observed and the missing generations are replayed from `records.sqlite`
    // instead.
    let observed = lexical::observe(epoch_dir, config, generation, marker)?;
    match observed.status {
        // Nothing was missed: the lexical index already describes this
        // generation, and only the publication of the snapshot is missing. The
        // catch-up this step used to run committed an empty generation for
        // nothing (ADR-0088, decision 2).
        LexicalStatus::Ready { .. } => Ok(LexicalStep {
            index: observed.index.map(Arc::new),
            status: observed.status,
            worked: true,
        }),
        LexicalStatus::Behind { committed } => {
            let since = committed.unwrap_or_else(|| Generation::new(0));
            let reader = catch_up_lexical(records, epoch_dir, config, generation, since)?;
            failpoint!("after_lexical_commit");
            Ok(LexicalStep {
                index: Some(Arc::new(reader)),
                status: LexicalStatus::Ready {
                    committed: generation,
                },
                worked: true,
            })
        }
        LexicalStatus::Ahead { committed } => {
            let case = format!(
                "the payload {} is ahead of the committed generation {}",
                committed.get(),
                generation.get()
            );
            let reader = rebuild(records, epoch_dir, config, generation, &case)?;
            failpoint!("after_lexical_commit");
            Ok(LexicalStep {
                index: Some(Arc::new(reader)),
                status: LexicalStatus::Ready {
                    committed: generation,
                },
                worked: true,
            })
        }
        LexicalStatus::Missing | LexicalStatus::NotCertified { .. } | LexicalStatus::Unreadable => {
            let reader = rebuild(
                records,
                epoch_dir,
                config,
                generation,
                "the lexical index is missing, unreadable or not certified",
            )?;
            failpoint!("after_lexical_commit");
            Ok(LexicalStep {
                index: Some(Arc::new(reader)),
                status: LexicalStatus::Ready {
                    committed: generation,
                },
                worked: true,
            })
        }
        LexicalStatus::OtherChain => {
            let reader = rebuild(
                records,
                epoch_dir,
                config,
                generation,
                "the lexical index was built with another analysis chain",
            )?;
            failpoint!("after_lexical_commit");
            Ok(LexicalStep {
                index: Some(Arc::new(reader)),
                status: LexicalStatus::Ready {
                    committed: generation,
                },
                worked: true,
            })
        }
        LexicalStatus::Disabled => Ok(LexicalStep::reused(None, LexicalStatus::Disabled)),
    }
}

/// Whether `delta` writes a passage text the lexical index must follow: a
/// created passage still alive, or a tombstone.
///
/// A metadata patch changes neither, so it neither runs step 4 nor moves
/// `meta.passage_change_generation`; an attach that only rewrites `documents`
/// rows is in the same case. The predicate drives both writes, so the marker and
/// the tantivy commit can never disagree about whether a generation changed the
/// text (`docs/02`, step 3 and step 4; ADR-0088, decisions 1 and 3).
fn changes_passage_text(delta: &CommitDelta) -> bool {
    !delta.tombstones.is_empty() || delta.created.iter().any(|passage| !passage.dead)
}

/// Writes one [`GenerationChange`] inside the transaction of its generation,
/// and returns the first free `iid` after its passages.
fn write_generation_change(
    transaction: &Transaction<'_>,
    change: &GenerationChange,
    generation: Generation,
    next_iid: u32,
) -> Result<u32, CribleError> {
    let GenerationChange {
        segments,
        documents,
        passages,
        deletions,
    } = change;

    let mut next_iid = next_iid;
    // One compressor for the passages of this generation change (ADR-0048).
    let mut compressor = records::TextCompressor::new()?;
    for segment in segments {
        check_generation_matches(
            "segment",
            &segment.segment_id,
            segment.generation,
            generation,
        )?;
        records::insert_segment(transaction, segment)?;
    }
    for document in documents {
        check_generation_matches(
            "document",
            &document.doc_id,
            document.updated_generation,
            generation,
        )?;
        records::insert_document(transaction, document)?;
    }
    for passage in passages {
        check_generation_matches(
            "passage",
            &passage.doc_id,
            passage.created_generation,
            generation,
        )?;
        records::insert_passage(transaction, passage, &mut compressor)?;
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
    for iid in deletions {
        records::tombstone_passage(transaction, *iid, generation)?;
    }
    Ok(next_iid)
}

/// The delta one [`GenerationChange`] writes into `passages` and `documents`, in
/// the form [`FilterIndex::after_commit`] consumes (ADR-0066).
///
/// A change never removes a document row: removal belongs to an attach
/// (ADR-0041), whose plan feeds `removed_documents` itself.
fn delta_of(change: GenerationChange) -> CommitDelta {
    CommitDelta {
        created: change
            .passages
            .into_iter()
            .map(CreatedPassage::from)
            .collect(),
        rewritten_documents: change
            .documents
            .into_iter()
            .map(|document| document.doc_id)
            .collect(),
        tombstones: change.deletions,
        removed_documents: Vec::new(),
    }
}

/// Directory of one epoch: `<index>/epochs/<epoch>`.
pub(crate) fn epoch_directory(index_dir: &Path, epoch: Epoch) -> PathBuf {
    epochs_directory(index_dir).join(epoch.get().to_string())
}

/// Path of the reference database of one epoch of an index.
///
/// A reader of a document opens it on the epoch of the snapshot it holds, never
/// on the current state (ADR-0107, decision 6).
#[must_use]
pub fn epoch_records_path(index_dir: &Path, epoch: Epoch) -> PathBuf {
    epoch_directory(index_dir, epoch).join(RECORDS_FILE_NAME)
}

/// Directory holding every epoch of an index: `<index>/epochs`.
pub(crate) fn epochs_directory(index_dir: &Path) -> PathBuf {
    index_dir.join(EPOCHS_DIR_NAME)
}

/// Directory of the segments of one epoch of an index.
///
/// The search route opens its searcher on the epoch of the snapshot it holds,
/// not on the epoch the store currently publishes, so it needs the path for an
/// explicit epoch (ADR-0107, decision 6; ADR-0109).
#[must_use]
pub fn epoch_segments_directory(index_dir: &Path, epoch: Epoch) -> PathBuf {
    segments_directory(&epoch_directory(index_dir, epoch))
}

/// Directory of the segments of an epoch.
pub(crate) fn segments_directory(epoch_dir: &Path) -> PathBuf {
    epoch_dir.join(SEGMENTS_DIR_NAME)
}

/// Directory of the derived manifests of an epoch.
pub(crate) fn manifests_directory(epoch_dir: &Path) -> PathBuf {
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
///
/// A compaction reads it too: the epoch the store holds must be the one on disk
/// before it writes, switches or reclaims anything (see
/// [`crate::compact::compact`]).
pub(crate) fn read_current_epoch(index_dir: &Path) -> Result<Epoch, CribleError> {
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
pub(crate) fn write_current_epoch(index_dir: &Path, epoch: Epoch) -> Result<(), CribleError> {
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
/// `tmp-CURRENT_EPOCH` that an interrupted creation left in `index_dir`. The
/// `tmp-codes.bin` of a referenced segment is removed too: it lives inside the
/// segment directory, which the sweep of `segments/` does not enter. Every
/// segment directory absent from the `segments` table is removed: SQLite never
/// referenced it, so it belongs to a generation that was never committed
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
            // A referenced segment stays, but an interrupted rebuild may have
            // left its `tmp-codes.bin` inside it.
            codes::remove_temporary_codes(&path)?;
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

/// Rebuilds every `codes.bin` the published epoch of `index_dir` is missing.
///
/// `ADR-0069`, decision 1: the "reconstruction job" of the segment task is the
/// writer open. A missing `codes.bin` is rebuilt from the reference vectors of
/// its segment and persisted atomically, and the rebuild creates neither a
/// generation nor an epoch: the file is derived, invisible, and changes no
/// search result. Only a write open runs this; a read-only open builds the
/// missing codes in RAM and writes nothing under the epoch (decision 2).
///
/// A file of the wrong length is an [`ErrorCode::Internal`] error naming it
/// (decision 3), not a file to overwrite.
fn rebuild_missing_codes(records: &RecordStore, epoch_dir: &Path) -> Result<(), CribleError> {
    let segments = segments_directory(epoch_dir);
    for segment_id in records.segment_ids()? {
        codes::rebuild_codes(&segments.join(&segment_id))?;
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
/// A segment becomes visible with the generation it declares, a document names
/// the generation that last wrote it, and a passage is created by the
/// generation it declares: all three must be the generation this commit
/// publishes (G+1), otherwise the visible state would hold a generation number
/// that no commit ever produced, and a snapshot would mix two of them (INV-03).
/// A caller that prepares a change for another generation is out of step with
/// the store, which is an internal error, not a bad request.
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
/// The generation and every row come from SQLite, the only authority on what is
/// visible: the bitmap of the live passages is a reading of it, and a snapshot
/// takes its live iids from that bitmap rather than storing them a second time.
/// The segment metadata is read from the segments table and
/// from `segment.json` on disk, which carries the format version and the
/// precision, then sorted by `iid_start`: a snapshot is read in iid order, and
/// a restart produces the manifest in the same order as the commit that wrote
/// it. The filter structures are rebuilt from every row of `passages` and
/// `documents`, following `schema` (ADR-0018, ADR-0019).
///
/// The binary codes are read from `codes.bin` into RAM, through
/// `crate::codes::load_codes`. As long as that file is present no vector file
/// is opened here, so the snapshot does not pay the blake3 read of
/// `SegmentReader::open`: the writer open rebuilds a missing `codes.bin` before
/// this point (`rebuild_missing_codes`). When the file is missing — an index a
/// write open never touched, or a read-only open after a restore — the same
/// loader reads the reference vectors through a `SegmentReader`, checksum
/// included, and quantizes them in RAM.
fn build_snapshot(
    records: &RecordStore,
    epoch_dir: &Path,
    schema: &Schema,
    lexical: PublishedLexical,
    lease: Arc<EpochLease>,
) -> Result<Snapshot, CribleError> {
    let segments = segments_of(records, epoch_dir)?;
    let codes = snapshot_codes(&segments, epoch_dir, lease.epoch, None)?;
    Ok(Snapshot {
        generation: records.meta()?.generation,
        segments,
        codes,
        filters: FilterIndex::from_records(records.connection(), schema)?,
        lexical: lexical.reader,
        lexical_status: lexical.status,
        lease,
    })
}

/// Builds the snapshot that follows the commit of `generation`.
///
/// The filter structures are derived from those of the previous snapshot in
/// copy on write: only the rows `delta` names are read, and the bitmaps and
/// column blocks no change touches are shared with the generation the readers
/// still hold (INV-02, ADR-0066).
///
/// The previous snapshot is at generation G-1 whenever the commit succeeded, so
/// `delta` is the whole difference. It is not when an earlier commit stopped
/// **after** its SQLite commit (a manifest that could not be written): that
/// generation is committed but was never published, and `delta` would leave its
/// rows out. The snapshot is then rebuilt from `records.sqlite`, which is what
/// the next [`GenerationStore::open`] would do (INV-04).
fn build_snapshot_after_commit(
    records: &RecordStore,
    epoch_dir: &Path,
    previous: &Snapshot,
    generation: Generation,
    delta: &CommitDelta,
    lexical: PublishedLexical,
    lease: Arc<EpochLease>,
) -> Result<Snapshot, CribleError> {
    let filters = if previous.generation().get().checked_add(1) == Some(generation.get()) {
        previous
            .filters()
            .after_commit(records.connection(), delta)?
    } else {
        FilterIndex::from_records(records.connection(), previous.filters().schema())?
    };
    let segments = segments_of(records, epoch_dir)?;
    // A segment is immutable: the arrays of the previous snapshot are reused
    // through `Arc`, so a commit reads only the codes of the segments it adds.
    let codes = snapshot_codes(&segments, epoch_dir, lease.epoch, Some(previous))?;
    Ok(Snapshot {
        generation,
        segments,
        codes,
        filters,
        lexical: lexical.reader,
        lexical_status: lexical.status,
        lease,
    })
}

/// Binary codes of `segments`, in the same order, shared with `previous` for
/// every segment it already holds.
///
/// A segment never changes inside an epoch, so an array of the previous snapshot
/// is the right one for the next: the clone of an [`Arc`] costs no read. Two
/// guards make the sharing safe rather than assumed. The previous snapshot must
/// read **the same epoch** as `epoch`: identifiers are only unique inside an
/// epoch, and after a compaction the same identifier may name another segment.
/// And the shared array must hold `count × words_per_code(dims)` words, the
/// length the segment metadata declares; an array of another length is not the
/// one this segment needs, so it is read again instead of shared.
fn snapshot_codes(
    segments: &[SegmentMeta],
    epoch_dir: &Path,
    epoch: Epoch,
    previous: Option<&Snapshot>,
) -> Result<Vec<Arc<Vec<u64>>>, CribleError> {
    let segments_dir = segments_directory(epoch_dir);
    let shareable = previous.filter(|snapshot| snapshot.epoch() == epoch);
    let mut codes = Vec::with_capacity(segments.len());
    for meta in segments {
        if let Some(shared) = shareable.and_then(|snapshot| snapshot.shared_codes(&meta.segment_id))
            && Some(shared.len()) == expected_code_words(meta)
        {
            codes.push(Arc::clone(shared));
            continue;
        }
        codes.push(Arc::new(codes::load_codes(
            &segments_dir.join(&meta.segment_id),
            meta,
        )?));
    }
    Ok(codes)
}

/// Number of `u64` words the codes of `meta` hold:
/// `count × words_per_code(dims)`.
///
/// `None` when the product does not fit in a `usize`: no segment that opened
/// can reach it, and the caller then reads the codes again instead of sharing
/// an array whose length it cannot compare.
fn expected_code_words(meta: &SegmentMeta) -> Option<usize> {
    usize::try_from(meta.count)
        .ok()?
        .checked_mul(codes::words_per_code(meta.dims))
}

/// Metadata of the segments the reference state references, in `iid_start`
/// order then by identifier.
fn segments_of(records: &RecordStore, epoch_dir: &Path) -> Result<Vec<SegmentMeta>, CribleError> {
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
    Ok(segments)
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
    use super::{
        CURRENT_EPOCH_FILE_NAME, EPOCHS_DIR_NAME, GenerationChange, GenerationStore,
        INDEX_CONFIG_FILE_NAME, RECORDS_FILE_NAME, read_current_epoch,
    };
    use crible_core::config::{IndexConfig, ReferencePrecision, Schema};
    use crible_core::error::ErrorCode;
    use crible_core::ids::{Epoch, Generation, Iid};
    use rusqlite::trace::{TraceEvent, TraceEventCodes};
    use serde_json::json;
    use std::fs;
    use std::sync::Mutex;
    use tantivy::schema::IndexRecordOption;

    use crate::records::{DocumentRecord, PassageRecord};

    /// JSON schema of an index that indexes no field.
    const EMPTY_SCHEMA_JSON: &str = r#"{ "document_fields": {}, "passage_fields": {} }"#;

    /// JSON schema of an index indexing one `integer` document field.
    const YEAR_SCHEMA_JSON: &str =
        r#"{ "document_fields": { "annee": { "type": "integer" } }, "passage_fields": {} }"#;

    /// Schema of an index that indexes no field: these tests commit no row, so
    /// their filter structures stay empty.
    fn empty_schema() -> Schema {
        Schema::from_json_str(EMPTY_SCHEMA_JSON)
            .unwrap_or_else(|error| panic!("the empty schema must be valid: {error}"))
    }

    /// Schema indexing one `integer` document field.
    fn schema_with_year() -> Schema {
        Schema::from_json_str(YEAR_SCHEMA_JSON)
            .unwrap_or_else(|error| panic!("the year schema must be valid: {error}"))
    }

    /// Writes an index configuration declaring `schema` as its schema.
    fn write_config(index: &std::path::Path, schema: &str) {
        let raw = test_config("test", schema);
        super::write_index_config(index, &raw)
            .unwrap_or_else(|error| panic!("the index configuration: {error}"));
    }

    /// Index configuration declaring `schema`, with the settings the store
    /// ignores.
    fn test_config(name: &str, schema: &str) -> String {
        format!(
            r#"{{"name":"{name}","model":"m","reference_precision":"fp16","schema":{schema},"cluster_by":[],"lexical":{{"enabled":false,"language":"fr","positions":false}},"search":{{"scan":"none","rescore_candidates":10,"exact_threshold":10,"candidates_per_branch":10,"fusion":{{"method":"rrf"}}}},"ingest":{{"acceptance_sample":1,"acceptance_min_cosine":0.9}},"compaction":{{"dead_ratio":0.1}}}}"#
        )
    }

    /// Writes an index configuration of the empty schema that enables the
    /// lexical branch, French, without positions.
    fn write_lexical_config(index: &std::path::Path) {
        let raw = test_config("test", EMPTY_SCHEMA_JSON)
            .replace(r#""enabled":false"#, r#""enabled":true"#);
        super::write_index_config(index, &raw)
            .unwrap_or_else(|error| panic!("the index configuration: {error}"));
    }

    /// Creation writes the pointer file, readable as a plain decimal number, and
    /// the manifest of generation 0, so a fresh index is inspectable.
    #[test]
    fn create_writes_the_current_epoch_and_the_manifest_of_generation_zero() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let index = temp.path().join("index");
        write_config(&index, EMPTY_SCHEMA_JSON);
        let store = GenerationStore::create(&index, Epoch::new(3), &empty_schema())
            .expect("the index is created");

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

    /// Creating an index that already exists is refused by the store, before
    /// anything is written, and the existing index keeps its epoch and its
    /// generation.
    #[test]
    fn create_refuses_an_existing_index_and_leaves_it_intact() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let index = temp.path().join("index");
        write_config(&index, EMPTY_SCHEMA_JSON);
        let store = GenerationStore::create(&index, Epoch::new(4), &empty_schema())
            .expect("the index is created");
        let pointer = index.join(CURRENT_EPOCH_FILE_NAME);
        let pointer_before = fs::read(&pointer).expect("the pointer is readable");
        let state_before = store.state().expect("the state is readable");
        drop(store);

        let error = GenerationStore::create(&index, Epoch::new(9), &empty_schema())
            .err()
            .expect("creating an existing index must be refused");
        assert_eq!(
            error.code(),
            ErrorCode::InvalidRequest,
            "wrong error code: {}",
            error.message()
        );

        assert_eq!(
            fs::read(&pointer).expect("the pointer is still readable"),
            pointer_before,
            "the refusal must not rewrite CURRENT_EPOCH"
        );
        let reopened =
            GenerationStore::open(&index, &empty_schema()).expect("the index still opens");
        assert_eq!(reopened.epoch(), Epoch::new(4));
        let state_after = reopened.state().expect("the state is readable");
        assert_eq!(
            (
                state_before.epoch,
                state_before.generation,
                state_before.documents,
                state_before.live_passages,
            ),
            (
                state_after.epoch,
                state_after.generation,
                state_after.documents,
                state_after.live_passages,
            ),
            "the refusal must leave the index untouched"
        );
    }

    /// A directory without `index.json` is not a complete index: creation is
    /// refused before anything is written, so the store never builds filter
    /// structures from a schema no configuration declares.
    #[test]
    fn create_refuses_an_index_without_a_declared_schema() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let index = temp.path().join("index");
        fs::create_dir_all(&index).expect("the index directory is created");

        let error = GenerationStore::create(&index, Epoch::new(1), &empty_schema())
            .err()
            .expect("an index without index.json must be refused");
        assert_eq!(
            error.code(),
            ErrorCode::InvalidRequest,
            "wrong error code: {}",
            error.message()
        );
        assert!(
            !index.join(CURRENT_EPOCH_FILE_NAME).exists(),
            "a refused creation must leave no index"
        );
        assert!(
            !index.join("epochs").exists(),
            "a refused creation must not write the epoch"
        );
    }

    /// A directory holding an epoch but no `CURRENT_EPOCH` is a torn index:
    /// writing a configuration over it would let `create` reopen a populated
    /// `records.sqlite` with another schema, so it is refused as a partial tree.
    #[test]
    fn write_index_config_refuses_a_torn_index_state() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let index = temp.path().join("index");
        let epoch = index.join("epochs").join("1");
        fs::create_dir_all(&epoch).expect("the epoch directory is created");
        fs::write(epoch.join("records.sqlite"), b"stale").expect("the stale database is written");

        let error = super::write_index_config(&index, &test_config("test", EMPTY_SCHEMA_JSON))
            .expect_err("a torn index must be refused");
        assert_eq!(
            error.code(),
            ErrorCode::InvalidRequest,
            "wrong error code: {}",
            error.message()
        );
        assert!(
            error.message().contains("partial index tree"),
            "the message must say the tree is not a usable index: {}",
            error.message()
        );
        assert!(
            error.message().contains("epochs"),
            "the message must name what is present: {}",
            error.message()
        );
        assert!(
            error.message().contains(CURRENT_EPOCH_FILE_NAME),
            "the message must name the pointer that is missing: {}",
            error.message()
        );
        assert!(
            !error.message().contains("already exists"),
            "a torn tree is not an existing index: {}",
            error.message()
        );
        assert!(
            !index.join(INDEX_CONFIG_FILE_NAME).exists(),
            "a refused write must leave no index.json"
        );
    }

    /// A single `records.sqlite` at the index root is the fourth state a
    /// directory can hold: no epoch directory and no pointer, only the trace of
    /// a tree moved or interrupted by hand. It is refused as a partial tree too.
    #[test]
    fn write_index_config_refuses_a_records_file_at_the_index_root() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let index = temp.path().join("index");
        fs::create_dir_all(&index).expect("the index directory is created");
        let records = index.join(RECORDS_FILE_NAME);
        fs::write(&records, b"stale").expect("the stale database is written");

        let error = super::write_index_config(&index, &test_config("test", EMPTY_SCHEMA_JSON))
            .expect_err("a records.sqlite at the index root must be refused");
        assert_eq!(
            error.code(),
            ErrorCode::InvalidRequest,
            "wrong error code: {}",
            error.message()
        );
        assert!(
            error.message().contains("partial index tree"),
            "the message must say the tree is not a usable index: {}",
            error.message()
        );
        assert!(
            error.message().contains(&records.display().to_string()),
            "the message must name the file it found: {}",
            error.message()
        );
        assert!(
            !index.join(INDEX_CONFIG_FILE_NAME).exists(),
            "a refused write must leave no index.json"
        );
        assert!(
            !index.join(EPOCHS_DIR_NAME).exists(),
            "a refused write must not create the epoch directory"
        );
    }

    /// A configuration that does not parse as an [`IndexConfig`] is refused
    /// before anything is written: the store never leaves an `index.json` that
    /// every later `open` would have to reject.
    #[test]
    fn write_index_config_refuses_a_configuration_that_does_not_parse() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let index = temp.path().join("index");

        for broken in ["{ not json", r#"{"name":"test"}"#] {
            let error = super::write_index_config(&index, broken)
                .expect_err("a configuration that does not parse must be refused");
            assert_eq!(
                error.code(),
                ErrorCode::InvalidRequest,
                "wrong error code for {broken}: {}",
                error.message()
            );
        }
        assert!(
            !index.join(INDEX_CONFIG_FILE_NAME).exists(),
            "a refused write must leave no index.json"
        );
        assert!(
            !index.exists(),
            "a refused write must not even create the directory"
        );
    }

    /// A residual `index.json` is the trace of a creation that stopped before
    /// its first epoch: nothing else exists, so replacing it loses nothing and
    /// the creation then succeeds.
    #[test]
    fn write_index_config_replaces_a_residual_configuration() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let index = temp.path().join("index");
        fs::create_dir_all(&index).expect("the index directory is created");
        fs::write(index.join(INDEX_CONFIG_FILE_NAME), r#"{"stale":true}"#)
            .expect("the residual configuration is written");

        let raw = test_config("test", EMPTY_SCHEMA_JSON);
        super::write_index_config(&index, &raw).expect("the residual configuration is replaced");
        assert_eq!(
            fs::read_to_string(index.join(INDEX_CONFIG_FILE_NAME))
                .expect("the configuration is readable"),
            raw,
            "the residual configuration must be replaced"
        );
        GenerationStore::create(&index, Epoch::new(1), &empty_schema())
            .expect("the index is created after the replacement");
    }

    /// `write_index_config` refuses a complete index, like `create`: the message
    /// names the pointer it found, points at the commands that use the index,
    /// and never invites to remove it.
    #[test]
    fn write_index_config_refuses_an_existing_index() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let index = temp.path().join("index");
        write_config(&index, EMPTY_SCHEMA_JSON);
        drop(
            GenerationStore::create(&index, Epoch::new(1), &empty_schema())
                .expect("the index is created"),
        );

        let error = super::write_index_config(&index, &test_config("test", EMPTY_SCHEMA_JSON))
            .expect_err("an existing index must be refused");
        assert_eq!(
            error.code(),
            ErrorCode::InvalidRequest,
            "wrong error code: {}",
            error.message()
        );
        assert!(
            error.message().contains("already exists"),
            "the message must say the index exists: {}",
            error.message()
        );
        assert!(
            error.message().contains(CURRENT_EPOCH_FILE_NAME),
            "the message must name what is present: {}",
            error.message()
        );
        for command in ["attach", "search", "inspect"] {
            assert!(
                error.message().contains(command),
                "the message must point at `crible {command}`: {}",
                error.message()
            );
        }
        assert!(
            !error.message().contains("remove"),
            "a complete index is usable, so the message must not invite to remove it: {}",
            error.message()
        );
        assert!(
            !error.message().contains("partial"),
            "a complete index is not a partial tree: {}",
            error.message()
        );
    }

    /// The schema of `index.json` is the authority: a different schema is
    /// refused at creation, before the epoch exists.
    #[test]
    fn create_refuses_a_schema_that_differs_from_the_index_configuration() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let index = temp.path().join("index");
        write_config(
            &index,
            r#"{ "document_fields": { "annee": { "type": "integer" } }, "passage_fields": {} }"#,
        );

        let error = GenerationStore::create(&index, Epoch::new(1), &empty_schema())
            .err()
            .expect("a different schema must be refused");
        assert_eq!(
            error.code(),
            ErrorCode::ImmutableSetting,
            "wrong error code: {}",
            error.message()
        );
        assert!(
            !index.join(CURRENT_EPOCH_FILE_NAME).exists(),
            "a refused creation must leave no index"
        );
    }

    /// Opening an index with a schema other than the declared one is refused,
    /// instead of silently building different filter structures.
    #[test]
    fn open_refuses_a_schema_that_differs_from_the_index_configuration() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let index = temp.path().join("index");
        write_config(
            &index,
            r#"{ "document_fields": { "annee": { "type": "integer" } }, "passage_fields": {} }"#,
        );
        let store = GenerationStore::create(&index, Epoch::new(1), &schema_with_year())
            .expect("the declared schema creates the index");
        drop(store);

        let error = GenerationStore::open(&index, &empty_schema())
            .err()
            .expect("a different schema must be refused");
        assert_eq!(
            error.code(),
            ErrorCode::ImmutableSetting,
            "wrong error code: {}",
            error.message()
        );

        let reopened = GenerationStore::open(&index, &schema_with_year())
            .expect("the declared schema still opens the index");
        assert_eq!(reopened.epoch(), Epoch::new(1));
    }

    #[test]
    fn rebuild_lexical_refuses_a_store_opened_read_only() {
        // The job removes and commits files under the epoch, so a store a read
        // command opened must refuse it instead of taking the tantivy writer
        // lock a live server holds (ADR-0087, decision 1).
        let temp = tempfile::tempdir().expect("temporary directory");
        let index = temp.path().join("index");
        write_lexical_config(&index);
        drop(
            GenerationStore::create(&index, Epoch::new(1), &empty_schema())
                .expect("the index is created"),
        );

        let mut store = GenerationStore::open_read_only(&index, &empty_schema())
            .expect("the index opens for reading");
        let error = super::rebuild_lexical(&mut store)
            .expect_err("a read-only store must not rebuild the lexical index");
        assert_eq!(
            error.code(),
            ErrorCode::Internal,
            "wrong error code: {}",
            error.message()
        );
        assert!(
            !index
                .join(EPOCHS_DIR_NAME)
                .join("1")
                .join("lexical")
                .exists(),
            "a refused rebuild must write nothing under the epoch"
        );
    }

    /// An epoch that never carried a passage has nothing to rebuild, and gets no
    /// `lexical/` directory: the branch appears with the first passage
    /// (`docs/02`, step 4).
    #[test]
    fn rebuild_lexical_leaves_an_epoch_without_passage_without_a_lexical_directory() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let index = temp.path().join("index");
        write_lexical_config(&index);
        drop(
            GenerationStore::create(&index, Epoch::new(1), &empty_schema())
                .expect("the index is created"),
        );

        let mut store =
            GenerationStore::open(&index, &empty_schema()).expect("the index opens for writing");
        let outcome = super::rebuild_lexical(&mut store)
            .expect("rebuilding an epoch without passage is a no-op, not an error");
        assert_eq!(outcome.generation, Generation::new(0));
        assert!(
            !outcome.rebuilt,
            "an epoch without passage has nothing to rebuild"
        );
        assert!(
            !index
                .join(EPOCHS_DIR_NAME)
                .join("1")
                .join("lexical")
                .exists(),
            "an epoch without passage must not gain a lexical directory"
        );
        assert_eq!(
            store.snapshot().lexical_status(),
            super::LexicalStatus::Missing,
            "the branch still has nothing to serve"
        );
    }

    /// The reason of the job: a change of `lexical.positions` changes no text, so
    /// the state stays `Ready` and the recovery of a write open repairs nothing
    /// (`ADR-0088`, decision 2). Only [`super::rebuild_lexical`] applies the new
    /// analysis chain, rebuilds the index with it, and publishes a snapshot whose
    /// reader is the one it just opened.
    #[test]
    fn rebuild_lexical_applies_the_analysis_chain_and_publishes_its_reader() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let index = temp.path().join("index");
        write_lexical_config(&index);
        let mut store = GenerationStore::create(&index, Epoch::new(1), &empty_schema())
            .expect("the index is created");
        plan_test_first_generation(&mut store);

        let committed = super::LexicalStatus::Ready {
            committed: Generation::new(1),
        };
        assert_eq!(
            store.snapshot().lexical_status(),
            committed,
            "the commit of a generation with passages builds the lexical index"
        );
        // The fixture discriminates: the commit built the index with the chain
        // `index.json` declares, without positions.
        assert_eq!(
            published_text_record(&store),
            IndexRecordOption::WithFreqs,
            "the fixture index must start without term positions"
        );

        // What a `PATCH` of `lexical.positions` will do to the live store (P6):
        // the setting changes, no text does, and the state stays `Ready`.
        store.lexical.positions = true;
        let outcome = super::rebuild_lexical(&mut store)
            .expect("the job rebuilds the lexical index of an epoch with passages");
        assert_eq!(
            outcome.generation,
            Generation::new(1),
            "the job creates no generation and returns the committed one"
        );
        assert!(outcome.rebuilt, "the job must report the rebuild it ran");

        let snapshot = store.snapshot();
        assert_eq!(
            snapshot.lexical_status(),
            committed,
            "the rebuilt index commits the reference generation as its payload"
        );
        assert!(
            snapshot.lexical().is_some(),
            "the published snapshot must carry the reader the job opened"
        );
        assert_eq!(
            published_text_record(&store),
            IndexRecordOption::WithFreqsAndPositions,
            "the job must rebuild the index with the analysis chain of the store"
        );
    }

    /// The index option the published lexical index records for its text field,
    /// which is the observable of `lexical.positions` in the rebuilt index.
    fn published_text_record(store: &GenerationStore) -> IndexRecordOption {
        let snapshot = store.snapshot();
        let lexical = snapshot
            .lexical()
            .unwrap_or_else(|| panic!("the snapshot must publish a lexical reader"));
        let searcher = lexical.searcher();
        let schema = searcher.index().schema();
        let field = schema
            .get_field("text")
            .unwrap_or_else(|_| panic!("the lexical schema must declare the text field"));
        let entry = schema.get_field_entry(field);
        let tantivy::schema::FieldType::Str(options) = entry.field_type() else {
            panic!("the text field of the lexical schema must be a text field");
        };
        options
            .get_indexing_options()
            .unwrap_or_else(|| panic!("the text field of the lexical schema must be indexed"))
            .index_option()
    }

    /// Opening an index whose `index.json` disappeared is refused: the store
    /// must not rebuild the filter structures from the caller's schema alone.
    #[test]
    fn open_refuses_an_index_without_a_declared_schema() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let index = temp.path().join("index");
        write_config(&index, EMPTY_SCHEMA_JSON);
        drop(
            GenerationStore::create(&index, Epoch::new(1), &empty_schema())
                .expect("the index is created"),
        );
        fs::remove_file(index.join(INDEX_CONFIG_FILE_NAME))
            .expect("the configuration is removed to simulate a lost file");

        let error = GenerationStore::open(&index, &empty_schema())
            .err()
            .expect("an index without index.json must be refused");
        assert_eq!(
            error.code(),
            ErrorCode::InvalidRequest,
            "wrong error code: {}",
            error.message()
        );
    }

    // --- The plan of the commit path (ADR-0066) ---

    /// JSON schema of an index indexing one field of each level: the commit
    /// path reads the two tables that carry those fields, so a test that pins
    /// its plans has both of them in play.
    const PLAN_SCHEMA_JSON: &str = r#"{ "document_fields": { "annee": { "type": "integer" } },
             "passage_fields": { "rang": { "type": "integer" } } }"#;

    /// Relations a `SCAN` line may name without the test failing: the
    /// reference database holds at most one row of `meta` and one of `batches`
    /// per batch, and `segments` holds one row per segment. A full scan of
    /// those costs nothing at the scale targeted by ADR-0066, whereas a scan of
    /// `passages` or `documents` costs the size of the index.
    const SCANNABLE_RELATIONS: [&str; 3] = ["meta", "batches", "segments"];

    /// Words after a relation that introduce no alias: the statement continues
    /// with a clause instead.
    const NOT_AN_ALIAS: [&str; 30] = [
        "WHERE",
        "ON",
        "USING",
        "SET",
        "VALUES",
        "SELECT",
        "LEFT",
        "RIGHT",
        "INNER",
        "OUTER",
        "CROSS",
        "NATURAL",
        "JOIN",
        "ORDER",
        "GROUP",
        "LIMIT",
        "OFFSET",
        "HAVING",
        "WINDOW",
        "UNION",
        "EXCEPT",
        "INTERSECT",
        "INDEXED",
        "NOT",
        "AND",
        "OR",
        "AS",
        "BY",
        "NULLS",
        "DO",
    ];

    /// Words that introduce the relation of a clause.
    const RELATION_INTRODUCERS: [&str; 4] = ["FROM", "JOIN", "INTO", "UPDATE"];

    /// Relations a plan line may scan without being a table: a constant row, a
    /// subquery, a materialised set.
    const NOT_A_RELATION: [&str; 3] = ["CONSTANT", "SUBQUERY", "ROW"];

    /// Statements recorded by [`record_statement`].
    ///
    /// The trace callback of rusqlite is a plain function pointer, so the
    /// statements land in one process-wide buffer. Only the connection the test
    /// traces writes into it, and that connection belongs to one test.
    static TRACED_STATEMENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

    /// Trace callback: keeps the SQL text of every statement that starts.
    fn record_statement(event: TraceEvent<'_>) {
        if let TraceEvent::Stmt(_, sql) = event
            && let Ok(mut statements) = TRACED_STATEMENTS.lock()
        {
            statements.push(sql.to_owned());
        }
    }

    /// Splits SQL into words and single punctuation characters, skipping string
    /// literals and comments.
    fn sql_tokens(sql: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut chars = sql.chars().peekable();
        while let Some(character) = chars.next() {
            if character.is_whitespace() {
                continue;
            }
            if character == '\'' {
                // A string literal: a doubled quote is an escaped one.
                while let Some(inner) = chars.next() {
                    if inner == '\'' {
                        if chars.peek() == Some(&'\'') {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                }
                continue;
            }
            if character == '-' && chars.peek() == Some(&'-') {
                for inner in chars.by_ref() {
                    if inner == '\n' {
                        break;
                    }
                }
                continue;
            }
            if character == '/' && chars.peek() == Some(&'*') {
                chars.next();
                let mut previous = '\0';
                for inner in chars.by_ref() {
                    if previous == '*' && inner == '/' {
                        break;
                    }
                    previous = inner;
                }
                continue;
            }
            if character.is_alphanumeric() || character == '_' || character == '$' {
                let mut word = String::from(character);
                while let Some(&inner) = chars.peek() {
                    if inner.is_alphanumeric() || inner == '_' || inner == '$' {
                        word.push(inner);
                        chars.next();
                    } else {
                        break;
                    }
                }
                tokens.push(word);
                continue;
            }
            tokens.push(character.to_string());
        }
        tokens
    }

    /// Every `(alias, relation)` pair one statement declares, with the relation
    /// name standing for itself when the statement gives it no alias.
    fn relations_of(sql: &str) -> Vec<(String, String)> {
        let tokens = sql_tokens(sql);
        let mut relations = Vec::new();
        let introduced = |token: &str| {
            RELATION_INTRODUCERS
                .iter()
                .any(|keyword| keyword.eq_ignore_ascii_case(token))
        };
        let mut position = 0;
        while position < tokens.len() {
            if !introduced(&tokens[position]) {
                position += 1;
                continue;
            }
            let mut next = position + 1;
            if tokens.get(next).map(String::as_str) == Some("(") {
                position = next;
                continue;
            }
            let Some(relation) = tokens.get(next) else {
                break;
            };
            // A schema-qualified name: `main.passages`.
            if tokens.get(next + 1).map(String::as_str) == Some(".") {
                if let Some(qualified) = tokens.get(next + 2) {
                    relations.push((qualified.clone(), qualified.clone()));
                    next += 2;
                }
            } else {
                relations.push((relation.clone(), relation.clone()));
            }
            let mut alias_at = next + 1;
            if tokens
                .get(alias_at)
                .is_some_and(|token| token.eq_ignore_ascii_case("AS"))
            {
                alias_at += 1;
            }
            if let Some(alias) = tokens.get(alias_at) {
                let is_word = alias
                    .chars()
                    .all(|character| character.is_alphanumeric() || character == '_');
                let is_clause = NOT_AN_ALIAS
                    .iter()
                    .any(|keyword| keyword.eq_ignore_ascii_case(alias))
                    || alias == "("
                    || alias == ",";
                if is_word && !is_clause {
                    relations.push((alias.clone(), relation.clone()));
                }
            }
            position = next + 1;
        }
        relations
    }

    /// The relation one `SCAN` line of an execution plan reads, or `None` when
    /// the line scans no relation at all.
    fn scanned_relation(sql: &str, detail: &str) -> Option<String> {
        let tokens: Vec<&str> = detail.split_whitespace().collect();
        let scan = tokens.first()?;
        if !scan.eq_ignore_ascii_case("SCAN") {
            return None;
        }
        let target = if tokens
            .get(1)
            .is_some_and(|token| token.eq_ignore_ascii_case("TABLE"))
        {
            tokens.get(2)?
        } else {
            tokens.get(1)?
        };
        if NOT_A_RELATION
            .iter()
            .any(|keyword| keyword.eq_ignore_ascii_case(target))
        {
            return None;
        }
        let relations = relations_of(sql);
        // Every relation the alias may denote: an alias reused by a subquery
        // names both, and the test then treats the scan as touching the
        // expensive one too, which is the safe direction.
        let mut denoted: Vec<&str> = relations
            .iter()
            .filter(|(alias, _)| alias.eq_ignore_ascii_case(target))
            .map(|(_, relation)| relation.as_str())
            .collect();
        if denoted.is_empty() {
            denoted.push(target);
        }
        Some(denoted.join(" or "))
    }

    /// Executable statements of a trace: the ones with a plan to explain.
    fn explains(statement: &str) -> bool {
        sql_tokens(statement).first().is_some_and(|first| {
            ["SELECT", "WITH", "INSERT", "UPDATE", "DELETE"]
                .iter()
                .any(|verb| verb.eq_ignore_ascii_case(first))
        })
    }

    /// Number of parameters a statement binds, so its plan can be explained
    /// without running it: SQLite requires them bound even for
    /// `EXPLAIN QUERY PLAN`.
    fn parameter_count(sql: &str) -> usize {
        let tokens = sql_tokens(sql);
        let mut numbered = 0;
        let mut anonymous = 0;
        let mut position = 0;
        while position < tokens.len() {
            if tokens[position] == "?" {
                match tokens
                    .get(position + 1)
                    .and_then(|token| token.parse::<usize>().ok())
                {
                    Some(number) => {
                        numbered = numbered.max(number);
                        position += 2;
                        continue;
                    }
                    None => anonymous += 1,
                }
            }
            position += 1;
        }
        numbered.max(anonymous)
    }

    /// A commit of one generation that creates a document and two passages, so
    /// the following commit has a document to update and a passage to
    /// tombstone.
    fn plan_test_first_generation(store: &mut GenerationStore) {
        store
            .commit(GenerationChange {
                segments: Vec::new(),
                documents: vec![DocumentRecord {
                    doc_id: "doc-a".to_owned(),
                    doc_hash: "blake3:doc-a".to_owned(),
                    fields: json!({ "annee": 2021 }),
                    updated_generation: Generation::new(1),
                }],
                passages: vec![
                    PassageRecord {
                        iid: Iid::new(0),
                        doc_id: "doc-a".to_owned(),
                        ord: 0,
                        text: "doc-a#0".to_owned(),
                        passage_hash: "blake3:passage-0".to_owned(),
                        fields: json!({ "rang": 0 }),
                        segment_id: "seg-a".to_owned(),
                        created_generation: Generation::new(1),
                        dead_since_gen: None,
                    },
                    PassageRecord {
                        iid: Iid::new(1),
                        doc_id: "doc-a".to_owned(),
                        ord: 1,
                        text: "doc-a#1".to_owned(),
                        passage_hash: "blake3:passage-1".to_owned(),
                        fields: json!({ "rang": 1 }),
                        segment_id: "seg-a".to_owned(),
                        created_generation: Generation::new(1),
                        dead_since_gen: None,
                    },
                ],
                deletions: Vec::new(),
            })
            .expect("the first generation commits");
    }

    /// The commit of a generation must not scan `passages` or `documents`.
    ///
    /// The test does not list the queries to watch: it records **every**
    /// statement the connection of the commit really runs, then asks SQLite for
    /// the execution plan of each of them. A query added to the commit path is
    /// therefore examined without anyone updating a list here (ADR-0066,
    /// "À surveiller").
    ///
    /// The generation committed here touches the four steps of the derivation:
    /// a document and a passage created, a document rewritten, a passage
    /// tombstoned, and the removed-documents query, which runs even when it
    /// finds nothing.
    #[test]
    fn commit_path_plans_no_scan_on_passages_or_documents() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let index = temp.path().join("index");
        write_config(&index, PLAN_SCHEMA_JSON);
        let schema = Schema::from_json_str(PLAN_SCHEMA_JSON)
            .unwrap_or_else(|error| panic!("the plan schema must be valid: {error}"));
        let mut store =
            GenerationStore::create(&index, Epoch::new(1), &schema).expect("the index is created");
        plan_test_first_generation(&mut store);

        TRACED_STATEMENTS
            .lock()
            .expect("the trace buffer is not poisoned")
            .clear();
        {
            let connection = store.records.connection();
            connection.trace_v2(TraceEventCodes::SQLITE_TRACE_STMT, Some(record_statement));
        }
        let outcome = store.commit(GenerationChange {
            segments: Vec::new(),
            documents: vec![
                DocumentRecord {
                    doc_id: "doc-a".to_owned(),
                    doc_hash: "blake3:doc-a".to_owned(),
                    fields: json!({ "annee": 2022 }),
                    updated_generation: Generation::new(2),
                },
                DocumentRecord {
                    doc_id: "doc-b".to_owned(),
                    doc_hash: "blake3:doc-b".to_owned(),
                    fields: json!({ "annee": 2022 }),
                    updated_generation: Generation::new(2),
                },
            ],
            passages: vec![PassageRecord {
                iid: Iid::new(2),
                doc_id: "doc-b".to_owned(),
                ord: 0,
                text: "doc-b#0".to_owned(),
                passage_hash: "blake3:passage-2".to_owned(),
                fields: json!({ "rang": 0 }),
                segment_id: "seg-b".to_owned(),
                created_generation: Generation::new(2),
                dead_since_gen: None,
            }],
            deletions: vec![Iid::new(1)],
        });
        {
            let connection = store.records.connection();
            connection.trace_v2(TraceEventCodes::SQLITE_TRACE_STMT, None);
        }
        outcome.expect("the traced generation commits");

        let mut statements = TRACED_STATEMENTS
            .lock()
            .expect("the trace buffer is not poisoned")
            .clone();
        statements.sort();
        statements.dedup();
        assert!(
            !statements.is_empty(),
            "the trace recorded no statement of the commit: the test would prove nothing"
        );
        for table in ["passages", "documents"] {
            assert!(
                statements
                    .iter()
                    .any(|statement| statement.to_lowercase().contains(table)),
                "the trace recorded no statement naming {table}, so the commit path it captured \
                 is not the one this test means to check: {statements:#?}"
            );
        }

        let connection = store.records.connection();
        let mut offenders = Vec::new();
        let mut documents_by_primary_key = false;
        let mut passages_by_doc_id = false;
        for statement in &statements {
            if !explains(statement) {
                continue;
            }
            let mut plan = connection
                .prepare(&format!("EXPLAIN QUERY PLAN {statement}"))
                .unwrap_or_else(|error| {
                    panic!(
                        "the plan of a statement of the commit path must be explainable; \
                         the query is:\n{statement}\nSQLite said: {error}"
                    )
                });
            let parameters: Vec<rusqlite::types::Value> = (0..parameter_count(statement))
                .map(|_| rusqlite::types::Value::Null)
                .collect();
            let details = plan
                .query_map(rusqlite::params_from_iter(parameters), |row| {
                    row.get::<_, String>(3)
                })
                .unwrap_or_else(|error| {
                    panic!(
                        "the plan of a statement of the commit path must be readable; \
                         the query is:\n{statement}\nSQLite said: {error}"
                    )
                })
                .collect::<Result<Vec<String>, _>>()
                .unwrap_or_else(|error| {
                    panic!(
                        "the plan of a statement of the commit path must be read; \
                         the query is:\n{statement}\nSQLite said: {error}"
                    )
                });
            let is_select = sql_tokens(statement)
                .first()
                .is_some_and(|token| token.eq_ignore_ascii_case("SELECT"));
            for detail in &details {
                // The two accesses ADR-0066 allows are bounded SELECTs:
                // `documents` by its primary key and `passages` by its `doc_id`
                // index. Requiring the statement itself to be a SELECT keeps an
                // upsert, whose plan also searches the primary key, from
                // standing in for the read the commit must really run.
                let plan = detail.to_lowercase();
                if plan.starts_with("search documents") && plan.contains("(doc_id=?") {
                    documents_by_primary_key |= is_select;
                }
                if plan.starts_with("search passages") && plan.contains("(doc_id=?") {
                    passages_by_doc_id |= is_select;
                }
                let Some(relation) = scanned_relation(statement, detail) else {
                    continue;
                };
                let expensive = relation.split(" or ").any(|name| {
                    name.eq_ignore_ascii_case("passages") || name.eq_ignore_ascii_case("documents")
                });
                let allowed = relation.split(" or ").all(|name| {
                    SCANNABLE_RELATIONS
                        .iter()
                        .any(|table| table.eq_ignore_ascii_case(name))
                });
                if expensive || !allowed {
                    offenders.push(format!(
                        "  query:\n{statement}\n  plan line: {detail}\n  scans: {relation}"
                    ));
                }
            }
        }
        assert!(
            documents_by_primary_key,
            "the trace planned no bounded read of `documents` by its primary key. The commit \
             must read the fields of a rewritten document (`SELECT fields FROM documents \
             WHERE doc_id = ?1`), which ADR-0066 allows because the plan is bounded: {statements:#?}"
        );
        assert!(
            passages_by_doc_id,
            "the trace planned no bounded read of `passages` by `passages(doc_id)`. The commit \
             must read the passages of a rewritten or removed document (`SELECT iid FROM \
             passages WHERE doc_id = ?1`), which ADR-0066 allows because the plan is bounded: \
             {statements:#?}"
        );
        assert!(
            offenders.is_empty(),
            "the commit path scans a table whose size is the size of the index. A generation \
             must be derived from the delta the transaction wrote, not re-read by \
             generation stamp (ADR-0066); a bounded read goes through passages(doc_id) or the \
             primary key of documents. Offending statements:\n{}",
            offenders.join("\n")
        );
    }

    /// ADR-0108, decision 1: `CURRENT_EPOCH` is the authority. A store still
    /// holding epoch E while the pointer names E+1 — the state a compaction
    /// leaves behind when it dies after its switch — must refuse the commit
    /// before opening its transaction, and write nothing.
    #[test]
    fn commit_refuses_when_current_epoch_names_another_epoch() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let index = temp.path().join("index");
        write_config(&index, EMPTY_SCHEMA_JSON);
        let schema = empty_schema();
        let mut store =
            GenerationStore::create(&index, Epoch::new(1), &schema).expect("the index is created");
        let before = store
            .with_connection(crate::records::read_meta)
            .expect("the meta of the created index is readable");

        super::write_current_epoch(&index, Epoch::new(2))
            .expect("the pointer of a switched epoch is written");
        let error = store
            .commit(GenerationChange {
                segments: Vec::new(),
                documents: Vec::new(),
                passages: Vec::new(),
                deletions: Vec::new(),
            })
            .expect_err("a store on epoch 1 must not commit while the pointer names epoch 2");
        assert_eq!(
            error.code(),
            ErrorCode::Internal,
            "the epoch mismatch is an internal error, not a request error: {}",
            error.message()
        );
        assert!(
            error.message().contains("ADR-0108"),
            "the refusal must name the decision it applies, so the log points at it: {}",
            error.message()
        );

        let after = store
            .with_connection(crate::records::read_meta)
            .expect("the meta stays readable after the refused commit");
        assert_eq!(
            after.generation, before.generation,
            "the refused commit must not advance the generation"
        );
        assert_eq!(
            after.next_iid, before.next_iid,
            "the refused commit must not consume an internal identifier"
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

    /// The codes of a snapshot of another epoch are never shared, even when the
    /// segment identifier matches and the array has the right length: an
    /// identifier is only unique inside one epoch.
    ///
    /// The length guard is not reachable from the public API — a shared array is
    /// always built from the metadata of the same segment — so only the epoch
    /// guard has a test; the length check stays the defensive half.
    #[test]
    fn snapshot_codes_does_not_share_arrays_across_epochs() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let index = temp.path().join("index");
        write_config(&index, EMPTY_SCHEMA_JSON);
        let store = GenerationStore::create(&index, Epoch::new(1), &empty_schema())
            .expect("the index is created");
        let epoch_dir = super::epoch_directory(&index, Epoch::new(1));
        let segments = super::segments_directory(&epoch_dir);
        fs::create_dir_all(&segments).expect("the segments directory is created");
        let spec = crate::segment::SegmentSpec {
            segment_id: "seg-shared".to_owned(),
            iid_start: Iid::new(0),
            dims: 64,
            generation: Generation::new(1),
            batch_id: "batch-shared".to_owned(),
            precision: ReferencePrecision::Fp16,
        };
        let mut writer = crate::segment::SegmentWriter::create(&segments, spec)
            .expect("the segment writer starts");
        let mut vector = vec![0.0_f32; 64];
        vector[0] = 1.0;
        writer.append(&vector).expect("the vector is appended");
        let meta = writer.finish().expect("the segment is committed");

        // A snapshot of another epoch holding the same identifier and an array
        // of the right length, but all zeros: sharing it would be visible.
        let base = store.snapshot();
        let previous = super::Snapshot {
            generation: base.generation(),
            segments: vec![meta.clone()],
            codes: vec![std::sync::Arc::new(vec![0_u64])],
            filters: base.filters().clone(),
            lexical: None,
            lexical_status: super::LexicalStatus::Disabled,
            lease: std::sync::Arc::new(super::EpochLease {
                epoch: Epoch::new(2),
            }),
        };

        let shared = super::snapshot_codes(
            std::slice::from_ref(&meta),
            &epoch_dir,
            Epoch::new(1),
            Some(&previous),
        )
        .expect("the codes of the epoch are read");
        assert_eq!(
            shared[0].as_slice(),
            &[1_u64],
            "the codes of another epoch must not be shared"
        );
    }
    /// Schema whose `annee` field starts unfilterable and unreturned.
    const RECONFIGURE_SCHEMA_BEFORE: &str = r#"{ "document_fields": { "annee": { "type": "integer", "filterable": false, "returned": false } }, "passage_fields": {} }"#;

    /// Same schema after the patch: `filterable` and `returned` are now true.
    const RECONFIGURE_SCHEMA_AFTER: &str = r#"{ "document_fields": { "annee": { "type": "integer", "filterable": true, "returned": true } }, "passage_fields": {} }"#;

    #[test]
    fn reconfigure_refuses_a_read_only_store() {
        // ADR-0105, decision 4: a `PATCH` job may have to rebuild the lexical
        // index, which writes under the epoch; a store open for reading cannot.
        let temp = tempfile::tempdir().expect("temporary directory");
        let index = temp.path().join("index");
        write_config(&index, EMPTY_SCHEMA_JSON);
        drop(
            GenerationStore::create(&index, Epoch::new(1), &empty_schema())
                .expect("the index is created"),
        );
        let mut store = GenerationStore::open_read_only(&index, &empty_schema())
            .expect("the read-only open succeeds");
        let config = IndexConfig::from_json_str(&test_config("test", EMPTY_SCHEMA_JSON))
            .expect("the configuration parses");

        let error = store
            .reconfigure(&config)
            .expect_err("a read-only store must refuse a reconfigure");
        assert_eq!(
            error.code(),
            ErrorCode::Internal,
            "wrong error code: {}",
            error.message()
        );
    }

    #[test]
    fn reconfigure_rebuilds_the_filter_structures_from_the_new_schema() {
        // `docs/04` and ADR-0105, decision 4: the snapshot a patch calls for is
        // rebuilt from `records.sqlite` with the new schema, so `filterable` and
        // `returned` take effect; the snapshot is returned, not published.
        let temp = tempfile::tempdir().expect("temporary directory");
        let index = temp.path().join("index");
        write_config(&index, RECONFIGURE_SCHEMA_BEFORE);
        let schema =
            Schema::from_json_str(RECONFIGURE_SCHEMA_BEFORE).expect("the before schema is valid");
        let mut store =
            GenerationStore::create(&index, Epoch::new(1), &schema).expect("the index is created");
        let after = IndexConfig::from_json_str(&test_config("test", RECONFIGURE_SCHEMA_AFTER))
            .expect("the after configuration parses");

        let snapshot = store
            .reconfigure(&after)
            .expect("the reconfigure builds a snapshot");
        let (spec, _level) = snapshot
            .filters()
            .schema()
            .field("annee")
            .expect("the new schema declares annee");
        assert!(spec.filterable, "the rebuilt structures cover filterable");
        assert!(spec.returned, "the rebuilt structures cover returned");
        assert!(
            snapshot.filters().column("annee").is_some(),
            "the filter structure of the field is rebuilt"
        );

        let held = store.snapshot();
        let (published, _level) = held
            .filters()
            .schema()
            .field("annee")
            .expect("the published schema declares annee");
        assert!(
            !published.filterable && !published.returned,
            "reconfigure returns the snapshot without publishing it"
        );
    }
}
