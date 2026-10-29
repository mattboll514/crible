// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `crible rebuild-lexical`, the command of `RECONSTRUIRE-LEXICAL`.
//!
//! References: the `RECONSTRUIRE-LEXICAL` task of
//! `docs/phases/P5-lexical-et-hybride.md` ("la commande `crible rebuild-lexical`",
//! "un index reconstruit donne les mêmes résultats BM25 que l'index construit au
//! fil des lots, une fois les passages morts retirés"), `docs/02-architecture.md`
//! ("Reprise au démarrage", the rebuild runs on a write open and creates no
//! generation) and `docs/03-recherche.md` ("Branche lexicale (tantivy)",
//! "Reconstruction").
//!
//! # What these tests can observe
//!
//! The BM25 branch (`RECHERCHER-BM25-DANS-LE-POOL`) does not exist yet, so a
//! command-line test cannot compare scores: it compares the **state** of the
//! lexical index, which is the input of that branch. `crible_store::lexical::
//! read_state` opens the index of an epoch read-only and reports the generation
//! of its last commit and the iids it holds, and tantivy's `meta.json` records
//! the analysis chain of the index, so a test proves four things without a query:
//!
//! - the command rebuilds a missing `lexical/` **itself** — the directory is
//!   removed, read back as empty, and only the command can fill it again, since a
//!   read-only read never writes;
//! - the rebuilt index holds exactly the live passages, the tombstoned ones
//!   removed: that is the "une fois les passages morts retirés" of the sheet;
//! - the rebuild creates no generation, no epoch and no manifest: the index is
//!   visible exactly as before (`docs/02`, ADR-0088);
//! - a changed `lexical.positions` takes effect: the rebuild stores term
//!   positions when the configuration asks for them, although the state stays
//!   `Ready` and a write open repairs nothing (ADR-0088, decision 2).
//!
//! The score equality of the same criterion is proved on a fixture index by
//! `crates/crible-store/tests/inv_09_lexical_compaction.rs`, which runs tantivy
//! itself.
//!
//! # Dataset
//!
//! The three batches of `synth-small` (`tools/gen_synth.py`) are attached, with
//! the `lexical` block of the dataset turned on: `l3` deletes documents, so the
//! index really carries tombstones to remove. Nothing is written outside the
//! temporary directory.
//!
//! ```text
//! cargo test -p crible --test rebuild_lexical_cli
//! ```

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use arrow::array::{ArrayRef, StringArray, new_null_array};
use arrow::datatypes::{DataType, Field, Schema as ArrowSchema};
use crible_core::config::{IndexConfig, Schema};
use crible_store::generation::GenerationStore;
use crible_store::lexical::read_state;
use crible_store::records::RecordStore;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::Value;

use crate::common::{SYNTH_BATCHES, crible_ok, path, require_synth, synth_root};

/// External model configuration of `synth-small` (768 dimensions, `normalize`).
const MODEL_JSON: &str =
    r#"{ "name": "synthetic-external", "kind": "external", "dims": 768, "normalize": true }"#;

/// The index of a test, ready to be rebuilt.
struct Prepared {
    /// Temporary directory holding the data volume and the model.
    _temp: tempfile::TempDir,
    /// Root data directory, `<data>/indexes/<index>`.
    data: PathBuf,
    /// Directory of the index.
    index_dir: PathBuf,
    /// Directory of the committed epoch.
    epoch_dir: PathBuf,
}

/// Creates the index of `synth-small` with its lexical branch enabled and
/// attaches its three batches: the third one deletes documents, so the epoch
/// carries tombstones.
fn prepare() -> Prepared {
    let prepared = create_empty();
    let root = synth_root();
    let models = prepared
        .data
        .parent()
        .unwrap_or_else(|| panic!("{} must have a parent directory", prepared.data.display()))
        .join("models");
    for (batch, _) in SYNTH_BATCHES {
        crible_ok(&[
            "attach",
            "--data",
            path(&prepared.data),
            "--models",
            path(&models),
            "--index",
            "synth-small",
            path(&root.join(batch)),
        ]);
    }
    prepared
}

/// Creates the index of `synth-small` with its lexical branch enabled, and
/// attaches no batch: the epoch of generation 0 holds no passage.
fn create_empty() -> Prepared {
    let root = synth_root();
    require_synth(&root);

    let temp = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
    let data = temp.path().join("data");
    let models = temp.path().join("models");
    let model = models.join("synthetic-external");
    fs::create_dir_all(&model)
        .unwrap_or_else(|error| panic!("the model directory must be created: {error}"));
    fs::write(model.join("model.json"), MODEL_JSON)
        .unwrap_or_else(|error| panic!("the model file must be written: {error}"));

    // The lexical branch is the one of this task: the dataset leaves it off.
    let raw = fs::read_to_string(root.join("index.json"))
        .unwrap_or_else(|error| panic!("the dataset index.json must be readable: {error}"));
    let mut config: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("the dataset index.json must be valid: {error}"));
    let lexical = config
        .get_mut("lexical")
        .unwrap_or_else(|| panic!("index.json must declare a lexical block"));
    lexical["enabled"] = Value::Bool(true);
    let config_path = temp.path().join("index.json");
    fs::write(
        &config_path,
        serde_json::to_string_pretty(&config).unwrap_or_else(|error| {
            panic!("the index configuration must be serializable: {error}")
        }),
    )
    .unwrap_or_else(|error| panic!("the index configuration must be written: {error}"));

    crible_ok(&[
        "index",
        "create",
        "--data",
        path(&data),
        "--config",
        path(&config_path),
    ]);

    let index_dir = data.join("indexes").join("synth-small");
    let epoch_dir = index_dir
        .join("epochs")
        .join(read_current_epoch(&index_dir).to_string());
    Prepared {
        _temp: temp,
        data,
        index_dir,
        epoch_dir,
    }
}

/// Reads `CURRENT_EPOCH` as a decimal number.
fn read_current_epoch(index_dir: &Path) -> u32 {
    let raw = fs::read_to_string(index_dir.join("CURRENT_EPOCH")).unwrap_or_else(|error| {
        panic!(
            "{} must exist: {error}",
            index_dir.join("CURRENT_EPOCH").display()
        )
    });
    raw.trim()
        .parse::<u32>()
        .unwrap_or_else(|error| panic!("CURRENT_EPOCH must hold a number, got {raw:?}: {error}"))
}

/// Sorted names of the manifest files of `epoch_dir`.
fn manifests(epoch_dir: &Path) -> Vec<String> {
    let directory = epoch_dir.join("manifests");
    let mut names: Vec<String> = fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", directory.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("a manifest entry must be readable: {error}"))
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// The iids the reference state of `prepared` holds alive, in ascending order.
fn live_iids(prepared: &Prepared) -> Vec<u32> {
    let records = RecordStore::open(&prepared.epoch_dir.join("records.sqlite"))
        .unwrap_or_else(|error| panic!("the records of the epoch must open: {error}"));
    let mut iids: Vec<u32> = records
        .live_iids()
        .unwrap_or_else(|error| panic!("the live iids must be readable: {error}"))
        .into_iter()
        .map(|iid| iid.get())
        .collect();
    iids.sort_unstable();
    iids
}

/// Number of documents the last batch of the dataset removes.
///
/// The count comes from the manifest the generator wrote, so the fixture proves
/// it carries tombstones without reading a table the store keeps private.
fn removed_documents() -> u64 {
    let manifest = synth_root().join("l3").join("manifest.json");
    let raw = fs::read_to_string(&manifest)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", manifest.display()));
    let value: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("{} must be valid: {error}", manifest.display()));
    value["counts"]["deletions"]
        .as_u64()
        .unwrap_or_else(|| panic!("{} must count its deletions", manifest.display()))
}

/// External identifiers of every document the reference state still holds.
fn live_doc_ids(prepared: &Prepared) -> Vec<String> {
    let records = RecordStore::open(&prepared.epoch_dir.join("records.sqlite"))
        .unwrap_or_else(|error| panic!("the records of the epoch must open: {error}"));
    let mut ids = BTreeSet::new();
    for iid in records
        .live_iids()
        .unwrap_or_else(|error| panic!("the live iids must be readable: {error}"))
    {
        let passage = records
            .read_passage(iid)
            .unwrap_or_else(|error| panic!("the passage of {iid:?} must be readable: {error}"))
            .unwrap_or_else(|| panic!("the live iid {iid:?} must have a passage row"));
        ids.insert(passage.doc_id);
    }
    ids.into_iter().collect()
}

/// Writes a deletion-only batch that removes every `doc_id` of `deletions`.
///
/// The manifest is the one of `template`, with the identifier and the counts of
/// this batch; `documents.parquet` and `passages.parquet` keep the schema of the
/// template and hold no row, which `docs/04` allows for a batch that only
/// deletes (ADR-0040, ADR-0041). The attach then removes every document of the
/// epoch in one transaction.
fn write_deletion_only_batch(
    root: &Path,
    template: &Path,
    batch_id: &str,
    deletions: &[String],
) -> PathBuf {
    let directory = root.join(batch_id);
    fs::create_dir_all(&directory)
        .unwrap_or_else(|error| panic!("{} must be creatable: {error}", directory.display()));

    empty_parquet_with_schema(
        &template.join("documents.parquet"),
        &directory.join("documents.parquet"),
    );
    empty_parquet_with_schema(
        &template.join("passages-0001.parquet"),
        &directory.join("passages.parquet"),
    );
    let schema = Arc::new(ArrowSchema::new(vec![Field::new(
        "doc_id",
        DataType::Utf8,
        false,
    )]));
    let ids: StringArray = deletions.iter().map(|id| Some(id.as_str())).collect();
    common::write_parquet(
        &directory.join("deletions.parquet"),
        &schema,
        vec![Arc::new(ids)],
    );

    let mut files = serde_json::Map::new();
    for name in ["documents.parquet", "passages.parquet", "deletions.parquet"] {
        let bytes = fs::read(directory.join(name))
            .unwrap_or_else(|error| panic!("{name} must be readable: {error}"));
        files.insert(
            name.to_owned(),
            Value::from(format!("blake3:{}", blake3::hash(&bytes).to_hex())),
        );
    }

    let raw = fs::read_to_string(template.join("manifest.json"))
        .unwrap_or_else(|error| panic!("the template manifest must be readable: {error}"));
    let mut manifest: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("the template manifest must be valid JSON: {error}"));
    manifest["batch_id"] = Value::from(batch_id);
    manifest["mode"] = Value::from("upsert");
    manifest["counts"] = serde_json::json!({
        "documents": 0,
        "passages": 0,
        "deletions": deletions.len(),
    });
    manifest["files"] = Value::Object(files);
    let manifest = serde_json::to_string_pretty(&manifest)
        .unwrap_or_else(|error| panic!("the manifest must serialize: {error}"));
    fs::write(directory.join("manifest.json"), manifest)
        .unwrap_or_else(|error| panic!("manifest.json must be written: {error}"));
    directory
}

/// Writes an empty Parquet file with the Arrow schema of `source`.
///
/// A batch carries at least one `documents.parquet` and one `passages.parquet`
/// file (`docs/04`), even when it only deletes; copying the schema of the
/// template keeps the columns and types `attach` checks.
fn empty_parquet_with_schema(source: &Path, target: &Path) {
    let file = fs::File::open(source)
        .unwrap_or_else(|error| panic!("{} must open: {error}", source.display()));
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .unwrap_or_else(|error| panic!("{} must be Parquet: {error}", source.display()));
    let schema = builder.schema().clone();
    let columns: Vec<ArrayRef> = schema
        .fields()
        .iter()
        .map(|field| new_null_array(field.data_type(), 0))
        .collect();
    common::write_parquet(target, &schema, columns);
}

/// The generation and the iids the lexical index of `prepared` holds, or `None`
/// when the epoch has no committed index.
fn indexed_state(prepared: &Prepared) -> Option<(u64, Vec<u32>)> {
    let state = read_state(&prepared.epoch_dir)
        .unwrap_or_else(|error| panic!("the lexical index must be readable: {error}"));
    let generation = state.generation?;
    let mut iids: Vec<u32> = state.iids.iter().map(|iid| iid.get()).collect();
    iids.sort_unstable();
    Some((generation.get(), iids))
}

/// Reads the `record` option tantivy stores for the `text` field of the lexical
/// index of `prepared`: `freq` without term positions, `position` with them.
///
/// The schema of a tantivy index is part of its `meta.json`, so the analysis
/// chain of the built index is observable without opening it, and the option is
/// the only trace a rebuild leaves of `lexical.positions` (`docs/03`, "Branche
/// lexicale").
fn text_record(prepared: &Prepared) -> String {
    let meta = prepared.epoch_dir.join("lexical").join("meta.json");
    let raw = fs::read_to_string(&meta)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", meta.display()));
    let value: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("{} must be valid: {error}", meta.display()));
    let fields = value["schema"].as_array().unwrap_or_else(|| {
        panic!(
            "{} must declare the schema of the index it describes",
            meta.display()
        )
    });
    let text = fields
        .iter()
        .find(|field| field["name"] == "text")
        .unwrap_or_else(|| panic!("{} must declare the text field", meta.display()));
    text["options"]["indexing"]["record"]
        .as_str()
        .unwrap_or_else(|| {
            panic!(
                "{} must record the text field index options",
                meta.display()
            )
        })
        .to_owned()
}

/// The configuration `prepare` created, parsed as the command parses it.
fn declared_config(prepared: &Prepared) -> IndexConfig {
    let config_path = prepared.index_dir.join("index.json");
    let raw = fs::read_to_string(&config_path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", config_path.display()));
    IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("{} must be valid: {error}", config_path.display()))
}

/// The schema declared by the configuration of `prepared`, which a store open
/// asks for.
fn declared_schema(prepared: &Prepared) -> Schema {
    declared_config(prepared).schema
}

/// Turns `lexical.positions` on in the configuration `prepare` created, as a
/// `PATCH` of the setting will do on a running instance (`docs/04`, "Ce qui est
/// figé, ce qui est réglable").
fn enable_positions(prepared: &Prepared) {
    let config_path = prepared.index_dir.join("index.json");
    let raw = fs::read_to_string(&config_path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", config_path.display()));
    let mut config: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("{} must be valid: {error}", config_path.display()));
    config["lexical"]["positions"] = Value::Bool(true);
    fs::write(
        &config_path,
        serde_json::to_string_pretty(&config).unwrap_or_else(|error| {
            panic!("{} must be serializable: {error}", config_path.display())
        }),
    )
    .unwrap_or_else(|error| panic!("{} must be written: {error}", config_path.display()));
}

/// Runs `crible rebuild-lexical` on the index of `prepared`, asserting that it
/// succeeds.
fn rebuild_lexical(prepared: &Prepared) -> std::process::Output {
    let output = Command::new(env!("CARGO_BIN_EXE_crible"))
        .args([
            "rebuild-lexical",
            "--data",
            path(&prepared.data),
            "--index",
            "synth-small",
        ])
        .output()
        .unwrap_or_else(|error| panic!("cannot run crible rebuild-lexical: {error}"));
    assert!(
        output.status.success(),
        "crible rebuild-lexical failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// Acceptance criterion of `RECONSTRUIRE-LEXICAL`: the command rebuilds a missing
/// lexical index of the committed generation, without a write open by the test.
#[test]
fn rebuild_lexical_command_rebuilds_a_missing_lexical_index_at_the_committed_generation() {
    let prepared = prepare();
    let (generation, before) = indexed_state(&prepared).unwrap_or_else(|| {
        panic!("the attach of three batches must leave a committed lexical index")
    });
    assert!(!before.is_empty(), "the fixture index must hold passages");

    fs::remove_dir_all(prepared.epoch_dir.join("lexical")).unwrap_or_else(|error| {
        panic!(
            "the lexical directory of {} must be removable: {error}",
            prepared.epoch_dir.display()
        )
    });
    assert_eq!(
        indexed_state(&prepared),
        None,
        "the removed lexical directory must read as absent: only the command can fill it, a \
         read-only read never writes"
    );

    rebuild_lexical(&prepared);

    let (rebuilt_generation, rebuilt) = indexed_state(&prepared)
        .unwrap_or_else(|| panic!("crible rebuild-lexical must leave a committed lexical index"));
    assert_eq!(
        rebuilt, before,
        "the rebuilt index must hold the same passages as the one it replaces"
    );
    assert_eq!(
        rebuilt_generation, generation,
        "the rebuild must commit the generation of the reference state"
    );
}

/// Acceptance criterion of `RECONSTRUIRE-LEXICAL`: the rebuilt index holds the
/// live passages only, so a tombstoned passage cannot be returned even if tantivy
/// never deleted it.
#[test]
fn rebuild_lexical_command_rebuilds_from_the_live_passages_only() {
    let prepared = prepare();
    let live = live_iids(&prepared);
    assert!(
        removed_documents() > 0,
        "the third batch must delete documents, otherwise the rebuild proves nothing: the \
         fixture would hold no tombstone to remove"
    );

    fs::remove_dir_all(prepared.epoch_dir.join("lexical")).unwrap_or_else(|error| {
        panic!(
            "the lexical directory of {} must be removable: {error}",
            prepared.epoch_dir.display()
        )
    });
    rebuild_lexical(&prepared);

    let (_, rebuilt) = indexed_state(&prepared)
        .unwrap_or_else(|| panic!("crible rebuild-lexical must leave a committed lexical index"));
    assert_eq!(
        rebuilt, live,
        "the rebuilt index must hold exactly the live passages of the reference state, the \
         tombstoned ones removed"
    );
}

/// The reason of the job (`RECONSTRUIRE-LEXICAL`, ADR-0088 decision 2, revised by
/// ADR-0106 decision 4): a change of `lexical.positions` changes the analysis
/// chain recorded in the payload, so a write open rebuilds the lexical index
/// before serving. The command opens the index for writing before it rebuilds;
/// that open already applies the new chain, and the command must keep the same
/// passages at the same generation.
#[test]
fn rebuild_lexical_command_applies_a_changed_analysis_chain() {
    let prepared = prepare();
    let before = indexed_state(&prepared)
        .unwrap_or_else(|| panic!("the attach of three batches must leave a lexical index"));
    assert_eq!(
        text_record(&prepared),
        "freq",
        "the fixture must start without term positions, otherwise the command proves nothing"
    );

    enable_positions(&prepared);
    drop(
        GenerationStore::open(&prepared.index_dir, &declared_schema(&prepared))
            .unwrap_or_else(|error| panic!("the index must open for writing: {}", error.message())),
    );
    assert_eq!(
        text_record(&prepared),
        "position",
        "the write open compares the recorded chain with the configuration and rebuilds it \
         (ADR-0106, decision 4)"
    );

    let output = rebuild_lexical(&prepared);

    // The write open above already applied the chain, so the final text alone
    // would pass even if the command did nothing. Its own report is the proof
    // that it rebuilt the index (ADR-0106, decision 5).
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!(
            "lexical index rebuilt from the reference state at generation {}",
            before.0
        )),
        "the command must report its own rebuild, not rely on the open before it: {stdout}"
    );
    assert_eq!(
        text_record(&prepared),
        "position",
        "the command must leave the lexical index on the analysis chain the configuration asks for"
    );
    assert_eq!(
        indexed_state(&prepared),
        Some(before),
        "the rebuilt index must hold the same passages at the same generation"
    );
}

/// An epoch that never carried a passage has nothing to rebuild: the command
/// says so instead of reporting a rebuild that wrote nothing (`docs/02`, step 4).
#[test]
fn rebuild_lexical_command_reports_an_epoch_without_passage_as_nothing_to_rebuild() {
    let prepared = create_empty();
    assert_eq!(
        indexed_state(&prepared),
        None,
        "a fresh index must hold no lexical index"
    );

    let output = rebuild_lexical(&prepared);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("nothing to rebuild"),
        "the command must tell a no-op from a rebuild, got: {stdout}"
    );
    assert!(
        !prepared.epoch_dir.join("lexical").exists(),
        "an epoch without passage must not gain a lexical directory"
    );
    assert_eq!(
        indexed_state(&prepared),
        None,
        "nothing was rebuilt, so nothing may appear"
    );
}

/// An epoch that carried passages and then lost every one of them keeps a
/// positive `passage_change_generation` marker: the command must rebuild and say
/// so, unlike an epoch that never carried a passage (`docs/02`, step 4).
///
/// This is the divergence the message follows since NETTOYER-P5: a rule based on
/// the live count alone would answer "nothing to rebuild" here, while the marker
/// based rule of ADR-0088, decision 3, rebuilds. The last batch of `synth-small`
/// deletes documents, so the epoch already carries tombstones; a deletion-only
/// batch then removes every remaining document.
#[test]
fn rebuild_lexical_command_rebuilds_an_epoch_emptied_by_deletions() {
    let prepared = prepare();
    let documents = live_doc_ids(&prepared);
    assert!(
        !documents.is_empty(),
        "the attached epoch must hold live documents before the deletion"
    );
    let root = synth_root();
    let batch = write_deletion_only_batch(
        &prepared.data.join("batches"),
        &root.join("l3"),
        "empty-epoch",
        &documents,
    );
    let models = prepared
        .data
        .parent()
        .unwrap_or_else(|| panic!("{} must have a parent directory", prepared.data.display()))
        .join("models");
    crible_ok(&[
        "attach",
        "--data",
        path(&prepared.data),
        "--models",
        path(&models),
        "--index",
        "synth-small",
        path(&batch),
    ]);
    assert!(
        live_iids(&prepared).is_empty(),
        "the deletion-only batch must empty the epoch"
    );

    let output = rebuild_lexical(&prepared);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("lexical index rebuilt"),
        "an epoch emptied by deletions carried passages, so the command must rebuild and say \
         so, got: {stdout}"
    );
    assert!(
        !stdout.contains("nothing to rebuild"),
        "the live count alone must not decide, got: {stdout}"
    );
    let (_, rebuilt) = indexed_state(&prepared)
        .unwrap_or_else(|| panic!("crible rebuild-lexical must leave a committed lexical index"));
    assert!(
        rebuilt.is_empty(),
        "the rebuilt index must hold the live passages, of which there are none"
    );
}

/// The rebuild is invisible: no generation, no epoch and no manifest is created,
/// and running the command twice leaves the same index (`docs/02`, ADR-0088).
#[test]
fn rebuild_lexical_command_publishes_no_generation_and_no_manifest() {
    let prepared = prepare();
    let epoch = read_current_epoch(&prepared.index_dir);
    let manifests_before = manifests(&prepared.epoch_dir);
    let before = indexed_state(&prepared)
        .unwrap_or_else(|| panic!("the attach of three batches must leave a lexical index"));

    rebuild_lexical(&prepared);

    assert_eq!(
        read_current_epoch(&prepared.index_dir),
        epoch,
        "the rebuild creates no epoch"
    );
    assert_eq!(
        manifests(&prepared.epoch_dir),
        manifests_before,
        "the rebuild creates no manifest: the visible generations are unchanged"
    );
    assert_eq!(
        indexed_state(&prepared),
        Some(before),
        "the rebuild of an up-to-date index must return the same passages at the same generation"
    );

    // A second run is the job an operator launches when the analysis chain
    // changes: it must be idempotent on a state that already describes G.
    rebuild_lexical(&prepared);
    assert_eq!(
        manifests(&prepared.epoch_dir),
        manifests_before,
        "a second rebuild must create no manifest either"
    );
}
