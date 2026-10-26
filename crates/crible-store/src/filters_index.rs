// SPDX-License-Identifier: Apache-2.0

//! Filter structures of one generation, held in RAM.
//!
//! References: `docs/03-recherche.md` ("Filtres → pool"), ADR-0018 (interval
//! filters on value columns), ADR-0019 (fields stored as canonical JSON),
//! ADR-0030 (a field name designates one level), ADR-0035 (the semantics of
//! null values).
//!
//! # Structures
//!
//! One structure per declared field, derived from `records.sqlite`:
//!
//! | Field type | Structure |
//! |---|---|
//! | `keyword` | one bitmap per distinct value, plus a "present" bitmap |
//! | `bool` | a bitmap for `false`, one for `true`, plus a "present" bitmap |
//! | `integer`, `float`, `date` | a value column indexed by iid, plus a "present" bitmap |
//!
//! A `date` is stored as a count of days since 1970-01-01, so the column is an
//! `i32` (ADR-0018). A column holds a sentinel at every iid it does not cover:
//! `i64::MIN`, `i32::MIN` and `NaN`. The sentinel never decides anything on its
//! own, because **the "present" bitmap is the authority**: an iid marked present
//! reads the value stored in the column, even when that value equals the
//! sentinel.
//!
//! "Present" means **present with a non-null value** (ADR-0035): a field that
//! the reference state stores as `null`, or that the row does not carry at all,
//! is absent from the present bitmap. `exists: true` is therefore exactly that
//! bitmap, and `ne`/`nin` are computed as `present ∩ ¬bitmap(...)` by the task
//! that evaluates filters.
//!
//! # Which rows the structures cover
//!
//! Every row of `passages`, dead ones included, so that the structures are a
//! plain image of the reference state and a tombstone never has to rewrite
//! them. The `live` bitmap carries the distinction, and `docs/03-recherche.md`
//! asks for the pool to be intersected with the live passages at the end of the
//! evaluation. This is what makes a copy-on-write update cheap: a tombstone only
//! clears one bit.
//!
//! A passage inherits the fields of its document (`docs/03-recherche.md`). The
//! structures therefore materialise the value a passage actually carries: a
//! document-level field comes from the `documents` row of the passage, a
//! passage-level field from the `passages` row (ADR-0030: a name designates one
//! level, never both). A dead passage whose `documents` row was removed by its
//! deletion (`docs/04-donnees-et-ingestion.md`, "Document retiré", ADR-0041) has
//! no document-level value left, exactly as a field stored as `null` has none.
//!
//! # The delta of a commit
//!
//! [`FilterIndex::after_commit`] receives the delta the transaction has just
//! written, as [`CommitDelta`] (ADR-0066): the passages it created, the
//! documents whose row it rewrote, the passages it tombstoned, and the
//! documents it removed. Re-reading those rows by generation stamp
//! (`created_generation`, `updated_generation`) would make the commit cost a
//! full scan of `passages` and of `documents`, so the cost of a generation
//! would follow the size of the index instead of the size of the change.
//!
//! Two SQL accesses remain, both bounded: the passages of one document by
//! `passages(doc_id)`, and the fields of one document by the primary key
//! `documents(doc_id)`. The caller that fills the delta is the only description
//! of the change: a document it forgets leaves the structures wrong in silence,
//! which is why the equality with [`FilterIndex::from_records`] is the guard
//! (ADR-0066, "À surveiller"). The acceptance test of `CONSTRUIRE-BITMAPS`
//! checks it on `synth-small`, whose `L3` removes 150 documents, then after a
//! metadata patch (ADR-0060).
//!
//! # Update in copy on write
//!
//! [`FilterIndex::after_commit`] derives the structures of generation G+1 from
//! those of G plus that delta. Each bitmap sits behind an `Arc`, and each column
//! is a sequence of `Arc` blocks, so a value no change touches is shared
//! between the two generations instead of being copied: a reader holding the
//! previous snapshot keeps reading it whole (INV-02, INV-03).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crible_core::config::{FieldLevel, FieldType, Schema};
use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::Iid;
use roaring::RoaringBitmap;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use crate::records::{PassageRecord, canonical_json, database_error, from_sql_u32};

/// Number of values one block of a value column holds.
///
/// A block is the unit of copy on write of a column: an update clones the
/// blocks it touches, and one block at most ends up half full. The size is a
/// constant of the code, not a setting (rule 12 of `AGENTS.md`).
const COLUMN_BLOCK: usize = 1 << 16;

/// Estimated heap size of the filter structures of one generation, in bytes.
///
/// The measure [`FilterIndex::memory`] returns and `crible inspect` reports. It
/// counts the bytes a bitmap would take serialized, the bytes of the value
/// columns, and the bytes of the `keyword` values themselves; it ignores the
/// maps, the `Arc` headers and the header an empty bitmap keeps, which are a
/// small, fixed part of the total. A structure that holds no passage therefore
/// reports nothing, and a fresh index reports a null size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FilterMemory {
    /// The bitmap of the live passages.
    pub live: usize,
    /// The per-value and present bitmaps of the `keyword` fields, plus the values.
    pub keywords: usize,
    /// The bitmaps of the `bool` fields.
    pub booleans: usize,
    /// The value columns and their present bitmaps.
    pub columns: usize,
}

impl FilterMemory {
    /// Sum of the four parts, in bytes.
    #[must_use]
    pub fn total(&self) -> usize {
        self.live + self.keywords + self.booleans + self.columns
    }
}

/// Bytes a bitmap contributes to [`FilterMemory`].
///
/// A roaring bitmap that holds no value still serializes a header; the measure
/// counts the data, not the fixed skeleton of the structures, so an empty
/// bitmap reports nothing at all.
fn bitmap_bytes(bitmap: &RoaringBitmap) -> usize {
    if bitmap.is_empty() {
        0
    } else {
        bitmap.serialized_size()
    }
}

/// One passage a committed transaction created, as the derivation of the next
/// filter structures needs it (ADR-0066).
///
/// The text and the hash are left out: they never reach a filter structure, and
/// keeping them would double the memory the delta holds until the snapshot is
/// published.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CreatedPassage {
    /// Internal identifier the transaction assigned, never exposed (INV-01).
    pub iid: Iid,
    /// External identifier of the document the passage belongs to.
    pub doc_id: String,
    /// Passage fields, with the values the transaction wrote.
    pub fields: Value,
    /// Whether the transaction marked the passage dead as well.
    pub dead: bool,
}

impl From<PassageRecord> for CreatedPassage {
    fn from(record: PassageRecord) -> Self {
        Self {
            iid: record.iid,
            doc_id: record.doc_id,
            fields: record.fields,
            dead: record.dead_since_gen.is_some(),
        }
    }
}

/// What one committed transaction wrote into `passages` and `documents`, in the
/// form [`FilterIndex::after_commit`] consumes (ADR-0066).
///
/// The caller fills it from the change it wrote: `GenerationChange` on the
/// commit path, the plan of the attach on its path. Every access
/// [`FilterIndex::after_commit`] still makes to SQLite is bounded by an index
/// of `docs/02-architecture.md`, so the commit never scans a table whose size
/// is the size of the index.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct CommitDelta {
    /// Passages the transaction created.
    pub created: Vec<CreatedPassage>,
    /// Documents whose `documents` row the transaction wrote: their passages
    /// inherit the new document-level values.
    pub rewritten_documents: Vec<String>,
    /// Passages the transaction tombstoned.
    pub tombstones: Vec<Iid>,
    /// Documents whose `documents` row the transaction removed.
    pub removed_documents: Vec<String>,
}

/// The filter structures of one generation.
///
/// Built by [`FilterIndex::from_records`] at creation and at startup, then
/// derived one generation at a time by [`FilterIndex::after_commit`].
#[derive(Debug, Clone, PartialEq)]
pub struct FilterIndex {
    schema: Schema,
    live: Arc<RoaringBitmap>,
    data: FieldData,
}

/// Every per-field structure, keyed by field name.
#[derive(Debug, Clone, PartialEq, Default)]
struct FieldData {
    keywords: BTreeMap<String, KeywordField>,
    booleans: BTreeMap<String, BoolField>,
    columns: BTreeMap<String, ValueColumn>,
}

impl FieldData {
    /// Writes one value of one field for one passage.
    ///
    /// A null value writes nothing for a `keyword` or a `bool` field, and writes
    /// the sentinel of the column for an `integer`, `float` or `date` field: a
    /// column is indexed by iid, so it holds one slot per passage, the sentinel
    /// standing in for the nulls (ADR-0018). Only a non-null value marks the
    /// passage present (ADR-0035).
    ///
    /// `placement` decides for a value column only: it says whether the value
    /// extends the dense range of iids (a row a generation creates) or replaces
    /// one the column covers (a value a metadata-only update rewrites). A
    /// `keyword` or a `bool` field is keyed by value, so it has no order to
    /// respect.
    fn insert(
        &mut self,
        field: &str,
        field_type: FieldType,
        iid: Iid,
        value: &Value,
        placement: Placement,
    ) -> Result<(), CribleError> {
        match field_type {
            FieldType::Keyword => {
                if value.is_null() {
                    return Ok(());
                }
                let text = value
                    .as_str()
                    .ok_or_else(|| type_mismatch(field, field_type))?;
                self.keyword(field)?.insert(iid, text);
                Ok(())
            }
            FieldType::Bool => {
                if value.is_null() {
                    return Ok(());
                }
                let flag = value
                    .as_bool()
                    .ok_or_else(|| type_mismatch(field, field_type))?;
                self.boolean(field)?.insert(iid, flag);
                Ok(())
            }
            FieldType::Integer => {
                let number = match value.as_i64() {
                    Some(number) => Some(number),
                    None if value.is_null() => None,
                    None => return Err(type_mismatch(field, field_type)),
                };
                self.column(field, field_type)?
                    .push_integer(iid, number, placement)
            }
            FieldType::Float => {
                let number = match value.as_f64() {
                    Some(number) => Some(number),
                    None if value.is_null() => None,
                    None => return Err(type_mismatch(field, field_type)),
                };
                self.column(field, field_type)?
                    .push_float(iid, number, placement)
            }
            FieldType::Date => {
                let days = match value {
                    Value::Null => None,
                    Value::String(text) => {
                        Some(date_days(text).ok_or_else(|| invalid_date(field, text))?)
                    }
                    _ => return Err(type_mismatch(field, field_type)),
                };
                self.column(field, field_type)?
                    .push_days(iid, days, placement)
            }
        }
    }

    /// Clears every structure of one field for one passage.
    ///
    /// A column takes its sentinel back at that iid, so the structures stay a
    /// plain image of the reference state: a null and an untouched slot read the
    /// same, and a rebuild gives the same column.
    fn clear(&mut self, field: &str, field_type: FieldType, iid: Iid) -> Result<(), CribleError> {
        match field_type {
            FieldType::Keyword => self.keyword(field)?.clear(iid),
            FieldType::Bool => self.boolean(field)?.clear(iid),
            FieldType::Integer | FieldType::Float | FieldType::Date => {
                self.column(field, field_type)?.clear(iid)?;
            }
        }
        Ok(())
    }

    fn keyword(&mut self, field: &str) -> Result<&mut KeywordField, CribleError> {
        self.keywords
            .get_mut(field)
            .ok_or_else(|| not_indexed(field, FieldType::Keyword))
    }

    fn boolean(&mut self, field: &str) -> Result<&mut BoolField, CribleError> {
        self.booleans
            .get_mut(field)
            .ok_or_else(|| not_indexed(field, FieldType::Bool))
    }

    fn column(
        &mut self,
        field: &str,
        field_type: FieldType,
    ) -> Result<&mut ValueColumn, CribleError> {
        self.columns
            .get_mut(field)
            .ok_or_else(|| not_indexed(field, field_type))
    }
}

/// Inserts one iid into a present bitmap, copying the bitmap only when it is
/// shared with another generation.
fn insert_present(present: &mut Arc<RoaringBitmap>, iid: Iid) {
    Arc::make_mut(present).insert(iid.get());
}

/// Removes one iid from a present bitmap.
fn clear_present(present: &mut Arc<RoaringBitmap>, iid: Iid) {
    if present.contains(iid.get()) {
        Arc::make_mut(present).remove(iid.get());
    }
}

/// The bitmaps of one `keyword` field: one per distinct value, plus presence.
///
/// A key is an `Arc<str>` and a value bitmap an `Arc<RoaringBitmap>`, both
/// shared with the generation the field was read from. Cloning the field copies
/// the `BTreeMap` itself: every node is allocated again, while each key and each
/// bitmap is only handed over through its `Arc`, so no value of the field is
/// duplicated. An insert then clones the `O(log n)` nodes of the path it
/// touches, plus the one bitmap it extends.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KeywordField {
    values: BTreeMap<Arc<str>, Arc<RoaringBitmap>>,
    present: Arc<RoaringBitmap>,
}

impl KeywordField {
    /// Passages carrying `value`, or `None` when no passage carries it.
    #[must_use]
    pub fn bitmap(&self, value: &str) -> Option<&RoaringBitmap> {
        self.values.get(value).map(Arc::as_ref)
    }

    /// Passages carrying a non-null value of this field.
    #[must_use]
    pub fn present(&self) -> &RoaringBitmap {
        &self.present
    }

    /// Every distinct value, in ascending order, with its bitmap.
    pub fn values(&self) -> impl Iterator<Item = (&str, &RoaringBitmap)> {
        self.values
            .iter()
            .map(|(value, bitmap)| (value.as_ref(), bitmap.as_ref()))
    }

    /// Number of distinct values.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// True when no passage carries a value of this field.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    fn insert(&mut self, iid: Iid, value: &str) {
        match self.values.get_mut(value) {
            Some(bitmap) => {
                Arc::make_mut(bitmap).insert(iid.get());
            }
            None => {
                let mut bitmap = RoaringBitmap::new();
                bitmap.insert(iid.get());
                self.values.insert(Arc::from(value), Arc::new(bitmap));
            }
        }
        insert_present(&mut self.present, iid);
    }

    /// Clears the value one passage carries.
    ///
    /// The structure holds no index from an iid back to its value
    /// (`docs/03-recherche.md` asks for one bitmap per value, nothing more), so
    /// finding the value of one passage reads every distinct value of the field.
    /// A `keyword` field whose cardinality approaches the number of passages, on
    /// a document that a metadata-only update rewrites, therefore costs one full
    /// sweep per passage of that document. That cost is assumed here: adding an
    /// inverse `iid -> value` structure would be another design, and is left to
    /// a measure if such a field ever appears.
    fn clear(&mut self, iid: Iid) {
        clear_present(&mut self.present, iid);
        let mut emptied: Vec<Arc<str>> = Vec::new();
        for (value, bitmap) in &mut self.values {
            if bitmap.contains(iid.get()) {
                Arc::make_mut(bitmap).remove(iid.get());
                if bitmap.is_empty() {
                    emptied.push(Arc::clone(value));
                }
            }
        }
        for value in emptied {
            self.values.remove(value.as_ref());
        }
    }

    /// Bytes of the bitmaps and of the values they are keyed by.
    fn memory(&self) -> usize {
        let values: usize = self
            .values
            .iter()
            .map(|(value, bitmap)| value.len() + bitmap_bytes(bitmap))
            .sum();
        values + bitmap_bytes(&self.present)
    }
}

/// The bitmaps of one `bool` field: `false`, `true`, plus presence.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BoolField {
    falses: Arc<RoaringBitmap>,
    trues: Arc<RoaringBitmap>,
    present: Arc<RoaringBitmap>,
}

impl BoolField {
    /// Passages whose value is `value`.
    #[must_use]
    pub fn bitmap(&self, value: bool) -> &RoaringBitmap {
        if value { &self.trues } else { &self.falses }
    }

    /// Passages carrying a non-null value of this field.
    #[must_use]
    pub fn present(&self) -> &RoaringBitmap {
        &self.present
    }

    fn insert(&mut self, iid: Iid, value: bool) {
        let bitmap = if value {
            &mut self.trues
        } else {
            &mut self.falses
        };
        Arc::make_mut(bitmap).insert(iid.get());
        insert_present(&mut self.present, iid);
    }

    fn clear(&mut self, iid: Iid) {
        clear_present(&mut self.present, iid);
        clear_present(&mut self.falses, iid);
        clear_present(&mut self.trues, iid);
    }

    fn memory(&self) -> usize {
        bitmap_bytes(&self.falses) + bitmap_bytes(&self.trues) + bitmap_bytes(&self.present)
    }
}

/// The value column of one `integer`, `float` or `date` field, plus presence.
///
/// One variant per column type, so an exact comparison never goes through an
/// `f64` it was not stored as: `i64` and the `i32` count of days stay exact.
#[derive(Debug, Clone, PartialEq)]
pub struct ValueColumn {
    field_type: FieldType,
    values: ColumnValues,
    present: Arc<RoaringBitmap>,
}

impl ValueColumn {
    /// An empty column, of the type its field declares.
    ///
    /// A `keyword` or a `bool` field is indexed by bitmaps, never by a column:
    /// asking for one is a contradiction of the schema and is refused.
    fn new(field_type: FieldType) -> Result<Self, CribleError> {
        let values = match field_type {
            FieldType::Integer => ColumnValues::Integers(Column::new()),
            FieldType::Float => ColumnValues::Floats(Column::new()),
            FieldType::Date => ColumnValues::Days(Column::new()),
            FieldType::Keyword | FieldType::Bool => return Err(wrong_type(field_type)),
        };
        Ok(Self {
            field_type,
            values,
            present: Arc::new(RoaringBitmap::new()),
        })
    }

    /// Type of the field the column belongs to.
    #[must_use]
    pub fn field_type(&self) -> FieldType {
        self.field_type
    }

    /// Passages carrying a non-null value of this field.
    #[must_use]
    pub fn present(&self) -> &RoaringBitmap {
        &self.present
    }

    /// Number of iids the column covers, from iid 0.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// True when the column covers no iid at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Value of an `integer` field at `iid`, or `None` when it is null.
    ///
    /// Returns `None` too when the column holds another type: a caller reads it
    /// through [`ValueColumn::field_type`] first, and the task that evaluates
    /// filters dispatches on that type.
    #[must_use]
    pub fn integer(&self, iid: Iid) -> Option<i64> {
        match &self.values {
            ColumnValues::Integers(values) => self
                .present
                .contains(iid.get())
                .then(|| values.get(iid.get())),
            ColumnValues::Floats(_) | ColumnValues::Days(_) => None,
        }
    }

    /// Value of a `float` field at `iid`, or `None` when it is null.
    #[must_use]
    pub fn float(&self, iid: Iid) -> Option<f64> {
        match &self.values {
            ColumnValues::Floats(values) => self
                .present
                .contains(iid.get())
                .then(|| values.get(iid.get())),
            ColumnValues::Integers(_) | ColumnValues::Days(_) => None,
        }
    }

    /// Day of a `date` field at `iid`, in days since 1970-01-01, or `None` when
    /// it is null.
    #[must_use]
    pub fn date_days(&self, iid: Iid) -> Option<i32> {
        match &self.values {
            ColumnValues::Days(values) => self
                .present
                .contains(iid.get())
                .then(|| values.get(iid.get())),
            ColumnValues::Integers(_) | ColumnValues::Floats(_) => None,
        }
    }

    /// Writes an `integer` value, or the sentinel of the column when it is null.
    fn push_integer(
        &mut self,
        iid: Iid,
        value: Option<i64>,
        placement: Placement,
    ) -> Result<(), CribleError> {
        let ColumnValues::Integers(values) = &mut self.values else {
            return Err(wrong_type(FieldType::Integer));
        };
        values.write(iid, value.unwrap_or(i64::ABSENT), placement)?;
        if value.is_some() {
            insert_present(&mut self.present, iid);
        }
        Ok(())
    }

    /// Writes a `float` value, or the sentinel of the column when it is null.
    fn push_float(
        &mut self,
        iid: Iid,
        value: Option<f64>,
        placement: Placement,
    ) -> Result<(), CribleError> {
        let ColumnValues::Floats(values) = &mut self.values else {
            return Err(wrong_type(FieldType::Float));
        };
        values.write(iid, value.unwrap_or(f64::ABSENT), placement)?;
        if value.is_some() {
            insert_present(&mut self.present, iid);
        }
        Ok(())
    }

    /// Writes a day count, or the sentinel of the column when the date is null.
    fn push_days(
        &mut self,
        iid: Iid,
        value: Option<i32>,
        placement: Placement,
    ) -> Result<(), CribleError> {
        let ColumnValues::Days(values) = &mut self.values else {
            return Err(wrong_type(FieldType::Date));
        };
        values.write(iid, value.unwrap_or(i32::ABSENT), placement)?;
        if value.is_some() {
            insert_present(&mut self.present, iid);
        }
        Ok(())
    }

    /// Clears the value at `iid`: the column takes its sentinel back, so a
    /// rebuild and a copy-on-write update read the same.
    fn clear(&mut self, iid: Iid) -> Result<(), CribleError> {
        match &mut self.values {
            ColumnValues::Integers(values) => {
                values.write(iid, i64::ABSENT, Placement::Overwrite)?
            }
            ColumnValues::Floats(values) => values.write(iid, f64::ABSENT, Placement::Overwrite)?,
            ColumnValues::Days(values) => values.write(iid, i32::ABSENT, Placement::Overwrite)?,
        }
        clear_present(&mut self.present, iid);
        Ok(())
    }

    fn memory(&self) -> usize {
        self.values.memory() + bitmap_bytes(&self.present)
    }
}

/// The storage of a value column, one variant per declared type.
#[derive(Debug, Clone, PartialEq)]
enum ColumnValues {
    Integers(Column<i64>),
    Floats(Column<f64>),
    Days(Column<i32>),
}

impl ColumnValues {
    fn len(&self) -> usize {
        match self {
            Self::Integers(values) => values.len(),
            Self::Floats(values) => values.len(),
            Self::Days(values) => values.len(),
        }
    }

    fn memory(&self) -> usize {
        match self {
            Self::Integers(values) => values.memory(),
            Self::Floats(values) => values.memory(),
            Self::Days(values) => values.memory(),
        }
    }
}

/// A column asked to take a value of another type than the one it holds.
fn wrong_type(field_type: FieldType) -> CribleError {
    CribleError::new(
        ErrorCode::Internal,
        format!(
            "the filter structures hold no {} column for this field",
            field_type.as_str()
        ),
    )
}

/// Sentinel stored at every iid a column does not cover.
///
/// The presence bitmap is the authority, so a real value equal to the sentinel
/// still reads correctly; the sentinel only fills the slots a null leaves.
trait Sentinel: Copy {
    /// Value of an iid the column does not carry.
    const ABSENT: Self;

    /// Equality as the structures mean it, which is not the equality of the
    /// type: the sentinel of a `float` column is `NaN`, and a `NaN` slot has to
    /// equal the `NaN` slot a rebuild writes at the same iid.
    fn same(self, other: Self) -> bool;
}

impl Sentinel for i64 {
    const ABSENT: Self = i64::MIN;

    fn same(self, other: Self) -> bool {
        self == other
    }
}

impl Sentinel for i32 {
    const ABSENT: Self = i32::MIN;

    fn same(self, other: Self) -> bool {
        self == other
    }
}

impl Sentinel for f64 {
    const ABSENT: Self = f64::NAN;

    fn same(self, other: Self) -> bool {
        self == other || (self.is_nan() && other.is_nan())
    }
}

/// The column does not cover the slot a write names.
fn slot_missing() -> CribleError {
    CribleError::new(
        ErrorCode::Internal,
        "the value column has no slot for the row to write",
    )
}

/// How a value writes into a column.
///
/// A column is indexed by iid, so it holds exactly one slot per passage, from 0
/// to its length: a row a generation creates extends it, and a value a
/// generation rewrites replaces the one at an iid it already covers. Telling
/// the two apart keeps a created passage that does not extend the range from
/// silently overwriting another one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placement {
    /// The value of a row the column does not cover yet: the column grows by
    /// one slot, and an iid that is not the next one is refused.
    Append,
    /// The value of an iid the column already covers: it replaces the old one,
    /// and an iid the column does not cover is refused.
    Overwrite,
}

/// A value column indexed by iid, in blocks shared between generations.
///
/// The column grows by [`Column::push`], in ascending iid order, which is how
/// [`FilterIndex::after_commit`] appends the passages of a generation. A block
/// is copied only when one of its values changes, so publishing a generation
/// does not copy the column.
///
/// Equality is the equality of the structures, one slot after the other, with
/// [`Sentinel::same`] deciding for one value: `#[derive(PartialEq)]` would make
/// a column differ from its rebuild on every null `float`, because `NaN` is not
/// equal to itself.
#[derive(Debug, Clone)]
struct Column<T> {
    blocks: Vec<Arc<Vec<T>>>,
    len: usize,
}

impl<T: Sentinel> PartialEq for Column<T> {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len
            && self.blocks.len() == other.blocks.len()
            && self.blocks.iter().zip(&other.blocks).all(|(left, right)| {
                left.len() == right.len()
                    && left
                        .iter()
                        .zip(right.iter())
                        .all(|(left, right)| left.same(*right))
            })
    }
}

impl<T: Sentinel> Column<T> {
    /// A column that covers no iid yet.
    fn new() -> Self {
        Self {
            blocks: Vec::new(),
            len: 0,
        }
    }

    fn len(&self) -> usize {
        self.len
    }

    fn push(&mut self, value: T) {
        match self.blocks.last_mut() {
            Some(last) if last.len() < COLUMN_BLOCK => Arc::make_mut(last).push(value),
            _ => self.blocks.push(Arc::new(vec![value])),
        }
        self.len += 1;
    }

    fn get(&self, index: u32) -> T {
        let index = index as usize;
        let block = index / COLUMN_BLOCK;
        let offset = index % COLUMN_BLOCK;
        self.blocks
            .get(block)
            .and_then(|values| values.get(offset))
            .copied()
            .unwrap_or(T::ABSENT)
    }

    /// Writes the value of `iid`, at the place `placement` names: a created row
    /// extends the range, any other write replaces a value the column covers.
    fn write(&mut self, iid: Iid, value: T, placement: Placement) -> Result<(), CribleError> {
        match placement {
            Placement::Append if iid.get() as usize == self.len => {
                self.push(value);
                Ok(())
            }
            Placement::Append => Err(CribleError::new(
                ErrorCode::Internal,
                format!(
                    "the value column covers {} values, and a passage created at iid {} would \
                     not extend that dense range",
                    self.len,
                    iid.get()
                ),
            )),
            Placement::Overwrite => self.set(iid.get(), value),
        }
    }

    fn set(&mut self, index: u32, value: T) -> Result<(), CribleError> {
        let position = index as usize;
        let block = position / COLUMN_BLOCK;
        let offset = position % COLUMN_BLOCK;
        let Some(values) = self.blocks.get_mut(block) else {
            return Err(slot_missing());
        };
        let Some(slot) = Arc::make_mut(values).get_mut(offset) else {
            return Err(slot_missing());
        };
        *slot = value;
        Ok(())
    }

    fn memory(&self) -> usize {
        std::mem::size_of::<T>() * self.len
    }
}

impl FilterIndex {
    /// Builds the structures of the reference state, from `records.sqlite`.
    ///
    /// Every declared field gets its structure, empty when no row carries a
    /// value for it, so the shape of the index follows the schema and not the
    /// data. Every row of `passages` is read, in ascending iid order.
    pub fn from_records(connection: &Connection, schema: &Schema) -> Result<Self, CribleError> {
        let mut index = Self::empty(schema)?;
        let mut statement = connection
            .prepare(
                "SELECT p.iid, p.fields, d.fields, p.dead_since_gen
                 FROM passages p LEFT JOIN documents d ON d.doc_id = p.doc_id
                 ORDER BY p.iid",
            )
            .map_err(|error| database_error("prepare the filter structures query", &error))?;
        let mut rows = statement
            .query([])
            .map_err(|error| database_error("query the filter structures", &error))?;
        while let Some(row) = rows
            .next()
            .map_err(|error| database_error("read a row of the filter structures", &error))?
        {
            let iid = Iid::new(from_sql_u32(
                row.get(0)
                    .map_err(|error| database_error("read an iid", &error))?,
                "iid",
            )?);
            let passage = parse_fields(
                &row.get::<_, String>(1)
                    .map_err(|error| database_error("read the passage fields", &error))?,
            )?;
            let document = match row
                .get::<_, Option<String>>(2)
                .map_err(|error| database_error("read the document fields", &error))?
            {
                Some(raw) => Some(parse_fields(&raw)?),
                None => None,
            };
            let dead = row
                .get::<_, Option<i64>>(3)
                .map_err(|error| database_error("read the tombstone of a passage", &error))?;

            index.insert_row(iid, document.as_ref(), &passage)?;
            if dead.is_none() {
                Arc::make_mut(&mut index.live).insert(iid.get());
            }
        }
        Ok(index)
    }

    /// Derives the structures of the generation a transaction has just
    /// committed, from those of the previous generation and from `delta`
    /// (ADR-0066).
    ///
    /// `delta` is what the transaction wrote, as the caller filled it while
    /// writing: the passages it created, the documents whose row it rewrote,
    /// the passages it tombstoned, and the documents it removed. Nothing is
    /// re-read by generation stamp, so the commit never scans `passages` or
    /// `documents`; the two accesses that remain are bounded, by
    /// `passages(doc_id)` and by the primary key of `documents`. A value no
    /// change touches is shared with the previous generation, not copied.
    ///
    /// The result equals [`FilterIndex::from_records`] on the same reference
    /// state, which the acceptance test of `CONSTRUIRE-BITMAPS` checks after
    /// `L1`, `L2`, `L3` and a metadata patch (ADR-0060, ADR-0066).
    pub(crate) fn after_commit(
        &self,
        connection: &Connection,
        delta: &CommitDelta,
    ) -> Result<Self, CribleError> {
        let mut next = self.clone();
        next.insert_created_passages(connection, delta)?;
        next.apply_tombstones(&delta.tombstones);
        next.refresh_updated_documents(connection, &delta.rewritten_documents)?;
        next.clear_removed_documents(connection, &delta.removed_documents)?;
        Ok(next)
    }

    /// The schema the structures were built from.
    #[must_use]
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Bitmap of the live passages: every passage SQLite does not tombstone.
    ///
    /// This is the bitmap a caller reads: [`crate::generation::Snapshot::live_iids`]
    /// walks it directly, so no live identifier is materialised twice.
    #[must_use]
    pub fn live(&self) -> &RoaringBitmap {
        &self.live
    }

    /// Structures of one `keyword` field, or `None` when the schema declares no
    /// field of that name and type.
    #[must_use]
    pub fn keyword(&self, field: &str) -> Option<&KeywordField> {
        self.data.keywords.get(field)
    }

    /// Structures of one `bool` field, or `None` when the schema declares no
    /// field of that name and type.
    #[must_use]
    pub fn boolean(&self, field: &str) -> Option<&BoolField> {
        self.data.booleans.get(field)
    }

    /// Column of one `integer`, `float` or `date` field, or `None` when the
    /// schema declares no field of that name and type.
    #[must_use]
    pub fn column(&self, field: &str) -> Option<&ValueColumn> {
        self.data.columns.get(field)
    }

    /// Estimated heap size of the structures, for `crible inspect`.
    #[must_use]
    pub fn memory(&self) -> FilterMemory {
        let keywords: usize = self.data.keywords.values().map(KeywordField::memory).sum();
        let booleans: usize = self.data.booleans.values().map(BoolField::memory).sum();
        let columns: usize = self.data.columns.values().map(ValueColumn::memory).sum();
        FilterMemory {
            live: bitmap_bytes(&self.live),
            keywords,
            booleans,
            columns,
        }
    }

    /// An index with one empty structure per declared field.
    fn empty(schema: &Schema) -> Result<Self, CribleError> {
        let mut data = FieldData::default();
        for (name, _, spec) in schema.fields() {
            let name = name.to_owned();
            match spec.field_type {
                FieldType::Keyword => {
                    data.keywords.insert(name, KeywordField::default());
                }
                FieldType::Bool => {
                    data.booleans.insert(name, BoolField::default());
                }
                FieldType::Integer | FieldType::Float | FieldType::Date => {
                    data.columns
                        .insert(name, ValueColumn::new(spec.field_type)?);
                }
            }
        }
        Ok(Self {
            schema: schema.clone(),
            live: Arc::new(RoaringBitmap::new()),
            data,
        })
    }

    /// Materialises the value every declared field carries for one passage.
    ///
    /// A document-level field comes from the `documents` row of the passage, a
    /// passage-level one from the `passages` row. A missing member is a null:
    /// `attach` writes every declared field of its level, and a document row
    /// removed by a deletion leaves its dead passages without a document-level
    /// value at all.
    fn insert_row(
        &mut self,
        iid: Iid,
        document: Option<&Value>,
        passage: &Value,
    ) -> Result<(), CribleError> {
        for (name, level, spec) in self.schema.fields() {
            let holder = match level {
                FieldLevel::Document => document,
                FieldLevel::Passage => Some(passage),
            };
            let value = holder
                .and_then(|fields| fields.get(name))
                .unwrap_or(&Value::Null);
            self.data
                .insert(name, spec.field_type, iid, value, Placement::Append)?;
        }
        Ok(())
    }

    /// Replaces the document-level values one passage inherits.
    ///
    /// The passage keeps its own fields: only the values it inherits from its
    /// document change, which is what a metadata-only update does (`docs/04`).
    fn refresh_document_fields(&mut self, iid: Iid, document: &Value) -> Result<(), CribleError> {
        for (name, level, spec) in self.schema.fields() {
            if level != FieldLevel::Document {
                continue;
            }
            self.data.clear(name, spec.field_type, iid)?;
            let value = document.get(name).unwrap_or(&Value::Null);
            self.data
                .insert(name, spec.field_type, iid, value, Placement::Overwrite)?;
        }
        Ok(())
    }

    /// Clears the document-level values one passage inherited.
    fn clear_document_fields(&mut self, iid: Iid) -> Result<(), CribleError> {
        for (name, level, spec) in self.schema.fields() {
            if level != FieldLevel::Document {
                continue;
            }
            self.data.clear(name, spec.field_type, iid)?;
        }
        Ok(())
    }

    /// Step 1 of the delta: the passages this generation created.
    ///
    /// The passages come from the delta, in ascending `iid` order because a
    /// value column appends in that order. Their document-level values are the
    /// ones the transaction wrote, read back by the primary key of `documents`:
    /// one bounded lookup per distinct document, cached for the whole step.
    fn insert_created_passages(
        &mut self,
        connection: &Connection,
        delta: &CommitDelta,
    ) -> Result<(), CribleError> {
        let mut created: Vec<&CreatedPassage> = delta.created.iter().collect();
        created.sort_by_key(|passage| passage.iid.get());
        let mut documents: HashMap<&str, Option<Value>> = HashMap::new();
        for passage in created {
            let document = match documents.get(passage.doc_id.as_str()) {
                Some(fields) => fields.clone(),
                None => {
                    let fields = document_fields(connection, &passage.doc_id)?;
                    documents.insert(passage.doc_id.as_str(), fields.clone());
                    fields
                }
            };
            // The fields go through the same canonical JSON the transaction
            // stored, then back through the parser: a rebuild reads those bytes,
            // and serde_json's parser is not bit-exact on `float` by default, so
            // using the in-memory value would make the derived structures differ
            // from `from_records` by one unit in the last place.
            let fields = parse_fields(&canonical_json(&passage.fields)?)?;
            self.insert_row(passage.iid, document.as_ref(), &fields)?;
            if !passage.dead {
                Arc::make_mut(&mut self.live).insert(passage.iid.get());
            }
        }
        Ok(())
    }

    /// Step 2 of the delta: the passages this generation tombstoned are no
    /// longer live. Their other structures stay: a rebuild keeps them too.
    fn apply_tombstones(&mut self, tombstones: &[Iid]) {
        for iid in tombstones {
            Arc::make_mut(&mut self.live).remove(iid.get());
        }
    }

    /// Step 3 of the delta: the passages whose document had its row rewritten
    /// inherit the new values, the way a rebuild would give them.
    ///
    /// The passages of the document are read by `passages(doc_id)` and its
    /// fields by the primary key of `documents`, both bounded (ADR-0066).
    fn refresh_updated_documents(
        &mut self,
        connection: &Connection,
        rewritten: &[String],
    ) -> Result<(), CribleError> {
        for doc_id in rewritten {
            let Some(document) = document_fields(connection, doc_id)? else {
                continue;
            };
            for iid in passage_iids_of_document(connection, doc_id)? {
                self.refresh_document_fields(iid, &document)?;
            }
        }
        Ok(())
    }

    /// Step 4 of the delta: a document removed by its deletion leaves no
    /// document-level value to the passages it still has
    /// (`docs/04-donnees-et-ingestion.md`, "Document retiré", ADR-0041).
    ///
    /// The removed documents come from the delta, so no row has to record the
    /// removal; their passages are read by `passages(doc_id)`, which is bounded
    /// by the index of `docs/02-architecture.md`.
    fn clear_removed_documents(
        &mut self,
        connection: &Connection,
        removed: &[String],
    ) -> Result<(), CribleError> {
        for doc_id in removed {
            for iid in passage_iids_of_document(connection, doc_id)? {
                self.clear_document_fields(iid)?;
            }
        }
        Ok(())
    }
}

/// Reads the fields of one document by its primary key, or `None` when the
/// document has no row (ADR-0066).
fn document_fields(connection: &Connection, doc_id: &str) -> Result<Option<Value>, CribleError> {
    let raw = connection
        .query_row(
            "SELECT fields FROM documents WHERE doc_id = ?1",
            params![doc_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| database_error("read the fields of a document", &error))?;
    raw.map(|raw| parse_fields(&raw)).transpose()
}

/// Reads the iids of one document's passages, bounded by `passages(doc_id)`
/// (ADR-0066).
///
/// The dead passages are included: a rebuild gives them their document-level
/// values too, and the `live` bitmap alone hides them from a query.
fn passage_iids_of_document(
    connection: &Connection,
    doc_id: &str,
) -> Result<Vec<Iid>, CribleError> {
    let mut statement = connection
        .prepare("SELECT iid FROM passages WHERE doc_id = ?1 ORDER BY iid")
        .map_err(|error| database_error("prepare the passages of a document query", &error))?;
    let iids = statement
        .query_map(params![doc_id], |row| row.get::<_, i64>(0))
        .map_err(|error| database_error("query the passages of a document", &error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| database_error("read an iid of a document", &error))?;
    iids.into_iter()
        .map(|iid| from_sql_u32(iid, "iid").map(Iid::new))
        .collect()
}

/// Parses the canonical JSON of one row's fields.
fn parse_fields(raw: &str) -> Result<Value, CribleError> {
    serde_json::from_str(raw).map_err(|error| {
        CribleError::new(
            ErrorCode::Internal,
            format!("the reference state holds fields that are not valid JSON: {error}"),
        )
    })
}

/// A value of another JSON type than its field declares: the reference state
/// contradicts the schema of the index.
fn type_mismatch(field: &str, field_type: FieldType) -> CribleError {
    CribleError::new(
        ErrorCode::Internal,
        format!(
            "field '{field}' is declared {} and the reference state holds a value of another \
             JSON type",
            field_type.as_str()
        ),
    )
}

/// A field the schema declares, but whose structure the index does not hold.
fn not_indexed(field: &str, field_type: FieldType) -> CribleError {
    CribleError::new(
        ErrorCode::Internal,
        format!(
            "the filter structures hold nothing for field '{field}' ({})",
            field_type.as_str()
        ),
    )
}

/// A date the reference state holds in another form than `YYYY-MM-DD`.
fn invalid_date(field: &str, text: &str) -> CribleError {
    CribleError::new(
        ErrorCode::Internal,
        format!("field '{field}' holds '{text}', which is not a date written YYYY-MM-DD"),
    )
}

/// Days since 1970-01-01 of a date written `YYYY-MM-DD`.
///
/// The form is the one `attach` writes for a Parquet `date32`, so a value this
/// function refuses contradicts the reference state. The conversion is
/// Howard Hinnant's `days_from_civil`, the inverse of the `civil_from_days` the
/// attach uses.
///
/// Callers turn the date of a validated filter into the `i32` count of days a
/// [`ValueColumn`] holds: comparing the two forms directly would compare a
/// text with an integer.
#[must_use]
pub fn date_days(text: &str) -> Option<i32> {
    if text.len() != 10 || !text.is_ascii() {
        return None;
    }
    let bytes = text.as_bytes();
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes[..4].iter().all(u8::is_ascii_digit)
        || !bytes[5..7].iter().all(u8::is_ascii_digit)
        || !bytes[8..].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let year: i64 = text[..4].parse().ok()?;
    let month: i64 = text[5..7].parse().ok()?;
    let day: i64 = text[8..].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=days_in_month(year, month)).contains(&day) {
        return None;
    }
    let days = days_from_civil(year, month, day);
    i32::try_from(days).ok()
}

/// Length of a civil month, the leap day of a `2020-02-29` included.
///
/// The Gregorian rule, applied to a year of the proleptic calendar: a year is a
/// leap year when it is a multiple of 4, except a multiple of 100 that is not a
/// multiple of 400.
fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        _ => 28,
    }
}

/// Converts a civil date to a count of days since 1970-01-01.
///
/// Howard Hinnant's `days_from_civil`, exact over the whole `i32` range of a
/// `date32` and independent of any platform clock or time zone.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::{Column, Placement, Sentinel, date_days, days_from_civil};
    use crible_core::ids::Iid;

    /// The reference epoch, the leap day of a leap year, and the day before the
    /// epoch, which only a signed count can represent.
    #[test]
    fn date_days_counts_days_since_the_epoch() {
        assert_eq!(date_days("1970-01-01"), Some(0));
        assert_eq!(date_days("2020-02-29"), Some(18_321));
        assert_eq!(date_days("1969-12-31"), Some(-1));
        assert_eq!(date_days("2000-03-01"), Some(11_017));
        assert_eq!(days_from_civil(1970, 1, 1), 0);
    }

    /// A value that is not a `YYYY-MM-DD` date is refused, never silently read
    /// as some day: neither a shape SQLite did not write, nor a day its month
    /// does not have, which a mere `1..=31` check would let through.
    #[test]
    fn date_days_refuses_a_value_that_is_not_a_date() {
        assert_eq!(date_days("2020-2-29"), None);
        assert_eq!(date_days("2020-02-30x"), None);
        assert_eq!(date_days("2020-13-01"), None);
        assert_eq!(date_days("2020-02-00"), None);
        assert_eq!(date_days("not a date"), None);
        assert_eq!(date_days("+020-02-29"), None);
        assert_eq!(date_days("2020-02-30"), None);
        assert_eq!(date_days("2020-02-31"), None);
        assert_eq!(date_days("2021-02-29"), None);
        assert_eq!(date_days("2020-04-31"), None);
        assert_eq!(date_days("2020-00-10"), None);
    }

    /// The leap rule that decides the length of February.
    #[test]
    fn february_follows_the_gregorian_leap_rule() {
        assert_eq!(date_days("2000-02-29"), Some(11_016));
        assert_eq!(date_days("1900-02-28"), Some(-25_509));
        assert_eq!(date_days("1900-03-01"), Some(-25_508));
        assert_eq!(date_days("1900-02-29"), None);
    }

    /// A column grows in blocks and reads the sentinel outside the values it
    /// covers.
    #[test]
    fn a_column_reads_the_sentinel_outside_its_values() {
        let mut column: Column<i64> = Column::new();
        assert_eq!(column.get(0), i64::MIN);
        column.push(42);
        column.push(-7);
        assert_eq!(column.len(), 2);
        assert_eq!(column.get(0), 42);
        assert_eq!(column.get(1), -7);
        assert_eq!(column.get(2), i64::MIN);
        assert!(column.set(1, 5).is_ok());
        assert_eq!(column.get(1), 5);
    }

    /// A created passage must extend the dense range of a column: an append that
    /// would land before its end is refused, instead of overwriting the value of
    /// another passage. Replacing a value the column covers stays possible, and
    /// stays an error beyond the range.
    #[test]
    fn appending_a_column_out_of_sequence_is_refused() {
        let mut column: Column<i64> = Column::new();
        let error = column
            .write(Iid::new(1), 7, Placement::Append)
            .expect_err("a first value can only land at iid 0");
        assert_eq!(error.code(), crible_core::error::ErrorCode::Internal);

        assert!(column.write(Iid::new(0), 1, Placement::Append).is_ok());
        assert!(column.write(Iid::new(1), 2, Placement::Append).is_ok());
        assert!(
            column.write(Iid::new(0), 9, Placement::Append).is_err(),
            "a created passage never lands on the iid of an older one"
        );
        assert_eq!(column.get(0), 1, "the refused write must change nothing");
        assert!(
            column.write(Iid::new(9), 9, Placement::Overwrite).is_err(),
            "an overwrite beyond the range has no slot to write"
        );
        assert!(column.write(Iid::new(0), 9, Placement::Overwrite).is_ok());
        assert_eq!(column.get(0), 9);
    }

    /// A block is copied only when it is written: the column a clone reads is
    /// untouched by the writes of the copy.
    #[test]
    fn writing_a_copy_leaves_the_original_column_untouched() {
        let mut original: Column<i64> = Column::new();
        original.push(1);
        original.push(2);
        let mut copy = original.clone();

        assert!(copy.set(0, 9).is_ok());
        copy.push(3);

        assert_eq!(
            original.get(0),
            1,
            "the clone must share the untouched block"
        );
        assert_eq!(original.len(), 2);
        assert_eq!(copy.get(0), 9);
        assert_eq!(copy.get(2), 3);
        assert_eq!(copy.len(), 3);
    }

    /// Two columns that hold the sentinel at the same slots are equal, which a
    /// derived `PartialEq` would deny for a `float` column: the sentinel is a
    /// `NaN`, and `NaN` is not equal to itself. A rebuild and a copy-on-write
    /// update must still compare equal.
    #[test]
    fn two_columns_holding_the_sentinel_compare_equal() {
        let mut left: Column<f64> = Column::new();
        left.push(f64::ABSENT);
        left.push(1.5);
        left.push(f64::ABSENT);
        let mut right: Column<f64> = Column::new();
        right.push(f64::ABSENT);
        right.push(1.5);
        right.push(f64::ABSENT);

        assert_eq!(left, right, "the sentinel slots must compare equal");

        let mut other = right.clone();
        assert!(other.set(1, 2.5).is_ok());
        assert_ne!(
            left, other,
            "a value that differs must make the columns differ"
        );
    }
}
