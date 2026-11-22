// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixtures and assertions of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! The `trim` line of the journal, at the end of a real job (ADR-0160).
//!
//! | Test | What it proves |
//! |---|---|
//! | `the_trim_line_of_a_finished_job_carries_only_numbers_and_names` | one `trim` line per job at level `debug`, exactly the six fields of decision 4, `pad = 0` and `trimmed` in `{0, 1}` |
//! | `the_memory_line_of_job_end_precedes_the_trim_line` | the `memory` line of `job_end` is emitted **before** the `trim` line, so its figure keeps the same sense as in 1.0 (decision 4) |
//!
//! The journal is read through the sink of `tests/common`. A sink is installed
//! once per test binary and the first level wins (`journal_at_level` in
//! `tests/common/mod.rs`), so this binary asks for `debug`: a `trim`
//! line is silent under the `info` default, and a binary that asked for `info`
//! first could never see one.
//!
//! **glibc only.** The whole binary is under `cfg(all(target_os = "linux",
//! target_env = "gnu"))`: off glibc no allocator is called and no line is
//! written (ADR-0160, decision 1), so there is nothing to read there. The tests
//! therefore run in the Linux CI job only, never on the macOS development
//! machine.
//!
//! ```text
//! cargo test -p crible-server --test malloc_trim
//! ```

#![cfg(all(target_os = "linux", target_env = "gnu"))]

mod common;

use std::sync::Arc;

use axum::Router;
use axum::http::{Method, StatusCode};
use crible_server::state::AppState;
use serde_json::Value;

use common::{
    ADMIN_SECRET, INDEX, Instance, instance_with_alpha, online_batch, post_documents, send,
    terminal_job, trim_lines,
};

/// The six fields of a `trim` line, with its message (ADR-0160, decision 4).
const TRIM_FIELDS: [&str; 7] = [
    "duration_us",
    "index",
    "job_id",
    "kind",
    "message",
    "pad",
    "trimmed",
];

/// Creates the fixture index and attaches one document, so the compaction that
/// follows has a live passage to compact.
async fn instance_with_one_passage() -> (Instance, Arc<AppState>, Router) {
    let (instance, state, router) = instance_with_alpha().await;
    let (status, accepted) = post_documents(
        &router,
        online_batch("trim-attached", "le contrat fait la loi", "1240"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the batch attaches: {accepted}");
    let attached = terminal_job(&router, &state, &accepted).await;
    assert_eq!(
        attached["state"], "succeeded",
        "the source batch attaches: {attached}"
    );
    (instance, state, router)
}

/// Runs one `compact` job on the fixture index and returns its finished object.
async fn compact(router: &Router, state: &Arc<AppState>) -> Value {
    let path = format!("/v1/indexes/{INDEX}/compact?wait=true");
    let (status, accepted) = send(router, Method::POST, &path, ADMIN_SECRET, None).await;
    assert!(
        status == StatusCode::OK || status == StatusCode::ACCEPTED,
        "the compaction is accepted: {accepted}"
    );
    let finished = terminal_job(router, state, &accepted).await;
    assert_eq!(
        finished["state"], "succeeded",
        "the compaction succeeds: {finished}"
    );
    assert_eq!(finished["kind"], "compact", "{finished}");
    finished
}

/// The end of a job emits **one** `trim` line, at level `debug`, whose fields are
/// exactly the six of ADR-0160, decision 4: `pad`, `trimmed`, `duration_us`,
/// `job_id`, `kind` and `index`, nothing else.
///
/// The exact set of names is what INV-13 rests on: a line with no free field
/// cannot carry a query text, a filter value or an internal identifier. `pad` is
/// `0`, the only argument the server passes, and `trimmed` is `0` or `1`, never a
/// boolean and never anything else.
#[tokio::test]
async fn the_trim_line_of_a_finished_job_carries_only_numbers_and_names() {
    let journal = common::journal_at_level("debug");
    let (_instance, state, router) = instance_with_one_passage().await;
    let job = compact(&router, &state).await;
    let id = job["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the job carries its identifier: {job}"))
        .to_owned();

    let text = journal.contents();
    let lines = trim_lines(&text, &id);
    assert_eq!(
        lines.len(),
        1,
        "the end of the job {id} logs its trim exactly once: {text}"
    );
    let fields = &lines[0];
    let mut names: Vec<&str> = fields
        .as_object()
        .unwrap_or_else(|| panic!("the trim line carries an object of fields: {fields}"))
        .keys()
        .map(String::as_str)
        .collect();
    names.sort_unstable();
    let mut expected = TRIM_FIELDS.to_vec();
    expected.sort_unstable();
    assert_eq!(
        names, expected,
        "the trim line carries these six fields and its message only, so no free \
         field can hold a text or a value (INV-13): {fields}"
    );
    assert_eq!(fields["message"], "trim", "{fields}");
    assert_eq!(fields["job_id"], id.as_str(), "{fields}");
    assert_eq!(fields["kind"], "compact", "{fields}");
    assert_eq!(fields["index"], INDEX, "{fields}");
    assert_eq!(
        fields["pad"], 0,
        "the padding is the argument the server passes, so `0`: {fields}"
    );
    assert!(
        fields["trimmed"]
            .as_u64()
            .is_some_and(|trimmed| trimmed <= 1),
        "`trimmed` is `1` when the call released pages and `0` otherwise, a \
         number and never a boolean: {fields}"
    );
    assert!(
        fields["duration_us"].as_u64().is_some(),
        "`duration_us` is a non-negative integer: {fields}"
    );

    // The level is `debug`, so the default `info` keeps the line silent
    // (ADR-0160, decision 4).
    let line = text
        .lines()
        .find(|line| {
            serde_json::from_str::<Value>(line).is_ok_and(|entry| {
                entry["fields"]["message"] == "trim" && entry["fields"]["job_id"] == id.as_str()
            })
        })
        .unwrap_or_else(|| panic!("the trim line of {id} is in the journal: {text}"));
    let entry: Value = serde_json::from_str(line).expect("the line is one JSON object");
    assert_eq!(entry["level"], "DEBUG", "{line}");
}

/// The `memory` line of `job_end` is emitted **before** the `trim` line: it
/// measures the memory at the end of the work of the job, not after a trim, so
/// its figure keeps the same sense between 1.0 and 1.1 (ADR-0160, decision 4).
#[tokio::test]
async fn the_memory_line_of_job_end_precedes_the_trim_line() {
    let journal = common::journal_at_level("debug");
    let (_instance, state, router) = instance_with_one_passage().await;
    let job = compact(&router, &state).await;
    let id = job["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the job carries its identifier: {job}"))
        .to_owned();

    let text = journal.contents();
    let mut memory_at = None;
    let mut trim_at = None;
    for (position, line) in text.lines().enumerate() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let fields = &entry["fields"];
        if fields["job_id"] != id.as_str() {
            continue;
        }
        if fields["message"] == "memory" && fields["step"] == "job_end" && memory_at.is_none() {
            memory_at = Some(position);
        }
        if fields["message"] == "trim" && trim_at.is_none() {
            trim_at = Some(position);
        }
    }
    let memory_at =
        memory_at.unwrap_or_else(|| panic!("the job {id} logs its memory line of job_end: {text}"));
    let trim_at = trim_at.unwrap_or_else(|| panic!("the job {id} logs its trim line: {text}"));
    assert!(
        memory_at < trim_at,
        "the memory line of job_end ({memory_at}) precedes the trim line ({trim_at}): {text}"
    );
}
