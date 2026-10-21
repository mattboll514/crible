// SPDX-License-Identifier: Apache-2.0

//! `BatchManifest` and `check_manifest`: format, version, mode, model
//! compatibility, counts and the file list.
//!
//! References: `docs/04-donnees-et-ingestion.md` (format de lot v1, contrôles 1
//! à 4), `docs/format-lot-v1.md`, and the `DEFINIR-FORMAT-LOT` task of
//! `docs/phases/P0-fondations.md`.

use crible_core::batch::{BatchManifest, BatchMode, check_manifest};
use crible_core::config::IndexConfig;
use crible_core::error::{CribleError, ErrorCode};
use crible_core::model::ModelConfig;
use serde_json::{Value, json};

/// `manifest.json` example of `docs/04-donnees-et-ingestion.md`, verbatim.
const MANIFEST_JSON: &str = r#"{
  "format": "crible-batch",
  "format_version": 1,
  "batch_id": "legifacile-decisions-2026-10-19",
  "index": "decisions",
  "mode": "upsert",
  "created_at": "2026-10-19T05:42:10Z",
  "producer": { "name": "legifacile-sync", "version": "…", "host": "macmini" },
  "model": {
    "hf_id": "lightonai/mDenseOn", "revision": "…", "dims": 768,
    "pooling": "cls", "normalize": true, "prompt_document": "document: ",
    "vector_dtype": "fp16",
    "runtime": { "torch": "…", "transformers": "…", "device": "mps" }
  },
  "chunker":  { "name": "decouper_juri", "version": "3", "params": { "max_tokens": 1024, "overlap": 0.15 } },
  "template": { "name": "juri_titre_corps", "version": "1" },
  "counts":   { "documents": 428, "passages": 932, "deletions": 221 },
  "files":    { "documents.parquet": "blake3:…", "passages.parquet": "blake3:…", "deletions.parquet": "blake3:…" }
}"#;

/// Index configuration example of `docs/04-donnees-et-ingestion.md`.
const INDEX_CONFIG_JSON: &str = r#"{
  "name": "decisions",
  "model": "mdenseon",
  "reference_precision": "fp16",
  "schema": {
    "document_fields": {
      "juridiction":   { "type": "keyword" },
      "formation":     { "type": "keyword" },
      "date_decision": { "type": "date" },
      "numero":        { "type": "keyword" },
      "titre":         { "type": "keyword", "filterable": false }
    },
    "passage_fields": {
      "partie": { "type": "keyword" }
    }
  },
  "cluster_by": ["juridiction"],
  "lexical":    { "enabled": true, "language": "fr", "positions": false },
  "search": {
    "scan": "binary",
    "rescore_candidates": 1000,
    "exact_threshold": 1000,
    "candidates_per_branch": 100,
    "fusion": { "method": "rrf" }
  },
  "ingest":     { "acceptance_sample": 200, "acceptance_min_cosine": 0.999 },
  "compaction": { "dead_ratio": 0.10 }
}"#;

/// ONNX model matching the manifest example, with the placeholder revision.
const MODEL_JSON: &str = r#"{
  "name": "mdenseon",
  "kind": "onnx",
  "hf_id": "lightonai/mDenseOn",
  "revision": "…",
  "dims": 768,
  "pooling": "cls",
  "normalize": true,
  "max_tokens": 8192,
  "prompts": { "query": "query: ", "document": "document: " },
  "files": {
    "fp32":      { "path": "model.onnx",       "blake3": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" },
    "tokenizer": { "path": "tokenizer.json",   "blake3": "aabbccddaabbccddaabbccddaabbccddaabbccddaabbccddaabbccddaabbccdd" }
  }
}"#;

fn manifest_value() -> Value {
    serde_json::from_str(MANIFEST_JSON)
        .unwrap_or_else(|error| panic!("the documented manifest is valid JSON: {error}"))
}

fn parse_manifest(value: &Value) -> BatchManifest {
    let raw = serde_json::to_string(value)
        .unwrap_or_else(|error| panic!("the manifest serializes: {error}"));
    BatchManifest::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("the manifest is valid: {error}"))
}

fn manifest() -> BatchManifest {
    BatchManifest::from_json_str(MANIFEST_JSON)
        .unwrap_or_else(|error| panic!("the documented manifest is valid: {error}"))
}

fn index_config() -> IndexConfig {
    IndexConfig::from_json_str(INDEX_CONFIG_JSON)
        .unwrap_or_else(|error| panic!("the documented index is valid: {error}"))
}

fn model_config() -> ModelConfig {
    ModelConfig::from_json_str(MODEL_JSON)
        .unwrap_or_else(|error| panic!("the model is valid: {error}"))
}

fn check(manifest: &BatchManifest) -> Result<(), CribleError> {
    check_manifest(manifest, &index_config(), &model_config())
}

fn mutated(mutate: impl FnOnce(&mut Value)) -> BatchManifest {
    let mut value = manifest_value();
    mutate(&mut value);
    parse_manifest(&value)
}

fn assert_invalid(result: Result<(), CribleError>, expected: &str) -> String {
    let error = match result {
        Ok(()) => panic!("the manifest must be rejected"),
        Err(error) => error,
    };
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(
        error.message().contains(expected),
        "the message names {expected}: {}",
        error.message()
    );
    error.message().to_owned()
}

#[test]
fn doc_example_manifest_is_parsed() {
    let manifest = manifest();

    assert_eq!(manifest.format, "crible-batch");
    assert_eq!(manifest.format_version, 1);
    assert_eq!(manifest.batch_id, "legifacile-decisions-2026-10-19");
    assert_eq!(manifest.index, "decisions");
    assert_eq!(manifest.mode, BatchMode::Upsert);
    assert_eq!(manifest.created_at, "2026-10-19T05:42:10Z");
    assert_eq!(manifest.producer.name, "legifacile-sync");
    assert_eq!(manifest.model.dims, 768);
    assert_eq!(manifest.model.pooling.as_deref(), Some("cls"));
    assert!(manifest.model.normalize);
    assert_eq!(
        manifest.model.prompt_document.as_deref(),
        Some("document: ")
    );
    assert_eq!(manifest.counts.documents, 428);
    assert_eq!(manifest.counts.passages, 932);
    assert_eq!(manifest.counts.deletions, 221);
    assert_eq!(manifest.files.len(), 3);
}

#[test]
fn doc_example_manifest_is_valid() {
    check(&manifest()).expect("the documented manifest matches the documented index");
}

#[test]
fn mode_defaults_to_upsert() {
    let manifest = mutated(|value| {
        value
            .as_object_mut()
            .expect("the manifest is an object")
            .remove("mode");
    });

    assert_eq!(manifest.mode, BatchMode::Upsert);
    check(&manifest).expect("upsert is the default mode");
}

#[test]
fn unknown_format_version_is_rejected() {
    let manifest = mutated(|value| value["format_version"] = json!(2));
    let message = assert_invalid(check(&manifest), "format_version");

    assert!(
        message.contains('2'),
        "the message names the version: {message}"
    );
    assert!(
        message.contains('1'),
        "the message names the supported version: {message}"
    );
}

#[test]
fn unknown_format_is_rejected() {
    let manifest = mutated(|value| value["format"] = json!("crible-lot"));
    let message = assert_invalid(check(&manifest), "format");

    assert!(
        message.contains("crible-batch"),
        "the message names the accepted format: {message}"
    );
}

#[test]
fn unknown_mode_is_rejected_when_parsing() {
    let mut value = manifest_value();
    value["mode"] = json!("merge");

    let error = BatchManifest::from_json_str(&serde_json::to_string(&value).expect("valid JSON"))
        .expect_err("only upsert and replace_all are documented");

    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(
        error.message().contains("merge"),
        "the message names the rejected mode: {}",
        error.message()
    );
}

/// One model field of the manifest and the mutation making it incompatible.
type Mutation = (&'static str, fn(&mut Value));

#[test]
fn incompatible_model_is_rejected_field_by_field() {
    let mutations: [Mutation; 7] = [
        ("hf_id", |value| {
            value["model"]["hf_id"] = json!("lightonai/other")
        }),
        ("revision", |value| {
            value["model"]["revision"] = json!("0000000000000000000000000000000000000000");
        }),
        ("dims", |value| value["model"]["dims"] = json!(512)),
        ("pooling", |value| value["model"]["pooling"] = json!("mean")),
        ("normalize", |value| {
            value["model"]["normalize"] = json!(false)
        }),
        ("prompt_document", |value| {
            value["model"]["prompt_document"] = json!("query: ");
        }),
        ("vector_dtype", |value| {
            value["model"]["vector_dtype"] = json!("fp32");
        }),
    ];

    for (field, mutate) in mutations {
        let manifest = mutated(mutate);
        let error = check(&manifest).expect_err("the model must be incompatible");
        assert_eq!(
            error.code(),
            ErrorCode::IncompatibleModel,
            "field {field} must be reported as incompatible_model"
        );
        assert!(
            error.message().contains(field),
            "the message names the field {field}: {}",
            error.message()
        );
    }
}

#[test]
fn replace_all_with_a_deletions_file_is_rejected() {
    let manifest = mutated(|value| value["mode"] = json!("replace_all"));
    let message = assert_invalid(check(&manifest), "deletions");

    assert!(
        message.contains("replace_all"),
        "the message names the mode: {message}"
    );
}

#[test]
fn replace_all_without_a_deletions_file_is_valid() {
    let manifest = mutated(|value| {
        value["mode"] = json!("replace_all");
        value["counts"]["deletions"] = json!(0);
        value["files"]
            .as_object_mut()
            .expect("files is an object")
            .remove("deletions.parquet");
    });

    check(&manifest).expect("replace_all is valid without deletions");
}

#[test]
fn deletions_count_without_a_file_is_rejected() {
    let manifest = mutated(|value| {
        value["files"]
            .as_object_mut()
            .expect("files is an object")
            .remove("deletions.parquet");
    });
    let message = assert_invalid(check(&manifest), "deletions");

    assert!(
        message.contains("221"),
        "the message names the declared count: {message}"
    );
}

#[test]
fn deletions_file_with_a_zero_count_is_rejected() {
    let manifest = mutated(|value| value["counts"]["deletions"] = json!(0));
    assert_invalid(check(&manifest), "deletions");
}

#[test]
fn missing_documents_file_is_rejected() {
    let manifest = mutated(|value| {
        value["files"]
            .as_object_mut()
            .expect("files is an object")
            .remove("documents.parquet");
    });
    assert_invalid(check(&manifest), "documents.parquet");
}

#[test]
fn missing_passages_file_is_rejected() {
    let manifest = mutated(|value| {
        value["files"]
            .as_object_mut()
            .expect("files is an object")
            .remove("passages.parquet");
    });
    assert_invalid(check(&manifest), "passages.parquet");
}

#[test]
fn unknown_file_is_rejected() {
    let manifest = mutated(|value| {
        value["files"]["other.parquet"] = json!("blake3:…");
    });
    let message = assert_invalid(check(&manifest), "other.parquet");

    assert!(
        message.contains("documents"),
        "the message names the accepted files: {message}"
    );
}

#[test]
fn checksum_without_the_blake3_prefix_is_rejected() {
    let manifest = mutated(|value| {
        value["files"]["documents.parquet"] = json!("…");
    });
    let message = assert_invalid(check(&manifest), "blake3");

    assert!(
        message.contains("documents.parquet"),
        "the message names the file: {message}"
    );
}

#[test]
fn numbered_parquet_files_are_accepted() {
    let manifest = mutated(|value| {
        let files = value["files"].as_object_mut().expect("files is an object");
        files.remove("documents.parquet");
        files.remove("passages.parquet");
        files.insert("documents-0001.parquet".to_owned(), json!("blake3:…"));
        files.insert("passages-0001.parquet".to_owned(), json!("blake3:…"));
    });

    check(&manifest).expect("numbered Parquet files are part of the format");
}

#[test]
fn passages_fewer_than_documents_are_rejected() {
    let manifest = mutated(|value| value["counts"]["passages"] = json!(100));
    assert_invalid(check(&manifest), "passages");
}

#[test]
fn passages_without_documents_are_rejected() {
    let manifest = mutated(|value| value["counts"]["documents"] = json!(0));
    assert_invalid(check(&manifest), "documents");
}

#[test]
fn manifest_for_another_index_is_rejected() {
    let manifest = mutated(|value| value["index"] = json!("textes"));
    let message = assert_invalid(check(&manifest), "textes");

    assert!(
        message.contains("decisions"),
        "the message names the target index: {message}"
    );
}

#[test]
fn empty_batch_id_is_rejected() {
    let mut value = manifest_value();
    value["batch_id"] = json!("");

    let error = BatchManifest::from_json_str(&serde_json::to_string(&value).expect("valid JSON"))
        .expect_err("a batch_id is required");

    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(error.message().contains("batch_id"));
}

#[test]
fn batch_id_cannot_escape_its_directory() {
    let mut value = manifest_value();
    value["batch_id"] = json!("../other");

    let error = BatchManifest::from_json_str(&serde_json::to_string(&value).expect("valid JSON"))
        .expect_err("a batch_id is one path segment");

    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(error.message().contains("path segment"));
}

#[test]
fn manifest_without_a_required_member_is_rejected() {
    let mut value = manifest_value();
    value
        .as_object_mut()
        .expect("the manifest is an object")
        .remove("format");

    let error = BatchManifest::from_json_str(&serde_json::to_string(&value).expect("valid JSON"))
        .expect_err("format is required");

    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(error.message().contains("format"));
}

#[test]
fn external_model_only_compares_dims_normalize_and_dtype() {
    let model = ModelConfig::from_json_str(
        r#"{ "name": "client-vectors", "kind": "external", "dims": 768, "normalize": true }"#,
    )
    .expect("an external model is valid");
    let manifest = mutated(|value| {
        value["model"]
            .as_object_mut()
            .expect("model is an object")
            .remove("hf_id");
        value["model"]
            .as_object_mut()
            .expect("model is an object")
            .remove("revision");
        value["model"]
            .as_object_mut()
            .expect("model is an object")
            .remove("pooling");
        value["model"]
            .as_object_mut()
            .expect("model is an object")
            .remove("prompt_document");
    });

    check_manifest(&manifest, &index_config(), &model)
        .expect("an external index ignores encoder fields");
}
