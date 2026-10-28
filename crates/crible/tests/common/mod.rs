// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! Fixtures shared by the test binaries of the `crible` crate.
//!
//! The batch-control tests (`INV-11`, `INV-12`) drive `crible attach` on the
//! versioned fake encoder of `crates/crible-encode/tests/fixtures/fake-model`,
//! and both need the same expected vectors: the graph returns
//! `[mean of the input ids, 1, 1, …]` on every row, CLS pooling keeps that row,
//! and the engine normalizes it. The vocabulary is read from the versioned
//! `tokenizer.json`, never a copy of it.
//!
//! The lifecycle-equivalence tests (`INV-08`, `INV-09`) share the rest of the
//! module: the query and answer types, the small JSON readers, the exact-search
//! runner of one generation, the comparison of two answers, the `synth-small`
//! and Legifacile sample fixtures, and the Parquet writer of a batch. One copy
//! serves both files, so a comparison rule or an error message cannot drift
//! between them.
//!
//! Every test binary includes this module with `mod common`, and no binary uses
//! every fixture. Marking each unused item separately would only add noise to
//! the test build, so the module allows dead code once, here; it never hides a
//! lint in the code under test.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use crible_core::config::{IndexConfig, Schema};
use crible_core::filter::Filter;
use crible_encode::{
    DEFAULT_QUERY_CACHE_ENTRIES, DEFAULT_QUERY_PRECISION, ModelRegistry, QueryEncoder,
};
use crible_search::dense::exact::{DenseSearcher, Hit};
use crible_search::pool::Pool;
use crible_store::generation::GenerationStore;
use serde_json::Value;

/// Dimensions of the fake encoder (`model.json` of the fixture).
pub const DIMS: usize = 768;

/// Writes one Parquet file holding `columns`, with the fixture writer settings.
///
/// Every test crate that builds a batch shares this writer, so the files a test
/// reads are the ones another test writes.
pub fn write_parquet(
    path: &Path,
    schema: &arrow::datatypes::SchemaRef,
    columns: Vec<arrow::array::ArrayRef>,
) {
    use parquet::arrow::ArrowWriter;
    use parquet::file::properties::WriterProperties;

    let batch = arrow::record_batch::RecordBatch::try_new(schema.clone(), columns)
        .unwrap_or_else(|error| panic!("the fixture batch must be valid: {error}"));
    let file = fs::File::create(path)
        .unwrap_or_else(|error| panic!("{} must be creatable: {error}", path.display()));
    let properties = WriterProperties::builder().build();
    let mut writer = ArrowWriter::try_new(file, schema.clone(), Some(properties))
        .unwrap_or_else(|error| panic!("the fixture Parquet writer must open: {error}"));
    writer
        .write(&batch)
        .unwrap_or_else(|error| panic!("the fixture batch must be written: {error}"));
    writer
        .close()
        .unwrap_or_else(|error| panic!("the fixture Parquet file must close: {error}"));
}

/// Directory of the versioned fake model.
pub fn fixture_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../crible-encode/tests/fixtures/fake-model")
}

/// Vocabulary of the fixture tokenizer: one id per whitespace-separated word.
pub fn vocabulary() -> BTreeMap<String, u32> {
    let path = fixture_directory().join("tokenizer.json");
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
    let tokenizer: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("{} must be valid JSON: {error}", path.display()));
    let entry = tokenizer
        .get("model")
        .and_then(|model| model.get("vocab"))
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("{} must declare model.vocab", path.display()));
    let mut vocabulary = BTreeMap::new();
    for (word, id) in entry {
        let id = id
            .as_u64()
            .unwrap_or_else(|| panic!("the id of '{word}' must be a positive integer"))
            as u32;
        vocabulary.insert(word.clone(), id);
    }
    vocabulary
}

/// Token ids of `text`, prefixed by the `document: ` prompt when asked.
///
/// The fixture tokenizer lowercases, splits on whitespace and maps each word to
/// one id; an unknown word is `[UNK]`, id 0, exactly as `tools/fake_encoder.py`
/// declares.
pub fn fake_ids(vocabulary: &BTreeMap<String, u32>, text: &str, with_prompt: bool) -> Vec<u32> {
    let mut ids = Vec::new();
    if with_prompt {
        let prompt = vocabulary
            .get("document:")
            .copied()
            .unwrap_or_else(|| panic!("the fixture vocabulary must hold the `document:` token"));
        ids.push(prompt);
    }
    for word in text.split_whitespace() {
        ids.push(vocabulary.get(&word.to_lowercase()).copied().unwrap_or(0));
    }
    ids
}

/// The vector the fake fp32 graph produces for `text`, normalized as the engine
/// does: CLS row `[mean(ids), 1, …, 1]`, then L2 normalization.
pub fn fake_vector(vocabulary: &BTreeMap<String, u32>, text: &str, with_prompt: bool) -> Vec<f32> {
    let ids = fake_ids(vocabulary, text, with_prompt);
    assert!(
        !ids.is_empty(),
        "the fixture text '{text}' must tokenize to at least one id"
    );
    let count = u32::try_from(ids.len()).unwrap_or_else(|_| panic!("a token count must fit a u32"));
    let sum: u32 = ids.iter().sum();
    let mean = sum as f32 / count as f32;
    let mut row = vec![1.0_f32; DIMS];
    row[0] = mean;
    normalize(&mut row);
    row
}

/// Normalizes `vector` in place the way `crible_encode::Model` does.
pub fn normalize(vector: &mut [f32]) {
    let norm = vector
        .iter()
        .map(|component| f64::from(*component) * f64::from(*component))
        .sum::<f64>()
        .sqrt() as f32;
    assert!(norm > 0.0, "a fixture vector must not be null");
    for component in vector {
        *component /= norm;
    }
}

// --- A query and its answer, shared by INV-08 and INV-09 ---

/// Tolerance on a score, from the acceptance criterion of the task sheets:
/// "mêmes clés et mêmes scores à `1e-6` près".
pub const SCORE_TOLERANCE: f32 = 1e-6;

/// The three batches of `synth-small`, and the generation each one publishes.
pub const SYNTH_BATCHES: [(&str, u64); 3] = [("l1", 1), ("l2", 2), ("l3", 3)];

/// Live passages `synth-small` holds after its three batches, before the
/// metadata patch of INV-08 (which changes no passage): 10 000 created, 300
/// documents replaced, 150 documents deleted (`tools/gen_synth.py`).
pub const SYNTH_LIVE_PASSAGES: u64 = 9_750;

/// Number of results asked of a Legifacile query.
pub const LEGIFACILE_K: usize = 100;

/// The structures of the Legifacile sample, in the order of its batches: one
/// index each, and one batch `legifacile-sample-<structure>-v1`.
pub const LEGIFACILE_STRUCTURES: [&str; 6] =
    ["dole", "articles", "jorf", "titres", "sarde", "decisions"];

/// Command that generates the synthetic dataset.
pub const SYNTH_DATASET_COMMAND: &str =
    "cd tools && uv sync && uv run python gen_synth.py --out ../data/synth-small";

/// Command that builds the Legifacile sample and its model (gitignored).
pub const LEGIFACILE_SAMPLE_COMMAND: &str = "build data/legifacile-sample/ as \
     data/legifacile-sample/README.md, section \"Reproduire\", describes (production \
     read-only), then:\n  cd tools && uv sync --group export && uv run python export_onnx.py \
     --out ../data/models/mdenseon --overwrite";

/// External model configuration of `synth-small` (768 dimensions, `normalize`),
/// the one the dataset declares and the J1 demo writes.
pub const EXTERNAL_MODEL_JSON: &str =
    r#"{ "name": "synthetic-external", "kind": "external", "dims": 768, "normalize": true }"#;

/// One query of an invariant: a label for the failure messages, the number of
/// results asked for, the vector, and the filter of its pool.
pub struct Query {
    pub label: String,
    pub k: usize,
    pub vector: Vec<f32>,
    pub filter: Filter,
}

/// What one query answered: the size of its pool and the top-k results.
pub struct Outcome {
    pub pool_size: u32,
    pub hits: Vec<Hit>,
}

// --- Tiny JSON readers with readable failures ---

pub fn read_file(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()))
}

pub fn read_json_lines(path: &Path) -> Vec<Value> {
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

pub fn array(value: &Value, name: &str) -> Vec<f32> {
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

pub fn unsigned<T>(value: &Value, name: &str) -> T
where
    T: TryFrom<u64>,
{
    let raw = value
        .get(name)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("{value} must carry an unsigned '{name}'"));
    T::try_from(raw).unwrap_or_else(|_| panic!("{name} = {raw} does not fit the expected type"))
}

pub fn text(value: &Value, name: &str) -> String {
    value
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{value} must carry a string '{name}'"))
        .to_owned()
}

// --- Filters ---

/// Parses and validates a filter, `{}` when the JSON carries none.
pub fn parse_filter(raw: &str, schema: &Schema) -> Filter {
    let filter = Filter::from_json_str(raw)
        .unwrap_or_else(|error| panic!("the filter {raw} must parse: {error}"));
    filter
        .validate(schema)
        .unwrap_or_else(|error| panic!("the filter {raw} must be valid: {error}"));
    filter
}

/// The empty conjunction: every live passage satisfies it, so its pool is the
/// whole live set (`{}` in the filter grammar).
pub fn match_all(schema: &Schema) -> Filter {
    parse_filter("{}", schema)
}

/// Parses and validates the filter of one query, `{}` when it carries none.
pub fn query_filter(value: &Value, schema: &Schema) -> Filter {
    let raw = match value.get("filter") {
        Some(filter) if filter.is_object() => filter.to_string(),
        _ => "{}".to_owned(),
    };
    parse_filter(&raw, schema)
}

// --- Running the exact search ---

/// Runs every query over the published generation, opening the searcher once.
///
/// The pool of a query is evaluated on the filter structures of the snapshot,
/// as `RECHERCHER-DENSE-EXACT` does: the step under test must not change the
/// pool either, so its size is returned and compared too.
pub fn search_all(store: &GenerationStore, queries: &[Query]) -> Vec<Outcome> {
    let snapshot = store.snapshot();
    let searcher = DenseSearcher::open(
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

// --- The comparison of INV-08 and INV-09 ---

/// Compares two answers of the same query, position by position.
///
/// The keys must be equal and in the same order, the scores must be within
/// [`SCORE_TOLERANCE`] and finite, and the answer must hold the same number of
/// results. The payload the export carries (the document and passage fields
/// merged, and the encoder input) is compared too, so a batch that lost a column
/// cannot pass. An error names the position and both values, with the value of
/// each argument under its own label, so a failure says what moved and where.
pub fn compare_results(before: &[Hit], after: &[Hit]) -> Result<(), String> {
    if before.len() != after.len() {
        return Err(format!(
            "{} result(s) before, {} after",
            before.len(),
            after.len()
        ));
    }
    for (position, (before_hit, after_hit)) in before.iter().zip(after).enumerate() {
        if before_hit.key != after_hit.key {
            return Err(format!(
                "position {position}: key {} before, {} after",
                before_hit.key, after_hit.key
            ));
        }
        let gap = (before_hit.score - after_hit.score).abs();
        if !gap.is_finite() || gap > SCORE_TOLERANCE {
            return Err(format!(
                "position {position}: key {} scored {:.9} before and {:.9} after, a gap of \
                 {gap:.9} that is not within the {SCORE_TOLERANCE} tolerance",
                before_hit.key, before_hit.score, after_hit.score
            ));
        }
        if before_hit.fields != after_hit.fields {
            return Err(format!(
                "position {position}: key {} carries the fields {} before and {} after",
                before_hit.key, before_hit.fields, after_hit.fields
            ));
        }
        if before_hit.text != after_hit.text {
            return Err(format!(
                "position {position}: key {} carries the text {:?} before and {:?} after",
                before_hit.key, before_hit.text, after_hit.text
            ));
        }
    }
    Ok(())
}

/// Asserts that one query answered the same thing before and after `change`
/// (for example "the import", "the compaction"), and that its pool did not move.
pub fn assert_same_results(change: &str, label: &str, before: &Outcome, after: &Outcome) {
    assert_eq!(
        before.pool_size, after.pool_size,
        "query {label}: the pool must hold the same live passages after {change} \
         ({} before, {} after)",
        before.pool_size, after.pool_size
    );
    compare_results(&before.hits, &after.hits)
        .unwrap_or_else(|message| panic!("query {label}: {message}"));
}

/// Checks the premises of a non-empty answer: the pool is not empty and the
/// answer holds `min(k, pool_size)` results. An empty pool would compare
/// nothing and read as a pass.
pub fn assert_answer_is_not_vacuous(query: &Query, answer: &Outcome) {
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

// --- The synthetic dataset ---

pub fn synth_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/synth-small")
}

/// Fails with the generator command when a file of the dataset is missing, as
/// every test of the crate does.
pub fn require_synth(root: &Path) {
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
pub fn synth_queries(root: &Path, index: &IndexConfig) -> Vec<Query> {
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

// --- The Legifacile sample ---

pub fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Paths of the Legifacile sample the ignored tests read.
#[derive(Debug)]
pub struct SamplePaths {
    /// Directory of the sample, holding `queries.jsonl` and `configs/`.
    pub directory: PathBuf,
    /// Root of the models, holding `<model>/model.json`.
    pub models: PathBuf,
}

impl SamplePaths {
    /// Configuration of one structure: `configs/<structure>.json`.
    pub fn config(&self, structure: &str) -> PathBuf {
        self.directory.join(format!("configs/{structure}.json"))
    }

    /// Batch of one structure: `batches/legifacile-sample-<structure>-v1`.
    pub fn batch(&self, structure: &str) -> PathBuf {
        self.directory
            .join(format!("batches/legifacile-sample-{structure}-v1"))
    }
}

/// Returns the paths of the sample, or the message naming the missing file and
/// the command that produces it (`docs/08`, "Sauts de test"; ADR-0057).
///
/// `test_name` is the `#[ignore]`d test the message blocks, so the reader knows
/// which file to run once the data exists.
pub fn legifacile_sample_paths(root: &Path, test_name: &str) -> Result<SamplePaths, String> {
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
                "{} is missing, so {test_name} cannot run; build the gitignored data with:\n  \
                 {LEGIFACILE_SAMPLE_COMMAND}",
                required.display()
            ));
        }
    }
    Ok(paths)
}

/// Runs the `crible` binary under test and returns its output.
pub fn crible(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_crible"))
        .args(arguments)
        .output()
        .unwrap_or_else(|error| panic!("cannot run crible {arguments:?}: {error}"))
}

/// Runs the `crible` binary under test and asserts it succeeded.
pub fn crible_ok(arguments: &[&str]) {
    let output = crible(arguments);
    assert!(
        output.status.success(),
        "crible {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn path(path: &Path) -> &str {
    path.to_str()
        .unwrap_or_else(|| panic!("the path {} must be UTF-8", path.display()))
}

/// The query encoder of the model of the sample, in fp32.
///
/// This is the path of `crible search --query`: the same `QueryEncoder`, the same
/// `DEFAULT_QUERY_PRECISION` (fp32, ADR-0052) and the same `query: ` prompt. One
/// encoder serves every structure, since they share the model.
pub fn legifacile_encoder(sample: &SamplePaths, index: &IndexConfig) -> QueryEncoder {
    let directory = sample.models.join(&index.model);
    QueryEncoder::new(
        &directory,
        Arc::new(ModelRegistry::new()),
        DEFAULT_QUERY_CACHE_ENTRIES,
    )
}

/// The queries of one structure, encoded from their question.
pub fn legifacile_queries(
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
