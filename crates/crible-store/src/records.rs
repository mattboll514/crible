// SPDX-License-Identifier: Apache-2.0

//! Reference state of one epoch: `records.sqlite`.
//!
//! The schema is the one of `docs/02-architecture.md` ("Tables SQLite"): `meta`,
//! `batches`, `segments`, `documents` and `passages`, plus the index on
//! `passages(doc_id)` and the one on `passages(dead_since_gen)`. Fields are
//! stored as canonical JSON (ADR-0019), passage texts as zstd frames and vectors
//! live in segments, not here.
//!
//! Every write commits before the call returns, so another connection of the
//! same WAL database observes it. Reads go through [`RecordStore`] or through a
//! [`ReadPool`], so payload reads never wait on the writer (INV-02).
//!
//! Every connection waits up to 1 000 ms on a SQLite lock and reports a
//! remaining contention as `overloaded`, not `internal` (ADR-0038).
//!
//! **The store commits each write on its own.** A change of the visible state
//! must therefore be written inside one transaction opened by the caller, which
//! belongs to the generation protocol: attach a batch, apply the tombstones,
//! move `meta.generation` and `meta.next_iid` in the same transaction, and commit
//! once (`GERER-GENERATIONS`, `ATTACHER-LOT`, ADR-0016). Without that envelope,
//! a loop over [`RecordStore::write_passage`] would expose a partial generation
//! and break INV-03. `RecordStore::begin` opens that envelope, and the typed
//! helpers `insert_segment`, `insert_passage`, `tombstone_passage` and
//! `advance_meta` write inside it, so the SQL of the commit point stays in
//! this module, the only writer of the database.

use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::{Epoch, Generation, Iid};
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OpenFlags, OptionalExtension, Row, Transaction, params};
use serde_json::{Map, Value};

use crate::segment::SegmentMeta;

/// Version of the `records.sqlite` on-disk format (INV-15).
pub const FORMAT_VERSION: u32 = 1;

/// Compression level used for `passages.text`.
const TEXT_COMPRESSION_LEVEL: i32 = zstd::DEFAULT_COMPRESSION_LEVEL;

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
const BUSY_TIMEOUT_MS: u64 = 1000;

/// Statements building the schema of a new epoch database.
///
/// `meta` holds a single row: its `id` is pinned to 1, which lets
/// `INSERT OR IGNORE` create the row exactly once.
const SCHEMA: [&str; 7] = [
    "CREATE TABLE IF NOT EXISTS meta (
         id             INTEGER PRIMARY KEY CHECK (id = 1),
         generation     INTEGER NOT NULL DEFAULT 0,
         epoch          INTEGER NOT NULL DEFAULT 0,
         format_version INTEGER NOT NULL,
         next_iid       INTEGER NOT NULL DEFAULT 0
     )",
    "CREATE TABLE IF NOT EXISTS batches (
         batch_id        TEXT PRIMARY KEY,
         generation      INTEGER NOT NULL,
         applied_at      TEXT NOT NULL,
         manifest_blake3 TEXT NOT NULL,
         manifest        TEXT NOT NULL,
         report          TEXT NOT NULL
     )",
    "CREATE TABLE IF NOT EXISTS segments (
         segment_id TEXT PRIMARY KEY,
         iid_start  INTEGER NOT NULL,
         count      INTEGER NOT NULL,
         generation INTEGER NOT NULL,
         batch_id   TEXT NOT NULL,
         checksums  TEXT NOT NULL
     )",
    "CREATE TABLE IF NOT EXISTS documents (
         doc_id             TEXT PRIMARY KEY,
         doc_hash           TEXT NOT NULL,
         fields             TEXT NOT NULL,
         updated_generation INTEGER NOT NULL
     )",
    "CREATE TABLE IF NOT EXISTS passages (
         iid                INTEGER PRIMARY KEY,
         doc_id             TEXT NOT NULL,
         ord                INTEGER NOT NULL,
         text               BLOB NOT NULL,
         passage_hash       TEXT NOT NULL,
         fields             TEXT NOT NULL,
         segment_id         TEXT NOT NULL,
         created_generation INTEGER NOT NULL,
         dead_since_gen     INTEGER
     )",
    "CREATE INDEX IF NOT EXISTS passages_doc_id ON passages(doc_id)",
    "CREATE INDEX IF NOT EXISTS passages_dead_since_gen ON passages(dead_since_gen)",
];

/// One row of `documents`.
///
/// `doc_id` stays a `String`: a row mirrors the external identifier exactly as
/// the producer sent it. Validation (`DocId`) happens once, at the ingestion
/// boundary, and every query accepts a `&str`.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentRecord {
    /// External identifier of the document, the only identifier visible outside
    /// the engine.
    pub doc_id: String,
    /// Hash of the document content, used to detect a real republication.
    pub doc_hash: String,
    /// Document fields, stored as canonical JSON.
    pub fields: Value,
    /// Generation that last wrote the document or its fields.
    pub updated_generation: Generation,
}

/// One row of `batches`, as the idempotence check of an attach reads it.
///
/// Only what a replay needs is kept: the digest that identifies its manifest
/// and the report of the original attach (INV-10, ADR-0041). `applied_at` names
/// the wall clock, which no decision depends on, and the generation of the
/// batch is read from the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredBatch {
    /// `blake3:<hex>` digest of the bytes of its `manifest.json` (ADR-0041).
    pub manifest_blake3: String,
    /// Report of the original attach, as JSON.
    pub report: String,
}

/// One row of `passages`.
#[derive(Debug, Clone, PartialEq)]
pub struct PassageRecord {
    /// Internal identifier, never exposed outside the engine (INV-01).
    pub iid: Iid,
    /// External identifier of the document this passage belongs to.
    pub doc_id: String,
    /// Position of the passage in its document, from 0.
    pub ord: u32,
    /// Exact encoder input, also indexed in BM25. Stored as a zstd frame.
    pub text: String,
    /// Hash of the passage content.
    pub passage_hash: String,
    /// Passage fields, stored as canonical JSON.
    pub fields: Value,
    /// Segment holding the reference vectors of this passage.
    pub segment_id: String,
    /// Generation that created the passage.
    pub created_generation: Generation,
    /// Generation that marked the passage dead, or `None` while it is alive.
    pub dead_since_gen: Option<Generation>,
}

/// Values of the single `meta` row, the authority on the visible generation.
///
/// `next_iid` is the first internal identifier the next attach may hand out:
/// an iid is never reused within an epoch (`docs/02-architecture.md`,
/// "Identifiants").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Meta {
    /// Generation last committed by [`RecordStore::begin`] callers.
    pub generation: Generation,
    /// Epoch the database belongs to, mirrored in `CURRENT_EPOCH`.
    pub epoch: Epoch,
    /// First internal identifier still free in this epoch.
    pub next_iid: u32,
}

/// Owner of the reference database of an epoch, and the only writer of it.
#[derive(Debug)]
pub struct RecordStore {
    connection: Connection,
    format_version: u32,
}

impl RecordStore {
    /// Creates the database at `path` if needed, writes the schema, enables WAL
    /// and records [`FORMAT_VERSION`].
    ///
    /// An existing database is accepted only when its format version is the one
    /// of this build; an unknown version, or a file that is not a records
    /// database, is refused before anything is written to it.
    pub fn create(path: &Path) -> Result<Self, CribleError> {
        let connection = if is_new_database(path) {
            open_or_create(path)?
        } else {
            let connection = open_writable(path)?;
            // Refuse before the DDL: a foreign database must not be extended
            // with our tables on the way out.
            check_format_version(&connection)?;
            connection
        };
        enable_wal(&connection)?;
        for statement in SCHEMA {
            connection
                .execute(statement, [])
                .map_err(|error| database_error("create the records schema", &error))?;
        }
        connection
            .execute(
                "INSERT OR IGNORE INTO meta (id, generation, epoch, format_version, next_iid)
                 VALUES (1, 0, 0, ?1, 0)",
                params![i64::from(FORMAT_VERSION)],
            )
            .map_err(|error| database_error("record the format version", &error))?;
        let format_version = check_format_version(&connection)?;
        Ok(Self {
            connection,
            format_version,
        })
    }

    /// Opens an existing database, refusing a format version this build does not
    /// know (INV-15), and reasserts WAL mode so readers never block the writer
    /// (INV-02).
    ///
    /// A `SQLITE_BUSY` waits on the lock for the busy timeout of ADR-0038
    /// (1 000 ms) and is then reported as `overloaded`, not `internal`: the
    /// contention is transient and the caller may retry.
    pub fn open(path: &Path) -> Result<Self, CribleError> {
        let connection = open_writable(path)?;
        let format_version = check_format_version(&connection)?;
        enable_wal(&connection)?;
        Ok(Self {
            connection,
            format_version,
        })
    }

    /// Format version stored in `meta.format_version`.
    #[must_use]
    pub fn format_version(&self) -> u32 {
        self.format_version
    }

    /// Inserts or replaces one document, and commits before returning.
    pub fn write_document(&mut self, record: &DocumentRecord) -> Result<(), CribleError> {
        upsert_document(&self.connection, record)
    }

    /// Reads one document, or `None` when the `doc_id` is unknown.
    pub fn read_document(&self, doc_id: &str) -> Result<Option<DocumentRecord>, CribleError> {
        document_by_id(&self.connection, doc_id)
    }

    /// Inserts or replaces one passage, and commits before returning.
    pub fn write_passage(&mut self, record: &PassageRecord) -> Result<(), CribleError> {
        let text = encode_text(&record.text)?;
        let fields = canonical_json(&record.fields)?;
        let dead_since_gen = record
            .dead_since_gen
            .map(|generation| to_sql_u64(generation.get(), "dead_since_gen"))
            .transpose()?;
        self.connection
            .execute(
                "INSERT INTO passages (iid, doc_id, ord, text, passage_hash, fields,
                                       segment_id, created_generation, dead_since_gen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(iid) DO UPDATE SET
                     doc_id = excluded.doc_id,
                     ord = excluded.ord,
                     text = excluded.text,
                     passage_hash = excluded.passage_hash,
                     fields = excluded.fields,
                     segment_id = excluded.segment_id,
                     created_generation = excluded.created_generation,
                     dead_since_gen = excluded.dead_since_gen",
                params![
                    i64::from(record.iid.get()),
                    record.doc_id.as_str(),
                    i64::from(record.ord),
                    text,
                    record.passage_hash.as_str(),
                    fields,
                    record.segment_id.as_str(),
                    to_sql_u64(record.created_generation.get(), "created_generation")?,
                    dead_since_gen,
                ],
            )
            .map_err(|error| database_error("write a passage", &error))?;
        Ok(())
    }

    /// Reads one passage by internal identifier, or `None` when it is unknown.
    ///
    /// Dead passages are returned too: this is the row-level view used for
    /// inspection and export.
    pub fn read_passage(&self, iid: Iid) -> Result<Option<PassageRecord>, CribleError> {
        let row = self
            .connection
            .query_row(
                "SELECT iid, doc_id, ord, text, passage_hash, fields, segment_id,
                        created_generation, dead_since_gen
                 FROM passages WHERE iid = ?1",
                params![i64::from(iid.get())],
                passage_row,
            )
            .optional()
            .map_err(|error| database_error("read a passage", &error))?;
        row.map(passage_from_row).transpose()
    }

    /// Passages of one document, in reading order.
    ///
    /// Dead passages are included on purpose: this is the inspection and export
    /// view of a document, the one used to explain why a passage is absent from
    /// the results. Search results only range over the live passages, as
    /// [`RecordStore::live_iids`] defines them.
    pub fn passages_of_document(&self, doc_id: &str) -> Result<Vec<PassageRecord>, CribleError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT iid, doc_id, ord, text, passage_hash, fields, segment_id,
                        created_generation, dead_since_gen
                 FROM passages WHERE doc_id = ?1 ORDER BY ord",
            )
            .map_err(|error| database_error("prepare the passages query", &error))?;
        let rows = statement
            .query_map(params![doc_id], passage_row)
            .map_err(|error| database_error("query the passages of a document", &error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| database_error("read the passages of a document", &error))?;
        rows.into_iter().map(passage_from_row).collect()
    }

    /// Internal identifiers of the passages that are not dead, in ascending
    /// order.
    pub fn live_iids(&self) -> Result<Vec<Iid>, CribleError> {
        let mut statement = self
            .connection
            .prepare("SELECT iid FROM passages WHERE dead_since_gen IS NULL ORDER BY iid")
            .map_err(|error| database_error("prepare the live iids query", &error))?;
        let iids = statement
            .query_map([], |row| row.get::<_, i64>(0))
            .map_err(|error| database_error("query the live iids", &error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| database_error("read the live iids", &error))?;
        iids.into_iter()
            .map(|iid| from_sql_u32(iid, "iid").map(Iid::new))
            .collect()
    }

    /// Borrows the writable connection of the reference database.
    ///
    /// Only `GenerationStore` uses it, to read the reference state outside a
    /// transaction while it holds the single writer of the index.
    pub(crate) fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Values of the `meta` row of this database.
    pub(crate) fn meta(&self) -> Result<Meta, CribleError> {
        read_meta(&self.connection)
    }

    /// Reads the `batches` row of `batch_id`, or `None` when the batch has
    /// never been applied (INV-10).
    pub(crate) fn batch(&self, batch_id: &str) -> Result<Option<StoredBatch>, CribleError> {
        batch_by_id(&self.connection, batch_id)
    }

    /// Identifiers of the segments SQLite references, in ascending order.
    ///
    /// This is the authoritative set of the visible segments: a directory of
    /// `segments/` that is absent from it is an orphan, left by an aborted
    /// attach (ADR-0016, INV-04).
    pub(crate) fn segment_ids(&self) -> Result<Vec<String>, CribleError> {
        segment_ids(&self.connection)
    }

    /// Records the epoch in `meta.epoch`, at the creation of the database.
    ///
    /// A database without its `meta` row cannot name its epoch, so the update
    /// must touch exactly one row.
    pub(crate) fn set_epoch(&mut self, epoch: Epoch) -> Result<(), CribleError> {
        let changed = self
            .connection
            .execute(
                "UPDATE meta SET epoch = ?1 WHERE id = 1",
                params![epoch.get()],
            )
            .map_err(|error| database_error("record the epoch", &error))?;
        if changed != 1 {
            return Err(CribleError::new(
                ErrorCode::Internal,
                "meta has no row, so the epoch cannot be recorded",
            ));
        }
        Ok(())
    }

    /// Opens the transaction that carries one whole generation change.
    ///
    /// The caller writes the batches, the segments, the documents, the
    /// passages, the tombstones and `meta.generation` through the functions of
    /// this module that take a [`Transaction`], then commits once: that commit
    /// is the single commit point of the index (ADR-0016). Dropping the
    /// transaction, which a returned error or a panic does, rolls it back, so a
    /// failed change leaves the previous generation untouched (INV-03).
    pub(crate) fn begin(&mut self) -> Result<Transaction<'_>, CribleError> {
        self.connection
            .transaction()
            .map_err(|error| database_error("open the commit transaction", &error))
    }
}

/// Reads the `meta` row, which [`RecordStore::create`] always writes.
pub(crate) fn read_meta(connection: &Connection) -> Result<Meta, CribleError> {
    let row = connection
        .query_row(
            "SELECT generation, epoch, next_iid FROM meta WHERE id = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| database_error("read meta", &error))?;
    let Some((generation, epoch, next_iid)) = row else {
        return Err(CribleError::new(
            ErrorCode::Internal,
            "meta is empty: the records database has no generation row",
        ));
    };
    Ok(Meta {
        generation: Generation::new(from_sql_u64(generation, "generation")?),
        epoch: Epoch::new(from_sql_u32(epoch, "epoch")?),
        next_iid: from_sql_u32(next_iid, "next_iid")?,
    })
}

/// Reads the segment identifiers of the `segments` table, in ascending order.
pub(crate) fn segment_ids(connection: &Connection) -> Result<Vec<String>, CribleError> {
    let mut statement = connection
        .prepare("SELECT segment_id FROM segments ORDER BY segment_id")
        .map_err(|error| database_error("prepare the segment ids query", &error))?;
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| database_error("query the segment ids", &error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| database_error("read the segment ids", &error))
}

/// Reads the `batches` row of one batch, or `None` when it was never applied.
pub(crate) fn batch_by_id(
    connection: &Connection,
    batch_id: &str,
) -> Result<Option<StoredBatch>, CribleError> {
    let row = connection
        .query_row(
            "SELECT manifest_blake3, report FROM batches WHERE batch_id = ?1",
            params![batch_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| database_error("read a batch", &error))?;
    row.map(|(manifest_blake3, report)| {
        Ok(StoredBatch {
            manifest_blake3,
            report,
        })
    })
    .transpose()
}

/// Records one applied batch in the transaction of its generation (INV-10).
///
/// `applied_at` is written by SQLite in RFC 3339 UTC: no decision of the engine
/// reads it, and the value is only there for an operator.
pub(crate) fn insert_batch(
    transaction: &Transaction<'_>,
    batch_id: &str,
    generation: Generation,
    manifest_blake3: &str,
    manifest: &str,
    report: &str,
) -> Result<(), CribleError> {
    transaction
        .execute(
            "INSERT INTO batches (batch_id, generation, applied_at, manifest_blake3, manifest, report)
             VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), ?3, ?4, ?5)",
            params![
                batch_id,
                to_sql_u64(generation.get(), "generation")?,
                manifest_blake3,
                manifest,
                report,
            ],
        )
        .map_err(|error| database_error("record the batch in the commit transaction", &error))?;
    Ok(())
}

/// `doc_id`s of the live documents, in ascending order.
///
/// A document row exists only for a living document (ADR-0041), so this is the
/// set a `replace_all` batch compares itself with.
pub(crate) fn document_ids(connection: &Connection) -> Result<Vec<String>, CribleError> {
    let mut statement = connection
        .prepare("SELECT doc_id FROM documents ORDER BY doc_id")
        .map_err(|error| database_error("prepare the document ids query", &error))?;
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| database_error("query the document ids", &error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| database_error("read the document ids", &error))
}

/// Internal identifiers of the live passages of one document, in ascending
/// order.
///
/// Read before the new passages of a replacement are added, so the returned
/// set is exactly the passages the replacement or the deletion must tombstone.
pub(crate) fn live_iids_of_document(
    connection: &Connection,
    doc_id: &str,
) -> Result<Vec<Iid>, CribleError> {
    let mut statement = connection
        .prepare(
            "SELECT iid FROM passages
             WHERE doc_id = ?1 AND dead_since_gen IS NULL ORDER BY iid",
        )
        .map_err(|error| database_error("prepare the live iids of a document", &error))?;
    let iids = statement
        .query_map(params![doc_id], |row| row.get::<_, i64>(0))
        .map_err(|error| database_error("query the live iids of a document", &error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| database_error("read the live iids of a document", &error))?;
    iids.into_iter()
        .map(|iid| from_sql_u32(iid, "iid").map(Iid::new))
        .collect()
}

/// Removes the `documents` row of a deleted document, in the transaction of its
/// generation (ADR-0041).
///
/// A removed document must exist: a missing row means the caller and the
/// reference state disagree, which is an internal error rather than a silent
/// no-op.
pub(crate) fn delete_document(
    transaction: &Transaction<'_>,
    doc_id: &str,
) -> Result<(), CribleError> {
    let changed = transaction
        .execute("DELETE FROM documents WHERE doc_id = ?1", params![doc_id])
        .map_err(|error| database_error("remove a document in the commit transaction", &error))?;
    if changed != 1 {
        return Err(CribleError::new(
            ErrorCode::Internal,
            format!("no document row has doc_id '{doc_id}', so it cannot be removed"),
        ));
    }
    Ok(())
}

/// Records one segment in the transaction of the generation that makes it
/// visible.
pub(crate) fn insert_segment(
    transaction: &Transaction<'_>,
    meta: &SegmentMeta,
) -> Result<(), CribleError> {
    let checksums = serde_json::to_string(&meta.checksums).map_err(|error| {
        CribleError::new(
            ErrorCode::Internal,
            format!(
                "cannot serialize the checksums of segment {}: {error}",
                meta.segment_id
            ),
        )
    })?;
    transaction
        .execute(
            "INSERT INTO segments (segment_id, iid_start, count, generation, batch_id, checksums)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                meta.segment_id.as_str(),
                meta.iid_start.get(),
                meta.count,
                to_sql_u64(meta.generation.get(), "generation")?,
                meta.batch_id.as_str(),
                checksums,
            ],
        )
        .map_err(|error| database_error("record a segment in the commit transaction", &error))?;
    Ok(())
}

/// Inserts or replaces one document inside the transaction of a generation.
///
/// A document row is mutable state, keyed by its external identifier: an attach
/// that only changes the fields of a document rewrites the row, without a new
/// passage (see the table of `docs/02-architecture.md`, "Suppressions, pierres
/// tombales et compaction").
pub(crate) fn insert_document(
    transaction: &Transaction<'_>,
    record: &DocumentRecord,
) -> Result<(), CribleError> {
    upsert_document(transaction, record)
}

/// Inserts one passage inside the transaction of a generation.
///
/// Unlike [`RecordStore::write_passage`], this never overwrites an existing
/// `iid`: within one epoch an iid designates one immutable passage
/// (`docs/02-architecture.md`, "Identifiants"), so reusing one is a bug and the
/// `INSERT` fails on the primary key, which rolls the whole generation back
/// (INV-03).
pub(crate) fn insert_passage(
    transaction: &Transaction<'_>,
    record: &PassageRecord,
) -> Result<(), CribleError> {
    let text = encode_text(&record.text)?;
    let fields = canonical_json(&record.fields)?;
    let dead_since_gen = record
        .dead_since_gen
        .map(|generation| to_sql_u64(generation.get(), "dead_since_gen"))
        .transpose()?;
    transaction
        .execute(
            "INSERT INTO passages (iid, doc_id, ord, text, passage_hash, fields,
                                   segment_id, created_generation, dead_since_gen)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                i64::from(record.iid.get()),
                record.doc_id.as_str(),
                i64::from(record.ord),
                text,
                record.passage_hash.as_str(),
                fields,
                record.segment_id.as_str(),
                to_sql_u64(record.created_generation.get(), "created_generation")?,
                dead_since_gen,
            ],
        )
        .map_err(|error| database_error("record a passage in the commit transaction", &error))?;
    Ok(())
}

/// Marks one live passage as dead at `generation`.
///
/// Only a live passage can be tombstoned: an iid with no live row means the
/// caller and the reference state disagree, which is an internal error, not a
/// silent no-op.
pub(crate) fn tombstone_passage(
    transaction: &Transaction<'_>,
    iid: Iid,
    generation: Generation,
) -> Result<(), CribleError> {
    let changed = transaction
        .execute(
            "UPDATE passages SET dead_since_gen = ?2
             WHERE iid = ?1 AND dead_since_gen IS NULL",
            params![
                i64::from(iid.get()),
                to_sql_u64(generation.get(), "dead_since_gen")?,
            ],
        )
        .map_err(|error| database_error("tombstone a passage in the commit transaction", &error))?;
    if changed != 1 {
        return Err(CribleError::new(
            ErrorCode::Internal,
            format!(
                "no live passage has iid {}, so it cannot be tombstoned at generation {}",
                iid.get(),
                generation.get()
            ),
        ));
    }
    Ok(())
}

/// Moves `meta` to the generation the change publishes.
pub(crate) fn advance_meta(
    transaction: &Transaction<'_>,
    generation: Generation,
    next_iid: u32,
) -> Result<(), CribleError> {
    let changed = transaction
        .execute(
            "UPDATE meta SET generation = ?1, next_iid = ?2 WHERE id = 1",
            params![to_sql_u64(generation.get(), "generation")?, next_iid],
        )
        .map_err(|error| database_error("advance meta in the commit transaction", &error))?;
    if changed != 1 {
        return Err(CribleError::new(
            ErrorCode::Internal,
            "meta has no row, so the generation cannot advance",
        ));
    }
    Ok(())
}

/// Bounded pool of **read-only** connections on one `records.sqlite` file.
///
/// The connections are opened with `SQLITE_OPEN_READ_ONLY`: the pool can only
/// read, so no caller can slip a write past the `&mut self` exclusivity of
/// [`RecordStore`], the single writer of an index and the only holder of the
/// commit point (ADR-0016). Reading a WAL database read-only works because the
/// epoch directory is writable, so SQLite may create the `-shm` file.
///
/// Readers never block the writer and the writer never blocks readers, because
/// the database runs in WAL mode. A connection is opened on demand, up to the
/// capacity of the pool, and returned by the pool when the closure ends.
#[derive(Debug)]
pub struct ReadPool {
    path: PathBuf,
    capacity: usize,
    state: Mutex<PoolState>,
    available: Condvar,
}

/// Connections currently idle, plus the number checked out.
#[derive(Debug, Default)]
struct PoolState {
    idle: Vec<Connection>,
    busy: usize,
}

impl ReadPool {
    /// Creates a pool of at most `capacity` connections to the database at
    /// `path`. No connection is opened until the first read.
    pub fn new(path: &Path, capacity: usize) -> Result<Self, CribleError> {
        if capacity == 0 {
            return Err(CribleError::new(
                ErrorCode::InvalidRequest,
                "the read pool needs at least one connection",
            ));
        }
        Ok(Self {
            path: path.to_path_buf(),
            capacity,
            state: Mutex::new(PoolState::default()),
            available: Condvar::new(),
        })
    }

    /// Runs `read` on a pooled connection and gives the connection back to the
    /// pool afterwards, even when `read` fails.
    ///
    /// If `read` panics, the connection is dropped and its slot is released, so
    /// the pool stays usable; a later read opens a fresh connection.
    pub fn with_connection<T>(
        &self,
        read: impl FnOnce(&Connection) -> Result<T, CribleError>,
    ) -> Result<T, CribleError> {
        let connection = self.checkout()?;
        let mut lease = LeaseGuard {
            pool: self,
            checked_in: false,
        };
        let outcome = read(&connection);
        self.checkin(connection);
        lease.checked_in = true;
        outcome
    }

    /// Waits for a free connection, opening one when the pool is below its
    /// capacity.
    fn checkout(&self) -> Result<Connection, CribleError> {
        let mut state = self.lock_state();
        loop {
            if let Some(connection) = state.idle.pop() {
                state.busy += 1;
                return Ok(connection);
            }
            if state.busy < self.capacity {
                let connection = open_read_only(&self.path)?;
                state.busy += 1;
                return Ok(connection);
            }
            state = self
                .available
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Locks the pool state, recovering from a poisoned lock instead of
    /// panicking.
    fn lock_state(&self) -> MutexGuard<'_, PoolState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Puts the connection back and wakes one waiting reader.
    fn checkin(&self, connection: Connection) {
        let mut state = self.lock_state();
        state.busy -= 1;
        state.idle.push(connection);
        drop(state);
        self.available.notify_one();
    }

    /// Releases the slot of a connection that was lost, so the pool stays
    /// usable after a panic inside a reader.
    fn abandon(&self) {
        let mut state = self.lock_state();
        state.busy -= 1;
        drop(state);
        self.available.notify_one();
    }
}

/// Releases the slot of a checked-out connection unless the reader returned it.
struct LeaseGuard<'a> {
    pool: &'a ReadPool,
    checked_in: bool,
}

impl Drop for LeaseGuard<'_> {
    fn drop(&mut self) {
        if !self.checked_in {
            self.pool.abandon();
        }
    }
}

/// Opens a database for reading and writing, without ever creating it.
fn open_writable(path: &Path) -> Result<Connection, CribleError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|error| database_error("open the records database", &error))?;
    configure_busy_timeout(&connection)?;
    Ok(connection)
}

/// Opens a database for reading only. The pool never writes, so no caller can
/// bypass the single writer of the index.
fn open_read_only(path: &Path) -> Result<Connection, CribleError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| database_error("open the records database for reading", &error))?;
    configure_busy_timeout(&connection)?;
    Ok(connection)
}

/// Opens a database for reading and writing, creating the file when missing.
fn open_or_create(path: &Path) -> Result<Connection, CribleError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )
    .map_err(|error| database_error("create the records database", &error))?;
    configure_busy_timeout(&connection)?;
    Ok(connection)
}

/// Applies the lock wait of ADR-0038 to `connection`.
///
/// Every connection of `records.sqlite` goes through here: the writer of
/// [`RecordStore`] and the readers of [`ReadPool`]. The setting is per
/// connection, so it must be applied each time one is opened.
fn configure_busy_timeout(connection: &Connection) -> Result<(), CribleError> {
    connection
        .busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))
        .map_err(|error| database_error("set the SQLite busy timeout", &error))
}

/// Switches the database to WAL, so readers never block the writer (INV-02).
fn enable_wal(connection: &Connection) -> Result<(), CribleError> {
    let mode: String = connection
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .map_err(|error| database_error("enable WAL mode", &error))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(CribleError::new(
            ErrorCode::Internal,
            format!("the records database refuses WAL mode, journal_mode is '{mode}'"),
        ));
    }
    Ok(())
}

/// Reads `meta.format_version` and refuses anything but [`FORMAT_VERSION`].
///
/// A missing `meta` table, a missing row, a non-integer value and an unknown
/// version are all `invalid_request` errors naming `format_version` (INV-15).
fn check_format_version(connection: &Connection) -> Result<u32, CribleError> {
    if !has_meta_table(connection)? {
        return Err(CribleError::new(
            ErrorCode::InvalidRequest,
            "the database has no meta table, so no format_version: \
             it is not a records database of this build",
        ));
    }
    let stored = connection
        .query_row("SELECT format_version FROM meta", [], |row| {
            row.get::<_, SqlValue>(0)
        })
        .optional()
        .map_err(|error| database_error("read meta.format_version", &error))?;

    let Some(stored) = stored else {
        return Err(CribleError::new(
            ErrorCode::InvalidRequest,
            "meta is empty: the records database has no format_version row",
        ));
    };
    let SqlValue::Integer(stored) = stored else {
        return Err(CribleError::new(
            ErrorCode::InvalidRequest,
            format!(
                "meta.format_version is not an integer, found {0:?}; \
                 this build reads format_version {FORMAT_VERSION}",
                stored
            ),
        ));
    };
    let version = u32::try_from(stored).map_err(|_| {
        CribleError::new(
            ErrorCode::InvalidRequest,
            format!(
                "meta.format_version {stored} is out of range; \
                 this build reads format_version {FORMAT_VERSION}"
            ),
        )
    })?;
    if version != FORMAT_VERSION {
        return Err(CribleError::new(
            ErrorCode::InvalidRequest,
            format!(
                "unsupported records format_version {version}; \
                 this build reads format_version {FORMAT_VERSION}"
            ),
        ));
    }
    Ok(version)
}

/// Wraps a SQLite failure, which is an internal error from the caller's point of
/// view.
///
/// The single exception is a lock contention: `SQLITE_BUSY` and its extended
/// codes (`SQLITE_BUSY_RECOVERY`, `SQLITE_BUSY_SNAPSHOT`, `SQLITE_BUSY_TIMEOUT`)
/// are transient, so they become `overloaded` — the client retries — instead of
/// a fake internal failure (ADR-0038). Every SQLite failure of this module goes
/// through here, so the mapping also covers connection opening and the write
/// transactions of the caller.
pub(crate) fn database_error(context: &str, error: &rusqlite::Error) -> CribleError {
    let code = match error.sqlite_error_code() {
        // `sqlite_error_code` masks the extended result code down to its
        // primary code, so every `SQLITE_BUSY_*` variant lands here.
        Some(rusqlite::ErrorCode::DatabaseBusy) => ErrorCode::Overloaded,
        _ => ErrorCode::Internal,
    };
    CribleError::new(code, format!("cannot {context}: {error}"))
}

/// True when `path` does not hold a database yet: missing file, or empty file
/// left by a failed creation.
fn is_new_database(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(metadata) => metadata.len() == 0,
        Err(_) => true,
    }
}

/// True when the database has a `meta` table.
///
/// Checked before reading `meta`, so a foreign SQLite file is reported as an
/// `invalid_request` instead of a raw SQLite error.
fn has_meta_table(connection: &Connection) -> Result<bool, CribleError> {
    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'meta'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| database_error("inspect the records schema", &error))?;
    Ok(count > 0)
}

/// Converts a counter to the signed integer SQLite stores.
fn to_sql_u64(value: u64, field: &str) -> Result<i64, CribleError> {
    i64::try_from(value).map_err(|_| {
        CribleError::new(
            ErrorCode::InvalidRequest,
            format!("{field} {value} does not fit in a SQLite integer"),
        )
    })
}

/// Converts a non-negative SQLite integer read back from disk.
fn from_sql_u64(value: i64, field: &str) -> Result<u64, CribleError> {
    u64::try_from(value).map_err(|_| {
        CribleError::new(
            ErrorCode::Internal,
            format!("stored {field} {value} is negative"),
        )
    })
}

/// Converts an integer read back from a `u32` column.
pub(crate) fn from_sql_u32(value: i64, field: &str) -> Result<u32, CribleError> {
    u32::try_from(value).map_err(|_| {
        CribleError::new(
            ErrorCode::Internal,
            format!("stored {field} {value} does not fit in u32"),
        )
    })
}

/// Serializes fields as canonical JSON: keys sorted at every level, no space
/// around separators (ADR-0019).
fn canonical_json(fields: &Value) -> Result<String, CribleError> {
    serde_json::to_string(&canonical_value(fields)).map_err(|error| {
        CribleError::new(
            ErrorCode::InvalidRequest,
            format!("cannot serialize fields as JSON: {error}"),
        )
    })
}

/// Rebuilds `value` with the keys of every object in lexicographic order.
///
/// Arrays keep their order: it carries meaning. Sorting is explicit here so the
/// on-disk bytes do not depend on the key order of the input, nor on the map
/// implementation of `serde_json`.
fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries: Vec<(&str, &Value)> = object
                .iter()
                .map(|(key, child)| (key.as_str(), child))
                .collect();
            entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
            let mut canonical = Map::new();
            for (key, child) in entries {
                canonical.insert(key.to_owned(), canonical_value(child));
            }
            Value::Object(canonical)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_value).collect()),
        scalar => scalar.clone(),
    }
}

/// Compresses a passage text into a zstd frame.
fn encode_text(text: &str) -> Result<Vec<u8>, CribleError> {
    zstd::stream::encode_all(text.as_bytes(), TEXT_COMPRESSION_LEVEL).map_err(|error| {
        CribleError::new(
            ErrorCode::Internal,
            format!("cannot compress a passage text: {error}"),
        )
    })
}

/// Decompresses a `passages.text` frame back to UTF-8.
fn decode_text(frame: &[u8]) -> Result<String, CribleError> {
    let bytes = zstd::stream::decode_all(frame).map_err(|error| {
        CribleError::new(
            ErrorCode::Internal,
            format!("cannot decompress a passage text: {error}"),
        )
    })?;
    String::from_utf8(bytes).map_err(|error| {
        CribleError::new(
            ErrorCode::Internal,
            format!("stored passage text is not UTF-8: {error}"),
        )
    })
}

/// Inserts or replaces one `documents` row.
///
/// Shared by the autocommit [`RecordStore::write_document`] and the commit
/// transaction of a generation, so both write the same bytes.
fn upsert_document(connection: &Connection, record: &DocumentRecord) -> Result<(), CribleError> {
    let fields = canonical_json(&record.fields)?;
    connection
        .execute(
            "INSERT INTO documents (doc_id, doc_hash, fields, updated_generation)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(doc_id) DO UPDATE SET
                 doc_hash = excluded.doc_hash,
                 fields = excluded.fields,
                 updated_generation = excluded.updated_generation",
            params![
                record.doc_id.as_str(),
                record.doc_hash.as_str(),
                fields,
                to_sql_u64(record.updated_generation.get(), "updated_generation")?,
            ],
        )
        .map_err(|error| database_error("write a document", &error))?;
    Ok(())
}

/// Reads one document, the query "documents by id".
pub(crate) fn document_by_id(
    connection: &Connection,
    doc_id: &str,
) -> Result<Option<DocumentRecord>, CribleError> {
    let row = connection
        .query_row(
            "SELECT doc_id, doc_hash, fields, updated_generation
             FROM documents WHERE doc_id = ?1",
            params![doc_id],
            document_row,
        )
        .optional()
        .map_err(|error| database_error("read a document", &error))?;
    row.map(document_from_row).transpose()
}

/// Raw columns of a `documents` row.
type DocumentColumns = (String, String, String, i64);

/// Raw columns of a `passages` row.
type PassageColumns = (
    i64,
    String,
    i64,
    Vec<u8>,
    String,
    String,
    String,
    i64,
    Option<i64>,
);

fn document_row(row: &Row<'_>) -> rusqlite::Result<DocumentColumns> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

fn passage_row(row: &Row<'_>) -> rusqlite::Result<PassageColumns> {
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

fn document_from_row(row: DocumentColumns) -> Result<DocumentRecord, CribleError> {
    let (doc_id, doc_hash, fields, updated_generation) = row;
    Ok(DocumentRecord {
        doc_id,
        doc_hash,
        fields: parse_fields(&fields)?,
        updated_generation: Generation::new(from_sql_u64(
            updated_generation,
            "updated_generation",
        )?),
    })
}

fn passage_from_row(row: PassageColumns) -> Result<PassageRecord, CribleError> {
    let (iid, doc_id, ord, text, passage_hash, fields, segment_id, created, dead) = row;
    Ok(PassageRecord {
        iid: Iid::new(from_sql_u32(iid, "iid")?),
        doc_id,
        ord: from_sql_u32(ord, "ord")?,
        text: decode_text(&text)?,
        passage_hash,
        fields: parse_fields(&fields)?,
        segment_id,
        created_generation: Generation::new(from_sql_u64(created, "created_generation")?),
        dead_since_gen: dead
            .map(|generation| from_sql_u64(generation, "dead_since_gen").map(Generation::new))
            .transpose()?,
    })
}

/// Parses the canonical JSON stored in a `fields` column.
fn parse_fields(fields: &str) -> Result<Value, CribleError> {
    serde_json::from_str(fields).map_err(|error| {
        CribleError::new(
            ErrorCode::Internal,
            format!("stored fields are not valid JSON: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds the SQLite failure carrying the result code `code`.
    fn sqlite_failure(code: i32) -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None)
    }

    #[test]
    fn busy_errors_become_overloaded() {
        // The plain code plus the variants SQLite raises in WAL mode: a stale
        // snapshot and a WAL recovery, which ADR-0038 lists explicitly.
        for code in [
            rusqlite::ffi::SQLITE_BUSY,
            rusqlite::ffi::SQLITE_BUSY_SNAPSHOT,
            rusqlite::ffi::SQLITE_BUSY_RECOVERY,
        ] {
            let error = database_error("write a document", &sqlite_failure(code));
            assert_eq!(
                error.code(),
                ErrorCode::Overloaded,
                "SQLite code {code} is a lock contention, so it must be retryable"
            );
        }
    }

    #[test]
    fn other_sqlite_errors_stay_internal() {
        for code in [
            rusqlite::ffi::SQLITE_READONLY,
            rusqlite::ffi::SQLITE_READONLY_RECOVERY,
            rusqlite::ffi::SQLITE_CORRUPT,
            rusqlite::ffi::SQLITE_CONSTRAINT,
        ] {
            let error = database_error("write a document", &sqlite_failure(code));
            assert_eq!(
                error.code(),
                ErrorCode::Internal,
                "SQLite code {code} is not a lock contention"
            );
        }
    }
}
