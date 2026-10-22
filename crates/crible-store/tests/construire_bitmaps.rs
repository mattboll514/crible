// SPDX-License-Identifier: Apache-2.0

//! `CONSTRUIRE-BITMAPS`: the filter structures of one generation, and their
//! update in copy on write after an attach.
//!
//! References: `docs/phases/P1-stockage-et-recherche-exacte.md` (task
//! `CONSTRUIRE-BITMAPS`), `docs/03-recherche.md` ("Filtres → pool"), ADR-0018
//! (interval filters on value columns), ADR-0019 (fields stored as canonical
//! JSON), ADR-0030 (a field name designates one level), ADR-0035 (the semantics
//! of null values).
//!
//! # What each test pins
//!
//! | Test | Criterion of the task sheet |
//! |---|---|
//! | `bitmaps_after_l1_l2_l3_carry_the_state_of_sqlite` | "après L1, L2 et L3, les bitmaps sont égales à une reconstruction complète depuis SQLite" |
//! | `the_live_bitmap_holds_the_iids_sqlite_keeps_alive` | "la bitmap des vivants" — the set SQLite keeps alive |
//! | `the_memory_of_the_filters_is_reported_in_four_parts` | "la taille mémoire est rapportée dans `crible inspect`", measured here through `Snapshot::filter_memory` |
//! | `copy_on_write_after_each_generation_equals_a_full_rebuild` | "la mise à jour en copie sur écriture après une attache" |
//! | `reopening_an_index_rebuilds_the_same_structures` | "la construction depuis SQLite, intégrée au `Snapshot`" |
//! | `a_null_value_is_absent_from_the_present_bitmaps` | ADR-0035, the present bitmap |
//! | `a_passage_inherits_the_fields_of_its_document` | "l'héritage des champs de document par les passages" |
//! | `an_empty_index_has_empty_structures` | creation |
//!
//! # API under test
//!
//! `crible_store::filters_index::FilterIndex` holds one structure per declared
//! field: a bitmap per `keyword` value, two bitmaps per `bool`, a value column
//! per `integer`, `float` and `date`, a "present" bitmap per field, and the
//! bitmap of the live passages. `FilterIndex::from_records` builds them from
//! `records.sqlite`; `FilterIndex::after_commit` derives the next generation from
//! the previous one, in copy on write. `Snapshot::filters` exposes them and
//! `Snapshot::filter_memory` reports their size.
//!
//! The equality the criteria ask for is the equality of the whole `FilterIndex`:
//! the same bitmaps, the same values and the same shape, not a digest of a few
//! counts. `FilterIndex`, `KeywordField`, `BoolField` and `ValueColumn` derive
//! `PartialEq` for that.
//!
//! # Datasets
//!
//! The `synth-small` batches `l1`, `l2` and `l3` come from `tools/gen_synth.py`;
//! the CI generates them before `cargo test`. `l2` replaces 300 documents and
//! creates 100, `l3` rewrites the metadata of 50 documents only and deletes 150:
//! the three generations therefore exercise a creation, a replacement, a
//! metadata-only update and a deletion, which is the whole delta of an attach.
//! The schema of the dataset declares all five field types at both levels, so
//! every structure is exercised.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crible_core::config::{IndexConfig, Schema};
use crible_core::ids::{Generation, Iid};
use crible_core::model::ModelConfig;
use crible_store::attach::attach;
use crible_store::filters_index::FilterIndex;
use crible_store::generation::{GenerationChange, GenerationStore, Snapshot};
use crible_store::records::{DocumentRecord, PassageRecord};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};

/// Command that generates the reference dataset, quoted by the failure message
/// of a missing batch.
const DATASET_COMMAND: &str =
    "cd tools && uv sync && uv run python gen_synth.py --out ../data/synth-small";

// --- Paths of the reference dataset ---

fn dataset_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/synth-small")
}

/// Locates one batch of `synth-small`, or explains how to generate the dataset.
fn dataset_batch(root: &Path, name: &str) -> Result<PathBuf, String> {
    let dir = root.join(name);
    if dir.join("manifest.json").is_file() {
        Ok(dir)
    } else {
        Err(format!(
            "the synthetic batch '{name}' is missing at {}; generate the dataset before running \
             `cargo test`:\n  {DATASET_COMMAND}",
            dir.display()
        ))
    }
}

fn require_batch(name: &str) -> PathBuf {
    dataset_batch(&dataset_root(), name).unwrap_or_else(|message| panic!("{message}"))
}

fn dataset_index() -> IndexConfig {
    let path = dataset_root().join("index.json");
    let raw = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{} must be readable; generate the dataset before running `cargo test`:\n  \
             {DATASET_COMMAND}: {error}",
            path.display()
        )
    });
    IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("the dataset index.json must be valid: {error}"))
}

/// Resolved model configuration of `synth-small` (768 dimensions, external, so
/// `check_manifest` compares only `dims`, `normalize` and the vector type).
fn dataset_model() -> ModelConfig {
    ModelConfig::from_json_str(
        r#"{ "name": "synthetic-external", "kind": "external", "dims": 768, "normalize": true }"#,
    )
    .unwrap_or_else(|error| panic!("the dataset model configuration is valid: {error}"))
}

/// Attaches one batch of `synth-small` to `store`.
fn attach_synth(store: &mut GenerationStore, name: &str, index: &IndexConfig, model: &ModelConfig) {
    let batch = require_batch(name);
    attach(store, &batch, index, model).unwrap_or_else(|error| {
        panic!(
            "the attach of {} must succeed, got {} ({}): {}",
            batch.display(),
            error.code().as_str(),
            error.code().http_status(),
            error.message()
        )
    });
}

/// Opens `records.sqlite` of the index for reading, from outside the store.
///
/// The rebuild the criteria compare against must not go through the store: it
/// reads the committed state of the reference database, exactly as the task
/// `EVALUER-FILTRES` will read it with `json_extract` (INV-07).
fn open_records(index: &Path) -> Connection {
    let path = common::records_path(index);
    let connection = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
    connection
        .busy_timeout(Duration::from_millis(1_000))
        .unwrap_or_else(|error| panic!("the busy timeout must be settable: {error}"));
    connection
}

/// Full rebuild of the filter structures from `records.sqlite`.
fn rebuild(index: &Path, schema: &Schema) -> FilterIndex {
    let connection = open_records(index);
    FilterIndex::from_records(&connection, schema)
        .unwrap_or_else(|error| panic!("the structures must rebuild from SQLite: {error}"))
}

/// Live identifiers of `records.sqlite`, in ascending order.
fn live_iids_of_sqlite(index: &Path) -> Vec<u32> {
    let connection = open_records(index);
    let mut statement = connection
        .prepare("SELECT iid FROM passages WHERE dead_since_gen IS NULL ORDER BY iid")
        .unwrap_or_else(|error| panic!("the live passages query must prepare: {error}"));
    let iids = statement
        .query_map([], |row| row.get::<_, i64>(0))
        .unwrap_or_else(|error| panic!("the live passages query must run: {error}"))
        .collect::<Result<Vec<i64>, _>>()
        .unwrap_or_else(|error| panic!("a live iid must be readable: {error}"));
    iids.into_iter()
        .map(|iid| {
            u32::try_from(iid)
                .unwrap_or_else(|error| panic!("a live iid must fit in u32, got {iid}: {error}"))
        })
        .collect()
}

// --- Test 1: the structures of the three generations ---

/// After `L1`, `L2` and `L3` the published structures are exactly the ones a
/// full rebuild from `records.sqlite` produces.
///
/// The check runs after **each** generation, not only at the end: the delta of a
/// creation, of a replacement, of a metadata-only update and of a deletion each
/// has to agree with the rebuild, and an error in one of them would be hidden by
/// the following ones.
#[test]
fn bitmaps_after_l1_l2_l3_carry_the_state_of_sqlite() {
    let _serial = common::SerialGuard::acquire();
    let index = dataset_index();
    let model = dataset_model();
    let (_temp, index_dir) = common::temp_index();
    let mut store = common::create_store_with(&index_dir, &index.schema);

    for (name, generation) in [("l1", 1), ("l2", 2), ("l3", 3)] {
        attach_synth(&mut store, name, &index, &model);
        let snapshot = store.snapshot();
        assert_eq!(
            snapshot.generation(),
            Generation::new(generation),
            "{name} must publish generation {generation}"
        );
        assert_eq!(
            snapshot.filters(),
            &rebuild(&index_dir, &index.schema),
            "the structures published after {name} must equal a full rebuild from SQLite"
        );
    }
}

/// The bitmap of the live passages, which the commit path derives, holds exactly
/// the live iids SQLite names.
#[test]
fn the_live_bitmap_holds_the_iids_sqlite_keeps_alive() {
    let _serial = common::SerialGuard::acquire();
    let index = dataset_index();
    let model = dataset_model();
    let (_temp, index_dir) = common::temp_index();
    let mut store = common::create_store_with(&index_dir, &index.schema);

    for name in ["l1", "l2", "l3"] {
        attach_synth(&mut store, name, &index, &model);
        let snapshot = store.snapshot();
        // The snapshot reads its live iids from that bitmap, so the comparison
        // below is the whole check: the same set, read from SQLite by another
        // connection, and compared identifier by identifier. A count would let a
        // wrong set of the right size through.
        let on_the_bitmap: Vec<u32> = snapshot.live_iids().map(|iid| iid.get()).collect();
        let stored = live_iids_of_sqlite(&index_dir);
        assert_eq!(
            on_the_bitmap, stored,
            "after {name} the live bitmap must hold exactly the live passages of SQLite"
        );
    }
}

// --- Test 2: the reported memory ---

/// `Snapshot::filter_memory` reports the size of the structures, in four parts
/// whose sum is the total, and the same size a full rebuild reports.
///
/// `crible inspect` will print that measure (task `CLI-PREMIERES-COMMANDES`);
/// this task exposes it and tests it, and states no performance figure (MES-13
/// is measured in P3).
#[test]
fn the_memory_of_the_filters_is_reported_in_four_parts() {
    let _serial = common::SerialGuard::acquire();
    let index = dataset_index();
    let model = dataset_model();
    let (_temp, index_dir) = common::temp_index();
    let mut store = common::create_store_with(&index_dir, &index.schema);
    attach_synth(&mut store, "l1", &index, &model);

    let memory = store.snapshot().filter_memory();
    assert_eq!(
        memory.total(),
        memory.live + memory.keywords + memory.booleans + memory.columns,
        "the total must be the sum of its parts"
    );
    assert!(memory.live > 0, "L1 leaves 10 000 live passages");
    assert!(
        memory.keywords > 0 && memory.booleans > 0 && memory.columns > 0,
        "the schema of synth-small declares a field of every type, so every part is non-empty: \
         {memory:?}"
    );
    assert_eq!(
        memory,
        rebuild(&index_dir, &index.schema).memory(),
        "the same structures must report the same size, however they were built"
    );
}

// --- Test 3: the update in copy on write ---

/// Schema of the generations this binary commits itself: one field of every
/// type at each level, so every structure takes part in the delta.
fn rich_schema() -> Schema {
    Schema::from_json_str(
        r#"{
            "document_fields": {
                "categorie": { "type": "keyword" },
                "annee": { "type": "integer" },
                "publie": { "type": "bool" },
                "date_publication": { "type": "date" },
                "score": { "type": "float" }
            },
            "passage_fields": {
                "partie": { "type": "keyword" },
                "rang": { "type": "integer" }
            }
        }"#,
    )
    .unwrap_or_else(|error| panic!("the schema of this test must be valid: {error}"))
}

/// One document row carrying every document field.
fn document(doc_id: &str, categorie: &str, generation: u64) -> DocumentRecord {
    DocumentRecord {
        doc_id: doc_id.to_owned(),
        doc_hash: format!("blake3:doc-{doc_id}"),
        fields: json!({
            "annee": 2021,
            "categorie": categorie,
            "date_publication": "2020-02-29",
            "publie": true,
            "score": 1.5,
        }),
        updated_generation: Generation::new(generation),
    }
}

/// One passage row carrying every passage field, or nulls when `null` is set.
fn passage(iid: u32, doc_id: &str, partie: &str, generation: u64, null: bool) -> PassageRecord {
    PassageRecord {
        iid: Iid::new(iid),
        doc_id: doc_id.to_owned(),
        ord: iid % 2,
        text: format!("{doc_id}#{iid}"),
        passage_hash: format!("blake3:passage-{iid}"),
        fields: if null {
            json!({ "partie": Value::Null, "rang": Value::Null })
        } else {
            json!({ "partie": partie, "rang": i64::from(iid) })
        },
        segment_id: "seg-fixture".to_owned(),
        created_generation: Generation::new(generation),
        dead_since_gen: None,
    }
}

/// Every generation this binary commits equals a full rebuild: creation, then a
/// metadata-only update with a tombstone and a new passage, then a second
/// metadata-only update.
///
/// This is the deliverable "mise à jour en copie sur écriture après une attache",
/// checked generation by generation. The update of a document is the hardest
/// case of the delta: the values its existing passages inherit change, and the
/// passage fields must not move.
///
/// Each snapshot is kept, with the structures a rebuild of its generation gives,
/// and asserted **again after the following commits**: a reader that took a
/// snapshot keeps reading its generation whole, bitmaps included, while the
/// store publishes new ones (INV-02). Sharing a bitmap between two generations
/// is what makes that possible, and an update that wrote in place would break
/// it.
#[test]
fn copy_on_write_after_each_generation_equals_a_full_rebuild() {
    let (_temp, index_dir) = common::temp_index();
    let schema = rich_schema();
    let mut store = common::create_store_with(&index_dir, &schema);

    let mut held: Vec<(&str, FilterIndex, Arc<Snapshot>)> = Vec::new();

    let first = GenerationChange {
        segments: Vec::new(),
        documents: vec![document("doc-a", "alpha", 1), document("doc-b", "alpha", 1)],
        passages: vec![
            passage(0, "doc-a", "corps", 1, false),
            passage(1, "doc-a", "annexe", 1, false),
            passage(2, "doc-b", "corps", 1, false),
        ],
        deletions: Vec::new(),
    };
    common::commit(&mut store, first);
    held.push((
        "the creation",
        rebuild(&index_dir, &schema),
        store.snapshot(),
    ));

    let second = GenerationChange {
        segments: Vec::new(),
        documents: vec![document("doc-a", "beta", 2)],
        passages: vec![passage(3, "doc-b", "annexe", 2, false)],
        deletions: vec![Iid::new(1)],
    };
    common::commit(&mut store, second);
    held.push((
        "a metadata-only update, a tombstone and a new passage",
        rebuild(&index_dir, &schema),
        store.snapshot(),
    ));

    let third = GenerationChange {
        segments: Vec::new(),
        documents: vec![document("doc-b", "gamma", 3)],
        passages: Vec::new(),
        deletions: Vec::new(),
    };
    common::commit(&mut store, third);
    held.push((
        "a second metadata-only update, on a document whose live and dead passages both change",
        rebuild(&index_dir, &schema),
        store.snapshot(),
    ));

    for (name, expected, snapshot) in &held {
        assert_eq!(
            snapshot.filters(),
            expected,
            "the structures published after {name} must still equal a rebuild of that generation, \
             once the following generations are committed"
        );
    }
}

// --- Test 4: startup ---

/// Reopening an index rebuilds the same structures from `records.sqlite`.
#[test]
fn reopening_an_index_rebuilds_the_same_structures() {
    let _serial = common::SerialGuard::acquire();
    let index = dataset_index();
    let model = dataset_model();
    let (_temp, index_dir) = common::temp_index();
    let mut store = common::create_store_with(&index_dir, &index.schema);
    attach_synth(&mut store, "l1", &index, &model);
    let before = store.snapshot();
    let published = before.filters().clone();
    let memory = before.filter_memory();
    drop(before);
    drop(store);

    let reopened = common::open_store_with(&index_dir, &index.schema);
    assert_eq!(
        reopened.snapshot().filters(),
        &published,
        "a restart must rebuild the structures the commit had published"
    );
    assert_eq!(
        reopened.snapshot().filter_memory(),
        memory,
        "a restart must report the same size"
    );
}

// --- Test 5: nulls ---

/// A null value is in no value bitmap and in no present bitmap (ADR-0035).
#[test]
fn a_null_value_is_absent_from_the_present_bitmaps() {
    let (_temp, index_dir) = common::temp_index();
    let schema = rich_schema();
    let mut store = common::create_store_with(&index_dir, &schema);

    let change = GenerationChange {
        segments: Vec::new(),
        documents: vec![document("doc-a", "alpha", 1)],
        passages: vec![
            passage(0, "doc-a", "corps", 1, false),
            passage(1, "doc-a", "corps", 1, true),
        ],
        deletions: Vec::new(),
    };
    common::commit(&mut store, change);
    let snapshot = store.snapshot();
    let filters = snapshot.filters();
    let missing = Iid::new(1);

    let partie = filters
        .keyword("partie")
        .unwrap_or_else(|| panic!("partie is a keyword field of the schema"));
    assert!(
        !partie.present().contains(missing.get()),
        "a passage whose keyword is null must be absent from the present bitmap"
    );
    assert!(
        !partie
            .values()
            .any(|(_, bitmap)| bitmap.contains(missing.get())),
        "a passage whose keyword is null must be in no value bitmap"
    );

    let rang = filters
        .column("rang")
        .unwrap_or_else(|| panic!("rang is an integer field of the schema"));
    assert_eq!(rang.field_type().as_str(), "integer");
    assert_eq!(
        rang.integer(missing),
        None,
        "a null integer reads as no value at all"
    );
    assert!(
        !rang.present().contains(missing.get()),
        "a passage whose integer is null must be absent from the present bitmap"
    );

    // The document-level field is inherited, so it is present on both passages.
    let categorie = filters
        .keyword("categorie")
        .unwrap_or_else(|| panic!("categorie is a keyword field of the schema"));
    assert!(
        categorie.present().contains(missing.get()),
        "a document field that is not null is present on every passage of its document"
    );
}

// --- Test 6: inheritance ---

/// A passage carries the values of the fields of its document, at the level the
/// schema declares them.
#[test]
fn a_passage_inherits_the_fields_of_its_document() {
    let (_temp, index_dir) = common::temp_index();
    let schema = rich_schema();
    let mut store = common::create_store_with(&index_dir, &schema);

    let change = GenerationChange {
        segments: Vec::new(),
        documents: vec![document("doc-a", "alpha", 1), document("doc-b", "beta", 1)],
        passages: vec![
            passage(0, "doc-a", "corps", 1, false),
            passage(1, "doc-a", "annexe", 1, false),
            passage(2, "doc-b", "corps", 1, false),
        ],
        deletions: Vec::new(),
    };
    common::commit(&mut store, change);
    let snapshot = store.snapshot();
    let filters = snapshot.filters();

    let categorie = filters
        .keyword("categorie")
        .unwrap_or_else(|| panic!("categorie is a keyword field of the schema"));
    let alpha = categorie
        .bitmap("alpha")
        .unwrap_or_else(|| panic!("doc-a carries categorie alpha"));
    assert!(
        alpha.contains(0) && alpha.contains(1),
        "both passages of doc-a inherit categorie alpha"
    );
    assert!(
        !alpha.contains(2),
        "the passage of doc-b must not inherit the value of doc-a"
    );

    let partie = filters
        .keyword("partie")
        .unwrap_or_else(|| panic!("partie is a keyword field of the schema"));
    let corps = partie
        .bitmap("corps")
        .unwrap_or_else(|| panic!("two passages carry partie corps"));
    assert!(
        corps.contains(0) && corps.contains(2) && !corps.contains(1),
        "a passage-level field belongs to the passage, not to the document"
    );

    let annee = filters
        .column("annee")
        .unwrap_or_else(|| panic!("annee is an integer field of the schema"));
    assert_eq!(annee.integer(Iid::new(2)), Some(2021));
    let date = filters
        .column("date_publication")
        .unwrap_or_else(|| panic!("date_publication is a date field of the schema"));
    assert_eq!(
        date.date_days(Iid::new(0)),
        Some(18_321),
        "2020-02-29 is 18 321 days after 1970-01-01"
    );
    let score = filters
        .column("score")
        .unwrap_or_else(|| panic!("score is a float field of the schema"));
    assert_eq!(score.float(Iid::new(0)), Some(1.5));
    let publie = filters
        .boolean("publie")
        .unwrap_or_else(|| panic!("publie is a bool field of the schema"));
    assert!(publie.bitmap(true).contains(1));
    assert!(!publie.bitmap(false).contains(1));
}

// --- Test 7: creation ---

/// A fresh index has one empty structure per declared field, no live passage
/// and a null size.
#[test]
fn an_empty_index_has_empty_structures() {
    let (_temp, index_dir) = common::temp_index();
    let schema = rich_schema();
    let store = common::create_store_with(&index_dir, &schema);
    let snapshot = store.snapshot();
    let filters = snapshot.filters();

    assert_eq!(snapshot.generation(), Generation::new(0));
    assert!(
        filters.live().is_empty(),
        "a fresh index has no live passage"
    );
    assert!(filters.live_iids().is_empty());
    assert_eq!(filters.keyword("partie").map(|field| field.len()), Some(0));
    assert_eq!(filters.memory().total(), 0);

    // Every field of the schema has its structure, empty, even before any row.
    for (name, _, spec) in schema.fields() {
        let found = match spec.field_type {
            crible_core::config::FieldType::Keyword => filters.keyword(name).is_some(),
            crible_core::config::FieldType::Bool => filters.boolean(name).is_some(),
            _ => filters.column(name).is_some_and(|column| column.is_empty()),
        };
        assert!(found, "field '{name}' must have its empty structure");
    }
}
