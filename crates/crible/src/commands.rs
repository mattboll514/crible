// SPDX-License-Identifier: Apache-2.0

//! Implementations of the first Crible commands.
//!
//! References: `docs/phases/P1-stockage-et-recherche-exacte.md` (task
//! `CLI-PREMIERES-COMMANDES`), `docs/02-architecture.md` ("Organisation sur
//! disque"), `docs/05-api.md` ("Recherche : réponse"), `docs/07` (the binary
//! `crible`).
//!
//! # Layout the commands read and write
//!
//! ```text
//! <data>/indexes/<index>/index.json        configuration of the index
//! <data>/indexes/<index>/CURRENT_EPOCH     written by `index create`
//! <models>/<model>/model.json              resolved model configuration
//! ```
//!
//! The index directory is the one of `docs/02-architecture.md`, rooted at
//! `--data`: `index create` copies the configuration to `index.json` before the
//! store opens the index, so the store can cross-check the frozen schema it
//! builds the filter structures from.
//!
//! The model configuration is the `<models>/<name>/model.json` file of
//! `docs/04-donnees-et-ingestion.md`. Models live on their own volume
//! (`[paths] models = "/models"` of `docs/07-stack-et-deploiement.md`), a
//! sibling of the data volume, so every command that reads one takes its root
//! as `--models`. P1 has no model of its own; a project that brings its vectors
//! writes an `external` model configuration (`dims` and `normalize`), which
//! `crible attach` reads.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crible_core::config::IndexConfig;
use crible_core::error::{CribleError, ErrorCode};
use crible_core::filter::Filter;
use crible_core::ids::Epoch;
use crible_core::model::{ModelConfig, ModelKind};
use crible_encode::{
    DEFAULT_QUERY_CACHE_ENTRIES, ModelRegistry, Precision, QueryEncoder, TokenCounter,
};
use crible_search::dense::exact::ExactSearcher;
use crible_search::pool::Pool;
use crible_search::scan_pool::ScanPool;
use crible_store::attach::attach;
use crible_store::generation::{
    CURRENT_EPOCH_FILE_NAME, GenerationStore, INDEX_CONFIG_FILE_NAME, IndexState,
    write_index_config,
};
use serde_json::{Value, json};

use crate::vector;

/// Epoch of a freshly created index.
///
/// The epoch changes at each compaction; the first epoch of an index is 1, the
/// value the J1 demo and the bench use. No on-disk format depends on this
/// value: `CURRENT_EPOCH`, `meta.epoch` and the manifests carry whatever epoch
/// the store commits, so this is a convention of the CLI, not a format
/// constant.
const INITIAL_EPOCH: u32 = 1;

/// Highest `k` accepted by a search (`docs/05-api.md`, "Recherche : requête").
const MAX_K: usize = 1_000;

/// `crible index create --data <dir> --config <json>`.
pub fn create_index(data: &Path, config_path: &Path) -> Result<(), CribleError> {
    let raw = fs::read_to_string(config_path).map_err(|error| {
        invalid_request(format!(
            "cannot read the index configuration {}: {error}",
            config_path.display()
        ))
    })?;
    let config = IndexConfig::from_json_str(&raw)?;
    let name = path_component(&config.name, "index name")?;
    let index_dir = index_directory(data, name);

    // `write_index_config` decides alone: it refuses an index that already
    // holds an epoch, and replaces a residual `index.json` left by a creation
    // that stopped before its first epoch. The store then cross-checks the
    // schema it builds the filter structures from.
    write_index_config(&index_dir, &raw)?;

    let store = GenerationStore::create(&index_dir, Epoch::new(INITIAL_EPOCH), &config.schema)?;
    println!(
        "index '{}' created at {} (epoch {}, generation {})",
        name,
        index_dir.display(),
        store.epoch().get(),
        store.generation().get()
    );
    Ok(())
}

/// `crible attach --data <dir> --models <dir> --index <i> <batch-dir>`.
pub fn attach_batch(
    data: &Path,
    models: &Path,
    index: &str,
    batch: &Path,
) -> Result<(), CribleError> {
    let name = path_component(index, "index name")?;
    let (index_dir, config) = load_index(data, name)?;
    let model = load_model(models, &config.model)?;
    // Control 8 of `docs/04` counts the tokens of every passage with the
    // tokenizer of an `onnx` model, without loading ONNX Runtime (INV-11). An
    // `external` model has no tokenizer and is out of that control.
    let token_counter = match model.kind {
        ModelKind::External => None,
        ModelKind::Onnx => Some(TokenCounter::load(model_directory(models, &config.model)?)?),
    };
    let mut store = GenerationStore::open(&index_dir, &config.schema)?;
    let report = attach(&mut store, batch, &config, &model, token_counter.as_ref())?;
    println!(
        "batch '{}' attached to index '{}': generation {} (added {}, replaced {}, \
         unchanged {}, metadata_updated {}, deleted {}, ignored {})",
        report.batch_id,
        name,
        report.generation.get(),
        report.added,
        report.replaced,
        report.unchanged,
        report.metadata_updated,
        report.deleted,
        report.ignored,
    );
    Ok(())
}

/// One `crible search` invocation.
///
/// The query comes either from a file of a provided vector (`--vector-file`) or
/// from text (`--query`); the two are mutually exclusive and one is required.
/// A text query needs `--models`, and an `onnx` encoder: the model of the index
/// by default, or the one named by `--model`, which is what lets `synth-small`
/// (an `external` index) be searched by text with a test encoder.
#[derive(Clone, Copy)]
pub struct SearchRequest<'a> {
    /// Root data directory (`<data>/indexes/<index>`).
    pub data: &'a Path,
    /// Index name, as declared in its `index.json`.
    pub index: &'a str,
    /// Vector file, when the caller provides the vector.
    pub vector_file: Option<&'a Path>,
    /// Text query, encoded by the server.
    pub query: Option<&'a str>,
    /// Root models directory; required with `query`.
    pub models: Option<&'a Path>,
    /// Encoder to use for `query`; the model of the index when omitted.
    pub model: Option<&'a str>,
    /// Filter as a JSON object.
    pub filter: Option<&'a str>,
    /// Number of results, between 1 and 1000.
    pub k: usize,
}

impl std::fmt::Debug for SearchRequest<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A query and a filter value must never reach a log (AGENTS.md,
        // "Confidentialité"), so their presence is shown, never their text.
        formatter
            .debug_struct("SearchRequest")
            .field("data", &self.data)
            .field("index", &self.index)
            .field("vector_file", &self.vector_file)
            .field("query", &self.query.map(|_| "<redacted>"))
            .field("models", &self.models)
            .field("model", &self.model)
            .field("filter", &self.filter.map(|_| "<redacted>"))
            .field("k", &self.k)
            .finish()
    }
}

/// `crible search --data <dir> --index <i> (--vector-file <file> | --query <text>)`.
///
/// The response follows the shape of `docs/05-api.md` ("Recherche : réponse"),
/// with the fields P1 and `ENCODER-REQUETES` can fill: `results`, `pool_size`,
/// `generation`, `took_ms` and `truncated_query`. The score is the cosine of the
/// exact dense branch, hence `score_type: "cosine"`. `took_ms` is the total,
/// from the entry of the command: it includes reading the index configuration,
/// the vector or the text encoding, the filter, evaluating the pool, opening
/// the readers and the scan itself.
///
/// `truncated_query` is the wire name of the `truncated` flag of
/// `crible_encode::EncodedQuery` (ADR-0051); the HTTP route of P6 keeps that
/// mapping.
pub fn search(request: &SearchRequest<'_>) -> Result<(), CribleError> {
    let started = Instant::now();
    if request.k == 0 || request.k > MAX_K {
        return Err(invalid_request(format!(
            "k must be between 1 and {MAX_K}, got {}",
            request.k
        )));
    }
    let name = path_component(request.index, "index name")?;
    let (index_dir, config) = load_index(request.data, name)?;
    let store = GenerationStore::open(&index_dir, &config.schema)?;
    let (vector, truncated_query) = match (request.vector_file, request.query) {
        (Some(path), None) => (vector::load(path)?, false),
        (None, Some(text)) => encode_text_query(request.models, &config, request.model, text)?,
        (Some(_), Some(_)) => {
            return Err(invalid_request(
                "--vector-file and --query are mutually exclusive; pass one of them",
            ));
        }
        (None, None) => {
            return Err(invalid_request(
                "search needs --vector-file <file> or --query <text>",
            ));
        }
    };
    let filter = match request.filter {
        Some(raw) => {
            let filter = Filter::from_json_str(raw)?;
            filter.validate(&config.schema)?;
            filter
        }
        None => Filter::And(Vec::new()),
    };

    let snapshot = store.snapshot();
    let pool = Pool::evaluate(snapshot.filters(), &filter)?;
    let scan = ScanPool::new(0)?;
    let segments_directory = store.segments_directory();
    let records_path = store.records_path();
    let segments = snapshot.segments().to_vec();
    let searcher =
        scan.install(|| ExactSearcher::open(&segments_directory, &segments, &records_path))?;
    let hits = scan.install(|| searcher.top_k(&vector, &pool, request.k))?;
    let took_ms = started.elapsed().as_secs_f64() * 1_000.0;

    let results: Vec<Value> = hits
        .into_iter()
        .map(|hit| {
            json!({
                "doc_id": hit.doc_id,
                "passage": hit.ord,
                "key": hit.key,
                "score": hit.score,
                "score_type": "cosine",
                "fields": hit.fields,
                "text": hit.text,
            })
        })
        .collect();
    let response = json!({
        "results": results,
        "pool_size": pool.len(),
        "generation": snapshot.generation().get(),
        "took_ms": took_ms,
        "truncated_query": truncated_query,
    });
    println!("{}", pretty(&response)?);
    Ok(())
}

/// Encodes a text query with the encoder named by `model`, or with the model of
/// the index.
///
/// The query cache and the INT8 default are those of `docs/03-recherche.md` and
/// ADR-0051. The instance configuration of P6 will pass
/// `[limits].query_cache_entries` and `[encoder].query_precision`; the CLI has
/// no `crible.toml` loader yet, so it uses the documented defaults.
fn encode_text_query(
    models: Option<&Path>,
    index: &IndexConfig,
    model: Option<&str>,
    text: &str,
) -> Result<(Vec<f32>, bool), CribleError> {
    let models = models.ok_or_else(|| {
        invalid_request(
            "--query needs --models <dir>, the [paths] models root of crible.toml holding \
             <model>/model.json",
        )
    })?;
    let name = model.unwrap_or(&index.model);
    let directory = model_directory(models, name)?;
    let config = load_model(models, name)?;
    if config.kind != ModelKind::Onnx {
        return Err(invalid_request(format!(
            "index '{}' uses model '{}' of kind external, which has no encoder; --query needs an \
             onnx model, pass --model <name>",
            index.name, name
        )));
    }

    let encoder = QueryEncoder::new(
        &directory,
        Arc::new(ModelRegistry::new()),
        DEFAULT_QUERY_CACHE_ENTRIES,
    );
    let encoded = encoder.encode_query(text, Precision::Int8)?;
    Ok((encoded.vector, encoded.truncated))
}

/// `crible inspect --data <dir> [--index <i>]`.
///
/// Like every read of the store, opening the index runs its recovery
/// (`docs/02-architecture.md`, "Reprise") and takes the SQLite write lock:
/// `inspect` removes leftover `tmp-*` entries and segments SQLite does not
/// reference, and rewrites a missing manifest. It therefore needs a writable
/// mount, not a read-only one.
pub fn inspect(data: &Path, index: Option<&str>) -> Result<(), CribleError> {
    let names = match index {
        Some(name) => vec![path_component(name, "index name")?.to_owned()],
        None => list_indexes(data)?,
    };
    if names.is_empty() {
        return Err(invalid_request(format!(
            "no index found under {}",
            indexes_directory(data).display()
        )));
    }
    for name in names {
        let (index_dir, config) = load_index(data, &name)?;
        let store = GenerationStore::open(&index_dir, &config.schema)?;
        let state = store.state()?;
        print_state(&name, &state);
    }
    Ok(())
}

/// Prints one index state, as `crible inspect` reports it.
fn print_state(name: &str, state: &IndexState) {
    println!("index {name}");
    println!("  epoch: {}", state.epoch.get());
    println!("  generation: {}", state.generation.get());
    println!("  documents: {}", state.documents);
    println!(
        "  passages: {} live, {} dead",
        state.live_passages, state.dead_passages
    );
    println!("  dead ratio: {:.4}", state.dead_ratio());
    println!("  segments: {}", state.segments.len());
    for segment in &state.segments {
        println!("    {}: {} vectors", segment.segment_id, segment.count);
    }
    let memory = state.filter_memory;
    println!(
        "  bitmaps: {} bytes (live {}, keywords {}, booleans {}, columns {})",
        memory.total(),
        memory.live,
        memory.keywords,
        memory.booleans,
        memory.columns
    );
}

/// Directory holding every index of an instance.
fn indexes_directory(data: &Path) -> PathBuf {
    data.join("indexes")
}

/// Directory of one index.
fn index_directory(data: &Path, name: &str) -> PathBuf {
    indexes_directory(data).join(name)
}

/// Reads and validates the configuration of an index.
///
/// The directory name and the `name` of the configuration must agree: a command
/// that names `synth-small` must not open another index.
fn load_index(data: &Path, name: &str) -> Result<(PathBuf, IndexConfig), CribleError> {
    let index_dir = index_directory(data, name);
    let path = index_dir.join(INDEX_CONFIG_FILE_NAME);
    let raw = fs::read_to_string(&path).map_err(|error| {
        CribleError::new(
            ErrorCode::UnknownIndex,
            format!(
                "cannot read the configuration of index '{name}' at {}: {error}",
                path.display()
            ),
        )
    })?;
    let config = IndexConfig::from_json_str(&raw)?;
    if config.name != name {
        return Err(invalid_request(format!(
            "index directory '{}' holds a configuration for index '{}'",
            index_dir.display(),
            config.name
        )));
    }
    Ok((index_dir, config))
}

/// Reads the resolved model configuration of `model` under the models root.
fn load_model(models: &Path, model: &str) -> Result<ModelConfig, CribleError> {
    let path = model_directory(models, model)?.join("model.json");
    let raw = fs::read_to_string(&path).map_err(|error| {
        invalid_request(format!(
            "cannot read the model configuration of '{model}' at {}: {error}; pass \
             --models <dir>, the [paths] models root of crible.toml, holding \
             <model>/model.json",
            path.display()
        ))
    })?;
    ModelConfig::from_json_str(&raw)
}

/// Directory of one model under the models root.
fn model_directory(models: &Path, model: &str) -> Result<PathBuf, CribleError> {
    let model = path_component(model, "model name")?;
    Ok(models.join(model))
}

/// Lists the index directories that carry a `CURRENT_EPOCH`, sorted by name.
fn list_indexes(data: &Path) -> Result<Vec<String>, CribleError> {
    let directory = indexes_directory(data);
    let entries = fs::read_dir(&directory).map_err(|error| {
        invalid_request(format!(
            "cannot list the indexes under {}: {error}",
            directory.display()
        ))
    })?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            internal(format!(
                "cannot read an entry of {}: {error}",
                directory.display()
            ))
        })?;
        let path = entry.path();
        if !path.is_dir() || !path.join(CURRENT_EPOCH_FILE_NAME).is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            return Err(internal(format!(
                "{} has no UTF-8 name, so it cannot be an index name",
                path.display()
            )));
        };
        names.push(name);
    }
    names.sort();
    Ok(names)
}

/// Accepts `name` only as one path component.
fn path_component<'a>(name: &'a str, what: &str) -> Result<&'a str, CribleError> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        return Err(invalid_request(format!(
            "{what} '{name}' must be a single path component"
        )));
    }
    Ok(name)
}

/// Serializes a response, or reports the serialization fault.
fn pretty(value: &Value) -> Result<String, CribleError> {
    serde_json::to_string_pretty(value)
        .map_err(|error| internal(format!("cannot serialize the response as JSON: {error}")))
}

/// Error raised when the request itself is malformed.
fn invalid_request(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::InvalidRequest, message)
}

/// Error raised when the process cannot carry out the request.
fn internal(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::Internal, message)
}
