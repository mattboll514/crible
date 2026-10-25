// SPDX-License-Identifier: Apache-2.0

//! MES-07 bench: duration of a compaction, and the two per-passage reads it
//! pays.
//!
//! References: `docs/08-invariants-et-tests.md` (MES-07),
//! `docs/02-architecture.md` ("Suppressions, pierres tombales et compaction",
//! the six steps), the protocol at the head of
//! `crates/crible-store/src/compact.rs`, and `MES-15` for the shape of the
//! decomposition (a whole operation plus replicas of its inner reads).
//!
//! # What is measured
//!
//! The whole compaction is the real one: `crible_store::compact::compact` on a
//! clone of a prepared index, so `CURRENT_EPOCH` moves and the new epoch is
//! written, verified and published. On the same clone, before the compaction,
//! two replicas isolate the reads the compaction performs once per live
//! passage:
//!
//! - `passage_read`: `RecordStore::read_passage` for every live iid, the same
//!   query and the same zstd decompression the compaction runs through
//!   `records::passage_by_iid` on its writer connection;
//! - `vector_read`: `SegmentReader::vector` for every live iid, the same
//!   `pread` of the reference vector the compaction runs through its
//!   `EpochVectors`;
//! - `segment_open`: `SegmentReader::open` for every source segment, which
//!   streams the vector file through its blake3 checksum. The compaction pays
//!   this once on the source epoch (step 3) and once on the new epoch (step 4);
//! - `compress_new` / `compress_old`: the texts of a sample of live passages,
//!   compressed with a reused `zstd::bulk::Compressor` (what production does,
//!   ADR-0048) and with `zstd::stream::encode_all` per text (what it did
//!   before). The gap is the per-passage compression cost the compaction pays
//!   on every copied text.
//!
//! A replica measures a copy of the production read, not the production call:
//! the report labels it. The compaction itself is measured end to end, and the
//! difference between its duration and the sum of the replicas is the write
//! path (segment writes, SQLite inserts, the transaction and the swap), which
//! this bench does not isolate further.
//!
//! # Why clones
//!
//! A compaction replaces the epoch, so repeating it on the same directory would
//! measure the second compaction of a compacted index. Each repetition clones
//! the prepared index (APFS `cp -c`, falling back to `cp -R`) and compacts the
//! clone.
//!
//! # Usage
//!
//! ```text
//! compact_cost measure --index <dir> --work <dir> --repeats <n>
//!                     [--compress-sample 20000]
//!                     --commit <sha> --machine <label> --date <date>
//!                     --out <file.json>
//! ```
//!
//! # Platform
//!
//! macOS only, for `cp -c` and `ps`. The crate compiles the bench anywhere; CI
//! does not run it.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::time::Instant;

use crible_core::config::IndexConfig;
use crible_store::compact::compact;
use crible_store::generation::GenerationStore;
use crible_store::records::RecordStore;
use crible_store::segment::SegmentReader;
use serde_json::{Value, json};

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse(std::env::args().skip(1))?;
    match args.mode.as_str() {
        "measure" => measure(&args),
        other => Err(fail(format!("unknown mode {other:?}: expected `measure`"))),
    }
}

fn fail(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        message.into(),
    ))
}

/// Parsed command line: a mode and `--key value` options.
struct Args {
    mode: String,
    values: BTreeMap<String, Vec<String>>,
}

impl Args {
    fn parse(raw: impl Iterator<Item = String>) -> Result<Self, Box<dyn Error>> {
        let mut mode = None;
        let mut values: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut tokens = raw;
        while let Some(token) = tokens.next() {
            let Some(key) = token.strip_prefix("--") else {
                if mode.is_some() {
                    return Err(fail(format!("unexpected argument {token:?}")));
                }
                mode = Some(token);
                continue;
            };
            if matches!(key, "bench" | "test") {
                continue;
            }
            let value = tokens
                .next()
                .ok_or_else(|| fail(format!("--{key} expects a value")))?;
            values.entry(key.to_owned()).or_default().push(value);
        }
        let mode = mode.ok_or_else(|| fail("missing mode: `measure`"))?;
        Ok(Self { mode, values })
    }

    fn one(&self, key: &str) -> Result<&str, Box<dyn Error>> {
        self.values
            .get(key)
            .and_then(|values| values.first())
            .map(String::as_str)
            .ok_or_else(|| fail(format!("--{key} is required")))
    }

    fn number<T: FromStr>(&self, key: &str) -> Result<T, Box<dyn Error>>
    where
        T::Err: std::fmt::Display,
    {
        let raw = self.one(key)?;
        raw.parse::<T>()
            .map_err(|error| fail(format!("--{key} {raw:?}: {error}")))
    }

    fn number_or<T: FromStr>(&self, key: &str, fallback: T) -> Result<T, Box<dyn Error>>
    where
        T::Err: std::fmt::Display,
    {
        match self.values.get(key).and_then(|values| values.first()) {
            Some(raw) => raw
                .parse::<T>()
                .map_err(|error| fail(format!("--{key} {raw:?}: {error}"))),
            None => Ok(fallback),
        }
    }
}

/// One repetition of the campaign.
fn measure(args: &Args) -> Result<(), Box<dyn Error>> {
    let index = PathBuf::from(args.one("index")?);
    let work = PathBuf::from(args.one("work")?);
    let repeats: usize = args.number("repeats")?;
    let compress_sample: usize = args.number_or("compress-sample", 20_000)?;
    let out = PathBuf::from(args.one("out")?);
    if repeats == 0 {
        return Err(fail("--repeats must be positive"));
    }
    let config = read_config(&index)?;
    fs::create_dir_all(&work)?;

    let mut points = Vec::with_capacity(repeats);
    for repetition in 0..repeats {
        let clone_dir = work.join(format!("rep-{repetition}"));
        remove_directory(&clone_dir)?;
        clone_directory(&index, &clone_dir)?;

        let mut store = GenerationStore::open(&clone_dir, &config.schema)?;
        let before = store.state()?;
        let live = store.snapshot().live_iids().count();
        let source_segments = store.snapshot().segments().len();
        let live_iids: Vec<_> = store.snapshot().live_iids().collect();

        // Replica 1: the payload read of every live passage, the same one the
        // compaction runs through `records::passage_by_iid`.
        // `GenerationStore::records` is crate-private; the replica opens the
        // same database with the public `RecordStore`, so the SQL and the zstd
        // decompression are the ones of `records::passage_by_iid`.
        let records = RecordStore::open(&store.records_path())?;
        let mut texts: Vec<String> = Vec::with_capacity(compress_sample.min(live));
        let started = Instant::now();
        for (position, iid) in live_iids.iter().enumerate() {
            let passage = records.read_passage(*iid)?.ok_or_else(|| {
                fail(format!(
                    "live iid {} is absent from the reference state",
                    iid.get()
                ))
            })?;
            if texts.len() < compress_sample && position % 97 == 0 {
                texts.push(passage.text);
            }
        }
        let passage_read_ms = millis(started.elapsed());
        drop(records);

        // Replica 2 and 3: opening each source segment (blake3 checksum stream)
        // then reading every live vector through the same reader.
        let segments_dir = store.segments_directory();
        let metas = store.snapshot().segments().to_vec();
        let started = Instant::now();
        let mut readers = Vec::with_capacity(metas.len());
        for meta in &metas {
            readers.push(SegmentReader::open(&segments_dir.join(&meta.segment_id))?);
        }
        let segment_open_ms = millis(started.elapsed());
        let started = Instant::now();
        let mut vector_count = 0_usize;
        // The compaction reads the vector of every live iid, and only those.
        for iid in &live_iids {
            let position = metas
                .partition_point(|meta| meta.iid_start <= *iid)
                .checked_sub(1)
                .ok_or_else(|| fail(format!("iid {} precedes every segment", iid.get())))?;
            readers[position].vector(*iid)?;
            vector_count += 1;
        }
        let vector_read_ms = millis(started.elapsed());
        drop(readers);

        // Replica 4: the compression of the sampled texts, reused context
        // against one context per text.
        let (compress_new_ms, compress_old_ms) = compression(&texts)?;

        let started = Instant::now();
        let report = compact(&mut store, &config)?;
        let compaction_ms = millis(started.elapsed());
        let after = store.state()?;
        let new_segments = store.snapshot().segments().len();
        let epoch_bytes = directory_bytes(&store.epoch_directory())?;
        drop(store);

        points.push(json!({
            "repetition": repetition,
            "live_passages": live,
            "dead_passages": before.dead_passages,
            "dead_ratio": before.dead_ratio(),
            "source_segments": source_segments,
            "compaction_ms": compaction_ms,
            "removed_passages": report.removed_passages,
            "generation": report.generation.get(),
            "new_segments": new_segments,
            "new_epoch_bytes": epoch_bytes,
            "passage_read": {"ms": passage_read_ms, "count": live},
            "segment_open": {"ms": segment_open_ms, "count": metas.len()},
            "vector_read": {"ms": vector_read_ms, "count": vector_count},
            "compress_new_ms": compress_new_ms,
            "compress_old_ms": compress_old_ms,
            "compress_sample": texts.len(),
            "live_after": after.live_passages,
        }));
        remove_directory(&clone_dir)?;
    }

    let report = json!({
        "measure": "MES-07",
        "commit": args.one("commit")?,
        "machine": args.one("machine")?,
        "date": args.one("date")?,
        "index": index.display().to_string(),
        "repeats": repeats,
        "points": points,
        "compaction_ms": stats_of(&points, "compaction_ms"),
        "passage_read_ms": stats_of_field(&points, "passage_read", "ms"),
        "segment_open_ms": stats_of_field(&points, "segment_open", "ms"),
        "vector_read_ms": stats_of_field(&points, "vector_read", "ms"),
        "compress_new_ms": stats_of(&points, "compress_new_ms"),
        "compress_old_ms": stats_of(&points, "compress_old_ms"),
    });
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&out, serde_json::to_string_pretty(&report)?)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

/// Compresses `texts` twice: once with a reused context, once with a context
/// per text. Returns `(reused_ms, per_text_ms)`.
fn compression(texts: &[String]) -> Result<(f64, f64), Box<dyn Error>> {
    let level = zstd::DEFAULT_COMPRESSION_LEVEL;
    let started = Instant::now();
    let mut compressor = zstd::bulk::Compressor::new(level)?;
    for text in texts {
        let _ = compressor.compress(text.as_bytes())?;
    }
    let reused = millis(started.elapsed());
    let started = Instant::now();
    for text in texts {
        let _ = zstd::stream::encode_all(text.as_bytes(), level)?;
    }
    let per_text = millis(started.elapsed());
    Ok((reused, per_text))
}

fn read_config(index: &Path) -> Result<IndexConfig, Box<dyn Error>> {
    let raw = fs::read_to_string(index.join("index.json"))?;
    Ok(IndexConfig::from_json_str(&raw)?)
}

/// Clones `source` into `target` with an APFS clone when the filesystem
/// supports it, and copies otherwise.
fn clone_directory(source: &Path, target: &Path) -> Result<(), Box<dyn Error>> {
    let cloned = Command::new("cp")
        .args(["-cR"])
        .arg(source)
        .arg(target)
        .status()?;
    if cloned.success() {
        return Ok(());
    }
    let copied = Command::new("cp")
        .args(["-R"])
        .arg(source)
        .arg(target)
        .status()?;
    if copied.success() {
        Ok(())
    } else {
        Err(fail(format!(
            "cannot clone {} into {}",
            source.display(),
            target.display()
        )))
    }
}

fn remove_directory(path: &Path) -> Result<(), Box<dyn Error>> {
    if path.exists() {
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

fn directory_bytes(path: &Path) -> Result<u64, Box<dyn Error>> {
    let mut total = 0_u64;
    if !path.exists() {
        return Ok(0);
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            total += directory_bytes(&entry.path())?;
        } else {
            total += metadata.len();
        }
    }
    Ok(total)
}

fn millis(duration: std::time::Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn stats_of(points: &[Value], key: &str) -> Value {
    let values: Vec<u64> = points
        .iter()
        .filter_map(|point| point.get(key))
        .filter_map(|value| value.as_f64())
        .map(|value| value.round() as u64)
        .collect();
    stats(&values)
}

fn stats_of_field(points: &[Value], section: &str, key: &str) -> Value {
    let values: Vec<u64> = points
        .iter()
        .filter_map(|point| point.get(section))
        .filter_map(|section| section.get(key))
        .filter_map(|value| value.as_f64())
        .map(|value| value.round() as u64)
        .collect();
    stats(&values)
}

/// Percentiles by nearest rank, the convention of `MES-15`.
fn stats(values: &[u64]) -> Value {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    json!({
        "p50": percentile(&sorted, 50),
        "p95": percentile(&sorted, 95),
        "p99": percentile(&sorted, 99),
        "min": sorted.first().copied(),
        "max": sorted.last().copied(),
        "values": values,
    })
}

fn percentile(sorted: &[u64], percent: usize) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (percent * sorted.len()).div_ceil(100);
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted.get(index).copied()
}
