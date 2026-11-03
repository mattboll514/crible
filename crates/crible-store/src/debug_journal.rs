// SPDX-License-Identifier: Apache-2.0

//! `debug.sqlite`: the journal of the debug requests of an instance.
//!
//! References: ADR-0121 (file, schema, payload, protections) and ADR-0122
//! (activation, capture, channel, flush, purge, route). The file lives at the
//! root of `<data>`, next to `instance.json`, `analytics.sqlite` and
//! `jobs.sqlite` (`docs/02-architecture.md`). It is opened and written here, in
//! `crible-store`, which already carries `rusqlite` (ADR-0038), so
//! `crible-server` keeps no edge to that crate (ADR-0103, ADR-0117,
//! ADR-0121 decision 1).
//!
//! # What the file holds
//!
//! One table, `queries`, at `meta.format_version` 1 (ADR-0121, decision 3). A
//! row carries the columns of the route: the request identifier, the index
//! (`NULL` for every multi-index request), the name of the key, the mode, the
//! status, the total duration and the payload as JSON text. The payload is
//! built and serialised by `crible-server`; this module never reads its shape
//! (ADR-0121, decision 5).
//!
//! # Clock
//!
//! The code owns no clock: `created_at` is read by SQLite at the insertion, as
//! `jobs.sqlite` and `analytics.sqlite` already do (ADR-0103, ADR-0118
//! decision 3, ADR-0121 decision 3). A row is therefore attached to the flush
//! that writes it, up to one flush interval away from the request.
//!
//! # Protections
//!
//! The journal **contains** the text and the filter by design; it is disabled
//! by default, read by `admin` only and kept for a short life (ADR-0121,
//! decision 6). This module stores what it is given and never writes a log
//! line: no content ever leaves through a log (ADR-0122, decision 10).
//!
//! # Size cap
//!
//! Two independent caps bound the file: [`JournalCap::max_rows`] and
//! [`JournalCap::max_bytes`] (ADR-0127, decision 1). A [`DebugJournalStore::flush`]
//! inserts its rows, then deletes the oldest ones — `created_at ASC, id ASC` —
//! until the journal is back under both caps (decision 3). The comparison uses
//! the **used** size of the file, `(page_count - freelist_count) × page_size`
//! (decision 2). The file is created with `auto_vacuum = INCREMENTAL`, and
//! [`DebugJournalStore::reclaim`] gives a bounded budget of pages back to the
//! system, outside any transaction (decision 5). The schema and its
//! `format_version` do not change: no new member is persisted (INV-15).

use std::path::Path;
use std::sync::Mutex;

use crible_core::error::{CribleError, ErrorCode};
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OpenFlags, params, params_from_iter};

use crate::sqlite::{check_format_version, configure_busy_timeout, enable_wal};

/// Name of the debug journal file at the root of `<data>` (ADR-0121,
/// decision 1).
pub const DEBUG_JOURNAL_FILE_NAME: &str = "debug.sqlite";

/// Format of the file this build reads and writes (ADR-0121, decision 2).
pub const FORMAT_VERSION: u32 = 1;

/// Pages one [`DebugJournalStore::reclaim`] gives back at most (ADR-0127,
/// decision 5).
///
/// 256 pages are 1 MiB with a page of 4 KiB: a flush is never monopolised by
/// the recovery, and a large file comes back down over several flushes.
pub const RECLAIM_PAGES: i64 = 256;

/// Rows deleted per statement when the row cap drives the trim (ADR-0127,
/// decision 3).
///
/// A batch keeps the number of statements bounded on a big file; the byte cap
/// deletes one row at a time, because a batch could overshoot the cap.
const TRIM_BATCH_ROWS: usize = 1_000;

/// The two independent caps of `debug.sqlite` (ADR-0127, decision 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JournalCap {
    /// Largest number of rows kept.
    pub max_rows: usize,
    /// Largest **used** size kept, in bytes (ADR-0127, decision 2).
    pub max_bytes: usize,
}

impl JournalCap {
    /// Whether `measure` is above one of the two caps.
    fn exceeded_by(&self, measure: &JournalMeasure) -> bool {
        measure.rows > self.max_rows as u64 || measure.bytes > self.max_bytes as u64
    }
}

/// What one measurement of `debug.sqlite` says (ADR-0127, decisions 2 and 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JournalMeasure {
    /// Number of rows of `queries`.
    pub rows: u64,
    /// Used size, `(page_count - freelist_count) × page_size`.
    pub bytes: u64,
    /// Physical size of the file, `page_count × page_size`, WAL excluded.
    pub file_bytes: u64,
}

/// What one [`DebugJournalStore::flush`] did (ADR-0127, decision 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlushReport {
    /// Rows the flush inserted.
    pub inserted: usize,
    /// Rows the cap deleted, purge excluded (ADR-0127, decision 7).
    pub trimmed: u64,
    /// Whether the control found the journal above a cap before trimming.
    pub capped: bool,
    /// Measurement taken once the transaction committed.
    pub measure: JournalMeasure,
}

/// One row of `queries`, as a writer deposits it.
///
/// [`DebugJournalStore::flush`] takes rows of this type; [`read_queries`]
/// returns [`DebugQuery`], the same row with the `id` SQLite assigned. A writer
/// leaves [`DebugQueryRow::created_at`] empty and SQLite fills it at the
/// insertion (ADR-0121, decision 3), a reader receives the stored value.
#[derive(Debug, Clone, PartialEq)]
pub struct DebugQueryRow {
    /// Timestamp of the flush, RFC 3339 UTC with milliseconds; empty on input.
    pub created_at: String,
    /// Request identifier of ADR-0098.
    pub request_id: String,
    /// Name of the index, `None` for every multi-index request.
    pub index: Option<String>,
    /// Name of the API key, never the key nor its digest (ADR-0100).
    pub client: String,
    /// `dense`, `lexical`, `hybrid`, or the empty string when the mode is
    /// unknown (ADR-0117, decision 1).
    pub mode: String,
    /// `ok` or an error code of the API (`docs/05-api.md`).
    pub status: String,
    /// Duration of the request, `None` when `fetch` did not finish.
    pub total_ms: Option<f64>,
    /// The payload of ADR-0121, decision 4, as JSON text.
    pub payload: String,
}

/// One stored row of `queries`, with its `id`.
#[derive(Debug, Clone, PartialEq)]
pub struct DebugQuery {
    /// Primary key, the cursor of the keyset pagination (ADR-0122, decision 9).
    pub id: i64,
    /// Timestamp written by SQLite at the flush, RFC 3339 UTC.
    pub created_at: String,
    /// Request identifier of ADR-0098.
    pub request_id: String,
    /// Name of the index, `None` for a multi-index row.
    pub index: Option<String>,
    /// Name of the API key (ADR-0100).
    pub client: String,
    /// Mode of the request, empty when unknown.
    pub mode: String,
    /// `ok` or an error code of the API.
    pub status: String,
    /// Total duration of the request, `None` when `fetch` did not finish.
    pub total_ms: Option<f64>,
    /// Payload JSON text, parsed by the route before it is rendered.
    pub payload: String,
}

/// Filters of the `GET /v1/debug/queries` route (ADR-0122, decision 9).
///
/// Every member is `None` when the caller did not ask for it. The `index` is
/// matched on the column for a simple row and by membership in
/// `payload.targets[]` for a multi-index row (ADR-0122, decision 4).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JournalFilter {
    /// Exact name of the index, or a target of a multi-index row.
    pub index: Option<String>,
    /// Lower bound, included, of `total_ms`; a `NULL` total is excluded.
    pub min_total_ms: Option<f64>,
    /// Lower bound, included, of `created_at`, RFC 3339 UTC.
    pub from: Option<String>,
    /// Upper bound, included, of `created_at`, RFC 3339 UTC.
    pub to: Option<String>,
    /// Number of rows of the page, 1 to 1 000.
    pub limit: usize,
    /// Keyset cursor: only the rows of a strictly lower `id`.
    pub before_id: Option<i64>,
}

/// The debug journal file of an instance: one connection, one writer.
pub struct DebugJournalStore {
    connection: Mutex<Connection>,
}

impl DebugJournalStore {
    /// Opens `<data>/debug.sqlite`, creating it with its schema and its
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
        .map_err(|error| {
            crate::records::database_error("open the debug journal database", &error)
        })?;
        configure_busy_timeout(&connection)?;
        if fresh {
            // ADR-0127, decision 5: the mode only takes on a new file (or
            // through a `VACUUM`), it must be set before the first table is
            // created, and SQLite ignores it once the file is in WAL.
            enable_incremental_auto_vacuum(&connection)?;
        }
        enable_wal(&connection, "debug journal")?;
        if fresh {
            create_schema(&mut connection)?;
        } else {
            check_format_version(&connection, FORMAT_VERSION, "debug journal")?;
        }
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Writes one flush in a single transaction, then applies the cap.
    ///
    /// Every row is attached to the instant SQLite reads at the insertion
    /// (ADR-0121, decision 3); either every row lands or none does
    /// (ADR-0122, decision 7). The cap is checked on every flush, **empty
    /// flush included**, so the open control and a file left above the cap are
    /// both handled by the same code (ADR-0127, decision 4). The rows of the
    /// flush are inserted first: a received row is never refused by the trim of
    /// the same flush (decision 3).
    ///
    /// # Errors
    ///
    /// The SQLite failure.
    pub fn flush(
        &self,
        rows: &[DebugQueryRow],
        cap: JournalCap,
    ) -> Result<FlushReport, CribleError> {
        self.run_flush(rows, cap, &mut || {})
    }

    /// Same as [`Self::flush`], and calls `observer` once the rows are inserted
    /// and before the first deletion of the trim.
    ///
    /// Test hook: it only exists with the `failpoints` cargo feature, never in a
    /// release build. The observer runs on the caller's thread, inside the
    /// transaction, so a test that blocks in it holds the trim at that exact
    /// point (ADR-0127, decision 3).
    ///
    /// # Errors
    ///
    /// The SQLite failure.
    #[cfg(feature = "failpoints")]
    pub fn flush_with_observer<F>(
        &self,
        rows: &[DebugQueryRow],
        cap: JournalCap,
        observer: F,
    ) -> Result<FlushReport, CribleError>
    where
        F: FnMut(),
    {
        let mut observer = observer;
        self.run_flush(rows, cap, &mut observer)
    }

    /// Writes `rows` and applies `cap` in one transaction.
    ///
    /// A flush with nothing to insert still checks the cap (ADR-0127,
    /// decision 4), but it only opens the write transaction when the journal is
    /// above a bound: a quiet flush of a journal under its caps costs one read
    /// and no write. The measurement of the report is the one the trim ended on,
    /// never a second count.
    fn run_flush(
        &self,
        rows: &[DebugQueryRow],
        cap: JournalCap,
        observer: &mut dyn FnMut(),
    ) -> Result<FlushReport, CribleError> {
        self.with_mut(|connection| {
            if rows.is_empty() {
                let measure = measure_connection(connection)?;
                if !cap.exceeded_by(&measure) {
                    return Ok(FlushReport {
                        inserted: 0,
                        trimmed: 0,
                        capped: false,
                        measure,
                    });
                }
            }
            let transaction = connection.transaction().map_err(|error| {
                crate::records::database_error("open the debug journal flush", &error)
            })?;
            {
                let mut statement = transaction
                    .prepare(
                        "INSERT INTO queries
                           (created_at, request_id, \"index\", client, mode, status, total_ms, payload)
                         VALUES
                           (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    )
                    .map_err(|error| {
                        crate::records::database_error("prepare the debug journal insert", &error)
                    })?;
                for row in rows {
                    statement
                        .execute(params![
                            row.request_id,
                            row.index,
                            row.client,
                            row.mode,
                            row.status,
                            row.total_ms,
                            row.payload,
                        ])
                        .map_err(|error| {
                            crate::records::database_error("insert a debug journal row", &error)
                        })?;
                }
            }
            let (trimmed, capped, measure) = trim(&transaction, cap, observer)?;
            transaction.commit().map_err(|error| {
                crate::records::database_error("commit the debug journal flush", &error)
            })?;
            Ok(FlushReport {
                inserted: rows.len(),
                trimmed,
                capped,
                measure,
            })
        })
    }

    /// Deletes the rows older than `ttl_days` days and returns their number
    /// (ADR-0122, decision 8).
    ///
    /// The cut-off is read by SQLite, by the same clock as `created_at`: no
    /// calendar clock enters the code.
    ///
    /// # Errors
    ///
    /// The SQLite failure.
    pub fn purge(&self, ttl_days: usize) -> Result<usize, CribleError> {
        let offset = format!("-{ttl_days} days");
        self.with_mut(|connection| {
            let transaction = connection.transaction().map_err(|error| {
                crate::records::database_error("open the debug journal purge", &error)
            })?;
            let purged = transaction
                .execute(
                    "DELETE FROM queries
                     WHERE created_at < strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1)",
                    params![offset],
                )
                .map_err(|error| {
                    crate::records::database_error("purge the debug journal", &error)
                })?;
            transaction.commit().map_err(|error| {
                crate::records::database_error("commit the debug journal purge", &error)
            })?;
            Ok(purged)
        })
    }

    /// Gives back at most [`RECLAIM_PAGES`] free pages to the system and returns
    /// the number of pages it asked for (ADR-0127, decision 5).
    ///
    /// It runs **outside** any transaction: `incremental_vacuum` takes the write
    /// lock for itself. On a file created without `auto_vacuum = INCREMENTAL`,
    /// SQLite accepts the pragma and does nothing; a full `VACUUM` is never run
    /// by the server (decision 5).
    ///
    /// # Errors
    ///
    /// The SQLite failure.
    pub fn reclaim(&self) -> Result<u64, CribleError> {
        self.with_mut(|connection| {
            let freelist: i64 = pragma(connection, "freelist_count")?;
            let pages = freelist.clamp(0, RECLAIM_PAGES);
            if pages == 0 {
                return Ok(0);
            }
            // `incremental_vacuum` yields one row per page it gives back, and
            // SQLite vacates a page per step: the rows must be drained, not
            // stepped once (`sqlite3_exec` and `execute_batch` stop after the
            // first page).
            let mut statement = connection
                .prepare(&format!("PRAGMA incremental_vacuum({pages})"))
                .map_err(|error| {
                    crate::records::database_error("prepare the debug journal reclaim", &error)
                })?;
            let mut rows = statement.query([]).map_err(|error| {
                crate::records::database_error("reclaim the debug journal pages", &error)
            })?;
            let mut reclaimed = 0u64;
            while rows
                .next()
                .map_err(|error| {
                    crate::records::database_error("reclaim the debug journal pages", &error)
                })?
                .is_some()
            {
                reclaimed += 1;
            }
            Ok(reclaimed)
        })
    }

    /// Measures the file: rows, used size and physical size (ADR-0127,
    /// decision 2).
    ///
    /// # Errors
    ///
    /// The SQLite failure.
    pub fn measure(&self) -> Result<JournalMeasure, CribleError> {
        self.with_mut(|connection| measure_connection(connection))
    }

    /// Inserts `row` with an explicit `created_at`, for the purge test only
    /// (ADR-0122, decision 8).
    ///
    /// The production path never dates a row: SQLite does (ADR-0121,
    /// decision 3). The hook exists only with the `failpoints` feature, never in
    /// a release build, so the public API of this module keeps no test-only
    /// method.
    ///
    /// # Errors
    ///
    /// The SQLite failure.
    #[cfg(feature = "failpoints")]
    pub fn insert_dated_for_tests(
        &self,
        row: &DebugQueryRow,
        created_at: &str,
    ) -> Result<i64, CribleError> {
        self.with_mut(|connection| {
            connection
                .execute(
                    "INSERT INTO queries
                       (created_at, request_id, \"index\", client, mode, status, total_ms, payload)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        created_at,
                        row.request_id,
                        row.index,
                        row.client,
                        row.mode,
                        row.status,
                        row.total_ms,
                        row.payload,
                    ],
                )
                .map_err(|error| {
                    crate::records::database_error("insert a dated debug journal row", &error)
                })?;
            Ok(connection.last_insert_rowid())
        })
    }

    /// Runs `action` on the mutable connection under its lock.
    fn with_mut<T>(
        &self,
        action: impl FnOnce(&mut Connection) -> Result<T, CribleError>,
    ) -> Result<T, CribleError> {
        let mut connection = self.connection.lock().map_err(|_| {
            CribleError::new(
                ErrorCode::Internal,
                "the debug journal database is poisoned",
            )
        })?;
        action(&mut connection)
    }
}

/// Reads every row of `queries`, ordered by `id`.
///
/// It opens the file **read-only**, so a test or a tool never competes with the
/// writer of the instance, and refuses a version this build does not read.
///
/// # Errors
///
/// [`ErrorCode::InvalidRequest`] for a file of another format or a row that
/// this build cannot read, and the SQLite failure otherwise.
pub fn read_queries(path: &Path) -> Result<Vec<DebugQuery>, CribleError> {
    let connection = open_read_only(path)?;
    let mut statement = connection
        .prepare(
            "SELECT id, created_at, request_id, \"index\", client, mode, status, total_ms, payload
             FROM queries
             ORDER BY id",
        )
        .map_err(|error| {
            crate::records::database_error("prepare the debug journal query", &error)
        })?;
    let rows = statement
        .query_map([], read_query_row)
        .map_err(|error| crate::records::database_error("query the debug journal", &error))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| crate::records::database_error("read the debug journal", &error))?;
    rows.into_iter().map(decode_query_row).collect()
}

/// Reads one page of `queries` for `GET /v1/debug/queries` (ADR-0122,
/// decision 9).
///
/// The conditions of the columns are applied by SQLite, `index` included: a
/// simple row of another index is excluded there, through `queries_index`
/// (ADR-0122, decision 4), and a multi-index row is kept by the `IS NULL` leg.
/// The rows a multi-index search wrote are then retained by membership in
/// `payload.targets[]`, which SQLite cannot read; when such a row is skipped,
/// the page is completed by the next block of the keyset (`id <` the last row
/// read), so `limit` stays the number of rows of the page.
///
/// # Errors
///
/// [`ErrorCode::InvalidRequest`] for a file of another format or a row that
/// this build cannot read, and the SQLite failure otherwise.
pub fn read_page(path: &Path, filter: &JournalFilter) -> Result<Vec<DebugQuery>, CribleError> {
    let connection = open_read_only(path)?;
    let mut kept: Vec<DebugQuery> = Vec::with_capacity(filter.limit);
    let mut cursor = filter.before_id;
    while kept.len() < filter.limit {
        let remaining = filter.limit - kept.len();
        let block = read_block(&connection, filter, cursor, remaining)?;
        let Some(last) = block.last().map(|row| row.id) else {
            break;
        };
        // The keyset always moves down: `id < cursor` is strict, so a block
        // that is skipped entirely by the membership filter still advances.
        cursor = Some(last);
        for row in block {
            if names_target(&row, filter.index.as_deref()) {
                kept.push(row);
                if kept.len() == filter.limit {
                    break;
                }
            }
        }
    }
    Ok(kept)
}

/// Reads at most `limit` rows of the keyset below `before`, newest first.
fn read_block(
    connection: &Connection,
    filter: &JournalFilter,
    before: Option<i64>,
    limit: usize,
) -> Result<Vec<DebugQuery>, CribleError> {
    let mut sql = String::from(
        "SELECT id, created_at, request_id, \"index\", client, mode, status, total_ms, payload
         FROM queries WHERE 1 = 1",
    );
    let mut arguments: Vec<SqlValue> = Vec::new();
    if let Some(name) = &filter.index {
        // A simple row of another index is settled here, by `queries_index`
        // (ADR-0122, decisions 4 and 9); a multi-index row is read and checked
        // by `names_target`.
        sql.push_str(" AND (\"index\" = ? OR \"index\" IS NULL)");
        arguments.push(SqlValue::Text(name.clone()));
    }
    if let Some(minimum) = filter.min_total_ms {
        sql.push_str(" AND total_ms >= ?");
        arguments.push(SqlValue::Real(minimum));
    }
    if let Some(from) = &filter.from {
        sql.push_str(" AND created_at >= ?");
        arguments.push(SqlValue::Text(from.clone()));
    }
    if let Some(to) = &filter.to {
        sql.push_str(" AND created_at <= ?");
        arguments.push(SqlValue::Text(to.clone()));
    }
    if let Some(before) = before {
        sql.push_str(" AND id < ?");
        arguments.push(SqlValue::Integer(before));
    }
    sql.push_str(" ORDER BY created_at DESC, id DESC LIMIT ?");
    arguments.push(SqlValue::Integer(i64::try_from(limit).unwrap_or(i64::MAX)));

    let mut statement = connection.prepare(&sql).map_err(|error| {
        crate::records::database_error("prepare the debug journal page query", &error)
    })?;
    statement
        .query_map(params_from_iter(arguments), read_query_row)
        .map_err(|error| crate::records::database_error("query a debug journal page", &error))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| crate::records::database_error("read a debug journal page", &error))?
        .into_iter()
        .map(decode_query_row)
        .collect()
}

/// Whether `row` is a simple row of `name`, or a multi-index row that targets
/// it (ADR-0122, decisions 4 and 9); `None` retains every row.
fn names_target(row: &DebugQuery, name: Option<&str>) -> bool {
    let Some(name) = name else {
        return true;
    };
    if row.index.as_deref() == Some(name) {
        return true;
    }
    if row.index.is_some() {
        return false;
    }
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(&row.payload) else {
        return false;
    };
    payload
        .get("targets")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|targets| {
            targets
                .iter()
                .any(|target| target.get("index").and_then(serde_json::Value::as_str) == Some(name))
        })
}

/// Opens the journal read-only and checks its format (INV-15).
fn open_read_only(path: &Path) -> Result<Connection, CribleError> {
    let connection =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|error| {
            crate::records::database_error("open the debug journal database", &error)
        })?;
    configure_busy_timeout(&connection)?;
    check_format_version(&connection, FORMAT_VERSION, "debug journal")?;
    Ok(connection)
}

/// One row of `queries` as SQLite returned it, before the conversion.
type RawQuery = (
    i64,
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    Option<f64>,
    String,
);

/// Reads one row of `queries` from a statement.
fn read_query_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawQuery> {
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
    ))
}

/// Converts a raw row into [`DebugQuery`], without touching the payload text.
fn decode_query_row(row: RawQuery) -> Result<DebugQuery, CribleError> {
    let (id, created_at, request_id, index, client, mode, status, total_ms, payload) = row;
    Ok(DebugQuery {
        id,
        created_at,
        request_id,
        index,
        client,
        mode,
        status,
        total_ms,
        payload,
    })
}

/// Whether `path` does not hold a database yet: missing file, or empty file
/// left by a failed creation.
fn is_new_database(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(metadata) => metadata.len() == 0,
        Err(_) => true,
    }
}

/// Enables `auto_vacuum = INCREMENTAL` on a fresh file (ADR-0127, decision 5).
///
/// The pragma must be set before the first table is created; a file created
/// without it keeps its high-water mark, and the operator reclaims it with an
/// offline `VACUUM` (`docs/06-observabilite.md`).
fn enable_incremental_auto_vacuum(connection: &Connection) -> Result<(), CribleError> {
    connection
        .execute_batch("PRAGMA auto_vacuum = INCREMENTAL")
        .map_err(|error| {
            crate::records::database_error(
                "enable incremental auto_vacuum on the debug journal",
                &error,
            )
        })
}

/// Deletes the oldest rows, `created_at ASC, id ASC`, until the journal is back
/// under **both** caps, and reports `(trimmed, capped, measure)` (ADR-0127,
/// decision 3).
///
/// The caller has already inserted the rows of the flush, inside the same
/// transaction; `observer` runs once, before the first deletion, so a test holds
/// the trim at that exact point. The row cap deletes in bounded batches; the
/// byte cap deletes one row at a time, because a batch could overshoot the cap
/// and delete a row the cap did not require. The measure returned is the last
/// one the loop took, so the caller neither counts nor measures twice.
fn trim(
    connection: &Connection,
    cap: JournalCap,
    observer: &mut dyn FnMut(),
) -> Result<(u64, bool, JournalMeasure), CribleError> {
    let page_size: i64 = pragma(connection, "page_size")?;
    let page_size = u64::try_from(page_size).map_err(|_| {
        CribleError::new(
            ErrorCode::Internal,
            format!("the debug journal reports a negative page size: {page_size}"),
        )
    })?;
    let mut measure = measure_with_rows(connection, page_size, count_rows(connection)?)?;
    if !cap.exceeded_by(&measure) {
        return Ok((0, false, measure));
    }
    observer();
    let mut trimmed = 0u64;
    while cap.exceeded_by(&measure) && measure.rows > 0 {
        let batch = if measure.rows > cap.max_rows as u64 {
            let excess = measure.rows - cap.max_rows as u64;
            usize::try_from(excess.min(TRIM_BATCH_ROWS as u64)).unwrap_or(TRIM_BATCH_ROWS)
        } else {
            1
        };
        let deleted = delete_oldest(connection, batch)?;
        if deleted == 0 {
            break;
        }
        trimmed += deleted as u64;
        measure = measure_with_rows(connection, page_size, measure.rows - deleted as u64)?;
    }
    Ok((trimmed, true, measure))
}

/// Deletes at most `rows` of the oldest rows, `created_at ASC, id ASC`.
///
/// The subquery walks the index on `created_at` and `id` breaks a tie, so no
/// full scan is needed (ADR-0127, decision 3).
fn delete_oldest(connection: &Connection, rows: usize) -> Result<usize, CribleError> {
    connection
        .execute(
            "DELETE FROM queries WHERE id IN (
                 SELECT id FROM queries ORDER BY created_at ASC, id ASC LIMIT ?1)",
            params![i64::try_from(rows).unwrap_or(i64::MAX)],
        )
        .map_err(|error| crate::records::database_error("trim the debug journal", &error))
}

/// Number of rows of `queries`.
fn count_rows(connection: &Connection) -> Result<u64, CribleError> {
    let rows: i64 = connection
        .query_row("SELECT count(*) FROM queries", [], |row| row.get(0))
        .map_err(|error| crate::records::database_error("count the debug journal rows", &error))?;
    u64::try_from(rows).map_err(|_| {
        CribleError::new(
            ErrorCode::Internal,
            format!("the debug journal reports a negative row count: {rows}"),
        )
    })
}

/// Reads one integer pragma of the connection.
fn pragma(connection: &Connection, name: &str) -> Result<i64, CribleError> {
    connection
        .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
        .map_err(|error| {
            crate::records::database_error(&format!("read the debug journal {name}"), &error)
        })
}

/// Measures the file and counts its rows.
fn measure_connection(connection: &Connection) -> Result<JournalMeasure, CribleError> {
    let page_size: i64 = pragma(connection, "page_size")?;
    let page_size = u64::try_from(page_size).map_err(|_| {
        CribleError::new(
            ErrorCode::Internal,
            format!("the debug journal reports a negative page size: {page_size}"),
        )
    })?;
    measure_with_rows(connection, page_size, count_rows(connection)?)
}

/// Measures the file for a row count already known.
///
/// The used size is `(page_count - freelist_count) × page_size`; the physical
/// size is `page_count × page_size`, WAL excluded (ADR-0127, decision 2).
fn measure_with_rows(
    connection: &Connection,
    page_size: u64,
    rows: u64,
) -> Result<JournalMeasure, CribleError> {
    let page_count: i64 = pragma(connection, "page_count")?;
    let freelist: i64 = pragma(connection, "freelist_count")?;
    let used_pages = u64::try_from(page_count.saturating_sub(freelist).max(0)).unwrap_or(0);
    let file_pages = u64::try_from(page_count.max(0)).unwrap_or(0);
    Ok(JournalMeasure {
        rows,
        bytes: used_pages * page_size,
        file_bytes: file_pages * page_size,
    })
}

/// Creates the schema of a fresh debug journal and its version (ADR-0121,
/// decisions 2 and 3).
fn create_schema(connection: &mut Connection) -> Result<(), CribleError> {
    let transaction = connection
        .transaction()
        .map_err(|error| crate::records::database_error("open the debug journal schema", &error))?;
    transaction
        .execute_batch(
            "CREATE TABLE meta (format_version INTEGER NOT NULL);
             CREATE TABLE queries (
               id           INTEGER PRIMARY KEY,
               created_at   TEXT NOT NULL,
               request_id   TEXT NOT NULL,
               \"index\"      TEXT,
               client       TEXT NOT NULL,
               mode         TEXT NOT NULL,
               status       TEXT NOT NULL,
               total_ms     REAL,
               payload      TEXT NOT NULL
             );
             CREATE INDEX queries_created_at ON queries (created_at);
             CREATE INDEX queries_index      ON queries (\"index\");
             CREATE INDEX queries_total_ms   ON queries (total_ms);",
        )
        .map_err(|error| {
            crate::records::database_error("create the debug journal schema", &error)
        })?;
    transaction
        .execute(
            "INSERT INTO meta (format_version) VALUES (?1)",
            params![FORMAT_VERSION],
        )
        .map_err(|error| {
            crate::records::database_error("write the debug journal format_version", &error)
        })?;
    transaction
        .commit()
        .map_err(|error| crate::records::database_error("commit the debug journal schema", &error))
}

/// A row of the tests, with the columns of ADR-0121, decision 3.
#[cfg(test)]
fn sample_row(request_id: &str, payload: &str) -> DebugQueryRow {
    DebugQueryRow {
        created_at: String::new(),
        request_id: request_id.to_owned(),
        index: Some("alpha".to_owned()),
        client: "reader".to_owned(),
        mode: "dense".to_owned(),
        status: "ok".to_owned(),
        total_ms: Some(1.5),
        payload: payload.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEBUG_JOURNAL_FILE_NAME, DebugJournalStore, FORMAT_VERSION, JournalCap, JournalFilter,
        RECLAIM_PAGES, read_page, read_queries, sample_row,
    };
    use rusqlite::Connection;

    /// A temporary data directory and its journal path.
    fn journal() -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempfile::tempdir().expect("a temporary directory is creatable");
        let path = directory.path().join(DEBUG_JOURNAL_FILE_NAME);
        (directory, path)
    }

    /// The cap of a test that only cares about `max_rows`.
    fn rows_cap(max_rows: usize) -> JournalCap {
        JournalCap {
            max_rows,
            max_bytes: 2_147_483_648,
        }
    }

    /// The cap of a test that must never trim.
    fn no_cap() -> JournalCap {
        JournalCap {
            max_rows: usize::MAX,
            max_bytes: usize::MAX,
        }
    }

    /// Writes `row` through the store, without a cap.
    fn write_one(store: &DebugJournalStore, row: &super::DebugQueryRow) {
        store
            .flush(std::slice::from_ref(row), no_cap())
            .expect("the flush writes");
    }

    /// Inserts a row with an explicit `created_at`, so a test controls the order.
    ///
    /// The production path lets SQLite date the row (ADR-0121, decision 3); the
    /// trim orders by `created_at ASC, id ASC`, and a test needs dated rows to
    /// prove that order without a clock.
    fn insert_dated(
        path: &std::path::Path,
        request_id: &str,
        created_at: &str,
        payload: &str,
    ) -> i64 {
        let connection = Connection::open(path).expect("the journal opens");
        connection
            .execute(
                "INSERT INTO queries
                   (created_at, request_id, \"index\", client, mode, status, total_ms, payload)
                 VALUES (?1, ?2, 'alpha', 'reader', 'dense', 'ok', 1.0, ?3)",
                rusqlite::params![created_at, request_id, payload],
            )
            .expect("the dated row is insertable");
        connection.last_insert_rowid()
    }

    /// Reads one integer pragma of the file.
    fn file_pragma(path: &std::path::Path, name: &str) -> i64 {
        let connection = Connection::open(path).expect("the journal opens");
        connection
            .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
            .unwrap_or_else(|error| panic!("PRAGMA {name} is readable: {error}"))
    }

    /// Reads `meta.format_version` with a raw connection.
    fn stored_version(path: &std::path::Path) -> i64 {
        let connection = Connection::open(path).expect("the journal opens");
        connection
            .query_row("SELECT format_version FROM meta", [], |row| row.get(0))
            .expect("the version row exists")
    }

    #[test]
    fn the_file_is_created_with_its_schema_and_version() {
        // ADR-0121, decisions 2 and 3: one `meta` row at version 1 and the
        // `queries` table, so a new instance starts on a readable file.
        let (_directory, path) = journal();
        let _store = DebugJournalStore::open_or_create(&path).expect("the journal opens");
        assert!(path.exists(), "the journal file is created");
        assert_eq!(stored_version(&path), i64::from(FORMAT_VERSION));
        assert!(
            read_queries(&path).expect("the journal reads").is_empty(),
            "a fresh journal holds no row"
        );
    }

    #[test]
    fn an_unknown_format_version_is_refused() {
        // INV-15: an existing file of another version is refused, never read.
        let (_directory, path) = journal();
        let _store = DebugJournalStore::open_or_create(&path).expect("the journal opens");
        drop(_store);
        let connection = Connection::open(&path).expect("the journal opens");
        connection
            .execute(
                "UPDATE meta SET format_version = ?1",
                [i64::from(FORMAT_VERSION) + 1],
            )
            .expect("the version is writable");
        drop(connection);
        let failure = DebugJournalStore::open_or_create(&path)
            .err()
            .expect("another version is refused");
        assert!(
            failure.message().contains("format_version"),
            "the refusal names format_version: {}",
            failure.message()
        );
    }

    #[test]
    fn a_written_row_is_read_back_with_the_columns_of_adr_0121() {
        let (_directory, path) = journal();
        let store = DebugJournalStore::open_or_create(&path).expect("the journal opens");
        let row = sample_row("request-1", r#"{"request":{"query":"contrat"}}"#);
        write_one(&store, &row);
        let rows = read_queries(&path).expect("the journal reads");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].request_id, "request-1");
        assert_eq!(rows[0].index.as_deref(), Some("alpha"));
        assert_eq!(rows[0].client, "reader");
        assert_eq!(rows[0].mode, "dense");
        assert_eq!(rows[0].status, "ok");
        assert_eq!(rows[0].total_ms, Some(1.5));
        assert_eq!(rows[0].payload, row.payload);
        assert!(
            rows[0].created_at.ends_with('Z') && rows[0].created_at.contains('T'),
            "SQLite wrote an RFC 3339 timestamp: {}",
            rows[0].created_at
        );
    }

    #[cfg(feature = "failpoints")]
    #[test]
    fn the_purge_removes_only_the_rows_older_than_the_ttl() {
        // ADR-0122, decision 8: the cut-off is read by SQLite, the young row
        // stays and the aged one goes.
        let (_directory, path) = journal();
        let store = DebugJournalStore::open_or_create(&path).expect("the journal opens");
        write_one(&store, &sample_row("fresh", "{}"));
        let expired = store
            .insert_dated_for_tests(&sample_row("expired", "{}"), "2000-01-01T00:00:00.000Z")
            .expect("the aged row is inserted");
        assert_eq!(
            read_queries(&path).expect("the journal reads").len(),
            2,
            "both rows are in the file before the purge"
        );
        let purged = store.purge(1).expect("the purge runs");
        assert_eq!(purged, 1, "the purge removes the expired row alone");
        let rows = read_queries(&path).expect("the journal reads");
        assert_eq!(rows.len(), 1, "the young row stays: {rows:?}");
        assert_ne!(rows[0].id, expired, "the expired row is gone: {rows:?}");
        assert_eq!(rows[0].request_id, "fresh");
    }

    #[test]
    fn a_fresh_file_uses_incremental_auto_vacuum() {
        // ADR-0127, decision 5: the mode is set before the first table, so a new
        // file reports `2` (INCREMENTAL) and `incremental_vacuum` can give pages
        // back later.
        let (_directory, path) = journal();
        let _store = DebugJournalStore::open_or_create(&path).expect("the journal opens");
        assert_eq!(
            file_pragma(&path, "auto_vacuum"),
            2,
            "a new debug journal is created with auto_vacuum = INCREMENTAL"
        );
    }

    #[test]
    fn the_cap_trims_the_oldest_rows_and_keeps_the_newest() {
        // ADR-0127, decisions 3 and 4: above `max_rows`, the oldest rows leave,
        // `created_at ASC, id ASC`, and the flush counts them in `trimmed`.
        let (_directory, path) = journal();
        let store = DebugJournalStore::open_or_create(&path).expect("the journal opens");
        for day in 1..=5 {
            insert_dated(
                &path,
                &format!("request-{day}"),
                &format!("2026-01-0{day}T00:00:00.000Z"),
                "{}",
            );
        }
        let report = store
            .flush(&[], rows_cap(2))
            .expect("the check of the cap runs on an empty flush");
        assert_eq!(report.trimmed, 3, "three oldest rows are trimmed");
        assert!(report.capped, "the control found the journal above the cap");
        assert_eq!(report.measure.rows, 2);
        let rows = read_queries(&path).expect("the journal reads");
        assert_eq!(
            rows.iter()
                .map(|row| row.request_id.as_str())
                .collect::<Vec<_>>(),
            vec!["request-4", "request-5"],
            "the newest rows stay"
        );
    }

    #[test]
    fn the_trim_breaks_a_tie_on_created_at_by_id() {
        // ADR-0127, decision 3: the order is total, `created_at` then `id`.
        let (_directory, path) = journal();
        let store = DebugJournalStore::open_or_create(&path).expect("the journal opens");
        let first = insert_dated(&path, "first", "2026-01-01T00:00:00.000Z", "{}");
        let second = insert_dated(&path, "second", "2026-01-01T00:00:00.000Z", "{}");
        assert_eq!(
            second,
            first + 1,
            "the two rows share their `created_at` and differ by `id`"
        );
        let report = store
            .flush(&[], rows_cap(1))
            .expect("the check of the cap runs");
        assert_eq!(report.trimmed, 1);
        let rows = read_queries(&path).expect("the journal reads");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].id, second,
            "the lower id of the tie leaves first: {rows:?}"
        );
    }

    #[test]
    fn the_byte_cap_trims_the_oldest_rows() {
        // ADR-0127, decision 2: `max_bytes` compares the **used** size, so a
        // payload large enough to cross the cap takes the oldest row out.
        let (_directory, path) = journal();
        let store = DebugJournalStore::open_or_create(&path).expect("the journal opens");
        let empty = store.measure().expect("the file is measurable");
        let payload = "x".repeat(64 * 1024);
        for position in 0..3 {
            insert_dated(
                &path,
                &format!("big-{position}"),
                &format!("2026-01-0{}T00:00:00.000Z", position + 1),
                &payload,
            );
        }
        let cap = JournalCap {
            max_rows: usize::MAX,
            max_bytes: usize::try_from(empty.bytes + 100 * 1024).expect("a cap fits in usize"),
        };
        let report = store.flush(&[], cap).expect("the check of the cap runs");
        assert!(report.capped, "the file is above the byte cap");
        assert!(
            report.trimmed >= 1,
            "at least the oldest row leaves: {report:?}"
        );
        assert!(report.measure.bytes <= cap.max_bytes as u64);
        let rows = read_queries(&path).expect("the journal reads");
        assert_eq!(
            rows.last().map(|row| row.request_id.as_str()),
            Some("big-2"),
            "the newest row stays: {rows:?}"
        );
        assert!(
            rows.iter().all(|row| row.request_id != "big-0"),
            "the oldest row left: {rows:?}"
        );
    }

    #[test]
    fn a_row_bigger_than_the_byte_cap_is_trimmed() {
        // The criterion of the card: one row above `max_bytes` alone is deleted
        // by the trim, and the journal falls back to its empty size.
        let (_directory, path) = journal();
        let store = DebugJournalStore::open_or_create(&path).expect("the journal opens");
        let empty = store.measure().expect("the file is measurable");
        let payload = "x".repeat(512 * 1024);
        insert_dated(&path, "huge", "2026-01-01T00:00:00.000Z", &payload);
        let cap = JournalCap {
            max_rows: usize::MAX,
            max_bytes: usize::try_from(empty.bytes + 100 * 1024).expect("a cap fits in usize"),
        };
        let report = store.flush(&[], cap).expect("the check of the cap runs");
        assert_eq!(report.trimmed, 1, "the single row leaves: {report:?}");
        assert_eq!(report.measure.rows, 0);
        assert!(
            report.measure.bytes <= cap.max_bytes as u64,
            "the empty file is back under the cap: {report:?}"
        );
        assert!(
            read_queries(&path).expect("the journal reads").is_empty(),
            "no row is left"
        );
    }

    #[test]
    fn a_trim_for_one_cap_leaves_the_other_far_from_its_bound() {
        // ADR-0127, decision 1: the two caps are independent. A trim for
        // `max_bytes` leaves `max_rows` far from its bound, and the reverse.
        let (_directory, path) = journal();
        let store = DebugJournalStore::open_or_create(&path).expect("the journal opens");
        let empty = store.measure().expect("the file is measurable");
        let payload = "x".repeat(64 * 1024);
        for position in 0..3 {
            insert_dated(
                &path,
                &format!("row-{position}"),
                &format!("2026-01-0{}T00:00:00.000Z", position + 1),
                &payload,
            );
        }
        let by_bytes = JournalCap {
            max_rows: 1_000,
            max_bytes: usize::try_from(empty.bytes + 100 * 1024).expect("a cap fits in usize"),
        };
        let report = store.flush(&[], by_bytes).expect("the byte cap runs");
        assert!(
            report.measure.rows < by_bytes.max_rows as u64,
            "the row cap is far from its bound: {report:?}"
        );

        // The other direction, on a journal of its own: the byte cap stays far
        // from its bound when the row cap drives the trim.
        let (_other_directory, other_path) = journal();
        let other = DebugJournalStore::open_or_create(&other_path).expect("the journal opens");
        for position in 0..3 {
            write_one(&other, &sample_row(&format!("small-{position}"), "{}"));
        }
        let by_rows = JournalCap {
            max_rows: 1,
            max_bytes: 2_147_483_648,
        };
        let report = other.flush(&[], by_rows).expect("the row cap runs");
        assert_eq!(report.measure.rows, 1);
        assert!(
            report.measure.bytes < by_rows.max_bytes as u64,
            "the byte cap is far from its bound: {report:?}"
        );
    }

    #[test]
    fn a_purge_before_a_trim_leaves_only_the_trim_in_the_count() {
        // ADR-0127, decision 7: the purge of the retention runs first, and
        // `trimmed` counts the cap alone.
        let (_directory, path) = journal();
        let store = DebugJournalStore::open_or_create(&path).expect("the journal opens");
        insert_dated(&path, "expired", "2000-01-01T00:00:00.000Z", "{}");
        for position in 0..3 {
            write_one(&store, &sample_row(&format!("fresh-{position}"), "{}"));
        }
        let purged = store.purge(1).expect("the purge runs");
        assert_eq!(purged, 1, "the purge removes the aged row alone");
        let report = store.flush(&[], rows_cap(2)).expect("the trim runs");
        assert_eq!(
            report.trimmed, 1,
            "only the row the cap deletes is counted: {report:?}"
        );
        assert_eq!(report.measure.rows, 2);
    }

    #[test]
    fn both_caps_are_enforced_in_the_same_flush() {
        // The card (`docs/phases/P8-docker-et-deploiement.md`, « les deux
        // plafonds ensemble »): the row cap deletes a batch, then the byte cap
        // still exceeds its bound and deletes one row at a time. Both branches
        // of the loop run in one flush.
        let (_directory, path) = journal();
        let store = DebugJournalStore::open_or_create(&path).expect("the journal opens");
        let empty = store.measure().expect("the file is measurable");
        let payload = "x".repeat(64 * 1024);
        for position in 0..3 {
            insert_dated(
                &path,
                &format!("big-{position}"),
                &format!("2026-01-0{}T00:00:00.000Z", position + 1),
                &payload,
            );
        }
        let cap = JournalCap {
            max_rows: 2,
            max_bytes: usize::try_from(empty.bytes + 70 * 1024).expect("a cap fits in usize"),
        };
        let report = store.flush(&[], cap).expect("the check of the cap runs");
        assert!(report.capped, "the journal is above both caps: {report:?}");
        assert_eq!(
            report.trimmed, 2,
            "one row leaves for the row cap, then one more for the byte cap: {report:?}"
        );
        assert_eq!(report.measure.rows, 1, "one row fits under both caps");
        assert!(
            report.measure.bytes <= cap.max_bytes as u64,
            "the used size is back under the byte cap: {report:?}"
        );
        let rows = read_queries(&path).expect("the journal reads");
        assert_eq!(
            rows.iter()
                .map(|row| row.request_id.as_str())
                .collect::<Vec<_>>(),
            vec!["big-2"],
            "the newest row is the one kept: {rows:?}"
        );
    }

    #[test]
    fn the_open_control_brings_a_file_left_above_the_caps_back_under_them() {
        // ADR-0127, decision 4: an empty flush applies the cap, which is how a
        // file left above by a crash or a lower configuration is recovered.
        let (_directory, path) = journal();
        let store = DebugJournalStore::open_or_create(&path).expect("the journal opens");
        for position in 0..5 {
            write_one(&store, &sample_row(&format!("left-{position}"), "{}"));
        }
        assert_eq!(store.measure().expect("the file is measurable").rows, 5);
        let report = store
            .flush(&[], rows_cap(2))
            .expect("the open control runs");
        assert!(report.capped);
        assert_eq!(report.trimmed, 3);
        assert_eq!(report.measure.rows, 2);
    }

    #[test]
    fn the_reclaim_lowers_the_freelist_without_changing_the_rows() {
        // ADR-0127, decision 5: `incremental_vacuum` gives free pages back and
        // leaves the rows untouched.
        let (_directory, path) = journal();
        let store = DebugJournalStore::open_or_create(&path).expect("the journal opens");
        let empty = store.measure().expect("the file is measurable");
        let payload = "x".repeat(2 * 1024 * 1024);
        insert_dated(&path, "huge", "2026-01-01T00:00:00.000Z", &payload);
        let cap = JournalCap {
            max_rows: usize::MAX,
            max_bytes: usize::try_from(empty.bytes + 100 * 1024).expect("a cap fits in usize"),
        };
        store.flush(&[], cap).expect("the trim runs");
        let before = file_pragma(&path, "freelist_count");
        assert!(before > 0, "the trimmed pages are free: {before}");
        let freed = store.reclaim().expect("the reclaim runs");
        let after = file_pragma(&path, "freelist_count");
        assert!(freed >= 1, "the reclaim asked for pages: {freed}");
        assert!(
            freed <= u64::try_from(RECLAIM_PAGES).expect("the budget is positive"),
            "the budget of {RECLAIM_PAGES} pages is respected: {freed}"
        );
        assert_eq!(
            after,
            before - i64::try_from(freed).expect("the count fits in i64"),
            "the freelist fell by the pages reclaimed"
        );
        assert!(
            read_queries(&path).expect("the journal reads").is_empty(),
            "the reclaim changes no row"
        );
    }

    #[test]
    fn the_reclaim_is_bounded_to_the_pages_of_the_budget() {
        // ADR-0127, decision 5: one flush never reclaims more than the budget,
        // so the file comes back down over several flushes.
        let (_directory, path) = journal();
        let store = DebugJournalStore::open_or_create(&path).expect("the journal opens");
        let empty = store.measure().expect("the file is measurable");
        let payload = "x".repeat(4 * 1024 * 1024);
        insert_dated(&path, "huge", "2026-01-01T00:00:00.000Z", &payload);
        let cap = JournalCap {
            max_rows: usize::MAX,
            max_bytes: usize::try_from(empty.bytes + 100 * 1024).expect("a cap fits in usize"),
        };
        store.flush(&[], cap).expect("the trim runs");
        let before = file_pragma(&path, "freelist_count");
        assert!(
            before > RECLAIM_PAGES,
            "the file leaves more free pages than the budget: {before}"
        );
        let freed = store.reclaim().expect("the reclaim runs");
        assert_eq!(
            freed,
            u64::try_from(RECLAIM_PAGES).expect("the budget is positive"),
            "one flush reclaims exactly its budget"
        );
        assert_eq!(
            file_pragma(&path, "freelist_count"),
            before - RECLAIM_PAGES,
            "the rest waits for the next flush"
        );
    }

    #[test]
    fn a_file_of_the_previous_build_is_read_and_capped() {
        // INV-15 and ADR-0127, decision 5: the schema and `format_version` do
        // not change, so a file created without `auto_vacuum = INCREMENTAL` (the
        // shape of the previous build) opens, reads and trims.
        let (_directory, path) = journal();
        let connection = Connection::open(&path).expect("the journal opens");
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 CREATE TABLE meta (format_version INTEGER NOT NULL);
                 CREATE TABLE queries (
                   id           INTEGER PRIMARY KEY,
                   created_at   TEXT NOT NULL,
                   request_id   TEXT NOT NULL,
                   \"index\"      TEXT,
                   client       TEXT NOT NULL,
                   mode         TEXT NOT NULL,
                   status       TEXT NOT NULL,
                   total_ms     REAL,
                   payload      TEXT NOT NULL
                 );
                 CREATE INDEX queries_created_at ON queries (created_at);
                 INSERT INTO meta (format_version) VALUES (1);
                 INSERT INTO queries
                   (created_at, request_id, client, mode, status, total_ms, payload)
                 VALUES
                   ('2026-01-01T00:00:00.000Z', 'old-1', 'reader', 'dense', 'ok', 1.0, '{}'),
                   ('2026-01-02T00:00:00.000Z', 'old-2', 'reader', 'dense', 'ok', 1.0, '{}');",
            )
            .expect("the previous build's schema is created");
        assert_eq!(file_pragma(&path, "auto_vacuum"), 0, "no incremental mode");
        drop(connection);

        let store = DebugJournalStore::open_or_create(&path).expect("the old file opens");
        assert_eq!(stored_version(&path), i64::from(FORMAT_VERSION));
        assert_eq!(
            read_queries(&path).expect("the old file reads").len(),
            2,
            "the rows of the previous build are readable"
        );
        let report = store.flush(&[], rows_cap(1)).expect("the cap runs");
        assert_eq!(report.trimmed, 1, "the old file is capped like a new one");
        assert_eq!(report.measure.rows, 1);
        // `reclaim` must not fail on a file SQLite did not create with the mode.
        store
            .reclaim()
            .expect("the reclaim is a no-op, not an error");
    }

    #[test]
    fn the_page_orders_by_created_at_then_id_and_pages_by_before_id() {
        // ADR-0122, decision 9: the newest first, and a strictly lower id.
        let (_directory, path) = journal();
        let store = DebugJournalStore::open_or_create(&path).expect("the journal opens");
        let mut written = Vec::new();
        for position in 0..3 {
            write_one(&store, &sample_row(&format!("request-{position}"), "{}"));
            written.push(
                read_queries(&path)
                    .expect("the journal reads")
                    .last()
                    .expect("a row")
                    .id,
            );
        }
        let page = read_page(
            &path,
            &JournalFilter {
                limit: 2,
                ..JournalFilter::default()
            },
        )
        .expect("the page reads");
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].id, written[2], "the newest row starts the page");
        assert!(page[0].id > page[1].id, "the order is total: {page:?}");
        let rest = read_page(
            &path,
            &JournalFilter {
                limit: 2,
                before_id: Some(page[1].id),
                ..JournalFilter::default()
            },
        )
        .expect("the page reads");
        assert_eq!(rest.len(), 1);
        assert!(rest[0].id < page[1].id, "before_id is strict");
    }

    #[test]
    fn the_index_filter_matches_a_multi_index_row_by_membership() {
        // ADR-0122, decision 4: the column is `NULL`, `targets[]` names the
        // indexes, and the filter retains the row for each of them.
        let (_directory, path) = journal();
        let store = DebugJournalStore::open_or_create(&path).expect("the journal opens");
        let payload = r#"{"request":{},"targets":[{"index":"alpha"},{"index":"beta"}]}"#;
        let mut row = sample_row("multi", payload);
        row.index = None;
        write_one(&store, &row);
        for name in ["alpha", "beta"] {
            let page = read_page(
                &path,
                &JournalFilter {
                    index: Some(name.to_owned()),
                    limit: 100,
                    ..JournalFilter::default()
                },
            )
            .expect("the page reads");
            assert_eq!(page.len(), 1, "the row targets {name}: {page:?}");
        }
        let other = read_page(
            &path,
            &JournalFilter {
                index: Some("gamma".to_owned()),
                limit: 100,
                ..JournalFilter::default()
            },
        )
        .expect("the page reads");
        assert!(other.is_empty(), "gamma is not a target: {other:?}");
    }

    #[test]
    fn the_index_page_is_completed_when_multi_rows_are_skipped() {
        // ADR-0122, decision 9: `limit` is the number of rows of the page. A
        // simple row of another index is settled by SQL; a multi-index row that
        // does not target the name is read and skipped here, so the page is
        // completed from the next keyset block instead of coming back short.
        let (_directory, path) = journal();
        let store = DebugJournalStore::open_or_create(&path).expect("the journal opens");
        let mut written = Vec::new();
        for position in 0..5 {
            // Alternating writes: the newest rows of the file are `alpha`, then a
            // multi-index row that targets `beta` alone, then two `beta` rows.
            write_one(&store, &sample_row(&format!("alpha-{position}"), "{}"));
            let mut multi = sample_row(
                &format!("multi-{position}"),
                r#"{"targets":[{"index":"beta"}]}"#,
            );
            multi.index = None;
            let mut beta = sample_row(&format!("beta-{position}"), "{}");
            beta.index = Some("beta".to_owned());
            store
                .flush(&[multi, beta], no_cap())
                .expect("the rows are written");
            written.push(
                read_queries(&path)
                    .expect("the journal reads")
                    .into_iter()
                    .rev()
                    .find(|row| row.request_id == format!("alpha-{position}"))
                    .expect("the alpha row")
                    .id,
            );
        }
        let page = read_page(
            &path,
            &JournalFilter {
                index: Some("alpha".to_owned()),
                limit: 2,
                ..JournalFilter::default()
            },
        )
        .expect("the page reads");
        assert_eq!(page.len(), 2, "the page is full of `alpha` rows: {page:?}");
        assert!(page.iter().all(|row| row.index.as_deref() == Some("alpha")));
        assert_eq!(
            page.iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![written[4], written[3]],
            "the two newest `alpha` rows start the page"
        );

        let rest = read_page(
            &path,
            &JournalFilter {
                index: Some("alpha".to_owned()),
                limit: 2,
                before_id: Some(page[1].id),
                ..JournalFilter::default()
            },
        )
        .expect("the page reads");
        assert_eq!(
            rest.iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![written[2], written[1]],
            "`before_id` continues without a gap: {rest:?}"
        );
    }

    #[test]
    fn the_index_page_continues_between_simple_rows_of_one_index() {
        // Five `alpha` rows and five `beta` rows, interleaved: the newest two of
        // `alpha` make the page, and the cursor walks the same order.
        let (_directory, path) = journal();
        let store = DebugJournalStore::open_or_create(&path).expect("the journal opens");
        let mut alpha = Vec::new();
        for position in 0..5 {
            let mut beta = sample_row(&format!("beta-{position}"), "{}");
            beta.index = Some("beta".to_owned());
            let mut row = sample_row(&format!("alpha-{position}"), "{}");
            row.index = Some("alpha".to_owned());
            store
                .flush(&[row, beta], no_cap())
                .expect("the rows are written");
            alpha.push(
                read_queries(&path)
                    .expect("the journal reads")
                    .into_iter()
                    .rev()
                    .find(|row| row.request_id == format!("alpha-{position}"))
                    .expect("the alpha row")
                    .id,
            );
        }
        let first = read_page(
            &path,
            &JournalFilter {
                index: Some("alpha".to_owned()),
                limit: 2,
                ..JournalFilter::default()
            },
        )
        .expect("the page reads");
        assert_eq!(first.len(), 2, "`limit` bounds the page: {first:?}");
        assert_eq!(first[0].id, alpha[4]);
        assert_eq!(first[1].id, alpha[3]);

        let second = read_page(
            &path,
            &JournalFilter {
                index: Some("alpha".to_owned()),
                limit: 2,
                before_id: Some(first[1].id),
                ..JournalFilter::default()
            },
        )
        .expect("the page reads");
        assert_eq!(
            second.iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![alpha[2], alpha[1]],
            "`before_id` resumes without a repeat: {second:?}"
        );
    }

    #[test]
    fn the_min_total_filter_excludes_a_null_total() {
        let (_directory, path) = journal();
        let store = DebugJournalStore::open_or_create(&path).expect("the journal opens");
        let mut unfinished = sample_row("unfinished", "{}");
        unfinished.total_ms = None;
        store
            .flush(&[sample_row("finished", "{}"), unfinished], no_cap())
            .expect("the rows are written");
        let page = read_page(
            &path,
            &JournalFilter {
                min_total_ms: Some(0.0),
                limit: 100,
                ..JournalFilter::default()
            },
        )
        .expect("the page reads");
        assert_eq!(page.len(), 1, "the NULL total is excluded: {page:?}");
        assert_eq!(page[0].request_id, "finished");
    }
}
