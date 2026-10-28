// SPDX-License-Identifier: Apache-2.0

//! A/B driver of the rescoring step (MES-03, `REDUIRE-COUT-DES-CLES`).
//!
//! This example is a harness, not production code, and it changes nothing in
//! `crates/*/src/`. It exists because the two other drivers do not measure the
//! step the task changes:
//!
//! - `dense_latency rescore` passes `candidates.len()` as the consumer count, so
//!   it reads one key per candidate and would hide the whole effect of reading
//!   the keys after the score (ADR-0079, decision 3);
//! - `rescore_profile` loops for a number of seconds and is built for `sample`,
//!   not for a fixed, alternated series of passes.
//!
//! It therefore builds the pool and the candidate sets the way production does
//! (`Pool::evaluate` with the empty filter, then the engine's own
//! `dense::binary::top_k`), and times **only** `ExactSearcher::rescore`, called
//! with the consumer count production gives it: `search.candidates_per_branch`,
//! that is `k`, as `planner::candidates` does (`dense/planner.rs`, `rescore(…,
//! k)`). Base before the task has no consumer count and is called without it,
//! which is what production does there.
//!
//! ```text
//! rescore_ab measure --data D --index I --vectors Q --ns N --passes P
//!                    --out O [--queries n]
//! ```
//!
//! The document holds the parameters, the candidate diagnostics (count,
//! distinct identifiers, identifier order), every duration of every pass in
//! milliseconds, and the per-pass and overall percentiles by nearest rank
//! (ADR-0077). Nothing is filtered or excluded.

use std::collections::HashSet;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crible_core::config::IndexConfig;
use crible_core::filter::Filter;
use crible_search::dense::binary::{self, Candidate};
use crible_search::dense::exact::ExactSearcher;
use crible_search::pool::Pool;
use crible_search::scan_pool::ScanPool;
use crible_store::codes::quantize;
use crible_store::generation::GenerationStore;
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().collect();
    let mode = arguments.get(1).cloned().ok_or(
        "usage: rescore_ab measure --data D --index I --vectors Q --ns N --passes P --out O",
    )?;
    match mode.as_str() {
        "measure" => measure_mode(&arguments),
        other => Err(format!("unknown mode '{other}': expected 'measure'").into()),
    }
}

fn measure_mode(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(required(arguments, "--out")?);
    let data = PathBuf::from(required(arguments, "--data")?);
    let index = required(arguments, "--index")?;
    let vectors = PathBuf::from(required(arguments, "--vectors")?);
    let n: usize = required(arguments, "--ns")?.parse()?;
    let passes: usize = required(arguments, "--passes")?.parse()?;
    let query_count: Option<usize> = optional(arguments, "--queries")
        .map(|value| value.parse())
        .transpose()?;

    let index_dir = data.join("indexes").join(&index);
    let raw = fs::read_to_string(index_dir.join("index.json"))?;
    let config = IndexConfig::from_json_str(&raw)?;
    let store = GenerationStore::open(&index_dir, &config.schema)?;
    let snapshot = store.snapshot();
    let dims = snapshot
        .segments()
        .first()
        .ok_or("the index holds no segment")?
        .dims as usize;
    let mut queries = read_matrix(&vectors, dims)?;
    if let Some(count) = query_count {
        queries.truncate(count);
    }
    let k = config.search.candidates_per_branch as usize;
    let live = snapshot.filters().live().len() as u32;
    let pool = Pool::evaluate(snapshot.filters(), &Filter::from_json_str("{}")?)?;
    let threads = ScanPool::new(0)?.threads();
    let scan_pool = ScanPool::new(threads)?;
    let wanted = n.max(k);
    let query_codes: Vec<Vec<u64>> = queries.iter().map(|query| quantize(query)).collect();
    let mut candidate_sets: Vec<Vec<Candidate>> = Vec::with_capacity(queries.len());
    for query_code in &query_codes {
        candidate_sets
            .push(scan_pool.install(|| binary::top_k(&snapshot, &pool, query_code, wanted))?);
    }

    let mut distinct: HashSet<u32> = HashSet::new();
    let mut iid_inversions = 0_usize;
    let mut pairs = 0_usize;
    for candidates in &candidate_sets {
        for candidate in candidates {
            distinct.insert(candidate.iid.get());
        }
        for pair in candidates.windows(2) {
            pairs += 1;
            if pair[1].iid.get() <= pair[0].iid.get() {
                iid_inversions += 1;
            }
        }
    }

    let segments_directory = store.segments_directory();
    let records_path = store.records_path();
    let segments = snapshot.segments().to_vec();
    let searcher =
        scan_pool.install(|| ExactSearcher::open(&segments_directory, &segments, &records_path))?;

    let mut pass_durations: Vec<Vec<f64>> = Vec::with_capacity(passes);
    let mut returned_lengths: Vec<usize> = Vec::with_capacity(queries.len() * passes);
    for pass in 0..passes {
        let mut durations = Vec::with_capacity(queries.len());
        for (query, candidates) in queries.iter().zip(&candidate_sets) {
            let started = Instant::now();
            let scored = searcher.rescore(query, candidates, k)?;
            durations.push(started.elapsed().as_secs_f64() * 1_000.0);
            returned_lengths.push(scored.len());
            black_box(scored.len());
        }
        pass_durations.push(durations);
        let mut sorted: Vec<f64> = pass_durations[pass].clone();
        println!(
            "[{}] pass {} of N = {n}: p50 {:.3} ms, p95 {:.3} ms",
            index,
            pass,
            nearest_rank(&mut sorted, 0.50),
            nearest_rank(&mut sorted, 0.95)
        );
    }

    let all: Vec<f64> = pass_durations.iter().flatten().copied().collect();
    let mut all_sorted = all.clone();
    let mut per_pass = Vec::with_capacity(passes);
    for durations in &pass_durations {
        let mut sorted = durations.clone();
        per_pass.push(json!({
            "p50": nearest_rank(&mut sorted, 0.50),
            "p95": nearest_rank(&mut sorted, 0.95),
            "p99": nearest_rank(&mut sorted, 0.99),
            "min": sorted.first().copied().unwrap_or(f64::NAN),
            "max": sorted.last().copied().unwrap_or(f64::NAN),
            "n": durations.len(),
        }));
    }
    let document = json!({
        "kind": "rescore-ab",
        "index": index,
        "dims": dims,
        "live": live,
        "k": k,
        "n": n,
        "limit": k,
        "queries": queries.len(),
        "passes": passes,
        "candidates_min": candidate_sets.iter().map(Vec::len).min(),
        "candidates_max": candidate_sets.iter().map(Vec::len).max(),
        "distinct_iids": distinct.len(),
        "candidate_pairs": pairs,
        "iid_inversions": iid_inversions,
        "returned_min": returned_lengths.iter().copied().min(),
        "returned_max": returned_lengths.iter().copied().max(),
        "durations_ms": pass_durations,
        "per_pass": per_pass,
        "summary": {
            "p50": nearest_rank(&mut all_sorted, 0.50),
            "p95": nearest_rank(&mut all_sorted, 0.95),
            "p99": nearest_rank(&mut all_sorted, 0.99),
            "min": all_sorted.first().copied().unwrap_or(f64::NAN),
            "max": all_sorted.last().copied().unwrap_or(f64::NAN),
            "n": all.len(),
        },
    });
    fs::write(&out, serde_json::to_vec(&document)?)?;
    println!(
        "rescore_ab: N = {n}, {} passes, p50 {:.3} ms, p95 {:.3} ms, wrote {}",
        passes,
        nearest_rank(&mut all.clone(), 0.50),
        nearest_rank(&mut all.clone(), 0.95),
        out.display()
    );
    Ok(())
}

/// Nearest-rank percentile (ADR-0077), sorting in place.
fn nearest_rank(values: &mut [f64], fraction: f64) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    values.sort_by(f64::total_cmp);
    let rank = (fraction * values.len() as f64).ceil().max(1.0) as usize;
    values[rank.min(values.len()) - 1]
}

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
        let mut vector = Vec::with_capacity(dims);
        for component in 0..dims {
            let offset = row * dims * 4 + component * 4;
            vector.push(f32::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ]));
        }
        matrix.push(vector);
    }
    Ok(matrix)
}

fn required(arguments: &[String], name: &str) -> Result<String, String> {
    optional(arguments, name).ok_or_else(|| format!("missing {name}"))
}

fn optional(arguments: &[String], name: &str) -> Option<String> {
    let position = arguments.iter().position(|argument| argument == name)?;
    arguments.get(position + 1).cloned()
}
