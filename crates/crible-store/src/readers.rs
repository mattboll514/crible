// SPDX-License-Identifier: Apache-2.0

//! Cache of the dense readers of a published generation.
//!
//! References: ADR-0126, decisions 1, 3, 4 and 5. Opening a [`SegmentReader`]
//! verifies the blake3 checksum of the whole vector file, which costs about
//! 8.3 s for the 13.2 GB of `synth-large`
//! (`docs/mesures/lecteurs-denses-ouverture-2026-11-01.md`). That cost belongs
//! to a **publication**, once per process and per segment, never to a query.
//!
//! [`DenseReaders`] is that cache. It holds:
//!
//! - one [`SegmentReader`] per **segment**, keyed by `segment_id`. Segments are
//!   immutable (ADR-0006), so a reader stays valid as long as the segment is
//!   part of a published generation, whatever the generation;
//! - one payload pool of read-only connections per **epoch**: `records.sqlite`
//!   changes with the epoch, not with the generation, so a metadata patch does
//!   not reopen it.
//!
//! [`DenseReaders::filled`] returns a new value and does the I/O: it opens the
//! segments the generation adds, reuses the readers already open, drops the ones
//! the generation no longer names, and opens the payload pool when the epoch
//! changes. The store keeps no lock across it, so a publication fills **before**
//! it takes the write lock of the published state (decision 4).
//!
//! The readers are `Arc`s and the cache holds **no** `EpochLease`: a reader the
//! generation retires lives on exactly as long as the last request that took it,
//! and the epochs stay governed by the snapshots alone (`reclaim_epochs`,
//! ADR-0065, ADR-0017). A cached reader never loads the binary codes: those live
//! in the [`Snapshot`](crate::generation::Snapshot) (ADR-0069, ADR-0126,
//! decision 5).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crible_core::error::{CribleError, ErrorCode};

use crate::records::ReadPool;
use crate::segment::{SegmentMeta, SegmentReader};

/// Readers of the vectors of every segment of a generation, and the payload
/// pool of its epoch.
///
/// The value is cheap to clone: the readers and the pool are shared by
/// reference count, so a query that took its published state keeps serving the
/// generation it started on, even after a publication replaced the cache.
#[derive(Debug, Clone, Default)]
pub struct DenseReaders {
    /// One reader per segment identifier.
    segments: BTreeMap<String, Arc<SegmentReader>>,
    /// Payload pool of the epoch the readers serve.
    records: Option<Payloads>,
}

/// The payload pool of one epoch, with the path it was opened on.
#[derive(Debug, Clone)]
struct Payloads {
    /// `records.sqlite` the pool reads.
    path: PathBuf,
    /// Read-only connections, shared by every request of the epoch.
    pool: Arc<ReadPool>,
}

impl DenseReaders {
    /// A cache with no reader and no pool, from which a first fill starts.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Number of segment readers the cache holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// True when the cache holds no reader.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Returns the cache of the generation `segments` of the epoch at
    /// `segments_dir`, reusing what this cache already holds.
    ///
    /// A segment of `segments` whose reader is already open is **reused**: its
    /// metadata must equal the one the generation publishes, or the call fails,
    /// because two generations would then disagree on a segment identifier. A
    /// segment that is not open is opened here, which pays its blake3 check once
    /// per process (ADR-0126, decision 2). A reader whose segment the generation
    /// does not name is **not** part of the returned cache: it is released when
    /// the last owner of this one drops it (decision 5).
    ///
    /// The payload pool is opened on `records_path` when none is open, or when
    /// the epoch changed; it is reused otherwise. `connections` is the capacity
    /// of that pool: the caller passes the number of scan workers,
    /// `ScanPool::threads()` in `crible-search`, one read-only connection per
    /// worker as the searcher has always sized it (ADR-0126, decision 1). This
    /// module holds no rule of its own: sizing belongs to the caller that owns
    /// the scan pool.
    ///
    /// # Errors
    ///
    /// The errors of [`SegmentReader::open`] — a corrupt or truncated segment,
    /// or a checksum that does not match —, of [`ReadPool::new`] and of
    /// [`ReadPool::verify`], plus [`ErrorCode::Internal`] when a cached reader
    /// of the same identifier describes another segment.
    pub fn filled(
        &self,
        segments_dir: &Path,
        segments: &[SegmentMeta],
        records_path: &Path,
        connections: usize,
    ) -> Result<Self, CribleError> {
        let mut opened = BTreeMap::new();
        for meta in segments {
            let reader = match self.segments.get(&meta.segment_id) {
                Some(reader) => {
                    if reader.meta() != meta {
                        return Err(CribleError::new(
                            ErrorCode::Internal,
                            format!(
                                "the cached reader of segment '{}' does not describe the \
                                 generation being published",
                                meta.segment_id
                            ),
                        ));
                    }
                    Arc::clone(reader)
                }
                None => Arc::new(SegmentReader::open(&segments_dir.join(&meta.segment_id))?),
            };
            opened.insert(meta.segment_id.clone(), reader);
        }
        let records = match &self.records {
            Some(payloads) if payloads.path == records_path => Some(payloads.clone()),
            _ => {
                let pool = Arc::new(ReadPool::new(records_path, connections)?);
                // `ReadPool` opens its connections lazily and SQLite does not
                // validate the file header when a connection opens, so a wrong
                // path or a foreign file would only fail at the first search.
                // One trivial query is run here, so the fill reports the fault.
                pool.verify()?;
                Some(Payloads {
                    path: records_path.to_path_buf(),
                    pool,
                })
            }
        };
        Ok(Self {
            segments: opened,
            records,
        })
    }

    /// The readers of the generation, sorted by ascending `iid_start`.
    ///
    /// This is the order the dense searcher of `crible-search` searches in: it
    /// locates the segment of an identifier by dichotomising on the first
    /// `iid_start` above it.
    ///
    /// The vector is built per call and holds `Arc`s only: no reader is opened
    /// and no vector byte is read.
    #[must_use]
    pub fn readers(&self) -> Vec<Arc<SegmentReader>> {
        let mut readers: Vec<Arc<SegmentReader>> = self.segments.values().map(Arc::clone).collect();
        readers.sort_by_key(|reader| reader.meta().iid_start.get());
        readers
    }

    /// The payload pool of the epoch, shared by every request of the generation.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the cache holds no pool, which a published
    /// generation never does: the fill opens it before the publication
    /// (ADR-0126, decision 4).
    pub fn records(&self) -> Result<Arc<ReadPool>, CribleError> {
        self.records
            .as_ref()
            .map(|payloads| Arc::clone(&payloads.pool))
            .ok_or_else(|| {
                CribleError::new(
                    ErrorCode::Internal,
                    "the payload pool of the published generation is not open",
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::DenseReaders;

    #[test]
    fn an_empty_cache_has_no_reader_and_no_pool() {
        let cache = DenseReaders::empty();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert!(cache.readers().is_empty());
        assert!(
            cache.records().is_err(),
            "an unfilled cache holds no payload pool"
        );
    }
}
