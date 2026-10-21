// SPDX-License-Identifier: Apache-2.0

//! Acceptance tests of `crible_store::records`: text and fields round-trip
//! through the store, canonical JSON is stable (ADR-0019), text is zstd
//! compressed, and the documented read queries behave.
//!
//! References: `docs/02-architecture.md` ("Tables SQLite"),
//! `docs/adr/0019-champs-en-json-canonique.md` and the
//! `STOCKER-ENREGISTREMENTS-SQLITE` task of
//! `docs/phases/P1-stockage-et-recherche-exacte.md`.
//!
//! The raw-column assertions read `records.sqlite` with `rusqlite` directly.
//! They check what is actually written on disk, not how the store builds it.

use std::path::Path;

use crible_core::ids::{Generation, Iid};
use crible_store::records::{DocumentRecord, PassageRecord, RecordStore};
use rusqlite::Connection;
use serde_json::{Value, json};

/// zstd frame magic number, little-endian on disk: `28 B5 2F FD`.
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

fn raw_connection(path: &Path) -> Connection {
    Connection::open(path).unwrap_or_else(|error| panic!("cannot open {path:?}: {error}"))
}

fn raw_bytes(path: &Path, sql: &str, key: impl rusqlite::ToSql) -> Vec<u8> {
    raw_connection(path)
        .query_row(sql, [key], |row| row.get(0))
        .unwrap_or_else(|error| panic!("raw read failed for {sql}: {error}"))
}

fn raw_string(path: &Path, sql: &str, key: impl rusqlite::ToSql) -> String {
    raw_connection(path)
        .query_row(sql, [key], |row| row.get(0))
        .unwrap_or_else(|error| panic!("raw read failed for {sql}: {error}"))
}

fn document(doc_id: &str, fields: Value) -> DocumentRecord {
    DocumentRecord {
        doc_id: doc_id.to_owned(),
        doc_hash: format!("blake3-of-{doc_id}"),
        fields,
        updated_generation: Generation::new(1),
    }
}

fn passage(iid: u32, doc_id: &str, ord: u32, text: &str) -> PassageRecord {
    PassageRecord {
        iid: Iid::new(iid),
        doc_id: doc_id.to_owned(),
        ord,
        text: text.to_owned(),
        passage_hash: format!("blake3-of-{doc_id}#{ord}"),
        fields: json!({ "partie": "corps" }),
        segment_id: "segment-0001".to_owned(),
        created_generation: Generation::new(1),
        dead_since_gen: None,
    }
}

#[test]
fn document_fields_roundtrip_through_the_store() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");
    let fields = json!({
        "juridiction": "Cour de cassation — chambre sociale",
        "formation": "chambre sociale",
        "numero": "21-12.345",
        "date_decision": "2026-10-20",
        "titre": "Arrêt n° 2026-001 — 日本語 😀",
        "publie": true,
        "precision": 0.999,
        "annexe": null,
        "meta": { "source": "legifacile", "rang": 3_i64 }
    });

    let mut store = RecordStore::create(&path).expect("the database is created");
    store
        .write_document(&document("LEGIARTI000012345678", fields.clone()))
        .expect("the document is written");
    let read = store
        .read_document("LEGIARTI000012345678")
        .expect("the document is read")
        .expect("the document exists");

    assert_eq!(read.fields, fields, "fields survive the round-trip");
    assert_eq!(read.doc_id, "LEGIARTI000012345678");
    assert_eq!(read.doc_hash, "blake3-of-LEGIARTI000012345678");
    assert_eq!(read.updated_generation, Generation::new(1));
}

#[test]
fn passage_text_roundtrip_through_the_store() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");
    let samples = [
        "",
        "Article 1er — l'arrêté prévoit la publication au Journal officiel.",
        "Ligne 1\nLigne 2\r\nLigne 3\tTabulation",
        "日本語 😀 e\u{301}",
        "a",
    ];

    let mut store = RecordStore::create(&path).expect("the database is created");
    for (index, text) in samples.iter().enumerate() {
        let iid = u32::try_from(index).expect("the sample index fits in u32");
        store
            .write_passage(&passage(iid, "doc-1", iid, text))
            .expect("the passage is written");
        let read = store
            .read_passage(Iid::new(iid))
            .expect("the passage is read")
            .expect("the passage exists");

        assert_eq!(&read.text, text, "text survives the round-trip");
        assert_eq!(read.ord, iid);
        assert_eq!(read.doc_id, "doc-1");
        assert_eq!(read.segment_id, "segment-0001");
    }
}

#[test]
fn passage_text_is_stored_as_a_zstd_frame() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");
    let text = "Le présent arrêté entre en vigueur le lendemain de sa publication. ".repeat(40);

    {
        let mut store = RecordStore::create(&path).expect("the database is created");
        store
            .write_passage(&passage(7, "doc-1", 0, &text))
            .expect("the passage is written");
    }

    let raw = raw_bytes(&path, "SELECT text FROM passages WHERE iid = ?1", 7i64);
    assert!(
        raw.starts_with(&ZSTD_MAGIC),
        "passages.text must be a zstd frame, got {:?}",
        &raw[..raw.len().min(4)]
    );
    assert!(
        raw.len() < text.len() / 2,
        "a repetitive text of {} bytes compresses to {} bytes",
        text.len(),
        raw.len()
    );

    let decoded = zstd::decode_all(&raw[..]).expect("the stored text decodes as zstd");
    assert_eq!(
        String::from_utf8(decoded).expect("the decoded text is UTF-8"),
        text
    );
}

#[test]
fn document_fields_are_stored_with_sorted_keys() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");

    {
        let mut store = RecordStore::create(&path).expect("the database is created");
        store
            .write_document(&document(
                "doc-1",
                json!({ "titre": "T", "annee": 2026, "juridiction": "CE" }),
            ))
            .expect("the document is written");
    }

    let raw = raw_string(
        &path,
        "SELECT fields FROM documents WHERE doc_id = ?1",
        "doc-1",
    );
    assert_eq!(
        raw, r#"{"annee":2026,"juridiction":"CE","titre":"T"}"#,
        "canonical JSON: keys sorted, no space around separators"
    );
}

#[test]
fn nested_fields_are_stored_with_sorted_keys() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");

    {
        let mut store = RecordStore::create(&path).expect("the database is created");
        store
            .write_passage(&passage(1, "doc-1", 0, "texte"))
            .expect("the passage is written");
    }

    let raw = raw_string(&path, "SELECT fields FROM passages WHERE iid = ?1", 1i64);
    assert_eq!(raw, r#"{"partie":"corps"}"#);

    {
        let mut store = RecordStore::open(&path).expect("the database reopens");
        let mut record = passage(2, "doc-1", 1, "texte");
        record.fields = json!({ "z": { "y": 1, "b": [true, null] }, "a": "x" });
        store
            .write_passage(&record)
            .expect("the passage is written");
    }

    let raw = raw_string(&path, "SELECT fields FROM passages WHERE iid = ?1", 2i64);
    assert_eq!(
        raw, r#"{"a":"x","z":{"b":[true,null],"y":1}}"#,
        "sorting applies at every level, arrays keep their order"
    );
}

#[test]
fn equal_fields_serialize_identically_whatever_the_insertion_order() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");

    {
        let mut store = RecordStore::create(&path).expect("the database is created");
        store
            .write_document(&document(
                "doc-a",
                json!({ "a": 1, "b": { "d": 4, "c": 3 } }),
            ))
            .expect("the first document is written");
        store
            .write_document(&document(
                "doc-b",
                json!({ "b": { "c": 3, "d": 4 }, "a": 1 }),
            ))
            .expect("the second document is written");
    }

    let first = raw_string(
        &path,
        "SELECT fields FROM documents WHERE doc_id = ?1",
        "doc-a",
    );
    let second = raw_string(
        &path,
        "SELECT fields FROM documents WHERE doc_id = ?1",
        "doc-b",
    );
    assert_eq!(
        first, second,
        "the same value must produce the same bytes, whatever the key order"
    );
}

#[test]
fn live_iids_excludes_dead_passages() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");

    {
        let mut store = RecordStore::create(&path).expect("the database is created");
        let mut dead = passage(1, "doc-1", 0, "ancien texte");
        dead.dead_since_gen = Some(Generation::new(2));
        for record in [
            passage(0, "doc-1", 0, "texte"),
            dead,
            passage(2, "doc-2", 0, "texte"),
        ] {
            store
                .write_passage(&record)
                .expect("the passage is written");
        }

        // The order is not part of the contract, only the set is.
        let mut live = store.live_iids().expect("live iids are listed");
        live.sort_unstable();
        assert_eq!(
            live,
            vec![Iid::new(0), Iid::new(2)],
            "a dead passage leaves the live set"
        );
    }
}

#[test]
fn passages_of_document_are_ordered_by_ord() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");

    let mut store = RecordStore::create(&path).expect("the database is created");
    for (iid, ord) in [(10, 2), (11, 0), (12, 1)] {
        store
            .write_passage(&passage(iid, "doc-1", ord, "texte"))
            .expect("the passage is written");
    }
    store
        .write_passage(&passage(20, "doc-2", 0, "autre"))
        .expect("the other passage is written");

    let ords: Vec<u32> = store
        .passages_of_document("doc-1")
        .expect("the passages of the document are read")
        .iter()
        .map(|record| record.ord)
        .collect();
    assert_eq!(ords, vec![0, 1, 2], "passages come back in reading order");
}

#[test]
fn unknown_document_and_iid_read_as_none() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");

    let store = RecordStore::create(&path).expect("the database is created");
    assert!(
        store
            .read_document("absent")
            .expect("the lookup succeeds")
            .is_none(),
        "an unknown doc_id is not an error"
    );
    assert!(
        store
            .read_passage(Iid::new(99))
            .expect("the lookup succeeds")
            .is_none(),
        "an unknown iid is not an error"
    );
}

#[test]
fn documents_and_passages_survive_reopen() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");

    {
        let mut store = RecordStore::create(&path).expect("the database is created");
        store
            .write_document(&document("doc-1", json!({ "titre": "T" })))
            .expect("the document is written");
        store
            .write_passage(&passage(3, "doc-1", 0, "texte accentué"))
            .expect("the passage is written");
    }

    let store = RecordStore::open(&path).expect("the database reopens");
    let document = store
        .read_document("doc-1")
        .expect("the document is read")
        .expect("the document survived the reopen");
    assert_eq!(document.fields, json!({ "titre": "T" }));
    let passage = store
        .read_passage(Iid::new(3))
        .expect("the passage is read")
        .expect("the passage survived the reopen");
    assert_eq!(passage.text, "texte accentué");
}

#[test]
fn database_uses_wal_journal_mode() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("records.sqlite");

    {
        RecordStore::create(&path).expect("the database is created");
    }

    let mode: String = raw_connection(&path)
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap_or_else(|error| panic!("journal_mode is readable: {error}"));
    assert_eq!(mode, "wal", "readers must not block writers (INV-02)");
}
