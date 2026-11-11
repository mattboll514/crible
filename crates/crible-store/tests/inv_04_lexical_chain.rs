// SPDX-License-Identifier: Apache-2.0

//! ADR-0106 — the analysis chain of the lexical index is recorded beside its
//! generation, and a write open rebuilds an index built with another chain.
//!
//! ADR-0148 extends both halves: the chain carries `lexical.identifiers`, the
//! payload passes to version 3 and is read field by field, so versions 1 and 2
//! are served without a rebuild and a changed `identifiers` is rebuilt on a
//! write open and refused on a read-only one.
//!
//! References: `docs/02-architecture.md` ("Reprise au démarrage"),
//! `docs/03-recherche.md` ("Branche lexicale"), `docs/04-donnees-et-ingestion.md`
//! (`lexical.language`, `lexical.positions`, `lexical.identifiers`),
//! `docs/08-invariants-et-tests.md` (INV-04), ADR-0087 (recovery on a write
//! open), ADR-0105 (the `PATCH` job), ADR-0106 (payload of the commit,
//! comparison, legacy payload) and ADR-0148 (the canonical identifier token,
//! payload version 3, field-by-field reading).
//!
//! What each test pins:
//!
//! | Test | Criterion of the fiche |
//! |---|---|
//! | `inv_04_a_commit_records_the_chain_of_the_configuration` | the chain is written in the payload of the commit |
//! | `inv_04_a_commit_writes_a_version_3_payload_with_identifiers` | the payload is version 3 and names `identifiers` |
//! | `inv_04_a_write_open_rebuilds_an_index_built_with_another_chain` | a changed chain rebuilds before serving |
//! | `inv_04_a_write_open_rebuilds_a_changed_identifiers` | a changed `identifiers` rebuilds before serving |
//! | `inv_04_a_read_only_open_refuses_another_chain_and_keeps_the_files` | the read-only open refuses and writes nothing |
//! | `inv_04_a_read_only_open_refuses_another_identifiers_and_keeps_the_files` | idem for `identifiers` |
//! | `inv_04_a_legacy_payload_is_served_without_rebuild` | a version 1 payload is read, never rebuilt |
//! | `inv_04_a_write_open_rebuilds_a_legacy_payload_asked_to_serve_identifiers_v1` | a version 1 payload reads `identifiers` as `"none"` and is rebuilt for `"v1"` |
//! | `inv_04_a_read_only_open_refuses_a_legacy_payload_asked_to_serve_identifiers_v1` | idem on a read-only open, which keeps the files |
//! | `inv_04_a_version_2_payload_is_read_with_identifiers_none_without_rebuild` | a version 2 payload reads with `"none"`, never rebuilt |
//! | `inv_04_a_compaction_records_the_current_chain` | a compaction writes the current chain |
//! | `inv_04_a_canonical_identifier_keeps_its_position_in_the_segments` | the canonical token keeps the position of its first piece through the serialization |
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

use crible_core::config::{IndexConfig, LexicalIdentifiers, Schema};
use crible_core::model::ModelConfig;
use crible_store::attach::attach;
use crible_store::generation::GenerationStore;
use crible_store::lexical::{AnalysisChain, LexicalStatus, TEXT_FIELD_NAME, read_state};

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
    index_with_a_passage_and_identifiers(None)
}

/// [`index_with_a_passage`], with `lexical.identifiers` written as
/// `identifiers` when it is `Some` (ADR-0148).
fn index_with_a_passage_and_identifiers(
    identifiers: Option<&str>,
) -> (tempfile::TempDir, std::path::PathBuf, Schema) {
    let (temp, index) = common::temp_index();
    let schema = common::fixture_schema();
    let config = common::test_config_with_lexical_identifiers("test", &schema, true, identifiers);
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

/// [`index_with_a_passage_and_identifiers`] with `"v1"`, `lexical.positions`
/// set to `true` — the shared fixture fixes it to `false` — and the text of the
/// single document set to `text`, so the positions of the chain reach the
/// segments.
fn index_with_positions_and_identifiers(
    text: &'static str,
) -> (tempfile::TempDir, std::path::PathBuf, Schema) {
    let (temp, index) = common::temp_index();
    let schema = common::fixture_schema();
    let raw = common::test_config_with_lexical_identifiers("test", &schema, true, Some("v1"));
    let mut config = IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("the fixture configuration must be valid: {error}"));
    config.lexical.positions = true;
    let mut store = common::create_store_with_config(&index, &schema, &config.to_json_string());
    let batch = common::write_batch(
        &temp.path().join("batches"),
        "lexical-positions",
        &[common::FixtureDocument {
            doc_id: "doc-a",
            text,
            vector: [1.0, 0.0, 0.0, 0.0],
        }],
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

/// Positions of `term` in the text field of every segment of the single epoch.
///
/// The postings are read back from the files, not from the stream of the
/// analyzer: that is what proves the position of the canonical identifier
/// survives the serialization of a segment.
fn term_positions(index: &Path, term: &str) -> Vec<u32> {
    use tantivy::DocSet;
    use tantivy::postings::Postings;

    let directory = common::epoch_dir(index).join("lexical");
    let lexical = tantivy::Index::open_in_dir(&directory).unwrap_or_else(|error| {
        panic!(
            "the lexical index {} must open: {error}",
            directory.display()
        )
    });
    let field = lexical
        .schema()
        .get_field(TEXT_FIELD_NAME)
        .unwrap_or_else(|error| panic!("the lexical index must declare the text field: {error}"));
    let term = tantivy::Term::from_field_text(field, term);
    let mut positions = Vec::new();
    let segments = lexical
        .searchable_segments()
        .unwrap_or_else(|error| panic!("the lexical segments must list: {error}"));
    for segment in segments {
        let reader = tantivy::index::SegmentReader::open(&segment)
            .unwrap_or_else(|error| panic!("a lexical segment must open: {error}"));
        let inverted = reader
            .inverted_index(field)
            .unwrap_or_else(|error| panic!("the text field must be inverted-indexed: {error}"));
        let found = inverted
            .read_postings(
                &term,
                tantivy::schema::IndexRecordOption::WithFreqsAndPositions,
            )
            .unwrap_or_else(|error| panic!("the postings of {term:?} must be readable: {error}"));
        let Some(mut postings) = found else { continue };
        while postings.doc() != tantivy::TERMINATED {
            postings.positions(&mut positions);
            postings.advance();
        }
    }
    positions
}

/// Rewrites `index.json` with `language`, as the `PATCH` job of ADR-0105 does,
/// after the store of the creation has been dropped.
fn set_language(index: &Path, language: &str) {
    let mut config = index_config_of(index);
    config.lexical.language = language.to_owned();
    crible_store::generation::rewrite_index_config(index, &config.to_json_string())
        .unwrap_or_else(|error| panic!("the configuration must be rewritten: {error}"));
}

/// Rewrites `index.json` with another `lexical.identifiers`, as the `PATCH` job
/// of ADR-0105 does, after the store of the creation has been dropped.
fn set_identifiers(index: &Path, identifiers: LexicalIdentifiers) {
    let mut config = index_config_of(index);
    config.lexical.identifiers = identifiers;
    crible_store::generation::rewrite_index_config(index, &config.to_json_string())
        .unwrap_or_else(|error| panic!("the configuration must be rewritten: {error}"));
}

/// Path of the `meta.json` of the tantivy index of the single epoch.
fn lexical_meta_path(index: &Path) -> std::path::PathBuf {
    common::epoch_dir(index).join("lexical").join("meta.json")
}

/// Payload of the last tantivy commit of the epoch, exactly as stored: the bare
/// decimal number of the first format, or a JSON object.
fn raw_payload(index: &Path) -> String {
    let path = lexical_meta_path(index);
    let raw = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "the tantivy metadata {} must be readable: {error}",
            path.display()
        )
    });
    let meta: serde_json::Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("the tantivy metadata must be JSON: {error}"));
    meta["payload"]
        .as_str()
        .unwrap_or_else(|| panic!("the commit must carry a payload, got {}", meta["payload"]))
        .to_owned()
}

/// Decoded payload of the last tantivy commit of the epoch.
fn payload_value(index: &Path) -> serde_json::Value {
    let payload = raw_payload(index);
    serde_json::from_str(&payload)
        .unwrap_or_else(|error| panic!("the payload {payload:?} must be a JSON object: {error}"))
}

/// Rewrites the payload of the tantivy commit of the epoch.
fn write_payload(index: &Path, payload: serde_json::Value) {
    let path = lexical_meta_path(index);
    let raw = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "the tantivy metadata {} must be readable: {error}",
            path.display()
        )
    });
    let mut meta: serde_json::Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("the tantivy metadata must be JSON: {error}"));
    meta["payload"] = serde_json::Value::String(payload.to_string());
    fs::write(&path, meta.to_string()).unwrap_or_else(|error| {
        panic!(
            "the tantivy metadata {} must be writable: {error}",
            path.display()
        )
    });
}

/// Rewrites the payload of the tantivy commit to the format of the previous
/// build: the bare generation number (ADR-0106, decision 3).
fn write_legacy_payload(index: &Path) {
    let path = lexical_meta_path(index);
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

/// Rewrites the payload to the version 2 format: an object with `language` and
/// `positions`, and no `identifiers` (ADR-0148, decision 7).
fn write_version_2_payload(index: &Path) {
    write_payload(
        index,
        serde_json::json!({
            "version": 2,
            "generation": 1,
            "language": "fr",
            "positions": false,
        }),
    );
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
            identifiers: LexicalIdentifiers::None,
        }),
        "the commit records the chain of index.json"
    );
}

#[test]
fn inv_04_a_commit_writes_a_version_3_payload_with_identifiers() {
    // ADR-0148, decision 7: the payload passes to version 3 and always names the
    // three fields of the chain, `identifiers` included, even when it is
    // `"none"`.
    let (_temp, index, _schema) = index_with_a_passage();
    let payload = payload_value(&index);
    assert_eq!(
        payload["version"], 3,
        "the payload of this build is version 3: {payload}"
    );
    assert_eq!(payload["language"], "fr", "{payload}");
    assert_eq!(payload["positions"], false, "{payload}");
    assert_eq!(
        payload["identifiers"], "none",
        "version 3 always names identifiers: {payload}"
    );

    let (_temp, index, _schema) = index_with_a_passage_and_identifiers(Some("v1"));
    let payload = payload_value(&index);
    assert_eq!(payload["version"], 3, "{payload}");
    assert_eq!(payload["identifiers"], "v1", "{payload}");
    let state = read_state(&common::epoch_dir(&index))
        .unwrap_or_else(|error| panic!("the lexical state must be readable: {error}"));
    assert_eq!(
        state.chain,
        Some(AnalysisChain {
            language: "fr".to_owned(),
            positions: false,
            identifiers: LexicalIdentifiers::V1,
        }),
        "the chain read back carries the identifiers setting"
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
            identifiers: LexicalIdentifiers::None,
        }),
        "the rebuilt index records the new chain"
    );
}

#[test]
fn inv_04_a_write_open_rebuilds_a_changed_identifiers() {
    // ADR-0148, decisions 6 and 7: the setting enters the fingerprint. An index
    // built with the key absent is `"none"`; a configuration that names `"v1"`
    // asks for another analysis, so a write open rebuilds before serving.
    let (_temp, index, schema) = index_with_a_passage();
    assert_eq!(
        payload_value(&index)["identifiers"],
        "none",
        "the fixture starts on the 1.0 chain"
    );
    set_identifiers(&index, LexicalIdentifiers::V1);

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

    let payload = payload_value(&index);
    assert_eq!(payload["version"], 3, "{payload}");
    assert_eq!(
        payload["identifiers"], "v1",
        "the rebuilt payload records the new setting: {payload}"
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
            identifiers: LexicalIdentifiers::None,
        }),
        "a read-only open never rewrites the payload (ADR-0056)"
    );
}

#[test]
fn inv_04_a_read_only_open_refuses_another_identifiers_and_keeps_the_files() {
    // ADR-0148, decisions 6 and 7, with ADR-0106, decision 4: an index built
    // with `"none"` served by a configuration that names `"v1"` is another
    // chain. The read-only open writes nothing, reports `OtherChain`, and the
    // payload keeps the setting it was built with.
    let (_temp, index, schema) = index_with_a_passage();
    let before = payload_value(&index);
    set_identifiers(&index, LexicalIdentifiers::V1);

    let store = GenerationStore::open_read_only(&index, &schema)
        .unwrap_or_else(|error| panic!("a read-only open must not fail on another chain: {error}"));
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::OtherChain,
        "the lexical branch is refused, the dense branch stays served"
    );
    drop(store);

    assert_eq!(
        payload_value(&index),
        before,
        "a read-only open never rewrites the payload (ADR-0056)"
    );
}

#[test]
fn inv_04_a_legacy_payload_is_served_without_rebuild() {
    // ADR-0106, decision 3, and ADR-0148, decision 7: a payload of the previous
    // format (a bare generation number) carries no chain. Its `language` and
    // `positions` are those of `index.json`, and its absent `identifiers` is the
    // 1.0 value, `"none"`. The fixture names no identifier form, so the branch
    // serves without any rebuild, in a read-only open as in a write open, and
    // the payload keeps its old format.
    let (_temp, index, schema) = index_with_a_passage();
    assert_eq!(
        index_config_of(&index).lexical.identifiers,
        LexicalIdentifiers::None,
        "the fixture names no identifier form, which the legacy payload reads as \"none\""
    );
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
fn inv_04_a_version_2_payload_is_read_with_identifiers_none_without_rebuild() {
    // ADR-0148, decision 7: a version 2 payload names `language` and
    // `positions` and no `identifiers`. It is read field by field, so
    // `identifiers` takes its 1.0 value, `"none"`, and the branch is served
    // without a rebuild on both opens. The payload keeps its old version.
    let (_temp, index, schema) = index_with_a_passage();
    write_version_2_payload(&index);

    let store = GenerationStore::open_read_only(&index, &schema)
        .unwrap_or_else(|error| panic!("the read-only open must succeed: {error}"));
    assert!(store.snapshot().lexical().is_some(), "the branch is served");
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::Ready {
            committed: crible_core::ids::Generation::new(1)
        },
        "a version 2 payload describes the committed generation"
    );
    drop(store);
    let state = read_state(&common::epoch_dir(&index))
        .unwrap_or_else(|error| panic!("the lexical state must be readable: {error}"));
    assert_eq!(
        state.chain,
        Some(AnalysisChain {
            language: "fr".to_owned(),
            positions: false,
            identifiers: LexicalIdentifiers::None,
        }),
        "a version 2 payload is read as built with the 1.0 chain"
    );

    let store = GenerationStore::open(&index, &schema)
        .unwrap_or_else(|error| panic!("the write open must succeed: {error}"));
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::Ready {
            committed: crible_core::ids::Generation::new(1)
        },
        "a write open does not rebuild a version 2 payload"
    );
    drop(store);

    let payload = payload_value(&index);
    assert_eq!(
        payload["version"], 2,
        "a write open that rebuilds nothing leaves the payload as it was: {payload}"
    );
    assert!(
        payload.get("identifiers").is_none(),
        "the version 2 payload carries no identifiers: {payload}"
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
            identifiers: LexicalIdentifiers::None,
        }),
        "the compaction records the current chain"
    );
}

#[test]
fn inv_04_a_canonical_identifier_keeps_its_position_in_the_segments() {
    // ADR-0148, decision 4: the canonical token carries the position of the
    // first piece and increments nothing, so its position is already used by
    // the pieces that follow it and the positions of the tokens after the
    // identifier do not move. Only an index built with
    // `lexical.positions = true` stores positions, and only a re-read of the
    // segments proves the serialization keeps that decreasing position.
    let (_temp, index, schema) = index_with_positions_and_identifiers("avant L. 1132-1 apres");

    let store = GenerationStore::open(&index, &schema)
        .unwrap_or_else(|error| panic!("the write open of the same chain must not fail: {error}"));
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::Ready {
            committed: crible_core::ids::Generation::new(1)
        },
        "the chain read back is the one of index.json: no rebuild"
    );
    drop(store);

    let state = read_state(&common::epoch_dir(&index))
        .unwrap_or_else(|error| panic!("the lexical state must be readable: {error}"));
    assert_eq!(
        state.chain,
        Some(AnalysisChain {
            language: "fr".to_owned(),
            positions: true,
            identifiers: LexicalIdentifiers::V1,
        }),
        "the commit records the positions flag and the identifier form"
    );

    assert_eq!(
        term_positions(&index, "l1132-1"),
        vec![1],
        "the canonical token is stored at the position of the `l` piece"
    );
    assert_eq!(
        term_positions(&index, "1132"),
        vec![2],
        "the piece after the canonical token keeps its 1.0 position"
    );
    assert_eq!(
        term_positions(&index, "1"),
        vec![3],
        "the last piece keeps its 1.0 position"
    );
    assert_eq!(
        term_positions(&index, "apre"),
        vec![4],
        "the token after the identifier keeps the position of the 1.0 chain"
    );
}

#[test]
fn inv_04_a_write_open_rebuilds_a_legacy_payload_asked_to_serve_identifiers_v1() {
    // ADR-0148, decision 7, with ADR-0106, decision 4: a version 1 payload
    // carries no chain, and its absent `identifiers` is the 1.0 value, `"none"`,
    // whatever `index.json` names. An index whose configuration asks for `"v1"`
    // was built by the 1.0 chain: the write open rebuilds it, and the payload
    // becomes the version 3 of the current build.
    let (_temp, index, schema) = index_with_a_passage();
    write_legacy_payload(&index);
    set_identifiers(&index, LexicalIdentifiers::V1);

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

    let payload = payload_value(&index);
    assert_eq!(
        payload["version"], 3,
        "the rebuild writes the payload of this build: {payload}"
    );
    assert_eq!(payload["generation"], 1, "{payload}");
    assert_eq!(
        payload["identifiers"], "v1",
        "the rebuilt index names the configured identifier form: {payload}"
    );
    let state = read_state(&common::epoch_dir(&index))
        .unwrap_or_else(|error| panic!("the rebuilt lexical state must be readable: {error}"));
    assert_eq!(
        state.chain,
        Some(AnalysisChain {
            language: "fr".to_owned(),
            positions: false,
            identifiers: LexicalIdentifiers::V1,
        }),
        "the rebuilt index records the configured chain"
    );
}

#[test]
fn inv_04_a_read_only_open_refuses_a_legacy_payload_asked_to_serve_identifiers_v1() {
    // ADR-0148, decision 7, with ADR-0106, decision 4, second half: the same
    // index served by a read-only open is another chain. It writes nothing, so
    // the payload keeps its version 1, and the refusal is stable across reads.
    let (_temp, index, schema) = index_with_a_passage();
    write_legacy_payload(&index);
    set_identifiers(&index, LexicalIdentifiers::V1);
    assert_eq!(
        raw_payload(&index),
        "1",
        "the fixture starts on the payload of the first format"
    );

    for _ in 0..2 {
        let store = GenerationStore::open_read_only(&index, &schema).unwrap_or_else(|error| {
            panic!("a read-only open must not fail on another chain: {error}")
        });
        assert_eq!(
            store.snapshot().lexical_status(),
            LexicalStatus::OtherChain,
            "the lexical branch is refused, the dense branch stays served"
        );
        drop(store);
    }
    assert_eq!(
        raw_payload(&index),
        "1",
        "a read-only open never rewrites the payload (ADR-0056)"
    );
}
