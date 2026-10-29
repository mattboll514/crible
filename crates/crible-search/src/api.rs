// SPDX-License-Identifier: Apache-2.0

//! The parts of a search request and answer the API and the CLI share.
//!
//! References: `docs/05-api.md` ("Recherche : requête" and "Recherche :
//! réponse"), ADR-0075 (`k`, regrouping), ADR-0109 (the route of P6 renders the
//! same answer as `crible search`) and ADR-0043 (only the fields the schema
//! declares `returned` reach a result).
//!
//! # Why this module exists
//!
//! ADR-0109 makes the HTTP route and `crible search` two callers of one search:
//! the criterion of `ROUTE-RECHERCHE` is that they return the same keys, in the
//! same order, with the same scores and the same payload. The regroupment rule
//! already lives in [`crate::group`]; this module holds the other two pieces
//! that must not be written twice:
//!
//! - the **bounds and names** of the request (`k`, `group_by`, `dedup`), so a
//!   flag and a body field cannot refuse the same value with two different
//!   words;
//! - the **shape of one result** (`doc_id`, `passage`, `key`, `score`,
//!   `score_type`, `fields`, `text`), with the `return` selection of the API.
//!   The CLI asks for the default selection: every field the schema declares
//!   `returned`, and the text.
//!
//! A request still parses its own syntax: the CLI reads flags, the route reads
//! JSON and names the path of a wrong member. The two paths meet on the values
//! this module validates, never on the spelling of a flag or a JSON key.

use std::collections::BTreeSet;

use crible_core::config::{FieldLevel, Schema};
use crible_core::error::{CribleError, ErrorCode};
use serde_json::{Map, Value, json};

use crate::group::Grouping;
use crate::pipeline::{Answer, RankedHit, ScoreType};

/// Highest `k` a search request accepts (`docs/05-api.md`, "Recherche :
/// requête").
pub const MAX_K: usize = 1_000;

/// Checks the `k` of a request against the documented domain.
///
/// The message is the one the CLI has always printed, so a caller that moves
/// from the binary to the route reads the same refusal.
///
/// # Errors
///
/// [`ErrorCode::InvalidRequest`] when `k` is zero or above [`MAX_K`].
pub fn check_k(k: usize) -> Result<(), CribleError> {
    if k == 0 || k > MAX_K {
        return Err(CribleError::new(
            ErrorCode::InvalidRequest,
            format!("k must be between 1 and {MAX_K}, got {k}"),
        ));
    }
    Ok(())
}

/// Reads a `group_by` value: `"document"` groups, absent groups nothing.
///
/// # Errors
///
/// [`ErrorCode::InvalidRequest`] for any other value.
pub fn group_by(value: Option<&str>) -> Result<bool, CribleError> {
    match value {
        None => Ok(false),
        Some("document") => Ok(true),
        Some(other) => Err(CribleError::new(
            ErrorCode::InvalidRequest,
            format!("group_by must be \"document\" or absent, got {other:?}"),
        )),
    }
}

/// Reads a `dedup` value: `"content"` deduplicates, absent does not.
///
/// # Errors
///
/// [`ErrorCode::InvalidRequest`] for any other value.
pub fn dedup(value: Option<&str>) -> Result<bool, CribleError> {
    match value {
        None => Ok(false),
        Some("content") => Ok(true),
        Some(other) => Err(CribleError::new(
            ErrorCode::InvalidRequest,
            format!("dedup must be \"content\" or absent, got {other:?}"),
        )),
    }
}

/// Builds the regrouping options from the parsed values.
///
/// The domain of `passages_per_document` is checked even when the request groups
/// nothing, as `docs/05-api.md` requires (ADR-0075).
///
/// # Errors
///
/// [`ErrorCode::InvalidRequest`] when the options leave their domain.
pub fn grouping(
    by_document: bool,
    passages_per_document: usize,
    dedup_content: bool,
) -> Result<Grouping, CribleError> {
    let grouping = Grouping {
        by_document,
        passages_per_document,
        dedup_content,
    };
    grouping.validate()?;
    Ok(grouping)
}

/// Which fields and text a result carries (`return` of `docs/05-api.md`).
///
/// `fields` is `None` for `"all"`: every field the schema declares `returned`
/// and that the result holds. A `Some` set is a selection; an empty one renders
/// no `fields` member at all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReturnSelection {
    fields: Option<BTreeSet<String>>,
    text: bool,
}

impl ReturnSelection {
    /// The default of `docs/05-api.md`: every returned field, and the text.
    #[must_use]
    pub fn all() -> Self {
        Self {
            fields: None,
            text: true,
        }
    }

    /// The selection of a request: `None` means `"all"`.
    #[must_use]
    pub fn new(fields: Option<BTreeSet<String>>, text: bool) -> Self {
        Self { fields, text }
    }

    /// Whether the results carry their text.
    #[must_use]
    pub fn text(&self) -> bool {
        self.text
    }

    /// The requested names, or `None` for `"all"`.
    #[must_use]
    pub fn names(&self) -> Option<&BTreeSet<String>> {
        self.fields.as_ref()
    }
}

/// Names of the fields of `level` the API may render (ADR-0043).
#[must_use]
pub fn returned_fields(schema: &Schema, level: FieldLevel) -> BTreeSet<String> {
    schema
        .fields()
        .filter(|(_, field_level, spec)| *field_level == level && spec.returned)
        .map(|(name, _, _)| name.to_owned())
        .collect()
}

/// Renders one result in the shape of `docs/05-api.md`.
///
/// Only the fields the schema declares `returned` are candidates; the selection
/// of the request then keeps the names it asks for. A selection that names no
/// field renders no `fields` member, and `return.text: false` renders no `text`
/// member.
#[must_use]
pub fn result_json(
    result: &RankedHit,
    score_type: ScoreType,
    schema: &Schema,
    selection: &ReturnSelection,
) -> Value {
    let mut rendered = Map::new();
    rendered.insert("doc_id".to_owned(), json!(result.hit.doc_id));
    rendered.insert("passage".to_owned(), json!(result.hit.ord));
    rendered.insert("key".to_owned(), json!(result.hit.key));
    rendered.insert("score".to_owned(), json!(result.hit.score));
    rendered.insert("score_type".to_owned(), json!(score_type.as_str()));

    let allowed = returned_fields(schema, FieldLevel::Document);
    let passage_allowed = returned_fields(schema, FieldLevel::Passage);
    let visible = visible_fields(&result.hit.fields, &allowed, &passage_allowed, selection);
    if let Some(fields) = visible {
        rendered.insert("fields".to_owned(), fields);
    }
    if selection.text {
        rendered.insert("text".to_owned(), json!(result.hit.text));
    }
    Value::Object(rendered)
}

/// Renders every result of an answer, in the order the pipeline produced.
#[must_use]
pub fn results_json(answer: &Answer, schema: &Schema, selection: &ReturnSelection) -> Vec<Value> {
    answer
        .results
        .iter()
        .map(|result| result_json(result, answer.score_type, schema, selection))
        .collect()
}

/// Keeps the members of `fields` the schema returns and the selection asks for.
///
/// `None` means the selection renders no `fields` member: only an explicit empty
/// `return.fields` does that. Otherwise the rendered object may be empty, which
/// is the truthful "no field available for this result" of `docs/05-api.md`.
fn visible_fields(
    fields: &Value,
    document_fields: &BTreeSet<String>,
    passage_fields: &BTreeSet<String>,
    selection: &ReturnSelection,
) -> Option<Value> {
    let selected = selection.names();
    if selected.is_some_and(BTreeSet::is_empty) {
        return None;
    }
    let stored = fields.as_object().cloned().unwrap_or_default();
    let mut rendered = Map::new();
    for (name, value) in stored {
        let returned = document_fields.contains(&name) || passage_fields.contains(&name);
        let wanted = selected.is_none_or(|selected| selected.contains(&name));
        if returned && wanted {
            rendered.insert(name, value);
        }
    }
    Some(Value::Object(rendered))
}

#[cfg(test)]
mod tests {
    use super::{MAX_K, ReturnSelection, check_k, dedup, group_by, returned_fields};
    use crible_core::config::{FieldLevel, Schema};
    use crible_core::error::ErrorCode;

    #[test]
    fn k_is_bounded_by_the_documented_domain() {
        assert!(check_k(1).is_ok());
        assert!(check_k(MAX_K).is_ok());
        for refused in [0, MAX_K + 1] {
            let failure = check_k(refused).expect_err("k is refused");
            assert_eq!(failure.code(), ErrorCode::InvalidRequest);
            assert!(failure.message().contains("k must be between 1 and"));
        }
    }

    #[test]
    fn grouping_names_are_the_documented_ones() {
        assert_eq!(group_by(Some("document")).ok(), Some(true));
        assert_eq!(group_by(None).ok(), Some(false));
        assert!(group_by(Some("article")).is_err());
        assert_eq!(dedup(Some("content")).ok(), Some(true));
        assert_eq!(dedup(None).ok(), Some(false));
        assert!(dedup(Some("text")).is_err());
    }

    #[test]
    fn only_returned_fields_are_candidates() {
        let schema = Schema::from_json_str(
            r#"{"document_fields":{"visible":{"type":"keyword"},"hidden":{"type":"keyword","returned":false}},
                "passage_fields":{"partie":{"type":"keyword"}}}"#,
        )
        .expect("the schema is valid");
        assert!(returned_fields(&schema, FieldLevel::Document).contains("visible"));
        assert!(!returned_fields(&schema, FieldLevel::Document).contains("hidden"));
        assert!(!returned_fields(&schema, FieldLevel::Passage).contains("visible"));
        assert!(ReturnSelection::all().text());
    }
}
