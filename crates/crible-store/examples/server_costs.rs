// SPDX-License-Identifier: Apache-2.0

//! Isolation of two costs the server pays on read routes and no timer of the
//! engine reports (task `MESURER-EN-CONTENEUR-MAC`, passage B, ADR-0135).
//!
//! This example is a harness, not production code. It calls the production
//! readers and nothing else:
//!
//! - mode `walk` times the recursive size of an index directory, the work
//!   `sizes.disk_bytes` of `GET /v1/indexes/{index}` does through
//!   `indexes::directory_size` (`read_dir`, `symlink_metadata`, one `len()` per
//!   file, symlinks skipped, subdirectories recursed). The walk is repeated here
//!   because that function is `pub(crate)`, and it follows it step for step;
//! - mode `document` times, on the `records.sqlite` of one epoch, the same calls
//!   `GET /v1/indexes/{index}/documents/{doc_id}` makes on a **fresh**
//!   `ReadPool` of capacity 1 (`ReadPool::new`, then `read_document` and
//!   `passages_of_document`), and the same calls again on a pool opened once and
//!   reused. The difference is the part of the route the opening costs.
//!
//! It prints durations and counts only: no document identifier, no text, no
//! filter value (AGENTS.md, "Confidentialité"). The identifiers it reads come
//! from a JSON array written by the campaign's HTTP client, so the harness
//! measures the documents the route really serves.
//!
//! ```text
//! server_costs walk     --directory <dir> [--rounds 5] [--out <file>]
//! server_costs document --records <records.sqlite> --docs <json> \
//!     [--rounds 3] [--out <file>]
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crible_store::records::ReadPool;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().collect();
    let mode = arguments
        .get(1)
        .ok_or("usage: server_costs <walk|document> …")?;
    match mode.as_str() {
        "walk" => walk(&arguments),
        "document" => document(&arguments),
        other => Err(format!("unknown mode {other:?}: expected `walk` or `document`").into()),
    }
}

/// Mode `walk`: the cost of `sizes.disk_bytes`, once per round.
fn walk(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let directory = PathBuf::from(required(arguments, "--directory")?);
    let rounds: usize = optional(arguments, "--rounds", 5_usize)?;
    let mut files = 0_usize;
    let mut bytes = 0_u64;
    // Microseconds, as every series this harness reports: one `summary` unit for
    // both modes, so a figure is never read in the wrong scale.
    let mut durations_us = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let started = Instant::now();
        let (counted_files, counted_bytes) = tree_size(&directory)?;
        durations_us.push(started.elapsed().as_secs_f64() * 1_000_000.0);
        files = counted_files;
        bytes = counted_bytes;
    }
    let summary = summary(&durations_us);
    println!(
        "walk: {files} files, {bytes} bytes, p50 {:.3} ms, p95 {:.3} ms",
        summary["p50_ms"].as_f64().unwrap_or_default(),
        summary["p95_ms"].as_f64().unwrap_or_default()
    );
    write_out(
        arguments,
        &serde_json::json!({
            "mode": "walk",
            "directory": directory.display().to_string(),
            "files": files,
            "bytes": bytes,
            "rounds": rounds,
            "durations_us": durations_us,
            "summary": summary,
        }),
    )
}

/// Mode `document`: fresh pool per document against one reused pool.
fn document(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let records = PathBuf::from(required(arguments, "--records")?);
    let docs_path = PathBuf::from(required(arguments, "--docs")?);
    let rounds: usize = optional(arguments, "--rounds", 3_usize)?;
    let docs: Vec<String> = serde_json::from_str(&fs::read_to_string(&docs_path)?)?;
    if docs.is_empty() {
        return Err("the document list is empty".into());
    }

    let mut fresh_us = Vec::with_capacity(rounds * docs.len());
    let mut open_us = Vec::with_capacity(rounds * docs.len());
    let mut reused_us = Vec::with_capacity(rounds * docs.len());
    let mut passages = 0_usize;
    for _ in 0..rounds {
        // Production: one `ReadPool::new` per request, then two reads. The
        // opening is split out so the route's share is named, not guessed.
        for doc_id in &docs {
            let started = Instant::now();
            let pool = ReadPool::new(&records, 1)?;
            open_us.push(started.elapsed().as_secs_f64() * 1_000_000.0);
            let started = Instant::now();
            let record = pool
                .read_document(doc_id)?
                .ok_or("a document of the list has no row in the epoch")?;
            let rows = pool.passages_of_document(&record.doc_id)?;
            fresh_us.push(started.elapsed().as_secs_f64() * 1_000_000.0);
            passages += rows.len();
        }
        // Control: one pool, verified once, then the same reads.
        let pool = ReadPool::new(&records, 1)?;
        pool.verify()?;
        for doc_id in &docs {
            let started = Instant::now();
            let record = pool
                .read_document(doc_id)?
                .ok_or("a document of the list has no row in the epoch")?;
            let _ = pool.passages_of_document(&record.doc_id)?;
            reused_us.push(started.elapsed().as_secs_f64() * 1_000_000.0);
        }
    }
    let fresh = summary(&fresh_us);
    let open = summary(&open_us);
    let reused = summary(&reused_us);
    println!(
        "document: {} ids, {rounds} rounds; fresh pool open p50 {:.3} µs, \
         read p50 {:.3} µs; reused pool read p50 {:.3} µs",
        docs.len(),
        open["p50_us"].as_f64().unwrap_or_default(),
        fresh["p50_us"].as_f64().unwrap_or_default(),
        reused["p50_us"].as_f64().unwrap_or_default()
    );
    write_out(
        arguments,
        &serde_json::json!({
            "mode": "document",
            "records": records.display().to_string(),
            "documents": docs.len(),
            "rounds": rounds,
            "passages_read": passages,
            "open_us": open_us,
            "fresh_read_us": fresh_us,
            "reused_read_us": reused_us,
            "open": open,
            "fresh_read": fresh,
            "reused_read": reused,
        }),
    )
}

/// Recursive size of `directory`, as `directory_size` computes it.
fn tree_size(directory: &Path) -> Result<(usize, u64), Box<dyn std::error::Error>> {
    let mut files = 0_usize;
    let mut bytes = 0_u64;
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        // `symlink_metadata` describes the link itself: the walk neither
        // descends into one nor counts its target.
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            let (nested_files, nested_bytes) = tree_size(&path)?;
            files += nested_files;
            bytes = bytes.saturating_add(nested_bytes);
        } else {
            files += 1;
            bytes = bytes.saturating_add(metadata.len());
        }
    }
    Ok((files, bytes))
}

/// p50, p95 and p99 of a series of microseconds, in microseconds and in
/// milliseconds. The series is not filtered: every call is in the artefact.
fn summary(values: &[f64]) -> serde_json::Value {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let percentile = |percent: f64| -> f64 {
        if sorted.is_empty() {
            return f64::NAN;
        }
        let position = (percent / 100.0) * (sorted.len() - 1) as f64;
        let lower = position.floor() as usize;
        let upper = position.ceil() as usize;
        sorted[lower] + (sorted[upper] - sorted[lower]) * (position - lower as f64)
    };
    serde_json::json!({
        "count": sorted.len(),
        "min_us": sorted.first().copied(),
        "max_us": sorted.last().copied(),
        "p50_us": percentile(50.0),
        "p95_us": percentile(95.0),
        "p99_us": percentile(99.0),
        "p50_ms": percentile(50.0) / 1000.0,
        "p95_ms": percentile(95.0) / 1000.0,
        "p99_ms": percentile(99.0) / 1000.0,
        "mean_us": sorted.iter().sum::<f64>() / sorted.len() as f64,
    })
}

/// Writes the JSON document and prints it; `--out` is optional but a campaign
/// passes it, so an artefact is never read back from a terminal scroll.
fn write_out(
    arguments: &[String],
    document: &serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let text = serde_json::to_string_pretty(document)?;
    match arguments
        .iter()
        .position(|argument| argument == "--out")
        .and_then(|index| arguments.get(index + 1))
    {
        None => println!("{text}"),
        Some(path) => fs::write(path, format!("{text}\n"))?,
    }
    Ok(())
}

fn required(arguments: &[String], name: &str) -> Result<String, String> {
    arguments
        .iter()
        .position(|argument| argument == name)
        .and_then(|index| arguments.get(index + 1))
        .cloned()
        .ok_or_else(|| format!("missing {name}"))
}

fn optional<T: std::str::FromStr>(
    arguments: &[String],
    name: &str,
    default: T,
) -> Result<T, String> {
    match arguments
        .iter()
        .position(|argument| argument == name)
        .and_then(|index| arguments.get(index + 1))
    {
        None => Ok(default),
        Some(value) => value
            .parse()
            .map_err(|_| format!("cannot read {name}: {value:?}")),
    }
}
