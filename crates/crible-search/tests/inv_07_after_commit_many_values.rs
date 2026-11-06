// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `INV-07` on a commit of many documents and many distinct document values.
//!
//! References: `docs/phases/P10-validation-a-l-echelle-reelle.md` (task
//! `CORRIGER-BLOCAGE-APRES-COMMIT`), `docs/08-invariants-et-tests.md` (INV-07),
//! `docs/03-recherche.md` ("Filtres → pool"), `docs/05-api.md` ("Grammaire des
//! filtres"), ADR-0018, ADR-0035 and ADR-0066.
//!
//! # What this test adds
//!
//! `inv_07_filter_bitmap_matches_sql.rs` checks the filter → bitmap equality on
//! 40 documents. The task `CORRIGER-BLOCAGE-APRES-COMMIT` touches `FieldData::clear`,
//! whose cost grows with the number of passages and with the number of distinct
//! values of the document-level fields. This file therefore repeats the property
//! on a larger state: 300 documents with 30 distinct `fonds` values, 40 distinct
//! `annee` values, 20 distinct dates and 20 distinct scores, plus the passage
//! fields of every type, then a second generation that rewrites 60 documents
//! (the `clear` + insert path of an inherited value) and tombstones 40 passages.
//!
//! Two checks run on that state:
//!
//! 1. the structures `after_commit` derived equal `FilterIndex::from_records` on
//!    the same reference state — the guard ADR-0066 names, here on a state with
//!    many distinct values;
//! 2. a fixed battery of filters covering `eq`, `in`, `ne`, `nin`, `exists`, the
//!    four interval operators, `$not`, `$and` and `$or` on both levels, then a
//!    seeded property of 256 generated filters, each compared to the SQL oracle
//!    of [`common::sql_matches`] on `records.sqlite`.
//!
//! The test measures nothing: it asserts the result the correction must
//! preserve, so it passes on the code of before the correction, which is
//! correct but slow.

mod common;

use crible_core::config::{FieldType, Schema};
use crible_core::filter::Filter;
use crible_store::filters_index::FilterIndex;
use proptest::prelude::*;
use proptest::test_runner::{Config, RngSeed, TestCaseError, TestRunner};
use serde_json::{Map, Value, json};

use common::{Fixture, document, fixture_schema, passage, sorted, sql_matches};

/// Documents of the reference dataset.
const DOCUMENT_COUNT: usize = 300;
/// Passages per document.
const PASSAGES_PER_DOCUMENT: usize = 2;
/// Distinct values of the document-level keyword `fonds`.
const FONDS_VALUES: usize = 30;
/// Seed of the dataset, fixed so a run is reproducible.
const DATASET_SEED: u64 = 0x5EED_0007;
/// Seed of the filter generator, fixed so a run is reproducible.
const PROPERTY_SEED: u64 = 0x494E_0007;
/// Filters the property checks (INV-07 asks for 256 at least).
const CASES: u32 = 256;

/// Field type of the fixture schema, as the generator needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Keyword,
    Integer,
    Float,
    Date,
    Bool,
}

/// Every field of the fixture schema, at both levels.
const FIELDS: &[(&str, Kind)] = &[
    ("fonds", Kind::Keyword),
    ("annee", Kind::Integer),
    ("score", Kind::Float),
    ("vigueur", Kind::Date),
    ("archive", Kind::Bool),
    ("partie", Kind::Keyword),
    ("rang", Kind::Integer),
    ("poids", Kind::Float),
    ("jour", Kind::Date),
    ("principal", Kind::Bool),
];

fn kind_of(field_type: FieldType) -> Kind {
    match field_type {
        FieldType::Keyword => Kind::Keyword,
        FieldType::Integer => Kind::Integer,
        FieldType::Float => Kind::Float,
        FieldType::Date => Kind::Date,
        FieldType::Bool => Kind::Bool,
    }
}

// --- The reference dataset ---

/// A small deterministic generator, so the fixture never reads the clock nor
/// the ordering of a hash map.
struct Draw(u64);

impl Draw {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

fn fonds_value(index: usize) -> String {
    format!("fonds-{index:03}")
}

fn partie_value(index: usize) -> String {
    ["corps", "moyens", "faits", "annexe"][index % 4].to_owned()
}

fn date_value(index: usize) -> String {
    format!("{:04}-{:02}-01", 2000 + index % 20, 1 + index % 12)
}

fn document_fields(draw: &mut Draw, salt: usize) -> Value {
    let mut fields = Map::new();
    if draw.below(5) != 0 {
        fields.insert(
            "fonds".to_owned(),
            json!(fonds_value(
                (draw.below(FONDS_VALUES) + salt) % FONDS_VALUES
            )),
        );
    }
    if draw.below(5) != 0 {
        fields.insert(
            "annee".to_owned(),
            json!(1990 + ((draw.below(40) + salt) % 40) as i64),
        );
    }
    if draw.below(5) != 0 {
        fields.insert(
            "score".to_owned(),
            json!(((draw.below(20) + salt) % 20) as f64 / 4.0),
        );
    }
    if draw.below(5) != 0 {
        fields.insert(
            "vigueur".to_owned(),
            json!(date_value(draw.below(20) + salt)),
        );
    }
    if draw.below(5) != 0 {
        fields.insert("archive".to_owned(), json!(draw.below(2) == 0));
    }
    Value::Object(fields)
}

fn passage_fields(draw: &mut Draw) -> Value {
    let mut fields = Map::new();
    if draw.below(6) != 0 {
        fields.insert("partie".to_owned(), json!(partie_value(draw.below(12))));
    }
    fields.insert("rang".to_owned(), json!(draw.below(10) as i64));
    fields.insert("poids".to_owned(), json!(draw.below(12) as f64 / 3.0));
    fields.insert("jour".to_owned(), json!(date_value(draw.below(12))));
    fields.insert("principal".to_owned(), json!(draw.below(2) == 0));
    Value::Object(fields)
}

/// Builds the index of the run: generation 1 with every document, then a
/// generation 2 that rewrites 60 documents and tombstones 40 passages.
fn reference_index() -> Fixture {
    let mut fixture = Fixture::create(fixture_schema());
    let mut draw = Draw::new(DATASET_SEED);
    let doc_ids: Vec<String> = (0..DOCUMENT_COUNT)
        .map(|index| format!("doc-{index:04}"))
        .collect();

    let documents: Vec<_> = doc_ids
        .iter()
        .map(|doc_id| document(doc_id, document_fields(&mut draw, 0), 1))
        .collect();
    let mut passages = Vec::new();
    for (index, doc_id) in doc_ids.iter().enumerate() {
        for ord in 0..PASSAGES_PER_DOCUMENT {
            let iid = (index * PASSAGES_PER_DOCUMENT + ord) as u32;
            passages.push(passage(
                iid,
                doc_id,
                ord as u32,
                passage_fields(&mut draw),
                1,
            ));
        }
    }
    fixture.commit(documents, passages, Vec::new());

    let rewritten: Vec<_> = doc_ids
        .iter()
        .take(60)
        .map(|doc_id| document(doc_id, document_fields(&mut draw, 1), 2))
        .collect();
    let deletions = (0..40).map(|step| crible_core::ids::Iid::new(step * 5));
    fixture.commit(rewritten, Vec::new(), deletions.collect());

    fixture
}

// --- The SQL comparison ---

/// Checks one filter of the grammar against the structures and the oracle.
fn check(fixture: &Fixture, schema: &Schema, raw: &str) {
    let filter = Filter::from_json_str(raw)
        .unwrap_or_else(|error| panic!("the filter must parse: {error}: {raw}"));
    filter
        .validate(schema)
        .unwrap_or_else(|error| panic!("the filter must validate: {error}: {raw}"));
    let snapshot = fixture.snapshot();
    let pool = crible_search::pool::Pool::evaluate(snapshot.filters(), &filter)
        .unwrap_or_else(|error| panic!("the filter must evaluate: {error}: {raw}"));
    let expected = sql_matches(&fixture.records(), &filter, schema);
    assert_eq!(
        sorted(pool.bitmap()),
        expected,
        "a filter turned into a bitmap must equal the SQL evaluation of the same filter: {raw}"
    );
    assert!(
        pool.bitmap().is_subset(snapshot.filters().live()),
        "the pool must be a subset of the live passages: {raw}"
    );
}

/// The derived structures equal a full rebuild on the same reference state, and
/// every operator of the grammar gives the result SQL gives on that state.
///
/// The battery is fixed, so each operator is exercised whatever the property
/// draws; the property then varies the shapes (`$and`, `$or`, `$not`, several
/// clauses on one field and values no row carries).
#[test]
fn inv_07_after_commit_of_many_distinct_document_values_matches_sql() {
    let fixture = reference_index();
    let schema = fixture.schema();
    let snapshot = fixture.snapshot();
    assert_eq!(
        snapshot.live_iids().count(),
        DOCUMENT_COUNT * PASSAGES_PER_DOCUMENT - 40,
        "the reference state must carry the tombstones of generation 2; without them the \
         property would not see a derived live bitmap"
    );
    assert_eq!(
        snapshot.filters(),
        &FilterIndex::from_records(&fixture.records(), schema)
            .unwrap_or_else(|error| panic!("the structures must rebuild: {error}")),
        "the structures `after_commit` derived on a state with many distinct document values \
         must equal `FilterIndex::from_records` on the same reference state"
    );

    // One filter per operator, on a document-level and on a passage-level field.
    let battery = [
        r#"{"fonds":"fonds-003"}"#,
        r#"{"fonds":{"eq":"fonds-007"}}"#,
        r#"{"fonds":{"in":["fonds-003","fonds-017","absent"]}}"#,
        r#"{"fonds":{"ne":"fonds-003"}}"#,
        r#"{"fonds":{"nin":["fonds-003","fonds-017"]}}"#,
        r#"{"fonds":{"exists":true}}"#,
        r#"{"fonds":{"exists":false}}"#,
        r#"{"annee":{"eq":2010}}"#,
        r#"{"annee":{"lt":2005}}"#,
        r#"{"annee":{"lte":2005}}"#,
        r#"{"annee":{"gt":2015}}"#,
        r#"{"annee":{"gte":2015}}"#,
        r#"{"annee":{"in":[2000,2010,2020]}}"#,
        r#"{"annee":{"exists":true}}"#,
        r#"{"score":{"gt":2.5}}"#,
        r#"{"score":{"lte":1.25}}"#,
        r#"{"vigueur":{"gte":"2010-01-01"}}"#,
        r#"{"vigueur":{"lt":"2005-06-01"}}"#,
        r#"{"vigueur":{"exists":false}}"#,
        r#"{"archive":true}"#,
        r#"{"archive":{"exists":false}}"#,
        r#"{"partie":"corps"}"#,
        r#"{"partie":{"in":["corps","faits"]}}"#,
        r#"{"partie":{"ne":"corps"}}"#,
        r#"{"partie":{"nin":["corps","faits"]}}"#,
        r#"{"partie":{"exists":false}}"#,
        r#"{"rang":{"gte":5}}"#,
        r#"{"rang":{"in":[0,2,4]}}"#,
        r#"{"poids":{"gt":2.0}}"#,
        r#"{"jour":{"lte":"2008-12-01"}}"#,
        r#"{"principal":false}"#,
        r#"{"$not":{"archive":true}}"#,
        r#"{"$not":{"fonds":{"exists":true}}}"#,
        r#"{"$and":[{"fonds":"fonds-003"},{"annee":{"gte":2005}}]}"#,
        r#"{"$or":[{"fonds":"fonds-003"},{"partie":"annexe"}]}"#,
        r#"{"$and":[{"fonds":{"ne":"fonds-003"}},{"$not":{"archive":true}}]}"#,
        r#"{"partie":"corps","rang":{"lt":5},"principal":true}"#,
        r#"{"$or":[{"$and":[{"fonds":"fonds-011"},{"annee":{"lt":2010}}]},{"$not":{"jour":{"exists":true}}}]}"#,
    ];
    for raw in battery {
        check(&fixture, schema, raw);
    }

    let config = Config {
        cases: CASES,
        rng_seed: RngSeed::Fixed(PROPERTY_SEED),
        failure_persistence: None,
        ..Config::default()
    };
    let mut runner = TestRunner::new(config);
    // A property that never keeps a passage would compare two empty sets on
    // every case: count the outcomes that matter, so a broken dataset or
    // generator cannot make the run vacuous.
    let kept = std::cell::Cell::new(0_u32);
    let emptied = std::cell::Cell::new(0_u32);
    runner
        .run(&filter(2).prop_map(|value| value.to_string()), |raw| {
            let parsed = Filter::from_json_str(&raw).map_err(|error| {
                TestCaseError::fail(format!("the generated filter must parse: {error}: {raw}"))
            })?;
            parsed.validate(schema).map_err(|error| {
                TestCaseError::fail(format!(
                    "the generated filter must validate: {error}: {raw}"
                ))
            })?;
            let pool = crible_search::pool::Pool::evaluate(fixture.snapshot().filters(), &parsed)
                .map_err(|error| {
                TestCaseError::fail(format!("the filter must evaluate: {error}: {raw}"))
            })?;
            let expected = sql_matches(&fixture.records(), &parsed, schema);
            if expected.is_empty() {
                emptied.set(emptied.get() + 1);
            } else {
                kept.set(kept.get() + 1);
            }
            prop_assert_eq!(&sorted(pool.bitmap()), &expected, "filter {}", raw);
            Ok(())
        })
        .unwrap();
    assert!(
        kept.get() > 0 && emptied.get() > 0,
        "the property must meet both a filter that keeps passages and one that empties the \
         pool: {} kept, {} emptied",
        kept.get(),
        emptied.get()
    );
}

// --- The filter generator ---

fn operands(field: &'static str) -> BoxedStrategy<Value> {
    let values: Vec<Value> = match field {
        "fonds" => (0..FONDS_VALUES)
            .map(|index| json!(fonds_value(index)))
            .chain(std::iter::once(json!("absent")))
            .collect(),
        "partie" => (0..12)
            .map(|index| json!(partie_value(index)))
            .chain(std::iter::once(json!("absente")))
            .collect(),
        "annee" => (0..40)
            .map(|index| json!(1990 + index as i64))
            .chain(std::iter::once(json!(1800)))
            .collect(),
        "rang" => (0..10)
            .map(|index| json!(index as i64))
            .chain(std::iter::once(json!(99)))
            .collect(),
        "score" | "poids" => (0..20)
            .map(|index| json!(index as f64 / 4.0))
            .chain(std::iter::once(json!(9.5)))
            .collect(),
        "vigueur" | "jour" => (0..20)
            .map(date_value)
            .map(|date| json!(date))
            .chain(std::iter::once(json!("1800-01-01")))
            .collect(),
        "archive" | "principal" => vec![json!(true), json!(false)],
        other => panic!("the generator knows no field '{other}'"),
    };
    proptest::sample::select(values).boxed()
}

fn object(entries: Vec<(String, Value)>) -> Value {
    let mut object = Map::new();
    for (key, value) in entries {
        object.insert(key, value);
    }
    Value::Object(object)
}

fn condition_object(field: &'static str, operator: &'static str, operand: Value) -> Value {
    object(vec![(
        field.to_owned(),
        object(vec![(operator.to_owned(), operand)]),
    )])
}

fn operator_condition(
    field: &'static str,
    operator: &'static str,
    operands: BoxedStrategy<Value>,
) -> BoxedStrategy<Value> {
    operands
        .prop_map(move |operand| condition_object(field, operator, operand))
        .boxed()
}

fn exists_condition(field: &'static str) -> BoxedStrategy<Value> {
    proptest::sample::select(vec![true, false])
        .prop_map(move |flag| condition_object(field, "exists", json!(flag)))
        .boxed()
}

fn list_operand(field: &'static str) -> BoxedStrategy<Value> {
    proptest::collection::vec(operands(field), 0..=3)
        .prop_map(Value::Array)
        .boxed()
}

/// One field condition of `field`: the scalar shorthand (which is `eq`), or an
/// operator object restricted to the operators `docs/05-api.md` allows.
fn condition(field: &'static str, kind: Kind) -> BoxedStrategy<Value> {
    let shorthand = operands(field).prop_map(move |value| object(vec![(field.to_owned(), value)]));
    match kind {
        Kind::Bool => prop_oneof![
            1 => shorthand,
            2 => operator_condition(field, "eq", operands(field)),
            1 => exists_condition(field),
        ]
        .boxed(),
        Kind::Keyword => prop_oneof![
            1 => shorthand,
            2 => operator_condition(field, "eq", operands(field)),
            1 => operator_condition(field, "ne", operands(field)),
            1 => operator_condition(field, "in", list_operand(field)),
            1 => operator_condition(field, "nin", list_operand(field)),
            1 => exists_condition(field),
        ]
        .boxed(),
        Kind::Integer | Kind::Float | Kind::Date => prop_oneof![
            1 => shorthand,
            2 => operator_condition(field, "eq", operands(field)),
            1 => operator_condition(field, "ne", operands(field)),
            1 => operator_condition(field, "lt", operands(field)),
            1 => operator_condition(field, "lte", operands(field)),
            1 => operator_condition(field, "gt", operands(field)),
            1 => operator_condition(field, "gte", operands(field)),
            1 => operator_condition(field, "in", list_operand(field)),
            1 => exists_condition(field),
        ]
        .boxed(),
    }
}

fn leaf() -> BoxedStrategy<Value> {
    proptest::sample::select(FIELDS.to_vec())
        .prop_flat_map(|(field, kind)| condition(field, kind))
        .boxed()
}

/// An object of one to three field conditions: the implicit AND of
/// `docs/05-api.md`.
fn clauses() -> BoxedStrategy<Value> {
    proptest::collection::vec(leaf(), 1..=3)
        .prop_map(|leaves| {
            let mut merged = Map::new();
            for leaf in leaves {
                if let Value::Object(entries) = leaf {
                    for (key, value) in entries {
                        merged.insert(key, value);
                    }
                }
            }
            Value::Object(merged)
        })
        .boxed()
}

/// A filter of the grammar, at most `depth` logical levels deep.
fn filter(depth: u32) -> BoxedStrategy<Value> {
    if depth == 0 {
        return clauses();
    }
    prop_oneof![
        4 => clauses(),
        1 => proptest::collection::vec(filter(depth - 1), 1..=3)
            .prop_map(|items| object(vec![("$and".to_owned(), Value::Array(items))])),
        1 => proptest::collection::vec(filter(depth - 1), 1..=3)
            .prop_map(|items| object(vec![("$or".to_owned(), Value::Array(items))])),
        1 => filter(depth - 1)
            .prop_map(|child| object(vec![("$not".to_owned(), child)])),
        1 => (clauses(), filter(depth - 1)).prop_map(|(base, child)| {
            let Value::Object(mut entries) = base else {
                panic!("a clause object must be an object")
            };
            entries.insert("$not".to_owned(), child);
            Value::Object(entries)
        }),
    ]
    .boxed()
}

/// The schema declares every field the generator draws; a mismatch would make
/// `validate` refuse a filter the property believes valid.
#[test]
fn inv_07_generator_fields_are_declared_by_the_fixture_schema() {
    let schema = fixture_schema();
    for (name, kind) in FIELDS {
        let (spec, _) = schema
            .field(name)
            .unwrap_or_else(|| panic!("the fixture schema must declare '{name}'"));
        assert_eq!(
            kind_of(spec.field_type),
            *kind,
            "'{name}' must have the type the generator assumes"
        );
    }
}
