// SPDX-License-Identifier: Apache-2.0

//! The `Filter` AST: parsing from JSON and validation against a schema.
//!
//! References: `docs/05-api.md`, section "Grammaire des filtres", and the
//! `DEFINIR-CONFIG-INDEX` task of `docs/phases/P0-fondations.md`.
//!
//! Error paths are rooted at `$` for the filter object itself, so a message for
//! the field `juridiction` contains `$.juridiction`. The API layer prefixes the
//! path with the position of the filter in the request body.

mod common;

use crible_core::config::{FieldType, Schema};
use crible_core::error::{CribleError, ErrorCode};
use crible_core::filter::{Filter, Operator};
use proptest::sample::select;
use proptest::strategy::Strategy;
use proptest::test_runner::TestCaseError;
use serde_json::{Value, json};

/// Schema used by the filter tests. It covers the fields of the two filters of
/// `docs/05-api.md` plus one field per `FieldType`, and a non-filterable field.
const SCHEMA_JSON: &str = r#"{
  "document_fields": {
    "fonds":       { "type": "keyword" },
    "date_debut":  { "type": "date" },
    "date_fin":    { "type": "date" },
    "juridiction": { "type": "keyword" },
    "numero":      { "type": "keyword" },
    "annee":       { "type": "integer" },
    "score":       { "type": "float" },
    "actif":       { "type": "bool" },
    "titre":       { "type": "keyword", "filterable": false }
  },
  "passage_fields": {
    "partie": { "type": "keyword" }
  }
}"#;

fn schema() -> Schema {
    Schema::from_json_str(SCHEMA_JSON)
        .unwrap_or_else(|error| panic!("the test schema is valid: {error}"))
}

fn parse(raw: &str) -> Filter {
    Filter::from_json_str(raw).unwrap_or_else(|error| panic!("the filter must be parsed: {error}"))
}

fn accept(raw: &str) {
    parse(raw)
        .validate(&schema())
        .unwrap_or_else(|error| panic!("the filter must be valid against the schema: {error}"));
}

/// Returns the error of the first rejecting stage, parsing or validation.
fn reject(raw: &str) -> CribleError {
    match Filter::from_json_str(raw) {
        Ok(filter) => match filter.validate(&schema()) {
            Ok(()) => panic!("the filter must be rejected: {raw}"),
            Err(error) => error,
        },
        Err(error) => error,
    }
}

fn assert_rejected(raw: &str, needles: &[&str]) -> CribleError {
    let error = reject(raw);
    assert_eq!(
        error.code(),
        ErrorCode::InvalidFilter,
        "wrong error code for {raw}: {}",
        error.message()
    );
    for needle in needles {
        assert!(
            error.message().contains(needle),
            "message {:?} must contain {needle:?} for {raw}",
            error.message()
        );
    }
    error
}

fn nested_not(depth: usize) -> String {
    let mut raw = String::from(r#"{"fonds":"LEGI"}"#);
    for _ in 0..depth {
        raw = format!(r#"{{"$not":{raw}}}"#);
    }
    raw
}

fn or_with(clauses: usize) -> String {
    let items = vec![r#"{"partie":"motifs"}"#; clauses].join(",");
    format!(r#"{{"$or":[{items}]}}"#)
}

fn nested_or(levels: usize) -> String {
    let mut raw = String::from(r#"{"fonds":"LEGI"}"#);
    for _ in 0..levels {
        raw = format!(r#"{{"$or":[{raw}]}}"#);
    }
    raw
}

fn values_in(field: &str, operator: &str, count: usize) -> String {
    let values = (0..count)
        .map(|index| format!("\"v{index}\""))
        .collect::<Vec<_>>()
        .join(",");
    format!(r#"{{"{field}":{{"{operator}":[{values}]}}}}"#)
}

#[test]
fn filter_doc_example_in_force_is_valid() {
    // `docs/05-api.md`: articles of fonds LEGI in force on 1 January 2020.
    accept(
        r#"{ "fonds": "LEGI", "date_debut": { "lte": "2020-01-01" },
            "$or": [ { "date_fin": { "gt": "2020-01-01" } },
                     { "date_fin": { "exists": false } } ] }"#,
    );
}

#[test]
fn filter_doc_example_jurisdiction_is_valid() {
    // `docs/05-api.md`: Cour de cassation and Conseil d'État, without "moyens"
    // and "faits" passages.
    accept(
        r#"{ "juridiction": { "in": ["cass", "ce"] }, "partie": { "nin": ["moyens", "faits"] } }"#,
    );
}

#[test]
fn filter_doc_example_multi_index_targets_are_valid() {
    // `docs/05-api.md`, section "Recherche multi-index".
    accept(r#"{ "fonds": "LEGI" }"#);
    accept(r#"{ "partie": { "nin": ["moyens", "faits"] } }"#);
}

#[test]
fn filter_scalar_shorthand_is_an_equality() {
    let filter = parse(r#"{ "fonds": "LEGI" }"#);

    match filter {
        Filter::Condition(condition) => {
            assert_eq!(condition.field, "fonds");
            assert_eq!(condition.operator, Operator::Eq);
            assert_eq!(condition.value, json!("LEGI"));
        }
        other => panic!("a scalar value must be an equality, got {other:?}"),
    }
}

#[test]
fn filter_object_is_an_implicit_and() {
    let filter = parse(r#"{ "fonds": "LEGI", "partie": "motifs" }"#);

    match filter {
        Filter::And(children) => {
            let mut fields: Vec<&str> = children
                .iter()
                .filter_map(|child| match child {
                    Filter::Condition(condition) => Some(condition.field.as_str()),
                    _ => None,
                })
                .collect();
            fields.sort_unstable();
            assert_eq!(fields, ["fonds", "partie"]);
        }
        other => panic!("an object with several keys must be an AND, got {other:?}"),
    }
}

#[test]
fn filter_operator_table_matches_the_documented_grammar() {
    use Operator::{Eq, Exists, Gt, Gte, In, Lt, Lte, Ne, Nin};

    assert_eq!(
        FieldType::Keyword.operators().to_vec(),
        vec![Eq, Ne, In, Nin, Exists]
    );
    assert_eq!(
        FieldType::Integer.operators().to_vec(),
        vec![Eq, Ne, Lt, Lte, Gt, Gte, In, Exists]
    );
    assert_eq!(
        FieldType::Float.operators().to_vec(),
        vec![Eq, Ne, Lt, Lte, Gt, Gte, In, Exists]
    );
    assert_eq!(
        FieldType::Date.operators().to_vec(),
        vec![Eq, Ne, Lt, Lte, Gt, Gte, In, Exists]
    );
    assert_eq!(FieldType::Bool.operators().to_vec(), vec![Eq, Exists]);
}

#[test]
fn filter_rejects_an_unknown_field() {
    let error = assert_rejected(
        r#"{ "juridicton": "cass" }"#,
        &["juridicton", "$.juridicton", "did you mean 'juridiction'"],
    );
    assert_eq!(error.code(), ErrorCode::InvalidFilter);
}

#[test]
fn filter_rejects_an_unknown_operator() {
    assert_rejected(
        r#"{ "fonds": { "between": "LEGI" } }"#,
        &["between", "$.fonds", "eq"],
    );
}

#[test]
fn filter_rejects_an_operator_forbidden_for_the_field_type() {
    // `lt` is documented for integer, float and date, never for keyword.
    assert_rejected(
        r#"{ "fonds": { "lt": "LEGI" } }"#,
        &["lt", "$.fonds", "keyword"],
    );
}

#[test]
fn filter_rejects_in_on_a_boolean_field() {
    assert_rejected(
        r#"{ "actif": { "in": [true] } }"#,
        &["in", "$.actif", "bool"],
    );
}

#[test]
fn filter_rejects_a_string_value_for_an_integer_field() {
    assert_rejected(r#"{ "annee": "2020" }"#, &["annee", "$.annee", "integer"]);
}

#[test]
fn filter_rejects_a_string_value_for_a_float_field() {
    assert_rejected(r#"{ "score": "high" }"#, &["score", "$.score", "float"]);
}

#[test]
fn filter_rejects_a_string_value_for_a_boolean_field() {
    assert_rejected(r#"{ "actif": "yes" }"#, &["actif", "$.actif", "bool"]);
}

#[test]
fn filter_rejects_an_array_value_for_a_keyword_field() {
    // The grammar allows a scalar or an object of operators, nothing else.
    assert_rejected(
        r#"{ "fonds": ["LEGI"] }"#,
        &["$.fonds", "expected", "operator"],
    );
}

#[test]
fn filter_rejects_a_malformed_scalar_date() {
    assert_rejected(
        r#"{ "date_debut": "2020-13-45" }"#,
        &["date_debut", "$.date_debut", "YYYY-MM-DD"],
    );
}

#[test]
fn filter_rejects_a_malformed_date_operand() {
    assert_rejected(
        r#"{ "date_debut": { "gt": "2020/01/01" } }"#,
        &["date_debut", "$.date_debut", "YYYY-MM-DD"],
    );
}

#[test]
fn filter_rejects_an_impossible_calendar_date() {
    // 2021 is not a leap year.
    assert_rejected(
        r#"{ "date_debut": "2021-02-29" }"#,
        &["date_debut", "$.date_debut", "YYYY-MM-DD"],
    );
}

#[test]
fn filter_rejects_a_number_for_a_date_field() {
    assert_rejected(
        r#"{ "date_debut": 20200101 }"#,
        &["date_debut", "$.date_debut", "date"],
    );
}

#[test]
fn filter_rejects_more_than_1000_values_in_in() {
    assert_rejected(&values_in("fonds", "in", 1001), &["in", "$.fonds", "1000"]);
}

#[test]
fn filter_rejects_more_than_1000_values_in_nin() {
    assert_rejected(
        &values_in("partie", "nin", 1001),
        &["nin", "$.partie", "1000"],
    );
}

#[test]
fn filter_rejects_more_than_8_levels_of_nesting() {
    assert_rejected(&nested_not(9), &["depth", "8", "$.$not"]);
}

#[test]
fn filter_counts_the_or_array_as_a_depth_level() {
    // ADR-0029: the root object is level 1, the `$or` array level 2 and the
    // element object level 3.
    accept(r#"{ "fonds": "LEGI", "$or": [ { "partie": "motifs" } ] }"#);
}

#[test]
fn filter_rejects_four_nested_or_arrays() {
    // Each nesting adds the array and its element: the fourth element object
    // sits at depth 9, above the documented maximum of 8.
    assert_rejected(&nested_or(4), &["depth", "8", "$.$or"]);
    accept(&nested_or(3));
}

#[test]
fn filter_rejects_more_than_64_clauses() {
    assert_rejected(&or_with(65), &["64", "clauses", "$.$or"]);
}

#[test]
fn filter_rejects_a_non_array_or_operand() {
    assert_rejected(
        r#"{ "$or": { "fonds": "LEGI" } }"#,
        &["$or", "$.$or", "array"],
    );
}

#[test]
fn filter_rejects_a_non_object_and_element() {
    assert_rejected(r#"{ "$and": ["x"] }"#, &["$and", "$.$and[0]", "object"]);
}

#[test]
fn filter_rejects_a_non_object_not_operand() {
    assert_rejected(r#"{ "$not": "x" }"#, &["$not", "$.$not", "object"]);
}

#[test]
fn filter_rejects_an_unknown_logical_operator() {
    assert_rejected(
        r#"{ "$xor": [ { "fonds": "LEGI" } ] }"#,
        &["$xor", "$.$xor", "expected"],
    );
}

#[test]
fn filter_rejects_a_field_marked_not_filterable() {
    assert_rejected(
        r#"{ "titre": "responsabilite" }"#,
        &["titre", "$.titre", "filterable"],
    );
}

#[test]
fn filter_rejects_a_bare_array() {
    assert_rejected(r#"[ { "fonds": "LEGI" } ]"#, &["$", "expected"]);
}

#[test]
fn filter_rejects_an_empty_operator_object() {
    assert_rejected(r#"{ "fonds": {} }"#, &["fonds", "$.fonds", "operator"]);
}

#[test]
fn filter_accepts_exactly_1000_values() {
    accept(&values_in("fonds", "in", 1000));
}

#[test]
fn filter_accepts_64_clauses() {
    accept(&or_with(64));
}

#[test]
fn filter_accepts_nested_logical_operators() {
    // Four levels of `$not`: well inside the documented limit of 8.
    accept(&nested_not(4));
}

#[test]
fn filter_accepts_a_leap_day() {
    accept(r#"{ "date_debut": "2020-02-29" }"#);
}

#[test]
fn filter_accepts_numeric_operands_on_numeric_fields() {
    accept(r#"{ "annee": { "gte": 2010 } }"#);
    accept(r#"{ "score": { "lt": 0.75 } }"#);
    accept(r#"{ "score": { "in": [0.25, 0.5] } }"#);
}

fn field_of(field_type: FieldType) -> &'static str {
    match field_type {
        FieldType::Keyword => "fonds",
        FieldType::Integer => "annee",
        FieldType::Float => "score",
        FieldType::Date => "date_debut",
        FieldType::Bool => "actif",
    }
}

/// A value of the right type for the field, shaped for the operator.
fn probe_value(field_type: FieldType, operator: Operator) -> Value {
    let scalar = match field_type {
        FieldType::Keyword => json!("x"),
        FieldType::Integer => json!(1),
        FieldType::Float => json!(1.5),
        FieldType::Date => json!("2020-01-01"),
        FieldType::Bool => json!(true),
    };
    match operator {
        Operator::In | Operator::Nin => json!([scalar]),
        Operator::Exists => json!(true),
        _ => scalar,
    }
}

fn condition_json(field_type: FieldType, operator: Operator) -> String {
    let mut operators = serde_json::Map::new();
    operators.insert(
        operator.as_str().to_string(),
        probe_value(field_type, operator),
    );
    let mut fields = serde_json::Map::new();
    fields.insert(field_of(field_type).to_string(), Value::Object(operators));
    Value::Object(fields).to_string()
}

fn field_type_strategy() -> impl Strategy<Value = FieldType> {
    select(FieldType::ALL.to_vec())
}

fn operator_strategy() -> impl Strategy<Value = Operator> {
    select(Operator::ALL.to_vec())
}

#[test]
fn filter_rejects_every_operator_outside_the_documented_table() {
    common::check_property(
        (field_type_strategy(), operator_strategy()),
        |(field_type, operator)| {
            if field_type.operators().contains(&operator) {
                return Ok(());
            }
            let raw = condition_json(field_type, operator);
            let error = reject(&raw);
            if error.code() == ErrorCode::InvalidFilter {
                Ok(())
            } else {
                Err(TestCaseError::fail(format!(
                    "{field_type:?} with {operator:?} must be invalid_filter, got {:?} for {raw}",
                    error.code()
                )))
            }
        },
    );
}

#[test]
fn filter_accepts_every_operator_of_the_documented_table() {
    common::check_property(
        (field_type_strategy(), operator_strategy()),
        |(field_type, operator)| {
            if !field_type.operators().contains(&operator) {
                return Ok(());
            }
            let raw = condition_json(field_type, operator);
            match parse(&raw).validate(&schema()) {
                Ok(()) => Ok(()),
                Err(error) => Err(TestCaseError::fail(format!(
                    "{field_type:?} with {operator:?} must be valid, got {:?} for {raw}",
                    error.message()
                ))),
            }
        },
    );
}
