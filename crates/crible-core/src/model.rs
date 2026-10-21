// SPDX-License-Identifier: Apache-2.0

//! Model configuration, including the fingerprint of its frozen part.
//!
//! References: `docs/04-donnees-et-ingestion.md`, section "Le modèle", and
//! ADR-0028 (canonical form of the fingerprint).

use serde_json::{Map, Value};

use crate::error::CribleError;
use crate::json;

/// Kind of model used by an index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    /// ONNX encoder shipped with the index.
    Onnx,
    /// Vectors produced outside Crible; the client sends them.
    External,
}

impl ModelKind {
    /// Wire name used in `model.json`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Onnx => "onnx",
            Self::External => "external",
        }
    }

    fn parse(value: &Value, path: &str) -> Result<Self, CribleError> {
        let raw = json::string(value, path)?;
        match raw.as_str() {
            "onnx" => Ok(Self::Onnx),
            "external" => Ok(Self::External),
            other => Err(json::invalid(format!(
                "unknown model kind '{other}' at {path}; expected onnx or external"
            ))),
        }
    }
}

/// Prompts prefixed to the encoder input, one per side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompts {
    /// Prefix of a query.
    pub query: String,
    /// Prefix of a document.
    pub document: String,
}

impl Prompts {
    fn from_value(value: &Value, path: &str) -> Result<Self, CribleError> {
        let object = json::object(value, path)?;
        json::reject_unknown_keys(object, path, &["query", "document"])?;
        Ok(Self {
            query: json::string_member(object, path, "query")?,
            document: json::string_member(object, path, "document")?,
        })
    }
}

/// One file of the model directory, identified by its content hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFile {
    /// Path relative to the model directory.
    pub path: String,
    /// Lowercase hexadecimal blake3 digest of the file.
    pub blake3: String,
}

impl ModelFile {
    fn from_value(value: &Value, path: &str) -> Result<Self, CribleError> {
        let object = json::object(value, path)?;
        json::reject_unknown_keys(object, path, &["path", "blake3"])?;
        let file = Self {
            path: json::string_member(object, path, "path")?,
            blake3: json::string_member(object, path, "blake3")?,
        };
        if !valid_blake3(&file.blake3) {
            return Err(json::invalid(format!(
                "blake3 at {path}.blake3 must be 64 lowercase hexadecimal characters"
            )));
        }
        Ok(file)
    }
}

/// Files composing an ONNX model directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFiles {
    /// Reference encoder, used for the acceptance check.
    pub fp32: ModelFile,
    /// Query encoder; not part of the fingerprint.
    pub int8: Option<ModelFile>,
    /// Tokenizer files.
    pub tokenizer: ModelFile,
}

impl ModelFiles {
    fn from_value(value: &Value, path: &str) -> Result<Self, CribleError> {
        let object = json::object(value, path)?;
        json::reject_unknown_keys(object, path, &["fp32", "int8", "tokenizer"])?;
        Ok(Self {
            fp32: ModelFile::from_value(
                required_file(object, path, "fp32")?,
                &format!("{path}.fp32"),
            )?,
            int8: match object.get("int8") {
                Some(value) => Some(ModelFile::from_value(value, &format!("{path}.int8"))?),
                None => None,
            },
            tokenizer: ModelFile::from_value(
                required_file(object, path, "tokenizer")?,
                &format!("{path}.tokenizer"),
            )?,
        })
    }
}

fn required_file<'a>(
    object: &'a Map<String, Value>,
    path: &str,
    key: &str,
) -> Result<&'a Value, CribleError> {
    object.get(key).ok_or_else(|| {
        json::invalid(format!(
            "missing required file '{key}' at {path}; an onnx model needs it"
        ))
    })
}

fn valid_blake3(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

/// Configuration of the model of an index.
///
/// The fingerprint covers the frozen part only: `hf_id`, `revision`, `dims`,
/// `pooling`, `normalize`, `prompts`, and the `fp32` and `tokenizer` hashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelConfig {
    /// Model name, also the name of its directory.
    pub name: String,
    /// Kind of model.
    pub kind: ModelKind,
    /// Hugging Face repository of an ONNX encoder.
    pub hf_id: Option<String>,
    /// Commit of the Hugging Face repository.
    pub revision: Option<String>,
    /// Number of dimensions of a vector.
    pub dims: u32,
    /// Pooling used by the export.
    pub pooling: Option<String>,
    /// Whether vectors are L2-normalized by the model.
    pub normalize: bool,
    /// Maximum sequence length supported by the export.
    pub max_tokens: Option<u32>,
    /// Prompts prefixed before encoding.
    pub prompts: Option<Prompts>,
    /// Files of an ONNX model directory.
    pub files: Option<ModelFiles>,
}

const MODEL_KEYS: [&str; 10] = [
    "name",
    "kind",
    "hf_id",
    "revision",
    "dims",
    "pooling",
    "normalize",
    "max_tokens",
    "prompts",
    "files",
];

impl ModelConfig {
    /// Parses and validates a model configuration from its JSON form.
    pub fn from_json_str(raw: &str) -> Result<Self, CribleError> {
        let value: Value = serde_json::from_str(raw)
            .map_err(|error| json::invalid(format!("invalid model JSON: {error}")))?;
        Self::from_value(&value)
    }

    /// Parses and validates a model configuration from a decoded JSON value.
    pub fn from_value(value: &Value) -> Result<Self, CribleError> {
        let object = json::object(value, "$")?;
        json::reject_unknown_keys(object, "$", &MODEL_KEYS)?;
        let name = json::string_member(object, "$", "name")?;
        if name.is_empty() {
            return Err(json::invalid("setting 'name' must not be empty at $.name"));
        }
        let kind = ModelKind::parse(json::member(object, "$", "kind")?, "$.kind")?;
        let dims = json::u32_member(object, "$", "dims")?;
        if dims == 0 {
            return Err(json::invalid("setting 'dims' must be positive at $.dims"));
        }
        let model = Self {
            name,
            kind,
            hf_id: optional_string(object, "$", "hf_id")?,
            revision: optional_string(object, "$", "revision")?,
            dims,
            pooling: optional_string(object, "$", "pooling")?,
            normalize: json::boolean_member(object, "$", "normalize")?,
            max_tokens: match object.get("max_tokens") {
                Some(value) => Some(json::u32_value(value, "$.max_tokens")?),
                None => None,
            },
            prompts: match object.get("prompts") {
                Some(value) => Some(Prompts::from_value(value, "$.prompts")?),
                None => None,
            },
            files: match object.get("files") {
                Some(value) => Some(ModelFiles::from_value(value, "$.files")?),
                None => None,
            },
        };
        model.validate()?;
        Ok(model)
    }

    fn validate(&self) -> Result<(), CribleError> {
        if self.kind != ModelKind::Onnx {
            return Ok(());
        }
        for (member, present) in [
            ("hf_id", self.hf_id.is_some()),
            ("revision", self.revision.is_some()),
            ("pooling", self.pooling.is_some()),
            ("prompts", self.prompts.is_some()),
            ("files", self.files.is_some()),
        ] {
            if !present {
                return Err(json::invalid(format!(
                    "missing required member '{member}' at $; an onnx model needs it"
                )));
            }
        }
        Ok(())
    }

    /// Canonical JSON hashed to build the fingerprint (ADR-0028).
    ///
    /// Keys are sorted, the JSON is compact, and non-ASCII characters stay as
    /// UTF-8 instead of being escaped.
    #[must_use]
    pub fn fingerprint_payload(&self) -> String {
        let mut payload = Map::new();
        payload.insert("dims".to_owned(), Value::from(self.dims));
        payload.insert("normalize".to_owned(), Value::from(self.normalize));
        if let Some(hf_id) = &self.hf_id {
            payload.insert("hf_id".to_owned(), Value::from(hf_id.clone()));
        }
        if let Some(revision) = &self.revision {
            payload.insert("revision".to_owned(), Value::from(revision.clone()));
        }
        if let Some(pooling) = &self.pooling {
            payload.insert("pooling".to_owned(), Value::from(pooling.clone()));
        }
        if let Some(prompts) = &self.prompts {
            let mut object = Map::new();
            object.insert("query".to_owned(), Value::from(prompts.query.clone()));
            object.insert("document".to_owned(), Value::from(prompts.document.clone()));
            payload.insert("prompts".to_owned(), Value::Object(object));
        }
        if let Some(files) = &self.files {
            let mut object = Map::new();
            object.insert("fp32".to_owned(), Value::from(files.fp32.blake3.clone()));
            object.insert(
                "tokenizer".to_owned(),
                Value::from(files.tokenizer.blake3.clone()),
            );
            payload.insert("files".to_owned(), Value::Object(object));
        }
        Value::Object(payload).to_string()
    }

    /// blake3 of the canonical JSON, lowercase hexadecimal.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        blake3::hash(self.fingerprint_payload().as_bytes())
            .to_hex()
            .to_string()
    }
}

fn optional_string(
    object: &Map<String, Value>,
    path: &str,
    key: &str,
) -> Result<Option<String>, CribleError> {
    match object.get(key) {
        Some(value) => json::string(value, &format!("{path}.{key}")).map(Some),
        None => Ok(None),
    }
}
