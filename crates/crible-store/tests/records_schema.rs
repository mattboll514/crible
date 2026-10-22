// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! Acceptance tests of the schema created by `RecordStore::create`: the five
//! tables and the two documented indexes of `docs/02-architecture.md`
//! ("Tables SQLite") exist, `meta` has its single row, and a database of an
//! unknown version is refused before anything is written to it (INV-15).
//!
//! References: `docs/02-architecture.md`, and the
//! `STOCKER-ENREGISTREMENTS-SQLITE` task of
//! `docs/phases/P1-stockage-et-recherche-exacte.md`.

use std::path::Path;

use crible_core::error::ErrorCode;
use crible_core::ids::Generation;
use crible_store::records::{DocumentRecord, FORMAT_VERSION, RecordStore};
use rusqlite::Connection;
use serde_json::json;

/// Names of the objects of the given type declared in the database.
fn schema_names(path: &Path, kind: &str) -> Vec<String> {
    let connection =
        Connection::open(path).unwrap_or_else(|error| panic!("cannot open {path:?}: {error}"));
    let mut statement = connection
        .prepare("SELECT name FROM sqlite_schema WHERE type = ?1 ORDER BY name")
        .unwrap_or_else(|error| panic!("cannot prepare the schema query: {error}"));
    statement
        .query_map([kind], |row| row.get::<_, String>(0))
        .unwrap_or_else(|error| panic!("cannot query the schema objects: {error}"))
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|error| panic!("cannot read the schema objects: {error}"))
}

#[test]
fn create_writes_the_five_tables_of_the_design() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");
    {
        RecordStore::create(&path).expect("the database is created");
    }

    assert_eq!(
        schema_names(&path, "table"),
        vec![
            "batches".to_owned(),
            "documents".to_owned(),
            "meta".to_owned(),
            "passages".to_owned(),
            "segments".to_owned(),
        ],
        "the schema must hold exactly the documented tables"
    );
}

#[test]
fn create_writes_the_documented_indexes() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");
    {
        RecordStore::create(&path).expect("the database is created");
    }

    // Indexes declared by the design, plus the implicit ones SQLite creates for
    // the primary keys of `batches`, `documents` and `segments`.
    let names = schema_names(&path, "index");
    for expected in ["passages_doc_id", "passages_dead_since_gen"] {
        assert!(
            names.iter().any(|name| name == expected),
            "index {expected} is missing, found {names:?}"
        );
    }
}

#[test]
fn create_writes_one_meta_row_with_the_documented_columns() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");
    {
        RecordStore::create(&path).expect("the database is created");
    }

    let connection = Connection::open(&path).expect("the database opens");
    let (rows, generation, epoch, format_version, next_iid) = connection
        .query_row(
            "SELECT count(*), max(generation), max(epoch), max(format_version), max(next_iid)
             FROM meta",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .unwrap_or_else(|error| panic!("meta is readable: {error}"));

    assert_eq!(rows, 1, "meta holds a single row");
    assert_eq!(generation, 0, "a new epoch starts at generation 0");
    assert_eq!(epoch, 0, "a new database starts at epoch 0");
    assert_eq!(format_version, i64::from(FORMAT_VERSION));
    assert_eq!(next_iid, 0, "no iid is attributed yet");
}

#[test]
fn create_accepts_an_existing_database_of_the_same_version() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");
    {
        let mut store = RecordStore::create(&path).expect("the database is created");
        store
            .write_document(&DocumentRecord {
                doc_id: "doc-1".to_owned(),
                doc_hash: "blake3-of-doc-1".to_owned(),
                fields: json!({ "titre": "T" }),
                updated_generation: Generation::new(1),
            })
            .expect("the document is written");
    }

    let store = RecordStore::create(&path).expect("the same version is accepted again");
    assert_eq!(store.format_version(), FORMAT_VERSION);
    let document = store
        .read_document("doc-1")
        .expect("the document is read")
        .expect("create kept the existing data");
    assert_eq!(document.fields, json!({ "titre": "T" }));
}

#[test]
fn create_refuses_an_unknown_version_without_touching_the_database() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");
    // A foreign database that only carries the meta table of the design, with a
    // version this build does not know.
    Connection::open(&path)
        .and_then(|connection| {
            connection.execute_batch(
                "CREATE TABLE meta (
                     id             INTEGER PRIMARY KEY CHECK (id = 1),
                     generation     INTEGER NOT NULL DEFAULT 0,
                     epoch          INTEGER NOT NULL DEFAULT 0,
                     format_version INTEGER NOT NULL,
                     next_iid       INTEGER NOT NULL DEFAULT 0
                 );
                 INSERT INTO meta (id, generation, epoch, format_version, next_iid)
                 VALUES (1, 0, 0, 999, 0);",
            )
        })
        .unwrap_or_else(|error| panic!("cannot build the foreign database: {error}"));

    let error = match RecordStore::create(&path) {
        Ok(_) => panic!("an unknown format version must be refused"),
        Err(error) => error,
    };
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(
        error.message().contains("999"),
        "message {:?} must name the version read",
        error.message()
    );
    assert_eq!(
        schema_names(&path, "table"),
        vec!["meta".to_owned()],
        "create must not add its tables to a database of another version"
    );
}

#[test]
fn open_refuses_a_database_without_meta_table() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");
    Connection::open(&path)
        .and_then(|connection| connection.execute_batch("CREATE TABLE unrelated (x INTEGER)"))
        .unwrap_or_else(|error| panic!("cannot build the foreign database: {error}"));

    let error = match RecordStore::open(&path) {
        Ok(_) => panic!("a database without a meta table must be refused"),
        Err(error) => error,
    };
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(
        error.message().contains("format_version"),
        "message {:?} must name format_version",
        error.message()
    );
}

#[test]
fn open_reasserts_wal_mode() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");
    {
        RecordStore::create(&path).expect("the database is created");
    }

    let restored = Connection::open(&path).expect("the database opens");
    let mode: String = restored
        .query_row("PRAGMA journal_mode = DELETE", [], |row| row.get(0))
        .unwrap_or_else(|error| panic!("cannot switch the journal mode: {error}"));
    assert_eq!(mode.to_ascii_lowercase(), "delete");
    drop(restored);

    {
        let store = RecordStore::open(&path).expect("the database reopens");
        assert_eq!(store.format_version(), FORMAT_VERSION);
    }

    let mode: String = Connection::open(&path)
        .and_then(|connection| connection.query_row("PRAGMA journal_mode", [], |row| row.get(0)))
        .unwrap_or_else(|error| panic!("journal_mode is readable: {error}"));
    assert_eq!(
        mode.to_ascii_lowercase(),
        "wal",
        "opening the store must restore the WAL mode (INV-02)"
    );
}
