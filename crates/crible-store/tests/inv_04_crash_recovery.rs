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
//! | attach | 4 | `after_sqlite_commit` | right after `COMMIT`, before the lexical index is written |
//! | attach | 4 | `after_lexical_commit` | tantivy committed `G+1` and the reader is reloaded |
//! | attach | 5 | `after_manifest` | `manifests/<G+1>.json` written |
//! | attach | 6 | `after_publish` | snapshot published |
//! | compaction | 1 | `compaction_writer_suspended` | writer suspended, nothing read yet |
//! | compaction | 2 | `compaction_passages_read` | live passages read and ordered |
//! | compaction | 3 | `compaction_epoch_written` | E+1 renamed into `epochs/<E+1>`, not verified |
//! | compaction | 4 | `before_epoch_switch` | E+1 verified, `CURRENT_EPOCH` still E |
//! | compaction | 5 | `after_epoch_switch` | `CURRENT_EPOCH` is E+1, nothing published |
//! | compaction | 6 | `after_compaction_publish` | snapshot of E+1 published, E not reclaimed |
//!
//! Step 4 is the lexical step of `docs/02`. It spans two points, because a
//! crash inside it and a crash after it leave different states: at
//! `after_sqlite_commit` the generation is committed and tantivy still names
//! `G`, so the restart must catch the lexical index up (the `Gt < G` branch of
//! "Reprise au démarrage"); at `after_lexical_commit` tantivy has committed the
//! batch of `G+1` with `G+1` as its payload and the reader is reloaded, so the
//! restart has a lexical index that already names `G+1`. A generation that
//! carries no attach (a tombstone-only change, or a `replace_all`) walks the
//! same steps 3 to 6, so the deletion scenario below arms the same two points.
//! The two existing attach points `after_attach_documents` and
//! `after_attach_passages` refine step 3 and are covered by `attacher_lot.rs`.
//!
//! # The lexical index of a generation
//!
//! INV-04 covers the derived lexical index like every other derived structure:
//! after a restart, the payload of its last commit is the committed generation
//! and the iids it holds are exactly the live passages of that generation. The
//! tests read that state through [`crible_store::lexical::read_state`], which
//! opens the index of one epoch read-only; a state that still names an older
//! generation, or that holds an iid the generation has no live passage for, is
//! the failure the invariant forbids.
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

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::Duration;

use crible_core::config::IndexConfig;
use crible_core::ids::{Epoch, Generation, Iid};
use crible_core::model::ModelConfig;
use crible_store::attach::{AttachReport, attach};
use crible_store::compact::{CompactReport, compact, reclaim_epochs};
use crible_store::lexical::read_state;
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
/// A generation that only tombstones and appends, without attaching a batch.
const DELETION_SCENARIO: &str = "deletion";

/// Segment the deletion scenario appends: one passage, at iid 2.
const DELETION_SEGMENT: &str = "seg-crash-deletion";

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
    /// Expected lexical state: `None` for an index that disables the lexical
    /// branch, which must then have no lexical directory at all.
    lexical: Option<ExpectedLexical>,
}

/// Lexical state of a generation: the payload of the last tantivy commit and
/// the iids the index holds.
#[derive(Clone, Copy)]
struct ExpectedLexical {
    generation: u64,
    iids: &'static [u32],
}

/// Directory of the lexical index inside an epoch (`docs/02`, "Organisation sur
/// disque").
const LEXICAL_DIR: &str = "lexical";

fn attach_old_state() -> ExpectedState<'static> {
    ExpectedState {
        epoch: common::EPOCH,
        generation: 0,
        segments: &[],
        live_iids: &[],
        documents: 0,
        passages: 0,
        lexical: None,
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
        lexical: None,
    }
}

/// [`attach_new_state`] on an index that enables the lexical branch: the batch
/// of generation 1 is indexed under the iids it produced.
fn lexical_attach_new_state() -> ExpectedState<'static> {
    ExpectedState {
        lexical: Some(ExpectedLexical {
            generation: 1,
            iids: &[0, 1],
        }),
        ..attach_new_state()
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
        lexical: None,
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
        lexical: None,
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

/// Index configuration of the index `index`, read from its `index.json`.
///
/// The parent and the child must run the attach with the configuration the
/// index declares: an index whose fixture enabled the lexical branch would
/// otherwise be attached with a configuration that disables it.
fn index_config_of(index: &Path) -> IndexConfig {
    let path = index.join("index.json");
    let raw = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "the index configuration {} must be readable: {error}",
            path.display()
        )
    });
    IndexConfig::from_json_str(&raw).unwrap_or_else(|error| {
        panic!(
            "the index configuration {} must be valid: {error}",
            path.display()
        )
    })
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
///
/// The index disables the lexical branch: this is the fixture of the attach
/// steps that have nothing to do with the lexical index.
fn attach_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let (temp, index) = common::temp_index();
    drop(common::create_store(&index));
    let batch = common::write_batch(
        &temp.path().join("batches"),
        "crash-attach",
        &common::TWO_DOCUMENTS,
    );
    (temp, index, batch)
}

/// Same fixture as [`attach_fixture`], except that the index enables the
/// lexical branch: the index `docs/02` step 4 builds.
fn lexical_attach_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let (temp, index) = common::temp_index();
    let config = common::test_config_with_lexical("test", &common::fixture_schema(), true);
    drop(common::create_store_with_config(
        &index,
        &common::fixture_schema(),
        &config,
    ));
    let batch = common::write_batch(
        &temp.path().join("batches"),
        "crash-attach",
        &common::TWO_DOCUMENTS,
    );
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
        DELETION_SCENARIO => run_deletion(&index),
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
    let config = index_config_of(index);
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

/// Commits the generation [`deletion_fixture`] planned: it tombstones iid 0 and
/// appends the passage of `seg-b` at iid 2. No batch is attached, so the
/// generation only walks steps 3 to 6 of `docs/02`, step 4 included.
fn run_deletion(index: &Path) -> ChildRun {
    let mut store = common::open_store(index);
    let segment = common::write_segment(index, DELETION_SEGMENT, 2, 2, &[&[0.0, 0.0, 1.0, 0.0]]);
    let change = common::change(
        vec![segment],
        vec![common::document("doc-c", 2)],
        vec![common::passage(2, "doc-c", 0, DELETION_SEGMENT, 2)],
        vec![Iid::new(0)],
    );
    match store.commit(change) {
        Ok(generation) => ChildRun::Completed {
            operation: "deletion",
            generation: generation.get(),
        },
        Err(error) => ChildRun::Failed {
            operation: "deletion",
            error: format!("{}: {}", error.code().as_str(), error.message()),
        },
    }
}

fn run_compaction(index: &Path) -> ChildRun {
    let config = index_config_of(index);
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
    common::assert_segment_directories(index, expected.epoch, expected.segments);
    assert_lexical_state(&epoch_dir, expected.generation, expected.lexical);
}

/// Asserts the lexical index of one epoch, the way INV-04 reads it: a disabled
/// branch leaves no directory at all, an enabled one names the committed
/// generation in its last commit and indexes exactly the live iids of that
/// generation.
fn assert_lexical_state(epoch_dir: &Path, generation: u64, expected: Option<ExpectedLexical>) {
    let lexical_dir = epoch_dir.join(LEXICAL_DIR);
    let Some(expected) = expected else {
        assert!(
            !lexical_dir.exists(),
            "{} must not exist: the index disables the lexical branch, and a directory the \
             configuration does not enable is a derived state no generation produced",
            lexical_dir.display()
        );
        return;
    };

    let state = read_state(epoch_dir).unwrap_or_else(|error| {
        panic!(
            "the lexical index of {} must be readable: {}: {}",
            epoch_dir.display(),
            error.code().as_str(),
            error.message()
        )
    });
    assert_eq!(
        state.generation,
        Some(Generation::new(expected.generation)),
        "after a restart, the payload of the last tantivy commit of generation {generation} \
         must name that generation (INV-04); Gt is {:?} and the iids it holds are {:?}",
        state.generation,
        state.iids
    );
    let mut actual: Vec<u32> = state.iids.iter().map(|iid| iid.get()).collect();
    actual.sort_unstable();
    let mut wanted = expected.iids.to_vec();
    wanted.sort_unstable();
    assert_eq!(
        actual, wanted,
        "the lexical index of generation {generation} must hold exactly the iids of its live \
         passages (INV-04): a passage it still holds after its tombstone, or one it misses, \
         makes the lexical branch disagree with the reference state"
    );
}

/// Asserts the lexical index on disk is *behind* `committed`: the crash left
/// the generation committed in SQLite and tantivy on an older one. This is the
/// gap the `Gt < G` branch of recovery must close, so the test covers it.
fn assert_lexical_is_behind(index: &Path, epoch: u32, committed: u64, absent_iids: &[u32]) {
    let epoch_dir = index.join("epochs").join(epoch.to_string());
    let state = read_state(&epoch_dir).unwrap_or_else(|error| {
        panic!(
            "the lexical index of {} must be readable: {}: {}",
            epoch_dir.display(),
            error.code().as_str(),
            error.message()
        )
    });
    assert!(
        state
            .generation
            .is_none_or(|generation| generation.get() < committed),
        "the crash must leave the lexical index behind generation {committed}, otherwise the \
         test does not exercise the catch-up of the restart; Gt is {:?}",
        state.generation
    );
    for iid in absent_iids {
        assert!(
            !state.iids.contains(&Iid::new(*iid)),
            "the lexical index must not hold iid {iid} yet: the crash is before step 4 of \
             generation {committed}. It holds {:?}",
            state.iids
        );
    }
}

/// Asserts no `tmp-*` entry survives in the index directory, any epoch directory
/// or its `segments/` and `manifests/` directories.
fn assert_no_temporary_entries(index: &Path) {
    let epochs = index.join("epochs");
    let mut checked = vec![index.to_path_buf(), epochs.clone()];
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
    let config = index_config_of(index);
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
    let config = index_config_of(index);
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
    // Before the replay: a `tmp-*` the crash left behind must be gone, not
    // hidden by the replay.
    assert_no_temporary_entries(&index);

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

// --- Attach with the lexical branch: step 4 of `docs/02` ---

/// Creates an index at generation 1 with the lexical branch enabled: two
/// documents and two passages at generation 1, indexed by tantivy.
///
/// Generation 2 is the change the child commits: it tombstones iid 0 and
/// appends `DELETION_SEGMENT`, whose single passage becomes iid 2. The
/// generation carries no batch, so it walks steps 3 to 6 only, step 4 included.
fn deletion_fixture() -> (tempfile::TempDir, PathBuf) {
    let (temp, index) = common::temp_index();
    let config = common::test_config_with_lexical("test", &common::fixture_schema(), true);
    let mut store = common::create_store_with_config(&index, &common::fixture_schema(), &config);
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

    let state = read_state(&common::epoch_dir(&index)).unwrap_or_else(|error| {
        panic!("the lexical index of generation 1 must be readable: {error}")
    });
    assert_eq!(
        state.generation,
        Some(Generation::new(1)),
        "the fixture must start with a lexical index at generation 1: {:?}",
        state.generation
    );
    (temp, index)
}

/// The criterion of the fiche: a crash between step 3 (`COMMIT`) and step 4
/// (tantivy) followed by a restart must find the lexical index at the
/// generation SQLite committed.
///
/// The batch of generation 1 is committed in SQLite and nowhere in tantivy. The
/// restart must index iids 0 and 1, so the lexical branch of `G+1` answers what
/// the dense branch already sees (INV-04, "Gt < G" of "Reprise au démarrage").
#[test]
fn inv_04_crash_before_lexical_commit_recovers_the_lexical_to_the_committed_generation() {
    let (_temp, index, batch) = lexical_attach_fixture();
    let outcome = spawn_child("after_sqlite_commit", ATTACH_SCENARIO, &index, Some(&batch));
    assert_child_was_killed("after_sqlite_commit", &outcome);

    // The gap is real: the crash sits at the start of step 4, so the state on
    // disk is before the lexical step, not after it.
    assert_lexical_is_behind(&index, common::EPOCH, 1, &[0, 1]);

    // Opening the store runs recovery, which must catch the lexical index up.
    assert_index_state(&index, &lexical_attach_new_state());
    assert_no_temporary_entries(&index);

    let report = replay_attach(&index, &batch);
    assert_eq!(
        report.generation.get(),
        1,
        "replaying the batch after a crash at after_sqlite_commit must return generation 1 \
         without writing (INV-10)"
    );
    assert_index_state(&index, &lexical_attach_new_state());
    assert_epochs(&index, &[common::EPOCH]);
}

/// A crash at the end of step 4 leaves tantivy at `G+1` with `G+1` as its
/// payload. The restart must not rebuild what is already consistent, and must
/// keep the generation whole (INV-04: the state is the committed generation).
#[test]
fn inv_04_crash_after_lexical_commit_keeps_the_committed_generation() {
    let (_temp, index, batch) = lexical_attach_fixture();
    let outcome = spawn_child(
        "after_lexical_commit",
        ATTACH_SCENARIO,
        &index,
        Some(&batch),
    );
    assert_child_was_killed("after_lexical_commit", &outcome);

    assert_index_state(&index, &lexical_attach_new_state());

    let report = replay_attach(&index, &batch);
    assert_eq!(
        report.generation.get(),
        1,
        "replaying the batch after a crash at after_lexical_commit must return generation 1 \
         without writing (INV-10)"
    );
    assert_index_state(&index, &lexical_attach_new_state());
    assert_epochs(&index, &[common::EPOCH]);
    assert_no_temporary_entries(&index);
}

/// A generation that tombstones iid 0 and appends iid 2, crashed between steps
/// 3 and 4: after the restart, the lexical index must name generation 2 and
/// hold exactly the live iids of that generation, `{1, 2}`. The tombstoned
/// passage is gone from tantivy and the new one is indexed (INV-04: "Gt < G"
/// re-indexes the passages created after `Gt` and removes those dead after it).
#[test]
fn inv_04_crash_after_deletion_commit_recovers_the_lexical_to_the_committed_generation() {
    let (_temp, index) = deletion_fixture();
    let outcome = spawn_child("after_sqlite_commit", DELETION_SCENARIO, &index, None);
    assert_child_was_killed("after_sqlite_commit", &outcome);

    // Generation 2 is committed in SQLite, the lexical index still names
    // generation 1 and still holds the tombstoned iid 0.
    assert_lexical_is_behind(&index, common::EPOCH, 2, &[2]);

    let expected = ExpectedState {
        epoch: common::EPOCH,
        generation: 2,
        segments: &["seg-a", DELETION_SEGMENT],
        live_iids: &[1, 2],
        documents: 3,
        passages: 3,
        lexical: Some(ExpectedLexical {
            generation: 2,
            iids: &[1, 2],
        }),
    };
    assert_index_state(&index, &expected);
    assert_no_temporary_entries(&index);
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
    // Before the replay: recovery, which `assert_index_state` ran by opening
    // the store, must have removed the `epochs/tmp-*` a crash can leave.
    assert_no_temporary_entries(&index);

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
