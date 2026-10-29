// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! Fixture of the lexical branch tests (`INV-05`, task
//! `RECHERCHER-BM25-DANS-LE-POOL`, phase P5).
//!
//! [`LexicalFixture`] builds a real index in a temporary directory: one segment
//! per generation, with the vectors the dense branch reads and the text the
//! lexical branch indexes. The store runs step 4 of `docs/02-architecture.md` at
//! every commit, so the tantivy index is the one a real attach builds — no
//! hand-made index is ever handed to the search.
//!
//! The helpers below the fixture exist because a **read-only** open is what
//! publishes a lexical state (`ADR-0088`): a test closes the writer, puts the
//! derived directory into the state it wants to observe, then opens the index
//! with [`GenerationStore::open_read_only`]. Only derived state is moved or
//! rewritten — `lexical/`, `meta.json` of tantivy, `index.json` and the marker
//! column — never a vector, a passage row or a segment.
//!
//! # What the states of `ADR-0088`, decision 5, cost to build
//!
//! | State | Fixture |
//! |---|---|
//! | `Behind { committed: Some(Gt) }` | commit, save `lexical/`, commit again, restore the saved directory |
//! | `Behind { committed: None }` | commit with the branch disabled, then enable it in `index.json` |
//! | `Ahead { committed }` | commit twice, restore the `records.sqlite` of the first generation |
//! | `NotCertified { committed: Some(Gt) }` | commit, patch a document, drop the marker column |
//! | `NotCertified { committed: None }` | disable, commit, enable, drop the marker column |
//! | `Unreadable` | rewrite the payload of `meta.json`, or overwrite `meta.json` |

// The module is included by several test binaries of this crate, and none uses
// every helper. The same allowance as `crates/crible-store/tests/common/mod.rs`.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crible_core::config::{ReferencePrecision, Schema};
use crible_core::error::CribleError;
use crible_core::filter::Filter;
use crible_core::ids::{Epoch, Generation, Iid};
use crible_search::dense::exact::DenseSearcher;
use crible_search::pool::Pool;
use crible_store::generation::{GenerationChange, GenerationStore, Snapshot, write_index_config};
use crible_store::patch::{DocumentPatch, PatchSource};
use crible_store::records::{DocumentRecord, PassageRecord};
use crible_store::segment::{SegmentSpec, SegmentWriter};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Map, Value, json};

/// Epoch every fixture index creates.
pub const EPOCH: u32 = 1;

/// Name of the tantivy directory inside an epoch directory (`docs/02`).
pub const LEXICAL_DIR: &str = "lexical";

/// Dimensions of the fixture vectors, which every sample must match.
pub const DIMS: u32 = 4;

/// Schema of the fixture index, shared with the `INV-07` fixtures: a document
/// carries `fonds` (a `keyword`), a passage carries `rang` (an `integer`). The
/// lexical branch indexes the text of a passage, so the fields only serve the
/// filters and the payload.
pub fn fixture_schema() -> Schema {
    super::fixture_schema()
}

/// The empty conjunction: every live passage satisfies it, so its pool is the
/// whole live set (`{}` in the filter grammar).
pub fn match_all() -> Filter {
    Filter::And(Vec::new())
}

/// One document the fixture writes: one passage, its text and its vector.
#[derive(Debug, Clone)]
pub struct Doc {
    /// External identifier of the document, which the answer names.
    pub doc_id: String,
    /// Encoder input of the passage: exactly the text both branches read.
    pub text: String,
    /// Reference vector of the passage, written in `fp16`.
    pub vector: [f32; DIMS as usize],
    /// Value of the document field `fonds`, the filter of the `INV-05` tests.
    pub fonds: String,
    /// Value of `passage_hash`, the content digest `dedup: "content"` compares.
    ///
    /// [`doc`] derives it from the document identifier; a test that needs two
    /// passages of the same content sets the same value on both.
    pub passage_hash: String,
}

/// A document of the fixture, with `fonds` set to `doc_id`.
pub fn doc(doc_id: &str, text: &str, vector: [f32; DIMS as usize]) -> Doc {
    Doc {
        doc_id: doc_id.to_owned(),
        text: text.to_owned(),
        vector,
        fonds: doc_id.to_owned(),
        passage_hash: format!("blake3:passage-{doc_id}"),
    }
}

/// One index in a temporary directory, its writer while a test commits, and the
/// rows it holds.
pub struct LexicalFixture {
    // The directory must outlive the index: dropping it removes the files.
    temp: tempfile::TempDir,
    index: PathBuf,
    schema: Schema,
    /// Writer of the index, `None` once the test closed it to reopen the index
    /// itself (the read-only open is what publishes a lexical state).
    store: Option<GenerationStore>,
    next_iid: u32,
}

impl LexicalFixture {
    /// Creates an empty index of the fixture schema, with or without the
    /// lexical branch.
    ///
    /// The configuration is written the way `crible index create` does it: the
    /// store takes `lexical.enabled` from `index.json`, never from a caller.
    pub fn create(lexical: bool) -> Self {
        Self::create_in(
            tempfile::tempdir().unwrap_or_else(|error| {
                panic!("a temporary directory for the index must be creatable: {error}")
            }),
            lexical,
        )
    }

    /// Creates an empty index under `temp`, with or without the lexical branch.
    pub fn create_in(temp: tempfile::TempDir, lexical: bool) -> Self {
        let index = temp.path().join("indexes").join("lexical");
        let schema = fixture_schema();
        let config = super::test_config_with_precision_and_lexical(
            "lexical",
            &schema,
            ReferencePrecision::Fp16,
            lexical,
        );
        write_index_config(&index, &config)
            .unwrap_or_else(|error| panic!("the index configuration must be written: {error}"));
        let store = GenerationStore::create(&index, Epoch::new(EPOCH), &schema)
            .unwrap_or_else(|error| panic!("the fixture index must be created: {error}"));
        Self {
            temp,
            index,
            schema,
            store: Some(store),
            next_iid: 0,
        }
    }

    /// Commits one generation holding `docs` as one passage each, in order: the
    /// internal identifiers are handed out in that order.
    pub fn add_generation(&mut self, docs: &[Doc]) -> Generation {
        assert!(
            !docs.is_empty(),
            "a fixture generation must carry at least one document"
        );
        // Resolved before the writer is borrowed mutably: the accessor borrows
        // the whole fixture.
        let segments = self.segments_dir();
        let store = self.store.as_mut().unwrap_or_else(|| {
            panic!("the fixture writer is closed: a test commits before it closes the index")
        });
        let generation = Generation::new(store.generation().get() + 1);
        let segment_id = format!("segment-{}", generation.get());
        fs::create_dir_all(&segments).unwrap_or_else(|error| {
            panic!(
                "the segments directory {} must be creatable: {error}",
                segments.display()
            )
        });

        let spec = SegmentSpec {
            segment_id: segment_id.clone(),
            iid_start: Iid::new(self.next_iid),
            dims: DIMS,
            generation,
            batch_id: format!("batch-{}", generation.get()),
            precision: ReferencePrecision::Fp16,
        };
        let mut writer = SegmentWriter::create(&segments, spec)
            .unwrap_or_else(|error| panic!("the segment writer must start: {error}"));
        for document in docs {
            writer.append(&document.vector).unwrap_or_else(|error| {
                panic!(
                    "the vector of {} must be appended: {error}",
                    document.doc_id
                )
            });
        }
        let meta = writer
            .finish()
            .unwrap_or_else(|error| panic!("the segment must be finished: {error}"));

        let mut documents = Vec::with_capacity(docs.len());
        let mut passages = Vec::with_capacity(docs.len());
        for (position, document) in docs.iter().enumerate() {
            let iid = self.next_iid
                + u32::try_from(position).unwrap_or_else(|_| {
                    panic!("a fixture generation holds at most u32::MAX documents")
                });
            documents.push(DocumentRecord {
                doc_id: document.doc_id.clone(),
                doc_hash: format!("blake3:doc-{}-{}", document.doc_id, generation.get()),
                fields: json!({ "fonds": document.fonds }),
                updated_generation: generation,
            });
            passages.push(PassageRecord {
                iid: Iid::new(iid),
                doc_id: document.doc_id.clone(),
                ord: 0,
                text: document.text.clone(),
                passage_hash: document.passage_hash.clone(),
                fields: json!({ "rang": position }),
                segment_id: segment_id.clone(),
                created_generation: generation,
                dead_since_gen: None,
            });
        }
        self.next_iid += u32::try_from(docs.len())
            .unwrap_or_else(|_| panic!("a fixture generation holds at most u32::MAX documents"));

        store
            .commit(GenerationChange {
                segments: vec![meta],
                documents,
                passages,
                deletions: Vec::new(),
            })
            .unwrap_or_else(|error| panic!("the fixture commit must succeed: {error}"));
        generation
    }

    /// Tombstones the passages of `doc_id` and removes its `documents` row, as
    /// an attach that drops a document does (ADR-0041, ADR-0043).
    pub fn remove_document(&mut self, doc_id: &str) {
        let connection = Connection::open(self.records_path())
            .unwrap_or_else(|error| panic!("the fixture records must open: {error}"));
        let deletions: Vec<Iid> = {
            let mut statement = connection
                .prepare("SELECT iid FROM passages WHERE doc_id = ?1")
                .unwrap_or_else(|error| panic!("the fixture passages must be readable: {error}"));
            let rows = statement
                .query_map([doc_id], |row| row.get::<_, i64>(0))
                .unwrap_or_else(|error| panic!("the fixture document must be listed: {error}"));
            rows.map(|row| {
                let iid = row.unwrap_or_else(|error| {
                    panic!("a fixture passage iid must be readable: {error}")
                });
                Iid::new(
                    u32::try_from(iid)
                        .unwrap_or_else(|_| panic!("a fixture passage iid must fit u32: {iid}")),
                )
            })
            .collect()
        };
        assert!(
            !deletions.is_empty(),
            "the fixture must tombstone at least one passage of {doc_id}"
        );
        self.store
            .as_mut()
            .unwrap_or_else(|| panic!("the fixture writer is closed"))
            .commit(GenerationChange {
                segments: Vec::new(),
                documents: Vec::new(),
                passages: Vec::new(),
                deletions,
            })
            .unwrap_or_else(|error| panic!("the fixture removal commit must succeed: {error}"));
        connection
            .execute("DELETE FROM documents WHERE doc_id = ?1", [doc_id])
            .unwrap_or_else(|error| panic!("the fixture document row must be deleted: {error}"));
    }

    /// Patches the document fields of `doc_id`, which commits a generation and
    /// runs no step 4: the text does not change (`docs/02`, ADR-0060).
    pub fn patch_document(&mut self, doc_id: &str, fields: Map<String, Value>) -> Generation {
        let store = self
            .store
            .as_mut()
            .unwrap_or_else(|| panic!("the fixture writer is closed"));
        let patch = DocumentPatch {
            doc_id: doc_id.to_owned(),
            fields,
        };
        let report = crible_store::patch::apply(store, &self.schema, PatchSource::Body, &[patch])
            .unwrap_or_else(|error| {
                panic!(
                    "the fixture patch must succeed, got {}: {}",
                    error.code().as_str(),
                    error.message()
                )
            });
        report.generation
    }

    /// Drops the writer: the caller then opens the index itself, which is what
    /// a read-only caller of the engine does.
    pub fn close(&mut self) {
        self.store = None;
    }

    /// Opens the index for reading, as `crible search` does.
    pub fn reopen_read_only(&self) -> GenerationStore {
        GenerationStore::open_read_only(&self.index, &self.schema).unwrap_or_else(|error| {
            panic!(
                "a read-only open must succeed, got {}: {}",
                error.code().as_str(),
                error.message()
            )
        })
    }

    /// Snapshot of the writer's published generation.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.store
            .as_ref()
            .unwrap_or_else(|| panic!("the fixture writer is closed"))
            .snapshot()
    }

    /// Pool of `filter` on the published generation.
    pub fn pool(&self, filter: &Filter) -> Pool {
        let snapshot = self.snapshot();
        Pool::evaluate(snapshot.filters(), filter)
            .unwrap_or_else(|error| panic!("the fixture filter must evaluate: {error}"))
    }

    /// Searcher of the dense branch over `snapshot`, which serves the same
    /// generation as the lexical branch.
    pub fn searcher(&self, snapshot: &Snapshot) -> DenseSearcher {
        DenseSearcher::open(
            &self.segments_dir(),
            snapshot.segments(),
            &self.records_path(),
        )
        .unwrap_or_else(|error| panic!("the fixture searcher must open: {error}"))
    }

    /// Rewrites `index.json` so that `lexical.enabled` is true, keeping every
    /// other setting: the state of an epoch whose branch was enabled after its
    /// passages were committed.
    ///
    /// [`write_index_config`] refuses an occupied index directory, which is why
    /// the member is edited in place. The document is validated by the next
    /// open, as any configuration is.
    pub fn enable_lexical(&self) {
        let path = self.index.join("index.json");
        let raw = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
        let mut value: Value = serde_json::from_str(&raw)
            .unwrap_or_else(|error| panic!("{} must hold JSON: {error}", path.display()));
        value["lexical"]["enabled"] = json!(true);
        fs::write(
            &path,
            serde_json::to_string(&value)
                .unwrap_or_else(|error| panic!("the configuration must serialize: {error}")),
        )
        .unwrap_or_else(|error| panic!("{} must be writable: {error}", path.display()));
    }

    /// Temporary directory the fixture lives in, for a copy of a derived state.
    pub fn temp(&self) -> &Path {
        self.temp.path()
    }

    /// Root of the index.
    pub fn index_dir(&self) -> &Path {
        &self.index
    }

    /// Directory of the single epoch.
    pub fn epoch_dir(&self) -> PathBuf {
        self.index.join("epochs").join(EPOCH.to_string())
    }

    /// Directory of the lexical index of the epoch.
    pub fn lexical_dir(&self) -> PathBuf {
        self.epoch_dir().join(LEXICAL_DIR)
    }

    /// Directory of the segments of the epoch.
    pub fn segments_dir(&self) -> PathBuf {
        self.epoch_dir().join("segments")
    }

    /// Path of the reference database of the epoch.
    pub fn records_path(&self) -> PathBuf {
        self.epoch_dir().join("records.sqlite")
    }
}

/// Copies the directory `from` into `to`, recursively.
pub fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to)
        .unwrap_or_else(|error| panic!("{} must be creatable: {error}", to.display()));
    for entry in fs::read_dir(from)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", from.display()))
    {
        let entry = entry.unwrap_or_else(|error| panic!("an entry must be readable: {error}"));
        let source = entry.path();
        let target = to.join(entry.file_name());
        if source.is_dir() {
            copy_tree(&source, &target);
        } else {
            fs::copy(&source, &target).unwrap_or_else(|error| {
                panic!(
                    "{} must be copied to {}: {error}",
                    source.display(),
                    target.display()
                )
            });
        }
    }
}

/// Removes `to` when it exists, then copies `from` to it.
pub fn replace_tree(from: &Path, to: &Path) {
    match fs::remove_dir_all(to) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("{} must be removable: {error}", to.display()),
    }
    copy_tree(from, to);
}

/// Rewrites the payload of the last tantivy commit of `meta_json`, leaving every
/// other member as tantivy wrote it.
///
/// The payload is the generation a commit indexed (`ADR-0088`, decision 1). A
/// fixture uses this to put an index in a state only a crash or a restore
/// produces: an index whose documents a later generation tombstoned, but whose
/// payload still claims to describe the committed generation.
pub fn rewrite_payload(meta_json: &Path, payload: &str) {
    let raw = fs::read_to_string(meta_json)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", meta_json.display()));
    let mut value: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("{} must hold JSON: {error}", meta_json.display()));
    value["payload"] = json!(payload);
    fs::write(
        meta_json,
        serde_json::to_string(&value)
            .unwrap_or_else(|error| panic!("the tampered meta.json must serialize: {error}")),
    )
    .unwrap_or_else(|error| panic!("{} must be writable: {error}", meta_json.display()));
}

/// Rewrites the payload of the last tantivy commit with a value that is not a
/// generation, which makes `Gt` undecidable (`ADR-0088`, decision 6).
pub fn break_payload(meta_json: &Path) {
    rewrite_payload(meta_json, "not-a-generation");
}

/// Drops the `meta.passage_change_generation` column, which is the state of an
/// epoch written before P5 and never opened for writing by this build
/// (`ADR-0088`, decision 4).
pub fn drop_marker_column(records: &Path) {
    let connection = Connection::open(records)
        .unwrap_or_else(|error| panic!("{} must open: {error}", records.display()));
    connection
        .execute("ALTER TABLE meta DROP COLUMN passage_change_generation", [])
        .unwrap_or_else(|error| panic!("the marker column must be removable: {error}"));
}

/// `meta.generation` of the epoch, read from a connection of its own.
pub fn stored_generation(records: &Path) -> u64 {
    let connection = Connection::open_with_flags(records, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap_or_else(|error| panic!("{} must open read-only: {error}", records.display()));
    let generation: i64 = connection
        .query_row("SELECT generation FROM meta WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap_or_else(|error| panic!("meta.generation must be readable: {error}"));
    u64::try_from(generation).unwrap_or_else(|_| panic!("a generation is never negative"))
}

/// Message of a refusal, or a panic that names what came back instead.
pub fn refusal(error: CribleError) -> String {
    assert_eq!(
        error.code(),
        crible_core::error::ErrorCode::Overloaded,
        "a lexical search without a reader refuses in overloaded (503), got {}: {}",
        error.code().as_str(),
        error.message()
    );
    error.message().to_owned()
}
