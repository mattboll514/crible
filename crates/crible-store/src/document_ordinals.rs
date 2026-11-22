// SPDX-License-Identifier: Apache-2.0

//! Derived table from an internal identifier to the ordinal of its document.
//!
//! References: ADR-0154 (decisions 5 and 11), `docs/03-recherche.md`
//! ("Filtres → pool") and `docs/02-architecture.md` ("Mémoire : ce qui est en
//! RAM").
//!
//! # Why a derived table
//!
//! The facets route counts the distinct documents of a pool. A bitmap of
//! passages does not know how many `doc_id` it covers, and a snapshot keeps no
//! mapping between an `iid` and its document: reading `records.sqlite` by iid
//! per request would cost one query per passage of the pool. This table is that
//! mapping, held in RAM: one dense array indexed by iid, holding a **document
//! ordinal** assigned once per distinct `doc_id` of the epoch.
//!
//! # Construction and update
//!
//! [`DocumentOrdinals::from_records`] builds it with the filter structures at
//! the open, from a scan of `passages` ordered by `doc_id`. That order is the
//! one of the `passages_doc_id` index, so the scan streams and no map of every
//! `doc_id` is ever held in RAM: the ordinal is allocated when the `doc_id`
//! changes, and the next rows of the same document reuse it.
//!
//! `DocumentOrdinals::after_commit` derives it from the delta of a commit, as
//! the bitmaps are derived (ADR-0066): only the passages the transaction
//! created can introduce a document ordinal, and one bounded query by
//! `passages(doc_id)` per distinct created document tells whether the document
//! already had one. A generation that creates no passage shares the array of
//! the previous one through an [`Arc`], so a metadata-only commit copies
//! nothing.
//!
//! The table is never written: no file, no `format_version`, no migration and
//! no entry in a snapshot or an export. Its bytes are counted in
//! `filter_memory_bytes` beside the filter structures.

use std::collections::BTreeMap;
use std::sync::Arc;

use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::Iid;
use roaring::RoaringBitmap;
use rusqlite::Connection;

use crate::filters_index::CommitDelta;

/// Ordinal of a passage whose document the table does not know.
///
/// Every row of `passages` gets an ordinal at the open, and every created
/// passage gets one at its commit, so a live passage always has one. The value
/// is the sentinel of an unassigned slot; a passage of the pool that still
/// holds it is a divergence between the filters and the table, reported as an
/// internal error rather than silently undercounted.
pub const NO_ORDINAL: u32 = u32::MAX;

/// Dense table `iid → ordinal de document` of one generation.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DocumentOrdinals {
    /// Ordinal of the document of each iid, [`NO_ORDINAL`] until it is set.
    ///
    /// The array is behind an [`Arc`] so that a generation that creates no
    /// passage shares it with the previous one instead of copying it.
    by_iid: Arc<Vec<u32>>,
    /// Number of distinct documents the table has seen: the next ordinal.
    documents: u32,
}

impl DocumentOrdinals {
    /// Builds the table of the reference state, from `records.sqlite`.
    ///
    /// The scan is ordered by `doc_id`, so it walks the `passages_doc_id`
    /// index and allocates one ordinal per change of document. Every row of
    /// `passages` is covered, dead ones included: a tombstone never rewrites
    /// the table, and the pool alone decides which passages are counted.
    ///
    /// # Errors
    ///
    /// The errors of the SQLite queries, and [`ErrorCode::Internal`] when an
    /// iid or an ordinal leaves its range.
    pub fn from_records(connection: &Connection) -> Result<Self, CribleError> {
        let rows = connection
            .query_row(
                "SELECT COALESCE(MAX(iid) + 1, 0) FROM passages",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| database_error("count the ids of the document ordinals", &error))?;
        let slots = usize::try_from(rows).map_err(|_| {
            CribleError::new(
                ErrorCode::Internal,
                "the passages of the reference state do not fit in memory",
            )
        })?;
        let mut by_iid = vec![NO_ORDINAL; slots];
        let mut documents: u32 = 0;
        let mut ordinal = NO_ORDINAL;
        let mut previous: Option<String> = None;
        let mut statement = connection
            .prepare("SELECT iid, doc_id FROM passages ORDER BY doc_id")
            .map_err(|error| database_error("prepare the document ordinals query", &error))?;
        let mut rows = statement
            .query([])
            .map_err(|error| database_error("query the document ordinals", &error))?;
        while let Some(row) = rows
            .next()
            .map_err(|error| database_error("read a row of the document ordinals", &error))?
        {
            let iid = row
                .get::<_, i64>(0)
                .map_err(|error| database_error("read an iid of the document ordinals", &error))?;
            let iid = usize::try_from(iid).map_err(|_| {
                CribleError::new(
                    ErrorCode::Internal,
                    "the reference state holds a negative passage id",
                )
            })?;
            let doc_id = row
                .get::<_, String>(1)
                .map_err(|error| database_error("read a document id", &error))?;
            if previous.as_deref() != Some(doc_id.as_str()) {
                ordinal = documents;
                documents = documents.checked_add(1).ok_or_else(|| {
                    CribleError::new(
                        ErrorCode::Internal,
                        "the reference state holds more documents than an ordinal can count",
                    )
                })?;
                previous = Some(doc_id);
            }
            let slot = by_iid.get_mut(iid).ok_or_else(|| {
                CribleError::new(
                    ErrorCode::Internal,
                    "the reference state holds a passage id above its own bound",
                )
            })?;
            *slot = ordinal;
        }
        Ok(Self {
            by_iid: Arc::new(by_iid),
            documents,
        })
    }

    /// Derives the table of the generation a transaction has just committed,
    /// from the previous one and from `delta` (ADR-0066).
    ///
    /// Only the passages the transaction created can introduce a document: a
    /// tombstone, a removed document and a rewritten document row keep the
    /// document of their passages. One bounded query by `passages(doc_id)`
    /// gives the smallest iid of a document: when it is below the created
    /// passage, the document already had an ordinal, which the slot of that
    /// iid holds; otherwise the document is new and takes the next ordinal.
    ///
    /// # Errors
    ///
    /// The errors of the SQLite queries, and [`ErrorCode::Internal`] when the
    /// previous table has no ordinal for a document it should already cover.
    pub(crate) fn after_commit(
        &self,
        connection: &Connection,
        delta: &CommitDelta,
    ) -> Result<Self, CribleError> {
        let mut next = self.clone();
        if delta.created.is_empty() {
            return Ok(next);
        }
        // The created passages are grouped by document: one lookup by document
        // decides its ordinal, and every passage the commit adds to it takes
        // that ordinal. A document added twice in one commit is therefore seen
        // once, and the smallest iid of the group is the one the lookup
        // compares with the smallest iid of the document in `passages`.
        let mut by_document: BTreeMap<&str, Vec<Iid>> = BTreeMap::new();
        for passage in &delta.created {
            by_document
                .entry(passage.doc_id.as_str())
                .or_default()
                .push(passage.iid);
        }
        let mut assignments: Vec<(Iid, u32)> = Vec::with_capacity(delta.created.len());
        let mut documents = next.documents;
        for (doc_id, mut iids) in by_document {
            iids.sort_unstable_by_key(|iid| iid.get());
            let Some(first) = iids.first().copied() else {
                continue;
            };
            let ordinal = next.ordinal_for(connection, doc_id, first, &mut documents)?;
            assignments.extend(iids.into_iter().map(|iid| (iid, ordinal)));
        }
        let slots = Arc::make_mut(&mut next.by_iid);
        // A generation that creates its first passages extends the array, and a
        // later one adds its slots after the last iid of the previous
        // generation. The gap, if any, keeps [`NO_ORDINAL`]: only the passages
        // that exist have to be covered, and the pool is the judge of that.
        let needed = assignments
            .iter()
            .map(|(iid, _)| iid.get() as usize + 1)
            .max()
            .unwrap_or(0);
        if needed > slots.len() {
            slots.resize(needed, NO_ORDINAL);
        }
        for (iid, ordinal) in assignments {
            let slot = slots.get_mut(iid.get() as usize).ok_or_else(|| {
                CribleError::new(
                    ErrorCode::Internal,
                    "a created passage has no slot in the document ordinals",
                )
            })?;
            *slot = ordinal;
        }
        next.documents = documents;
        Ok(next)
    }

    /// Ordinal of `doc_id` for the created passage `iid`, allocating one when
    /// the document is new.
    fn ordinal_for(
        &self,
        connection: &Connection,
        doc_id: &str,
        iid: Iid,
        documents: &mut u32,
    ) -> Result<u32, CribleError> {
        let first: Option<i64> = connection
            .query_row(
                "SELECT MIN(iid) FROM passages WHERE doc_id = ?1",
                [doc_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(|error| database_error("read the first passage of a document", &error))?;
        let first = first.and_then(|raw| u32::try_from(raw).ok()).map(Iid::new);
        if let Some(first) = first.filter(|first| first.get() < iid.get()) {
            let ordinal = self
                .by_iid
                .get(first.get() as usize)
                .copied()
                .filter(|ordinal| *ordinal != NO_ORDINAL)
                .ok_or_else(|| {
                    CribleError::new(
                        ErrorCode::Internal,
                        "the document ordinals hold no ordinal for a document that already has \
                         passages",
                    )
                })?;
            return Ok(ordinal);
        }
        let ordinal = *documents;
        *documents = documents.checked_add(1).ok_or_else(|| {
            CribleError::new(
                ErrorCode::Internal,
                "the index holds more documents than an ordinal can count",
            )
        })?;
        Ok(ordinal)
    }

    /// Number of distinct documents the table has seen, dead ones included.
    #[must_use]
    pub fn documents(&self) -> u32 {
        self.documents
    }

    /// Ordinal of the document of `iid`, or `None` when the table holds none.
    #[must_use]
    pub fn ordinal(&self, iid: Iid) -> Option<u32> {
        self.by_iid
            .get(iid.get() as usize)
            .copied()
            .filter(|ordinal| *ordinal != NO_ORDINAL)
    }

    /// Number of distinct documents among the passages of `pool`.
    ///
    /// Each iid of the pool marks its document ordinal in a bitmap whose size
    /// follows the number of documents of the index, not the pool (ADR-0154,
    /// decision 5). `check` runs after every iid, so the caller can stop the
    /// count at its deadline: a partial count is never returned.
    ///
    /// # Errors
    ///
    /// The failure `check` returns, and [`ErrorCode::Internal`] when a passage
    /// of the pool has no ordinal.
    pub fn count_documents<F>(&self, pool: &RoaringBitmap, mut check: F) -> Result<u64, CribleError>
    where
        F: FnMut() -> Result<(), CribleError>,
    {
        let mut documents = RoaringBitmap::new();
        for iid in pool.iter() {
            let ordinal = self
                .by_iid
                .get(iid as usize)
                .copied()
                .filter(|ordinal| *ordinal != NO_ORDINAL);
            let ordinal = ordinal.ok_or_else(|| {
                CribleError::new(
                    ErrorCode::Internal,
                    "the document ordinals hold no ordinal for a passage of the pool",
                )
            })?;
            documents.insert(ordinal);
            check()?;
        }
        Ok(documents.len())
    }

    /// Estimated heap size of the table, in bytes: four bytes per passage.
    #[must_use]
    pub fn memory(&self) -> usize {
        self.by_iid.len() * std::mem::size_of::<u32>()
    }
}

/// Builds the internal error of a failed SQLite query.
fn database_error(action: &str, error: &rusqlite::Error) -> CribleError {
    CribleError::new(ErrorCode::Internal, format!("cannot {action}: {error}"))
}
