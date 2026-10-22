// SPDX-License-Identifier: Apache-2.0

//! Fixtures shared by the `INV-07` tests (`EVALUER-FILTRES`, phase P1).
//!
//! The module builds a real index in a temporary directory, commits the
//! reference rows through `GenerationStore`, and evaluates a filter a second
//! time in SQL over the same `records.sqlite`, which is the oracle of INV-07.
//!
//! # Assumed API
//!
//! `crible_search::pool` does not exist yet; these fixtures assume the
//! following minimal surface (see the header of
//! `inv_07_filter_bitmap_matches_sql.rs`):
//!
//! ```text
//! pub struct Pool { /* the bitmap of the pool and its size */ }
//!
//! impl Pool {
//!     pub fn evaluate(filters: &FilterIndex, filter: &Filter)
//!         -> Result<Pool, CribleError>;
//!     pub fn bitmap(&self) -> &RoaringBitmap;
//!     pub fn len(&self) -> u32;
//!     pub fn is_empty(&self) -> bool;
//! }
//! ```
//!
//! # The SQL oracle
//!
//! [`sql_matches`] translates a validated [`Filter`] into one `SELECT` over
//! `records.sqlite` and returns the iids it keeps. The translation follows
//! `docs/05-api.md` ("Grammaire des filtres"), `docs/03-recherche.md`
//! ("Filtres → pool") and ADR-0035, never the future Rust code:
//!
//! - a comparison is never satisfied by a null, so every comparison is wrapped
//!   in `COALESCE(..., 0)`: `NULL = 'a'` is UNKNOWN, which `COALESCE` turns into
//!   false, exactly as ADR-0035 asks. `ne` is `COALESCE(v <> x, 0)` and not
//!   `NOT (v = x)`, which would keep the nulls;
//! - `nin` is `COALESCE(v NOT IN (...), 0)`, the SQL form of
//!   `present ∩ ¬bitmap(in)`;
//! - `exists: true` is `json_extract(...) IS NOT NULL`, and `exists: false` is
//!   its negation, which covers a field stored as `null` and a field the row
//!   does not carry at all: JSON1 returns NULL in both cases (ADR-0035);
//! - `$not` is the complement over the live passages, so it wraps its child in
//!   `NOT (...)`. Every node of the translation is two-valued, so `NOT` never
//!   meets the UNKNOWN of SQL;
//! - the rows are the live passages (`dead_since_gen IS NULL`), so a pool that
//!   keeps a tombstone cannot match;
//! - a document-level field is read in `documents` (`d.fields`) and a
//!   passage-level field in `passages` (`p.fields`), which is the inheritance
//!   of `docs/03-recherche.md`.
//!
//! A `date` is compared here as the `YYYY-MM-DD` text SQLite holds, while the
//! engine compares a count of days. The two orders agree because the text is
//! zero-padded, so an interval on a date is the same interval in both.

// Every invariant binary includes this module with `mod common`, and none uses
// every helper: a build that only checks the translation never creates an
// index. The same allowance as `crates/crible-store/tests/common/mod.rs`.
#![allow(dead_code)]

/// Fixtures of the dense exact search tests (`INV-06`, `RECHERCHER-DENSE-EXACT`),
/// included here so every binary of this crate can share them.
pub mod dense;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crible_core::config::{FieldLevel, Schema};
use crible_core::filter::{Condition, Filter, Operator};
use crible_core::ids::{Epoch, Generation, Iid};
use crible_store::generation::{GenerationChange, GenerationStore, Snapshot, write_index_config};
use crible_store::records::{DocumentRecord, PassageRecord};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Map, Value};

/// Epoch every fixture index creates.
pub const EPOCH: u32 = 1;

/// JSON schema of the fixture index: the five field types at both levels.
const SCHEMA_JSON: &str = r#"{
    "document_fields": {
        "fonds":   { "type": "keyword" },
        "annee":   { "type": "integer" },
        "score":   { "type": "float" },
        "vigueur": { "type": "date" },
        "archive": { "type": "bool" }
    },
    "passage_fields": {
        "partie":    { "type": "keyword" },
        "rang":      { "type": "integer" },
        "poids":     { "type": "float" },
        "jour":      { "type": "date" },
        "principal": { "type": "bool" }
    }
}"#;

/// Schema of the fixture index.
pub fn fixture_schema() -> Schema {
    Schema::from_json_str(SCHEMA_JSON)
        .unwrap_or_else(|error| panic!("the fixture schema must be valid: {error}"))
}

/// Index configuration declaring `schema`, as `crible index create` writes it.
pub fn test_config(name: &str, schema: &Schema) -> String {
    format!(
        r#"{{"name":"{name}","model":"m","reference_precision":"fp16","schema":{},"cluster_by":[],"lexical":{{"enabled":false,"language":"fr","positions":false}},"search":{{"scan":"none","rescore_candidates":10,"exact_threshold":10,"candidates_per_branch":10,"fusion":{{"method":"rrf"}}}},"ingest":{{"acceptance_sample":1,"acceptance_min_cosine":0.9}},"compaction":{{"dead_ratio":0.1}}}}"#,
        schema.to_json_string()
    )
}

/// One index in a temporary directory, its writer, and the rows it holds.
pub struct Fixture {
    // The directory must outlive the index: dropping it removes the files.
    _temp: tempfile::TempDir,
    index: PathBuf,
    schema: Schema,
    store: GenerationStore,
}

impl Fixture {
    /// Creates an empty index in a fresh temporary directory.
    pub fn create(schema: Schema) -> Self {
        let temp = tempfile::tempdir().unwrap_or_else(|error| {
            panic!("a temporary directory for the index must be creatable: {error}")
        });
        let index = temp.path().join("indexes").join("inv-07");
        write_index_config(&index, &test_config("inv-07", &schema))
            .unwrap_or_else(|error| panic!("the index configuration must be written: {error}"));
        let store = GenerationStore::create(&index, Epoch::new(EPOCH), &schema)
            .unwrap_or_else(|error| panic!("the index must be created: {error}"));
        Self {
            _temp: temp,
            index,
            schema,
            store,
        }
    }

    /// Schema of the index, the one the filters are validated against.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Path of the reference database of the epoch, the SQL oracle's target.
    pub fn records_path(&self) -> PathBuf {
        self.index
            .join("epochs")
            .join(EPOCH.to_string())
            .join("records.sqlite")
    }

    /// Commits one generation and returns its number.
    pub fn commit(
        &mut self,
        documents: Vec<DocumentRecord>,
        passages: Vec<PassageRecord>,
        deletions: Vec<Iid>,
    ) -> Generation {
        self.store
            .commit(GenerationChange {
                segments: Vec::new(),
                documents,
                passages,
                deletions,
            })
            .unwrap_or_else(|error| panic!("the fixture commit must succeed: {error}"))
    }

    /// Snapshot of the current generation.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.store.snapshot()
    }

    /// Opens `records.sqlite` read-only, the way the oracle reads it.
    pub fn records(&self) -> Connection {
        let path = self.records_path();
        Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap_or_else(|error| panic!("{} must open read-only: {error}", path.display()))
    }
}

/// One `documents` row, stamped at `generation`.
pub fn document(doc_id: &str, fields: Value, generation: u64) -> DocumentRecord {
    DocumentRecord {
        doc_id: doc_id.to_owned(),
        doc_hash: format!("blake3:doc-{doc_id}-{generation}"),
        fields,
        updated_generation: Generation::new(generation),
    }
}

/// One `passages` row, alive, created at `generation`.
///
/// `segment_id` names no segment of the fixture: the filter structures read
/// `records.sqlite` only, and no test here reads a vector file.
pub fn passage(iid: u32, doc_id: &str, ord: u32, fields: Value, generation: u64) -> PassageRecord {
    PassageRecord {
        iid: Iid::new(iid),
        doc_id: doc_id.to_owned(),
        ord,
        text: format!("{doc_id}#{ord}"),
        passage_hash: format!("blake3:passage-{doc_id}-{ord}"),
        fields,
        segment_id: "fixture-segment".to_owned(),
        created_generation: Generation::new(generation),
        dead_since_gen: None,
    }
}

/// Iids `sql_matches` keeps for `filter`, in ascending order.
pub fn sql_matches(connection: &Connection, filter: &Filter, schema: &Schema) -> Vec<u32> {
    let sql = format!(
        "SELECT p.iid FROM passages p \
         LEFT JOIN documents d ON d.doc_id = p.doc_id \
         WHERE p.dead_since_gen IS NULL AND {} ORDER BY p.iid",
        sql_expression(filter, schema)
    );
    let mut statement = connection
        .prepare(&sql)
        .unwrap_or_else(|error| panic!("the oracle query must prepare: {error}\n{sql}"));
    let rows = statement
        .query_map([], |row| row.get::<_, i64>(0))
        .unwrap_or_else(|error| panic!("the oracle query must run: {error}\n{sql}"));
    rows.map(|row| {
        let iid = row.unwrap_or_else(|error| panic!("the oracle must read an iid: {error}"));
        u32::try_from(iid).unwrap_or_else(|error| panic!("an iid must fit in u32: {error}"))
    })
    .collect()
}

/// SQL expression that is true for exactly the rows `filter` matches.
///
/// Every node is two-valued: SQL's UNKNOWN never escapes a leaf, so `NOT` and
/// `AND`/`OR` compose exactly like the bitmaps do (`docs/03-recherche.md`).
pub fn sql_expression(filter: &Filter, schema: &Schema) -> String {
    match filter {
        Filter::Condition(condition) => condition_expression(condition, schema),
        Filter::And(children) if children.is_empty() => "(1)".to_owned(),
        Filter::Or(children) if children.is_empty() => "(0)".to_owned(),
        Filter::And(children) => format!("({})", join(children, schema, "AND")),
        Filter::Or(children) => format!("({})", join(children, schema, "OR")),
        Filter::Not(child) => format!("(NOT {})", sql_expression(child, schema)),
    }
}

fn join(children: &[Filter], schema: &Schema, operator: &str) -> String {
    children
        .iter()
        .map(|child| sql_expression(child, schema))
        .collect::<Vec<_>>()
        .join(&format!(" {operator} "))
}

fn condition_expression(condition: &Condition, schema: &Schema) -> String {
    let alias = match schema.field(&condition.field) {
        Some((_, FieldLevel::Document)) => "d",
        Some((_, FieldLevel::Passage)) => "p",
        None => panic!(
            "the oracle must not receive an undeclared field '{}'",
            condition.field
        ),
    };
    let path = format!("json_extract({alias}.fields, '$.{}')", condition.field);
    let value = &condition.value;
    match condition.operator {
        Operator::Eq => format!("COALESCE({path} = {}, 0)", sql_literal(value)),
        Operator::Ne => format!("COALESCE({path} <> {}, 0)", sql_literal(value)),
        Operator::Lt => format!("COALESCE({path} < {}, 0)", sql_literal(value)),
        Operator::Lte => format!("COALESCE({path} <= {}, 0)", sql_literal(value)),
        Operator::Gt => format!("COALESCE({path} > {}, 0)", sql_literal(value)),
        Operator::Gte => format!("COALESCE({path} >= {}, 0)", sql_literal(value)),
        // An empty `in` is the empty union and an empty `nin` is its complement
        // inside the present set (ADR-0035). SQLite's `IN ()` already returns
        // false, but `NOT IN ()` returns true even for a NULL, which would
        // wrongly keep a null passage; the two empty cases are therefore
        // rendered explicitly.
        Operator::In if is_empty_list(value) => "(0)".to_owned(),
        Operator::Nin if is_empty_list(value) => format!("({path} IS NOT NULL)"),
        Operator::In => format!("COALESCE({path} IN ({}), 0)", sql_list(value)),
        Operator::Nin => format!("COALESCE({path} NOT IN ({}), 0)", sql_list(value)),
        Operator::Exists => {
            let expected = value.as_bool().unwrap_or_else(|| {
                panic!("a validated filter must carry a boolean for 'exists', got {value}")
            });
            if expected {
                format!("({path} IS NOT NULL)")
            } else {
                format!("({path} IS NULL)")
            }
        }
    }
}

/// True when the operand of an `in` or `nin` is an empty array.
fn is_empty_list(value: &Value) -> bool {
    value.as_array().is_some_and(Vec::is_empty)
}

fn sql_list(value: &Value) -> String {
    let values = value
        .as_array()
        .unwrap_or_else(|| panic!("a validated 'in' must carry an array, got {value}"));
    values
        .iter()
        .map(sql_literal)
        .collect::<Vec<_>>()
        .join(", ")
}

fn sql_literal(value: &Value) -> String {
    match value {
        Value::String(text) => format!("'{}'", text.replace('\'', "''")),
        Value::Bool(flag) => if *flag { "1" } else { "0" }.to_owned(),
        Value::Number(number) => match (number.as_i64(), number.as_f64()) {
            (Some(integer), _) => integer.to_string(),
            (None, Some(float)) => format!("{float:?}"),
            (None, None) => panic!("the oracle cannot render the number {number}"),
        },
        other => panic!("the oracle cannot render the operand {other}"),
    }
}

/// Fields of the fixture document carried by `value`, for failure messages.
pub fn json_object(entries: Vec<(&str, Value)>) -> Value {
    let mut object = Map::new();
    for (key, value) in entries {
        object.insert(key.to_owned(), value);
    }
    Value::Object(object)
}

/// Sorted iids of a bitmapped set, for readable assertions.
pub fn sorted(bitmap: &roaring::RoaringBitmap) -> Vec<u32> {
    bitmap.iter().collect()
}

/// Path of the reference database of an index directory, without a `Fixture`.
pub fn records_path_of(index: &Path) -> PathBuf {
    index
        .join("epochs")
        .join(EPOCH.to_string())
        .join("records.sqlite")
}
