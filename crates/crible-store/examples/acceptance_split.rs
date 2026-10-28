// SPDX-License-Identifier: Apache-2.0

//! Measurement driver of `ACCELERER-RECETTE` (livrable a): it splits the step
//! `validation` of an attach into posts, on the Legifacile sample.
//!
//! This example is a **harness, not production code**. It changes nothing in
//! the engine: it opens the batch with the production reader, reads the
//! reference state with a read-only SQLite connection, and calls the production
//! encoder and token counter. It exists because `validation` is a single
//! duration in `AttachDurations` (`docs/04`), and the posts inside it (model
//! load, token counting, the batch passes, the draw, the 200 re-encodings, the
//! per-document plan) have no separate timer.
//!
//! # What it measures
//!
//! ## Mode `posts`
//!
//! One repetition replays, with the production calls, the sequence the attach
//! runs inside `validation`:
//!
//! | Post | Production code it replays |
//! |---|---|
//! | `open` | `BatchReader::open`: manifest, controls 4 and 5, checksums |
//! | `validate` | `BatchReader::validate`: controls 6 and 7, one full pass |
//! | `length_read` / `length_count` | control 8: passages read, then `TokenCounter::count_documents` per chunk |
//! | `draw_read` / `draw_work` | control 9 draw: passages read, then priority, text copy and vector decode of the kept rows |
//! | `plan_read` / `plan_sql` | `plan_documents`: documents read, then the `documents` row read of every document |
//! | `encode` / `cosine` | control 9 loop: `Model::encode(Side::Document, …)` on each drawn passage, then the cosine |
//!
//! The draw is a **replica** of `draw_acceptance_sample`: same blake3 priority
//! over `(batch_id digest, row index)`, same `acceptance_sample` cap, same
//! ordering by row index. The plan post is a **replica of the read only**: the
//! harness runs the same `SELECT … FROM documents WHERE doc_id = ?1`, but not
//! the record building, the counters nor `retire_passages`; what it misses is in
//! the residual, never guessed.
//!
//! The modes `load`, `posts` and `both` print no text, no vector component and
//! no document identifier: durations, counts and the number of bytes of the
//! drawn sample only (AGENTS.md, "Confidentialité").
//!
//! The mode `dump` is the one exception, and it refuses to run without `--out`:
//! it writes the texts of the drawn passages into a file under `data/`
//! (gitignored), for a harness that re-encodes them outside this crate. It never
//! prints them, so a missing `--out` cannot send them to a terminal or a log.
//!
//! ## Mode `load`
//!
//! Loads the fp32 graph `--repetitions` times in one process. Run once per
//! repetition from a fresh process to get the cold load of an attach, and once
//! to get the warm reload the repetitions 1..n of a bench process pay.
//!
//! # Usage
//!
//! ## Mode `both`
//!
//! Same process, same repetition: the posts replica and the real `attach` on a
//! clone of the index, one after the other. It isolates the part of `validation`
//! that no timer of the engine reports: if the two runs of one repetition differ
//! on the same machine, the difference is inside `attach`, not in the ambient
//! load.
//!
//! ```text
//! acceptance_split load  --model <dir> [--repetitions 1] [--out <file>]
//! acceptance_split posts --index <dir> --models <dir> --batch <dir> \
//!     [--repetitions 5] [--out <file>]
//! acceptance_split both  --index <dir> --models <dir> --batch <dir> \
//!     --work <dir> [--repetitions 3] [--after 0] [--out <file>]
//! acceptance_split dump  --index <dir> --models <dir> --batch <dir> --out <file>
//! ```
//!
//! `ORT_DYLIB_PATH` must name the ONNX Runtime library, as for every command
//! that loads an `onnx` model (`tools/README.md`).

use std::collections::BinaryHeap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use arrow::array::{Array, FixedSizeBinaryArray, StringArray};
use crible_core::config::{IndexConfig, ReferencePrecision};
use crible_core::model::{ModelConfig, ModelKind};
use crible_encode::{Model, Precision, Side, TokenCounter};
use crible_store::attach::attach;
use crible_store::batch_reader::BatchReader;
use crible_store::generation::GenerationStore;
use half::f16;
use serde_json::{Value, json};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().collect();
    let mode = arguments.get(1).ok_or("missing mode: `load` or `posts`")?;
    match mode.as_str() {
        "load" => load(&arguments),
        "posts" => posts(&arguments),
        "both" => both(&arguments),
        "dump" => dump(&arguments),
        other => Err(
            format!("unknown mode {other:?}: expected `load`, `posts`, `both` or `dump`").into(),
        ),
    }
}

/// Mode `load`: times `--repetitions` loads of the fp32 graph in this process.
fn load(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let model = PathBuf::from(required(arguments, "--model")?);
    let repetitions = optional(arguments, "--repetitions", 1_usize)?;
    if repetitions == 0 {
        return Err("--repetitions must be positive".into());
    }
    let mut values = Vec::with_capacity(repetitions);
    for repetition in 0..repetitions {
        let started = Instant::now();
        let loaded = Model::load_precision(&model, Precision::Fp32)?;
        let elapsed = millis(started.elapsed());
        drop(loaded);
        println!("load {repetition}: {elapsed:.1} ms");
        values.push(elapsed);
    }
    let report = json!({
        "mode": "load",
        "model_dir": model.display().to_string(),
        "precision": "fp32",
        "load_ms": values,
    });
    write_out(arguments, &report)
}

/// Mode `posts`: replays and times the posts of `validation`, repetition by
/// repetition, on the batch given.
fn posts(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let index = PathBuf::from(required(arguments, "--index")?);
    let models = PathBuf::from(required(arguments, "--models")?);
    let batch = PathBuf::from(required(arguments, "--batch")?);
    let repetitions = optional(arguments, "--repetitions", 5_usize)?;
    if repetitions == 0 {
        return Err("--repetitions must be positive".into());
    }
    let config = read_config(&index)?;
    let model_config = read_model_config(&models, &config)?;
    let model_dir = models.join(&config.model);
    if model_config.kind != ModelKind::Onnx {
        return Err(format!(
            "model {:?} is not an onnx model: `validation` has no acceptance check to split",
            config.model
        )
        .into());
    }
    let counter = TokenCounter::load_with_config(&model_dir, &model_config)?;
    let sample = usize::try_from(config.ingest.acceptance_sample)?;
    let epoch = index.join("epochs").join(read_current_epoch(&index)?);
    let records = epoch.join("records.sqlite");

    // The graph is loaded once, before the timed repetitions, and the load is
    // measured separately: it is the first load of the process, as in a CLI
    // `crible attach`. The repetitions then reuse the warm session, as the
    // repetitions 1..n of the attach bench do.
    let started = Instant::now();
    let encoder = Model::load_precision(&model_dir, Precision::Fp32)?;
    let load_ms = millis(started.elapsed());

    let mut points = Vec::with_capacity(repetitions);
    for repetition in 0..repetitions {
        let mut point = one_repetition(
            &batch,
            &records,
            &config,
            &model_config,
            &counter,
            &encoder,
            sample,
        )?;
        if let Some(object) = point.as_object_mut() {
            object.insert("repetition".to_owned(), json!(repetition));
        }
        points.push(point);
    }

    let report = json!({
        "mode": "posts",
        "index": index.display().to_string(),
        "index_json": {
            "model": config.model,
            "reference_precision": format!("{:?}", config.reference_precision),
            "acceptance_sample": config.ingest.acceptance_sample,
            "acceptance_min_cosine": config.ingest.acceptance_min_cosine,
        },
        "batch": batch.display().to_string(),
        "model_dir": model_dir.display().to_string(),
        "model_load_fp32_ms": load_ms,
        "machine_threads": std::thread::available_parallelism().map_or(0, |n| n.get()),
        "points": points,
        "summary": summary_of(&points),
    });
    write_out(arguments, &report)
}

/// Mode `dump`: writes the drawn sample of one batch, with its texts and token
/// lengths, so that a harness outside this crate can re-encode them with any
/// batch size without reading Parquet again.
fn dump(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    // The sample carries the texts of the corpus: a missing `--out` would print
    // them on the standard output, which a shell redirection turns into a log
    // (AGENTS.md, "Confidentialité").
    if !arguments.iter().any(|argument| argument == "--out") {
        return Err(
            "dump writes the drawn texts and needs --out (a file under data/, gitignored)".into(),
        );
    }
    let index = PathBuf::from(required(arguments, "--index")?);
    let models = PathBuf::from(required(arguments, "--models")?);
    let batch = PathBuf::from(required(arguments, "--batch")?);
    let config = read_config(&index)?;
    let model_config = read_model_config(&models, &config)?;
    let model_dir = models.join(&config.model);
    let counter = TokenCounter::load_with_config(&model_dir, &model_config)?;
    let sample = usize::try_from(config.ingest.acceptance_sample)?;
    let reader = BatchReader::open(&batch, &config, &model_config)?;
    let batch_id = reader.manifest().batch_id.clone();
    let precision = config.reference_precision;

    let seed = blake3::hash(batch_id.as_bytes());
    let mut heap: BinaryHeap<(u64, u64)> = BinaryHeap::with_capacity(sample);
    let mut drawn: Vec<(u64, String)> = Vec::with_capacity(sample);
    let mut position = 0_u64;
    let mut chunks = reader.passages();
    loop {
        let Some(chunk) = chunks.next() else {
            break;
        };
        let chunk = chunk?;
        let texts = string_column(&chunk, "text")?;
        for row in 0..chunk.num_rows() {
            let priority = acceptance_priority(seed.as_bytes(), position);
            let kept_position = position;
            position = position.saturating_add(1);
            if let Some(worst) = heap.peek()
                && heap.len() >= sample
                && (priority, kept_position) >= *worst
            {
                continue;
            }
            if heap.len() == sample {
                heap.pop();
            }
            heap.push((priority, kept_position));
            drawn.push((kept_position, texts.value(row).to_owned()));
        }
    }
    let kept: std::collections::HashSet<u64> = heap.iter().map(|(_, kept)| *kept).collect();
    let mut rows: Vec<(u64, String)> = drawn
        .into_iter()
        .filter(|(kept_position, _)| kept.contains(kept_position))
        .collect();
    rows.sort_by_key(|(kept_position, _)| *kept_position);

    let mut sample_rows = Vec::with_capacity(rows.len());
    let mut tokens_total = 0_usize;
    for (kept_position, text) in rows {
        let tokens = counter.count_document(&text)?;
        tokens_total = tokens_total.saturating_add(tokens);
        sample_rows.push(json!({
            "position": kept_position,
            "tokens": tokens,
            "text": text,
        }));
    }
    let report = json!({
        "mode": "dump",
        "batch": batch.display().to_string(),
        "batch_id": batch_id,
        "precision": format!("{precision:?}"),
        "acceptance_sample": sample,
        "sample": sample_rows.len(),
        "tokens_total": tokens_total,
        "passages": sample_rows,
    });
    write_out(arguments, &report)
}

/// Mode `both`: the posts replica and the real `attach`, one after the other in
/// the same process, repetition by repetition, on a clone of the index.
fn both(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let index = PathBuf::from(required(arguments, "--index")?);
    let models = PathBuf::from(required(arguments, "--models")?);
    let batch = PathBuf::from(required(arguments, "--batch")?);
    let work = PathBuf::from(required(arguments, "--work")?);
    let repetitions = optional(arguments, "--repetitions", 3_usize)?;
    let after = optional(arguments, "--after", 0_usize)?;
    if repetitions == 0 {
        return Err("--repetitions must be positive".into());
    }
    let config = read_config(&index)?;
    let model_config = read_model_config(&models, &config)?;
    let model_dir = models.join(&config.model);
    let counter = TokenCounter::load_with_config(&model_dir, &model_config)?;
    let sample = usize::try_from(config.ingest.acceptance_sample)?;
    let records = index
        .join("epochs")
        .join(read_current_epoch(&index)?)
        .join("records.sqlite");
    fs::create_dir_all(&work)?;
    let label = index.file_name().map_or_else(
        || "index".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );

    let mut points = Vec::with_capacity(repetitions);
    for repetition in 0..repetitions {
        let clone = work.join(format!("{label}-both-{repetition}"));
        remove_directory(&clone)?;
        clone_directory(&index, &clone)?;

        // The posts, with an encoder loaded for this repetition only.
        let load_started = Instant::now();
        let encoder = Model::load_precision(&model_dir, Precision::Fp32)?;
        let posts_load_ms = millis(load_started.elapsed());
        let mut point = one_repetition(
            &batch,
            &records,
            &config,
            &model_config,
            &counter,
            &encoder,
            sample,
        )?;
        drop(encoder);

        // The real attach, on the clone, with the lazy fp32 load its caller
        // gives it; the load is timed as it happens inside `validation`.
        let mut store = GenerationStore::open(&clone, &config.schema)?;
        let load_ms = std::cell::Cell::new(0.0_f64);
        let load_encoder = || {
            let started = Instant::now();
            let model = Model::load_precision(&model_dir, Precision::Fp32)?;
            load_ms.set(millis(started.elapsed()));
            Ok(std::sync::Arc::new(model))
        };
        let document_encoder: Option<&dyn Fn() -> Result<std::sync::Arc<Model>, _>> =
            match model_config.kind {
                ModelKind::External => None,
                ModelKind::Onnx => Some(&load_encoder),
            };
        let started = Instant::now();
        let report = attach(
            &mut store,
            &batch,
            &config,
            &model_config,
            Some(&counter),
            document_encoder,
        )?;
        let attach_total_ms = millis(started.elapsed());
        let durations = report
            .durations
            .ok_or("the attach report carries no durations")?;
        let validation_ms = millis(durations.validation);
        drop(store);
        remove_directory(&clone)?;

        let acceptance = report.acceptance;
        // Replica again, after the attach, in the same process: if it is as slow
        // as the attach's `validation`, the difference was the machine; if it
        // stays fast, the difference is inside `attach`.
        let mut after_points = Vec::with_capacity(after);
        for _ in 0..after {
            let load_started = Instant::now();
            let encoder = Model::load_precision(&model_dir, Precision::Fp32)?;
            let after_load_ms = millis(load_started.elapsed());
            let mut replica = one_repetition(
                &batch,
                &records,
                &config,
                &model_config,
                &counter,
                &encoder,
                sample,
            )?;
            if let Some(object) = replica.as_object_mut() {
                object.insert("load_fp32_ms".to_owned(), json!(after_load_ms));
            }
            after_points.push(replica);
        }
        let posts_total_ms = point.get("total_ms").and_then(Value::as_f64).unwrap_or(0.0);
        let sum = posts_total_ms + load_ms.get();
        if let Some(object) = point.as_object_mut() {
            object.insert("repetition".to_owned(), json!(repetition));
            object.insert("posts_load_fp32_ms".to_owned(), json!(posts_load_ms));
            object.insert("attach_load_fp32_ms".to_owned(), json!(load_ms.get()));
            object.insert("attach_validation_ms".to_owned(), json!(validation_ms));
            object.insert("attach_total_ms".to_owned(), json!(attach_total_ms));
            object.insert("sum_posts_plus_load_ms".to_owned(), json!(sum));
            object.insert("residual_ms".to_owned(), json!(validation_ms - sum));
            object.insert("after_posts".to_owned(), json!(after_points));
            object.insert(
                "attach_acceptance".to_owned(),
                match acceptance {
                    None => Value::Null,
                    Some(report) => json!({
                        "sample": report.sample,
                        "min_cosine": report.min_cosine,
                        "p1": report.p1,
                        "p50": report.p50,
                    }),
                },
            );
        }
        points.push(point);
    }

    let report = json!({
        "mode": "both",
        "index": index.display().to_string(),
        "batch": batch.display().to_string(),
        "points": points,
    });
    write_out(arguments, &report)
}

/// One repetition: every post of `validation`, timed one after the other.
fn one_repetition(
    batch: &Path,
    records: &Path,
    config: &IndexConfig,
    model_config: &ModelConfig,
    counter: &TokenCounter,
    encoder: &Model,
    sample: usize,
) -> Result<Value, Box<dyn std::error::Error>> {
    let started = Instant::now();
    let open_started = Instant::now();
    let reader = BatchReader::open(batch, config, model_config)?;
    let open_ms = millis(open_started.elapsed());
    let passages = reader.manifest().counts.passages;
    let documents = reader.manifest().counts.documents;
    let batch_id = reader.manifest().batch_id.clone();

    let validate_started = Instant::now();
    let counts = reader.validate()?;
    let validate_ms = millis(validate_started.elapsed());
    if counts.passages != passages || counts.documents != documents {
        return Err("the validated counts differ from the manifest".into());
    }

    // Control 8: the passages are read again and their texts counted, chunk by
    // chunk, exactly as `check_passage_lengths` does.
    let length_started = Instant::now();
    let mut length_read = Duration::ZERO;
    let mut length_count = Duration::ZERO;
    let mut counted = 0_u64;
    let mut passages_chunks = reader.passages();
    loop {
        let chunk_started = Instant::now();
        let Some(chunk) = passages_chunks.next() else {
            break;
        };
        length_read += chunk_started.elapsed();
        let chunk = chunk?;
        let texts = string_column(&chunk, "text")?;
        let values: Vec<&str> = (0..chunk.num_rows()).map(|row| texts.value(row)).collect();
        let count_started = Instant::now();
        let counts = counter.count_documents(&values)?;
        length_count += count_started.elapsed();
        counted = counted.saturating_add(u64::try_from(counts.len())?);
    }
    let length_ms = millis(length_started.elapsed());

    // Control 9 draw: same priority, same cap, same order as
    // `draw_acceptance_sample`; the texts and the vectors of the kept rows only.
    let draw_started = Instant::now();
    let seed = blake3::hash(batch_id.as_bytes());
    // A max-heap on `(priority, position)`: its top is the worst kept row, as in
    // `draw_acceptance_sample`.
    let mut heap: BinaryHeap<(u64, u64)> = BinaryHeap::with_capacity(sample);
    let mut drawn: Vec<(u64, String, Vec<f32>)> = Vec::with_capacity(sample);
    let mut draw_read = Duration::ZERO;
    let mut draw_work = Duration::ZERO;
    let mut index = 0_u64;
    let precision = config.reference_precision;
    let mut draw_chunks = reader.passages();
    loop {
        let chunk_started = Instant::now();
        let Some(chunk) = draw_chunks.next() else {
            break;
        };
        draw_read += chunk_started.elapsed();
        let chunk = chunk?;
        let texts = string_column(&chunk, "text")?;
        let vectors = fixed_binary_column(&chunk, "vector")?;
        let work_started = Instant::now();
        for row in 0..chunk.num_rows() {
            let priority = acceptance_priority(seed.as_bytes(), index);
            let position = index;
            index = index.saturating_add(1);
            if let Some(worst) = heap.peek()
                && heap.len() >= sample
                && (priority, position) >= *worst
            {
                continue;
            }
            let text = texts.value(row).to_owned();
            let vector = decode_vector(vectors.value(row), precision)?;
            if heap.len() == sample {
                heap.pop();
            }
            heap.push((priority, position));
            drawn.push((position, text, vector));
        }
        draw_work += work_started.elapsed();
    }
    // The heap keeps the same rows as the production draw; the replica stores
    // every kept candidate, so it drops the rows that were later evicted.
    let kept: std::collections::HashSet<u64> = heap.iter().map(|(_, position)| *position).collect();
    let mut sample_rows: Vec<(u64, String, Vec<f32>)> = drawn
        .into_iter()
        .filter(|(position, _, _)| kept.contains(position))
        .collect();
    sample_rows.sort_by_key(|(position, _, _)| *position);
    let draw_ms = millis(draw_started.elapsed());
    let sample_bytes: usize = sample_rows.iter().map(|(_, text, _)| text.len()).sum();
    // Token length of each drawn passage, with the production counter: it is
    // what decides the padding of a length-sorted batch (prompt included).
    let mut lengths: Vec<usize> = Vec::with_capacity(sample_rows.len());
    for (_, text, _) in &sample_rows {
        lengths.push(counter.count_document(text)?);
    }
    lengths.sort_unstable();
    let length_of = |percent: usize| -> usize {
        if lengths.is_empty() {
            return 0;
        }
        let rank = (percent * lengths.len()).div_ceil(100).max(1);
        lengths
            .get(rank.min(lengths.len()) - 1)
            .copied()
            .unwrap_or(0)
    };

    // `plan_documents`: the documents pass, then the read of one row per
    // document. Replica of the read only; see the module documentation.
    let plan_started = Instant::now();
    let connection =
        rusqlite::Connection::open_with_flags(records, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut statement =
        connection.prepare("SELECT doc_hash, fields FROM documents WHERE doc_id = ?1")?;
    let mut plan_read = Duration::ZERO;
    let mut plan_sql = Duration::ZERO;
    let mut planned = 0_u64;
    let mut found = 0_u64;
    let mut document_chunks = reader.documents();
    loop {
        let chunk_started = Instant::now();
        let Some(chunk) = document_chunks.next() else {
            break;
        };
        plan_read += chunk_started.elapsed();
        let chunk = chunk?;
        let ids = string_column(&chunk, "doc_id")?;
        let sql_started = Instant::now();
        for row in 0..chunk.num_rows() {
            let doc_id = ids.value(row);
            let mut rows = statement.query([doc_id])?;
            if rows.next()?.is_some() {
                found = found.saturating_add(1);
            }
            planned = planned.saturating_add(1);
        }
        plan_sql += sql_started.elapsed();
    }
    let plan_ms = millis(plan_started.elapsed());
    drop(statement);
    drop(connection);

    // Control 9 loop: one `encode` per drawn passage, then the cosine with the
    // vector the batch stores.
    let encode_started = Instant::now();
    let mut encode = Duration::ZERO;
    let mut cosine_time = Duration::ZERO;
    let mut cosines: Vec<f64> = Vec::with_capacity(sample_rows.len());
    for (_, text, vector) in &sample_rows {
        let started = Instant::now();
        let reencoded = encoder.encode(Side::Document, text)?;
        encode += started.elapsed();
        let cosine_started = Instant::now();
        cosines.push(cosine(vector, &reencoded, &batch_id)?);
        cosine_time += cosine_started.elapsed();
    }
    cosines.sort_by(f64::total_cmp);
    let minimum = cosines.first().copied().unwrap_or(f64::NAN);
    let p1 = percentile(&cosines, 1);
    let p50 = percentile(&cosines, 50);
    let encode_ms = millis(encode_started.elapsed());

    Ok(json!({
        "total_ms": millis(started.elapsed()),
        "passages": passages,
        "documents": documents,
        "batch_id": batch_id,
        "open_ms": open_ms,
        "validate_ms": validate_ms,
        "length_ms": length_ms,
        "length_read_ms": millis(length_read),
        "length_count_ms": millis(length_count),
        "length_rows": counted,
        "draw_ms": draw_ms,
        "draw_read_ms": millis(draw_read),
        "draw_work_ms": millis(draw_work),
        "sample": sample_rows.len(),
        "sample_text_bytes": sample_bytes,
        "sample_tokens_total": lengths.iter().sum::<usize>(),
        "sample_tokens_p50": length_of(50),
        "sample_tokens_p90": length_of(90),
        "sample_tokens_max": lengths.last().copied().unwrap_or(0),
        "sample_tokens": lengths,
        "plan_ms": plan_ms,
        "plan_read_ms": millis(plan_read),
        "plan_sql_ms": millis(plan_sql),
        "plan_rows": planned,
        "plan_found": found,
        "encode_ms": encode_ms,
        "encode_loop_ms": encode_ms,
        "encode_only_ms": millis(encode - cosine_time),
        "cosine_ms": millis(cosine_time),
        "min_cosine": minimum,
        "p1": p1,
        "p50": p50,
    }))
}

/// Summary of the repetitions: p50, min and max of every duration, in
/// milliseconds.
fn summary_of(points: &[Value]) -> Value {
    let mut keys: Vec<&str> = Vec::new();
    for point in points {
        if let Some(object) = point.as_object() {
            for key in object.keys() {
                if key.ends_with("_ms") && !keys.contains(&key.as_str()) {
                    keys.push(key.as_str());
                }
            }
        }
    }
    let mut summary = serde_json::Map::new();
    for key in keys {
        let mut values: Vec<f64> = points
            .iter()
            .filter_map(|point| point.get(key))
            .filter_map(Value::as_f64)
            .collect();
        values.sort_by(f64::total_cmp);
        let Some(minimum) = values.first().copied() else {
            continue;
        };
        let maximum = values.last().copied().unwrap_or(minimum);
        let middle = values.get(values.len() / 2).copied().unwrap_or(minimum);
        summary.insert(
            key.to_owned(),
            json!({"p50": middle, "min": minimum, "max": maximum}),
        );
    }
    Value::Object(summary)
}

/// Priority of one batch row in the acceptance draw, exactly as
/// `acceptance_priority` computes it.
fn acceptance_priority(seed: &[u8; 32], index: u64) -> u64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(seed);
    hasher.update(&index.to_le_bytes());
    let digest = hasher.finalize();
    let mut head = [0_u8; 8];
    head.copy_from_slice(&digest.as_bytes()[..8]);
    u64::from_le_bytes(head)
}

/// Decodes a stored vector exactly as `decode_vector` does.
fn decode_vector(
    bytes: &[u8],
    precision: ReferencePrecision,
) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
    let width = match precision {
        ReferencePrecision::Fp16 => 2,
        ReferencePrecision::Fp32 => 4,
    };
    if !bytes.len().is_multiple_of(width) {
        return Err("a stored vector is not a multiple of its component width".into());
    }
    Ok(bytes
        .chunks_exact(width)
        .map(|component| match precision {
            ReferencePrecision::Fp16 => {
                let raw: [u8; 2] = [component[0], component[1]];
                f32::from(f16::from_bits(u16::from_le_bytes(raw)))
            }
            ReferencePrecision::Fp32 => {
                let raw: [u8; 4] = [component[0], component[1], component[2], component[3]];
                f32::from_le_bytes(raw)
            }
        })
        .collect())
}

/// Nearest-rank percentile of a sorted slice, the convention of the attach.
fn percentile(sorted: &[f64], percent: u32) -> f64 {
    let len = sorted.len();
    if len == 0 {
        return f64::NAN;
    }
    let scaled = u64::from(percent) * u64::try_from(len).unwrap_or(u64::MAX);
    let rank = usize::try_from(scaled.div_ceil(100)).unwrap_or(len).max(1);
    sorted.get(rank.min(len) - 1).copied().unwrap_or(f64::NAN)
}

/// Cosine of two vectors, computed in `f64` as `attach::cosine` does.
fn cosine(left: &[f32], right: &[f32], batch_id: &str) -> Result<f64, Box<dyn std::error::Error>> {
    if left.len() != right.len() {
        return Err(format!(
            "vectors of {} and {} components in batch '{batch_id}'",
            left.len(),
            right.len()
        )
        .into());
    }
    let mut dot = 0.0_f64;
    let mut left_norm = 0.0_f64;
    let mut right_norm = 0.0_f64;
    for (left, right) in left.iter().zip(right) {
        dot += f64::from(*left) * f64::from(*right);
        left_norm += f64::from(*left) * f64::from(*left);
        right_norm += f64::from(*right) * f64::from(*right);
    }
    let cosine = dot / (left_norm.sqrt() * right_norm.sqrt());
    if !cosine.is_finite() {
        return Err("a sampled vector has no cosine with its re-encoding".into());
    }
    Ok(cosine.clamp(-1.0, 1.0))
}

fn string_column<'a>(
    chunk: &'a arrow::record_batch::RecordBatch,
    name: &str,
) -> Result<&'a StringArray, Box<dyn std::error::Error>> {
    chunk
        .column_by_name(name)
        .ok_or_else(|| format!("the chunk carries no `{name}` column"))?
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| format!("the `{name}` column is not UTF-8"))
        .map_err(Into::into)
}

fn fixed_binary_column<'a>(
    chunk: &'a arrow::record_batch::RecordBatch,
    name: &str,
) -> Result<&'a FixedSizeBinaryArray, Box<dyn std::error::Error>> {
    chunk
        .column_by_name(name)
        .ok_or_else(|| format!("the chunk carries no `{name}` column"))?
        .as_any()
        .downcast_ref::<FixedSizeBinaryArray>()
        .ok_or_else(|| format!("the `{name}` column is not fixed-size binary"))
        .map_err(Into::into)
}

/// Reads `index.json` of an index directory.
fn read_config(index: &Path) -> Result<IndexConfig, Box<dyn std::error::Error>> {
    let raw = fs::read_to_string(index.join("index.json"))?;
    Ok(IndexConfig::from_json_str(&raw)?)
}

/// Reads `model.json` of the model the index declares.
fn read_model_config(
    models: &Path,
    config: &IndexConfig,
) -> Result<ModelConfig, Box<dyn std::error::Error>> {
    let raw = fs::read_to_string(models.join(&config.model).join("model.json"))?;
    Ok(ModelConfig::from_json_str(&raw)?)
}

/// Reads `CURRENT_EPOCH` of an index directory.
fn read_current_epoch(index: &Path) -> Result<String, Box<dyn std::error::Error>> {
    Ok(fs::read_to_string(index.join("CURRENT_EPOCH"))?
        .trim()
        .to_owned())
}

/// Clones the index with APFS `cp -c`, falling back to `cp -R` (MES-08 protocol).
fn clone_directory(source: &Path, target: &Path) -> Result<(), Box<dyn std::error::Error>> {
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
    if !copied.success() {
        return Err(format!(
            "cannot clone {} into {}",
            source.display(),
            target.display()
        )
        .into());
    }
    Ok(())
}

/// Removes a clone left by a repetition.
fn remove_directory(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if path.exists() {
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn write_out(arguments: &[String], report: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = serde_json::to_string_pretty(report)?;
    match arguments.iter().position(|argument| argument == "--out") {
        None => println!("{pretty}"),
        Some(position) => {
            let out = arguments.get(position + 1).ok_or("--out needs a value")?;
            if let Some(parent) = Path::new(out).parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(out, pretty)?;
            println!("written {out}");
        }
    }
    Ok(())
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

/// Returns the value following `name`, or `default` when the flag is absent.
fn optional<T: std::str::FromStr>(
    arguments: &[String],
    name: &str,
    default: T,
) -> Result<T, Box<dyn std::error::Error>>
where
    T::Err: std::error::Error + 'static,
{
    match arguments.iter().position(|argument| argument == name) {
        None => Ok(default),
        Some(position) => {
            let raw = arguments
                .get(position + 1)
                .ok_or_else(|| format!("{name} needs a value"))?;
            Ok(raw.parse()?)
        }
    }
}
