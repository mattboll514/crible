// SPDX-License-Identifier: Apache-2.0

//! INV-15 — every on-disk format is explicitly little-endian and versioned.
//!
//! For `records.sqlite`, the invariant is the format version: it is written in
//! `meta` when the database is created, checked on open, and an unknown version
//! is refused with a typed error.
//!
//! References: `docs/08-invariants-et-tests.md` (INV-15),
//! `docs/02-architecture.md` ("Tables SQLite") and the
//! `STOCKER-ENREGISTREMENTS-SQLITE` task of
//! `docs/phases/P1-stockage-et-recherche-exacte.md`.
//!
//! Assumptions on the schema, taken from `docs/02-architecture.md`, which lists
//! `generation`, `epoch`, `format_version` and `next_iid` as the columns of
//! `meta`:
//!
//! - `meta` holds a single row and `format_version` is a column of it;
//! - every write of `RecordStore` is committed before the call returns, so a
//!   second `rusqlite` connection opened by the test observes it.

use std::path::Path;

use crible_core::error::ErrorCode;
use crible_store::records::{FORMAT_VERSION, RecordStore};
use rusqlite::Connection;
use rusqlite::types::Value;

/// Opens the raw database file, the way an external tool would.
fn raw_connection(path: &Path) -> Connection {
    Connection::open(path).unwrap_or_else(|error| panic!("cannot open {path:?}: {error}"))
}

/// Reads `meta.format_version` without going through `RecordStore`.
fn raw_format_version(path: &Path) -> Value {
    raw_connection(path)
        .query_row("SELECT format_version FROM meta", [], |row| row.get(0))
        .unwrap_or_else(|error| panic!("meta.format_version is readable: {error}"))
}

/// Rewrites `meta.format_version`, bypassing the format check on purpose.
///
/// Returns the number of changed rows, so a test can assert that the mutation
/// really happened instead of passing vacuously.
fn set_raw_format_version(path: &Path, value: &str) -> usize {
    raw_connection(path)
        .execute("UPDATE meta SET format_version = ?1", [value])
        .unwrap_or_else(|error| {
            panic!(
                "the test must be able to write a raw format_version into meta; \
                 if the column is declared STRICT or NOT NULL with a numeric type, \
                 the fixture needs another way to corrupt it: {error}"
            )
        })
}

/// Asserts that opening `path` is refused with the expected code and message.
fn assert_open_rejected(path: &Path, needles: &[&str]) {
    let error = match RecordStore::open(path) {
        Ok(_) => panic!("a database without a usable format_version must be refused"),
        Err(error) => error,
    };

    assert_eq!(
        error.code(),
        ErrorCode::InvalidRequest,
        "wrong error code: {}",
        error.message()
    );
    for needle in needles {
        assert!(
            error.message().contains(needle),
            "message {:?} must contain {needle:?}",
            error.message()
        );
    }
}

#[test]
fn inv_15_created_database_records_current_format_version() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");

    {
        let store = RecordStore::create(&path).expect("the database is created");
        assert_eq!(store.format_version(), FORMAT_VERSION);
    }

    assert_eq!(
        raw_format_version(&path),
        Value::Integer(i64::from(FORMAT_VERSION)),
        "meta.format_version must hold the version of this format"
    );
}

#[test]
fn inv_15_created_database_reopens_without_error() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");

    {
        RecordStore::create(&path).expect("the database is created");
    }

    let reopened = RecordStore::open(&path).expect("a database of this version reopens");
    assert_eq!(reopened.format_version(), FORMAT_VERSION);
}

#[test]
fn inv_15_unknown_format_version_is_rejected_on_open() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");

    {
        RecordStore::create(&path).expect("the database is created");
    }

    let changed = set_raw_format_version(&path, "999");
    assert_eq!(
        changed, 1,
        "the fixture must change the single meta row of the database"
    );

    assert_open_rejected(&path, &["999", &FORMAT_VERSION.to_string()]);
}

#[test]
fn inv_15_missing_format_version_is_rejected_on_open() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");

    {
        RecordStore::create(&path).expect("the database is created");
    }

    let deleted = raw_connection(&path)
        .execute("DELETE FROM meta", [])
        .unwrap_or_else(|error| panic!("the test must be able to empty meta: {error}"));
    assert_eq!(deleted, 1, "the fixture must empty the meta table");

    assert_open_rejected(&path, &["format_version"]);
}

#[test]
fn inv_15_non_numeric_format_version_is_rejected_on_open() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");

    {
        RecordStore::create(&path).expect("the database is created");
    }

    let changed = set_raw_format_version(&path, "v1");
    assert_eq!(changed, 1, "the fixture must change the meta row");

    assert_open_rejected(&path, &["format_version"]);
}
