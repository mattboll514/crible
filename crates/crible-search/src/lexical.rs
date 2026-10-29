// SPDX-License-Identifier: Apache-2.0

//! Lexical branch of a search: the BM25 score of the passages of the pool.
//!
//! References: `docs/03-recherche.md` ("Branche lexicale (tantivy)", "Filtres →
//! pool"), `docs/05-api.md` (`overloaded`, `invalid_request`),
//! `docs/08-invariants-et-tests.md` (INV-01, INV-05), `docs/phases/P5-lexical-et-
//! hybride.md` (`RECHERCHER-BM25-DANS-LE-POOL`), ADR-0045 and ADR-0079 (the order
//! of the answer), ADR-0087 (a read-only open repairs nothing), ADR-0088 (when a
//! lexical index is up to date, and the four refusal messages) and ADR-0089 (the
//! two states without a tantivy commit, `Missing`, and a disabled branch).
//!
//! # What the branch does
//!
//! 1. It asks the snapshot for its lexical reader. A state that serves nothing
//!    refuses in `overloaded` (503), with the message of the state observed
//!    (ADR-0088, decision 5; ADR-0089, decision 1); a branch the configuration
//!    disables refuses in `invalid_request` (400) (ADR-0089, decision 3); an
//!    epoch that never carried a passage answers an empty list, without error
//!    (ADR-0089, decision 2).
//! 2. It analyses the query with the chain of the index and turns it into a bag
//!    of terms — a `BooleanQuery` of `Should` clauses — which tantivy scores in
//!    BM25 (`docs/03`, "Requête"). A query that analyses to no term matches
//!    nothing and returns no result, without error.
//! 3. It collects the matching documents into a heap of `k` and keeps only those
//!    of the pool, read from the `iid` fast field. The pool is the authority on
//!    both the filter and the tombstones (INV-05, `docs/03`: "la bitmap des
//!    vivants reste l'autorité"): tantivy may still hold a passage a later
//!    generation killed.
//! 4. It orders the result: descending score, then ascending key `doc_id#ord`,
//!    the order of the dense branch and of the answer (ADR-0045, ADR-0079). One
//!    type carries it, [`Scored`] (ADR-0072, decision 3), so the two branches
//!    cannot drift apart on a tie.
//!
//! # The key, not the identifier
//!
//! The heap of a segment orders its candidates on the internal identifier,
//! because a segment cannot read a key: that order is an **intermediate** one,
//! never the order of the answer. Keys are read from `records.sqlite` after the
//! search, for the candidates that survive it only (ADR-0079), and the final sort
//! compares them as text. An identifier never leaves the engine (INV-01).
//!
//! When the `k`-th score is tied, the identifier cannot decide which tied
//! passage belongs to the answer: the keys of the ties have to. The collector
//! notices such a tie at the boundary (a candidate turned away, or evicted, with
//! the score of the worst kept one) and the search then reruns the query to
//! collect every pool passage at or above that score, reads their keys, and lets
//! the key decide. Without that pass, the answer would depend on the order the
//! segments enumerate their documents in.
//!
//! # `explain`
//!
//! [`explain`] renders tantivy's own [`Explanation`] for one passage named by its
//! external key, whether it is in the pool or in the top-k or neither: the lookup
//! goes through `records.sqlite` (key to identifier) and the `iid` field of the
//! index (identifier to document). The structure of a diagnostic response belongs
//! to P7 (`docs/06-observabilite.md`); this module returns tantivy's value only.

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;
use std::sync::Arc;

use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::{Generation, Iid, PassageKey};
use crible_store::generation::Snapshot;
use crible_store::lexical::{IID_FIELD_NAME, LexicalIndex, LexicalStatus, TEXT_FIELD_NAME};
use roaring::RoaringBitmap;
use tantivy::collector::{Collector, DocSetCollector, SegmentCollector};
use tantivy::fastfield::Column;
use tantivy::query::{BooleanQuery, Explanation, Query, TermQuery};
use tantivy::schema::{Field, IndexRecordOption, Term};
use tantivy::{DocAddress, DocId, Score, Searcher, SegmentOrdinal, SegmentReader};

use crate::dense::Scored;
use crate::pool::Pool;

/// Returns the `k` best passages of `pool` for `query`, by descending BM25
/// score, ties broken on the ascending key `doc_id#ord`.
///
/// The result belongs to the two branches and to the answer: [`Scored`] carries
/// the same order the dense branch returns (ADR-0072, decision 3). An empty pool
/// and `k == 0` return no result and no error; a `k` above the size of the pool
/// returns the whole pool.
///
/// A snapshot without a usable lexical reader does not answer an empty list: it
/// refuses, before any work, with the message of the state observed
/// (ADR-0087, ADR-0088, ADR-0089), so a caller can never read "nothing matched"
/// where the branch could not run.
pub fn search(
    snapshot: &Snapshot,
    pool: &Pool,
    query: &str,
    k: usize,
) -> Result<Vec<Scored>, CribleError> {
    let Some(index) = reader_of(snapshot)? else {
        return Ok(Vec::new());
    };
    if k == 0 || pool.is_empty() {
        return Ok(Vec::new());
    }
    let terms = index.terms(query)?;
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let searcher = index.searcher();
    let query = query_of(&searcher, &terms)?;
    let bitmap = Arc::new(pool.bitmap().clone());

    let found = searcher
        .search(&query, &PoolTopK::new(Arc::clone(&bitmap), k))
        .map_err(|error| internal(format!("the lexical search failed: {error}")))?;
    if found.foreign_iid {
        return Err(internal(
            "the lexical index holds a value in its iid fast field that is not a Crible internal \
             identifier",
        ));
    }
    let candidates = if found.ambiguous {
        resolve_boundary(&searcher, &query, &bitmap, &found.candidates)?
    } else {
        found.candidates
    };

    let mut scored: Vec<Scored> = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let iid = Iid::new(candidate.iid);
        let key = index.passage_key(iid)?.ok_or_else(|| {
            internal("a passage of the pool has no passage row in the reference state")
        })?;
        scored.push(Scored {
            iid,
            key: key.to_string(),
            score: candidate.score,
        });
    }
    scored.sort_unstable_by(|left, right| right.cmp(left));
    scored.truncate(k);
    Ok(scored)
}

/// Renders tantivy's explanation of one passage for `query`, or `None` when the
/// branch has nothing to explain.
///
/// `key` is the external key `doc_id#ord` of a passage, never an internal
/// identifier (INV-01); a key that is not of that form is an `invalid_request`
/// error. The passage may be outside the pool and outside the top-k: the lookup
/// reads `records.sqlite` for its identifier, then the `iid` field of the index
/// for its document, whatever the filter keeps and whatever the ranking does.
///
/// `None` is returned when the reference state holds no such passage, when the
/// commit the snapshot reads holds no such document (a passage a later generation
/// killed, or one the lexical index has not reached yet), and when the query
/// analyses to no term — that is, when there is nothing to explain rather than an
/// error. A state that serves nothing refuses as [`search`] does.
pub fn explain(
    snapshot: &Snapshot,
    key: &str,
    query: &str,
) -> Result<Option<Explanation>, CribleError> {
    let key: PassageKey = key.parse()?;
    let Some(index) = reader_of(snapshot)? else {
        return Ok(None);
    };
    let Some(iid) = index.passage_iid(&key)? else {
        return Ok(None);
    };
    let terms = index.terms(query)?;
    if terms.is_empty() {
        return Ok(None);
    }
    let searcher = index.searcher();
    let query = query_of(&searcher, &terms)?;
    let Some(address) = document_of(&searcher, iid)? else {
        return Ok(None);
    };
    query
        .explain(&searcher, address)
        .map(Some)
        .map_err(|error| internal(format!("the lexical explanation failed: {error}")))
}

/// Lexical reader a query may use, `None` when the epoch has nothing to serve.
///
/// The refusal comes first: a snapshot without a reader refuses even when the
/// pool is empty, because an empty answer would be indistinguishable from
/// "nothing matched" (`docs/05-api.md`, `overloaded`).
fn reader_of(snapshot: &Snapshot) -> Result<Option<&LexicalIndex>, CribleError> {
    if let Some(error) = refusal(snapshot.lexical_status(), snapshot.generation()) {
        return Err(error);
    }
    match snapshot.lexical() {
        Some(index) => Ok(Some(index)),
        // `Missing`: the epoch never carried a passage, so the empty answer is
        // exact (ADR-0089, decision 2).
        None if snapshot.lexical_status() == LexicalStatus::Missing => Ok(None),
        // `observe` opens a reader whenever it publishes `Ready`; a snapshot
        // that contradicts it mixes two generations (INV-02, INV-03).
        None => Err(internal(
            "the snapshot reports a lexical state it publishes no reader for",
        )),
    }
}

/// Error a lexical search must raise for `status`, or `None` when the branch may
/// run or has nothing to serve.
///
/// The messages are those of ADR-0088, decision 5, and of ADR-0089, decision 1:
/// they name the generations and the state observed, never a passage, an
/// identifier or a text (INV-01).
fn refusal(status: LexicalStatus, generation: Generation) -> Option<CribleError> {
    let generation = generation.get();
    match status {
        // A property of the index, known when its configuration was validated:
        // retrying cannot succeed, so the code is not `overloaded` (ADR-0089,
        // decision 3).
        LexicalStatus::Disabled => Some(CribleError::new(
            ErrorCode::InvalidRequest,
            "lexical branch is disabled for this index",
        )),
        // Nothing to serve: an empty answer is exact (ADR-0089, decision 2).
        LexicalStatus::Missing | LexicalStatus::Ready { .. } => None,
        LexicalStatus::Behind {
            committed: Some(behind),
        } => Some(overloaded(format!(
            "lexical index behind generation {} of {generation}; open the index for writing to \
             repair it",
            behind.get()
        ))),
        // No commit at all: the next write open rebuilds the index of the epoch,
        // it does not catch it up (ADR-0089, decision 1).
        LexicalStatus::Behind { committed: None } => Some(overloaded(format!(
            "no lexical index is committed in this epoch although passages exist (generation \
             {generation}); open the index for writing to rebuild it"
        ))),
        LexicalStatus::Ahead { committed: ahead } => Some(overloaded(format!(
            "lexical index ahead of generation {} of {generation}; open the index for writing to \
             repair it",
            ahead.get()
        ))),
        LexicalStatus::NotCertified {
            committed: Some(uncertified),
        } => Some(overloaded(format!(
            "lexical index state at generation {} is not certified for generation {generation}; \
             open the index for writing to repair it",
            uncertified.get()
        ))),
        LexicalStatus::NotCertified { committed: None } => Some(overloaded(format!(
            "lexical index state is not certified for generation {generation}; open the index for \
             writing to repair it"
        ))),
        LexicalStatus::Unreadable => Some(overloaded(format!(
            "lexical index is unreadable at generation {generation}; open the index for writing to \
             repair it"
        ))),
    }
}

/// The bag of terms `terms` as the query tantivy scores in BM25.
///
/// One `Should` clause per term, so a passage matching any of them is scored as
/// the sum of the scores of the terms it holds (`docs/03`, "sac de termes (OU)").
fn query_of(searcher: &Searcher, terms: &[String]) -> Result<BooleanQuery, CribleError> {
    let field = text_field(searcher)?;
    let terms: Vec<Term> = terms
        .iter()
        .map(|term| Term::from_field_text(field, term))
        .collect();
    Ok(BooleanQuery::new_multiterms_query(terms))
}

/// Field holding the encoder input of a passage, from the schema of the index.
fn text_field(searcher: &Searcher) -> Result<Field, CribleError> {
    searcher
        .schema()
        .get_field(TEXT_FIELD_NAME)
        .map_err(|error| {
            internal(format!(
                "the lexical index has no field '{TEXT_FIELD_NAME}': {error}"
            ))
        })
}

/// Address of the document the lexical index holds under `iid`, if it holds one.
///
/// The `iid` field is indexed as well as fast (`docs/03`), so a term query finds
/// the document of a passage without scanning the segments. A passage the index
/// no longer holds — killed by a later generation and deleted at a commit, or not
/// reached yet by a catch-up — answers `None`.
fn document_of(searcher: &Searcher, iid: Iid) -> Result<Option<DocAddress>, CribleError> {
    let field = searcher
        .schema()
        .get_field(IID_FIELD_NAME)
        .map_err(|error| {
            internal(format!(
                "the lexical index has no field '{IID_FIELD_NAME}': {error}"
            ))
        })?;
    let term = Term::from_field_u64(field, u64::from(iid.get()));
    let found = searcher
        .search(
            &TermQuery::new(term, IndexRecordOption::Basic),
            &DocSetCollector,
        )
        .map_err(|error| internal(format!("the lexical lookup failed: {error}")))?;
    let mut addresses: Vec<DocAddress> = found.into_iter().collect();
    // One passage has one document, so the choice only makes a corrupt index
    // deterministic instead of arbitrary.
    addresses.sort_unstable_by_key(|address| (address.segment_ord, address.doc_id));
    Ok(addresses.into_iter().next())
}

/// Every passage of the pool scoring at least the boundary score of `boundary`,
/// in the order of the answer.
///
/// This is the second pass of a tie at the boundary of the top-k: the identifier
/// cannot decide between passages of equal score, because the answer is ordered
/// on their keys. The keys of the whole boundary group are read by the caller,
/// then the key decides.
fn resolve_boundary(
    searcher: &Searcher,
    query: &BooleanQuery,
    bitmap: &Arc<RoaringBitmap>,
    boundary: &[Ranked],
) -> Result<Vec<Ranked>, CribleError> {
    let Some(floor) = boundary.last().map(|candidate| candidate.score) else {
        return Ok(Vec::new());
    };
    let mut candidates = searcher
        .search(query, &PoolAbove::new(Arc::clone(bitmap), floor))
        .map_err(|error| internal(format!("the lexical search failed: {error}")))?;
    candidates.sort_unstable_by(|left, right| right.cmp(left));
    Ok(candidates)
}

/// Error raised when the structures of the epoch contradict each other, or when
/// tantivy fails where the index claimed to be readable.
fn internal(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::Internal, message)
}

/// Error raised when the lexical branch is not available for this index.
fn overloaded(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::Overloaded, message)
}

/// One candidate of the lexical branch while it competes for the top-k.
///
/// The order is the **intermediate** one of the scan: a higher score first, then
/// the smaller internal identifier. It decides what a segment keeps, never the
/// order of the answer, which compares keys (ADR-0045, ADR-0079). Greater is
/// better, as [`Scored`] does, so a maximum heap of the `k` best can be kept
/// through [`std::cmp::Reverse`].
#[derive(Debug, Clone, Copy)]
struct Ranked {
    /// BM25 score of the passage.
    score: Score,
    /// Internal identifier read from the fast field of the segment.
    iid: u32,
}

impl PartialEq for Ranked {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Ranked {}

impl PartialOrd for Ranked {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Ranked {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score
            .total_cmp(&other.score)
            .then_with(|| other.iid.cmp(&self.iid))
    }
}

/// The `k` best candidates seen so far, and whether a tie makes them ambiguous.
///
/// One instance serves a segment; the merge keeps a second one over the
/// candidates of every segment, with the same rule, so the two cannot disagree.
struct TopK {
    /// Number of candidates to keep.
    k: usize,
    /// Worst first: `Reverse` makes the top of the heap the candidate that goes.
    heap: BinaryHeap<Reverse<Ranked>>,
    /// True once a candidate tied the score of the worst kept one and could not
    /// be separated from it, which makes the membership of the top-k depend on
    /// keys the heap does not hold.
    ambiguous: bool,
}

impl TopK {
    /// A heap of the `k` best candidates, `k` being positive.
    fn new(k: usize) -> Self {
        Self {
            k,
            heap: BinaryHeap::new(),
            ambiguous: false,
        }
    }

    /// Offers one candidate to the heap.
    fn offer(&mut self, candidate: Ranked) {
        if self.k == 0 {
            return;
        }
        if self.heap.len() < self.k {
            self.heap.push(Reverse(candidate));
            return;
        }
        let Some(&Reverse(worst)) = self.heap.peek() else {
            return;
        };
        if candidate.score.total_cmp(&worst.score) == Ordering::Equal {
            // A key may rank the candidate before the worst kept one, and no
            // segment can read a key: the boundary has to be resolved later.
            self.ambiguous = true;
        }
        if candidate > worst {
            self.heap.pop();
            self.heap.push(Reverse(candidate));
        }
    }

    /// Offers every candidate of `other`, with the rule of [`TopK::offer`].
    fn absorb(&mut self, other: impl IntoIterator<Item = Ranked>) {
        for candidate in other {
            self.offer(candidate);
        }
    }

    /// Candidates kept, best first, and the ambiguity gathered on the way.
    fn into_sorted(self) -> (Vec<Ranked>, bool) {
        let mut candidates: Vec<Ranked> = self
            .heap
            .into_iter()
            .map(|Reverse(candidate)| candidate)
            .collect();
        candidates.sort_unstable_by(|left, right| right.cmp(left));
        (candidates, self.ambiguous)
    }
}

/// Fruit of the first pass: the candidates the segments kept, and what the
/// search must do about them.
struct Found {
    /// Candidates kept, best first, at most `k` of them.
    candidates: Vec<Ranked>,
    /// True when a tie at the boundary must be resolved on the keys.
    ambiguous: bool,
    /// True when a value of the `iid` fast field is not a Crible identifier.
    foreign_iid: bool,
}

/// Collector of the `k` best passages of the pool, by BM25 score.
///
/// The pool bitmap is shared with the segment collectors, which must own their
/// inputs: a [`SegmentCollector`] is `'static`, so it cannot borrow the
/// [`Pool`] of the query.
struct PoolTopK {
    bitmap: Arc<RoaringBitmap>,
    k: usize,
}

impl PoolTopK {
    /// Collector of the `k` best passages of `bitmap`.
    fn new(bitmap: Arc<RoaringBitmap>, k: usize) -> Self {
        Self { bitmap, k }
    }
}

impl Collector for PoolTopK {
    type Fruit = Found;
    type Child = PoolTopKSegment;

    fn for_segment(
        &self,
        _segment_ord: SegmentOrdinal,
        segment: &SegmentReader,
    ) -> tantivy::Result<Self::Child> {
        Ok(PoolTopKSegment {
            bitmap: Arc::clone(&self.bitmap),
            iid: segment.fast_fields().u64(IID_FIELD_NAME)?,
            top: TopK::new(self.k),
            foreign_iid: false,
        })
    }

    fn requires_scoring(&self) -> bool {
        true
    }

    fn merge_fruits(&self, fruits: Vec<SegmentFound>) -> tantivy::Result<Found> {
        let mut top = TopK::new(self.k);
        let mut foreign_iid = false;
        for fruit in fruits {
            top.absorb(fruit.candidates);
            top.ambiguous = top.ambiguous || fruit.ambiguous;
            foreign_iid = foreign_iid || fruit.foreign_iid;
        }
        let (candidates, ambiguous) = top.into_sorted();
        Ok(Found {
            candidates,
            ambiguous,
            foreign_iid,
        })
    }
}

/// Candidates one segment kept.
struct SegmentFound {
    /// Candidates of the segment, in its own intermediate order.
    candidates: Vec<Ranked>,
    /// Ambiguity the segment gathered.
    ambiguous: bool,
    /// Whether the segment held a value the `iid` field cannot carry.
    foreign_iid: bool,
}

/// Collector of one segment: it reads the `iid` fast field, drops the documents
/// outside the pool, and offers the others to a heap of `k`.
struct PoolTopKSegment {
    bitmap: Arc<RoaringBitmap>,
    iid: Column<u64>,
    top: TopK,
    foreign_iid: bool,
}

impl SegmentCollector for PoolTopKSegment {
    type Fruit = SegmentFound;

    fn collect(&mut self, doc: DocId, score: Score) {
        let Some(value) = self.iid.first(doc) else {
            return;
        };
        let Ok(iid) = u32::try_from(value) else {
            self.foreign_iid = true;
            return;
        };
        if !self.bitmap.contains(iid) {
            return;
        }
        self.top.offer(Ranked { score, iid });
    }

    fn harvest(self) -> SegmentFound {
        let (mut candidates, ambiguous) = self.top.into_sorted();
        candidates.shrink_to_fit();
        SegmentFound {
            candidates,
            ambiguous,
            foreign_iid: self.foreign_iid,
        }
    }
}

/// Collector of every pool passage scoring at least `floor`.
///
/// It is the second pass of a tie at the boundary of the top-k: the whole
/// boundary group is kept, because only the keys can say which of its passages
/// belong to the answer.
struct PoolAbove {
    bitmap: Arc<RoaringBitmap>,
    floor: Score,
}

impl PoolAbove {
    /// Collector of the passages of `bitmap` scoring at least `floor`.
    fn new(bitmap: Arc<RoaringBitmap>, floor: Score) -> Self {
        Self { bitmap, floor }
    }
}

impl Collector for PoolAbove {
    type Fruit = Vec<Ranked>;
    type Child = PoolAboveSegment;

    fn for_segment(
        &self,
        _segment_ord: SegmentOrdinal,
        segment: &SegmentReader,
    ) -> tantivy::Result<Self::Child> {
        Ok(PoolAboveSegment {
            bitmap: Arc::clone(&self.bitmap),
            floor: self.floor,
            iid: segment.fast_fields().u64(IID_FIELD_NAME)?,
            found: Vec::new(),
        })
    }

    fn requires_scoring(&self) -> bool {
        true
    }

    fn merge_fruits(&self, fruits: Vec<Vec<Ranked>>) -> tantivy::Result<Vec<Ranked>> {
        Ok(fruits.into_iter().flatten().collect())
    }
}

/// Collector of one segment above the boundary score.
struct PoolAboveSegment {
    bitmap: Arc<RoaringBitmap>,
    floor: Score,
    iid: Column<u64>,
    found: Vec<Ranked>,
}

impl SegmentCollector for PoolAboveSegment {
    type Fruit = Vec<Ranked>;

    fn collect(&mut self, doc: DocId, score: Score) {
        if score.total_cmp(&self.floor) == Ordering::Less {
            return;
        }
        let Some(value) = self.iid.first(doc) else {
            return;
        };
        let Ok(iid) = u32::try_from(value) else {
            return;
        };
        if !self.bitmap.contains(iid) {
            return;
        }
        self.found.push(Ranked { score, iid });
    }

    fn harvest(self) -> Vec<Ranked> {
        self.found
    }
}

#[cfg(test)]
mod tests {
    use super::{Ranked, TopK};

    /// A candidate is better with a higher score, then with a smaller
    /// identifier: the intermediate order of the scan, which decides what a
    /// segment keeps.
    #[test]
    fn the_intermediate_order_ranks_score_then_identifier() {
        let low = Ranked { score: 1.0, iid: 0 };
        let high = Ranked { score: 2.0, iid: 9 };
        assert!(high > low, "a higher score wins whatever the identifier");
        let first = Ranked { score: 1.0, iid: 1 };
        let second = Ranked { score: 1.0, iid: 2 };
        assert!(first > second, "at equal score the smaller identifier wins");
    }

    /// A tie with the worst kept candidate is ambiguous: the heap cannot know
    /// whether the turned-away passage has the smaller key, so the search has to
    /// read the keys of the boundary.
    #[test]
    fn a_tie_with_the_worst_kept_candidate_is_ambiguous() {
        let mut top = TopK::new(2);
        top.offer(Ranked { score: 3.0, iid: 1 });
        top.offer(Ranked { score: 2.0, iid: 2 });
        let (kept, ambiguous) = top.into_sorted();
        assert!(
            !ambiguous,
            "a full heap with distinct scores is not ambiguous"
        );
        assert_eq!(kept.len(), 2);

        let mut top = TopK::new(2);
        top.offer(Ranked { score: 3.0, iid: 1 });
        top.offer(Ranked { score: 2.0, iid: 2 });
        top.offer(Ranked { score: 2.0, iid: 0 });
        let (kept, ambiguous) = top.into_sorted();
        assert!(
            ambiguous,
            "a tie with the worst kept candidate is ambiguous"
        );
        let iids: Vec<u32> = kept.iter().map(|candidate| candidate.iid).collect();
        assert_eq!(
            iids,
            vec![1, 0],
            "the intermediate order still keeps the smaller identifier"
        );
    }

    /// `k == 0` keeps nothing, and no candidate can make it ambiguous.
    #[test]
    fn an_empty_top_k_keeps_nothing() {
        let mut top = TopK::new(0);
        top.offer(Ranked { score: 1.0, iid: 0 });
        let (kept, ambiguous) = top.into_sorted();
        assert!(kept.is_empty(), "a heap of size zero keeps no candidate");
        assert!(!ambiguous, "nothing is ambiguous when nothing is kept");
    }
}
