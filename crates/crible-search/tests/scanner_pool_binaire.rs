// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `SCANNER-POOL-BINAIRE` — the binary scan returns the top-N by Hamming
//! distance, as a full sort does.
//!
//! References: `docs/phases/P4-recherche-rapide.md` (`SCANNER-POOL-BINAIRE`,
//! criteria "égal à un tri complet par Hamming sur `synth-small`" and
//! "égalités départagées par iid"), `docs/03-recherche.md` ("Branche dense",
//! strategy BINARY and "Parallélisme"), ADR-0004, ADR-0069, ADR-0081 (a single
//! traversal of the scan) and ADR-0082 (the segment cursor).
//!
//! # Oracle
//!
//! A full sort is recomputed here from the codes of the snapshot, with
//! `u64::count_ones` and no call into `crible_search::kernels`: the oracle is
//! independent of the implementation under test. The order is the one of the
//! scan: ascending distance, then ascending internal identifier.
//!
//! # Datasets
//!
//! - `synth-small` (the 200 queries of `tools/queries.jsonl`) proves the
//!   criterion on the reference dataset; `data/` is gitignored, so a missing
//!   file is reported with the command that creates it;
//! - a fixture with 50 passages lets each test choose the shape of its pool
//!   through a filter (full, dense, sparse, empty) and control the codes, which
//!   is what the tie-break test needs.
//!
//! The scan itself is compared with the oracle by
//! `crates/crible-search/src/dense/binary.rs`, on the same pool; here the public
//! entry point is scanned through the `Pool` a filter builds.

mod common;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use crible_core::config::IndexConfig;
use crible_core::error::ErrorCode;
use crible_core::filter::Filter;
use crible_core::ids::{Epoch, Generation};
use crible_core::model::ModelConfig;
use crible_search::dense::binary::{Candidate, top_k};
use crible_search::pool::Pool;
use crible_search::scan_pool::ScanPool;
use crible_store::attach::attach;
use crible_store::codes::{quantize, words_per_code};
use crible_store::generation::{GenerationStore, Snapshot, write_index_config};
use serde_json::Value;

use common::dense::{DenseFixture, Sample, dataset_root, match_all};
use common::fixture_schema;

/// Epoch the fixture indexes create.
const EPOCH: u32 = 1;

/// Command that generates the reference dataset.
const DATASET_COMMAND: &str =
    "cd tools && uv sync && uv run python gen_synth.py --out ../data/synth-small";

/// Dimensions of the fixture: three words per code, like a 384-dimension model.
const DIMS: u32 = 130;

/// Number of passages of the fixture of the shape tests.
const SAMPLES: u32 = 50;

// --- Oracle ---

/// Code of `iid`, read from the snapshot as the scan reads it.
fn code_of(snapshot: &Snapshot, dims: u32, iid: u32) -> Vec<u64> {
    let segment = snapshot
        .segments()
        .iter()
        .find(|meta| {
            let start = meta.iid_start.get();
            iid >= start && iid - start < meta.count
        })
        .unwrap_or_else(|| panic!("iid {iid} must belong to a segment of the generation"));
    let words = words_per_code(dims);
    let codes = snapshot
        .codes_of(&segment.segment_id)
        .unwrap_or_else(|| panic!("segment '{}' must hold codes", segment.segment_id));
    let offset = usize::try_from(iid - segment.iid_start.get()).unwrap_or(usize::MAX) * words;
    codes[offset..offset + words].to_vec()
}

/// Full Hamming sort of the pool: the oracle of the scan.
fn full_sort(snapshot: &Snapshot, dims: u32, pool: &Pool, query_code: &[u64]) -> Vec<(u32, u64)> {
    let mut results: Vec<(u32, u64)> = pool
        .bitmap()
        .iter()
        .map(|iid| {
            let code = code_of(snapshot, dims, iid);
            let distance: u64 = query_code
                .iter()
                .zip(&code)
                .map(|(left, right)| u64::from((left ^ right).count_ones()))
                .sum();
            (iid, distance)
        })
        .collect();
    results.sort_unstable_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
    results
}

/// The scan result as `(iid, distance)` pairs, in its own order.
fn as_pairs(candidates: &[Candidate]) -> Vec<(u32, u64)> {
    candidates
        .iter()
        .map(|candidate| (candidate.iid.get(), candidate.distance))
        .collect()
}

// --- Fixtures ---

/// Deterministic vector whose signs vary with the passage index.
fn vector(seed: u32) -> Vec<f32> {
    (0..DIMS)
        .map(|component| {
            let value = (u64::from(seed) * 31 + u64::from(component) * 17) % 7;
            value as f32 - 3.0
        })
        .collect()
}

/// Fixture of [`SAMPLES`] passages, one per document `D0` … `D49`.
fn fixture() -> DenseFixture {
    let mut fixture = DenseFixture::create(DIMS);
    let samples: Vec<Sample> = (0..SAMPLES)
        .map(|index| Sample::new(&format!("D{index}"), 0, vector(index)))
        .collect();
    fixture.add_generation(&samples);
    fixture
}

/// Filter of a JSON expression, validated against the fixture schema.
fn filter(json: &str) -> Filter {
    let filter = Filter::from_json_str(json)
        .unwrap_or_else(|error| panic!("the filter {json} must parse: {error}"));
    filter
        .validate(&fixture_schema())
        .unwrap_or_else(|error| panic!("the filter {json} must be valid: {error}"));
    filter
}

/// Query code of the fixture scan.
fn query_code() -> Vec<u64> {
    let vector: Vec<f32> = (0..DIMS)
        .map(|component| ((component * 13) % 5) as f32 - 2.0)
        .collect();
    quantize(&vector)
}

/// Sizes of the segments of the cursor fixture, in identifier order.
///
/// They are deliberately unequal, and include a segment of one passage, one of
/// a thousand and one that some pools below never enter: a segment cursor that
/// advances with the pool iterator has to cross every one of those boundaries
/// (ADR-0081, decision 2; ADR-0082, decision 1).
const CURSOR_SEGMENT_SIZES: [u32; 5] = [1, 3, 64, 1_000, 7];

/// Fixture whose segments hold [`CURSOR_SEGMENT_SIZES`] passages, hence the
/// identifier ranges `0..1`, `1..4`, `4..68`, `68..1_068` and `1_068..1_075`.
///
/// One document carries every passage of one segment and its identifier is
/// `S<index>`: a `fonds` condition keeps a whole segment or none of it, and a
/// `rang` condition keeps chosen positions inside a segment, which is how the
/// tests below select pools that skip whole segments or stop on their edges.
fn unequal_segments() -> DenseFixture {
    let mut fixture = DenseFixture::create(DIMS);
    let mut seed = 0_u32;
    for (index, size) in CURSOR_SEGMENT_SIZES.iter().enumerate() {
        let doc_id = format!("S{index}");
        let samples: Vec<Sample> = (0..*size)
            .map(|ord| {
                let sample = Sample::new(&doc_id, ord, vector(seed));
                seed += 1;
                sample
            })
            .collect();
        fixture.add_generation(&samples);
    }
    fixture
}

/// Checks that the cursor fixture has the segment layout its tests claim.
fn assert_cursor_segment_shape(snapshot: &Snapshot) {
    let segments = snapshot.segments();
    assert_eq!(
        segments.len(),
        CURSOR_SEGMENT_SIZES.len(),
        "the cursor fixture must have one segment per declared size"
    );
    let mut start = 0_u32;
    for (index, (segment, size)) in segments.iter().zip(CURSOR_SEGMENT_SIZES).enumerate() {
        assert_eq!(
            segment.iid_start.get(),
            start,
            "segment {index} must start at identifier {start}"
        );
        assert_eq!(
            segment.count, size,
            "segment {index} must hold {size} passages"
        );
        start += size;
    }
}

/// Runs the public scan on a fixed worker count, so the partitioning of the
/// pool is reproducible.
fn scan_on_workers(
    snapshot: &Snapshot,
    pool: &Pool,
    query_code: &[u64],
    n: usize,
    workers: usize,
) -> Vec<Candidate> {
    ScanPool::new(workers)
        .unwrap_or_else(|error| panic!("a {workers}-worker scan pool must be buildable: {error}"))
        .install(|| top_k(snapshot, pool, query_code, n))
        .unwrap_or_else(|error| panic!("the {workers}-worker scan must succeed: {error}"))
}

// --- Tests ---

/// The scan equals a full Hamming sort on every shape of pool: full, dense,
/// sparse, empty, and with an `n` above the pool size.
#[test]
fn every_pool_shape_matches_a_full_hamming_sort() {
    let fixture = fixture();
    let snapshot = fixture.snapshot();
    let query_code = query_code();

    let dense: Vec<String> = (0..40).map(|index| format!("\"D{index}\"")).collect();
    let cases = [
        (match_all(), 10_usize),
        (
            filter(&format!("{{\"fonds\": {{\"in\": [{}]}}}}", dense.join(","))),
            10,
        ),
        (filter("{\"fonds\": \"D7\"}"), 10),
        (filter("{\"fonds\": \"nowhere\"}"), 10),
        (match_all(), 1_000),
    ];

    for (case, (filter, n)) in cases.iter().enumerate() {
        let pool = fixture.pool(filter);
        let mut expected = full_sort(&snapshot, DIMS, &pool, &query_code);
        expected.truncate(*n);
        let got = top_k(&snapshot, &pool, &query_code, *n)
            .unwrap_or_else(|error| panic!("the scan of case {case} must succeed: {error}"));
        assert_eq!(
            as_pairs(&got),
            expected,
            "case {case} (pool of {} passages, n = {n}) must equal a full Hamming sort",
            pool.len()
        );
    }
}

/// Equal distances are broken by ascending internal identifier, whatever the
/// document identifiers: the fixture hands out the iids in the sample order,
/// and the sample order here is the reverse of the alphabetical one.
#[test]
fn equal_distances_are_broken_by_ascending_iid() {
    let mut fixture = DenseFixture::create(DIMS);
    // The query code is all ones over the 130 dimensions, so the distance of a
    // vector is the number of its negative components. Every sample has exactly
    // two, at a different pair of positions: all the distances are equal.
    let query_vector = vec![1.0_f32; DIMS as usize];
    let query_code = quantize(&query_vector);
    let samples: Vec<Sample> = (0..6)
        .map(|index| {
            let mut vector = vec![1.0_f32; DIMS as usize];
            let position = usize::try_from(index).unwrap_or(0);
            vector[position] = -1.0;
            vector[position + 1] = -1.0;
            let doc_id = format!("D{}", 5 - index);
            Sample::new(&doc_id, 0, vector)
        })
        .collect();
    fixture.add_generation(&samples);
    let snapshot = fixture.snapshot();
    let pool = fixture.pool(&match_all());

    let got = top_k(&snapshot, &pool, &query_code, 3)
        .unwrap_or_else(|error| panic!("the tie-break scan must succeed: {error}"));
    assert_eq!(
        as_pairs(&got),
        vec![(0, 2), (1, 2), (2, 2)],
        "three equal distances must keep the three smallest iids, in ascending order"
    );

    let all = top_k(&snapshot, &pool, &query_code, 6)
        .unwrap_or_else(|error| panic!("the tie-break scan must succeed: {error}"));
    assert_eq!(
        as_pairs(&all),
        (0..6).map(|iid| (iid, 2)).collect::<Vec<_>>(),
        "the whole pool must be ordered by ascending iid when every distance is equal"
    );
}

/// The answer is the same with one worker and with several: the partitioning of
/// the pool cannot change it.
#[test]
fn the_result_does_not_depend_on_the_thread_count() {
    let fixture = fixture();
    let snapshot = fixture.snapshot();
    let query_code = query_code();
    let pools = [
        fixture.pool(&match_all()),
        fixture.pool(&filter("{\"fonds\": \"D13\"}")),
        fixture.pool(&filter("{\"fonds\": \"nowhere\"}")),
    ];

    for (case, pool) in pools.iter().enumerate() {
        let single = ScanPool::new(1)
            .unwrap_or_else(|error| panic!("a one-worker scan pool must be buildable: {error}"))
            .install(|| top_k(&snapshot, pool, &query_code, 7));
        let several = ScanPool::new(4)
            .unwrap_or_else(|error| panic!("a four-worker scan pool must be buildable: {error}"))
            .install(|| top_k(&snapshot, pool, &query_code, 7));
        let single = single.unwrap_or_else(|error| panic!("the one-worker scan must run: {error}"));
        let several =
            several.unwrap_or_else(|error| panic!("the four-worker scan must run: {error}"));
        assert_eq!(
            single, several,
            "case {case}: one worker and four workers must give the same answer"
        );
    }
}

// --- Segment cursor: unequal segments ---

/// The scan of pools laid out across segments of very unequal sizes equals a
/// full Hamming sort.
///
/// The pools cover every case ADR-0081 and ADR-0082 name: the whole live set,
/// which crosses every segment boundary; two pools that skip whole segments,
/// one dense and one sparse; a sparse pool spread over the five segments; and a
/// sparse pool that never enters the segment of one passage. Each is scanned on
/// one worker and on three, because a window boundary can fall inside a segment
/// and the segment lookup must agree there too.
#[test]
fn a_pool_across_unequal_segments_matches_a_full_hamming_sort() {
    let fixture = unequal_segments();
    let snapshot = fixture.snapshot();
    assert_cursor_segment_shape(&snapshot);
    let query_code = query_code();

    let cases = [
        ("the whole live set", match_all()),
        (
            "the dense pool that skips S0 and S4",
            filter("{\"fonds\": {\"in\": [\"S1\", \"S3\"]}}"),
        ),
        (
            "the sparse pool that skips S1 and S3",
            filter("{\"fonds\": {\"in\": [\"S0\", \"S2\", \"S4\"]}}"),
        ),
        (
            "a sparse pool crossing every boundary",
            filter("{\"rang\": {\"lte\": 2}}"),
        ),
        (
            "a sparse pool that never enters the one-passage segment",
            filter("{\"rang\": 1}"),
        ),
    ];

    for (case, (name, filter)) in cases.iter().enumerate() {
        let pool = fixture.pool(filter);
        let mut expected = full_sort(&snapshot, DIMS, &pool, &query_code);
        expected.truncate(10);
        for workers in [1, 3] {
            let got = scan_on_workers(&snapshot, &pool, &query_code, 10, workers);
            assert_eq!(
                as_pairs(&got),
                expected,
                "case {case} ({name}, {} passages, {workers} worker(s)) must equal a full \
                 Hamming sort",
                pool.len()
            );
        }
    }
}

/// A pool made only of segment edges equals a full Hamming sort: the first and
/// last identifier of a segment, the first identifier of every segment, and a
/// last identifier alone.
///
/// These are the identifiers where a segment cursor starts, has to move on, or
/// stops, so an off-by-one in the cursor shows here first.
#[test]
fn a_pool_limited_to_segment_borders_matches_a_full_hamming_sort() {
    let fixture = unequal_segments();
    let snapshot = fixture.snapshot();
    assert_cursor_segment_shape(&snapshot);
    let query_code = query_code();

    let cases = [
        (
            "the first and last identifier of S2 and S3",
            filter("{\"fonds\": {\"in\": [\"S2\", \"S3\"]}, \"rang\": {\"in\": [0, 63, 999]}}"),
        ),
        (
            "the first identifier of every segment",
            filter("{\"rang\": 0}"),
        ),
        (
            "the last identifier of a segment alone",
            filter("{\"fonds\": \"S4\", \"rang\": 6}"),
        ),
        (
            "the last identifier of the thousand-passage segment",
            filter("{\"fonds\": \"S3\", \"rang\": 999}"),
        ),
    ];

    for (case, (name, filter)) in cases.iter().enumerate() {
        let pool = fixture.pool(filter);
        let mut expected = full_sort(&snapshot, DIMS, &pool, &query_code);
        expected.truncate(10);
        assert_eq!(
            as_pairs(&scan_on_workers(&snapshot, &pool, &query_code, 10, 1)),
            expected,
            "case {case} ({name}) must equal a full Hamming sort"
        );
    }
}

/// An `n` above a multi-segment pool returns the whole pool, in the order of a
/// full sort.
#[test]
fn n_above_a_multi_segment_pool_returns_the_whole_pool() {
    let fixture = unequal_segments();
    let snapshot = fixture.snapshot();
    assert_cursor_segment_shape(&snapshot);
    let query_code = query_code();
    let pool = fixture.pool(&filter("{\"fonds\": {\"in\": [\"S0\", \"S1\", \"S3\"]}}"));
    let expected = full_sort(&snapshot, DIMS, &pool, &query_code);

    let got = scan_on_workers(&snapshot, &pool, &query_code, 10_000, 3);
    assert_eq!(
        got.len(),
        usize::try_from(pool.len()).unwrap_or(usize::MAX),
        "the scan must return the whole pool when n exceeds it"
    );
    assert_eq!(
        as_pairs(&got),
        expected,
        "the whole pool must come back in the order of a full Hamming sort"
    );
}

/// A query code whose length is not `ceil(dims / 64)` words is an invalid
/// request, never a common-prefix distance.
#[test]
fn a_query_code_of_the_wrong_length_is_refused() {
    let fixture = fixture();
    let snapshot = fixture.snapshot();
    let pool = fixture.pool(&match_all());
    let words = words_per_code(DIMS);

    for length in [words - 1, words + 1] {
        let query_code = vec![0_u64; length];
        let error = top_k(&snapshot, &pool, &query_code, 10)
            .err()
            .unwrap_or_else(|| panic!("a query code of {length} words must be refused"));
        assert_eq!(
            error.code(),
            ErrorCode::InvalidRequest,
            "a query code of {length} words where the index holds {words} is a bad request"
        );
    }
}

// --- synth-small ---

/// One line of `queries.jsonl`.
#[derive(Debug)]
struct Query {
    /// Reference vector of the query, 768 components.
    vector: Vec<f32>,
    /// JSON filter, or `{}` for the whole live set.
    filter: Value,
}

/// Reads `queries.jsonl`.
fn queries(root: &Path) -> Vec<Query> {
    let path = root.join("queries.jsonl");
    let text = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{} must be readable; generate the dataset:\n  {DATASET_COMMAND}\n  {error}",
            path.display()
        )
    });
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let value: Value = serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("{} must hold JSON lines: {error}", path.display()));
            let vector = value
                .get("vector")
                .and_then(Value::as_array)
                .unwrap_or_else(|| panic!("a query must carry a vector: {value}"))
                .iter()
                .map(|item| {
                    item.as_f64()
                        .map(|number| number as f32)
                        .unwrap_or_else(|| panic!("a query vector must hold numbers: {value}"))
                })
                .collect();
            let filter = value
                .get("filter")
                .cloned()
                .filter(Value::is_object)
                .unwrap_or_else(|| serde_json::json!({}));
            Query { vector, filter }
        })
        .collect()
}

/// Model configuration of `synth-small` (external, 768 dimensions).
fn dataset_model() -> ModelConfig {
    ModelConfig::from_json_str(
        r#"{ "name": "synthetic-external", "kind": "external", "dims": 768, "normalize": true }"#,
    )
    .unwrap_or_else(|error| panic!("the dataset model configuration is valid: {error}"))
}

/// Builds the `synth-small` index and returns its snapshot.
fn synth_small(temp: &Path) -> (Arc<Snapshot>, IndexConfig) {
    let root = dataset_root();
    let config_path = root.join("index.json");
    let text = fs::read_to_string(&config_path).unwrap_or_else(|error| {
        panic!(
            "{} must be readable; generate the dataset:\n  {DATASET_COMMAND}\n  {error}",
            config_path.display()
        )
    });
    let index = IndexConfig::from_json_str(&text)
        .unwrap_or_else(|error| panic!("the dataset index.json must be valid: {error}"));
    let index_dir = temp.join("indexes").join("scanner-pool-binaire");
    write_index_config(&index_dir, &text)
        .unwrap_or_else(|error| panic!("the index configuration must be written: {error}"));
    let mut store = GenerationStore::create(&index_dir, Epoch::new(EPOCH), &index.schema)
        .unwrap_or_else(|error| panic!("the index must be created: {error}"));

    let model = dataset_model();
    for (name, generation) in [("l1", 1), ("l2", 2), ("l3", 3)] {
        let batch = root.join(name);
        assert!(
            batch.join("manifest.json").is_file(),
            "the synthetic batch '{name}' is missing at {}; generate the dataset:\n  \
             {DATASET_COMMAND}",
            batch.display()
        );
        attach(&mut store, &batch, &index, &model, None, None).unwrap_or_else(|error| {
            panic!(
                "the attach of {} must succeed, got {}: {}",
                batch.display(),
                error.code().as_str(),
                error.message()
            )
        });
        assert_eq!(
            store.generation(),
            Generation::new(generation),
            "'{name}' must publish generation {generation}"
        );
    }
    (store.snapshot(), index)
}

/// The 200 queries of `synth-small` return the top 10 of a full Hamming sort.
#[test]
fn synth_small_matches_a_full_hamming_sort() {
    let temp = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
    let (snapshot, index) = synth_small(temp.path());
    let dims = snapshot
        .segments()
        .first()
        .map(|meta| meta.dims)
        .unwrap_or_else(|| panic!("the synth-small index must hold a segment"));

    let queries = queries(&dataset_root());
    assert_eq!(
        queries.len(),
        200,
        "queries.jsonl must hold the 200 queries"
    );

    for (position, query) in queries.iter().enumerate() {
        let filter = Filter::from_json_str(&query.filter.to_string())
            .unwrap_or_else(|error| panic!("query {position}: the filter must parse: {error}"));
        filter.validate(&index.schema).unwrap_or_else(|error| {
            panic!("query {position}: the filter must be valid against the schema: {error}")
        });
        let pool = Pool::evaluate(snapshot.filters(), &filter)
            .unwrap_or_else(|error| panic!("query {position}: the pool must evaluate: {error}"));
        let query_code = quantize(&query.vector);
        let mut expected = full_sort(&snapshot, dims, &pool, &query_code);
        expected.truncate(10);
        let got = top_k(&snapshot, &pool, &query_code, 10)
            .unwrap_or_else(|error| panic!("query {position}: the scan must succeed: {error}"));
        assert_eq!(
            as_pairs(&got),
            expected,
            "query {position}: the scan must equal a full Hamming sort over {} passages",
            pool.len()
        );
    }
}
