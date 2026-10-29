// SPDX-License-Identifier: Apache-2.0

// Test fixtures may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixtures of this file build a batch and a schema.
#![allow(clippy::panic)]

//! `INV-09` with the lexical branch: a compaction and a rebuild of the lexical
//! index return the same BM25 results as the index built batch by batch.
//!
//! References: `docs/08-invariants-et-tests.md` (INV-09, "Compaction ⇒ mêmes
//! résultats avant et après"), the `RECONSTRUIRE-LEXICAL` task of
//! `docs/phases/P5-lexical-et-hybride.md`, `docs/02-architecture.md`
//! ("Suppressions, pierres tombales et compaction", step 3 "tantivy reconstruit
//! depuis le texte", and the note that a tombstone skews the BM25 statistics
//! until the segments are merged) and `docs/03-recherche.md` ("Branche lexicale
//! (tantivy)").
//!
//! # What the invariant means here
//!
//! The compaction renumbers the internal identifiers and writes a new epoch,
//! whose lexical index is rebuilt from the texts SQLite stores. The rebuild job
//! of `RECONSTRUIRE-LEXICAL` does the same over an epoch, in place. Nothing a
//! caller observes may change: the same queries must return the same public keys
//! in the same order.
//!
//! The scores are a second matter, and `docs/02` says why: tantivy counts the
//! documents it has deleted in its statistics — total number of documents and
//! average field length — as long as its segments are not merged. A tombstoned
//! passage therefore moves every BM25 score of the epoch, slightly, until the
//! compaction writes an index that never held it. The tests are written the way
//! the invariant can be true:
//!
//! - **without dead passage**, the compaction and the rebuild copy everything, so
//!   the statistics are the same on both sides: keys, order **and** scores are
//!   equal within 1e-6;
//! - **with dead passages**, keys and order are still equal, because the same
//!   live passages carry the same terms; the scores are compared with a
//!   **reference index** built by `attach` from the live passages alone, which is
//!   the "une fois les passages morts retirés" of the task sheet. That comparison
//!   is exact within 1e-6, both indexes holding the same live passages and no
//!   tombstone at all.
//!
//! # How the queries run
//!
//! The BM25 branch of the engine (`RECHERCHER-BM25-DANS-LE-POOL`) does not exist
//! yet, so the tests run the query tantivy would get: the text is analysed by
//! the analysis chain the epoch registered under the schema's tokenizer name,
//! and the terms form a `Should` bag (`docs/03`, "Requête"), scored by tantivy's
//! BM25 through [`TopDocs`]. The iid read from the `iid` fast field is turned
//! back into the public `<doc_id>#<ord>` key through the reference state, so no
//! internal identifier is compared across the compaction: identifiers are
//! renumbered, keys are not (INV-01).
//!
//! ```text
//! cargo test -p crible-store --test inv_09_lexical_compaction
//! ```

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow::array::{FixedSizeBinaryArray, Int64Array, StringArray, UInt32Array};
use arrow::datatypes::{DataType, Field, Schema as ArrowSchema, SchemaRef};
use crible_core::config::IndexConfig;
use crible_core::ids::{Generation, Iid};
use crible_core::model::ModelConfig;
use crible_store::attach::attach;
use crible_store::compact::compact;
use crible_store::generation::GenerationStore;
use crible_store::lexical::LexicalStatus;
use crible_store::records::ReadPool;
use half::f16;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;

/// Dimensionality of the fixture vectors.
const DIMS: usize = 4;

/// Number of results compared per query.
///
/// Larger than the eight live documents of the fixture, so that the comparison
/// covers the whole ranking and not only its head.
const LIMIT: usize = 20;

/// One document of a fixture batch: its external id, the hash of its content,
/// the text of its single passage and the vector of that passage.
///
/// The text is the encoder input of both branches (`docs/03`): the lexical index
/// stores exactly it. The hash is what makes a batch replace a document — the
/// same `doc_id` with another `doc_hash` tombstones the old passage (`docs/02`,
/// "Suppressions, pierres tombales et compaction").
#[derive(Debug, Clone, Copy)]
struct FixtureDoc {
    doc_id: &'static str,
    hash: &'static str,
    text: &'static str,
    vector: [f32; 4],
}

/// First batch: six documents, one passage each, no tombstone.
const L1: [FixtureDoc; 6] = [
    FixtureDoc {
        doc_id: "civil-1240",
        hash: "blake3:civil-1240-v1",
        text: "responsabilite du fait personnel article 1240 code civil",
        vector: [1.0, 0.0, 0.0, 0.0],
    },
    FixtureDoc {
        doc_id: "civil-1241",
        hash: "blake3:civil-1241-v1",
        text: "responsabilite du fait des choses article 1241 code civil garde",
        vector: [0.0, 1.0, 0.0, 0.0],
    },
    FixtureDoc {
        doc_id: "civil-1101",
        hash: "blake3:civil-1101-v1",
        text: "contrat convention loi parties article 1101 code civil",
        vector: [0.0, 0.0, 1.0, 0.0],
    },
    FixtureDoc {
        doc_id: "civil-1134",
        hash: "blake3:civil-1134-v1",
        text: "contrat force obligatoire article 1134 code",
        vector: [0.0, 0.0, 0.0, 1.0],
    },
    FixtureDoc {
        doc_id: "civil-544",
        hash: "blake3:civil-544-v1",
        // `paisiblement` keeps the text longer than the one of `civil-1582`:
        // both hold `propriété` once, so a tie in field length would let the
        // document address — which the compaction renumbers — decide their
        // order, and the test could not tell a real change of ranking from the
        // renumbering.
        text: "propriete droit de jouir paisiblement article 544 code civil",
        vector: [0.6, 0.8, 0.0, 0.0],
    },
    FixtureDoc {
        doc_id: "civil-1582",
        hash: "blake3:civil-1582-v1",
        text: "vente convention transfert propriete article 1582 code",
        vector: [0.8, 0.6, 0.0, 0.0],
    },
];

/// Second batch: two further documents, no tombstone.
const L2: [FixtureDoc; 2] = [
    FixtureDoc {
        doc_id: "civil-1242",
        hash: "blake3:civil-1242-v1",
        text: "responsabilite du fait des animaux article 1242 code civil garde elevage",
        vector: [0.6, 0.0, 0.8, 0.0],
    },
    FixtureDoc {
        doc_id: "civil-1102",
        hash: "blake3:civil-1102-v1",
        text: "contrat offre acceptation article 1102 code civil",
        vector: [0.0, 0.6, 0.0, 0.8],
    },
];

/// Third batch: two replacements, which tombstone the two passages of `L1`.
const L3: [FixtureDoc; 2] = [
    FixtureDoc {
        doc_id: "civil-1240",
        hash: "blake3:civil-1240-v2",
        text: "responsabilite du fait personnel article 1240 responsabilite code civil",
        vector: [1.0, 0.0, 0.0, 0.0],
    },
    FixtureDoc {
        doc_id: "civil-544",
        hash: "blake3:civil-544-v2",
        text: "propriete droit de jouir et disposer article 544 code civil",
        vector: [0.6, 0.8, 0.0, 0.0],
    },
];

/// The eight live documents of the fixture after `L1`, `L2` and `L3`: the
/// corpus a reference index is built from once the dead passages are removed.
const LIVE: [FixtureDoc; 8] = [L1[1], L1[2], L1[3], L1[5], L2[0], L2[1], L3[0], L3[1]];

/// Queries compared before and after a compaction or a rebuild.
///
/// Each query is a single term once analysed, and the documents holding it have
/// pairwise distinct lengths, so the ranking is decided by the term frequency
/// and the field length alone: it does not depend on the statistics the dead
/// passages move (`docs/02`), and no two documents tie — a tie would be broken by
/// a document address, which the compaction renumbers.
const QUERIES: [&str; 6] = [
    "responsabilité",
    "contrat",
    "propriété",
    "vente",
    "animaux",
    "obligatoire",
];

/// One result of a lexical query: its public key and its BM25 score.
#[derive(Debug, Clone)]
struct Hit {
    key: String,
    score: f32,
}

/// Schema of the `documents.parquet` of a fixture batch.
fn document_schema() -> SchemaRef {
    Arc::new(ArrowSchema::new(vec![
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("doc_hash", DataType::Utf8, false),
        Field::new("title", DataType::Utf8, true),
    ]))
}

/// Schema of the `passages.parquet` of a fixture batch.
fn passage_schema() -> SchemaRef {
    Arc::new(ArrowSchema::new(vec![
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("ord", DataType::UInt32, false),
        Field::new("text", DataType::Utf8, false),
        Field::new("part", DataType::Int64, true),
        Field::new("vector", DataType::FixedSizeBinary(DIMS as i32 * 2), false),
    ]))
}

/// The `fp16` little-endian bytes of `vector`.
fn fp16(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| f16::from_f32(*value).to_bits().to_le_bytes())
        .collect()
}

/// Writes a batch of `documents`, one passage each, under `<root>/<batch_id>/`.
///
/// The fixture is the one of `tests/common/mod.rs`, with the document hash taken
/// from the fixture instead of derived from the identifier: that is what lets
/// `L3` replace the documents of `L1` and leave the tombstones the compaction
/// has to remove (`docs/02`, "Suppressions, pierres tombales et compaction").
fn write_batch(root: &Path, batch_id: &str, documents: &[FixtureDoc]) -> PathBuf {
    let dir = root.join(batch_id);
    fs::create_dir_all(&dir)
        .unwrap_or_else(|error| panic!("{} must be creatable: {error}", dir.display()));

    let ids: StringArray = documents.iter().map(|doc| Some(doc.doc_id)).collect();
    let hashes: StringArray = documents.iter().map(|doc| Some(doc.hash)).collect();
    let titles: StringArray = documents.iter().map(|doc| Some(doc.doc_id)).collect();
    common::write_parquet(
        &dir.join("documents.parquet"),
        &document_schema(),
        vec![Arc::new(ids), Arc::new(hashes), Arc::new(titles)],
    );

    let passage_ids: StringArray = documents.iter().map(|doc| Some(doc.doc_id)).collect();
    let ords = UInt32Array::from(vec![0_u32; documents.len()]);
    let texts: StringArray = documents.iter().map(|doc| Some(doc.text)).collect();
    let parts = Int64Array::from(
        (0..documents.len())
            .map(|position| Some(position as i64))
            .collect::<Vec<_>>(),
    );
    let vectors =
        FixedSizeBinaryArray::try_from_iter(documents.iter().map(|doc| fp16(&doc.vector)))
            .unwrap_or_else(|error| panic!("the fixture vectors must share one width: {error}"));
    common::write_parquet(
        &dir.join("passages.parquet"),
        &passage_schema(),
        vec![
            Arc::new(passage_ids),
            Arc::new(ords),
            Arc::new(texts),
            Arc::new(parts),
            Arc::new(vectors),
        ],
    );

    write_manifest(&dir, batch_id, documents.len());
    dir
}

/// Writes `manifest.json` of a batch of `passages` documents, one passage each.
fn write_manifest(dir: &Path, batch_id: &str, passages: usize) {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", dir.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("a batch entry must be readable: {error}"))
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with(".parquet"))
        .collect();
    names.sort();

    let mut files = serde_json::Map::new();
    for name in &names {
        let bytes = fs::read(dir.join(name))
            .unwrap_or_else(|error| panic!("the batch file {name} must be readable: {error}"));
        files.insert(
            name.clone(),
            serde_json::Value::String(format!("blake3:{}", blake3::hash(&bytes).to_hex())),
        );
    }

    let documents = u64::try_from(passages).unwrap_or_else(|_| {
        panic!("the fixture batch holds at most u64::MAX documents, got {passages}")
    });
    let manifest = serde_json::json!({
        "format": "crible-batch",
        "format_version": 1,
        "batch_id": batch_id,
        "index": "test",
        "mode": "upsert",
        "created_at": "2026-01-01T00:00:00Z",
        "producer": { "name": "test", "version": "1", "host": "test" },
        "model": { "dims": DIMS, "normalize": true, "vector_dtype": "fp16" },
        "chunker": { "name": "test", "version": "1", "params": {} },
        "template": { "name": "test", "version": "1" },
        "counts": { "documents": documents, "passages": documents, "deletions": 0 },
        "files": files,
    });
    fs::write(
        dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)
            .unwrap_or_else(|error| panic!("the manifest must be serializable: {error}")),
    )
    .unwrap_or_else(|error| panic!("manifest.json must be written: {error}"));
}

/// External model of the fixture batches: four dimensions, normalized vectors.
///
/// `external` is the kind whose vectors the batch provides, so the attach needs
/// no ONNX Runtime and no model directory (`docs/04`).
fn fixture_model() -> ModelConfig {
    ModelConfig::from_json_str(
        r#"{ "name": "lexical-rebuild-external", "kind": "external", "dims": 4, "normalize": true }"#,
    )
    .unwrap_or_else(|error| panic!("the fixture model configuration must be valid: {error}"))
}

/// Configuration of the fixture index, lexical branch enabled in French.
fn config_json() -> String {
    common::test_config_with_lexical("test", &common::fixture_schema(), true)
}

/// Creates the fixture index, with a lexical branch and no passage.
fn create_index(temp: &Path) -> (PathBuf, GenerationStore) {
    let index_dir = temp.join("indexes").join("test");
    let store =
        common::create_store_with_config(&index_dir, &common::fixture_schema(), &config_json());
    (index_dir, store)
}

/// Configuration of the fixture index, read from its `index.json`.
fn fixture_config(index_dir: &Path) -> IndexConfig {
    let raw = fs::read_to_string(index_dir.join("index.json")).unwrap_or_else(|error| {
        panic!(
            "the index configuration of {} must be readable: {error}",
            index_dir.display()
        )
    });
    IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("the index configuration must be valid: {error}"))
}

/// Attaches the batch of `batch` to the index, with the configuration and the
/// model of the fixture.
fn attach_batch(store: &mut GenerationStore, index_dir: &Path, batch: &Path) {
    let config = fixture_config(index_dir);
    attach(store, batch, &config, &fixture_model(), None, None).unwrap_or_else(|error| {
        panic!(
            "the attach of {} must succeed, got {}: {}",
            batch.display(),
            error.code().as_str(),
            error.message()
        )
    });
}

/// Opens a second index holding `documents` alone, built by `attach` the way the
/// engine builds an index batch by batch.
///
/// This is the reference of the task sheet, "l'index construit au fil des lots,
/// une fois les passages morts retirés": the same texts, no tombstone at all, so
/// its statistics are the ones a rebuilt index must reach.
fn reference_index(temp: &Path, documents: &[FixtureDoc]) -> GenerationStore {
    let index_dir = temp.join("reference").join("indexes").join("test");
    let mut store =
        common::create_store_with_config(&index_dir, &common::fixture_schema(), &config_json());
    let batch = write_batch(&temp.join("reference-batches"), "live", documents);
    attach_batch(&mut store, &index_dir, &batch);
    store
}

/// Runs the BM25 query `query` on the lexical index of the snapshot `store`
/// publishes, and returns its `limit` best passages by public key.
///
/// The query is the one `RECHERCHER-BM25-DANS-LE-POOL` will build: the analysis
/// chain of the epoch turns the text into terms, which a `Should` bag scores with
/// tantivy's BM25 (`docs/03`, "Requête"). The pool restriction is not applied
/// here: the lexical index of an up-to-date epoch holds the live passages only,
/// so the whole index is the pool.
fn lexical_hits(store: &GenerationStore, query: &str, limit: usize) -> Vec<Hit> {
    let snapshot = store.snapshot();
    let status = snapshot.lexical_status();
    let Some(lexical) = snapshot.lexical() else {
        panic!(
            "the lexical branch must be available on generation {}, got {status:?}: a \
             compaction or a rebuild must publish the index of its epoch (RECONSTRUIRE-LEXICAL)",
            snapshot.generation().get()
        );
    };
    let searcher = lexical.searcher();
    let index = searcher.index();
    let schema = index.schema();
    let text_field = schema.get_field("text").unwrap_or_else(|error| {
        panic!("the lexical schema of the epoch must hold the field 'text': {error}")
    });
    let parser = QueryParser::new(schema, vec![text_field], index.tokenizers().clone());
    let parsed = parser
        .parse_query(query)
        .unwrap_or_else(|error| panic!("the query {query:?} must be analysed: {error}"));
    let top = searcher
        .search(&parsed, &TopDocs::with_limit(limit).order_by_score())
        .unwrap_or_else(|error| panic!("the lexical search of {query:?} must run: {error}"));

    let pool = ReadPool::new(&store.records_path(), 2)
        .unwrap_or_else(|error| panic!("the read pool of the epoch must open: {error}"));
    let mut hits = Vec::with_capacity(top.len());
    for (score, address) in top {
        let reader = searcher.segment_reader(address.segment_ord);
        let column = reader.fast_fields().u64("iid").unwrap_or_else(|error| {
            panic!("the lexical segment must hold the 'iid' fast field: {error}")
        });
        let value = column.first(address.doc_id).unwrap_or_else(|| {
            panic!(
                "the lexical index returned a passage at document {} without its iid fast field",
                address.doc_id
            )
        });
        let iid = u32::try_from(value).map(Iid::new).unwrap_or_else(|_| {
            panic!("the iid fast field value {value} is not a Crible internal identifier")
        });
        let key = pool
            .read_passage_key(iid)
            .unwrap_or_else(|error| panic!("the key of iid {} must read: {error}", iid.get()))
            .unwrap_or_else(|| {
                panic!(
                    "the lexical index returned iid {}, which the reference state of the epoch \
                     does not hold",
                    iid.get()
                )
            });
        hits.push(Hit {
            key: key.to_string(),
            score,
        });
    }
    hits
}

/// Runs every query of [`QUERIES`] on the lexical index `store` publishes.
fn lexical_answers(store: &GenerationStore, queries: &[&str]) -> Vec<Vec<Hit>> {
    queries
        .iter()
        .map(|query| {
            let hits = lexical_hits(store, query, LIMIT);
            assert!(
                !hits.is_empty(),
                "the query {query:?} must find at least one passage: a comparison of empty \
                 answers proves nothing"
            );
            hits
        })
        .collect()
}

/// Asserts that two rankings hold the same public keys in the same order.
///
/// This is the whole of INV-09: the keys and their order, never the internal
/// identifiers, which the compaction renumbers.
fn assert_same_ranking(context: &str, query: &str, expected: &[Hit], actual: &[Hit]) {
    let expected_keys: Vec<&str> = expected.iter().map(|hit| hit.key.as_str()).collect();
    let actual_keys: Vec<&str> = actual.iter().map(|hit| hit.key.as_str()).collect();
    assert_eq!(
        expected_keys, actual_keys,
        "INV-09: {context} changed the passages the query {query:?} returns, or their order"
    );
}

/// Asserts that two answers hold the same keys in the same order, with the same
/// scores within 1e-6.
///
/// Only a comparison of two indexes holding the same live passages and no
/// tombstone may ask for this: `docs/02` records that tantivy keeps counting a
/// deleted document in its statistics until its segments are merged, which moves
/// every score of the epoch.
fn assert_same_results(context: &str, query: &str, expected: &[Hit], actual: &[Hit]) {
    assert_same_ranking(context, query, expected, actual);
    for (expected, actual) in expected.iter().zip(actual) {
        assert!(
            (expected.score - actual.score).abs() <= 1e-6,
            "INV-09: {context} changed the BM25 score of {} for the query {query:?}, from {} to \
             {}",
            expected.key,
            expected.score,
            actual.score
        );
    }
}

/// `INV-09` without tombstone: a compaction copies every passage, so the
/// statistics of both lexical indexes are the same and the results are equal,
/// scores within 1e-6.
#[test]
fn inv_09_lexical_results_are_identical_after_compaction_without_dead_passages() {
    let temp = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
    let (index_dir, mut store) = create_index(temp.path());
    let batches = temp.path().join("batches");
    attach_batch(&mut store, &index_dir, &write_batch(&batches, "l1", &L1));
    attach_batch(&mut store, &index_dir, &write_batch(&batches, "l2", &L2));

    let before = store
        .state()
        .unwrap_or_else(|error| panic!("the state of the index must be readable: {error}"));
    assert_eq!(
        before.dead_passages, 0,
        "this test compares scores exactly, which is sound only when the epoch holds no \
         tombstone (docs/02: a deleted document still counts in the BM25 statistics until the \
         segments are merged)"
    );
    assert_eq!(
        before.live_passages, 8,
        "the two batches add eight passages"
    );

    let answers_before = lexical_answers(&store, &QUERIES);

    let report = compact(&mut store, &fixture_config(&index_dir)).unwrap_or_else(|error| {
        panic!(
            "the compaction must succeed, got {}: {}",
            error.code().as_str(),
            error.message()
        )
    });
    assert_eq!(report.removed_passages, 0);
    assert_eq!(report.live_passages, before.live_passages);

    let after = store.state().unwrap_or_else(|error| {
        panic!("the state of the compacted index must be readable: {error}")
    });
    assert_eq!(
        after.generation,
        Generation::new(before.generation.get() + 1)
    );
    assert_eq!(after.dead_passages, 0);
    assert!(
        matches!(
            store.snapshot().lexical_status(),
            LexicalStatus::Ready { .. }
        ),
        "the compaction must publish the lexical index of the new epoch, got {:?}",
        store.snapshot().lexical_status()
    );

    let answers_after = lexical_answers(&store, &QUERIES);
    for ((query, expected), actual) in QUERIES.iter().zip(&answers_before).zip(&answers_after) {
        assert_same_results("the compaction", query, expected, actual);
    }
}

/// `INV-09` with tombstones: the compaction still returns the same passages in
/// the same order.
///
/// The scores are not asserted here; the next test compares them with a reference
/// index built from the live passages alone, which is the only index they can be
/// equal to.
#[test]
fn inv_09_lexical_rankings_are_identical_after_compaction_with_dead_passages() {
    let temp = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
    let (index_dir, mut store) = create_index(temp.path());
    let batches = temp.path().join("batches");
    attach_batch(&mut store, &index_dir, &write_batch(&batches, "l1", &L1));
    attach_batch(&mut store, &index_dir, &write_batch(&batches, "l2", &L2));
    attach_batch(&mut store, &index_dir, &write_batch(&batches, "l3", &L3));

    let before = store
        .state()
        .unwrap_or_else(|error| panic!("the state of the index must be readable: {error}"));
    assert_eq!(
        before.dead_passages, 2,
        "the replacement batch must tombstone the two passages it replaces, otherwise the \
         compaction proves nothing"
    );
    assert_eq!(before.live_passages, 8);

    let answers_before = lexical_answers(&store, &QUERIES);

    let report = compact(&mut store, &fixture_config(&index_dir)).unwrap_or_else(|error| {
        panic!(
            "the compaction must succeed, got {}: {}",
            error.code().as_str(),
            error.message()
        )
    });
    assert_eq!(
        report.removed_passages, before.dead_passages,
        "the compaction must remove exactly the tombstones of the replacement batch"
    );
    assert_eq!(report.live_passages, before.live_passages);

    let after = store.state().unwrap_or_else(|error| {
        panic!("the state of the compacted index must be readable: {error}")
    });
    assert_eq!(after.dead_passages, 0);
    assert!(
        matches!(
            store.snapshot().lexical_status(),
            LexicalStatus::Ready { .. }
        ),
        "the compaction must publish the lexical index of the new epoch, got {:?}",
        store.snapshot().lexical_status()
    );

    let answers_after = lexical_answers(&store, &QUERIES);
    let mut largest_drift = 0.0_f32;
    for ((query, expected), actual) in QUERIES.iter().zip(&answers_before).zip(&answers_after) {
        assert_same_ranking("the compaction", query, expected, actual);
        for (expected, actual) in expected.iter().zip(actual) {
            if expected.key == actual.key {
                largest_drift = largest_drift.max((expected.score - actual.score).abs());
            }
        }
    }
    eprintln!(
        "INV-09 (lexical, dead passages): {} queries compared, {} tombstone(s) removed, largest \
         score drift attributed to the BM25 statistics {largest_drift:e}",
        QUERIES.len(),
        report.removed_passages
    );
}

/// Acceptance criterion of `RECONSTRUIRE-LEXICAL`, compaction path: the lexical
/// index a compaction rebuilds scores like an index built from the live passages
/// alone, which is the only one it may be equal to once the dead passages are
/// removed.
#[test]
fn inv_09_the_compacted_lexical_index_scores_like_an_index_built_from_the_live_passages() {
    let temp = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
    let (index_dir, mut store) = create_index(temp.path());
    let batches = temp.path().join("batches");
    attach_batch(&mut store, &index_dir, &write_batch(&batches, "l1", &L1));
    attach_batch(&mut store, &index_dir, &write_batch(&batches, "l2", &L2));
    attach_batch(&mut store, &index_dir, &write_batch(&batches, "l3", &L3));

    let before = store
        .state()
        .unwrap_or_else(|error| panic!("the state of the index must be readable: {error}"));
    assert!(
        before.dead_passages > 0,
        "the replacement batch must tombstone passages, otherwise the test compares two \
         indexes that differ by nothing: {before:?}"
    );

    compact(&mut store, &fixture_config(&index_dir)).unwrap_or_else(|error| {
        panic!(
            "the compaction must succeed, got {}: {}",
            error.code().as_str(),
            error.message()
        )
    });
    let answers = lexical_answers(&store, &QUERIES);

    let reference = reference_index(temp.path(), &LIVE);
    let expected = lexical_answers(&reference, &QUERIES);
    for ((query, expected), actual) in QUERIES.iter().zip(&expected).zip(&answers) {
        assert_same_results(
            "the lexical index rebuilt by the compaction, against an index built from the live \
             passages alone",
            query,
            expected,
            actual,
        );
    }
}

/// Acceptance criterion of `RECONSTRUIRE-LEXICAL`, rebuild path: an epoch whose
/// lexical index is rebuilt from `records.sqlite` returns the same BM25 results
/// as the index the attaches built, when no passage is dead.
///
/// The rebuild is the one a write open runs when the epoch has no lexical index
/// (`docs/02`, "Reprise au démarrage"; ADR-0088), the path the rebuild job of the
/// task uses.
#[test]
fn inv_09_a_rebuilt_lexical_index_returns_the_same_results_as_the_index_it_replaces() {
    let temp = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
    let (index_dir, mut store) = create_index(temp.path());
    let batches = temp.path().join("batches");
    attach_batch(&mut store, &index_dir, &write_batch(&batches, "l1", &L1));
    attach_batch(&mut store, &index_dir, &write_batch(&batches, "l2", &L2));

    let generation = store.generation();
    let before = store
        .state()
        .unwrap_or_else(|error| panic!("the state of the index must be readable: {error}"));
    assert_eq!(before.dead_passages, 0);

    let answers_before = lexical_answers(&store, &QUERIES);

    // The lexical directory is derived: removing it is the state a write open
    // repairs by rebuilding it from the texts of `records.sqlite` (ADR-0088).
    drop(store);
    let epoch_dir = common::epoch_dir(&index_dir);
    fs::remove_dir_all(epoch_dir.join("lexical")).unwrap_or_else(|error| {
        panic!(
            "the lexical directory of {} must be removable: {error}",
            epoch_dir.display()
        )
    });
    let store = GenerationStore::open(&index_dir, &common::fixture_schema())
        .unwrap_or_else(|error| panic!("the index must reopen and rebuild: {error}"));

    assert_eq!(
        store.generation(),
        generation,
        "rebuilding the lexical index creates no generation (ADR-0088)"
    );
    assert_eq!(
        store.snapshot().lexical_status(),
        LexicalStatus::Ready {
            committed: generation
        },
        "the rebuild must publish the lexical index of the committed generation"
    );

    let answers_after = lexical_answers(&store, &QUERIES);
    for ((query, expected), actual) in QUERIES.iter().zip(&answers_before).zip(&answers_after) {
        assert_same_results("the rebuild of the lexical index", query, expected, actual);
    }
}

/// Acceptance criterion of `RECONSTRUIRE-LEXICAL`, rebuild path with tombstones:
/// the rebuilt index scores like an index built from the live passages alone.
#[test]
fn inv_09_a_rebuilt_lexical_index_over_dead_passages_scores_like_an_index_built_from_the_live_passages()
 {
    let temp = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
    let (index_dir, mut store) = create_index(temp.path());
    let batches = temp.path().join("batches");
    attach_batch(&mut store, &index_dir, &write_batch(&batches, "l1", &L1));
    attach_batch(&mut store, &index_dir, &write_batch(&batches, "l2", &L2));
    attach_batch(&mut store, &index_dir, &write_batch(&batches, "l3", &L3));

    let before = store
        .state()
        .unwrap_or_else(|error| panic!("the state of the index must be readable: {error}"));
    assert_eq!(before.dead_passages, 2, "the fixture must hold tombstones");

    drop(store);
    let epoch_dir = common::epoch_dir(&index_dir);
    fs::remove_dir_all(epoch_dir.join("lexical")).unwrap_or_else(|error| {
        panic!(
            "the lexical directory of {} must be removable: {error}",
            epoch_dir.display()
        )
    });
    let store = GenerationStore::open(&index_dir, &common::fixture_schema())
        .unwrap_or_else(|error| panic!("the index must reopen and rebuild: {error}"));

    let answers = lexical_answers(&store, &QUERIES);
    let reference = reference_index(temp.path(), &LIVE);
    let expected = lexical_answers(&reference, &QUERIES);
    for ((query, expected), actual) in QUERIES.iter().zip(&expected).zip(&answers) {
        assert_same_results(
            "the lexical index rebuilt from records.sqlite, against an index built from the \
             live passages alone",
            query,
            expected,
            actual,
        );
    }
}
