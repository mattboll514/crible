// SPDX-License-Identifier: Apache-2.0

//! Fixtures shared by the generation invariant tests (`INV-03`, `INV-04`).
//!
//! The API of `crible_store::generation` assumed by these fixtures is
//! documented at the top of `inv_03_generation_visibility.rs` and
//! `inv_04_recovery.rs`. Every path helper follows the on-disk layout of
//! `docs/02-architecture.md` ("Organisation sur disque"):
//!
//! ```text
//! <index>/CURRENT_EPOCH
//! <index>/epochs/<epoch>/records.sqlite
//! <index>/epochs/<epoch>/segments/<segment_id>/{segment.json,vectors.f16}
//! <index>/epochs/<epoch>/manifests/<generation>.json
//! ```

// Every invariant test binary includes this module with `mod common`, and no
// binary uses every fixture: the two `INV-03` tests never read the manifest
// helpers, the no-feature `INV-04` build never reads the failpoint ones. Marking
// each unused item separately would only add noise to the test build, so the
// module allows dead code once, here. This is the only lint allowed in the test
// fixtures, and it never hides a lint in the code under test.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use crible_core::config::{ReferencePrecision, Schema};
use crible_core::ids::{Epoch, Generation, Iid};
use crible_store::generation::{GenerationChange, GenerationStore, Snapshot};
use crible_store::records::{DocumentRecord, PassageRecord};
use crible_store::segment::{SegmentMeta, SegmentSpec, SegmentWriter};
use serde_json::{Value, json};

/// Epoch created by every test.
pub const EPOCH: u32 = 1;

/// Dimensionality of the fixture vectors.
pub const DIMS: u32 = 4;

/// Creates an empty temporary directory and the path of the index inside it.
///
/// The index directory itself does not exist yet: `GenerationStore::create`
/// is expected to create it, as `RecordStore::create` does for its database.
pub fn temp_index() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| {
        panic!("a temporary directory for the index must be creatable: {error}")
    });
    let index = temp.path().join("indexes").join("invariants");
    (temp, index)
}

/// Directory of the single epoch created by the tests.
pub fn epoch_dir(index: &Path) -> PathBuf {
    index.join("epochs").join(EPOCH.to_string())
}

/// Path of the reference database of the epoch.
pub fn records_path(index: &Path) -> PathBuf {
    epoch_dir(index).join("records.sqlite")
}

/// Directory of the segments of the epoch.
pub fn segments_dir(index: &Path) -> PathBuf {
    epoch_dir(index).join("segments")
}

/// Directory of the derived manifests of the epoch.
pub fn manifests_dir(index: &Path) -> PathBuf {
    epoch_dir(index).join("manifests")
}

/// Path of the derived manifest of one generation.
pub fn manifest_path(index: &Path, generation: Generation) -> PathBuf {
    manifests_dir(index).join(format!("{}.json", generation.get()))
}

/// Path of the file naming the current epoch.
pub fn current_epoch_path(index: &Path) -> PathBuf {
    index.join("CURRENT_EPOCH")
}

/// Schema of the fixtures of this module.
///
/// One field per level, of the types the fixtures write: a document carries
/// `title` (a `keyword`), a passage carries `part` (an `integer`). A test that
/// indexes another schema passes it to [`create_store_with`].
pub fn fixture_schema() -> Schema {
    Schema::from_json_str(
        r#"{
            "document_fields": { "title": { "type": "keyword" } },
            "passage_fields": { "part": { "type": "integer" } }
        }"#,
    )
    .unwrap_or_else(|error| panic!("the fixture schema must be valid: {error}"))
}

/// Creates a generation store on an index that does not exist yet, indexing the
/// fields of [`fixture_schema`].
pub fn create_store(index: &Path) -> GenerationStore {
    create_store_with(index, &fixture_schema())
}

/// Creates a generation store whose filter structures follow `schema`.
pub fn create_store_with(index: &Path, schema: &Schema) -> GenerationStore {
    GenerationStore::create(index, Epoch::new(EPOCH), schema).unwrap_or_else(|error| {
        panic!(
            "the generation store must be created under {}: {error}",
            index.display()
        )
    })
}

/// Opens an existing index, which runs recovery, the way a restart does.
pub fn open_store(index: &Path) -> GenerationStore {
    open_store_with(index, &fixture_schema())
}

/// Opens an existing index whose filter structures follow `schema`.
pub fn open_store_with(index: &Path, schema: &Schema) -> GenerationStore {
    GenerationStore::open(index, schema).unwrap_or_else(|error| {
        panic!(
            "the generation store must reopen and recover {}: {error}",
            index.display()
        )
    })
}

/// Commits one generation and returns it, panicking if the commit fails.
pub fn commit(store: &mut GenerationStore, change: GenerationChange) -> Generation {
    store
        .commit(change)
        .unwrap_or_else(|error| panic!("the fixture commit must succeed: {error}"))
}

/// Writes a finished segment under `segments/<segment_id>/` and returns its
/// metadata, exactly as an attach does before the SQLite commit.
///
/// The writer performs the atomic write of the segment task: `tmp-<id>`,
/// fsync, then rename.
pub fn write_segment(
    index: &Path,
    segment_id: &str,
    iid_start: u32,
    generation: u64,
    vectors: &[&[f32]],
) -> SegmentMeta {
    let segments = segments_dir(index);
    fs::create_dir_all(&segments).unwrap_or_else(|error| {
        panic!(
            "the segments directory {} must be creatable: {error}",
            segments.display()
        )
    });
    let spec = SegmentSpec {
        segment_id: segment_id.to_owned(),
        iid_start: Iid::new(iid_start),
        dims: DIMS,
        generation: Generation::new(generation),
        batch_id: format!("batch-{segment_id}"),
        precision: ReferencePrecision::Fp16,
    };
    let mut writer = SegmentWriter::create(&segments, spec)
        .unwrap_or_else(|error| panic!("the segment writer of {segment_id} must start: {error}"));
    for vector in vectors {
        assert_eq!(
            vector.len(),
            DIMS as usize,
            "the fixture vectors must have {DIMS} components"
        );
        writer.append(vector).unwrap_or_else(|error| {
            panic!("the vector must be appended to segment {segment_id}: {error}")
        });
    }
    writer
        .finish()
        .unwrap_or_else(|error| panic!("segment {segment_id} must be finished: {error}"))
}

/// Builds one `documents` row at the given generation.
pub fn document(doc_id: &str, generation: u64) -> DocumentRecord {
    DocumentRecord {
        doc_id: doc_id.to_owned(),
        doc_hash: format!("blake3:doc-{doc_id}"),
        fields: json!({ "title": doc_id }),
        updated_generation: Generation::new(generation),
    }
}

/// Builds one `passages` row, alive, at the given generation.
pub fn passage(
    iid: u32,
    doc_id: &str,
    ord: u32,
    segment_id: &str,
    generation: u64,
) -> PassageRecord {
    PassageRecord {
        iid: Iid::new(iid),
        doc_id: doc_id.to_owned(),
        ord,
        text: format!("{doc_id}#{ord}"),
        passage_hash: format!("blake3:passage-{doc_id}-{ord}"),
        fields: json!({ "part": ord }),
        segment_id: segment_id.to_owned(),
        created_generation: Generation::new(generation),
        dead_since_gen: None,
    }
}

/// Builds the change committed by one generation.
pub fn change(
    segments: Vec<SegmentMeta>,
    documents: Vec<DocumentRecord>,
    passages: Vec<PassageRecord>,
    deletions: Vec<Iid>,
) -> GenerationChange {
    GenerationChange {
        segments,
        documents,
        passages,
        deletions,
    }
}

/// Asserts that a snapshot describes exactly one generation: the expected
/// generation number, exactly these segments, and exactly these live iids.
pub fn assert_view(snapshot: &Snapshot, generation: u64, segment_ids: &[&str], live_iids: &[u32]) {
    assert_eq!(
        snapshot.generation(),
        Generation::new(generation),
        "the snapshot must be taken on generation {generation}, \
         so a reader never mixes two generations"
    );

    let mut actual_segments: Vec<&str> = snapshot
        .segments()
        .iter()
        .map(|meta| meta.segment_id.as_str())
        .collect();
    actual_segments.sort_unstable();
    let mut expected_segments = segment_ids.to_vec();
    expected_segments.sort_unstable();
    assert_eq!(
        actual_segments, expected_segments,
        "the snapshot of generation {generation} must reference exactly these segments"
    );

    let mut actual_iids: Vec<u32> = snapshot.live_iids().map(|iid| iid.get()).collect();
    actual_iids.sort_unstable();
    let mut expected_iids = live_iids.to_vec();
    expected_iids.sort_unstable();
    assert_eq!(
        actual_iids, expected_iids,
        "the snapshot of generation {generation} must see exactly these live iids"
    );
}

/// Sorted names of the entries of `dir`, or an empty vector when it does not
/// exist yet (a fresh index may not have created every directory).
pub fn directory_entries(dir: &Path) -> Vec<String> {
    if !dir.exists() {
        return Vec::new();
    }
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("cannot list {}: {error}", dir.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| {
                    panic!("cannot read an entry of {}: {error}", dir.display())
                })
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort_unstable();
    names
}

/// Asserts that the segments directory holds exactly these directories: no
/// orphan and no leftover `tmp-*` survived recovery.
pub fn assert_segment_directories(index: &Path, expected: &[&str]) {
    let mut expected_sorted: Vec<String> = expected.iter().map(|id| (*id).to_owned()).collect();
    expected_sorted.sort_unstable();
    assert_eq!(
        directory_entries(&segments_dir(index)),
        expected_sorted,
        "recovery must keep exactly the segments referenced by SQLite: \
         an orphan or a tmp-* directory would be a mix of two generations (INV-04)"
    );
}

/// Temporary segment directories left under `segments/`.
pub fn temporary_segment_directories(index: &Path) -> Vec<String> {
    directory_entries(&segments_dir(index))
        .into_iter()
        .filter(|name| name.starts_with("tmp-"))
        .collect()
}

/// Reads `CURRENT_EPOCH` as a decimal number, ignoring trailing whitespace.
pub fn read_current_epoch(index: &Path) -> u32 {
    let path = current_epoch_path(index);
    let raw = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{} must exist after a commit or a recovery: {error}",
            path.display()
        )
    });
    raw.trim().parse::<u32>().unwrap_or_else(|error| {
        panic!(
            "{} must contain the decimal number of the current epoch, got {raw:?}: {error}",
            path.display()
        )
    })
}

/// Parses the derived manifest of one generation.
pub fn manifest(index: &Path, generation: Generation) -> Value {
    let path = manifest_path(index, generation);
    let raw = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "the manifest {} must exist, because recovery rewrites it from SQLite: {error}",
            path.display()
        )
    });
    serde_json::from_str(&raw).unwrap_or_else(|error| {
        panic!(
            "the manifest {} must be valid JSON, got {raw:?}: {error}",
            path.display()
        )
    })
}

/// Asserts that the manifest of `generation` describes that generation and
/// names exactly the expected segments.
///
/// The tests only rely on what `docs/02-architecture.md` promises: a manifest
/// is a derived view of one generation that names its segments. They therefore
/// look for the generation number under `generation` and for the segment
/// identifiers among the JSON strings, without fixing the exact shape. The
/// `segments` array must hold one entry per expected segment, so an empty
/// expectation is a real assertion too: the manifest of generation 0 must name
/// no segment at all.
pub fn assert_manifest_describes(index: &Path, generation: u64, segment_ids: &[&str]) {
    let value = manifest(index, Generation::new(generation));
    assert_eq!(
        value.get("generation").and_then(Value::as_u64),
        Some(generation),
        "the manifest of generation {generation} must name it: {value}"
    );

    let listed = value
        .get("segments")
        .and_then(Value::as_array)
        .unwrap_or_else(|| {
            panic!("the manifest of generation {generation} must hold a segments array: {value}")
        });
    assert_eq!(
        listed.len(),
        segment_ids.len(),
        "the manifest of generation {generation} must name exactly {} segment(s), \
         so an empty list means no segment at all: {value}",
        segment_ids.len()
    );

    let mut strings = Vec::new();
    collect_strings(&value, &mut strings);
    for segment_id in segment_ids {
        assert!(
            strings.iter().any(|candidate| candidate == segment_id),
            "the manifest of generation {generation} must name segment {segment_id}: {value}"
        );
    }
}

/// Collects every string of a JSON value, recursively.
fn collect_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => out.push(text.clone()),
        Value::Array(items) => {
            for item in items {
                collect_strings(item, out);
            }
        }
        Value::Object(entries) => {
            for (key, item) in entries {
                out.push(key.clone());
                collect_strings(item, out);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// Serialises the tests of one binary that touch `CRIBLE_FAILPOINT`.
///
/// `CRIBLE_FAILPOINT` is read by the store when it reaches a step, so the
/// variable is process-global: a test that arms a failpoint and a test that
/// commits normally would observe each other's value, and cargo runs the
/// tests of one binary concurrently. Every test of a binary that commits must
/// therefore hold this guard, not only the tests that arm a failpoint.
pub struct SerialGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl SerialGuard {
    /// Waits for the other committing tests of this binary to finish.
    ///
    /// A poison is expected and ignored: a test that arms a failpoint panics
    /// on purpose while holding the guard, so the mutex is poisoned even
    /// though the simulated crash is the normal outcome of that test.
    pub fn acquire() -> Self {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let lock = match LOCK.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        Self { _lock: lock }
    }
}

/// Guard that arms one failpoint and serialises every fault-injection test of
/// the binary.
#[cfg(feature = "failpoints")]
pub struct FailpointGuard {
    _serial: SerialGuard,
}

#[cfg(feature = "failpoints")]
impl FailpointGuard {
    /// Name of the environment variable read by the store.
    pub const ENV: &'static str = "CRIBLE_FAILPOINT";

    /// Arms `step` for the current thread of the test binary.
    pub fn arm(step: &str) -> Self {
        let serial = SerialGuard::acquire();
        // SAFETY: `SerialGuard` serialises every test of this binary that
        // reads or writes `CRIBLE_FAILPOINT`, and no other thread of the
        // process calls `std::env` concurrently.
        unsafe { std::env::set_var(Self::ENV, step) };
        Self { _serial: serial }
    }

    /// Name of the armed step, for assertion messages.
    pub fn step(&self) -> String {
        std::env::var(Self::ENV).unwrap_or_default()
    }
}

#[cfg(feature = "failpoints")]
impl Drop for FailpointGuard {
    fn drop(&mut self) {
        // SAFETY: see `FailpointGuard::arm`; the lock is still held here.
        unsafe { std::env::remove_var(Self::ENV) };
    }
}

/// Runs a commit that must panic at the armed failpoint, and checks that the
/// panic really comes from the failpoint and not from another bug.
#[cfg(feature = "failpoints")]
pub fn commit_expecting_failpoint(store: &mut GenerationStore, change: GenerationChange) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.commit(change)));
    match outcome {
        Ok(Ok(generation)) => panic!(
            "CRIBLE_FAILPOINT was armed but the commit completed generation {}; \
             the failpoint is not implemented",
            generation.get()
        ),
        Ok(Err(error)) => panic!(
            "the commit returned {error} instead of hitting the armed failpoint; \
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
                "the commit panicked, but not at a failpoint: {message}"
            );
        }
    }
}
