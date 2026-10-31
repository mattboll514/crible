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

/// One row of `queries`, as a writer deposits it or a reader receives it.
///
/// The same type is the input of [`DebugJournalStore::write`] and the output of
/// [`read_queries`]: a writer leaves [`DebugQueryRow::created_at`] empty and
/// SQLite fills it at the insertion (ADR-0121, decision 3), a reader receives
/// the stored value.
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

    /// Writes one flush in a single transaction.
    ///
    /// Every row is attached to the instant SQLite reads at the insertion
    /// (ADR-0121, decision 3); either every row lands or none does
    /// (ADR-0122, decision 7).
    ///
    /// # Errors
    ///
    /// The SQLite failure.
    pub fn write(&self, rows: &[DebugQueryRow]) -> Result<(), CribleError> {
        if rows.is_empty() {
            return Ok(());
        }
        self.with_mut(|connection| {
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
            transaction.commit().map_err(|error| {
                crate::records::database_error("commit the debug journal flush", &error)
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
        DEBUG_JOURNAL_FILE_NAME, DebugJournalStore, FORMAT_VERSION, JournalFilter, read_page,
        read_queries, sample_row,
    };
    use rusqlite::Connection;

    /// A temporary data directory and its journal path.
    fn journal() -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempfile::tempdir().expect("a temporary directory is creatable");
        let path = directory.path().join(DEBUG_JOURNAL_FILE_NAME);
        (directory, path)
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
        store
            .write(std::slice::from_ref(&row))
            .expect("the flush writes");
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
        store
            .write(&[sample_row("fresh", "{}")])
            .expect("the fresh row is written");
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
    fn the_page_orders_by_created_at_then_id_and_pages_by_before_id() {
        // ADR-0122, decision 9: the newest first, and a strictly lower id.
        let (_directory, path) = journal();
        let store = DebugJournalStore::open_or_create(&path).expect("the journal opens");
        let mut written = Vec::new();
        for position in 0..3 {
            store
                .write(&[sample_row(&format!("request-{position}"), "{}")])
                .expect("the row is written");
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
        store.write(&[row]).expect("the row is written");
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
            store
                .write(&[sample_row(&format!("alpha-{position}"), "{}")])
                .expect("the row is written");
            let mut multi = sample_row(
                &format!("multi-{position}"),
                r#"{"targets":[{"index":"beta"}]}"#,
            );
            multi.index = None;
            store
                .write(&[multi, {
                    let mut beta = sample_row(&format!("beta-{position}"), "{}");
                    beta.index = Some("beta".to_owned());
                    beta
                }])
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
            store.write(&[row, beta]).expect("the rows are written");
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
            .write(&[sample_row("finished", "{}"), unfinished])
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
