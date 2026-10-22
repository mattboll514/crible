// SPDX-License-Identifier: Apache-2.0

//! Public batch manifest (format v1) and the validation rules that do not
//! require reading the Parquet files.
//!
//! References: `docs/format-lot-v1.md`,
//! `docs/04-donnees-et-ingestion.md` (format de lot v1, contrôles 1 à 4), and
//! ADR-0012 (public batch format).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{IndexConfig, ReferencePrecision};
use crate::error::{CribleError, ErrorCode};
use crate::json;
use crate::model::{ModelConfig, ModelKind};

/// Value of the `format` member of every batch manifest.
pub const BATCH_FORMAT: &str = "crible-batch";

/// Only version of the batch manifest understood by this build.
pub const BATCH_FORMAT_VERSION: u32 = 1;

/// How a batch changes the content of an index (`docs/04`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchMode {
    /// Create, replace or update the listed documents; leave the others alone.
    #[default]
    Upsert,
    /// The batch is the whole content of the index; other documents disappear.
    ReplaceAll,
}

impl BatchMode {
    /// Wire name used in `manifest.json` and in error messages.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Upsert => "upsert",
            Self::ReplaceAll => "replace_all",
        }
    }
}

/// Storage type of the vectors of a batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorDtype {
    /// Half precision, two bytes per dimension.
    Fp16,
    /// Single precision, four bytes per dimension.
    Fp32,
}

impl VectorDtype {
    /// Wire name used in `manifest.json` and in error messages.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fp16 => "fp16",
            Self::Fp32 => "fp32",
        }
    }

    /// Storage type matching the reference precision frozen in an index.
    #[must_use]
    pub const fn from_reference_precision(precision: ReferencePrecision) -> Self {
        match precision {
            ReferencePrecision::Fp16 => Self::Fp16,
            ReferencePrecision::Fp32 => Self::Fp32,
        }
    }
}

/// Program that produced the batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Producer {
    /// Program name, for example `legifacile-sync`.
    pub name: String,
    /// Version of the producer.
    pub version: String,
    /// Identifier of the machine, or of the producer, that produced the batch
    /// (ADR-0036).
    pub host: String,
}

/// Model used to encode the vectors of the batch.
///
/// The optional encoder members are compared with the model of the index when
/// that model is an ONNX encoder. An `external` index only compares `dims`,
/// `normalize` and the storage type of the vectors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchModel {
    /// Hugging Face repository of the encoder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hf_id: Option<String>,
    /// Commit of the Hugging Face repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// Number of dimensions of a vector.
    pub dims: u32,
    /// Pooling used by the export.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pooling: Option<String>,
    /// Whether the producer L2-normalized the vectors.
    pub normalize: bool,
    /// Document prompt prefixed before encoding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_document: Option<String>,
    /// Storage type of the vectors in `passages.parquet`.
    pub vector_dtype: VectorDtype,
    /// Versions of the encoding runtime, as free-form key/value pairs.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub runtime: BTreeMap<String, String>,
}

/// Chunker that split the documents into passages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chunker {
    /// Chunker name.
    pub name: String,
    /// Chunker version.
    pub version: String,
    /// Chunker parameters, opaque to the engine.
    #[serde(default)]
    pub params: serde_json::Map<String, Value>,
}

/// Template used to compose the encoder input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    /// Template name.
    pub name: String,
    /// Template version.
    pub version: String,
}

/// Row counts declared by the batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Counts {
    /// Number of rows in the document files.
    pub documents: u64,
    /// Number of rows in the passage files.
    pub passages: u64,
    /// Number of rows in the deletion file.
    pub deletions: u64,
}

/// `manifest.json` of a batch, format v1.
///
/// Parsing accepts the manifest; [`check_manifest`] then checks it against an
/// index. The `files` map associates each Parquet file name with a
/// `blake3:<hex>` checksum.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchManifest {
    /// Format discriminator; always [`BATCH_FORMAT`].
    pub format: String,
    /// Format version; always [`BATCH_FORMAT_VERSION`].
    pub format_version: u32,
    /// Identifier of the batch, also the name of its directory.
    pub batch_id: String,
    /// Index the batch applies to.
    pub index: String,
    /// Effect of the batch on existing documents.
    #[serde(default)]
    pub mode: BatchMode,
    /// Creation time, RFC 3339 in UTC.
    pub created_at: String,
    /// Program that produced the batch.
    pub producer: Producer,
    /// Model used to encode the vectors.
    pub model: BatchModel,
    /// Chunker used to split the documents.
    pub chunker: Chunker,
    /// Template used to compose the encoder input.
    pub template: Template,
    /// Row counts declared by the producer.
    pub counts: Counts,
    /// Parquet file name to `blake3:<hex>` checksum.
    pub files: BTreeMap<String, String>,
}

impl BatchManifest {
    /// Parses a manifest from its JSON form.
    ///
    /// This only checks the JSON shape and the identity of the batch
    /// (`batch_id`, `index`, `created_at`). Format, version, model and file
    /// consistency are checked by [`check_manifest`].
    pub fn from_json_str(raw: &str) -> Result<Self, CribleError> {
        let manifest: Self = serde_json::from_str(raw)
            .map_err(|error| json::invalid(format!("invalid batch manifest JSON: {error}")))?;
        manifest.check_identity()?;
        Ok(manifest)
    }

    fn check_identity(&self) -> Result<(), CribleError> {
        check_batch_id(&self.batch_id)?;
        if self.index.is_empty() {
            return Err(json::invalid("member 'index' must not be empty at $.index"));
        }
        if self.created_at.is_empty() {
            return Err(json::invalid(
                "member 'created_at' must not be empty at $.created_at",
            ));
        }
        if self.producer.name.is_empty() {
            return Err(json::invalid(
                "member 'producer.name' must not be empty at $.producer.name",
            ));
        }
        Ok(())
    }
}

/// A `batch_id` names the batch directory, so it must be one safe path segment.
fn check_batch_id(batch_id: &str) -> Result<(), CribleError> {
    if batch_id.is_empty() {
        return Err(json::invalid(
            "member 'batch_id' must not be empty at $.batch_id",
        ));
    }
    if batch_id == "." || batch_id == ".." {
        return Err(json::invalid(format!(
            "batch_id '{batch_id}' is not a directory name at $.batch_id"
        )));
    }
    if batch_id
        .chars()
        .any(|character| character.is_control() || matches!(character, '/' | '\\'))
    {
        return Err(json::invalid(format!(
            "batch_id '{batch_id}' must be a single path segment at $.batch_id"
        )));
    }
    Ok(())
}

/// Checks a parsed manifest against the target index (controls 1 to 4 of
/// `docs/04` that do not need the Parquet files).
///
/// `model_config` must be the configuration resolved for
/// `index_config.model`; this function does not read `models/<name>/model.json`
/// itself.
///
/// In P0, the manifest-only part of control 4 (file list and checksum shape)
/// runs before control 3 (model compatibility), because it needs nothing but
/// the manifest. At attach time (P1), the order of `docs/04` applies: the model
/// is checked before the checksum values.
///
/// # Errors
///
/// - [`ErrorCode::InvalidRequest`] for an unknown format or version, a batch
///   built for another index, an inconsistent `files` list, or inconsistent
///   counts;
/// - [`ErrorCode::IncompatibleModel`] when the batch model differs from the
///   index model, field by field.
pub fn check_manifest(
    manifest: &BatchManifest,
    index_config: &IndexConfig,
    model_config: &ModelConfig,
) -> Result<(), CribleError> {
    manifest.check_identity()?;
    check_format(manifest)?;
    check_index(manifest, index_config)?;
    let files = classify_files(manifest)?;
    check_mode(manifest, &files)?;
    check_model(manifest, index_config, model_config)?;
    check_counts(manifest, &files)
}

fn check_format(manifest: &BatchManifest) -> Result<(), CribleError> {
    if manifest.format != BATCH_FORMAT {
        return Err(json::invalid(format!(
            "unknown batch format '{}' at $.format; expected '{BATCH_FORMAT}'",
            manifest.format
        )));
    }
    if manifest.format_version != BATCH_FORMAT_VERSION {
        return Err(json::invalid(format!(
            "unsupported batch format_version {} at $.format_version; this build reads version {BATCH_FORMAT_VERSION}",
            manifest.format_version
        )));
    }
    Ok(())
}

fn check_index(manifest: &BatchManifest, index_config: &IndexConfig) -> Result<(), CribleError> {
    if manifest.index != index_config.name {
        return Err(json::invalid(format!(
            "batch index '{}' at $.index does not match index '{}'",
            manifest.index, index_config.name
        )));
    }
    Ok(())
}

fn check_mode(manifest: &BatchManifest, files: &BatchFiles) -> Result<(), CribleError> {
    if manifest.mode == BatchMode::ReplaceAll && files.deletions().is_some() {
        return Err(json::invalid(
            "mode 'replace_all' at $.mode forbids a deletions file; the batch is the complete content of the index",
        ));
    }
    Ok(())
}

fn check_model(
    manifest: &BatchManifest,
    index_config: &IndexConfig,
    model_config: &ModelConfig,
) -> Result<(), CribleError> {
    let batch = &manifest.model;
    if batch.dims != model_config.dims {
        return Err(incompatible("dims", batch.dims, model_config.dims));
    }
    if batch.normalize != model_config.normalize {
        return Err(incompatible(
            "normalize",
            batch.normalize,
            model_config.normalize,
        ));
    }
    let expected_dtype = VectorDtype::from_reference_precision(index_config.reference_precision);
    if batch.vector_dtype != expected_dtype {
        return Err(incompatible(
            "vector_dtype",
            batch.vector_dtype.as_str(),
            expected_dtype.as_str(),
        ));
    }
    if model_config.kind == ModelKind::Onnx {
        let prompt_document = model_config
            .prompts
            .as_ref()
            .map(|prompts| prompts.document.as_str());
        for (field, batch_value, index_value) in [
            (
                "hf_id",
                batch.hf_id.as_deref(),
                model_config.hf_id.as_deref(),
            ),
            (
                "revision",
                batch.revision.as_deref(),
                model_config.revision.as_deref(),
            ),
            (
                "pooling",
                batch.pooling.as_deref(),
                model_config.pooling.as_deref(),
            ),
            (
                "prompt_document",
                batch.prompt_document.as_deref(),
                prompt_document,
            ),
        ] {
            if batch_value != index_value {
                return Err(incompatible(
                    field,
                    display_option(batch_value),
                    display_option(index_value),
                ));
            }
        }
    }
    Ok(())
}

fn display_option(value: Option<&str>) -> String {
    value.map_or_else(|| "<missing>".to_owned(), |text| format!("'{text}'"))
}

fn incompatible(
    field: &str,
    batch: impl std::fmt::Display,
    index: impl std::fmt::Display,
) -> CribleError {
    CribleError::new(
        ErrorCode::IncompatibleModel,
        format!(
            "incompatible model: field '{field}' is {batch} in the batch but {index} in the index; re-encode the batch with the index model"
        ),
    )
}

/// Files of a batch, grouped by role after their name has been checked.
///
/// Names keep the sorted order of the `files` map of the manifest, so two
/// reads of the same batch visit the files in the same order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchFiles {
    documents: Vec<String>,
    passages: Vec<String>,
    deletions: Option<String>,
}

impl BatchFiles {
    /// Names of the `documents` files, sorted.
    #[must_use]
    pub fn documents(&self) -> &[String] {
        &self.documents
    }

    /// Names of the `passages` files, sorted.
    #[must_use]
    pub fn passages(&self) -> &[String] {
        &self.passages
    }

    /// Name of the `deletions` file, if the batch has one.
    #[must_use]
    pub fn deletions(&self) -> Option<&str> {
        self.deletions.as_deref()
    }
}

/// Groups the files of a manifest by role, refusing any other name.
///
/// The checksum itself is only checked for the `blake3:` prefix here: the
/// strict digest form and the value against the file bytes are the checksum
/// verification of `crible-store::batch_reader` (control 4 at attach time).
///
/// # Errors
///
/// [`ErrorCode::InvalidRequest`] for a file name that is not one of the three
/// documented shapes, or a checksum without a non-empty digest.
pub fn classify_files(manifest: &BatchManifest) -> Result<BatchFiles, CribleError> {
    let mut files = BatchFiles {
        documents: Vec::new(),
        passages: Vec::new(),
        deletions: None,
    };
    for (name, checksum) in &manifest.files {
        if !checksum.starts_with("blake3:") || checksum.len() == "blake3:".len() {
            return Err(json::invalid(format!(
                "checksum of '{name}' at $.files must be 'blake3:' followed by a non-empty digest, got '{checksum}'"
            )));
        }
        if name == "deletions.parquet" {
            files.deletions = Some(name.clone());
        } else if matches_parquet_name(name, "documents") {
            files.documents.push(name.clone());
        } else if matches_parquet_name(name, "passages") {
            files.passages.push(name.clone());
        } else {
            return Err(json::invalid(format!(
                "unknown file '{name}' at $.files; expected documents[-NNNN].parquet, passages[-NNNN].parquet or deletions.parquet"
            )));
        }
    }
    Ok(files)
}

fn matches_parquet_name(name: &str, stem: &str) -> bool {
    if name == format!("{stem}.parquet") {
        return true;
    }
    let Some(rest) = name.strip_prefix(stem) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix('-') else {
        return false;
    };
    let Some(digits) = rest.strip_suffix(".parquet") else {
        return false;
    };
    digits.len() >= 4 && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn check_counts(manifest: &BatchManifest, files: &BatchFiles) -> Result<(), CribleError> {
    let counts = manifest.counts;

    if files.documents().is_empty() {
        return Err(json::invalid(
            "files at $.files must list at least one documents.parquet file",
        ));
    }
    if files.passages().is_empty() {
        return Err(json::invalid(
            "files at $.files must list at least one passages.parquet file",
        ));
    }
    if counts.documents == 0 && counts.passages != 0 {
        return Err(json::invalid(format!(
            "counts declare {} passages for zero documents at $.counts; every passage belongs to a document",
            counts.passages
        )));
    }
    if counts.passages < counts.documents {
        return Err(json::invalid(format!(
            "counts declare {} documents and only {} passages at $.counts; every document has at least one passage",
            counts.documents, counts.passages
        )));
    }
    if counts.deletions == 0 && files.deletions().is_some() {
        return Err(json::invalid(
            "files list a deletions file but counts.deletions is zero at $.counts.deletions",
        ));
    }
    if counts.deletions > 0 && files.deletions().is_none() {
        return Err(json::invalid(format!(
            "counts declare {} deletions at $.counts.deletions but files at $.files do not list deletions.parquet",
            counts.deletions
        )));
    }
    Ok(())
}

/// True for a blake3 digest written as 64 lowercase hexadecimal digits.
///
/// The strict form is required of the `blake3` member of a model file and of
/// the checksums of a batch's files. The manifest validation of P0 only checks
/// the `blake3:` prefix; the digest itself is verified at attach time.
#[must_use]
pub fn is_blake3_digest(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

/// Parses a `blake3:<hex>` checksum into its digest.
///
/// The hexadecimal form is strict: 64 lowercase digits after the prefix, so
/// the digest has exactly the 32 bytes of a blake3 hash.
///
/// # Errors
///
/// [`ErrorCode::InvalidRequest`] when the prefix is missing or the digest is
/// not 64 lowercase hexadecimal characters.
pub fn parse_checksum(checksum: &str) -> Result<blake3::Hash, CribleError> {
    let Some(digest) = checksum.strip_prefix("blake3:") else {
        return Err(json::invalid(format!(
            "checksum '{checksum}' must start with 'blake3:'"
        )));
    };
    if !is_blake3_digest(digest) {
        return Err(json::invalid(format!(
            "checksum '{checksum}' must be 'blake3:' followed by 64 lowercase hexadecimal digits"
        )));
    }
    blake3::Hash::from_hex(digest).map_err(|error| {
        json::invalid(format!(
            "checksum '{checksum}' is not a valid blake3 digest: {error}"
        ))
    })
}
