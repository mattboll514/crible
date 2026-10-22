// SPDX-License-Identifier: Apache-2.0

//! Evaluation of a validated filter into the pool of matching passages.
//!
//! References: `docs/03-recherche.md` ("Filtres → pool"), `docs/05-api.md`
//! ("Grammaire des filtres"), ADR-0018 (interval filters on value columns),
//! ADR-0019 (fields stored as canonical JSON), ADR-0029 (depth and clause
//! limits), ADR-0030 (a field name designates one level) and ADR-0035 (the
//! semantics of null values).
//!
//! # What a filter keeps
//!
//! [`Pool::evaluate`] turns the AST of a filter into one bitmap of internal
//! identifiers, intersected with the live passages:
//!
//! - a `keyword` or `bool` condition reads the bitmap of the value, or the
//!   "present" bitmap for `exists`; an `And` therefore evaluates those clauses
//!   first, from the most selective to the least selective;
//! - an `integer`, `float` or `date` condition scans the value column of the
//!   field, but only over the domain the preceding clauses already kept
//!   (ADR-0018). A date of the filter is first turned into the `i32` count of
//!   days the column holds, through [`crible_store::filters_index::date_days`],
//!   so the comparison never mixes a text with an integer;
//! - `Or` is the union of its children, an empty `Or` the empty pool; an empty
//!   `And` is the whole live set, because `{}` is the empty conjunction;
//! - `Not` is the complement over the live passages, so it keeps the passages
//!   whose field is null (ADR-0035).
//!
//! # Null values
//!
//! ADR-0035 applies the three-valued logic of SQL: a comparison (`eq`, `ne`,
//! `lt`, `lte`, `gt`, `gte`, `in`, `nin`) is never satisfied by a null, while
//! `exists: true` reads the "present" bitmap and `exists: false` is its
//! complement over the live passages. `ne` and `nin` are therefore
//! `present ∩ ¬bitmap(...)`, never a complement over the live set: a null
//! passage is kept by `$not` and dropped by `ne`.
//!
//! # Subset of the live passages
//!
//! Every intermediate set may still hold a tombstoned passage, because the
//! structures cover every row of `records.sqlite` and only the live bitmap hides
//! the dead ones (`docs/03-recherche.md`). The final intersection removes them,
//! and the complement of `Not` is taken on the live set directly, so the result
//! is always a subset of the live passages. A pool never returns a tombstone.

use crible_core::config::{FieldType, Schema};
use crible_core::error::{CribleError, ErrorCode};
use crible_core::filter::{Condition, Filter, Operator};
use crible_core::ids::Iid;
use crible_store::filters_index::{FilterIndex, ValueColumn, date_days};
use roaring::RoaringBitmap;
use serde_json::Value;

/// Passages a filter keeps, and their count.
///
/// The bitmap is the exact pool `docs/03-recherche.md` describes: the passages
/// the filter matches among the live ones, at the generation the
/// [`FilterIndex`] was built from.
#[derive(Debug, Clone, PartialEq)]
pub struct Pool {
    bitmap: RoaringBitmap,
    len: u32,
}

impl Pool {
    /// Evaluates a filter validated against the schema of the index.
    ///
    /// The filter is expected to have passed [`Filter::validate`], which
    /// `docs/05-api.md` performs before a search. The checks are repeated here
    /// anyway, so a direct caller gets [`ErrorCode::InvalidFilter`] instead of a
    /// silently empty pool.
    pub fn evaluate(filters: &FilterIndex, filter: &Filter) -> Result<Self, CribleError> {
        let mut bitmap = evaluate_filter(filters, filter, None)?;
        bitmap &= filters.live();
        let len = u32::try_from(bitmap.len()).map_err(|_| {
            CribleError::new(
                ErrorCode::Internal,
                "the pool holds more passages than a u32 can count",
            )
        })?;
        Ok(Self { bitmap, len })
    }

    /// Bitmap of the pool: every live passage the filter keeps.
    #[must_use]
    pub fn bitmap(&self) -> &RoaringBitmap {
        &self.bitmap
    }

    /// Number of passages of the pool, equal to `bitmap().len()`.
    #[must_use]
    pub fn len(&self) -> u32 {
        self.len
    }

    /// True when the filter keeps no live passage.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Evaluates one node of the AST.
///
/// `domain` is the set the enclosing conjunction already kept. A value column
/// scans only inside it (ADR-0018), and `$not` complements inside it, so the
/// promise of `docs/03-recherche.md` holds at every nested level and not only
/// at the first one. A condition answered by a bitmap still returns the whole
/// bitmap; the enclosing conjunction intersects it, and [`Pool::evaluate`] cuts
/// the dead passages at the end.
fn evaluate_filter(
    filters: &FilterIndex,
    filter: &Filter,
    domain: Option<&RoaringBitmap>,
) -> Result<RoaringBitmap, CribleError> {
    match filter {
        Filter::Condition(condition) => evaluate_condition(filters, condition, domain),
        Filter::And(children) => evaluate_and(filters, children, domain),
        Filter::Or(children) => {
            let mut result = RoaringBitmap::new();
            for child in children {
                result |= evaluate_filter(filters, child, domain)?;
            }
            Ok(result)
        }
        Filter::Not(child) => {
            // ADR-0035: the complement is taken on the live passages, nulls
            // included, which is what tells `$not` from `ne`. Under a
            // conjunction, the domain is already a subset of the live set, so
            // complementing inside it gives the same set for less work.
            let base = domain.unwrap_or_else(|| filters.live()).clone();
            let child = evaluate_filter(filters, child, Some(&base))?;
            let mut result = base;
            result -= child;
            Ok(result)
        }
    }
}

/// Evaluates a conjunction: bitmap clauses first, then the rest on the reduced
/// domain.
fn evaluate_and(
    filters: &FilterIndex,
    children: &[Filter],
    domain: Option<&RoaringBitmap>,
) -> Result<RoaringBitmap, CribleError> {
    if children.is_empty() {
        // `{}` is the empty conjunction, and every live passage satisfies it.
        return Ok(domain.map_or_else(|| filters.live().clone(), RoaringBitmap::clone));
    }

    let mut bitmap_clauses: Vec<&Condition> = Vec::new();
    let mut others: Vec<&Filter> = Vec::new();
    for child in children {
        match child {
            Filter::Condition(condition) if is_bitmap_condition(filters, condition) => {
                bitmap_clauses.push(condition);
            }
            other => others.push(other),
        }
    }

    // The clauses that are bitmaps are cheap and prune the most, so they run
    // first, from the most selective to the least selective
    // (`docs/03-recherche.md`).
    let mut bitmaps: Vec<RoaringBitmap> = Vec::with_capacity(bitmap_clauses.len());
    for condition in bitmap_clauses {
        bitmaps.push(evaluate_condition(filters, condition, domain)?);
    }
    bitmaps.sort_by_key(|bitmap| bitmap.len());

    // Every clause is evaluated before the intersection is complete, even when
    // an earlier one already emptied the domain: a clause the schema rejects or
    // a structure that contradicts it must raise its error whatever the order
    // of the clauses, instead of hiding behind a silently empty pool.
    let mut accumulator = domain.map_or_else(|| filters.live().clone(), RoaringBitmap::clone);
    for bitmap in &bitmaps {
        accumulator &= bitmap;
    }

    for child in others {
        let bitmap = match child {
            Filter::Condition(condition) if is_column_condition(filters, condition) => {
                // The interval clauses walk the column only over the domain the
                // previous clauses kept (ADR-0018). An emptied domain makes the
                // scan trivial, without skipping the check of the operands.
                evaluate_condition(filters, condition, Some(&accumulator))?
            }
            other => evaluate_filter(filters, other, Some(&accumulator))?,
        };
        accumulator &= bitmap;
    }
    Ok(accumulator)
}

/// True when `condition` reads a bitmap instead of scanning a value column.
fn is_bitmap_condition(filters: &FilterIndex, condition: &Condition) -> bool {
    if condition.operator == Operator::Exists {
        return true;
    }
    matches!(
        field_type(filters.schema(), condition),
        Some(FieldType::Keyword | FieldType::Bool)
    )
}

/// True when `condition` scans a value column.
fn is_column_condition(filters: &FilterIndex, condition: &Condition) -> bool {
    matches!(
        field_type(filters.schema(), condition),
        Some(FieldType::Integer | FieldType::Float | FieldType::Date)
    )
}

/// Declared type of the field a condition names, or `None` when the schema does
/// not declare it.
fn field_type(schema: &Schema, condition: &Condition) -> Option<FieldType> {
    schema
        .field(&condition.field)
        .map(|(spec, _level)| spec.field_type)
}

/// Evaluates one field condition, optionally restricted to `domain`.
///
/// `domain` is the set the enclosing conjunction already kept; a value column
/// scans only its identifiers. Bitmap conditions ignore it: intersecting their
/// result afterwards costs nothing.
fn evaluate_condition(
    filters: &FilterIndex,
    condition: &Condition,
    domain: Option<&RoaringBitmap>,
) -> Result<RoaringBitmap, CribleError> {
    let (spec, _level) = filters
        .schema()
        .field(&condition.field)
        .ok_or_else(|| unknown_field(condition))?;
    if !spec.filterable {
        return Err(invalid(format!(
            "field '{}' is not filterable",
            condition.field
        )));
    }
    if !spec.field_type.operators().contains(&condition.operator) {
        return Err(operator_not_allowed(condition, spec.field_type));
    }

    match spec.field_type {
        FieldType::Keyword => keyword_condition(filters, condition),
        FieldType::Bool => boolean_condition(filters, condition),
        FieldType::Integer | FieldType::Float | FieldType::Date => {
            let column = filters.column(&condition.field).ok_or_else(|| {
                inconsistent(
                    condition,
                    "a keyword or bool field where a column is expected",
                )
            })?;
            if column.field_type() != spec.field_type {
                return Err(inconsistent(condition, "a column of another type"));
            }
            column_condition(filters, condition, column, domain)
        }
    }
}

/// Evaluates a `keyword` condition from the bitmaps of the field.
fn keyword_condition(
    filters: &FilterIndex,
    condition: &Condition,
) -> Result<RoaringBitmap, CribleError> {
    let field = filters
        .keyword(&condition.field)
        .ok_or_else(|| inconsistent(condition, "a missing keyword field"))?;
    match condition.operator {
        Operator::Eq => {
            let value = keyword_operand(condition)?;
            Ok(field.bitmap(value).cloned().unwrap_or_default())
        }
        Operator::Ne => {
            // ADR-0035: `present ∩ ¬bitmap(eq)`, never the complement over the
            // live passages, which would keep the nulls.
            let value = keyword_operand(condition)?;
            let mut result = field.present().clone();
            if let Some(bitmap) = field.bitmap(value) {
                result -= bitmap;
            }
            Ok(result)
        }
        Operator::In => {
            let mut result = RoaringBitmap::new();
            for value in keyword_list(condition)? {
                if let Some(bitmap) = field.bitmap(value) {
                    result |= bitmap;
                }
            }
            Ok(result)
        }
        Operator::Nin => {
            let mut result = field.present().clone();
            for value in keyword_list(condition)? {
                if let Some(bitmap) = field.bitmap(value) {
                    result -= bitmap;
                }
            }
            Ok(result)
        }
        Operator::Exists => {
            let expected = boolean_operand(condition)?;
            Ok(exists_bitmap(filters.live(), field.present(), expected))
        }
        Operator::Lt | Operator::Lte | Operator::Gt | Operator::Gte => {
            Err(operator_not_allowed(condition, FieldType::Keyword))
        }
    }
}

/// Evaluates a `bool` condition from the bitmaps of the field.
fn boolean_condition(
    filters: &FilterIndex,
    condition: &Condition,
) -> Result<RoaringBitmap, CribleError> {
    let field = filters
        .boolean(&condition.field)
        .ok_or_else(|| inconsistent(condition, "a missing bool field"))?;
    match condition.operator {
        Operator::Eq => Ok(field.bitmap(boolean_operand(condition)?).clone()),
        Operator::Exists => {
            let expected = boolean_operand(condition)?;
            Ok(exists_bitmap(filters.live(), field.present(), expected))
        }
        Operator::Ne
        | Operator::Lt
        | Operator::Lte
        | Operator::Gt
        | Operator::Gte
        | Operator::In
        | Operator::Nin => Err(operator_not_allowed(condition, FieldType::Bool)),
    }
}

/// Evaluates an `integer`, `float` or `date` condition over the value column.
fn column_condition(
    filters: &FilterIndex,
    condition: &Condition,
    column: &ValueColumn,
    domain: Option<&RoaringBitmap>,
) -> Result<RoaringBitmap, CribleError> {
    if condition.operator == Operator::Exists {
        let expected = boolean_operand(condition)?;
        return Ok(exists_bitmap(filters.live(), column.present(), expected));
    }

    match column.field_type() {
        FieldType::Integer => {
            if condition.operator == Operator::In {
                let list = integer_list(condition)?;
                Ok(scan_column(
                    column.present(),
                    domain,
                    |iid| column.integer(Iid::new(iid)),
                    |value| list.contains(&value),
                ))
            } else {
                let operand = integer_operand(condition)?;
                Ok(scan_column(
                    column.present(),
                    domain,
                    |iid| column.integer(Iid::new(iid)),
                    |value| matches_scalar(condition.operator, value, operand),
                ))
            }
        }
        FieldType::Float => {
            if condition.operator == Operator::In {
                let list = float_list(condition)?;
                Ok(scan_column(
                    column.present(),
                    domain,
                    |iid| column.float(Iid::new(iid)),
                    |value| list.contains(&value),
                ))
            } else {
                let operand = float_operand(condition)?;
                Ok(scan_column(
                    column.present(),
                    domain,
                    |iid| column.float(Iid::new(iid)),
                    |value| matches_scalar(condition.operator, value, operand),
                ))
            }
        }
        FieldType::Date => {
            if condition.operator == Operator::In {
                let list = date_list(condition)?;
                Ok(scan_column(
                    column.present(),
                    domain,
                    |iid| column.date_days(Iid::new(iid)),
                    |value| list.contains(&value),
                ))
            } else {
                let operand = date_operand(condition)?;
                Ok(scan_column(
                    column.present(),
                    domain,
                    |iid| column.date_days(Iid::new(iid)),
                    |value| matches_scalar(condition.operator, value, operand),
                ))
            }
        }
        FieldType::Keyword | FieldType::Bool => Err(inconsistent(
            condition,
            "a bitmap field where a column is expected",
        )),
    }
}

/// Collects the identifiers of `present` (optionally restricted to `domain`)
/// whose value satisfies the condition.
fn scan_column<T>(
    present: &RoaringBitmap,
    domain: Option<&RoaringBitmap>,
    read: impl Fn(u32) -> Option<T>,
    keep: impl Fn(T) -> bool,
) -> RoaringBitmap {
    // One intersection, so the loop is bounded by |present ∩ domain| and not
    // by |present|: a reduced domain really shortens the scan (ADR-0018).
    let scan = domain.map_or_else(|| present.clone(), |domain| present & domain);
    let mut result = RoaringBitmap::new();
    for iid in scan.iter() {
        if let Some(value) = read(iid)
            && keep(value)
        {
            result.insert(iid);
        }
    }
    result
}

/// True when `actual` satisfies the scalar `operator` against `operand`.
///
/// `in` is a list and is handled before the scan; `exists` reads the present
/// bitmap and `nin` is not allowed on a column (`docs/05-api.md`), so neither
/// reaches this comparison.
fn matches_scalar<T: PartialOrd>(operator: Operator, actual: T, operand: T) -> bool {
    match operator {
        Operator::Eq => actual == operand,
        Operator::Ne => actual != operand,
        Operator::Lt => actual < operand,
        Operator::Lte => actual <= operand,
        Operator::Gt => actual > operand,
        Operator::Gte => actual >= operand,
        Operator::In | Operator::Nin | Operator::Exists => false,
    }
}

/// Bitmap of the passages carrying (`true`) or lacking (`false`) a value.
///
/// "Present" holds a non-null value (ADR-0035); `false` is therefore the
/// complement over the live passages, which covers both a null and a key the
/// row does not carry.
fn exists_bitmap(live: &RoaringBitmap, present: &RoaringBitmap, expected: bool) -> RoaringBitmap {
    if expected {
        present.clone()
    } else {
        let mut result = live.clone();
        result -= present;
        result
    }
}

fn keyword_operand(condition: &Condition) -> Result<&str, CribleError> {
    condition
        .value
        .as_str()
        .ok_or_else(|| invalid(format!("expected a string for field '{}'", condition.field)))
}

fn keyword_list(condition: &Condition) -> Result<Vec<&str>, CribleError> {
    operands(condition)?
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| invalid(format!("expected strings for field '{}'", condition.field)))
        })
        .collect()
}

fn integer_operand(condition: &Condition) -> Result<i64, CribleError> {
    condition.value.as_i64().ok_or_else(|| {
        invalid(format!(
            "expected an integer for field '{}'",
            condition.field
        ))
    })
}

fn integer_list(condition: &Condition) -> Result<Vec<i64>, CribleError> {
    operands(condition)?
        .iter()
        .map(|value| {
            value.as_i64().ok_or_else(|| {
                invalid(format!("expected integers for field '{}'", condition.field))
            })
        })
        .collect()
}

fn float_operand(condition: &Condition) -> Result<f64, CribleError> {
    condition
        .value
        .as_f64()
        .ok_or_else(|| invalid(format!("expected a number for field '{}'", condition.field)))
}

fn float_list(condition: &Condition) -> Result<Vec<f64>, CribleError> {
    operands(condition)?
        .iter()
        .map(|value| {
            value
                .as_f64()
                .ok_or_else(|| invalid(format!("expected numbers for field '{}'", condition.field)))
        })
        .collect()
}

fn date_operand(condition: &Condition) -> Result<i32, CribleError> {
    let text = keyword_operand(condition)?;
    date_days(text).ok_or_else(|| invalid(date_format_error(condition)))
}

fn date_list(condition: &Condition) -> Result<Vec<i32>, CribleError> {
    keyword_list(condition)?
        .into_iter()
        .map(|text| date_days(text).ok_or_else(|| invalid(date_format_error(condition))))
        .collect()
}

/// Message for a date the filter carries in another form than `YYYY-MM-DD`.
///
/// It names the field and the operator but never repeats the value: an error
/// message may reach the logs, and a filter value must not (rule 11 of
/// `AGENTS.md`).
fn date_format_error(condition: &Condition) -> String {
    format!(
        "expected a date in YYYY-MM-DD format for field '{}' on operator '{}'",
        condition.field,
        condition.operator.as_str()
    )
}

fn boolean_operand(condition: &Condition) -> Result<bool, CribleError> {
    condition.value.as_bool().ok_or_else(|| {
        invalid(format!(
            "expected a boolean for field '{}'",
            condition.field
        ))
    })
}

fn operands(condition: &Condition) -> Result<&[Value], CribleError> {
    condition
        .value
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| {
            invalid(format!(
                "expected an array for '{}' on field '{}'",
                condition.operator.as_str(),
                condition.field
            ))
        })
}

fn invalid(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::InvalidFilter, message)
}

fn unknown_field(condition: &Condition) -> CribleError {
    invalid(format!("unknown field '{}'", condition.field))
}

fn operator_not_allowed(condition: &Condition, field_type: FieldType) -> CribleError {
    invalid(format!(
        "operator '{}' is not allowed for field '{}' of type {}",
        condition.operator.as_str(),
        condition.field,
        field_type.as_str()
    ))
}

/// Error raised when the structures of the index contradict its schema.
///
/// The filter is valid there, so the fault is on the engine side, not the
/// caller's.
fn inconsistent(condition: &Condition, found: &str) -> CribleError {
    CribleError::new(
        ErrorCode::Internal,
        format!(
            "the filter structures of field '{}' are inconsistent: found {}",
            condition.field, found
        ),
    )
}
