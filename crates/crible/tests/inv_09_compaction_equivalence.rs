// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `INV-09`: "Compaction ⇒ mêmes résultats avant et après", checked on the 200
//! queries of `synth-small` and on the queries of the Legifacile sample, as the
//! task `VERIFIER-EQUIVALENCE-COMPACTION` (phase P3) requires.
//!
//! References: `docs/08-invariants-et-tests.md` (INV-09), the task sheet of
//! `docs/phases/P3-cycle-de-vie.md` and the J3 milestone ("trois lots quotidiens
//! simulés, puis une compaction, donnent des résultats identiques avant et
//! après"), `docs/03-recherche.md` (exact dense search), `docs/02-architecture.md`
//! ("Suppressions, pierres tombales et compaction") and ADR-0006.
//!
//! # What the invariant means here
//!
//! The compaction renumbers the internal identifiers and rewrites the segments
//! (`crible_store::compact`). Nothing a caller can observe may change: the same
//! queries, over the same pool, must return the same public keys in the same
//! order, with the same scores within 1e-6. The tests therefore compare the
//! answers of the exact dense search, query by query, before and after the
//! compaction, and pin the premise that makes the comparison meaningful:
//!
//! - there are dead passages before the compaction, otherwise the compaction
//!   copies everything and proves nothing;
//! - there are none after it, so the tombstones really are what it removed;
//! - the live count and the pool size of every query are unchanged.
//!
//! # Why the `crible` crate
//!
//! This is the only test crate holding the exact search (`crible-search`), the
//! store and the compaction (`crible-store`) together, plus the CLI that
//! attaches a batch of an `onnx` index. The library path serves `synth-small`,
//! whose model is `external` and needs no ONNX Runtime; the CLI path serves
//! Legifacile, whose model is `mdenseon`, so the model resolution and the
//! acceptance check stay those of `crible attach`.
//!
//! # Datasets
//!
//! | Test | Data | Runs by default |
//! |---|---|---|
//! | `inv_09_synth_small_200_queries_return_the_same_results_after_compaction` | `data/synth-small/` | yes |
//! | `inv_09_the_legifacile_sample_test_fails_without_its_prerequisites` | none | yes |
//! | `inv_09_legifacile_sample_queries_return_the_same_results_after_compaction` | `data/legifacile-sample/` + `data/models/mdenseon/` + ONNX Runtime | no, `#[ignore]` |
//!
//! `synth-small` holds three upsert batches which really replace and delete
//! documents: `l2` replaces 300 documents and adds 100, `l3` deletes 150
//! documents and updates 50 metadata-only (`tools/gen_synth.py`). Those are the
//! "trois lots quotidiens simulés" of J3, and they leave tombstones to compact.
//!
//! The Legifacile sample holds one batch per structure and no replacement batch.
//! The test attaches every structure of the sample, the way `RECETTE-IMPORT`
//! does, and creates the tombstones the compaction needs with a batch of
//! deletions attached through the CLI: the production path of a removed document
//! (`docs/04`, ADR-0040 and ADR-0041). `attach` tombstones the passages and
//! deletes the `documents` row in the same transaction, so the state the test
//! compacts is one production really reaches. Nothing is written outside the
//! temporary directory.
//!
//! # Queries of the Legifacile sample
//!
//! `data/legifacile-sample/queries.jsonl` holds the query **question** and its
//! expected documents, no query vector. The questions are encoded here with
//! `QueryEncoder`, the encoder `crible search --query` uses, in fp32 (the default
//! of ADR-0052), so the exact search receives the vector a real query produces.
//! Ranking quality belongs to the eval, not to INV-09: only the stability of the
//! answers across the compaction is compared.
//!
//! The sample carries 300 questions spread over its six structures, and every
//! one of them is compared. The test attaches all six structures, so it takes a
//! long time and stays `#[ignore]`d; its duration is recorded at J3 and no
//! bound is prescribed (ADR-0062).
//!
//! ```text
//! cargo test -p crible --test inv_09_compaction_equivalence
//! ORT_DYLIB_PATH=/path/to/libonnxruntime.dylib \
//!   cargo test -p crible --test inv_09_compaction_equivalence -- --ignored
//! ```

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use arrow::array::{
    ArrayRef, BooleanArray, FixedSizeBinaryArray, Float64Array, Int64Array, StringArray,
    UInt32Array,
};
use arrow::datatypes::{DataType, Field, Schema as ArrowSchema};
use crible_core::config::{FieldLevel, FieldType, IndexConfig};
use crible_core::ids::{Epoch, Generation};
use crible_core::model::ModelConfig;
use crible_encode::QueryEncoder;
use crible_store::attach::attach;
use crible_store::generation::{GenerationStore, write_index_config};
use crible_store::records::{DocumentRecord, PassageRecord, ReadPool, RecordStore};
use crible_store::segment::SegmentReader;
use serde_json::{Value, json};

mod common;

use crate::common::{
    EXTERNAL_MODEL_JSON, LEGIFACILE_STRUCTURES, SYNTH_BATCHES, SYNTH_LIVE_PASSAGES, SamplePaths,
    assert_answer_is_not_vacuous, assert_same_results, crible_ok, legifacile_encoder,
    legifacile_queries, legifacile_sample_paths, path, read_file, repository_root, require_synth,
    search_all, synth_queries, synth_root,
};

/// INV-09 on `synth-small`: the 200 queries return the same keys, in the same
/// order, with the same scores within 1e-6, after a compaction that removes the
/// tombstones of the three simulated daily batches.
///
/// Acceptance criterion of `VERIFIER-EQUIVALENCE-COMPACTION`.
#[test]
fn inv_09_synth_small_200_queries_return_the_same_results_after_compaction() {
    let root = synth_root();
    require_synth(&root);
    let raw = read_file(&root.join("index.json"));
    let index = IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("the dataset index.json must be valid: {error}"));
    let model = ModelConfig::from_json_str(EXTERNAL_MODEL_JSON)
        .unwrap_or_else(|error| panic!("the dataset model configuration is valid: {error}"));

    let temp = tempfile::tempdir().unwrap_or_else(|error| {
        panic!("a temporary directory for the index must be creatable: {error}")
    });
    let index_dir = temp.path().join("indexes").join("synth-small");
    write_index_config(&index_dir, &raw)
        .unwrap_or_else(|error| panic!("the index configuration must be written: {error}"));
    let mut store = GenerationStore::create(&index_dir, Epoch::new(1), &index.schema)
        .unwrap_or_else(|error| panic!("the index must be created: {error}"));

    for (batch, generation) in SYNTH_BATCHES {
        let path = root.join(batch);
        attach(&mut store, &path, &index, &model, None, None).unwrap_or_else(|error| {
            panic!(
                "the attach of {batch} must succeed, got {}: {}",
                error.code().as_str(),
                error.message()
            )
        });
        assert_eq!(
            store.generation(),
            Generation::new(generation),
            "{batch} must publish generation {generation}"
        );
    }

    // Premise: the three batches really leave tombstones, and the live set is
    // the one `tools/gen_synth.py` describes.
    let before = store
        .state()
        .unwrap_or_else(|error| panic!("the state of the index must be readable: {error}"));
    assert_eq!(
        before.live_passages, SYNTH_LIVE_PASSAGES,
        "the three batches must leave {SYNTH_LIVE_PASSAGES} live passages"
    );
    assert!(
        before.dead_passages > 0,
        "the three batches must leave tombstones, otherwise the compaction proves \
         nothing: {before:?}"
    );

    let queries = synth_queries(&root, &index);
    assert_eq!(queries.len(), 200);

    let answers_before = search_all(&store, &queries);
    for (query, answer) in queries.iter().zip(&answers_before) {
        assert_answer_is_not_vacuous(query, answer);
    }

    let report = crible_store::compact::compact(&mut store, &index)
        .unwrap_or_else(|error| panic!("the compaction must succeed: {error}"));
    assert_eq!(
        report.removed_passages, before.dead_passages,
        "the compaction must remove exactly the tombstones of the three batches"
    );
    assert_eq!(report.live_passages, before.live_passages);

    let after = store.state().unwrap_or_else(|error| {
        panic!("the state of the compacted index must be readable: {error}")
    });
    assert_eq!(
        after.dead_passages, 0,
        "no dead passage may remain after the compaction"
    );
    assert_eq!(after.live_passages, before.live_passages);
    assert_eq!(after.generation, Generation::new(4));
    assert_eq!(after.epoch, Epoch::new(2));

    let answers_after = search_all(&store, &queries);
    assert_eq!(answers_before.len(), answers_after.len());
    for ((query, before), after) in queries.iter().zip(&answers_before).zip(&answers_after) {
        assert_same_results("the compaction", &query.label, before, after);
    }
    eprintln!(
        "INV-09: 200 queries of synth-small compared, {} tombstone(s) removed, \
         {} live passage(s) kept",
        report.removed_passages, report.live_passages
    );
}

// --- The Legifacile sample ---

/// An `#[ignore]` test launched explicitly without its prerequisite fails,
/// instead of passing while proving nothing (ADR-0057).
#[test]
fn inv_09_the_legifacile_sample_test_fails_without_its_prerequisites() {
    let root = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));

    let error = legifacile_sample_paths(
        root.path(),
        "inv_09_legifacile_sample_queries_return_the_same_results_after_compaction",
    )
    .expect_err("an empty root holds no Legifacile sample, so the check must fail");

    assert!(
        error.contains("data/legifacile-sample/configs/dole.json"),
        "the message must name the missing file: {error}"
    );
    assert!(
        error.contains("data/legifacile-sample/README.md"),
        "the message must say where the sample is produced: {error}"
    );
    assert!(
        error.contains("export_onnx.py"),
        "the message must give the command that produces the model: {error}"
    );
    assert!(
        error.contains("inv_09_legifacile_sample_queries_return_the_same_results_after_compaction"),
        "the message must name the test it blocks: {error}"
    );
}

/// Stable rank of a document identifier: the same identifier always falls on
/// the same side, so the set of removed documents is deterministic.
fn stable_rank(doc_id: &str) -> u64 {
    doc_id
        .bytes()
        .fold(0_u64, |sum, byte| sum + u64::from(byte))
}

/// Identifiers of every document holding a live passage, in ascending order.
fn live_documents(store: &GenerationStore) -> Vec<String> {
    let pool = ReadPool::new(&store.records_path(), 2)
        .unwrap_or_else(|error| panic!("the read pool must open: {error}"));
    let mut documents = BTreeSet::new();
    for iid in store.snapshot().live_iids() {
        let key = pool
            .read_passage_key(iid)
            .unwrap_or_else(|error| panic!("the key of iid {} must read: {error}", iid.get()))
            .unwrap_or_else(|| panic!("iid {} is live but has no passage row", iid.get()));
        documents.insert(key.doc_id().as_str().to_owned());
    }
    documents.into_iter().collect()
}

/// The documents to remove: one in ten, chosen by [`stable_rank`] so that the
/// fixture is deterministic.
fn documents_to_remove(documents: &[String]) -> Vec<String> {
    let removed: Vec<String> = documents
        .iter()
        .filter(|doc_id| stable_rank(doc_id).is_multiple_of(10))
        .cloned()
        .collect();
    assert!(
        !removed.is_empty(),
        "the fixture must find documents to remove"
    );
    removed
}

/// One live document copied from the index, with the reference vector of its
/// first passage.
///
/// The batch format requires at least one `documents.parquet` and one
/// `passages.parquet` (`docs/04`, ADR-0040), so a batch that removes documents
/// still carries one document. Copying a document that stays in the index keeps
/// the attach from writing anything for it: the same `doc_hash` and the same
/// fields make it `unchanged`, and a `deletion-only` batch does not exist.
struct CopiedDocument {
    document: DocumentRecord,
    passage: PassageRecord,
    vector: Vec<f32>,
}

fn copy_live_document(
    store: &GenerationStore,
    documents: &[String],
    removed: &BTreeSet<String>,
) -> CopiedDocument {
    let records = RecordStore::open(&store.records_path())
        .unwrap_or_else(|error| panic!("the records of the epoch must open: {error}"));
    for doc_id in documents {
        if removed.contains(doc_id) {
            continue;
        }
        let Some(document) = records
            .read_document(doc_id)
            .unwrap_or_else(|error| panic!("the document {doc_id} must be readable: {error}"))
        else {
            continue;
        };
        let passages = records
            .passages_of_document(doc_id)
            .unwrap_or_else(|error| panic!("the passages of {doc_id} must be readable: {error}"));
        let Some(passage) = passages
            .into_iter()
            .find(|passage| passage.ord == 0 && passage.dead_since_gen.is_none())
        else {
            continue;
        };
        let directory = store.segments_directory().join(&passage.segment_id);
        let reader = SegmentReader::open(&directory).unwrap_or_else(|error| {
            panic!(
                "the segment {} must open: {error}",
                passage.segment_id.as_str()
            )
        });
        let vector = reader.vector(passage.iid).unwrap_or_else(|error| {
            panic!("the vector of {doc_id}#{} must read: {error}", passage.ord)
        });
        return CopiedDocument {
            document,
            passage,
            vector,
        };
    }
    panic!("the fixture must find a document to copy into the deletion batch");
}

/// Writes a batch that removes `deletions` and carries one unchanged `kept`
/// document, then returns the directory of the batch.
///
/// This is the production path of a removal: the manifest is the one of
/// `template`, with the identifiers and the counts of this batch, and `attach`
/// applies it (`docs/04`, ADR-0040). The attach deletes the `documents` row of
/// every removed document along with its passages, in one transaction
/// (ADR-0041, decision 1).
fn write_deletion_batch(
    template: &Path,
    batches: &Path,
    batch_id: &str,
    index: &IndexConfig,
    kept: &CopiedDocument,
    deletions: &[String],
) -> PathBuf {
    let directory = batches.join(batch_id);
    fs::create_dir_all(&directory)
        .unwrap_or_else(|error| panic!("{} must be creatable: {error}", directory.display()));

    let mut document_fields = vec![
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("doc_hash", DataType::Utf8, false),
    ];
    let mut document_columns: Vec<ArrayRef> = vec![
        Arc::new(single_string(&kept.document.doc_id)),
        Arc::new(single_string(&kept.document.doc_hash)),
    ];
    for (name, level, spec) in index.schema.fields() {
        if level != FieldLevel::Document {
            continue;
        }
        document_fields.push(Field::new(name, arrow_type(spec.field_type, name), true));
        document_columns.push(field_column(
            spec.field_type,
            kept.document.fields.get(name).unwrap_or(&Value::Null),
            name,
        ));
    }

    let bytes: Vec<u8> = kept
        .vector
        .iter()
        .flat_map(|value| half::f16::from_f32(*value).to_bits().to_le_bytes())
        .collect();
    let width = i32::try_from(bytes.len())
        .unwrap_or_else(|error| panic!("the vector width must fit an i32: {error}"));
    let mut passage_fields = vec![
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("ord", DataType::UInt32, false),
        Field::new("text", DataType::Utf8, false),
        Field::new("vector", DataType::FixedSizeBinary(width), false),
    ];
    let vector =
        FixedSizeBinaryArray::try_from_sparse_iter_with_size(std::iter::once(Some(bytes)), width)
            .unwrap_or_else(|error| panic!("the fixture vector has one width: {error}"));
    let mut passage_columns: Vec<ArrayRef> = vec![
        Arc::new(single_string(&kept.document.doc_id)),
        Arc::new(UInt32Array::from(vec![kept.passage.ord])),
        Arc::new(single_string(&kept.passage.text)),
        Arc::new(vector),
    ];
    for (name, level, spec) in index.schema.fields() {
        if level != FieldLevel::Passage {
            continue;
        }
        passage_fields.push(Field::new(name, arrow_type(spec.field_type, name), true));
        passage_columns.push(field_column(
            spec.field_type,
            kept.passage.fields.get(name).unwrap_or(&Value::Null),
            name,
        ));
    }

    let documents = directory.join("documents.parquet");
    common::write_parquet(
        &documents,
        &Arc::new(ArrowSchema::new(document_fields)),
        document_columns,
    );
    let passages = directory.join("passages.parquet");
    common::write_parquet(
        &passages,
        &Arc::new(ArrowSchema::new(passage_fields)),
        passage_columns,
    );
    let deletions_path = directory.join("deletions.parquet");
    common::write_parquet(
        &deletions_path,
        &Arc::new(ArrowSchema::new(vec![Field::new(
            "doc_id",
            DataType::Utf8,
            false,
        )])),
        vec![Arc::new(
            deletions
                .iter()
                .map(|doc_id| Some(doc_id.as_str()))
                .collect::<StringArray>(),
        )],
    );

    let mut files = serde_json::Map::new();
    for path in [&documents, &passages, &deletions_path] {
        let bytes = fs::read(path)
            .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_else(|| panic!("{} must have a UTF-8 name", path.display()));
        files.insert(
            name.to_owned(),
            Value::from(format!("blake3:{}", blake3::hash(&bytes).to_hex())),
        );
    }

    let mut manifest: Value = serde_json::from_str(&read_file(template))
        .unwrap_or_else(|error| panic!("{} must be valid JSON: {error}", template.display()));
    manifest["batch_id"] = Value::from(batch_id);
    manifest["mode"] = Value::from("upsert");
    manifest["counts"] = json!({
        "documents": 1,
        "passages": 1,
        "deletions": deletions.len(),
    });
    manifest["files"] = Value::Object(files);
    let manifest = serde_json::to_string_pretty(&manifest)
        .unwrap_or_else(|error| panic!("the manifest must serialize: {error}"));
    fs::write(directory.join("manifest.json"), manifest)
        .unwrap_or_else(|error| panic!("manifest.json must be written: {error}"));
    directory
}

/// One string array holding a single value.
fn single_string(value: &str) -> StringArray {
    std::iter::once(Some(value)).collect()
}

/// Arrow data type of one schema field, as the batch format writes it.
fn arrow_type(field_type: FieldType, name: &str) -> DataType {
    match field_type {
        FieldType::Keyword => DataType::Utf8,
        FieldType::Integer => DataType::Int64,
        FieldType::Float => DataType::Float64,
        FieldType::Bool => DataType::Boolean,
        FieldType::Date => {
            panic!("the fixture does not build the date field '{name}'; its Arrow form is Date32")
        }
    }
}

/// One single-row column holding a field value read from the index.
fn field_column(field_type: FieldType, value: &Value, name: &str) -> ArrayRef {
    if value.is_null() {
        return match field_type {
            FieldType::Keyword => Arc::new(StringArray::from(vec![None::<&str>])),
            FieldType::Integer => Arc::new(Int64Array::from(vec![None::<i64>])),
            FieldType::Float => Arc::new(Float64Array::from(vec![None::<f64>])),
            FieldType::Bool => Arc::new(BooleanArray::from(vec![None::<bool>])),
            FieldType::Date => panic!(
                "the fixture does not build the date field '{name}'; its Arrow form is Date32"
            ),
        };
    }
    match field_type {
        FieldType::Keyword => {
            let value = value
                .as_str()
                .unwrap_or_else(|| panic!("the field '{name}' must be a string, got {value}"));
            Arc::new(single_string(value))
        }
        FieldType::Integer => {
            let value = value
                .as_i64()
                .unwrap_or_else(|| panic!("the field '{name}' must be an integer, got {value}"));
            Arc::new(Int64Array::from(vec![value]))
        }
        FieldType::Float => {
            let value = value
                .as_f64()
                .unwrap_or_else(|| panic!("the field '{name}' must be a number, got {value}"));
            Arc::new(Float64Array::from(vec![value]))
        }
        FieldType::Bool => {
            let value = value
                .as_bool()
                .unwrap_or_else(|| panic!("the field '{name}' must be a boolean, got {value}"));
            Arc::new(BooleanArray::from(vec![value]))
        }
        FieldType::Date => {
            panic!("the fixture does not build the date field '{name}'; its Arrow form is Date32")
        }
    }
}

/// INV-09 on the Legifacile sample: the queries of every structure return the
/// same keys, in the same order, with the same scores within 1e-6, after a
/// compaction that removes the tombstones.
///
/// The sample holds one batch per structure and no replacement batch, so the
/// tombstones come from a batch of deletions attached the way production removes
/// a document. The query vectors are encoded from the questions of the sample
/// with the query encoder of the model, in fp32.
#[test]
#[ignore = "needs data/legifacile-sample, data/models/mdenseon and ORT_DYLIB_PATH; run: ORT_DYLIB_PATH=/path/to/libonnxruntime.dylib cargo test -p crible --test inv_09_compaction_equivalence -- --ignored"]
fn inv_09_legifacile_sample_queries_return_the_same_results_after_compaction() {
    let root = repository_root();
    let sample = legifacile_sample_paths(
        &root,
        "inv_09_legifacile_sample_queries_return_the_same_results_after_compaction",
    )
    .unwrap_or_else(|reason| panic!("{reason}"));
    assert!(
        std::env::var_os("ORT_DYLIB_PATH").is_some(),
        "ORT_DYLIB_PATH must name the ONNX Runtime library: the attach of an 'onnx' index \
         re-encodes a sample (control 9 of docs/04) and the questions are encoded with the model"
    );

    let temp = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));

    // One encoder for the whole sample: every structure uses the same model.
    let first_config = sample.config(LEGIFACILE_STRUCTURES[0]);
    let first_raw = read_file(&first_config);
    let first_index = IndexConfig::from_json_str(&first_raw)
        .unwrap_or_else(|error| panic!("{} must be valid: {error}", first_config.display()));
    let encoder = legifacile_encoder(&sample, &first_index);

    let started = Instant::now();
    let mut compared = 0_usize;
    for structure in LEGIFACILE_STRUCTURES {
        compared += check_structure(&sample, temp.path(), structure, &encoder);
    }
    assert_eq!(
        compared, 300,
        "the sample must hold the 300 queries of queries.jsonl, got {compared}"
    );
    eprintln!(
        "INV-09: {compared} queries of the Legifacile sample compared over {} structure(s) in \
         {} s",
        LEGIFACILE_STRUCTURES.len(),
        started.elapsed().as_secs()
    );
}

/// Runs INV-09 on one structure of the sample, and returns the number of queries
/// compared.
fn check_structure(
    sample: &SamplePaths,
    temp: &Path,
    structure: &str,
    encoder: &QueryEncoder,
) -> usize {
    let config = sample.config(structure);
    let batch = sample.batch(structure);
    let data = temp.join("data");
    crible_ok(&[
        "index",
        "create",
        "--data",
        path(&data),
        "--config",
        path(&config),
    ]);
    crible_ok(&[
        "attach",
        "--data",
        path(&data),
        "--models",
        path(&sample.models),
        "--index",
        structure,
        path(&batch),
    ]);

    let raw = read_file(&config);
    let index = IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("{} must be valid: {error}", config.display()));
    let index_dir = data.join("indexes").join(structure);
    let store = GenerationStore::open(&index_dir, &index.schema)
        .unwrap_or_else(|error| panic!("the attached index {structure} must open: {error}"));

    let queries = legifacile_queries(sample, structure, &index, encoder);

    let attached = store
        .state()
        .unwrap_or_else(|error| panic!("the state of {structure} must be readable: {error}"));
    assert_eq!(
        attached.dead_passages, 0,
        "structure {structure} holds no tombstone before the simulated deletion"
    );

    // The simulated deletion goes through `attach`, the only production writer:
    // a batch that declares the removals in `deletions` deletes the `documents`
    // row along with the passages in one transaction (ADR-0041, decision 1), so
    // the state is one production reaches.
    let documents = live_documents(&store);
    let victims = documents_to_remove(&documents);
    let victim_set: BTreeSet<String> = victims.iter().cloned().collect();
    let victims_count = u64::try_from(victims.len())
        .unwrap_or_else(|error| panic!("the number of removed documents fits a u64: {error}"));
    assert!(
        victims_count < attached.documents,
        "the fixture must keep documents, got {victims_count} removed of {} in {structure}",
        attached.documents
    );
    let kept = copy_live_document(&store, &documents, &victim_set);
    let deletion_batch = write_deletion_batch(
        &batch.join("manifest.json"),
        &temp.join("batches"),
        &format!("legifacile-sample-{structure}-deletions-v1"),
        &index,
        &kept,
        &victims,
    );
    drop(store);
    crible_ok(&[
        "attach",
        "--data",
        path(&data),
        "--models",
        path(&sample.models),
        "--index",
        structure,
        path(&deletion_batch),
    ]);

    let mut store = GenerationStore::open(&index_dir, &index.schema)
        .unwrap_or_else(|error| panic!("the attached index {structure} must open again: {error}"));
    let deleted = store.state().unwrap_or_else(|error| {
        panic!("the state of {structure} after the deletion must be readable: {error}")
    });
    assert_eq!(
        deleted.documents,
        attached.documents - victims_count,
        "the attach of a batch of deletions must delete the documents rows of {structure} \
         (ADR-0041)"
    );
    let records = RecordStore::open(&store.records_path())
        .unwrap_or_else(|error| panic!("the records of the epoch must open: {error}"));
    for doc_id in &victims {
        assert!(
            records
                .read_document(doc_id)
                .unwrap_or_else(|error| panic!("the document {doc_id} must be readable: {error}"))
                .is_none(),
            "the document {doc_id} of {structure} has no live passage left but keeps a \
             documents row, a state production cannot reach"
        );
    }
    drop(records);

    let before = store
        .state()
        .unwrap_or_else(|error| panic!("the state of {structure} must be readable: {error}"));
    assert!(
        before.dead_passages > 0,
        "the simulated deletion of {structure} must leave tombstones, otherwise the \
         compaction proves nothing"
    );
    assert!(
        before.dead_ratio() > 0.01,
        "the fixture of {structure} must hold a noticeable share of tombstones, got {}",
        before.dead_ratio()
    );

    let answers_before = search_all(&store, &queries);
    for (query, answer) in queries.iter().zip(&answers_before) {
        assert_answer_is_not_vacuous(query, answer);
    }

    let report = crible_store::compact::compact(&mut store, &index)
        .unwrap_or_else(|error| panic!("the compaction of {structure} must succeed: {error}"));
    assert_eq!(report.removed_passages, before.dead_passages);
    assert_eq!(report.live_passages, before.live_passages);

    let after = store.state().unwrap_or_else(|error| {
        panic!("the state of the compacted {structure} must be readable: {error}")
    });
    assert_eq!(
        after.dead_passages, 0,
        "no dead passage may remain in {structure} after the compaction"
    );
    assert_eq!(after.live_passages, before.live_passages);
    assert_eq!(
        after.documents, before.documents,
        "the compaction must keep one documents row per live document in {structure}"
    );
    assert_eq!(
        after.generation,
        Generation::new(before.generation.get() + 1),
        "the compaction publishes G+1"
    );
    assert_eq!(after.epoch.get(), before.epoch.get() + 1);

    let answers_after = search_all(&store, &queries);
    for ((query, before), after) in queries.iter().zip(&answers_before).zip(&answers_after) {
        assert_same_results("the compaction", &query.label, before, after);
    }
    eprintln!(
        "INV-09: {structure}: {} queries compared, {} document(s) and {} tombstone(s) removed, \
         {} live passage(s) kept",
        queries.len(),
        victims_count,
        report.removed_passages,
        report.live_passages
    );
    queries.len()
}
