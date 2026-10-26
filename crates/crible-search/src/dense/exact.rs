// SPDX-License-Identifier: Apache-2.0

//! Exact dense search: the top-k cosine over the pool, from a provided vector.
//!
//! References: `docs/03-recherche.md` ("Branche dense", strategy EXACT),
//! `docs/05-api.md` ("Recherche : réponse"), `docs/08-invariants-et-tests.md`
//! (INV-06) and ADR-0019 (fields stored as canonical JSON).
//!
//! # What the exact strategy computes
//!
//! The vectors stored in the segments are normalised, so the dot product *is*
//! the cosine (`docs/03-recherche.md`). The query is normalised here, in `f32`,
//! so the score of a passage stays its cosine whatever the norm of the input
//! vector. Every component is read from the segment as fp16 and widened to
//! `f32`; the product is the plain scalar loop the task sheet asks for, because
//! the SIMD version belongs to P4.
//!
//! # Parallelism and determinism
//!
//! The pool is cut into at most one contiguous window of internal identifiers
//! per rayon worker, each window holding about the same number of live
//! passages; the dead identifiers inside a window are skipped through the live
//! bitmap. Each window keeps its own bounded heap of candidates, and the heaps
//! are merged at the end (`docs/03-recherche.md`, "Parallélisme").
//! The result never depends on the partitioning: the order is descending score,
//! ties broken on the ascending key `doc_id#ord`, and that order is total.
//!
//! **The ranking compares the textual key**, the string `doc_id#ord`, and not
//! the `Ord` of [`crible_core::ids::PassageKey`]. The two differ as soon as a
//! document has ten passages or more (`doc#10` sorts before `doc#2` as text),
//! and the text is the order `tools/reference_topk.py` uses
//! (`tools/reference_topk.py`, `top_k`, sorting on `passage.key`). Changing it
//! would break INV-06; the question of aligning `PassageKey::Ord` belongs to
//! `explain_ids` (P4/P6), not here.
//!
//! A score is checked finite before it may enter a heap: [`f32::total_cmp`]
//! would otherwise rank a NaN above every number, and a corrupt vector would
//! silently win the top-k. A non-finite cosine stops the search with an
//! [`ErrorCode::Internal`] error instead. The vectors are written normalised
//! and their checksum is verified when a segment opens, so a non-finite score
//! means the stored data contradicts the format, not a bad request. The query
//! itself is checked earlier: a non-finite component, or finite components whose
//! squared norm overflows `f32`, is an [`ErrorCode::InvalidRequest`] error
//! before any scan starts.
//!
//! # The pool is the only input set
//!
//! [`ExactSearcher::top_k`] scores the passages of the pool and nothing else; a
//! tombstoned passage is never in the pool, because [`Pool`] already
//! intersects the filter with the live bitmap. During the scan, only the key of
//! a candidate is read; the payload of the `k` retained results comes from
//! `records.sqlite` afterwards, through a read-only pool of connections
//! (`docs/03-recherche.md`, step 8, and `docs/02-architecture.md`,
//! "Concurrence"). The payload reads the `records.sqlite` of the epoch the
//! searcher opened on, at one fixed path, never a path recomputed per query
//! (ADR-0043, retention rule).
//!
//! When the `documents` row of a retained result is absent, because the document
//! was removed after the snapshot was taken, the result keeps its text and its
//! passage fields, carries no document field, and raises no error: that is a
//! normal state of the database, not an inconsistency (ADR-0043, option A,
//! without marker). `internal` stays for a missing segment, a missing `passages`
//! row or a stored `fields` value that is not a JSON object. An internal
//! identifier never leaves the engine: a [`Hit`] names its passage by its
//! external key only (rule 11 of `AGENTS.md`).

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;
use std::path::Path;

use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::Iid;
use crible_store::records::{PassageRecord, ReadPool};
use crible_store::segment::{SegmentMeta, SegmentReader};
use rayon::prelude::*;
use serde_json::{Map, Value};

use super::windows;
use crate::pool::Pool;

/// Number of read-only connections the searcher keeps.
///
/// Key reads happen inside the rayon windows, so the pool is sized on the rayon
/// workers serving the scan: one connection per worker lets every window read
/// its key without queueing on the `Condvar` behind a single connection. The
/// value is a bound on connections, not a measured optimum, and no measurement
/// backs it. The capacity is fixed when the searcher opens; a searcher opened
/// inside a one-thread pool and used from a wider one would serialise its key
/// reads, which is correct but slower.
///
/// The searcher owns this pool privately in P1: no other branch shares it. A
/// later design that shares one read pool between two branches (the lexical
/// branch of `docs/03-recherche.md`, step 5) would have to revisit the capacity
/// with a measurement.
///
/// **Nesting rule.** A caller must not call [`ExactSearcher::top_k`] while it
/// already holds a connection of this same [`ReadPool`]: `checkout` is never
/// re-entered here, and an outer holder that waited for the search would wait on
/// itself once every connection is checked out.
fn read_connections() -> usize {
    rayon::current_num_threads().max(1)
}

/// One result of the exact search: a passage, its cosine and its payload.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// External identifier of the document the passage belongs to.
    pub doc_id: String,
    /// Position of the passage in its document.
    pub ord: u32,
    /// Key `doc_id#ord`, the tie-break of the ranking.
    pub key: String,
    /// Cosine between the query and the passage, computed in `f32`.
    pub score: f32,
    /// Document fields merged with the passage fields (ADR-0019, ADR-0030).
    pub fields: Value,
    /// Exact encoder input of the passage, decompressed.
    pub text: String,
}

/// Exact top-k cosine search over the segments of one generation.
///
/// The searcher opens every segment once, at construction, so it serves many
/// queries: [`SegmentReader::open`] verifies the blake3 checksum of the whole
/// vector file, which must not be paid per query.
#[derive(Debug)]
pub struct ExactSearcher {
    /// One reader per segment, sorted by `iid_start`.
    segments: Vec<SegmentReader>,
    /// Read-only connections used for the payloads.
    records: ReadPool,
    /// Dimensionality of the vectors, shared by every segment.
    dims: u32,
}

impl ExactSearcher {
    /// Opens one reader per segment and the payload reader of the epoch.
    ///
    /// `segments_dir` is the `segments/` directory of the epoch and `segments`
    /// the metadata of the generation being served: the readers are opened
    /// from the two, at once, and kept for every later query. The
    /// `records_path` is the `records.sqlite` file of the same epoch.
    pub fn open(
        segments_dir: &Path,
        segments: &[SegmentMeta],
        records_path: &Path,
    ) -> Result<Self, CribleError> {
        let mut readers = Vec::with_capacity(segments.len());
        let mut dims: Option<u32> = None;
        for meta in segments {
            let reader = SegmentReader::open(&segments_dir.join(&meta.segment_id))?;
            if reader.meta() != meta {
                return Err(internal(format!(
                    "the segment directory '{}' does not match the metadata of the generation",
                    meta.segment_id
                )));
            }
            if let Some(expected) = dims
                && expected != meta.dims
            {
                return Err(internal(format!(
                    "segment '{}' holds {} dimensions where the others hold {expected}",
                    meta.segment_id, meta.dims
                )));
            }
            dims = Some(meta.dims);
            readers.push(reader);
        }
        readers.sort_by_key(|reader| reader.meta().iid_start.get());
        for pair in readers.windows(2) {
            let (left, right) = (pair[0].meta(), pair[1].meta());
            let left_end = u64::from(left.iid_start.get()) + u64::from(left.count);
            if left_end > u64::from(right.iid_start.get()) {
                return Err(internal(format!(
                    "segments '{}' and '{}' overlap their internal identifiers",
                    left.segment_id, right.segment_id
                )));
            }
        }
        let records = ReadPool::new(records_path, read_connections())?;
        // `ReadPool` opens its connections lazily and SQLite does not validate
        // the file header when a connection opens, so a wrong path or a foreign
        // file would only fail at the first search. One trivial query is run
        // here, through the pool, so `open` reports the fault instead.
        records.verify()?;
        Ok(Self {
            segments: readers,
            records,
            dims: dims.unwrap_or(0),
        })
    }

    /// Returns the `k` best passages of `pool`, by descending cosine, ties
    /// broken on the ascending key `doc_id#ord`.
    ///
    /// An empty pool, or `k == 0`, returns no result and no error. A `k` above
    /// the pool size returns the whole pool. The query must carry one
    /// component per dimension of the index and must not be the zero vector,
    /// because its cosine would be undefined.
    ///
    /// A generation without a segment has no dimensionality to compare against.
    /// Its pool is necessarily empty (a passage always comes with a vector), so
    /// any query returns no result and no error; this is the normal state right
    /// after `crible index create`. The zero vector stays refused, because no
    /// cosine exists whatever the generation holds.
    ///
    /// A caller must not hold a connection of the searcher's read pool when it
    /// calls this method: the key reads would wait for it, and the pool would
    /// run out of connections.
    pub fn top_k(&self, query: &[f32], pool: &Pool, k: usize) -> Result<Vec<Hit>, CribleError> {
        let empty = pool.is_empty();
        if !empty && self.segments.is_empty() {
            return Err(internal(
                "the generation holds no segment, yet its pool is not empty",
            ));
        }
        // The request is validated before the empty pool short-circuits: a
        // malformed vector is refused whatever the filter keeps.
        if !self.segments.is_empty() && query.len() != self.dims as usize {
            return Err(CribleError::new(
                ErrorCode::InvalidRequest,
                format!(
                    "the query vector holds {} components where the index holds {} dimensions",
                    query.len(),
                    self.dims
                ),
            ));
        }
        let query = normalise(query)?;
        if k == 0 || empty {
            return Ok(Vec::new());
        }

        let windows = windows(
            pool.bitmap(),
            pool.len(),
            rayon::current_num_threads().max(1),
        );
        let heaps: Vec<Vec<Candidate>> = windows
            .par_iter()
            .map(|&(from, to)| self.score_slice(&query, pool, from, to, k))
            .collect::<Result<Vec<_>, CribleError>>()?;

        let mut candidates: Vec<Candidate> = heaps.into_iter().flatten().collect();
        candidates.sort_unstable_by(|left, right| right.cmp(left));
        candidates.truncate(k);

        candidates
            .into_iter()
            .map(|candidate| self.payload(candidate))
            .collect()
    }

    /// Scores the live passages of one window and keeps its `k` best
    /// candidates.
    ///
    /// The window is a range of internal identifiers; the scan walks the live
    /// bitmap from `from` and stops after `to`, so it scores only live passages
    /// however fragmented the pool is. It reads the key of a passage only when
    /// that passage can still enter the heap: a score strictly below the worst
    /// candidate of a full heap cannot, and the key tie-break never has to be
    /// resolved for it. A candidate carries its internal identifier until the
    /// payload step, which reads the text and the fields of the `k` retained
    /// results only.
    fn score_slice(
        &self,
        query: &[f32],
        pool: &Pool,
        from: u32,
        to: u32,
        k: usize,
    ) -> Result<Vec<Candidate>, CribleError> {
        // `Reverse` turns the max-heap into a heap whose top is the worst
        // candidate: `pop` then drops exactly the one a new candidate beats.
        let mut heap: BinaryHeap<Reverse<Candidate>> =
            BinaryHeap::with_capacity(k.min(usize::try_from(pool.len()).unwrap_or(usize::MAX)));
        let mut live = pool.bitmap().iter();
        live.advance_to(from);
        for raw in live {
            if raw > to {
                break;
            }
            let iid = Iid::new(raw);
            let vector = self.reader(iid)?.vector(iid)?;
            let score = dot(query, &vector);
            if !score.is_finite() {
                return Err(internal(
                    "a stored vector produces a non-finite cosine, so the ranking is undefined",
                ));
            }
            if heap.len() == k {
                let worst = heap.peek().ok_or_else(|| {
                    internal("a candidate heap of the exact search is unexpectedly empty")
                })?;
                if score.total_cmp(&worst.0.score) == Ordering::Less {
                    continue;
                }
            }
            let candidate = Candidate::new(score, iid, self.key_of(iid)?);
            heap.push(Reverse(candidate));
            if heap.len() > k {
                heap.pop();
            }
        }
        Ok(heap
            .into_iter()
            .map(|Reverse(candidate)| candidate)
            .collect())
    }

    /// Segment holding `iid`.
    fn reader(&self, iid: Iid) -> Result<&SegmentReader, CribleError> {
        let position = self
            .segments
            .partition_point(|reader| reader.meta().iid_start.get() <= iid.get());
        let reader = position
            .checked_sub(1)
            .and_then(|index| self.segments.get(index))
            .ok_or_else(|| internal("no segment holds a passage of the pool"))?;
        if !reader.contains(iid) {
            return Err(internal(
                "a passage of the pool falls outside the segments of the generation",
            ));
        }
        Ok(reader)
    }

    /// Key of `iid`, read from its identity columns only.
    fn key_of(&self, iid: Iid) -> Result<String, CribleError> {
        let key = self
            .records
            .read_passage_key(iid)?
            .ok_or_else(|| internal("a passage of the pool has no passage row"))?;
        Ok(key.to_string())
    }

    /// Passage row of `iid`, payload included, through the read-only
    /// connections.
    fn passage_of(&self, iid: Iid) -> Result<PassageRecord, CribleError> {
        self.records
            .read_passage(iid)?
            .ok_or_else(|| internal("a passage of the pool has no passage row"))
    }

    /// Builds the result of one retained candidate, reading its payload and the
    /// fields of its document.
    ///
    /// This is the payload step of `docs/03-recherche.md` (step 8): it runs
    /// after the merge, so only the `k` results pay for the text and the
    /// fields. A missing document row is not an inconsistency: the document may
    /// have been removed after the snapshot was taken, and the result then
    /// carries its passage fields only (ADR-0043, option A, without marker).
    fn payload(&self, candidate: Candidate) -> Result<Hit, CribleError> {
        let passage = self.passage_of(candidate.iid)?;
        let document = self.records.read_document(&passage.doc_id)?;
        Ok(Hit {
            key: candidate.key,
            score: candidate.score,
            fields: merge_fields(
                document.as_ref().map(|document| &document.fields),
                &passage.fields,
            )?,
            doc_id: passage.doc_id,
            ord: passage.ord,
            text: passage.text,
        })
    }
}

/// One scored passage of a slice, ordered best first.
///
/// The order is the one of the answer: descending score, then ascending
/// **textual** key `doc_id#ord`. It is not the `Ord` of
/// [`crible_core::ids::PassageKey`], which compares `(doc_id, ord)` with `ord`
/// as a number; the two diverge from `ord = 10` on, and the text is the order
/// `tools/reference_topk.py` uses. A score is finite here: [`ExactSearcher`]
/// refuses a non-finite cosine before a candidate can enter a heap, so
/// [`f32::total_cmp`] only provides the total order a heap requires.
#[derive(Debug)]
struct Candidate {
    /// Cosine between the query and the passage.
    score: f32,
    /// Internal identifier of the passage, kept until the payload step.
    iid: Iid,
    /// Textual key `doc_id#ord`, the tie-break.
    key: String,
}

impl Candidate {
    /// Candidate for the passage `iid` whose textual key is `key`.
    fn new(score: f32, iid: Iid, key: String) -> Self {
        Self { score, iid, key }
    }
}

impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Candidate {}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        // Greater is better: a higher score wins, and at equal score the
        // smaller key wins. A slice wraps its heap in `Reverse`, so the top of
        // the heap is the candidate that `pop` removes first.
        self.score
            .total_cmp(&other.score)
            .then_with(|| other.key.cmp(&self.key))
    }
}

/// Dot product of a normalised query and one stored vector, in `f32`.
///
/// The loop is the scalar implementation of the task sheet: fp16 is widened to
/// `f32` by the segment reader, and the sum runs in component order, so the
/// score is reproducible bit for bit and does not depend on a SIMD width.
fn dot(query: &[f32], vector: &[f32]) -> f32 {
    let mut sum = 0.0_f32;
    for (query, vector) in query.iter().zip(vector) {
        sum += query * vector;
    }
    sum
}

/// Divides `query` by its Euclidean norm in `f32`, as the numpy reference does.
///
/// The query comes from the caller, so a non-finite component, or finite
/// components whose squares overflow `f32`, is an invalid request: the norm
/// would be infinite and every component would become zero in silence, scoring
/// the whole pool at zero. A zero vector has no cosine either, so it is refused
/// rather than scoring every passage at zero, which would be meaningless.
fn normalise(query: &[f32]) -> Result<Vec<f32>, CribleError> {
    let mut squares = 0.0_f32;
    for component in query {
        if !component.is_finite() {
            return Err(invalid_request(
                "the query vector holds a non-finite component",
            ));
        }
        squares += component * component;
    }
    let norm = squares.sqrt();
    if !norm.is_finite() {
        return Err(invalid_request(
            "the squared norm of the query vector overflows f32",
        ));
    }
    if norm == 0.0 {
        return Err(invalid_request(
            "the query vector has a zero norm, so no cosine is defined",
        ));
    }
    Ok(query.iter().map(|component| component / norm).collect())
}

/// Document fields merged with the passage fields, as a result carries them.
///
/// ADR-0030 makes a field name designate exactly one level, so the two objects
/// cannot collide. A stored `fields` value is always a canonical JSON object
/// (ADR-0019); anything else means the epoch is corrupt. A missing document
/// contributes no field and is not an error (ADR-0043).
fn merge_fields(document: Option<&Value>, passage: &Value) -> Result<Value, CribleError> {
    let mut merged = Map::new();
    for fields in document.into_iter().chain(std::iter::once(passage)) {
        let Value::Object(entries) = fields else {
            return Err(internal(
                "a stored fields value is not a JSON object, so the payload cannot be merged",
            ));
        };
        for (name, value) in entries {
            merged.insert(name.clone(), value.clone());
        }
    }
    Ok(Value::Object(merged))
}

/// Error raised when the structures of the epoch contradict each other.
fn internal(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::Internal, message)
}

/// Error raised when the request itself is malformed.
fn invalid_request(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::InvalidRequest, message)
}
