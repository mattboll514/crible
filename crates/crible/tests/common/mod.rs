// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! Fixtures shared by the batch-control tests of the `crible` crate
//! (`INV-11`, `INV-12`).
//!
//! Both test binaries drive `crible attach` on the versioned fake encoder of
//! `crates/crible-encode/tests/fixtures/fake-model`, and both need the same
//! expected vectors: the graph returns `[mean of the input ids, 1, 1, …]` on
//! every row, CLS pooling keeps that row, and the engine normalizes it. The
//! vocabulary is read from the versioned `tokenizer.json`, never a copy of it.
//!
//! Every test binary includes this module with `mod common`, and no binary uses
//! every fixture. Marking each unused item separately would only add noise to
//! the test build, so the module allows dead code once, here; it never hides a
//! lint in the code under test.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Dimensions of the fake encoder (`model.json` of the fixture).
pub const DIMS: usize = 768;

/// Directory of the versioned fake model.
pub fn fixture_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../crible-encode/tests/fixtures/fake-model")
}

/// Vocabulary of the fixture tokenizer: one id per whitespace-separated word.
pub fn vocabulary() -> BTreeMap<String, u32> {
    let path = fixture_directory().join("tokenizer.json");
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
    let tokenizer: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("{} must be valid JSON: {error}", path.display()));
    let entry = tokenizer
        .get("model")
        .and_then(|model| model.get("vocab"))
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("{} must declare model.vocab", path.display()));
    let mut vocabulary = BTreeMap::new();
    for (word, id) in entry {
        let id = id
            .as_u64()
            .unwrap_or_else(|| panic!("the id of '{word}' must be a positive integer"))
            as u32;
        vocabulary.insert(word.clone(), id);
    }
    vocabulary
}

/// Token ids of `text`, prefixed by the `document: ` prompt when asked.
///
/// The fixture tokenizer lowercases, splits on whitespace and maps each word to
/// one id; an unknown word is `[UNK]`, id 0, exactly as `tools/fake_encoder.py`
/// declares.
pub fn fake_ids(vocabulary: &BTreeMap<String, u32>, text: &str, with_prompt: bool) -> Vec<u32> {
    let mut ids = Vec::new();
    if with_prompt {
        let prompt = vocabulary
            .get("document:")
            .copied()
            .unwrap_or_else(|| panic!("the fixture vocabulary must hold the `document:` token"));
        ids.push(prompt);
    }
    for word in text.split_whitespace() {
        ids.push(vocabulary.get(&word.to_lowercase()).copied().unwrap_or(0));
    }
    ids
}

/// The vector the fake fp32 graph produces for `text`, normalized as the engine
/// does: CLS row `[mean(ids), 1, …, 1]`, then L2 normalization.
pub fn fake_vector(vocabulary: &BTreeMap<String, u32>, text: &str, with_prompt: bool) -> Vec<f32> {
    let ids = fake_ids(vocabulary, text, with_prompt);
    assert!(
        !ids.is_empty(),
        "the fixture text '{text}' must tokenize to at least one id"
    );
    let count = u32::try_from(ids.len()).unwrap_or_else(|_| panic!("a token count must fit a u32"));
    let sum: u32 = ids.iter().sum();
    let mean = sum as f32 / count as f32;
    let mut row = vec![1.0_f32; DIMS];
    row[0] = mean;
    normalize(&mut row);
    row
}

/// Normalizes `vector` in place the way `crible_encode::Model` does.
pub fn normalize(vector: &mut [f32]) {
    let norm = vector
        .iter()
        .map(|component| f64::from(*component) * f64::from(*component))
        .sum::<f64>()
        .sqrt() as f32;
    assert!(norm > 0.0, "a fixture vector must not be null");
    for component in vector {
        *component /= norm;
    }
}
