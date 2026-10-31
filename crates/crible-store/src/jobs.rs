// SPDX-License-Identifier: Apache-2.0

//! `jobs.sqlite`: the job table of an instance.
//!
//! References: ADR-0103 (support, schema, state transitions, retention,
//! restart) and ADR-0101 (the `internal` message of a job error). The file lives
//! at the root of `<data>`, next to `instance.json`, `analytics.sqlite` and
//! `debug.sqlite`; it is opened by this module of `crible-store`, which already
//! carries `rusqlite` (ADR-0038), so `crible-server` keeps no edge to that
//! crate.
//!
//! One connection serves the writers of every index. Its settings are those of
//! `records.sqlite`, `busy_timeout` of ADR-0038 included, and it runs in WAL so
//! a reader never waits for a writer.
//!
//! The table holds one row per job, written at each **state transition only**
//! (creation, start, end): the fine progression lives in memory between two
//! transitions. `progress`, `result` and `error` are JSON text; `error` has the
//! shape of ADR-0101 without `request_id`, because a job is not a request.
//!
//! Timestamps follow `batches.applied_at`: SQLite writes them as RFC 3339 in
//! UTC with `strftime('%Y-%m-%dT%H:%M:%SZ', 'now')`, so the code owns no clock.

use std::path::Path;
use std::sync::Mutex;

use crible_core::error::{CribleError, ErrorCode};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::sqlite::{check_format_version, configure_busy_timeout, enable_wal};

/// Name of the job table file at the root of `<data>` (ADR-0103, decision 1).
pub const JOBS_FILE_NAME: &str = "jobs.sqlite";

/// Format of the file this build reads and writes (ADR-0103, decision 1).
pub const FORMAT_VERSION: u32 = 1;

/// Life of a finished job, in days (ADR-0103, decision 6). Fixed, not a key.
pub const RETENTION_DAYS: u32 = 30;

/// Fixed message of a job a restart interrupted (ADR-0103, decision 5).
pub const INTERRUPTED_MESSAGE: &str = "the job was interrupted by a server restart";

/// Columns of one job, in the order [`JobRecord::from_row`] reads them.
const COLUMNS: &str = "id, kind, \"index\", state, created_at, started_at, finished_at, generation, progress, result, error";

/// State of a job (`docs/05-api.md`, "Jobs"; ADR-0103, decision 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// Accepted, waiting for its index writer.
    Queued,
    /// Running on the writer of its index.
    Running,
    /// Finished without an error.
    Succeeded,
    /// Finished with an error.
    Failed,
}

impl JobState {
    /// Every state of `docs/05-api.md`, in the order it lists them.
    pub const ALL: [Self; 4] = [Self::Queued, Self::Running, Self::Succeeded, Self::Failed];

    /// Wire name used in the table and in the API.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }

    /// Parses a wire name, or `None` when it is not one of the four.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    /// Whether the state is terminal.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

/// Kind of work a job carries (`docs/05-api.md`, "Jobs"; the list is closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// Attach a batch deposited in `exchange/inbox`.
    AttachBatch,
    /// Apply an online batch to an index.
    OnlineBatch,
    /// Update document metadata.
    PatchDocuments,
    /// Compact an epoch.
    Compact,
    /// Export the live data of an index.
    Export,
    /// Snapshot an epoch.
    Snapshot,
    /// Rebuild the lexical branch.
    RebuildLexical,
    /// Apply a configuration patch (ADR-0105).
    PatchConfig,
    /// Delete an index.
    DeleteIndex,
}

impl JobKind {
    /// Every kind of the closed list.
    pub const ALL: [Self; 9] = [
        Self::AttachBatch,
        Self::OnlineBatch,
        Self::PatchDocuments,
        Self::Compact,
        Self::Export,
        Self::Snapshot,
        Self::RebuildLexical,
        Self::PatchConfig,
        Self::DeleteIndex,
    ];

    /// Wire name used in the table and in the API.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AttachBatch => "attach_batch",
            Self::OnlineBatch => "online_batch",
            Self::PatchDocuments => "patch_documents",
            Self::Compact => "compact",
            Self::Export => "export",
            Self::Snapshot => "snapshot",
            Self::RebuildLexical => "rebuild_lexical",
            Self::PatchConfig => "patch_config",
            Self::DeleteIndex => "delete_index",
        }
    }

    /// Parses a wire name, or `None` when the list does not hold it.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "attach_batch" => Some(Self::AttachBatch),
            "online_batch" => Some(Self::OnlineBatch),
            "patch_documents" => Some(Self::PatchDocuments),
            "compact" => Some(Self::Compact),
            "export" => Some(Self::Export),
            "snapshot" => Some(Self::Snapshot),
            "rebuild_lexical" => Some(Self::RebuildLexical),
            "patch_config" => Some(Self::PatchConfig),
            "delete_index" => Some(Self::DeleteIndex),
            _ => None,
        }
    }
}

/// One row of the job table, as it is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRecord {
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
    /// Fine progression, JSON text, written at a transition only.
    pub progress: Option<String>,
    /// Report of the job, JSON text, written when it is already persisted
    /// elsewhere (ADR-0042, ADR-0060).
    pub result: Option<String>,
    /// `{ "code", "message", "details" }` as JSON text (ADR-0103, decision 4).
    pub error: Option<String>,
}

impl JobRecord {
    /// Reads one row whose columns are [`COLUMNS`], in that order.
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        let id: String = row.get(0)?;
        let kind: String = row.get(1)?;
        let index: String = row.get(2)?;
        let state: String = row.get(3)?;
        let created_at: String = row.get(4)?;
        let started_at: Option<String> = row.get(5)?;
        let finished_at: Option<String> = row.get(6)?;
        let generation: Option<i64> = row.get(7)?;
        let progress: Option<String> = row.get(8)?;
        let result: Option<String> = row.get(9)?;
        let error: Option<String> = row.get(10)?;
        let kind = JobKind::parse(&kind).ok_or_else(|| unknown_value("kind", &kind, &id))?;
        let state = JobState::parse(&state).ok_or_else(|| unknown_value("state", &state, &id))?;
        Ok(Self {
            id,
            kind,
            index,
            state,
            created_at,
            started_at,
            finished_at,
            generation: generation.map(u64::try_from).transpose().map_err(|_| {
                rusqlite::Error::InvalidColumnType(
                    7,
                    "generation".to_owned(),
                    rusqlite::types::Type::Integer,
                )
            })?,
            progress,
            result,
            error,
        })
    }
}

/// Builds the SQLite error of a value the table does not know.
///
/// The caller wraps it as an [`ErrorCode::Internal`] through
/// [`database_error`](crate::records::database_error), naming the job: a row
/// this build cannot read is a corruption of the file, not a request fault.
fn unknown_value(column: &str, value: &str, id: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::other(format!(
            "job '{id}' has an unknown {column} '{value}'"
        ))),
    )
}

/// The job table of an instance: one connection serving every writer.
pub struct JobStore {
    connection: Mutex<Connection>,
}

impl JobStore {
    /// Opens `<data>/jobs.sqlite`, creating it with its schema and its
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
        .map_err(|error| database_error("open the job database", &error))?;
        configure_busy_timeout(&connection)?;
        enable_wal(&connection, "job")?;
        if fresh {
            create_schema(&mut connection)?;
        } else {
            check_format_version(&connection, FORMAT_VERSION, "job")?;
        }
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Inserts a job in `queued` and returns its `created_at`.
    ///
    /// `created_at` is written by SQLite, with the clock and the format of
    /// `batches.applied_at` (ADR-0103, decision 2).
    ///
    /// # Errors
    ///
    /// The SQLite failure, through `crate::records::database_error`.
    pub fn insert(&self, id: &str, kind: JobKind, index: &str) -> Result<String, CribleError> {
        self.with(|connection| {
            connection
                .query_row(
                    "INSERT INTO jobs (id, kind, \"index\", state, created_at)
                     VALUES (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
                     RETURNING created_at",
                    params![id, kind.as_str(), index, JobState::Queued.as_str()],
                    |row| row.get(0),
                )
                .map_err(|error| database_error("insert a job", &error))
        })
    }

    /// Moves `id` to `running` and writes the progression of that instant.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the job is not `queued`, and the SQLite
    /// failure otherwise.
    pub fn mark_running(&self, id: &str, progress: Option<&str>) -> Result<JobRecord, CribleError> {
        self.with(|connection| {
            let changed = connection
                .execute(
                    "UPDATE jobs
                     SET state = ?2, started_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                         progress = ?3
                     WHERE id = ?1 AND state = ?4",
                    params![
                        id,
                        JobState::Running.as_str(),
                        progress,
                        JobState::Queued.as_str()
                    ],
                )
                .map_err(|error| database_error("start a job", &error))?;
            transitioned(changed, id, JobState::Running)
        })?;
        self.job_or_internal(id)
    }

    /// Moves `id` to `succeeded` and writes the generation, the progression and
    /// the report of that instant.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the job is not `running`, and the SQLite
    /// failure otherwise.
    pub fn finish_succeeded(
        &self,
        id: &str,
        generation: Option<u64>,
        progress: Option<&str>,
        result: Option<&str>,
    ) -> Result<JobRecord, CribleError> {
        let generation = generation
            .map(i64::try_from)
            .transpose()
            .map_err(|_| CribleError::new(ErrorCode::Internal, "job generation is out of range"))?;
        self.with(|connection| {
            let changed = connection
                .execute(
                    "UPDATE jobs
                     SET state = ?2, finished_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                         generation = ?3, progress = ?4, result = ?5
                     WHERE id = ?1 AND state = ?6",
                    params![
                        id,
                        JobState::Succeeded.as_str(),
                        generation,
                        progress,
                        result,
                        JobState::Running.as_str()
                    ],
                )
                .map_err(|error| database_error("finish a job", &error))?;
            transitioned(changed, id, JobState::Succeeded)
        })?;
        self.job_or_internal(id)
    }

    /// Moves `id` to `failed` and writes the progression and the error of that
    /// instant.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the job is not `running`, and the SQLite
    /// failure otherwise.
    pub fn finish_failed(
        &self,
        id: &str,
        progress: Option<&str>,
        error: &str,
    ) -> Result<JobRecord, CribleError> {
        self.with(|connection| {
            let changed = connection
                .execute(
                    "UPDATE jobs
                     SET state = ?2, finished_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                         progress = ?3, error = ?4
                     WHERE id = ?1 AND state = ?5",
                    params![
                        id,
                        JobState::Failed.as_str(),
                        progress,
                        error,
                        JobState::Running.as_str()
                    ],
                )
                .map_err(|error| database_error("fail a job", &error))?;
            transitioned(changed, id, JobState::Failed)
        })?;
        self.job_or_internal(id)
    }

    /// Marks every `queued` or `running` row `failed`, at startup (ADR-0103,
    /// decision 5).
    ///
    /// No job is replayed. `finished_at` is the startup time, `error.code` is
    /// `internal` and the message is [`INTERRUPTED_MESSAGE`].
    ///
    /// # Errors
    ///
    /// The SQLite failure.
    pub fn fail_interrupted(&self) -> Result<usize, CribleError> {
        let error = job_error_body(
            ErrorCode::Internal,
            INTERRUPTED_MESSAGE,
            serde_json::json!({}),
        );
        self.with(|connection| {
            connection
                .execute(
                    "UPDATE jobs
                     SET state = ?1, finished_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                         error = ?2
                     WHERE state IN (?3, ?4)",
                    params![
                        JobState::Failed.as_str(),
                        error,
                        JobState::Queued.as_str(),
                        JobState::Running.as_str()
                    ],
                )
                .map_err(|error| database_error("fail the interrupted jobs", &error))
        })
    }

    /// Deletes the jobs finished more than [`RETENTION_DAYS`] days ago and
    /// returns their identifiers.
    ///
    /// A job still `queued` or `running` is never purged (ADR-0103, decision 6).
    ///
    /// # Errors
    ///
    /// The SQLite failure.
    pub fn purge(&self) -> Result<Vec<String>, CribleError> {
        let older = format!("-{RETENTION_DAYS} days");
        self.with(|connection| {
            let mut statement = connection
                .prepare(
                    "SELECT id FROM jobs
                     WHERE state IN (?1, ?2)
                       AND finished_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?3)",
                )
                .map_err(|error| database_error("prepare the purge of the jobs", &error))?;
            let purged = statement
                .query_map(
                    params![
                        JobState::Succeeded.as_str(),
                        JobState::Failed.as_str(),
                        older
                    ],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|error| database_error("list the jobs to purge", &error))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| database_error("read the jobs to purge", &error))?;
            drop(statement);
            connection
                .execute(
                    "DELETE FROM jobs
                     WHERE state IN (?1, ?2)
                       AND finished_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?3)",
                    params![
                        JobState::Succeeded.as_str(),
                        JobState::Failed.as_str(),
                        older
                    ],
                )
                .map_err(|error| database_error("purge the jobs", &error))?;
            Ok(purged)
        })
    }

    /// Reads one job, or `None` when the table does not hold it.
    ///
    /// # Errors
    ///
    /// The SQLite failure, or [`ErrorCode::Internal`] for a row whose `kind` or
    /// `state` is not one of this build.
    pub fn job(&self, id: &str) -> Result<Option<JobRecord>, CribleError> {
        self.with(|connection| {
            connection
                .query_row(
                    &format!("SELECT {COLUMNS} FROM jobs WHERE id = ?1"),
                    params![id],
                    JobRecord::from_row,
                )
                .optional()
                .map_err(|error| database_error("read a job", &error))
        })
    }

    /// Lists the retained jobs, filtered by `index` and `state`, by
    /// `created_at` descending then `id` (ADR-0103, decision 7).
    ///
    /// # Errors
    ///
    /// The SQLite failure, or [`ErrorCode::Internal`] for a row of this build
    /// cannot read.
    pub fn list(
        &self,
        index: Option<&str>,
        state: Option<JobState>,
    ) -> Result<Vec<JobRecord>, CribleError> {
        self.with(|connection| {
            let mut statement = connection
                .prepare(&format!(
                    "SELECT {COLUMNS} FROM jobs
                     WHERE (?1 IS NULL OR \"index\" = ?1)
                       AND (?2 IS NULL OR state = ?2)
                     ORDER BY created_at DESC, id ASC"
                ))
                .map_err(|error| database_error("prepare the list of the jobs", &error))?;
            let rows = statement
                .query_map(
                    params![index, state.map(JobState::as_str)],
                    JobRecord::from_row,
                )
                .map_err(|error| database_error("list the jobs", &error))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| database_error("read the list of the jobs", &error))?;
            Ok(rows)
        })
    }

    /// Deletes one job, used when a submission finds the queue full: the job was
    /// never accepted, so it must not be visible.
    ///
    /// # Errors
    ///
    /// The SQLite failure.
    pub fn delete(&self, id: &str) -> Result<(), CribleError> {
        self.with(|connection| {
            connection
                .execute("DELETE FROM jobs WHERE id = ?1", params![id])
                .map_err(|error| database_error("delete a job", &error))?;
            Ok(())
        })
    }

    /// Runs `action` on the one connection of the instance.
    fn with<T>(
        &self,
        action: impl FnOnce(&Connection) -> Result<T, CribleError>,
    ) -> Result<T, CribleError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| CribleError::new(ErrorCode::Internal, "the job database is poisoned"))?;
        action(&connection)
    }

    /// Reads back `id` after a transition, which must exist.
    fn job_or_internal(&self, id: &str) -> Result<JobRecord, CribleError> {
        self.job(id)?.ok_or_else(|| {
            CribleError::new(
                ErrorCode::Internal,
                format!("job '{id}' disappeared after its transition"),
            )
        })
    }
}

/// Refuses a transition that matched no row.
fn transitioned(changed: usize, id: &str, state: JobState) -> Result<(), CribleError> {
    if changed == 1 {
        return Ok(());
    }
    Err(CribleError::new(
        ErrorCode::Internal,
        format!(
            "job '{id}' could not move to {}: it is not in the state that transition expects",
            state.as_str()
        ),
    ))
}

/// Renders the `{ "code", "message", "details" }` body of a job error
/// (ADR-0101), without `request_id`: a job is not a request (ADR-0103,
/// decision 4).
///
/// The shape lives here once for the job table; the server has its own factory
/// for the jobs it keeps in memory.
pub(crate) fn job_error_body(code: ErrorCode, message: &str, details: serde_json::Value) -> String {
    serde_json::json!({
        "code": code.as_str(),
        "message": message,
        "details": details,
    })
    .to_string()
}

/// True when `path` holds no database yet: missing file, or empty file left by
/// a failed creation.
fn is_new_database(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(metadata) => metadata.len() == 0,
        Err(_) => true,
    }
}

/// Creates the schema of a fresh job database and its version.
fn create_schema(connection: &mut Connection) -> Result<(), CribleError> {
    let transaction = connection
        .transaction()
        .map_err(|error| database_error("open the transaction of the job schema", &error))?;
    transaction
        .execute_batch(
            "CREATE TABLE meta (format_version INTEGER NOT NULL);
             CREATE TABLE jobs (
                 id           TEXT PRIMARY KEY,
                 kind         TEXT NOT NULL,
                 \"index\"    TEXT NOT NULL,
                 state        TEXT NOT NULL,
                 created_at   TEXT NOT NULL,
                 started_at   TEXT,
                 finished_at  TEXT,
                 generation   INTEGER,
                 progress     TEXT,
                 result       TEXT,
                 error        TEXT
             );
             CREATE INDEX jobs_listing ON jobs (created_at DESC, id ASC);
             CREATE INDEX jobs_state ON jobs (state);",
        )
        .map_err(|error| database_error("create the job schema", &error))?;
    transaction
        .execute(
            "INSERT INTO meta (format_version) VALUES (?1)",
            params![FORMAT_VERSION],
        )
        .map_err(|error| database_error("write the job format version", &error))?;
    transaction
        .commit()
        .map_err(|error| database_error("commit the job schema", &error))
}

/// Wraps a SQLite failure: a transient lock is `overloaded` (ADR-0038), any
/// other failure is `internal`.
fn database_error(context: &str, error: &rusqlite::Error) -> CribleError {
    crate::records::database_error(context, error)
}
