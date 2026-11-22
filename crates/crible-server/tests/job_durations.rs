// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixtures and assertions of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! Durations of an attach, as the member `durations` of the object `Job`
//! renders them (ADR-0149; `docs/05-api.md`, "Jobs").
//!
//! | Test | What it proves |
//! |---|---|
//! | `an_online_attach_renders_the_six_durations_of_its_journal_line` | a successful `online_batch` renders `durations`, six whole-millisecond integers equal to the `attach durations` line of the same `job_id` |
//! | `a_deposited_batch_renders_the_six_durations_of_its_journal_line` | the same for the other attach route, `POST /v1/indexes/{index}/batches` |
//! | `the_job_list_renders_the_same_durations_as_the_job_detail` | `GET /v1/jobs` renders the member of `GET /v1/jobs/{job_id}` |
//! | `wait_true_renders_the_durations_of_the_finished_attach` | `?wait=true` renders the same object `Job`, so the same member |
//! | `a_step_the_protocol_skipped_counts_zero_milliseconds` | a step that did not cost anything is `0`, key present, never `null` |
//! | `the_other_kinds_render_null_durations` | `compact`, `export` and `snapshot` keep the member and render `null` |
//! | `a_running_attach_renders_null_durations` | a job held before its commit point is `running` or `queued` and renders `null`, member present (failpoints build) |
//! | `a_failed_attach_renders_null_durations` | a job that failed after its commit point renders `null`, as no line is published |
//! | `a_job_read_after_a_restart_renders_null_durations` | the member lives in memory only |
//! | `inv_10_a_replayed_batch_keeps_the_stored_report_without_durations` | a replay returns the stored report, key for key, and no report carries a duration (INV-10) |
//! | `inv_10_a_replayed_batch_renders_no_durations` | a replay plays no step, so it renders `null` (INV-10) |
//!
//! The six names are the ones of the journal line, so the two can be compared
//! without conversion (ADR-0149, decision 1). The journal is read through the
//! sink of `tests/common`, installed once per test binary.
//!
//! Every test takes the slot of `common::lock_barrier_environment`: one of them
//! arms the process-global `CRIBLE_TEST_BARRIER` of the `failpoints` build, and
//! a job of another test running meanwhile would read that variable.
//!
//! ```text
//! cargo test -p crible-server --test job_durations
//! ```

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_store::jobs::{JOBS_FILE_NAME, JobStore};
use serde_json::{Value, json};

#[cfg(feature = "failpoints")]
use common::AttachBarrier;
use common::{
    ADMIN_SECRET, INDEX, Instance, WRITE_SECRET, assert_attach_duration_line, export_index,
    index_config, instance_with_alpha, online_batch, post_documents, send, terminal_job,
};

/// The six members of `durations`, in the order of the journal line
/// (ADR-0149, decision 1).
const DURATION_KEYS: [&str; 6] = [
    "validation_ms",
    "segment_ms",
    "transaction_ms",
    "lexical_ms",
    "manifest_ms",
    "snapshot_ms",
];

/// Identifier of a job, from the answer of its submission.
fn id_of(job: &Value) -> String {
    job["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the job carries its identifier: {job}"))
        .to_owned()
}

/// Asserts that `job` carries `durations` as a closed object of six
/// non-negative whole-millisecond integers, and returns it.
///
/// The member is **always present** and `null` when there is nothing to
/// render, so a job without it is a fault, not a case to tolerate
/// (ADR-0149, decision 1).
fn assert_six_durations(label: &str, job: &Value) -> Value {
    let durations = job.get("durations").unwrap_or_else(|| {
        panic!("{label}: the job carries a `durations` member, always present (ADR-0149): {job}")
    });
    let object = durations.as_object().unwrap_or_else(|| {
        panic!("{label}: `durations` is an object after a real attach (ADR-0149): {job}")
    });
    let mut names: Vec<&str> = object.keys().map(String::as_str).collect();
    names.sort_unstable();
    let mut expected = DURATION_KEYS.to_vec();
    expected.sort_unstable();
    assert_eq!(
        names, expected,
        "{label}: `durations` holds these six keys only, and no other (ADR-0149, decision 1): {job}"
    );
    for key in DURATION_KEYS {
        assert!(
            object[key].as_u64().is_some(),
            "{label}: `{key}` is a non-negative whole number of milliseconds: {job}"
        );
    }
    durations.clone()
}

/// Asserts that the member `durations` is present and null.
fn assert_null_durations(label: &str, job: &Value) {
    assert!(
        job.get("durations").is_some(),
        "{label}: the member is present even when it has nothing to render (ADR-0149): {job}"
    );
    assert_eq!(
        job["durations"],
        Value::Null,
        "{label}: `durations` is null here (ADR-0149, decision 2): {job}"
    );
}

/// Asserts that no key of `value` at any depth is a duration key.
///
/// INV-10 stays true because the member is **outside** `result`: neither the
/// report of `result` nor the persisted report may carry one (ADR-0149,
/// decision 4).
fn assert_no_duration_key(label: &str, value: &Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                assert!(
                    !DURATION_KEYS.contains(&key.as_str()) && !key.ends_with("_ms"),
                    "{label} carries the duration key '{key}' (INV-10, ADR-0149): {value}"
                );
                assert_no_duration_key(label, child);
            }
        }
        Value::Array(items) => {
            for item in items {
                assert_no_duration_key(label, item);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

/// The `result` column of `id`, as `jobs.sqlite` holds it.
///
/// This is the only report an attach persists in the job table: `batches.report`
/// is not readable from this crate, and the replay reads it (INV-10), so the
/// stored report of the original attach is the one a replay returns.
fn stored_result(instance: &Instance, id: &str) -> Value {
    let path = instance.root().join("data").join(JOBS_FILE_NAME);
    let store = JobStore::open_or_create(&path)
        .unwrap_or_else(|error| panic!("{} opens: {}", path.display(), error.message()));
    let record = store
        .job(id)
        .unwrap_or_else(|error| panic!("the job {id} is readable: {}", error.message()))
        .unwrap_or_else(|| panic!("the job {id} is in the table"));
    let text = record.result.unwrap_or_else(|| {
        panic!("the attach job {id} keeps its report in the table, as it is persisted elsewhere")
    });
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("the result of {id} is JSON: {error}"))
}

/// The fields of the `attach durations` line of `job_id`, exactly one line.
fn journal_fields(journal_text: &str, job_id: &str) -> Value {
    let mut found: Vec<Value> = Vec::new();
    for line in journal_text.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let fields = &entry["fields"];
        if fields["message"] == "attach durations" && fields["job_id"] == job_id {
            found.push(fields.clone());
        }
    }
    assert_eq!(
        found.len(),
        1,
        "the attach job {job_id} logs its durations exactly once: {journal_text}"
    );
    found.remove(0)
}

/// The member `durations` and the journal line of one job carry the same six
/// numbers, step by step (ADR-0149, decision 1, "une seule source").
fn assert_member_equals_line(durations: &Value, fields: &Value) {
    for key in DURATION_KEYS {
        assert_eq!(
            durations[key], fields[key],
            "the member and the `attach durations` line publish the same `{key}`: {durations} / {fields}"
        );
    }
}

/// The directory of the single epoch of the fixture index.
fn single_epoch(instance: &Instance) -> PathBuf {
    let epochs = instance
        .root()
        .join("data")
        .join("indexes")
        .join(INDEX)
        .join("epochs");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&epochs)
        .unwrap_or_else(|error| panic!("{} is readable: {error}", epochs.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("an epoch entry is readable: {error}"))
                .path()
        })
        .collect();
    assert_eq!(entries.len(), 1, "the fixture holds one epoch: {entries:?}");
    entries.remove(0)
}

/// `GET /v1/jobs/{id}`.
async fn job(router: &Router, id: &str) -> Value {
    let path = format!("/v1/jobs/{id}");
    let (status, body) = send(router, Method::GET, &path, WRITE_SECRET, None).await;
    assert_eq!(status, StatusCode::OK, "the job is readable: {body}");
    body
}

/// A successful `online_batch` renders `durations`: six whole-millisecond
/// integers, equal to the numbers of the `attach durations` line of the same
/// `job_id` (ADR-0149, decision 1; `docs/05-api.md`, "Jobs").
#[tokio::test]
async fn an_online_attach_renders_the_six_durations_of_its_journal_line() {
    let _slot = common::lock_barrier_environment().await;
    let journal = common::journal();
    let (_instance, state, router) = instance_with_alpha().await;
    let (status, accepted) = post_documents(
        &router,
        online_batch("durations-online", "le contrat fait la loi", "1240"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the batch attaches: {accepted}");
    assert_eq!(accepted["kind"], "online_batch", "{accepted}");
    let finished = terminal_job(&router, &state, &accepted).await;
    assert_eq!(
        finished["state"], "succeeded",
        "the attach succeeds: {finished}"
    );

    let id = id_of(&finished);
    let durations = assert_six_durations("an online attach", &finished);
    let text = journal.contents();
    assert_attach_duration_line(&text, &id, "online_batch", INDEX);
    let fields = journal_fields(&text, &id);
    assert_member_equals_line(&durations, &fields);
}

/// The other attach route, `POST /v1/indexes/{index}/batches`, renders the same
/// member, equal to its own journal line (ADR-0149, decision 3).
#[tokio::test]
async fn a_deposited_batch_renders_the_six_durations_of_its_journal_line() {
    let _slot = common::lock_barrier_environment().await;
    let journal = common::journal();
    let (first, first_state, first_router) = instance_with_alpha().await;
    let (status, accepted) = post_documents(
        &first_router,
        online_batch("durations-exported", "le contrat fait la loi", "1240"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the source attaches: {accepted}");
    let source = terminal_job(&first_router, &first_state, &accepted).await;
    assert_eq!(source["state"], "succeeded", "the source: {source}");
    let exported = export_index(&first_router, &first_state).await;

    let deposited = "durations-deposited";
    let from = first.exchange().join(&exported);
    let to = first.inbox().join(deposited);
    std::fs::rename(&from, &to)
        .unwrap_or_else(|error| panic!("{} is movable: {error}", from.display()));

    let second = Instance::sharing(first.exchange());
    let second_state = second.state();
    let second_router = crible_server::http::build_router(Arc::clone(&second_state));
    let (status, body) = send(
        &second_router,
        Method::POST,
        "/v1/indexes",
        ADMIN_SECRET,
        Some(index_config()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "the index is created: {body}");

    let path = format!("/v1/indexes/{INDEX}/batches?wait=true");
    let (status, accepted) = send(
        &second_router,
        Method::POST,
        &path,
        WRITE_SECRET,
        Some(json!({ "directory": deposited })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the deposited batch attaches within the delay: {accepted}"
    );
    assert_eq!(accepted["kind"], "attach_batch", "{accepted}");

    let id = id_of(&accepted);
    let durations = assert_six_durations("a deposited batch", &accepted);
    let text = journal.contents();
    assert_attach_duration_line(&text, &id, "attach_batch", INDEX);
    let fields = journal_fields(&text, &id);
    assert_member_equals_line(&durations, &fields);

    let detail = job(&second_router, &id).await;
    assert_eq!(
        detail["durations"], durations,
        "`?wait=true` renders the same object `Job` as the detail (ADR-0149, decision 3): {detail}"
    );
}

/// `GET /v1/jobs` renders the member of `GET /v1/jobs/{job_id}` for the same job
/// (ADR-0149, decision 3).
#[tokio::test]
async fn the_job_list_renders_the_same_durations_as_the_job_detail() {
    let _slot = common::lock_barrier_environment().await;
    let (_instance, state, router) = instance_with_alpha().await;
    let (status, accepted) = post_documents(
        &router,
        online_batch("durations-list", "le contrat fait la loi", "1240"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the batch attaches: {accepted}");
    let finished = terminal_job(&router, &state, &accepted).await;
    assert_eq!(
        finished["state"], "succeeded",
        "the attach succeeds: {finished}"
    );
    let id = id_of(&finished);
    let detail = assert_six_durations("the detail", &finished);

    let (status, list) = send(
        &router,
        Method::GET,
        &format!("/v1/jobs?index={INDEX}&state=succeeded"),
        WRITE_SECRET,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the list is readable: {list}");
    let entry = list["jobs"]
        .as_array()
        .unwrap_or_else(|| panic!("the list carries `jobs`: {list}"))
        .iter()
        .find(|entry| entry["id"] == id.as_str())
        .unwrap_or_else(|| panic!("the list holds the job {id}: {list}"));
    let listed = assert_six_durations("the list", entry);
    assert_eq!(
        listed, detail,
        "the list and the detail render the same member for the same job: {list}"
    );
}

/// `?wait=true` of an attach that finishes within the delay renders `200` with
/// the same object `Job`, so with the same member (ADR-0149, decision 3).
#[tokio::test]
async fn wait_true_renders_the_durations_of_the_finished_attach() {
    let _slot = common::lock_barrier_environment().await;
    let (_instance, _state, router) = instance_with_alpha().await;
    let (status, accepted) = post_documents(
        &router,
        online_batch("durations-wait", "le contrat fait la loi", "1240"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the fixture attach finishes within `wait_timeout_ms`: {accepted}"
    );
    assert_eq!(accepted["state"], "succeeded", "{accepted}");
    let waited = assert_six_durations("`?wait=true`", &accepted);

    let detail = job(&router, &id_of(&accepted)).await;
    assert_eq!(
        detail["durations"], waited,
        "the answer of `?wait=true` and the detail agree on the member: {detail}"
    );
}

/// A step the protocol did not play is present and worth `0`, never `null`: the
/// fixture index has no lexical branch, so `lexical` is measured and costs zero
/// (ADR-0149, decision 1; `crates/crible-store/src/attach.rs`, `lexical`).
#[tokio::test]
async fn a_step_the_protocol_skipped_counts_zero_milliseconds() {
    let _slot = common::lock_barrier_environment().await;
    let (_instance, state, router) = instance_with_alpha().await;
    let (status, accepted) = post_documents(
        &router,
        online_batch("durations-lexical", "le contrat fait la loi", "1240"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the batch attaches: {accepted}");
    let finished = terminal_job(&router, &state, &accepted).await;
    assert_eq!(
        finished["state"], "succeeded",
        "the attach succeeds: {finished}"
    );
    let durations = assert_six_durations("the fixture index", &finished);
    assert_eq!(
        durations["lexical_ms"],
        json!(0),
        "the lexical step does not run on an index without a lexical branch, so it costs 0 and the key stays a number: {finished}"
    );
}

/// A kind outside `attach_batch` and `online_batch` keeps the member and renders
/// `null` (ADR-0149, decision 2).
#[tokio::test]
async fn the_other_kinds_render_null_durations() {
    let _slot = common::lock_barrier_environment().await;
    let (_instance, state, router) = instance_with_alpha().await;
    let (status, accepted) = post_documents(
        &router,
        online_batch("durations-maintenance", "le contrat fait la loi", "1240"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the batch attaches: {accepted}");
    let finished = terminal_job(&router, &state, &accepted).await;
    assert_eq!(
        finished["state"], "succeeded",
        "the attach succeeds: {finished}"
    );

    for (kind, suffix) in [
        ("compact", "compact"),
        ("export", "exports"),
        ("snapshot", "snapshots"),
    ] {
        let path = format!("/v1/indexes/{INDEX}/{suffix}?wait=true");
        let (status, accepted) = send(&router, Method::POST, &path, ADMIN_SECRET, None).await;
        assert_eq!(status, StatusCode::OK, "the {kind} finishes: {accepted}");
        assert_eq!(accepted["state"], "succeeded", "the {kind}: {accepted}");
        assert_null_durations(&format!("a {kind} job"), &accepted);
    }
}

/// A running attach renders `null`: the report does not exist yet, so there is
/// nothing to render, and the member appears at the terminal transition only
/// (ADR-0149, decision 2).
///
/// The job is held **before the SQLite commit** of its generation by the barrier
/// of the `failpoints` build, which is a deterministic stop, never a sleep.
#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_running_attach_renders_null_durations() {
    let _slot = common::lock_barrier_environment().await;
    let (_instance, state, router) = instance_with_alpha().await;
    let barrier = AttachBarrier::arm();

    let path = format!("/v1/indexes/{INDEX}/documents?wait=false");
    let (status, accepted) = send(
        &router,
        Method::POST,
        &path,
        WRITE_SECRET,
        Some(online_batch(
            "durations-held",
            "le contrat fait la loi",
            "1240",
        )),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "the batch is queued: {accepted}"
    );
    // The barrier holds the attach before its commit point, so the job is
    // `queued` or `running` here and never terminal; both states render `null`,
    // and the acceptance itself proves the member is always present
    // (ADR-0149, decision 2).
    assert_null_durations("a queued attach", &accepted);
    let id = id_of(&accepted);

    barrier.wait_before_commit().await;

    let running = job(&router, &id).await;
    assert_eq!(running["state"], "running", "the job is held: {running}");
    assert_null_durations("a running attach", &running);

    barrier.release_before_commit();
    let finished = terminal_job(&router, &state, &accepted).await;
    assert_eq!(
        finished["state"], "succeeded",
        "the attach succeeds: {finished}"
    );
    assert_six_durations("a finished attach", &finished);
}

/// A job that failed after its commit point renders `null`: the attach leaves
/// by `?` before the journal line, so the member has nothing to render
/// (ADR-0149, decision 2).
#[tokio::test]
async fn a_failed_attach_renders_null_durations() {
    let _slot = common::lock_barrier_environment().await;
    let (instance, state, router) = instance_with_alpha().await;
    // The manifest directory replaced by a regular file makes `write_manifest`
    // fail **after** the commit of the generation, so the attach runs steps and
    // still fails (the fault of `tests/job_failure_after_commit.rs`).
    let manifests = single_epoch(&instance).join("manifests");
    std::fs::remove_dir_all(&manifests)
        .unwrap_or_else(|error| panic!("{} is removable: {error}", manifests.display()));
    std::fs::write(&manifests, b"not a directory")
        .unwrap_or_else(|error| panic!("{} is replaceable: {error}", manifests.display()));

    let (status, accepted) = post_documents(
        &router,
        online_batch("durations-failed", "le contrat fait la loi", "1240"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the batch is accepted: {accepted}");
    let finished = terminal_job(&router, &state, &accepted).await;
    assert_eq!(
        finished["state"], "failed",
        "the attach fails after its steps: {finished}"
    );
    assert_null_durations("a failed attach", &finished);
}

/// The member lives in memory only: a job read back from `jobs.sqlite` after a
/// restart renders `null` (ADR-0149, decision 2).
///
/// A restart is a new process: the writer task of the first instance holds the
/// writer lock of the index (ADR-0116, decision 2) until its runtime goes away,
/// so the first phase runs in a runtime of its own, dropped before the second
/// instance opens the same data root.
#[test]
fn a_job_read_after_a_restart_renders_null_durations() {
    // The slot of the barrier tests is taken without a runtime of the test: the
    // guard of a `tokio::sync::Mutex` outlives the runtime that awaited it.
    let _slot = {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime is creatable");
        runtime.block_on(common::lock_barrier_environment())
    };
    let (instance, id) = {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a runtime is creatable");
        let (instance, state, router) = runtime.block_on(instance_with_alpha());
        let id = runtime.block_on(async {
            let (status, accepted) = post_documents(
                &router,
                online_batch("durations-restart", "le contrat fait la loi", "1240"),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "the batch attaches: {accepted}");
            let finished = terminal_job(&router, &state, &accepted).await;
            assert_eq!(
                finished["state"], "succeeded",
                "the attach succeeds: {finished}"
            );
            assert_six_durations("the attach before the restart", &finished);
            id_of(&finished)
        });
        drop(router);
        drop(state);
        drop(runtime);
        (instance, id)
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime is creatable");
    let restarted = {
        let _entered = runtime.enter();
        instance.state()
    };
    let router = crible_server::http::build_router(Arc::clone(&restarted));
    runtime.block_on(async {
        let again = job(&router, &id).await;
        assert_eq!(
            again["state"], "succeeded",
            "the finished job survived the restart: {again}"
        );
        assert!(
            again["result"]["report"].is_object(),
            "the report is persisted, so it is read back: {again}"
        );
        assert_null_durations("a job read after a restart", &again);
    });
}

/// INV-10: a replay of an already applied `batch_id` returns the stored report,
/// key for key, and no report carries a duration: the member is outside
/// `result`, and the table gains no column (ADR-0149, decision 4).
#[tokio::test]
async fn inv_10_a_replayed_batch_keeps_the_stored_report_without_durations() {
    let _slot = common::lock_barrier_environment().await;
    let (instance, state, router) = instance_with_alpha().await;
    let body = online_batch("durations-replay", "le contrat fait la loi", "1240");

    let (status, accepted) = post_documents(&router, body.clone()).await;
    assert_eq!(status, StatusCode::OK, "the batch attaches: {accepted}");
    let first = terminal_job(&router, &state, &accepted).await;
    assert_eq!(first["state"], "succeeded", "the original attach: {first}");
    let first_report = first["result"]["report"].clone();
    assert!(
        first_report.is_object(),
        "the attach publishes a report: {first}"
    );

    let (status, accepted) = post_documents(&router, body).await;
    assert_eq!(status, StatusCode::OK, "the replay is accepted: {accepted}");
    let second = terminal_job(&router, &state, &accepted).await;
    assert_eq!(second["state"], "succeeded", "the replay: {second}");

    assert_eq!(
        second["result"]["report"], first_report,
        "a replay returns the report of the original attach, key for key (INV-10)"
    );
    assert_eq!(
        second["generation"], first["generation"],
        "a replay publishes no generation (INV-10)"
    );

    for (label, report) in [
        ("the report of the original attach", &first_report),
        ("the report of the replay", &second["result"]["report"]),
    ] {
        assert_no_duration_key(label, report);
    }
    let stored = stored_result(&instance, &id_of(&first));
    assert_no_duration_key("the row of jobs.sqlite", &stored);
    assert_eq!(
        stored["report"], first_report,
        "the persisted report is the one the job renders: {stored}"
    );
}

/// INV-10: a replay plays no step, so its succeeded job renders
/// `durations: null`, member present, as it renders `progress: null`
/// (ADR-0149, decision 2; ADR-0134).
#[tokio::test]
async fn inv_10_a_replayed_batch_renders_no_durations() {
    let _slot = common::lock_barrier_environment().await;
    let (_instance, state, router) = instance_with_alpha().await;
    let body = online_batch("durations-replayed", "le contrat fait la loi", "1240");

    let (status, accepted) = post_documents(&router, body.clone()).await;
    assert_eq!(status, StatusCode::OK, "the batch attaches: {accepted}");
    let first = terminal_job(&router, &state, &accepted).await;
    assert_eq!(first["state"], "succeeded", "the original attach: {first}");
    assert_six_durations("the original attach", &first);

    let (status, accepted) = post_documents(&router, body).await;
    assert_eq!(status, StatusCode::OK, "the replay is accepted: {accepted}");
    let second = terminal_job(&router, &state, &accepted).await;
    assert_eq!(second["state"], "succeeded", "the replay: {second}");
    assert_null_durations("a replayed batch", &second);
}
