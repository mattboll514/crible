// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! Fixtures and reference comparison shared by the dense exact search tests
//! (`INV-06`, task `RECHERCHER-DENSE-EXACT`, phase P1).
//!
//! # What this module builds
//!
//! [`DenseFixture`] builds a real index in a temporary directory: a schema with
//! one document field and one passage field, one generation per
//! [`DenseFixture::add_generation`] call, one fp16 segment per generation, and
//! the matching rows in `records.sqlite`. The samples give the vectors, the
//! texts and both levels of fields, so a test states exactly which passages it
//! searches and why the expected order is the one it asserts.
//!
//! # The reference comparison
//!
//! [`compare_to_reference`] is the only place that knows the tolerances of the
//! task sheet. It compares one query's results with the reference of
//! `tools/reference_topk.py` (`expected.jsonl`) and enforces, in this order:
//!
//! 1. the number of results, equal to the reference: `min(k, pool_size)`;
//! 2. no duplicate key in the answer;
//! 3. every key of the reference is returned, except at most **one** key at the
//!    k-th place, and then a key the reference does not hold replaces the last
//!    one, with a score within `SCORE_TOLERANCE` of the displaced key: that is
//!    the only replacement the sheet allows;
//! 4. the score of every key within `SCORE_TOLERANCE` of its reference score;
//! 5. the answer is sorted by descending score, with a tie-break on the key in
//!    ascending order;
//! 6. the reference order is preserved, except for an exchange of two
//!    **adjacent** reference results whose reference scores differ by less than
//!    `SCORE_TOLERANCE`. Any other inversion is an error, in particular an
//!    exchange of non-adjacent results and an inversion of two results whose
//!    scores differ by more than the tolerance.
//!
//! The two tolerances of the sheet are therefore not a blanket "compare with a
//! tolerance": they are attached to the two cases the sheet describes. The unit
//! tests in `inv_06_dense_exact_synth.rs` pin that the comparison rejects what
//! it must reject.

// The module is included by every test binary of this crate, and none uses every
// helper. The same allowance as `crates/crible-store/tests/common/mod.rs`.
#![allow(dead_code)]

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crible_core::config::{ReferencePrecision, Schema};
use crible_core::filter::Filter;
use crible_core::ids::{Epoch, Generation, Iid};
use crible_search::dense::exact::{ExactSearcher, Hit};
use crible_search::pool::Pool;
use crible_store::generation::{GenerationChange, GenerationStore, Snapshot, write_index_config};
use crible_store::records::{DocumentRecord, PassageRecord};
use crible_store::segment::{SegmentSpec, SegmentWriter};
use serde_json::{Map, Value, json};

/// Epoch every fixture index creates.
pub const EPOCH: u32 = 1;

/// Tolerance on a score, from the acceptance criteria of the task sheet:
/// "égalité à 1e-5 près sur les scores".
pub const SCORE_TOLERANCE: f32 = 1e-5;

/// JSON schema of the fixture index.
///
/// One field per level, of the two levels the payload of a result merges: a
/// document field (`fonds`, a `keyword`) and a passage field (`rang`, an
/// `integer`). ADR-0030 forbids the same name at both levels, so the merged
/// object the payload returns is unambiguous.
const SCHEMA_JSON: &str = r#"{
    "document_fields": { "fonds": { "type": "keyword" } },
    "passage_fields": { "rang": { "type": "integer" } }
}"#;

/// Schema of the fixture index.
pub fn fixture_schema() -> Schema {
    Schema::from_json_str(SCHEMA_JSON)
        .unwrap_or_else(|error| panic!("the fixture schema must be valid: {error}"))
}

/// The empty conjunction: every live passage satisfies it, so its pool is the
/// whole live set (`{}` in the filter grammar).
pub fn match_all() -> Filter {
    Filter::And(Vec::new())
}

/// One passage the fixture writes: its identity, its payload and its vector.
pub struct Sample {
    /// External identifier of the document the passage belongs to.
    pub doc_id: String,
    /// Position of the passage in its document.
    pub ord: u32,
    /// Document fields, stored once per document.
    pub doc_fields: Value,
    /// Passage fields.
    pub passage_fields: Value,
    /// Text of the passage, stored as a zstd frame in `records.sqlite`.
    pub text: String,
    /// Reference vector of the passage, written in fp16.
    pub vector: Vec<f32>,
    /// Content digest of the passage; the default of `(doc_id, ord)` when
    /// `None`. `dedup: content` compares it, so a test that wants two passages
    /// of identical content sets the same digest on both.
    passage_hash: Option<String>,
}

impl Sample {
    /// Sample with the default payload of `(doc_id, ord)`: `fonds = doc_id`,
    /// `rang = ord`, and a text naming the key.
    pub fn new(doc_id: &str, ord: u32, vector: Vec<f32>) -> Self {
        Self {
            doc_id: doc_id.to_owned(),
            ord,
            doc_fields: json!({ "fonds": doc_id }),
            passage_fields: json!({ "rang": ord }),
            text: format!("{doc_id}#{ord} text"),
            vector,
            passage_hash: None,
        }
    }

    /// Replaces the document fields.
    #[must_use]
    pub fn with_doc_fields(mut self, fields: Value) -> Self {
        self.doc_fields = fields;
        self
    }

    /// Replaces the passage fields.
    #[must_use]
    pub fn with_passage_fields(mut self, fields: Value) -> Self {
        self.passage_fields = fields;
        self
    }

    /// Replaces the text.
    #[must_use]
    pub fn with_text(mut self, text: &str) -> Self {
        self.text = text.to_owned();
        self
    }

    /// Replaces the content digest of the passage, which `dedup: content`
    /// compares instead of the text.
    #[must_use]
    pub fn with_passage_hash(mut self, hash: &str) -> Self {
        self.passage_hash = Some(hash.to_owned());
        self
    }
}

/// One index in a temporary directory, its writer and its iid counter.
pub struct DenseFixture {
    // The directory must outlive the index: dropping it removes the files.
    _temp: tempfile::TempDir,
    index: PathBuf,
    dims: u32,
    precision: ReferencePrecision,
    store: GenerationStore,
    next_iid: u32,
}

impl DenseFixture {
    /// Creates an empty index whose vectors have `dims` dimensions, stored in
    /// `fp16`, the default reference precision of `docs/03-recherche.md`.
    pub fn create(dims: u32) -> Self {
        Self::create_with_precision(dims, ReferencePrecision::Fp16)
    }

    /// Creates an empty index whose vectors have `dims` dimensions, stored in
    /// `precision`.
    ///
    /// The precision is frozen at creation (ADR-0005): it is written in the
    /// configuration and used for every segment the fixture adds.
    pub fn create_with_precision(dims: u32, precision: ReferencePrecision) -> Self {
        let temp = tempfile::tempdir().unwrap_or_else(|error| {
            panic!("a temporary directory for the index must be creatable: {error}")
        });
        let index = temp.path().join("indexes").join("dense-exact");
        let schema = fixture_schema();
        write_index_config(
            &index,
            &super::test_config_with_precision("dense-exact", &schema, precision),
        )
        .unwrap_or_else(|error| panic!("the index configuration must be written: {error}"));
        let store = GenerationStore::create(&index, Epoch::new(EPOCH), &schema)
            .unwrap_or_else(|error| panic!("the fixture index must be created: {error}"));
        Self {
            _temp: temp,
            index,
            dims,
            precision,
            store,
            next_iid: 0,
        }
    }

    /// Commits one generation holding `samples` as passages, in order.
    ///
    /// The internal identifiers are handed out in the sample order, so a
    /// tie-break on the iid and a tie-break on the key cannot be confused: the
    /// constructed tests choose the doc identifiers so that the two orders
    /// differ.
    pub fn add_generation(&mut self, samples: &[Sample]) -> Generation {
        assert!(
            !samples.is_empty(),
            "a fixture generation must carry at least one sample"
        );
        let generation = self.store.generation().get() + 1;
        let generation = Generation::new(generation);
        let segment_id = format!("segment-{}", generation.get());
        let segments = self.segments_dir();
        fs::create_dir_all(&segments).unwrap_or_else(|error| {
            panic!(
                "the segments directory {} must be creatable: {error}",
                segments.display()
            )
        });

        let spec = SegmentSpec {
            segment_id: segment_id.clone(),
            iid_start: Iid::new(self.next_iid),
            dims: self.dims,
            generation,
            batch_id: format!("batch-{}", generation.get()),
            precision: self.precision,
        };
        let mut writer = SegmentWriter::create(&segments, spec)
            .unwrap_or_else(|error| panic!("the segment writer must start: {error}"));
        for sample in samples {
            assert_eq!(
                sample.vector.len(),
                self.dims as usize,
                "sample {}#{} must have {} components",
                sample.doc_id,
                sample.ord,
                self.dims
            );
            writer.append(&sample.vector).unwrap_or_else(|error| {
                panic!(
                    "the vector of {}#{} must be appended: {error}",
                    sample.doc_id, sample.ord
                )
            });
        }
        let meta = writer
            .finish()
            .unwrap_or_else(|error| panic!("the segment must be finished: {error}"));

        let mut documents = Vec::new();
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut passages = Vec::new();
        let mut iid = self.next_iid;
        for sample in samples {
            if seen.insert(sample.doc_id.as_str()) {
                documents.push(DocumentRecord {
                    doc_id: sample.doc_id.clone(),
                    doc_hash: format!("blake3:doc-{}", sample.doc_id),
                    fields: sample.doc_fields.clone(),
                    updated_generation: generation,
                });
            }
            passages.push(PassageRecord {
                iid: Iid::new(iid),
                doc_id: sample.doc_id.clone(),
                ord: sample.ord,
                text: sample.text.clone(),
                passage_hash: sample
                    .passage_hash
                    .clone()
                    .unwrap_or_else(|| format!("blake3:passage-{}-{}", sample.doc_id, sample.ord)),
                fields: sample.passage_fields.clone(),
                segment_id: segment_id.clone(),
                created_generation: generation,
                dead_since_gen: None,
            });
            iid += 1;
        }
        self.next_iid = iid;

        self.store
            .commit(GenerationChange {
                segments: vec![meta],
                documents,
                passages,
                deletions: Vec::new(),
            })
            .unwrap_or_else(|error| panic!("the fixture commit must succeed: {error}"));
        generation
    }

    /// Removes one document from a new generation, as an attach does when the
    /// producer drops it: the passages of the document are tombstoned at the
    /// new generation, and its `documents` row is deleted (ADR-0041, ADR-0043).
    ///
    /// The row deletion uses SQL directly, because [`GenerationStore::commit`]
    /// alone cannot remove a document: the attach owns that write, inside its
    /// own transaction, and a test cannot build a batch here.
    pub fn remove_document(&mut self, doc_id: &str) {
        let connection = rusqlite::Connection::open(self.records_path())
            .unwrap_or_else(|error| panic!("the fixture records must open: {error}"));
        let deletions: Vec<Iid> = {
            let mut statement = connection
                .prepare("SELECT iid FROM passages WHERE doc_id = ?1")
                .unwrap_or_else(|error| panic!("the fixture passages must be readable: {error}"));
            let rows = statement
                .query_map([doc_id], |row| row.get::<_, i64>(0))
                .unwrap_or_else(|error| panic!("the fixture document must be listed: {error}"));
            rows.map(|row| {
                let iid = row.unwrap_or_else(|error| {
                    panic!("a fixture passage iid must be readable: {error}")
                });
                Iid::new(
                    u32::try_from(iid)
                        .unwrap_or_else(|_| panic!("a fixture passage iid must fit u32: {iid}")),
                )
            })
            .collect()
        };
        self.store
            .commit(GenerationChange {
                segments: Vec::new(),
                documents: Vec::new(),
                passages: Vec::new(),
                deletions,
            })
            .unwrap_or_else(|error| panic!("the fixture removal commit must succeed: {error}"));
        connection
            .execute("DELETE FROM documents WHERE doc_id = ?1", [doc_id])
            .unwrap_or_else(|error| panic!("the fixture document row must be deleted: {error}"));
    }

    /// Directory of the segments of the single epoch.
    pub fn segments_dir(&self) -> PathBuf {
        self.index
            .join("epochs")
            .join(EPOCH.to_string())
            .join("segments")
    }

    /// Path of the reference database of the index.
    pub fn records_path(&self) -> PathBuf {
        self.index
            .join("epochs")
            .join(EPOCH.to_string())
            .join("records.sqlite")
    }

    /// Snapshot of the published generation.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.store.snapshot()
    }

    /// Pool of `filter` on the published generation.
    pub fn pool(&self, filter: &Filter) -> Pool {
        let snapshot = self.snapshot();
        Pool::evaluate(snapshot.filters(), filter)
            .unwrap_or_else(|error| panic!("the fixture filter must evaluate: {error}"))
    }

    /// Searcher over the segments of the published generation.
    pub fn searcher(&self) -> ExactSearcher {
        let snapshot = self.snapshot();
        ExactSearcher::open(
            &self.segments_dir(),
            snapshot.segments(),
            &self.records_path(),
        )
        .unwrap_or_else(|error| panic!("the fixture searcher must open: {error}"))
    }

    /// Runs the exact search of `query` over the pool of `filter`.
    pub fn search(&self, query: &[f32], filter: &Filter, k: usize) -> Vec<Hit> {
        let pool = self.pool(filter);
        let searcher = self.searcher();
        searcher
            .top_k(query, &pool, k)
            .unwrap_or_else(|error| panic!("the fixture search must succeed: {error}"))
    }
}

/// One reference result, as read from `expected.jsonl`.
#[derive(Debug, Clone, PartialEq)]
pub struct ExpectedResult {
    /// Key `doc_id#ord` of the passage.
    pub key: String,
    /// Exact cosine computed by the numpy reference, in f32.
    pub score: f32,
}

impl ExpectedResult {
    /// Reference result with `key` and `score`.
    pub fn new(key: &str, score: f32) -> Self {
        Self {
            key: key.to_owned(),
            score,
        }
    }
}

/// What the comparison observed while it accepted a query's results.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    /// Pairs of adjacent reference results the engine returned in the other
    /// order, which the tolerance allows.
    pub inverted_pairs: usize,
    /// Queries where the k-th key of the reference was replaced by another key
    /// whose score is within the tolerance, which the tolerance allows.
    pub boundary_replacements: usize,
}

/// Compares one query's results with the reference.
///
/// `query_id` only appears in the error messages. An accepted answer returns
/// what the tolerance allowed, so a test can report it; anything the sheet
/// forbids returns an `Err` with a message that names the query, the position
/// and the two keys or scores involved.
pub fn compare_to_reference(
    query_id: u64,
    expected: &[ExpectedResult],
    actual: &[Hit],
) -> Result<Summary, String> {
    if actual.len() != expected.len() {
        return Err(format!(
            "query {query_id}: {} result(s) returned for {} expected; the answer must hold \
             min(k, pool_size) results",
            actual.len(),
            expected.len()
        ));
    }

    let mut expected_index: HashMap<&str, usize> = HashMap::with_capacity(expected.len());
    for (position, result) in expected.iter().enumerate() {
        expected_index.insert(result.key.as_str(), position);
    }

    let mut seen: HashMap<&str, usize> = HashMap::with_capacity(actual.len());
    for (position, hit) in actual.iter().enumerate() {
        if let Some(previous) = seen.insert(hit.key.as_str(), position) {
            return Err(format!(
                "query {query_id}: key {} returned twice, at positions {previous} and {position}",
                hit.key
            ));
        }
    }

    // 1. Keys: every reference key must come back, except at the k-th place.
    let mut extra: Option<&Hit> = None;
    for (position, hit) in actual.iter().enumerate() {
        if expected_index.contains_key(hit.key.as_str()) {
            continue;
        }
        if position + 1 != actual.len() {
            return Err(format!(
                "query {query_id}: key {} is absent from the reference and returned at \
                 position {position}, but only the k-th place may be replaced",
                hit.key
            ));
        }
        extra = Some(hit);
    }

    // 2. Scores: by key, so an accepted inversion cannot compare the score of
    // one key with the score of another.
    for hit in actual {
        if let Some(&position) = expected_index.get(hit.key.as_str()) {
            let gap = (hit.score - expected[position].score).abs();
            if gap > SCORE_TOLERANCE {
                return Err(format!(
                    "query {query_id}: key {} scored {:.9} against {:.9} in the reference, \
                     a gap of {gap:.9} above the {SCORE_TOLERANCE} tolerance",
                    hit.key, hit.score, expected[position].score
                ));
            }
        }
    }

    // 3. Sorted by descending score, ties broken by ascending key.
    for window in actual.windows(2) {
        let (left, right) = (&window[0], &window[1]);
        let ordered = left.score > right.score
            || (left.score == right.score && left.key.as_str() < right.key.as_str());
        if !ordered {
            return Err(format!(
                "query {query_id}: {} (score {:.9}) is returned before {} (score {:.9}); \
                 results must be sorted by descending score, ties by ascending key",
                left.key, left.score, right.key, right.score
            ));
        }
    }

    // 4. Boundary replacement: only the last key, only against the last
    // reference result, and only within the tolerance.
    let mut boundary_replacements = 0;
    if let Some(extra_hit) = extra {
        let last = expected.len() - 1;
        if expected_index.contains_key(extra_hit.key.as_str()) {
            return Err(format!(
                "query {query_id}: key {} cannot replace the k-th result, it is already in the \
                 reference",
                extra_hit.key
            ));
        }
        if actual
            .iter()
            .any(|hit| hit.key == expected[last].key && hit.key != extra_hit.key)
        {
            return Err(format!(
                "query {query_id}: key {} is absent from the answer while the reference key {} \
                 is still returned elsewhere; a k-th place replacement removes it",
                extra_hit.key, expected[last].key
            ));
        }
        let gap = (extra_hit.score - expected[last].score).abs();
        if gap > SCORE_TOLERANCE {
            return Err(format!(
                "query {query_id}: key {} replaces the k-th result at position {last} with score \
                 {:.9}, {gap:.9} away from the reference score {:.9}; a replacement is only \
                 allowed within {SCORE_TOLERANCE}",
                extra_hit.key, extra_hit.score, expected[last].score
            ));
        }
        boundary_replacements = 1;
    }

    // 5. Order: the reference order is kept, except between adjacent reference
    // results whose scores differ by less than the tolerance.
    let compared = if boundary_replacements == 0 {
        actual
    } else {
        &actual[..actual.len() - 1]
    };
    let positions: Vec<usize> = compared
        .iter()
        .map(|hit| {
            *expected_index.get(hit.key.as_str()).unwrap_or_else(|| {
                panic!(
                    "query {query_id}: key {} was accepted above but has no reference position",
                    hit.key
                )
            })
        })
        .collect();

    let mut inverted_pairs = 0;
    for (left_index, &left_position) in positions.iter().enumerate() {
        for &right_position in &positions[left_index + 1..] {
            if right_position >= left_position {
                continue;
            }
            let gap = (expected[left_position].score - expected[right_position].score).abs();
            if left_position - right_position != 1 {
                return Err(format!(
                    "query {query_id}: results {} (position {left_position}) and {} \
                     (position {right_position}) are inverted, but they are not adjacent in the \
                     reference; only an exchange of adjacent results is allowed",
                    expected[right_position].key, expected[left_position].key
                ));
            }
            if gap >= SCORE_TOLERANCE {
                return Err(format!(
                    "query {query_id}: results {} (position {left_position}) and {} \
                     (position {right_position}) are inverted, but their reference scores differ \
                     by {gap:.9}, which is not below the {SCORE_TOLERANCE} tolerance",
                    expected[right_position].key, expected[left_position].key
                ));
            }
            inverted_pairs += 1;
        }
    }

    Ok(Summary {
        inverted_pairs,
        boundary_replacements,
    })
}

/// Merges the document fields and the passage fields of a result, as the
/// payload of one result carries them.
///
/// ADR-0030 makes a field name designate exactly one level, so the two objects
/// cannot collide and the merge is the reading of `docs/05-api.md`, whose
/// result has a single `fields` object.
pub fn merged_fields(document: &Value, passage: &Value) -> Value {
    let mut merged = Map::new();
    for value in [document, passage] {
        let Value::Object(entries) = value else {
            panic!("the fields of a record must be a JSON object, got {value}");
        };
        for (name, field) in entries {
            merged.insert(name.clone(), field.clone());
        }
    }
    Value::Object(merged)
}

/// Path of the `synth-small` dataset, the game the invariant is measured on.
pub fn dataset_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/synth-small")
}
