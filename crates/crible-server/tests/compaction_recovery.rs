// SPDX-License-Identifier: Apache-2.0

// The only test of this file arms the store failpoint `after_epoch_switch`, so
// the whole binary exists only in the server's `failpoints` build
// (complement of 2026-10-29 to ADR-0068, decision 2). A plain `cargo test`
// compiles no test here.
#![cfg(feature = "failpoints")]
// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the checks of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! Recovery of the writer after a compaction that dies after its epoch switch
//! (`docs/phases/P6-serveur-et-api.md`, `ROUTES-DOCUMENTS-ET-LOTS`, criterion
//! "une compaction en échec ne perd aucune écriture"; ADR-0108).
//!
//! The failpoint `after_epoch_switch` panics after `CURRENT_EPOCH` names E+1 and
//! before the in-memory store adopts that epoch: the store is left on E exactly
//! as a failed `adopt_epoch` leaves it. The writer must fail the job, reopen the
//! index from disk — `CURRENT_EPOCH` is the authority — and republish before the
//! next job. The next write then lands in E+1, and the reads keep answering
//! (INV-03, INV-04).
//!
//! The variable is process-global, so this binary holds a single test and the
//! guard removes it before the next job runs.
//!
//! ```text
//! cargo test -p crible-server --test compaction_recovery
//! ```

mod common;

use std::sync::{Mutex, MutexGuard};

use axum::http::{Method, StatusCode};
use serde_json::json;

use common::{
    ADMIN_SECRET, DOC_ID, INDEX, WRITE_SECRET, instance_with_alpha, online_batch, post_documents,
    send, terminal_job,
};

/// Failpoint of the compaction, after `write_current_epoch` (ADR-0108).
const FAILPOINT: &str = "after_epoch_switch";

/// Serialises the access to the process-global variable inside this binary.
static ENVIRONMENT: Mutex<()> = Mutex::new(());

/// Arms the failpoint and returns the guard that removes it on drop.
struct Armed<'a> {
    _serial: MutexGuard<'a, ()>,
}

impl<'a> Armed<'a> {
    /// Arms `FAILPOINT` for the current process.
    fn new() -> Self {
        let serial = ENVIRONMENT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // SAFETY: every test of this binary goes through `ENVIRONMENT`, so no
        // other thread reads or writes the environment while the failpoint is
        // armed. The variable is removed on drop.
        unsafe { std::env::set_var("CRIBLE_FAILPOINT", FAILPOINT) };
        Self { _serial: serial }
    }
}

impl Drop for Armed<'_> {
    fn drop(&mut self) {
        // SAFETY: see `Armed::new`; the guard still holds `ENVIRONMENT`.
        unsafe { std::env::remove_var("CRIBLE_FAILPOINT") };
    }
}

/// A compaction that dies after the switch leaves the writer on the epoch named
/// by `CURRENT_EPOCH`, so the next write is not lost (ADR-0108, decision 2).
#[tokio::test]
async fn a_compaction_lost_after_its_switch_reopens_before_the_next_job() {
    let (_instance, state, router) = instance_with_alpha().await;
    let (status, accepted) = post_documents(
        &router,
        online_batch("batch-before-compaction", "le contrat fait la loi", "1240"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    let job = terminal_job(&router, &state, &accepted).await;
    assert_eq!(
        job["state"], "succeeded",
        "the source batch attaches: {job}"
    );
    let before = job["generation"].clone();

    let path = format!("/v1/indexes/{INDEX}/compact?wait=true");
    {
        let _armed = Armed::new();
        let (status, accepted) = send(&router, Method::POST, &path, ADMIN_SECRET, None).await;
        assert!(
            status == StatusCode::OK || status == StatusCode::ACCEPTED,
            "the compaction is accepted: {accepted}"
        );
        let job = terminal_job(&router, &state, &accepted).await;
        assert_eq!(
            job["state"], "failed",
            "the compaction died at its failpoint: {job}"
        );
        assert_eq!(
            job["error"]["code"], "internal",
            "a failed compaction is an internal error: {job}"
        );
    }

    // The store was left on E while `CURRENT_EPOCH` names E+1; the writer must
    // have reopened the index, so this patch commits into E+1 instead of being
    // refused by the epoch guard of ADR-0108, decision 1.
    let path = format!("/v1/indexes/{INDEX}/documents?wait=true");
    let (status, accepted) = send(
        &router,
        Method::PATCH,
        &path,
        WRITE_SECRET,
        Some(json!({
            "documents": [{ "doc_id": DOC_ID, "fields": { "numero": "1241" } }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the patch is queued: {accepted}");
    let job = terminal_job(&router, &state, &accepted).await;
    assert_eq!(
        job["state"], "succeeded",
        "the writer reopened the epoch CURRENT_EPOCH names: {job}"
    );
    let generation = job["generation"]
        .as_u64()
        .unwrap_or_else(|| panic!("the patch publishes a generation: {job}"));
    let before_generation = before
        .as_u64()
        .unwrap_or_else(|| panic!("the source batch published a generation: {before}"));
    assert!(
        generation > before_generation,
        "the patch advanced the generation after the failed compaction: {generation} > \
         {before_generation}"
    );

    // The reads keep answering the last published snapshot, now the one of E+1.
    let (status, document) = send(
        &router,
        Method::GET,
        &format!("/v1/indexes/{INDEX}/documents/{DOC_ID}"),
        WRITE_SECRET,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the document is still read: {document}"
    );
    assert_eq!(
        document["fields"]["numero"], "1241",
        "the document carries the patched field: {document}"
    );
    assert_eq!(
        document["generation"], generation,
        "the read snapshot is the one the patch published: {document}"
    );
}
