// SPDX-License-Identifier: Apache-2.0

//! Sampling-profile driver of the rescoring step (MES-03, `REDUIRE-COUT-DES-CLES`).
//!
//! References: `docs/08-invariants-et-tests.md` (MES-03), **ADR-0079** (decision
//! 1: sample the rescoring before any change of code), ADR-0077 (measurement
//! protocol), `docs/phases/P4-recherche-rapide.md` (`REDUIRE-COUT-DES-CLES`).
//!
//! This example is a harness, not production code, and it changes nothing in
//! `crates/*/src/`. It exists because a sampling profiler (`/usr/bin/sample`,
//! `xctrace`, `samply`) has to attach to a process that lives long enough: the
//! engine's own measurement driver (`dense_latency rescore`) runs one pass of
//! 300 queries in a few seconds, while a profile wants tens of seconds of the
//! same loop to collect thousands of samples.
//!
//! # What it does
//!
//! It builds the pool the way production does (`Pool::evaluate` with the empty
//! filter, that is the whole live set), keeps the `max(N, k)` candidates of the
//! **engine's own** binary scan (`dense::binary::top_k`, no replica: the
//! candidates are the ones the rescoring really receives), and then repeats
//! `ExactSearcher::rescore` over the `--vectors` queries for `--seconds`. Only
//! the rescoring runs in that loop, so every sample of the loop belongs to the
//! step under study.
//!
//! # Modes
//!
//! ```text
//! rescore_profile run   --data D --index I --vectors Q --ns N --seconds S
//!                       --out O [--queries Q] [--ready-file F] [--warmup-passes P]
//! rescore_profile order --data D --index I --vectors Q --ns N --out O [--queries Q]
//! ```
//!
//! `run` writes one JSON document: the parameters, the ordered candidate
//! diagnostics, every per-iteration duration in milliseconds and their
//! percentiles by nearest rank (ADR-0077). `--warmup-passes P` runs `P` full
//! passes over the same queries before the sampled loop, and records the p50 of
//! each pass: a warm series of ADR-0077 needs its pages resident before the
//! measurement starts, and the harness says how warm it actually got. `order` writes the candidate
//! diagnostics only: how the binary scan orders the candidates it hands to the
//! rescoring, which ADR-0079 decision 2 needs to know before adding a sort.
//!
//! The loop is sequential and single-threaded, as production is
//! (`rescore.rs`: "the loop is sequential"): a sampling profiler then reads one
//! stack and the attribution is unambiguous.
//!
//! Commands and readings are in `docs/mesures/MES-03-latence-rescoring.md`.

use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crible_core::config::IndexConfig;
use crible_core::filter::Filter;
use crible_search::dense::binary::{self, Candidate};
use crible_search::dense::exact::ExactSearcher;
use crible_search::pool::Pool;
use crible_search::scan_pool::ScanPool;
use crible_store::codes::quantize;
use crible_store::generation::GenerationStore;
use serde_json::{Value, json};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().collect();
    let mode = arguments.get(1).cloned().ok_or(
        "usage: rescore_profile <run|order> --data D --index I --vectors Q --ns N --out O",
    )?;
    match mode.as_str() {
        "run" => run_mode(&arguments),
        "order" => order_mode(&arguments),
        other => Err(format!("unknown mode '{other}': expected 'run' or 'order'").into()),
    }
}

/// Where the epoch lives and how the measurement copy resolves the dense plan.
struct Opened {
    /// Search settings of the measurement copy.
    config: IndexConfig,
    /// Generation store: the snapshot, the bitmaps and the binary codes.
    store: GenerationStore,
    /// Absolute path of the vectors directory of the epoch.
    segments_directory: PathBuf,
    /// Absolute path of the `records.sqlite` of the epoch.
    records_path: PathBuf,
    /// Worker count of the scan pool.
    threads: usize,
}

impl Opened {
    fn open(data: &Path, index: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let index_dir = data.join("indexes").join(index);
        let raw = fs::read_to_string(index_dir.join("index.json"))?;
        let config = IndexConfig::from_json_str(&raw)?;
        let store = GenerationStore::open(&index_dir, &config.schema)?;
        let segments_directory = store.segments_directory();
        let records_path = store.records_path();
        let threads = ScanPool::new(0)?.threads();
        Ok(Self {
            config,
            store,
            segments_directory,
            records_path,
            threads,
        })
    }

    fn snapshot(&self) -> std::sync::Arc<crible_store::generation::Snapshot> {
        self.store.snapshot()
    }

    /// Opens the exact searcher, which reads and checksums every vector file.
    fn searcher(&self) -> Result<ExactSearcher, Box<dyn std::error::Error>> {
        let pool = ScanPool::new(self.threads)?;
        let segments = self.snapshot().segments().to_vec();
        let directory = self.segments_directory.clone();
        let records = self.records_path.clone();
        Ok(pool.install(|| ExactSearcher::open(&directory, &segments, &records))?)
    }
}

/// Everything the two modes share: queries, pool, and their candidate sets.
struct Prepared {
    opened: Opened,
    queries: Vec<Vec<f32>>,
    candidate_sets: Vec<Vec<Candidate>>,
    live: u32,
    dims: usize,
    k: usize,
}

impl Prepared {
    fn build(arguments: &[String], n: usize) -> Result<Self, Box<dyn std::error::Error>> {
        let data = PathBuf::from(required(arguments, "--data")?);
        let index = required(arguments, "--index")?;
        let vectors = PathBuf::from(required(arguments, "--vectors")?);
        let query_count: Option<usize> = optional(arguments, "--queries")
            .map(|value| value.parse())
            .transpose()?;
        let opened = Opened::open(&data, &index)?;
        let snapshot = opened.snapshot();
        let dims = snapshot
            .segments()
            .first()
            .ok_or("the index holds no segment")?
            .dims as usize;
        let mut queries = read_matrix(&vectors, dims)?;
        if let Some(count) = query_count {
            queries.truncate(count);
        }
        let k = opened.config.search.candidates_per_branch as usize;
        // The empty filter keeps the whole live set: the pool of a search
        // without a filter, and the pool MES-03 measures (ADR-0077).
        let filter = Filter::from_json_str("{}")?;
        let live = snapshot.filters().live().len() as u32;
        let pool = Pool::evaluate(snapshot.filters(), &filter)?;
        let scan_pool = ScanPool::new(opened.threads)?;
        let query_codes: Vec<Vec<u64>> = queries.iter().map(|query| quantize(query)).collect();
        let wanted = n.max(k);
        let mut candidate_sets = Vec::with_capacity(queries.len());
        for query_code in &query_codes {
            candidate_sets
                .push(scan_pool.install(|| binary::top_k(&snapshot, &pool, query_code, wanted))?);
        }
        Ok(Self {
            opened,
            queries,
            candidate_sets,
            live,
            dims,
            k,
        })
    }

    /// Per-query diagnostics of the order the scan hands to the rescoring.
    ///
    /// ADR-0079 decision 2 asks the rescoring to walk its candidates in
    /// ascending `iid`; the figure says whether the scan already does it.
    fn order_diagnostics(&self) -> Value {
        let mut documents = Vec::with_capacity(self.queries.len());
        let mut iid_inversions = 0_usize;
        let mut distance_inversions = 0_usize;
        let mut pairs = 0_usize;
        let mut iid_steps = Vec::new();
        for (position, candidates) in self.candidate_sets.iter().enumerate() {
            let mut query_iid_inversions = 0_usize;
            let mut query_distance_inversions = 0_usize;
            for pair in candidates.windows(2) {
                let (left, right) = (pair[0], pair[1]);
                pairs += 1;
                if right.iid.get() <= left.iid.get() {
                    query_iid_inversions += 1;
                }
                if right.distance < left.distance {
                    query_distance_inversions += 1;
                }
                iid_steps.push(i64::from(right.iid.get()) - i64::from(left.iid.get()));
            }
            iid_inversions += query_iid_inversions;
            distance_inversions += query_distance_inversions;
            documents.push(json!({
                "query": position,
                "candidates": candidates.len(),
                "iid_inversions": query_iid_inversions,
                "distance_inversions": query_distance_inversions,
                "first": candidates.iter().take(5).map(|candidate| json!({
                    "iid": candidate.iid.get(),
                    "distance": candidate.distance,
                })).collect::<Vec<Value>>(),
                "last": candidates.iter().rev().take(3).map(|candidate| json!({
                    "iid": candidate.iid.get(),
                    "distance": candidate.distance,
                })).collect::<Vec<Value>>(),
            }));
        }
        // Distinct candidates of the whole series: the pages their vectors and
        // their rows touch must stay in the page cache for the loop to be warm,
        // so this count is the working set of the warm series, in pages.
        let mut distinct: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for candidates in &self.candidate_sets {
            for candidate in candidates {
                distinct.insert(candidate.iid.get());
            }
        }
        let ascending_iid_pairs = iid_steps.iter().filter(|step| **step > 0).count();
        let negative_steps = iid_steps.iter().filter(|step| **step < 0).count();
        let mean_step = if iid_steps.is_empty() {
            0.0
        } else {
            iid_steps.iter().sum::<i64>() as f64 / iid_steps.len() as f64
        };
        json!({
            "queries": self.queries.len(),
            "pairs": pairs,
            "distinct_iids": distinct.len(),
            "iid_inversions": iid_inversions,
            "distance_inversions": distance_inversions,
            "ascending_iid_pairs": ascending_iid_pairs,
            "descending_iid_pairs": negative_steps,
            "mean_iid_step": mean_step,
            "per_query": documents,
        })
    }
}

// ------------------------------------------------------------------ modes -----

fn run_mode(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(required(arguments, "--out")?);
    let n: usize = required(arguments, "--ns")?.parse()?;
    let seconds: f64 = required(arguments, "--seconds")?.parse()?;
    let ready = optional(arguments, "--ready-file");
    let warmup_passes: usize = optional(arguments, "--warmup-passes")
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(0);
    let prepared = Prepared::build(arguments, n)?;
    let searcher = prepared.opened.searcher()?;
    let order = prepared.order_diagnostics();

    // Warm-up passes: the pages of the candidates of these queries enter the
    // page cache, and no duration of those passes enters the percentiles.
    let mut warmup_pass_ms: Vec<f64> = Vec::with_capacity(warmup_passes);
    for _ in 0..warmup_passes {
        let mut pass: Vec<f64> = Vec::with_capacity(prepared.queries.len());
        for (query, candidates) in prepared.queries.iter().zip(&prepared.candidate_sets) {
            let start = Instant::now();
            let scored = searcher.rescore(query, candidates, candidates.len())?;
            black_box(&scored);
            pass.push(start.elapsed().as_secs_f64() * 1_000.0);
        }
        warmup_pass_ms.push(nearest_rank(&mut pass, 0.50));
    }

    // Opening the searcher verifies the checksum of every vector file, which
    // takes tens of seconds at this scale. A sampler started before that point
    // would sample the opening, so the harness announces the first iteration by
    // creating `--ready-file`: the driver waits for it before it samples.
    if let Some(path) = &ready {
        fs::write(path, b"ready\n")?;
    }

    let budget = Duration::from_secs_f64(seconds);
    let mut durations: Vec<f64> = Vec::new();
    let mut iterations = 0_usize;
    let mut warmup_iterations = 0_usize;
    let started = Instant::now();
    // The warm-up is the time the caller gives the sampler to attach before the
    // per-iteration durations start counting; it is declared in the report.
    let warmup = Duration::from_secs(2);
    'outer: loop {
        for (query, candidates) in prepared.queries.iter().zip(&prepared.candidate_sets) {
            if started.elapsed() >= budget {
                break 'outer;
            }
            let start = Instant::now();
            let scored = searcher.rescore(query, candidates, candidates.len())?;
            let elapsed = start.elapsed();
            black_box(&scored);
            if started.elapsed() < warmup {
                warmup_iterations += 1;
                continue;
            }
            durations.push(elapsed.as_secs_f64() * 1_000.0);
            iterations += 1;
        }
    }
    let elapsed = started.elapsed().as_secs_f64();

    let mut sorted = durations.clone();
    sorted.sort_by(f64::total_cmp);
    let percentile = |fraction: f64| -> Option<f64> {
        if sorted.is_empty() {
            return None;
        }
        // Nearest rank (ADR-0077): ceil(fraction × n), one-based.
        let rank = (fraction * sorted.len() as f64).ceil().max(1.0) as usize;
        Some(sorted[rank.min(sorted.len()) - 1])
    };
    let document = json!({
        "kind": "rescore-profile",
        "commit": commit(),
        "n": n,
        "k": prepared.k,
        "live": prepared.live,
        "dims": prepared.dims,
        "queries": prepared.queries.len(),
        "candidates_min": prepared.candidate_sets.iter().map(|set| set.len()).min(),
        "candidates_max": prepared.candidate_sets.iter().map(|set| set.len()).max(),
        "seconds_requested": seconds,
        "seconds_elapsed": elapsed,
        "warmup_iterations": warmup_iterations,
        "warmup_seconds": 2.0,
        "warmup_passes": warmup_passes,
        "warmup_pass_ms": warmup_pass_ms,
        "iterations": iterations,
        "ms": {
            "p50": percentile(0.50),
            "p95": percentile(0.95),
            "p99": percentile(0.99),
            "min": sorted.first().copied(),
            "max": sorted.last().copied(),
            "mean": if durations.is_empty() { None } else {
                Some(durations.iter().sum::<f64>() / durations.len() as f64)
            },
        },
        "durations_ms": durations,
        "order": order,
    });
    write_json(&out, &document)?;
    println!(
        "rescore_profile run: N = {n}, {} iterations in {elapsed:.1} s, p50 {:.3} ms, p95 {:.3} ms, wrote {}",
        iterations,
        percentile(0.50).unwrap_or(f64::NAN),
        percentile(0.95).unwrap_or(f64::NAN),
        out.display()
    );
    Ok(())
}

fn order_mode(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(required(arguments, "--out")?);
    let n: usize = required(arguments, "--ns")?.parse()?;
    let prepared = Prepared::build(arguments, n)?;
    let order = prepared.order_diagnostics();
    let document = json!({
        "kind": "rescore-order",
        "commit": commit(),
        "n": n,
        "k": prepared.k,
        "live": prepared.live,
        "order": order,
    });
    write_json(&out, &document)?;
    println!(
        "rescore_profile order: N = {n}, {} queries, wrote {}",
        prepared.queries.len(),
        out.display()
    );
    Ok(())
}

// ------------------------------------------------------------------ inputs ---

/// Nearest-rank percentile of an unsorted slice (ADR-0077), in its own unit.
fn nearest_rank(values: &mut [f64], fraction: f64) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    values.sort_by(f64::total_cmp);
    let rank = (fraction * values.len() as f64).ceil().max(1.0) as usize;
    values[rank.min(values.len()) - 1]
}

/// Reads a row-major `f32` matrix of `dims`-component query vectors.
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

fn write_json(path: &Path, document: &Value) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut text = serde_json::to_string_pretty(document)?;
    text.push('\n');
    fs::write(path, text)?;
    Ok(())
}

fn commit() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.trim().to_owned())
        .unwrap_or_default()
}

/// Returns the value following `name`, when present.
fn optional(arguments: &[String], name: &str) -> Option<String> {
    let position = arguments.iter().position(|argument| argument == name)?;
    arguments.get(position + 1).cloned()
}

/// Returns the value following `name`, or an error naming it.
fn required(arguments: &[String], name: &str) -> Result<String, String> {
    optional(arguments, name).ok_or_else(|| format!("missing {name}"))
}
