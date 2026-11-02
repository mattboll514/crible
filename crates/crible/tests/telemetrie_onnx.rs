// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `COUPER-TELEMETRIE-ONNX` (outside the plan, preparation of J7) at the process
//! boundary: a `crible search` that loads ONNX Runtime must create no 1DS client
//! file under the home directory of its process.
//!
//! References: the task sheet (`docs/phases/P7-observabilite.md:147-169`) and
//! ADR-0128 (decisions 2, 5 and 8). ONNX Runtime 1.31.0 carries the Microsoft
//! 1DS telemetry client: without `ORT_DISABLE_TELEMETRY`, it creates a device id
//! and a SQLite store under
//! `~/Library/Application Support/Microsoft/DeveloperTools/.onnxruntime/`
//! (macOS) and sends usage data to a third party. The ADR cuts it at three
//! points: the variable set at the start of `main` (decision 2), the `[env]`
//! section of `.cargo/config.toml` (decision 5) and `with_telemetry(false)` on
//! the ORT environment (decision 3).
//!
//! This test is the proof of the first point alone, since it is the one that
//! covers the shipped binary. It launches the real `crible` binary, with a
//! temporary `HOME`, `XDG_DATA_HOME` and `XDG_CONFIG_HOME`, and **removes**
//! `ORT_DISABLE_TELEMETRY` from the environment of the child: what it checks is
//! the variable set by `main`, not the one `.cargo/config.toml` gives the test
//! process itself. The other variables that make ONNX Runtime silent on a CI
//! runner are removed for the same reason (decision 8), so the test keeps its
//! value on a developer machine and in the `rust-onnx` job.
//!
//! A text query is what loads ONNX Runtime: the fake encoder of the versioned
//! fixture encodes the query through the `ort` crate, and the environment is
//! created there. The index is built by the library, without an encoder, so the
//! whole test costs one ONNX session.
//!
//! ```sh
//! ORT_DYLIB_PATH=/path/to/libonnxruntime.dylib \
//!   cargo test -p crible --test telemetrie_onnx -- --ignored
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crible_core::config::{ReferencePrecision, Schema};
use crible_core::ids::{Epoch, Generation, Iid};
use crible_store::generation::{GenerationChange, GenerationStore};
use crible_store::records::{DocumentRecord, PassageRecord};
use crible_store::segment::{SegmentMeta, SegmentSpec, SegmentWriter};
use serde_json::Value;

/// Name of the index.
const INDEX: &str = "telemetrie";

/// Dimensions of the fake encoder (`model.json` of the fixture).
const DIMS: u32 = 768;

/// External name of the model, which the index declares.
const MODEL: &str = "fake-model";

/// Schema of the index: one document field, one passage field.
const SCHEMA: &str = r#"{
    "document_fields": { "fonds": { "type": "keyword" } },
    "passage_fields": { "partie": { "type": "keyword" } }
}"#;

/// Variables that make ONNX Runtime silent on its own, removed from the child.
///
/// `ORT_DISABLE_TELEMETRY` is the one under test; the five others are the CI
/// markers of ADR-0128, decision 8. Removing them keeps the test meaningful on
/// a runner, where the CI detections would otherwise hide a regression.
const REMOVED: [&str; 6] = [
    "ORT_DISABLE_TELEMETRY",
    "TF_BUILD",
    "GITHUB_ACTIONS",
    "GITLAB_CI",
    "CI",
    "ORT_RUNNING_UNIT_TESTS",
];

/// Names of the 1DS client artifacts: the store directory and its database.
const TELEMETRY_NAMES: [&str; 2] = [".onnxruntime", "onnxruntime.db"];

/// Directory of the versioned fake model.
fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../crible-encode/tests/fixtures/fake-model")
}

/// One temporary instance: its own roots, isolated from the developer's home.
struct Instance {
    /// Keeps the temporary directory alive for the whole test.
    temp: tempfile::TempDir,
    /// `HOME` of the child, where the 1DS client would write on macOS.
    home: PathBuf,
    /// `XDG_DATA_HOME` of the child.
    xdg_data: PathBuf,
    /// `XDG_CONFIG_HOME` of the child.
    xdg_config: PathBuf,
    /// Root of the data directory, passed as `--data`.
    data: PathBuf,
    /// Root of the models directory, passed as `--models`.
    models: PathBuf,
}

impl Instance {
    /// Creates the three isolated roots, the model copy and the index.
    fn new() -> Self {
        let temp = tempfile::tempdir()
            .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
        let home = temp.path().join("home");
        let xdg_data = temp.path().join("xdg-data");
        let xdg_config = temp.path().join("xdg-config");
        let data = temp.path().join("data");
        let models = temp.path().join("models");
        for directory in [&home, &xdg_data, &xdg_config, &data, &models] {
            fs::create_dir_all(directory).unwrap_or_else(|error| {
                panic!("{} must be creatable: {error}", directory.display())
            });
        }
        copy_fixture(&models.join(MODEL));
        let instance = Self {
            temp,
            home,
            xdg_data,
            xdg_config,
            data,
            models,
        };
        instance.create_index();
        instance.commit_fixture();
        instance
    }

    /// Runs `crible index create` with the configuration of the fixture.
    ///
    /// The command loads no ONNX Runtime today, but it runs in the same
    /// isolated environment as the search: a future release that builds a
    /// session here must fail this test instead of writing in the real home of
    /// the developer.
    fn create_index(&self) {
        let config = self.temp.path().join("index.json");
        fs::write(&config, configuration())
            .unwrap_or_else(|error| panic!("{} must be written: {error}", config.display()));
        let output = self.crible(&[
            "index",
            "create",
            "--data",
            path(&self.data),
            "--config",
            path(&config),
        ]);
        assert!(
            output.status.success(),
            "index create failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Runs the binary of the tests in the isolated environment of the instance.
    ///
    /// Every child of the test, search and index creation alike, gets the three
    /// temporary roots and loses the six variables of `REMOVED`, so no command
    /// can reach the real home nor the telemetry switch of this process.
    fn crible(&self, arguments: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_crible"));
        command
            .args(arguments)
            .env("HOME", &self.home)
            .env("XDG_DATA_HOME", &self.xdg_data)
            .env("XDG_CONFIG_HOME", &self.xdg_config);
        for name in REMOVED {
            command.env_remove(name);
        }
        command
            .output()
            .unwrap_or_else(|error| panic!("cannot run crible {arguments:?}: {error}"))
    }

    /// Commits generation 1 with three documents and three passages.
    ///
    /// The vectors are one-hot, so the search has a predictable best key; the
    /// texts are never read by this test, only encoded as the query is by the
    /// fake encoder.
    fn commit_fixture(&self) {
        let index = self.data.join("indexes").join(INDEX);
        let schema = Schema::from_json_str(SCHEMA)
            .unwrap_or_else(|error| panic!("the fixture schema must be valid: {error}"));
        let mut store = GenerationStore::open(&index, &schema)
            .unwrap_or_else(|error| panic!("the fixture index must open: {error}"));
        let segment = write_segment(&index);
        store
            .commit(GenerationChange {
                segments: vec![segment],
                documents: (0..3).map(document).collect(),
                passages: (0..3).map(passage).collect(),
                deletions: Vec::new(),
            })
            .unwrap_or_else(|error| panic!("the fixture generation must commit: {error}"));
    }

    /// Runs `crible search` on a text query, in the isolated environment.
    fn search(&self) -> Output {
        self.crible(&[
            "search",
            "--data",
            path(&self.data),
            "--index",
            INDEX,
            "--models",
            path(&self.models),
            "--mode",
            "dense",
            "--query",
            "contrat de travail",
            "--k",
            "3",
        ])
    }

    /// Every artifact of the 1DS client found under the three isolated roots.
    fn telemetry_paths(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        for root in [&self.home, &self.xdg_data, &self.xdg_config] {
            collect_telemetry(root, &mut found);
        }
        found
    }
}

/// Copies the versioned fake model into the models root.
fn copy_fixture(destination: &Path) {
    let fixture = fixture_root();
    fs::create_dir_all(destination)
        .unwrap_or_else(|error| panic!("{} must be creatable: {error}", destination.display()));
    for name in [
        "model.json",
        "model.onnx",
        "model.int8.onnx",
        "tokenizer.json",
    ] {
        let source = fixture.join(name);
        assert!(
            source.is_file(),
            "{} is missing; the fake model fixture is versioned",
            source.display()
        );
        fs::copy(&source, destination.join(name))
            .unwrap_or_else(|error| panic!("{name} is copied: {error}"));
    }
}

/// Configuration of the index, with the settings the store requires.
fn configuration() -> String {
    format!(
        r#"{{"name":"{INDEX}","model":"{MODEL}","reference_precision":"fp16","schema":{SCHEMA},
            "cluster_by":[],
            "lexical":{{"enabled":true,"language":"fr","positions":false}},
            "search":{{"scan":"none","rescore_candidates":10,"exact_threshold":10,
                      "candidates_per_branch":10,"fusion":{{"method":"rrf"}}}},
            "ingest":{{"acceptance_sample":1,"acceptance_min_cosine":0.9}},
            "compaction":{{"dead_ratio":0.1}}}}"#
    )
}

/// Builds one `documents` row of generation 1.
fn document(iid: u32) -> DocumentRecord {
    DocumentRecord {
        doc_id: format!("D{iid}"),
        doc_hash: format!("blake3:doc-{iid}"),
        fields: serde_json::json!({ "fonds": "LEGI" }),
        updated_generation: Generation::new(1),
    }
}

/// Builds one `passages` row of generation 1.
fn passage(iid: u32) -> PassageRecord {
    PassageRecord {
        iid: Iid::new(iid),
        doc_id: format!("D{iid}"),
        ord: 0,
        text: format!("contrat numero {iid}"),
        passage_hash: format!("blake3:D{iid}-0"),
        fields: serde_json::json!({ "partie": "corps" }),
        segment_id: "seg-1".to_owned(),
        created_generation: Generation::new(1),
        dead_since_gen: None,
    }
}

/// Writes the only segment of the fixture and returns its metadata.
fn write_segment(index: &Path) -> SegmentMeta {
    let segments = index
        .join("epochs")
        .join(Epoch::new(1).get().to_string())
        .join("segments");
    fs::create_dir_all(&segments)
        .unwrap_or_else(|error| panic!("the segments directory must be creatable: {error}"));
    let mut writer = SegmentWriter::create(
        &segments,
        SegmentSpec {
            segment_id: "seg-1".to_owned(),
            iid_start: Iid::new(0),
            dims: DIMS,
            generation: Generation::new(1),
            batch_id: "batch-telemetrie".to_owned(),
            precision: ReferencePrecision::Fp16,
        },
    )
    .unwrap_or_else(|error| panic!("the segment writer must start: {error}"));
    for iid in 0..3 {
        let mut vector = vec![0.0_f32; DIMS as usize];
        vector[iid as usize] = 1.0;
        writer
            .append(&vector)
            .unwrap_or_else(|error| panic!("the fixture vector must be appended: {error}"));
    }
    writer
        .finish()
        .unwrap_or_else(|error| panic!("the fixture segment must be finished: {error}"))
}

/// Adds to `found` every 1DS artifact under `root`.
fn collect_telemetry(root: &Path, found: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) => panic!("{} must be readable: {error}", root.display()),
    };
    for entry in entries {
        let entry =
            entry.unwrap_or_else(|error| panic!("{} must be readable: {error}", root.display()));
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if TELEMETRY_NAMES.contains(&name.as_ref()) {
            found.push(path.clone());
        }
        if entry
            .file_type()
            .unwrap_or_else(|error| panic!("{} must be inspectable: {error}", path.display()))
            .is_dir()
        {
            collect_telemetry(&path, found);
        }
    }
}

/// Path as a `&str`, the test building it from a temporary directory.
fn path(path: &Path) -> &str {
    path.to_str()
        .unwrap_or_else(|| panic!("{} must be UTF-8", path.display()))
}

#[test]
#[ignore = "needs ONNX Runtime (ORT_DYLIB_PATH); run: ORT_DYLIB_PATH=/path/to/libonnxruntime.dylib cargo test -p crible --test telemetrie_onnx -- --ignored"]
fn a_text_search_creates_no_telemetry_file_under_a_temporary_home() {
    // Acceptance criteria 1 and 5 of `COUPER-TELEMETRIE-ONNX`: `crible search`
    // loads ONNX Runtime through the fake encoder, succeeds, and leaves no 1DS
    // client file behind (ADR-0128, decisions 2 and 8).
    let instance = Instance::new();
    let output = instance.search();
    assert!(
        output.status.success(),
        "crible search failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("the search answer must be JSON: {error}"));
    assert!(
        !response["results"]
            .as_array()
            .unwrap_or_else(|| panic!("the answer must hold a results array: {response}"))
            .is_empty(),
        "the query must reach the encoder, which is what creates the ORT environment: {response}"
    );
    let found = instance.telemetry_paths();
    assert!(
        found.is_empty(),
        "the 1DS client of ONNX Runtime wrote under the temporary home: {found:?}"
    );
}
