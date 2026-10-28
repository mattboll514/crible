// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `INV-07`: a filter turned into a bitmap equals the SQL evaluation of the
//! same filter.
//!
//! References: `docs/08-invariants-et-tests.md` (INV-07), task `EVALUER-FILTRES`
//! of `docs/phases/P1-stockage-et-recherche-exacte.md`, `docs/03-recherche.md`
//! ("Filtres → pool"), `docs/05-api.md` ("Grammaire des filtres"), ADR-0018,
//! ADR-0019, ADR-0029, ADR-0030 and ADR-0035.
//!
//! # API under test
//!
//! `crible_search::pool`, as `src/pool.rs` implements it:
//!
//! ```text
//! crible_search::pool::Pool::evaluate(filters: &FilterIndex, filter: &Filter)
//!     -> Result<Pool, CribleError>;
//! Pool::bitmap(&self) -> &RoaringBitmap;
//! Pool::len(&self) -> u32;
//! Pool::is_empty(&self) -> bool;
//! ```
//!
//! The test passes `snapshot.filters()`, which carries both the per-field
//! structures and the bitmap of the live passages (`FilterIndex::live`), and
//! reads `Snapshot::live_iids()` to describe the expected set.
//!
//! # What the property states
//!
//! For every generated filter, `Pool::evaluate` and the oracle of
//! [`common::sql_matches`] (one `SELECT` over `records.sqlite` with
//! `json_extract`) return the same set of iids, and that set is a subset of the
//! live passages. The pool is therefore exactly the pool SQL describes, not
//! merely a subset of it, and a tombstone never comes back.
//!
//! The run also counts the interval clauses it met: the property asserts that it
//! exercised an interval clause on the `integer`, `float` and `date` columns, so
//! a change of the generator cannot leave the value-column scan (`scan_column` in
//! `src/pool.rs`, rewritten by `INSERER-POOL-EN-ORDRE`) checked on some column
//! types only.
//!
//! The generator covers the whole grammar: every operator of `docs/05-api.md`
//! on the type that allows it, the implicit AND of an object, `$and`, `$or`,
//! `$not`, document-level and passage-level fields, values that no row carries,
//! and the three states ADR-0035 distinguishes: a value, an explicit null and
//! an absent key. The generator stays well inside the limits of ADR-0029 (depth
//! 6 at most, 27 clauses at most), so a rejected filter would be a bug of the
//! generator, not a limit under test.
//!
//! # Dataset
//!
//! One index of [`DOCUMENT_COUNT`] documents and three passages each, drawn
//! from a seeded generator (`SEED`), so the test is deterministic and never
//! reads the clock or the network. A second generation rewrites the fields of a
//! few documents and tombstones some passages, so the structures under test are
//! the ones derived in copy on write, not a fresh build. The oracle reads the
//! same `records.sqlite`, so it follows the reference state whatever the
//! derivation does.

mod common;

use crible_core::config::{FieldType, Schema};
use crible_core::filter::{Filter, Operator};
use crible_core::ids::Iid;
use proptest::prelude::*;
use proptest::test_runner::{Config, RngSeed, TestCaseError, TestRunner};
use serde_json::{Map, Value, json};

use common::{Fixture, document, fixture_schema, passage, sorted, sql_matches};

/// Number of documents of the reference dataset.
const DOCUMENT_COUNT: usize = 40;
/// Number of passages per document.
const PASSAGES_PER_DOCUMENT: usize = 3;
/// Seed of the dataset generator, and of the filter generator.
const SEED: u64 = 0x5EED_0007;
/// Number of filters the property checks (`INV-07` asks for 1 000 at least).
const CASES: u32 = 1_000;
/// Seed of the filter generator, fixed so a run is reproducible.
const PROPTEST_SEED: u64 = 0x494E_5607;

/// Field type of the fixture schema, as the generator needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Keyword,
    Integer,
    Float,
    Date,
    Bool,
}

/// One field of the fixture schema.
#[derive(Debug, Clone, Copy)]
struct FieldDef {
    name: &'static str,
    kind: Kind,
}

/// Every field of the fixture schema, at both levels.
///
/// A document-level field is inherited by the passages of its document, so the
/// generator has to reach both levels to cover `docs/03-recherche.md`.
const FIELDS: &[FieldDef] = &[
    FieldDef {
        name: "fonds",
        kind: Kind::Keyword,
    },
    FieldDef {
        name: "annee",
        kind: Kind::Integer,
    },
    FieldDef {
        name: "score",
        kind: Kind::Float,
    },
    FieldDef {
        name: "vigueur",
        kind: Kind::Date,
    },
    FieldDef {
        name: "archive",
        kind: Kind::Bool,
    },
    FieldDef {
        name: "partie",
        kind: Kind::Keyword,
    },
    FieldDef {
        name: "rang",
        kind: Kind::Integer,
    },
    FieldDef {
        name: "poids",
        kind: Kind::Float,
    },
    FieldDef {
        name: "jour",
        kind: Kind::Date,
    },
    FieldDef {
        name: "principal",
        kind: Kind::Bool,
    },
];

// --- The reference dataset ---

/// Keywords of the fixture documents.
const FONDS: [&str; 3] = ["LEGI", "JORF", "CNIL"];
/// Keywords of the fixture passages.
const PARTIES: [&str; 4] = ["corps", "moyens", "faits", "annexe"];
/// Integers of the fixture documents.
const ANNEES: [i64; 5] = [1990, 2000, 2010, 2020, 2026];
/// Integers of the fixture passages.
const RANGS: [i64; 4] = [0, 1, 2, 5];
/// Floats of both levels. Every value is exactly representable and reads back
/// to the same double through the JSON text SQLite parses, so an equality in
/// the engine and in SQL compares the same number.
const SCORES: [f64; 5] = [-1.5, 0.0, 0.5, 1.25, 3.0];
/// Dates of both levels, zero-padded, so a text order is a chronological order.
const DATES: [&str; 4] = ["1990-01-01", "2000-06-15", "2010-12-31", "2020-01-01"];

/// A small deterministic generator, so the fixture never depends on the clock
/// nor on the ordering of a hash map.
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

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    /// One of the three states ADR-0035 has to tell apart: a value, an
    /// explicit null, or no key at all.
    fn tri_state(&mut self, value: impl FnOnce(&mut Self) -> Value) -> Option<Value> {
        match self.below(10) {
            0 | 1 => None,
            2 | 3 => Some(Value::Null),
            _ => Some(value(self)),
        }
    }

    fn pick<T: Copy>(&mut self, values: &[T]) -> T {
        values[self.below(values.len() as u64) as usize]
    }
}

fn draw_fields(draw: &mut Draw, level: &[(&'static str, Kind)]) -> Value {
    let mut fields = Map::new();
    for (name, kind) in level {
        let name = *name;
        let value = draw.tri_state(|draw| match kind {
            Kind::Keyword => match name {
                "fonds" => json!(draw.pick(&FONDS)),
                _ => json!(draw.pick(&PARTIES)),
            },
            Kind::Integer => match name {
                "annee" => json!(draw.pick(&ANNEES)),
                _ => json!(draw.pick(&RANGS)),
            },
            Kind::Float => json!(draw.pick(&SCORES)),
            Kind::Date => json!(draw.pick(&DATES)),
            Kind::Bool => json!(draw.below(2) == 0),
        });
        if let Some(value) = value {
            fields.insert(name.to_owned(), value);
        }
    }
    Value::Object(fields)
}

const DOCUMENT_LEVEL: [(&str, Kind); 5] = [
    ("fonds", Kind::Keyword),
    ("annee", Kind::Integer),
    ("score", Kind::Float),
    ("vigueur", Kind::Date),
    ("archive", Kind::Bool),
];

const PASSAGE_LEVEL: [(&str, Kind); 5] = [
    ("partie", Kind::Keyword),
    ("rang", Kind::Integer),
    ("poids", Kind::Float),
    ("jour", Kind::Date),
    ("principal", Kind::Bool),
];

/// Builds the index of the run: one generation, then a generation that rewrites
/// documents and tombstones passages.
fn reference_index() -> Fixture {
    let mut fixture = Fixture::create(fixture_schema());
    let mut draw = Draw::new(SEED);

    let doc_ids: Vec<String> = (0..DOCUMENT_COUNT)
        .map(|index| format!("doc-{index:03}"))
        .collect();

    let documents: Vec<_> = doc_ids
        .iter()
        .map(|doc_id| document(doc_id, draw_fields(&mut draw, &DOCUMENT_LEVEL), 1))
        .collect();
    let mut passages = Vec::new();
    for (index, doc_id) in doc_ids.iter().enumerate() {
        for ord in 0..PASSAGES_PER_DOCUMENT {
            let iid = (index * PASSAGES_PER_DOCUMENT + ord) as u32;
            let fields = draw_fields(&mut draw, &PASSAGE_LEVEL);
            passages.push(passage(iid, doc_id, ord as u32, fields, 1));
        }
    }
    fixture.commit(documents, passages, Vec::new());

    // A second generation, so the structures under test are the ones derived in
    // copy on write: a rewritten document changes the inherited fields of its
    // passages, and a tombstone removes a passage from the live set.
    let rewritten: Vec<_> = doc_ids
        .iter()
        .take(4)
        .map(|doc_id| document(doc_id, draw_fields(&mut draw, &DOCUMENT_LEVEL), 2))
        .collect();
    let deletions: Vec<Iid> = (0..7).map(|step| Iid::new(step * 5)).collect();
    fixture.commit(rewritten, Vec::new(), deletions);

    fixture
}

// --- The filter generator ---

/// Operands of one field, drawn from the fixture data and from values no row
/// carries, so `eq` sometimes keeps a passage and sometimes empties the pool.
fn operands(field: &'static str) -> BoxedStrategy<Value> {
    let values: Vec<Value> = match field {
        "fonds" => vec![
            json!("LEGI"),
            json!("JORF"),
            json!("CNIL"),
            json!("INCONNU"),
        ],
        "partie" => vec![
            json!("corps"),
            json!("moyens"),
            json!("faits"),
            json!("annexe"),
        ],
        "annee" => ANNEES
            .iter()
            .map(|year| json!(year))
            .chain(std::iter::once(json!(1800)))
            .collect(),
        "rang" => RANGS
            .iter()
            .map(|rank| json!(rank))
            .chain(std::iter::once(json!(99)))
            .collect(),
        "score" | "poids" => SCORES
            .iter()
            .map(|score| json!(score))
            .chain(std::iter::once(json!(4.75)))
            .collect(),
        "vigueur" | "jour" => DATES
            .iter()
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
    // The empty list is part of the grammar the validation accepts, and the
    // oracle renders it explicitly, so the property has to draw it too.
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
        .prop_flat_map(|field| condition(field.name, field.kind))
        .boxed()
}

/// An object of one to three field conditions: the implicit AND of
/// `docs/05-api.md`. Two leaves may name the same field; the merge then keeps
/// one condition, which is a valid filter too.
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

// --- Coverage of the interval clauses ---

/// Column types on which the property has exercised an interval clause.
///
/// `scan_column` is the only reader of a value column, and the task
/// `INSERER-POOL-EN-ORDRE` rewrites how it fills its result bitmap. The property
/// must therefore meet an interval clause on every column type: a generator
/// change that stopped drawing one would leave the rewrite checked on the
/// remaining types only, and the test would stay green for the wrong reason.
#[derive(Debug, Default, PartialEq, Eq)]
struct IntervalCoverage {
    integer: bool,
    float: bool,
    date: bool,
}

impl IntervalCoverage {
    /// Notes the interval clauses of `filter`, at every level of the AST.
    fn record(&mut self, filter: &Filter, schema: &Schema) {
        match filter {
            Filter::Condition(condition) => {
                if !matches!(
                    condition.operator,
                    Operator::Lt | Operator::Lte | Operator::Gt | Operator::Gte
                ) {
                    return;
                }
                match schema
                    .field(&condition.field)
                    .map(|(spec, _level)| spec.field_type)
                {
                    Some(FieldType::Integer) => self.integer = true,
                    Some(FieldType::Float) => self.float = true,
                    Some(FieldType::Date) => self.date = true,
                    Some(FieldType::Keyword | FieldType::Bool) | None => {}
                }
            }
            Filter::And(children) | Filter::Or(children) => {
                for child in children {
                    self.record(child, schema);
                }
            }
            Filter::Not(child) => self.record(child, schema),
        }
    }

    /// True once every column type has been met.
    fn is_complete(&self) -> bool {
        self.integer && self.float && self.date
    }
}

// --- The property ---

#[test]
fn inv_07_filter_bitmap_matches_sql() {
    let fixture = reference_index();
    let schema = fixture.schema();
    let snapshot = fixture.snapshot();
    let filters = snapshot.filters();
    let records = fixture.records();
    let live: Vec<u32> = snapshot.live_iids().map(Iid::get).collect();

    assert_eq!(
        live.len(),
        DOCUMENT_COUNT * PASSAGES_PER_DOCUMENT - 7,
        "the reference index must hold every passage of the dataset minus the tombstones; \
         without a live set the property would be vacuous"
    );

    let config = Config {
        cases: CASES,
        rng_seed: RngSeed::Fixed(PROPTEST_SEED),
        failure_persistence: None,
        ..Config::default()
    };
    let mut runner = TestRunner::new(config);

    // The interval clauses of the run are counted, so the property is known to
    // cover the value-column scan on every type and not only on the one the seed
    // happens to favour.
    let coverage = std::cell::RefCell::new(IntervalCoverage::default());

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
            coverage.borrow_mut().record(&parsed, schema);

            let pool = crible_search::pool::Pool::evaluate(filters, &parsed).map_err(|error| {
                TestCaseError::fail(format!("the filter must evaluate: {error}: {raw}"))
            })?;

            let actual = sorted(pool.bitmap());
            let expected = sql_matches(&records, &parsed, schema);

            prop_assert_eq!(&actual, &expected, "filter {}", raw);
            prop_assert!(
                pool.bitmap().is_subset(filters.live()),
                "the pool must be a subset of the live passages: filter {}",
                raw
            );
            prop_assert_eq!(
                u64::from(pool.len()),
                pool.bitmap().len(),
                "the size the pool reports must be the size of its bitmap: filter {}",
                raw
            );
            prop_assert_eq!(pool.is_empty(), expected.is_empty());
            Ok(())
        })
        .unwrap();

    let coverage = coverage.into_inner();
    assert!(
        coverage.is_complete(),
        "the property must exercise an interval clause on every value-column type, \
         otherwise the scan of a column is not checked against SQL for all of them: \
         {coverage:?}"
    );
}

/// The property is only as good as its dataset: every field must carry a value,
/// an explicit null and an absent key, otherwise `exists`, `ne` and `$not` would
/// be checked on a state that never tells the three of ADR-0035 apart.
#[test]
fn inv_07_filter_dataset_covers_values_nulls_and_absent_keys() {
    let fixture = reference_index();
    let schema = fixture.schema();
    let records = fixture.records();
    for field in FIELDS {
        let (table, alias, rows) = match schema
            .field(field.name)
            .unwrap_or_else(|| panic!("'{}' is declared", field.name))
            .1
        {
            crible_core::config::FieldLevel::Document => ("documents", "d", DOCUMENT_COUNT as i64),
            crible_core::config::FieldLevel::Passage => (
                "passages",
                "p",
                (DOCUMENT_COUNT * PASSAGES_PER_DOCUMENT) as i64,
            ),
        };
        let count = |condition: &str| -> i64 {
            records
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} {alias} WHERE {condition}"),
                    [],
                    |row| row.get(0),
                )
                .unwrap_or_else(|error| panic!("the dataset must be readable: {error}"))
        };
        let name = field.name;
        let with_value = count(&format!(
            "json_extract({alias}.fields, '$.{name}') IS NOT NULL"
        ));
        let with_null = count(&format!(
            "json_extract({alias}.fields, '$.{name}') IS NULL \
             AND EXISTS (SELECT 1 FROM json_each({alias}.fields) WHERE key = '{name}')"
        ));
        let absent = count(&format!(
            "NOT EXISTS (SELECT 1 FROM json_each({alias}.fields) WHERE key = '{name}')"
        ));
        assert!(
            with_value > 0 && with_null > 0 && absent > 0,
            "field '{name}' must appear with a value, as an explicit null and not at all: \
             {with_value} values, {with_null} nulls, {absent} absent, {rows} rows"
        );
        assert!(
            with_value + with_null + absent == rows,
            "every row of {table} must be one of the three states for '{name}'"
        );
    }
}
