// SPDX-License-Identifier: Apache-2.0

//! Measurement driver of `CONTROLER-LONGUEURS` (control 8, INV-11).
//!
//! This example is a harness, not production code. It times the **second pass**
//! that the attach runs for its length control: the passage files are read again
//! in Arrow record batches of [`DEFAULT_BATCH_ROWS`] rows (all their columns, as
//! the reader of the batch does), the texts of one chunk are counted with the
//! real tokenizer, and the counts are reported; the attach compares each count
//! with `max_tokens`. It exists so that the cost of the control can be
//! reproduced on a real batch instead of being copied from a session.
//!
//! ```text
//! measure_lengths --passages <passages.parquet> --model <dir> \
//!     [--count 10000] [--repetitions 3]
//! ```
//!
//! `<dir>` is a model directory holding `model.json` and its tokenizer, for
//! example `data/models/mdenseon`. `<passages.parquet>` is one passage file of a
//! batch, for example
//! `data/legifacile-sample/batches/legifacile-sample-articles-v1/passages-0001.parquet`;
//! the harness reads the first `--count` rows of that file and stops.
//!
//! No path of the machine and no text is compiled into the harness or printed:
//! it reports durations, counts and the longest passage length only (AGENTS.md,
//! "Confidentialité"). The ONNX graph is never loaded: control 8 counts with the
//! tokenizer alone.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use arrow::array::{Array, StringArray};
use crible_encode::TokenCounter;
use crible_store::batch_reader::DEFAULT_BATCH_ROWS;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().collect();
    let passages = PathBuf::from(required(&arguments, "--passages")?);
    let model = PathBuf::from(required(&arguments, "--model")?);
    let count = optional(&arguments, "--count", 10_000)?;
    let repetitions = optional(&arguments, "--repetitions", 3)?;
    if count == 0 || repetitions == 0 {
        return Err("--count and --repetitions must be positive".into());
    }

    let started = Instant::now();
    let counter = TokenCounter::load(&model)?;
    let load = started.elapsed();
    println!(
        "tokenizer load: {:?} (max_tokens {})",
        load,
        counter.max_tokens()
    );

    let mut total_passages = 0;
    let mut total_tokens = 0_u64;
    let mut longest = 0;
    let mut runs: Vec<Duration> = Vec::with_capacity(repetitions);
    for repetition in 1..=repetitions {
        let run = measure(&passages, &counter, count)?;
        println!(
            "run {repetition}: total {:?}, read {:?}, count {:?}, {} passages, {} tokens, \
             longest passage {}",
            run.total, run.read, run.count, run.passages, run.tokens, run.longest
        );
        total_passages = run.passages;
        total_tokens = run.tokens;
        longest = run.longest;
        runs.push(run.total);
    }

    let mean = runs.iter().map(|run| run.as_secs_f64()).sum::<f64>() / runs.len() as f64;
    println!(
        "mean {mean:.3} s for {total_passages} passages ({:.1} µs per passage), \
         {total_tokens} tokens counted, longest passage {longest}",
        mean * 1e6 / total_passages as f64,
    );
    Ok(())
}

/// Durations and totals of one measured pass.
struct Run {
    total: Duration,
    read: Duration,
    count: Duration,
    passages: usize,
    tokens: u64,
    longest: usize,
}

/// Reads the first `count` passages of `path` and counts their tokens, chunk by
/// chunk, exactly as the attach does.
fn measure(
    path: &Path,
    counter: &TokenCounter,
    count: usize,
) -> Result<Run, Box<dyn std::error::Error>> {
    let started = Instant::now();
    let file = File::open(path)?;
    let mut reader = ParquetRecordBatchReaderBuilder::try_new(file)?
        .with_batch_size(DEFAULT_BATCH_ROWS)
        .build()?;
    let mut read = Duration::ZERO;
    let mut counting = Duration::ZERO;
    let mut passages = 0;
    let mut tokens = 0_u64;
    let mut longest = 0;

    loop {
        let chunk_started = Instant::now();
        let Some(chunk) = reader.next() else {
            break;
        };
        let chunk = chunk?;
        read += chunk_started.elapsed();
        let texts = chunk
            .column_by_name("text")
            .ok_or("the passage file has no 'text' column")?
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or("the 'text' column is not UTF-8")?;
        let remaining = count.saturating_sub(passages);
        let rows = texts.len().min(remaining);
        let values: Vec<&str> = (0..rows).map(|row| texts.value(row)).collect();

        let count_started = Instant::now();
        let counts = counter.count_documents(&values)?;
        counting += count_started.elapsed();

        passages += rows;
        for value in counts {
            tokens = tokens.saturating_add(u64::try_from(value).unwrap_or(u64::MAX));
            longest = longest.max(value);
        }
        if passages == count {
            break;
        }
    }

    Ok(Run {
        total: started.elapsed(),
        read,
        count: counting,
        passages,
        tokens,
        longest,
    })
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
