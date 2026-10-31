// SPDX-License-Identifier: Apache-2.0

//! The debug journal of the instance: activation, bounded queue and its single
//! writer.
//!
//! References: ADR-0121 (file, schema, payload, protections) and ADR-0122
//! (activation, capture, channel, flush, purge, route). The file lives in
//! `debug.sqlite`, which `crible-store` owns ([`crible_store::debug_journal`]);
//! this module knows the shape of the payload, the queue and the writer.
//!
//! # Two ways to be recorded
//!
//! A request that asks `debug: true` is always recorded when the journal is on,
//! whatever `sample_rate`; the rate only samples the other requests
//! (ADR-0122, decision 1). The draw is [`should_sample`], a pure function of the
//! rate and one `u64` from the system source, so a test proves it without
//! randomness; an unavailable source is treated as "not sampled" and never
//! fails a request.
//!
//! # A request never waits
//!
//! A request deposits its row with `try_send` on a bounded channel of
//! [`QUEUE_CAPACITY`] rows. A full channel drops the **new** row and increments
//! the process-wide [`dropped`] counter (ADR-0122, decision 6); the request
//! answers as if the row had been recorded. No request holds a lock and no
//! request shares the writer: INV-17 is true by construction.
//!
//! # The writer
//!
//! One task receives the rows and writes them in one transaction every
//! [`FLUSH_INTERVAL`] and when asked. The timestamp is read by SQLite, so the
//! code owns no clock (ADR-0121, decision 3). The purge of the retention runs at
//! the open and then at the first flush whose previous purge is older than
//! [`PURGE_INTERVAL`] (ADR-0122, decision 8). At the shutdown the writer drains
//! the queue and flushes, bounded by [`STOP_BOUND`]; a failure is logged and the
//! instance keeps stopping.
//!
//! # No content in a log
//!
//! The journal **holds** the text and the filter by design; no line of this
//! module ever logs one (ADR-0122, decision 10). The only lines are `warn`s of
//! counting: dropped rows, a failed flush, a failed purge.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crible_core::config::FusionConfig;
use crible_core::error::{CribleError, ErrorCode};
use crible_search::debug::{CapturedEntry, DebugCapture};
use crible_search::pipeline::Mode;
use crible_store::debug_journal::{
    DEBUG_JOURNAL_FILE_NAME, DebugJournalStore, DebugQueryRow, JournalFilter, read_page,
};
use serde_json::{Map, Value, json};
use tokio::sync::{Notify, mpsc, oneshot};
use tracing::{debug, warn};

use crate::analytics::StageDurations;
use crate::auth::Client;
use crate::config::DebugJournalConfig;
use crate::debug::fusion_name;
use crate::error::internal;
use crate::state::AppState;

/// Number of rows the bounded channel holds (ADR-0122, decision 6).
///
/// A constant, not a configuration key: 4 096 requests cover ten seconds of 400
/// requests per second, so a full channel signals a blocked writer, not a normal
/// peak.
pub const QUEUE_CAPACITY: usize = 4_096;

/// Interval between two periodic flushes (ADR-0122, decision 7).
pub const FLUSH_INTERVAL: Duration = Duration::from_secs(10);

/// Bound of the flush of the shutdown (ADR-0122, decision 7).
pub const STOP_BOUND: Duration = Duration::from_secs(2);

/// Interval between two purges of the retention (ADR-0122, decision 8).
pub const PURGE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Default number of rows of a page of `GET /v1/debug/queries` (ADR-0122,
/// decision 9).
pub const DEFAULT_PAGE: usize = 100;

/// Largest page the route renders (ADR-0122, decision 9).
pub const MAX_PAGE: usize = 1_000;

/// Rows the channel refused since the start of the process (ADR-0122,
/// decision 6).
///
/// It is process-wide and never persisted: it measures the state of a running
/// instance, not a history. A restart resets it, like the queue it describes.
static DROPPED: AtomicU64 = AtomicU64::new(0);

/// Number of rows the bounded channel refused since the start of the process.
#[must_use]
pub fn dropped() -> u64 {
    DROPPED.load(Ordering::Relaxed)
}

/// Counts one refused row.
fn count_dropped() {
    let _ = DROPPED.fetch_add(1, Ordering::Relaxed);
}

/// Whether a request is recorded by the draw of `rate`, for an integer in
/// `[0, u64::MAX]` (ADR-0122, decision 1).
///
/// It is pure, so a test proves the two edges without randomness: `0.0` samples
/// nothing, `1.0` samples everything, and an intermediate rate keeps a fraction
/// of the integers of the domain.
#[must_use]
pub fn should_sample(rate: f64, draw: u64) -> bool {
    if rate <= 0.0 {
        return false;
    }
    if rate >= 1.0 {
        return true;
    }
    // `u64::MAX as f64 + 1.0` is 2^64, the size of the domain of the draw.
    rate * (u64::MAX as f64 + 1.0) > draw as f64
}

/// Draws the integer of the sampling, or `false` when the source fails.
///
/// A failure of the source never fails the request: the search is treated as
/// not sampled and goes on (ADR-0122, decision 1).
fn drawn(rate: f64) -> bool {
    if rate <= 0.0 {
        return false;
    }
    if rate >= 1.0 {
        return true;
    }
    let mut bytes = [0u8; 8];
    match getrandom::fill(&mut bytes) {
        Ok(()) => should_sample(rate, u64::from_ne_bytes(bytes)),
        Err(_) => false,
    }
}

/// Whether the search route must record this request (ADR-0122, decision 1).
///
/// The draw happens at the entry of the route, before the concurrency token: it
/// costs no token and never extends `took_ms`. The configuration is read from
/// the loaded state, so a disabled journal draws nothing at all.
#[must_use]
pub(crate) fn sampled(state: &AppState) -> bool {
    match state.loaded().map(|loaded| loaded.debug_journal_config()) {
        Some(config) if config.enabled => drawn(config.sample_rate),
        _ => false,
    }
}

/// One row a route hands to the queue, with the columns of ADR-0121,
/// decision 3.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Deposit {
    /// Request identifier of ADR-0098; filled by the route.
    pub(crate) request_id: String,
    /// Name of the index, `None` for every multi-index request.
    pub(crate) index: Option<String>,
    /// Name of the API key (ADR-0100); filled by the route.
    pub(crate) client: String,
    /// Mode of the request, empty when it could not be read.
    pub(crate) mode: String,
    /// `ok` or the code of the error the route rendered.
    pub(crate) status: String,
    /// Total duration, `None` when `fetch` did not finish.
    pub(crate) total_ms: Option<f64>,
    /// The payload of ADR-0121, decision 4.
    pub(crate) payload: Value,
}

/// One command the writer serves between two rows.
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
                warn!("the debug journal command queue is poisoned; a flush is lost");
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

/// Handle of the journal of an instance: the queue and its commands.
pub struct DebugJournal {
    sender: mpsc::Sender<Deposit>,
    control: Arc<Control>,
    /// `true` once the shutdown started the drain (ADR-0122, decision 7).
    closed: AtomicBool,
}

impl DebugJournal {
    /// Opens the journal file of `data` and starts its writer.
    ///
    /// A disabled journal writes no file and opens none: `None` is returned
    /// before any path is touched (ADR-0121, decision 1). When it is enabled, the
    /// purge of the retention runs at the open (ADR-0122, decision 8). The writer
    /// is spawned when a tokio runtime is present, which is the case of the
    /// server and of any test that drives the routes; an instance loaded outside
    /// a runtime records nothing and answers an empty flush.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidRequest`] when the file is of another format
    /// (INV-15), and the SQLite failure otherwise.
    pub(crate) fn start(
        data: &Path,
        config: DebugJournalConfig,
    ) -> Result<Option<Arc<Self>>, CribleError> {
        if !config.enabled {
            return Ok(None);
        }
        let store = DebugJournalStore::open_or_create(&data.join(DEBUG_JOURNAL_FILE_NAME))?;
        match store.purge(config.ttl_days) {
            Ok(purged) => debug!(
                purged,
                ttl_days = config.ttl_days,
                "the debug journal retention is purged at the open"
            ),
            Err(failure) => warn!(
                code = failure.code().as_str(),
                cause = failure.message(),
                "the debug journal retention could not be purged at the open"
            ),
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            // No runtime: no writer. Nothing records, and a flush answers `Ok`
            // with nothing to do.
            return Ok(None);
        };
        let (sender, receiver) = mpsc::channel(QUEUE_CAPACITY);
        let control = Arc::new(Control::default());
        handle.spawn(writer(
            receiver,
            Arc::clone(&control),
            Arc::new(store),
            config.ttl_days,
        ));
        Ok(Some(Arc::new(Self {
            sender,
            control,
            closed: AtomicBool::new(false),
        })))
    }

    /// Deposits one row, or counts its loss (ADR-0122, decision 6).
    ///
    /// A row deposited once the shutdown closed the queue is counted too: the
    /// drain of [`Self::stop`] never loses a row in silence.
    fn record(&self, deposit: Deposit) {
        if self.closed.load(Ordering::SeqCst) {
            count_dropped();
            return;
        }
        match self.sender.try_send(deposit) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_) | mpsc::error::TrySendError::Closed(_)) => {
                count_dropped();
            }
        }
    }

    /// Drains the queue into `debug.sqlite`, through the writer.
    ///
    /// # Errors
    ///
    /// The error of the flush the writer reports, or [`ErrorCode::Internal`]
    /// when the writer stopped before answering.
    pub async fn flush(&self) -> Result<(), CribleError> {
        let (reply, answer) = oneshot::channel();
        self.control.push(command::flush(reply));
        answer
            .await
            .map_err(|_| internal("the debug journal writer stopped before the flush answered"))?
    }

    /// Closes the queue, drains it, writes it and stops the writer, bounded by
    /// [`STOP_BOUND`].
    ///
    /// The queue is closed **before** the drain (ADR-0122, decision 7): a row a
    /// request deposits during the shutdown is counted in [`dropped`], never
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
                "the debug journal writer stopped before the flush of the shutdown answered",
            )),
            Err(_) => Err(internal(
                "the debug journal flush of the shutdown exceeded its bound",
            )),
        }
    }
}

/// Deposits the row of one request, with the name of the key that reached it
/// and the identifier of the request (ADR-0098).
pub(crate) fn deposit(state: &AppState, mut deposit: Deposit, client: &Client, request_id: &str) {
    let Some(journal) = state.debug_journal() else {
        return;
    };
    deposit.client.clone_from(&client.name);
    deposit.request_id = request_id.to_owned();
    journal.record(deposit);
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

/// The writer of the journal: receives the rows and flushes them.
///
/// The barrier of the `failpoints` build stands at the head of the loop,
/// **before** anything is received, which is how INV-17 is proved
/// (ADR-0122, decision 11). The write itself runs on the blocking pool, never on
/// an async worker.
async fn writer(
    mut receiver: mpsc::Receiver<Deposit>,
    control: Arc<Control>,
    store: Arc<DebugJournalStore>,
    ttl_days: usize,
) {
    let mut pending: Vec<Deposit> = Vec::new();
    let mut last_purge = Instant::now();
    let mut last_dropped = dropped();
    let mut next_flush = Instant::now() + FLUSH_INTERVAL;
    loop {
        // Head of the loop, before anything is received: a test fills the
        // channel here (ADR-0122, decision 11).
        #[cfg(feature = "failpoints")]
        crate::barriers::wait_debug_journal_flush().await;
        let command = control.take();
        let due = Instant::now() >= next_flush;
        let stopping = matches!(command, Some(Command::Stop(_)));
        if command.is_some() || due {
            // `try_recv` never blocks, so the drain stays on the async task; the
            // write it feeds goes to the blocking one.
            while let Ok(deposit) = receiver.try_recv() {
                pending.push(deposit);
            }
            let flush = Flush {
                store: Arc::clone(&store),
                rows: std::mem::take(&mut pending),
                ttl_days,
                last_purge,
                last_dropped,
            };
            let flushed = match tokio::task::spawn_blocking(move || flush.run()).await {
                Ok(flushed) => flushed,
                Err(failure) => {
                    warn!(
                        cause = %failure,
                        "the debug journal flush task failed; the rows of this flush are lost"
                    );
                    Flushed::lost(last_purge, last_dropped)
                }
            };
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
                    Some(deposit) => pending.push(deposit),
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
    store: Arc<DebugJournalStore>,
    /// Rows received since the previous flush.
    rows: Vec<Deposit>,
    /// Days of history kept (ADR-0122, decision 8).
    ttl_days: usize,
    /// Instant of the previous purge.
    last_purge: Instant,
    /// Value of [`dropped`] already reported.
    last_dropped: u64,
}

/// What a flush gives back: the counters the writer carries to the next one.
struct Flushed {
    /// Result of the write, answered to the command that asked for it.
    result: Result<(), CribleError>,
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
                "the debug journal flush task failed",
            )),
            last_purge,
            last_dropped,
        }
    }
}

impl Flush {
    /// Writes the rows, then purges if due.
    fn run(mut self) -> Flushed {
        let events = self.rows.len();
        let rows = self.rows.drain(..).map(row_of).collect::<Vec<_>>();
        let result = self.store.write(&rows);
        match &result {
            Err(failure) => warn!(
                code = failure.code().as_str(),
                cause = failure.message(),
                rows = events,
                "the debug journal flush failed; the rows of this flush are lost"
            ),
            Ok(()) => debug!(rows = events, "the debug journal is flushed"),
        }
        let total = dropped();
        if total > self.last_dropped {
            warn!(
                lost = total - self.last_dropped,
                total, "debug_journal_dropped grew: the debug journal queue was full (INV-17)"
            );
            self.last_dropped = total;
        }
        if self.last_purge.elapsed() >= PURGE_INTERVAL {
            match self.store.purge(self.ttl_days) {
                Ok(purged) => debug!(purged, "the debug journal retention is purged"),
                Err(failure) => warn!(
                    code = failure.code().as_str(),
                    cause = failure.message(),
                    "the debug journal retention could not be purged"
                ),
            }
            self.last_purge = Instant::now();
        }
        Flushed {
            result,
            last_purge: self.last_purge,
            last_dropped: self.last_dropped,
        }
    }
}

/// Turns one deposit into the row of ADR-0121, decision 3.
///
/// The payload is serialised here, on the blocking thread, so a request never
/// pays for it (ADR-0121, decision 5). A payload that cannot be serialised is
/// written as an empty object rather than failing the whole flush: a `Value`
/// always serialises, and the fallback keeps the row's columns readable.
fn row_of(deposit: Deposit) -> DebugQueryRow {
    let payload = serde_json::to_string(&deposit.payload).unwrap_or_else(|_| "{}".to_owned());
    DebugQueryRow {
        created_at: String::new(),
        request_id: deposit.request_id,
        index: deposit.index,
        client: deposit.client,
        mode: deposit.mode,
        status: deposit.status,
        total_ms: deposit.total_ms,
        payload,
    }
}

/// The values of the `request` member of a payload (ADR-0121, decision 4).
pub(crate) struct RequestView<'a> {
    /// Body of the request, as received.
    pub(crate) body: &'a Value,
    /// Mode the request resolved to.
    pub(crate) mode: Mode,
    /// Number of results asked.
    pub(crate) k: usize,
    /// Whether the request forces the exact dense calculation.
    pub(crate) exact: bool,
    /// Fusion of the branches, resolved for the index.
    pub(crate) fusion: FusionConfig,
    /// Whether the request groups by document.
    pub(crate) by_document: bool,
    /// Passages kept per document under `group_by`.
    pub(crate) passages_per_document: usize,
    /// Whether identical passages are merged.
    pub(crate) dedup_content: bool,
    /// Whether the filter is a member of the request; false in multi-index,
    /// where each target carries its own (ADR-0110).
    pub(crate) with_filter: bool,
    /// Dimension of the provided vector, when the body carries one.
    pub(crate) dimension: Option<usize>,
}

/// Builds the `request` member of the payload from the body as received.
///
/// `filter` and `query` travel **as received**, never canonicalised
/// (ADR-0121, decision 4); the vector is reduced to its presence and its
/// dimension, never copied.
#[must_use]
pub(crate) fn request_payload(view: &RequestView<'_>) -> Value {
    let mut rendered = Map::new();
    if let Some(query) = view.body.get("query").and_then(Value::as_str) {
        rendered.insert("query".to_owned(), json!(query));
    }
    if view.with_filter
        && let Some(filter) = view.body.get("filter")
        && !filter.is_null()
    {
        rendered.insert("filter".to_owned(), filter.clone());
    }
    rendered.insert("mode".to_owned(), json!(view.mode.as_str()));
    rendered.insert("k".to_owned(), json!(view.k));
    rendered.insert("exact".to_owned(), json!(view.exact));
    rendered.insert("fusion".to_owned(), json!(fusion_name(view.fusion)));
    if view.by_document {
        rendered.insert("group_by".to_owned(), json!("document"));
        rendered.insert(
            "passages_per_document".to_owned(),
            json!(view.passages_per_document),
        );
    }
    if view.dedup_content {
        rendered.insert("dedup".to_owned(), json!("content"));
    }
    if let Some(specification) = view.body.get("return") {
        rendered.insert("return".to_owned(), specification.clone());
    }
    let mut vector = Map::new();
    vector.insert("present".to_owned(), json!(view.dimension.is_some()));
    if let Some(dimension) = view.dimension {
        vector.insert("dimension".to_owned(), json!(dimension));
    }
    rendered.insert("vector".to_owned(), Value::Object(vector));
    Value::Object(rendered)
}

/// Renders the capture of the branch tops (ADR-0119, decision 8).
///
/// A branch that brought no entry is omitted: a branch that did not run has no
/// entry, and writing an empty array would say it ran without a hit
/// (ADR-0119, decision 3).
#[must_use]
pub(crate) fn capture_payload(capture: &DebugCapture) -> Value {
    let mut rendered = Map::new();
    if !capture.dense().is_empty() {
        rendered.insert("dense".to_owned(), entries_payload(capture.dense()));
    }
    if !capture.lexical().is_empty() {
        rendered.insert("lexical".to_owned(), entries_payload(capture.lexical()));
    }
    Value::Object(rendered)
}

/// Renders one branch of a capture: `(key, rank, score)` per entry.
fn entries_payload(entries: &[CapturedEntry]) -> Value {
    Value::Array(
        entries
            .iter()
            .map(|entry| {
                json!({
                    "key": entry.key,
                    "rank": entry.rank,
                    "score": f64::from(entry.score),
                })
            })
            .collect(),
    )
}

/// Renders `timings_ms` of the payload from the durations of the route.
#[must_use]
pub(crate) fn timings_payload(stages: &StageDurations) -> Value {
    crate::debug::timings(stages)
}

/// Renders `timings_ms` from the steps that were really measured.
///
/// A `503` or a `504` writes the row of a request whose work did not start, or
/// did not end. A step that was not measured is omitted, never rendered `0.0`
/// (ADR-0122, decision 5, which prevails over ADR-0121, decision 4).
pub(crate) fn measured_timings_payload(stages: &StageDurations) -> Value {
    let mut rendered = Map::new();
    for (key, duration) in crate::debug::TIMING_KEYS.iter().zip(stages.stages()) {
        if let Some(duration) = duration {
            rendered.insert(
                (*key).to_owned(),
                json!(crate::debug::round_ms(duration.as_secs_f64() * 1_000.0)),
            );
        }
    }
    Value::Object(rendered)
}

/// Reads one page of the journal file (ADR-0122, decision 9).
///
/// It runs on the blocking pool: the file is read by SQLite, never on an async
/// worker. The path is the one of `data`, so the route does not need the writer.
///
/// # Errors
///
/// [`ErrorCode::InvalidRequest`] for a file of another format, and the SQLite
/// failure otherwise.
pub(crate) async fn page(
    data: &Path,
    filter: JournalFilter,
) -> Result<Vec<crible_store::debug_journal::DebugQuery>, CribleError> {
    let path = data.join(DEBUG_JOURNAL_FILE_NAME);
    tokio::task::spawn_blocking(move || read_page(&path, &filter))
        .await
        .map_err(|failure| internal(format!("the debug journal read task failed: {failure}")))?
}

#[cfg(test)]
mod tests {
    use super::{MAX_PAGE, PURGE_INTERVAL, QUEUE_CAPACITY, STOP_BOUND, should_sample};

    #[test]
    fn the_queue_capacity_is_the_constant_of_the_decision() {
        // ADR-0122, decision 6: a constant, never a configuration key.
        assert_eq!(QUEUE_CAPACITY, 4_096);
    }

    #[test]
    fn the_bounds_and_the_page_are_the_ones_of_the_decisions() {
        // ADR-0122, decisions 7 and 9.
        assert_eq!(STOP_BOUND.as_secs(), 2);
        assert_eq!(PURGE_INTERVAL.as_secs(), 24 * 60 * 60);
        assert_eq!(MAX_PAGE, 1_000);
    }

    #[test]
    fn the_draw_samples_the_two_edges_exactly() {
        // ADR-0122, decision 1: `0.0` records no ordinary request, `1.0` records
        // every one, whatever the integer the source draws.
        for draw in [0, 1, 123_456, u64::MAX] {
            assert!(!should_sample(0.0, draw), "rate 0.0, draw {draw}");
            assert!(should_sample(1.0, draw), "rate 1.0, draw {draw}");
        }
    }

    #[test]
    fn the_draw_keeps_the_fraction_of_the_domain_of_the_rate() {
        // A middle rate keeps a middle fraction of the integers: the small
        // draws are kept, the large ones are refused.
        let low = 1u64 << 61;
        assert!(should_sample(0.25, 0), "a small draw is kept at 1/4");
        assert!(should_sample(0.25, low), "a draw below a quarter is kept");
        assert!(
            !should_sample(0.25, 1u64 << 62),
            "a draw at a quarter is refused"
        );
        assert!(!should_sample(0.25, 1u64 << 63), "a large draw is refused");
        assert!(should_sample(0.5, low), "a draw below a half is kept");
        assert!(
            !should_sample(0.5, 1u64 << 63),
            "a draw at a half is refused"
        );
    }
}
