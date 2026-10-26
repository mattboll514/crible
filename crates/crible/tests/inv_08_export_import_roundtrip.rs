// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `INV-08`: "Export puis import ⇒ mêmes résultats (mêmes clés, scores à 1e-6)",
//! checked on the 200 queries of `synth-small` and on the queries of the
//! Legifacile sample, as the task `VERIFIER-ALLER-RETOUR-EXPORT` (phase P3)
//! requires.
//!
//! References: `docs/08-invariants-et-tests.md` (INV-08), the task sheet of
//! `docs/phases/P3-cycle-de-vie.md`, `docs/04-donnees-et-ingestion.md` ("Export,
//! instantanés, restauration"), `docs/format-lot-v1.md` ("Lot d'export"),
//! `docs/03-recherche.md` (exact dense search), ADR-0061 (manifest of an exported
//! batch) and ADR-0063 (the local Legifacile test).
//!
//! # What the invariant means here
//!
//! The export writes the live data of one generation as a `replace_all` batch
//! (`crible_store::export`), and the import is the attach of that batch into an
//! empty index of the same configuration (`crible_store::attach`). Nothing a
//! caller can observe may change: the same queries, over the same pool, must
//! return the same public keys in the same order, with the same scores within
//! 1e-6, and the same payload. The tests therefore compare the answers of the
//! exact dense search, query by query, of the source index and of the index the
//! batch was attached into, and pin the premises that make the comparison
//! meaningful:
//!
//! - the source index holds tombstones, so the export really drops them instead
//!   of copying everything;
//! - a metadata patch has changed a live document, so the export carries a
//!   document state no batch of `synth-small` wrote;
//! - the exported batch carries exactly the live rows;
//! - the imported index is empty before the attach, holds no tombstone after it,
//!   and the same live and document counts as the source;
//! - the query set holds filtered and unfiltered queries, and the pools they
//!   keep are non-empty.
//!
//! # Datasets
//!
//! | Test | Data | Runs by default |
//! |---|---|---|
//! | `inv_08_synth_small_200_queries_return_the_same_results_after_export_and_import` | `data/synth-small/` | yes |
//! | `inv_08_the_legifacile_sample_test_fails_without_its_prerequisites` | none | yes |
//! | `inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import` | `data/legifacile-sample/` + `data/models/mdenseon/` + ONNX Runtime | no, `#[ignore]` (ADR-0057, ADR-0063) |
//!
//! `synth-small` holds three upsert batches which really replace and delete
//! documents: `l2` replaces 300 documents and adds 100, `l3` deletes 150
//! documents and updates 50 metadata-only (`tools/gen_synth.py`). They leave
//! the tombstones the export must skip. The test then patches the metadata of
//! one live document, so the exported batch carries a field state that no
//! producer wrote.
//!
//! The Legifacile sample holds one batch per structure, whose model is
//! `mdenseon`: the import goes through `crible attach`, so it needs ONNX Runtime
//! and the model, as ADR-0063 states. The test exports and imports every
//! structure, and compares the answers of the 300 questions of the sample.
//!
//! # Why the `crible` crate
//!
//! This is the only test crate holding the exact search (`crible-search`), the
//! store, the export and the attach (`crible-store`) together, plus the CLI the
//! Legifacile path needs for model resolution and the acceptance check.
//!
//! ```text
//! cargo test -p crible --test inv_08_export_import_roundtrip
//! ORT_DYLIB_PATH=/path/to/libonnxruntime.dylib \
//!   cargo test -p crible --test inv_08_export_import_roundtrip -- --ignored
//! ```

mod common;

use std::path::Path;
use std::time::Instant;

use crible_core::config::IndexConfig;
use crible_core::ids::{Epoch, Generation};
use crible_core::model::ModelConfig;
use crible_encode::QueryEncoder;
use crible_store::attach::attach;
use crible_store::export::export;
use crible_store::generation::{GenerationStore, write_index_config};
use crible_store::patch::{DocumentPatch, PatchReport, PatchSource};
use crible_store::records::{ReadPool, RecordStore};
use serde_json::{Map, Value};

use crate::common::{
    EXTERNAL_MODEL_JSON, LEGIFACILE_STRUCTURES, Query, SYNTH_BATCHES, SYNTH_LIVE_PASSAGES,
    SamplePaths, array, assert_answer_is_not_vacuous, assert_same_results, crible_ok,
    legifacile_encoder, legifacile_queries, legifacile_sample_paths, parse_filter, path, read_file,
    read_json_lines, repository_root, require_synth, search_all, synth_queries, synth_root,
};

/// The two filters every query is also run with, alone and never on top of the
/// query's own filter: a keyword equality and an interval, both non-trivial on
/// `synth-small` (`annee` runs from 1990 to 2025, `categorie` has 16 values).
const FIXED_FILTERS: [&str; 2] = [
    r#"{ "categorie": "cat-00" }"#,
    r#"{ "annee": { "gte": 2010 } }"#,
];

/// The vector of the first query, used by the fixed-filter queries.
fn first_vector(root: &Path) -> Vec<f32> {
    let rows = read_json_lines(&root.join("queries.jsonl"));
    let first = rows
        .first()
        .unwrap_or_else(|| panic!("queries.jsonl must hold at least one query"));
    array(first, "vector")
}

/// Patches the metadata of the first live document, so the exported batch
/// carries a document state no producer wrote.
///
/// The patch goes through `crible_store::patch`, the production writer of a
/// metadata-only change: the fields change, `doc_hash` does not, and one
/// generation `G+1` is committed (ADR-0060). The new values are chosen against
/// the stored ones so the patch is guaranteed to change something; a patch that
/// changed nothing would publish no generation and make this premise vacuous.
fn patch_one_document(store: &mut GenerationStore, index: &IndexConfig) -> (String, PatchReport) {
    let doc_id = {
        let pool = ReadPool::new(&store.records_path(), 2)
            .unwrap_or_else(|error| panic!("the read pool must open: {error}"));
        store
            .snapshot()
            .live_iids()
            .next()
            .map(|iid| {
                pool.read_passage_key(iid)
                    .unwrap_or_else(|error| {
                        panic!("the key of iid {} must read: {error}", iid.get())
                    })
                    .unwrap_or_else(|| panic!("iid {} is live but has no passage row", iid.get()))
                    .doc_id()
                    .as_str()
                    .to_owned()
            })
            .unwrap_or_else(|| panic!("the index must hold a live passage to patch"))
    };

    let document = {
        let records = RecordStore::open(&store.records_path())
            .unwrap_or_else(|error| panic!("the records of the epoch must open: {error}"));
        records
            .read_document(&doc_id)
            .unwrap_or_else(|error| panic!("the document {doc_id} must be readable: {error}"))
            .unwrap_or_else(|| panic!("the document {doc_id} is live but has no documents row"))
    };

    let stored_year = document.fields.get("annee").and_then(Value::as_i64);
    let year = if stored_year == Some(1_999) {
        2_000
    } else {
        1_999
    };
    let stored_published = document.fields.get("publie").and_then(Value::as_bool);
    let mut fields = Map::new();
    fields.insert("annee".to_owned(), Value::from(year));
    fields.insert(
        "publie".to_owned(),
        Value::from(!stored_published.unwrap_or(false)),
    );

    let patch = DocumentPatch {
        doc_id: doc_id.clone(),
        fields,
    };
    let report = crible_store::patch::apply(store, &index.schema, PatchSource::File, &[patch])
        .unwrap_or_else(|error| {
            panic!(
                "the metadata patch of {doc_id} must apply, got {}: {}",
                error.code().as_str(),
                error.message()
            )
        });
    (doc_id, report)
}

/// The stored document fields of one document, read from the index.
fn document_fields(store: &GenerationStore, doc_id: &str) -> Value {
    let records = RecordStore::open(&store.records_path())
        .unwrap_or_else(|error| panic!("the records of the epoch must open: {error}"));
    records
        .read_document(doc_id)
        .unwrap_or_else(|error| panic!("the document {doc_id} must be readable: {error}"))
        .unwrap_or_else(|| panic!("the document {doc_id} must keep a documents row"))
        .fields
}

/// Creates an index of the given configuration under `data_root`, and returns
/// it open, empty.
fn create_empty_index(
    data_root: &Path,
    name: &str,
    raw: &str,
    index: &IndexConfig,
) -> GenerationStore {
    let index_dir = data_root.join("indexes").join(name);
    write_index_config(&index_dir, raw)
        .unwrap_or_else(|error| panic!("the index configuration must be written: {error}"));
    let store = GenerationStore::create(&index_dir, Epoch::new(1), &index.schema)
        .unwrap_or_else(|error| panic!("the index must be created: {error}"));
    let state = store
        .state()
        .unwrap_or_else(|error| panic!("the state of the new index must be readable: {error}"));
    assert_eq!(
        (state.live_passages, state.documents),
        (0, 0),
        "the index {name} must be created empty"
    );
    store
}

/// INV-08 on `synth-small`: the 200 queries return the same keys, in the same
/// order, with the same scores within 1e-6 and the same payload, after the live
/// data of the source index is exported and attached into an empty index of the
/// same configuration.
///
/// Acceptance criterion of `VERIFIER-ALLER-RETOUR-EXPORT`.
#[test]
fn inv_08_synth_small_200_queries_return_the_same_results_after_export_and_import() {
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
    let mut source = create_empty_index(&temp.path().join("source"), "synth-small", &raw, &index);

    for (batch, generation) in SYNTH_BATCHES {
        let path = root.join(batch);
        attach(&mut source, &path, &index, &model, None, None).unwrap_or_else(|error| {
            panic!(
                "the attach of {batch} must succeed, got {}: {}",
                error.code().as_str(),
                error.message()
            )
        });
        assert_eq!(
            source.generation(),
            Generation::new(generation),
            "{batch} must publish generation {generation}"
        );
    }

    // Premise: the three batches leave tombstones, so the export really drops
    // them, and the live set is the one `tools/gen_synth.py` describes.
    let after_batches = source
        .state()
        .unwrap_or_else(|error| panic!("the state of the source index must be readable: {error}"));
    assert_eq!(
        after_batches.live_passages, SYNTH_LIVE_PASSAGES,
        "the three batches must leave {SYNTH_LIVE_PASSAGES} live passages"
    );
    assert!(
        after_batches.dead_passages > 0,
        "the three batches must leave tombstones, otherwise the export only copies: \
         {after_batches:?}"
    );
    let documents_before_patch = after_batches.documents;

    // Premise: one live document carries fields no producer wrote, and the
    // patch publishes a generation.
    let generation_before_patch = source.generation();
    let (patched_doc_id, patch_report) = patch_one_document(&mut source, &index);
    assert_eq!(
        patch_report.updated, 1,
        "the metadata patch of {patched_doc_id} must change exactly one document"
    );
    assert_eq!(
        patch_report.generation,
        Generation::new(generation_before_patch.get() + 1),
        "a metadata patch that changes a document publishes G+1 (ADR-0060)"
    );

    let exported_state = source.state().unwrap_or_else(|error| {
        panic!("the state of the source index after the patch must be readable: {error}")
    });
    assert_eq!(exported_state.documents, documents_before_patch);
    let patched_fields = document_fields(&source, &patched_doc_id);
    assert!(
        patched_fields.as_object().is_some_and(|fields| {
            fields
                .get("annee")
                .is_some_and(|year| year.as_i64().is_some())
                && fields.contains_key("publie")
        }),
        "the patched document {patched_doc_id} must carry the patched fields, got \
         {patched_fields}"
    );

    // Export: the live data of the published generation, as a `replace_all`
    // batch under `<exchange>/outbox/synth-small-g<G>/`.
    let exchange = temp.path().join("exchange");
    let report = export(&source, &index, &model, &exchange).unwrap_or_else(|error| {
        panic!(
            "the export of synth-small must succeed, got {}: {}",
            error.code().as_str(),
            error.message()
        )
    });
    assert_eq!(report.generation, exported_state.generation);
    assert_eq!(
        report.documents, exported_state.documents,
        "the export must carry every live document"
    );
    assert_eq!(
        report.passages, exported_state.live_passages,
        "the export must carry every live passage and no tombstone"
    );

    // Import: an empty index of the same configuration, and the attach of the
    // exported batch, which is the production path of an import.
    let mut imported =
        create_empty_index(&temp.path().join("imported"), "synth-small", &raw, &index);
    let attach_report = attach(&mut imported, &report.directory, &index, &model, None, None)
        .unwrap_or_else(|error| {
            panic!(
                "the attach of the exported batch must succeed, got {}: {}",
                error.code().as_str(),
                error.message()
            )
        });
    assert_eq!(
        attach_report.added, exported_state.documents,
        "the import must add every document of the export"
    );
    assert_eq!(
        attach_report.deleted, 0,
        "the import goes into an empty index, so a replace_all batch deletes nothing"
    );

    let imported_state = imported.state().unwrap_or_else(|error| {
        panic!("the state of the imported index must be readable: {error}")
    });
    assert_eq!(
        imported_state.live_passages, exported_state.live_passages,
        "the import must hold the same live passages as the export"
    );
    assert_eq!(
        imported_state.documents, exported_state.documents,
        "the import must hold the same documents as the export"
    );
    assert_eq!(
        imported_state.dead_passages, 0,
        "a replace_all batch carries no tombstone, so the import holds none"
    );
    // Premise: the metadata patch of the source survived the export and the
    // import, so the round trip carries the document state, not only the
    // vectors.
    assert_eq!(
        document_fields(&imported, &patched_doc_id),
        patched_fields,
        "the imported index must carry the patched metadata of {patched_doc_id}"
    );

    // The queries: the 200 of the dataset, which mix unfiltered and filtered
    // ones, then the same vector under each fixed filter.
    let queries = synth_queries(&root, &index);
    assert_eq!(queries.len(), 200);
    let unfiltered = read_json_lines(&root.join("queries.jsonl"))
        .iter()
        .filter(|row| !matches!(row.get("filter"), Some(filter) if filter.is_object()))
        .count();
    assert!(
        unfiltered > 0 && unfiltered < queries.len(),
        "the query set must hold both unfiltered and filtered queries, got {unfiltered} \
         unfiltered of {}",
        queries.len()
    );
    let vector = first_vector(&root);
    let fixed: Vec<Query> = FIXED_FILTERS
        .iter()
        .map(|raw| Query {
            label: format!("synth-fixed{raw}"),
            k: 10,
            vector: vector.clone(),
            filter: parse_filter(raw, &index.schema),
        })
        .collect();

    let before = search_all(&source, &queries);
    for (query, answer) in queries.iter().zip(&before) {
        assert_answer_is_not_vacuous(query, answer);
    }
    let before_fixed = search_all(&source, &fixed);
    for (query, answer) in fixed.iter().zip(&before_fixed) {
        assert_answer_is_not_vacuous(query, answer);
        assert!(
            usize::try_from(answer.pool_size).unwrap_or(usize::MAX) < SYNTH_LIVE_PASSAGES as usize,
            "the fixed filter {} must keep a strict subset of the live set, got {} passages",
            query.label,
            answer.pool_size
        );
    }

    let after = search_all(&imported, &queries);
    for ((query, before), after) in queries.iter().zip(&before).zip(&after) {
        assert_same_results("the import", &query.label, before, after);
    }
    let after_fixed = search_all(&imported, &fixed);
    for ((query, before), after) in fixed.iter().zip(&before_fixed).zip(&after_fixed) {
        assert_same_results("the import", &query.label, before, after);
    }
    eprintln!(
        "INV-08: {} queries of synth-small compared ({} unfiltered, {} filtered, {} fixed \
         filters), {} document(s) and {} live passage(s) exported after {} tombstone(s) were \
         dropped, and imported into an empty index",
        queries.len(),
        unfiltered,
        queries.len() - unfiltered,
        fixed.len(),
        report.documents,
        report.passages,
        after_batches.dead_passages
    );
}

/// An `#[ignore]` test launched explicitly without its prerequisite fails,
/// instead of passing while proving nothing (ADR-0057).
#[test]
fn inv_08_the_legifacile_sample_test_fails_without_its_prerequisites() {
    let root = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));

    let error = legifacile_sample_paths(
        root.path(),
        "inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import",
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
        error.contains(
            "inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import"
        ),
        "the message must name the test it blocks: {error}"
    );
}

/// INV-08 on the Legifacile sample: the queries of every structure return the
/// same keys, in the same order, with the same scores within 1e-6, after the
/// export of the structure and its attach into an empty index of the same
/// configuration.
///
/// The import goes through the attach of an `onnx` index, so it runs the
/// acceptance check and needs ONNX Runtime and the model to open: hence the
/// test is `#[ignore]`d (ADR-0057, ADR-0063).
#[test]
#[ignore = "needs data/legifacile-sample, data/models/mdenseon and ORT_DYLIB_PATH; run: ORT_DYLIB_PATH=/path/to/libonnxruntime.dylib cargo test -p crible --test inv_08_export_import_roundtrip -- --ignored"]
fn inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import() {
    let root = repository_root();
    let sample = legifacile_sample_paths(
        &root,
        "inv_08_legifacile_sample_queries_return_the_same_results_after_export_and_import",
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
        "INV-08: {compared} queries of the Legifacile sample compared over {} structure(s) in \
         {} s",
        LEGIFACILE_STRUCTURES.len(),
        started.elapsed().as_secs()
    );
}

/// Runs INV-08 on one structure of the sample, and returns the number of queries
/// compared.
///
/// The source index is built by the CLI (`index create`, `attach`), which is the
/// path that resolves the model and runs the acceptance check; the export is
/// `crible export`; the import is `crible attach` of the exported batch into a
/// second data root holding an empty index of the same configuration. The
/// searches are then compared in process, over the two stores.
fn check_structure(
    sample: &SamplePaths,
    temp: &Path,
    structure: &str,
    encoder: &QueryEncoder,
) -> usize {
    let config = sample.config(structure);
    let batch = sample.batch(structure);
    let data = temp.join(structure).join("data");
    let imported_data = temp.join(structure).join("imported");
    let exchange = temp.join(structure).join("exchange");

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
    crible_ok(&[
        "export",
        "--data",
        path(&data),
        "--models",
        path(&sample.models),
        "--exchange",
        path(&exchange),
        "--index",
        structure,
    ]);

    let raw = read_file(&config);
    let index = IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("{} must be valid: {error}", config.display()));

    crible_ok(&[
        "index",
        "create",
        "--data",
        path(&imported_data),
        "--config",
        path(&config),
    ]);
    let exported = exchange.join("outbox").join(format!("{structure}-g1"));
    assert!(
        exported.join("manifest.json").is_file(),
        "the export must write {}",
        exported.display()
    );
    crible_ok(&[
        "attach",
        "--data",
        path(&imported_data),
        "--models",
        path(&sample.models),
        "--index",
        structure,
        path(&exported),
    ]);

    let source = GenerationStore::open(&data.join("indexes").join(structure), &index.schema)
        .unwrap_or_else(|error| panic!("the source index {structure} must open: {error}"));
    let imported = GenerationStore::open(
        &imported_data.join("indexes").join(structure),
        &index.schema,
    )
    .unwrap_or_else(|error| panic!("the imported index {structure} must open: {error}"));

    let source_state = source
        .state()
        .unwrap_or_else(|error| panic!("the state of {structure} must be readable: {error}"));
    let imported_state = imported.state().unwrap_or_else(|error| {
        panic!("the state of the imported {structure} must be readable: {error}")
    });
    assert_eq!(
        imported_state.live_passages, source_state.live_passages,
        "the imported {structure} must hold the same live passages as its source"
    );
    assert_eq!(
        imported_state.documents, source_state.documents,
        "the imported {structure} must hold the same documents as its source"
    );
    assert_eq!(
        imported_state.dead_passages, 0,
        "an export carries no tombstone, so the imported {structure} holds none"
    );

    let queries = legifacile_queries(sample, structure, &index, encoder);
    let before = search_all(&source, &queries);
    for (query, answer) in queries.iter().zip(&before) {
        assert_answer_is_not_vacuous(query, answer);
    }
    let after = search_all(&imported, &queries);
    for ((query, before), after) in queries.iter().zip(&before).zip(&after) {
        assert_same_results("the import", &query.label, before, after);
    }
    eprintln!(
        "INV-08: {structure}: {} queries compared, {} document(s) and {} live passage(s) exported \
         and imported",
        queries.len(),
        source_state.documents,
        source_state.live_passages
    );
    queries.len()
}

// --- The comparison cannot pass by accident ---

/// Unit tests of [`compare_results`], the only place that knows the tolerance.
/// They feed it answers an import must never produce and check that it rejects
/// them.
mod comparison_guards {
    use serde_json::json;

    use crible_search::dense::exact::Hit;

    use crate::common::compare_results;

    fn hit(key: &str, score: f32) -> Hit {
        let (doc_id, ord) = key.split_once('#').unwrap_or((key, "0"));
        Hit {
            doc_id: doc_id.to_owned(),
            ord: ord.parse().unwrap_or(0),
            key: key.to_owned(),
            score,
            fields: json!({}),
            text: String::new(),
        }
    }

    #[test]
    fn identical_answers_are_accepted() {
        let before = [hit("a#0", 0.9), hit("b#0", 0.5)];
        let after = [hit("a#0", 0.9), hit("b#0", 0.5)];
        assert!(
            compare_results(&before, &after).is_ok(),
            "identical answers must be accepted"
        );
    }

    #[test]
    fn a_moved_score_within_the_tolerance_is_accepted() {
        let before = [hit("a#0", 0.5)];
        let after = [hit("a#0", 0.5 + 5e-7)];
        assert!(
            compare_results(&before, &after).is_ok(),
            "a score gap under 1e-6 must be accepted"
        );
    }

    #[test]
    fn a_moved_score_above_the_tolerance_is_rejected() {
        let before = [hit("a#0", 0.5)];
        let after = [hit("a#0", 0.5 + 2e-6)];
        assert!(
            compare_results(&before, &after).is_err(),
            "a score gap above 1e-6 must be rejected"
        );
    }

    #[test]
    fn a_non_finite_score_is_rejected() {
        let before = [hit("a#0", f32::NAN)];
        let after = [hit("a#0", f32::NAN)];
        assert!(
            compare_results(&before, &after).is_err(),
            "two NaN scores are not a stable answer and must be rejected"
        );
        let before = [hit("a#0", 0.5)];
        let after = [hit("a#0", f32::INFINITY)];
        assert!(
            compare_results(&before, &after).is_err(),
            "an infinite score must be rejected"
        );
    }

    #[test]
    fn a_different_key_is_rejected() {
        let before = [hit("a#0", 0.9), hit("b#0", 0.5)];
        let after = [hit("a#0", 0.9), hit("c#0", 0.5)];
        assert!(
            compare_results(&before, &after).is_err(),
            "a different key at the same position must be rejected"
        );
    }

    #[test]
    fn a_reordered_pair_is_rejected() {
        let before = [hit("a#0", 0.9), hit("b#0", 0.5)];
        let after = [hit("b#0", 0.5), hit("a#0", 0.9)];
        assert!(
            compare_results(&before, &after).is_err(),
            "an exchange of two results must be rejected, even with the same scores"
        );
    }

    #[test]
    fn a_shorter_answer_is_rejected() {
        let before = [hit("a#0", 0.9), hit("b#0", 0.5)];
        let after = [hit("a#0", 0.9)];
        assert!(
            compare_results(&before, &after).is_err(),
            "an answer that lost a result must be rejected"
        );
    }

    #[test]
    fn a_lost_payload_is_rejected() {
        let mut with_fields = hit("a#0", 0.9);
        with_fields.fields = json!({ "categorie": "cat-00" });
        with_fields.text = "document: text".to_owned();
        let without_fields = hit("a#0", 0.9);
        assert!(
            compare_results(&[with_fields.clone()], &[without_fields]).is_err(),
            "an import that lost the fields of a result must be rejected"
        );
        let mut other_text = with_fields.clone();
        other_text.text = "document: other".to_owned();
        assert!(
            compare_results(&[with_fields], &[other_text]).is_err(),
            "an import that lost the text of a result must be rejected"
        );
    }
}
