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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use crible_core::config::IndexConfig;
use crible_core::error::{CribleError, ErrorCode};
use crible_store::generation::{GenerationStore, LastBatch, Snapshot};
use crible_store::jobs::{JobKind, JobRecord, JobState, JobStore};
use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc, watch};
use tracing::{error, warn};
use uuid::Uuid;

use crate::error;
use crate::lifecycle::Shutdown;

/// Message of the `503 overloaded` a draining instance renders (ADR-0104).
pub const SHUTTING_DOWN_MESSAGE: &str = "the server is shutting down";

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
    value: Arc<Mutex<Value>>,
}

impl JobProgress {
    /// Creates the empty progression of the job `id`.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            value: Arc::new(Mutex::new(Value::Null)),
        }
    }

    /// Identifier of the job.
    #[must_use]
    pub fn job_id(&self) -> &str {
        &self.id
    }

    /// Publishes the progression of the job.
    ///
    /// The step is a server word (`validation`, `commit`), `done` and `total`
    /// count passages for an attach and documents for a patch
    /// (`docs/05-api.md`, "Jobs").
    pub fn set(&self, step: &str, done: u64, total: u64) {
        if let Ok(mut value) = self.value.lock() {
            *value = json!({ "step": step, "done": done, "total": total });
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
    /// Report kept with the job, when it is persisted elsewhere (ADR-0042,
    /// ADR-0060).
    pub result: Option<Value>,
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

/// One queued job: its identifier, its work and its progression handle.
struct Command {
    id: String,
    work: JobWork,
    progress: JobProgress,
}

/// In-memory board of the jobs of an instance, backed by `jobs.sqlite`.
pub struct JobBoard {
    store: JobStore,
    jobs: RwLock<HashMap<String, watch::Sender<JobView>>>,
    pending: AtomicUsize,
    finished: Notify,
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
        let progress = JobProgress::new(id.clone());
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
            result: None,
            error: None,
        };
        {
            let mut jobs = self.jobs.write().map_err(|_| poisoned("the job board"))?;
            let (sender, _receiver) = watch::channel(view.clone());
            jobs.insert(id.clone(), sender);
        }
        self.pending.fetch_add(1, Ordering::AcqRel);
        permit.send(Command { id, work, progress });
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
    fn mark_finished(
        &self,
        id: &str,
        outcome: Result<JobOutcome, CribleError>,
        progress: &JobProgress,
    ) {
        let progress = progress.current();
        match outcome {
            Ok(outcome) => {
                let result = outcome.result.map(|value| value.to_string());
                match self.store.finish_succeeded(
                    id,
                    outcome.generation,
                    progress.as_deref(),
                    result.as_deref(),
                ) {
                    Ok(record) => self.publish(record),
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
        match JobView::from_record(record) {
            Ok(view) => self.store_and_notify(view),
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

    /// Waits until every accepted job is terminal.
    ///
    /// The future is pinned and registered with [`Notified::enable`] before the
    /// second count, so the last notification, emitted between the two counts,
    /// is never lost: `notify_waiters` keeps no permit and only wakes a waiter
    /// already registered (ADR-0104).
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
            notified.await;
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
/// configuration with a snapshot of another generation. The three parts live
/// under one `RwLock`, and a reader takes a single read guard
/// ([`IndexWriter::published`]).
#[derive(Debug, Clone)]
pub struct Published {
    /// Configuration of the index, replaced only at the end of a `PATCH` job.
    pub config: IndexConfig,
    /// Snapshot of the published generation.
    pub snapshot: Arc<Snapshot>,
    /// Counts of that generation, for the two `GET` routes.
    pub summary: IndexSummary,
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
    /// Documents the reference state holds.
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
}

impl IndexSummary {
    /// Reads the summary of the generation `store` publishes.
    ///
    /// # Errors
    ///
    /// The errors of [`GenerationStore::state`] and of the `batches` query.
    pub fn of(store: &GenerationStore) -> Result<Self, CribleError> {
        Self::of_snapshot(store, &store.snapshot())
    }

    /// Reads the summary of `snapshot`, a generation `store` already holds.
    ///
    /// A `PATCH` job builds its snapshot with
    /// [`GenerationStore::reconfigure`](crible_store::generation::GenerationStore::reconfigure)
    /// and computes the summary from that snapshot **before** publishing the
    /// triplet, so the counts and the configuration describe the same state.
    ///
    /// # Errors
    ///
    /// The errors of the `batches` query and of the reference state of
    /// `snapshot`.
    pub fn of_snapshot(
        store: &GenerationStore,
        snapshot: &Arc<Snapshot>,
    ) -> Result<Self, CribleError> {
        let state = store.state()?;
        Ok(Self {
            epoch: u64::from(state.epoch.get()),
            generation: snapshot.generation().get(),
            documents: state.documents,
            live_passages: state.live_passages,
            dead_passages: state.dead_passages,
            segments: state.segments.len(),
            filter_memory_bytes: u64::try_from(snapshot.filter_memory().total())
                .unwrap_or(u64::MAX),
            codes_memory_bytes: snapshot.codes_memory(),
            last_batch: store.last_batch()?,
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
    queue: mpsc::Sender<Command>,
    receiver: Mutex<Option<mpsc::Receiver<Command>>>,
    store: Mutex<Option<GenerationStore>>,
    board: Arc<JobBoard>,
    published: RwLock<Published>,
    deleted: AtomicBool,
}

impl IndexWriter {
    /// Builds the writer of `index`, with a queue of `writer_queue` waiting
    /// jobs.
    ///
    /// A declared depth of zero is read as one: a job must be acceptable at
    /// all, and `docs/07` fixes the default at 16.
    ///
    /// # Errors
    ///
    /// The errors of the first summary of the store.
    pub fn new(
        index: String,
        store: GenerationStore,
        config: IndexConfig,
        writer_queue: usize,
        board: Arc<JobBoard>,
    ) -> Result<Self, CribleError> {
        let snapshot = store.snapshot();
        let summary = IndexSummary::of_snapshot(&store, &snapshot)?;
        let (queue, receiver) = mpsc::channel(writer_queue.max(1));
        Ok(Self {
            index,
            queue,
            receiver: Mutex::new(Some(receiver)),
            store: Mutex::new(Some(store)),
            board,
            published: RwLock::new(Published {
                config,
                snapshot,
                summary,
            }),
            deleted: AtomicBool::new(false),
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
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the lock is poisoned.
    pub fn publish(
        &self,
        config: IndexConfig,
        snapshot: Arc<Snapshot>,
        summary: IndexSummary,
    ) -> Result<(), CribleError> {
        let mut published = self
            .published
            .write()
            .map_err(|_| error::internal("the published state of an index is poisoned"))?;
        *published = Published {
            config,
            snapshot,
            summary,
        };
        Ok(())
    }

    /// Refreshes the snapshot and the summary after a job that committed, keeping
    /// the configuration: the three parts still change under one write lock.
    fn refresh(&self, snapshot: Arc<Snapshot>, summary: IndexSummary) -> Result<(), CribleError> {
        let mut published = self
            .published
            .write()
            .map_err(|_| error::internal("the published state of an index is poisoned"))?;
        published.snapshot = snapshot;
        published.summary = summary;
        Ok(())
    }

    /// Marks the index as deleted, so the writer fails the jobs queued behind
    /// `delete_index` in `unknown_index` and stops (ADR-0105, decision 6).
    pub fn mark_deleted(&self) {
        self.deleted.store(true, Ordering::Release);
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

    /// Accepts a job, or refuses it when the queue is full.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Overloaded`] when the queue is full, and the errors of the
    /// job table.
    pub fn submit(&self, kind: JobKind, work: JobWork) -> Result<JobView, CribleError> {
        self.board.accept(&self.queue, &self.index, kind, work)
    }

    /// Runs the queue until every sender is dropped: the running job and every
    /// waiting one run, in arrival order, on a blocking thread.
    async fn run(
        self: Arc<Self>,
        mut receiver: mpsc::Receiver<Command>,
        mut store: GenerationStore,
    ) {
        while let Some(command) = receiver.recv().await {
            self.board.mark_running(&command.id, &command.progress);
            let progress = command.progress.clone();
            let work = command.work;
            let id = command.id.clone();
            let writer = Arc::clone(&self);
            let joined = tokio::task::spawn_blocking(move || {
                let outcome = work(&mut store, &progress, &writer);
                let snapshot = store.snapshot();
                if let Ok(summary) = IndexSummary::of_snapshot(&store, &snapshot)
                    && let Err(failure) = writer.refresh(snapshot, summary)
                {
                    error!(
                        index = %writer.index,
                        cause = failure.message(),
                        "the published state of the index could not be refreshed"
                    );
                }
                (store, outcome)
            })
            .await;
            match joined {
                Ok((returned, outcome)) => {
                    store = returned;
                    self.board.mark_finished(&id, outcome, &command.progress);
                    if self.deleted.load(Ordering::Acquire) {
                        fail_queued(&mut receiver, &self.board, &self.index).await;
                        return;
                    }
                }
                Err(join) => {
                    // The store is lost with a panicking job. The job is failed,
                    // and the jobs still queued are failed too rather than left
                    // `queued` forever.
                    self.board.mark_finished(
                        &id,
                        Err(error::internal(format!(
                            "the blocking task of a write job failed: {join}"
                        ))),
                        &command.progress,
                    );
                    while let Some(command) = receiver.recv().await {
                        self.board.mark_running(&command.id, &command.progress);
                        self.board.mark_finished(
                            &command.id,
                            Err(error::internal(
                                "the writer of the index stopped after a failed job",
                            )),
                            &command.progress,
                        );
                    }
                    return;
                }
            }
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
            Err(CribleError::new(
                ErrorCode::UnknownIndex,
                format!("unknown index '{index}'"),
            )),
            &command.progress,
        );
    }
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
