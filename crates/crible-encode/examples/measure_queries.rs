// SPDX-License-Identifier: Apache-2.0

//! Measurement driver of MES-04 and MES-05.
//!
//! This example is a harness, not production code. It drives
//! [`crible_encode::QueryEncoder`] over a JSONL list of queries and either
//! writes the vectors or times the encoder. It exists so that MES-04 (parity)
//! and MES-05 (latency) measure the encoder the engine actually runs, with its
//! prompt, tokenizer, truncation and normalization, instead of a replica.
//!
//! ```text
//! measure_queries vectors --model <dir> --queries <jsonl> --out-dir <dir>
//! measure_queries latency --model <dir> --queries <jsonl> --precision int8 \
//!     --warmup 300 --count 1000 --out <json>
//! ```
//!
//! The JSONL file holds one `{"id": "...", "text": "..."}` object per line. The
//! query text never reaches a log: the harness prints no query, only counts and
//! durations (AGENTS.md, "Confidentialité").
//!
//! The query cache is **disabled** (capacity 0) in both modes. A latency
//! measurement must time the encoder, not a cache hit, and a vectors run must
//! encode every query even when two of them share a text.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crible_encode::{ModelRegistry, Precision, QueryEncoder};

/// One input query: an opaque identifier and its text.
struct Query {
    id: String,
    text: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().collect();
    let mode = arguments
        .get(1)
        .ok_or("usage: measure_queries <vectors|latency> --model <dir> --queries <jsonl>")?;
    let model = PathBuf::from(required(&arguments, "--model")?);
    let queries_path = PathBuf::from(required(&arguments, "--queries")?);
    let queries = read_queries(&queries_path)?;
    if queries.is_empty() {
        return Err("the query file is empty".into());
    }

    match mode.as_str() {
        "vectors" => {
            let out_dir = PathBuf::from(required(&arguments, "--out-dir")?);
            write_vectors(&model, &queries, &out_dir)?;
        }
        "latency" => {
            let precision = match required(&arguments, "--precision")?.as_str() {
                "int8" => Precision::Int8,
                "fp32" => Precision::Fp32,
                other => return Err(format!("unknown precision '{other}'").into()),
            };
            let count: usize = required(&arguments, "--count")?.parse()?;
            let warmup: usize = required(&arguments, "--warmup")?.parse()?;
            let out = PathBuf::from(required(&arguments, "--out")?);
            write_latency(&model, &queries, precision, warmup, count, &out)?;
        }
        other => return Err(format!("unknown mode '{other}'").into()),
    }
    Ok(())
}

/// Wire name of a precision, as written in the JSON artifacts.
fn precision_name(precision: Precision) -> &'static str {
    match precision {
        Precision::Int8 => "int8",
        Precision::Fp32 => "fp32",
    }
}

/// Returns the value following `name` on the command line.
fn required(arguments: &[String], name: &str) -> Result<String, String> {
    let position = arguments
        .iter()
        .position(|argument| argument == name)
        .ok_or_else(|| format!("missing {name}"))?;
    arguments
        .get(position + 1)
        .cloned()
        .ok_or_else(|| format!("{name} needs a value"))
}

/// Reads `{"id": "...", "text": "..."}` lines.
fn read_queries(path: &Path) -> Result<Vec<Query>, Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let mut queries = Vec::new();
    for (line_number, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(&line)?;
        let id = value
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("line {} has no string 'id'", line_number + 1))?;
        let text = value
            .get("text")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("line {} has no string 'text'", line_number + 1))?;
        queries.push(Query {
            id: id.to_owned(),
            text: text.to_owned(),
        });
    }
    Ok(queries)
}

/// Encodes every query in both precisions and writes the vectors.
///
/// The two files are row-major little-endian `f32`, one row per query, in the
/// order of the input file. The sidecar JSON carries the order, the truncation
/// flags and the precision actually served for each matrix (`precision_effective`,
/// the same key as the latency mode), so a reader never has to guess which row is
/// which query or which graph produced it.
///
/// The two modes write that key with different shapes on purpose: the vectors
/// mode produces two matrices in one artifact, so `precision_effective` is an
/// object keyed by `int8` and `fp32`; the latency mode measures one matrix and
/// writes a string. The meaning is the same in both: the precision the encoder
/// actually served, never the one that was requested.
fn write_vectors(
    model: &Path,
    queries: &[Query],
    out_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(out_dir)?;
    let registry = Arc::new(ModelRegistry::new());
    let encoder = QueryEncoder::new(model, registry, 0);

    let mut int8 = Vec::with_capacity(queries.len());
    let mut fp32 = Vec::with_capacity(queries.len());
    let mut truncated_int8 = Vec::with_capacity(queries.len());
    let mut truncated_fp32 = Vec::with_capacity(queries.len());
    let mut effective_int8 = Precision::Int8;
    let mut effective_fp32 = Precision::Fp32;

    for query in queries {
        let encoded = encoder.encode_query(&query.text, Precision::Int8)?;
        if encoded.precision != Precision::Int8 {
            return Err(format!(
                "the encoder served {} for a requested INT8 query; the model declares no INT8 \
                 graph, so the comparison would mix two precisions",
                precision_name(encoded.precision)
            )
            .into());
        }
        effective_int8 = encoded.precision;
        truncated_int8.push(encoded.truncated);
        int8.push(encoded.vector);
        let encoded = encoder.encode_query(&query.text, Precision::Fp32)?;
        if encoded.precision != Precision::Fp32 {
            return Err("the encoder served another precision for a requested fp32 query".into());
        }
        effective_fp32 = encoded.precision;
        truncated_fp32.push(encoded.truncated);
        fp32.push(encoded.vector);
    }

    let dims = int8.first().map_or(0, Vec::len);
    if dims == 0 || fp32.iter().any(|vector| vector.len() != dims) {
        return Err("the encoder returned inconsistent vector widths".into());
    }

    write_matrix(&out_dir.join("vectors-int8.f32"), &int8)?;
    write_matrix(&out_dir.join("vectors-fp32.f32"), &fp32)?;
    let stats = encoder.cache_stats()?;
    let meta = serde_json::json!({
        "queries": queries.len(),
        "dims": dims,
        "layout": "row-major little-endian f32, one row per input line",
        "ids": queries.iter().map(|query| query.id.as_str()).collect::<Vec<_>>(),
        "truncated_int8": truncated_int8,
        "truncated_fp32": truncated_fp32,
        "precision_effective": {
            "int8": precision_name(effective_int8),
            "fp32": precision_name(effective_fp32),
        },
        "cache_entries": stats.capacity,
        "cache_misses": stats.misses,
        "cache_hits": stats.hits,
    });
    fs::write(
        out_dir.join("rust-vectors.json"),
        serde_json::to_vec_pretty(&meta)?,
    )?;
    println!(
        "wrote {} queries of {dims} dimensions to {} (cache capacity {})",
        queries.len(),
        out_dir.display(),
        stats.capacity
    );
    Ok(())
}

/// Writes a matrix of `f32` rows in little-endian order.
fn write_matrix(path: &Path, rows: &[Vec<f32>]) -> Result<(), Box<dyn std::error::Error>> {
    let mut writer = BufWriter::new(File::create(path)?);
    for row in rows {
        for component in row {
            writer.write_all(&component.to_le_bytes())?;
        }
    }
    writer.flush()?;
    Ok(())
}

/// Times `count` encodings of `precision` after `warmup` encodings.
///
/// The first call of the process is timed on its own: it carries the hash
/// verification of the graph and the creation of the ONNX session, which a
/// serving process pays once and not per query. The measured calls then cycle
/// through the input queries; the cache is disabled, so no cycle is served from
/// memory.
fn write_latency(
    model: &Path,
    queries: &[Query],
    precision: Precision,
    warmup: usize,
    count: usize,
    out: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if count == 0 {
        return Err("count must be positive".into());
    }
    let registry = Arc::new(ModelRegistry::new());
    let encoder = QueryEncoder::new(model, registry, 0);

    let started = Instant::now();
    let first = encoder.encode_query(&queries[0].text, precision)?;
    let first_call_ms = started.elapsed().as_secs_f64() * 1_000.0;
    // A model that declares no INT8 graph falls back to fp32: timing another
    // graph than the requested one would publish MES-05 rows under a wrong
    // label. The MES-04 harness always uses a model that declares both.
    if first.precision != precision {
        return Err(format!(
            "the encoder served {} for a requested {} query; refuse to measure another graph",
            precision_name(first.precision),
            precision_name(precision)
        )
        .into());
    }
    let precision_effective = first.precision;

    for index in 0..warmup {
        let query = &queries[index % queries.len()];
        encoder.encode_query(&query.text, precision)?;
    }

    let mut calls_us = Vec::with_capacity(count);
    for index in 0..count {
        let query = &queries[index % queries.len()];
        let started = Instant::now();
        encoder.encode_query(&query.text, precision)?;
        calls_us.push(started.elapsed().as_secs_f64() * 1_000_000.0);
    }

    let stats = encoder.cache_stats()?;
    let payload = serde_json::json!({
        "precision": precision_name(precision),
        "precision_effective": precision_name(precision_effective),
        "model": model.display().to_string(),
        "queries": queries.len(),
        "warmup": warmup,
        "count": count,
        "first_call_ms": first_call_ms,
        "calls_us": calls_us,
        "cache_capacity": stats.capacity,
        "cache_hits": stats.hits,
        "cache_misses": stats.misses,
    });
    fs::write(out, serde_json::to_vec(&payload)?)?;
    println!(
        "timed {count} {:?} encodings (warmup {warmup}, first call {first_call_ms:.1} ms) -> {}",
        precision,
        out.display()
    );
    Ok(())
}
