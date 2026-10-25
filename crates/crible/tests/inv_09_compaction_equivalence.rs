// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `INV-09`: "Compaction ⇒ mêmes résultats avant et après", checked on the 200
//! queries of `synth-small` and on the queries of the Legifacile sample, as the
//! task `VERIFIER-EQUIVALENCE-COMPACTION` (phase P3) requires.
//!
//! References: `docs/08-invariants-et-tests.md` (INV-09), the task sheet of
//! `docs/phases/P3-cycle-de-vie.md` and the J3 milestone ("trois lots quotidiens
//! simulés, puis une compaction, donnent des résultats identiques avant et
//! après"), `docs/03-recherche.md` (exact dense search), `docs/02-architecture.md`
//! ("Suppressions, pierres tombales et compaction") and ADR-0006.
//!
//! # What the invariant means here
//!
//! The compaction renumbers the internal identifiers and rewrites the segments
//! (`crible_store::compact`). Nothing a caller can observe may change: the same
//! queries, over the same pool, must return the same public keys in the same
//! order, with the same scores within 1e-6. The tests therefore compare the
//! answers of the exact dense search, query by query, before and after the
//! compaction, and pin the premise that makes the comparison meaningful:
//!
//! - there are dead passages before the compaction, otherwise the compaction
//!   copies everything and proves nothing;
//! - there are none after it, so the tombstones really are what it removed;
//! - the live count and the pool size of every query are unchanged.
//!
//! # Why the `crible` crate
//!
//! This is the only test crate holding the exact search (`crible-search`), the
//! store and the compaction (`crible-store`) together, plus the CLI that
//! attaches a batch of an `onnx` index. The library path serves `synth-small`,
//! whose model is `external` and needs no ONNX Runtime; the CLI path serves
//! Legifacile, whose model is `mdenseon`, so the model resolution and the
//! acceptance check stay those of `crible attach`.
//!
//! # Datasets
//!
//! | Test | Data | Runs by default |
//! |---|---|---|
//! | `inv_09_synth_small_200_queries_return_the_same_results_after_compaction` | `data/synth-small/` | yes |
//! | `inv_09_the_legifacile_sample_test_fails_without_its_prerequisites` | none | yes |
//! | `inv_09_legifacile_sample_queries_return_the_same_results_after_compaction` | `data/legifacile-sample/` + `data/models/mdenseon/` + ONNX Runtime | no, `#[ignore]` |
//!
//! `synth-small` holds three upsert batches which really replace and delete
//! documents: `l2` replaces 300 documents and adds 100, `l3` deletes 150
//! documents and updates 50 metadata-only (`tools/gen_synth.py`). Those are the
//! "trois lots quotidiens simulés" of J3, and they leave tombstones to compact.
//!
//! The Legifacile sample holds one batch per structure and no replacement batch.
//! The test attaches every structure of the sample, the way `RECETTE-IMPORT`
//! does, and creates the tombstones the compaction needs with a batch of
//! deletions attached through the CLI: the production path of a removed document
//! (`docs/04`, ADR-0040 and ADR-0041). `attach` tombstones the passages and
//! deletes the `documents` row in the same transaction, so the state the test
//! compacts is one production really reaches. Nothing is written outside the
//! temporary directory.
//!
//! # Queries of the Legifacile sample
//!
//! `data/legifacile-sample/queries.jsonl` holds the query **question** and its
//! expected documents, no query vector. The questions are encoded here with
//! `QueryEncoder`, the encoder `crible search --query` uses, in fp32 (the default
//! of ADR-0052), so the exact search receives the vector a real query produces.
//! Ranking quality belongs to the eval, not to INV-09: only the stability of the
//! answers across the compaction is compared.
//!
//! The sample carries 300 questions spread over its six structures, and every
//! one of them is compared. The test attaches all six structures, so it takes a
//! long time and stays `#[ignore]`d; its duration and any bound on it belong to
//! the pending ADR on running it, not to this comment.
//!
//! ```text
//! cargo test -p crible --test inv_09_compaction_equivalence
//! ORT_DYLIB_PATH=/path/to/libonnxruntime.dylib \
//!   cargo test -p crible --test inv_09_compaction_equivalence -- --ignored
//! ```

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::time::Instant;

use arrow::array::{
    ArrayRef, BooleanArray, FixedSizeBinaryArray, Float64Array, Int64Array, StringArray,
    UInt32Array,
};
use arrow::datatypes::{DataType, Field, Schema as ArrowSchema, SchemaRef};
use arrow::record_batch::RecordBatch;
use crible_core::config::{FieldLevel, FieldType, IndexConfig, Schema};
use crible_core::filter::Filter;
use crible_core::ids::{Epoch, Generation};
use crible_core::model::ModelConfig;
use crible_encode::{
    DEFAULT_QUERY_CACHE_ENTRIES, DEFAULT_QUERY_PRECISION, ModelRegistry, QueryEncoder,
};
use crible_search::dense::exact::{ExactSearcher, Hit};
use crible_search::pool::Pool;
use crible_store::attach::attach;
use crible_store::generation::{GenerationStore, write_index_config};
use crible_store::records::{DocumentRecord, PassageRecord, ReadPool, RecordStore};
use crible_store::segment::SegmentReader;
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;
use serde_json::{Value, json};

/// Tolerance on a score, from the acceptance criterion of the task sheet:
/// "mêmes clés et mêmes scores à `1e-6` près".
const SCORE_TOLERANCE: f32 = 1e-6;

/// The three batches of `synth-small`, and the generation each one publishes.
const SYNTH_BATCHES: [(&str, u64); 3] = [("l1", 1), ("l2", 2), ("l3", 3)];

/// Live passages `synth-small` holds after its three batches: 10 000 created,
/// 300 documents replaced, 150 documents deleted, 100 + 50 new or updated
/// documents of five passages (`tools/gen_synth.py`). The count the J1 demo
/// asserts too.
const SYNTH_LIVE_PASSAGES: u64 = 9_750;

/// Number of results asked of a Legifacile query.
const LEGIFACILE_K: usize = 100;

/// The structures of the Legifacile sample, in the order of its batches: one
/// index each, and one batch `legifacile-sample-<structure>-v1`.
const LEGIFACILE_STRUCTURES: [&str; 6] =
    ["dole", "articles", "jorf", "titres", "sarde", "decisions"];

/// Command that generates the synthetic dataset.
const SYNTH_DATASET_COMMAND: &str =
    "cd tools && uv sync && uv run python gen_synth.py --out ../data/synth-small";

/// Command that builds the Legifacile sample and its model (gitignored).
const LEGIFACILE_SAMPLE_COMMAND: &str = "build data/legifacile-sample/ as \
     data/legifacile-sample/README.md, section \"Reproduire\", describes (production \
     read-only), then:\n  cd tools && uv sync --group export && uv run python export_onnx.py \
     --out ../data/models/mdenseon --overwrite";

/// External model configuration of `synth-small` (768 dimensions, `normalize`),
/// the one the dataset declares and the J1 demo writes.
const EXTERNAL_MODEL_JSON: &str =
    r#"{ "name": "synthetic-external", "kind": "external", "dims": 768, "normalize": true }"#;

// --- One query and one answer ---

/// One query of the invariant: a label for the failure messages, the number of
/// results asked for, the vector, and the filter of its pool.
struct Query {
    label: String,
    k: usize,
    vector: Vec<f32>,
    filter: Filter,
}

/// What one query answered: the size of its pool and the top-k results.
struct Outcome {
    pool_size: u32,
    hits: Vec<Hit>,
}

// --- Tiny JSON readers with readable failures ---

fn read_file(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()))
}

fn read_json_lines(path: &Path) -> Vec<Value> {
    read_file(path)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).unwrap_or_else(|error| {
                panic!(
                    "{} must hold one JSON value per line: {error}",
                    path.display()
                )
            })
        })
        .collect()
}

fn array(value: &Value, name: &str) -> Vec<f32> {
    value
        .get(name)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{value} must carry an array '{name}'"))
        .iter()
        .map(|item| {
            item.as_f64()
                .map(|number| number as f32)
                .unwrap_or_else(|| panic!("the '{name}' of {value} must hold numbers"))
        })
        .collect()
}

fn unsigned<T>(value: &Value, name: &str) -> T
where
    T: TryFrom<u64>,
{
    let raw = value
        .get(name)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("{value} must carry an unsigned '{name}'"));
    T::try_from(raw).unwrap_or_else(|_| panic!("{name} = {raw} does not fit the expected type"))
}

fn text(value: &Value, name: &str) -> String {
    value
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{value} must carry a string '{name}'"))
        .to_owned()
}

// --- Filters ---

/// The empty conjunction: every live passage satisfies it, so its pool is the
/// whole live set (`{}` in the filter grammar).
fn match_all(schema: &Schema) -> Filter {
    let filter = Filter::from_json_str("{}")
        .unwrap_or_else(|error| panic!("the match-all filter must parse: {error}"));
    filter
        .validate(schema)
        .unwrap_or_else(|error| panic!("the match-all filter must be valid: {error}"));
    filter
}

/// Parses and validates the filter of one query, `{}` when it carries none.
fn query_filter(value: &Value, schema: &Schema) -> Filter {
    let raw = match value.get("filter") {
        Some(filter) if filter.is_object() => filter.to_string(),
        _ => "{}".to_owned(),
    };
    let filter = Filter::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("the filter {raw} must parse: {error}"));
    filter
        .validate(schema)
        .unwrap_or_else(|error| panic!("the filter {raw} must be valid: {error}"));
    filter
}

// --- Running the exact search ---

/// Runs every query over the published generation, opening the searcher once.
///
/// The pool of a query is evaluated on the filter structures of the snapshot,
/// as `RECHERCHER-DENSE-EXACT` does: the compaction must not change the pool
/// either, so its size is returned and compared too.
fn search_all(store: &GenerationStore, queries: &[Query]) -> Vec<Outcome> {
    let snapshot = store.snapshot();
    let searcher = ExactSearcher::open(
        &store.segments_directory(),
        snapshot.segments(),
        &store.records_path(),
    )
    .unwrap_or_else(|error| {
        panic!(
            "the exact searcher must open on epoch {}: {error}",
            snapshot.epoch().get()
        )
    });

    queries
        .iter()
        .map(|query| {
            let pool = Pool::evaluate(snapshot.filters(), &query.filter).unwrap_or_else(|error| {
                panic!("the pool of query {} must evaluate: {error}", query.label)
            });
            let pool_size = pool.len();
            let hits = searcher
                .top_k(&query.vector, &pool, query.k)
                .unwrap_or_else(|error| {
                    panic!("the search of query {} must succeed: {error}", query.label)
                });
            Outcome { pool_size, hits }
        })
        .collect()
}

// --- The comparison of INV-09 ---

/// Compares two answers of the same query, position by position.
///
/// The keys must be equal and in the same order, the scores must be within
/// [`SCORE_TOLERANCE`] and finite, and the answer must hold the same number of
/// results. An error names the position and both values, so a failure says what
/// moved.
fn compare_results(before: &[Hit], after: &[Hit]) -> Result<(), String> {
    if before.len() != after.len() {
        return Err(format!(
            "{} result(s) after the compaction for {} before it",
            after.len(),
            before.len()
        ));
    }
    for (position, (left, right)) in before.iter().zip(after).enumerate() {
        if left.key != right.key {
            return Err(format!(
                "position {position}: key {} after the compaction, {} before it",
                right.key, left.key
            ));
        }
        let gap = (left.score - right.score).abs();
        if !gap.is_finite() || gap > SCORE_TOLERANCE {
            return Err(format!(
                "position {position}: key {} scored {:.9} after the compaction and {:.9} \
                 before it, a gap of {gap:.9} that is not within the {SCORE_TOLERANCE} \
                 tolerance",
                left.key, right.score, left.score
            ));
        }
    }
    Ok(())
}

/// Asserts that one query answered the same thing before and after the
/// compaction, and that its pool did not move.
fn assert_same_results(label: &str, before: &Outcome, after: &Outcome) {
    assert_eq!(
        before.pool_size, after.pool_size,
        "query {label}: the pool must hold the same live passages after the compaction \
         ({} before, {} after)",
        before.pool_size, after.pool_size
    );
    compare_results(&before.hits, &after.hits)
        .unwrap_or_else(|message| panic!("query {label}: {message}"));
}

// --- The synthetic dataset ---

fn synth_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/synth-small")
}

/// Fails with the generator command when a file of the dataset is missing, as
/// every test of the crate does.
fn require_synth(root: &Path) {
    for name in ["index.json", "queries.jsonl"] {
        let path = root.join(name);
        assert!(
            path.is_file(),
            "{} is missing; generate the dataset before running `cargo test`:\n  \
             {SYNTH_DATASET_COMMAND}",
            path.display()
        );
    }
    for (batch, _) in SYNTH_BATCHES {
        let path = root.join(batch).join("manifest.json");
        assert!(
            path.is_file(),
            "the synthetic batch '{batch}' is missing at {}; generate the dataset:\n  \
             {SYNTH_DATASET_COMMAND}",
            path.display()
        );
    }
}

/// The 200 queries of `queries.jsonl`, in file order, with their own filters.
fn synth_queries(root: &Path, index: &IndexConfig) -> Vec<Query> {
    let rows = read_json_lines(&root.join("queries.jsonl"));
    assert_eq!(rows.len(), 200, "queries.jsonl must hold the 200 queries");
    rows.iter()
        .map(|row| Query {
            label: format!("synth#{}", unsigned::<u64>(row, "query_id")),
            k: unsigned::<usize>(row, "k"),
            vector: array(row, "vector"),
            filter: query_filter(row, &index.schema),
        })
        .collect()
}

/// INV-09 on `synth-small`: the 200 queries return the same keys, in the same
/// order, with the same scores within 1e-6, after a compaction that removes the
/// tombstones of the three simulated daily batches.
///
/// Acceptance criterion of `VERIFIER-EQUIVALENCE-COMPACTION`.
#[test]
fn inv_09_synth_small_200_queries_return_the_same_results_after_compaction() {
    let root = synth_root();
    require_synth(&root);
    let raw = read_file(&root.join("index.json"));
    let index = IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("the dataset index.json must be valid: {error}"));
    let model = ModelConfig::from_json_str(EXTERNAL_MODEL_JSON)
        .unwrap_or_else(|error| panic!("the dataset model configuration is valid: {error}"));

    let temp = tempfile::tempdir().unwrap_or_else(|error| {
        panic!("a temporary directory for the index must be creatable: {error}")
    });
    let index_dir = temp.path().join("indexes").join("synth-small");
    write_index_config(&index_dir, &raw)
        .unwrap_or_else(|error| panic!("the index configuration must be written: {error}"));
    let mut store = GenerationStore::create(&index_dir, Epoch::new(1), &index.schema)
        .unwrap_or_else(|error| panic!("the index must be created: {error}"));

    for (batch, generation) in SYNTH_BATCHES {
        let path = root.join(batch);
        attach(&mut store, &path, &index, &model, None, None).unwrap_or_else(|error| {
            panic!(
                "the attach of {batch} must succeed, got {}: {}",
                error.code().as_str(),
                error.message()
            )
        });
        assert_eq!(
            store.generation(),
            Generation::new(generation),
            "{batch} must publish generation {generation}"
        );
    }

    // Premise: the three batches really leave tombstones, and the live set is
    // the one `tools/gen_synth.py` describes.
    let before = store
        .state()
        .unwrap_or_else(|error| panic!("the state of the index must be readable: {error}"));
    assert_eq!(
        before.live_passages, SYNTH_LIVE_PASSAGES,
        "the three batches must leave {SYNTH_LIVE_PASSAGES} live passages"
    );
    assert!(
        before.dead_passages > 0,
        "the three batches must leave tombstones, otherwise the compaction proves \
         nothing: {before:?}"
    );

    let queries = synth_queries(&root, &index);
    assert_eq!(queries.len(), 200);

    let answers_before = search_all(&store, &queries);
    for (query, answer) in queries.iter().zip(&answers_before) {
        assert!(
            answer.pool_size > 0,
            "query {} must have a non-empty pool, otherwise it proves nothing",
            query.label
        );
        assert_eq!(
            answer.hits.len(),
            usize::try_from(answer.pool_size)
                .unwrap_or(usize::MAX)
                .min(query.k),
            "query {} must return min(k, pool_size) results",
            query.label
        );
    }

    let report = crible_store::compact::compact(&mut store, &index)
        .unwrap_or_else(|error| panic!("the compaction must succeed: {error}"));
    assert_eq!(
        report.removed_passages, before.dead_passages,
        "the compaction must remove exactly the tombstones of the three batches"
    );
    assert_eq!(report.live_passages, before.live_passages);

    let after = store.state().unwrap_or_else(|error| {
        panic!("the state of the compacted index must be readable: {error}")
    });
    assert_eq!(
        after.dead_passages, 0,
        "no dead passage may remain after the compaction"
    );
    assert_eq!(after.live_passages, before.live_passages);
    assert_eq!(after.generation, Generation::new(4));
    assert_eq!(after.epoch, Epoch::new(2));

    let answers_after = search_all(&store, &queries);
    assert_eq!(answers_before.len(), answers_after.len());
    for ((query, before), after) in queries.iter().zip(&answers_before).zip(&answers_after) {
        assert_same_results(&query.label, before, after);
    }
    eprintln!(
        "INV-09: 200 queries of synth-small compared, {} tombstone(s) removed, \
         {} live passage(s) kept",
        report.removed_passages, report.live_passages
    );
}

// --- The Legifacile sample ---

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Paths of the Legifacile sample the ignored test reads.
#[derive(Debug)]
struct SamplePaths {
    /// Directory of the sample, holding `queries.jsonl` and `configs/`.
    directory: PathBuf,
    /// Root of the models, holding `<model>/model.json`.
    models: PathBuf,
}

impl SamplePaths {
    /// Configuration of one structure: `configs/<structure>.json`.
    fn config(&self, structure: &str) -> PathBuf {
        self.directory.join(format!("configs/{structure}.json"))
    }

    /// Batch of one structure: `batches/legifacile-sample-<structure>-v1`.
    fn batch(&self, structure: &str) -> PathBuf {
        self.directory
            .join(format!("batches/legifacile-sample-{structure}-v1"))
    }
}

/// Returns the paths of the sample, or the message naming the missing file and
/// the command that produces it (`docs/08`, "Sauts de test"; ADR-0057).
fn legifacile_sample_paths(root: &Path) -> Result<SamplePaths, String> {
    let sample = root.join("data/legifacile-sample");
    let paths = SamplePaths {
        directory: sample,
        models: root.join("data/models"),
    };
    let model_json = paths.models.join("mdenseon/model.json");
    let mut required = Vec::new();
    for structure in LEGIFACILE_STRUCTURES {
        required.push(paths.config(structure));
        required.push(paths.batch(structure).join("manifest.json"));
    }
    required.push(paths.directory.join("queries.jsonl"));
    required.push(model_json);
    for required in &required {
        if !required.is_file() {
            return Err(format!(
                "{} is missing, so \
                 inv_09_legifacile_sample_queries_return_the_same_results_after_compaction \
                 cannot run; build the gitignored data with:\n  {LEGIFACILE_SAMPLE_COMMAND}",
                required.display()
            ));
        }
    }
    Ok(paths)
}

/// An `#[ignore]` test launched explicitly without its prerequisite fails,
/// instead of passing while proving nothing (ADR-0057).
#[test]
fn inv_09_the_legifacile_sample_test_fails_without_its_prerequisites() {
    let root = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));

    let error = legifacile_sample_paths(root.path())
        .expect_err("an empty root holds no Legifacile sample, so the check must fail");

    assert!(
        error.contains("data/legifacile-sample/configs/dole.json"),
        "the message must name the missing file: {error}"
    );
    assert!(
        error.contains("data/legifacile-sample/README.md"),
        "the message must say where the sample is produced: {error}"
    );
    assert!(
        error.contains("export_onnx.py"),
        "the message must give the command that produces the model: {error}"
    );
    assert!(
        error.contains("inv_09_legifacile_sample_queries_return_the_same_results_after_compaction"),
        "the message must name the test it blocks: {error}"
    );
}

fn crible(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_crible"))
        .args(arguments)
        .output()
        .unwrap_or_else(|error| panic!("cannot run crible {arguments:?}: {error}"))
}

fn crible_ok(arguments: &[&str]) {
    let output = crible(arguments);
    assert!(
        output.status.success(),
        "crible {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn path(path: &Path) -> &str {
    path.to_str()
        .unwrap_or_else(|| panic!("the path {} must be UTF-8", path.display()))
}

/// The query encoder of the model of the sample, in fp32.
///
/// This is the path of `crible search --query`: the same `QueryEncoder`, the same
/// `DEFAULT_QUERY_PRECISION` (fp32, ADR-0052) and the same `query: ` prompt. One
/// encoder serves every structure, since they share the model.
fn legifacile_encoder(sample: &SamplePaths, index: &IndexConfig) -> QueryEncoder {
    let directory = sample.models.join(&index.model);
    QueryEncoder::new(
        &directory,
        Arc::new(ModelRegistry::new()),
        DEFAULT_QUERY_CACHE_ENTRIES,
    )
}

/// The queries of one structure, encoded from their question.
fn legifacile_queries(
    sample: &SamplePaths,
    structure: &str,
    index: &IndexConfig,
    encoder: &QueryEncoder,
) -> Vec<Query> {
    let rows = read_json_lines(&sample.directory.join("queries.jsonl"));
    let filter = match_all(&index.schema);
    let mut queries = Vec::new();

    for row in &rows {
        if row.get("structure").and_then(Value::as_str) != Some(structure) {
            continue;
        }
        let label = text(row, "id");
        let question = text(row, "question");
        let encoded = encoder
            .encode_query(&question, DEFAULT_QUERY_PRECISION)
            .unwrap_or_else(|error| {
                panic!(
                    "query {label} must be encodable, got {}: {}",
                    error.code().as_str(),
                    error.message()
                )
            });
        assert!(
            !encoded.vector.is_empty(),
            "query {label} must encode to a non-empty vector"
        );
        queries.push(Query {
            label: format!("legifacile#{label}"),
            k: LEGIFACILE_K,
            vector: encoded.vector,
            filter: filter.clone(),
        });
    }

    assert!(
        !queries.is_empty(),
        "the sample must hold the queries of the '{structure}' structure"
    );
    queries
}

/// Stable rank of a document identifier: the same identifier always falls on
/// the same side, so the set of removed documents is deterministic.
fn stable_rank(doc_id: &str) -> u64 {
    doc_id
        .bytes()
        .fold(0_u64, |sum, byte| sum + u64::from(byte))
}

/// Identifiers of every document holding a live passage, in ascending order.
fn live_documents(store: &GenerationStore) -> Vec<String> {
    let pool = ReadPool::new(&store.records_path(), 2)
        .unwrap_or_else(|error| panic!("the read pool must open: {error}"));
    let mut documents = BTreeSet::new();
    for iid in store.snapshot().live_iids() {
        let key = pool
            .read_passage_key(iid)
            .unwrap_or_else(|error| panic!("the key of iid {} must read: {error}", iid.get()))
            .unwrap_or_else(|| panic!("iid {} is live but has no passage row", iid.get()));
        documents.insert(key.doc_id().as_str().to_owned());
    }
    documents.into_iter().collect()
}

/// The documents to remove: one in ten, chosen by [`stable_rank`] so that the
/// fixture is deterministic.
fn documents_to_remove(documents: &[String]) -> Vec<String> {
    let removed: Vec<String> = documents
        .iter()
        .filter(|doc_id| stable_rank(doc_id).is_multiple_of(10))
        .cloned()
        .collect();
    assert!(
        !removed.is_empty(),
        "the fixture must find documents to remove"
    );
    removed
}

/// One live document copied from the index, with the reference vector of its
/// first passage.
///
/// The batch format requires at least one `documents.parquet` and one
/// `passages.parquet` (`docs/04`, ADR-0040), so a batch that removes documents
/// still carries one document. Copying a document that stays in the index keeps
/// the attach from writing anything for it: the same `doc_hash` and the same
/// fields make it `unchanged`, and a `deletion-only` batch does not exist.
struct CopiedDocument {
    document: DocumentRecord,
    passage: PassageRecord,
    vector: Vec<f32>,
}

fn copy_live_document(
    store: &GenerationStore,
    documents: &[String],
    removed: &BTreeSet<String>,
) -> CopiedDocument {
    let records = RecordStore::open(&store.records_path())
        .unwrap_or_else(|error| panic!("the records of the epoch must open: {error}"));
    for doc_id in documents {
        if removed.contains(doc_id) {
            continue;
        }
        let Some(document) = records
            .read_document(doc_id)
            .unwrap_or_else(|error| panic!("the document {doc_id} must be readable: {error}"))
        else {
            continue;
        };
        let passages = records
            .passages_of_document(doc_id)
            .unwrap_or_else(|error| panic!("the passages of {doc_id} must be readable: {error}"));
        let Some(passage) = passages
            .into_iter()
            .find(|passage| passage.ord == 0 && passage.dead_since_gen.is_none())
        else {
            continue;
        };
        let directory = store.segments_directory().join(&passage.segment_id);
        let reader = SegmentReader::open(&directory).unwrap_or_else(|error| {
            panic!(
                "the segment {} must open: {error}",
                passage.segment_id.as_str()
            )
        });
        let vector = reader.vector(passage.iid).unwrap_or_else(|error| {
            panic!("the vector of {doc_id}#{} must read: {error}", passage.ord)
        });
        return CopiedDocument {
            document,
            passage,
            vector,
        };
    }
    panic!("the fixture must find a document to copy into the deletion batch");
}

/// Writes a batch that removes `deletions` and carries one unchanged `kept`
/// document, then returns the directory of the batch.
///
/// This is the production path of a removal: the manifest is the one of
/// `template`, with the identifiers and the counts of this batch, and `attach`
/// applies it (`docs/04`, ADR-0040). The attach deletes the `documents` row of
/// every removed document along with its passages, in one transaction
/// (ADR-0041, decision 1).
fn write_deletion_batch(
    template: &Path,
    batches: &Path,
    batch_id: &str,
    index: &IndexConfig,
    kept: &CopiedDocument,
    deletions: &[String],
) -> PathBuf {
    let directory = batches.join(batch_id);
    fs::create_dir_all(&directory)
        .unwrap_or_else(|error| panic!("{} must be creatable: {error}", directory.display()));

    let mut document_fields = vec![
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("doc_hash", DataType::Utf8, false),
    ];
    let mut document_columns: Vec<ArrayRef> = vec![
        Arc::new(single_string(&kept.document.doc_id)),
        Arc::new(single_string(&kept.document.doc_hash)),
    ];
    for (name, level, spec) in index.schema.fields() {
        if level != FieldLevel::Document {
            continue;
        }
        document_fields.push(Field::new(name, arrow_type(spec.field_type, name), true));
        document_columns.push(field_column(
            spec.field_type,
            kept.document.fields.get(name).unwrap_or(&Value::Null),
            name,
        ));
    }

    let bytes: Vec<u8> = kept
        .vector
        .iter()
        .flat_map(|value| half::f16::from_f32(*value).to_bits().to_le_bytes())
        .collect();
    let width = i32::try_from(bytes.len())
        .unwrap_or_else(|error| panic!("the vector width must fit an i32: {error}"));
    let mut passage_fields = vec![
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("ord", DataType::UInt32, false),
        Field::new("text", DataType::Utf8, false),
        Field::new("vector", DataType::FixedSizeBinary(width), false),
    ];
    let vector =
        FixedSizeBinaryArray::try_from_sparse_iter_with_size(std::iter::once(Some(bytes)), width)
            .unwrap_or_else(|error| panic!("the fixture vector has one width: {error}"));
    let mut passage_columns: Vec<ArrayRef> = vec![
        Arc::new(single_string(&kept.document.doc_id)),
        Arc::new(UInt32Array::from(vec![kept.passage.ord])),
        Arc::new(single_string(&kept.passage.text)),
        Arc::new(vector),
    ];
    for (name, level, spec) in index.schema.fields() {
        if level != FieldLevel::Passage {
            continue;
        }
        passage_fields.push(Field::new(name, arrow_type(spec.field_type, name), true));
        passage_columns.push(field_column(
            spec.field_type,
            kept.passage.fields.get(name).unwrap_or(&Value::Null),
            name,
        ));
    }

    let documents = directory.join("documents.parquet");
    write_parquet(
        &documents,
        &Arc::new(ArrowSchema::new(document_fields)),
        document_columns,
    );
    let passages = directory.join("passages.parquet");
    write_parquet(
        &passages,
        &Arc::new(ArrowSchema::new(passage_fields)),
        passage_columns,
    );
    let deletions_path = directory.join("deletions.parquet");
    write_parquet(
        &deletions_path,
        &Arc::new(ArrowSchema::new(vec![Field::new(
            "doc_id",
            DataType::Utf8,
            false,
        )])),
        vec![Arc::new(
            deletions
                .iter()
                .map(|doc_id| Some(doc_id.as_str()))
                .collect::<StringArray>(),
        )],
    );

    let mut files = serde_json::Map::new();
    for path in [&documents, &passages, &deletions_path] {
        let bytes = fs::read(path)
            .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_else(|| panic!("{} must have a UTF-8 name", path.display()));
        files.insert(
            name.to_owned(),
            Value::from(format!("blake3:{}", blake3::hash(&bytes).to_hex())),
        );
    }

    let mut manifest: Value = serde_json::from_str(&read_file(template))
        .unwrap_or_else(|error| panic!("{} must be valid JSON: {error}", template.display()));
    manifest["batch_id"] = Value::from(batch_id);
    manifest["mode"] = Value::from("upsert");
    manifest["counts"] = json!({
        "documents": 1,
        "passages": 1,
        "deletions": deletions.len(),
    });
    manifest["files"] = Value::Object(files);
    let manifest = serde_json::to_string_pretty(&manifest)
        .unwrap_or_else(|error| panic!("the manifest must serialize: {error}"));
    fs::write(directory.join("manifest.json"), manifest)
        .unwrap_or_else(|error| panic!("manifest.json must be written: {error}"));
    directory
}

/// One string array holding a single value.
fn single_string(value: &str) -> StringArray {
    std::iter::once(Some(value)).collect()
}

/// Arrow data type of one schema field, as the batch format writes it.
fn arrow_type(field_type: FieldType, name: &str) -> DataType {
    match field_type {
        FieldType::Keyword => DataType::Utf8,
        FieldType::Integer => DataType::Int64,
        FieldType::Float => DataType::Float64,
        FieldType::Bool => DataType::Boolean,
        FieldType::Date => {
            panic!("the fixture does not build the date field '{name}'; its Arrow form is Date32")
        }
    }
}

/// One single-row column holding a field value read from the index.
fn field_column(field_type: FieldType, value: &Value, name: &str) -> ArrayRef {
    if value.is_null() {
        return match field_type {
            FieldType::Keyword => Arc::new(StringArray::from(vec![None::<&str>])),
            FieldType::Integer => Arc::new(Int64Array::from(vec![None::<i64>])),
            FieldType::Float => Arc::new(Float64Array::from(vec![None::<f64>])),
            FieldType::Bool => Arc::new(BooleanArray::from(vec![None::<bool>])),
            FieldType::Date => panic!(
                "the fixture does not build the date field '{name}'; its Arrow form is Date32"
            ),
        };
    }
    match field_type {
        FieldType::Keyword => {
            let value = value
                .as_str()
                .unwrap_or_else(|| panic!("the field '{name}' must be a string, got {value}"));
            Arc::new(single_string(value))
        }
        FieldType::Integer => {
            let value = value
                .as_i64()
                .unwrap_or_else(|| panic!("the field '{name}' must be an integer, got {value}"));
            Arc::new(Int64Array::from(vec![value]))
        }
        FieldType::Float => {
            let value = value
                .as_f64()
                .unwrap_or_else(|| panic!("the field '{name}' must be a number, got {value}"));
            Arc::new(Float64Array::from(vec![value]))
        }
        FieldType::Bool => {
            let value = value
                .as_bool()
                .unwrap_or_else(|| panic!("the field '{name}' must be a boolean, got {value}"));
            Arc::new(BooleanArray::from(vec![value]))
        }
        FieldType::Date => {
            panic!("the fixture does not build the date field '{name}'; its Arrow form is Date32")
        }
    }
}

fn write_parquet(path: &Path, schema: &SchemaRef, columns: Vec<ArrayRef>) {
    let batch = RecordBatch::try_new(schema.clone(), columns)
        .unwrap_or_else(|error| panic!("the fixture batch is valid: {error}"));
    let file = fs::File::create(path)
        .unwrap_or_else(|error| panic!("the Parquet file is created: {error}"));
    let properties = WriterProperties::builder().build();
    let mut writer = ArrowWriter::try_new(file, schema.clone(), Some(properties))
        .unwrap_or_else(|error| panic!("the Parquet writer opens: {error}"));
    writer
        .write(&batch)
        .unwrap_or_else(|error| panic!("the batch is written: {error}"));
    writer
        .close()
        .unwrap_or_else(|error| panic!("the Parquet file closes: {error}"));
}

/// INV-09 on the Legifacile sample: the queries of every structure return the
/// same keys, in the same order, with the same scores within 1e-6, after a
/// compaction that removes the tombstones.
///
/// The sample holds one batch per structure and no replacement batch, so the
/// tombstones come from a batch of deletions attached the way production removes
/// a document. The query vectors are encoded from the questions of the sample
/// with the query encoder of the model, in fp32.
#[test]
#[ignore = "needs data/legifacile-sample, data/models/mdenseon and ORT_DYLIB_PATH; run: ORT_DYLIB_PATH=/path/to/libonnxruntime.dylib cargo test -p crible --test inv_09_compaction_equivalence -- --ignored"]
fn inv_09_legifacile_sample_queries_return_the_same_results_after_compaction() {
    let root = repository_root();
    let sample = legifacile_sample_paths(&root).unwrap_or_else(|reason| panic!("{reason}"));
    assert!(
        std::env::var_os("ORT_DYLIB_PATH").is_some(),
        "ORT_DYLIB_PATH must name the ONNX Runtime library: the attach of an 'onnx' index \
         re-encodes a sample (control 9 of docs/04) and the questions are encoded with the model"
    );

    let temp = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));

    // One encoder for the whole sample: every structure uses the same model.
    let first_config = sample.config(LEGIFACILE_STRUCTURES[0]);
    let first_raw = read_file(&first_config);
    let first_index = IndexConfig::from_json_str(&first_raw)
        .unwrap_or_else(|error| panic!("{} must be valid: {error}", first_config.display()));
    let encoder = legifacile_encoder(&sample, &first_index);

    let started = Instant::now();
    let mut compared = 0_usize;
    for structure in LEGIFACILE_STRUCTURES {
        compared += check_structure(&sample, temp.path(), structure, &encoder);
    }
    assert_eq!(
        compared, 300,
        "the sample must hold the 300 queries of queries.jsonl, got {compared}"
    );
    eprintln!(
        "INV-09: {compared} queries of the Legifacile sample compared over {} structure(s) in \
         {} s",
        LEGIFACILE_STRUCTURES.len(),
        started.elapsed().as_secs()
    );
}

/// Runs INV-09 on one structure of the sample, and returns the number of queries
/// compared.
fn check_structure(
    sample: &SamplePaths,
    temp: &Path,
    structure: &str,
    encoder: &QueryEncoder,
) -> usize {
    let config = sample.config(structure);
    let batch = sample.batch(structure);
    let data = temp.join("data");
    crible_ok(&[
        "index",
        "create",
        "--data",
        path(&data),
        "--config",
        path(&config),
    ]);
    crible_ok(&[
        "attach",
        "--data",
        path(&data),
        "--models",
        path(&sample.models),
        "--index",
        structure,
        path(&batch),
    ]);

    let raw = read_file(&config);
    let index = IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("{} must be valid: {error}", config.display()));
    let index_dir = data.join("indexes").join(structure);
    let store = GenerationStore::open(&index_dir, &index.schema)
        .unwrap_or_else(|error| panic!("the attached index {structure} must open: {error}"));

    let queries = legifacile_queries(sample, structure, &index, encoder);

    let attached = store
        .state()
        .unwrap_or_else(|error| panic!("the state of {structure} must be readable: {error}"));
    assert_eq!(
        attached.dead_passages, 0,
        "structure {structure} holds no tombstone before the simulated deletion"
    );

    // The simulated deletion goes through `attach`, the only production writer:
    // a batch that declares the removals in `deletions` deletes the `documents`
    // row along with the passages in one transaction (ADR-0041, decision 1), so
    // the state is one production reaches.
    let documents = live_documents(&store);
    let victims = documents_to_remove(&documents);
    let victim_set: BTreeSet<String> = victims.iter().cloned().collect();
    let victims_count = u64::try_from(victims.len())
        .unwrap_or_else(|error| panic!("the number of removed documents fits a u64: {error}"));
    assert!(
        victims_count < attached.documents,
        "the fixture must keep documents, got {victims_count} removed of {} in {structure}",
        attached.documents
    );
    let kept = copy_live_document(&store, &documents, &victim_set);
    let deletion_batch = write_deletion_batch(
        &batch.join("manifest.json"),
        &temp.join("batches"),
        &format!("legifacile-sample-{structure}-deletions-v1"),
        &index,
        &kept,
        &victims,
    );
    drop(store);
    crible_ok(&[
        "attach",
        "--data",
        path(&data),
        "--models",
        path(&sample.models),
        "--index",
        structure,
        path(&deletion_batch),
    ]);

    let mut store = GenerationStore::open(&index_dir, &index.schema)
        .unwrap_or_else(|error| panic!("the attached index {structure} must open again: {error}"));
    let deleted = store.state().unwrap_or_else(|error| {
        panic!("the state of {structure} after the deletion must be readable: {error}")
    });
    assert_eq!(
        deleted.documents,
        attached.documents - victims_count,
        "the attach of a batch of deletions must delete the documents rows of {structure} \
         (ADR-0041)"
    );
    let records = RecordStore::open(&store.records_path())
        .unwrap_or_else(|error| panic!("the records of the epoch must open: {error}"));
    for doc_id in &victims {
        assert!(
            records
                .read_document(doc_id)
                .unwrap_or_else(|error| panic!("the document {doc_id} must be readable: {error}"))
                .is_none(),
            "the document {doc_id} of {structure} has no live passage left but keeps a \
             documents row, a state production cannot reach"
        );
    }
    drop(records);

    let before = store
        .state()
        .unwrap_or_else(|error| panic!("the state of {structure} must be readable: {error}"));
    assert!(
        before.dead_passages > 0,
        "the simulated deletion of {structure} must leave tombstones, otherwise the \
         compaction proves nothing"
    );
    assert!(
        before.dead_ratio() > 0.01,
        "the fixture of {structure} must hold a noticeable share of tombstones, got {}",
        before.dead_ratio()
    );

    let answers_before = search_all(&store, &queries);
    for (query, answer) in queries.iter().zip(&answers_before) {
        assert!(
            answer.pool_size > 0,
            "query {} must have a non-empty pool",
            query.label
        );
        assert_eq!(
            answer.hits.len(),
            usize::try_from(answer.pool_size)
                .unwrap_or(usize::MAX)
                .min(query.k),
            "query {} must return min(k, pool_size) results",
            query.label
        );
    }

    let report = crible_store::compact::compact(&mut store, &index)
        .unwrap_or_else(|error| panic!("the compaction of {structure} must succeed: {error}"));
    assert_eq!(report.removed_passages, before.dead_passages);
    assert_eq!(report.live_passages, before.live_passages);

    let after = store.state().unwrap_or_else(|error| {
        panic!("the state of the compacted {structure} must be readable: {error}")
    });
    assert_eq!(
        after.dead_passages, 0,
        "no dead passage may remain in {structure} after the compaction"
    );
    assert_eq!(after.live_passages, before.live_passages);
    assert_eq!(
        after.documents, before.documents,
        "the compaction must keep one documents row per live document in {structure}"
    );
    assert_eq!(
        after.generation,
        Generation::new(before.generation.get() + 1),
        "the compaction publishes G+1"
    );
    assert_eq!(after.epoch.get(), before.epoch.get() + 1);

    let answers_after = search_all(&store, &queries);
    for ((query, before), after) in queries.iter().zip(&answers_before).zip(&answers_after) {
        assert_same_results(&query.label, before, after);
    }
    eprintln!(
        "INV-09: {structure}: {} queries compared, {} document(s) and {} tombstone(s) removed, \
         {} live passage(s) kept",
        queries.len(),
        victims_count,
        report.removed_passages,
        report.live_passages
    );
    queries.len()
}

// --- The comparison cannot pass by accident ---

/// Unit tests of [`compare_results`], the only place that knows the tolerance.
/// They feed it answers a compaction must never produce and check that it
/// rejects them.
mod comparison_guards {
    use serde_json::json;

    use crible_search::dense::exact::Hit;

    use crate::compare_results;

    fn hit(key: &str, score: f32) -> Hit {
        let (doc_id, ord) = key.split_once('#').unwrap_or((key, "0"));
        Hit {
            doc_id: doc_id.to_owned(),
            ord: ord.parse().unwrap_or(0),
            key: key.to_owned(),
            score,
            fields: json!({}),
            text: String::new(),
        }
    }

    #[test]
    fn identical_answers_are_accepted() {
        let before = [hit("a#0", 0.9), hit("b#0", 0.5)];
        let after = [hit("a#0", 0.9), hit("b#0", 0.5)];
        assert!(
            compare_results(&before, &after).is_ok(),
            "identical answers must be accepted"
        );
    }

    #[test]
    fn a_moved_score_within_the_tolerance_is_accepted() {
        let before = [hit("a#0", 0.5)];
        let after = [hit("a#0", 0.5 + 5e-7)];
        assert!(
            compare_results(&before, &after).is_ok(),
            "a score gap under 1e-6 must be accepted"
        );
    }

    #[test]
    fn a_moved_score_above_the_tolerance_is_rejected() {
        let before = [hit("a#0", 0.5)];
        let after = [hit("a#0", 0.5 + 2e-6)];
        assert!(
            compare_results(&before, &after).is_err(),
            "a score gap above 1e-6 must be rejected"
        );
    }

    #[test]
    fn a_non_finite_score_is_rejected() {
        let before = [hit("a#0", f32::NAN)];
        let after = [hit("a#0", f32::NAN)];
        assert!(
            compare_results(&before, &after).is_err(),
            "two NaN scores are not a stable answer and must be rejected"
        );
        let before = [hit("a#0", 0.5)];
        let after = [hit("a#0", f32::INFINITY)];
        assert!(
            compare_results(&before, &after).is_err(),
            "an infinite score must be rejected"
        );
    }

    #[test]
    fn a_different_key_is_rejected() {
        let before = [hit("a#0", 0.9), hit("b#0", 0.5)];
        let after = [hit("a#0", 0.9), hit("c#0", 0.5)];
        assert!(
            compare_results(&before, &after).is_err(),
            "a different key at the same position must be rejected"
        );
    }

    #[test]
    fn a_reordered_pair_is_rejected() {
        let before = [hit("a#0", 0.9), hit("b#0", 0.5)];
        let after = [hit("b#0", 0.5), hit("a#0", 0.9)];
        assert!(
            compare_results(&before, &after).is_err(),
            "an exchange of two results must be rejected, even with the same scores"
        );
    }

    #[test]
    fn a_shorter_answer_is_rejected() {
        let before = [hit("a#0", 0.9), hit("b#0", 0.5)];
        let after = [hit("a#0", 0.9)];
        assert!(
            compare_results(&before, &after).is_err(),
            "an answer that lost a result must be rejected"
        );
    }
}
