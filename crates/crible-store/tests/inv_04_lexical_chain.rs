// SPDX-License-Identifier: Apache-2.0

//! ADR-0106 — the analysis chain of the lexical index is recorded beside its
//! generation, and a write open rebuilds an index built with another chain.
//!
//! References: `docs/02-architecture.md` ("Reprise au démarrage"),
//! `docs/03-recherche.md` ("Branche lexicale"), `docs/04-donnees-et-ingestion.md`
//! (`lexical.language`, `lexical.positions`), `docs/08-invariants-et-tests.md`
//! (INV-04), ADR-0087 (recovery on a write open), ADR-0105 (the `PATCH` job) and
//! ADR-0106 (payload of the commit, comparison, legacy payload).
//!
//! What each test pins:
//!
//! | Test | Criterion of the fiche |
//! |---|---|
//! | `inv_04_a_commit_records_the_chain_of_the_configuration` | the chain is written in the payload of the commit |
//! | `inv_04_a_write_open_rebuilds_an_index_built_with_another_chain` | a changed chain rebuilds before serving |
//! | `inv_04_a_read_only_open_refuses_another_chain_and_keeps_the_files` | the read-only open refuses and writes nothing |
//! | `inv_04_a_legacy_payload_is_served_without_rebuild` | a payload of the previous format is read, never rebuilt |
//! | `inv_04_a_compaction_records_the_current_chain` | a compaction writes the current chain |
//!
//! ```text
//! cargo test -p crible-store --test inv_04_lexical_chain
//! ```

// The fixtures of this file call `panic!` outside the `#[test]` functions, as
// the other integration tests do; see `tests/common/mod.rs`.
#![allow(clippy::panic)]

mod common;

use std::fs;
use std::path::Path;

use crible_core::config::{IndexConfig, Schema};
use crible_core::model::ModelConfig;
use crible_store::attach::attach;
use crible_store::generation::GenerationStore;
use crible_store::lexical::{AnalysisChain, LexicalStatus, read_state};

/// External model of the fixture batch: four dimensions, normalized vectors.
fn fixture_model() -> ModelConfig {
    ModelConfig::from_json_str(
        r#"{ "name": "lexical-chain-external", "kind": "external", "dims": 4, "normalize": true }"#,
    )
    .unwrap_or_else(|error| panic!("the fixture model configuration must be valid: {error}"))
}

/// Reads the configuration the index declares.
fn index_config_of(index: &Path) -> IndexConfig {
    let raw = fs::read_to_string(index.join("index.json")).unwrap_or_else(|error| {
        panic!(
            "the index configuration of {} must be readable: {error}",
            index.display()
        )
    });
    IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("the index configuration must be valid: {error}"))
}

/// Creates an index whose lexical branch is enabled, then attaches two
/// documents: generation 1, two live passages, a tantivy commit at `Gt = 1`.
fn index_with_a_passage() -> (tempfile::TempDir, std::path::PathBuf, Schema) {
    let (temp, index) = common::temp_index();
    let schema = common::fixture_schema();
    let config = common::test_config_with_lexical("test", &schema, true);
    let mut store = common::create_store_with_config(&index, &schema, &config);
    let batch = common::write_batch(
        &temp.path().join("batches"),
        "lexical-chain",
        &common::TWO_DOCUMENTS,
    );
    attach(
        &mut store,
        &batch,
        &index_config_of(&index),
        &fixture_model(),
        None,
        None,
    )
    .unwrap_or_else(|error| {
        panic!(
            "the fixture attach of {} must succeed: {}: {}",
            batch.display(),
            error.code().as_str(),
            error.message()
        )
    });
    (temp, index, schema)
}

/// Rewrites `index.json` with `language`, as the `PATCH` job of ADR-0105 does,
/// after the store of the creation has been dropped.
fn set_language(index: &Path, language: &str) {
    let mut config = index_config_of(index);
    config.lexical.language = language.to_owned();
    crible_store::generation::rewrite_index_config(index, &config.to_json_string())
        .unwrap_or_else(|error| panic!("the configuration must be rewritten: {error}"));
}

/// Rewrites the payload of the tantivy commit to the format of the previous
/// build: the bare generation number (ADR-0106, decision 3).
fn write_legacy_payload(index: &Path) {
    let path = common::epoch_dir(index).join("lexical").join("meta.json");
    let raw = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "the tantivy metadata {} must be readable: {error}",
            path.display()
        )
    });
    let mut meta: serde_json::Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("the tantivy metadata must be JSON: {error}"));
    meta["payload"] = serde_json::Value::String("1".to_owned());
    fs::write(&path, meta.to_string()).unwrap_or_else(|error| {
        panic!(
            "the tantivy metadata {} must be writable: {error}",
            path.display()
        )
    });
}

#[test]
fn inv_04_a_commit_records_the_chain_of_the_configuration() {
    // ADR-0106, decision 1: the payload of the last commit names the language
    // and the positions flag the text was analysed with.
    let (_temp, index, _schema) = index_with_a_passage();
    let state = read_state(&common::epoch_dir(&index))
        .unwrap_or_else(|error| panic!("the lexical state must be readable: {error}"));
    assert_eq!(
        state.chain,
        Some(AnalysisChain {
            language: "fr".to_owned(),
            positions: false,
        }),
        "the commit records the chain of index.json"
    );
}

#[test]
fn inv_04_a_write_open_rebuilds_an_index_built_with_another_chain() {
    // ADR-0106, decision 4: a write open compares the recorded chain with the
    // configuration and rebuilds before serving. The generation does not move:
    // the rebuild is a derived file of the same generation.
    let (_temp, index, schema) = index_with_a_passage();
    set_language(&index, "en");

    let store = GenerationStore::open(&index, &schema)
        .unwrap_or_else(|error| panic!("the write open must rebuild and succeed: {error}"));
    assert_eq!(
        store.generation().get(),
        1,
        "the rebuild publishes the same generation"
    );
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::Ready {
            committed: crible_core::ids::Generation::new(1)
        },
        "the rebuilt index describes the committed generation"
    );
    drop(store);

    let state = read_state(&common::epoch_dir(&index))
        .unwrap_or_else(|error| panic!("the rebuilt lexical state must be readable: {error}"));
    assert_eq!(
        state.chain,
        Some(AnalysisChain {
            language: "en".to_owned(),
            positions: false,
        }),
        "the rebuilt index records the new chain"
    );
}

#[test]
fn inv_04_a_read_only_open_refuses_another_chain_and_keeps_the_files() {
    // ADR-0106, decision 4, second half: a read-only open writes nothing. It
    // reports `OtherChain`, and the payload under `lexical/` keeps the chain it
    // was built with, so the refusal is stable across reads.
    let (_temp, index, schema) = index_with_a_passage();
    set_language(&index, "en");

    let store = GenerationStore::open_read_only(&index, &schema)
        .unwrap_or_else(|error| panic!("a read-only open must not fail on another chain: {error}"));
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::OtherChain,
        "the lexical branch is refused, the dense branch stays served"
    );
    drop(store);

    let state = read_state(&common::epoch_dir(&index))
        .unwrap_or_else(|error| panic!("the lexical state must stay readable: {error}"));
    assert_eq!(
        state.chain,
        Some(AnalysisChain {
            language: "fr".to_owned(),
            positions: false,
        }),
        "a read-only open never rewrites the payload (ADR-0056)"
    );
}

#[test]
fn inv_04_a_legacy_payload_is_served_without_rebuild() {
    // ADR-0106, decision 3: a payload of the previous format (a bare generation
    // number) carries no chain. It is read as built with the chain of
    // `index.json`, so the branch serves without any rebuild, in a read-only
    // open as in a write open, and the payload keeps its old format.
    let (_temp, index, schema) = index_with_a_passage();
    write_legacy_payload(&index);

    let store = GenerationStore::open_read_only(&index, &schema)
        .unwrap_or_else(|error| panic!("the read-only open must succeed: {error}"));
    assert!(store.snapshot().lexical().is_some(), "the branch is served");
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::Ready {
            committed: crible_core::ids::Generation::new(1)
        },
        "a legacy payload describes the committed generation"
    );
    drop(store);
    let state = read_state(&common::epoch_dir(&index))
        .unwrap_or_else(|error| panic!("the lexical state must be readable: {error}"));
    assert_eq!(state.chain, None, "a read-only open writes nothing");

    let store = GenerationStore::open(&index, &schema)
        .unwrap_or_else(|error| panic!("the write open must succeed: {error}"));
    assert!(store.snapshot().lexical().is_some(), "the branch is served");
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::Ready {
            committed: crible_core::ids::Generation::new(1)
        },
        "a write open does not rebuild a legacy payload"
    );
    drop(store);
    let state = read_state(&common::epoch_dir(&index))
        .unwrap_or_else(|error| panic!("the lexical state must be readable: {error}"));
    assert_eq!(
        state.chain, None,
        "a legacy payload is read with the chain of index.json, never rebuilt"
    );
}

#[test]
fn inv_04_a_compaction_records_the_current_chain() {
    // ADR-0106, decision 5: the compaction rebuilds the lexical index of the new
    // epoch from `records.sqlite`, so the commit of the new epoch records the
    // chain of the configuration.
    let (_temp, index, schema) = index_with_a_passage();
    let config = index_config_of(&index);
    let mut store = GenerationStore::open(&index, &schema)
        .unwrap_or_else(|error| panic!("the write open must succeed: {error}"));
    let report = crible_store::compact::compact(&mut store, &config)
        .unwrap_or_else(|error| panic!("the compaction must succeed: {error}"));
    drop(store);

    let state = read_state(&common::epoch_dir_of(&index, report.to_epoch.get()))
        .unwrap_or_else(|error| panic!("the lexical state must be readable: {error}"));
    assert_eq!(
        state.chain,
        Some(AnalysisChain {
            language: "fr".to_owned(),
            positions: false,
        }),
        "the compaction records the current chain"
    );
}
