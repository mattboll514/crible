// SPDX-License-Identifier: Apache-2.0

//! Query encoding and its cache (`ENCODER-REQUETES`, phase P2).
//!
//! The acceptance criteria checked here are the ones that need a real ONNX
//! session: the cache serves a second identical query without a second run, the
//! truncation at `max_tokens` is reported, and an absent INT8 graph falls back
//! to fp32. The fixture is the tiny versioned model of
//! `tests/fixtures/fake-model`: a WordLevel tokenizer and a graph whose CLS row
//! is `[mean token id, 1, …, 1]`, so the normalized vector reveals which token
//! ids were used. Its INT8 graph scales the mean by two, so the two precisions
//! produce distinct vectors. It is deterministic and needs no downloaded model.
//! `tools/fake_encoder.py` regenerates all four files; do not edit them by hand.
//!
//! ONNX Runtime is still needed, through `ORT_DYLIB_PATH`, and CI has none, so
//! these tests are `#[ignore]`d: they show as ignored instead of passing without
//! checking anything. `tools/README.md` documents the library path, and the
//! command is repeated on each test:
//!
//! ```sh
//! ORT_DYLIB_PATH=/path/to/libonnxruntime.dylib \
//!   cargo test -p crible-encode --test encoder_requetes -- --ignored
//! ```

#![allow(clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crible_core::error::ErrorCode;
use crible_encode::{DEFAULT_QUERY_CACHE_ENTRIES, ModelRegistry, Precision, QueryEncoder};
use serde_json::{Value, json};
use tokenizers::Tokenizer;

const DIMS: usize = 768;

/// Directory of the versioned fake model.
fn fixture_directory() -> PathBuf {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-model");
    assert!(
        directory.join("model.json").is_file(),
        "{} is missing; the fake model fixture is versioned",
        directory.display()
    );
    directory
}

/// Copies the fixture into a temporary directory, optionally without its INT8
/// graph, and returns the directory root.
///
/// The `model.json` of the fixture is rewritten with the hashes of the copied
/// files, so the variant without INT8 is consistent: `files.int8` is removed
/// and the file is not copied.
fn model_directory(include_int8: bool) -> (tempfile::TempDir, PathBuf) {
    let fixture = fixture_directory();
    let temp = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("a temporary directory is created: {error}"));
    let root = temp.path().to_path_buf();
    for name in ["model.onnx", "tokenizer.json"] {
        std::fs::copy(fixture.join(name), root.join(name))
            .unwrap_or_else(|error| panic!("{name} is copied: {error}"));
    }
    if include_int8 {
        std::fs::copy(
            fixture.join("model.int8.onnx"),
            root.join("model.int8.onnx"),
        )
        .unwrap_or_else(|error| panic!("model.int8.onnx is copied: {error}"));
    }
    let raw = std::fs::read_to_string(fixture.join("model.json"))
        .unwrap_or_else(|error| panic!("model.json is readable: {error}"));
    let mut config: serde_json::Value =
        serde_json::from_str(&raw).unwrap_or_else(|error| panic!("model.json is JSON: {error}"));
    if !include_int8 {
        let files = config["files"]
            .as_object_mut()
            .unwrap_or_else(|| panic!("model.json carries a files object"));
        files.remove("int8");
    }
    let config = serde_json::to_string_pretty(&config)
        .unwrap_or_else(|error| panic!("model.json serializes: {error}"));
    std::fs::write(root.join("model.json"), config)
        .unwrap_or_else(|error| panic!("model.json is written: {error}"));
    (temp, root)
}

/// Rewrites `model.json` of a temporary model directory.
fn edit_model_json(root: &Path, edit: impl FnOnce(&mut Value)) {
    let path = root.join("model.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("model.json is readable: {error}"));
    let mut config: Value =
        serde_json::from_str(&raw).unwrap_or_else(|error| panic!("model.json is JSON: {error}"));
    edit(&mut config);
    let config = serde_json::to_string_pretty(&config)
        .unwrap_or_else(|error| panic!("model.json serializes: {error}"));
    std::fs::write(path, config).unwrap_or_else(|error| panic!("model.json is written: {error}"));
}

/// Declares a truncation in `tokenizer.json` and updates its hash in
/// `model.json`, as a real tokenizer does (`max_length: 8192` in mDenseOn).
fn declare_tokenizer_truncation(root: &Path, max_length: u32) {
    let path = root.join("tokenizer.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("tokenizer.json is readable: {error}"));
    let mut tokenizer: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("tokenizer.json is JSON: {error}"));
    tokenizer["truncation"] = json!({
        "direction": "Right",
        "max_length": max_length,
        "strategy": "LongestFirst",
        "stride": 0,
    });
    let contents = serde_json::to_string_pretty(&tokenizer)
        .unwrap_or_else(|error| panic!("tokenizer.json serializes: {error}"));
    std::fs::write(&path, &contents)
        .unwrap_or_else(|error| panic!("tokenizer.json is written: {error}"));
    let digest = blake3::hash(contents.as_bytes()).to_hex().to_string();
    edit_model_json(root, |config| {
        config["files"]["tokenizer"]["blake3"] = Value::String(digest);
    });
}

/// Declares an INT8 file without writing it: the directory is then damaged.
fn declare_missing_int8(root: &Path) {
    edit_model_json(root, |config| {
        config["files"]["int8"] = json!({
            "path": "model.int8.onnx",
            "blake3": "0".repeat(64),
        });
    });
}

/// Ids of the fixture tokenizer for `input`, prompt included.
fn token_ids(input: &str) -> Vec<u32> {
    let tokenizer = Tokenizer::from_file(fixture_directory().join("tokenizer.json"))
        .unwrap_or_else(|error| panic!("the fixture tokenizer loads: {error}"));
    tokenizer
        .encode(input, true)
        .unwrap_or_else(|error| panic!("the input is tokenized: {error}"))
        .get_ids()
        .to_vec()
}

/// First two components of the normalized vector the fixture graph produces
/// from `ids`: `[mean, 1, …, 1]` divided by its norm, scaled for INT8.
fn expected_vector(ids: &[u32], scale: f64) -> (f64, f64) {
    let mean = ids.iter().map(|id| f64::from(*id)).sum::<f64>() / ids.len() as f64;
    let mean = mean * scale;
    let norm = (mean * mean + (DIMS - 1) as f64).sqrt();
    (mean / norm, 1.0 / norm)
}

#[test]
#[ignore = "needs ONNX Runtime (ORT_DYLIB_PATH); run: ORT_DYLIB_PATH=/path/to/libonnxruntime.dylib cargo test -p crible-encode --test encoder_requetes -- --ignored"]
fn serves_the_second_identical_query_from_the_cache() {
    // Acceptance criterion: the cache serves the second identical query. The
    // miss counter stays at one, so the second call did not run ONNX again.
    let (_temp, root) = model_directory(true);
    let encoder = QueryEncoder::new(
        &root,
        Arc::new(ModelRegistry::new()),
        DEFAULT_QUERY_CACHE_ENTRIES,
    );

    let first = encoder
        .encode_query("droit du contrat", Precision::Int8)
        .expect("the first query is encoded");
    assert_eq!(first.precision, Precision::Int8, "INT8 is the default");
    assert!(!first.truncated, "a short query is not truncated");
    assert_eq!(first.vector.len(), DIMS);
    // The fixture INT8 graph scales the pooled mean by two: its vector is
    // recognizably the INT8 one, not a copy of the fp32 graph.
    let ids = token_ids("query: droit du contrat");
    let (expected_mean, expected_other) = expected_vector(&ids, 2.0);
    assert!((f64::from(first.vector[0]) - expected_mean).abs() < 1e-5);
    assert!((f64::from(first.vector[1]) - expected_other).abs() < 1e-5);
    let stats = encoder.cache_stats().expect("the cache is readable");
    assert_eq!((stats.hits, stats.misses), (0, 1), "the first call misses");

    let second = encoder
        .encode_query("droit du contrat", Precision::Int8)
        .expect("the second query is served");
    assert_eq!(second.vector, first.vector, "the cached vector is returned");
    let stats = encoder.cache_stats().expect("the cache is readable");
    assert_eq!((stats.hits, stats.misses), (1, 1), "the second call hits");
    assert_eq!(stats.entries, 1, "one entry for one text and precision");

    // The precision is part of the key: the same text in fp32 is another entry
    // and another ONNX run (ADR-0051).
    let fp32 = encoder
        .encode_query("droit du contrat", Precision::Fp32)
        .expect("the fp32 query is encoded");
    assert_eq!(fp32.precision, Precision::Fp32);
    assert_ne!(
        fp32.vector, first.vector,
        "the two graphs produce distinct vectors"
    );
    let (expected_mean, expected_other) = expected_vector(&ids, 1.0);
    assert!((f64::from(fp32.vector[0]) - expected_mean).abs() < 1e-5);
    assert!((f64::from(fp32.vector[1]) - expected_other).abs() < 1e-5);
    let stats = encoder.cache_stats().expect("the cache is readable");
    assert_eq!((stats.hits, stats.misses), (1, 2), "fp32 is a separate key");
    assert_eq!(stats.entries, 2);
}

#[test]
fn a_declared_but_missing_int8_file_is_an_integrity_error() {
    // The fallback only covers a model that declares no INT8 graph. A model
    // that declares one which is missing is damaged: it must fail, not serve
    // fp32 silently. The refusal happens before ONNX Runtime, so this test runs
    // in CI.
    let (_temp, root) = model_directory(false);
    declare_missing_int8(&root);
    let encoder = QueryEncoder::new(&root, Arc::new(ModelRegistry::new()), 8);

    let error = encoder
        .encode_query("droit", Precision::Int8)
        .expect_err("a declared but missing INT8 file is refused");

    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(
        error.message().contains("model.int8.onnx"),
        "the message names the file: {}",
        error.message()
    );
    assert!(
        error.message().contains("is missing"),
        "the message says what is wrong: {}",
        error.message()
    );
}

#[test]
#[ignore = "needs ONNX Runtime (ORT_DYLIB_PATH); run: ORT_DYLIB_PATH=/path/to/libonnxruntime.dylib cargo test -p crible-encode --test encoder_requetes -- --ignored"]
fn counts_the_prompt_and_the_special_tokens_in_max_tokens() {
    // Acceptance criterion: truncation is signalled, and the prompt counts.
    // The fixture prompt `query: ` is one id (measured below), so a body of 31
    // words is exactly 32 ids and must not be truncated, while 32 words are 33
    // ids and must be. The vectors prove that the model received the first 31
    // ids plus the final id, not another count.
    let (_temp, root) = model_directory(true);
    let encoder = QueryEncoder::new(&root, Arc::new(ModelRegistry::new()), 8);
    let words = [
        "droit",
        "contrat",
        "personne",
        "responsabilite",
        "code",
        "article",
    ];

    let prompt_ids = token_ids("query: ");
    assert_eq!(prompt_ids.len(), 1, "the fixture prompt is one id");
    assert_eq!(prompt_ids[0], 1, "the prompt id is `query:`");

    let body_at_limit = (0..31)
        .map(|index| words[index % words.len()])
        .collect::<Vec<_>>()
        .join(" ");
    let at_limit_input = format!("query: {body_at_limit}");
    let at_limit_ids = token_ids(&at_limit_input);
    assert_eq!(at_limit_ids.len(), 32, "31 words plus the prompt");

    let at_limit = encoder
        .encode_query(&body_at_limit, Precision::Int8)
        .expect("a query at the limit is encoded");
    assert!(!at_limit.truncated, "32 ids fit max_tokens = 32");
    let (expected_mean, expected_other) = expected_vector(&at_limit_ids, 2.0);
    assert!((f64::from(at_limit.vector[0]) - expected_mean).abs() < 1e-5);
    assert!((f64::from(at_limit.vector[1]) - expected_other).abs() < 1e-5);

    let body_over = (0..32)
        .map(|index| words[index % words.len()])
        .collect::<Vec<_>>()
        .join(" ");
    let over_input = format!("query: {body_over}");
    let over_ids = token_ids(&over_input);
    assert_eq!(over_ids.len(), 33, "32 words plus the prompt");

    let over = encoder
        .encode_query(&body_over, Precision::Int8)
        .expect("an over-long query is truncated, not refused");
    assert!(over.truncated, "33 ids exceed max_tokens = 32");
    let mut kept = over_ids[..31].to_vec();
    kept.push(*over_ids.last().expect("the input is not empty"));
    let (expected_mean, expected_other) = expected_vector(&kept, 2.0);
    assert!((f64::from(over.vector[0]) - expected_mean).abs() < 1e-5);
    assert!((f64::from(over.vector[1]) - expected_other).abs() < 1e-5);
}

#[test]
#[ignore = "needs ONNX Runtime (ORT_DYLIB_PATH); run: ORT_DYLIB_PATH=/path/to/libonnxruntime.dylib cargo test -p crible-encode --test encoder_requetes -- --ignored"]
fn ignores_the_truncation_declared_by_the_tokenizer() {
    // The production tokenizer.json declares `max_length: 8192`. Crible removes
    // it (CHARGER-MODELE) and truncates only at `max_tokens`, with the flag of
    // ADR-0051; otherwise a long query would be cut silently and reported as
    // `truncated: false` (INV-11). The fixture declares no truncation, so this
    // test builds a variant that declares four tokens, hashes it, and compares
    // it with the clear fixture: same seven-token query, same vector.
    let (_truncated_temp, truncated_root) = model_directory(true);
    declare_tokenizer_truncation(&truncated_root, 4);
    let (_clear_temp, clear_root) = model_directory(true);
    let truncated = QueryEncoder::new(&truncated_root, Arc::new(ModelRegistry::new()), 8);
    let clear = QueryEncoder::new(&clear_root, Arc::new(ModelRegistry::new()), 8);

    let body = "droit contrat personne responsabilite code article";
    let input = format!("query: {body}");
    let ids = token_ids(&input);
    assert_eq!(ids.len(), 7, "six words plus the prompt");

    // The variant really declares a four-token limit: a plain Tokenizer cuts
    // this input, which proves the test would catch a missing
    // `with_truncation(None)`.
    let declared = Tokenizer::from_file(truncated_root.join("tokenizer.json"))
        .unwrap_or_else(|error| panic!("the variant tokenizer loads: {error}"));
    let cut = declared
        .encode(input.as_str(), true)
        .unwrap_or_else(|error| panic!("the variant tokenizer encodes: {error}"));
    assert_eq!(
        cut.get_ids().len(),
        4,
        "the variant tokenizer truncates at four ids"
    );

    let with_declared_limit = truncated
        .encode_query(body, Precision::Fp32)
        .expect("the declared tokenizer limit is removed");
    assert!(
        !with_declared_limit.truncated,
        "seven ids fit max_tokens = 32; the tokenizer's four do not apply"
    );
    let without_limit = clear
        .encode_query(body, Precision::Fp32)
        .expect("the clear tokenizer encodes the same query");
    assert_eq!(
        with_declared_limit.vector, without_limit.vector,
        "the declared tokenizer limit did not cut the input at four ids"
    );
    let (expected_mean, expected_other) = expected_vector(&ids, 1.0);
    assert!((f64::from(with_declared_limit.vector[0]) - expected_mean).abs() < 1e-5);
    assert!((f64::from(with_declared_limit.vector[1]) - expected_other).abs() < 1e-5);

    // The truncation flag follows max_tokens, not the tokenizer: 41 ids are
    // cut at 32 even though the tokenizer declares four.
    let long_body = vec!["droit"; 40].join(" ");
    let long = truncated
        .encode_query(&long_body, Precision::Fp32)
        .expect("the over-long query is truncated at max_tokens");
    assert!(long.truncated, "41 ids exceed max_tokens = 32");
    assert_eq!(long.vector.len(), DIMS, "the vector is still produced");
}

#[test]
#[ignore = "needs ONNX Runtime (ORT_DYLIB_PATH); run: ORT_DYLIB_PATH=/path/to/libonnxruntime.dylib cargo test -p crible-encode --test encoder_requetes -- --ignored"]
fn falls_back_to_fp32_when_the_int8_file_is_absent() {
    // Deliverable: the fallback to fp32 when the INT8 graph is absent. The
    // encoder reports the precision it actually used, so MES-04 can see it.
    let (_temp, root) = model_directory(false);
    let encoder = QueryEncoder::new(&root, Arc::new(ModelRegistry::new()), 8);

    let encoded = encoder
        .encode_query("droit du contrat", Precision::Int8)
        .expect("the fp32 graph serves the query");
    assert_eq!(
        encoded.precision,
        Precision::Fp32,
        "an absent INT8 graph falls back to fp32"
    );
    assert_eq!(encoded.vector.len(), DIMS);
}

#[test]
#[ignore = "needs ONNX Runtime (ORT_DYLIB_PATH); run: ORT_DYLIB_PATH=/path/to/libonnxruntime.dylib cargo test -p crible-encode --test encoder_requetes -- --ignored"]
fn keeps_the_two_precisions_of_one_model_as_distinct_sessions() {
    // ADR-0051: the registry is keyed by (fingerprint, precision), so the two
    // graphs of one model are two sessions with one fingerprint and two loads.
    let (_temp, root) = model_directory(true);
    let registry = ModelRegistry::new();

    let int8 = registry
        .get_or_load_precision(&root, Precision::Int8)
        .expect("the INT8 graph loads");
    let fp32 = registry
        .get_or_load_precision(&root, Precision::Fp32)
        .expect("the fp32 graph loads");

    assert_eq!(int8.precision(), Precision::Int8);
    assert_eq!(fp32.precision(), Precision::Fp32);
    assert_eq!(
        int8.fingerprint(),
        fp32.fingerprint(),
        "the INT8 file stays outside the fingerprint"
    );
    assert!(!Arc::ptr_eq(&int8, &fp32), "two separate sessions");
    assert_eq!(registry.loads(), 2, "each precision loads once");
    assert_eq!(registry.len(), 2);
}
