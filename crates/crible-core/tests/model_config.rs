// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `ModelConfig`, the frozen/replaceable split of its fields, and the model
//! fingerprint.
//!
//! References: `docs/04-donnees-et-ingestion.md`, section "Le modèle", ADR-0028
//! (canonical form of the fingerprint), and the `DEFINIR-CONFIG-INDEX` task of
//! `docs/phases/P0-fondations.md`.
//!
//! The documented example carries placeholders (`"<sha du commit HF>"`,
//! `"…"`) for the revision and the hashes. This file substitutes concrete
//! values so that the fingerprint can be frozen; the structure is unchanged.

use crible_core::error::ErrorCode;
use crible_core::model::{ModelConfig, ModelKind};
use serde_json::{Value, json};
use std::io::Write;
use std::process::{Command, Stdio};

const FP32_BLAKE3: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const INT8_BLAKE3: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
const TOKENIZER_BLAKE3: &str = "aabbccddaabbccddaabbccddaabbccddaabbccddaabbccddaabbccddaabbccdd";
/// A well-formed digest, different from the three above.
const OTHER_BLAKE3: &str = "1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f";
const REVISION: &str = "9c1f3e2d4b5a60718293a4b5c6d7e8f901234567";

/// `models/<name>/model.json` of `docs/04-donnees-et-ingestion.md`, with the
/// placeholders replaced by concrete hashes.
const MODEL_JSON: &str = r#"{
  "name": "mdenseon",
  "kind": "onnx",
  "hf_id": "lightonai/mDenseOn",
  "revision": "9c1f3e2d4b5a60718293a4b5c6d7e8f901234567",
  "dims": 768,
  "pooling": "cls",
  "normalize": true,
  "max_tokens": 8192,
  "prompts": { "query": "query: ", "document": "document: " },
  "files": {
    "fp32":      { "path": "model.onnx",       "blake3": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" },
    "int8":      { "path": "model.int8.onnx",  "blake3": "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210" },
    "tokenizer": { "path": "tokenizer.json",   "blake3": "aabbccddaabbccddaabbccddaabbccddaabbccddaabbccddaabbccddaabbccdd" }
  }
}"#;

/// blake3 of the canonical JSON of the documented example. Any change to the
/// canonical form or to a hashed field breaks this constant on purpose.
const KNOWN_FINGERPRINT: &str = "12ffe409aedb22c444bbd686a04d0393703c44047e64c7f80ab74b9094348cb3";

fn fixture() -> Value {
    serde_json::from_str(MODEL_JSON)
        .unwrap_or_else(|error| panic!("the example is valid JSON: {error}"))
}

fn parse(raw: &str) -> ModelConfig {
    ModelConfig::from_json_str(raw).unwrap_or_else(|error| panic!("the model is valid: {error}"))
}

fn parse_value(value: &Value) -> ModelConfig {
    ModelConfig::from_value(value).unwrap_or_else(|error| panic!("the model is valid: {error}"))
}

fn fingerprint_of(mutate: fn(&mut Value)) -> String {
    let mut value = fixture();
    mutate(&mut value);
    parse_value(&value).fingerprint()
}

/// `MODEL_JSON` with non-ASCII characters in both prompts, spelled out as
/// literal UTF-8 (`ê`, `ó`) rather than as `\u00ea`/`\u00f3` escape sequences.
///
/// ADR-0028: the hashed JSON keeps non-ASCII characters as UTF-8 bytes.
const ACCENTED_MODEL_JSON: &str = r#"{
  "name": "mdenseon",
  "kind": "onnx",
  "hf_id": "lightonai/mDenseOn",
  "revision": "9c1f3e2d4b5a60718293a4b5c6d7e8f901234567",
  "dims": 768,
  "pooling": "cls",
  "normalize": true,
  "max_tokens": 8192,
  "prompts": { "query": "requête: ", "document": "documentación: " },
  "files": {
    "fp32":      { "path": "model.onnx",       "blake3": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" },
    "int8":      { "path": "model.int8.onnx",  "blake3": "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210" },
    "tokenizer": { "path": "tokenizer.json",   "blake3": "aabbccddaabbccddaabbccddaabbccddaabbccddaabbccddaabbccddaabbccdd" }
  }
}"#;

/// `ACCENTED_MODEL_JSON` with the same prompts written as `\u00e9` escapes.
/// JSON decoding yields the same strings, so the fingerprint must match.
const ESCAPED_ACCENTED_MODEL_JSON: &str = r#"{
  "name": "mdenseon",
  "kind": "onnx",
  "hf_id": "lightonai/mDenseOn",
  "revision": "9c1f3e2d4b5a60718293a4b5c6d7e8f901234567",
  "dims": 768,
  "pooling": "cls",
  "normalize": true,
  "max_tokens": 8192,
  "prompts": { "query": "requ\u00eate: ", "document": "documentaci\u00f3n: " },
  "files": {
    "fp32":      { "path": "model.onnx",       "blake3": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" },
    "int8":      { "path": "model.int8.onnx",  "blake3": "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210" },
    "tokenizer": { "path": "tokenizer.json",   "blake3": "aabbccddaabbccddaabbccddaabbccddaabbccddaabbccddaabbccddaabbccdd" }
  }
}"#;

/// Python side of the cross-language check, taken verbatim from ADR-0028:
///
/// ```text
/// python3 -c "$PYTHON_CANONICAL_JSON" < model.json   # model JSON on stdin
/// ```
///
/// The script rebuilds the fingerprinted object from the model configuration
/// and prints the canonical JSON bytes on stdout, with exactly
/// `json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False)`
/// and a UTF-8 encoding of the result.
const PYTHON_CANONICAL_JSON: &str = r#"
import json, sys

model = json.loads(sys.stdin.buffer.read().decode("utf-8"))
files = model["files"]
payload = {
    "hf_id": model["hf_id"],
    "revision": model["revision"],
    "dims": model["dims"],
    "pooling": model["pooling"],
    "normalize": model["normalize"],
    "prompts": model["prompts"],
    "files": {
        "fp32": files["fp32"]["blake3"],
        "tokenizer": files["tokenizer"]["blake3"],
    },
}
canonical = json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
sys.stdout.buffer.write(canonical.encode("utf-8"))
"#;

/// Returns the canonical JSON that Python builds from `model_json`, as UTF-8.
///
/// Fails loudly when `python3` is missing or fails: a skipped cross-language
/// check would hide an escaping regression.
fn python_canonical_payload(model_json: &str) -> String {
    let mut child = Command::new("python3")
        .args(["-c", PYTHON_CANONICAL_JSON])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| {
            panic!("python3 is required for the cross-language fingerprint test: {error}")
        });

    child
        .stdin
        .take()
        .unwrap_or_else(|| panic!("stdin is piped"))
        .write_all(model_json.as_bytes())
        .unwrap_or_else(|error| {
            panic!("python3 reads the model configuration from stdin: {error}")
        });

    let output = child
        .wait_with_output()
        .unwrap_or_else(|error| panic!("python3 exits: {error}"));
    assert!(
        output.status.success(),
        "python3 failed to build the canonical JSON: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("python3 writes the canonical JSON as UTF-8: {error}"))
}

#[test]
fn model_config_doc_example_is_parsed() {
    let model = parse(MODEL_JSON);

    assert_eq!(model.name, "mdenseon");
    assert_eq!(model.kind, ModelKind::Onnx);
    assert_eq!(model.hf_id.as_deref(), Some("lightonai/mDenseOn"));
    assert_eq!(model.revision.as_deref(), Some(REVISION));
    assert_eq!(model.dims, 768);
    assert_eq!(model.pooling.as_deref(), Some("cls"));
    assert!(model.normalize);
    assert_eq!(model.max_tokens, Some(8192));

    let prompts = model.prompts.as_ref().expect("prompts are declared");
    assert_eq!(prompts.query, "query: ");
    assert_eq!(prompts.document, "document: ");

    let files = model.files.as_ref().expect("files are declared");
    assert_eq!(files.fp32.path, "model.onnx");
    assert_eq!(files.fp32.blake3, FP32_BLAKE3);
    assert_eq!(files.tokenizer.path, "tokenizer.json");
    assert_eq!(files.tokenizer.blake3, TOKENIZER_BLAKE3);
    let int8 = files.int8.as_ref().expect("the int8 file is declared");
    assert_eq!(int8.path, "model.int8.onnx");
    assert_eq!(int8.blake3, INT8_BLAKE3);
}

#[test]
fn model_fingerprint_payload_is_the_documented_canonical_json() {
    let model = parse(MODEL_JSON);

    // `docs/04`: the fingerprint covers hf_id, revision, dims, pooling,
    // normalize, prompts, and the fp32 and tokenizer hashes. The canonical form
    // is compact JSON with object keys in byte order.
    let expected = serde_json::to_string(&json!({
        "hf_id": "lightonai/mDenseOn",
        "revision": REVISION,
        "dims": 768,
        "pooling": "cls",
        "normalize": true,
        "prompts": { "query": "query: ", "document": "document: " },
        "files": {
            "fp32": FP32_BLAKE3,
            "tokenizer": TOKENIZER_BLAKE3
        }
    }))
    .expect("the canonical payload is valid JSON");

    assert_eq!(model.fingerprint_payload(), expected);
}

#[test]
fn model_fingerprint_is_frozen_for_the_documented_example() {
    let model = parse(MODEL_JSON);
    let fingerprint = model.fingerprint();

    assert_eq!(fingerprint, KNOWN_FINGERPRINT);
    assert_eq!(fingerprint.len(), 64, "a blake3 digest is 32 bytes of hex");
    assert!(
        fingerprint
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase()),
        "the digest is lowercase hexadecimal: {fingerprint}"
    );
    assert_eq!(
        blake3::hash(model.fingerprint_payload().as_bytes())
            .to_hex()
            .as_str(),
        KNOWN_FINGERPRINT,
        "the frozen value is the blake3 of the canonical payload"
    );
}

#[test]
fn model_fingerprint_is_stable_across_reconstructions() {
    let first = parse(MODEL_JSON).fingerprint();
    let second = parse(MODEL_JSON).fingerprint();
    let third = parse_value(&fixture()).fingerprint();

    assert_eq!(first, second);
    assert_eq!(first, third);
}

#[test]
fn model_fingerprint_ignores_the_int8_file() {
    // `docs/04`: the INT8 file only serves queries, so it is not part of the
    // fingerprint. Upgrading the int8 export must not invalidate a whole index.
    let base = parse(MODEL_JSON).fingerprint();

    assert_eq!(
        fingerprint_of(|value| value["files"]["int8"]["blake3"] = json!(OTHER_BLAKE3)),
        base
    );
    assert_eq!(
        fingerprint_of(|value| {
            value["files"]["int8"]["path"] = json!("other/model.int8.onnx");
        }),
        base
    );
}

/// A named mutation of the documented example, applied before parsing.
type Mutation = (&'static str, fn(&mut Value));

#[test]
fn model_fingerprint_tracks_every_documented_field() {
    let base = parse(MODEL_JSON).fingerprint();

    let mutations: [Mutation; 7] = [
        ("hf_id", |value| {
            value["hf_id"] = json!("lightonai/other");
        }),
        ("revision", |value| {
            value["revision"] = json!("0000000000000000000000000000000000000000");
        }),
        ("dims", |value| value["dims"] = json!(512)),
        ("normalize", |value| value["normalize"] = json!(false)),
        ("prompts.query", |value| {
            value["prompts"]["query"] = json!("q: ");
        }),
        ("prompts.document", |value| {
            value["prompts"]["document"] = json!("d: ");
        }),
        ("files.fp32.blake3", |value| {
            value["files"]["fp32"]["blake3"] = json!(OTHER_BLAKE3);
        }),
    ];

    for (field, mutate) in mutations {
        let fingerprint = fingerprint_of(mutate);
        assert_ne!(
            fingerprint, base,
            "changing {field} must change the fingerprint"
        );
    }

    assert_ne!(
        fingerprint_of(|value| value["files"]["tokenizer"]["blake3"] = json!(OTHER_BLAKE3)),
        base,
        "changing the tokenizer hash must change the fingerprint"
    );
}

#[test]
fn model_fingerprint_ignores_max_tokens() {
    // `docs/04` lists the fingerprinted fields and does not include max_tokens.
    let base = parse(MODEL_JSON).fingerprint();
    assert_eq!(
        fingerprint_of(|value| value["max_tokens"] = json!(1024)),
        base
    );
}

#[test]
fn model_fingerprint_matches_python_for_non_ascii_prompt() {
    // ADR-0028: the hashed JSON is UTF-8 without escaping non-ASCII characters,
    // so `ê` stays `ê`. Python builds the canonical JSON independently from the
    // model configuration (see `PYTHON_CANONICAL_JSON`), and both sides must
    // agree on the bytes and on the blake3 digest.
    let model = parse(ACCENTED_MODEL_JSON);
    let rust_payload = model.fingerprint_payload();
    let python_payload = python_canonical_payload(ACCENTED_MODEL_JSON);

    assert_eq!(
        rust_payload, python_payload,
        "Rust and Python must hash the same canonical JSON bytes"
    );

    // The bytes really carry UTF-8, and no `\u` escape at all: an escaping
    // implementation would keep the same decoded strings but hash other bytes.
    assert!(
        rust_payload.contains('ê'),
        "the canonical JSON keeps `ê` as UTF-8: {rust_payload}"
    );
    assert!(
        rust_payload.contains('ó'),
        "the canonical JSON keeps `ó` as UTF-8: {rust_payload}"
    );
    assert!(
        !rust_payload.contains(r"\u"),
        "the canonical JSON must not escape non-ASCII characters: {rust_payload}"
    );

    let python_fingerprint = blake3::hash(python_payload.as_bytes()).to_hex().to_string();
    assert_eq!(
        model.fingerprint(),
        python_fingerprint,
        "the fingerprint is the blake3 of the UTF-8 canonical JSON built by Python"
    );
}

#[test]
fn model_fingerprint_is_equal_whether_accents_are_literal_or_escaped() {
    // Only the decoded strings are hashed, never the source spelling: the same
    // model written with `\u00e9` escapes gets the same fingerprint as the
    // literal UTF-8 spelling.
    assert_eq!(
        parse(ESCAPED_ACCENTED_MODEL_JSON).fingerprint(),
        parse(ACCENTED_MODEL_JSON).fingerprint()
    );
}

#[test]
fn model_fingerprint_tracks_a_non_ascii_prompt_edition() {
    // Editing an accent changes the bytes and therefore the fingerprint; if the
    // accents were escaped, both spellings would still decode differently, so
    // this also keeps `é` distinguishable from `e`.
    let accented = parse(ACCENTED_MODEL_JSON).fingerprint();
    let unaccented = fingerprint_of(|value| {
        value["prompts"]["query"] = json!("requete: ");
        value["prompts"]["document"] = json!("documentacion: ");
    });

    assert_ne!(accented, unaccented);
}

#[test]
fn external_model_is_valid_without_encoder_files() {
    // `docs/04`: an `external` model only declares dims and normalize.
    let model = parse(
        r#"{ "name": "client-vectors", "kind": "external", "dims": 512, "normalize": true }"#,
    );

    assert_eq!(model.kind, ModelKind::External);
    assert_eq!(model.dims, 512);
    assert!(model.normalize);
    assert!(model.files.is_none());
    assert!(model.prompts.is_none());

    let fingerprint = model.fingerprint();
    assert_eq!(fingerprint.len(), 64);
    assert_eq!(
        model.fingerprint_payload(),
        r#"{"dims":512,"normalize":true}"#
    );
    assert_ne!(
        fingerprint,
        parse(
            r#"{ "name": "client-vectors", "kind": "external", "dims": 768, "normalize": true }"#
        )
        .fingerprint()
    );
}

#[test]
fn model_kind_rejects_an_unknown_kind() {
    let mut value = fixture();
    value["kind"] = json!("torch");
    let error = ModelConfig::from_value(&value).expect_err("only onnx and external are documented");

    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    let message = error.message();
    assert!(
        message.contains("torch"),
        "the message names the value: {message}"
    );
    assert!(
        message.contains("onnx") && message.contains("external"),
        "the message names the accepted kinds: {message}"
    );
}

#[test]
fn onnx_model_without_fp32_file_is_rejected() {
    let mut value = fixture();
    value["files"]
        .as_object_mut()
        .expect("files is an object")
        .remove("fp32");
    let error = ModelConfig::from_value(&value).expect_err("the fp32 file is required");

    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(
        error.message().contains("fp32"),
        "the message names the missing file: {}",
        error.message()
    );
}

#[test]
fn onnx_model_without_tokenizer_is_rejected() {
    let mut value = fixture();
    value["files"]
        .as_object_mut()
        .expect("files is an object")
        .remove("tokenizer");
    let error = ModelConfig::from_value(&value).expect_err("the tokenizer is required");

    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(
        error.message().contains("tokenizer"),
        "the message names the missing file: {}",
        error.message()
    );
}

#[test]
fn onnx_model_without_prompts_is_rejected() {
    let mut value = fixture();
    value
        .as_object_mut()
        .expect("the model is an object")
        .remove("prompts");
    let error = ModelConfig::from_value(&value).expect_err("the prompts are required");

    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(
        error.message().contains("prompts"),
        "the message names the missing member: {}",
        error.message()
    );
}

#[test]
fn model_config_rejects_malformed_json() {
    let error = ModelConfig::from_json_str("{ \"kind\": ").expect_err("truncated JSON is invalid");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
}

/// The `export` object of `docs/04-donnees-et-ingestion.md`: the versions of the
/// libraries that produced the fp32 export (ADR-0050).
const EXPORT_VERSIONS: &str = r#"{
    "transformers": "5.14.1",
    "torch": "2.14.1",
    "onnxruntime": "1.31.0"
  }"#;

#[test]
fn export_is_accepted_and_stays_out_of_the_fingerprint() {
    // ADR-0050: `export` records the versions of the export for diagnosis. It
    // is outside the fingerprint (ADR-0028), so adding or changing it must not
    // invalidate an index nor refuse a batch.
    let without_export = parse(MODEL_JSON);
    let mut value = fixture();
    value["export"] = serde_json::from_str(EXPORT_VERSIONS).expect("the export object is JSON");
    let with_export = parse_value(&value);

    let export = with_export
        .export
        .as_ref()
        .expect("the export versions are parsed");
    assert_eq!(export.transformers, "5.14.1");
    assert_eq!(export.torch, "2.14.1");
    assert_eq!(export.onnxruntime, "1.31.0");

    assert_eq!(
        with_export.fingerprint(),
        without_export.fingerprint(),
        "the export versions stay out of the fingerprint"
    );
    assert_eq!(
        with_export.fingerprint_payload(),
        without_export.fingerprint_payload(),
        "the canonical payload does not carry the export versions"
    );
    assert!(
        !with_export.fingerprint_payload().contains("export"),
        "no export field leaks into the hashed JSON: {}",
        with_export.fingerprint_payload()
    );
    assert_eq!(
        with_export.fingerprint(),
        KNOWN_FINGERPRINT,
        "a model with export keeps the frozen fingerprint of the documented example"
    );
}

#[test]
fn export_requires_every_version_and_rejects_unknown_keys() {
    let mut missing = fixture();
    missing["export"] = json!({ "transformers": "5.14.1", "torch": "2.14.1" });
    let error = ModelConfig::from_value(&missing).expect_err("onnxruntime is required");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(
        error.message().contains("onnxruntime"),
        "the message names the missing version: {}",
        error.message()
    );

    let mut unknown = fixture();
    unknown["export"] = json!({
        "transformers": "5.14.1",
        "torch": "2.14.1",
        "onnxruntime": "1.31.0",
        "optimum": "2.0.0"
    });
    let error = ModelConfig::from_value(&unknown).expect_err("optimum is not an export version");
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(
        error.message().contains("optimum"),
        "the message names the unknown key: {}",
        error.message()
    );
}

#[test]
#[ignore = "needs the gitignored model directory; run: cargo test -p crible-core --test model_config -- --ignored"]
fn real_model_json_with_export_is_accepted() {
    // The model directory of the project is gitignored, so this test is
    // ignored unless asked for. It proves that the `model.json` written by
    // `tools/export_onnx.py`, with its root `export` object, loads without
    // complaint. Generate it first with:
    //   cd tools && uv sync --group export
    //   uv run python export_onnx.py --out ../data/models/mdenseon --overwrite
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("data/models/mdenseon/model.json");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{} is missing or unreadable ({error}); export the model before running \
             `--ignored`:\n  cd tools && uv run python export_onnx.py --out \
             ../data/models/mdenseon --overwrite",
            path.display()
        )
    });
    let model = parse(&raw);
    let export = model
        .export
        .as_ref()
        .expect("the exported model.json carries its export versions");
    assert!(!export.transformers.is_empty());
    assert!(!export.torch.is_empty());
    assert!(!export.onnxruntime.is_empty());

    let mut value: Value = serde_json::from_str(&raw).expect("the model.json is valid JSON");
    value
        .as_object_mut()
        .expect("the model is an object")
        .remove("export");
    let without_export =
        ModelConfig::from_value(&value).expect("the model is valid without export");
    assert_eq!(
        model.fingerprint(),
        without_export.fingerprint(),
        "the real export versions stay out of the fingerprint"
    );
}

#[test]
fn model_file_paths_must_stay_inside_the_model_directory() {
    // `ModelFile.path` is documented relative to the model directory. A
    // `model.json` copied from another machine must not load a file elsewhere,
    // so an absolute path or a parent component is refused at parsing time.
    for (member, bad) in [
        ("fp32", "/tmp/model.onnx"),
        ("fp32", "../model.onnx"),
        ("fp32", "sub/../../model.onnx"),
        ("tokenizer", ""),
        ("int8", "other/../model.int8.onnx"),
    ] {
        let mut value = fixture();
        value["files"][member]["path"] = json!(bad);
        let error = ModelConfig::from_value(&value)
            .expect_err("a path that escapes the model directory is refused");
        assert_eq!(error.code(), ErrorCode::InvalidRequest);
        assert!(
            error
                .message()
                .contains("must stay inside the model directory"),
            "the message says what is wrong: {}",
            error.message()
        );
        assert!(
            error.message().contains(member),
            "the message names the file role: {}",
            error.message()
        );
    }

    let mut value = fixture();
    value["files"]["fp32"]["path"] = json!("sub/model.onnx");
    let model = parse_value(&value);
    let files = model.files.expect("the files are declared");
    assert_eq!(
        files.fp32.path, "sub/model.onnx",
        "a subdirectory is allowed"
    );
}
