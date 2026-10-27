// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `PLANIFIER-STRATEGIE` — ADR-0074, decision 2: `search.exact_threshold` is
//! optional in `index.json`, and the creation materialises it.
//!
//! References: `docs/phases/P4-recherche-rapide.md` (`PLANIFIER-STRATEGIE`,
//! phase P4), `docs/03-recherche.md` ("Réglages de précision": the threshold
//! defaults to `N` at creation and is then materialised), `docs/04-donnees-et-
//! ingestion.md` ("Ce qui est figé, ce qui est réglable") and ADR-0074,
//! decisions 1 and 2.
//!
//! # What the decision says
//!
//! - the key is optional in `index.json`; absent at creation, it takes the
//!   value of `search.rescore_candidates` at that moment;
//! - the creation **writes** that value into the stored configuration, so the
//!   file is self-descriptive and a later `PATCH` of `N` cannot move the
//!   threshold;
//! - an index whose configuration already names the key keeps its value, and
//!   the file is written verbatim.
//!
//! # What each test pins
//!
//! | Test | Criterion |
//! |---|---|
//! | `an_index_created_without_the_threshold_materialises_n` | absent ⇒ `N`, written in `index.json` |
//! | `a_patch_of_n_does_not_move_the_materialised_threshold` | the stored threshold survives a `PATCH` of `N` |
//! | `an_index_that_names_the_threshold_keeps_it_verbatim` | the key present is read as is, and the file is not reformatted |

use std::fs;
use std::path::PathBuf;

use crible_core::config::{IndexConfig, IndexPatch, Schema, apply_patch};
use crible_core::ids::Epoch;
use crible_store::generation::{GenerationStore, INDEX_CONFIG_FILE_NAME, write_index_config};
use serde_json::Value;

/// Epoch a creation starts on, as `crible index create` does.
const EPOCH: u32 = 1;

/// JSON schema of the test index: no field, so no filter structure is built.
const SCHEMA: &str = r#"{ "document_fields": {}, "passage_fields": {} }"#;

/// Configuration of `docs/04` with `rescore_candidates = rescore` and,
/// optionally, an explicit `exact_threshold`.
fn config(rescore: u32, exact_threshold: Option<u32>) -> String {
    let threshold = exact_threshold.map_or_else(String::new, |threshold| {
        format!(r#""exact_threshold": {threshold}, "#)
    });
    format!(
        r#"{{
  "name": "planifier-strategie",
  "model": "m",
  "reference_precision": "fp16",
  "schema": {SCHEMA},
  "cluster_by": [],
  "lexical": {{ "enabled": false, "language": "fr", "positions": false }},
  "search": {{
    "scan": "binary",
    "rescore_candidates": {rescore},
    {threshold}"candidates_per_branch": 100,
    "fusion": {{ "method": "rrf" }}
  }},
  "ingest": {{ "acceptance_sample": 1, "acceptance_min_cosine": 0.9 }},
  "compaction": {{ "dead_ratio": 0.1 }}
}}"#
    )
}

/// Schema of [`SCHEMA`].
fn schema() -> Schema {
    Schema::from_json_str(SCHEMA)
        .unwrap_or_else(|error| panic!("the schema must be valid: {error}"))
}

/// Creates an index from `raw` and returns its directory and the stored
/// configuration, read back from `index.json`.
fn create(raw: &str) -> (tempfile::TempDir, PathBuf, String) {
    let temp = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
    let index = temp.path().join("indexes").join("planifier-strategie");
    write_index_config(&index, raw)
        .unwrap_or_else(|error| panic!("the configuration must be written: {error}"));
    GenerationStore::create(&index, Epoch::new(EPOCH), &schema())
        .unwrap_or_else(|error| panic!("the index must be created: {error}"));
    let stored = fs::read_to_string(index.join(INDEX_CONFIG_FILE_NAME))
        .unwrap_or_else(|error| panic!("the stored configuration must be readable: {error}"));
    (temp, index.clone(), stored)
}

/// Parses a stored configuration, refusing anything invalid.
fn parse(stored: &str) -> IndexConfig {
    IndexConfig::from_json_str(stored)
        .unwrap_or_else(|error| panic!("the stored configuration must parse: {error}"))
}

/// ADR-0074, decision 2: absent at creation, `search.exact_threshold` takes the
/// value of `search.rescore_candidates`, and the creation writes it into
/// `index.json`.
///
/// The two halves are checked: the configuration read from the file names the
/// threshold, so a reader does not have to replay the rule, and the in-memory
/// default of the raw form already gives the same value.
#[test]
fn an_index_created_without_the_threshold_materialises_n() {
    let raw = config(1_234, None);
    assert_eq!(
        parse(&raw).search.exact_threshold,
        1_234,
        "the in-memory default of an absent threshold must be N"
    );

    let (_temp, index, stored) = create(&raw);

    assert_eq!(
        parse(&stored).search.exact_threshold,
        1_234,
        "the creation must write the threshold into index.json, equal to N"
    );
    let value: Value = serde_json::from_str(&stored)
        .unwrap_or_else(|error| panic!("the stored configuration must be JSON: {error}"));
    assert_eq!(
        value["search"]["exact_threshold"], 1_234,
        "the stored file must name search.exact_threshold, not leave it implicit"
    );
    assert_eq!(
        parse(&stored).search.rescore_candidates,
        1_234,
        "the materialisation must not change N itself"
    );
    assert!(
        index.join(INDEX_CONFIG_FILE_NAME).is_file(),
        "the index must keep its index.json"
    );
}

/// ADR-0074, decision 2: a later `PATCH` of `search.rescore_candidates` does not
/// move the stored threshold; only a `PATCH` of `exact_threshold` changes it.
///
/// The `PATCH` contract itself is unchanged (ADR-0031): the keys it does not
/// name keep their value.
#[test]
fn a_patch_of_n_does_not_move_the_materialised_threshold() {
    let (_temp, _index, stored) = create(&config(1_234, None));
    let mut config = parse(&stored);

    let patch = IndexPatch::from_json_str(r#"{ "search": { "rescore_candidates": 10 } }"#)
        .unwrap_or_else(|error| panic!("the patch must parse: {error}"));
    apply_patch(&mut config, &patch)
        .unwrap_or_else(|error| panic!("the patch must apply: {error}"));

    assert_eq!(
        config.search.rescore_candidates, 10,
        "the patch must change N"
    );
    assert_eq!(
        config.search.exact_threshold, 1_234,
        "a PATCH of N must not move the materialised threshold"
    );

    let patch = IndexPatch::from_json_str(r#"{ "search": { "exact_threshold": 7 } }"#)
        .unwrap_or_else(|error| panic!("the threshold patch must parse: {error}"));
    apply_patch(&mut config, &patch)
        .unwrap_or_else(|error| panic!("the threshold patch must apply: {error}"));
    assert_eq!(
        config.search.exact_threshold, 7,
        "only a PATCH of exact_threshold changes the threshold"
    );
    assert_eq!(
        config.search.rescore_candidates, 10,
        "the threshold patch must leave N alone"
    );
}

/// ADR-0074, decision 2: a configuration that already names the threshold is
/// read as is, and the creation does not reformat a file it does not have to
/// complete.
#[test]
fn an_index_that_names_the_threshold_keeps_it_verbatim() {
    let raw = config(1_234, Some(7));

    let (_temp, _index, stored) = create(&raw);

    assert_eq!(
        parse(&stored).search.exact_threshold,
        7,
        "an explicit threshold must be read as is, not replaced by N"
    );
    assert_eq!(
        stored, raw,
        "a configuration that names every default must be stored verbatim"
    );
}
