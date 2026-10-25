// SPDX-License-Identifier: Apache-2.0

//! INV-04 — after a restart, the state is the last committed generation;
//! everything else is rebuilt or cleaned. INV-03 — a generation is visible
//! entirely or not at all.
//!
//! References: `docs/08-invariants-et-tests.md` (INV-03, INV-04 and the
//! "Pannes" row), `docs/02-architecture.md` ("Générations et point de commit",
//! "Reprise au démarrage", "Suppressions, pierres tombales et compaction"),
//! ADR-0016 (the SQLite commit is the only commit point), ADR-0017 (a compaction
//! suspends the writer, never the readers) and ADR-0065 (recovery does not touch
//! the `epochs/` directory beyond the current epoch), and the
//! `TESTER-REPRISE-APRES-PANNE` task of `docs/phases/P3-cycle-de-vie.md`.
//!
//! # What is simulated
//!
//! Every injection point of the attach (steps 1 to 6 of `docs/02`) and of the
//! compaction (steps 1 to 6) is armed through `CRIBLE_FAILPOINT=<step>`, then the
//! operation runs in a **child process** which **kills itself with `SIGKILL`**
//! from its panic hook as soon as the store panics at the armed step. Killing
//! the process, instead of letting the panic unwind, is the safe choice: an
//! unwind would run `Drop` and roll the SQLite transaction back in the same
//! process, so it would not simulate a process death. A `SIGKILL` gives the
//! kernel no chance to flush anything, which is exactly what the restart must
//! survive.
//!
//! The child is this same test binary, re-executed with
//! `--exact crash_child_entry --ignored`; `CRIBLE_CRASH_CHILD` selects the
//! scenario and `CRIBLE_CRASH_INDEX` names the index. The parent waits for the
//! child to end and requires `SIGKILL`; a missing injection point makes the
//! child complete the operation and exit with code 41, so the test fails with
//! the name of the point and of the step to implement. No test sleeps.
//!
//! # Injection points
//!
//! | Protocol | Step | `CRIBLE_FAILPOINT` | Position |
//! |---|---|---|---|
//! | attach | 1 | `attach_validated` | validation and acceptance done, nothing written |
//! | attach | 2 | `after_attach_segment` | segment fsynced and renamed, transaction not open |
//! | attach | 3 | `before_sqlite_commit` | rows written, before `COMMIT` |
//! | attach | 4 | `after_sqlite_commit` | right after `COMMIT`; lexical is not in P3 |
//! | attach | 5 | `after_manifest` | `manifests/<G+1>.json` written |
//! | attach | 6 | `after_publish` | snapshot published |
//! | compaction | 1 | `compaction_writer_suspended` | writer suspended, nothing read yet |
//! | compaction | 2 | `compaction_passages_read` | live passages read and ordered |
//! | compaction | 3 | `compaction_epoch_written` | E+1 renamed into `epochs/<E+1>`, not verified |
//! | compaction | 4 | `before_epoch_switch` | E+1 verified, `CURRENT_EPOCH` still E |
//! | compaction | 5 | `after_epoch_switch` | `CURRENT_EPOCH` is E+1, nothing published |
//! | compaction | 6 | `after_compaction_publish` | snapshot of E+1 published, E not reclaimed |
//!
//! Attach step 4 is a lexical step that does not exist in P3 (`crible-store` has
//! no tantivy yet, see `docs/02`); its crash state is the one right after the
//! commit, so the existing `after_sqlite_commit` point stands at the start of
//! that step. The two existing attach points `after_attach_documents` and
//! `after_attach_passages` refine step 3 and are covered by `attacher_lot.rs`.
//!
//! # Orphans
//!
//! `docs/02` ("Reprise au démarrage") defines what recovery removes: every
//! `tmp-*` entry of the index directory, of `segments/` and of `manifests/`, and
//! every segment directory SQLite does not reference. `epochs/tmp-*` and a
//! complete epoch `CURRENT_EPOCH` does not name are *not* removed by recovery
//! (ADR-0065); the next compaction or `reclaim_epochs` removes them, and the
//! tests assert that outcome after the replay.
//!
//! Run this file with:
//!
//! ```text
//! cargo test -p crible-store --features failpoints --test inv_04_crash_recovery
//! ```

#![cfg(all(feature = "failpoints", unix))]
// The fixtures of this file build an index and call `panic!` outside the
// `#[test]` functions; the workspace lint allows a panic in a test function
// only, as in `common`.
#![allow(clippy::panic)]

mod common;

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::Arc;
use std::time::Duration;

use arrow::array::{ArrayRef, FixedSizeBinaryArray, Int64Array, StringArray, UInt32Array};
use arrow::datatypes::{DataType, Field, Schema as ArrowSchema, SchemaRef};
use arrow::record_batch::RecordBatch;
use crible_core::config::IndexConfig;
use crible_core::ids::{Epoch, Iid};
use crible_core::model::ModelConfig;
use crible_store::attach::{AttachReport, attach};
use crible_store::compact::{CompactReport, compact, reclaim_epochs};
use half::f16;
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

// --- Child process protocol ---

/// Failpoint variable read by `crible-store` in a build with `failpoints`.
const FAILPOINT_ENV: &str = "CRIBLE_FAILPOINT";

/// Scenario a child must run: `attach` or `compaction`.
const CHILD_ENV: &str = "CRIBLE_CRASH_CHILD";

/// Index the child works on.
const INDEX_ENV: &str = "CRIBLE_CRASH_INDEX";

/// Batch the child attaches, for the attach scenario only.
const BATCH_ENV: &str = "CRIBLE_CRASH_BATCH";

/// Name of the child entry point, as libtest lists it.
const CHILD_TEST: &str = "crash_child_entry";

const ATTACH_SCENARIO: &str = "attach";
const COMPACTION_SCENARIO: &str = "compaction";

/// Exit code of a child whose operation completed: the failpoint is missing.
const EXIT_FAILPOINT_MISSING: i32 = 41;

/// Exit code of a child whose operation returned an error before the failpoint.
const EXIT_OPERATION_FAILED: i32 = 42;

/// Number of `SIGKILL` on Unix.
const SIGKILL: i32 = 9;

/// Outcome of one child run, with the text it printed for the failure message.
struct ChildOutcome {
    status: ExitStatus,
    stderr: String,
}

/// What a crash must leave behind: exactly one of the two complete states.
struct ExpectedState<'a> {
    epoch: u32,
    generation: u64,
    segments: &'a [&'a str],
    live_iids: &'a [u32],
    documents: u64,
    passages: u64,
}

fn attach_old_state() -> ExpectedState<'static> {
    ExpectedState {
        epoch: common::EPOCH,
        generation: 0,
        segments: &[],
        live_iids: &[],
        documents: 0,
        passages: 0,
    }
}

fn attach_new_state() -> ExpectedState<'static> {
    ExpectedState {
        epoch: common::EPOCH,
        generation: 1,
        segments: &["seg-crash-attach"],
        live_iids: &[0, 1],
        documents: 2,
        passages: 2,
    }
}

fn compaction_old_state() -> ExpectedState<'static> {
    ExpectedState {
        epoch: common::EPOCH,
        generation: 2,
        segments: &["seg-a"],
        live_iids: &[1],
        documents: 2,
        passages: 2,
    }
}

fn compaction_new_state() -> ExpectedState<'static> {
    ExpectedState {
        epoch: common::EPOCH + 1,
        generation: 3,
        segments: &["c2-0000000000"],
        live_iids: &[0],
        documents: 2,
        passages: 1,
    }
}

/// Injection point of each attach step of `docs/02`, and whether the new
/// generation is already committed when the process dies there.
const ATTACH_POINTS: [(&str, bool); 6] = [
    ("attach_validated", false),
    ("after_attach_segment", false),
    ("before_sqlite_commit", false),
    ("after_sqlite_commit", true),
    ("after_manifest", true),
    ("after_publish", true),
];

// --- Fixtures ---

/// Index configuration shared by the parent and every child.
fn fixture_config() -> IndexConfig {
    let schema = common::fixture_schema();
    IndexConfig::from_json_str(&common::test_config("test", &schema))
        .unwrap_or_else(|error| panic!("the fixture configuration must be valid: {error}"))
}

/// External model of the fixture batch: four dimensions, normalized vectors.
fn fixture_model() -> ModelConfig {
    ModelConfig::from_json_str(
        r#"{ "name": "crash-recovery-external", "kind": "external", "dims": 4, "normalize": true }"#,
    )
    .unwrap_or_else(|error| panic!("the fixture model configuration must be valid: {error}"))
}

/// Creates an empty index at generation 0 and one batch that adds two documents
/// with one passage each.
fn attach_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let (temp, index) = common::temp_index();
    drop(common::create_store(&index));
    let batch = write_batch(&temp.path().join("batches"), "crash-attach");
    (temp, index, batch)
}

/// Creates an index at generation 2: two documents and two passages at
/// generation 1, then the tombstone of iid 0 at generation 2.
fn compaction_fixture() -> (tempfile::TempDir, PathBuf) {
    let (temp, index) = common::temp_index();
    let mut store = common::create_store(&index);
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
    common::commit(
        &mut store,
        common::change(Vec::new(), Vec::new(), Vec::new(), vec![Iid::new(0)]),
    );
    drop(store);
    (temp, index)
}

// --- Batch writer (4 dimensions, schema of `common::fixture_schema`) ---

fn document_schema() -> SchemaRef {
    Arc::new(ArrowSchema::new(vec![
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("doc_hash", DataType::Utf8, false),
        Field::new("title", DataType::Utf8, true),
    ]))
}

fn passage_schema() -> SchemaRef {
    Arc::new(ArrowSchema::new(vec![
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("ord", DataType::UInt32, false),
        Field::new("text", DataType::Utf8, false),
        Field::new("part", DataType::Int64, true),
        Field::new("vector", DataType::FixedSizeBinary(8), false),
    ]))
}

fn fp16(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| f16::from_f32(*value).to_bits().to_le_bytes())
        .collect()
}

fn write_parquet(path: &Path, schema: &SchemaRef, columns: Vec<ArrayRef>) {
    let batch = RecordBatch::try_new(schema.clone(), columns)
        .unwrap_or_else(|error| panic!("the fixture batch must be valid: {error}"));
    let file = File::create(path)
        .unwrap_or_else(|error| panic!("the fixture file must be created: {error}"));
    let properties = WriterProperties::builder().build();
    let mut writer = ArrowWriter::try_new(file, schema.clone(), Some(properties))
        .unwrap_or_else(|error| panic!("the fixture writer must open: {error}"));
    writer
        .write(&batch)
        .unwrap_or_else(|error| panic!("the fixture rows must be written: {error}"));
    writer
        .close()
        .unwrap_or_else(|error| panic!("the fixture file must close: {error}"));
}

/// Writes a batch of two documents and two passages, each passage covering a
/// distinct document, and its manifest.
fn write_batch(root: &Path, batch_id: &str) -> PathBuf {
    let dir = root.join(batch_id);
    fs::create_dir_all(&dir).unwrap_or_else(|error| panic!("the batch directory: {error}"));

    let ids: StringArray = ["doc-a", "doc-b"].iter().map(|id| Some(*id)).collect();
    let hashes: StringArray = ["blake3:doc-a", "blake3:doc-b"]
        .iter()
        .map(|hash| Some(*hash))
        .collect();
    let titles: StringArray = ["doc-a", "doc-b"]
        .iter()
        .map(|title| Some(*title))
        .collect();
    write_parquet(
        &dir.join("documents.parquet"),
        &document_schema(),
        vec![Arc::new(ids), Arc::new(hashes), Arc::new(titles)],
    );

    let passage_ids: StringArray = ["doc-a", "doc-b"].iter().map(|id| Some(*id)).collect();
    let ords = UInt32Array::from(vec![0_u32, 0_u32]);
    let texts: StringArray = ["doc-a#0", "doc-b#0"]
        .iter()
        .map(|text| Some(*text))
        .collect();
    let parts = Int64Array::from(vec![Some(0_i64), Some(1_i64)]);
    let vectors = FixedSizeBinaryArray::try_from_iter(
        [fp16(&[1.0, 0.0, 0.0, 0.0]), fp16(&[0.0, 1.0, 0.0, 0.0])].into_iter(),
    )
    .unwrap_or_else(|error| panic!("the fixture vectors must share one width: {error}"));
    write_parquet(
        &dir.join("passages.parquet"),
        &passage_schema(),
        vec![
            Arc::new(passage_ids),
            Arc::new(ords),
            Arc::new(texts),
            Arc::new(parts),
            Arc::new(vectors),
        ],
    );

    write_manifest(&dir, batch_id);
    dir
}

fn write_manifest(dir: &Path, batch_id: &str) {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("the batch directory must be readable: {error}"))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("a batch entry must be readable: {error}"))
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with(".parquet"))
        .collect();
    names.sort();

    let mut files = BTreeMap::new();
    for name in names {
        let bytes = fs::read(dir.join(&name))
            .unwrap_or_else(|error| panic!("the batch file {name} must be readable: {error}"));
        files.insert(name, format!("blake3:{}", blake3::hash(&bytes).to_hex()));
    }

    let manifest = serde_json::json!({
        "format": "crible-batch",
        "format_version": 1,
        "batch_id": batch_id,
        "index": "test",
        "mode": "upsert",
        "created_at": "2026-01-01T00:00:00Z",
        "producer": { "name": "test", "version": "1", "host": "test" },
        "model": { "dims": 4, "normalize": true, "vector_dtype": "fp16" },
        "chunker": { "name": "test", "version": "1", "params": {} },
        "template": { "name": "test", "version": "1" },
        "counts": { "documents": 2, "passages": 2, "deletions": 0 },
        "files": files,
    });
    fs::write(
        dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)
            .unwrap_or_else(|error| panic!("the manifest must be serializable: {error}")),
    )
    .unwrap_or_else(|error| panic!("manifest.json must be written: {error}"));
}

// --- Child entry point ---

#[test]
#[ignore = "child process of the crash tests; the parent arms CRIBLE_FAILPOINT and spawns it"]
fn crash_child_entry() {
    let scenario = std::env::var(CHILD_ENV).unwrap_or_else(|_| {
        panic!(
            "{CHILD_ENV} is not set: this is the child entry point of the crash tests and must \
             not run on its own. Run the parent tests with:\n  cargo test -p crible-store \
             --features failpoints --test inv_04_crash_recovery"
        )
    });
    install_sigkill_hook();

    let index = PathBuf::from(
        std::env::var_os(INDEX_ENV)
            .unwrap_or_else(|| panic!("{INDEX_ENV} must name the index the child works on")),
    );

    let run = match scenario.as_str() {
        ATTACH_SCENARIO => run_attach(&index),
        COMPACTION_SCENARIO => run_compaction(&index),
        other => panic!("unknown {CHILD_ENV}={other}"),
    };
    let step = std::env::var(FAILPOINT_ENV).unwrap_or_default();
    match run {
        ChildRun::Completed {
            operation,
            generation,
        } => {
            eprintln!(
                "CRIBLE_CRASH_CHILD: the {operation} completed generation {generation} without \
                 reaching CRIBLE_FAILPOINT={step}; the failpoint is missing or unreachable"
            );
            std::process::exit(EXIT_FAILPOINT_MISSING);
        }
        ChildRun::Failed { operation, error } => {
            eprintln!(
                "CRIBLE_CRASH_CHILD: the {operation} returned an error before reaching \
                 CRIBLE_FAILPOINT={step}: {error}"
            );
            std::process::exit(EXIT_OPERATION_FAILED);
        }
    }
}

enum ChildRun {
    Completed {
        operation: &'static str,
        generation: u64,
    },
    Failed {
        operation: &'static str,
        error: String,
    },
}

fn run_attach(index: &Path) -> ChildRun {
    let batch = PathBuf::from(
        std::env::var_os(BATCH_ENV)
            .unwrap_or_else(|| panic!("{BATCH_ENV} must name the batch the child attaches")),
    );
    let config = fixture_config();
    let model = fixture_model();
    let mut store = common::open_store(index);
    match attach(&mut store, &batch, &config, &model, None, None) {
        Ok(report) => ChildRun::Completed {
            operation: "attach",
            generation: report.generation.get(),
        },
        Err(error) => ChildRun::Failed {
            operation: "attach",
            error: format!("{}: {}", error.code().as_str(), error.message()),
        },
    }
}

fn run_compaction(index: &Path) -> ChildRun {
    let config = fixture_config();
    let mut store = common::open_store(index);
    match compact(&mut store, &config) {
        Ok(report) => ChildRun::Completed {
            operation: "compaction",
            generation: report.generation.get(),
        },
        Err(error) => ChildRun::Failed {
            operation: "compaction",
            error: format!("{}: {}", error.code().as_str(), error.message()),
        },
    }
}

/// Replaces the default panic hook: a panic raised by a failpoint kills the
/// process with `SIGKILL` before any unwind runs, so nothing is flushed and no
/// SQLite transaction is rolled back by `Drop`. Any other panic keeps the
/// default behaviour.
fn install_sigkill_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .map(|text| (*text).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        if message.contains("CRIBLE_FAILPOINT") {
            eprintln!("CRIBLE_CRASH_CHILD: reached the failpoint, dying by SIGKILL: {message}");
            kill_own_process();
        }
        previous(info);
    }));
}

/// Sends `SIGKILL` to this process. `kill` is tried by absolute path first,
/// then through `PATH`; `abort` is only a safety net.
fn kill_own_process() {
    let pid = std::process::id().to_string();
    for program in ["/bin/kill", "kill"] {
        let _ = Command::new(program).arg("-9").arg(&pid).status();
    }
    std::process::abort();
}

// --- Parent side ---

/// Runs the child on `index`, arms `step` and waits for it.
fn spawn_child(step: &str, scenario: &str, index: &Path, batch: Option<&Path>) -> ChildOutcome {
    let executable = std::env::current_exe()
        .unwrap_or_else(|error| panic!("the test binary path must be known: {error}"));
    let mut command = Command::new(executable);
    command
        .arg("--exact")
        .arg(CHILD_TEST)
        .arg("--ignored")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(CHILD_ENV, scenario)
        .env(FAILPOINT_ENV, step)
        .env(INDEX_ENV, index)
        .env_remove(BATCH_ENV);
    if let Some(batch) = batch {
        command.env(BATCH_ENV, batch);
    }

    let output = command
        .output()
        .unwrap_or_else(|error| panic!("the crash child must be spawnable: {error}"));
    ChildOutcome {
        status: output.status,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Requires the child to have died by `SIGKILL`, and explains a missing point.
fn assert_child_was_killed(step: &str, outcome: &ChildOutcome) {
    use std::os::unix::process::ExitStatusExt;
    if outcome.status.signal() == Some(SIGKILL) {
        return;
    }
    panic!(
        "CRIBLE_FAILPOINT={step}: the child did not die by SIGKILL (status {:?}). The injection \
         point '{step}' is missing or unreachable: add `stop_at_failpoint(\"{step}\")` at that \
         step of `docs/02-architecture.md` (`crible-store::attach` or `crible-store::compact`).\n\
         child stderr:\n{}",
        outcome.status, outcome.stderr
    );
}

/// Reads the reference state of one epoch: document rows, passage rows and live
/// passages.
fn records_counts(epoch_dir: &Path) -> (u64, u64, u64) {
    let path = epoch_dir.join("records.sqlite");
    let connection = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
    connection
        .busy_timeout(Duration::from_millis(1_000))
        .unwrap_or_else(|error| panic!("the busy timeout must be settable: {error}"));
    let count = |sql: &str| -> u64 {
        let value: i64 = connection
            .query_row(sql, [], |row| row.get(0))
            .unwrap_or_else(|error| panic!("the count '{sql}' must run: {error}"));
        u64::try_from(value).unwrap_or_else(|_| panic!("the count '{sql}' must not be negative"))
    };
    (
        count("SELECT COUNT(*) FROM documents"),
        count("SELECT COUNT(*) FROM passages"),
        count("SELECT COUNT(*) FROM passages WHERE dead_since_gen IS NULL"),
    )
}

fn directories(dir: &Path) -> Vec<String> {
    common::directory_entries(dir)
}

/// Parses the manifest of `generation` in `epoch_dir` and checks it names the
/// generation and exactly these segments.
fn assert_manifest(epoch_dir: &Path, generation: u64, segments: &[&str]) {
    let path = epoch_dir
        .join("manifests")
        .join(format!("{generation}.json"));
    let raw = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "the manifest {} must exist after a restart, because recovery rewrites it from \
             SQLite: {error}",
            path.display()
        )
    });
    let value: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("{} must be valid JSON: {error}", path.display()));
    assert_eq!(
        value.get("generation").and_then(Value::as_u64),
        Some(generation),
        "the manifest of generation {generation} must name it: {value}"
    );
    let listed = value
        .get("segments")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("the manifest must hold a segments array: {value}"));
    assert_eq!(
        listed.len(),
        segments.len(),
        "the manifest of generation {generation} must name exactly {} segment(s): {value}",
        segments.len()
    );
    let mut names: Vec<&str> = listed
        .iter()
        .filter_map(|entry| entry.get("segment_id").and_then(Value::as_str))
        .collect();
    names.sort_unstable();
    let mut expected = segments.to_vec();
    expected.sort_unstable();
    assert_eq!(
        names, expected,
        "the manifest of generation {generation} must name the segments of that generation"
    );
}

/// Asserts the state a restart exposes: the current epoch, the generation, the
/// segments, the live iids (the pool) and the reference counts. Exactly one
/// complete generation, never a mixture.
fn assert_index_state(index: &Path, expected: &ExpectedState) {
    assert_eq!(
        common::read_current_epoch(index),
        expected.epoch,
        "CURRENT_EPOCH must name the epoch of the committed generation"
    );

    {
        let store = common::open_store(index);
        assert_eq!(
            store.epoch().get(),
            expected.epoch,
            "the reopened store must hold the epoch CURRENT_EPOCH names"
        );
        common::assert_view(
            &store.snapshot(),
            expected.generation,
            expected.segments,
            expected.live_iids,
        );
    }

    let epoch_dir = index.join("epochs").join(expected.epoch.to_string());
    let (documents, passages, live) = records_counts(&epoch_dir);
    assert_eq!(
        documents, expected.documents,
        "the reference state of generation {} must hold exactly {} document row(s)",
        expected.generation, expected.documents
    );
    assert_eq!(
        passages, expected.passages,
        "the reference state of generation {} must hold exactly {} passage row(s)",
        expected.generation, expected.passages
    );
    assert_eq!(
        live,
        expected.live_iids.len() as u64,
        "the reference state of generation {} must hold exactly {} live passage(s)",
        expected.generation,
        expected.live_iids.len()
    );

    assert_manifest(&epoch_dir, expected.generation, expected.segments);
    assert_eq!(
        directories(&epoch_dir.join("segments")),
        {
            let mut expected_names: Vec<String> = expected
                .segments
                .iter()
                .map(|id| (*id).to_owned())
                .collect();
            expected_names.sort_unstable();
            expected_names
        },
        "the segments of the current epoch must be exactly the ones SQLite references: an \
         orphan or a tmp-* directory would be a mixture of two generations (INV-04)"
    );
}

/// Asserts no `tmp-*` entry survives in the index directory, any epoch directory
/// or its `segments/` and `manifests/` directories.
fn assert_no_temporary_entries(index: &Path) {
    let mut checked = vec![index.to_path_buf()];
    let epochs = index.join("epochs");
    for epoch in directories(&epochs) {
        let epoch_dir = epochs.join(&epoch);
        checked.push(epoch_dir.join("segments"));
        checked.push(epoch_dir.join("manifests"));
        checked.push(epoch_dir);
    }
    for dir in checked {
        for name in directories(&dir) {
            assert!(
                !name.starts_with("tmp-"),
                "a tmp-* entry survived the restart in {}: {name}",
                dir.display()
            );
        }
    }
}

/// Asserts the epoch directories that remain: the current one, and no other.
fn assert_epochs(index: &Path, expected: &[u32]) {
    let mut actual = directories(&index.join("epochs"));
    let mut expected_names: Vec<String> = expected.iter().map(u32::to_string).collect();
    actual.sort_unstable();
    expected_names.sort_unstable();
    assert_eq!(
        actual, expected_names,
        "after the replay, exactly the expected epoch directories must remain"
    );
}

// --- Replays ---

fn replay_attach(index: &Path, batch: &Path) -> AttachReport {
    let config = fixture_config();
    let model = fixture_model();
    let mut store = common::open_store(index);
    attach(&mut store, batch, &config, &model, None, None).unwrap_or_else(|error| {
        panic!(
            "replaying the batch after the crash must converge: {}: {}",
            error.code().as_str(),
            error.message()
        )
    })
}

fn replay_compaction(index: &Path) -> CompactReport {
    let config = fixture_config();
    let mut store = common::open_store(index);
    compact(&mut store, &config).unwrap_or_else(|error| {
        panic!(
            "replaying the compaction after the crash must converge: {}: {}",
            error.code().as_str(),
            error.message()
        )
    })
}

/// Reclaims the epoch a compaction replaced, as `docs/02` says an operator (or
/// the next compaction) does after a crash past the switch.
fn reclaim_replaced_epoch(index: &Path) -> Vec<Epoch> {
    let store = common::open_store(index);
    reclaim_epochs(&store)
        .unwrap_or_else(|error| panic!("the replaced epoch must be reclaimable: {error}"))
}

// --- Attach: crash at each step ---

fn crash_attach_at(step: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let (temp, index, batch) = attach_fixture();
    let outcome = spawn_child(step, ATTACH_SCENARIO, &index, Some(&batch));
    assert_child_was_killed(step, &outcome);
    (temp, index, batch)
}

fn assert_attach_crash(step: &str, expected: &ExpectedState) {
    let (_temp, index, batch) = crash_attach_at(step);
    assert_index_state(&index, expected);

    let report = replay_attach(&index, &batch);
    assert_eq!(
        report.generation.get(),
        attach_new_state().generation,
        "replaying the batch after a crash at {step} must converge to G+1"
    );
    assert_index_state(&index, &attach_new_state());
    assert_epochs(&index, &[common::EPOCH]);
    assert_no_temporary_entries(&index);
}

#[test]
fn inv_04_crash_after_attach_validation_keeps_old_generation() {
    assert_attach_crash("attach_validated", &attach_old_state());
}

#[test]
fn inv_04_crash_after_attach_segment_keeps_old_generation() {
    assert_attach_crash("after_attach_segment", &attach_old_state());
}

#[test]
fn inv_04_crash_before_sqlite_commit_keeps_old_generation() {
    assert_attach_crash("before_sqlite_commit", &attach_old_state());
}

#[test]
fn inv_04_crash_after_sqlite_commit_keeps_new_generation() {
    assert_attach_crash("after_sqlite_commit", &attach_new_state());
}

#[test]
fn inv_04_crash_after_manifest_keeps_new_generation() {
    assert_attach_crash("after_manifest", &attach_new_state());
}

#[test]
fn inv_04_crash_after_publish_keeps_new_generation() {
    assert_attach_crash("after_publish", &attach_new_state());
}

/// INV-03 at restart: whatever attach step the process dies at, the reopened
/// index exposes generation G or G+1 in full. The pair (document rows, passage
/// rows) is (0, 0) at G and (2, 2) at G+1; anything else is half a generation.
#[test]
fn inv_03_a_crashed_attach_never_exposes_half_a_generation() {
    for (step, committed) in ATTACH_POINTS {
        let (_temp, index, _batch) = crash_attach_at(step);
        let expected = if committed {
            attach_new_state()
        } else {
            attach_old_state()
        };
        assert_index_state(&index, &expected);

        let epoch_dir = index.join("epochs").join(common::EPOCH.to_string());
        let (documents, passages, live) = records_counts(&epoch_dir);
        assert!(
            (documents, passages, live) == (0, 0, 0) || (documents, passages, live) == (2, 2, 2),
            "a crash at {step} exposed {documents} document(s), {passages} passage(s) and \
             {live} live passage(s): that is half a generation, not G nor G+1 (INV-03)"
        );
        assert_no_temporary_entries(&index);
    }
}

// --- Compaction: crash at each step ---

/// Runs the compaction crash test.
///
/// `replay` is true when the crash leaves the old generation current, so the
/// compaction must be replayed to converge to G+1. At `after_epoch_switch` and
/// `after_compaction_publish` the new generation is already committed, so a
/// replay would build G+2: the test only reclaims the replaced epoch, which is
/// what `docs/02` leaves to the next operation.
fn assert_compaction_crash(step: &str, expected: &ExpectedState, replay: bool) {
    let (_temp, index) = compaction_fixture();
    let outcome = spawn_child(step, COMPACTION_SCENARIO, &index, None);
    assert_child_was_killed(step, &outcome);
    assert_index_state(&index, expected);

    if replay {
        let report = replay_compaction(&index);
        assert_eq!(
            report.generation.get(),
            compaction_new_state().generation,
            "replaying the compaction after a crash at {step} must converge to G+1"
        );
    } else {
        let reclaimed = reclaim_replaced_epoch(&index);
        assert_eq!(
            reclaimed,
            vec![Epoch::new(common::EPOCH)],
            "the epoch the compaction replaced must be reclaimable after a crash at {step}"
        );
    }

    assert_index_state(&index, &compaction_new_state());
    assert_epochs(&index, &[common::EPOCH + 1]);
    assert_no_temporary_entries(&index);
}

#[test]
fn inv_04_crash_after_compaction_writer_suspended_keeps_old_generation() {
    assert_compaction_crash("compaction_writer_suspended", &compaction_old_state(), true);
}

#[test]
fn inv_04_crash_after_compaction_passages_read_keeps_old_generation() {
    assert_compaction_crash("compaction_passages_read", &compaction_old_state(), true);
}

#[test]
fn inv_04_crash_after_compaction_epoch_written_keeps_old_generation() {
    assert_compaction_crash("compaction_epoch_written", &compaction_old_state(), true);
}

#[test]
fn inv_04_crash_before_epoch_switch_keeps_old_generation() {
    assert_compaction_crash("before_epoch_switch", &compaction_old_state(), true);
}

#[test]
fn inv_04_crash_after_epoch_switch_keeps_new_generation() {
    assert_compaction_crash("after_epoch_switch", &compaction_new_state(), false);
}

#[test]
fn inv_04_crash_after_compaction_publish_keeps_new_generation() {
    assert_compaction_crash("after_compaction_publish", &compaction_new_state(), false);
}
