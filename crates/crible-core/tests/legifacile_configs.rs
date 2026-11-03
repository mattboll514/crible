// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! The six index configurations of Legifacile, read and validated by
//! `crible-core`.
//!
//! References: `DEFINIR-INDEX-LEGIFACILE` (P10, J10),
//! `docs/legifacile/index-legifacile.md` and `docs/04-donnees-et-ingestion.md`,
//! section "Configuration d'un index". Every file is loaded from
//! `docs/legifacile/configs/` with `IndexConfig::from_json_str`, the same path
//! `crible-store` uses for a stored `index.json`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crible_core::config::{FieldLevel, FieldType, IndexConfig};

/// The six configurations, in the order of `docs/02-architecture.md`.
const INDEXES: [&str; 6] = ["articles", "jorf", "decisions", "titres", "sarde", "dole"];

/// Declared document fields with their type, passage fields with their type,
/// and clustering of each index, as documented in
/// `docs/legifacile/index-legifacile.md`.
type Expected = (
    &'static [(&'static str, FieldType)],
    &'static [(&'static str, FieldType)],
    &'static [&'static str],
);

fn expected(name: &str) -> Expected {
    use FieldType::{Date, Integer, Keyword};
    match name {
        "articles" => (
            &[
                ("fonds", Keyword),
                ("etat", Keyword),
                ("annee", Integer),
                ("code", Keyword),
                ("num", Keyword),
                ("texte_cid", Keyword),
                ("date_debut", Date),
                ("date_fin", Date),
            ],
            &[],
            &["code"],
        ),
        "jorf" => (
            &[
                ("fonds", Keyword),
                ("etat", Keyword),
                ("annee", Integer),
                ("nature", Keyword),
                ("num", Keyword),
                ("texte_cid", Keyword),
            ],
            &[],
            &["annee"],
        ),
        "decisions" => (
            &[
                ("section", Keyword),
                ("juridiction", Keyword),
                ("formation", Keyword),
                ("nature", Keyword),
                ("numero", Keyword),
                ("annee", Integer),
                ("date_decision", Date),
            ],
            &[("partie", Keyword)],
            &["juridiction"],
        ),
        "titres" => (
            &[
                ("fonds", Keyword),
                ("nature", Keyword),
                ("etat", Keyword),
                ("annee", Integer),
                ("num", Keyword),
                ("texte_cid", Keyword),
                ("date_publi", Date),
            ],
            &[],
            &["nature"],
        ),
        "sarde" => (
            &[("theme", Keyword), ("nature_texte", Keyword)],
            &[],
            &["theme"],
        ),
        "dole" => (
            &[
                ("type", Keyword),
                ("annee", Integer),
                ("date_creation", Date),
            ],
            &[],
            &["type"],
        ),
        other => panic!("no expectation is declared for index {other}"),
    }
}

fn config_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/legifacile/configs")
        .join(format!("{name}.json"))
}

fn load(name: &str) -> IndexConfig {
    let path = config_path(name);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
    IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("index {name}: the configuration must be valid: {error}"))
}

fn declared(config: &IndexConfig, level: FieldLevel) -> BTreeMap<&str, FieldType> {
    config
        .schema
        .fields()
        .filter(|(_, field_level, _)| *field_level == level)
        .map(|(name, _, spec)| (name, spec.field_type))
        .collect()
}

#[test]
fn each_legifacile_config_is_read_and_validated() {
    for name in INDEXES {
        let config = load(name);
        assert_eq!(
            config.name, name,
            "the file docs/legifacile/configs/{name}.json declares another index name"
        );
        assert!(
            !config.cluster_by.is_empty(),
            "index {name} must declare cluster_by"
        );
    }
}

#[test]
fn each_legifacile_config_declares_the_documented_fields() {
    for name in INDEXES {
        let config = load(name);
        let (documents, passages, cluster_by) = expected(name);
        let documents: BTreeMap<&str, FieldType> = documents.iter().copied().collect();
        let passages: BTreeMap<&str, FieldType> = passages.iter().copied().collect();

        assert_eq!(
            declared(&config, FieldLevel::Document),
            documents,
            "index {name}: document_fields or their types differ from docs/legifacile/index-legifacile.md"
        );
        assert_eq!(
            declared(&config, FieldLevel::Passage),
            passages,
            "index {name}: passage_fields or their types differ from docs/legifacile/index-legifacile.md"
        );
        assert_eq!(
            config.cluster_by, cluster_by,
            "index {name}: cluster_by differs from docs/legifacile/index-legifacile.md"
        );
    }
}
