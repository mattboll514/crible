// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `INV-07` and `INV-04` on the created-passage step of `FilterIndex::after_commit`,
//! the step the task `CORRIGER-MEMOIRE-APRES-COMMIT` bounds.
//!
//! References: `docs/phases/P10-validation-a-l-echelle-reelle.md` (task
//! `CORRIGER-MEMOIRE-APRES-COMMIT`), `docs/08-invariants-et-tests.md` (INV-07,
//! INV-04), `docs/03-recherche.md` ("Filtres → pool"), ADR-0018, ADR-0035,
//! ADR-0060 and ADR-0066.
//!
//! # What this file adds
//!
//! `FilterIndex::insert_created_passages` reads the `documents` row of every
//! created passage and keeps one value per distinct `doc_id` for the whole
//! step. The task bounds that cache, or replaces it by one lookup per document
//! by primary key. The correction may change how many rows it holds, never the
//! structures it derives: the whole `FilterIndex` after the commit must still
//! equal `FilterIndex::from_records`, and the filters must still return what SQL
//! returns.
//!
//! The existing tests already cover the cost of `FieldData::clear`, a large
//! `replace_all`, many distinct document values and the null, absent and
//! explicit-`null` states. They all list the passages of one document **in
//! ascending `iid` order, grouped by document**. A bounded cache that assumes
//! that grouping would go unnoticed. This file therefore pins the cases the
//! `insert_created_passages` path can get wrong:
//!
//! | Test | Criterion or invariant |
//! |---|---|
//! | `inv_07_created_passages_of_one_document_interleaved_with_another_keep_their_document_fields` | INV-07: the passages of one document arrive non-contiguous in `CommitDelta.created`, in an arbitrary order, and the document fields are of every type, one document carrying explicit nulls and one carrying no document field at all |
//! | `inv_07_document_rewritten_with_new_passages_in_the_same_generation_keeps_the_new_values` | INV-07: a replacement — the `documents` row changes while new passages are created for it — gives the new values to the created passages **and** to the passages the document already had that are not in `created` |
//! | `inv_07_metadata_patch_after_a_generation_that_created_passages_keeps_the_derived_structures` | INV-07: a metadata patch (ADR-0060) after a generation that created passages refreshes the inherited values, the dead passages included, and leaves the structures equal to a full rebuild |
//! | `inv_04_reopening_after_created_passages_keeps_the_committed_generation_and_its_filters` | INV-04: a restart publishes the last committed generation, with the filters the commit published |
//!
//! # The dataset
//!
//! Three documents, nine passages, small on purpose: every test runs in a
//! second or two in a debug build and measures neither time nor memory. The
//! schema declares a document field and a passage field of each of the five
//! types (`keyword`, `integer`, `float`, `date`, `bool`).
//!
//! | iid | document | document fields | passage fields |
//! |---|---|---|---|
//! | 0 | `doc-a` | `fonds` LEGI, `annee` 2020, `score` 1.25, `vigueur` 2020-01-01, `archive` false | `partie` corps, `rang` 0, `poids` 1.25, `jour` 2020-01-01, `principal` true |
//! | 2 | `doc-a` | same row | `partie` null, `rang` 1, `poids` null, `jour` null, `principal` false |
//! | 5 | `doc-a` | same row | `partie` absent, `rang` 2, `poids` 4.0, `jour` 2017-05-05, `principal` false |
//! | 1 | `doc-b` | `fonds` null, `annee` 2021, `score` null, `vigueur` null, `archive` null | `partie` moyens, `rang` 0, `poids` 2.5, `jour` 2021-02-02, `principal` false |
//! | 4 | `doc-b` | same row | `partie` corps, `rang` 1, `poids` 0.5, `jour` 2018-04-04, `principal` true |
//! | 3 | `doc-c` | no document field at all (`{}`) | `partie` faits, `rang` 0, `poids` 3.75, `jour` 2019-03-03, `principal` true |
//!
//! Generation 2 replaces `doc-a` — its row changes and the passages 6 and 8 are
//! created for it — creates the passage 7 for `doc-b` and tombstones the passage
//! numbered 1. The passages of one document are therefore non-contiguous after
//! the `iid` sort the step applies (`doc-a`: 0, 2, 5, 6, 8; `doc-b`: 1, 4, 7),
//! which is the shape the bounded cache must survive.

mod common;

use crible_core::filter::Filter;
use crible_core::ids::{Generation, Iid};
use crible_search::pool::Pool;
use crible_store::filters_index::FilterIndex;
use crible_store::patch::{DocumentPatch, PatchSource};
use serde_json::{Map, Value, json};

use common::{Fixture, document, fixture_schema, json_object, passage, sorted, sql_matches};

/// The document a fixture passage belongs to.
fn doc_of(iid: u32) -> &'static str {
    match iid {
        0 | 2 | 5 | 6 | 8 => "doc-a",
        1 | 4 | 7 => "doc-b",
        3 => "doc-c",
        other => panic!("the fixture has no passage {other}"),
    }
}

/// Rank of a fixture passage inside its document.
fn ord_of(iid: u32) -> u32 {
    match iid {
        0 => 0,
        2 => 1,
        5 => 2,
        6 => 3,
        8 => 4,
        1 => 0,
        4 => 1,
        7 => 2,
        3 => 0,
        other => panic!("the fixture has no passage {other}"),
    }
}

/// Passage-level fields of passage `iid`, one of each type; `iid` 2 carries
/// explicit nulls and `iid` 5 omits `partie`.
fn passage_fields(iid: u32) -> Value {
    match iid {
        0 => json_object(vec![
            ("partie", json!("corps")),
            ("rang", json!(0)),
            ("poids", json!(1.25)),
            ("jour", json!("2020-01-01")),
            ("principal", json!(true)),
        ]),
        1 => json_object(vec![
            ("partie", json!("moyens")),
            ("rang", json!(0)),
            ("poids", json!(2.5)),
            ("jour", json!("2021-02-02")),
            ("principal", json!(false)),
        ]),
        2 => json_object(vec![
            ("partie", json!(null)),
            ("rang", json!(1)),
            ("poids", json!(null)),
            ("jour", json!(null)),
            ("principal", json!(false)),
        ]),
        3 => json_object(vec![
            ("partie", json!("faits")),
            ("rang", json!(0)),
            ("poids", json!(3.75)),
            ("jour", json!("2019-03-03")),
            ("principal", json!(true)),
        ]),
        4 => json_object(vec![
            ("partie", json!("corps")),
            ("rang", json!(1)),
            ("poids", json!(0.5)),
            ("jour", json!("2018-04-04")),
            ("principal", json!(true)),
        ]),
        5 => json_object(vec![
            ("rang", json!(2)),
            ("poids", json!(4.0)),
            ("jour", json!("2017-05-05")),
            ("principal", json!(false)),
        ]),
        6 => json_object(vec![
            ("partie", json!("annexe")),
            ("rang", json!(3)),
            ("poids", json!(1.5)),
            ("jour", json!("2022-07-07")),
            ("principal", json!(true)),
        ]),
        7 => json_object(vec![
            ("partie", json!("faits")),
            ("rang", json!(2)),
            ("poids", json!(2.0)),
            ("jour", json!("2023-08-08")),
            ("principal", json!(false)),
        ]),
        8 => json_object(vec![
            ("partie", json!(null)),
            ("rang", json!(4)),
            ("poids", json!(null)),
            ("jour", json!(null)),
            ("principal", json!(true)),
        ]),
        other => panic!("the fixture has no passage {other}"),
    }
}

/// Document-level fields of `doc-a`, one of each type.
fn doc_a_fields() -> Value {
    json_object(vec![
        ("fonds", json!("LEGI")),
        ("annee", json!(2020)),
        ("score", json!(1.25)),
        ("vigueur", json!("2020-01-01")),
        ("archive", json!(false)),
    ])
}

/// Document-level fields of `doc-b`: explicit nulls for four of the five
/// fields, one value for the integer.
fn doc_b_fields() -> Value {
    json_object(vec![
        ("fonds", json!(null)),
        ("annee", json!(2021)),
        ("score", json!(null)),
        ("vigueur", json!(null)),
        ("archive", json!(null)),
    ])
}

/// Document-level fields of `doc-c`: the row carries no declared field at all.
fn doc_c_fields() -> Value {
    json_object(vec![])
}

/// Document-level fields of `doc-a` in generation 2, all five changed.
fn doc_a_fields_second_generation() -> Value {
    json_object(vec![
        ("fonds", json!("JORF")),
        ("annee", json!(2001)),
        ("score", json!(9.5)),
        ("vigueur", json!("1999-12-31")),
        ("archive", json!(true)),
    ])
}

/// Generation 1: three documents, six passages, in an order that is neither
/// `iid` order nor grouped by document.
fn first_generation(fixture: &mut Fixture) {
    let documents = vec![
        document("doc-a", doc_a_fields(), 1),
        document("doc-b", doc_b_fields(), 1),
        document("doc-c", doc_c_fields(), 1),
    ];
    // The delta of a commit keeps the order of `GenerationChange::passages`;
    // `insert_created_passages` sorts it by `iid`. Both orders are arbitrary
    // here, which is the point.
    let order = [5_u32, 3, 0, 4, 1, 2];
    let passages = order
        .iter()
        .map(|iid| passage(*iid, doc_of(*iid), ord_of(*iid), passage_fields(*iid), 1))
        .collect();
    fixture.commit(documents, passages, Vec::new());
}

/// Generation 2: `doc-a` is replaced (its row changes, the passages 6 and 8 are
/// created for it), `doc-b` gains the passage 7 and loses the passage 1.
fn second_generation(fixture: &mut Fixture) {
    let documents = vec![document("doc-a", doc_a_fields_second_generation(), 2)];
    // `doc-a` already owns 0, 2 and 5; its new passages 6 and 8 are interleaved
    // with `doc-b`'s 7 in the delta and after the sort.
    let order = [7_u32, 6, 8];
    let passages = order
        .iter()
        .map(|iid| passage(*iid, doc_of(*iid), ord_of(*iid), passage_fields(*iid), 2))
        .collect();
    fixture.commit(documents, passages, vec![Iid::new(1)]);
}

/// Full rebuild of the structures from `records.sqlite`, the reference
/// `after_commit` must equal.
fn rebuilt(fixture: &Fixture) -> FilterIndex {
    FilterIndex::from_records(&fixture.records(), fixture.schema())
        .unwrap_or_else(|error| panic!("the structures must rebuild from SQLite: {error}"))
}

/// Iids the pool of `raw` holds, checked against the SQL oracle of the same
/// filter on the same `records.sqlite`.
///
/// Every call therefore checks both halves of INV-07: the filter turned into a
/// bitmap equals the SQL evaluation of the same filter, and the pool is a
/// subset of the live passages.
fn matches(fixture: &Fixture, raw: &str) -> Vec<u32> {
    let filter = Filter::from_json_str(raw)
        .unwrap_or_else(|error| panic!("the filter must parse: {error}: {raw}"));
    filter
        .validate(fixture.schema())
        .unwrap_or_else(|error| panic!("the filter must validate: {error}: {raw}"));
    let snapshot = fixture.snapshot();
    let pool = Pool::evaluate(snapshot.filters(), &filter)
        .unwrap_or_else(|error| panic!("the filter must evaluate: {error}: {raw}"));
    let oracle = sql_matches(&fixture.records(), &filter, fixture.schema());
    assert_eq!(
        sorted(pool.bitmap()),
        oracle,
        "a filter turned into a bitmap must equal the SQL evaluation of the same filter: {raw}"
    );
    assert!(
        pool.bitmap().is_subset(snapshot.filters().live()),
        "the pool must be a subset of the live passages: {raw}"
    );
    oracle
}

/// The passages of one document come non-contiguous in `CommitDelta.created`,
/// in an arbitrary order, and the step still gives each one the document-level
/// values of its own document.
///
/// The structures must equal a full rebuild, and the filters of every declared
/// type must equal the SQL evaluation: a bounded cache that assumed the
/// passages of a document arrive grouped, or that kept the row of one document
/// beyond its last passage, would give `doc-b` the values of `doc-a` (or the
/// nulls of `doc-b` to `doc-a`), which both checks catch.
#[test]
fn inv_07_created_passages_of_one_document_interleaved_with_another_keep_their_document_fields() {
    let mut fixture = Fixture::create(fixture_schema());
    first_generation(&mut fixture);

    let snapshot = fixture.snapshot();
    assert_eq!(
        snapshot.filters(),
        &rebuilt(&fixture),
        "the structures derived from an interleaved creation must equal a full rebuild (INV-07)"
    );
    assert_eq!(
        snapshot.live_iids().count(),
        6,
        "generation 1 creates six live passages"
    );

    assert_eq!(
        matches(&fixture, r#"{"fonds":"LEGI"}"#),
        vec![0, 2, 5],
        "the three passages of doc-a inherit its document row"
    );
    assert_eq!(
        matches(&fixture, r#"{"fonds":{"exists":false}}"#),
        vec![1, 3, 4],
        "doc-b stores an explicit null and doc-c carries no key: both are absent (ADR-0035)"
    );
    assert_eq!(
        matches(&fixture, r#"{"annee":{"gte":2021}}"#),
        vec![1, 4],
        "the integer column of doc-b follows its own row, not the row of its neighbour"
    );
    assert_eq!(
        matches(&fixture, r#"{"score":{"lte":1.25}}"#),
        vec![0, 2, 5],
        "the float column of doc-a follows its own row"
    );
    assert_eq!(
        matches(&fixture, r#"{"vigueur":{"gte":"2020-01-01"}}"#),
        vec![0, 2, 5],
        "the date column of doc-a follows its own row"
    );
    assert_eq!(
        matches(&fixture, r#"{"archive":false}"#),
        vec![0, 2, 5],
        "the boolean field of doc-a follows its own row"
    );
    assert_eq!(
        matches(&fixture, r#"{"principal":true}"#),
        vec![0, 3, 4],
        "a passage field belongs to the passage, not to the document"
    );
    assert_eq!(
        matches(&fixture, r#"{"partie":{"exists":false}}"#),
        vec![2, 5],
        "an explicit null and an omitted passage field are both absent (ADR-0035)"
    );
    assert_eq!(
        matches(&fixture, r#"{"rang":{"gte":2}}"#),
        vec![5],
        "an interval on a passage column reads the passage rows"
    );
    assert_eq!(
        matches(&fixture, r#"{"$not":{"fonds":"LEGI"}}"#),
        vec![1, 3, 4],
        "the complement is taken over the live passages, whatever the interleaving"
    );
    assert_eq!(
        matches(
            &fixture,
            r#"{"$and":[{"archive":false},{"rang":{"lte":1}}]}"#
        ),
        vec![0, 2],
        "a conjunction of a document field and a passage field follows the two rows"
    );
}

/// A replacement — the `documents` row changes while new passages are created
/// for the same document — gives the new values to the created passages and to
/// the passages the document already had that are not in `created`.
///
/// The step 1 of `after_commit` inserts the created passages with the row the
/// transaction wrote; step 3 refreshes the other passages of the same document
/// and must skip the created ones. A cache that read the row of `doc-a` once and
/// reused it, or a step 3 that refreshed the created passages, would leave the
/// old `LEGI` / 2020 / 1.25 values on some passages: the equality with a full
/// rebuild and the SQL oracle both catch it.
#[test]
fn inv_07_document_rewritten_with_new_passages_in_the_same_generation_keeps_the_new_values() {
    let mut fixture = Fixture::create(fixture_schema());
    first_generation(&mut fixture);
    second_generation(&mut fixture);

    assert_eq!(fixture.snapshot().generation(), Generation::new(2));
    assert_eq!(
        fixture.snapshot().filters(),
        &rebuilt(&fixture),
        "a replacement whose new passages arrive interleaved must equal a full rebuild (INV-07)"
    );

    assert_eq!(
        matches(&fixture, r#"{"fonds":"JORF"}"#),
        vec![0, 2, 5, 6, 8],
        "the passages doc-a already had and the ones created in the same generation all carry the new value"
    );
    assert_eq!(
        matches(&fixture, r#"{"fonds":"LEGI"}"#),
        Vec::<u32>::new(),
        "no passage of the replaced document keeps the old keyword"
    );
    assert_eq!(
        matches(&fixture, r#"{"annee":{"gte":2021}}"#),
        vec![4, 7],
        "doc-b keeps its own integer value; its passage 1 is tombstoned"
    );
    assert_eq!(
        matches(&fixture, r#"{"score":{"gte":9.5}}"#),
        vec![0, 2, 5, 6, 8],
        "the float column of the replaced document follows the new row"
    );
    assert_eq!(
        matches(&fixture, r#"{"vigueur":{"lte":"1999-12-31"}}"#),
        vec![0, 2, 5, 6, 8],
        "the date column of the replaced document follows the new row"
    );
    assert_eq!(
        matches(&fixture, r#"{"archive":true}"#),
        vec![0, 2, 5, 6, 8],
        "the boolean field of the replaced document follows the new row"
    );
    assert_eq!(
        matches(&fixture, r#"{"$not":{"fonds":"JORF"}}"#),
        vec![3, 4, 7],
        "the created passages of doc-a are in the pool the complement is built from"
    );
}

/// A metadata patch that follows a generation which created passages refreshes
/// the inherited values of every passage of the document, the dead ones
/// included, and leaves the derived structures equal to a full rebuild.
///
/// `doc-b` is patched after generation 2: its live passages 4 and 7 and its dead
/// passage 1 must all carry the new values, because a rebuild gives the
/// document-level fields to a dead passage too. The patch does not touch
/// `insert_created_passages`, but it reads the same `documents` rows the cache
/// holds, so a cache that outlived its step would be visible here.
#[test]
fn inv_07_metadata_patch_after_a_generation_that_created_passages_keeps_the_derived_structures() {
    let mut fixture = Fixture::create(fixture_schema());
    first_generation(&mut fixture);
    second_generation(&mut fixture);

    let live_before: Vec<u32> = fixture
        .snapshot()
        .live_iids()
        .map(|iid| iid.get())
        .collect();
    let patches = vec![DocumentPatch {
        doc_id: "doc-b".to_owned(),
        fields: Map::from_iter([
            ("fonds".to_owned(), json!("JORF")),
            ("annee".to_owned(), json!(null)),
            ("score".to_owned(), json!(7.5)),
            ("vigueur".to_owned(), json!("2005-06-06")),
            ("archive".to_owned(), json!(true)),
        ]),
    }];
    let report = crible_store::patch::apply(
        fixture.store_mut(),
        &fixture_schema(),
        PatchSource::File,
        &patches,
    )
    .unwrap_or_else(|error| panic!("the metadata patch of doc-b must succeed: {error}"));

    assert_eq!(report.updated, 1, "exactly one document must change");
    assert_eq!(
        report.generation,
        Generation::new(3),
        "a patch that changes a document commits the next generation (ADR-0060)"
    );
    assert_eq!(
        fixture.snapshot().filters(),
        &rebuilt(&fixture),
        "the structures after a metadata patch must equal a full rebuild (INV-07)"
    );

    assert_eq!(
        matches(&fixture, r#"{"fonds":"JORF"}"#),
        vec![0, 2, 4, 5, 6, 7, 8],
        "the live passages of doc-b inherit the patched keyword; those of doc-a keep theirs"
    );
    assert_eq!(
        matches(&fixture, r#"{"annee":{"exists":false}}"#),
        vec![3, 4, 7],
        "the patch set doc-b's integer to a null, which is absent (ADR-0035)"
    );
    assert_eq!(
        matches(&fixture, r#"{"score":{"eq":7.5}}"#),
        vec![4, 7],
        "the patched float reaches the passages doc-b already had and the one created in generation 2"
    );
    assert_eq!(
        matches(&fixture, r#"{"vigueur":{"gte":"2005-01-01"}}"#),
        vec![4, 7],
        "the patched date reaches every live passage of the document"
    );
    assert_eq!(
        matches(&fixture, r#"{"archive":true}"#),
        vec![0, 2, 4, 5, 6, 7, 8],
        "the patched boolean reaches doc-b and leaves doc-a alone"
    );

    // The dead passage 1 keeps its row and inherits the patched values too: a
    // rebuild reads it, so the derived structures must carry them.
    let filters = fixture.snapshot();
    let inherited = filters
        .filters()
        .keyword("fonds")
        .unwrap_or_else(|| panic!("fonds is a keyword field of the schema"));
    let jorf = inherited
        .bitmap("JORF")
        .unwrap_or_else(|| panic!("only doc-a carried JORF before the patch"));
    assert!(
        jorf.contains(1),
        "the dead passage 1 inherits the patched value, as a rebuild gives it to it"
    );
    let live_after: Vec<u32> = fixture
        .snapshot()
        .live_iids()
        .map(|iid| iid.get())
        .collect();
    assert_eq!(
        live_after, live_before,
        "a metadata patch must not change which passages are alive"
    );
}

/// `INV-04`: a restart publishes the last committed generation, with the filters
/// the commit published, after a generation whose created passages arrived
/// interleaved.
///
/// The restart rebuilds from `records.sqlite`: the document rows the transaction
/// wrote must be the ones the rebuild reads, whatever the cache of the commit
/// held.
#[test]
fn inv_04_reopening_after_created_passages_keeps_the_committed_generation_and_its_filters() {
    let mut fixture = Fixture::create(fixture_schema());
    first_generation(&mut fixture);
    second_generation(&mut fixture);

    let published = fixture.snapshot();
    assert_eq!(
        published.generation(),
        Generation::new(2),
        "the second generation is the last committed one"
    );
    let published_filters = published.filters().clone();
    let published_live: Vec<u32> = published.live_iids().map(|iid| iid.get()).collect();
    drop(published);

    let reopened = fixture.reopen();
    let snapshot = reopened.snapshot();
    assert_eq!(
        snapshot.generation(),
        Generation::new(2),
        "INV-04: a restart publishes the last committed generation"
    );
    assert_eq!(
        snapshot.filters(),
        &published_filters,
        "a restart must rebuild the filters the commit had published, not another state"
    );
    assert_eq!(
        snapshot.filters(),
        &rebuilt(&reopened),
        "the rebuilt filters must equal `FilterIndex::from_records` on the same reference state"
    );
    let live: Vec<u32> = snapshot.live_iids().map(|iid| iid.get()).collect();
    assert_eq!(
        live, published_live,
        "a restart must see the same live passages as the commit published"
    );
}
