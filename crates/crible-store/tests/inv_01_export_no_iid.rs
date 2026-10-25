// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! INV-01 and the acceptance criteria of `EXPORTER-LOT`: an exported batch
//! carries no internal identifier.
//!
//! References: `docs/phases/P3-cycle-de-vie.md` (task `EXPORTER-LOT`),
//! `docs/04-donnees-et-ingestion.md` ("Export, instantanés, restauration"),
//! `docs/format-lot-v1.md`, `docs/08-invariants-et-tests.md` (INV-01),
//! ADR-0012 (public batch format).
//!
//! # What each test pins
//!
//! | Test | Criterion or invariant |
//! |---|---|
//! | `inv_01_exported_batch_names_no_iid` | INV-01: no `iid` column in the Parquet files, no `iid` key in `manifest.json` |
//! | `inv_01_exported_identifiers_are_doc_ids_and_ords` | INV-01: the exported identifiers are `doc_id` and `ord`, never an iid standing in for one |
//! | `exported_batch_passes_batch_reader_validation` | "le lot exporté passe la validation de `LIRE-LOT-PARQUET`" |
//! | `export_writes_a_replace_all_batch_under_outbox_index_generation` | `replace_all` batch under `exchange/outbox/<index>-g<G>/` |
//!
//! # Assumed API
//!
//! `crible_store::export` carries the API below, shaped like
//! [`crible_store::compact::compact`] and [`crible_store::patch::apply`].
//!
//! ```rust,ignore
//! // crible_store::export
//!
//! pub struct ExportReport {
//!     /// Generation the export read: the generation of the snapshot it held.
//!     pub generation: Generation,
//!     /// Batch directory actually written:
//!     /// `<exchange>/outbox/<index>-g<G>/`.
//!     pub directory: PathBuf,
//!     /// Document rows written.
//!     pub documents: u64,
//!     /// Passage rows written.
//!     pub passages: u64,
//! }
//!
//! /// Writes a `replace_all` batch of the live data of the generation
//! /// `store` currently publishes, into
//! /// `<exchange>/outbox/<index_config.name>-g<G>/`.
//! ///
//! /// The store is taken by shared reference: the export only reads, through
//! /// the published snapshot and a read transaction, so the writer is not
//! /// blocked (`docs/04`, "Export, instantanés, restauration").
//! ///
//! /// `exchange` is the `[paths] exchange` root of `crible.toml`
//! /// (`docs/07-stack-et-deploiement.md`); the export creates `outbox/` under
//! /// it when it does not exist yet.
//! ///
//! /// `model_config` fills the model member of the manifest, which
//! /// `docs/format-lot-v1.md` requires and `IndexConfig` does not carry
//! /// (`dims`, `normalize`); it is the same configuration the exported batch is
//! /// checked against below.
//! pub fn export(
//!     store: &GenerationStore,
//!     index_config: &IndexConfig,
//!     model_config: &ModelConfig,
//!     exchange: &Path,
//! ) -> Result<ExportReport, CribleError>;
//! ```
//!
//! The batch it writes must be readable by
//! [`BatchReader`](crible_store::batch_reader::BatchReader), which is the
//! reader of `LIRE-LOT-PARQUET`: this is what
//! `exported_batch_passes_batch_reader_validation` checks.
//!
//! # The fixture
//!
//! The six passages carry iids `0..=5` and the store is built so that no iid
//! coincides with the business pairs it must not leak:
//!
//! | iid | `doc_id` | `ord` |
//! |---|---|---|
//! | 0 | `doc-a` | 0 |
//! | 1 | `doc-c` | 1 |
//! | 2 | `doc-c` | 0 |
//! | 3 | `doc-c` | 2 |
//! | 4 | `doc-b` | 0 |
//! | 5 | `doc-a` | 1 |
//!
//! So an export that wrote the iid in the `ord` column, or the iid in the
//! `doc_id` column, or that emitted the rows in iid order with `ord = iid`,
//! fails `inv_01_exported_identifiers_are_doc_ids_and_ords`. The text of each
//! passage is `"<doc_id>#<ord>"`, so a text replaced by an iid is caught too.

mod common;

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use arrow::array::{Array, StringArray, UInt32Array};
use crible_core::batch::BatchMode;
use crible_core::config::IndexConfig;
use crible_core::model::ModelConfig;
use crible_store::batch_reader::BatchReader;
use crible_store::export::{ExportReport, export};
use crible_store::generation::GenerationStore;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::Value;

/// Name of the fixture index: the export directory is `<name>-g<G>`.
const INDEX: &str = "test";

/// Generation the fixture publishes before exporting.
const GENERATION: u64 = 1;

/// The six exported rows, sorted by `doc_id` then `ord`: exactly what a batch
/// of the live data holds, whatever the iids are.
const EXPECTED_ROWS: [(&str, u32); 6] = [
    ("doc-a", 0),
    ("doc-a", 1),
    ("doc-b", 0),
    ("doc-c", 0),
    ("doc-c", 1),
    ("doc-c", 2),
];

/// Temporary index and exchange roots, the store at generation 1, and the
/// configuration the export takes.
struct Fixture {
    /// Kept alive for the whole test: it owns both roots.
    _temp: tempfile::TempDir,
    /// `[paths] exchange` root: the export must write `outbox/` under it.
    exchange: PathBuf,
    /// Store holding the six passages of the module comment.
    store: GenerationStore,
    /// Configuration of the index, as `crible index create` wrote it.
    index_config: IndexConfig,
    /// Resolved configuration of the fixture model (`external`, 4 dims).
    model_config: ModelConfig,
}

impl Fixture {
    /// Directory the export is expected to create.
    fn expected_directory(&self) -> PathBuf {
        self.exchange
            .join("outbox")
            .join(format!("{INDEX}-g{GENERATION}"))
    }
}

/// Builds the fixture: a store at generation 1, its six passages split over two
/// segments, and the paths an export needs.
///
/// `seg-a` covers iids `0..=3` and `seg-b` covers iids `4..=5`, the contiguous
/// ranges a segment holds; the document order and the `ord` order deliberately
/// differ from the iid order, as the module comment describes. Every vector is
/// a unit vector, as the fixture model declares `normalize: true`.
fn fixture() -> Fixture {
    let (temp, index_dir) = common::temp_index();
    let exchange = temp.path().join("exchange");
    let mut store = common::create_store(&index_dir);
    let segment_a = common::write_segment(
        &index_dir,
        "seg-a",
        0,
        GENERATION,
        &[
            &[1.0, 0.0, 0.0, 0.0],
            &[0.0, 1.0, 0.0, 0.0],
            &[0.0, 0.0, 1.0, 0.0],
            &[0.0, 0.0, 0.0, 1.0],
        ],
    );
    let segment_b = common::write_segment(
        &index_dir,
        "seg-b",
        4,
        GENERATION,
        &[&[1.0, 0.0, 0.0, 0.0], &[0.0, 1.0, 0.0, 0.0]],
    );
    common::commit(
        &mut store,
        common::change(
            vec![segment_a, segment_b],
            vec![
                common::document("doc-a", GENERATION),
                common::document("doc-b", GENERATION),
                common::document("doc-c", GENERATION),
            ],
            vec![
                common::passage(0, "doc-a", 0, "seg-a", GENERATION),
                common::passage(1, "doc-c", 1, "seg-a", GENERATION),
                common::passage(2, "doc-c", 0, "seg-a", GENERATION),
                common::passage(3, "doc-c", 2, "seg-a", GENERATION),
                common::passage(4, "doc-b", 0, "seg-b", GENERATION),
                common::passage(5, "doc-a", 1, "seg-b", GENERATION),
            ],
            Vec::new(),
        ),
    );
    Fixture {
        exchange,
        store,
        index_config: index_config(),
        model_config: model_config(),
        _temp: temp,
    }
}

/// Index configuration written by [`common::create_store`].
fn index_config() -> IndexConfig {
    IndexConfig::from_json_str(&common::test_config(INDEX, &common::fixture_schema()))
        .unwrap_or_else(|error| panic!("the fixture index configuration must be valid: {error}"))
}

/// Configuration resolved for the fixture model.
fn model_config() -> ModelConfig {
    ModelConfig::from_json_str(r#"{"name":"m","kind":"external","dims":4,"normalize":true}"#)
        .unwrap_or_else(|error| panic!("the fixture model configuration must be valid: {error}"))
}

/// Runs the export of the fixture and returns the report.
fn run_export(fixture: &Fixture) -> ExportReport {
    export(
        &fixture.store,
        &fixture.index_config,
        &fixture.model_config,
        &fixture.exchange,
    )
    .unwrap_or_else(|error| panic!("the export of generation {GENERATION} must succeed: {error}"))
}

/// Every Parquet file of a batch directory, sorted by name.
fn parquet_files(directory: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(directory)
        .unwrap_or_else(|error| {
            panic!(
                "the exported batch directory {} must be listable: {error}",
                directory.display()
            )
        })
        .map(|entry| {
            entry
                .unwrap_or_else(|error| {
                    panic!("cannot read an entry of {}: {error}", directory.display())
                })
                .path()
        })
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "parquet")
        })
        .collect();
    files.sort_unstable();
    files
}

/// Arrow column names of one Parquet file, read from its footer only.
fn column_names(path: &Path) -> Vec<String> {
    let file = File::open(path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap_or_else(|error| {
        panic!(
            "{} must be a readable Parquet file: {error}",
            path.display()
        )
    });
    builder
        .schema()
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect()
}

/// Every object key of a JSON value, recursively.
fn collect_keys(value: &Value, keys: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, item) in map {
                keys.push(key.clone());
                collect_keys(item, keys);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_keys(item, keys);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

/// Parsed `manifest.json` of the exported batch.
fn manifest(directory: &Path) -> Value {
    let path = directory.join("manifest.json");
    let raw = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "the exported batch must hold a manifest at {}: {error}",
            path.display()
        )
    });
    serde_json::from_str(&raw).unwrap_or_else(|error| {
        panic!(
            "{} must be valid JSON, got {raw:?}: {error}",
            path.display()
        )
    })
}

/// Reads the `(doc_id, ord, text)` rows of one passages file, in file order.
fn passage_rows(path: &Path) -> Vec<(String, u32, String)> {
    let file = File::open(path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .unwrap_or_else(|error| {
            panic!(
                "{} must be a readable Parquet file: {error}",
                path.display()
            )
        })
        .build()
        .unwrap_or_else(|error| panic!("{} must be readable row by row: {error}", path.display()));
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch.unwrap_or_else(|error| {
            panic!("{} must yield record batches: {error}", path.display())
        });
        let doc_ids = batch
            .column_by_name("doc_id")
            .unwrap_or_else(|| panic!("{} must hold a doc_id column", path.display()))
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap_or_else(|| panic!("doc_id of {} must be a string", path.display()));
        let ords = batch
            .column_by_name("ord")
            .unwrap_or_else(|| panic!("{} must hold an ord column", path.display()))
            .as_any()
            .downcast_ref::<UInt32Array>()
            .unwrap_or_else(|| panic!("ord of {} must be a uint32", path.display()));
        let texts = batch
            .column_by_name("text")
            .unwrap_or_else(|| panic!("{} must hold a text column", path.display()))
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap_or_else(|| panic!("text of {} must be a string", path.display()));
        for row in 0..batch.num_rows() {
            rows.push((
                doc_ids.value(row).to_owned(),
                ords.value(row),
                texts.value(row).to_owned(),
            ));
        }
    }
    rows
}

/// Reads the `doc_id` values of one documents file, in file order.
fn document_ids(path: &Path) -> Vec<String> {
    let file = File::open(path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .unwrap_or_else(|error| {
            panic!(
                "{} must be a readable Parquet file: {error}",
                path.display()
            )
        })
        .build()
        .unwrap_or_else(|error| panic!("{} must be readable row by row: {error}", path.display()));
    let mut ids = Vec::new();
    for batch in reader {
        let batch = batch.unwrap_or_else(|error| {
            panic!("{} must yield record batches: {error}", path.display())
        });
        let doc_ids = batch
            .column_by_name("doc_id")
            .unwrap_or_else(|| panic!("{} must hold a doc_id column", path.display()))
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap_or_else(|| panic!("doc_id of {} must be a string", path.display()));
        for row in 0..batch.num_rows() {
            ids.push(doc_ids.value(row).to_owned());
        }
    }
    ids
}

/// Splits the Parquet files of a batch into its two roles, by file name.
fn roles(directory: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut documents = Vec::new();
    let mut passages = Vec::new();
    for path in parquet_files(directory) {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if name.starts_with("documents") {
            documents.push(path);
        } else if name.starts_with("passages") {
            passages.push(path);
        } else {
            panic!("the exported batch holds an unexpected Parquet file '{name}'");
        }
    }
    (documents, passages)
}

/// INV-01: no name of any exported artefact mentions an internal identifier.
///
/// A column of any Parquet file, a key of `manifest.json` at any depth, and a
/// file name would all be a way out of the engine for an iid. The check is on
/// the substring, so `internal_iid` or `iid_start` would be caught too; no
/// legitimate column or key of a batch v1 contains it.
#[test]
fn inv_01_exported_batch_names_no_iid() {
    let fixture = fixture();
    let report = run_export(&fixture);
    let directory = &report.directory;

    let files = parquet_files(directory);
    assert!(
        !files.is_empty(),
        "the exported batch must hold Parquet files: {}",
        directory.display()
    );
    for path in &files {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        assert!(
            !name.to_ascii_lowercase().contains("iid"),
            "a Parquet file of the export is named after an internal identifier: {name}"
        );
        for column in column_names(path) {
            assert!(
                !column.to_ascii_lowercase().contains("iid"),
                "column '{column}' of {name} exports an internal identifier; \
                 a batch v1 only carries doc_id, ord and the declared fields"
            );
        }
    }

    let manifest = manifest(directory);
    let mut keys = Vec::new();
    collect_keys(&manifest, &mut keys);
    assert!(
        !keys.is_empty(),
        "the manifest of {} must not be empty: {manifest}",
        directory.display()
    );
    for key in keys {
        assert!(
            !key.to_ascii_lowercase().contains("iid"),
            "key '{key}' of manifest.json exports an internal identifier: {manifest}"
        );
    }
}

/// INV-01: the identifiers of the exported rows are the business ids.
///
/// The fixture maps iids to `(doc_id, ord)` pairs that no iid reproduces, so
/// an export that substituted an iid for a `doc_id`, for an `ord` or for the
/// text fails here. The test also pins the order of `docs/format-lot-v1.md`:
/// the rows of one document arrive in ascending `ord` order.
#[test]
fn inv_01_exported_identifiers_are_doc_ids_and_ords() {
    let fixture = fixture();
    let report = run_export(&fixture);
    let (documents, passages) = roles(&report.directory);

    assert_eq!(
        documents.len(),
        1,
        "six passages and three documents fit in one part each: {:?}",
        report.directory
    );
    assert_eq!(
        passages.len(),
        1,
        "one passages part is enough for six rows"
    );

    let mut exported_ids = document_ids(&documents[0]);
    exported_ids.sort_unstable();
    let mut expected_ids: Vec<String> = EXPECTED_ROWS
        .iter()
        .map(|(doc_id, _)| (*doc_id).to_owned())
        .collect();
    expected_ids.sort_unstable();
    expected_ids.dedup();
    assert_eq!(
        exported_ids, expected_ids,
        "the exported documents must be named by their doc_id, not by an iid"
    );

    let rows = passage_rows(&passages[0]);
    assert_eq!(
        rows.len(),
        EXPECTED_ROWS.len(),
        "the export must carry one row per live passage: {rows:?}"
    );

    let mut exported: Vec<(String, u32)> = rows
        .iter()
        .map(|(doc_id, ord, _)| (doc_id.clone(), *ord))
        .collect();
    exported.sort_unstable();
    let mut expected: Vec<(String, u32)> = EXPECTED_ROWS
        .iter()
        .map(|(doc_id, ord)| ((*doc_id).to_owned(), *ord))
        .collect();
    expected.sort_unstable();
    assert_eq!(
        exported, expected,
        "the exported (doc_id, ord) pairs must be the business pairs, \
         never an iid substituted for one of them"
    );

    for (doc_id, ord, text) in &rows {
        assert_eq!(
            text,
            &format!("{doc_id}#{ord}"),
            "the exported text must be the stored encoder input, \
             not an internal identifier"
        );
        assert!(
            !is_iid_value(doc_id),
            "doc_id '{doc_id}' is the decimal form of an iid"
        );
    }

    // The rows of one document must arrive in ascending ord order
    // (`docs/format-lot-v1.md`, ADR-0039), which is only meaningful if the
    // ords are the business ords.
    let mut last_ord: BTreeMap<String, u32> = BTreeMap::new();
    for (doc_id, ord, _) in &rows {
        if let Some(previous_ord) = last_ord.insert(doc_id.clone(), *ord) {
            assert!(
                *ord > previous_ord,
                "the rows of document '{doc_id}' must ascend in ord, \
                 got {previous_ord} then {ord}"
            );
        }
    }
}

/// True when `value` is the decimal form of one of the fixture iids.
fn is_iid_value(value: &str) -> bool {
    (0..u32::try_from(EXPECTED_ROWS.len()).unwrap_or(u32::MAX)).any(|iid| value == iid.to_string())
}

/// Criterion: the exported batch passes the validation of the existing reader.
///
/// `BatchReader` is the reader of `LIRE-LOT-PARQUET`: it checks the manifest,
/// the checksums, the Parquet schema against the index schema, the counts, the
/// contiguity of the `ord` values, the uniqueness of the `doc_id` values, and
/// the normalization of the vectors.
#[test]
fn exported_batch_passes_batch_reader_validation() {
    let fixture = fixture();
    let report = run_export(&fixture);

    let reader = BatchReader::open(
        &report.directory,
        &fixture.index_config,
        &fixture.model_config,
    )
    .unwrap_or_else(|error| {
        panic!(
            "the exported batch {} must pass the validation of LIRE-LOT-PARQUET: {error}",
            report.directory.display()
        )
    });
    let counts = reader.validate().unwrap_or_else(|error| {
        panic!(
            "the exported batch {} must validate its rows: {error}",
            report.directory.display()
        )
    });

    assert_eq!(
        counts.documents, 3,
        "the export must carry one row per live document"
    );
    assert_eq!(
        counts.passages, 6,
        "the export must carry every live passage"
    );
    assert_eq!(
        counts.deletions, 0,
        "a replace_all batch carries no deletion file (docs/format-lot-v1.md)"
    );
    assert_eq!(
        reader.manifest().mode,
        BatchMode::ReplaceAll,
        "the export is the complete content of the index"
    );
    assert_eq!(
        reader.manifest().index,
        INDEX,
        "the exported manifest must name the index it came from"
    );
    assert_eq!(
        (report.documents, report.passages),
        (3, 6),
        "the report must count what it wrote"
    );
}

/// Criterion: `crible-store::export` writes a `replace_all` batch under
/// `exchange/outbox/<index>-g<G>/`, at the generation it read.
#[test]
fn export_writes_a_replace_all_batch_under_outbox_index_generation() {
    let fixture = fixture();
    let report = run_export(&fixture);

    assert_eq!(
        report.generation,
        crible_core::ids::Generation::new(GENERATION),
        "the export must report the generation of the snapshot it read"
    );
    assert_eq!(
        report.directory,
        fixture.expected_directory(),
        "the export must write into exchange/outbox/<index>-g<G>/"
    );
    assert!(
        report.directory.is_dir(),
        "the export must create {}",
        report.directory.display()
    );

    let manifest = manifest(&report.directory);
    let batch_id = manifest
        .get("batch_id")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("manifest.json must carry a batch_id: {manifest}"));
    assert_eq!(
        batch_id,
        format!("{INDEX}-g{GENERATION}"),
        "the batch_id is also the name of its directory (docs/format-lot-v1.md)"
    );
    assert_eq!(
        manifest.get("mode").and_then(Value::as_str),
        Some("replace_all"),
        "the export is a replace_all batch: {manifest}"
    );
    assert_eq!(
        manifest
            .get("counts")
            .and_then(|counts| counts.get("deletions"))
            .and_then(Value::as_u64),
        Some(0),
        "replace_all forbids deletions: {manifest}"
    );
    assert!(
        !report.directory.join("deletions.parquet").exists(),
        "a replace_all batch must not hold a deletion file"
    );
}
