// SPDX-License-Identifier: Apache-2.0

//! A/B driver of the rescoring step (MES-03, `REDUIRE-COUT-DES-CLES`,
//! `GROUPER-LECTURES-VECTEURS`).
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
//!                    --out O [--queries n] [--candidates-cache F]
//!                    [--cold --vectors-file V] [--displace-gb G
//!                    --displace-source P …] [--idle-seconds S] [--sample-rss]
//! rescore_ab contend --data D --index I --vectors Q --ns N --passes P
//!                    --out O [--queries n] [--candidates-cache F]
//! ```
//!
//! The document holds the parameters, the candidate diagnostics (count,
//! distinct identifiers, identifier order), every duration of every pass in
//! milliseconds, and the per-pass and overall percentiles by nearest rank
//! (ADR-0077). Nothing is filtered or excluded.
//!
//! Three additions serve the campaign of ADR-0080:
//!
//! - **`--candidates-cache`** stores the candidate sets of one `N` on disk and
//!   reuses them. The scan that builds them walks 7,74 M codes and costs about a
//!   second per query on the measurement machine, so recomputing it in every
//!   invocation would dominate the series. The document records a digest of the
//!   identifiers it used, so a report can prove that every variant of a round
//!   scored the same candidates.
//! - **`--cold`** declares the page state of the vector file, request by
//!   request: `mincore` over the pages the candidates of that request touch,
//!   exactly as MES-03 does for its cold series. It needs `--vectors-file`, the
//!   `vectors.f16` of the segment. `--displace-gb` and `--displace-source` read
//!   unrelated files **after the searcher is opened**, which is the series
//!   ADR-0080, decision 6, adds: the blake3 seal check of the opening reads the
//!   whole vector file, so a cold engine cannot be obtained by giving it an
//!   uncached copy. `--idle-seconds` waits that long, after the opening and the
//!   displacement, before the first pass (the 10-minute series of ADR-0077).
//! - **`contend`** measures the contention of the grouped read with the binary
//!   scan that shares the same rayon pool (ADR-0080, "Conséquences" and "À
//!   surveiller"): for every query it times the scan alone, the rescoring alone,
//!   then the two concurrently, each on the `ScanPool` of the production, and
//!   reports the four durations. `--sample-rss` samples the RSS of the process
//!   from a thread of the harness, which is the only way to see the transient
//!   buffer of the grouped read (7,7 MB at N = 5000): it is freed before the
//!   ranking, so it never shows in a state read after a pass.

use std::collections::HashSet;
use std::fs::{self, File};
use std::hint::black_box;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crible_core::config::IndexConfig;
use crible_core::filter::Filter;
use crible_core::ids::Iid;
use crible_search::dense::binary::{self, Candidate};
use crible_search::dense::exact::ExactSearcher;
use crible_search::pool::Pool;
use crible_search::scan_pool::ScanPool;
use crible_store::codes::quantize;
use crible_store::generation::{GenerationStore, Snapshot};
use memmap2::Mmap;
use serde_json::{Value, json};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().collect();
    let mode = arguments.get(1).cloned().ok_or(
        "usage: rescore_ab measure|contend --data D --index I --vectors Q --ns N \
         --passes P --out O",
    )?;
    match mode.as_str() {
        "measure" => measure_mode(&arguments),
        "contend" => contend_mode(&arguments),
        other => Err(format!("unknown mode '{other}': expected 'measure' or 'contend'").into()),
    }
}

/// Everything one invocation needs: the index, the query matrix, the candidate
/// sets and the pool the production searches on.
struct Prepared {
    index: String,
    n: usize,
    k: usize,
    dims: usize,
    live: u32,
    queries: Vec<Vec<f32>>,
    query_codes: Vec<Vec<u64>>,
    candidate_sets: Vec<Vec<Candidate>>,
    wanted: usize,
    cache: Option<PathBuf>,
    digest: String,
    distinct_iids: usize,
    candidate_pairs: usize,
    iid_inversions: usize,
    scan_pool: ScanPool,
    searcher: ExactSearcher,
    snapshot: Arc<Snapshot>,
    pool: Pool,
}

/// Builds the pool, the candidate sets and the searcher of one `N`.
fn prepare(arguments: &[String]) -> Result<Prepared, Box<dyn std::error::Error>> {
    let data = PathBuf::from(required(arguments, "--data")?);
    let index = required(arguments, "--index")?;
    let vectors = PathBuf::from(required(arguments, "--vectors")?);
    let n: usize = required(arguments, "--ns")?.parse()?;
    let query_count: Option<usize> = optional(arguments, "--queries")
        .map(|value| value.parse())
        .transpose()?;
    let cache: Option<PathBuf> = optional(arguments, "--candidates-cache").map(PathBuf::from);

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
    let candidate_sets: Vec<Vec<Candidate>> = match &cache {
        Some(path) if path.exists() => read_candidates(path, n, queries.len())?,
        _ => {
            let mut sets: Vec<Vec<Candidate>> = Vec::with_capacity(queries.len());
            for query_code in &query_codes {
                sets.push(
                    scan_pool.install(|| binary::top_k(&snapshot, &pool, query_code, wanted))?,
                );
            }
            if let Some(path) = &cache {
                write_candidates(path, n, k, queries.len(), dims, &sets)?;
            }
            sets
        }
    };
    // The scan is deterministic for a fixed pool and a fixed `N`, so a cached
    // set is the set the engine would return. An index that changed under the
    // cache would break that, and the count check catches it before the
    // measurement instead of after.
    if candidate_sets.len() != queries.len() {
        return Err(format!(
            "the candidate cache holds {} sets where {} queries are read",
            candidate_sets.len(),
            queries.len()
        )
        .into());
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
    let digest = digest_of(&candidate_sets);

    let segments_directory = store.segments_directory();
    let records_path = store.records_path();
    let segments = snapshot.segments().to_vec();
    let searcher =
        scan_pool.install(|| ExactSearcher::open(&segments_directory, &segments, &records_path))?;

    // Keep the store alive for the whole run: the snapshot holds its epoch, and
    // dropping the store would release the lease the files of that epoch rely
    // on. Boxed so the field is not dead in the eyes of the linter.
    let _store = Box::new(store);

    Ok(Prepared {
        index,
        n,
        k,
        dims,
        live,
        queries,
        query_codes,
        candidate_sets,
        wanted,
        cache,
        digest,
        distinct_iids: distinct.len(),
        candidate_pairs: pairs,
        iid_inversions,
        scan_pool,
        searcher,
        snapshot,
        pool,
    })
}

fn measure_mode(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(required(arguments, "--out")?);
    let passes: usize = required(arguments, "--passes")?.parse()?;
    let prepared = prepare(arguments)?;
    let Prepared {
        index,
        n,
        k,
        dims,
        live,
        queries,
        candidate_sets,
        cache,
        digest,
        distinct_iids,
        candidate_pairs,
        iid_inversions,
        scan_pool,
        searcher,
        ..
    } = prepared;

    let cold = arguments.iter().any(|argument| argument == "--cold");
    let vectors_file = optional(arguments, "--vectors-file").map(PathBuf::from);
    if cold && vectors_file.is_none() {
        return Err("--cold needs --vectors-file, the vectors.f16 of the segment".into());
    }
    let idle_seconds: Option<u64> = optional(arguments, "--idle-seconds")
        .map(|value| value.parse())
        .transpose()?;
    let displace_gb: Option<f64> = optional(arguments, "--displace-gb")
        .map(|value| value.parse())
        .transpose()?;
    let sample_rss = arguments.iter().any(|argument| argument == "--sample-rss");

    // The mapping is opened for `mincore` only: it never feeds a score, so the
    // residency it reports is the residency of the file the searcher reads.
    let residency_map = match (&vectors_file, cold) {
        (Some(path), true) => Some(open_map(path)?),
        _ => None,
    };

    let mut displaced: Option<(f64, u64, Vec<PathBuf>)> = None;
    if let Some(gigabytes) = displace_gb {
        let sources: Vec<PathBuf> = arguments
            .iter()
            .enumerate()
            .filter(|(_, argument)| argument.as_str() == "--displace-source")
            .filter_map(|(position, _)| arguments.get(position + 1).map(PathBuf::from))
            .collect();
        if sources.is_empty() {
            return Err("--displace-gb needs at least one --displace-source".into());
        }
        let read = displace_cache(gigabytes, &sources)?;
        displaced = Some((gigabytes, read, sources));
    }
    if let Some(seconds) = idle_seconds {
        println!("idle: waiting {seconds} s before the first pass");
        std::thread::sleep(Duration::from_secs(seconds));
    }

    let sampler = if sample_rss {
        Some(RssSampler::start(Duration::from_millis(100)))
    } else {
        None
    };

    let mut pass_durations: Vec<Vec<f64>> = Vec::with_capacity(passes);
    let mut returned_lengths: Vec<usize> = Vec::with_capacity(queries.len() * passes);
    let mut resident_fractions: Vec<f64> = Vec::with_capacity(queries.len() * passes);
    let mut touched_pages: Vec<usize> = Vec::with_capacity(queries.len() * passes);
    for pass in 0..passes {
        let mut durations = Vec::with_capacity(queries.len());
        for (query, candidates) in queries.iter().zip(&candidate_sets) {
            if let Some(map) = &residency_map {
                let iids: Vec<u32> = candidates
                    .iter()
                    .map(|candidate| candidate.iid.get())
                    .collect();
                let pages = pages_of_iids(&iids, dims);
                let resident = resident_pages(map, &pages)?;
                resident_fractions.push(resident as f64 / pages.len() as f64);
                touched_pages.push(pages.len());
            }
            let started = Instant::now();
            let scored = scan_pool.install(|| searcher.rescore(query, candidates, k))?;
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

    let rss = sampler.map(RssSampler::stop);
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
        "distinct_iids": distinct_iids,
        "candidate_pairs": candidate_pairs,
        "iid_inversions": iid_inversions,
        "candidates_cache": cache.map(|path| path.display().to_string()),
        "candidates_digest": digest,
        "returned_min": returned_lengths.iter().copied().min(),
        "returned_max": returned_lengths.iter().copied().max(),
        "cold": cold,
        "vectors_file": vectors_file.as_ref().map(|path| path.display().to_string()),
        "resident_fraction": if resident_fractions.is_empty() {
            Value::Null
        } else {
            json!({
                "first": resident_fractions[0],
                "mean": resident_fractions.iter().sum::<f64>() / resident_fractions.len() as f64,
                "min": resident_fractions.iter().copied().fold(f64::INFINITY, f64::min),
                "max": resident_fractions.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                "per_request": resident_fractions,
                "pages_mean": touched_pages.iter().sum::<usize>() as f64 / touched_pages.len() as f64,
            })
        },
        "displace_gb": displaced.as_ref().map(|(target, _, _)| *target),
        "displaced_bytes": displaced.as_ref().map(|(_, read, _)| *read),
        "displace_sources": displaced.as_ref().map(|(_, _, sources)| {
            sources.iter().map(|path| path.display().to_string()).collect::<Vec<_>>()
        }),
        "idle_seconds": idle_seconds,
        "rss": rss,
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

/// Contention of the rescoring with the binary scan of the shared pool.
///
/// For every query and every pass, three measurements, in this order:
///
/// 1. **the scan alone**: `dense::binary::top_k` on the whole live pool, the
///    work a production request does before the rescoring;
/// 2. **the rescoring alone**: `ExactSearcher::rescore` on the candidates of the
///    query, as `measure` times it;
/// 3. **the two concurrently**: the same scan and the same rescoring, launched
///    in two threads that both install themselves on the `ScanPool`, which is
///    what two concurrent requests of a server do.
///
/// The order is fixed and declared: the scan of step 1 warms the codes (they are
/// in RAM, ADR-0069) and the rescoring of step 2 warms the vectors of that
/// query, so step 3 is measured on a page state at least as favourable as
/// steps 1 and 2. It therefore under-states the contention, never over-states
/// it.
fn contend_mode(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(required(arguments, "--out")?);
    let passes: usize = required(arguments, "--passes")?.parse()?;
    let prepared = prepare(arguments)?;
    let Prepared {
        index,
        n,
        k,
        dims,
        live,
        queries,
        query_codes,
        candidate_sets,
        wanted,
        cache,
        digest,
        scan_pool,
        searcher,
        snapshot,
        pool,
        ..
    } = prepared;

    let mut per_pass = Vec::with_capacity(passes);
    let mut scans_alone: Vec<f64> = Vec::new();
    let mut rescores_alone: Vec<f64> = Vec::new();
    let mut scans_concurrent: Vec<f64> = Vec::new();
    let mut rescores_concurrent: Vec<f64> = Vec::new();
    let mut same_candidates = true;
    for pass in 0..passes {
        let mut scans_alone_pass = Vec::with_capacity(queries.len());
        let mut rescores_alone_pass = Vec::with_capacity(queries.len());
        let mut scans_concurrent_pass = Vec::with_capacity(queries.len());
        let mut rescores_concurrent_pass = Vec::with_capacity(queries.len());
        for ((query, query_code), candidates) in
            queries.iter().zip(&query_codes).zip(&candidate_sets)
        {
            let started = Instant::now();
            let scan = scan_pool.install(|| binary::top_k(&snapshot, &pool, query_code, wanted))?;
            scans_alone_pass.push(started.elapsed().as_secs_f64() * 1_000.0);
            if scan.len() != candidates.len() {
                same_candidates = false;
            }

            let started = Instant::now();
            let scored = scan_pool.install(|| searcher.rescore(query, candidates, k))?;
            rescores_alone_pass.push(started.elapsed().as_secs_f64() * 1_000.0);
            black_box(scored.len());

            let (scan_us, rescore_us) = std::thread::scope(|scope| {
                let scan = scope.spawn(|| {
                    let started = Instant::now();
                    let result =
                        scan_pool.install(|| binary::top_k(&snapshot, &pool, query_code, wanted));
                    (started.elapsed().as_secs_f64() * 1_000.0, result)
                });
                let rescore = scope.spawn(|| {
                    let started = Instant::now();
                    let result = scan_pool.install(|| searcher.rescore(query, candidates, k));
                    (started.elapsed().as_secs_f64() * 1_000.0, result)
                });
                let scan = scan.join().map_err(|_| {
                    Box::<dyn std::error::Error + Send + Sync>::from("the scan thread panicked")
                })?;
                let rescore = rescore.join().map_err(|_| {
                    Box::<dyn std::error::Error + Send + Sync>::from(
                        "the rescoring thread panicked",
                    )
                })?;
                let scan_set = scan.1?;
                let scored = rescore.1?;
                black_box(scored.len());
                if scan_set.len() != candidates.len() {
                    return Err(Box::<dyn std::error::Error + Send + Sync>::from(
                        "the concurrent scan returned another count",
                    ));
                }
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>((scan.0, rescore.0))
            })
            .map_err(|error| error.to_string())?;
            scans_concurrent_pass.push(scan_us);
            rescores_concurrent_pass.push(rescore_us);
        }
        scans_alone.extend(scans_alone_pass.iter().copied());
        rescores_alone.extend(rescores_alone_pass.iter().copied());
        scans_concurrent.extend(scans_concurrent_pass.iter().copied());
        rescores_concurrent.extend(rescores_concurrent_pass.iter().copied());
        let mut scan_sorted = scans_concurrent_pass.clone();
        let mut rescore_sorted = rescores_concurrent_pass.clone();
        println!(
            "[{}] contend pass {} of N = {n}: scan alone p50 {:.3}, scan under load p50 {:.3}, \
             rescore alone p50 {:.3}, rescore under scan p50 {:.3}",
            index,
            pass,
            nearest_rank(&mut scans_alone_pass.clone(), 0.50),
            nearest_rank(&mut scan_sorted, 0.50),
            nearest_rank(&mut rescores_alone_pass.clone(), 0.50),
            nearest_rank(&mut rescore_sorted, 0.50)
        );
        per_pass.push(json!({
            "scan_alone": summarise(&scans_alone_pass),
            "scan_concurrent": summarise(&scans_concurrent_pass),
            "rescore_alone": summarise(&rescores_alone_pass),
            "rescore_concurrent": summarise(&rescores_concurrent_pass),
        }));
    }

    let document = json!({
        "kind": "rescore-ab-contend",
        "index": index,
        "dims": dims,
        "live": live,
        "k": k,
        "n": n,
        "queries": queries.len(),
        "passes": passes,
        "candidates_cache": cache.map(|path| path.display().to_string()),
        "candidates_digest": digest,
        "scan_matches_candidates": same_candidates,
        "scan_alone": summarise(&scans_alone),
        "scan_concurrent": summarise(&scans_concurrent),
        "rescore_alone": summarise(&rescores_alone),
        "rescore_concurrent": summarise(&rescores_concurrent),
        "per_pass": per_pass,
    });
    fs::write(&out, serde_json::to_vec(&document)?)?;
    println!(
        "rescore_ab contend: N = {n}, {} passes, wrote {}",
        passes,
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

/// Percentiles and extremes of one series of durations, in milliseconds.
fn summarise(values: &[f64]) -> Value {
    let mut sorted = values.to_vec();
    json!({
        "p50": nearest_rank(&mut sorted.clone(), 0.50),
        "p95": nearest_rank(&mut sorted.clone(), 0.95),
        "p99": nearest_rank(&mut sorted, 0.99),
        "min": values.iter().copied().fold(f64::INFINITY, f64::min),
        "max": values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        "n": values.len(),
    })
}

/// Digest of the identifier sequence the variants scored.
///
/// FNV-1a 64 over the little-endian bytes of every identifier, in the arrival
/// order of the candidates. Two variants of a round must print the same digest;
/// a difference means they did not score the same list and the comparison of
/// their durations would be meaningless.
fn digest_of(sets: &[Vec<Candidate>]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for set in sets {
        for candidate in set {
            for byte in candidate.iid.get().to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    format!("{hash:016x}")
}

/// Reads the candidate sets a previous invocation stored for the same `N`.
fn read_candidates(
    path: &Path,
    n: usize,
    queries: usize,
) -> Result<Vec<Vec<Candidate>>, Box<dyn std::error::Error>> {
    let document: Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    let stored_n = document["n"]
        .as_u64()
        .ok_or("the candidate cache holds no 'n'")?;
    if stored_n as usize != n {
        return Err(format!(
            "the candidate cache of {} holds N = {stored_n}, not {n}",
            path.display()
        )
        .into());
    }
    let sets: Vec<Vec<Candidate>> = document["sets"]
        .as_array()
        .ok_or("the candidate cache holds no 'sets'")?
        .iter()
        .map(|set| {
            set.as_array()
                .ok_or("a cached candidate set is not an array")?
                .iter()
                .map(|pair| {
                    let iid = pair[0]
                        .as_u64()
                        .ok_or("a cached candidate has no identifier")?;
                    let distance = pair[1]
                        .as_u64()
                        .ok_or("a cached candidate has no distance")?;
                    Ok(Candidate {
                        iid: Iid::new(u32::try_from(iid)?),
                        distance,
                    })
                })
                .collect::<Result<Vec<Candidate>, Box<dyn std::error::Error>>>()
        })
        .collect::<Result<Vec<Vec<Candidate>>, Box<dyn std::error::Error>>>()?;
    if sets.len() != queries {
        return Err(format!(
            "the candidate cache of {} holds {} sets where {queries} queries are read",
            path.display(),
            sets.len()
        )
        .into());
    }
    Ok(sets)
}

/// Stores the candidate sets of one `N`, so later invocations skip the scan.
fn write_candidates(
    path: &Path,
    n: usize,
    k: usize,
    queries: usize,
    dims: usize,
    sets: &[Vec<Candidate>],
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let document = json!({
        "kind": "rescore-ab-candidates",
        "n": n,
        "k": k,
        "queries": queries,
        "dims": dims,
        "digest": digest_of(sets),
        "sets": sets
            .iter()
            .map(|set| {
                set.iter()
                    .map(|candidate| vec![u64::from(candidate.iid.get()), candidate.distance])
                    .collect::<Vec<Vec<u64>>>()
            })
            .collect::<Vec<Vec<Vec<u64>>>>(),
    });
    fs::write(path, serde_json::to_vec(&document)?)?;
    Ok(())
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

// ------------------------------------------------------------- residency -----

/// Maps a file read-only, for `mincore` only.
fn open_map(path: &Path) -> Result<Mmap, Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    // SAFETY: the file is opened read-only and the mapping is never written;
    // `MAP_SHARED` with `PROT_READ` reflects the file as the searcher sees it.
    let map = unsafe { Mmap::map(&file)? };
    Ok(map)
}

/// Page size of the machine, asked to the C library.
fn page_size() -> usize {
    // SAFETY: `getpagesize` takes no argument and returns a positive constant.
    unsafe { getpagesize() as usize }
}

/// Resident pages among the page set `pages`, read from `mincore` on `map`.
///
/// The pages are grouped into sorted runs, so one system call covers each
/// contiguous stretch instead of one call per page. The set is deduplicated
/// first: a page two candidates share counts once, in the numerator as in the
/// denominator.
fn resident_pages(map: &Mmap, pages: &[u64]) -> Result<usize, Box<dyn std::error::Error>> {
    let size = page_size();
    let mut sorted = pages.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut resident = 0;
    let mut index = 0;
    while index < sorted.len() {
        let first = sorted[index];
        let mut last = first;
        index += 1;
        while index < sorted.len() && sorted[index] == last + 1 {
            last = sorted[index];
            index += 1;
        }
        let length = usize::try_from(last - first + 1)? * size;
        let offset = usize::try_from(first)? * size;
        let mut vector = vec![0_u8; (last - first + 1) as usize];
        // SAFETY: the range lies inside the mapping, the vector is as long as
        // the range in pages, and `mincore` only writes those bytes.
        let status = unsafe {
            mincore(
                map.as_ptr().add(offset) as *mut core::ffi::c_void,
                length,
                vector.as_mut_ptr(),
            )
        };
        if status != 0 {
            return Err(format!(
                "mincore failed on {} pages at offset {offset}",
                vector.len()
            )
            .into());
        }
        resident += vector.iter().filter(|flag| *flag & 1 == 1).count();
    }
    Ok(resident)
}

/// Pages a set of `f16` rows occupies in a vector file of `dims` dimensions.
///
/// The result holds each page **once**, sorted: a page that several rows of the
/// same query touch counts for one, so the residency fraction is a fraction of
/// distinct pages, not of page touches.
fn pages_of_iids(iids: &[u32], dims: usize) -> Vec<u64> {
    let size = page_size() as u64;
    let row = dims as u64 * 2;
    let mut pages = Vec::with_capacity(iids.len());
    for iid in iids {
        let begin = u64::from(*iid) * row;
        let end = begin + row - 1;
        let first = begin / size;
        let last = end / size;
        for page in first..=last {
            pages.push(page);
        }
    }
    pages.sort_unstable();
    pages.dedup();
    pages
}

// ----------------------------------------------------------- displacement ----

/// Reads unrelated files until `gigabytes` have gone through the page cache.
///
/// The reads fill the unified buffer cache with pages the index does not need,
/// which is what evicts the vectors of the index without touching the index.
/// It runs **after** the searcher is opened, so the blake3 seal check of the
/// opening cannot warm the file again: this is the series ADR-0080, decision 6,
/// adds.
fn displace_cache(gigabytes: f64, sources: &[PathBuf]) -> Result<u64, Box<dyn std::error::Error>> {
    let target = (gigabytes * 1024.0 * 1024.0 * 1024.0) as u64;
    let mut read = 0_u64;
    let mut buffer = vec![0_u8; 8 * 1024 * 1024];
    for source in sources {
        let mut file = File::open(source)?;
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 || read >= target {
                break;
            }
            black_box(buffer[0]);
            read += count as u64;
        }
        if read >= target {
            break;
        }
    }
    println!("displacement: read {read} bytes of unrelated files");
    Ok(read)
}

// ------------------------------------------------------------------- RSS -----

/// Samples the RSS of this process from a thread, so the transient buffer of
/// the grouped read is visible while it exists.
///
/// The sampler asks `ps -o rss= -p <pid>`, the same figure MES-09 uses for the
/// total, every `period`. It measures the **RSS**, which includes the mapped
/// pages of the vector file: the difference between two variants of the same
/// series is what matters, not the absolute value.
struct RssSampler {
    stop: Arc<AtomicBool>,
    samples: Arc<std::sync::Mutex<Vec<u64>>>,
    period: Duration,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl RssSampler {
    fn start(period: Duration) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let samples = Arc::new(std::sync::Mutex::new(Vec::new()));
        let handle = {
            let stop = Arc::clone(&stop);
            let samples = Arc::clone(&samples);
            Some(std::thread::spawn(move || {
                let pid = std::process::id();
                while !stop.load(Ordering::Relaxed) {
                    if let Some(kib) = read_rss(pid)
                        && let Ok(mut guard) = samples.lock()
                    {
                        guard.push(kib / 1024);
                    }
                    std::thread::sleep(period);
                }
            }))
        };
        RssSampler {
            stop,
            samples,
            period,
            handle,
        }
    }

    fn stop(mut self) -> Value {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        let mut values = match self.samples.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        values.sort_unstable();
        let max = values.last().copied().unwrap_or(0);
        let p50 = values.get(values.len() / 2).copied().unwrap_or(0);
        let first = values.first().copied().unwrap_or(0);
        json!({
            "period_ms": self.period.as_millis(),
            "samples": values.len(),
            "first_mib": first,
            "p50_mib": p50,
            "max_mib": max,
            "peak_above_first_mib": max.saturating_sub(first),
        })
    }
}

/// RSS of a process in kibibytes, read from `ps`.
fn read_rss(pid: u32) -> Option<u64> {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

// SAFETY: these are the two C functions of `libc` the harness needs, declared
// with their macOS ABI (`mincore` writes one byte per page of the range it is
// given, and `getpagesize` returns a constant). No other unsafe code touches
// them, and no dependency is added: `libc` is not in `docs/07`.
mod system {
    use core::ffi::c_void;

    unsafe extern "C" {
        pub fn mincore(address: *mut c_void, length: usize, vector: *mut u8) -> i32;
        pub fn getpagesize() -> i32;
    }
}

use system::{getpagesize, mincore};

fn required(arguments: &[String], name: &str) -> Result<String, String> {
    optional(arguments, name).ok_or_else(|| format!("missing {name}"))
}

fn optional(arguments: &[String], name: &str) -> Option<String> {
    let position = arguments.iter().position(|argument| argument == name)?;
    arguments.get(position + 1).cloned()
}
