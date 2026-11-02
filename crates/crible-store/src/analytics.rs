// SPDX-License-Identifier: Apache-2.0

//! `analytics.sqlite`: the aggregated analytics of an instance.
//!
//! References: ADR-0117 (schema, histograms, percentile, filter shapes) and
//! ADR-0118 (bounded queue, flushes, purge, operations). The file lives at the
//! root of `<data>`, next to `instance.json` and `jobs.sqlite`
//! (`docs/02-architecture.md`); it is opened by this module of `crible-store`,
//! which already carries `rusqlite` (ADR-0038), so `crible-server` keeps no edge
//! to that crate (ADR-0103, ADR-0117 decision 1).
//!
//! # What the file holds
//!
//! Four tables, at `meta.format_version` 1 (ADR-0117, decision 1). The rows of
//! `minute_events` are aggregates of one minute, one index, one operation, one
//! status, one client, one mode and one dense strategy; `filter_shapes` counts
//! the canonical forms of the filters apart, so a new filter never creates a new
//! minute row; `day_indexes` is one snapshot per day and per index.
//!
//! # Content
//!
//! No text, no filter value, no hash and no identifier ever reaches this file
//! (INV-13, ADR-0014): the tables have no column for them, and the writer only
//! receives the closed lists of ADR-0117, decision 5.
//!
//! # Histograms
//!
//! A latency histogram is 64 `u32` counters in little-endian, bucket `i`
//! covering `[0.05 ms × 1.25^i ; 0.05 ms × 1.25^(i+1))` (ADR-0117, decision 2).
//! A pool histogram is 33 `u32` counters in little-endian, bucket 0 holding an
//! empty pool, bucket `i` of 1 to 31 covering `[2^(i-1) ; 2^i)`, and bucket 32
//! holding `2^31` and above (decision 3). A BLOB of another length is refused,
//! never truncated nor completed (INV-15).
//!
//! # Clock
//!
//! The code owns no clock: the minute of a flush and the day of a daily row are
//! read from SQLite, as `jobs.sqlite` already does (ADR-0103, ADR-0118
//! decisions 3 and 5). An event deposited by a request is therefore attached to
//! the minute of the flush that writes it, up to one flush interval away from
//! the instant it happened.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use crible_core::error::{CribleError, ErrorCode};
use crible_core::path::path_component;
use crible_core::time::{format_minute, parse_iso8601};
use rusqlite::types::Value;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params, params_from_iter};
use serde::{Deserialize, Serialize};

use crate::sqlite::{check_format_version, configure_busy_timeout, enable_wal};

/// Name of the analytics file at the root of `<data>` (ADR-0117, decision 1).
pub const ANALYTICS_FILE_NAME: &str = "analytics.sqlite";

/// Format of the file this build reads and writes (ADR-0117, decision 1).
pub const FORMAT_VERSION: u32 = 1;

/// Number of buckets of a latency histogram (ADR-0117, decision 2).
pub const LATENCY_BUCKETS: usize = 64;

/// Number of buckets of a pool histogram (ADR-0117, decision 3).
pub const POOL_BUCKETS: usize = 33;

/// First bound of the latency buckets, in milliseconds (ADR-0117, decision 2).
const LATENCY_FIRST_MS: f64 = 0.05;

/// Ratio between two consecutive latency bounds (ADR-0117, decision 2).
const LATENCY_RATIO: f64 = 1.25;

/// Number of `latency_*` columns of `minute_events`, in the order of the stages
/// of `docs/03-recherche.md`: `validate`, `encode`, `filter`, `dense`, `lexical`,
/// `fusion`, `group`, `fetch`, `total`.
const STAGES: usize = 9;

/// Names of the `latency_*` columns, in the order of [`STAGES`].
const STAGE_COLUMNS: [&str; STAGES] = [
    "latency_validate",
    "latency_encode",
    "latency_filter",
    "latency_dense",
    "latency_lexical",
    "latency_fusion",
    "latency_group",
    "latency_fetch",
    "latency_total",
];

/// One aggregate row of `minute_events`.
///
/// The same type is the input of [`AnalyticsStore::write`] and the output of
/// [`read_minute_events`]: a writer leaves [`MinuteEvent::minute`] empty and
/// SQLite fills it at the flush (ADR-0118, decision 3), a reader receives the
/// stored value.
///
/// The six count columns are `u64` here and `INTEGER` in the file: they count
/// events, so adding a period can never overflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinuteEvent {
    /// Minute the row is attached to, `AAAA-MM-JJTHH:MM:00Z`; empty on input.
    pub minute: String,
    /// Index the events targeted.
    pub index: String,
    /// Operation of the closed list of `docs/06-observabilite.md`.
    pub operation: String,
    /// `ok` or an error code of the API.
    pub status: String,
    /// Name of the API key, never the key nor its digest (ADR-0100).
    pub client: String,
    /// `dense`, `lexical` or `hybrid`; empty when the operation has no mode.
    pub mode: String,
    /// `exact` or `binary`; empty when no dense branch ran (ADR-0074).
    pub dense_strategy: String,
    /// Number of events of the row.
    pub count: u64,
    /// Number of those events that returned no result.
    pub zero_results: u64,
    /// Number of those events whose query was truncated.
    pub truncated: u64,
    /// Number of events whose results came from the dense branch alone.
    pub dense_only: u64,
    /// Number of events whose results came from the lexical branch alone.
    pub lexical_only: u64,
    /// Number of events whose results came from both branches.
    pub both: u64,
    /// Histogram of `validate`, 64 buckets.
    pub latency_validate: Vec<u32>,
    /// Histogram of `encode`, 64 buckets.
    pub latency_encode: Vec<u32>,
    /// Histogram of `filter`, 64 buckets.
    pub latency_filter: Vec<u32>,
    /// Histogram of `dense`, 64 buckets.
    pub latency_dense: Vec<u32>,
    /// Histogram of `lexical`, 64 buckets.
    pub latency_lexical: Vec<u32>,
    /// Histogram of `fusion`, 64 buckets.
    pub latency_fusion: Vec<u32>,
    /// Histogram of `group`, 64 buckets.
    pub latency_group: Vec<u32>,
    /// Histogram of `fetch`, 64 buckets.
    pub latency_fetch: Vec<u32>,
    /// Histogram of `total`, 64 buckets.
    pub latency_total: Vec<u32>,
    /// Histogram of the pool, 33 buckets.
    pub pool: Vec<u32>,
}

impl MinuteEvent {
    /// The nine latency histograms, in the order of `STAGE_COLUMNS`.
    fn histograms(&self) -> [&Vec<u32>; STAGES] {
        [
            &self.latency_validate,
            &self.latency_encode,
            &self.latency_filter,
            &self.latency_dense,
            &self.latency_lexical,
            &self.latency_fusion,
            &self.latency_group,
            &self.latency_fetch,
            &self.latency_total,
        ]
    }

    /// The nine latency histograms, mutable, in the order of `STAGE_COLUMNS`.
    fn histograms_mut(&mut self) -> [&mut Vec<u32>; STAGES] {
        [
            &mut self.latency_validate,
            &mut self.latency_encode,
            &mut self.latency_filter,
            &mut self.latency_dense,
            &mut self.latency_lexical,
            &mut self.latency_fusion,
            &mut self.latency_group,
            &mut self.latency_fetch,
            &mut self.latency_total,
        ]
    }

    /// An empty row for `index`, `operation` and the dimensions of a key.
    #[must_use]
    pub fn empty(
        index: impl Into<String>,
        operation: impl Into<String>,
        status: impl Into<String>,
    ) -> Self {
        Self {
            minute: String::new(),
            index: index.into(),
            operation: operation.into(),
            status: status.into(),
            client: String::new(),
            mode: String::new(),
            dense_strategy: String::new(),
            count: 0,
            zero_results: 0,
            truncated: 0,
            dense_only: 0,
            lexical_only: 0,
            both: 0,
            latency_validate: vec![0; LATENCY_BUCKETS],
            latency_encode: vec![0; LATENCY_BUCKETS],
            latency_filter: vec![0; LATENCY_BUCKETS],
            latency_dense: vec![0; LATENCY_BUCKETS],
            latency_lexical: vec![0; LATENCY_BUCKETS],
            latency_fusion: vec![0; LATENCY_BUCKETS],
            latency_group: vec![0; LATENCY_BUCKETS],
            latency_fetch: vec![0; LATENCY_BUCKETS],
            latency_total: vec![0; LATENCY_BUCKETS],
            pool: vec![0; POOL_BUCKETS],
        }
    }

    /// Adds one sample to the bucket `bucket` of the stage `stage`, saturating.
    ///
    /// `stage` is the index of the stage in `STAGE_COLUMNS`; an index beyond
    /// the nine does nothing, so a caller can never panic on it.
    pub fn add_latency(&mut self, stage: usize, bucket: usize) {
        if stage >= STAGES || bucket >= LATENCY_BUCKETS {
            return;
        }
        let histograms = self.histograms_mut();
        let value = &mut histograms[stage][bucket];
        *value = value.saturating_add(1);
    }

    /// Adds one sample to the bucket `bucket` of the pool histogram, saturating.
    pub fn add_pool(&mut self, bucket: usize) {
        if bucket >= POOL_BUCKETS {
            return;
        }
        self.pool[bucket] = self.pool[bucket].saturating_add(1);
    }

    /// Merges `other` into this row, bucket by bucket, in saturating `u32`.
    ///
    /// The counters of the key are added too; the histograms gain the samples of
    /// both, which is the addition ADR-0117, decision 4 requires.
    pub fn merge(&mut self, other: &Self) {
        self.count = self.count.saturating_add(other.count);
        self.zero_results = self.zero_results.saturating_add(other.zero_results);
        self.truncated = self.truncated.saturating_add(other.truncated);
        self.dense_only = self.dense_only.saturating_add(other.dense_only);
        self.lexical_only = self.lexical_only.saturating_add(other.lexical_only);
        self.both = self.both.saturating_add(other.both);
        for (left, right) in self
            .histograms_mut()
            .into_iter()
            .zip(other.histograms().into_iter().map(Vec::as_slice))
        {
            merge_histogram(left, right);
        }
        merge_histogram(&mut self.pool, &other.pool);
    }
}

/// One row of `filter_shapes`: a canonical form and its count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapeCount {
    /// Index the forms were exercised on.
    pub index: String,
    /// Canonical form of ADR-0117, decision 6; never empty.
    pub shape: String,
    /// Number of search events that carried this form.
    pub count: u64,
}

/// One row of `day_indexes`: the state of an index at the first flush of a day.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DayIndex {
    /// Day, `AAAA-MM-JJ`; empty on input.
    pub day: String,
    /// Index the row describes.
    pub index: String,
    /// Documents of the reference state.
    pub documents: u64,
    /// Passages alive at the generation.
    pub live_passages: u64,
    /// Passages tombstoned at the generation or an earlier one.
    pub dead_passages: u64,
    /// Bytes the index directory occupies.
    pub disk_bytes: u64,
    /// Published generation.
    pub generation: u64,
    /// Batches the reference state applied.
    pub batches: u64,
}

/// Bucket of the latency histogram a duration falls in (ADR-0117, decision 2).
///
/// Bucket 0 receives every duration strictly below 0.05 ms, including zero;
/// bucket 63 is closed on the right and receives every overflow.
#[must_use]
pub fn latency_bucket(duration: std::time::Duration) -> usize {
    let millis = duration.as_secs_f64() * 1_000.0;
    let mut bound = LATENCY_FIRST_MS;
    for bucket in 0..LATENCY_BUCKETS - 1 {
        bound *= LATENCY_RATIO;
        if millis < bound {
            return bucket;
        }
    }
    LATENCY_BUCKETS - 1
}

/// Upper bound of the latency bucket `bucket`, in milliseconds.
///
/// It is the value a percentile renders for that bucket (ADR-0117, decision 4).
#[must_use]
pub fn latency_bucket_upper_ms(bucket: usize) -> f64 {
    let bucket = bucket.min(LATENCY_BUCKETS - 1);
    let mut bound = LATENCY_FIRST_MS;
    for _ in 0..=bucket {
        bound *= LATENCY_RATIO;
    }
    bound
}

/// Bucket of the pool histogram a size falls in (ADR-0117, decision 3).
///
/// Bucket 0 holds an empty pool; bucket `i` of 1 to 31 holds `[2^(i-1) ; 2^i)`;
/// bucket 32 holds `2^31` and above.
#[must_use]
pub fn pool_bucket(pool: u32) -> usize {
    if pool == 0 {
        return 0;
    }
    // `floor(log2(pool)) + 1` is the bucket the table of decision 3 names:
    // 1 for 1, 2 for 2 and 3, 32 for `2^31` and above.
    let bucket = (u32::BITS - pool.leading_zeros()) as usize;
    bucket.min(POOL_BUCKETS - 1)
}

/// Upper bound of the pool bucket `bucket`, in passages.
///
/// The buckets follow `2^i` (ADR-0117, decision 4), with two exceptions of the
/// decision: the bucket 0 is the empty pool and reads `0`, and the last bucket is
/// **open at the right** — decision 3 reads it as "2^31 and beyond", so the bound
/// a row of it reports is its lower bound, `2^31` (POOL_BUCKETS is 33).
#[must_use]
pub fn pool_bucket_upper(bucket: usize) -> u64 {
    match bucket {
        0 => 0,
        bucket if bucket >= POOL_BUCKETS - 1 => 1 << (POOL_BUCKETS - 2),
        bucket => 1_u64 << bucket,
    }
}

/// Adds `right` into `left`, bucket by bucket, in saturating `u32`.
///
/// A histogram of another length is ignored: the caller validates the lengths
/// against [`FORMAT_VERSION`] when it decodes a BLOB.
pub fn merge_histogram(left: &mut [u32], right: &[u32]) {
    for (target, value) in left.iter_mut().zip(right) {
        *target = target.saturating_add(*value);
    }
}

/// Nearest-rank percentile of a latency histogram (ADR-0117, decision 4).
///
/// `n` is the `u64` sum of the buckets, `r = ceil(p × n / 100)` with at least
/// 1, and the result is the upper bound of the bucket that contains `r`. An
/// empty histogram has no percentile and returns `None`.
#[must_use]
pub fn latency_percentile(buckets: &[u32], p: u32) -> Option<f64> {
    nearest_rank(buckets, p).map(latency_bucket_upper_ms)
}

/// Nearest-rank percentile of a pool histogram (ADR-0117, decision 4).
#[must_use]
pub fn pool_percentile(buckets: &[u32], p: u32) -> Option<u64> {
    nearest_rank(buckets, p).map(pool_bucket_upper)
}

/// Index of the bucket that contains the nearest rank of `p`.
fn nearest_rank(buckets: &[u32], p: u32) -> Option<usize> {
    let total: u64 = buckets.iter().map(|bucket| u64::from(*bucket)).sum();
    if total == 0 {
        return None;
    }
    let rank = u64::from(p).saturating_mul(total).div_ceil(100).max(1);
    let mut cumulative = 0_u64;
    for (index, count) in buckets.iter().enumerate() {
        cumulative = cumulative.saturating_add(u64::from(*count));
        if cumulative >= rank {
            return Some(index);
        }
    }
    buckets.len().checked_sub(1)
}

/// Encodes 64 latency counters as little-endian `u32` (ADR-0117, decision 1).
#[must_use]
pub fn encode_latency(buckets: &[u32]) -> Vec<u8> {
    encode_histogram(buckets)
}

/// Encodes 33 pool counters as little-endian `u32`.
#[must_use]
pub fn encode_pool(buckets: &[u32]) -> Vec<u8> {
    encode_histogram(buckets)
}

/// Encodes one histogram as little-endian `u32`, without a header.
fn encode_histogram(buckets: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(buckets.len() * 4);
    for bucket in buckets {
        bytes.extend_from_slice(&bucket.to_le_bytes());
    }
    bytes
}

/// Decodes a latency BLOB of exactly 64 little-endian `u32`.
///
/// # Errors
///
/// [`ErrorCode::InvalidRequest`] when the BLOB is not 256 bytes, naming
/// `analytics.sqlite` and the column (INV-15).
pub fn decode_latency(bytes: &[u8], column: &str) -> Result<Vec<u32>, CribleError> {
    decode_histogram(bytes, LATENCY_BUCKETS, column)
}

/// Decodes a pool BLOB of exactly 33 little-endian `u32`.
///
/// # Errors
///
/// [`ErrorCode::InvalidRequest`] when the BLOB is not 132 bytes (INV-15).
pub fn decode_pool(bytes: &[u8], column: &str) -> Result<Vec<u32>, CribleError> {
    decode_histogram(bytes, POOL_BUCKETS, column)
}

/// Decodes a histogram of `buckets` little-endian `u32`.
fn decode_histogram(bytes: &[u8], buckets: usize, column: &str) -> Result<Vec<u32>, CribleError> {
    if bytes.len() != buckets * 4 {
        return Err(CribleError::new(
            ErrorCode::InvalidRequest,
            format!(
                "{ANALYTICS_FILE_NAME}: column {column} holds {} bytes, expected {} \
                 (format_version {FORMAT_VERSION})",
                bytes.len(),
                buckets * 4
            ),
        ));
    }
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| u32::from_le_bytes(*chunk))
        .collect())
}

/// The analytics file of an instance: one connection, one writer.
pub struct AnalyticsStore {
    connection: Mutex<Connection>,
}

impl AnalyticsStore {
    /// Opens `<data>/analytics.sqlite`, creating it with its schema and its
    /// `meta.format_version` when the file is missing or empty.
    ///
    /// An existing file whose version is not [`FORMAT_VERSION`], or that has no
    /// `meta` table, is refused with [`ErrorCode::InvalidRequest`] (INV-15).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidRequest`] for a file of another format, and the
    /// SQLite failure otherwise (ADR-0038 maps `SQLITE_BUSY` to
    /// [`ErrorCode::Overloaded`]).
    pub fn open_or_create(path: &Path) -> Result<Self, CribleError> {
        let fresh = is_new_database(path);
        let mut connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(|error| crate::records::database_error("open the analytics database", &error))?;
        configure_busy_timeout(&connection)?;
        enable_wal(&connection, "analytics")?;
        if fresh {
            create_schema(&mut connection)?;
        } else {
            check_format_version(&connection, FORMAT_VERSION, "analytics")?;
        }
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Writes one flush in a single transaction.
    ///
    /// The rows of `minute` are attached to the minute SQLite reads at the
    /// start of the transaction (ADR-0118, decision 3) and merged bucket by
    /// bucket with a row of the same key. The shapes of `shapes` are counted on
    /// the same minute, and the rows of `days` are written for the day SQLite
    /// reads then (decision 5). Either every row lands or none does.
    ///
    /// # Errors
    ///
    /// The SQLite failure, or [`ErrorCode::Internal`] for a value out of range.
    pub fn write(
        &self,
        minute: &[MinuteEvent],
        shapes: &[ShapeCount],
        days: &[DayIndex],
    ) -> Result<(), CribleError> {
        if minute.is_empty() && shapes.is_empty() && days.is_empty() {
            return Ok(());
        }
        self.with_mut(|connection| {
            let transaction = connection.transaction().map_err(|error| {
                crate::records::database_error("open the analytics flush", &error)
            })?;
            let now: String = transaction
                .query_row("SELECT strftime('%Y-%m-%dT%H:%M:00Z', 'now')", [], |row| {
                    row.get(0)
                })
                .map_err(|error| crate::records::database_error("read the minute", &error))?;
            for event in minute {
                write_minute(&transaction, &now, event)?;
            }
            for shape in shapes {
                write_shape(&transaction, &now, shape)?;
            }
            if !days.is_empty() {
                let day: String = transaction
                    .query_row("SELECT strftime('%Y-%m-%d', 'now')", [], |row| row.get(0))
                    .map_err(|error| crate::records::database_error("read the day", &error))?;
                for row in days {
                    write_day(&transaction, &day, row)?;
                }
            }
            transaction.commit().map_err(|error| {
                crate::records::database_error("commit the analytics flush", &error)
            })
        })
    }

    /// Names of `indexes` that have no `day_indexes` row for the current UTC day
    /// (ADR-0118, decision 5).
    ///
    /// The caller computes only those rows: a daily snapshot walks the disk of
    /// each index, and an instance that already wrote its day must not walk it
    /// again at every flush.
    ///
    /// # Errors
    ///
    /// The SQLite failure.
    pub fn missing_days(&self, indexes: &[String]) -> Result<Vec<String>, CribleError> {
        let present = self.with(|connection| {
            let mut statement = connection
                .prepare(
                    "SELECT \"index\" FROM day_indexes
                     WHERE day = strftime('%Y-%m-%d', 'now')",
                )
                .map_err(|error| {
                    crate::records::database_error("prepare the daily rows query", &error)
                })?;
            let names = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|error| crate::records::database_error("list the daily rows", &error))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| crate::records::database_error("read the daily rows", &error))?;
            Ok(names)
        })?;
        Ok(indexes
            .iter()
            .filter(|name| !present.contains(name))
            .cloned()
            .collect())
    }

    /// Deletes the rows older than `retention_days` days and returns their
    /// number (ADR-0118, decision 4).
    ///
    /// The three tables are purged in one transaction: a minute, its filter
    /// shapes and the daily rows older than the retention all go together.
    ///
    /// # Errors
    ///
    /// The SQLite failure.
    pub fn purge(&self, retention_days: usize) -> Result<usize, CribleError> {
        let offset = format!("-{retention_days} days");
        self.with_mut(|connection| {
            let transaction = connection.transaction().map_err(|error| {
                crate::records::database_error("open the analytics purge", &error)
            })?;
            let mut purged = 0;
            purged += transaction
                .execute(
                    "DELETE FROM minute_events
                     WHERE minute < strftime('%Y-%m-%dT%H:%M:00Z', 'now', ?1)",
                    params![offset],
                )
                .map_err(|error| {
                    crate::records::database_error("purge the minute events", &error)
                })?;
            purged += transaction
                .execute(
                    "DELETE FROM filter_shapes
                     WHERE minute < strftime('%Y-%m-%dT%H:%M:00Z', 'now', ?1)",
                    params![offset],
                )
                .map_err(|error| {
                    crate::records::database_error("purge the filter shapes", &error)
                })?;
            purged += transaction
                .execute(
                    "DELETE FROM day_indexes WHERE day < strftime('%Y-%m-%d', 'now', ?1)",
                    params![offset],
                )
                .map_err(|error| crate::records::database_error("purge the daily rows", &error))?;
            transaction.commit().map_err(|error| {
                crate::records::database_error("commit the analytics purge", &error)
            })?;
            Ok(purged)
        })
    }

    /// Runs `action` on the connection under its lock.
    fn with<T>(
        &self,
        action: impl FnOnce(&Connection) -> Result<T, CribleError>,
    ) -> Result<T, CribleError> {
        let connection = self.connection.lock().map_err(|_| {
            CribleError::new(ErrorCode::Internal, "the analytics database is poisoned")
        })?;
        action(&connection)
    }

    /// Runs `action` on the mutable connection under its lock.
    fn with_mut<T>(
        &self,
        action: impl FnOnce(&mut Connection) -> Result<T, CribleError>,
    ) -> Result<T, CribleError> {
        let mut connection = self.connection.lock().map_err(|_| {
            CribleError::new(ErrorCode::Internal, "the analytics database is poisoned")
        })?;
        action(&mut connection)
    }
}

/// Reads every row of `minute_events`, ordered by minute then by key.
///
/// It opens the file **read-only**, so a test or a tool never competes with the
/// writer of the instance, and refuses a version this build does not read.
///
/// # Errors
///
/// [`ErrorCode::InvalidRequest`] for a file of another format, a row that this
/// build cannot read or a BLOB of another length, and the SQLite failure
/// otherwise.
pub fn read_minute_events(path: &Path) -> Result<Vec<MinuteEvent>, CribleError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| crate::records::database_error("open the analytics database", &error))?;
    configure_busy_timeout(&connection)?;
    check_format_version(&connection, FORMAT_VERSION, "analytics")?;
    let mut statement = connection
        .prepare(
            "SELECT minute, \"index\", operation, status, client, mode, dense_strategy,
                    count, zero_results, truncated, dense_only, lexical_only, both,
                    latency_validate, latency_encode, latency_filter, latency_dense,
                    latency_lexical, latency_fusion, latency_group, latency_fetch,
                    latency_total, pool
             FROM minute_events
             ORDER BY minute, \"index\", operation, status, client, mode, dense_strategy",
        )
        .map_err(|error| crate::records::database_error("prepare the analytics query", &error))?;
    let rows = statement
        .query_map([], read_minute_row)
        .map_err(|error| crate::records::database_error("query the analytics", &error))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| crate::records::database_error("read the analytics", &error))?;
    rows.into_iter().map(decode_minute_row).collect()
}

/// One row of `minute_events` as SQLite returned it, BLOBs included.
type RawMinute = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
);

/// Reads one row of [`read_minute_events`].
fn read_minute_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawMinute> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
        row.get(14)?,
        row.get(15)?,
        row.get(16)?,
        row.get(17)?,
        row.get(18)?,
        row.get(19)?,
        row.get(20)?,
        row.get(21)?,
        row.get(22)?,
    ))
}

/// Converts a row of [`read_minute_events`] and decodes its histograms.
#[allow(clippy::too_many_arguments)]
fn decode_minute_row(raw: RawMinute) -> Result<MinuteEvent, CribleError> {
    let (
        minute,
        index,
        operation,
        status,
        client,
        mode,
        dense_strategy,
        count,
        zero_results,
        truncated,
        dense_only,
        lexical_only,
        both,
        latency_validate,
        latency_encode,
        latency_filter,
        latency_dense,
        latency_lexical,
        latency_fusion,
        latency_group,
        latency_fetch,
        latency_total,
        pool,
    ) = raw;
    let blobs = [
        latency_validate,
        latency_encode,
        latency_filter,
        latency_dense,
        latency_lexical,
        latency_fusion,
        latency_group,
        latency_fetch,
        latency_total,
    ];
    let mut histograms = Vec::with_capacity(STAGES);
    for (column, bytes) in STAGE_COLUMNS.iter().zip(blobs.iter()) {
        histograms.push(decode_latency(bytes, column)?);
    }
    Ok(MinuteEvent {
        minute,
        index,
        operation,
        status,
        client,
        mode,
        dense_strategy,
        count: count_u64(count, "count")?,
        zero_results: count_u64(zero_results, "zero_results")?,
        truncated: count_u64(truncated, "truncated")?,
        dense_only: count_u64(dense_only, "dense_only")?,
        lexical_only: count_u64(lexical_only, "lexical_only")?,
        both: count_u64(both, "both")?,
        latency_validate: histograms.remove(0),
        latency_encode: histograms.remove(0),
        latency_filter: histograms.remove(0),
        latency_dense: histograms.remove(0),
        latency_lexical: histograms.remove(0),
        latency_fusion: histograms.remove(0),
        latency_group: histograms.remove(0),
        latency_fetch: histograms.remove(0),
        latency_total: histograms.remove(0),
        pool: decode_pool(&pool, "pool")?,
    })
}

/// True when `path` does not hold a database yet: missing file, or empty file
/// left by a failed creation.
fn is_new_database(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(metadata) => metadata.len() == 0,
        Err(_) => true,
    }
}

/// Creates the schema of a fresh analytics database and its version
/// (ADR-0117, decision 1).
fn create_schema(connection: &mut Connection) -> Result<(), CribleError> {
    let transaction = connection
        .transaction()
        .map_err(|error| crate::records::database_error("open the analytics schema", &error))?;
    transaction
        .execute_batch(
            "CREATE TABLE meta (format_version INTEGER NOT NULL);
             CREATE TABLE minute_events (
               minute            TEXT NOT NULL,
               \"index\"           TEXT NOT NULL,
               operation         TEXT NOT NULL,
               status            TEXT NOT NULL,
               client            TEXT NOT NULL,
               mode              TEXT NOT NULL,
               dense_strategy    TEXT NOT NULL,
               count             INTEGER NOT NULL,
               zero_results      INTEGER NOT NULL,
               truncated         INTEGER NOT NULL,
               dense_only        INTEGER NOT NULL,
               lexical_only      INTEGER NOT NULL,
               both              INTEGER NOT NULL,
               latency_validate  BLOB NOT NULL,
               latency_encode    BLOB NOT NULL,
               latency_filter    BLOB NOT NULL,
               latency_dense     BLOB NOT NULL,
               latency_lexical   BLOB NOT NULL,
               latency_fusion    BLOB NOT NULL,
               latency_group     BLOB NOT NULL,
               latency_fetch     BLOB NOT NULL,
               latency_total     BLOB NOT NULL,
               pool              BLOB NOT NULL,
               PRIMARY KEY (minute, \"index\", operation, status, client, mode, dense_strategy)
             );
             CREATE TABLE day_indexes (
               day             TEXT NOT NULL,
               \"index\"         TEXT NOT NULL,
               documents       INTEGER NOT NULL,
               live_passages   INTEGER NOT NULL,
               dead_passages   INTEGER NOT NULL,
               disk_bytes      INTEGER NOT NULL,
               generation      INTEGER NOT NULL,
               batches         INTEGER NOT NULL,
               PRIMARY KEY (day, \"index\")
             );
             CREATE TABLE filter_shapes (
               minute   TEXT NOT NULL,
               \"index\"  TEXT NOT NULL,
               shape    TEXT NOT NULL,
               count    INTEGER NOT NULL,
               PRIMARY KEY (minute, \"index\", shape)
             );
             INSERT INTO meta (format_version) VALUES (1);",
        )
        .map_err(|error| crate::records::database_error("create the analytics schema", &error))?;
    transaction
        .commit()
        .map_err(|error| crate::records::database_error("commit the analytics schema", &error))
}

/// Reads the row of `event` keyed like it, merges it and writes it back.
fn write_minute(
    transaction: &rusqlite::Transaction<'_>,
    minute: &str,
    event: &MinuteEvent,
) -> Result<(), CribleError> {
    let existing = read_existing_minute(transaction, minute, event)?;
    let mut merged = existing.unwrap_or_else(|| {
        MinuteEvent::empty(
            event.index.as_str(),
            event.operation.as_str(),
            event.status.as_str(),
        )
    });
    merged.client.clone_from(&event.client);
    merged.mode.clone_from(&event.mode);
    merged.dense_strategy.clone_from(&event.dense_strategy);
    merged.merge(event);
    merged.minute = minute.to_owned();

    let columns = [
        merged.minute.clone(),
        merged.index.clone(),
        merged.operation.clone(),
        merged.status.clone(),
        merged.client.clone(),
        merged.mode.clone(),
        merged.dense_strategy.clone(),
    ];
    let mut values = Vec::with_capacity(3 + STAGES + 1);
    values.push(to_sql(merged.count, "count")?);
    values.push(to_sql(merged.zero_results, "zero_results")?);
    values.push(to_sql(merged.truncated, "truncated")?);
    values.push(to_sql(merged.dense_only, "dense_only")?);
    values.push(to_sql(merged.lexical_only, "lexical_only")?);
    values.push(to_sql(merged.both, "both")?);
    let mut blobs = Vec::with_capacity(STAGES + 1);
    for histogram in merged.histograms() {
        blobs.push(encode_latency(histogram));
    }
    blobs.push(encode_pool(&merged.pool));

    // Seven key columns, six counters, one histogram per stage and the pool.
    let placeholders = (1..=7 + 6 + STAGES + 1)
        .map(|position| format!("?{position}"))
        .collect::<Vec<_>>()
        .join(", ");
    let statement = format!(
        "INSERT OR REPLACE INTO minute_events
           (minute, \"index\", operation, status, client, mode, dense_strategy,
            count, zero_results, truncated, dense_only, lexical_only, both,
            latency_validate, latency_encode, latency_filter, latency_dense,
            latency_lexical, latency_fusion, latency_group, latency_fetch,
            latency_total, pool)
         VALUES ({placeholders})"
    );
    let mut parameters: Vec<&dyn rusqlite::ToSql> =
        Vec::with_capacity(values.len() + blobs.len() + 7);
    parameters.push(&columns[0]);
    parameters.push(&columns[1]);
    parameters.push(&columns[2]);
    parameters.push(&columns[3]);
    parameters.push(&columns[4]);
    parameters.push(&columns[5]);
    parameters.push(&columns[6]);
    for value in &values {
        parameters.push(value);
    }
    for blob in &blobs {
        parameters.push(blob);
    }
    transaction
        .execute(&statement, parameters.as_slice())
        .map_err(|error| crate::records::database_error("write an analytics minute", &error))?;
    Ok(())
}

/// Reads the row of `minute_events` with the key of `event`, if it exists.
fn read_existing_minute(
    transaction: &rusqlite::Transaction<'_>,
    minute: &str,
    event: &MinuteEvent,
) -> Result<Option<MinuteEvent>, CribleError> {
    let mut statement = transaction
        .prepare(
            "SELECT minute, \"index\", operation, status, client, mode, dense_strategy,
                    count, zero_results, truncated, dense_only, lexical_only, both,
                    latency_validate, latency_encode, latency_filter, latency_dense,
                    latency_lexical, latency_fusion, latency_group, latency_fetch,
                    latency_total, pool
             FROM minute_events
             WHERE minute = ?1 AND \"index\" = ?2 AND operation = ?3 AND status = ?4
               AND client = ?5 AND mode = ?6 AND dense_strategy = ?7",
        )
        .map_err(|error| crate::records::database_error("prepare the analytics merge", &error))?;
    let row = statement
        .query_row(
            params![
                minute,
                event.index.as_str(),
                event.operation.as_str(),
                event.status.as_str(),
                event.client.as_str(),
                event.mode.as_str(),
                event.dense_strategy.as_str()
            ],
            read_minute_row,
        )
        .optional()
        .map_err(|error| crate::records::database_error("read an analytics minute", &error))?;
    drop(statement);
    row.map(decode_minute_row).transpose()
}

/// Adds one to the row of `shape` for `minute`.
fn write_shape(
    transaction: &rusqlite::Transaction<'_>,
    minute: &str,
    shape: &ShapeCount,
) -> Result<(), CribleError> {
    let existing: Option<i64> = transaction
        .query_row(
            "SELECT count FROM filter_shapes WHERE minute = ?1 AND \"index\" = ?2 AND shape = ?3",
            params![minute, shape.index.as_str(), shape.shape.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| crate::records::database_error("read a filter shape", &error))?;
    let count = existing
        .map(|count| count_u64(count, "filter_shapes.count"))
        .transpose()?
        .unwrap_or(0)
        .saturating_add(shape.count);
    transaction
        .execute(
            "INSERT OR REPLACE INTO filter_shapes (minute, \"index\", shape, count)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                minute,
                shape.index.as_str(),
                shape.shape.as_str(),
                to_sql(count, "filter_shapes.count")?
            ],
        )
        .map_err(|error| crate::records::database_error("write a filter shape", &error))?;
    Ok(())
}

/// Writes one daily row for `day`.
fn write_day(
    transaction: &rusqlite::Transaction<'_>,
    day: &str,
    row: &DayIndex,
) -> Result<(), CribleError> {
    transaction
        .execute(
            "INSERT OR REPLACE INTO day_indexes
               (day, \"index\", documents, live_passages, dead_passages, disk_bytes,
                generation, batches)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                day,
                row.index.as_str(),
                to_sql(row.documents, "documents")?,
                to_sql(row.live_passages, "live_passages")?,
                to_sql(row.dead_passages, "dead_passages")?,
                to_sql(row.disk_bytes, "disk_bytes")?,
                to_sql(row.generation, "generation")?,
                to_sql(row.batches, "batches")?,
            ],
        )
        .map_err(|error| crate::records::database_error("write a daily row", &error))?;
    Ok(())
}

/// Converts a `u64` count into the `i64` SQLite stores.
fn to_sql(value: u64, column: &str) -> Result<i64, CribleError> {
    i64::try_from(value).map_err(|_| {
        CribleError::new(
            ErrorCode::Internal,
            format!("{ANALYTICS_FILE_NAME}: the {column} counter is out of range"),
        )
    })
}

/// Converts an `i64` read from SQLite into a `u64` counter.
fn count_u64(value: i64, column: &str) -> Result<u64, CribleError> {
    u64::try_from(value).map_err(|_| {
        CribleError::new(
            ErrorCode::InvalidRequest,
            format!("{ANALYTICS_FILE_NAME}: column {column} is negative ({value})"),
        )
    })
}

/// Number of entries of the two rankings (ADR-0124, decisions 6 and 7).
pub const TOP_ENTRIES: usize = 10;

/// The ten operations of `docs/06-observabilite.md` (ADR-0118, decision 6).
///
/// The list is closed: a request that names another operation is refused
/// (ADR-0124, decision 2). The names are the ones [`MinuteEvent::operation`]
/// stores.
pub const OPERATIONS: [&str; 10] = [
    "search",
    "search_multi",
    "get_document",
    "online_batch",
    "attach_batch",
    "patch_documents",
    "compact",
    "export",
    "snapshot",
    "index_admin",
];

/// The two search operations, the denominator of `zero_results_rate`
/// (ADR-0124, decision 5).
const SEARCH_OPERATIONS: [&str; 2] = ["search", "search_multi"];

/// Width of the buckets of a statistics answer (ADR-0124, decision 2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatsGroupBy {
    /// One bucket per stored minute, all keys of the minute together.
    Minute,
    /// One bucket per hour, the default.
    #[default]
    Hour,
    /// One bucket per UTC day.
    Day,
}

impl StatsGroupBy {
    /// Wire name of the width, the one `GET /v1/stats` echoes.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Minute => "minute",
            Self::Hour => "hour",
            Self::Day => "day",
        }
    }

    /// Reads a wire name; `None` when it is outside the closed domain.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "minute" => Some(Self::Minute),
            "hour" => Some(Self::Hour),
            "day" => Some(Self::Day),
            _ => None,
        }
    }

    /// The SQL expression that renders the start of a bucket.
    fn sql(self) -> &'static str {
        match self {
            Self::Minute => "minute",
            Self::Hour => "substr(minute, 1, 13) || ':00:00Z'",
            Self::Day => "substr(minute, 1, 10) || 'T00:00:00Z'",
        }
    }
}

/// The five parameters of the statistics read, checked and normalised
/// (ADR-0124, decision 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatsFilter {
    /// Lower bound, included, in the `minute` form SQLite stores.
    pub from: Option<String>,
    /// Upper bound, included, in the `minute` form SQLite stores.
    pub to: Option<String>,
    /// Index whose events are kept.
    pub index: Option<String>,
    /// Operation whose events are kept, from [`OPERATIONS`].
    pub operation: Option<String>,
    /// Width of the buckets.
    pub group_by: StatsGroupBy,
}

impl StatsFilter {
    /// Reads the five parameters of `GET /v1/stats` and of `crible stats`.
    ///
    /// A date is an ISO 8601 date with its offset, read at the resolution of a
    /// minute because a row counts a whole minute; both bounds are included. The
    /// window defaults to the whole retention, which [`read_stats`] resolves
    /// from the file. Every refusal is [`ErrorCode::InvalidRequest`] and names
    /// the parameter (ADR-0122, decision 9; ADR-0124, decision 2).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidRequest`] for a date outside ISO 8601, a `from` after
    /// the `to`, an `index` outside the naming rules, an `operation` outside
    /// [`OPERATIONS`] and a `group_by` outside its closed domain.
    pub fn parse(
        from: Option<&str>,
        to: Option<&str>,
        index: Option<&str>,
        operation: Option<&str>,
        group_by: Option<&str>,
    ) -> Result<Self, CribleError> {
        let from = from.map(|raw| minute_bound(raw, "from")).transpose()?;
        let to = to.map(|raw| minute_bound(raw, "to")).transpose()?;
        if let (Some(from), Some(to)) = (&from, &to)
            && from > to
        {
            return Err(CribleError::new(
                ErrorCode::InvalidRequest,
                "'from' must not be after 'to'".to_owned(),
            ));
        }
        let index = match index {
            Some(raw) => {
                path_component(raw, "the parameter 'index'")?;
                Some(raw.to_owned())
            }
            None => None,
        };
        let operation = match operation {
            Some(raw) if OPERATIONS.contains(&raw) => Some(raw.to_owned()),
            Some(raw) => {
                return Err(CribleError::new(
                    ErrorCode::InvalidRequest,
                    format!(
                        "'operation' is not one of {}: '{raw}'",
                        OPERATIONS.join(", ")
                    ),
                ));
            }
            None => None,
        };
        let group_by = match group_by {
            Some(raw) => StatsGroupBy::parse(raw).ok_or_else(|| {
                CribleError::new(
                    ErrorCode::InvalidRequest,
                    format!("'group_by' must be 'minute', 'hour' or 'day': '{raw}'"),
                )
            })?,
            None => StatsGroupBy::default(),
        };
        Ok(Self {
            from,
            to,
            index,
            operation,
            group_by,
        })
    }
}

/// Reads one ISO 8601 bound at the resolution of a minute.
fn minute_bound(raw: &str, parameter: &str) -> Result<String, CribleError> {
    let instant = parse_iso8601(raw, parameter)?;
    Ok(format_minute(instant.floor_to_minute()))
}

/// The answer of `GET /v1/stats`, without `analytics_dropped`, which belongs to
/// a running process and not to the file (ADR-0124, decisions 1 and 9).
///
/// A member without a value is omitted, never rendered `null` nor filled with an
/// invented `0` (ADR-0119, decision 3). The four lists are always present, even
/// empty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    /// Lower bound applied, in UTC; absent when the file holds no row and no
    /// bound was asked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Upper bound applied, in UTC; absent for the same reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Width of the buckets, echoed.
    pub group_by: StatsGroupBy,
    /// Buckets that hold at least one event, in chronological order.
    pub buckets: Vec<StatsBucket>,
    /// The same aggregate over the whole window, without `start`; absent when
    /// the window holds no event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub totals: Option<StatsTotals>,
    /// At most ten indexes, by count descending then name.
    pub top_indexes: Vec<StatsIndexCount>,
    /// At most ten filter forms, by count descending then form.
    pub top_filter_shapes: Vec<StatsFilterShape>,
    /// The last daily row of each index in the window, by name.
    pub indexes: Vec<StatsIndexState>,
}

/// One bucket of a statistics answer (ADR-0124, decision 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatsBucket {
    /// Start of the bucket in UTC, truncated to the width.
    pub start: String,
    /// Number of events of the bucket.
    pub count: u64,
    /// Events that failed, by error code; omitted without an error.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub errors: BTreeMap<String, u64>,
    /// Share of the successful searches that returned nothing; omitted when no
    /// successful search ran (ADR-0124, decision 5).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zero_results_rate: Option<f64>,
    /// Percentiles per stage; a stage without a sample is omitted.
    #[serde(skip_serializing_if = "StatsLatency::is_empty")]
    pub latency_ms: StatsLatency,
}

/// The aggregate of the whole window, the shape of a bucket without `start`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatsTotals {
    /// Number of events of the window.
    pub count: u64,
    /// Events that failed, by error code; omitted without an error.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub errors: BTreeMap<String, u64>,
    /// Share of the successful searches that returned nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zero_results_rate: Option<f64>,
    /// Percentiles per stage over the window.
    #[serde(skip_serializing_if = "StatsLatency::is_empty")]
    pub latency_ms: StatsLatency,
}

/// The three percentiles of one stage, in milliseconds (ADR-0124, decision 4).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StatsPercentiles {
    /// Nearest-rank median.
    pub p50: f64,
    /// Nearest-rank 95th percentile.
    pub p95: f64,
    /// Nearest-rank 99th percentile.
    pub p99: f64,
}

/// The percentiles of the nine stages of `StageDurations`.
///
/// The fields keep the order of the stages, so the serialised object is stable.
/// A stage without a sample in the window is omitted.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StatsLatency {
    /// Step 1, validation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validate: Option<StatsPercentiles>,
    /// Step 2, encoding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encode: Option<StatsPercentiles>,
    /// Step 3, pool.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<StatsPercentiles>,
    /// Step 4, dense branch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dense: Option<StatsPercentiles>,
    /// Step 5, lexical branch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lexical: Option<StatsPercentiles>,
    /// Step 6, fusion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fusion: Option<StatsPercentiles>,
    /// Step 7, regrouping.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<StatsPercentiles>,
    /// Step 8, payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetch: Option<StatsPercentiles>,
    /// Total of the request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<StatsPercentiles>,
}

impl StatsLatency {
    /// True when no stage has a sample, so the member is omitted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.validate.is_none()
            && self.encode.is_none()
            && self.filter.is_none()
            && self.dense.is_none()
            && self.lexical.is_none()
            && self.fusion.is_none()
            && self.group.is_none()
            && self.fetch.is_none()
            && self.total.is_none()
    }
}

/// One entry of `top_indexes` (ADR-0124, decision 7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatsIndexCount {
    /// Name of the index.
    pub index: String,
    /// Events of that index on the window.
    pub count: u64,
}

/// One entry of `top_filter_shapes` (ADR-0124, decision 6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatsFilterShape {
    /// Canonical form of ADR-0117, decision 6: field names and operators only.
    pub shape: String,
    /// Searches that carried that form on the window.
    pub count: u64,
}

/// One entry of `indexes`: the last daily row of an index in the window
/// (ADR-0124, decision 8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatsIndexState {
    /// Day of the row, `AAAA-MM-JJ`.
    pub day: String,
    /// Index the row describes.
    pub index: String,
    /// Documents of the reference state.
    pub documents: u64,
    /// Passages alive at the generation.
    pub live_passages: u64,
    /// Passages tombstoned at the generation or an earlier one.
    pub dead_passages: u64,
    /// Derived `dead_passages / (live_passages + dead_passages)`, never stored.
    pub dead_ratio: f64,
    /// Bytes the index directory occupies.
    pub disk_bytes: u64,
    /// Published generation.
    pub generation: u64,
    /// Batches the reference state applied.
    pub batches: u64,
}

/// Aggregation of a window or a bucket, histograms included.
///
/// The counters and the histograms are accumulated in the same pass, and the
/// histograms are merged bucket by bucket, so a percentile of the aggregate is
/// the percentile of the raw events at one bucket's resolution (ADR-0117,
/// decision 4; ADR-0124, decision 12).
#[derive(Debug, Clone)]
struct Aggregate {
    /// Number of events.
    count: u64,
    /// Events of the successful searches that returned nothing.
    zero_results: u64,
    /// Events of the successful searches, the denominator of the rate.
    searches: u64,
    /// Events that failed, by error code.
    errors: BTreeMap<String, u64>,
    /// Merged histograms, in the order of `STAGE_COLUMNS`.
    histograms: [Vec<u32>; STAGES],
}

impl Default for Aggregate {
    fn default() -> Self {
        Self {
            count: 0,
            zero_results: 0,
            searches: 0,
            errors: BTreeMap::new(),
            histograms: std::array::from_fn(|_| vec![0; LATENCY_BUCKETS]),
        }
    }
}

impl Aggregate {
    /// Adds the counters and the histograms of `other`, saturating.
    fn merge(&mut self, other: &Self) {
        self.count = self.count.saturating_add(other.count);
        self.zero_results = self.zero_results.saturating_add(other.zero_results);
        self.searches = self.searches.saturating_add(other.searches);
        for (status, count) in &other.errors {
            let entry = self.errors.entry(status.clone()).or_insert(0);
            *entry = entry.saturating_add(*count);
        }
        for (left, right) in self.histograms.iter_mut().zip(other.histograms.iter()) {
            merge_histogram(left, right);
        }
    }

    /// Share of the successful searches that returned nothing, or `None` when
    /// no successful search ran (ADR-0124, decision 5).
    fn rate(&self) -> Option<f64> {
        if self.searches == 0 {
            return None;
        }
        Some(self.zero_results as f64 / self.searches as f64)
    }

    /// The percentiles of the nine stages, an empty stage omitted.
    fn latency(&self) -> StatsLatency {
        let stage = |position: usize| percentiles(&self.histograms[position]);
        StatsLatency {
            validate: stage(0),
            encode: stage(1),
            filter: stage(2),
            dense: stage(3),
            lexical: stage(4),
            fusion: stage(5),
            group: stage(6),
            fetch: stage(7),
            total: stage(8),
        }
    }
}

/// The three percentiles of one histogram, or `None` when it is empty.
fn percentiles(buckets: &[u32]) -> Option<StatsPercentiles> {
    Some(StatsPercentiles {
        p50: latency_percentile(buckets, 50)?,
        p95: latency_percentile(buckets, 95)?,
        p99: latency_percentile(buckets, 99)?,
    })
}

/// Reads the aggregated analytics of `path` over the window of `filter`.
///
/// It opens the file **read-only** on its own connection, so a read never waits
/// for a flush and never blocks the writer; it writes nothing (ADR-0116,
/// decision 1; ADR-0124, decision 12). The aggregation is done by SQL for the
/// counters and by merging the stored histograms for the percentiles, which
/// SQLite cannot add.
///
/// # Errors
///
/// [`ErrorCode::InvalidRequest`] for a file of another format, a row this build
/// cannot read or a BLOB of another length (INV-15), and the SQLite failure
/// otherwise.
pub fn read_stats(path: &Path, filter: &StatsFilter) -> Result<Stats, CribleError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| crate::records::database_error("open the analytics database", &error))?;
    configure_busy_timeout(&connection)?;
    check_format_version(&connection, FORMAT_VERSION, "analytics")?;

    let (oldest, newest) = retained_bounds(&connection)?;
    let from = filter.from.clone().or(oldest);
    let to = filter.to.clone().or(newest);

    let mut buckets: BTreeMap<String, Aggregate> = BTreeMap::new();
    let mut total = Aggregate::default();
    read_counters(
        &connection,
        filter,
        from.as_deref(),
        to.as_deref(),
        &mut buckets,
        &mut total,
    )?;
    read_histograms(
        &connection,
        filter,
        from.as_deref(),
        to.as_deref(),
        &mut buckets,
        &mut total,
    )?;

    let buckets = buckets
        .into_iter()
        .map(|(start, aggregate)| StatsBucket {
            start,
            count: aggregate.count,
            zero_results_rate: aggregate.rate(),
            latency_ms: aggregate.latency(),
            errors: aggregate.errors,
        })
        .collect();
    let totals = (total.count > 0).then(|| StatsTotals {
        count: total.count,
        zero_results_rate: total.rate(),
        latency_ms: total.latency(),
        errors: total.errors,
    });
    let top_indexes = read_top_indexes(&connection, filter, from.as_deref(), to.as_deref())?;
    let top_filter_shapes = read_top_shapes(&connection, filter, from.as_deref(), to.as_deref())?;
    let indexes = read_index_states(&connection, filter, from.as_deref(), to.as_deref())?;
    Ok(Stats {
        from,
        to,
        group_by: filter.group_by,
        buckets,
        totals,
        top_indexes,
        top_filter_shapes,
        indexes,
    })
}

/// The oldest and the newest stored minute, the default window (ADR-0124,
/// decision 2).
fn retained_bounds(
    connection: &Connection,
) -> Result<(Option<String>, Option<String>), CribleError> {
    connection
        .query_row(
            "SELECT MIN(minute), MAX(minute) FROM minute_events",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| crate::records::database_error("read the retained window", &error))
}

/// Accumulates the counters and the error codes of each bucket and of the whole
/// window.
fn read_counters(
    connection: &Connection,
    filter: &StatsFilter,
    from: Option<&str>,
    to: Option<&str>,
    buckets: &mut BTreeMap<String, Aggregate>,
    total: &mut Aggregate,
) -> Result<(), CribleError> {
    let (clause, values) = minute_conditions(filter, from, to, true);
    let sql = format!(
        "SELECT {bucket} AS bucket, status, SUM(count),
                SUM(CASE WHEN operation IN ({searches}) AND status = 'ok'
                         THEN zero_results ELSE 0 END),
                SUM(CASE WHEN operation IN ({searches}) AND status = 'ok'
                         THEN count ELSE 0 END)
         FROM minute_events WHERE {clause}
         GROUP BY bucket, status
         ORDER BY bucket",
        bucket = filter.group_by.sql(),
        searches = quoted(SEARCH_OPERATIONS),
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| crate::records::database_error("prepare the statistics", &error))?;
    let rows = statement
        .query_map(params_from_iter(values), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|error| crate::records::database_error("query the statistics", &error))?;
    for row in rows {
        let (start, status, count, zero_results, searches) =
            row.map_err(|error| crate::records::database_error("read the statistics", &error))?;
        let mut aggregate = Aggregate {
            count: count_u64(count, "count")?,
            zero_results: count_u64(zero_results, "zero_results")?,
            searches: count_u64(searches, "count")?,
            ..Aggregate::default()
        };
        if status != "ok" {
            aggregate.errors.insert(status, aggregate.count);
        }
        buckets.entry(start).or_default().merge(&aggregate);
        total.merge(&aggregate);
    }
    Ok(())
}

/// Merges the nine latency histograms of each bucket and of the whole window.
fn read_histograms(
    connection: &Connection,
    filter: &StatsFilter,
    from: Option<&str>,
    to: Option<&str>,
    buckets: &mut BTreeMap<String, Aggregate>,
    total: &mut Aggregate,
) -> Result<(), CribleError> {
    let (clause, values) = minute_conditions(filter, from, to, true);
    let columns = STAGE_COLUMNS.join(", ");
    let sql = format!(
        "SELECT {bucket} AS bucket, {columns} FROM minute_events WHERE {clause} ORDER BY bucket",
        bucket = filter.group_by.sql(),
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| crate::records::database_error("prepare the histograms", &error))?;
    let mut rows = statement
        .query(params_from_iter(values))
        .map_err(|error| crate::records::database_error("query the histograms", &error))?;
    while let Some(row) = rows
        .next()
        .map_err(|error| crate::records::database_error("read the histograms", &error))?
    {
        let start: String = row.get(0).map_err(|error| {
            crate::records::database_error("read the bucket of a histogram", &error)
        })?;
        let bucket = buckets.entry(start).or_default();
        for (position, column) in STAGE_COLUMNS.iter().enumerate() {
            let bytes: Vec<u8> = row
                .get(1 + position)
                .map_err(|error| crate::records::database_error("read a histogram", &error))?;
            let decoded = decode_latency(&bytes, column)?;
            merge_histogram(&mut bucket.histograms[position], &decoded);
            merge_histogram(&mut total.histograms[position], &decoded);
        }
    }
    Ok(())
}

/// The ten busiest indexes of the window (ADR-0124, decision 7).
fn read_top_indexes(
    connection: &Connection,
    filter: &StatsFilter,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Vec<StatsIndexCount>, CribleError> {
    let (clause, values) = minute_conditions(filter, from, to, true);
    let sql = format!(
        "SELECT \"index\", SUM(count) AS total FROM minute_events WHERE {clause}
         GROUP BY \"index\" ORDER BY total DESC, \"index\" ASC LIMIT {TOP_ENTRIES}"
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| crate::records::database_error("prepare the top indexes", &error))?;
    let rows = statement
        .query_map(params_from_iter(values), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| crate::records::database_error("query the top indexes", &error))?;
    let mut entries = Vec::new();
    for row in rows {
        let (index, count) =
            row.map_err(|error| crate::records::database_error("read the top indexes", &error))?;
        entries.push(StatsIndexCount {
            index,
            count: count_u64(count, "count")?,
        });
    }
    Ok(entries)
}

/// The ten busiest filter forms of the window (ADR-0124, decision 6).
fn read_top_shapes(
    connection: &Connection,
    filter: &StatsFilter,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Vec<StatsFilterShape>, CribleError> {
    let (clause, values) = minute_conditions(filter, from, to, false);
    let sql = format!(
        "SELECT shape, SUM(count) AS total FROM filter_shapes WHERE {clause}
         GROUP BY shape ORDER BY total DESC, shape ASC LIMIT {TOP_ENTRIES}"
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| crate::records::database_error("prepare the top shapes", &error))?;
    let rows = statement
        .query_map(params_from_iter(values), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| crate::records::database_error("query the top shapes", &error))?;
    let mut entries = Vec::new();
    for row in rows {
        let (shape, count) =
            row.map_err(|error| crate::records::database_error("read the top shapes", &error))?;
        entries.push(StatsFilterShape {
            shape,
            count: count_u64(count, "count")?,
        });
    }
    Ok(entries)
}

/// The last daily row of each index in the window (ADR-0124, decision 8).
fn read_index_states(
    connection: &Connection,
    filter: &StatsFilter,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Vec<StatsIndexState>, CribleError> {
    let (clause, values) = day_conditions(filter, from, to);
    let sql = format!(
        "SELECT day, \"index\", documents, live_passages, dead_passages, disk_bytes,
                generation, batches
         FROM day_indexes WHERE {clause} ORDER BY \"index\" ASC, day DESC"
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| crate::records::database_error("prepare the index states", &error))?;
    let rows = statement
        .query_map(params_from_iter(values), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })
        .map_err(|error| crate::records::database_error("query the index states", &error))?;
    let mut entries: Vec<StatsIndexState> = Vec::new();
    for row in rows {
        let (day, index, documents, live, dead, disk_bytes, generation, batches) =
            row.map_err(|error| crate::records::database_error("read the index states", &error))?;
        // The rows arrive by index then by day descending: the first of each
        // index is its last line of the window, and a missing day is never
        // filled (ADR-0118, decision 5).
        if entries.last().is_some_and(|entry| entry.index == index) {
            continue;
        }
        let live = count_u64(live, "live_passages")?;
        let dead = count_u64(dead, "dead_passages")?;
        let passages = live.saturating_add(dead);
        entries.push(StatsIndexState {
            day,
            index,
            documents: count_u64(documents, "documents")?,
            live_passages: live,
            dead_passages: dead,
            dead_ratio: if passages == 0 {
                0.0
            } else {
                dead as f64 / passages as f64
            },
            disk_bytes: count_u64(disk_bytes, "disk_bytes")?,
            generation: count_u64(generation, "generation")?,
            batches: count_u64(batches, "batches")?,
        });
    }
    Ok(entries)
}

/// The conditions of the `minute_events` and `filter_shapes` reads.
///
/// `with_operation` adds the `operation` filter; `filter_shapes` has no such
/// column, and ADR-0124, decision 2, gives `operation` only to the events.
fn minute_conditions(
    filter: &StatsFilter,
    from: Option<&str>,
    to: Option<&str>,
    with_operation: bool,
) -> (String, Vec<Value>) {
    let mut conditions = Vec::new();
    let mut values = Vec::new();
    if let Some(from) = from {
        conditions.push("minute >= ?".to_owned());
        values.push(Value::Text(from.to_owned()));
    }
    if let Some(to) = to {
        conditions.push("minute <= ?".to_owned());
        values.push(Value::Text(to.to_owned()));
    }
    if let Some(index) = &filter.index {
        conditions.push("\"index\" = ?".to_owned());
        values.push(Value::Text(index.clone()));
    }
    if with_operation && let Some(operation) = &filter.operation {
        conditions.push("operation = ?".to_owned());
        values.push(Value::Text(operation.clone()));
    }
    (clause(conditions), values)
}

/// The conditions of the `day_indexes` read, at the resolution of a day.
fn day_conditions(
    filter: &StatsFilter,
    from: Option<&str>,
    to: Option<&str>,
) -> (String, Vec<Value>) {
    let mut conditions = Vec::new();
    let mut values = Vec::new();
    if let Some(day) = from.and_then(day_of) {
        conditions.push("day >= ?".to_owned());
        values.push(Value::Text(day.to_owned()));
    }
    if let Some(day) = to.and_then(day_of) {
        conditions.push("day <= ?".to_owned());
        values.push(Value::Text(day.to_owned()));
    }
    if let Some(index) = &filter.index {
        conditions.push("\"index\" = ?".to_owned());
        values.push(Value::Text(index.clone()));
    }
    (clause(conditions), values)
}

/// The `AAAA-MM-JJ` day of a stored minute.
fn day_of(minute: &str) -> Option<&str> {
    minute.get(..10)
}

/// Joins the conditions, or yields an always-true one when there is none.
fn clause(conditions: Vec<String>) -> String {
    if conditions.is_empty() {
        "1 = 1".to_owned()
    } else {
        conditions.join(" AND ")
    }
}

/// The SQL list of `names`, quoted, for a closed list interpolated in a query.
fn quoted(names: [&str; 2]) -> String {
    names
        .iter()
        .map(|name| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use super::{
        ANALYTICS_FILE_NAME, AnalyticsStore, DayIndex, FORMAT_VERSION, LATENCY_BUCKETS,
        MinuteEvent, POOL_BUCKETS, ShapeCount, latency_bucket, latency_bucket_upper_ms,
        latency_percentile, merge_histogram, pool_bucket, pool_bucket_upper, pool_percentile,
        read_minute_events,
    };

    /// A store on a fresh temporary file, with the path kept alive.
    fn store() -> (tempfile::TempDir, std::path::PathBuf, AnalyticsStore) {
        let directory = tempfile::tempdir().expect("a temporary directory is creatable");
        let path = directory.path().join(ANALYTICS_FILE_NAME);
        let store = AnalyticsStore::open_or_create(&path).expect("the file opens");
        (directory, path, store)
    }

    /// A search row with `count` events and one sample per histogram.
    fn event(bucket: usize) -> MinuteEvent {
        let mut event = MinuteEvent::empty("alpha", "search", "ok");
        event.client = "reader".to_owned();
        event.mode = "dense".to_owned();
        event.dense_strategy = "exact".to_owned();
        event.count = 1;
        event.add_latency(8, bucket);
        event.add_pool(3);
        event
    }

    #[test]
    fn an_unknown_format_version_is_refused() {
        // INV-15 through ADR-0117, decision 1: the file of another build is
        // refused, never read half way.
        let directory = tempfile::tempdir().expect("a temporary directory is creatable");
        let path = directory.path().join(ANALYTICS_FILE_NAME);
        let store = AnalyticsStore::open_or_create(&path).expect("the file opens");
        drop(store);
        let connection = rusqlite::Connection::open(&path).expect("the file is open");
        connection
            .execute("UPDATE meta SET format_version = ?1", [FORMAT_VERSION + 1])
            .expect("the version is writable");
        drop(connection);
        let error = match AnalyticsStore::open_or_create(&path) {
            Ok(_) => panic!("the version is refused"),
            Err(error) => error,
        };
        assert!(
            error.message().contains("format_version"),
            "the refusal names format_version: {}",
            error.message()
        );
    }

    #[test]
    fn histograms_are_little_endian_u32_blobs() {
        // ADR-0117, decision 1: counters are little-endian `u32` without a
        // header, 256 bytes for a latency histogram and 132 for a pool one.
        let buckets: Vec<u32> = (0..LATENCY_BUCKETS as u32).collect();
        let bytes = super::encode_latency(&buckets);
        assert_eq!(bytes.len(), 256, "64 counters of 4 bytes");
        assert_eq!(
            &bytes[..8],
            &[0, 0, 0, 0, 1, 0, 0, 0],
            "the first two counters are little-endian"
        );
        assert_eq!(
            super::decode_latency(&bytes, "latency_total").expect("the blob decodes"),
            buckets
        );
        let pool: Vec<u32> = (0..POOL_BUCKETS as u32).collect();
        let bytes = super::encode_pool(&pool);
        assert_eq!(bytes.len(), 132, "33 counters of 4 bytes");
        assert_eq!(
            super::decode_pool(&bytes, "pool").expect("the blob decodes"),
            pool
        );
    }

    #[test]
    fn a_blob_of_another_length_is_refused() {
        // ADR-0117, decision 1: a BLOB of another length is refused, never
        // truncated nor completed in silence.
        let error =
            super::decode_latency(&[0; 100], "latency_dense").expect_err("a short blob is refused");
        assert!(
            error.message().contains("latency_dense"),
            "the refusal names the column: {}",
            error.message()
        );
    }

    #[test]
    fn the_latency_buckets_follow_the_documented_bounds() {
        // ADR-0117, decision 2: bucket 0 holds zero and everything below 0.05 ms,
        // bucket 1 starts at 0.05 ms, and bucket 63 holds the overflow.
        assert_eq!(latency_bucket(Duration::ZERO), 0);
        assert_eq!(latency_bucket(Duration::from_micros(10)), 0);
        assert_eq!(latency_bucket(Duration::from_micros(50)), 0);
        assert_eq!(latency_bucket(Duration::from_micros(63)), 1);
        assert_eq!(latency_bucket(Duration::from_hours(1)), 63);
        assert_eq!(latency_bucket(Duration::MAX), 63);
    }

    #[test]
    fn the_pool_buckets_follow_the_documented_bounds() {
        // ADR-0117, decision 3: 0 has its own bucket, 1 is in bucket 1, 2 and 3
        // in bucket 2, and `2^31` and above in bucket 32.
        assert_eq!(pool_bucket(0), 0);
        assert_eq!(pool_bucket(1), 1);
        assert_eq!(pool_bucket(2), 2);
        assert_eq!(pool_bucket(3), 2);
        assert_eq!(pool_bucket(4), 3);
        assert_eq!(pool_bucket((1 << 31) - 1), 31);
        assert_eq!(pool_bucket(1 << 31), 32);
        assert_eq!(pool_bucket(u32::MAX), 32);
    }

    #[test]
    fn the_last_pool_bucket_is_open_and_reports_its_lower_bound() {
        // ADR-0117, decision 3 reads the last bucket as "2^31 and beyond", and
        // decision 4 gives it `2^31`: it is open at the right, so the bound read
        // from its row is its lower bound, never the `2^32` the `2^i` rule of the
        // other buckets would give.
        assert_eq!(pool_bucket_upper(0), 0);
        assert_eq!(pool_bucket_upper(1), 2);
        assert_eq!(pool_bucket_upper(30), 1 << 30);
        assert_eq!(pool_bucket_upper(31), 1 << 31);
        assert_eq!(pool_bucket_upper(32), 1 << 31);
        assert_eq!(pool_bucket_upper(POOL_BUCKETS - 1), 1 << 31);
    }

    #[test]
    fn the_percentile_is_the_upper_bound_of_the_nearest_rank() {
        // ADR-0117, decision 4: `r = ceil(p × n / 100)`, at least 1, and the
        // value is the upper bound of the bucket that contains it.
        let mut buckets = vec![0; LATENCY_BUCKETS];
        buckets[10] = 95;
        buckets[20] = 5;
        let p95 = latency_percentile(&buckets, 95).expect("a non-empty histogram");
        assert!(
            (p95 - latency_bucket_upper_ms(10)).abs() < f64::EPSILON,
            "p95 stays in bucket 10: {p95}"
        );
        let p99 = latency_percentile(&buckets, 99).expect("a non-empty histogram");
        assert!(
            (p99 - latency_bucket_upper_ms(20)).abs() < f64::EPSILON,
            "p99 reaches bucket 20: {p99}"
        );
        assert!(latency_percentile(&vec![0; LATENCY_BUCKETS], 95).is_none());
        let mut pool = vec![0; POOL_BUCKETS];
        pool[0] = 1;
        assert_eq!(pool_percentile(&pool, 50), Some(0));
        pool[5] = 1;
        assert_eq!(pool_percentile(&pool, 95), Some(pool_bucket_upper(5)));
    }

    #[test]
    fn merged_histograms_give_the_percentile_of_the_raw_events() {
        // Criterion of `docs/phases/P7-observabilite.md`: the p95 of two merged
        // minutes equals the p95 of the raw events, at one bucket's resolution.
        let first = [
            Duration::from_millis(1),
            Duration::from_millis(2),
            Duration::from_millis(3),
        ];
        let second = [
            Duration::from_millis(4),
            Duration::from_millis(5),
            Duration::from_millis(40),
            Duration::from_millis(60),
        ];
        let mut raw = vec![0; LATENCY_BUCKETS];
        for duration in first.iter().chain(second.iter()) {
            raw[latency_bucket(*duration)] += 1;
        }
        let mut left = vec![0; LATENCY_BUCKETS];
        for duration in first {
            left[latency_bucket(duration)] += 1;
        }
        let mut right = vec![0; LATENCY_BUCKETS];
        for duration in second {
            right[latency_bucket(duration)] += 1;
        }
        merge_histogram(&mut left, &right);
        assert_eq!(left, raw, "the merged buckets are the raw buckets");
        assert_eq!(
            latency_percentile(&left, 95),
            latency_percentile(&raw, 95),
            "the merged p95 is the raw p95"
        );
    }

    #[test]
    fn a_flush_merges_the_rows_of_one_minute() {
        // ADR-0118, decision 3: a flush is one transaction, and a row of the same
        // key gains the counts and the buckets of the flush.
        let (_directory, path, store) = store();
        let first = event(3);
        store
            .write(std::slice::from_ref(&first), &[], &[])
            .expect("the first flush writes");
        let mut second = event(3);
        second.zero_results = 1;
        store
            .write(&[second], &[], &[])
            .expect("the second flush writes");
        let rows = read_minute_events(&path).expect("the file is readable");
        assert_eq!(
            rows.len(),
            1,
            "one row for one key and one minute: {rows:?}"
        );
        assert_eq!(rows[0].count, 2, "the counts are added");
        assert_eq!(rows[0].zero_results, 1);
        assert_eq!(
            rows[0].latency_total[3], 2,
            "the buckets are added sample by sample"
        );
    }

    #[test]
    fn the_purge_drops_the_rows_older_than_the_retention() {
        // ADR-0118, decision 4: the three tables are purged together, and a row
        // of the retention stays.
        let (_directory, path, store) = store();
        let recent = event(1);
        let shape = ShapeCount {
            index: "alpha".to_owned(),
            shape: "fonds:eq".to_owned(),
            count: 1,
        };
        let day = DayIndex {
            index: "alpha".to_owned(),
            documents: 1,
            ..DayIndex::default()
        };
        store
            .write(&[recent], std::slice::from_ref(&shape), &[day])
            .expect("the flush writes");
        let connection = rusqlite::Connection::open(&path).expect("the file is open");
        connection
            .execute(
                "UPDATE minute_events SET minute = strftime('%Y-%m-%dT%H:%M:00Z', 'now', '-100 days')",
                [],
            )
            .expect("the minute is aged");
        connection
            .execute(
                "UPDATE filter_shapes SET minute = strftime('%Y-%m-%dT%H:%M:00Z', 'now', '-100 days')",
                [],
            )
            .expect("the shape is aged");
        connection
            .execute(
                "UPDATE day_indexes SET day = strftime('%Y-%m-%d', 'now', '-100 days')",
                [],
            )
            .expect("the day is aged");
        drop(connection);
        let purged = store.purge(90).expect("the purge runs");
        assert_eq!(purged, 3, "one row of each purged table");
        assert!(
            read_minute_events(&path)
                .expect("the file is readable")
                .is_empty(),
            "the expired minute is gone"
        );
    }

    #[test]
    fn a_daily_row_is_written_once_per_day_and_index() {
        // ADR-0118, decision 5: the first flush of the day writes the row, the
        // next ones leave it alone.
        let (_directory, path, store) = store();
        assert_eq!(
            store
                .missing_days(&["alpha".to_owned(), "beta".to_owned()])
                .expect("the missing days are readable"),
            vec!["alpha".to_owned(), "beta".to_owned()],
            "no index has a row yet"
        );
        let rows = [
            DayIndex {
                index: "alpha".to_owned(),
                documents: 2,
                live_passages: 3,
                dead_passages: 4,
                disk_bytes: 5,
                generation: 6,
                batches: 7,
                ..DayIndex::default()
            },
            DayIndex {
                index: "beta".to_owned(),
                documents: 1,
                ..DayIndex::default()
            },
        ];
        store.write(&[], &[], &rows).expect("the daily rows write");
        assert!(
            store
                .missing_days(&["alpha".to_owned(), "beta".to_owned()])
                .expect("the missing days are readable")
                .is_empty(),
            "both indexes have their row of the day"
        );
        let connection = rusqlite::Connection::open(&path).expect("the file is open");
        let stored: (i64, i64, i64, i64, i64, i64) = connection
            .query_row(
                "SELECT documents, live_passages, dead_passages, disk_bytes, generation, batches
                 FROM day_indexes WHERE \"index\" = 'alpha'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .expect("the daily row is stored");
        assert_eq!(stored, (2, 3, 4, 5, 6, 7), "every column is stored");
    }

    #[test]
    fn an_empty_flush_writes_nothing() {
        // A flush with nothing to write must not open a transaction: the writer
        // runs every ten seconds on an idle instance.
        let (_directory, path, store) = store();
        store
            .write(&[], &[], &[])
            .expect("the empty flush is a no-op");
        assert!(Path::new(&path).exists(), "the file was created at open");
    }
}
