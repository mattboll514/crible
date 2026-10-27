// SPDX-License-Identifier: Apache-2.0

//! Measurement driver of MES-01 and MES-12 (fidelity of the binary scan).
//!
//! This example is a harness, not production code. It runs the dense branch of
//! the engine over a list of queries and a provided matrix of `f32` vectors, and
//! writes the top-k it returns together with the dense plan the engine actually
//! built (`dense_strategy` and `rescore_candidates`). It exists so that the
//! fidelity measurement compares the engine's own BINARY answer to its own
//! EXACT answer, and so that a figure is only kept when the plan says
//! `"binary"` (ADR-0076, decision 4).
//!
//! ```text
//! fidelity --data <dir> --index <name> --vectors <matrix.f32> \
//!     --queries <queries.jsonl> [--exact] [--k 10] --out <out.json>
//! ```
//!
//! The matrix is row-major little-endian `f32`, one row per query, in the order
//! of the input file; its width is the `dims` of the segments. A query line is
//! `{"id": "...", "filter": {...}}`; the filter is optional and follows the
//! grammar of `docs/05-api.md`. The output keeps no query text and no document
//! value, only the opaque query identifier, the pool size, the plan and the
//! `doc_id#ord` keys with their scores.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crible_core::config::IndexConfig;
use crible_core::filter::Filter;
use crible_search::dense::exact::ExactSearcher;
use crible_search::dense::{plan as dense_plan, search as dense_search};
use crible_search::pool::Pool;
use crible_search::scan_pool::ScanPool;
use crible_store::generation::GenerationStore;
use serde_json::{Value, json};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().collect();
    let data = PathBuf::from(required(&arguments, "--data")?);
    let index = required(&arguments, "--index")?;
    let vectors_path = PathBuf::from(required(&arguments, "--vectors")?);
    let queries_path = PathBuf::from(required(&arguments, "--queries")?);
    let out = PathBuf::from(required(&arguments, "--out")?);
    let exact = arguments.iter().any(|argument| argument == "--exact");
    let k: usize = optional(&arguments, "--k")
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(10);

    let index_dir = data.join("indexes").join(&index);
    let raw = fs::read_to_string(index_dir.join("index.json"))?;
    let config = IndexConfig::from_json_str(&raw)?;
    let queries = read_queries(&queries_path)?;

    let store = GenerationStore::open(&index_dir, &config.schema)?;
    let snapshot = store.snapshot();
    let dims = snapshot
        .segments()
        .first()
        .map(|meta| meta.dims as usize)
        .ok_or("the index holds no segment, so its dimensionality is unknown")?;
    let matrix = read_matrix(&vectors_path, dims)?;
    if matrix.len() != queries.len() {
        return Err(format!(
            "the matrix holds {} rows where the query file holds {} lines",
            matrix.len(),
            queries.len()
        )
        .into());
    }

    let scan = ScanPool::new(0)?;
    let segments_directory = store.segments_directory();
    let records_path = store.records_path();
    let segments = snapshot.segments().to_vec();
    let searcher =
        scan.install(|| ExactSearcher::open(&segments_directory, &segments, &records_path))?;

    let mut records = Vec::with_capacity(queries.len());
    let mut binary_runs = 0_usize;
    for (query, vector) in queries.iter().zip(&matrix) {
        let filter = match &query.filter {
            Some(filter) => Filter::from_json_str(&filter.to_string())?,
            None => Filter::And(Vec::new()),
        };
        let pool = Pool::evaluate(snapshot.filters(), &filter)?;
        let plan = dense_plan(&config.search, pool.len(), k, exact);
        if plan.strategy.as_str() == "binary" {
            binary_runs += 1;
        }
        let hits = scan.install(|| {
            dense_search(
                &snapshot,
                &searcher,
                &pool,
                vector,
                k,
                &config.search,
                exact,
            )
        })?;
        let results: Vec<Value> = hits
            .iter()
            .map(|hit| {
                json!({
                    "key": hit.key,
                    "doc_id": hit.doc_id,
                    "ord": hit.ord,
                    "score": hit.score,
                })
            })
            .collect();
        records.push(json!({
            "id": query.id,
            "pool_size": pool.len(),
            "dense_strategy": plan.strategy.as_str(),
            "rescore_candidates": plan.rescore_candidates,
            "results": results,
        }));
    }

    let report = json!({
        "index": index,
        "exact": exact,
        "k": k,
        "dims": dims,
        "queries": records.len(),
        "binary_runs": binary_runs,
        "search": {
            "scan": match config.search.scan {
                crible_core::config::ScanStrategy::Binary => "binary",
                crible_core::config::ScanStrategy::None => "none",
            },
            "rescore_candidates": config.search.rescore_candidates,
            "exact_threshold": config.search.exact_threshold,
            "candidates_per_branch": config.search.candidates_per_branch,
        },
        "results": records,
    });
    fs::write(&out, serde_json::to_vec(&report)?)?;
    println!(
        "index '{index}': {} queries, {binary_runs} binary run(s), k {k}, exact {exact}, wrote {}",
        records.len(),
        out.display()
    );
    Ok(())
}

/// One input query: an opaque identifier and an optional filter.
struct Query {
    id: String,
    filter: Option<Value>,
}

/// Returns the value following `name` on the command line.
fn required(arguments: &[String], name: &str) -> Result<String, String> {
    optional(arguments, name).ok_or_else(|| format!("missing {name}"))
}

/// Returns the value following `name`, when present.
fn optional(arguments: &[String], name: &str) -> Option<String> {
    let position = arguments.iter().position(|argument| argument == name)?;
    arguments.get(position + 1).cloned()
}

/// Reads `{"id": "...", "filter": {...}}` lines.
fn read_queries(path: &Path) -> Result<Vec<Query>, Box<dyn std::error::Error>> {
    let file = fs::File::open(path)?;
    let mut queries = Vec::new();
    for (line_number, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(&line)?;
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("line {} has no string 'id'", line_number + 1))?;
        let filter = value
            .get("filter")
            .filter(|filter| !filter.is_null())
            .cloned();
        queries.push(Query {
            id: id.to_owned(),
            filter,
        });
    }
    Ok(queries)
}

/// Reads a row-major little-endian `f32` matrix of `dims` columns.
fn read_matrix(path: &Path, dims: usize) -> Result<Vec<Vec<f32>>, Box<dyn std::error::Error>> {
    let bytes = fs::read(path)?;
    if dims == 0 || bytes.len() % (dims * 4) != 0 {
        return Err(format!(
            "{} holds {} bytes, not a whole number of {dims}-component f32 rows",
            path.display(),
            bytes.len()
        )
        .into());
    }
    let rows = bytes.len() / (dims * 4);
    let mut matrix = Vec::with_capacity(rows);
    for row in 0..rows {
        let start = row * dims * 4;
        let mut vector = Vec::with_capacity(dims);
        for component in 0..dims {
            let offset = start + component * 4;
            let value = f32::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ]);
            vector.push(value);
        }
        matrix.push(vector);
    }
    Ok(matrix)
}
