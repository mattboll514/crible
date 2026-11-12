// SPDX-License-Identifier: Apache-2.0

//! Analytics of the instance: the closed event types, the aggregator, the
//! bounded queue and its single writer.
//!
//! References: ADR-0117 (closed fields of `SearchEvent` and `OpEvent`, filter
//! shapes, histograms, percentile) and ADR-0118 (bounded channel, `try_send`,
//! one writer, `analytics_dropped`, flushes, purge, daily rows). The tables live
//! in `analytics.sqlite`, which `crible-store` owns ([`crible_store::analytics`]);
//! this module knows only the event types and the writer that turns them into
//! aggregate rows.
//!
//! # Never a content
//!
//! [`SearchEvent`] and [`OpEvent`] carry **exactly** the fields of ADR-0117,
//! decision 5, and none of them can hold the text of a query, the value of a
//! filter, a hash or an identifier: the types have no place for them (INV-13,
//! ADR-0014). Adding such a field needs an ADR, not a review. The canonical form
//! of a filter is field names and operators only ([`filter_shape`]), and it
//! travels beside the event, never inside it, so the closed list above stays
//! closed.
//!
//! # A request never waits
//!
//! A request deposits its event with `try_send` on a bounded channel of
//! [`QUEUE_CAPACITY`] events. A full channel drops the **new** event and
//! increments the process-wide [`dropped`] counter (ADR-0118, decision 2); the
//! request answers as if the event had been recorded. No request holds a lock
//! and no request shares the aggregator: the writer owns it alone, which is
//! INV-17 by construction (ADR-0118, decision 1).
//!
//! # The writer
//!
//! One task receives the events, aggregates them in memory and flushes the
//! aggregate into the file every [`FLUSH_INTERVAL`] and when asked. A flush is
//! one transaction (ADR-0118, decision 3); its minute is read by SQLite, so the
//! code owns no clock. The purge of the retention runs at the open and then at
//! most once per [`PURGE_INTERVAL`] (decision 4). The daily rows are computed at
//! the first flush that has none for the current UTC day (decision 5).
//!
//! At the shutdown the writer drains the queue and flushes, bounded by
//! [`STOP_BOUND`]; a failure is logged and the instance keeps stopping.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crible_core::error::{CribleError, ErrorCode};
use crible_core::filter::Filter;
use crible_search::dense::Strategy;
use crible_search::pipeline::Mode;
use crible_store::analytics::{
    ANALYTICS_FILE_NAME, AnalyticsStore, DayIndex, LATENCY_BUCKETS, MinuteEvent, POOL_BUCKETS,
    ShapeCount, latency_bucket, pool_bucket,
};
use tokio::sync::{Notify, mpsc, oneshot};
use tracing::{info, warn};

use crate::auth::Client;
use crate::error::internal;
use crate::state::{AppState, Loaded};

/// Number of events the bounded channel holds (ADR-0118, decision 2).
///
/// A constant, not a configuration key: 4 096 events cover ten seconds of 400
/// requests per second, so a full channel signals a blocked writer, not a
/// normal peak.
pub const QUEUE_CAPACITY: usize = 4_096;

/// Interval between two periodic flushes (ADR-0118, decision 3).
pub const FLUSH_INTERVAL: Duration = Duration::from_secs(10);

/// Bound of the flush of the shutdown (ADR-0118, decision 3).
pub const STOP_BOUND: Duration = Duration::from_secs(2);

/// Interval between two purges of the retention (ADR-0118, decision 4).
pub const PURGE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Events the channel refused since the start of the process (ADR-0118,
/// decision 2).
///
/// It is process-wide and never persisted: it measures the state of a running
/// instance, not a history. A restart resets it, like the queue it describes.
static DROPPED: AtomicU64 = AtomicU64::new(0);

/// Number of events the bounded channel refused since the start of the process.
#[must_use]
pub fn dropped() -> u64 {
    DROPPED.load(Ordering::Relaxed)
}

/// Counts one refused event and warns when a flush observes the increase.
fn count_dropped() {
    let total = DROPPED.fetch_add(1, Ordering::Relaxed) + 1;
    // The line is emitted at the flush, never here: a request path must not pay
    // for formatting (ADR-0118, decision 2). `total` is read there.
    let _ = total;
}

/// Operation of the closed list of `docs/06-observabilite.md`
/// (ADR-0118, decision 6).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Operation {
    /// `POST /v1/indexes/{index}/search`.
    #[default]
    Search,
    /// `POST /v1/search`.
    SearchMulti,
    /// `POST /v1/indexes/{index}/facets` (ADR-0154, decision 9).
    Facets,
    /// `GET /v1/indexes/{index}/documents/{doc_id}`.
    GetDocument,
    /// `POST` or `PATCH /v1/indexes/{index}/documents`.
    OnlineBatch,
    /// `PATCH /v1/indexes/{index}/documents`.
    PatchDocuments,
    /// `POST /v1/indexes/{index}/batches`.
    AttachBatch,
    /// `POST /v1/indexes/{index}/compact`.
    Compact,
    /// `POST /v1/indexes/{index}/exports`.
    Export,
    /// `POST /v1/indexes/{index}/snapshots`.
    Snapshot,
    /// The administration routes of `docs/05-api.md`.
    IndexAdmin,
}

impl Operation {
    /// Wire name of the operation, the one the table stores.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::SearchMulti => "search_multi",
            Self::Facets => "facets",
            Self::GetDocument => "get_document",
            Self::OnlineBatch => "online_batch",
            Self::PatchDocuments => "patch_documents",
            Self::AttachBatch => "attach_batch",
            Self::Compact => "compact",
            Self::Export => "export",
            Self::Snapshot => "snapshot",
            Self::IndexAdmin => "index_admin",
        }
    }
}

/// Index of `total` among the nine stages of `docs/03-recherche.md`.
const STAGE_TOTAL: usize = 8;

/// Durations of the steps of one search, in the keys of `docs/03-recherche.md`.
///
/// A step the mode did not run is `None`: it produces no sample, so its
/// histogram sum is not `count`, while the sum of [`StageDurations::total`] is
/// (ADR-0117, decision 2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StageDurations {
    /// Step 1, validation.
    pub validate: Option<Duration>,
    /// Step 2, encoding.
    pub encode: Option<Duration>,
    /// Step 3, pool.
    pub filter: Option<Duration>,
    /// Step 4, dense branch.
    pub dense: Option<Duration>,
    /// Step 5, lexical branch.
    pub lexical: Option<Duration>,
    /// Step 6, fusion.
    pub fusion: Option<Duration>,
    /// Step 7, regrouping.
    pub group: Option<Duration>,
    /// Step 8, payload.
    pub fetch: Option<Duration>,
    /// Total of the request.
    pub total: Option<Duration>,
}

impl StageDurations {
    /// The nine durations, in the order of the columns of `minute_events`.
    #[must_use]
    pub fn stages(self) -> [Option<Duration>; 9] {
        [
            self.validate,
            self.encode,
            self.filter,
            self.dense,
            self.lexical,
            self.fusion,
            self.group,
            self.fetch,
            self.total,
        ]
    }
}

/// One search of the closed list of ADR-0117, decision 5.
///
/// The list is **closed**: no field can hold the text of a query, the value of
/// a filter, a hash, a result identifier or an internal id (INV-13, INV-01).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchEvent {
    /// Minute of the row; empty at deposit, filled by the flush (ADR-0118,
    /// decision 3).
    pub minute: String,
    /// Index the search targeted.
    pub index: String,
    /// `search` or `search_multi`.
    pub operation: Operation,
    /// `ok` or the code of the error the route rendered.
    pub status: String,
    /// Name of the API key (ADR-0100), never the key nor its digest.
    pub client: String,
    /// Branches the search ran, or `None` when the request failed before its
    /// mode was known (an unknown index, an unparsable body). The column keeps
    /// the empty string of ADR-0117, decision 1, for "sans objet".
    pub mode: Option<Mode>,
    /// Dense strategy that actually ran, when a dense branch ran (ADR-0074).
    pub dense_strategy: Option<Strategy>,
    /// Durations of the steps.
    pub stages: StageDurations,
    /// Size of the pool the filter kept.
    pub pool: u32,
    /// Whether the answer carried no result.
    pub zero_results: bool,
    /// Whether the query was truncated at the encoding.
    pub truncated: bool,
    /// Whether the dense branch alone brought a result (`hybrid` only).
    pub dense_only: bool,
    /// Whether the lexical branch alone brought a result (`hybrid` only).
    pub lexical_only: bool,
    /// Whether both branches brought a result (`hybrid` only).
    pub both: bool,
}

/// One operation of the closed list of ADR-0117, decision 5.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpEvent {
    /// Minute of the row; empty at deposit, filled by the flush.
    pub minute: String,
    /// Index the operation wrote, or empty for an operation of the instance.
    pub index: String,
    /// One of the eight non-search operations of `docs/06-observabilite.md`.
    pub operation: Operation,
    /// `ok` or the code of the error the route rendered.
    pub status: String,
    /// Name of the API key (ADR-0100).
    pub client: String,
    /// Duration of the whole operation.
    pub duration: Duration,
}

/// One message of the bounded channel.
///
/// The canonical form of a filter travels **beside** its event: ADR-0117,
/// decision 5 closes the fields of [`SearchEvent`], and decision 6 nevertheless
/// counts the forms. Keeping the shape out of the event honours both decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    /// A search, with its filter shape when the filter is not empty.
    Search {
        /// The event, at the closed list.
        event: SearchEvent,
        /// Canonical form of the filter (ADR-0117, decision 6).
        shape: Option<String>,
    },
    /// Any other operation of the closed list.
    Operation(OpEvent),
}

/// One command the writer serves between two events.
enum Command {
    /// Drain the channel, write it and answer.
    Flush(oneshot::Sender<Result<(), CribleError>>),
    /// Same as [`Command::Flush`], then stop the writer.
    Stop(oneshot::Sender<Result<(), CribleError>>),
}

/// Queue of the commands and the notification that wakes the writer.
#[derive(Default)]
struct Control {
    commands: Mutex<VecDeque<Command>>,
    notify: Notify,
}

impl Control {
    /// Queues `command` and wakes the writer.
    fn push(&self, command: Command) {
        match self.commands.lock() {
            Ok(mut commands) => commands.push_back(command),
            Err(_) => {
                warn!("the analytics command queue is poisoned; a flush is lost");
                return;
            }
        }
        self.notify.notify_one();
    }

    /// Takes the oldest pending command, if any.
    fn take(&self) -> Option<Command> {
        match self.commands.lock() {
            Ok(mut commands) => commands.pop_front(),
            Err(_) => None,
        }
    }
}

/// Handle of the analytics of an instance: the queue and its commands.
pub struct Analytics {
    sender: mpsc::Sender<Record>,
    control: Arc<Control>,
    /// `true` once the shutdown started the drain (ADR-0118, decision 3).
    closed: AtomicBool,
}

impl Analytics {
    /// Opens the analytics file of `data` and starts its writer.
    ///
    /// The purge of the retention runs at the open (ADR-0118, decision 4). The
    /// writer is spawned when a tokio runtime is present, which is the case of
    /// the server and of any test that drives the routes; an instance loaded
    /// outside a runtime — a plain unit test of the startup — records nothing and
    /// answers an empty flush, and that is stated here rather than hidden.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidRequest`] when the file is of another format
    /// (INV-15), and the SQLite failure otherwise.
    pub(crate) fn start(
        data: &Path,
        retention_days: usize,
        loaded: Arc<Loaded>,
    ) -> Result<Option<Arc<Self>>, CribleError> {
        let store = AnalyticsStore::open_or_create(&data.join(ANALYTICS_FILE_NAME))?;
        match store.purge(retention_days) {
            Ok(purged) => info!(
                purged,
                retention_days, "the analytics retention is purged at the open"
            ),
            Err(failure) => warn!(
                code = failure.code().as_str(),
                cause = failure.message(),
                "the analytics retention could not be purged at the open"
            ),
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            // No runtime: no writer. Nothing records, and `flush` answers `Ok`
            // with nothing to do.
            return Ok(None);
        };
        let (sender, receiver) = mpsc::channel(QUEUE_CAPACITY);
        let control = Arc::new(Control::default());
        handle.spawn(writer(
            receiver,
            Arc::clone(&control),
            Arc::new(store),
            retention_days,
            loaded,
        ));
        Ok(Some(Arc::new(Self {
            sender,
            control,
            closed: AtomicBool::new(false),
        })))
    }

    /// Deposits one record, or counts its loss (ADR-0118, decision 2).
    ///
    /// A record deposited once the shutdown closed the queue is counted too: the
    /// drain of [`Self::stop`] never loses an event in silence (ADR-0118,
    /// decision 3).
    fn record(&self, record: Record) {
        if self.closed.load(Ordering::SeqCst) {
            count_dropped();
            return;
        }
        match self.sender.try_send(record) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => count_dropped(),
            Err(mpsc::error::TrySendError::Closed(_)) => count_dropped(),
        }
    }

    /// Drains the queue into the file, through the writer.
    ///
    /// # Errors
    ///
    /// The error of the flush the writer reports, or
    /// [`ErrorCode::Internal`] when the writer stopped before answering.
    pub async fn flush(&self) -> Result<(), CribleError> {
        let (reply, answer) = oneshot::channel();
        self.control.push(command::flush(reply));
        answer
            .await
            .map_err(|_| internal("the analytics writer stopped before the flush answered"))?
    }

    /// Closes the queue, drains it, writes it and stops the writer, bounded by
    /// [`STOP_BOUND`].
    ///
    /// The queue is closed **before** the drain (ADR-0118, decision 3): an event
    /// a request deposits during the shutdown is counted in [`dropped`], never
    /// lost in silence.
    ///
    /// # Errors
    ///
    /// The error of the flush, or [`ErrorCode::Internal`] when the writer stopped
    /// before answering or when the bound was reached.
    pub async fn stop(&self) -> Result<(), CribleError> {
        self.closed.store(true, Ordering::SeqCst);
        let (reply, answer) = oneshot::channel();
        self.control.push(command::stop(reply));
        match tokio::time::timeout(STOP_BOUND, answer).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(internal(
                "the analytics writer stopped before the flush of the shutdown answered",
            )),
            Err(_) => Err(internal(
                "the analytics flush of the shutdown exceeded its bound",
            )),
        }
    }
}

/// Status the analytics store for an operation: `ok`, or the code of the
/// failure the route rendered (ADR-0117, decision 5).
///
/// A refusal of the API is an operation tried, not an absent operation
/// (ADR-0118, decision 6). The code is read from the response, never guessed
/// from its status: two codes share a status.
#[must_use]
pub(crate) fn status(answer: &axum::response::Response) -> String {
    if answer.status().is_success() {
        return "ok".to_owned();
    }
    crate::error::rendered_code(answer).map_or_else(
        || ErrorCode::Internal.as_str().to_owned(),
        |code| code.as_str().to_owned(),
    )
}

/// Records the [`OpEvent`] of one operation of the closed list (ADR-0118,
/// decision 6).
pub(crate) fn record_operation(
    state: &AppState,
    operation: Operation,
    index: &str,
    client: &Client,
    started: Instant,
    answer: &axum::response::Response,
) {
    let Some(analytics) = state.analytics() else {
        return;
    };
    analytics.record(Record::Operation(OpEvent {
        minute: String::new(),
        index: index.to_owned(),
        operation,
        status: status(answer),
        client: client.name.clone(),
        duration: started.elapsed(),
    }));
}

/// Records the [`SearchEvent`]s a search route built, with the name of the key
/// that reached it (ADR-0117, decision 5).
pub(crate) fn record(state: &AppState, records: Vec<Record>, client: &Client) {
    let Some(analytics) = state.analytics() else {
        return;
    };
    for mut record in records {
        match &mut record {
            Record::Search { event, .. } => event.client.clone_from(&client.name),
            Record::Operation(event) => event.client.clone_from(&client.name),
        }
        analytics.record(record);
    }
}

/// Constructors of the two commands, so the queue stays private.
mod command {
    use super::Command;
    use crible_core::error::CribleError;
    use tokio::sync::oneshot;

    pub(super) fn flush(reply: oneshot::Sender<Result<(), CribleError>>) -> Command {
        Command::Flush(reply)
    }

    pub(super) fn stop(reply: oneshot::Sender<Result<(), CribleError>>) -> Command {
        Command::Stop(reply)
    }
}

/// Key of a row of `minute_events`, without its minute (ADR-0117, decision 1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    index: String,
    operation: &'static str,
    status: String,
    client: String,
    mode: &'static str,
    dense_strategy: &'static str,
}

/// Counters of one key between two flushes.
#[derive(Debug, Clone)]
struct Row {
    count: u64,
    zero_results: u64,
    truncated: u64,
    dense_only: u64,
    lexical_only: u64,
    both: u64,
    latency: [[u32; LATENCY_BUCKETS]; 9],
    pool: [u32; POOL_BUCKETS],
}

impl Default for Row {
    fn default() -> Self {
        Self {
            count: 0,
            zero_results: 0,
            truncated: 0,
            dense_only: 0,
            lexical_only: 0,
            both: 0,
            latency: [[0; LATENCY_BUCKETS]; 9],
            pool: [0; POOL_BUCKETS],
        }
    }
}

/// In-memory aggregate of the writer between two flushes (ADR-0118, decision 1).
#[derive(Default)]
struct Aggregator {
    minutes: HashMap<Key, Row>,
    shapes: HashMap<(String, String), u64>,
}

impl Aggregator {
    /// Adds one record to the aggregate.
    fn push(&mut self, record: Record) {
        match record {
            Record::Search { event, shape } => self.push_search(event, shape),
            Record::Operation(event) => self.push_operation(event),
        }
    }

    /// Adds one search, with its filter shape when it has one.
    fn push_search(&mut self, event: SearchEvent, shape: Option<String>) {
        let key = Key {
            index: event.index.clone(),
            operation: event.operation.as_str(),
            status: event.status,
            client: event.client,
            mode: event.mode.map_or("", Mode::as_str),
            dense_strategy: event.dense_strategy.map_or("", Strategy::as_str),
        };
        let row = self.minutes.entry(key).or_default();
        row.count = row.count.saturating_add(1);
        row.zero_results = row
            .zero_results
            .saturating_add(u64::from(event.zero_results));
        row.truncated = row.truncated.saturating_add(u64::from(event.truncated));
        row.dense_only = row.dense_only.saturating_add(u64::from(event.dense_only));
        row.lexical_only = row
            .lexical_only
            .saturating_add(u64::from(event.lexical_only));
        row.both = row.both.saturating_add(u64::from(event.both));
        for (stage, duration) in event.stages.stages().into_iter().enumerate() {
            if let Some(duration) = duration {
                let bucket = latency_bucket(duration);
                row.latency[stage][bucket] = row.latency[stage][bucket].saturating_add(1);
            }
        }
        let bucket = pool_bucket(event.pool);
        row.pool[bucket] = row.pool[bucket].saturating_add(1);
        if let Some(shape) = shape
            && !shape.is_empty()
        {
            let count = self.shapes.entry((event.index, shape)).or_insert(0_u64);
            *count = count.saturating_add(1);
        }
    }

    /// Adds one operation event: one `total` sample, no mode, no strategy.
    fn push_operation(&mut self, event: OpEvent) {
        let key = Key {
            index: event.index,
            operation: event.operation.as_str(),
            status: event.status,
            client: event.client,
            mode: "",
            dense_strategy: "",
        };
        let row = self.minutes.entry(key).or_default();
        row.count = row.count.saturating_add(1);
        let bucket = latency_bucket(event.duration);
        row.latency[STAGE_TOTAL][bucket] = row.latency[STAGE_TOTAL][bucket].saturating_add(1);
    }

    /// Number of events the aggregate holds.
    fn len(&self) -> u64 {
        self.minutes
            .values()
            .map(|row| row.count)
            .fold(0_u64, u64::saturating_add)
    }

    /// Turns the aggregate into the rows of one flush.
    fn into_rows(self) -> (Vec<MinuteEvent>, Vec<ShapeCount>) {
        let minutes = self
            .minutes
            .into_iter()
            .map(|(key, row)| {
                let latency = row.latency;
                MinuteEvent {
                    minute: String::new(),
                    index: key.index,
                    operation: key.operation.to_owned(),
                    status: key.status,
                    client: key.client,
                    mode: key.mode.to_owned(),
                    dense_strategy: key.dense_strategy.to_owned(),
                    count: row.count,
                    zero_results: row.zero_results,
                    truncated: row.truncated,
                    dense_only: row.dense_only,
                    lexical_only: row.lexical_only,
                    both: row.both,
                    latency_validate: latency[0].to_vec(),
                    latency_encode: latency[1].to_vec(),
                    latency_filter: latency[2].to_vec(),
                    latency_dense: latency[3].to_vec(),
                    latency_lexical: latency[4].to_vec(),
                    latency_fusion: latency[5].to_vec(),
                    latency_group: latency[6].to_vec(),
                    latency_fetch: latency[7].to_vec(),
                    latency_total: latency[8].to_vec(),
                    pool: row.pool.to_vec(),
                }
            })
            .collect();
        let shapes = self
            .shapes
            .into_iter()
            .map(|((index, shape), count)| ShapeCount {
                index,
                shape,
                count,
            })
            .collect();
        (minutes, shapes)
    }
}

/// The single writer of the analytics (ADR-0118, decision 1).
///
/// It is held at the head of its loop by the `analytics-flush` test barrier
/// before it receives anything, which is how INV-17 is proved (decision 7). The
/// flush itself — one SQLite transaction and the daily rows, which walk the
/// directory of an index — runs on the blocking pool, never on an async worker
/// (`AGENTS.md`, rule 7).
async fn writer(
    mut receiver: mpsc::Receiver<Record>,
    control: Arc<Control>,
    store: Arc<AnalyticsStore>,
    retention_days: usize,
    loaded: Arc<Loaded>,
) {
    let mut aggregator = Aggregator::default();
    let mut last_purge = Instant::now();
    let mut last_dropped = dropped();
    let mut next_flush = Instant::now() + FLUSH_INTERVAL;
    loop {
        // Head of the loop, before anything is received: a test fills the
        // channel here (ADR-0118, decision 7).
        #[cfg(feature = "failpoints")]
        crate::barriers::wait_analytics_flush().await;
        let command = control.take();
        let due = Instant::now() >= next_flush;
        let stopping = matches!(command, Some(Command::Stop(_)));
        if command.is_some() || due {
            // `try_recv` never blocks, so the drain stays on the async task; the
            // aggregate it feeds goes to the blocking one.
            while let Ok(record) = receiver.try_recv() {
                aggregator.push(record);
            }
            let flush = Flush {
                store: Arc::clone(&store),
                aggregator: std::mem::take(&mut aggregator),
                loaded: Arc::clone(&loaded),
                retention_days,
                last_purge,
                last_dropped,
            };
            let flushed = match tokio::task::spawn_blocking(move || flush.run()).await {
                Ok(flushed) => flushed,
                Err(failure) => {
                    warn!(
                        cause = %failure,
                        "the analytics flush task failed; the events of this flush are lost"
                    );
                    Flushed::lost(last_purge, last_dropped)
                }
            };
            aggregator = flushed.aggregator;
            last_purge = flushed.last_purge;
            last_dropped = flushed.last_dropped;
            next_flush = Instant::now() + FLUSH_INTERVAL;
            if let Some(Command::Flush(reply) | Command::Stop(reply)) = command {
                let _ = reply.send(flushed.result);
            }
        }
        if stopping {
            break;
        }
        tokio::select! {
            received = receiver.recv() => {
                match received {
                    Some(record) => aggregator.push(record),
                    // Every sender is gone: the instance is being dropped.
                    None => break,
                }
            }
            () = control.notify.notified() => {}
            () = tokio::time::sleep_until(next_flush.into()) => {}
        }
    }
}

/// What the blocking task of one flush owns, and gives back to the writer.
struct Flush {
    /// The file, shared with the writer so the task can own a clone.
    store: Arc<AnalyticsStore>,
    /// Aggregate of the events received since the previous flush.
    aggregator: Aggregator,
    /// Indexes of the instance, for the daily rows (ADR-0118, decision 5).
    loaded: Arc<Loaded>,
    /// Days of history kept (ADR-0118, decision 4).
    retention_days: usize,
    /// Instant of the previous purge.
    last_purge: Instant,
    /// Value of [`dropped`] already reported.
    last_dropped: u64,
}

/// What a flush gives back: the counters the writer carries to the next one.
struct Flushed {
    /// Result of the write, answered to the command that asked for it.
    result: Result<(), CribleError>,
    /// Empty aggregate, ready for the events of the next interval.
    aggregator: Aggregator,
    /// Instant of the previous purge, moved when this flush purged.
    last_purge: Instant,
    /// Value of [`dropped`] reported by this flush.
    last_dropped: u64,
}

impl Flushed {
    /// The result of a flush whose blocking task panicked.
    fn lost(last_purge: Instant, last_dropped: u64) -> Self {
        Self {
            result: Err(CribleError::new(
                ErrorCode::Internal,
                "the analytics flush task failed",
            )),
            aggregator: Aggregator::default(),
            last_purge,
            last_dropped,
        }
    }
}

impl Flush {
    /// Writes the aggregate and the daily rows of the day, then purges if due.
    fn run(mut self) -> Flushed {
        let events = self.aggregator.len();
        let daily = daily_rows(&self.store, &self.loaded);
        let (minutes, shapes) = std::mem::take(&mut self.aggregator).into_rows();
        let result = self.store.write(&minutes, &shapes, &daily);
        match &result {
            Err(failure) => warn!(
                code = failure.code().as_str(),
                cause = failure.message(),
                events,
                "the analytics flush failed; the events of this flush are lost"
            ),
            Ok(()) => info!(
                events,
                rows = minutes.len(),
                shapes = shapes.len(),
                "the analytics are flushed"
            ),
        }
        let total = dropped();
        if total > self.last_dropped {
            warn!(
                lost = total - self.last_dropped,
                total, "analytics_dropped grew: the analytics queue was full (INV-17)"
            );
            self.last_dropped = total;
        }
        if self.last_purge.elapsed() >= PURGE_INTERVAL {
            match self.store.purge(self.retention_days) {
                Ok(purged) => info!(purged, "the analytics retention is purged"),
                Err(failure) => warn!(
                    code = failure.code().as_str(),
                    cause = failure.message(),
                    "the analytics retention could not be purged"
                ),
            }
            self.last_purge = Instant::now();
        }
        Flushed {
            result,
            aggregator: self.aggregator,
            last_purge: self.last_purge,
            last_dropped: self.last_dropped,
        }
    }
}

/// Daily rows the flush owes: one per served index whose row of the day is
/// missing (ADR-0118, decision 5).
///
/// A failure to read the state of an index or its disk size skips that index
/// with a `warn`: a daily row is an observation, and it must never turn a flush
/// into a failure.
fn daily_rows(store: &AnalyticsStore, loaded: &Loaded) -> Vec<DayIndex> {
    let indexes = match loaded.indexes() {
        Ok(indexes) => indexes,
        Err(failure) => {
            warn!(
                code = failure.code().as_str(),
                cause = failure.message(),
                "the daily analytics rows could not list the indexes"
            );
            return Vec::new();
        }
    };
    let names: Vec<String> = indexes.iter().map(|index| index.name.clone()).collect();
    let missing = match store.missing_days(&names) {
        Ok(missing) => missing,
        Err(failure) => {
            warn!(
                code = failure.code().as_str(),
                cause = failure.message(),
                "the daily analytics rows could not be read"
            );
            return Vec::new();
        }
    };
    let mut rows = Vec::with_capacity(missing.len());
    for name in missing {
        let Some(handle) = indexes.iter().find(|index| index.name == name) else {
            continue;
        };
        let published = match handle.published() {
            Ok(published) => published,
            Err(failure) => {
                warn!(
                    index = %name,
                    code = failure.code().as_str(),
                    cause = failure.message(),
                    "a daily analytics row is skipped: the index is unreadable"
                );
                continue;
            }
        };
        let summary = published.summary;
        let directory = loaded.index_directory(&name);
        let disk_bytes = match crate::indexes::directory_size(&directory) {
            Ok(bytes) => bytes,
            Err(failure) => {
                warn!(
                    index = %name,
                    code = failure.code().as_str(),
                    cause = failure.message(),
                    "a daily analytics row is skipped: the directory size is unreadable"
                );
                continue;
            }
        };
        rows.push(DayIndex {
            day: String::new(),
            index: name,
            documents: summary.documents,
            live_passages: summary.live_passages,
            dead_passages: summary.dead_passages,
            disk_bytes,
            generation: summary.generation,
            batches: summary.batches,
        });
    }
    rows
}

/// Canonical form of a filter: the sorted and deduplicated `field:operator`
/// pairs, joined by a comma, and `None` for an empty filter (ADR-0117,
/// decision 6).
///
/// `$and`, `$or` and `$not` are traversed and do not appear: the form describes
/// which fields and which operators are exercised, never the boolean structure,
/// and never a value.
#[must_use]
pub fn filter_shape(filter: &Filter) -> Option<String> {
    let mut pairs = Vec::new();
    collect_pairs(filter, &mut pairs);
    if pairs.is_empty() {
        return None;
    }
    pairs.sort_unstable();
    pairs.dedup();
    Some(pairs.join(","))
}

/// Collects the `field:operator` pair of every condition of `filter`.
fn collect_pairs(filter: &Filter, pairs: &mut Vec<String>) {
    match filter {
        Filter::Condition(condition) => {
            pairs.push(format!(
                "{}:{}",
                condition.field,
                condition.operator.as_str()
            ));
        }
        Filter::And(children) | Filter::Or(children) => {
            for child in children {
                collect_pairs(child, pairs);
            }
        }
        Filter::Not(child) => collect_pairs(child, pairs),
    }
}

#[cfg(test)]
mod tests {
    use crible_core::filter::Filter;

    use super::{Operation, QUEUE_CAPACITY, STAGE_TOTAL, filter_shape};

    /// Parses a filter of the grammar of `docs/05-api.md`.
    fn parse(raw: &str) -> Filter {
        Filter::from_json_str(raw).expect("the fixture filter is of the grammar")
    }

    #[test]
    fn a_scalar_is_an_equality_and_an_object_keeps_its_operators() {
        // ADR-0117, decision 6, rule 1: `{"fonds": "LEGI"}` gives `fonds:eq`,
        // `{"annee": {"gte": 2000, "lt": 2010}}` gives both pairs.
        assert_eq!(
            filter_shape(&parse(r#"{"fonds": "LEGI"}"#)).as_deref(),
            Some("fonds:eq")
        );
        assert_eq!(
            filter_shape(&parse(r#"{"annee": {"gte": 2000, "lt": 2010}}"#)).as_deref(),
            Some("annee:gte,annee:lt")
        );
    }

    #[test]
    fn the_connectors_are_traversed_and_do_not_appear() {
        // ADR-0117, decision 6, rule 2: `$and`, `$or` and `$not` do not appear,
        // and `$not` does not change the operator of its leaves.
        assert_eq!(
            filter_shape(&parse(r#"{"$and": [{"f": "a"}, {"b": "b"}]}"#)).as_deref(),
            Some("b:eq,f:eq")
        );
        assert_eq!(
            filter_shape(&parse(r#"{"$or": [{"b": "b"}, {"f": "a"}]}"#)).as_deref(),
            Some("b:eq,f:eq")
        );
        assert_eq!(
            filter_shape(&parse(r#"{"$not": {"f": {"eq": "a"}}}"#)).as_deref(),
            Some("f:eq")
        );
    }

    #[test]
    fn duplicates_are_removed_and_the_order_is_lexicographic() {
        // ADR-0117, decision 6, rules 3 and 4: sorted on the octets,
        // deduplicated, joined by a comma without a space.
        assert_eq!(
            filter_shape(&parse(r#"{"$and": [{"z": 1}, {"a": 2}, {"z": 3}]}"#)).as_deref(),
            Some("a:eq,z:eq")
        );
    }

    #[test]
    fn an_empty_filter_has_no_shape() {
        // ADR-0117, decision 6, rule 5: no pair, so no row in `filter_shapes`.
        assert_eq!(filter_shape(&parse("{}")), None);
        assert_eq!(filter_shape(&parse(r#"{"$and": []}"#)), None);
    }

    #[test]
    fn the_closed_operations_are_the_eleven_of_the_documentation() {
        // ADR-0118, decision 6: a closed list, and `index_admin` covers the
        // administration routes of the indexes; ADR-0154, decision 9, adds
        // `facets`.
        let names = [
            Operation::Search,
            Operation::SearchMulti,
            Operation::Facets,
            Operation::GetDocument,
            Operation::OnlineBatch,
            Operation::PatchDocuments,
            Operation::AttachBatch,
            Operation::Compact,
            Operation::Export,
            Operation::Snapshot,
            Operation::IndexAdmin,
        ]
        .map(Operation::as_str);
        assert_eq!(names.len(), 11);
        assert_eq!(names[10], "index_admin");
        assert_eq!(names[2], "facets");
        assert_eq!(names[0], "search");
    }

    #[test]
    fn the_queue_capacity_is_the_constant_of_the_decision() {
        // ADR-0118, decision 2: 4 096, a constant and not a key.
        assert_eq!(QUEUE_CAPACITY, 4_096);
    }

    #[test]
    fn the_total_stage_is_the_last_of_the_nine_columns() {
        // `docs/03-recherche.md`: `total` closes the table of the stages, and
        // the column list of ADR-0117 repeats it as `latency_total`.
        assert_eq!(STAGE_TOTAL, 8);
    }
}
