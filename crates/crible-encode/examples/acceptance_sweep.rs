// SPDX-License-Identifier: Apache-2.0

//! Batch-size sweep of the acceptance re-encoding (MES-08, control 9 of an
//! attach; tasks `ACCELERER-RECETTE` and `MESURER-EN-CONTENEUR-MAC`).
//!
//! This example is a harness, not production code. It re-encodes the 200
//! passages drawn by `acceptance_split dump` on a length-sorted, padded batch
//! and times the whole re-encoding for one batch size, so sizes 1, 4 and 8 can
//! be measured without touching the acceptance re-encoding, which encodes one
//! passage at a time: ADR-0073 removed the batch code and its size constant, so
//! the harness carries the batch shape on its own.
//!
//! It is the campaign of 2026-10-27 ported into the workspace: that campaign
//! kept its driver under the gitignored `data/` tree, which the container build
//! cannot reach (`tools/` and `data/` are outside the build context). The graph
//! call is unchanged: `input_ids` and `attention_mask` of shape `[rows, length]`,
//! pads after the real ids, CLS pooling of row 0 of every row, normalization
//! when `model.json` asks for it, default ONNX Runtime settings as `Model` uses
//! (ADR-0052).
//!
//! ```text
//! acceptance_sweep sweep  --model <dir> --sample <json> --sizes 1,4,8 \
//!     [--rounds 3] [--out <file>]
//! acceptance_sweep longest --model <dir> --sample <json> --copies 1,4,8 \
//!     [--rounds 3] [--out <file>]
//! ```
//!
//! `ORT_DYLIB_PATH` must name the ONNX Runtime library, as for every command
//! that loads an `onnx` model (`tools/README.md`). The sample file holds the
//! text of the drawn passages: nothing of it reaches stdout or a log
//! (AGENTS.md, "Confidentialité").

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ort::session::Session;
use tokenizers::Tokenizer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().collect();
    let mode = arguments
        .get(1)
        .ok_or("usage: acceptance_sweep <sweep|longest> --model <dir> --sample <json>")?;
    match mode.as_str() {
        "sweep" => sweep(&arguments),
        "longest" => longest(&arguments),
        other => Err(format!("unknown mode {other:?}: expected `sweep` or `longest`").into()),
    }
}

/// One text of the drawn sample.
struct Row {
    tokens: usize,
    text: String,
}

struct Encoder {
    tokenizer: Tokenizer,
    prompt: String,
    dims: usize,
    normalize: bool,
    session: Session,
}

impl Encoder {
    fn load(model: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let raw = std::fs::read_to_string(model.join("model.json"))?;
        let config: serde_json::Value = serde_json::from_str(&raw)?;
        let prompt = config["prompts"]["document"]
            .as_str()
            .ok_or("model.json declares no document prompt")?
            .to_owned();
        let dims = config["dims"]
            .as_u64()
            .ok_or("model.json declares no dims")? as usize;
        let normalize = config["normalize"].as_bool().unwrap_or(false);
        let graph = config["files"]["fp32"]["path"]
            .as_str()
            .ok_or("model.json declares no fp32 path")?;
        let tokenizer_path = config["files"]["tokenizer"]["path"]
            .as_str()
            .ok_or("model.json declares no tokenizer path")?;
        let mut tokenizer = Tokenizer::from_file(model.join(tokenizer_path))
            .map_err(|error| format!("cannot read the tokenizer: {error}"))?;
        tokenizer
            .with_truncation(None)
            .map_err(|error| format!("cannot clear the truncation: {error}"))?;
        let path = std::env::var("ORT_DYLIB_PATH")?;
        let environment = ort::init_from(&path)
            .map_err(|error| format!("cannot load ONNX Runtime from {path}: {error}"))?
            .build()
            .map_err(|error| format!("cannot build the environment: {error}"))?;
        let session = Session::builder(&environment)?
            .commit_from_file(model.join(graph))
            .map_err(|error| format!("cannot load the graph: {error}"))?;
        Ok(Self {
            tokenizer,
            prompt,
            dims,
            normalize,
            session,
        })
    }

    fn tokenize(&self, text: &str) -> Result<(Vec<u32>, Vec<u32>), Box<dyn std::error::Error>> {
        let input = format!("{}{}", self.prompt, text);
        let encoding = self
            .tokenizer
            .encode(input.as_str(), true)
            .map_err(|error| format!("cannot tokenize: {error}"))?;
        Ok((
            encoding.get_ids().to_vec(),
            encoding.get_attention_mask().to_vec(),
        ))
    }

    /// Encodes `positions` in one padded call, sorted by length by the caller.
    fn run(
        &mut self,
        inputs: &[(Vec<u32>, Vec<u32>)],
        positions: &[usize],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let rows = positions.len();
        let length = positions
            .iter()
            .map(|position| inputs[*position].0.len())
            .max()
            .unwrap_or(1);
        let mut ids: Vec<i64> = Vec::with_capacity(rows * length);
        let mut mask: Vec<i64> = Vec::with_capacity(rows * length);
        for position in positions {
            let (input_ids, input_mask) = &inputs[*position];
            ids.extend(input_ids.iter().map(|id| i64::from(*id)));
            mask.extend(input_mask.iter().map(|bit| i64::from(*bit)));
            let padding = length - input_ids.len();
            ids.resize(ids.len() + padding, 0);
            mask.resize(mask.len() + padding, 0);
        }
        let shape = [rows as i64, length as i64];
        let outputs = self.session.run(ort::inputs![
            "input_ids" => ort::value::Tensor::from_array((shape, ids))?,
            "attention_mask" => ort::value::Tensor::from_array((shape, mask))?,
        ])?;
        let output = outputs
            .get("last_hidden_state")
            .ok_or("the output last_hidden_state is missing")?;
        let (returned, hidden) = output.try_extract_tensor::<f32>()?;
        let expected = vec![shape[0], shape[1], self.dims as i64];
        if returned.iter().copied().collect::<Vec<i64>>() != expected {
            return Err(format!(
                "the graph returned {:?}, expected {expected:?}",
                returned.iter().copied().collect::<Vec<i64>>()
            )
            .into());
        }
        // Pooling, as `Model::pool` does: CLS row of every row, then the
        // normalization the configuration asks for.
        for row in 0..rows {
            let offset = row * length * self.dims;
            let mut vector = hidden[offset..offset + self.dims].to_vec();
            if self.normalize {
                let norm = vector
                    .iter()
                    .map(|component| f64::from(*component) * f64::from(*component))
                    .sum::<f64>()
                    .sqrt() as f32;
                for component in &mut vector {
                    *component /= norm;
                }
            }
            std::hint::black_box(&vector);
        }
        Ok(())
    }
}

fn read_sample(path: &Path) -> Result<Vec<Row>, Box<dyn std::error::Error>> {
    let raw = std::fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&raw)?;
    let passages = value["passages"]
        .as_array()
        .ok_or("the sample has no passages")?;
    let mut rows = Vec::with_capacity(passages.len());
    for passage in passages {
        rows.push(Row {
            tokens: passage["tokens"].as_u64().unwrap_or(0) as usize,
            text: passage["text"].as_str().unwrap_or_default().to_owned(),
        });
    }
    Ok(rows)
}

/// Sorted positions of `rows` by increasing length, as `batch::batches` does.
fn by_length(rows: &[Row]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by_key(|position| (rows[*position].tokens, *position));
    order
}

fn sweep(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let model = PathBuf::from(required(arguments, "--model")?);
    let sample = PathBuf::from(required(arguments, "--sample")?);
    let sizes = list(arguments, "--sizes", "1,4,8")?;
    let rounds = optional(arguments, "--rounds", 3_usize)?;
    let rows = read_sample(&sample)?;
    let mut encoder = Encoder::load(&model)?;
    let mut inputs = Vec::with_capacity(rows.len());
    for row in &rows {
        inputs.push(encoder.tokenize(&row.text)?);
    }
    let order = by_length(&rows);
    let total_tokens: usize = rows.iter().map(|row| row.tokens).sum();
    println!(
        "sample {} passages, {total_tokens} tokens, sizes {sizes:?}, {rounds} interleaved rounds",
        rows.len()
    );

    // Interleaved: every round runs every size, so a drift of the machine hits
    // all of them and not the last one only (protocol of the 2026-10-27
    // campaign).
    let mut results: Vec<(usize, Vec<f64>)> =
        sizes.iter().map(|size| (*size, Vec::new())).collect();
    for round in 0..rounds {
        for (size, values) in results.iter_mut() {
            let started = Instant::now();
            let mut padded_tokens = 0_usize;
            let mut done = 0_usize;
            for chunk in order.chunks(*size) {
                let length = chunk.iter().map(|position| inputs[*position].0.len()).max();
                padded_tokens += length.unwrap_or(0) * chunk.len();
                encoder.run(&inputs, chunk)?;
                done += chunk.len();
                // Progress only: the series lasts minutes, and a measurement
                // command may not stay silent (ADR-0135). The line is printed
                // inside the timed span, so the wall clock keeps its meaning;
                // its cost is far below the millisecond the campaign reports.
                if done % 25 < *size {
                    println!(
                        "round {round} B={size:3}: {done} of {} passages, {:.1} s so far",
                        rows.len(),
                        started.elapsed().as_secs_f64()
                    );
                }
            }
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            println!("round {round} B={size:3}: {elapsed:9.1} ms  padded {padded_tokens:6} tokens");
            values.push(elapsed);
        }
    }
    let mut summary = HashMap::new();
    for (size, values) in &results {
        let mut sorted = values.clone();
        sorted.sort_by(f64::total_cmp);
        let median = sorted[sorted.len() / 2];
        println!(
            "B={size:3}: median {median:9.1} ms  [{:.1}–{:.1}]  {:.3} ms/passage  {:.3} µs/token",
            sorted[0],
            sorted[sorted.len() - 1],
            median / rows.len() as f64,
            median * 1000.0 / total_tokens as f64
        );
        summary.insert(
            size.to_string(),
            serde_json::json!({
                "values_ms": values,
                "median_ms": median,
                "min_ms": sorted[0],
                "max_ms": sorted[sorted.len() - 1],
                "ms_per_passage": median / rows.len() as f64,
                "microseconds_per_token": median * 1000.0 / total_tokens as f64,
            }),
        );
    }
    write_out(
        arguments,
        &serde_json::json!({
            "mode": "sweep",
            "sample": sample.display().to_string(),
            "passages": rows.len(),
            "tokens_total": total_tokens,
            "sizes": sizes,
            "rounds": rounds,
            "results": summary,
        }),
    )
}

fn longest(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let model = PathBuf::from(required(arguments, "--model")?);
    let sample = PathBuf::from(required(arguments, "--sample")?);
    let copies = list(arguments, "--copies", "1,4,8")?;
    let rounds = optional(arguments, "--rounds", 3_usize)?;
    let rows = read_sample(&sample)?;
    let mut encoder = Encoder::load(&model)?;
    let longest = rows
        .iter()
        .max_by_key(|row| row.tokens)
        .ok_or("the sample is empty")?;
    let tokens = longest.tokens;
    let (ids, mask) = encoder.tokenize(&longest.text)?;
    println!("longest drawn passage: {tokens} tokens, {} ids", ids.len());

    let mut results: Vec<(usize, Vec<f64>)> =
        copies.iter().map(|size| (*size, Vec::new())).collect();
    for round in 0..rounds {
        for (size, values) in results.iter_mut() {
            let inputs: Vec<(Vec<u32>, Vec<u32>)> =
                (0..*size).map(|_| (ids.clone(), mask.clone())).collect();
            let positions: Vec<usize> = (0..*size).collect();
            let started = Instant::now();
            encoder.run(&inputs, &positions)?;
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            println!("round {round} copies={size:3}: {elapsed:9.1} ms");
            values.push(elapsed);
        }
    }
    let mut summary = HashMap::new();
    for (size, values) in &results {
        let mut sorted = values.clone();
        sorted.sort_by(f64::total_cmp);
        let median = sorted[sorted.len() / 2];
        println!(
            "copies={size:3}: median {median:9.1} ms  [{:.1}–{:.1}]  {:.3} ms/copie",
            sorted[0],
            sorted[sorted.len() - 1],
            median / *size as f64
        );
        summary.insert(
            size.to_string(),
            serde_json::json!({
                "values_ms": values,
                "median_ms": median,
                "min_ms": sorted[0],
                "max_ms": sorted[sorted.len() - 1],
            }),
        );
    }
    write_out(
        arguments,
        &serde_json::json!({
            "mode": "longest",
            "sample": sample.display().to_string(),
            "tokens": tokens,
            "copies": copies,
            "rounds": rounds,
            "results": summary,
        }),
    )
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
        Some(path) => std::fs::write(path, format!("{text}\n"))?,
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

fn list(arguments: &[String], name: &str, default: &str) -> Result<Vec<usize>, String> {
    let raw = optional(arguments, name, default.to_owned())?;
    raw.split(',')
        .map(|part| {
            part.trim()
                .parse::<usize>()
                .map_err(|_| format!("cannot read {name}: {part:?}"))
        })
        .collect()
}
