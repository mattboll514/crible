// SPDX-License-Identifier: Apache-2.0

//! Loading of an ONNX encoder and encoding of texts with it.
//!
//! The model directory is the one of `docs/04-donnees-et-ingestion.md`: a
//! `model.json` describing the model, an fp32 ONNX graph, an optional INT8 one,
//! and a tokenizer. The graph returns the last hidden state; Crible applies the
//! prompt, the CLS pooling and the L2 normalization itself, as the export does
//! not.
//!
//! A [`Model`] holds **one** precision. The registry of [`crate::ModelRegistry`]
//! keys sessions by `(fingerprint, precision)`, because the INT8 file is outside
//! the fingerprint and both graphs of one model must stay distinct (ADR-0051).
//!
//! ONNX Runtime is loaded dynamically (`ort` with the `load-dynamic` feature):
//! the library is named by `ORT_DYLIB_PATH`, never linked at build time.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crible_core::error::{CribleError, ErrorCode};
use crible_core::model::{ModelConfig, ModelKind, Prompts};
use ort::session::Session;
use rayon::prelude::*;
use tokenizers::Tokenizer;

/// Side of the model a text belongs to, which picks the prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// A search query, encoded with the `query` prompt.
    Query,
    /// A document passage, encoded with the `document` prompt.
    Document,
}

/// Precision of an ONNX graph of the model.
///
/// Queries use [`Precision::Int8`] by default, and the acceptance check and the
/// online document encoding use [`Precision::Fp32`] (`docs/04-donnees-et-ingestion.md`,
/// ADR-0051).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Precision {
    /// Dynamically quantized graph, used for queries.
    Int8,
    /// Reference graph, used for the acceptance check and online documents.
    Fp32,
}

/// A vector produced by an encoder, and whether its input was truncated.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedText {
    /// Pooled and, when the configuration asks for it, normalized vector.
    pub vector: Vec<f32>,
    /// Whether the token count exceeded the limit and the tail was dropped.
    pub truncated: bool,
}

/// An ONNX encoder loaded from a model directory, for one precision.
///
/// The registry keys the cached instance by `(fingerprint, precision)`: the same
/// directory loaded twice, or two directories describing the same model, share
/// one encoder per precision through [`crate::ModelRegistry`] (ADR-0051).
pub struct Model {
    config: ModelConfig,
    precision: Precision,
    tokenizer: Tokenizer,
    session: Mutex<Session>,
}

impl Model {
    /// Loads the fp32 graph of the model directory `directory`.
    ///
    /// The `fp32` and `tokenizer` files declared by `model.json` are hashed and
    /// compared to the declared digests before anything else, so a modified
    /// file is refused before ONNX Runtime runs it.
    ///
    /// # Errors
    ///
    /// - [`ErrorCode::InvalidRequest`] when `model.json` is malformed, a
    ///   declared file is missing or does not match its hash, or the
    ///   configuration asks for an encoder this module cannot build.
    /// - [`ErrorCode::Internal`] for an I/O failure or an ONNX Runtime failure.
    pub fn load(directory: impl AsRef<Path>) -> Result<Self, CribleError> {
        Self::load_precision(directory, Precision::Fp32)
    }

    /// Loads the graph of `precision` from the model directory `directory`.
    ///
    /// The graph file of the requested precision and the tokenizer are hashed
    /// and compared to the declared digests before ONNX Runtime opens a session,
    /// so a modified INT8 file is refused exactly as a modified fp32 one
    /// (ADR-0051).
    ///
    /// # Errors
    ///
    /// - [`ErrorCode::InvalidRequest`] when `model.json` is malformed, a
    ///   declared file is missing or does not match its hash, the INT8 graph is
    ///   requested but `model.json` declares none, or the configuration asks for
    ///   an encoder this module cannot build.
    /// - [`ErrorCode::Internal`] for an I/O failure or an ONNX Runtime failure.
    pub fn load_precision(
        directory: impl AsRef<Path>,
        precision: Precision,
    ) -> Result<Self, CribleError> {
        let directory = directory.as_ref();
        let config = read_config(directory)?;
        Self::load_with_config(directory, config, precision)
    }

    pub(crate) fn load_with_config(
        directory: &Path,
        config: ModelConfig,
        precision: Precision,
    ) -> Result<Self, CribleError> {
        if config.kind != crible_core::model::ModelKind::Onnx {
            return Err(invalid(format!(
                "model '{}' is external: it has no encoder to load",
                config.name
            )));
        }
        if config.pooling.as_deref() != Some("cls") {
            return Err(invalid(format!(
                "model '{}' uses pooling {:?}; only cls is supported",
                config.name, config.pooling
            )));
        }
        let files = config.files.as_ref().ok_or_else(|| {
            invalid(format!(
                "model '{}' declares no files; an onnx model needs them",
                config.name
            ))
        })?;
        let (graph_path, graph_hash) = match precision {
            Precision::Fp32 => (directory.join(&files.fp32.path), files.fp32.blake3.as_str()),
            Precision::Int8 => {
                let int8 = files.int8.as_ref().ok_or_else(|| {
                    invalid(format!(
                        "model '{}' declares no int8 file; model.json must list one under \
                         files.int8 to load the INT8 graph",
                        config.name
                    ))
                })?;
                (directory.join(&int8.path), int8.blake3.as_str())
            }
        };
        verify_hash(&graph_path, graph_hash)?;
        let tokenizer_path = directory.join(&files.tokenizer.path);
        verify_hash(&tokenizer_path, &files.tokenizer.blake3)?;
        let tokenizer = read_tokenizer(&tokenizer_path)?;

        let environment = environment()?;
        let session = Session::builder(environment)
            .and_then(|mut builder| builder.commit_from_file(&graph_path))
            .map_err(|error| {
                internal(format!(
                    "cannot load the ONNX graph {}: {error}",
                    graph_path.display()
                ))
            })?;

        Ok(Self {
            config,
            precision,
            tokenizer,
            session: Mutex::new(session),
        })
    }

    /// Configuration the model was loaded from.
    #[must_use]
    pub fn config(&self) -> &ModelConfig {
        &self.config
    }

    /// Precision of the loaded graph.
    #[must_use]
    pub fn precision(&self) -> Precision {
        self.precision
    }

    /// Fingerprint of the configuration (ADR-0028).
    #[must_use]
    pub fn fingerprint(&self) -> String {
        self.config.fingerprint()
    }

    /// Number of components of an encoded vector.
    #[must_use]
    pub fn dims(&self) -> u32 {
        self.config.dims
    }

    /// Encodes `text` without a token limit and returns the pooled, normalized
    /// vector.
    ///
    /// Documents use this form: a passage that exceeds `max_tokens` must be
    /// refused by the import check, never truncated silently (INV-11).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidRequest`] when the tokenizer fails, and
    /// [`ErrorCode::Internal`] when ONNX Runtime fails or returns a shape the
    /// configuration does not describe.
    pub fn encode(&self, side: Side, text: &str) -> Result<Vec<f32>, CribleError> {
        self.encode_with_limit(side, text, None)
            .map(|encoded| encoded.vector)
    }

    /// Encodes `text`, truncating it to `max_tokens` tokens when given.
    ///
    /// The prompt of `side` and the special tokens are part of the count: the
    /// tokenizer loaded by [`Model::load`] has no truncation, so this method is
    /// the only limit (ADR-0051). An input longer than `max_tokens` keeps its
    /// first `max_tokens - 1` ids and its last id, then reports
    /// `truncated: true`; the caller decides whether that is acceptable. The
    /// last id is kept because a post-processed tokenizer ends the encoding with
    /// its final special token (`</s>` for mDenseOn), which the exported graph
    /// expects; a tokenizer without post-processing keeps its last content
    /// token instead, as the fixture does.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidRequest`] when the tokenizer fails or `max_tokens`
    /// is zero, and [`ErrorCode::Internal`] when ONNX Runtime fails or returns a
    /// shape the configuration does not describe.
    pub fn encode_with_limit(
        &self,
        side: Side,
        text: &str,
        max_tokens: Option<u32>,
    ) -> Result<EncodedText, CribleError> {
        let prompts = self.config.prompts.as_ref().ok_or_else(|| {
            invalid(format!(
                "model '{}' declares no prompts; an onnx model needs them",
                self.config.name
            ))
        })?;
        let prompt = match side {
            Side::Query => &prompts.query,
            Side::Document => &prompts.document,
        };
        let (mut ids, mut mask) = tokenize(&self.tokenizer, side, prompt, text)?;
        let truncated = match max_tokens {
            Some(limit) => truncate(&mut ids, &mut mask, limit)?,
            None => false,
        };
        let length = ids.len();
        if length == 0 {
            return Err(invalid("the tokenizer produced an empty input"));
        }
        let ids: Vec<i64> = ids.into_iter().map(i64::from).collect();
        let mask: Vec<i64> = mask.into_iter().map(i64::from).collect();
        let shape = vec![
            1_i64,
            i64::try_from(length).map_err(|_| {
                internal(format!(
                    "an input of {length} tokens exceeds the i64 shape of ONNX Runtime"
                ))
            })?,
        ];

        let mut session = self.session.lock().map_err(|_| {
            internal(format!(
                "the encoder session of model '{}' is poisoned",
                self.config.name
            ))
        })?;
        let outputs = session
            .run(ort::inputs![
                "input_ids" => ort::value::Tensor::from_array((shape.clone(), ids))
                    .map_err(|error| internal(format!("cannot build input_ids: {error}")))?,
                "attention_mask" => ort::value::Tensor::from_array((shape, mask))
                    .map_err(|error| internal(format!("cannot build attention_mask: {error}")))?,
            ])
            .map_err(|error| internal(format!("ONNX Runtime failed to encode: {error}")))?;
        let output = outputs.get("last_hidden_state").ok_or_else(|| {
            internal(format!(
                "model '{}' output 'last_hidden_state' is missing",
                self.config.name
            ))
        })?;
        let (shape, hidden) = output
            .try_extract_tensor::<f32>()
            .map_err(|error| internal(format!("cannot read 'last_hidden_state': {error}")))?;
        let dims = self.config.dims as usize;
        let expected = [1, length as i64, self.config.dims as i64];
        if shape.iter().copied().collect::<Vec<i64>>() != expected {
            return Err(internal(format!(
                "model '{}' returned {:?}, expected {expected:?}",
                self.config.name, shape
            )));
        }

        // CLS pooling: row 0 of the [1, tokens, dims] output.
        let mut vector = hidden[..dims].to_vec();
        if self.config.normalize {
            let norm = vector
                .iter()
                .map(|component| f64::from(*component) * f64::from(*component))
                .sum::<f64>()
                .sqrt();
            if norm == 0.0 {
                return Err(internal(format!(
                    "model '{}' returned a zero vector, which cannot be normalized",
                    self.config.name
                )));
            }
            let norm = norm as f32;
            for component in &mut vector {
                *component /= norm;
            }
        }
        Ok(EncodedText { vector, truncated })
    }
}

impl std::fmt::Debug for Model {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Model")
            .field("name", &self.config.name)
            .field("dims", &self.config.dims)
            .field("precision", &self.precision)
            .field("fingerprint", &self.fingerprint())
            .finish_non_exhaustive()
    }
}

/// Tokenizer of one model, used to count the tokens of a text.
///
/// Control 8 of `docs/04-donnees-et-ingestion.md` counts the tokens of every
/// passage at attach, prompt included, and refuses the batch when one exceeds
/// `max_tokens` (**INV-11**). The count needs the tokenizer only: loading ONNX
/// Runtime is neither necessary nor wanted there, so [`TokenCounter`] reads
/// `model.json`, verifies the declared blake3 of `tokenizer.json`, removes the
/// truncation declared by that file, and counts the ids.
///
/// The truncation is cleared exactly as [`Model::load_precision`] does: a
/// `truncation` in `tokenizer.json` describes the Hugging Face revision, never
/// the check, so it must not hide an over-long passage.
#[derive(Debug)]
pub struct TokenCounter {
    prompts: Prompts,
    max_tokens: u32,
    tokenizer: Tokenizer,
}

impl TokenCounter {
    /// Loads the tokenizer of the `onnx` model directory `directory`.
    ///
    /// The tokenizer file is hashed and compared to the digest declared by
    /// `model.json` before it is read, as when a full [`Model`] is loaded.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidRequest`] when `model.json` is malformed, the model
    /// is `external` (it has no tokenizer, so it is out of control 8), the
    /// tokenizer is missing or does not match its declared hash, or the
    /// tokenizer cannot be read.
    pub fn load(directory: impl AsRef<Path>) -> Result<Self, CribleError> {
        let directory = directory.as_ref();
        let config = read_config(directory)?;
        Self::load_with_config(directory, &config)
    }

    fn load_with_config(directory: &Path, config: &ModelConfig) -> Result<Self, CribleError> {
        if config.kind != ModelKind::Onnx {
            return Err(invalid(format!(
                "model '{}' is external: it has no tokenizer to count the passage lengths",
                config.name
            )));
        }
        let files = config.files.as_ref().ok_or_else(|| {
            invalid(format!(
                "model '{}' declares no files; an onnx model needs them",
                config.name
            ))
        })?;
        let prompts = config.prompts.as_ref().ok_or_else(|| {
            invalid(format!(
                "model '{}' declares no prompts; an onnx model needs them",
                config.name
            ))
        })?;
        let max_tokens = config.max_tokens.ok_or_else(|| {
            invalid(format!(
                "model '{}' declares no max_tokens; an onnx model needs it",
                config.name
            ))
        })?;
        let tokenizer_path = directory.join(&files.tokenizer.path);
        verify_hash(&tokenizer_path, &files.tokenizer.blake3)?;
        Ok(Self {
            prompts: prompts.clone(),
            max_tokens,
            tokenizer: read_tokenizer(&tokenizer_path)?,
        })
    }

    /// `max_tokens` declared by `model.json`.
    ///
    /// `ModelConfig::validate` requires it for an `onnx` model, so the value is
    /// always present here.
    #[must_use]
    pub fn max_tokens(&self) -> u32 {
        self.max_tokens
    }

    /// Number of tokens of one input of `side`: the prompt, the text and the
    /// special tokens, with no truncation.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidRequest`] when the tokenizer fails.
    pub fn count(&self, side: Side, text: &str) -> Result<usize, CribleError> {
        let prompt = match side {
            Side::Query => &self.prompts.query,
            Side::Document => &self.prompts.document,
        };
        let (ids, _) = tokenize(&self.tokenizer, side, prompt, text)?;
        Ok(ids.len())
    }

    /// Number of tokens of several document passages, in the same order as
    /// `texts`.
    ///
    /// The passages are counted in parallel on the current rayon pool. The
    /// result keeps the order of the slice, so the caller can name the
    /// offending passages in the order of the batch rows.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidRequest`] when the tokenizer fails on one text.
    pub fn count_documents(&self, texts: &[&str]) -> Result<Vec<usize>, CribleError> {
        texts
            .par_iter()
            .map(|text| self.count(Side::Document, text))
            .collect()
    }
}

/// Counts the ids of `prompt` followed by `text`, with the special tokens.
///
/// The truncation of the tokenizer was cleared at load, so the ids are the whole
/// input; the prompt and the special tokens are part of them, which is what
/// control 8 of `docs/04` counts.
fn tokenize(
    tokenizer: &Tokenizer,
    side: Side,
    prompt: &str,
    text: &str,
) -> Result<(Vec<u32>, Vec<u32>), CribleError> {
    let input = format!("{prompt}{text}");
    let encoding = tokenizer.encode(input.as_str(), true).map_err(|error| {
        invalid(format!(
            "cannot tokenize an input of side {side:?}: {error}"
        ))
    })?;
    Ok((
        encoding.get_ids().to_vec(),
        encoding.get_attention_mask().to_vec(),
    ))
}

/// Reads `path` as a tokenizer and clears the truncation it declares.
///
/// `tokenizer.json` carries the 8192-token truncation limit of the Hugging Face
/// revision. Crible never truncates silently (INV-11): the limit is removed
/// here, so an over-long input reaches the caller instead of losing its tail.
/// Queries are truncated explicitly by [`Model::encode_with_limit`], with a
/// `truncated` flag (ADR-0051); documents are checked by
/// `CONTROLER-LONGUEURS`.
fn read_tokenizer(path: &Path) -> Result<Tokenizer, CribleError> {
    let mut tokenizer = Tokenizer::from_file(path).map_err(|error| {
        invalid(format!(
            "cannot read the tokenizer {}: {error}",
            path.display()
        ))
    })?;
    tokenizer.with_truncation(None).map_err(|error| {
        invalid(format!(
            "cannot clear the truncation of the tokenizer {}: {error}",
            path.display()
        ))
    })?;
    Ok(tokenizer)
}

/// Truncates `ids` and `mask` to `limit` tokens, reporting whether they were
/// longer.
///
/// Truncation keeps the first `limit - 1` tokens and the **last** one: a
/// post-processed tokenizer ends its encoding with a special token, and dropping
/// it would feed the graph an input it never saw during the export. The prompt
/// and the special tokens are already in `ids`, so they count towards `limit`
/// (ADR-0051).
fn truncate(ids: &mut Vec<u32>, mask: &mut Vec<u32>, limit: u32) -> Result<bool, CribleError> {
    if limit == 0 {
        return Err(invalid(
            "max_tokens is zero; an input cannot be encoded with no token",
        ));
    }
    let limit = limit as usize;
    let truncated = ids.len() > limit;
    if truncated {
        let last_id = ids.pop().unwrap_or_default();
        let last_mask = mask.pop().unwrap_or_default();
        ids.truncate(limit.saturating_sub(1));
        ids.push(last_id);
        mask.truncate(limit.saturating_sub(1));
        mask.push(last_mask);
    }
    Ok(truncated)
}

/// Reads and parses `<directory>/model.json`.
///
/// The configuration loader of the server will need it; until then the registry
/// and the query encoder are its only callers, inside this crate.
pub(crate) fn read_config(directory: &Path) -> Result<ModelConfig, CribleError> {
    let path = model_json_path(directory);
    let raw = std::fs::read_to_string(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            invalid(format!(
                "{} is missing; a model directory holds model.json",
                path.display()
            ))
        } else {
            internal(format!("cannot read {}: {error}", path.display()))
        }
    })?;
    ModelConfig::from_json_str(&raw)
}

/// Path of `model.json` in `directory`, for error messages.
pub(crate) fn model_json_path(directory: &Path) -> PathBuf {
    directory.join("model.json")
}

/// Checks the blake3 digest of `path` against `declared` without holding it in
/// memory.
fn verify_hash(path: &Path, declared: &str) -> Result<(), CribleError> {
    let mut file = std::fs::File::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            invalid(format!(
                "{} is missing; model.json declares it",
                path.display()
            ))
        } else {
            internal(format!("cannot open {}: {error}", path.display()))
        }
    })?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| internal(format!("cannot read {}: {error}", path.display())))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let computed = hasher.finalize().to_hex().to_string();
    if computed != declared {
        return Err(invalid(format!(
            "{} does not match its declared hash: model.json declares {declared}, the file \
             hashes to {computed}",
            path.display()
        )));
    }
    Ok(())
}

/// Process-wide ONNX Runtime environment, built once from `ORT_DYLIB_PATH`.
///
/// ONNX Runtime only accepts one environment per process, so two models share
/// this one. The mutex serializes the first creation; a second caller uses the
/// environment the first one built.
fn environment() -> Result<&'static ort::environment::Environment, CribleError> {
    static ENVIRONMENT: OnceLock<ort::environment::Environment> = OnceLock::new();
    static INIT: Mutex<()> = Mutex::new(());
    if let Some(environment) = ENVIRONMENT.get() {
        return Ok(environment);
    }
    let _guard = INIT
        .lock()
        .map_err(|_| internal("the ONNX Runtime environment initializer is poisoned"))?;
    if let Some(environment) = ENVIRONMENT.get() {
        return Ok(environment);
    }
    let path = dylib_path()?;
    let builder = ort::init_from(&path).map_err(|error| {
        internal(format!(
            "cannot load ONNX Runtime from {}: {error}",
            path.display()
        ))
    })?;
    let environment = builder.build().map_err(|error| {
        internal(format!(
            "cannot build the ONNX Runtime environment from {}: {error}",
            path.display()
        ))
    })?;
    let _ = ENVIRONMENT.set(environment);
    ENVIRONMENT
        .get()
        .ok_or_else(|| internal("the ONNX Runtime environment disappeared after it was built"))
}

/// Reads `ORT_DYLIB_PATH` and checks that it names a file.
fn dylib_path() -> Result<PathBuf, CribleError> {
    let raw = std::env::var("ORT_DYLIB_PATH").map_err(|_| {
        internal(
            "ORT_DYLIB_PATH is not set; it must name the ONNX Runtime dynamic library (see \
             tools/README.md)",
        )
    })?;
    if raw.is_empty() {
        return Err(internal(
            "ORT_DYLIB_PATH is empty; it must name the ONNX Runtime dynamic library (see \
             tools/README.md)",
        ));
    }
    let path = PathBuf::from(raw);
    if !path.is_file() {
        return Err(internal(format!(
            "ORT_DYLIB_PATH names {}, which is not a file",
            path.display()
        )));
    }
    Ok(path)
}

fn invalid(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::InvalidRequest, message)
}

fn internal(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::Internal, message)
}

#[cfg(test)]
mod tests {
    use super::truncate;

    #[test]
    fn truncates_ids_and_mask_at_the_limit() {
        let mut ids = vec![101_u32, 1, 2, 3, 4, 102];
        let mut mask = vec![1_u32; 6];
        assert!(
            truncate(&mut ids, &mut mask, 4).expect("a positive limit truncates"),
            "six ids exceed a limit of four"
        );
        assert_eq!(
            ids,
            vec![101, 1, 2, 102],
            "the first ids and the final special token are kept"
        );
        assert_eq!(mask.len(), 4, "the mask follows the ids");
    }

    #[test]
    fn reports_no_truncation_at_or_below_the_limit() {
        let mut ids = vec![101_u32, 1, 102];
        let mut mask = vec![1_u32; 3];
        assert!(
            !truncate(&mut ids, &mut mask, 3).expect("a positive limit is accepted"),
            "three ids fit a limit of three"
        );
        assert_eq!(ids.len(), 3, "an input at the limit is untouched");
    }

    #[test]
    fn refuses_a_zero_limit() {
        let mut ids = vec![101_u32];
        let mut mask = vec![1_u32];
        let error = truncate(&mut ids, &mut mask, 0).expect_err("a zero limit is refused");
        assert!(error.message().contains("max_tokens is zero"));
    }
}
