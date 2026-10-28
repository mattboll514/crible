// SPDX-License-Identifier: Apache-2.0

//! MES-08 bench: duration of a "daily type" attach, stage by stage.
//!
//! References: `docs/08-invariants-et-tests.md` (MES-08),
//! `docs/04-donnees-et-ingestion.md` ("Sémantique d'attache", the report and the
//! five measured stages), `ADR-0048` (the reused zstd compressor) and
//! `MES-15` (the cost of a generation commit at 8,58 M live passages).
//!
//! # What is measured
//!
//! The real `crible_store::attach::attach`, on a clone of the target index,
//! with the model of that index. The production code already measures its six
//! stages (`AttachDurations`: `validation`, `segment`, `transaction`,
//! `lexical`, `manifest`, `snapshot`) and returns them as call telemetry
//! (ADR-0042, ADR-0087), so the bench only has to run the call and read the
//! report. The total wall time of the call is measured too, which is the sum of
//! the stages plus the small bookkeeping between them.
//!
//! On the batch itself, the bench also replays the compression of its texts two
//! ways: with one reused `zstd::bulk::Compressor` (what production does since
//! `REUTILISER-COMPRESSEUR-ZSTD`, ADR-0048) and with one `zstd::stream::
//! encode_all` context per text (what it did before). The difference is the
//! gain of the task on this batch, and it is a **replica**: the production
//! transaction times the same compression inside `transaction`.
//!
//! # Why clones
//!
//! An attach changes the index (generation G+1, a new segment, tombstones), so
//! repeating it on the same directory would change `N` and would replay the
//! same `batch_id` as a no-op (INV-10). Each repetition clones the base index
//! (APFS `cp -c`, falling back to `cp -R`).
//!
//! # Usage
//!
//! ```text
//! attach_cost measure --index <dir> --models <dir> --batch <dir> [--batch ...]
//!                     --work <dir> --repeats <n>
//!                     --commit <sha> --machine <label> --date <date>
//!                     --out <file.json>
//! ```
//!
//! `ORT_DYLIB_PATH` is needed when the index uses an `onnx` model, exactly as
//! for `crible attach` (`tools/README.md`).
//!
//! # Platform
//!
//! macOS only, for `cp -c`. The crate compiles the bench anywhere; CI does not
//! run it.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

use arrow::array::{Array, AsArray};
use crible_core::config::IndexConfig;
use crible_core::model::{ModelConfig, ModelKind};
use crible_encode::{Model, Precision, TokenCounter};
use crible_store::attach::attach;
use crible_store::batch_reader::BatchReader;
use crible_store::generation::GenerationStore;
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

/// Parsed command line: a mode and `--key value` options, repeated keys kept.
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

    fn all(&self, key: &str) -> Vec<&str> {
        self.values
            .get(key)
            .map(|values| values.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }

    fn number<T: FromStr>(&self, key: &str) -> Result<T, Box<dyn Error>>
    where
        T::Err: std::fmt::Display,
    {
        let raw = self.one(key)?;
        raw.parse::<T>()
            .map_err(|error| fail(format!("--{key} {raw:?}: {error}")))
    }
}

/// One measured batch: its repetitions and the summary of each stage.
fn measure(args: &Args) -> Result<(), Box<dyn Error>> {
    let index = PathBuf::from(args.one("index")?);
    let models = PathBuf::from(args.one("models")?);
    let work = PathBuf::from(args.one("work")?);
    let repeats: usize = args.number("repeats")?;
    let out = PathBuf::from(args.one("out")?);
    let batches = args.all("batch");
    if repeats == 0 || batches.is_empty() {
        return Err(fail("--repeats and --batch must be given"));
    }
    let config = read_config(&index)?;
    let model_config = read_model_config(&models, &config)?;
    let model_dir = models.join(&config.model);
    let counter = match model_config.kind {
        ModelKind::External => None,
        ModelKind::Onnx => Some(TokenCounter::load_with_config(&model_dir, &model_config)?),
    };
    fs::create_dir_all(&work)?;

    let mut sections = Vec::with_capacity(batches.len());
    for batch in &batches {
        let batch_dir = PathBuf::from(batch);
        let reader = BatchReader::open(&batch_dir, &config, &model_config)?;
        let batch_id = reader.manifest().batch_id.clone();
        let passages = reader.manifest().counts.passages;
        let documents = reader.manifest().counts.documents;
        let texts = batch_texts(&reader)?;
        let (compress_new_ms, compress_old_ms) = compression(&texts)?;

        let mut points = Vec::with_capacity(repeats);
        for repetition in 0..repeats {
            let clone_dir = work.join(format!("{batch_id}-rep-{repetition}"));
            remove_directory(&clone_dir)?;
            clone_directory(&index, &clone_dir)?;
            let mut store = GenerationStore::open(&clone_dir, &config.schema)?;
            let before = store.state()?;
            let load_encoder = || Model::load_precision(&model_dir, Precision::Fp32).map(Arc::new);
            let document_encoder: Option<&dyn Fn() -> Result<Arc<Model>, _>> =
                match model_config.kind {
                    ModelKind::External => None,
                    ModelKind::Onnx => Some(&load_encoder),
                };
            let started = Instant::now();
            let report = attach(
                &mut store,
                &batch_dir,
                &config,
                &model_config,
                counter.as_ref(),
                document_encoder,
            )?;
            let total_ms = millis(started.elapsed());
            let durations = report
                .durations
                .ok_or_else(|| fail("the attach report carries no durations"))?;
            let after = store.state()?;
            points.push(json!({
                "repetition": repetition,
                "generation": report.generation.get(),
                "live_before": before.live_passages,
                "live_after": after.live_passages,
                "total_ms": total_ms,
                "validation_ms": millis(durations.validation),
                "segment_ms": millis(durations.segment),
                "transaction_ms": millis(durations.transaction),
                "lexical_ms": millis(durations.lexical),
                "manifest_ms": millis(durations.manifest),
                "snapshot_ms": millis(durations.snapshot),
                "added": report.added,
                "replaced": report.replaced,
                "unchanged": report.unchanged,
                "deleted": report.deleted,
            }));
            drop(store);
            remove_directory(&clone_dir)?;
        }
        sections.push(json!({
            "batch": batch,
            "batch_id": batch_id,
            "passages": passages,
            "documents": documents,
            "text_bytes_mean": mean_text_bytes(&texts),
            "compress_new_ms": compress_new_ms,
            "compress_old_ms": compress_old_ms,
            "points": points,
            "total_ms": stats_of(&points, "total_ms"),
            "validation_ms": stats_of(&points, "validation_ms"),
            "segment_ms": stats_of(&points, "segment_ms"),
            "transaction_ms": stats_of(&points, "transaction_ms"),
            "manifest_ms": stats_of(&points, "manifest_ms"),
            "snapshot_ms": stats_of(&points, "snapshot_ms"),
        }));
    }

    let report = json!({
        "measure": "MES-08",
        "commit": args.one("commit")?,
        "machine": args.one("machine")?,
        "date": args.one("date")?,
        "index": index.display().to_string(),
        "model": config.model,
        "model_kind": format!("{:?}", model_config.kind),
        "repeats": repeats,
        "batches": sections,
    });
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&out, serde_json::to_string_pretty(&report)?)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

/// Texts of the batch, in row order, read through the production reader.
fn batch_texts(reader: &BatchReader) -> Result<Vec<String>, Box<dyn Error>> {
    let mut texts = Vec::new();
    for chunk in reader.passages() {
        let chunk = chunk?;
        let column = chunk
            .column_by_name("text")
            .ok_or_else(|| fail("the passage chunk carries no `text` column"))?;
        let values = column.as_string::<i32>();
        for row in 0..values.len() {
            if values.is_null(row) {
                return Err(fail("a passage row carries a null text"));
            }
            texts.push(values.value(row).to_owned());
        }
    }
    Ok(texts)
}

/// Compresses `texts` twice: reused context, then one context per text.
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

fn mean_text_bytes(texts: &[String]) -> f64 {
    if texts.is_empty() {
        return 0.0;
    }
    let total: usize = texts.iter().map(|text| text.len()).sum();
    total as f64 / texts.len() as f64
}

fn read_config(index: &Path) -> Result<IndexConfig, Box<dyn Error>> {
    let raw = fs::read_to_string(index.join("index.json"))?;
    Ok(IndexConfig::from_json_str(&raw)?)
}

fn read_model_config(models: &Path, config: &IndexConfig) -> Result<ModelConfig, Box<dyn Error>> {
    let raw = fs::read_to_string(models.join(&config.model).join("model.json"))?;
    Ok(ModelConfig::from_json_str(&raw)?)
}

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
