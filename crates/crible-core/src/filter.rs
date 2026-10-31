// SPDX-License-Identifier: Apache-2.0

//! Filter AST, parsing from JSON, and validation against a schema.
//!
//! References: `docs/05-api.md`, section "Grammaire des filtres"; ADR-0029
//! (depth and clause counting); ADR-0030 (one level per field name).
//!
//! Error paths start at the `root` parameter of [`Filter::from_json_str_at`]:
//! the route passes the path of the filter in the body, so an error about the
//! field `juridiction` names `$.filter.juridiction` on the simple route and
//! `$.targets[0].filter.juridiction` on the multi-index route. [`Filter::from_json_str`]
//! passes `$`, the filter object itself. The same path travels in
//! `details.path` of `invalid_filter` (`docs/05-api.md`).

use serde_json::Value;

use crate::config::{FieldType, Schema};
use crate::error::{CribleError, ErrorCode};
use crate::json::json_type;

/// Maximum number of object and array levels (root counts as one).
pub const MAX_DEPTH: usize = 8;
/// Maximum number of field conditions.
pub const MAX_CLAUSES: usize = 64;
/// Maximum number of values in an `in` or `nin` operand.
pub const MAX_VALUES: usize = 1000;

/// Comparison operator applied to a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    /// Equals.
    Eq,
    /// Not equal.
    Ne,
    /// Lower than.
    Lt,
    /// Lower than or equal.
    Lte,
    /// Greater than.
    Gt,
    /// Greater than or equal.
    Gte,
    /// Member of a list.
    In,
    /// Not member of a list.
    Nin,
    /// Field is present (or absent).
    Exists,
}

impl Operator {
    /// Every documented operator.
    pub const ALL: [Self; 9] = [
        Self::Eq,
        Self::Ne,
        Self::Lt,
        Self::Lte,
        Self::Gt,
        Self::Gte,
        Self::In,
        Self::Nin,
        Self::Exists,
    ];

    /// Wire name used in the filter JSON.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Eq => "eq",
            Self::Ne => "ne",
            Self::Lt => "lt",
            Self::Lte => "lte",
            Self::Gt => "gt",
            Self::Gte => "gte",
            Self::In => "in",
            Self::Nin => "nin",
            Self::Exists => "exists",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|operator| operator.as_str() == raw)
    }

    /// Every operator name, in the documented order, for error messages.
    fn list() -> String {
        Self::ALL
            .iter()
            .map(|operator| operator.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// One field condition: `field operator value`.
#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    /// Field the condition applies to.
    pub field: String,
    /// Comparison applied to the field.
    pub operator: Operator,
    /// Operand, already shaped for the operator.
    pub value: Value,
    /// JSON path of the condition, used by validation messages.
    path: String,
}

impl Condition {
    fn validate(&self, schema: &Schema) -> Result<(), CribleError> {
        let Some((spec, _level)) = schema.field(&self.field) else {
            return Err(self.unknown_field(schema));
        };
        if !spec.filterable {
            return Err(invalid(
                &self.path,
                format!("field '{}' is not filterable at {}", self.field, self.path),
            ));
        }
        if !spec.field_type.operators().contains(&self.operator) {
            return Err(invalid(
                &self.path,
                format!(
                    "operator '{}' is not allowed for {} at {}; expected one of: {}",
                    self.operator.as_str(),
                    spec.field_type.as_str(),
                    self.path,
                    spec.field_type
                        .operators()
                        .iter()
                        .map(|operator| operator.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
        match self.operator {
            Operator::Exists => {
                if !self.value.is_boolean() {
                    return Err(invalid(
                        &self.path,
                        format!(
                            "expected a boolean at {} for 'exists', got {}",
                            self.path,
                            json_type(&self.value)
                        ),
                    ));
                }
                Ok(())
            }
            Operator::In | Operator::Nin => self.validate_values(spec.field_type),
            _ => check_scalar(spec.field_type, &self.value, &self.path),
        }
    }

    fn validate_values(&self, field_type: FieldType) -> Result<(), CribleError> {
        let Some(values) = self.value.as_array() else {
            return Err(invalid(
                &self.path,
                format!(
                    "expected an array at {} for '{}', got {}",
                    self.path,
                    self.operator.as_str(),
                    json_type(&self.value)
                ),
            ));
        };
        if values.len() > MAX_VALUES {
            return Err(invalid(
                &self.path,
                format!(
                    "too many values in '{}' at {}: {}, at most {}",
                    self.operator.as_str(),
                    self.path,
                    values.len(),
                    MAX_VALUES
                ),
            ));
        }
        for (index, value) in values.iter().enumerate() {
            check_scalar(field_type, value, &format!("{}[{}]", self.path, index))?;
        }
        Ok(())
    }

    fn unknown_field(&self, schema: &Schema) -> CribleError {
        let mut message = format!("unknown field '{}' at {}", self.field, self.path);
        if let Some(suggestion) = closest_field(schema, &self.field) {
            message.push_str(&format!("; did you mean '{suggestion}'?"));
        }
        invalid(&self.path, message)
    }
}

/// Filter expression tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
    /// A single field condition.
    Condition(Condition),
    /// All children must match.
    And(Vec<Self>),
    /// At least one child must match.
    Or(Vec<Self>),
    /// The child must not match.
    Not(Box<Self>),
}

impl Filter {
    /// Parses a filter from its JSON form.
    ///
    /// Structural rules (object shape, operators, limits) are checked here;
    /// fields and value types are checked by [`Filter::validate`].
    pub fn from_json_str(raw: &str) -> Result<Self, CribleError> {
        Self::from_json_str_at(raw, "$")
    }

    /// Parses a filter from its JSON form, naming its faults under `root`.
    ///
    /// `root` is the JSON path of the filter in the body of the request. The
    /// simple search route passes `$.filter`, the multi-index route
    /// `$.targets[i].filter`, so a clause outside the grammar names its full
    /// path (ADR-0110, decision 2). [`Filter::from_json_str`] passes `$`.
    pub fn from_json_str_at(raw: &str, root: &str) -> Result<Self, CribleError> {
        let value: Value = serde_json::from_str(raw)
            .map_err(|error| invalid(root, format!("invalid filter JSON: {error}")))?;
        let mut clauses = 0;
        Self::parse(&value, root, 1, &mut clauses)
    }

    /// Validates the filter against the schema of an index.
    pub fn validate(&self, schema: &Schema) -> Result<(), CribleError> {
        match self {
            Self::Condition(condition) => condition.validate(schema),
            Self::And(children) | Self::Or(children) => {
                children.iter().try_for_each(|child| child.validate(schema))
            }
            Self::Not(child) => child.validate(schema),
        }
    }

    fn parse(
        value: &Value,
        path: &str,
        depth: usize,
        clauses: &mut usize,
    ) -> Result<Self, CribleError> {
        if depth > MAX_DEPTH {
            return Err(depth_error(path));
        }
        let Some(object) = value.as_object() else {
            return Err(invalid(
                path,
                format!("expected an object at {path}, got {}", json_type(value)),
            ));
        };
        let mut children = Vec::new();
        for (key, operand) in object {
            match key.as_str() {
                "$and" | "$or" => {
                    children.push(Self::parse_set(key, operand, path, depth, clauses)?)
                }
                "$not" => {
                    let child = Self::parse(operand, &format!("{path}.$not"), depth + 1, clauses)?;
                    children.push(Self::Not(Box::new(child)));
                }
                other if other.starts_with('$') => {
                    return Err(invalid(
                        &format!("{path}.{other}"),
                        format!(
                            "unknown logical operator '{other}' at {path}.{other}; expected $and, $or or $not"
                        ),
                    ));
                }
                field => {
                    *clauses += 1;
                    if *clauses > MAX_CLAUSES {
                        return Err(invalid(
                            &format!("{path}.{field}"),
                            format!(
                                "too many clauses at {path}.{field}: at most {MAX_CLAUSES} clauses"
                            ),
                        ));
                    }
                    let condition_path = format!("{path}.{field}");
                    children.push(Self::parse_field(field, operand, &condition_path, depth)?);
                }
            }
        }
        Ok(if children.len() == 1 {
            children.swap_remove(0)
        } else {
            Self::And(children)
        })
    }

    fn parse_set(
        key: &str,
        operand: &Value,
        path: &str,
        depth: usize,
        clauses: &mut usize,
    ) -> Result<Self, CribleError> {
        let Some(items) = operand.as_array() else {
            return Err(invalid(
                &format!("{path}.{key}"),
                format!(
                    "expected an array at {path}.{key}, got {}",
                    json_type(operand)
                ),
            ));
        };
        let mut children = Vec::with_capacity(items.len());
        if depth + 1 > MAX_DEPTH {
            return Err(depth_error(&format!("{path}.{key}")));
        }
        for (index, item) in items.iter().enumerate() {
            let item_path = format!("{path}.{key}[{index}]");
            // ADR-0029: the array counts for one level, each element for the
            // next one.
            children.push(Self::parse(item, &item_path, depth + 2, clauses)?);
        }
        Ok(if key == "$and" {
            Self::And(children)
        } else {
            Self::Or(children)
        })
    }

    fn parse_field(
        field: &str,
        operand: &Value,
        path: &str,
        depth: usize,
    ) -> Result<Self, CribleError> {
        if operand.is_array() {
            return Err(invalid(
                path,
                format!("expected a scalar or an operator object at {path}, got array"),
            ));
        }
        let Some(operators) = operand.as_object() else {
            return Ok(Self::Condition(Condition {
                field: field.to_owned(),
                operator: Operator::Eq,
                value: operand.clone(),
                path: path.to_owned(),
            }));
        };
        if operators.is_empty() {
            return Err(invalid(
                path,
                format!("expected at least one operator at {path}"),
            ));
        }
        if depth + 1 > MAX_DEPTH {
            return Err(depth_error(path));
        }
        let mut children = Vec::with_capacity(operators.len());
        for (key, value) in operators {
            let Some(operator) = Operator::parse(key) else {
                return Err(invalid(
                    path,
                    format!(
                        "unknown operator '{key}' at {path}; expected one of: {}",
                        Operator::list()
                    ),
                ));
            };
            children.push(Self::Condition(Condition {
                field: field.to_owned(),
                operator,
                value: value.clone(),
                path: path.to_owned(),
            }));
        }
        Ok(if children.len() == 1 {
            children.swap_remove(0)
        } else {
            Self::And(children)
        })
    }
}

fn invalid(path: &str, message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::InvalidFilter, message).with_path(path)
}

fn depth_error(path: &str) -> CribleError {
    invalid(
        path,
        format!("filter depth exceeds the maximum of {MAX_DEPTH} at {path}"),
    )
}

/// Checks that `value` is a scalar of the given field type.
fn check_scalar(field_type: FieldType, value: &Value, path: &str) -> Result<(), CribleError> {
    match field_type {
        FieldType::Keyword => {
            if value.is_string() {
                Ok(())
            } else {
                Err(type_error(field_type, value, path))
            }
        }
        FieldType::Integer => {
            if value.as_i64().is_some() {
                Ok(())
            } else {
                Err(type_error(field_type, value, path))
            }
        }
        FieldType::Float => {
            if value.as_f64().is_some() {
                Ok(())
            } else {
                Err(type_error(field_type, value, path))
            }
        }
        FieldType::Bool => {
            if value.is_boolean() {
                Ok(())
            } else {
                Err(type_error(field_type, value, path))
            }
        }
        FieldType::Date => match value.as_str() {
            Some(text) if is_valid_date(text) => Ok(()),
            Some(text) => Err(invalid(
                path,
                format!("invalid date at {path}: '{text}'; expected YYYY-MM-DD"),
            )),
            None => Err(invalid(
                path,
                format!(
                    "expected a date in YYYY-MM-DD format at {path}, got {}",
                    json_type(value)
                ),
            )),
        },
    }
}

fn type_error(field_type: FieldType, value: &Value, path: &str) -> CribleError {
    invalid(
        path,
        format!(
            "expected {} at {path}, got {}",
            field_type.as_str(),
            json_type(value)
        ),
    )
}

/// True for a real calendar date written `YYYY-MM-DD`.
pub(crate) fn is_valid_date(text: &str) -> bool {
    if text.len() != 10 || !text.is_ascii() {
        return false;
    }
    let bytes = text.as_bytes();
    if bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let (Some(year), Some(month), Some(day)) = (
        text[..4].parse::<u32>().ok(),
        text[5..7].parse::<u32>().ok(),
        text[8..].parse::<u32>().ok(),
    ) else {
        return false;
    };
    if !(1..=12).contains(&month) || day == 0 {
        return false;
    }
    day <= days_in_month(year, month)
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

/// Closest declared field name, when it is close enough to be a typo.
fn closest_field<'a>(schema: &'a Schema, name: &str) -> Option<&'a str> {
    let mut best: Option<(&str, usize)> = None;
    for candidate in schema.names() {
        let distance = edit_distance(name, candidate);
        if distance <= 2 && best.is_none_or(|(_, best_distance)| distance < best_distance) {
            best = Some((candidate, distance));
        }
    }
    best.map(|(candidate, _)| candidate)
}

/// Levenshtein distance between two strings, in characters.
fn edit_distance(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];
    for (row, left_character) in left.iter().enumerate() {
        current[0] = row + 1;
        for (column, right_character) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(left_character != right_character);
            current[column + 1] = substitution
                .min(previous[column + 1] + 1)
                .min(current[column] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}
