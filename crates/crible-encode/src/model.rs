// SPDX-License-Identifier: Apache-2.0

//! Loading of an ONNX encoder and encoding of texts with it.
//!
//! The model directory is the one of `docs/04-donnees-et-ingestion.md`: a
//! `model.json` describing the model, an fp32 ONNX graph, and a tokenizer. The
//! graph returns the last hidden state; Crible applies the prompt, the CLS
//! pooling and the L2 normalization itself, as the export does not.
//!
//! ONNX Runtime is loaded dynamically (`ort` with the `load-dynamic` feature):
//! the library is named by `ORT_DYLIB_PATH`, never linked at build time.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crible_core::error::{CribleError, ErrorCode};
use crible_core::model::ModelConfig;
use ort::session::Session;
use tokenizers::Tokenizer;

/// Side of the model a text belongs to, which picks the prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// A search query, encoded with the `query` prompt.
    Query,
    /// A document passage, encoded with the `document` prompt.
    Document,
}

/// An ONNX encoder loaded from a model directory.
///
/// The fingerprint of the configuration keys the cached instance: the same
/// directory loaded twice, or two directories describing the same model, share
/// one encoder through [`crate::ModelRegistry`].
pub struct Model {
    config: ModelConfig,
    tokenizer: Tokenizer,
    session: Mutex<Session>,
}

impl Model {
    /// Loads the model directory `directory`.
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
        let directory = directory.as_ref();
        let config = read_config(directory)?;
        Self::load_with_config(directory, config)
    }

    pub(crate) fn load_with_config(
        directory: &Path,
        config: ModelConfig,
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
        let fp32_path = directory.join(&files.fp32.path);
        verify_hash(&fp32_path, &files.fp32.blake3)?;
        let tokenizer_path = directory.join(&files.tokenizer.path);
        verify_hash(&tokenizer_path, &files.tokenizer.blake3)?;

        let mut tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(|error| {
            invalid(format!(
                "cannot read the tokenizer {}: {error}",
                tokenizer_path.display()
            ))
        })?;
        // `tokenizer.json` carries the 8192-token truncation limit of the
        // Hugging Face revision. Crible never truncates silently (INV-11): the
        // limit is removed here, so an over-long input reaches ONNX Runtime
        // instead of losing its tail. Counting and reporting it belongs to
        // CONTROLER-LONGUEURS.
        tokenizer.with_truncation(None).map_err(|error| {
            invalid(format!(
                "cannot clear the truncation of the tokenizer {}: {error}",
                tokenizer_path.display()
            ))
        })?;

        let environment = environment()?;
        let session = Session::builder(environment)
            .and_then(|mut builder| builder.commit_from_file(&fp32_path))
            .map_err(|error| {
                internal(format!(
                    "cannot load the ONNX graph {}: {error}",
                    fp32_path.display()
                ))
            })?;

        Ok(Self {
            config,
            tokenizer,
            session: Mutex::new(session),
        })
    }

    /// Configuration the model was loaded from.
    #[must_use]
    pub fn config(&self) -> &ModelConfig {
        &self.config
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

    /// Encodes `text` and returns the pooled, normalized vector.
    ///
    /// The prompt of `side` is prefixed to the text, exactly as
    /// `tools/export_onnx.py` does when it builds the reference vectors.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidRequest`] when the tokenizer fails, and
    /// [`ErrorCode::Internal`] when ONNX Runtime fails or returns a shape the
    /// configuration does not describe.
    pub fn encode(&self, side: Side, text: &str) -> Result<Vec<f32>, CribleError> {
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
        let input = format!("{prompt}{text}");
        let encoding = self
            .tokenizer
            .encode(input.as_str(), true)
            .map_err(|error| {
                invalid(format!(
                    "cannot tokenize an input of side {side:?}: {error}"
                ))
            })?;
        let length = encoding.get_ids().len();
        if length == 0 {
            return Err(invalid("the tokenizer produced an empty input"));
        }
        let ids: Vec<i64> = encoding.get_ids().iter().map(|id| i64::from(*id)).collect();
        let mask: Vec<i64> = encoding
            .get_attention_mask()
            .iter()
            .map(|bit| i64::from(*bit))
            .collect();
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
        Ok(vector)
    }
}

impl std::fmt::Debug for Model {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Model")
            .field("name", &self.config.name)
            .field("dims", &self.config.dims)
            .field("fingerprint", &self.fingerprint())
            .finish_non_exhaustive()
    }
}

/// Reads and parses `<directory>/model.json`.
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
