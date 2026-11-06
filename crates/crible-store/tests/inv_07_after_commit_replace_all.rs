// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `INV-07` and `INV-04` on the commit path a large `replace_all` takes.
//!
//! References: `docs/phases/P10-validation-a-l-echelle-reelle.md` (task
//! `CORRIGER-BLOCAGE-APRES-COMMIT`), `docs/08-invariants-et-tests.md` (INV-07,
//! INV-04), `docs/03-recherche.md` ("Filtres → pool"), `docs/04-donnees-et-
//! ingestion.md` ("Sémantique d'attache", "Document retiré"), ADR-0018,
//! ADR-0030, ADR-0035, ADR-0041, ADR-0066.
//!
//! # What the task changes
//!
//! `crible attach` of a large `replace_all` batch into an empty index reaches
//! its commit point and then spends 96 to 98 % of its own time in
//! `FieldData::clear`, under `FilterIndex::after_commit`. The structures the
//! correction derives must keep the same **result**: the whole `FilterIndex`
//! after the commit must still equal `FilterIndex::from_records` on the same
//! reference state, and a restart must still publish the last committed
//! generation with those same filters.
//!
//! # What each test pins
//!
//! | Test | Criterion or invariant |
//! |---|---|
//! | `inv_07_after_commit_of_a_large_replace_all_equals_a_full_rebuild` | INV-07: after a generation 1 `replace_all` of many documents, then a generation 2 that keeps, replaces, updates the metadata of, removes and adds documents, the structures `after_commit` derived equal `FilterIndex::from_records` on the same state |
//! | `inv_04_reopening_after_a_large_replace_all_keeps_the_committed_generation_and_its_filters` | INV-04: a restart publishes the last committed generation, with filters identical to the ones the commit published |
//!
//! # Why this dataset
//!
//! The cost the task pursues grows with the number of passages **and** with the
//! number of distinct values of the document-level fields: `clear` walks the
//! value bitmaps of a keyword field. The fixture therefore writes 480 documents
//! with 60 distinct `rubrique` values, 35 distinct `annee` values and a
//! passage-level field of every type, split over two passages per document.
//! Generation 2 is a `replace_all` that exercises the four steps of
//! `after_commit`: creations, tombstones, rewritten documents (a replacement
//! and a metadata-only update) and removed documents.
//!
//! The sizes stay small on purpose: each test runs in a few seconds in a debug
//! build, and neither test measures time. They must pass on the code of before
//! the correction, which is correct but slow; a failure here is a defect of the
//! derivation, not of its speed.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow::array::{
    ArrayRef, BooleanArray, Date32Array, FixedSizeBinaryArray, Float64Array, Int64Array,
    StringArray, UInt32Array,
};
use arrow::datatypes::{DataType, Field as ArrowField, Schema as ArrowSchema, SchemaRef};
use crible_core::config::{IndexConfig, Schema};
use crible_core::ids::Generation;
use crible_core::model::ModelConfig;
use crible_core::time::days_from_civil;
use crible_store::attach::{AttachReport, attach};
use crible_store::filters_index::FilterIndex;
use crible_store::generation::GenerationStore;
use half::f16;
use rusqlite::{Connection, OpenFlags};
use serde_json::{Map, Value, json};

/// Documents of generation 2: 250 kept, 100 replaced, 50 metadata-only and 80
/// added. The 80 documents of generation 1 absent from its batch are removed.
const GENERATION_TWO_DOCUMENTS: usize = 250 + 100 + 50 + 80;

/// Documents of generation 1.
const DOCUMENTS: usize = 480;
/// Passages per document.
const PASSAGES_PER_DOCUMENT: usize = 2;
/// Dimensionality of the fixture vectors, as `docs/07` fixes for the fixtures.
const DIMS: u32 = 4;

/// Distinct values of the document-level keyword `rubrique`.
const RUBRIQUES: usize = 60;
/// Distinct values of the document-level integer `annee`.
const ANNEES: usize = 35;

/// Schema of the fixture index: the five field types at both levels.
///
/// The document level carries the fields whose clearing the task pursues; the
/// passage level checks that `clear` leaves the fields a passage owns alone.
const SCHEMA_JSON: &str = r#"{
    "document_fields": {
        "rubrique": { "type": "keyword" },
        "annee":    { "type": "integer" },
        "score":    { "type": "float" },
        "vigueur":  { "type": "date" },
        "archive":  { "type": "bool" }
    },
    "passage_fields": {
        "partie":    { "type": "keyword" },
        "rang":      { "type": "integer" },
        "poids":     { "type": "float" },
        "jour":      { "type": "date" },
        "principal": { "type": "bool" }
    }
}"#;

fn fixture_schema() -> Schema {
    Schema::from_json_str(SCHEMA_JSON)
        .unwrap_or_else(|error| panic!("the fixture schema must be valid: {error}"))
}

/// Model configuration of the fixture: an `external` model, so the attach
/// runs no encoder and the batch declares fp16 vectors.
fn fixture_model() -> ModelConfig {
    ModelConfig::from_json_str(
        r#"{ "name": "fixture-external", "kind": "external", "dims": 4, "normalize": true }"#,
    )
    .unwrap_or_else(|error| panic!("the fixture model configuration must be valid: {error}"))
}

// --- The rows of one batch ---

/// One document of a batch, with the passages it declares.
struct DocSpec {
    doc_id: String,
    doc_hash: String,
    fields: Map<String, Value>,
    passages: Vec<(u32, Map<String, Value>)>,
    /// Salt of the passage texts: a replacement changes the content, a
    /// metadata-only update keeps it.
    text_salt: usize,
}

/// Document-level fields of document `number`.
///
/// `salt` shifts every value, so a second generation can rewrite a document
/// (another salt) or keep it (the same salt). A fifth of the documents carry no
/// `rubrique`, so the absent state of ADR-0035 is part of the reference state.
fn document_fields(number: usize, salt: usize) -> Map<String, Value> {
    let mut fields = Map::new();
    if !number.is_multiple_of(5) {
        fields.insert(
            "rubrique".to_owned(),
            json!(format!("rubrique-{:03}", (number * 7 + salt) % RUBRIQUES)),
        );
    }
    fields.insert(
        "annee".to_owned(),
        json!(1990 + ((number + salt) % ANNEES) as i64),
    );
    fields.insert(
        "score".to_owned(),
        json!(((number + salt) % 23) as f64 / 3.0),
    );
    fields.insert(
        "vigueur".to_owned(),
        json!(format!(
            "{:04}-{:02}-15",
            2000 + (number + salt) % 25,
            1 + (number + salt) % 12
        )),
    );
    fields.insert(
        "archive".to_owned(),
        json!((number + salt).is_multiple_of(2)),
    );
    fields
}

/// Passage-level fields of passage `ord` of document `number`.
///
/// They never depend on the salt: a metadata-only update rewrites document
/// fields only, and a passage keeps the values it owns (`docs/04`).
fn passage_fields(number: usize, ord: u32) -> Map<String, Value> {
    let slot = number + ord as usize;
    let mut fields = Map::new();
    if !slot.is_multiple_of(6) {
        fields.insert(
            "partie".to_owned(),
            json!(["corps", "moyens", "faits", "annexe"][slot % 4]),
        );
    }
    fields.insert("rang".to_owned(), json!((slot % 7) as i64));
    fields.insert("poids".to_owned(), json!((slot % 9) as f64 / 3.0));
    fields.insert(
        "jour".to_owned(),
        json!(format!("{:04}-{:02}-01", 2001 + slot % 20, 1 + slot % 12)),
    );
    fields.insert("principal".to_owned(), json!(slot.is_multiple_of(2)));
    fields
}

/// One document of a batch with its text and hash salts.
fn doc_spec(number: usize, field_salt: usize, hash_salt: usize, text_salt: usize) -> DocSpec {
    let passages = (0..PASSAGES_PER_DOCUMENT)
        .map(|ord| (ord as u32, passage_fields(number, ord as u32)))
        .collect();
    DocSpec {
        doc_id: format!("doc-{number:04}"),
        doc_hash: format!("blake3:doc-{number}-{hash_salt}"),
        fields: document_fields(number, field_salt),
        passages,
        text_salt,
    }
}

// --- Writing a batch ---

fn document_arrow_schema() -> SchemaRef {
    Arc::new(ArrowSchema::new(vec![
        ArrowField::new("doc_id", DataType::Utf8, false),
        ArrowField::new("doc_hash", DataType::Utf8, false),
        ArrowField::new("rubrique", DataType::Utf8, true),
        ArrowField::new("annee", DataType::Int64, true),
        ArrowField::new("score", DataType::Float64, true),
        ArrowField::new("vigueur", DataType::Date32, true),
        ArrowField::new("archive", DataType::Boolean, true),
    ]))
}

fn passage_arrow_schema() -> SchemaRef {
    Arc::new(ArrowSchema::new(vec![
        ArrowField::new("doc_id", DataType::Utf8, false),
        ArrowField::new("ord", DataType::UInt32, false),
        ArrowField::new("text", DataType::Utf8, false),
        ArrowField::new("partie", DataType::Utf8, true),
        ArrowField::new("rang", DataType::Int64, true),
        ArrowField::new("poids", DataType::Float64, true),
        ArrowField::new("jour", DataType::Date32, true),
        ArrowField::new("principal", DataType::Boolean, true),
        ArrowField::new("vector", DataType::FixedSizeBinary(DIMS as i32 * 2), false),
    ]))
}

fn keyword_column<'a>(fields: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    fields.get(name).and_then(Value::as_str)
}

fn integer_column(fields: &Map<String, Value>, name: &str) -> Option<i64> {
    fields.get(name).and_then(Value::as_i64)
}

fn float_column(fields: &Map<String, Value>, name: &str) -> Option<f64> {
    fields.get(name).and_then(Value::as_f64)
}

fn bool_column(fields: &Map<String, Value>, name: &str) -> Option<bool> {
    fields.get(name).and_then(Value::as_bool)
}

fn date_column(fields: &Map<String, Value>, name: &str) -> Option<i32> {
    fields.get(name).and_then(Value::as_str).map(|text| {
        let (year, month, day) = parse_date(text);
        i32::try_from(days_from_civil(year, month, day))
            .unwrap_or_else(|error| panic!("{text} must fit in an i32 count of days: {error}"))
    })
}

/// Parses the `YYYY-MM-DD` form the fixture writes.
fn parse_date(text: &str) -> (i64, u32, u32) {
    let mut parts = text.split('-');
    let year = parts
        .next()
        .and_then(|part| part.parse().ok())
        .unwrap_or_else(|| panic!("{text} must start with a year"));
    let month = parts
        .next()
        .and_then(|part| part.parse().ok())
        .unwrap_or_else(|| panic!("{text} must carry a month"));
    let day = parts
        .next()
        .and_then(|part| part.parse().ok())
        .unwrap_or_else(|| panic!("{text} must end with a day"));
    (year, month, day)
}

/// Writes `documents.parquet`, `passages.parquet` and `manifest.json` of one
/// `replace_all` batch under `<root>/<batch_id>/`, which it returns.
fn write_replace_all_batch(root: &Path, batch_id: &str, docs: &[DocSpec]) -> PathBuf {
    let dir = root.join(batch_id);
    fs::create_dir_all(&dir)
        .unwrap_or_else(|error| panic!("the batch directory must be creatable: {error}"));

    let ids: StringArray = docs.iter().map(|doc| Some(doc.doc_id.as_str())).collect();
    let hashes: StringArray = docs.iter().map(|doc| Some(doc.doc_hash.as_str())).collect();
    let rubriques: StringArray = docs
        .iter()
        .map(|doc| keyword_column(&doc.fields, "rubrique"))
        .collect();
    let annees = Int64Array::from(
        docs.iter()
            .map(|doc| integer_column(&doc.fields, "annee"))
            .collect::<Vec<_>>(),
    );
    let scores = Float64Array::from(
        docs.iter()
            .map(|doc| float_column(&doc.fields, "score"))
            .collect::<Vec<_>>(),
    );
    let vigueurs = Date32Array::from(
        docs.iter()
            .map(|doc| date_column(&doc.fields, "vigueur"))
            .collect::<Vec<_>>(),
    );
    let archives = BooleanArray::from(
        docs.iter()
            .map(|doc| bool_column(&doc.fields, "archive"))
            .collect::<Vec<_>>(),
    );
    common::write_parquet(
        &dir.join("documents.parquet"),
        &document_arrow_schema(),
        vec![
            Arc::new(ids) as ArrayRef,
            Arc::new(hashes),
            Arc::new(rubriques),
            Arc::new(annees),
            Arc::new(scores),
            Arc::new(vigueurs),
            Arc::new(archives),
        ],
    );

    let mut passage_ids: Vec<String> = Vec::new();
    let mut ords: Vec<u32> = Vec::new();
    let mut texts: Vec<String> = Vec::new();
    let mut parties: Vec<Option<&str>> = Vec::new();
    let mut rangs: Vec<Option<i64>> = Vec::new();
    let mut poids: Vec<Option<f64>> = Vec::new();
    let mut jours: Vec<Option<i32>> = Vec::new();
    let mut principals: Vec<Option<bool>> = Vec::new();
    for doc in docs {
        for (ord, fields) in &doc.passages {
            passage_ids.push(doc.doc_id.clone());
            ords.push(*ord);
            texts.push(format!("{}#{ord}#{}", doc.doc_id, doc.text_salt));
            parties.push(keyword_column(fields, "partie"));
            rangs.push(integer_column(fields, "rang"));
            poids.push(float_column(fields, "poids"));
            jours.push(date_column(fields, "jour"));
            principals.push(bool_column(fields, "principal"));
        }
    }

    let vector: Vec<u8> = [1.0_f32, 0.0, 0.0, 0.0]
        .iter()
        .flat_map(|component| f16::from_f32(*component).to_bits().to_le_bytes())
        .collect();
    let vectors =
        FixedSizeBinaryArray::try_from_iter((0..passage_ids.len()).map(|_| vector.clone()))
            .unwrap_or_else(|error| panic!("the fixture vectors must share one width: {error}"));

    let passage_ids: StringArray = passage_ids.iter().map(|id| Some(id.as_str())).collect();
    let texts: StringArray = texts.iter().map(|text| Some(text.as_str())).collect();
    let parties: StringArray = parties.into_iter().collect();
    let rangs = Int64Array::from(rangs);
    let poids = Float64Array::from(poids);
    let jours = Date32Array::from(jours);
    let principals = BooleanArray::from(principals);
    common::write_parquet(
        &dir.join("passages.parquet"),
        &passage_arrow_schema(),
        vec![
            Arc::new(passage_ids) as ArrayRef,
            Arc::new(UInt32Array::from(ords)),
            Arc::new(texts),
            Arc::new(parties),
            Arc::new(rangs),
            Arc::new(poids),
            Arc::new(jours),
            Arc::new(principals),
            Arc::new(vectors),
        ],
    );

    let passages = docs.iter().map(|doc| doc.passages.len()).sum::<usize>();
    write_manifest(&dir, batch_id, docs.len(), passages);
    dir
}

/// Writes `manifest.json` of one `replace_all` batch, with the checksum of
/// every Parquet file it holds.
fn write_manifest(dir: &Path, batch_id: &str, documents: usize, passages: usize) {
    let mut files = std::collections::BTreeMap::new();
    for name in ["documents.parquet", "passages.parquet"] {
        let bytes = fs::read(dir.join(name))
            .unwrap_or_else(|error| panic!("{name} must be readable: {error}"));
        files.insert(name, format!("blake3:{}", blake3::hash(&bytes).to_hex()));
    }
    let manifest = json!({
        "format": "crible-batch",
        "format_version": 1,
        "batch_id": batch_id,
        "index": "test",
        "mode": "replace_all",
        "created_at": "2026-01-01T00:00:00Z",
        "producer": { "name": "test", "version": "1", "host": "test" },
        "model": { "dims": DIMS, "normalize": true, "vector_dtype": "fp16" },
        "chunker": { "name": "test", "version": "1", "params": {} },
        "template": { "name": "test", "version": "1" },
        "counts": { "documents": documents, "passages": passages, "deletions": 0 },
        "files": files,
    });
    fs::write(
        dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)
            .unwrap_or_else(|error| panic!("the manifest must be serializable: {error}")),
    )
    .unwrap_or_else(|error| panic!("manifest.json must be written: {error}"));
}

// --- Attaching the two generations ---

fn attach_batch(store: &mut GenerationStore, batch: &Path, index: &IndexConfig) -> AttachReport {
    let model = fixture_model();
    attach(store, batch, index, &model, None, None).unwrap_or_else(|error| {
        panic!(
            "the attach of {} must succeed, got {}: {}",
            batch.display(),
            error.code().as_str(),
            error.message()
        )
    })
}

/// The index configuration `common::create_store_with` writes.
fn index_config(index_dir: &Path) -> IndexConfig {
    let raw = fs::read_to_string(index_dir.join("index.json"))
        .unwrap_or_else(|error| panic!("index.json must be readable: {error}"));
    IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("index.json must be valid: {error}"))
}

/// Builds generation 1 with a `replace_all` of [`DOCUMENTS`] documents, then
/// generation 2 with a `replace_all` that keeps, replaces, updates the metadata
/// of, removes and adds documents.
///
/// Returns the store, open on generation 2.
fn two_generations(index_dir: &Path, batches: &Path) -> GenerationStore {
    let schema = fixture_schema();

    let generation_one: Vec<DocSpec> = (0..DOCUMENTS)
        .map(|number| doc_spec(number, 1, 1, 1))
        .collect();
    let first = write_replace_all_batch(batches, "replace-all-1", &generation_one);
    let second_specs = generation_two_specs();
    let second = write_replace_all_batch(batches, "replace-all-2", &second_specs);

    let mut store = common::create_store_with(index_dir, &schema);
    let config = index_config(index_dir);
    let first_report = attach_batch(&mut store, &first, &config);
    assert_eq!(
        first_report.generation,
        Generation::new(1),
        "the first attach into an empty index commits generation 1"
    );
    assert_eq!(
        first_report.added as usize, DOCUMENTS,
        "the first `replace_all` adds every document of the batch"
    );

    let second_report = attach_batch(&mut store, &second, &config);
    assert_eq!(
        (second_report.added, second_report.replaced),
        (80, 100),
        "generation 2 adds its 80 new documents and replaces the 100 whose hash changed"
    );
    assert_eq!(
        second_report.metadata_updated, 50,
        "generation 2 rewrites the fields of 50 documents whose hash did not change"
    );
    assert_eq!(
        second_report.deleted, 80,
        "generation 2 removes the 80 documents absent from the `replace_all` batch"
    );
    assert_eq!(
        second_report.unchanged, 250,
        "generation 2 leaves 250 documents untouched"
    );
    store
}

/// Documents of generation 2: 250 kept, 100 replaced, 50 metadata-only, 80
/// added; the 80 documents of generation 1 absent from the batch are removed.
fn generation_two_specs() -> Vec<DocSpec> {
    let mut specs = Vec::new();
    // Kept: same hash, same fields, same text.
    for number in 0..250 {
        specs.push(doc_spec(number, 1, 1, 1));
    }
    // Replaced: another hash, another content, other document fields.
    for number in 250..350 {
        specs.push(doc_spec(number, 2, 2, 2));
    }
    // Metadata-only: same hash and same text, other document fields.
    for number in 350..400 {
        specs.push(doc_spec(number, 3, 1, 1));
    }
    // Added: documents generation 1 never held.
    for number in 400..480 {
        specs.push(doc_spec(number + 80, 1, 1, 3));
    }
    specs
}

// --- The reference state ---

fn open_records(index_dir: &Path) -> Connection {
    let path = common::records_path(index_dir);
    Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()))
}

/// Rebuilds the structures from `records.sqlite`, outside the store.
fn rebuild(index_dir: &Path, schema: &Schema) -> FilterIndex {
    FilterIndex::from_records(&open_records(index_dir), schema)
        .unwrap_or_else(|error| panic!("the structures must rebuild from SQLite: {error}"))
}

/// Live passages of the reference state.
fn live_count(index_dir: &Path) -> i64 {
    open_records(index_dir)
        .query_row(
            "SELECT COUNT(*) FROM passages WHERE dead_since_gen IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|error| panic!("the live passages query must run: {error}"))
}

/// The structures `after_commit` derived are the structures a full rebuild
/// gives on the same reference state, after a generation 1 `replace_all` of
/// many documents and after a generation 2 that keeps, replaces, updates the
/// metadata of, removes and adds documents.
///
/// This is the result guard of `CORRIGER-BLOCAGE-APRES-COMMIT`: the correction
/// may change how the clearing is done, never what it leaves behind. The
/// equality is the one of the whole `FilterIndex` — live bitmap, keyword
/// bitmaps, present bitmaps, boolean bitmaps and value columns — because the
/// task touches `FieldData::clear`, which every one of those structures goes
/// through.
///
/// `inv_07_after_commit_of_a_large_replace_all_equals_a_full_rebuild`
#[test]
fn inv_07_after_commit_of_a_large_replace_all_equals_a_full_rebuild() {
    let _serial = common::SerialGuard::acquire();
    let (_index_temp, index_dir) = common::temp_index();
    let batches = batches_root();
    let schema = fixture_schema();

    let store = two_generations(&index_dir, batches.path());
    let snapshot = store.snapshot();
    let expected_live = GENERATION_TWO_DOCUMENTS * PASSAGES_PER_DOCUMENT;
    assert_eq!(
        live_count(&index_dir),
        expected_live as i64,
        "generation 2 must keep two passages per one of its 480 documents"
    );
    assert_eq!(
        snapshot.live_iids().count(),
        expected_live,
        "the published live bitmap must hold the live passages of the reference state"
    );
    assert_eq!(
        snapshot.filters(),
        &rebuild(&index_dir, &schema),
        "the structures derived after the two `replace_all` attaches must equal a full rebuild: \
         generation 2 exercises creations, tombstones, rewritten documents and removed documents, \
         which are the four steps of `after_commit` and the paths of `FieldData::clear`"
    );
}

/// `INV-04`: a restart publishes the last committed generation, with the
/// filters the commit published.
///
/// The blocking incident happens **after** the commit point: the correction
/// must not move that point nor change the order of the protocol. A restart
/// after the two `replace_all` attaches therefore has to read generation 2 from
/// SQLite, rebuild its structures, and give the same filters as the ones the
/// commit had published.
#[test]
fn inv_04_reopening_after_a_large_replace_all_keeps_the_committed_generation_and_its_filters() {
    let _serial = common::SerialGuard::acquire();
    let (_index_temp, index_dir) = common::temp_index();
    let batches = batches_root();
    let schema = fixture_schema();

    let store = two_generations(&index_dir, batches.path());
    let published = store.snapshot();
    assert_eq!(
        published.generation(),
        Generation::new(2),
        "the store must publish the generation of the second `replace_all`"
    );
    let published_filters = published.filters().clone();
    let published_live: Vec<u32> = published.live_iids().map(|iid| iid.get()).collect();
    drop(published);
    drop(store);

    let reopened = common::open_store_with(&index_dir, &schema);
    let snapshot = reopened.snapshot();
    assert_eq!(
        snapshot.generation(),
        Generation::new(2),
        "INV-04: a restart publishes the last committed generation, which is the one the \
         second `replace_all` committed"
    );
    assert_eq!(
        snapshot.filters(),
        &published_filters,
        "a restart must rebuild the filters the commit had published, not another state"
    );
    assert_eq!(
        snapshot.filters(),
        &rebuild(&index_dir, &schema),
        "the rebuilt filters must equal `FilterIndex::from_records` on the same reference state"
    );
    let live: Vec<u32> = snapshot.live_iids().map(|iid| iid.get()).collect();
    assert_eq!(
        live, published_live,
        "a restart must see the same live passages as the commit published"
    );
}

/// A temporary directory for the batches of one test.
fn batches_root() -> tempfile::TempDir {
    tempfile::tempdir().unwrap_or_else(|error| {
        panic!("a temporary directory for the batches must be creatable: {error}")
    })
}
