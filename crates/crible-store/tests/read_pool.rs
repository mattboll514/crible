// SPDX-License-Identifier: Apache-2.0

//! Acceptance tests of `crible_store::records::ReadPool`: several readers use
//! the database at the same time through the pool, and a commit of the writer
//! is visible to the next reader.
//!
//! References: `docs/02-architecture.md` ("Concurrence": payloads come from a
//! read connection pool, readers are never blocked by the writer),
//! `docs/08-invariants-et-tests.md` (INV-02) and the
//! `STOCKER-ENREGISTREMENTS-SQLITE` task of
//! `docs/phases/P1-stockage-et-recherche-exacte.md`.

use std::sync::{Barrier, mpsc};
use std::thread;
use std::time::Duration;

use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::Generation;
use crible_store::records::{DocumentRecord, ReadPool, RecordStore};
use rusqlite::Connection;
use serde_json::{Value, json};

fn document(doc_id: &str, fields: Value, generation: u64) -> DocumentRecord {
    DocumentRecord {
        doc_id: doc_id.to_owned(),
        doc_hash: format!("blake3-of-{doc_id}"),
        fields,
        updated_generation: Generation::new(generation),
    }
}

/// Reads the canonical `documents.fields` of `doc_id` on `connection`.
fn fields_of(connection: &Connection, doc_id: &str) -> Result<String, CribleError> {
    connection
        .query_row(
            "SELECT fields FROM documents WHERE doc_id = ?1",
            [doc_id],
            |row| row.get(0),
        )
        .map_err(|error| {
            CribleError::new(
                ErrorCode::Internal,
                format!("cannot read the fields of {doc_id}: {error}"),
            )
        })
}

/// Reads the canonical `documents.fields` of `doc_id` through the pool.
fn read_fields(pool: &ReadPool, doc_id: &str) -> Result<String, CribleError> {
    pool.with_connection(|connection| fields_of(connection, doc_id))
}

#[test]
fn read_pool_serves_two_readers_at_the_same_time() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");
    {
        let mut store = RecordStore::create(&path).expect("the database is created");
        store
            .write_document(&document("doc-a", json!({ "annee": 2026 }), 1))
            .expect("the first document is written");
        store
            .write_document(&document("doc-b", json!({ "titre": "T" }), 1))
            .expect("the second document is written");
    }

    let pool = ReadPool::new(&path, 2).expect("the pool is created");

    // Neither reader releases its connection before the other one has taken
    // one, so both connections are checked out at the same time.
    let barrier = Barrier::new(2);
    let read = |doc_id: &str| {
        pool.with_connection(|connection| {
            barrier.wait();
            fields_of(connection, doc_id)
        })
    };
    let (first, second) = thread::scope(|scope| {
        let first = scope.spawn(|| read("doc-a"));
        let second = scope.spawn(|| read("doc-b"));
        (first.join(), second.join())
    });

    let first = first
        .expect("the first reader finishes")
        .expect("the first read succeeds");
    let second = second
        .expect("the second reader finishes")
        .expect("the second read succeeds");
    assert_eq!(first, r#"{"annee":2026}"#);
    assert_eq!(second, r#"{"titre":"T"}"#);
}

#[test]
fn read_pool_sees_the_writes_committed_before_the_read() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");

    let mut store = RecordStore::create(&path).expect("the database is created");
    let pool = ReadPool::new(&path, 1).expect("the pool is created");

    store
        .write_document(&document("doc-1", json!({ "titre": "avant" }), 1))
        .expect("the document is written");
    assert_eq!(
        read_fields(&pool, "doc-1").expect("the first read succeeds"),
        r#"{"titre":"avant"}"#
    );

    store
        .write_document(&document("doc-1", json!({ "titre": "apres" }), 2))
        .expect("the document is rewritten");
    assert_eq!(
        read_fields(&pool, "doc-1").expect("the second read succeeds"),
        r#"{"titre":"apres"}"#,
        "a write is committed before it returns"
    );
}

#[test]
fn read_pool_waits_when_its_only_connection_is_busy() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");
    {
        let mut store = RecordStore::create(&path).expect("the database is created");
        store
            .write_document(&document("doc-a", json!({ "annee": 2026 }), 1))
            .expect("the document is written");
    }

    let pool = ReadPool::new(&path, 1).expect("the pool is created");
    let (held_tx, held_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();

    thread::scope(|scope| {
        let pool_ref = &pool;
        let holder = scope.spawn(move || {
            pool_ref.with_connection(|_| {
                held_tx
                    .send(())
                    .expect("the reader announces it holds the slot");
                release_rx
                    .recv_timeout(Duration::from_secs(5))
                    .expect("the test releases the slot");
                Ok(())
            })
        });
        let pool_ref = &pool;
        let waiter = scope.spawn(move || {
            done_tx
                .send(read_fields(pool_ref, "doc-a"))
                .expect("the second reader reports its result");
        });

        held_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the first reader holds the only connection");
        // A bounded pool has no second connection to give: the waiter stays on
        // the condition variable until the slot comes back.
        let early = done_rx.recv_timeout(Duration::from_millis(200));
        release_tx.send(()).expect("the first reader is released");
        let late = done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the second reader proceeds once the connection is back");

        assert!(
            early.is_err(),
            "a pool of capacity 1 must not serve two readers at once"
        );
        assert_eq!(late.expect("the second read succeeds"), r#"{"annee":2026}"#);

        holder
            .join()
            .expect("the first reader finishes")
            .expect("the first read succeeds");
        waiter.join().expect("the second reader finishes");
    });
}

#[test]
fn pooled_connections_cannot_write() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");
    RecordStore::create(&path).expect("the database is created");

    let pool = ReadPool::new(&path, 1).expect("the pool is created");
    let write = pool.with_connection(|connection| {
        connection
            .execute("DELETE FROM documents", [])
            .map_err(|error| {
                CribleError::new(
                    ErrorCode::Internal,
                    format!("the pool attempted a write: {error}"),
                )
            })
            .map(|_| ())
    });

    assert!(
        write.is_err(),
        "the pool must hand out read-only connections, never a second writer"
    );
}

#[test]
fn read_pool_refuses_an_empty_capacity() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");
    RecordStore::create(&path).expect("the database is created");

    let error = match ReadPool::new(&path, 0) {
        Ok(_) => panic!("a pool without connection must be refused"),
        Err(error) => error,
    };
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
}
