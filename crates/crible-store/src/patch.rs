// SPDX-License-Identifier: Apache-2.0

//! Metadata patch of documents, as ADR-0060 defines it.
//!
//! A patch updates `documents.fields` without re-encoding anything: it writes
//! no segment, no passage and no `batches` row, and it leaves `doc_hash`, the
//! vectors and the iid unchanged (`docs/02-architecture.md`, "Patch de
//! métadonnées"). It commits one generation, G+1, when at least one document
//! changes, and none otherwise.
//!
//! # What the commit writes
//!
//! Step 3 of `docs/02-architecture.md`: the `documents` rows of the patched
//! documents — `fields` and `updated_generation = G+1` — and
//! `meta.generation = G+1`. No `segments`, no `passages`, no `batches`. Step 5
//! then writes the manifest `G+1`, which repeats the segments of G, and step 6
//! publishes the snapshot with the filter structures of the generation, copied
//! on write. The `documents` rows this commit writes are the delta that selects
//! the documents whose inherited fields are refreshed for their passages
//! (`crate::filters_index::FilterIndex::after_commit`, ADR-0066).
//!
//! # Input
//!
//! One entry is `{ "doc_id": …, "fields": { … } }`, and the same type serves
//! the JSONL file of `crible patch` and the body of the P6 route (ADR-0060,
//! decision 1). Fields are merged: a key replaces the value of the field of the
//! same name, an absent key keeps its value, and `{}` is a no-op. A `null` sets
//! the value; there is no way to remove a key. A field name the schema does not
//! declare at document level, `doc_id` or `doc_hash`, and a value of the wrong
//! type are refused in [`ErrorCode::InvalidRequest`] before any write, and an
//! unknown or removed `doc_id` makes the whole input fail in
//! [`ErrorCode::NotFound`]. No message carries a field value.

use std::collections::HashMap;
use std::path::Path;

use crible_core::config::{FieldLevel, FieldType, Schema};
use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::Generation;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::filters_index::date_days;
use crate::generation::{GenerationChange, GenerationStore, generation_after};
use crate::records::{self, DocumentRecord};

/// Largest patch accepted, in bytes, for the CLI file and the HTTP body alike
/// (ADR-0060, decision 7).
///
/// The bound is the decimal megabyte of ADR-0060, `docs/04` and `docs/05`
/// ("32 Mo"), not the binary mebibyte the former `32 * 1024 * 1024` wrote, so
/// the code and the documents name the same number of bytes.
pub const MAX_BYTES: u64 = 32_000_000;

/// Largest number of entries accepted in one patch (ADR-0060, decision 7).
pub const MAX_ENTRIES: usize = 10_000;

/// One document of a patch, shared by `crible patch` and the P6 route.
///
/// The envelope refuses unknown members, as the online document does: a patch
/// entry has no passage by construction (ADR-0060, decision 1), and silently
/// dropping a key would let a caller believe it was written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentPatch {
    /// External identifier of the document to patch; a patch never creates one.
    pub doc_id: String,
    /// Document fields to write, merged into the stored ones.
    pub fields: Map<String, Value>,
}

/// Where a patch came from, which fixes the JSON path and the location an error
/// message names (ADR-0060, decision 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchSource {
    /// A JSONL file read by `crible patch`: `$[ligne].fields.<nom>`, lines being
    /// numbered from 1.
    File,
    /// The `documents` array of the P6 body: `$.documents[0].fields.<nom>`,
    /// entries being indexed from 0.
    Body,
}

impl PatchSource {
    /// JSON path of one field of one entry.
    fn field_path(self, position: usize, field: &str) -> String {
        match self {
            Self::File => format!("$[{}].fields.{field}", position + 1),
            Self::Body => format!("$.documents[{position}].fields.{field}"),
        }
    }

    /// JSON path of the `doc_id` member of one entry.
    fn document_path(self, position: usize) -> String {
        match self {
            Self::File => format!("$[{}].doc_id", position + 1),
            Self::Body => format!("$.documents[{position}].doc_id"),
        }
    }

    /// Location of one entry in the input, for an error message.
    fn at(self, position: usize) -> String {
        match self {
            Self::File => format!("line {}", position + 1),
            Self::Body => format!("documents[{position}]"),
        }
    }
}

/// Outcome of one patch: two document counts and the generation visible after
/// the call (ADR-0060, decision 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatchReport {
    /// Documents whose fields changed.
    pub updated: u64,
    /// Documents whose fields, in canonical JSON, did not change.
    pub unchanged: u64,
    /// Generation visible after the call: `G+1` when a document changed, `G`
    /// otherwise.
    pub generation: Generation,
}

/// Reads and parses a JSONL patch file, one entry per line.
///
/// The size is checked before the bytes are loaded and the entry count while
/// they are parsed, so a file above either bound is refused without being held
/// whole (ADR-0060, decision 7).
pub fn read_file(path: &Path) -> Result<Vec<DocumentPatch>, CribleError> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        CribleError::new(
            ErrorCode::InvalidRequest,
            format!("cannot read the patch file {}: {error}", path.display()),
        )
    })?;
    if metadata.len() > MAX_BYTES {
        return Err(too_many_bytes(metadata.len()));
    }
    let bytes = std::fs::read(path).map_err(|error| {
        CribleError::new(
            ErrorCode::InvalidRequest,
            format!("cannot read the patch file {}: {error}", path.display()),
        )
    })?;
    parse(&bytes)
}

/// Parses the bytes of a JSONL patch, one entry per line.
///
/// A line that is not an object `{doc_id, fields}` is refused with its number
/// and the column of the fault, never with the text of the line: it may hold a
/// field value.
pub fn parse(bytes: &[u8]) -> Result<Vec<DocumentPatch>, CribleError> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err(too_many_bytes(bytes.len() as u64));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| {
        invalid("the patch input is not UTF-8; a patch is one JSON object per line")
    })?;
    let mut patches = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        if line.trim().is_empty() {
            return Err(invalid(format!(
                "patch line {number} is empty; a patch holds one JSON object per line"
            )));
        }
        let patch: DocumentPatch = serde_json::from_str(line).map_err(|error| {
            invalid(format!(
                "patch line {number}, column {}: not a patch entry {{doc_id, fields}} ({:?})",
                error.column(),
                error.classify()
            ))
        })?;
        patches.push(patch);
        if patches.len() > MAX_ENTRIES {
            return Err(too_many_entries(patches.len()));
        }
    }
    Ok(patches)
}

/// Applies one patch, entirely or not at all (ADR-0060).
///
/// Every check happens before any write: the entry count, duplicate identifiers,
/// field names against the schema, value types, and the existence of each
/// `doc_id`. The input is then merged and compared in canonical JSON; when no
/// document changes, no generation is created, no manifest is written and no
/// snapshot is published, and the report carries the current generation.
///
/// # Errors
///
/// [`ErrorCode::PayloadTooLarge`] above [`MAX_ENTRIES`],
/// [`ErrorCode::InvalidRequest`] for a duplicate `doc_id`, an undeclared or
/// passage-level field name, a reserved name or an ill-typed value, and
/// [`ErrorCode::NotFound`] for an unknown or removed `doc_id`. No message names
/// a field value.
pub fn apply(
    store: &mut GenerationStore,
    schema: &Schema,
    source: PatchSource,
    patches: &[DocumentPatch],
) -> Result<PatchReport, CribleError> {
    if patches.len() > MAX_ENTRIES {
        return Err(too_many_entries(patches.len()));
    }
    check_duplicates(source, patches)?;
    for (position, patch) in patches.iter().enumerate() {
        check_fields(schema, source, position, &patch.fields)?;
    }

    let planned = store.with_connection(|connection| plan(connection, source, patches))?;
    let Plan {
        current,
        documents,
        updated,
        unchanged,
    } = planned;
    if updated == 0 {
        return Ok(PatchReport {
            updated: 0,
            unchanged,
            generation: current,
        });
    }

    let committed = store.commit(GenerationChange {
        segments: Vec::new(),
        documents,
        passages: Vec::new(),
        deletions: Vec::new(),
    })?;
    Ok(PatchReport {
        updated,
        unchanged,
        generation: committed,
    })
}

/// One planned generation: the rows to write, the counts, and the generation
/// visible before the call.
struct Plan {
    /// Generation visible before the call, `meta.generation`.
    current: Generation,
    /// Rows of `documents` the transaction rewrites, one per changed document.
    documents: Vec<DocumentRecord>,
    /// Documents whose fields change.
    updated: u64,
    /// Documents whose fields, in canonical JSON, stay identical.
    unchanged: u64,
}

/// Reads the reference state and decides what the commit writes. **No write
/// happens here**: the whole input is checked against the stored documents
/// before the transaction opens, which is what makes a refusal atomic.
fn plan(
    connection: &rusqlite::Connection,
    source: PatchSource,
    patches: &[DocumentPatch],
) -> Result<Plan, CribleError> {
    let meta = records::read_meta(connection)?;
    let generation = generation_after(meta.generation)?;
    let mut documents = Vec::new();
    let mut updated = 0_u64;
    let mut unchanged = 0_u64;

    for (position, patch) in patches.iter().enumerate() {
        let Some(previous) = records::document_by_id(connection, &patch.doc_id)? else {
            return Err(CribleError::new(
                ErrorCode::NotFound,
                format!(
                    "{}: no document '{}' in this index (path {}); a patch updates existing \
                     documents and never creates one",
                    source.at(position),
                    patch.doc_id,
                    source.document_path(position)
                ),
            ));
        };
        let merged = merge(&previous.fields, &patch.fields)?;
        if records::canonical_json(&merged)? == records::canonical_json(&previous.fields)? {
            unchanged = unchanged.saturating_add(1);
            continue;
        }
        updated = updated.saturating_add(1);
        documents.push(DocumentRecord {
            doc_id: patch.doc_id.clone(),
            doc_hash: previous.doc_hash,
            fields: merged,
            updated_generation: generation,
        });
    }

    Ok(Plan {
        current: meta.generation,
        documents,
        updated,
        unchanged,
    })
}

/// Merges the fields of one entry into the stored ones: a key replaces the
/// value of the field of the same name, an absent key keeps its value
/// (ADR-0060, decision 2). A `null` is a value, never a removal (decision 3).
fn merge(previous: &Value, changes: &Map<String, Value>) -> Result<Value, CribleError> {
    let mut merged = match previous {
        Value::Object(stored) => stored.clone(),
        _ => {
            return Err(CribleError::new(
                ErrorCode::Internal,
                "the stored fields of a document are not a JSON object",
            ));
        }
    };
    for (name, value) in changes {
        merged.insert(name.clone(), value.clone());
    }
    Ok(Value::Object(merged))
}

/// Refuses a `doc_id` named twice in one patch: the result of a patch would
/// then depend on the order of its lines (ADR-0060, decision 4).
fn check_duplicates(source: PatchSource, patches: &[DocumentPatch]) -> Result<(), CribleError> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for (position, patch) in patches.iter().enumerate() {
        if let Some(first) = seen.insert(patch.doc_id.as_str(), position) {
            return Err(invalid(format!(
                "document '{}' is patched twice, at {} and {}; a document appears once in a patch",
                patch.doc_id,
                source.at(first),
                source.at(position)
            )));
        }
    }
    Ok(())
}

/// Refuses a key of `fields` that is not a document field of the schema, and a
/// value whose type the field does not accept.
///
/// The message names the field, the expected level (`document_fields`), the
/// JSON path, the location in the input and, for a type error, the expected
/// type; it never names the value (ADR-0060, decisions 5 and 6).
fn check_fields(
    schema: &Schema,
    source: PatchSource,
    position: usize,
    fields: &Map<String, Value>,
) -> Result<(), CribleError> {
    for (name, value) in fields {
        let at = source.at(position);
        let path = source.field_path(position, name);
        let Some((spec, level)) = schema.field(name) else {
            let reserved = matches!(name.as_str(), "doc_id" | "doc_hash");
            return Err(invalid(if reserved {
                format!(
                    "{at}: '{name}' is a reserved name, not a document field (path {path}); a \
                     patch writes document_fields only"
                )
            } else {
                format!(
                    "{at}: field '{name}' is not declared in document_fields by the schema of \
                     this index (path {path})"
                )
            }));
        };
        if level != FieldLevel::Document {
            return Err(invalid(format!(
                "{at}: field '{name}' is declared in passage_fields, not in document_fields \
                 (path {path}); a patch writes document fields only"
            )));
        }
        check_value(name, spec.field_type, value, &at, &path)?;
    }
    Ok(())
}

/// Refuses a value that the declared type of its field does not accept.
///
/// A null is accepted by every field (`docs/04-donnees-et-ingestion.md`, "Tous
/// les champs acceptent null"). A date is checked against the form and the
/// domain of `YYYY-MM-DD`, as the reference state stores it.
fn check_value(
    name: &str,
    field_type: FieldType,
    value: &Value,
    at: &str,
    path: &str,
) -> Result<(), CribleError> {
    let accepted = match (field_type, value) {
        (_, Value::Null) => true,
        (FieldType::Keyword, Value::String(_)) => true,
        (FieldType::Bool, Value::Bool(_)) => true,
        (FieldType::Integer, value) => value.as_i64().is_some(),
        (FieldType::Float, value) => value.as_f64().is_some(),
        (FieldType::Date, Value::String(text)) => date_days(text).is_some(),
        _ => false,
    };
    if accepted {
        return Ok(());
    }
    let expected = match field_type {
        FieldType::Date => "a date written YYYY-MM-DD (type date)".to_owned(),
        other => format!("a value of type {}", other.as_str()),
    };
    Err(invalid(format!(
        "{at}: field '{name}' expects {expected} (path {path}); the value is not echoed"
    )))
}

/// Error raised when a patch holds more entries than one patch accepts.
fn too_many_entries(count: usize) -> CribleError {
    CribleError::new(
        ErrorCode::PayloadTooLarge,
        format!(
            "the patch holds {count} entries, above the {MAX_ENTRIES} entries of one patch \
             (ADR-0060); split the input"
        ),
    )
}

/// Error raised when a patch is larger than one patch accepts.
fn too_many_bytes(count: u64) -> CribleError {
    CribleError::new(
        ErrorCode::PayloadTooLarge,
        format!(
            "the patch is {count} bytes, above the {MAX_BYTES} bytes of one patch (ADR-0060); \
             split the input"
        ),
    )
}

/// Error raised by a patch input that does not have the shape ADR-0060 defines.
fn invalid(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::InvalidRequest, message)
}
