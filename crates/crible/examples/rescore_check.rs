// SPDX-License-Identifier: Apache-2.0

//! Answer-dump driver of the `REDUIRE-COUT-DES-CLES` measurements (MES-03).
//!
//! This example is a harness, not production code, and it changes nothing in
//! `crates/*/src/`. Both `dense_latency rescore` and `fidelity` drive the engine
//! but expose neither the per-query answer at an arbitrary `N` nor the same
//! answer from two builds: the first stores durations only, the second takes `N`
//! from `index.json`, which the measurement copies keep read-only.
//!
//! It therefore runs the production entry point (`dense::search`) on a clone of
//! the index configuration where `search.rescore_candidates` is the requested
//! `N` and `search.exact_threshold` is 0 (BINARY forced, ADR-0074), and writes the
//! top-`k` it returns: key, score, `doc_id` and `ord`, in order. Two builds then
//! compare their documents bit for bit, which is the equality control ADR-0079,
//! decision 6, asks for (ADR-0072, decision 3).
//!
//! ```text
//! rescore_check --data D --index I --vectors Q --ns 1000,2000,5000 --k 10 --out O
//! rescore_check --data D --index I --vectors Q --exact --k 10 --out O
//! ```
//!
//! The query file is a row-major little-endian `f32` matrix, one row per query,
//! as for `dense_latency` and `fidelity`; the filter is empty, so the pool is the
//! whole live set, which is what the MES-03 series measure.

use std::fs;
use std::path::{Path, PathBuf};

use crible_core::config::{IndexConfig, ScanStrategy};
use crible_core::filter::Filter;
use crible_search::dense::exact::DenseSearcher;
use crible_search::dense::search as dense_search;
use crible_search::pool::Pool;
use crible_search::scan_pool::ScanPool;
use crible_store::generation::GenerationStore;
use serde_json::{Value, json};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().collect();
    let data = PathBuf::from(required(&arguments, "--data")?);
    let index = required(&arguments, "--index")?;
    let vectors_path = PathBuf::from(required(&arguments, "--vectors")?);
    let out = PathBuf::from(required(&arguments, "--out")?);
    let exact = arguments.iter().any(|argument| argument == "--exact");
    let k: usize = optional(&arguments, "--k")
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(10);
    let ns: Vec<u32> = if exact {
        vec![0]
    } else {
        required(&arguments, "--ns")?
            .split(',')
            .map(|text| text.trim().parse())
            .collect::<Result<_, _>>()?
    };

    let index_dir = data.join("indexes").join(&index);
    let raw = fs::read_to_string(index_dir.join("index.json"))?;
    let config = IndexConfig::from_json_str(&raw)?;
    let store = GenerationStore::open(&index_dir, &config.schema)?;
    let snapshot = store.snapshot();
    let dims = snapshot
        .segments()
        .first()
        .map(|meta| meta.dims as usize)
        .ok_or("the index holds no segment, so its dimensionality is unknown")?;
    let matrix = read_matrix(&vectors_path, dims)?;

    let scan = ScanPool::new(0)?;
    let segments_directory = store.segments_directory();
    let records_path = store.records_path();
    let segments = snapshot.segments().to_vec();
    let searcher =
        scan.install(|| DenseSearcher::open(&segments_directory, &segments, &records_path))?;
    let pool = Pool::evaluate(snapshot.filters(), &Filter::And(Vec::new()))?;

    let mut points = Vec::with_capacity(ns.len());
    for n in &ns {
        let mut search = config.search.clone();
        if !exact {
            search.scan = ScanStrategy::Binary;
            search.exact_threshold = 0;
            search.rescore_candidates = *n;
        }
        let mut records = Vec::with_capacity(matrix.len());
        for (position, vector) in matrix.iter().enumerate() {
            let hits = scan
                .install(|| dense_search(&snapshot, &searcher, &pool, vector, k, &search, exact))?;
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
                "query": position,
                "pool_size": pool.len(),
                "results": results,
            }));
        }
        points.push(json!({
            "n": n,
            "rescore_candidates": search.rescore_candidates,
            "exact": exact,
            "results": records,
        }));
    }

    let report = json!({
        "index": index,
        "exact": exact,
        "k": k,
        "dims": dims,
        "live_passages": pool.len(),
        "points": points,
    });
    fs::write(&out, serde_json::to_vec(&report)?)?;
    println!(
        "rescore_check: index '{index}', {} point(s), k {k}, exact {exact}, wrote {}",
        ns.len(),
        out.display()
    );
    Ok(())
}

fn read_matrix(path: &Path, dims: usize) -> Result<Vec<Vec<f32>>, Box<dyn std::error::Error>> {
    let bytes = fs::read(path)?;
    let width = dims * std::mem::size_of::<f32>();
    if bytes.len() % width != 0 {
        return Err(format!(
            "{} holds {} bytes, not a whole number of {dims}-component rows",
            path.display(),
            bytes.len()
        )
        .into());
    }
    let rows = bytes.len() / width;
    let mut matrix = Vec::with_capacity(rows);
    for row in 0..rows {
        let start = row * width;
        let mut vector = Vec::with_capacity(dims);
        for column in 0..dims {
            let offset = start + column * 4;
            let mut component = [0_u8; 4];
            component.copy_from_slice(&bytes[offset..offset + 4]);
            vector.push(f32::from_le_bytes(component));
        }
        matrix.push(vector);
    }
    Ok(matrix)
}

fn required(arguments: &[String], name: &str) -> Result<String, String> {
    optional(arguments, name).ok_or_else(|| format!("missing required argument '{name}'"))
}

fn optional(arguments: &[String], name: &str) -> Option<String> {
    arguments
        .iter()
        .position(|argument| argument == name)
        .and_then(|position| arguments.get(position + 1))
        .cloned()
}
