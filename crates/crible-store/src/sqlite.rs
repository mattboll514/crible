// SPDX-License-Identifier: Apache-2.0

//! Connection settings shared by the SQLite files of an instance.
//!
//! References: ADR-0038 (a short lock wait, no configuration key) and
//! ADR-0103, decision 1 (the job database opens with the settings of
//! `records.sqlite`). `records.sqlite` and `jobs.sqlite` both open their
//! connection through these helpers, so the two files cannot drift apart.
//!
//! The `meta.format_version` check lives here too: every file of the instance
//! is versioned, and an unknown version is refused at open (INV-15).

use std::time::Duration;

use crible_core::error::{CribleError, ErrorCode};
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension};

/// Time a connection waits on a SQLite lock before it fails (ADR-0038).
///
/// One second because a legitimate lock never lasts that long: an attach
/// commits a few thousand rows and a checkpoint takes milliseconds. Waiting
/// longer means two writers compete for the database, which contradicts the
/// "one writer per index" rule and must fail fast. The value stays below
/// `limits.default_timeout_ms = 2000`, so a read never waits past its own
/// deadline.
///
/// This is a constant of the code, not a configuration key: no instance should
/// have to tune it (ADR-0038, rule 12 of `AGENTS.md`).
pub(crate) const BUSY_TIMEOUT_MS: u64 = 1000;

/// Applies the lock wait of ADR-0038 to `connection`.
///
/// Every connection of the instance goes through here: the writer and the
/// readers of `records.sqlite`, and the one connection of the job table. The
/// setting is per connection, so it must be applied each time one is opened.
pub(crate) fn configure_busy_timeout(connection: &Connection) -> Result<(), CribleError> {
    connection
        .busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))
        .map_err(|error| crate::records::database_error("set the SQLite busy timeout", &error))
}

/// Switches the database to WAL, so readers never block the writer (INV-02).
///
/// `what` names the file in the message (`records`, `job`).
pub(crate) fn enable_wal(connection: &Connection, what: &str) -> Result<(), CribleError> {
    let mode: String = connection
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .map_err(|error| {
            crate::records::database_error(
                &format!("enable WAL mode on the {what} database"),
                &error,
            )
        })?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(CribleError::new(
            ErrorCode::Internal,
            format!("the {what} database refuses WAL mode, journal_mode is '{mode}'"),
        ));
    }
    Ok(())
}

/// Whether `connection` holds a `meta` table.
pub(crate) fn has_meta_table(connection: &Connection, what: &str) -> Result<bool, CribleError> {
    let present: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'meta'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| {
            crate::records::database_error(&format!("inspect the {what} database"), &error)
        })?;
    Ok(present != 0)
}

/// Reads `meta.format_version` and refuses anything but `expected` (INV-15).
///
/// A missing `meta` table, a missing row, a non-integer value and an unknown
/// version are all `invalid_request` errors naming `format_version`. `what`
/// names the file in the messages (`records`, `job`).
pub(crate) fn check_format_version(
    connection: &Connection,
    expected: u32,
    what: &str,
) -> Result<u32, CribleError> {
    if !has_meta_table(connection, what)? {
        return Err(CribleError::new(
            ErrorCode::InvalidRequest,
            format!(
                "the {what} database has no meta table, so no format_version: \
                 it is not a {what} database of this build"
            ),
        ));
    }
    let stored = connection
        .query_row("SELECT format_version FROM meta", [], |row| {
            row.get::<_, SqlValue>(0)
        })
        .optional()
        .map_err(|error| {
            crate::records::database_error(
                &format!("read the format_version of the {what} database"),
                &error,
            )
        })?;

    let Some(stored) = stored else {
        return Err(CribleError::new(
            ErrorCode::InvalidRequest,
            format!("meta is empty: the {what} database has no format_version row"),
        ));
    };
    let SqlValue::Integer(stored) = stored else {
        return Err(CribleError::new(
            ErrorCode::InvalidRequest,
            format!(
                "meta.format_version of the {what} database is not an integer, found {stored:?}; \
                 this build reads format_version {expected}"
            ),
        ));
    };
    let version = u32::try_from(stored).map_err(|_| {
        CribleError::new(
            ErrorCode::InvalidRequest,
            format!(
                "meta.format_version {stored} of the {what} database is out of range; \
                 this build reads format_version {expected}"
            ),
        )
    })?;
    if version != expected {
        return Err(CribleError::new(
            ErrorCode::InvalidRequest,
            format!(
                "unsupported {what} format_version {version}; \
                 this build reads format_version {expected}"
            ),
        ));
    }
    Ok(version)
}
