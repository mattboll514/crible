// SPDX-License-Identifier: Apache-2.0

//! Write jobs of an index: one writer task, one bounded queue, one job board.
//!
//! References: ADR-0103 (one writer and one bounded queue per index, states,
//! table, retention) and ADR-0104 (jobs accepted before the signal finish, a
//! submission after it is `503 overloaded`, the drain is bounded by
//! `[limits].wait_timeout_ms`), ADR-0067 (`?wait=true`), ADR-0060 and ADR-0042
//! (a report that is not persisted stays in memory).
//!
//! The writer of an index owns its [`GenerationStore`], so every write of that
//! index is serialised (`docs/02-architecture.md`, "Concurrence") and the
//! computation runs on a blocking thread, never on a tokio thread. The queue
//! holds `[limits].writer_queue` **waiting** jobs: the running one has already
//! left it, as ADR-0103 decided.
//!
//! The board keeps the jobs in memory and writes the table at each state
//! transition only. The fine progression travels in a [`JobProgress`] handle,
//! shared with the work, and is persisted at the start and at the end, never in
//! between.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use crible_core::config::IndexConfig;
use crible_core::error::{CribleError, ErrorCode};
use crible_search::scan_pool::ScanPool;
use crible_store::attach::AttachDurations;
use crible_store::generation::{
    GenerationStore, LastBatch, Snapshot, epoch_records_path, epoch_segments_directory,
};
use crible_store::jobs::{JobKind, JobRecord, JobState, JobStore};
use crible_store::readers::DenseReaders;
use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc, watch};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::error;
use crate::lifecycle::Shutdown;

/// Message of the `503 overloaded` a draining instance renders (ADR-0104).
pub const SHUTTING_DOWN_MESSAGE: &str = "the server is shutting down";

/// Message of every job of an index that could not reopen after a failed
/// compaction; the cause is in the server log, once (ADR-0108, decision 2).
const BROKEN_MESSAGE: &str =
    "the index cannot write until the server restarts; its store could not be reopened";

/// One job as the API renders it (`docs/05-api.md`, "Jobs").
///
/// `error` is `{ "code", "message", "details" }` and never carries a
/// `request_id`: a job is not a request (ADR-0103, decision 4).
#[derive(Debug, Clone)]
pub struct JobView {
    /// `job_` followed by 32 lowercase hexadecimal characters.
    pub id: String,
    /// Kind of work.
    pub kind: JobKind,
    /// Index the job writes.
    pub index: String,
    /// Current state.
    pub state: JobState,
    /// Creation, RFC 3339 UTC.
    pub created_at: String,
    /// Start of the run, RFC 3339 UTC.
    pub started_at: Option<String>,
    /// End of the run, RFC 3339 UTC.
    pub finished_at: Option<String>,
    /// Generation the job published, when it has one.
    pub generation: Option<u64>,
    /// Fine progression, `{ "step", "done", "total" }`.
    pub progress: Option<Value>,
    /// Durations of an attach, six whole-millisecond steps, kept in memory and
    /// never persisted (ADR-0149).
    ///
    /// It is `None` when there is nothing to render: a job that is not
    /// `succeeded` yet, a failed attach, a replay, another kind, or a job read
    /// back from the table after a restart. It carries exactly the numbers the
    /// `attach durations` line publishes for the same job, as both come from
    /// the same conversion, `DurationsMillis`.
    pub durations: Option<Value>,
    /// Report of the job, when it is persisted elsewhere.
    pub result: Option<Value>,
    /// Error of a failed job.
    pub error: Option<Value>,
}

impl JobView {
    /// Converts a row of the table into the API view.
    fn from_record(record: JobRecord) -> Result<Self, CribleError> {
        Ok(Self {
            id: record.id.clone(),
            kind: record.kind,
            index: record.index,
            state: record.state,
            created_at: record.created_at,
            started_at: record.started_at,
            finished_at: record.finished_at,
            generation: record.generation,
            progress: parse_optional("progress", record.progress.as_deref(), &record.id)?,
            // The member lives in memory only: the table has no column for it
            // (ADR-0149, decisions 2 and 4), so a row read back renders `null`.
            durations: None,
            result: parse_optional("result", record.result.as_deref(), &record.id)?,
            error: parse_optional("error", record.error.as_deref(), &record.id)?,
        })
    }

    /// Renders the job in the shape of `docs/05-api.md`, "Jobs".
    ///
    /// The four states and the eight kinds are wire names, and every absent
    /// field is `null`, so a client reads the same keys on every job.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "kind": self.kind.as_str(),
            "index": self.index,
            "state": self.state.as_str(),
            "created_at": self.created_at,
            "started_at": self.started_at,
            "finished_at": self.finished_at,
            "generation": self.generation,
            "progress": self.progress,
            "durations": self.durations,
            "result": self.result,
            "error": self.error,
        })
    }
}

/// Parses one optional JSON column, naming the job when it is corrupt.
fn parse_optional(
    column: &str,
    text: Option<&str>,
    id: &str,
) -> Result<Option<Value>, CribleError> {
    text.map(|text| {
        serde_json::from_str(text).map_err(|failure| {
            CribleError::new(
                ErrorCode::Internal,
                format!("job '{id}' has an unreadable {column}: {failure}"),
            )
        })
    })
    .transpose()
}

/// Progression a job publishes while it runs, in memory only between two
/// transitions (ADR-0103, decision 3).
#[derive(Debug, Clone)]
pub struct JobProgress {
    /// Identifier of the job this handle belongs to, so a work can name files
    /// after it (ADR-0105, decision 6).
    id: String,
    /// Value the next state transition writes to the table.
    value: Arc<Mutex<Value>>,
    /// Published view the two job routes read while the job runs (ADR-0134,
    /// decision 3).
    view: watch::Sender<JobView>,
}

impl JobProgress {
    /// Creates the empty progression of the job `id`, published through `view`.
    #[must_use]
    pub fn new(id: impl Into<String>, view: watch::Sender<JobView>) -> Self {
        Self {
            id: id.into(),
            value: Arc::new(Mutex::new(Value::Null)),
            view,
        }
    }

    /// Identifier of the job.
    #[must_use]
    pub fn job_id(&self) -> &str {
        &self.id
    }

    /// Publishes the progression of the job.
    ///
    /// The step is a name of the write protocol — `validation`, `segment`,
    /// `transaction`, `lexical`, `manifest`, `snapshot` —; `done` and `total`
    /// count passages for an attach, documents for a patch, and bytes for a
    /// snapshot, whose steps are `copy`, `attest` and `rename` (`docs/05-api.md`,
    /// "Jobs"; ADR-0134, ADR-0153).
    ///
    /// The value is published **in memory** at once, so `GET /v1/jobs/{id}` and
    /// `GET /v1/jobs` read it while the job runs; the table is written only at
    /// the next state transition (ADR-0103, decision 3).
    pub fn set(&self, step: &str, done: u64, total: u64) {
        if let Ok(mut value) = self.value.lock() {
            *value = json!({ "step": step, "done": done, "total": total });
            self.view
                .send_modify(|view| view.progress = Some(value.clone()));
        }
    }

    /// Marks the last published step as finished: `done` becomes `total` when
    /// the job succeeded (ADR-0134, decision 2).
    ///
    /// A job that published no progression stays at `None`, and a failed job
    /// keeps the last value it published.
    fn complete(&self) {
        if let Ok(mut value) = self.value.lock()
            && let Some(object) = value.as_object_mut()
        {
            let total = object.get("total").and_then(Value::as_u64).unwrap_or(0);
            object.insert("done".to_owned(), json!(total));
            self.view
                .send_modify(|view| view.progress = Some(value.clone()));
        }
    }

    /// Current progression, `None` until the job publishes one.
    #[must_use]
    fn current(&self) -> Option<String> {
        let value = self.value.lock().ok()?.clone();
        if value.is_null() {
            None
        } else {
            Some(value.to_string())
        }
    }
}

/// Outcome of one job: the generation it published and the report it returns.
#[derive(Debug, Clone, Default)]
pub struct JobOutcome {
    /// Generation published by the job, when it has one.
    pub generation: Option<u64>,
    /// Durations of an attach, when its call measured them (ADR-0149).
    ///
    /// The member lives in memory only: the board publishes it with the
    /// terminal transition and never writes it to the table. It is `None` for
    /// every other kind and for the replay of an applied batch, which plays no
    /// step.
    pub durations: Option<Value>,
    /// Report kept with the job, when it is persisted elsewhere (ADR-0042,
    /// ADR-0060).
    pub result: Option<Value>,
}

/// The six steps of an attach in whole milliseconds: the single source of the
/// member `durations` of a job and of the fields of its `attach durations`
/// line (ADR-0149, decisions 1 and 2).
///
/// [`millis`] is the only conversion, so a step cannot render one number in the
/// object and another in the journal line. A step that did not run is `0`, never
/// absent: [`AttachDurations`] measures it and the line writes `0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DurationsMillis {
    /// `validation` step.
    validation_ms: u64,
    /// `segment` step.
    segment_ms: u64,
    /// `transaction` step.
    transaction_ms: u64,
    /// `lexical` step, zero when it does not run.
    lexical_ms: u64,
    /// `manifest` step.
    manifest_ms: u64,
    /// `snapshot` step.
    snapshot_ms: u64,
}

impl DurationsMillis {
    /// Converts the durations of the call that ran the attach.
    pub(crate) fn of(durations: AttachDurations) -> Self {
        Self {
            validation_ms: millis(durations.validation),
            segment_ms: millis(durations.segment),
            transaction_ms: millis(durations.transaction),
            lexical_ms: millis(durations.lexical),
            manifest_ms: millis(durations.manifest),
            snapshot_ms: millis(durations.snapshot),
        }
    }

    /// Renders the closed object of six keys of the member `durations`
    /// (ADR-0149, decision 1).
    pub(crate) fn to_json(self) -> Value {
        json!({
            "validation_ms": self.validation_ms,
            "segment_ms": self.segment_ms,
            "transaction_ms": self.transaction_ms,
            "lexical_ms": self.lexical_ms,
            "manifest_ms": self.manifest_ms,
            "snapshot_ms": self.snapshot_ms,
        })
    }
}

/// Work of a job, run on the writer of its index with the exclusive access to
/// the store, the progression handle and the publication of the index.
///
/// The third argument is the writer itself: a job that changes the configuration
/// publishes the triplet `(config, snapshot, summary)` through it, so a request
/// never reads a mixture (ADR-0105, decision 4). A job that only commits a new
/// generation lets the writer refresh the triplet after it returns.
pub type JobWork = Box<
    dyn FnOnce(&mut GenerationStore, &JobProgress, &IndexWriter) -> Result<JobOutcome, CribleError>
        + Send
        + 'static,
>;

/// One queued job: its identifier, its kind, its work and its progression handle.
struct Command {
    id: String,
    kind: JobKind,
    work: JobWork,
    progress: JobProgress,
}

/// In-memory board of the jobs of an instance, backed by `jobs.sqlite`.
pub struct JobBoard {
    store: JobStore,
    jobs: RwLock<HashMap<String, watch::Sender<JobView>>>,
    pending: AtomicUsize,
    finished: Notify,
    /// Resume point of the drain wait, armed by a unit test only: it stops
    /// [`JobBoard::drained`] between its second count and its `await`, so the
    /// test can prove that a notification emitted in that window is observed
    /// (ADR-0104).
    #[cfg(test)]
    resume: Mutex<Option<DrainResume>>,
}

/// Resume point of the drain wait of a unit test (`cfg(test)` only).
///
/// `reached` tells the test that [`JobBoard::drained`] sits in its window, and
/// `release` lets the test end the wait. The field never exists in a production
/// build.
#[cfg(test)]
struct DrainResume {
    reached: mpsc::UnboundedSender<()>,
    release: mpsc::UnboundedReceiver<()>,
}

impl JobBoard {
    /// Opens the job table, fails the jobs a restart interrupted, purges the
    /// expired ones, and loads the retained jobs into memory.
    ///
    /// Called before any writer starts, as ADR-0103, decision 5, requires.
    ///
    /// # Errors
    ///
    /// The errors of [`JobStore::open_or_create`], of the startup recovery, and
    /// [`ErrorCode::Internal`] for a row of the table this build cannot read.
    pub fn open(path: &std::path::Path) -> Result<Self, CribleError> {
        let store = JobStore::open_or_create(path)?;
        store.fail_interrupted()?;
        store.purge()?;
        let mut jobs = HashMap::new();
        let mut pending = 0;
        for record in store.list(None, None)? {
            let view = JobView::from_record(record)?;
            if !view.state.is_terminal() {
                pending += 1;
            }
            let (sender, _receiver) = watch::channel(view.clone());
            jobs.insert(view.id, sender);
        }
        Ok(Self {
            store,
            jobs: RwLock::new(jobs),
            pending: AtomicUsize::new(pending),
            finished: Notify::new(),
            #[cfg(test)]
            resume: Mutex::new(None),
        })
    }

    /// Accepts a job: reserves its place in the queue, writes its `queued` row,
    /// then enqueues it.
    ///
    /// The place is reserved **before** the row, so a refused job never appears
    /// in the table nor in the list, and no compensation delete is needed
    /// (ADR-0103, decision 9).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Overloaded`] when the queue is full, and the errors of the
    /// table otherwise.
    fn accept(
        &self,
        writer: &mpsc::Sender<Command>,
        index: &str,
        kind: JobKind,
        work: JobWork,
    ) -> Result<JobView, CribleError> {
        let permit = match writer.try_reserve() {
            Ok(permit) => permit,
            Err(mpsc::error::TrySendError::Full(())) => {
                return Err(CribleError::new(
                    ErrorCode::Overloaded,
                    format!("the write queue of index '{index}' is full"),
                ));
            }
            Err(mpsc::error::TrySendError::Closed(())) => {
                return Err(error::internal("the writer of the index is closed"));
            }
        };
        let id = new_job_id();
        let created_at = self.store.insert(&id, kind, index)?;
        let view = JobView {
            id: id.clone(),
            kind,
            index: index.to_owned(),
            state: JobState::Queued,
            created_at,
            started_at: None,
            finished_at: None,
            generation: None,
            progress: None,
            durations: None,
            result: None,
            error: None,
        };
        let (sender, _receiver) = watch::channel(view.clone());
        let progress = JobProgress::new(id.clone(), sender.clone());
        {
            let mut jobs = self.jobs.write().map_err(|_| poisoned("the job board"))?;
            jobs.insert(id.clone(), sender);
        }
        self.pending.fetch_add(1, Ordering::AcqRel);
        permit.send(Command {
            id,
            kind,
            work,
            progress,
        });
        Ok(view)
    }

    /// Writes the start of a job and publishes it.
    fn mark_running(&self, id: &str, progress: &JobProgress) {
        let progress = progress.current();
        match self.store.mark_running(id, progress.as_deref()) {
            Ok(record) => self.publish(record),
            Err(failure) => self.fail_transition(id, failure),
        }
    }

    /// Writes the end of a job and publishes it.
    ///
    /// The report of a `patch_documents` job lives in memory only: ADR-0060 and
    /// ADR-0103, decision 3, say it is never persisted, so a job read back after a
    /// restart carries none. Every other report is already persisted elsewhere
    /// (the `batches` history for an attach), so it is written to the table.
    ///
    /// The job end is also where the `memory` line of ADR-0151 is emitted, for
    /// every kind and every outcome: the job has done its work, and the line
    /// names it.
    fn mark_finished(
        &self,
        id: &str,
        kind: JobKind,
        index: &str,
        outcome: Result<JobOutcome, CribleError>,
        progress: &JobProgress,
    ) {
        crate::memory::log(crate::memory::Point::job_end(id, kind.as_str(), index));
        let progress = progress.current();
        match outcome {
            Ok(outcome) => {
                let in_memory = if kind == JobKind::PatchDocuments {
                    outcome.result.clone()
                } else {
                    None
                };
                // The durations of an attach appear with the terminal
                // transition, in the same published view, so a reader never
                // sees a succeeded attach without them when its call measured
                // some (ADR-0149, decision 2).
                let durations = outcome.durations;
                let persisted = if kind == JobKind::PatchDocuments {
                    None
                } else {
                    outcome.result.map(|value| value.to_string())
                };
                match self.store.finish_succeeded(
                    id,
                    outcome.generation,
                    progress.as_deref(),
                    persisted.as_deref(),
                ) {
                    Ok(record) => self.publish_with(record, in_memory, durations),
                    Err(failure) => self.fail_transition(id, failure),
                }
            }
            Err(failure) => {
                let message = if failure.code() == ErrorCode::Internal {
                    error!(
                        job_id = %id,
                        cause = failure.message(),
                        "a write job failed with an internal error"
                    );
                    error::INTERNAL_MESSAGE
                } else {
                    failure.message()
                };
                let body = job_error_body(failure.code(), message, json!({})).to_string();
                match self.store.finish_failed(id, progress.as_deref(), &body) {
                    Ok(record) => self.publish(record),
                    Err(failure) => self.fail_transition(id, failure),
                }
            }
        }
    }

    /// Fails a job whose transition could not be written, and keeps the writer
    /// alive: the state in memory is the honest one.
    fn fail_transition(&self, id: &str, failure: CribleError) {
        error!(
            job_id = %id,
            cause = failure.message(),
            "the job table refused a state transition"
        );
        let body =
            job_error_body(ErrorCode::Internal, error::INTERNAL_MESSAGE, json!({})).to_string();
        if let Ok(Some(record)) = self.store.job(id)
            && let Ok(view) = JobView::from_record(record)
        {
            let mut failed = view;
            failed.state = JobState::Failed;
            failed.error = serde_json::from_str(&body).ok();
            self.store_and_notify(failed);
        }
    }

    /// Publishes a row and decrements the pending count when it is terminal.
    fn publish(&self, record: JobRecord) {
        self.publish_with(record, None, None);
    }

    /// Publishes a row, adding `in_memory` as the result when the row carries
    /// none, and `durations` as the in-memory durations of an attach. This is
    /// how the report of a `patch_documents` job and the durations of an attach
    /// reach the reader without being persisted (ADR-0060, ADR-0103,
    /// ADR-0149).
    fn publish_with(&self, record: JobRecord, in_memory: Option<Value>, durations: Option<Value>) {
        match JobView::from_record(record) {
            Ok(mut view) => {
                if view.result.is_none() {
                    view.result = in_memory;
                }
                view.durations = durations;
                self.store_and_notify(view);
            }
            Err(failure) => error!(
                cause = failure.message(),
                "a job transitioned to a row this build cannot read"
            ),
        }
    }

    /// Stores `view` in memory, wakes its waiters, and signals the drain when it
    /// is the last unfinished job.
    fn store_and_notify(&self, view: JobView) {
        let terminal = view.state.is_terminal();
        if let Ok(jobs) = self.jobs.read()
            && let Some(sender) = jobs.get(&view.id)
        {
            let _ = sender.send_replace(view.clone());
        }
        if terminal {
            if let Ok(previous) =
                self.pending
                    .try_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                        pending.checked_sub(1)
                    })
                && previous == 1
            {
                self.finished.notify_waiters();
            }
            self.purge();
        }
    }

    /// Purges the jobs finished more than 30 days ago, from the table and from
    /// memory (ADR-0103, decision 6).
    fn purge(&self) {
        match self.store.purge() {
            Ok(ids) => {
                if let Ok(mut jobs) = self.jobs.write() {
                    for id in ids {
                        jobs.remove(&id);
                    }
                }
            }
            Err(failure) => warn!(
                message = failure.message(),
                "the retention of the jobs could not be applied"
            ),
        }
    }

    /// Reads one retained job.
    #[must_use]
    pub fn job(&self, id: &str) -> Option<JobView> {
        self.jobs
            .read()
            .ok()?
            .get(id)
            .map(|sender| sender.borrow().clone())
    }

    /// Lists the retained jobs, filtered by `index` and `state`, by `created_at`
    /// descending then `id` (ADR-0103, decision 7).
    #[must_use]
    pub fn list(&self, index: Option<&str>, state: Option<JobState>) -> Vec<JobView> {
        let Ok(jobs) = self.jobs.read() else {
            return Vec::new();
        };
        let mut views: Vec<JobView> = jobs
            .values()
            .map(|sender| sender.borrow().clone())
            .filter(|view| index.is_none_or(|index| view.index == index))
            .filter(|view| state.is_none_or(|state| view.state == state))
            .collect();
        views.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        views
    }

    /// The jobs that have not finished.
    #[must_use]
    pub fn unfinished(&self) -> Vec<JobView> {
        self.list(None, None)
            .into_iter()
            .filter(|view| !view.state.is_terminal())
            .collect()
    }

    /// The `delete_index` job that holds `index`, if any (ADR-0150,
    /// decision 3).
    ///
    /// A job holds the name until it is terminal: the name is released at the
    /// deregistration, which precedes the terminal state (ADR-0150, decision 4).
    /// When several of them are not terminal, the oldest is returned —
    /// `created_at` ascending, then `id`, the order the jobs of one index run
    /// in (ADR-0103, decision 7; ADR-0150, decision 3, rule 6). It reads the
    /// table in memory only: no file, no snapshot, and no state to reconcile
    /// after a restart (ADR-0150, decisions 3 and 6).
    #[must_use]
    pub fn deleting(&self, index: &str) -> Option<JobView> {
        self.unfinished()
            .into_iter()
            .filter(|view| view.kind == JobKind::DeleteIndex && view.index == index)
            .min_by(|left, right| {
                left.created_at
                    .cmp(&right.created_at)
                    .then_with(|| left.id.cmp(&right.id))
            })
    }

    /// Waits until every accepted job is terminal.
    ///
    /// The future is created **before** the second count: `notified` records the
    /// number of `notify_waiters` calls, so a broadcast after its creation is
    /// observed when the future is polled, and the second count then returns
    /// without waiting for a notification that already happened. `enable` queues
    /// the waiter for `notify_one`, which the board does not send today
    /// (ADR-0104).
    pub async fn drained(&self) {
        loop {
            if self.pending.load(Ordering::Acquire) == 0 {
                return;
            }
            let notified = self.finished.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.pending.load(Ordering::Acquire) == 0 {
                return;
            }
            #[cfg(test)]
            self.reach_resume_point().await;
            notified.await;
        }
    }

    /// Arms the resume point of the next drain wait (unit test only).
    #[cfg(test)]
    fn arm_resume_point(&self, resume: DrainResume) {
        if let Ok(mut slot) = self.resume.lock() {
            *slot = Some(resume);
        }
    }

    /// Stops the drain wait between its second count and its `await`, when a
    /// test armed the resume point.
    ///
    /// That window is where a wait armed too late loses the notification: the
    /// future must exist before the last job announces itself.
    #[cfg(test)]
    async fn reach_resume_point(&self) {
        let point = self.resume.lock().ok().and_then(|mut slot| slot.take());
        if let Some(mut point) = point {
            let _ = point.reached.send(());
            let _ = point.release.recv().await;
        }
    }

    /// Waits for `id` until it is terminal, the deadline of `timeout` expires,
    /// or the server begins to drain (ADR-0067, ADR-0104).
    pub async fn wait_for(
        &self,
        id: &str,
        timeout: Duration,
        shutdown: &Shutdown,
    ) -> Option<JobView> {
        let mut receiver = self.jobs.read().ok()?.get(id)?.subscribe();
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);
        loop {
            let current = receiver.borrow_and_update().clone();
            if current.state.is_terminal() || shutdown.is_draining() {
                return Some(current);
            }
            tokio::select! {
                changed = receiver.changed() => {
                    if changed.is_err() {
                        return Some(current);
                    }
                }
                () = shutdown.wait_draining() => {
                    return Some(receiver.borrow().clone());
                }
                () = &mut deadline => {
                    return Some(receiver.borrow().clone());
                }
            }
        }
    }
}

/// Everything a request reads from one index, published as **one** value.
///
/// ADR-0105, decision 4, requires the configuration and the snapshot to be
/// published together: a request sees the old triplet or the new one, never a
/// configuration with a snapshot of another generation. ADR-0126 adds the dense
/// readers of that generation to the same value: "une génération servie est une
/// génération publiée avec ses lecteurs" (decision 4). The four parts live under
/// one `RwLock`, and a reader takes a single read guard
/// ([`IndexWriter::published`]).
#[derive(Debug, Clone)]
pub struct Published {
    /// Configuration of the index, replaced only at the end of a `PATCH` job.
    pub config: IndexConfig,
    /// Snapshot of the published generation.
    pub snapshot: Arc<Snapshot>,
    /// Counts of that generation, for the two `GET` routes.
    pub summary: IndexSummary,
    /// Readers of the segments of that generation and the payload pool of its
    /// epoch, filled **before** this value was published (ADR-0126,
    /// decisions 1 and 4).
    pub readers: Arc<DenseReaders>,
}

/// Summary of the published state of an index, as the two `GET` routes of
/// `/v1/indexes` render it (ADR-0105, decision 3).
///
/// The writer refreshes it from the store after every job, on the blocking
/// thread that owns the store, so the HTTP routes never read `records.sqlite` of
/// a live index. `segments` is a **count**: the [`SegmentMeta`](crible_store::segment::SegmentMeta)
/// values carry iid bounds and never leave the engine (INV-01).
#[derive(Debug, Clone, Default)]
pub struct IndexSummary {
    /// Epoch directory holding the reference state.
    pub epoch: u64,
    /// Published generation.
    pub generation: u64,
    /// Documents the served generation holds.
    pub documents: u64,
    /// Passages alive at this generation.
    pub live_passages: u64,
    /// Passages tombstoned at this generation or an earlier one.
    pub dead_passages: u64,
    /// Number of segments the generation references.
    pub segments: usize,
    /// Estimated heap size of the filter structures, in bytes.
    pub filter_memory_bytes: u64,
    /// Bytes of RAM the binary codes hold.
    pub codes_memory_bytes: u64,
    /// Last applied batch, or `None` when the epoch holds none.
    pub last_batch: Option<LastBatch>,
    /// Batches the reference state applied, for the daily analytics row
    /// (ADR-0117, decision 1).
    pub batches: u64,
}

impl IndexSummary {
    /// Reads the summary of the generation `store` publishes.
    ///
    /// # Errors
    ///
    /// The errors of the `batches` queries and of the passage counts of the
    /// snapshot.
    pub fn of(store: &GenerationStore) -> Result<Self, CribleError> {
        Self::of_snapshot(store, &store.snapshot())
    }

    /// Reads the summary of `snapshot`, a generation `store` already holds.
    ///
    /// The counts describe `snapshot` alone: the documents and the passages were
    /// recorded when it was built, and the live passages are its bitmap
    /// (ADR-0105, decision 3). Only `last_batch` and `batches` still come from
    /// the reference state, and that asymmetry is wanted: `last_batch` is the
    /// last row of `batches`, whatever generation served the readers.
    ///
    /// A `PATCH` job builds its snapshot with
    /// [`GenerationStore::reconfigure`](crible_store::generation::GenerationStore::reconfigure)
    /// and computes the summary from that snapshot **before** publishing the
    /// triplet, so the counts and the configuration describe the same state.
    ///
    /// # Errors
    ///
    /// The errors of the `batches` queries and of the passage counts of
    /// `snapshot`.
    pub fn of_snapshot(
        store: &GenerationStore,
        snapshot: &Arc<Snapshot>,
    ) -> Result<Self, CribleError> {
        Ok(Self {
            epoch: u64::from(snapshot.epoch().get()),
            generation: snapshot.generation().get(),
            documents: snapshot.documents(),
            live_passages: snapshot.live_passages(),
            dead_passages: snapshot.dead_passages()?,
            segments: snapshot.segments().len(),
            filter_memory_bytes: u64::try_from(snapshot.filter_memory().total())
                .unwrap_or(u64::MAX),
            codes_memory_bytes: snapshot.codes_memory(),
            last_batch: store.last_batch()?,
            batches: store.batch_count()?,
        })
    }

    /// Ratio of dead passages among all the passages, or `0` when the epoch
    /// holds none.
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

/// One writer task per index, owning the writable store of that index.
pub struct IndexWriter {
    index: String,
    /// Directory of the index on disk, kept to reopen the store after a failed
    /// compaction (ADR-0108).
    directory: PathBuf,
    /// Sender of the jobs waiting for this writer.
    ///
    /// It is an `Option` because a deletion closes the queue: the sender lives
    /// inside this writer, which the task holds through its own `Arc`, so the
    /// channel would never close on its own, and the task would keep its store
    /// until the process ends. [`IndexWriter::mark_deleted`] takes it and drops
    /// it, which ends the drain of the queue and the task (ADR-0105, decision 6,
    /// for the deletion; `docs/02-architecture.md`, "Concurrence", for the writer
    /// task and its bounded queue).
    queue: Mutex<Option<mpsc::Sender<Command>>>,
    receiver: Mutex<Option<mpsc::Receiver<Command>>>,
    store: Mutex<Option<GenerationStore>>,
    board: Arc<JobBoard>,
    published: RwLock<Published>,
    /// Dedicated pool of the scans, read here only for its worker count: the
    /// payload pool of a generation takes one read-only connection per scan
    /// worker, as the searcher has always sized it (ADR-0126, decision 1).
    scan_pool: Arc<ScanPool>,
    deleted: AtomicBool,
    /// Set when the store could not be reopened after a failed compaction: the
    /// index still serves reads on its last snapshot, and every later job fails
    /// in `internal` until the restart (ADR-0108, decision 2).
    broken: AtomicBool,
}

impl IndexWriter {
    /// Builds the writer of `index`, with a queue of `writer_queue` waiting
    /// jobs.
    ///
    /// A declared depth of zero is read as one: a job must be acceptable at
    /// all, and `docs/07` fixes the default at 16.
    ///
    /// The dense readers of the generation the store publishes are filled here,
    /// **before** the published value exists: a startup and a creation serve no
    /// generation without them (ADR-0126, decision 4). The fill pays the blake3
    /// check of every segment once per process, which is the cost of the
    /// restart.
    ///
    /// # Errors
    ///
    /// The errors of the first summary of the store, and the errors of the fill
    /// of its readers: a segment that does not open, or whose checksum does not
    /// match, keeps the index from being served.
    pub fn new(
        index: String,
        store: GenerationStore,
        config: IndexConfig,
        writer_queue: usize,
        board: Arc<JobBoard>,
        scan_pool: Arc<ScanPool>,
    ) -> Result<Self, CribleError> {
        let snapshot = store.snapshot();
        let summary = IndexSummary::of_snapshot(&store, &snapshot)?;
        let directory = store.index_dir().to_path_buf();
        let readers = fill_readers(
            &directory,
            &snapshot,
            &DenseReaders::empty(),
            scan_pool.threads(),
        )?;
        let (queue, receiver) = mpsc::channel(writer_queue.max(1));
        Ok(Self {
            index,
            directory,
            queue: Mutex::new(Some(queue)),
            receiver: Mutex::new(Some(receiver)),
            store: Mutex::new(Some(store)),
            board,
            published: RwLock::new(Published {
                config,
                snapshot,
                summary,
                readers,
            }),
            scan_pool,
            deleted: AtomicBool::new(false),
            broken: AtomicBool::new(false),
        })
    }

    /// The triplet `(config, snapshot, summary)` of the index, read under one
    /// guard so the three parts always describe the same state (ADR-0105,
    /// decision 4).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the lock is poisoned.
    pub fn published(&self) -> Result<Published, CribleError> {
        self.published
            .read()
            .map(|published| published.clone())
            .map_err(|_| error::internal("the published state of an index is poisoned"))
    }

    /// Configuration currently published for the index.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the lock is poisoned.
    pub fn configuration(&self) -> Result<IndexConfig, CribleError> {
        self.published().map(|published| published.config)
    }

    /// Publishes a new triplet, atomically for every reader.
    ///
    /// The readers of `snapshot` are filled **before** the write lock is taken:
    /// a request keeps reading the previous published value while the new
    /// segments open, and the lock is never held across a fill (ADR-0126,
    /// decision 4). The readers already open for a segment of the new generation
    /// are reused, so a metadata patch opens nothing.
    ///
    /// # Errors
    ///
    /// The errors of the fill — a segment that does not open, or whose checksum
    /// does not match — and [`ErrorCode::Internal`] when the lock is poisoned.
    /// A failed fill publishes nothing: the index keeps serving the previous
    /// generation with its readers.
    pub fn publish(
        &self,
        config: IndexConfig,
        snapshot: Arc<Snapshot>,
        summary: IndexSummary,
    ) -> Result<(), CribleError> {
        let readers = self.fill_readers(&snapshot)?;
        let mut published = self
            .published
            .write()
            .map_err(|_| error::internal("the published state of an index is poisoned"))?;
        *published = Published {
            config,
            snapshot,
            summary,
            readers,
        };
        Ok(())
    }

    /// Republishes the snapshot, the summary and the readers of the store after
    /// a job, and returns the error the job must report.
    ///
    /// The refresh is **unconditional**: a job that fails *after* its commit
    /// point has still committed a generation (`crates/crible-store/src/attach.rs`,
    /// "An error raised after the commit point"), so skipping the refresh would
    /// leave the published state behind the store until the next restart, against
    /// INV-03 and INV-04. The error of the work stays the one the job reports,
    /// and a failure of the refresh is only logged: it never hides a failure of
    /// the work (ADR-0126, decision 4).
    ///
    /// # Errors
    ///
    /// The errors of the fill — a segment that does not open, or whose checksum
    /// does not match — and [`ErrorCode::Internal`] when the lock is poisoned,
    /// returned only when the work itself succeeded.
    fn refresh_after_job(
        &self,
        store: &GenerationStore,
        snapshot: Arc<Snapshot>,
        outcome: Result<JobOutcome, CribleError>,
    ) -> Result<JobOutcome, CribleError> {
        let summary = match IndexSummary::of_snapshot(store, &snapshot) {
            Ok(summary) => summary,
            Err(failure) => {
                error!(
                    index = %self.index,
                    cause = failure.message(),
                    "the counts of the generation could not be read; the published state is \
                     unchanged"
                );
                return outcome;
            }
        };
        match (outcome, self.refresh(snapshot, summary)) {
            (outcome, Ok(())) => outcome,
            // ADR-0126, decision 4: a fill that fails publishes nothing, and the
            // failure goes back to the caller of the commit. The index keeps
            // serving its previous generation, with the readers that generation
            // already holds.
            (Ok(_), Err(failure)) => Err(failure),
            (outcome @ Err(_), Err(failure)) => {
                error!(
                    index = %self.index,
                    cause = failure.message(),
                    "the readers of the generation could not be filled; the published state is \
                     unchanged, and the failure of the work is the one reported"
                );
                outcome
            }
        }
    }

    /// Refreshes the snapshot, the summary and the readers after a job that
    /// committed, keeping the configuration: the parts still change under one
    /// write lock.
    ///
    /// # Errors
    ///
    /// The errors of the fill and [`ErrorCode::Internal`] when the lock is
    /// poisoned.
    fn refresh(&self, snapshot: Arc<Snapshot>, summary: IndexSummary) -> Result<(), CribleError> {
        let readers = self.fill_readers(&snapshot)?;
        let mut published = self
            .published
            .write()
            .map_err(|_| error::internal("the published state of an index is poisoned"))?;
        published.snapshot = snapshot;
        published.summary = summary;
        published.readers = readers;
        Ok(())
    }

    /// Opens the readers the generation of `snapshot` adds, reusing the ones the
    /// published state already holds, outside every write lock.
    ///
    /// # Errors
    ///
    /// The errors of [`DenseReaders::filled`], plus [`ErrorCode::Internal`] when
    /// the published state is poisoned.
    fn fill_readers(&self, snapshot: &Snapshot) -> Result<Arc<DenseReaders>, CribleError> {
        let current = self
            .published
            .read()
            .map_err(|_| error::internal("the published state of an index is poisoned"))?
            .readers
            .clone();
        fill_readers(
            &self.directory,
            snapshot,
            &current,
            self.scan_pool.threads(),
        )
    }

    /// Marks the index as deleted, so the writer fails the jobs queued behind
    /// `delete_index` in `unknown_index` and stops (ADR-0105, decision 6).
    ///
    /// It also closes the write queue. The sender belongs to this writer, which
    /// the task holds through its own `Arc`; nobody else could drop it, so
    /// `recv` would never return `None`, the task would never end, and the store
    /// with its SQLite and tantivy descriptors would stay until the process
    /// ended. Taking the sender here is the signal that ends the drain of
    /// `fail_queued` and the task itself, which drops the store.
    pub fn mark_deleted(&self) {
        self.deleted.store(true, Ordering::Release);
        match self.queue.lock() {
            // Dropping the sender inside the guard closes the channel after the
            // submissions already holding the guard: a job is therefore either
            // enqueued, and failed by [`fail_queued`], or refused, never lost.
            Ok(mut queue) => {
                queue.take();
            }
            Err(poisoned) => {
                // A panic in a submission must not leave the queue open: the
                // task, its store and their descriptors would stay retained,
                // which is the defect this method closes.
                warn!(
                    index = %self.index,
                    "the write queue of a deleted index is poisoned; it is closed anyway"
                );
                poisoned.into_inner().take();
            }
        }
    }

    /// Name of the index this writer serves.
    #[must_use]
    pub fn index(&self) -> &str {
        &self.index
    }

    /// Starts the writer task once, when a runtime is available.
    ///
    /// The task is idempotent: a second call sees the receiver gone and returns.
    /// The submission also calls it, so a writer that no startup started still
    /// runs its first job.
    pub fn ensure_started(self: &Arc<Self>) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let receiver = match self.receiver.lock() {
            Ok(mut receiver) => receiver.take(),
            Err(_) => {
                warn!(index = %self.index, "the writer receiver is poisoned");
                return;
            }
        };
        let Some(receiver) = receiver else {
            return;
        };
        let store = match self.store.lock() {
            Ok(mut store) => store.take(),
            Err(_) => {
                warn!(index = %self.index, "the writer store is poisoned");
                return;
            }
        };
        let Some(store) = store else {
            warn!(index = %self.index, "the writer store is gone");
            return;
        };
        let writer = Arc::clone(self);
        handle.spawn(async move { writer.run(receiver, store).await });
    }

    /// Accepts a job, or refuses it when the queue is full or the index was
    /// deleted.
    ///
    /// The queue guard is held for the whole acceptance, the insertion of the
    /// job row and the send included: [`Self::mark_deleted`] takes the same
    /// guard to close the queue, so a job is either enqueued before the close —
    /// and failed in `unknown_index` by `fail_queued` — or refused here, never
    /// reserved and then dropped with its row left `queued`.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Overloaded`] when the queue is full,
    /// [`ErrorCode::UnknownIndex`] when the index was deleted, and the errors of
    /// the job table.
    pub fn submit(&self, kind: JobKind, work: JobWork) -> Result<JobView, CribleError> {
        let queue = self
            .queue
            .lock()
            .map_err(|_| poisoned("the write queue of an index"))?;
        let Some(queue) = queue.as_ref() else {
            return Err(CribleError::new(
                ErrorCode::UnknownIndex,
                format!("unknown index '{}'", self.index),
            ));
        };
        self.board.accept(queue, &self.index, kind, work)
    }

    /// Runs the queue until it is closed: the running job and every waiting one
    /// run, in arrival order, on a blocking thread.
    ///
    /// The loop ends when [`Self::mark_deleted`] closes the queue, after
    /// [`fail_queued`] failed the commands still buffered. The store this task
    /// owns is then dropped on a blocking thread, so a deleted index releases
    /// its descriptors and its disk space off the async threads, like every
    /// other store operation (ADR-0103, decision 8).
    ///
    /// A failed `compact` abandons the in-memory store and reopens the index from
    /// disk, then republishes the snapshot before the next job (ADR-0108,
    /// decision 2). If that reopen fails, the index keeps serving its last
    /// snapshot and the later jobs fail in `internal` until the restart, with a
    /// single `error` line.
    async fn run(self: Arc<Self>, receiver: mpsc::Receiver<Command>, store: GenerationStore) {
        let holder = self.run_queue(receiver, store).await;
        self.release_store_off_threads(holder).await;
    }

    /// The loop of [`Self::run`]: it returns the store it still holds when the
    /// queue closes.
    async fn run_queue(
        self: &Arc<Self>,
        mut receiver: mpsc::Receiver<Command>,
        store: GenerationStore,
    ) -> Option<GenerationStore> {
        let mut holder = Some(store);
        while let Some(command) = receiver.recv().await {
            self.board.mark_running(&command.id, &command.progress);
            let Some(current) = holder.take() else {
                self.board.mark_finished(
                    &command.id,
                    command.kind,
                    &self.index,
                    Err(error::internal(BROKEN_MESSAGE)),
                    &command.progress,
                );
                continue;
            };
            if self.broken.load(Ordering::Acquire) {
                holder = Some(current);
                self.board.mark_finished(
                    &command.id,
                    command.kind,
                    &self.index,
                    Err(error::internal(BROKEN_MESSAGE)),
                    &command.progress,
                );
                continue;
            }
            let progress = command.progress.clone();
            let kind = command.kind;
            let work = command.work;
            let id = command.id.clone();
            let writer = Arc::clone(self);
            let joined = tokio::task::spawn_blocking(move || {
                let mut store = current;
                let outcome = work(&mut store, &progress, &writer);
                let snapshot = store.snapshot();
                let outcome = writer.refresh_after_job(&store, snapshot, outcome);
                (store, outcome)
            })
            .await;
            match joined {
                Ok((returned, outcome)) => {
                    let failed = outcome.is_err();
                    holder = Some(returned);
                    if !failed {
                        // ADR-0134, decision 2: a successful job ends at `total`,
                        // on the step it last published.
                        command.progress.complete();
                    }
                    self.board
                        .mark_finished(&id, kind, &self.index, outcome, &command.progress);
                    if failed && kind == JobKind::Compact {
                        holder = self.reopen_off_threads(holder.take()).await;
                    }
                    if self.deleted.load(Ordering::Acquire) {
                        fail_queued(&mut receiver, &self.board, &self.index).await;
                        return holder;
                    }
                }
                Err(join) => {
                    // The store is lost with a panicking job. The job is failed,
                    // and the writer goes on from a reopened store when it can.
                    self.board.mark_finished(
                        &id,
                        kind,
                        &self.index,
                        Err(error::internal(format!(
                            "the blocking task of a write job failed: {join}"
                        ))),
                        &command.progress,
                    );
                    // A panic during a compaction is an aborted compaction: the
                    // same reopen as a returned error, without a store to drop,
                    // then the next job runs on the epoch `CURRENT_EPOCH` names
                    // (ADR-0108, decision 2). A store that cannot reopen leaves
                    // the writer broken: every later job fails in `internal` and
                    // the reads keep their last snapshot.
                    holder = self.reopen_off_threads(None).await;
                    if self.deleted.load(Ordering::Acquire) {
                        fail_queued(&mut receiver, &self.board, &self.index).await;
                        return holder;
                    }
                    continue;
                }
            }
        }
        holder
    }

    /// Drops the store a closed queue left, on a blocking thread.
    ///
    /// Closing the SQLite connection of the write epoch — with the WAL
    /// checkpoint of its last descriptor — and the tantivy writer is a store
    /// operation, paid off the async threads like every other (ADR-0103,
    /// decision 8). The index directory is already gone, so the cost is small,
    /// but the placement is the one of [`Self::reopen_off_threads`].
    async fn release_store_off_threads(&self, holder: Option<GenerationStore>) {
        let Some(store) = holder else {
            return;
        };
        let joined = tokio::task::spawn_blocking(move || {
            drop(store);
        })
        .await;
        if let Err(join) = joined {
            error!(
                index = %self.index,
                cause = %join,
                "the store of a deleted index panicked while it was released"
            );
        }
    }

    /// Abandons `previous` and reopens the index from disk, off the async
    /// threads, then republishes the snapshot and the summary (ADR-0108,
    /// decision 2).
    ///
    /// The reopen runs the recovery of `docs/02` and rebuilds the derived
    /// structures: it is a store operation, paid on a blocking thread like every
    /// other (ADR-0054). The failed store is dropped inside the same blocking
    /// task, because the write open needs the SQLite writer lock and the tantivy
    /// writer lock the old one still holds. `mark_broken` is called after the
    /// task returns, never from a store thread.
    async fn reopen_off_threads(
        self: &Arc<Self>,
        previous: Option<GenerationStore>,
    ) -> Option<GenerationStore> {
        let writer = Arc::clone(self);
        let joined = tokio::task::spawn_blocking(move || match previous {
            Some(previous) => writer.reopen(previous),
            None => writer.reopen_without_store(),
        })
        .await;
        match joined {
            Ok(Ok(reopened)) => Some(reopened),
            Ok(Err(())) => {
                self.mark_broken();
                None
            }
            Err(join) => {
                error!(
                    index = %self.index,
                    cause = %join,
                    "the reopen of an index panicked; it cannot write until the restart"
                );
                self.mark_broken();
                None
            }
        }
    }

    /// Abandons `previous` and reopens the index from disk, then republishes the
    /// snapshot and the summary (ADR-0108, decision 2).
    ///
    /// The failed store is dropped first: the write open needs the SQLite writer
    /// lock and the tantivy writer lock the old one still holds. The reopen runs
    /// the recovery of `docs/02`, which republishes the epoch `CURRENT_EPOCH`
    /// names.
    fn reopen(&self, previous: GenerationStore) -> Result<GenerationStore, ()> {
        drop(previous);
        self.reopen_without_store()
    }

    /// Reopens the index from disk and republishes its state.
    ///
    /// The cause of a failure is logged at `warn`: the single `error` line of a
    /// broken index is [`Self::mark_broken`]'s, so a failure of the reopen leaves
    /// one error line and not one per failure nor one per refused job (ADR-0108,
    /// decision 2).
    fn reopen_without_store(&self) -> Result<GenerationStore, ()> {
        let config = self.configuration().map_err(|failure| {
            warn!(
                index = %self.index,
                cause = failure.message(),
                "the published configuration of an index is unreadable; the index cannot reopen"
            );
        })?;
        let store = GenerationStore::open(&self.directory, &config.schema).map_err(|failure| {
            warn!(
                index = %self.index,
                cause = failure.message(),
                "the index could not reopen after a failed compaction"
            );
        })?;
        let snapshot = store.snapshot();
        let summary = IndexSummary::of_snapshot(&store, &snapshot).map_err(|failure| {
            warn!(
                index = %self.index,
                cause = failure.message(),
                "the reopened index could not publish its summary"
            );
        })?;
        self.publish(config, snapshot, summary).map_err(|failure| {
            warn!(
                index = %self.index,
                cause = failure.message(),
                "the reopened index could not publish its snapshot"
            );
        })?;
        Ok(store)
    }

    /// Marks the writer as unable to write until the restart, with one `error`
    /// line (ADR-0108, decision 2).
    fn mark_broken(&self) {
        if !self.broken.swap(true, Ordering::AcqRel) {
            error!(
                index = %self.index,
                "the index cannot reopen after a failed compaction; it still serves reads and \
                 its jobs fail with an internal error until the server restarts (ADR-0108)"
            );
        }
    }
}

/// Fails every job still queued behind a `delete_index`, in `unknown_index`
/// (ADR-0105, decision 6).
async fn fail_queued(receiver: &mut mpsc::Receiver<Command>, board: &JobBoard, index: &str) {
    while let Some(command) = receiver.recv().await {
        board.mark_running(&command.id, &command.progress);
        board.mark_finished(
            &command.id,
            command.kind,
            index,
            Err(CribleError::new(
                ErrorCode::UnknownIndex,
                format!("unknown index '{index}'"),
            )),
            &command.progress,
        );
    }
}

/// Opens the dense readers of the generation `snapshot` of the index at
/// `directory`, reusing the ones `current` already holds.
///
/// It is the only code that fills a cache of readers: the startup and creation
/// call it on an empty cache, and every publication calls it on the published
/// one (ADR-0126, decisions 3 and 4). It touches no lock, so the caller decides
/// when the publication happens.
///
/// # Errors
///
/// The errors of [`DenseReaders::filled`].
fn fill_readers(
    directory: &Path,
    snapshot: &Snapshot,
    current: &DenseReaders,
    connections: usize,
) -> Result<Arc<DenseReaders>, CribleError> {
    let segments_directory = epoch_segments_directory(directory, snapshot.epoch());
    let records_path = epoch_records_path(directory, snapshot.epoch());
    let filled = current.filled(
        &segments_directory,
        snapshot.segments(),
        &records_path,
        connections,
    )?;
    Ok(Arc::new(filled))
}

/// Logs the durations of an attach once, at the end of its job (ADR-0149;
/// replaces ADR-0107, decision 4, for the response only).
///
/// The six numbers come from [`DurationsMillis`], the same value the member
/// `durations` of the job renders, so the line and the member cannot diverge.
/// It names the job, its kind and its index, never a document, a field value or
/// a passage text (INV-13, ADR-0096).
pub(crate) fn log_attach_durations(
    job_id: &str,
    kind: JobKind,
    index: &str,
    durations: Option<DurationsMillis>,
) {
    let Some(durations) = durations else {
        return;
    };
    info!(
        job_id = %job_id,
        kind = kind.as_str(),
        index = %index,
        validation_ms = durations.validation_ms,
        segment_ms = durations.segment_ms,
        transaction_ms = durations.transaction_ms,
        lexical_ms = durations.lexical_ms,
        manifest_ms = durations.manifest_ms,
        snapshot_ms = durations.snapshot_ms,
        "attach durations"
    );
}

/// One duration in whole milliseconds, the unit of [`DurationsMillis`], so of
/// the member `durations` and of the line `attach durations` (ADR-0149).
fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// A fresh job identifier: `job_` followed by 32 lowercase hexadecimal
/// characters (ADR-0103, decision 2).
#[must_use]
pub fn new_job_id() -> String {
    format!("job_{}", Uuid::new_v4().simple())
}

/// Renders the `{ "code", "message", "details" }` body of a job error
/// (ADR-0101). A job is not a request: no `request_id` (ADR-0103, decision 4).
fn job_error_body(code: ErrorCode, message: &str, details: Value) -> Value {
    json!({
        "code": code.as_str(),
        "message": message,
        "details": details,
    })
}

/// Builds an [`ErrorCode::Internal`] error for a poisoned lock.
fn poisoned(what: &str) -> CribleError {
    error::internal(format!("{what} is poisoned"))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use tokio::sync::mpsc;

    use super::{DrainResume, JobBoard};

    /// A notification sent after the last count of `pending` is not lost.
    ///
    /// `notify_waiters` keeps no permit: the waiter must exist before the last
    /// job announces itself. `notified` records the number of `notify_waiters`
    /// calls, so a broadcast after its creation is observed; the wait creates
    /// its future before the second count, so the notification emitted in the
    /// resume point below is observed even before the future is polled.
    /// A wait armed any later would miss it, and the timeout below would fail
    /// (ADR-0104).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_notification_between_the_last_count_and_the_wait_is_not_lost() {
        let directory = tempfile::tempdir().expect("a temporary directory is creatable");
        let board = Arc::new(
            JobBoard::open(&directory.path().join("jobs.sqlite"))
                .expect("the job table is creatable"),
        );
        let (reached, mut in_window) = mpsc::unbounded_channel();
        let (release, released) = mpsc::unbounded_channel();
        board.arm_resume_point(DrainResume {
            reached,
            release: released,
        });
        // One accepted job, still running.
        board.pending.fetch_add(1, Ordering::AcqRel);

        let waiting = {
            let board = Arc::clone(&board);
            tokio::spawn(async move { board.drained().await })
        };
        // The waiter sits between its second count and its `await`.
        in_window
            .recv()
            .await
            .expect("the drain wait reaches the resume point");

        // The last job finishes exactly in that window: the notification must
        // be observed by the wait even before its future is polled.
        board.pending.fetch_sub(1, Ordering::AcqRel);
        board.finished.notify_waiters();
        release.send(()).expect("the wait resumes");

        tokio::time::timeout(Duration::from_secs(2), waiting)
            .await
            .expect("the armed wait is woken by the notification of the last job")
            .expect("the wait task joins");
    }
}
