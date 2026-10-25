// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `PATCHER-METADONNEES` (phase P3): `crible-store::patch`.
//!
//! References: `docs/phases/P3-cycle-de-vie.md` (task PATCHER-METADONNEES),
//! ADR-0060, `docs/02-architecture.md` ("Générations et point de commit",
//! "Patch de métadonnées"), `docs/04-donnees-et-ingestion.md` ("Patch de
//! métadonnées"), `docs/05-api.md` ("Patch de métadonnées").
//!
//! # What each test pins
//!
//! | Test | Criterion / decision |
//! |---|---|
//! | `patch_reflects_the_new_value_in_the_filters_of_the_document` | criterion 1: the filters see the new value, for every passage of the document |
//! | `patch_updates_the_columns_of_numeric_and_date_fields` | criterion 1: value columns follow too |
//! | `patch_leaves_the_vectors_the_iids_and_the_doc_hash_unchanged` | criterion 2, plus decision 9 on an effective patch: no `batches` row, no `segments` row |
//! | `a_mis_typed_value_is_refused_without_any_effect` | criterion 3 |
//! | `patch_merges_the_fields_and_keeps_the_others` | ADR-0060, decision 2 |
//! | `patch_stores_null_as_a_value` | decision 3, with ADR-0035 (`exists`) |
//! | `patch_refuses_an_unknown_document_and_writes_nothing` | decision 4, atomicity |
//! | `a_duplicate_document_id_is_refused` | decision 4, duplicate case |
//! | `patch_refuses_a_passage_field_a_reserved_name_and_an_unknown_field` | decision 5 |
//! | `patch_accepts_a_field_that_is_not_filterable_or_not_returned` | decision 5 |
//! | `a_patch_never_echoes_a_field_value` | decisions 5 and 6, no value in a message |
//! | `a_patch_without_effect_creates_no_generation` | decision 9, no-op |
//! | `patch_refuses_more_entries_than_the_limit` | decision 7, entry bound |
//! | `patch_refuses_more_bytes_than_the_limit` | decision 7, byte bound, in memory |
//! | `read_file_refuses_an_oversized_file_before_loading_it` | decision 7, byte bound, at the CLI boundary |
//! | `the_report_counts_updated_unchanged_and_the_generation` | decision 8 |
//!
//! # The fixture
//!
//! One index of the fixture schema, generation 1, one segment `seg-1` holding
//! three vectors, two documents: `D1` with passages 0 and 1, `D2` with passage
//! 2. The document fields are one of each type, plus a field that is neither
//! filterable nor returned.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use crible_core::config::Schema;
use crible_core::error::ErrorCode;
use crible_core::ids::{Generation, Iid};
use crible_store::filters_index::{FilterIndex, date_days};
use crible_store::generation::{GenerationStore, write_index_config};
use crible_store::patch::{self, DocumentPatch, PatchSource};
use crible_store::records::{DocumentRecord, PassageRecord, ReadPool};
use roaring::RoaringBitmap;
use serde_json::{Value, json};

use common::{
    assert_manifest_describes, change, commit, directory_entries, epoch_dir, manifests_dir,
    records_path, temp_index, write_segment,
};

/// Schema of the fixture: one document field per type, two fields whose options
/// are off, and one passage field.
fn fixture_schema() -> Schema {
    Schema::from_json_str(
        r#"{
            "document_fields": {
                "juridiction": { "type": "keyword" },
                "rank": { "type": "integer" },
                "published": { "type": "date" },
                "sensitive": { "type": "keyword", "filterable": false },
                "internal": { "type": "keyword", "returned": false }
            },
            "passage_fields": { "partie": { "type": "keyword" } }
        }"#,
    )
    .unwrap_or_else(|error| panic!("the fixture schema must be valid: {error}"))
}

/// One index at generation 1, ready to be patched.
struct Fixture {
    /// Keeps the temporary directory alive for the whole test.
    _temp: tempfile::TempDir,
    /// Directory of the index.
    index: PathBuf,
    /// Schema the index was created with.
    schema: Schema,
    /// Store of the index, generation 1.
    store: GenerationStore,
}

impl Fixture {
    /// Creates the index, commits generation 1 with `D1` (passages 0 and 1) and
    /// `D2` (passage 2), and returns the open store.
    fn new() -> Self {
        let (temp, index) = temp_index();
        let schema = fixture_schema();
        write_index_config(&index, &common::test_config("patch", &schema))
            .unwrap_or_else(|error| panic!("the index configuration must be written: {error}"));
        let mut store = GenerationStore::create(&index, crible_core::ids::Epoch::new(1), &schema)
            .unwrap_or_else(|error| panic!("the fixture index must be created: {error}"));

        let segment = write_segment(
            &index,
            "seg-1",
            0,
            1,
            &[
                &[1.0, 0.0, 0.0, 0.0],
                &[0.0, 1.0, 0.0, 0.0],
                &[0.0, 0.0, 1.0, 0.0],
            ],
        );
        let first = document("D1");
        let second = document("D2");
        commit(
            &mut store,
            change(
                vec![segment],
                vec![first, second],
                vec![
                    passage(0, "D1", 0, "motifs"),
                    passage(1, "D1", 1, "dispositif"),
                    passage(2, "D2", 0, "motifs"),
                ],
                Vec::new(),
            ),
        );

        Self {
            _temp: temp,
            index,
            schema,
            store,
        }
    }

    /// Applies one patch read from a file, as `crible patch` does.
    fn apply(&mut self, fields: &[(&str, Value)]) -> Result<patch::PatchReport, ErrorView> {
        let patches: Vec<DocumentPatch> = fields
            .iter()
            .map(|(doc_id, fields)| entry(doc_id, fields.clone()))
            .collect();
        patch::apply(&mut self.store, &self.schema, PatchSource::File, &patches)
            .map_err(ErrorView::of)
    }

    /// Reads one document from `records.sqlite` through the payload pool.
    fn read_document(&self, doc_id: &str) -> DocumentRecord {
        ReadPool::new(&records_path(&self.index), 1)
            .unwrap_or_else(|error| panic!("the payload pool must open: {error}"))
            .read_document(doc_id)
            .unwrap_or_else(|error| panic!("document {doc_id} must be readable: {error}"))
            .unwrap_or_else(|| panic!("document {doc_id} must exist"))
    }

    /// Bytes of the reference vectors of the only segment.
    fn vector_bytes(&self) -> Vec<u8> {
        let path = epoch_dir(&self.index)
            .join("segments")
            .join("seg-1")
            .join("vectors.f16");
        fs::read(&path)
            .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()))
    }
}

/// The code and message of a refused patch, so a test asserts on both without
/// matching on the whole error type.
struct ErrorView {
    /// Error code, as `docs/05-api.md` names it.
    code: ErrorCode,
    /// Message of the error.
    message: String,
}

impl ErrorView {
    /// Captures one error.
    fn of(error: crible_core::error::CribleError) -> Self {
        Self {
            code: error.code(),
            message: error.message().to_owned(),
        }
    }
}

/// Builds one document row of the fixture.
fn document(doc_id: &str) -> DocumentRecord {
    DocumentRecord {
        doc_id: doc_id.to_owned(),
        doc_hash: format!("blake3:doc-{doc_id}"),
        fields: json!({
            "juridiction": "judiciaire",
            "rank": 1,
            "published": "2020-01-01",
            "sensitive": "secret",
            "internal": "note",
        }),
        updated_generation: Generation::new(1),
    }
}

/// Builds one alive passage row of the fixture.
fn passage(iid: u32, doc_id: &str, ord: u32, partie: &str) -> PassageRecord {
    PassageRecord {
        iid: Iid::new(iid),
        doc_id: doc_id.to_owned(),
        ord,
        text: format!("{doc_id}#{ord}"),
        passage_hash: format!("blake3:passage-{doc_id}-{ord}"),
        fields: json!({ "partie": partie }),
        segment_id: "seg-1".to_owned(),
        created_generation: Generation::new(1),
        dead_since_gen: None,
    }
}

/// Builds one entry of a patch from the JSON of its `fields`.
fn entry(doc_id: &str, fields: Value) -> DocumentPatch {
    let Value::Object(fields) = fields else {
        panic!("the fields of a test entry must be a JSON object");
    };
    DocumentPatch {
        doc_id: doc_id.to_owned(),
        fields,
    }
}

/// The filter structures of one `keyword` field, value by value.
fn keyword_values(index: &FilterIndex, field: &str) -> BTreeMap<String, RoaringBitmap> {
    index
        .keyword(field)
        .map(|keyword| {
            keyword
                .values()
                .map(|(value, bitmap)| (value.to_owned(), bitmap.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// Everything a refused patch must leave untouched.
struct State {
    /// `meta.generation`.
    generation: u64,
    /// Generation of the published snapshot.
    snapshot: u64,
    /// Bitmap of the live passages.
    live: RoaringBitmap,
    /// Filter structures of `juridiction`.
    juridictions: BTreeMap<String, RoaringBitmap>,
    /// `documents` rows of the fixture.
    documents: Vec<DocumentRecord>,
    /// Manifests of the epoch.
    manifests: Vec<String>,
    /// Reference vectors of the segment.
    vectors: Vec<u8>,
    /// Rows of `batches`: a patch never records one (ADR-0060, decision 9).
    batches: i64,
    /// Rows of `segments`: a patch writes no segment (ADR-0060, decision 9).
    segments: i64,
}

/// Counts the `batches` rows of an index.
fn batch_count(index: &std::path::Path) -> i64 {
    row_count(index, "SELECT COUNT(*) FROM batches")
}

/// Counts the `segments` rows of an index.
fn segment_count(index: &std::path::Path) -> i64 {
    row_count(index, "SELECT COUNT(*) FROM segments")
}

/// Counts the rows one query returns on `records.sqlite`, opened read-only.
fn row_count(index: &std::path::Path, query: &str) -> i64 {
    let connection = rusqlite::Connection::open_with_flags(
        records_path(index),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap_or_else(|error| panic!("records.sqlite must open read-only: {error}"));
    connection
        .query_row(query, [], |row| row.get(0))
        .unwrap_or_else(|error| panic!("{query} must be readable: {error}"))
}

/// Captures the state a patch without effect must keep.
fn capture(fixture: &Fixture) -> State {
    let snapshot = fixture.store.snapshot();
    State {
        generation: fixture.store.generation().get(),
        snapshot: snapshot.generation().get(),
        live: snapshot.filters().live().clone(),
        juridictions: keyword_values(snapshot.filters(), "juridiction"),
        documents: vec![fixture.read_document("D1"), fixture.read_document("D2")],
        manifests: directory_entries(&manifests_dir(&fixture.index)),
        vectors: fixture.vector_bytes(),
        batches: batch_count(&fixture.index),
        segments: segment_count(&fixture.index),
    }
}

/// Asserts that nothing of [`State`] moved.
fn assert_unchanged(fixture: &Fixture, before: &State) {
    let after = capture(fixture);
    assert_eq!(
        after.generation, before.generation,
        "no generation may be created"
    );
    assert_eq!(
        after.snapshot, before.snapshot,
        "no snapshot may be published"
    );
    assert_eq!(after.live, before.live, "the live bitmap may not move");
    assert_eq!(
        after.juridictions, before.juridictions,
        "the filter structures may not move"
    );
    assert_eq!(
        after.documents, before.documents,
        "no documents row may change"
    );
    assert_eq!(
        after.manifests, before.manifests,
        "no manifest may be written"
    );
    assert_eq!(after.vectors, before.vectors, "no vector may change");
    assert_eq!(
        after.batches, before.batches,
        "a patch writes no batches row"
    );
    assert_eq!(
        after.segments, before.segments,
        "a patch writes no segments row"
    );
}

#[test]
fn patch_reflects_the_new_value_in_the_filters_of_the_document() {
    // Criterion 1: after a patch, the filters of the modified field see the new
    // value. `D1` has two passages: both inherit the document field, so both
    // move. `D2` keeps its value.
    let mut fixture = Fixture::new();
    let report = fixture
        .apply(&[("D1", json!({ "juridiction": "administratif" }))])
        .unwrap_or_else(|error| panic!("the patch must apply: {}", error.message));
    assert_eq!(report.updated, 1);
    assert_eq!(report.unchanged, 0);
    assert_eq!(report.generation.get(), 2);

    let snapshot = fixture.store.snapshot();
    assert_eq!(snapshot.generation().get(), 2);
    let filters = snapshot.filters();
    let administratif = filters
        .keyword("juridiction")
        .and_then(|keyword| keyword.bitmap("administratif"))
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        administratif,
        RoaringBitmap::from_iter([0, 1]),
        "the two passages of D1 must carry the new value"
    );
    assert_eq!(
        filters
            .keyword("juridiction")
            .and_then(|keyword| keyword.bitmap("judiciaire"))
            .cloned()
            .unwrap_or_default(),
        RoaringBitmap::from_iter([2]),
        "only D2, whose value does not change, keeps the old value"
    );
    let present = filters
        .keyword("juridiction")
        .map(|keyword| keyword.present().clone())
        .unwrap_or_default();
    assert_eq!(
        present,
        RoaringBitmap::from_iter([0, 1, 2]),
        "every passage still carries a non-null value"
    );
    assert_manifest_describes(&fixture.index, 2, &["seg-1"]);
}

#[test]
fn patch_updates_the_columns_of_numeric_and_date_fields() {
    // Criterion 1, for the fields held by a value column rather than a bitmap:
    // an integer and a date of the document reach every passage.
    let mut fixture = Fixture::new();
    fixture
        .apply(&[("D2", json!({ "rank": 7, "published": "2021-03-04" }))])
        .unwrap_or_else(|error| panic!("the patch must apply: {}", error.message));

    let snapshot = fixture.store.snapshot();
    let filters = snapshot.filters();
    let rank = filters
        .column("rank")
        .unwrap_or_else(|| panic!("rank is declared as an integer"));
    assert_eq!(rank.integer(Iid::new(2)), Some(7));
    assert_eq!(rank.integer(Iid::new(0)), Some(1), "D1 keeps its own rank");
    let published = filters
        .column("published")
        .unwrap_or_else(|| panic!("published is declared as a date"));
    assert_eq!(
        published.date_days(Iid::new(2)),
        date_days("2021-03-04"),
        "the date of the document reaches its passage"
    );
}

#[test]
fn patch_leaves_the_vectors_the_iids_and_the_doc_hash_unchanged() {
    // Criterion 2: a patch writes no segment and no passage row. The identifiers
    // are the same, the vectors are the same bytes, and `doc_hash` does not move.
    let mut fixture = Fixture::new();
    let before = capture(&fixture);
    let segments_before: Vec<String> = fixture
        .store
        .snapshot()
        .segments()
        .iter()
        .map(|segment| segment.segment_id.clone())
        .collect();

    fixture
        .apply(&[("D1", json!({ "juridiction": "administratif" }))])
        .unwrap_or_else(|error| panic!("the patch must apply: {}", error.message));

    let snapshot = fixture.store.snapshot();
    let after = capture(&fixture);
    assert_eq!(
        after.live, before.live,
        "the iid of every passage must be unchanged"
    );
    assert_eq!(
        snapshot
            .segments()
            .iter()
            .map(|segment| segment.segment_id.clone())
            .collect::<Vec<String>>(),
        segments_before,
        "no segment may be added or removed"
    );
    assert_eq!(after.vectors, before.vectors, "the vectors must not change");
    assert_eq!(
        after.documents[0].doc_hash, before.documents[0].doc_hash,
        "doc_hash is the hash of the passages, which a patch does not touch"
    );
    assert_eq!(
        after.documents[0].updated_generation,
        Generation::new(2),
        "the patched row names G+1"
    );
    // ADR-0060, decision 9, on an effective patch: the commit writes `documents`
    // and `meta` only, so no `batches` row and no `segments` row is added.
    assert_eq!(
        after.batches, before.batches,
        "an effective patch writes no batches row"
    );
    assert_eq!(
        after.segments, before.segments,
        "an effective patch writes no segments row"
    );
}

#[test]
fn a_mis_typed_value_is_refused_without_any_effect() {
    // Criterion 3: a value of the wrong type is refused before any write. The
    // generation, `records.sqlite`, the filter structures, the manifests and the
    // vectors stay exactly as they were.
    let mut fixture = Fixture::new();
    let before = capture(&fixture);

    let error = fixture
        .apply(&[("D1", json!({ "rank": "sept" }))])
        .err()
        .unwrap_or_else(|| panic!("a string in an integer field must be refused"));
    assert_eq!(error.code, ErrorCode::InvalidRequest);

    assert_unchanged(&fixture, &before);
}

#[test]
fn patch_merges_the_fields_and_keeps_the_others() {
    // ADR-0060, decision 2: a key of `fields` replaces the value of the field of
    // the same name, and an absent key keeps its value.
    let mut fixture = Fixture::new();
    fixture
        .apply(&[("D1", json!({ "rank": 42 }))])
        .unwrap_or_else(|error| panic!("the patch must apply: {}", error.message));

    let fields = fixture.read_document("D1").fields;
    assert_eq!(fields["rank"], json!(42));
    assert_eq!(fields["juridiction"], json!("judiciaire"));
    assert_eq!(fields["published"], json!("2020-01-01"));
    assert_eq!(fields["sensitive"], json!("secret"));
    assert_eq!(fields["internal"], json!("note"));
}

#[test]
fn patch_stores_null_as_a_value() {
    // ADR-0060, decision 3: `null` is a value, never a removal. The key stays in
    // the stored JSON, and ADR-0035 makes `exists` and comparisons ignore it.
    let mut fixture = Fixture::new();
    fixture
        .apply(&[("D1", json!({ "juridiction": null }))])
        .unwrap_or_else(|error| panic!("the patch must apply: {}", error.message));

    let fields = fixture.read_document("D1").fields;
    assert_eq!(
        fields.get("juridiction"),
        Some(&Value::Null),
        "the key stays, with a null value"
    );
    let snapshot = fixture.store.snapshot();
    let present = snapshot
        .filters()
        .keyword("juridiction")
        .map(|keyword| keyword.present().clone())
        .unwrap_or_default();
    assert_eq!(
        present,
        RoaringBitmap::from_iter([2]),
        "a null is absent for `exists`, so only D2 stays present"
    );
}

#[test]
fn patch_refuses_an_unknown_document_and_writes_nothing() {
    // ADR-0060, decision 4: one unknown `doc_id` refuses the whole file, so the
    // valid entry of the same file is not applied either.
    let mut fixture = Fixture::new();
    let before = capture(&fixture);

    let error = fixture
        .apply(&[("D1", json!({ "rank": 9 })), ("D999", json!({ "rank": 9 }))])
        .err()
        .unwrap_or_else(|| panic!("an unknown document must refuse the patch"));
    assert_eq!(error.code, ErrorCode::NotFound);
    assert!(
        error.message.contains("D999"),
        "the message names the doc_id"
    );

    assert_unchanged(&fixture, &before);
}

#[test]
fn a_duplicate_document_id_is_refused() {
    // ADR-0060, decision 4: a document appears once in a patch, so the result
    // does not depend on the order of the entries.
    let mut fixture = Fixture::new();
    let before = capture(&fixture);

    let error = fixture
        .apply(&[("D1", json!({ "rank": 2 })), ("D1", json!({ "rank": 3 }))])
        .err()
        .unwrap_or_else(|| panic!("a duplicate doc_id must be refused"));
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(error.message.contains("D1"));
    assert!(error.message.contains("twice"), "{}", error.message);

    assert_unchanged(&fixture, &before);
}

#[test]
fn patch_refuses_a_passage_field_a_reserved_name_and_an_unknown_field() {
    // ADR-0060, decision 5: only a document field of the schema is accepted. A
    // passage field, `doc_id`, `doc_hash` and an undeclared name are refused in
    // `invalid_request`, and the message names the field and the expected level.
    let mut fixture = Fixture::new();
    let before = capture(&fixture);

    for (name, expected) in [
        ("partie", "passage_fields"),
        ("doc_id", "reserved"),
        ("doc_hash", "reserved"),
        ("inconnu", "not declared"),
    ] {
        let error = fixture
            .apply(&[("D1", json!({ name: "x" }))])
            .err()
            .unwrap_or_else(|| panic!("the field {name} must be refused"));
        assert_eq!(error.code, ErrorCode::InvalidRequest, "field {name}");
        assert!(error.message.contains(name), "{}", error.message);
        assert!(
            error.message.contains("document_fields") || error.message.contains(expected),
            "the message of {name} must name the expected level: {}",
            error.message
        );
    }

    assert_unchanged(&fixture, &before);
}

#[test]
fn patch_accepts_a_field_that_is_not_filterable_or_not_returned() {
    // ADR-0060, decision 5: the value of a field that is not filterable or not
    // returned is stored, as an attach stores it. The patch does not have to
    // change the configuration first.
    let mut fixture = Fixture::new();
    fixture
        .apply(&[(
            "D1",
            json!({ "sensitive": "updated", "internal": "updated" }),
        )])
        .unwrap_or_else(|error| panic!("the patch must apply: {}", error.message));

    let fields = fixture.read_document("D1").fields;
    assert_eq!(fields["sensitive"], json!("updated"));
    assert_eq!(fields["internal"], json!("updated"));
}

#[test]
fn a_patch_never_echoes_a_field_value() {
    // ADR-0060, decisions 5 and 6: the message names the field and the expected
    // type, never the value, which may hold a document text (rule 11 of
    // `AGENTS.md`).
    let mut fixture = Fixture::new();
    let before = capture(&fixture);

    let error = fixture
        .apply(&[("D1", json!({ "rank": "une-valeur-confidentielle" }))])
        .err()
        .unwrap_or_else(|| panic!("a string in an integer field must be refused"));
    assert!(
        !error.message.contains("une-valeur-confidentielle"),
        "the message must not echo the value: {}",
        error.message
    );
    assert!(error.message.contains("rank"));
    assert!(error.message.contains("integer"), "{}", error.message);

    let error = fixture
        .apply(&[("D1", json!({ "published": "2021-02-30" }))])
        .err()
        .unwrap_or_else(|| panic!("a date out of the calendar must be refused"));
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(
        !error.message.contains("2021-02-30"),
        "the message must not echo the value: {}",
        error.message
    );
    assert!(error.message.contains("YYYY-MM-DD"), "{}", error.message);

    assert_unchanged(&fixture, &before);
}

#[test]
fn a_patch_without_effect_creates_no_generation() {
    // ADR-0060, decision 9: nothing visibly changes, so no generation, no
    // manifest and no snapshot. `fields: {}` is a no-op too (decision 2).
    let mut fixture = Fixture::new();
    let before = capture(&fixture);

    let report = fixture
        .apply(&[
            ("D1", json!({ "juridiction": "judiciaire" })),
            ("D2", json!({})),
        ])
        .unwrap_or_else(|error| panic!("the patch must apply: {}", error.message));
    assert_eq!(report.updated, 0);
    assert_eq!(report.unchanged, 2);
    assert_eq!(
        report.generation.get(),
        1,
        "the current generation is reported"
    );

    assert_unchanged(&fixture, &before);
}

#[test]
fn patch_refuses_more_entries_than_the_limit() {
    // ADR-0060, decision 7: 10 000 entries at most, refused in
    // `payload_too_large` before any write.
    let mut fixture = Fixture::new();
    let before = capture(&fixture);

    let patches: Vec<DocumentPatch> = (0..=patch::MAX_ENTRIES)
        .map(|index| entry(&format!("D{index}"), json!({ "rank": 1 })))
        .collect();
    let error = patch::apply(
        &mut fixture.store,
        &fixture.schema,
        PatchSource::File,
        &patches,
    )
    .map_err(ErrorView::of)
    .err()
    .unwrap_or_else(|| panic!("more than the limit must be refused"));
    assert_eq!(error.code, ErrorCode::PayloadTooLarge);
    assert!(
        error.message.contains(&patch::MAX_ENTRIES.to_string()),
        "{}",
        error.message
    );

    assert_unchanged(&fixture, &before);
}

#[test]
fn patch_refuses_more_bytes_than_the_limit() {
    // ADR-0060, decision 7: 32 MiB at most, refused in `payload_too_large`
    // before the bytes are parsed. The buffer is never a real patch, so the
    // check on the length is what the test pins.
    let oversized = vec![b'\n'; patch::MAX_BYTES as usize + 1];
    let error = patch::parse(&oversized)
        .err()
        .unwrap_or_else(|| panic!("a patch above the byte limit must be refused"));
    assert_eq!(error.code(), ErrorCode::PayloadTooLarge);
    assert!(
        error.message().contains(&patch::MAX_BYTES.to_string()),
        "{}",
        error.message()
    );
}

#[test]
fn read_file_refuses_an_oversized_file_before_loading_it() {
    // ADR-0060, decision 7, at the boundary of the CLI: `read_file` refuses on
    // the size the file system reports, before reading anything. The file is
    // sparse — `set_len` writes no block — so the test costs no real 32 MiB.
    let directory = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
    let path = directory.path().join("patch.jsonl");
    let file = fs::File::create(&path)
        .unwrap_or_else(|error| panic!("the patch file must be creatable: {error}"));
    file.set_len(patch::MAX_BYTES + 1)
        .unwrap_or_else(|error| panic!("the patch file must be resized: {error}"));

    let error = patch::read_file(&path)
        .err()
        .unwrap_or_else(|| panic!("an oversized patch file must be refused"));
    assert_eq!(error.code(), ErrorCode::PayloadTooLarge);
    assert!(
        error.message().contains(&patch::MAX_BYTES.to_string()),
        "{}",
        error.message()
    );
}

#[test]
fn the_report_counts_updated_unchanged_and_the_generation() {
    // ADR-0060, decision 8: two counts by document, and the generation visible
    // after the call.
    let mut fixture = Fixture::new();
    let report = fixture
        .apply(&[
            ("D1", json!({ "juridiction": "administratif" })),
            ("D2", json!({ "juridiction": "judiciaire" })),
        ])
        .unwrap_or_else(|error| panic!("the patch must apply: {}", error.message));

    assert_eq!(report.updated, 1, "only D1 changes");
    assert_eq!(report.unchanged, 1, "D2 already holds the value");
    assert_eq!(report.generation.get(), 2);
}

#[test]
fn a_patch_is_read_from_jsonl_lines() {
    // ADR-0060, decision 1: the CLI reads one `{doc_id, fields}` object per line.
    // The parser refuses a line that is not that object, with its number, and
    // stops above the entry limit.
    let patches = patch::parse(
        br#"{"doc_id":"D1","fields":{"juridiction":"administratif"}}
{"doc_id":"D2","fields":{}}
"#,
    )
    .unwrap_or_else(|error| panic!("a well-formed JSONL patch must parse: {error}"));
    assert_eq!(patches.len(), 2);
    assert_eq!(patches[1].doc_id, "D2");
    assert!(patches[1].fields.is_empty());

    let error = patch::parse(br#"{"doc_id":"D1"}"#)
        .err()
        .unwrap_or_else(|| panic!("an entry without fields must be refused"));
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    let error = patch::parse(b"line 2\n")
        .err()
        .unwrap_or_else(|| panic!("a line that is not an object must be refused"));
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    assert!(error.message().contains("line 1"), "{}", error.message());
}
