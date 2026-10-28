// SPDX-License-Identifier: Apache-2.0

//! Measurement driver of MES-02, MES-03 and MES-09 (dense search latency and
//! RSS on `synth-large`).
//!
//! References: `docs/08-invariants-et-tests.md` (MES-02, MES-03, MES-09),
//! `docs/phases/P4-recherche-rapide.md` (`MESURER-LATENCE-DENSE`), **ADR-0077**
//! (protocol: N ∈ {1000, 2000, 5000}, pools built by the harness at 1 k, 10 k,
//! 100 k, 1 M and all live passages, contiguous and dispersed, cold measured by
//! uncached copies with `mincore`), ADR-0071 (removed by ADR-0081: the scan has
//! a single traversal), ADR-0081 (a single sparse traversal of the scan and the
//! `cross`/`agree` campaign), ADR-0082 (the segment cursor and the A/B
//! verification), ADR-0074 (strategy and `exact_threshold`), ADR-0072 (one dot
//! product for the exact strategy and the rescoring).
//!
//! This example is a harness, not production code, and it changes nothing in
//! `crates/*/src/`. It exists because the engine exposes no way to build a
//! `Pool` from an arbitrary bitmap: `Pool::evaluate` is the only constructor
//! and it takes a filter. ADR-0077, decision 3, therefore allows the harness to
//! **replicate** the scan (precedent MES-13) instead of changing the public
//! API; the replica is anchored to the engine by the `compare` mode, which runs
//! the same pool through the engine and the replica and compares the top-k.
//!
//! Modes:
//!
//! ```text
//! dense_latency scan      --data D --index I --vectors Q --out O [--sizes …]
//!                         [--shapes contiguous,dispersed] [--ns 1000,2000,5000]
//!                         [--passes 3] [--seed S]
//! dense_latency cross     --data D --index I --vectors Q --out O [--pairs 3]
//!                         [--ns 5000] [--seed S] [--sizes 1000000,all]
//!                         [--densities 10,25,40,50,60,75]
//!                         [--spans 1000000,7740000]
//!                         (single traversal: `--pairs P` is P passes, ADR-0081)
//! dense_latency agree     --data D --index I --vectors Q --out O [--queries 5]
//!                         [--ns 5000] [--seed S] [--sizes …] [--densities …]
//!                         [--spans …]
//! dense_latency rescore   --data D --index I --vectors Q --out O --ns … [--passes 3]
//!                         [--cold] [--displace-gb G] [--displace-source P …]
//!                         [--dot-only] [--dot-vectors-file F]
//! dense_latency exact     --data D --index I --vectors Q --out O [--sizes …]
//!                         [--passes 3] [--pool-size N]
//! dense_latency compare   --data D --index I --vectors Q --out O [--filter F]
//! dense_latency rss       --data D --index I --vectors Q --out O [--passes 1]
//!                         [--model DIR] [--skip-pass]
//! dense_latency copy-nocache --src S --dst D
//! ```
//!
//! The replica of the scan follows `crates/crible-search/src/dense/binary.rs`
//! line for line: the same windows (`dense::windows`), the same single sparse
//! traversal with its segment cursor (ADR-0081, ADR-0082), the same bounded heap
//! and the same `kernels::hamming_function`.
//! The replica of the exact scoring follows `DenseSearcher::score_slice`: one
//! bounded heap per window, the production dot product (`kernels::dot` for a
//! stored `f16` row), no key read, since a random corpus produces no exact tie
//! and the harness reports the ties it met.

use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crible_core::config::IndexConfig;
use crible_core::filter::Filter;
use crible_core::ids::Iid;
use crible_search::dense::binary::Candidate;
use crible_search::dense::exact::DenseSearcher;
use crible_search::dense::{Strategy, plan, search as dense_search};
use crible_search::kernels;
use crible_search::pool::Pool;
use crible_search::scan_pool::ScanPool;
use crible_store::codes::quantize;
use crible_store::generation::GenerationStore;
use half::f16;
use memmap2::Mmap;
use rayon::prelude::*;
use roaring::RoaringBitmap;
use rusqlite::OptionalExtension;
use serde_json::{Value, json};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().collect();
    let mode = arguments.get(1).cloned().ok_or(
        "usage: dense_latency <scan|cross|agree|rescore|exact|compare|rss|copy-nocache> …",
    )?;
    match mode.as_str() {
        "copy-nocache" => {
            let source = PathBuf::from(required(&arguments, "--src")?);
            let destination = PathBuf::from(required(&arguments, "--dst")?);
            let bytes = copy_nocache(&source, &destination)?;
            println!("{bytes} bytes copied without caching");
        }
        _ => {
            let data = PathBuf::from(required(&arguments, "--data")?);
            let index = required(&arguments, "--index")?;
            let out = PathBuf::from(required(&arguments, "--out")?);
            let opened = Opened::open(&data, &index)?;
            match mode.as_str() {
                "scan" => scan_mode(&arguments, &opened, &out)?,
                "cross" => cross_mode(&arguments, &opened, &out)?,
                "agree" => agree_mode(&arguments, &opened, &out)?,
                "rescore" => rescore_mode(&arguments, &opened, &out)?,
                "exact" => exact_mode(&arguments, &opened, &out)?,
                "compare" => compare_mode(&arguments, &opened, &out)?,
                "rss" => rss_mode(&arguments, &opened, &out)?,
                other => return Err(format!("unknown mode '{other}'").into()),
            }
        }
    }
    Ok(())
}

/// One measurement index, opened once per process.
struct Opened {
    /// Name of the index, as declared in its `index.json`.
    index: String,
    /// Search settings of the measurement copy.
    config: IndexConfig,
    /// Generation store: the snapshot, the bitmaps and the binary codes.
    store: GenerationStore,
    /// Absolute path of the vectors directory of the epoch.
    segments_directory: PathBuf,
    /// Absolute path of the `records.sqlite` of the epoch.
    records_path: PathBuf,
    /// Worker count of the scan pool the campaign uses.
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
        let pool = ScanPool::new(0)?;
        let threads = pool.threads();
        Ok(Self {
            index: index.to_owned(),
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
    fn searcher(&self) -> Result<DenseSearcher, Box<dyn std::error::Error>> {
        let pool = ScanPool::new(self.threads)?;
        let segments = self.snapshot().segments().to_vec();
        let directory = self.segments_directory.clone();
        let records = self.records_path.clone();
        let searcher = pool.install(|| DenseSearcher::open(&directory, &segments, &records))?;
        Ok(searcher)
    }

    /// Path of the vector file of the single-segment measurement index.
    fn vectors_path(&self) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let snapshot = self.snapshot();
        let meta = snapshot
            .segments()
            .first()
            .ok_or("the index holds no segment")?;
        let directory = self.segments_directory.join(&meta.segment_id);
        let name = match meta.precision {
            crible_core::config::ReferencePrecision::Fp16 => "vectors.f16",
            crible_core::config::ReferencePrecision::Fp32 => "vectors.f32",
        };
        Ok(directory.join(name))
    }
}

// ---------------------------------------------------------------- scan -------

/// Replica of one segment of binary codes, as `binary::prepare` builds it.
struct Codes<'a> {
    iid_start: u32,
    count: u32,
    codes: &'a [u64],
}

impl Codes<'_> {
    fn end(&self) -> u64 {
        u64::from(self.iid_start) + u64::from(self.count)
    }

    fn code(&self, iid: u32, words: usize) -> Option<&[u64]> {
        let offset = usize::try_from(iid.checked_sub(self.iid_start)?).ok()?;
        let begin = offset.checked_mul(words)?;
        let end = begin.checked_add(words)?;
        self.codes.get(begin..end)
    }
}

/// Replica of `binary::SegmentCursor`: the segment of each live identifier,
/// followed by a cursor that only moves forward (ADR-0081, ADR-0082).
struct SegmentCursor<'a> {
    segments: &'a [Codes<'a>],
    position: Option<usize>,
}

impl<'a> SegmentCursor<'a> {
    /// Positions the cursor on the last segment that begins at or before `iid`.
    fn before(segments: &'a [Codes<'a>], iid: u32) -> Self {
        let position = segments
            .partition_point(|segment| segment.iid_start <= iid)
            .checked_sub(1);
        Self { segments, position }
    }

    /// Segment holding `iid`, or `None` when no segment covers it.
    fn segment_of(&mut self, iid: u32) -> Option<&'a Codes<'a>> {
        let segments = self.segments;
        loop {
            let index = self.position?;
            let segment = &segments[index];
            if u64::from(iid) < segment.end() {
                return (u64::from(iid) >= u64::from(segment.iid_start)).then_some(segment);
            }
            self.position = index.checked_add(1).filter(|next| *next < segments.len());
        }
    }
}

/// Replica of `dense::windows`: one contiguous range of live identifiers per
/// worker, read from the sorted bitmap.
fn windows(bitmap: &RoaringBitmap, total: u32, threads: usize) -> Vec<(u32, u32)> {
    let total = usize::try_from(total).unwrap_or(usize::MAX);
    if total == 0 {
        return Vec::new();
    }
    let width = total.div_ceil(threads.max(1)).max(1);
    let mut starts = Vec::with_capacity(threads.max(1));
    let mut last = 0_u32;
    for (position, iid) in bitmap.iter().enumerate() {
        if position % width == 0 {
            starts.push(iid);
        }
        last = iid;
    }
    let mut windows: Vec<(u32, u32)> = Vec::with_capacity(starts.len());
    for (position, &from) in starts.iter().enumerate() {
        let to = match starts.get(position + 1) {
            Some(&next) => next - 1,
            None => last,
        };
        windows.push((from, to));
    }
    windows
}

/// Replica of `binary::Scan`: one window, one bounded heap.
struct Scan<'a> {
    segments: &'a [Codes<'a>],
    words: usize,
    query_code: &'a [u64],
    n: usize,
    hamming: fn(&[u64], &[u64]) -> u64,
}

/// One scored passage of the exact replica: the production order, descending
/// score then ascending identifier (`Scored` of `dense/mod.rs`, key omitted).
#[derive(Debug, Clone, Copy, PartialEq)]
struct ScoreIid {
    score: f32,
    iid: u32,
}

impl Eq for ScoreIid {}

impl Ord for ScoreIid {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score
            .total_cmp(&other.score)
            .then_with(|| other.iid.cmp(&self.iid))
    }
}

impl PartialOrd for ScoreIid {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Replica of `binary::offer`.
fn offer(heap: &mut BinaryHeap<Reverse<Candidate>>, distance: u64, iid: u32, n: usize) {
    let candidate = Candidate {
        iid: Iid::new(iid),
        distance,
    };
    if heap.len() == n
        && let Some(worst) = heap.peek()
        && candidate.cmp(&worst.0) == Ordering::Less
    {
        return;
    }
    heap.push(Reverse(candidate));
    if heap.len() > n {
        heap.pop();
    }
}

impl Scan<'_> {
    fn window(&self, pool: &RoaringBitmap, from: u32, to: u32) -> Vec<Candidate> {
        let mut heap: BinaryHeap<Reverse<Candidate>> = BinaryHeap::with_capacity(
            self.n
                .min(usize::try_from(pool.len()).unwrap_or(usize::MAX)),
        );
        let mut live = pool.iter();
        live.advance_to(from);
        let mut cursor = SegmentCursor::before(self.segments, from);
        for iid in live {
            if iid > to {
                break;
            }
            let Some(segment) = cursor.segment_of(iid) else {
                continue;
            };
            let Some(code) = segment.code(iid, self.words) else {
                continue;
            };
            offer(
                &mut heap,
                (self.hamming)(self.query_code, code),
                iid,
                self.n,
            );
        }
        heap.into_iter()
            .map(|Reverse(candidate)| candidate)
            .collect()
    }
}

/// Replica of `binary::top_k`: the windows in parallel, then the merge.
fn scan_replica(
    segments: &[Codes<'_>],
    pool: &RoaringBitmap,
    pool_len: u32,
    query_code: &[u64],
    n: usize,
) -> Vec<Candidate> {
    if n == 0 || pool.is_empty() {
        return Vec::new();
    }
    let scan = Scan {
        segments,
        words: query_code.len(),
        query_code,
        n,
        hamming: kernels::hamming_function(),
    };
    let ranges = windows(pool, pool_len, rayon::current_num_threads().max(1));
    let heaps: Vec<Vec<Candidate>> = ranges
        .par_iter()
        .map(|&(from, to)| scan.window(pool, from, to))
        .collect();
    let mut candidates: Vec<Candidate> = heaps.into_iter().flatten().collect();
    candidates.sort_unstable_by(|left, right| right.cmp(left));
    candidates.truncate(n);
    candidates
}

/// Codes of the generation, ready for the replica.
fn codes_of<'a>(snapshot: &'a crible_store::generation::Snapshot) -> Vec<Codes<'a>> {
    let metas = snapshot.segments();
    let mut codes = Vec::with_capacity(metas.len());
    for meta in metas {
        if let Some(words) = snapshot.codes_of(&meta.segment_id) {
            codes.push(Codes {
                iid_start: meta.iid_start.get(),
                count: meta.count,
                codes: words,
            });
        }
    }
    codes
}

// --------------------------------------------------------------- pools -------

/// Shape of a constructed pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Contiguous,
    Dispersed,
    /// Every live passage: the two shapes coincide.
    Full,
}

impl Shape {
    fn as_str(self) -> &'static str {
        match self {
            Self::Contiguous => "contiguous",
            Self::Dispersed => "dispersed",
            Self::Full => "full",
        }
    }
}

/// `size` consecutive live identifiers, from a fixed fraction of the live set.
fn pool_contiguous(live: &RoaringBitmap, size: usize) -> RoaringBitmap {
    let total = usize::try_from(live.len()).unwrap_or(0);
    let size = size.min(total);
    let start = (total / 3).min(total.saturating_sub(size));
    let mut pool = RoaringBitmap::new();
    for (position, iid) in live.iter().enumerate() {
        if position < start {
            continue;
        }
        if position >= start + size {
            break;
        }
        pool.insert(iid);
    }
    pool
}

/// `size` live identifiers drawn from a fixed seed, sorted.
fn pool_dispersed(live: &RoaringBitmap, size: usize, seed: u64) -> RoaringBitmap {
    let total = usize::try_from(live.len()).unwrap_or(0);
    let size = size.min(total);
    let mut draw = Draw::new(seed);
    let mut chosen: HashSet<usize> = HashSet::with_capacity(size);
    while chosen.len() < size {
        let position = usize::try_from(draw.next() % total as u64).unwrap_or(0);
        chosen.insert(position);
    }
    let mut pool = RoaringBitmap::new();
    for (position, iid) in live.iter().enumerate() {
        if chosen.contains(&position) {
            pool.insert(iid);
        }
    }
    pool
}

/// Deterministic generator shared by the constructed pools.
struct Draw {
    state: u64,
}

impl Draw {
    fn new(seed: u64) -> Self {
        Self { state: seed | 1 }
    }

    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

/// A pool drawn in one window of identifiers, at a given density.
///
/// The density ADR-0071 defined as `pool size / iid span` is still the label of
/// these pools, although ADR-0081 removed the traversal rule that read it.
/// MES-02 only produced pools at the two ends (a contiguous window of live
/// passages, about 90 %, and a draw over the whole live set, about 0.1 %), so
/// the density band that the rule separated was never measured. This builder
/// fills it: it takes `fraction` of the identifiers of a window of `span`
/// identifiers, chosen among the live ones, uniformly.
///
/// `synth-large` holds about 90 % live passages, so a fraction above that is not
/// constructible in any window; the pool then takes every live passage of the
/// window and the caller reports the density it actually reached.
fn pool_window(live: &[u32], span: usize, fraction: f64, seed: u64) -> RoaringBitmap {
    let total = live.len();
    let last_live = live.last().copied().unwrap_or(0);
    let limit = usize::try_from(last_live).unwrap_or(0) + 1;
    let span = span.clamp(1, limit);
    let max_start = limit - span;
    let mut draw = Draw::new(seed);
    let mut candidates = vec![0_usize, max_start];
    for _ in 0..8 {
        candidates.push(usize::try_from(draw.next() % (max_start as u64 + 1)).unwrap_or(0));
    }
    let window = |start: usize| -> (usize, usize) {
        let lo = live.partition_point(|iid| usize::try_from(*iid).unwrap_or(usize::MAX) < start);
        let hi =
            live.partition_point(|iid| usize::try_from(*iid).unwrap_or(usize::MAX) < start + span);
        (lo, hi)
    };
    let wanted = ((fraction * span as f64).round() as usize).clamp(1, total.max(1));
    // The first candidate window that holds enough live passages, so a draw is
    // never short of its target density; the widest window is the fallback.
    let start = candidates
        .iter()
        .copied()
        .find(|start| window(*start).1 - window(*start).0 >= wanted)
        .or_else(|| {
            candidates.into_iter().max_by_key(|start| {
                let (lo, hi) = window(*start);
                hi - lo
            })
        })
        .unwrap_or(0);
    let (lo, hi) = window(start);
    let needed = wanted.min(hi - lo);
    let mut pool = RoaringBitmap::new();
    let mut selected = 0_usize;
    let mut slots = hi - lo;
    for iid in live.iter().take(hi).skip(lo) {
        let remaining = needed - selected;
        if remaining == 0 {
            break;
        }
        if draw.next() % (slots as u64) < remaining as u64 {
            pool.insert(*iid);
            selected += 1;
        }
        slots -= 1;
    }
    pool
}

/// Builds the pool of one point.
fn build_pool(live: &RoaringBitmap, size: usize, shape: Shape, seed: u64) -> RoaringBitmap {
    match shape {
        Shape::Contiguous => pool_contiguous(live, size),
        Shape::Dispersed => pool_dispersed(live, size, seed),
        Shape::Full => live.clone(),
    }
}

// ------------------------------------------------------------ statistics -----

/// Nearest-rank percentile of a sorted slice, as every other measure of the
/// repository computes it.
fn percentile(sorted: &[f64], ratio: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = (ratio * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// p50, p95, p99, median, maximum and count of one series.
///
/// A series that carries `NaN` (the engine rescoring was skipped) summarises to
/// `null`, so a missing figure is never read as a fast one.
fn summarise(values: &[f64]) -> Value {
    let values: Vec<f64> = values
        .iter()
        .copied()
        .filter(|value| !value.is_nan())
        .collect();
    if values.is_empty() {
        return Value::Null;
    }
    let mut sorted = values;
    sorted.sort_by(|left, right| left.total_cmp(right));
    json!({
        "n": sorted.len(),
        "p50": percentile(&sorted, 0.50),
        "p95": percentile(&sorted, 0.95),
        "p99": percentile(&sorted, 0.99),
        "median": percentile(&sorted, 0.50),
        "max": sorted.last().copied(),
        "min": sorted.first().copied(),
        "mean": sorted.iter().sum::<f64>() / sorted.len() as f64,
    })
}

/// Summaries of each pass of a series, in the order they ran.
fn per_pass(durations: &[f64], passes: usize, queries: usize) -> Value {
    let mut summaries = Vec::with_capacity(passes);
    for pass in 0..passes {
        let start = pass * queries;
        let end = (start + queries).min(durations.len());
        if start < end {
            summaries.push(summarise(&durations[start..end]));
        }
    }
    Value::Array(summaries)
}

// ---------------------------------------------------------------- vectors ----

/// Query matrix: one row of `dims` `f32` per query, little-endian.
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

/// Row-major `f16` vectors of one segment, mapped read-only.
struct FlatVectors {
    map: Mmap,
    dims: usize,
}

impl FlatVectors {
    fn open(path: &Path, dims: usize) -> Result<Self, Box<dyn std::error::Error>> {
        let file = File::open(path)?;
        // SAFETY: the file is opened read-only and the harness never writes it
        // while the mapping is alive; a measurement copy is written before it
        // is mapped.
        let map = unsafe { Mmap::map(&file)? };
        Ok(Self { map, dims })
    }

    fn row(&self, iid: u32) -> Option<&[u8]> {
        let begin = usize::try_from(iid)
            .ok()?
            .checked_mul(self.dims)?
            .checked_mul(2)?;
        self.map.get(begin..begin + self.dims * 2)
    }

    /// Dot product of one stored `f16` row and the normalised query, through the
    /// production kernel (ADR-0070, ADR-0072).
    ///
    /// `buffer` is reused across candidates, so the allocation is paid once per
    /// query, but **every row is copied** into it before the kernel runs, whereas
    /// production hands the kernel the mmap slice itself (`DenseSearcher`). The
    /// replica is therefore pessimistic: it pays one 1 536-byte copy per
    /// candidate, which the report declares rather than hides.
    fn dot_with(&self, iid: u32, query: &[f32], buffer: &mut Vec<f16>) -> Option<f32> {
        let row = self.row(iid)?;
        let (pairs, _) = row.as_chunks::<2>();
        buffer.clear();
        buffer.extend(
            pairs
                .iter()
                .map(|pair| f16::from_bits(u16::from_le_bytes(*pair))),
        );
        Some(kernels::dot(buffer, query))
    }
}

// ------------------------------------------------------------- residency -----

/// Page size of the machine, asked to the C library.
fn page_size() -> usize {
    // SAFETY: `getpagesize` takes no argument and returns a positive constant.
    unsafe { getpagesize() as usize }
}

/// Resident pages among the page set `pages`, read from `mincore` on `map`.
///
/// The pages are grouped into sorted runs, so one system call covers each
/// contiguous stretch instead of one call per page.
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

// ------------------------------------------------------------ uncached copy --

/// Copies `source` to `destination` with `F_NOCACHE` on both descriptors, so
/// neither the read nor the write leaves pages in the unified buffer cache.
fn copy_nocache(source: &Path, destination: &Path) -> Result<u64, Box<dyn std::error::Error>> {
    let mut input = File::open(source)?;
    let mut output = File::create(destination)?;
    // SAFETY: `fcntl` takes a valid descriptor and one integer argument.
    unsafe {
        if fcntl(std::os::fd::AsRawFd::as_raw_fd(&input), F_NOCACHE, 1) == -1 {
            return Err("F_NOCACHE refused on the source".into());
        }
        if fcntl(std::os::fd::AsRawFd::as_raw_fd(&output), F_NOCACHE, 1) == -1 {
            return Err("F_NOCACHE refused on the destination".into());
        }
    }
    let mut buffer = vec![0_u8; 8 * 1024 * 1024];
    let mut copied = 0_u64;
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
        copied += read as u64;
    }
    output.sync_all()?;
    Ok(copied)
}

// SAFETY: these are the four C functions of `libc` the harness needs, declared
// with their macOS ABI (`F_NOCACHE` is 48 on macOS, and `mincore` writes one
// byte per page of the range it is given). No other unsafe code touches them.
#[allow(dead_code)]
mod system {
    use core::ffi::c_void;

    pub const F_NOCACHE: i32 = 48;

    unsafe extern "C" {
        pub fn mincore(address: *mut c_void, length: usize, vector: *mut u8) -> i32;
        pub fn fcntl(descriptor: i32, command: i32, ...) -> i32;
        pub fn getpagesize() -> i32;
    }
}

use system::{F_NOCACHE, fcntl, getpagesize, mincore};

// ----------------------------------------------------------------- scan ------

fn scan_mode(
    arguments: &[String],
    opened: &Opened,
    out: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let vectors_path = PathBuf::from(required(arguments, "--vectors")?);
    let passes: usize = optional(arguments, "--passes")
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(3);
    let seed: u64 = optional(arguments, "--seed")
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(2026);
    let sizes =
        optional(arguments, "--sizes").unwrap_or_else(|| "1000,10000,100000,1000000,all".into());
    let shapes = optional(arguments, "--shapes").unwrap_or_else(|| "contiguous,dispersed".into());
    let ns = optional(arguments, "--ns").unwrap_or_else(|| "1000,2000,5000".into());

    let snapshot = opened.snapshot();
    let metas = snapshot.segments();
    let dims = metas.first().ok_or("the index holds no segment")?.dims as usize;
    let queries = read_matrix(&vectors_path, dims)?;
    let live = snapshot.filters().live().clone();
    let live_len = usize::try_from(live.len())?;
    let codes = codes_of(&snapshot);
    let query_codes: Vec<Vec<u64>> = queries.iter().map(|query| quantize(query)).collect();
    let k = opened.config.search.candidates_per_branch as usize;

    let scan_pool = ScanPool::new(opened.threads)?;
    let mut points = Vec::new();
    for size_text in sizes.split(',') {
        let (size, shape_list): (usize, Vec<Shape>) = if size_text.trim() == "all" {
            (live_len, vec![Shape::Full])
        } else {
            let size: usize = size_text.trim().parse()?;
            let shapes: Vec<Shape> = shapes
                .split(',')
                .filter(|text| !text.trim().is_empty())
                .map(|text| match text.trim() {
                    "contiguous" => Ok(Shape::Contiguous),
                    "dispersed" => Ok(Shape::Dispersed),
                    other => Err(format!("unknown shape '{other}'")),
                })
                .collect::<Result<Vec<_>, String>>()?;
            (size, shapes)
        };
        for shape in shape_list {
            let pool = build_pool(&live, size, shape, seed);
            let pool_len = pool.len() as u32;
            let planned = plan(&opened.config.search, pool_len, k, false);
            let (first, last) = match (pool.min(), pool.max()) {
                (Some(first), Some(last)) => (first, last),
                _ => (0, 0),
            };
            for n_text in ns.split(',') {
                let n: usize = n_text.trim().parse()?;
                let mut durations = Vec::with_capacity(queries.len() * passes);
                for _ in 0..passes {
                    for query_code in &query_codes {
                        let started = Instant::now();
                        let candidates = scan_pool.install(|| {
                            scan_replica(&codes, &pool, pool_len, query_code, n.max(k))
                        });
                        let micros = started.elapsed().as_secs_f64() * 1e6;
                        durations.push(micros);
                        std::hint::black_box(candidates.len());
                    }
                }
                points.push(json!({
                    "size": pool_len,
                    "requested_size": size,
                    "shape": shape.as_str(),
                    "n": n.max(k),
                    "traversal": "sparse",
                    "plan": planned.strategy.as_str(),
                    "rescore_candidates": planned.rescore_candidates,
                    "first_iid": first,
                    "last_iid": last,
                    "span": u64::from(last) - u64::from(first) + 1,
                    "passes": passes,
                    "queries": queries.len(),
                    "durations_us": durations,
                    "summary": summarise(&durations),
                    "pass_summaries": per_pass(&durations, passes, queries.len()),
                }));
            }
        }
    }

    let report = json!({
        "kind": "scan",
        "index": opened.index,
        "dims": dims,
        "threads": opened.threads,
        "live_passages": live_len,
        "vectors": vectors_path.display().to_string(),
        "queries": queries.len(),
        "points": points,
    });
    fs::write(out, serde_json::to_vec(&report)?)?;
    println!(
        "scan: {} points, {} queries, {} passes, wrote {}",
        report["points"].as_array().map_or(0, Vec::len),
        queries.len(),
        passes,
        out.display()
    );
    Ok(())
}

// ----------------------------------------------------------------- cross ------

/// One pool of the alternating campaign.
struct CrossPoint {
    label: String,
    kind: &'static str,
    requested_span: u64,
    pool: RoaringBitmap,
}

/// Builds the pools of a campaign, in the order `sizes` then `spans × densities`.
///
/// `cross` and `agree` share it, so the control checks the scan on exactly the
/// pools the campaign measured: same seed, same window, same draw.
fn cross_points(
    live: &RoaringBitmap,
    live_ids: &[u32],
    sizes: &str,
    spans: &str,
    densities: &str,
    seed: u64,
) -> Result<Vec<CrossPoint>, Box<dyn std::error::Error>> {
    let mut points: Vec<CrossPoint> = Vec::new();
    for size_text in sizes.split(',') {
        let size_text = size_text.trim();
        if size_text.is_empty() {
            continue;
        }
        if size_text == "all" {
            let (first, last) = match (live.min(), live.max()) {
                (Some(first), Some(last)) => (first, last),
                _ => (0, 0),
            };
            points.push(CrossPoint {
                label: "full".to_owned(),
                kind: "full",
                requested_span: u64::from(last) - u64::from(first) + 1,
                pool: live.clone(),
            });
            continue;
        }
        let size: usize = size_text.parse()?;
        points.push(CrossPoint {
            label: format!("contiguous-{size}"),
            kind: "contiguous",
            requested_span: 0,
            pool: pool_contiguous(live, size),
        });
    }
    for span_text in spans.split(',') {
        let span_text = span_text.trim();
        if span_text.is_empty() {
            continue;
        }
        let span: usize = span_text.parse()?;
        for density_text in densities.split(',') {
            let density_text = density_text.trim();
            if density_text.is_empty() {
                continue;
            }
            let percent: f64 = density_text.parse()?;
            points.push(CrossPoint {
                label: format!("density-{span}-{density_text}"),
                kind: "density",
                requested_span: span as u64,
                pool: pool_window(live_ids, span, percent / 100.0, seed),
            });
        }
    }
    if points.is_empty() {
        return Err("no pool: pass --sizes and/or --spans".into());
    }
    Ok(points)
}

/// Alternating campaign of ADR-0081: the engine's single traversal, on the same
/// pool and the same queries, pass after pass.
///
/// The campaign of the 2026-10-27 ran the dense and the sparse traversal back to
/// back and alternated their order inside a pair (ADR-0081, decision 3). ADR-0081
/// then removed the second traversal: there is nothing left to alternate, and
/// `--pairs P` now runs P passes of the one traversal — as many passes as each
/// side of an old pair — after a warm-up pass kept apart from them.
///
/// ADR-0082 keeps this mode as the A/B driver. The binary of `e7d4e08` (the
/// density rule) and the binary of the single traversal take the same command
/// line and print the same keys of a point; the old document names the traversal
/// its rule chose in `traversal_automatic`, and the A/B reads the series of that
/// traversal, point by point, against the `sparse` series of the new document.
fn cross_mode(
    arguments: &[String],
    opened: &Opened,
    out: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let vectors_path = PathBuf::from(required(arguments, "--vectors")?);
    let pairs: usize = optional(arguments, "--pairs")
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(3);
    let seed: u64 = optional(arguments, "--seed")
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(2026);
    let ns = optional(arguments, "--ns").unwrap_or_else(|| "5000".into());
    let densities =
        optional(arguments, "--densities").unwrap_or_else(|| "10,25,40,50,60,75".into());
    let spans = optional(arguments, "--spans").unwrap_or_else(|| "1000000,7740000".into());
    let sizes = optional(arguments, "--sizes").unwrap_or_default();

    let snapshot = opened.snapshot();
    let metas = snapshot.segments();
    let dims = metas.first().ok_or("the index holds no segment")?.dims as usize;
    let queries = read_matrix(&vectors_path, dims)?;
    let live = snapshot.filters().live().clone();
    let live_len = usize::try_from(live.len())?;
    let live_ids: Vec<u32> = live.iter().collect();
    let codes = codes_of(&snapshot);
    let query_codes: Vec<Vec<u64>> = queries.iter().map(|query| quantize(query)).collect();
    let k = opened.config.search.candidates_per_branch as usize;

    // Pools first, so the document lists them even if a later pass fails.
    let points = cross_points(&live, &live_ids, &sizes, &spans, &densities, seed)?;

    let scan_pool = ScanPool::new(opened.threads)?;
    let mut measured = Vec::with_capacity(points.len());
    for point in &points {
        let pool = &point.pool;
        let pool_len = pool.len() as u32;
        let (first, last) = match (pool.min(), pool.max()) {
            (Some(first), Some(last)) => (first, last),
            _ => (0, 0),
        };
        let span = u64::from(last) - u64::from(first) + 1;
        let plan = plan(&opened.config.search, pool_len, k, false);
        for n_text in ns.split(',') {
            let n: usize = n_text.trim().parse()?;
            let n = n.max(k);
            // Warm-up: one pass, kept apart from the measured passes.
            let mut warmup = Vec::with_capacity(query_codes.len());
            for query_code in &query_codes {
                let started = Instant::now();
                let candidates =
                    scan_pool.install(|| scan_replica(&codes, pool, pool_len, query_code, n));
                warmup.push(started.elapsed().as_secs_f64() * 1e6);
                std::hint::black_box(candidates.len());
            }
            // The measured passes: same queries, same pool, one after another.
            let mut detail = Vec::with_capacity(pairs);
            let mut all: Vec<f64> = Vec::with_capacity(query_codes.len() * pairs);
            for pass in 1..=pairs {
                let mut durations = Vec::with_capacity(query_codes.len());
                for query_code in &query_codes {
                    let started = Instant::now();
                    let candidates =
                        scan_pool.install(|| scan_replica(&codes, pool, pool_len, query_code, n));
                    durations.push(started.elapsed().as_secs_f64() * 1e6);
                    std::hint::black_box(candidates.len());
                }
                all.extend_from_slice(&durations);
                detail.push(json!({
                    "pass": pass,
                    "sparse": summarise(&durations),
                    "sparse_us": durations,
                }));
            }
            measured.push(json!({
                "label": point.label,
                "kind": point.kind,
                "requested_span": point.requested_span,
                "size": pool_len,
                "span": span,
                "density": pool_len as f64 / span as f64,
                "n": n,
                "first_iid": first,
                "last_iid": last,
                "traversal": "sparse",
                "plan": plan.strategy.as_str(),
                "pairs_count": pairs,
                "queries": queries.len(),
                "warmup": {
                    "summary": summarise(&warmup),
                    "durations_us": warmup,
                },
                "pairs": detail,
                "sparse": {
                    "durations_us": all,
                    "summary": summarise(&all),
                },
            }));
        }
    }

    let report = json!({
        "kind": "cross",
        "index": opened.index,
        "dims": dims,
        "threads": opened.threads,
        "live_passages": live_len,
        "segments": metas.len(),
        "segment_counts": metas.iter().map(|meta| meta.count).collect::<Vec<u32>>(),
        "segment_iid_starts": metas
            .iter()
            .map(|meta| meta.iid_start.get())
            .collect::<Vec<u32>>(),
        "vectors": vectors_path.display().to_string(),
        "queries": queries.len(),
        "points": measured,
    });
    fs::write(out, serde_json::to_vec(&report)?)?;
    println!(
        "cross: {} points, {} queries, {} pairs, wrote {}",
        report["points"].as_array().map_or(0, Vec::len),
        queries.len(),
        pairs,
        out.display()
    );
    Ok(())
}

// ----------------------------------------------------------------- agree ------

/// Full Hamming sort of the pool: the oracle of the `agree` control.
///
/// The distance is recomputed here with `u64::count_ones`, without
/// `kernels::hamming_function`, and the segment of an identifier comes from a
/// binary search per identifier — the pre-ADR-0082 lookup — so the oracle shares
/// no code path with the replica it checks. The order is the one of the scan:
/// ascending distance, then ascending internal identifier.
fn full_sort(
    segments: &[Codes<'_>],
    pool: &RoaringBitmap,
    query_code: &[u64],
    n: usize,
) -> Vec<Candidate> {
    let words = query_code.len();
    let mut candidates: Vec<Candidate> = pool
        .iter()
        .filter_map(|iid| {
            let position = segments.partition_point(|segment| segment.iid_start <= iid);
            let segment = position
                .checked_sub(1)
                .and_then(|index| segments.get(index))?;
            if u64::from(iid) >= segment.end() {
                return None;
            }
            let code = segment.code(iid, words)?;
            let distance = query_code
                .iter()
                .zip(code)
                .map(|(left, right)| u64::from((left ^ right).count_ones()))
                .sum();
            Some(Candidate {
                iid: Iid::new(iid),
                distance,
            })
        })
        .collect();
    candidates.sort_unstable_by(|left, right| right.cmp(left));
    candidates.truncate(n);
    candidates
}

/// Control of the campaign: the replica must equal a full Hamming sort.
///
/// A defect in the replica of the traversal would change the work measured
/// without changing anything visible, and the campaign would report the latency
/// of a computation nobody runs. Production answers that question for its own
/// scan by `the_scan_agrees_with_a_full_sort`; the harness makes the same claim
/// on **every pool of the campaign**, for a handful of queries: same candidates,
/// same distances, same order (ADR-0081, decision 5).
///
/// Since ADR-0081 there is one traversal, so there is no second one to compare
/// with; this control is the stronger claim, and it closes the A/B control of
/// ADR-0082 between the two binaries: the old document proves its dense and
/// sparse traversals agree, the new one proves its single traversal equals a full
/// sort, and both cover the same pools — hence the same candidates.
///
/// The control reads the codes and the pool exactly as `cross` does — same
/// builder, same seed — so a pool that agrees here is a pool the campaign
/// measured on valid ground.
fn agree_mode(
    arguments: &[String],
    opened: &Opened,
    out: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let vectors_path = PathBuf::from(required(arguments, "--vectors")?);
    let seed: u64 = optional(arguments, "--seed")
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(2026);
    let ns = optional(arguments, "--ns").unwrap_or_else(|| "5000".into());
    let densities =
        optional(arguments, "--densities").unwrap_or_else(|| "10,25,40,50,60,75".into());
    let spans = optional(arguments, "--spans").unwrap_or_else(|| "1000000,7740000".into());
    let sizes = optional(arguments, "--sizes").unwrap_or_default();
    let checked: usize = optional(arguments, "--queries")
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(5);

    let snapshot = opened.snapshot();
    let metas = snapshot.segments();
    let dims = metas.first().ok_or("the index holds no segment")?.dims as usize;
    let queries = read_matrix(&vectors_path, dims)?;
    let live = snapshot.filters().live().clone();
    let live_len = usize::try_from(live.len())?;
    let live_ids: Vec<u32> = live.iter().collect();
    let codes = codes_of(&snapshot);
    let query_codes: Vec<Vec<u64>> = queries.iter().map(|query| quantize(query)).collect();
    let k = opened.config.search.candidates_per_branch as usize;
    let points = cross_points(&live, &live_ids, &sizes, &spans, &densities, seed)?;
    let scan_pool = ScanPool::new(opened.threads)?;

    let mut checked_points = Vec::with_capacity(points.len());
    for point in &points {
        let pool = &point.pool;
        let pool_len = pool.len() as u32;
        let (first, last) = match (pool.min(), pool.max()) {
            (Some(first), Some(last)) => (first, last),
            _ => (0, 0),
        };
        let span = u64::from(last) - u64::from(first) + 1;
        for n_text in ns.split(',') {
            let n_text = n_text.trim();
            if n_text.is_empty() {
                continue;
            }
            let n: usize = n_text.parse()?;
            let n = n.max(k);
            let mut different = 0_usize;
            let mut first_difference: Option<Value> = None;
            for (index, query_code) in query_codes.iter().take(checked).enumerate() {
                let replica =
                    scan_pool.install(|| scan_replica(&codes, pool, pool_len, query_code, n));
                let oracle = full_sort(&codes, pool, query_code, n);
                if replica != oracle {
                    different += 1;
                    if first_difference.is_none() {
                        let position = replica
                            .iter()
                            .zip(&oracle)
                            .position(|(left, right)| left != right)
                            .unwrap_or(replica.len().min(oracle.len()));
                        first_difference = Some(json!({
                            "query": index,
                            "position": position,
                            "replica": replica.get(position).map(|c| json!({
                                "iid": c.iid.get(),
                                "distance": c.distance,
                            })),
                            "full_sort": oracle.get(position).map(|c| json!({
                                "iid": c.iid.get(),
                                "distance": c.distance,
                            })),
                            "lengths": [replica.len(), oracle.len()],
                        }));
                    }
                }
            }
            checked_points.push(json!({
                "label": point.label,
                "kind": point.kind,
                "size": pool_len,
                "span": span,
                "density": pool_len as f64 / span as f64,
                "n": n,
                "queries_checked": query_codes.len().min(checked),
                "identical": different == 0,
                "queries_different": different,
                "first_difference": first_difference,
            }));
        }
    }

    let identical = checked_points
        .iter()
        .all(|point| point["identical"] == Value::Bool(true));
    let report = json!({
        "kind": "agree",
        "index": opened.index,
        "dims": dims,
        "threads": opened.threads,
        "live_passages": live_len,
        "segments": metas.len(),
        "segment_counts": metas.iter().map(|meta| meta.count).collect::<Vec<u32>>(),
        "vectors": vectors_path.display().to_string(),
        "queries_checked": query_codes.len().min(checked),
        "pools_checked": checked_points.len(),
        "identical": identical,
        "points": checked_points,
    });
    fs::write(out, serde_json::to_vec(&report)?)?;
    println!(
        "agree: {} points, {}/{} identical, wrote {}",
        report["pools_checked"].as_u64().unwrap_or(0),
        checked_points
            .iter()
            .filter(|point| point["identical"] == Value::Bool(true))
            .count(),
        checked_points.len(),
        out.display()
    );
    Ok(())
}

// -------------------------------------------------------------- rescore ------

fn rescore_mode(
    arguments: &[String],
    opened: &Opened,
    out: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let vectors_path = PathBuf::from(required(arguments, "--vectors")?);
    let passes: usize = optional(arguments, "--passes")
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(3);
    let ns = optional(arguments, "--ns").unwrap_or_else(|| "1000,2000,5000".into());
    let cold = arguments.iter().any(|argument| argument == "--cold");
    let displace_gb: Option<f64> = optional(arguments, "--displace-gb")
        .map(|value| value.parse())
        .transpose()?;
    let dot_only = arguments.iter().any(|argument| argument == "--dot-only");
    let skip_engine = arguments.iter().any(|argument| argument == "--skip-engine");
    let key_only = arguments.iter().any(|argument| argument == "--key-only");
    let dot_vectors_file = optional(arguments, "--dot-vectors-file");
    let snapshot = opened.snapshot();
    let dims = snapshot
        .segments()
        .first()
        .ok_or("the index holds no segment")?
        .dims as usize;
    let queries = read_matrix(&vectors_path, dims)?;
    let live = snapshot.filters().live().clone();
    let live_len = live.len() as u32;
    let codes = codes_of(&snapshot);
    let query_codes: Vec<Vec<u64>> = queries.iter().map(|query| quantize(query)).collect();
    let k = opened.config.search.candidates_per_branch as usize;
    let scan_pool = ScanPool::new(opened.threads)?;
    // ADR-0077, decision 1: a figure is kept only if the plan returns `binary`.
    // The pool does not change during the run, so one plan covers every point,
    // and the harness refuses to measure a rescoring series that the planner
    // would not run as BINARY.
    let dense_plan = plan(&opened.config.search, live_len, k, false);
    if dense_plan.strategy != Strategy::Binary {
        return Err(format!(
            "the plan is '{}' on {live_len} live passages: the rescoring series of \
             ADR-0077 need 'binary', so the measurement copy must carry \
             search.exact_threshold = 0 and search.scan = \"binary\"",
            dense_plan.strategy.as_str()
        )
        .into());
    }

    // Candidate sets are the ones production scores: the binary scan of the
    // whole live pool, `max(N, k)` candidates, ADR-0074.
    let mut candidate_sets: Vec<Vec<Vec<Candidate>>> = Vec::new();
    for n_text in ns.split(',') {
        let n: usize = n_text.trim().parse()?;
        let mut per_query = Vec::with_capacity(queries.len());
        for query_code in &query_codes {
            per_query.push(
                scan_pool.install(|| scan_replica(&codes, &live, live_len, query_code, n.max(k))),
            );
        }
        candidate_sets.push(per_query);
    }

    let searcher = opened.searcher()?;
    let vectors_file = match &dot_vectors_file {
        Some(path) => PathBuf::from(path),
        None => opened.vectors_path()?,
    };
    let flat = FlatVectors::open(&vectors_file, dims)?;
    let residency_map = if cold {
        Some(FlatVectors::open(&vectors_file, dims)?.map)
    } else {
        None
    };

    let mut displaced: Option<(f64, Vec<PathBuf>)> = None;
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
        displace_cache(gigabytes, &sources)?;
        // Recorded, so a cold document says how much cache it displaced.
        displaced = Some((gigabytes, sources));
    }

    let connection = if key_only {
        Some(rusqlite::Connection::open_with_flags(
            &opened.records_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?)
    } else {
        None
    };
    let mut points = Vec::new();
    for (position, n_text) in ns.split(',').enumerate() {
        let n: usize = n_text.trim().parse()?;
        let mut totals = Vec::with_capacity(queries.len() * passes);
        let mut dots = Vec::with_capacity(queries.len() * passes);
        let mut keys = Vec::with_capacity(queries.len() * passes);
        let mut resident_fractions: Vec<f64> = Vec::new();
        let mut ties = 0_usize;
        for pass in 0..passes {
            for (query_index, (query, candidates)) in
                queries.iter().zip(&candidate_sets[position]).enumerate()
            {
                let iids: Vec<u32> = candidates
                    .iter()
                    .map(|candidate| candidate.iid.get())
                    .collect();
                let pages = pages_of_iids(&iids, dims);
                if let Some(map) = &residency_map {
                    let resident = resident_pages(map, &pages)?;
                    resident_fractions.push(resident as f64 / pages.len() as f64);
                }
                if cold && pass == 0 && query_index == 0 {
                    let resident = resident_pages(&flat.map, &pages)?;
                    println!(
                        "cold: first query of N={n} reads {} pages, {resident} already resident",
                        pages.len()
                    );
                }

                // The engine's rescoring: the exact cosine of each candidate,
                // the key read and the final sort (ADR-0072). This series keeps
                // the full-candidate behaviour on purpose (every key read), for
                // comparability with the MES-03 campaign; production passes `k`
                // (`planner::candidates`).
                if skip_engine {
                    totals.push(f64::NAN);
                } else {
                    let started = Instant::now();
                    let scored = searcher.rescore(query, candidates, candidates.len())?;
                    totals.push(started.elapsed().as_secs_f64() * 1e6);
                    for pair in scored.windows(2) {
                        if pair[0].score == pair[1].score {
                            ties += 1;
                        }
                    }
                }

                if let Some(connection) = &connection {
                    // Replica of `DenseSearcher::key_of`: the same statement,
                    // compiled per read as `query_row` does it, on a read-only
                    // connection of the same database.
                    let started = Instant::now();
                    let mut rows = 0_usize;
                    for iid in &iids {
                        let key: Option<(String, i64)> = connection
                            .query_row(
                                "SELECT doc_id, ord FROM passages WHERE iid = ?1",
                                [i64::from(*iid)],
                                |row| Ok((row.get(0)?, row.get(1)?)),
                            )
                            .optional()?;
                        if key.is_some() {
                            rows += 1;
                        }
                    }
                    std::hint::black_box(rows);
                    keys.push(started.elapsed().as_secs_f64() * 1e6);
                }

                if dot_only {
                    let started = Instant::now();
                    let mut sum = 0.0_f32;
                    let mut buffer = Vec::with_capacity(dims);
                    for iid in &iids {
                        if let Some(score) = flat.dot_with(*iid, query, &mut buffer) {
                            sum += score;
                        }
                    }
                    std::hint::black_box(sum);
                    dots.push(started.elapsed().as_secs_f64() * 1e6);
                }
            }
        }
        points.push(json!({
            "n": n,
            "candidates": candidate_sets[position].first().map_or(0, Vec::len),
            "traversal": "sparse",
            "plan": dense_plan.strategy.as_str(),
            "plan_candidates": dense_plan.rescore_candidates,
            "cold": cold,
            "passes": passes,
            "queries": queries.len(),
            "durations_us": totals,
            "summary": summarise(&totals),
            "pass_summaries": per_pass(&totals, passes, queries.len()),
            "dot_only_us": if dot_only { Some(dots.clone()) } else { None },
            "dot_only_summary": if dot_only { Some(summarise(&dots)) } else { None },
            "key_only_us": if key_only { Some(keys.clone()) } else { None },
            "key_only_summary": if key_only { Some(summarise(&keys)) } else { None },
            "resident_fraction": if resident_fractions.is_empty() {
                Value::Null
            } else {
                json!({
                    "mean": resident_fractions.iter().sum::<f64>() / resident_fractions.len() as f64,
                    "min": resident_fractions.iter().copied().fold(f64::INFINITY, f64::min),
                    "max": resident_fractions.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                    "values": resident_fractions,
                })
            },
            "score_ties": ties,
        }));
    }

    let report = json!({
        "kind": "rescore",
        "index": opened.index,
        "dims": dims,
        "threads": opened.threads,
        "live_passages": live_len,
        "plan": dense_plan.strategy.as_str(),
        "plan_candidates": dense_plan.rescore_candidates,
        "vectors": vectors_path.display().to_string(),
        "dot_vectors_file": vectors_file.display().to_string(),
        "displace_gb": displaced.as_ref().map(|(gigabytes, _)| *gigabytes),
        "displace_sources": displaced
            .as_ref()
            .map(|(_, sources)| sources.iter().map(|path| path.display().to_string()).collect::<Vec<_>>()),
        "queries": queries.len(),
        "cold": cold,
        "points": points,
    });
    fs::write(out, serde_json::to_vec(&report)?)?;
    println!(
        "rescore: {} points, cold {cold}, wrote {}",
        report["points"].as_array().map_or(0, Vec::len),
        out.display()
    );
    Ok(())
}

/// Fills the unified buffer cache with unrelated data, so the vector pages of
/// the measurement copy are evicted from it.
fn displace_cache(gigabytes: f64, sources: &[PathBuf]) -> Result<(), Box<dyn std::error::Error>> {
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
            std::hint::black_box(buffer[0]);
            read += count as u64;
        }
        if read >= target {
            break;
        }
    }
    println!("displacement: read {read} bytes of unrelated files");
    Ok(())
}

// ---------------------------------------------------------------- exact ------

fn exact_mode(
    arguments: &[String],
    opened: &Opened,
    out: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let vectors_path = PathBuf::from(required(arguments, "--vectors")?);
    let passes: usize = optional(arguments, "--passes")
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(3);
    let sizes = optional(arguments, "--sizes").unwrap_or_else(|| "1000,2000,5000".into());
    let seed: u64 = optional(arguments, "--seed")
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(2026);
    let k = opened.config.search.candidates_per_branch as usize;

    let snapshot = opened.snapshot();
    let dims = snapshot
        .segments()
        .first()
        .ok_or("the index holds no segment")?
        .dims as usize;
    let queries = read_matrix(&vectors_path, dims)?;
    let live = snapshot.filters().live().clone();
    let live_len = live.len() as u32;
    let flat = FlatVectors::open(&opened.vectors_path()?, dims)?;
    let scan_pool = ScanPool::new(opened.threads)?;

    let mut points = Vec::new();
    for size_text in sizes.split(',') {
        let size: usize = if size_text.trim() == "all" {
            usize::try_from(live_len)?
        } else {
            size_text.trim().parse()?
        };
        let pool = build_pool(&live, size, Shape::Contiguous, seed);
        let mut durations = Vec::with_capacity(queries.len() * passes);
        let mut ties = 0_usize;
        for _ in 0..passes {
            for query in &queries {
                let started = Instant::now();
                let ranges = windows(&pool, pool.len() as u32, opened.threads.max(1));
                let heaps: Vec<Vec<ScoreIid>> = scan_pool.install(|| {
                    ranges
                        .par_iter()
                        .map(|&(from, to)| {
                            // Replica of `DenseSearcher::score_slice`: a bounded
                            // max-heap of the window, worst on top.
                            let mut heap: BinaryHeap<Reverse<ScoreIid>> =
                                BinaryHeap::with_capacity(k);
                            let mut buffer: Vec<f16> = Vec::with_capacity(dims);
                            let mut live_iids = pool.iter();
                            live_iids.advance_to(from);
                            for iid in live_iids {
                                if iid > to {
                                    break;
                                }
                                let Some(score) = flat.dot_with(iid, query, &mut buffer) else {
                                    continue;
                                };
                                if heap.len() == k
                                    && let Some(worst) = heap.peek()
                                    && score < worst.0.score
                                {
                                    continue;
                                }
                                heap.push(Reverse(ScoreIid { score, iid }));
                                if heap.len() > k {
                                    heap.pop();
                                }
                            }
                            heap.into_iter().map(|Reverse(scored)| scored).collect()
                        })
                        .collect()
                });
                let mut scored: Vec<ScoreIid> = heaps.into_iter().flatten().collect();
                scored.sort_unstable_by(|left, right| right.cmp(left));
                scored.truncate(k);
                for pair in scored.windows(2) {
                    if pair[0].score == pair[1].score {
                        ties += 1;
                    }
                }
                durations.push(started.elapsed().as_secs_f64() * 1e6);
            }
        }
        points.push(json!({
            "pool_size": pool.len(),
            // The EXACT column always uses a contiguous pool of `size` live
            // ids, at the switching point of ADR-0074.
            "shape": "contiguous",
            "k": k,
            "passes": passes,
            "queries": queries.len(),
            "durations_us": durations,
            "summary": summarise(&durations),
            "pass_summaries": per_pass(&durations, passes, queries.len()),
            "score_ties": ties,
        }));
    }

    let report = json!({
        "kind": "exact",
        "index": opened.index,
        "dims": dims,
        "threads": opened.threads,
        "live_passages": live_len,
        "queries": queries.len(),
        "points": points,
    });
    fs::write(out, serde_json::to_vec(&report)?)?;
    println!("exact: wrote {}", out.display());
    Ok(())
}

// -------------------------------------------------------------- compare ------

/// Control: the engine and the replica must return the same top-k on the same
/// real pool.
fn compare_mode(
    arguments: &[String],
    opened: &Opened,
    out: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let vectors_path = PathBuf::from(required(arguments, "--vectors")?);
    let filter_text = optional(arguments, "--filter");
    let n: usize = optional(arguments, "--n")
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(1000);
    let top: usize = optional(arguments, "--top")
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(10);

    let snapshot = opened.snapshot();
    let dims = snapshot
        .segments()
        .first()
        .ok_or("the index holds no segment")?
        .dims as usize;
    let queries = read_matrix(&vectors_path, dims)?;
    let query = queries.first().ok_or("the query file is empty")?;
    let filter = match &filter_text {
        Some(text) => Filter::from_json_str(text)?,
        None => Filter::from_json_str("{}")?,
    };
    let pool = Pool::evaluate(snapshot.filters(), &filter)?;
    let codes = codes_of(&snapshot);
    let query_code = quantize(query);
    let k = opened.config.search.candidates_per_branch as usize;
    let scan_pool = ScanPool::new(opened.threads)?;
    let candidates = scan_pool
        .install(|| scan_replica(&codes, pool.bitmap(), pool.len(), &query_code, n.max(k)));

    let searcher = opened.searcher()?;
    let search = opened.config.search.clone();
    let engine = scan_pool
        .install(|| dense_search(&snapshot, &searcher, &pool, query, k, &search, false))?;
    // The replica keeps the full-candidate behaviour on purpose (every key
    // read), so it stays comparable with the MES-03 campaign; production passes
    // `k` (`planner::candidates`).
    let replica_scored = searcher.rescore(query, &candidates, candidates.len())?;
    let replica: Vec<Value> = replica_scored
        .iter()
        .take(top)
        .map(|scored| json!({"key": scored.key, "score": scored.score}))
        .collect();
    let engine_top: Vec<Value> = engine
        .iter()
        .take(top)
        .map(|hit| json!({"key": hit.key, "score": hit.score}))
        .collect();
    let keys_engine: Vec<&str> = engine
        .iter()
        .take(top)
        .map(|hit| hit.key.as_str())
        .collect();
    let keys_replica: Vec<&str> = replica_scored
        .iter()
        .take(top)
        .map(|scored| scored.key.as_str())
        .collect();
    let identical = keys_engine == keys_replica;
    let planned = plan(&opened.config.search, pool.len(), k, false);

    let report = json!({
        "kind": "compare",
        "index": opened.index,
        "filter": filter_text,
        "pool_size": pool.len(),
        "traversal": "sparse",
        "plan": planned.strategy.as_str(),
        "n": n.max(k),
        "top": top,
        "identical": identical,
        "engine": engine_top,
        "replica": replica,
    });
    fs::write(out, serde_json::to_vec(&report)?)?;
    println!(
        "compare: pool {} ({:?}), plan {}, identical {identical}, wrote {}",
        pool.len(),
        filter_text,
        planned.strategy.as_str(),
        out.display()
    );
    Ok(())
}

// ------------------------------------------------------------------ rss ------

fn rss_mode(
    arguments: &[String],
    opened: &Opened,
    out: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let vectors_path = PathBuf::from(required(arguments, "--vectors")?);
    let passes: usize = optional(arguments, "--passes")
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(1);
    let model = optional(arguments, "--model");
    let ns = optional(arguments, "--ns").unwrap_or_else(|| "1000".into());
    let n: usize = ns.split(',').next().ok_or("empty --ns")?.trim().parse()?;

    let snapshot = opened.snapshot();
    let dims = snapshot
        .segments()
        .first()
        .ok_or("the index holds no segment")?
        .dims as usize;
    let queries = read_matrix(&vectors_path, dims)?;
    let scan_pool = ScanPool::new(opened.threads)?;

    let after_open = vmmap_summary()?;
    let searcher = opened.searcher()?;
    let after_searcher = vmmap_summary()?;

    let filter = Filter::from_json_str("{}")?;
    let search = opened.config.search.clone();
    let k = search.candidates_per_branch as usize;
    let skip_pass = arguments.iter().any(|argument| argument == "--skip-pass");
    // `--skip-pass` runs no query at all: the document records it, so the
    // aggregation cannot report a pass that never happened.
    let effective_passes = if skip_pass { 0 } else { passes };
    let mut runs = 0_usize;
    for _ in 0..effective_passes {
        for query in &queries {
            let pool = Pool::evaluate(snapshot.filters(), &filter)?;
            let hits = scan_pool
                .install(|| dense_search(&snapshot, &searcher, &pool, query, k, &search, false))?;
            runs += hits.len();
        }
    }
    std::hint::black_box(runs);
    let after_pass = vmmap_summary()?;

    let mut model_report = Value::Null;
    if let Some(directory) = &model {
        let directory = PathBuf::from(directory);
        let registry = std::sync::Arc::new(crible_encode::ModelRegistry::new());
        let encoder = crible_encode::QueryEncoder::new(directory, registry, 0);
        let before = vmmap_summary()?;
        let encoded = encoder.encode_query("article 1240", crible_encode::Precision::Fp32)?;
        std::hint::black_box(encoded.vector.len());
        let after = vmmap_summary()?;
        model_report = json!({"before": before, "after": after});
    }

    let pages = page_size();
    let report = json!({
        "kind": "rss",
        "index": opened.index,
        "dims": dims,
        "threads": opened.threads,
        "live_passages": snapshot.filters().live().len(),
        "codes_bytes": snapshot.codes_memory(),
        "filter_memory_bytes": snapshot.filter_memory().total(),
        "queries": queries.len(),
        "passes": effective_passes,
        "n": n,
        "page_size": pages,
        "states": {
            "after_open": after_open,
            "after_searcher": after_searcher,
            "after_pass": after_pass,
        },
        "model": model_report,
    });
    fs::write(out, serde_json::to_vec(&report)?)?;
    println!("rss: wrote {}", out.display());
    Ok(())
}

/// Resident, dirty and mapped-file footprint of this process, read from
/// `vmmap -summary`, which needs no privilege on its own process.
fn vmmap_summary() -> Result<Value, Box<dyn std::error::Error>> {
    let pid = std::process::id().to_string();
    let output = std::process::Command::new("/usr/bin/vmmap")
        .args(["-summary", &pid])
        .output()?;
    let text = String::from_utf8_lossy(&output.stdout);
    // `vmmap` prints one decimal (`4.0G`, `787.4M`), so the split is exact to
    // about 0.05 G per row; `ps` gives the exact resident total in KiB.
    let ps = std::process::Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()?;
    let ps_rss = String::from_utf8_lossy(&ps.stdout).trim().parse::<u64>()? * 1024;
    let mut resident_pages = 0_u64;
    let mut dirty_pages = 0_u64;
    let mut swapped_pages = 0_u64;
    let mut mapped_file = 0_u64;
    let mut physical_footprint = 0_u64;
    let mut writable = json!({});
    let mut libraries = json!({});
    let mut regions: Vec<Value> = Vec::new();
    let mut in_region_table = false;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Physical footprint:") {
            physical_footprint = parse_bytes(rest.trim());
            continue;
        }
        if line.starts_with("Writable regions:") {
            writable = summary_fields(line);
            continue;
        }
        if line.starts_with("ReadOnly portion of Libraries:") {
            libraries = summary_fields(line);
            continue;
        }
        if line.contains("REGION TYPE") {
            in_region_table = true;
            continue;
        }
        if line.contains("TOTAL") {
            in_region_table = false;
            continue;
        }
        if !in_region_table || line.starts_with('=') {
            continue;
        }
        // A region line holds eight numeric columns (virtual, resident, dirty,
        // swapped, volatile, non-volatile, empty, region count) followed on
        // some rows by a note (`see MALLOC ZONE table below`), and the region
        // name may hold spaces (`mapped file`, `unused but dirty shlib`).
        let fields: Vec<&str> = line.split_whitespace().collect();
        let Some(start) = (1..fields.len().saturating_sub(7)).find(|index| {
            fields[*index..*index + 8]
                .iter()
                .all(|field| is_size(field))
        }) else {
            continue;
        };
        let name = fields[..start].join(" ");
        let resident = parse_bytes(fields[start + 1]);
        let dirty = parse_bytes(fields[start + 2]);
        resident_pages += resident;
        dirty_pages += dirty;
        swapped_pages += parse_bytes(fields[start + 3]);
        if name == "mapped file" {
            mapped_file += resident;
        }
        if resident >= 64 * 1024 * 1024 {
            regions.push(json!({
                "name": name,
                "resident_bytes": resident,
                "dirty_bytes": dirty,
            }));
        }
    }
    regions.sort_by(|left, right| {
        right["resident_bytes"]
            .as_u64()
            .cmp(&left["resident_bytes"].as_u64())
    });
    regions.truncate(12);
    Ok(json!({
        "resident_bytes": resident_pages,
        "dirty_bytes": dirty_pages,
        "swapped_bytes": swapped_pages,
        "file_backed_bytes": resident_pages.saturating_sub(dirty_pages),
        "mapped_file_bytes": mapped_file,
        "physical_footprint_bytes": physical_footprint,
        "ps_rss_bytes": ps_rss,
        "writable": writable,
        "libraries": libraries,
        "largest_regions": regions,
    }))
}

/// Tells whether a `vmmap` field is a size (`0K`, `787.4M`, `2.7G`).
fn is_size(text: &str) -> bool {
    let body = text.strip_suffix(['K', 'M', 'G', 'T']).unwrap_or(text);
    !body.is_empty()
        && body
            .chars()
            .all(|character| character.is_ascii_digit() || character == '.')
        && body.chars().next().is_some_and(|c| c.is_ascii_digit())
}

/// Parses a `vmmap` summary line such as
/// `Writable regions: Total=1.8G written=357.9M resident=583.6M(32%)`.
fn summary_fields(line: &str) -> Value {
    let mut fields = serde_json::Map::new();
    for word in line.split_whitespace() {
        let Some((key, value)) = word.split_once('=') else {
            continue;
        };
        let value = value.split('(').next().unwrap_or(value);
        if is_size(value) {
            fields.insert(key.to_owned(), json!(parse_bytes(value)));
        }
    }
    Value::Object(fields)
}

/// Parses a `vmmap` size such as `1313K`, `56.5M` or `1.2G` into bytes.
fn parse_bytes(text: &str) -> u64 {
    let text = text.trim();
    let (number, scale) = match text.char_indices().last() {
        Some((index, 'K')) => (&text[..index], 1024_f64),
        Some((index, 'M')) => (&text[..index], 1024_f64 * 1024.0),
        Some((index, 'G')) => (&text[..index], 1024_f64 * 1024.0 * 1024.0),
        Some((index, 'T')) => (&text[..index], 1024_f64 * 1024.0 * 1024.0 * 1024.0),
        _ => (text, 1.0),
    };
    let value: f64 = number.parse().unwrap_or(0.0);
    (value * scale) as u64
}

// -------------------------------------------------------------- arguments ----

/// Returns the value following `name` on the command line.
fn required(arguments: &[String], name: &str) -> Result<String, String> {
    optional(arguments, name).ok_or_else(|| format!("missing {name}"))
}

/// Returns the value following `name`, when present.
fn optional(arguments: &[String], name: &str) -> Option<String> {
    let position = arguments.iter().position(|argument| argument == name)?;
    arguments.get(position + 1).cloned()
}
