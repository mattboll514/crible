// SPDX-License-Identifier: Apache-2.0

//! Measurement driver of MES-06 (the lexical branch: size, build time and
//! filtered BM25 latency).
//!
//! References: `docs/08-invariants-et-tests.md` (MES-06), the task sheet
//! `docs/phases/P5-lexical-et-hybride.md` (`MESURER-LEXICAL`), **ADR-0091**
//! (protocol: size and build time on the Legifacile sample alone, extrapolated
//! in analysed tokens and given as a range; "with and without positions" is the
//! `lexical.positions` boolean; a real campaign on the sample and a plumbing
//! campaign on `synth-large`; one alert threshold by subtraction from the
//! 698 ms budget; peak RSS of the commit, segments, rebuild against 60 s and
//! the block-WAND finding), ADR-0007 (tantivy for BM25), ADR-0087 (the writer
//! budget and the commit) and ADR-0088 (the rebuild).
//!
//! This example is a harness, not production code, and it changes nothing in
//! `crates/*/src/`. It calls the **public** API only for what it measures:
//! `Pool::evaluate` builds every pool from a real filter, and
//! `crible_search::lexical::search` runs the branch itself — no replica of the
//! engine is used for latency. Two modes do read structures the public API does
//! not expose, and both say so in their document:
//!
//! - `tokens` reads `records.sqlite` and the stored texts directly, because the
//!   number of analysed tokens per structure is the base of the extrapolation of
//!   ADR-0091, decision 1;
//! - `wand` replicates the collector of `crates/crible-search/src/lexical.rs` to
//!   count the candidates a segment hands over, because the public function
//!   returns the answer and not its cost. The replica is anchored to the engine:
//!   the mode compares its top-k keys with the ones `lexical::search` returns.
//!
//! Modes:
//!
//! ```text
//! lexical_latency probe   --data D --index I --out O [--targets 1000,10000,50000]
//! lexical_latency tokens  --data D --index I --out O
//! lexical_latency size    --data D --index I --out O
//! lexical_latency latency --data D --index I --queries Q --pools P --out O
//!                         [--ks 10,100] [--passes 3] [--synthetic]
//! lexical_latency wand    --data D --index I --queries Q --pools P --out O
//!                         [--limit 20] [--k 10] [--synthetic]
//! lexical_latency rebuild --data D --index I --out O
//! lexical_latency copy-nocache --src S --dst D
//! ```

use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeMap, BinaryHeap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::time::{Duration, Instant};

use crible_core::config::{FieldType, IndexConfig};
use crible_core::filter::Filter;
use crible_search::lexical;
use crible_search::pool::Pool;
use crible_store::generation::{GenerationStore, Snapshot};
use roaring::RoaringBitmap;
use serde_json::{Map, Value, json};
use tantivy::collector::{Collector, Count, SegmentCollector};
use tantivy::fastfield::Column;
use tantivy::query::BooleanQuery;
use tantivy::schema::Term;
use tantivy::{DocId, Score, SegmentReader};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().collect();
    let mode = arguments
        .get(1)
        .cloned()
        .ok_or("usage: lexical_latency <probe|tokens|size|latency|wand|rebuild|copy-nocache> …")?;
    match mode.as_str() {
        "copy-nocache" => {
            let source = PathBuf::from(required(&arguments, "--src")?);
            let destination = PathBuf::from(required(&arguments, "--dst")?);
            let bytes = copy_tree_nocache(&source, &destination)?;
            println!("{bytes} bytes copied without caching");
        }
        _ => {
            let data = PathBuf::from(required(&arguments, "--data")?);
            let index = required(&arguments, "--index")?;
            let out = PathBuf::from(required(&arguments, "--out")?);
            match mode.as_str() {
                "probe" => probe_mode(&arguments, &data, &index, &out)?,
                "tokens" => tokens_mode(&data, &index, &out)?,
                "size" => size_mode(&data, &index, &out)?,
                "latency" => latency_mode(&arguments, &data, &index, &out)?,
                "wand" => wand_mode(&arguments, &data, &index, &out)?,
                "rebuild" => rebuild_mode(&data, &index, &out)?,
                other => return Err(format!("unknown mode '{other}'").into()),
            }
        }
    }
    Ok(())
}

// --------------------------------------------------------------- opening -----

/// One index of the measurement campaign, opened once per process.
struct Opened {
    /// Configuration read from `index.json`.
    config: IndexConfig,
    /// The store: the snapshot, and the write side for `rebuild`.
    store: GenerationStore,
}

impl Opened {
    /// Opens the index for reading: the snapshot publishes the lexical reader
    /// the state certifies (ADR-0087, ADR-0088).
    fn read(data: &Path, index: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let directory = data.join("indexes").join(index);
        let config = config_of(&directory)?;
        let store = GenerationStore::open_read_only(&directory, &config.schema)?;
        Ok(Self { config, store })
    }

    /// Opens the index for writing, as the rebuild job needs.
    fn write(data: &Path, index: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let directory = data.join("indexes").join(index);
        let config = config_of(&directory)?;
        let store = GenerationStore::open(&directory, &config.schema)?;
        Ok(Self { config, store })
    }

    /// Snapshot the campaign reads, held for the whole run so that a query
    /// never pays the opening of a reader.
    fn snapshot(&self) -> Arc<Snapshot> {
        self.store.snapshot()
    }

    /// `records.sqlite` of the committed epoch.
    fn records_path(&self) -> PathBuf {
        self.store.records_path()
    }

    /// Directory of the epoch's lexical index.
    fn lexical_directory(&self) -> PathBuf {
        self.store.epoch_directory().join("lexical")
    }
}

/// Configuration of the index at `directory`, parsed with the engine's own
/// parser so that a measurement copy cannot drift from what the engine reads.
fn config_of(directory: &Path) -> Result<IndexConfig, Box<dyn std::error::Error>> {
    let raw = fs::read_to_string(directory.join("index.json"))?;
    Ok(IndexConfig::from_json_str(&raw)?)
}

// --------------------------------------------------------------- probe -------

/// Proposes real filters whose pool is as close as possible to each target.
///
/// The counts are read from the reference state: one passage per live row of
/// `passages`, attributed to its document's field values. A candidate is then
/// **evaluated by `Pool::evaluate`**, which is the authority on the pool size;
/// the count from SQLite only chooses which candidates to try. Keyword values
/// are unioned in descending count order, so a target is reached with as few
/// clauses as possible under the 64-clause limit of ADR-0029; an integer field
/// is probed by intervals over its sorted values.
fn probe_mode(
    arguments: &[String],
    data: &Path,
    index: &str,
    out: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let targets = numbers(optional(arguments, "--targets"), &[1000, 10_000, 50_000])?;
    let keep = usize::try_from(number(optional(arguments, "--keep"), 5)?)?;
    let opened = Opened::read(data, index)?;
    let snapshot = opened.snapshot();
    let filters = snapshot.filters();

    let (values, live) = field_counts(&opened)?;
    let mut report: Vec<Value> = Vec::new();
    for target in &targets {
        let mut candidates: Vec<(u32, Value)> = Vec::new();
        for (field, counts) in &values {
            let spec = opened
                .config
                .schema
                .field(field)
                .map(|(spec, _)| spec.clone())
                .ok_or("the field of the reference state is not declared")?;
            match spec.field_type {
                FieldType::Keyword | FieldType::Bool => {
                    let mut ordered: Vec<(&String, &u64)> = counts.iter().collect();
                    ordered.sort_by(|left, right| right.1.cmp(left.1));
                    let mut values: Vec<&str> = Vec::new();
                    for (value, _) in ordered.iter().take(64) {
                        values.push(value.as_str());
                        let candidate = union_filter(field, &values);
                        if let Some(size) = pool_size(filters, &candidate)? {
                            candidates.push((size, candidate));
                        }
                    }
                }
                FieldType::Integer => {
                    let mut years: Vec<(i64, u64)> = counts
                        .iter()
                        .filter_map(|(value, count)| value.parse::<i64>().ok().map(|y| (y, *count)))
                        .collect();
                    years.sort_unstable();
                    for start in 0..years.len() {
                        let mut total = 0_u64;
                        for end in start..years.len() {
                            total += years[end].1;
                            if total >= *target {
                                let candidate =
                                    interval_filter(field, years[start].0, years[end].0);
                                if let Some(size) = pool_size(filters, &candidate)? {
                                    candidates.push((size, candidate));
                                }
                                break;
                            }
                        }
                    }
                }
                FieldType::Float | FieldType::Date => {}
            }
        }
        candidates.sort_by_key(|(size, _)| u64::from(*size).abs_diff(*target));
        let best: Vec<Value> = candidates
            .into_iter()
            .take(keep)
            .map(|(size, filter)| json!({"pool_size": size, "filter": filter}))
            .collect();
        report.push(json!({"target": target, "candidates": best}));
    }

    let document = json!({
        "index": index,
        "positions": opened.config.lexical.positions,
        "live_passages": live,
        "live_documents": document_count(&opened)?,
        "targets": report,
    });
    write_json(out, &document)?;
    println!("{index}: {} target(s) probed", targets.len());
    Ok(())
}

/// Live passages per value of every filterable and declared document field,
/// and the number of live passages.
type FieldCounts = (BTreeMap<String, HashMap<String, u64>>, u64);

/// Live passages per value of every filterable and declared document field.
fn field_counts(opened: &Opened) -> Result<FieldCounts, Box<dyn std::error::Error>> {
    let connection = rusqlite::Connection::open_with_flags(
        opened.records_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut documents: HashMap<String, Map<String, Value>> = HashMap::new();
    {
        let mut statement = connection.prepare("SELECT doc_id, fields FROM documents")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let doc_id: String = row.get(0)?;
            let raw: String = row.get(1)?;
            let fields: Value = serde_json::from_str(&raw)?;
            let object = fields
                .as_object()
                .cloned()
                .ok_or("document fields are not an object")?;
            documents.insert(doc_id, object);
        }
    }
    let mut counts: HashMap<String, HashMap<String, u64>> = HashMap::new();
    let mut live = 0_u64;
    {
        let mut statement = connection.prepare(
            "SELECT doc_id, COUNT(*) FROM passages WHERE dead_since_gen IS NULL GROUP BY doc_id",
        )?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let doc_id: String = row.get(0)?;
            let passages: i64 = row.get(1)?;
            let passages = u64::try_from(passages)?;
            live += passages;
            let Some(fields) = documents.get(&doc_id) else {
                continue;
            };
            for (field, value) in fields {
                let Some(spec) = opened.config.schema.field(field) else {
                    continue;
                };
                if !spec.0.filterable {
                    continue;
                }
                let key = match value {
                    Value::Null => continue,
                    other => other.to_string(),
                };
                *counts
                    .entry(field.clone())
                    .or_default()
                    .entry(key)
                    .or_insert(0) += passages;
            }
        }
    }
    Ok((counts.into_iter().collect(), live))
}

/// Number of documents of the reference state.
fn document_count(opened: &Opened) -> Result<u64, Box<dyn std::error::Error>> {
    let connection = rusqlite::Connection::open_with_flags(
        opened.records_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let count: i64 =
        connection.query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))?;
    Ok(u64::try_from(count)?)
}

/// `{field: [v1, v2, …]}` as a disjunction of equality clauses.
fn union_filter(field: &str, values: &[&str]) -> Value {
    if values.len() == 1 {
        let value = serde_json::from_str(values[0]).unwrap_or(Value::String(values[0].to_owned()));
        return json!({field: value});
    }
    let children: Vec<Value> = values
        .iter()
        .map(|value| {
            let value = serde_json::from_str(value).unwrap_or(Value::String((*value).to_owned()));
            json!({field: value})
        })
        .collect();
    json!({"$or": children})
}

/// `{field: {"$gte": from, "$lte": to}}` as an interval clause.
fn interval_filter(field: &str, from: i64, to: i64) -> Value {
    json!({field: {"$gte": from, "$lte": to}})
}

/// Size of the pool of `candidate`, or `None` when it exceeds the limits of
/// ADR-0029 or the schema refuses it.
fn pool_size(
    filters: &crible_store::filters_index::FilterIndex,
    candidate: &Value,
) -> Result<Option<u32>, Box<dyn std::error::Error>> {
    let raw = candidate.to_string();
    let Ok(filter) = Filter::from_json_str(&raw) else {
        return Ok(None);
    };
    if filter.validate(filters.schema()).is_err() {
        return Ok(None);
    }
    let pool = Pool::evaluate(filters, &filter)?;
    Ok(Some(pool.len()))
}

// --------------------------------------------------------------- tokens ------

/// Counts the analysed tokens of every live passage, per structure.
///
/// The base of the extrapolation of ADR-0091, decision 1: the size and the
/// build time are linear in **indexed tokens**, not in passages. The text read
/// here is the one `records.sqlite` stores — the encoder input, the exact text
/// tantivy indexed — and the chain is the one the index declares, rebuilt by
/// `crible_store::lexical::analyzer` from `lexical.language`.
///
/// The risk ADR-0091 names is a count that does not follow the analyser
/// exactly: it is the same function `analyzer::analyzer`, but the count is the
/// harness's, so it is declared as such in the report.
fn tokens_mode(data: &Path, index: &str, out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let opened = Opened::read(data, index)?;
    let mut analyzer = crible_store::lexical::analyzer::analyzer(&opened.config.lexical.language)?;
    let connection = rusqlite::Connection::open_with_flags(
        opened.records_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;

    let mut passages = 0_u64;
    let mut tokens = 0_u64;
    let mut characters = 0_u64;
    let mut distinct: HashSet<String> = HashSet::new();
    let mut shortest = u64::MAX;
    let mut longest = 0_u64;
    {
        let mut statement =
            connection.prepare("SELECT text FROM passages WHERE dead_since_gen IS NULL")?;
        let mut rows = statement.query([])?;
        let mut since = Instant::now();
        while let Some(row) = rows.next()? {
            let frame: Vec<u8> = row.get(0)?;
            let bytes = zstd::stream::decode_all(&frame[..])?;
            let text = String::from_utf8(bytes)?;
            characters += u64::try_from(text.chars().count())?;
            let mut stream = analyzer.token_stream(&text);
            let mut here = 0_u64;
            while let Some(token) = stream.next() {
                here += 1;
                distinct.insert(token.text.clone());
            }
            tokens += here;
            shortest = shortest.min(here);
            longest = longest.max(here);
            passages += 1;
            if since.elapsed() > Duration::from_secs(30) {
                println!("  {index}: {passages} passages analysed, {tokens} tokens");
                since = Instant::now();
            }
        }
    }

    let document = json!({
        "index": index,
        "language": opened.config.lexical.language,
        "positions": opened.config.lexical.positions,
        "passages": passages,
        "tokens": tokens,
        "distinct_terms": distinct.len(),
        "characters": characters,
        "tokens_per_passage": if passages == 0 { 0.0 } else { tokens as f64 / passages as f64 },
        "characters_per_passage": if passages == 0 { 0.0 } else { characters as f64 / passages as f64 },
        "shortest_passage": if passages == 0 { 0 } else { shortest },
        "longest_passage": longest,
    });
    write_json(out, &document)?;
    println!(
        "{index}: {passages} passages, {tokens} tokens ({:.1} per passage), {} distinct",
        document["tokens_per_passage"].as_f64().unwrap_or(0.0),
        distinct.len()
    );
    Ok(())
}

// ----------------------------------------------------------------- size -----

/// Size of the lexical index on disk, and the segments tantivy produced.
///
/// Bytes are summed over the files, not rounded to blocks, and the segment
/// count is the one of `meta.json` — the committed state, not the files a
/// merge may leave behind.
fn size_mode(data: &Path, index: &str, out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let opened = Opened::read(data, index)?;
    let directory = opened.lexical_directory();
    let (bytes, files, by_extension) = directory_size(&directory)?;
    let meta = read_meta(&directory)?;
    let segments = meta
        .get("segments")
        .and_then(Value::as_array)
        .map(|segments| segments.len())
        .unwrap_or(0);
    let state = opened.store.state()?;
    let document = json!({
        "index": index,
        "positions": opened.config.lexical.positions,
        "bytes": bytes,
        "files": files,
        "bytes_by_extension": by_extension,
        "segments": segments,
        "live_passages": state.live_passages,
        "dead_passages": state.dead_passages,
        "bytes_per_passage": if state.live_passages == 0 { 0.0 } else { bytes as f64 / state.live_passages as f64 },
        "index_directory": relative_path(&directory),
    });
    write_json(out, &document)?;
    println!(
        "{index}: {bytes} bytes, {files} files, {segments} segment(s), positions {}",
        opened.config.lexical.positions
    );
    Ok(())
}

/// Bytes, file count and bytes per extension of a directory.
type DirectorySize = (u64, u64, BTreeMap<String, u64>);

/// Bytes, file count and bytes per extension of the directory `directory`.
fn directory_size(directory: &Path) -> Result<DirectorySize, Box<dyn std::error::Error>> {
    let mut bytes = 0_u64;
    let mut files = 0_u64;
    let mut by_extension: BTreeMap<String, u64> = BTreeMap::new();
    let mut stack = vec![directory.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                stack.push(path);
                continue;
            }
            bytes += metadata.len();
            files += 1;
            let extension = path
                .extension()
                .map(|extension| extension.to_string_lossy().to_string())
                .unwrap_or_default();
            *by_extension.entry(extension).or_insert(0) += metadata.len();
        }
    }
    Ok((bytes, files, by_extension))
}

/// `meta.json` of the tantivy index, or an empty object when there is none.
fn read_meta(directory: &Path) -> Result<Value, Box<dyn std::error::Error>> {
    let path = directory.join("meta.json");
    let Ok(raw) = fs::read_to_string(&path) else {
        return Ok(Value::Object(Map::new()));
    };
    Ok(serde_json::from_str(&raw)?)
}

// -------------------------------------------------------------- latency -----

/// One measured point: a pool, a `k` and the durations of each query.
fn latency_mode(
    arguments: &[String],
    data: &Path,
    index: &str,
    out: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let queries_path = PathBuf::from(required(arguments, "--queries")?);
    let pools_path = PathBuf::from(required(arguments, "--pools")?);
    let ks: Vec<usize> = numbers(optional(arguments, "--ks"), &[10, 100])?
        .into_iter()
        .map(usize::try_from)
        .collect::<Result<Vec<usize>, _>>()?;
    let passes = usize::try_from(number(optional(arguments, "--passes"), 3)?)?;
    let synthetic = arguments.iter().any(|argument| argument == "--synthetic");
    // A cold series must not pay the warm-up pass first: `--no-warmup` is used
    // by the single cold bound of the campaign only.
    let warmup = !arguments.iter().any(|argument| argument == "--no-warmup");
    let queries = if synthetic {
        synthetic_queries()
    } else {
        read_queries(&queries_path, index)?
    };
    let pools = read_pools(&pools_path, index)?;

    let opened = Opened::read(data, index)?;
    let snapshot = opened.snapshot();
    if snapshot.lexical().is_none() {
        return Err(format!("{index}: the snapshot publishes no lexical reader").into());
    }

    let mut measured: Vec<Value> = Vec::new();
    for pool_spec in &pools {
        let started = Instant::now();
        let pool = match &pool_spec.filter {
            Some(filter) => {
                let raw = filter.to_string();
                let filter = Filter::from_json_str(&raw)?;
                filter.validate(&opened.config.schema)?;
                Pool::evaluate(snapshot.filters(), &filter)?
            }
            None => Pool::evaluate(snapshot.filters(), &Filter::from_json_str("{}")?)?,
        };
        let pool_millis = started.elapsed().as_secs_f64() * 1000.0;
        for k in &ks {
            // Warm-up: one pass over every query, never recorded, so the first
            // reading of the terms and of the key rows is out of the figures.
            if warmup {
                for query in &queries {
                    let _ = lexical::search(&snapshot, &pool, query, *k)?;
                }
            }
            let mut series: Vec<Vec<f64>> = Vec::with_capacity(passes);
            let mut results: u64 = 0;
            for pass in 0..passes {
                let mut durations: Vec<f64> = Vec::with_capacity(queries.len());
                for query in &queries {
                    let started = Instant::now();
                    let found = lexical::search(&snapshot, &pool, query, *k)?;
                    durations.push(started.elapsed().as_secs_f64() * 1_000_000.0);
                    if pass == 0 {
                        results += u64::try_from(found.len())?;
                    }
                }
                series.push(durations);
            }
            let summary = summarise(&series);
            println!(
                "{index} {} pool={} k={k}: p50 {:.3} ms, p95 {:.3} ms, n={}",
                pool_spec.label,
                pool.len(),
                summary["p50"].as_f64().unwrap_or(0.0) / 1000.0,
                summary["p95"].as_f64().unwrap_or(0.0) / 1000.0,
                summary["n"].as_u64().unwrap_or(0),
            );
            measured.push(json!({
                "pool": pool_spec.label,
                "filter": pool_spec.filter,
                "pool_size": pool.len(),
                "pool_millis": pool_millis,
                "k": k,
                "queries": queries.len(),
                "passes": series,
                "summary": summary,
                "results_in_first_pass": results,
            }));
        }
    }

    let document = json!({
        "index": index,
        "positions": opened.config.lexical.positions,
        "synthetic": synthetic,
        "queries_file": if synthetic { Value::Null } else { json!(relative_path(&queries_path)) },
        "pools_file": relative_path(&pools_path),
        "query_count": queries.len(),
        "warmup": warmup,
        "measured": measured,
    });
    write_json(out, &document)?;
    Ok(())
}

/// One pool of a campaign: a label, and the filter that builds it (`null` for
/// the whole index).
struct PoolSpec {
    label: String,
    filter: Option<Value>,
}

/// Reads the pool list of the campaign for `index`.
fn read_pools(path: &Path, index: &str) -> Result<Vec<PoolSpec>, Box<dyn std::error::Error>> {
    let raw = fs::read_to_string(path)?;
    let document: Value = serde_json::from_str(&raw)?;
    let entries = document
        .get(index)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("no pool list for index '{index}' in {}", path.display()))?;
    let mut pools = Vec::with_capacity(entries.len());
    for entry in entries {
        let label = entry
            .get("label")
            .and_then(Value::as_str)
            .ok_or("a pool has no label")?
            .to_owned();
        let filter = entry
            .get("filter")
            .filter(|value| !value.is_null())
            .cloned();
        pools.push(PoolSpec { label, filter });
    }
    Ok(pools)
}

/// The queries of `queries.jsonl` whose `structure` is `index`, in file order.
///
/// The text is the `question` field, replayed as it is: ADR-0091, decision 3,
/// asks for the 300 real queries, each on the index of its structure.
fn read_queries(path: &Path, index: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let raw = fs::read_to_string(path)?;
    let mut queries = Vec::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let row: Value = serde_json::from_str(line)?;
        if row.get("structure").and_then(Value::as_str) != Some(index) {
            continue;
        }
        let question = row
            .get("question")
            .and_then(Value::as_str)
            .ok_or("a query line has no question")?;
        queries.push(question.to_owned());
    }
    Ok(queries)
}

/// The synthetic query set of the plumbing campaign on `synth-large`.
///
/// ADR-0091, decision 3, plans the sixteen words of the vocabulary of
/// `tools/gen_synth.py` (`:134-151`) as a `df` ≈ 1/16 worst case. That
/// vocabulary only feeds the **small** variant: the large variant writes
/// `large passage <n>` (`tools/gen_synth.py:739`), so a query built from it
/// matches nothing (measured: zero result at every pool). The plumbing queries
/// are therefore the two content words every passage carries, alone and
/// together — `df` = 100 %, a heavier worst case than the planned 1/16 — plus
/// three numeric tokens of the same texts. None of them is a production query.
fn synthetic_queries() -> Vec<String> {
    vec![
        "large".to_owned(),
        "passage".to_owned(),
        "large passage".to_owned(),
        "42".to_owned(),
        "1234".to_owned(),
        "7740000".to_owned(),
    ]
}

/// Summary of a series of passes: nearest-rank percentiles over every sample.
///
/// The percentiles are the ones of ADR-0077 and ADR-0091: the value of rank
/// `ceil(ratio × n)`, ranked from 1, without interpolation.
fn summarise(passes: &[Vec<f64>]) -> Value {
    let mut values: Vec<f64> = passes.iter().flatten().copied().collect();
    if values.is_empty() {
        return json!({"n": 0});
    }
    values.sort_by(f64::total_cmp);
    let n = values.len();
    let rank = |ratio: f64| -> f64 {
        let rank = (ratio * n as f64).ceil() as usize;
        values[rank.clamp(1, n) - 1]
    };
    let sum: f64 = values.iter().sum();
    json!({
        "n": n,
        "p50": rank(0.50),
        "p95": rank(0.95),
        "p99": rank(0.99),
        "mean": sum / n as f64,
        "min": values[0],
        "max": values[n - 1],
        "pass_p50": passes.iter().map(|pass| {
            let mut pass = pass.clone();
            pass.sort_by(f64::total_cmp);
            let rank = (0.50 * pass.len() as f64).ceil() as usize;
            pass[rank.clamp(1, pass.len()) - 1]
        }).collect::<Vec<f64>>(),
    })
}

// ----------------------------------------------------------------- wand -----

/// Candidates a segment hands to the collector, against the size of the pool.
///
/// `docs/03-recherche.md:91` suspects that block-WAND does not apply with a
/// collector that filters on the pool. tantivy only offers block-WAND through
/// `Weight::for_each_pruning`, which `TopDocs` calls; a custom `Collector`
/// without an override is driven by `Weight::for_each`
/// (`tantivy-0.27.0/src/collector/mod.rs:173-205`), which enumerates every
/// matching document. The mode measures exactly that: it counts the `collect`
/// calls of a replica of the production collector, and compares them with the
/// number of documents matching the query and with the size of the pool.
///
/// The replica is anchored to the engine: for each query the mode compares its
/// top-`k` keys with the ones `lexical::search` returns on the same pool.
fn wand_mode(
    arguments: &[String],
    data: &Path,
    index: &str,
    out: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let queries_path = PathBuf::from(required(arguments, "--queries")?);
    let pools_path = PathBuf::from(required(arguments, "--pools")?);
    let k = usize::try_from(number(optional(arguments, "--k"), 10)?)?;
    let limit = usize::try_from(number(optional(arguments, "--limit"), 20)?)?;
    let synthetic = arguments.iter().any(|argument| argument == "--synthetic");
    let mut queries = if synthetic {
        synthetic_queries()
    } else {
        read_queries(&queries_path, index)?
    };
    queries.truncate(limit);
    let pools = read_pools(&pools_path, index)?;

    let opened = Opened::read(data, index)?;
    let snapshot = opened.snapshot();
    let lexical_index = snapshot
        .lexical()
        .ok_or("the snapshot publishes no lexical reader")?;
    let searcher = lexical_index.searcher();
    let field = searcher
        .schema()
        .get_field(crible_store::lexical::TEXT_FIELD_NAME)?;

    let mut measured: Vec<Value> = Vec::new();
    for pool_spec in &pools {
        let pool = match &pool_spec.filter {
            Some(filter) => {
                let raw = filter.to_string();
                let filter = Filter::from_json_str(&raw)?;
                filter.validate(&opened.config.schema)?;
                Pool::evaluate(snapshot.filters(), &filter)?
            }
            None => Pool::evaluate(snapshot.filters(), &Filter::from_json_str("{}")?)?,
        };
        let bitmap = Arc::new(pool.bitmap().clone());
        let mut rows: Vec<Value> = Vec::new();
        for query in &queries {
            let terms = lexical_index.terms(query)?;
            if terms.is_empty() {
                continue;
            }
            let clauses: Vec<Term> = terms
                .iter()
                .map(|term| Term::from_field_text(field, term))
                .collect();
            let boolean = BooleanQuery::new_multiterms_query(clauses);
            let matching = searcher.search(&boolean, &Count)?;
            let counter = Arc::new(AtomicU64::new(0));
            let passed = Arc::new(AtomicU64::new(0));
            let found = searcher.search(
                &boolean,
                &CountingCollector::new(
                    Arc::clone(&bitmap),
                    k,
                    Arc::clone(&counter),
                    Arc::clone(&passed),
                ),
            )?;
            let engine = lexical::search(&snapshot, &pool, query, k)?;
            let replica_keys: Vec<String> = found
                .iter()
                .filter_map(|candidate| {
                    lexical_index
                        .passage_key(crible_core::ids::Iid::new(candidate.iid))
                        .ok()
                        .flatten()
                        .map(|key| key.to_string())
                })
                .collect();
            let engine_keys: Vec<String> = engine.iter().map(|scored| scored.key.clone()).collect();
            // A tie at the `k`-th score makes the engine rerun the query and let
            // the key decide (the ambiguity path of `lexical::search`); the
            // replica stops at the first pass, so it can keep another member of
            // the tie. The anchor is therefore read twice: the key set ignores
            // order, and the sorted score multiset ignores which tie was kept.
            let mut replica_set = replica_keys.clone();
            replica_set.sort_unstable();
            let mut engine_set = engine_keys.clone();
            engine_set.sort_unstable();
            let mut replica_scores: Vec<u32> = found
                .iter()
                .map(|candidate| candidate.score.to_bits())
                .collect();
            replica_scores.sort_unstable();
            let mut engine_scores: Vec<u32> =
                engine.iter().map(|scored| scored.score.to_bits()).collect();
            engine_scores.sort_unstable();
            rows.push(json!({
                "query_terms": terms.len(),
                "matching_documents": matching,
                "visited": counter.load(AtomicOrdering::Relaxed),
                "in_pool": passed.load(AtomicOrdering::Relaxed),
                "kept": found.len(),
                "identical_to_engine": replica_keys == engine_keys,
                "same_key_set": replica_set == engine_set,
                "same_scores": replica_scores == engine_scores,
            }));
        }
        if let Some(row) = rows.first() {
            println!(
                "{index} {} pool={} k={k}: visited {} of {} matching, pool {}",
                pool_spec.label,
                pool.len(),
                row["visited"],
                row["matching_documents"],
                pool.len()
            );
        }
        measured.push(json!({
            "pool": pool_spec.label,
            "filter": pool_spec.filter,
            "pool_size": pool.len(),
            "k": k,
            "queries": rows,
        }));
    }

    let document = json!({
        "index": index,
        "synthetic": synthetic,
        "pools_file": relative_path(&pools_path),
        "measured": measured,
    });
    write_json(out, &document)?;
    Ok(())
}

/// One candidate of the replica, ordered like the production heap.
#[derive(Debug, Clone, Copy)]
struct Candidate {
    score: Score,
    iid: u32,
}

impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Candidate {}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score
            .total_cmp(&other.score)
            .then_with(|| other.iid.cmp(&self.iid))
    }
}

/// Replica of `PoolTopK` of `crates/crible-search/src/lexical.rs`, with two
/// counters: every `collect` call, and the ones the pool keeps.
struct CountingCollector {
    bitmap: Arc<RoaringBitmap>,
    k: usize,
    visited: Arc<AtomicU64>,
    passed: Arc<AtomicU64>,
}

impl CountingCollector {
    fn new(
        bitmap: Arc<RoaringBitmap>,
        k: usize,
        visited: Arc<AtomicU64>,
        passed: Arc<AtomicU64>,
    ) -> Self {
        Self {
            bitmap,
            k,
            visited,
            passed,
        }
    }
}

impl Collector for CountingCollector {
    type Fruit = Vec<Candidate>;
    type Child = CountingSegment;

    fn for_segment(
        &self,
        _segment_ord: u32,
        segment: &SegmentReader,
    ) -> tantivy::Result<Self::Child> {
        Ok(CountingSegment {
            bitmap: Arc::clone(&self.bitmap),
            iid: segment
                .fast_fields()
                .u64(crible_store::lexical::IID_FIELD_NAME)?,
            heap: BinaryHeap::new(),
            k: self.k,
            visited: Arc::clone(&self.visited),
            passed: Arc::clone(&self.passed),
        })
    }

    fn requires_scoring(&self) -> bool {
        true
    }

    fn merge_fruits(&self, fruits: Vec<Vec<Candidate>>) -> tantivy::Result<Vec<Candidate>> {
        let mut heap: BinaryHeap<Reverse<Candidate>> = BinaryHeap::new();
        for candidate in fruits.into_iter().flatten() {
            heap.push(Reverse(candidate));
            if heap.len() > self.k {
                heap.pop();
            }
        }
        let mut candidates: Vec<Candidate> = heap
            .into_iter()
            .map(|Reverse(candidate)| candidate)
            .collect();
        candidates.sort_unstable_by(|left, right| right.cmp(left));
        Ok(candidates)
    }
}

/// One segment of the replica, with its counters.
struct CountingSegment {
    bitmap: Arc<RoaringBitmap>,
    iid: Column<u64>,
    heap: BinaryHeap<Reverse<Candidate>>,
    k: usize,
    visited: Arc<AtomicU64>,
    passed: Arc<AtomicU64>,
}

impl SegmentCollector for CountingSegment {
    type Fruit = Vec<Candidate>;

    fn collect(&mut self, doc: DocId, score: Score) {
        self.visited.fetch_add(1, AtomicOrdering::Relaxed);
        let Some(value) = self.iid.first(doc) else {
            return;
        };
        let Ok(iid) = u32::try_from(value) else {
            return;
        };
        if !self.bitmap.contains(iid) {
            return;
        }
        self.passed.fetch_add(1, AtomicOrdering::Relaxed);
        if self.k == 0 {
            return;
        }
        let candidate = Candidate { score, iid };
        if self.heap.len() < self.k {
            self.heap.push(Reverse(candidate));
            return;
        }
        if let Some(&Reverse(worst)) = self.heap.peek()
            && candidate > worst
        {
            self.heap.pop();
            self.heap.push(Reverse(candidate));
        }
    }

    fn harvest(self) -> Vec<Candidate> {
        self.heap
            .into_iter()
            .map(|Reverse(candidate)| candidate)
            .collect()
    }
}

// --------------------------------------------------------------- rebuild ----

/// Runs the rebuild job of `RECONSTRUIRE-LEXICAL` and times it.
///
/// The call is the public one the CLI makes
/// (`crible_store::generation::rebuild_lexical`), on a store opened for
/// writing. Peak RSS is sampled from `ps` every 200 ms, plus one last read after
/// the rebuild: it is a sampled peak, not the exact maximum of `getrusage`, and
/// the report says so.
fn rebuild_mode(data: &Path, index: &str, out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut opened = Opened::write(data, index)?;
    let stop = Arc::new(AtomicBool::new(false));
    let pid = std::process::id();
    let watcher = Arc::clone(&stop);
    let sampler = std::thread::spawn(move || {
        let mut peak = 0_u64;
        while !watcher.load(AtomicOrdering::Relaxed) {
            if let Some(rss) = resident_bytes(pid) {
                peak = peak.max(rss);
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        if let Some(rss) = resident_bytes(pid) {
            peak = peak.max(rss);
        }
        peak
    });

    // The rebuild prints nothing of its own, and a large index takes minutes:
    // a progress line every ten seconds keeps the foreground run observable.
    let progress = Arc::new(AtomicBool::new(false));
    let progress_stop = Arc::clone(&progress);
    let progress_pid = std::process::id();
    let progress_label = index.to_owned();
    let progress_thread = std::thread::spawn(move || {
        let started = Instant::now();
        let mut tick: u64 = 0;
        while !progress_stop.load(AtomicOrdering::Relaxed) {
            std::thread::sleep(Duration::from_secs(1));
            tick += 1;
            if progress_stop.load(AtomicOrdering::Relaxed) {
                break;
            }
            if !tick.is_multiple_of(10) {
                continue;
            }
            let rss = resident_bytes(progress_pid).unwrap_or(0);
            println!(
                "{}: rebuilding, {:.0} s elapsed, RSS {:.1} MiB",
                progress_label,
                started.elapsed().as_secs_f64(),
                rss as f64 / (1024.0 * 1024.0)
            );
        }
    });

    let started = Instant::now();
    let generation = crible_store::generation::rebuild_lexical(&mut opened.store)?;
    let elapsed = started.elapsed();
    stop.store(true, AtomicOrdering::Relaxed);
    progress.store(true, AtomicOrdering::Relaxed);
    let _ = progress_thread.join();
    let peak = sampler.join().unwrap_or(0);

    let directory = opened.lexical_directory();
    let (bytes, files, _) = directory_size(&directory)?;
    let meta = read_meta(&directory)?;
    let segments = meta
        .get("segments")
        .and_then(Value::as_array)
        .map(|segments| segments.len())
        .unwrap_or(0);
    let document = json!({
        "index": index,
        "positions": opened.config.lexical.positions,
        "generation": generation.get(),
        "seconds": elapsed.as_secs_f64(),
        "peak_rss_bytes": peak,
        "segments": segments,
        "bytes": bytes,
        "files": files,
    });
    write_json(out, &document)?;
    println!(
        "{index}: rebuilt in {:.3} s, peak RSS {:.1} MiB, {segments} segment(s), {bytes} bytes",
        elapsed.as_secs_f64(),
        peak as f64 / (1024.0 * 1024.0)
    );
    Ok(())
}

/// Resident set of `pid` in bytes, read from `ps`.
fn resident_bytes(pid: u32) -> Option<u64> {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let kib: u64 = text.trim().parse().ok()?;
    Some(kib * 1024)
}

// -------------------------------------------------------- copy nocache ------

/// Copies `source` to `destination` with `F_NOCACHE` on both descriptors.
///
/// The cold series of ADR-0091, decision 3, reads a copy whose pages are absent
/// from the cache; the recipe of MES-02 and MES-03 used the same call
/// (ADR-0084). The copy is recursive so that a whole index can be copied.
fn copy_tree_nocache(source: &Path, destination: &Path) -> Result<u64, Box<dyn std::error::Error>> {
    if source.is_dir() {
        fs::create_dir_all(destination)?;
        let mut bytes = 0_u64;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            bytes += copy_tree_nocache(&entry.path(), &destination.join(entry.file_name()))?;
        }
        return Ok(bytes);
    }
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
        copied += u64::try_from(read)?;
    }
    output.sync_all()?;
    Ok(copied)
}

// SAFETY: `fcntl` is the C function of the platform, declared with the macOS
// ABI `F_NOCACHE` (48), exactly as `dense_latency.rs` does (ADR-0084). No other
// unsafe code of this harness touches it.
#[allow(dead_code)]
mod system {
    pub const F_NOCACHE: i32 = 48;

    unsafe extern "C" {
        pub fn fcntl(descriptor: i32, command: i32, ...) -> i32;
    }
}

use system::{F_NOCACHE, fcntl};

// --------------------------------------------------------------- helpers ----

fn required(arguments: &[String], flag: &str) -> Result<String, Box<dyn std::error::Error>> {
    optional(arguments, flag).ok_or_else(|| format!("missing {flag}").into())
}

fn optional(arguments: &[String], flag: &str) -> Option<String> {
    arguments
        .iter()
        .position(|argument| argument == flag)
        .and_then(|position| arguments.get(position + 1))
        .cloned()
}

fn number(value: Option<String>, default: u64) -> Result<u64, Box<dyn std::error::Error>> {
    match value {
        Some(text) => Ok(text.parse()?),
        None => Ok(default),
    }
}

fn numbers(value: Option<String>, default: &[u64]) -> Result<Vec<u64>, Box<dyn std::error::Error>> {
    let Some(text) = value else {
        return Ok(default.to_vec());
    };
    text.split(',')
        .map(|item| Ok(item.trim().parse()?))
        .collect()
}

/// Writes `document` as pretty JSON, creating the parent directory.
fn write_json(path: &Path, document: &Value) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = File::create(path)?;
    file.write_all(serde_json::to_string_pretty(document)?.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

/// `path` relative to the repository root, so that no artefact carries a local
/// absolute path. A path outside the repository is returned as it is, and the
/// analyser refuses it.
fn relative_path(path: &Path) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf);
    match root.and_then(|root| path.strip_prefix(root).ok().map(Path::to_path_buf)) {
        Some(relative) => relative.to_string_lossy().to_string(),
        None => path.to_string_lossy().to_string(),
    }
}
