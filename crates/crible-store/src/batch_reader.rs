// SPDX-License-Identifier: Apache-2.0

//! Reading and validating a batch (format v1) from its directory.
//!
//! References: `docs/format-lot-v1.md`, `docs/04-donnees-et-ingestion.md`
//! (controls 4 to 7), ADR-0012 (public batch format), ADR-0034 (blake3),
//! ADR-0039 (order of the rows of a document in a batch v1), ADR-0040 (order of
//! the parts by numeric suffix and unique `doc_id`s).
//!
//! [`BatchReader::open`] reads `manifest.json`, applies
//! [`crible_core::batch::check_manifest`] (controls 1 to 3 and the
//! manifest-only part of control 4), checks every declared checksum against the
//! bytes of its file, and checks the Parquet schema of every file (control 5).
//! [`BatchReader::validate`] then streams the files once and applies controls 6
//! and 7: counts, contiguous `ord`s, one passage per document, finite and
//! normalized vectors.
//!
//! **Order of the rows.** This order is part of format v1 (ADR-0039): the rows
//! of one document arrive in ascending `ord` order, starting at 0, and in that
//! order only, so a passage whose `ord` is not the next one expected for its
//! document is refused. A document may be split across several
//! `passages-*.parquet` parts, read in increasing numeric suffix order
//! (ADR-0040), as long as its `ord` sequence continues across the parts. The
//! order is also what keeps the memory of validation bounded: it holds one
//! entry per declared document, where accepting any row order would cost a
//! state proportional to the passages (the `ord`s already seen, per document).
//! The failure messages say that the rows must arrive in ascending `ord`
//! order, so a producer understands the refusal.
//!
//! **Uniqueness of a `doc_id`.** A `doc_id` appears at most once in the
//! documents of a batch, across all its parts, and at most once in its
//! deletions (ADR-0040). A duplicate is refused rather than merged: two
//! `doc_hash` values or two field sets for one document would make the result
//! depend on the order of the parts, and the rule "same `doc_hash`, same
//! content" would no longer hold.
//!
//! Memory stays bounded. Rows are read through Arrow record batches of at most
//! [`DEFAULT_BATCH_ROWS`] rows, and no Parquet file is ever read as a whole.
//! Validation keeps one entry per declared document, never one per passage, and
//! the set of deleted ids, which never grows past `counts.deletions`: a
//! manifest that declares fewer documents or deletions than its files hold is
//! refused before the excess is stored.
//!
//! A fault of the batch itself is an [`ErrorCode::InvalidRequest`] error: a
//! malformed manifest, an unknown or malformed checksum, a file that is missing
//! or does not match its checksum, a schema mismatch, an inconsistent count or
//! a bad vector. An unexpected state of the disk, such as a real I/O failure
//! while reading a file, is an [`ErrorCode::Internal`] error when the Parquet
//! or Arrow error keeps the I/O cause; see `arrow_failure` for the case where
//! arrow-rs flattens it.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};

use arrow::array::{Array, FixedSizeBinaryArray, RecordBatch, StringArray, UInt32Array};
use arrow::datatypes::{DataType, SchemaRef};
use crible_core::batch::{BatchManifest, VectorDtype, classify_files, parse_checksum};
use crible_core::config::{FieldLevel, FieldType, IndexConfig, ReferencePrecision, Schema};
use crible_core::error::{CribleError, ErrorCode};
use crible_core::model::ModelConfig;
use half::f16;
use parquet::arrow::arrow_reader::{ParquetRecordBatchReader, ParquetRecordBatchReaderBuilder};

/// Rows of one Arrow record batch by default.
///
/// A batch is read part by part with at most this many rows at once, so the
/// memory held does not depend on the size of the batch.
pub const DEFAULT_BATCH_ROWS: usize = 8_192;

/// Largest accepted deviation of the L2 norm of a vector from 1, when the batch
/// declares normalized vectors (`docs/format-lot-v1.md`, "Vecteurs").
pub const NORM_TOLERANCE: f64 = 1e-2;

/// Row counts actually read from a batch, after validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchCounts {
    /// Number of rows of the document files.
    pub documents: u64,
    /// Number of rows of the passage files.
    pub passages: u64,
    /// Number of rows of the deletion file.
    pub deletions: u64,
}

/// One role of a batch, which fixes the columns expected in its Parquet files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Documents,
    Passages,
    Deletions,
}

impl Role {
    /// Name used in error messages.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Documents => "documents",
            Self::Passages => "passages",
            Self::Deletions => "deletions",
        }
    }
}

/// Reads and validates one batch directory.
///
/// Opening a reader is cheap for everything but the checksums: `open` streams
/// every file once to compute its blake3 digest (control 4), so a reader never
/// trusts a manifest that lies about the content of its files. This is the same
/// cost as [`crate::segment::SegmentReader::open`], and it is paid once per
/// attach attempt, not once per generation.
#[derive(Debug)]
pub struct BatchReader {
    manifest: BatchManifest,
    manifest_json: String,
    manifest_blake3: String,
    schema: Schema,
    dims: u32,
    precision: ReferencePrecision,
    normalize: bool,
    documents: Vec<PathBuf>,
    passages: Vec<PathBuf>,
    deletions: Option<PathBuf>,
}

impl BatchReader {
    /// Opens a batch directory and checks controls 4 and 5 of `docs/04`.
    ///
    /// `index_config` and `model_config` are the configuration of the target
    /// index and the configuration resolved for its model; `check_manifest`
    /// compares the manifest with them.
    ///
    /// # Errors
    ///
    /// - [`ErrorCode::InvalidRequest`] for anything the batch itself gets
    ///   wrong: manifest, checksum, missing file, Parquet schema;
    /// - [`ErrorCode::IncompatibleModel`] when the batch model differs from the
    ///   index model;
    /// - [`ErrorCode::Internal`] for an I/O failure that is not a missing file.
    pub fn open(
        batch_dir: &Path,
        index_config: &IndexConfig,
        model_config: &ModelConfig,
    ) -> Result<Self, CribleError> {
        let (manifest, manifest_json) = read_manifest(batch_dir)?;
        crible_core::batch::check_manifest(&manifest, index_config, model_config)?;
        check_checksums(batch_dir, &manifest)?;

        let files = classify_files(&manifest)?;
        let reader = Self {
            manifest_blake3: format!("blake3:{}", blake3::hash(manifest_json.as_bytes()).to_hex()),
            manifest_json,
            schema: index_config.schema.clone(),
            dims: model_config.dims,
            precision: index_config.reference_precision,
            normalize: model_config.normalize,
            documents: paths_of(batch_dir, files.documents()),
            passages: paths_of(batch_dir, files.passages()),
            deletions: files.deletions().map(|name| batch_dir.join(name)),
            manifest,
        };

        reader.check_schemas()?;
        Ok(reader)
    }

    /// Manifest of the batch, as validated by [`BatchReader::open`].
    #[must_use]
    pub fn manifest(&self) -> &BatchManifest {
        &self.manifest
    }

    /// Raw bytes of `manifest.json`, decoded as UTF-8, exactly as they were
    /// read.
    ///
    /// The attach stores them in `batches.manifest`; its digest identifies the
    /// batch for INV-10 (ADR-0041).
    #[must_use]
    pub fn manifest_json(&self) -> &str {
        &self.manifest_json
    }

    /// `blake3:<hex>` digest of the bytes of `manifest.json` as read, never
    /// recomputed from the parsed structure (ADR-0041).
    #[must_use]
    pub fn manifest_blake3(&self) -> &str {
        &self.manifest_blake3
    }

    /// Rows of the document files, part by part.
    #[must_use]
    pub fn documents(&self) -> RowChunks {
        RowChunks::new(self.documents.clone(), DEFAULT_BATCH_ROWS)
    }

    /// Rows of the passage files, part by part.
    #[must_use]
    pub fn passages(&self) -> RowChunks {
        RowChunks::new(self.passages.clone(), DEFAULT_BATCH_ROWS)
    }

    /// Rows of the deletion file, part by part; empty when the batch has none.
    #[must_use]
    pub fn deletions(&self) -> RowChunks {
        let files = self.deletions.iter().cloned().collect();
        RowChunks::new(files, DEFAULT_BATCH_ROWS)
    }

    /// Reads every file once and applies controls 6 and 7 of `docs/04`.
    ///
    /// The files are streamed in bounded record batches, so validation never
    /// holds a whole role in memory. Documents are tracked one by one: their
    /// identifier and the next `ord` expected for them, never their passages.
    /// Deletions keep the set of their ids, bounded by `counts.deletions`, to
    /// refuse a duplicate (ADR-0040): a deletion beyond the declared count is
    /// refused before its id is stored.
    ///
    /// Controls 6 and 7 share one pass over the passages, so their order in
    /// `docs/04` is not strictly the order in which an error surfaces: a bad
    /// vector can be reported before a `counts.passages` mismatch that a later
    /// row would have revealed, and the `deletions` count is compared after the
    /// passages are read. Documents are always read first, so a documents count
    /// mismatch is reported before any passage is examined. The compromise
    /// keeps the batch to a single read of its passage files, which dominate
    /// the cost.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidRequest`] when a count differs from the manifest,
    /// when the rows of a document do not arrive in ascending `ord` order from
    /// 0, when a document has no passage, when a passage names an undeclared
    /// document, when a `doc_id` appears twice in `documents` or in
    /// `deletions` (ADR-0040), or when a vector is not finite or not normalized
    /// as declared.
    pub fn validate(&self) -> Result<BatchCounts, CribleError> {
        let (mut documents, document_rows) = self.read_documents()?;
        let passages = self.read_passages(&mut documents)?;
        check_every_document_has_a_passage(&documents)?;
        let deletions = self.read_deletions(&documents)?;
        Ok(BatchCounts {
            documents: document_rows,
            passages,
            deletions,
        })
    }

    /// Checks the Parquet schema of every file against the index schema
    /// (control 5).
    fn check_schemas(&self) -> Result<(), CribleError> {
        let documents = self.expected(Role::Documents)?;
        for path in &self.documents {
            check_schema(path, &documents)?;
        }
        let passages = self.expected(Role::Passages)?;
        for path in &self.passages {
            check_schema(path, &passages)?;
        }
        if let Some(path) = &self.deletions {
            check_schema(path, &self.expected(Role::Deletions)?)?;
        }
        Ok(())
    }

    /// Columns expected in a file of `role`, with their type and nullability.
    fn expected(&self, role: Role) -> Result<Vec<(String, DataType, bool)>, CribleError> {
        let mut columns = Vec::new();
        match role {
            Role::Documents => {
                columns.push(("doc_id".to_owned(), DataType::Utf8, false));
                columns.push(("doc_hash".to_owned(), DataType::Utf8, false));
                columns.extend(
                    self.schema
                        .fields()
                        .filter(|(_, level, _)| *level == FieldLevel::Document)
                        .map(|(name, _, spec)| {
                            (name.to_owned(), parquet_type(spec.field_type), true)
                        }),
                );
            }
            Role::Passages => {
                columns.push(("doc_id".to_owned(), DataType::Utf8, false));
                columns.push(("ord".to_owned(), DataType::UInt32, false));
                columns.push(("text".to_owned(), DataType::Utf8, false));
                columns.extend(
                    self.schema
                        .fields()
                        .filter(|(_, level, _)| *level == FieldLevel::Passage)
                        .map(|(name, _, spec)| {
                            (name.to_owned(), parquet_type(spec.field_type), true)
                        }),
                );
                columns.push((
                    "vector".to_owned(),
                    DataType::FixedSizeBinary(self.vector_width_i32()?),
                    false,
                ));
            }
            Role::Deletions => {
                columns.push(("doc_id".to_owned(), DataType::Utf8, false));
            }
        }
        Ok(columns)
    }

    /// Width of one stored vector as Arrow wants it, an `i32`.
    ///
    /// A `dims × component size` above `i32::MAX` cannot be expressed by
    /// `FIXED_SIZE_BINARY`, so it is a configuration the engine cannot serve
    /// rather than a fault of the batch.
    fn vector_width_i32(&self) -> Result<i32, CribleError> {
        let width = vector_width(self.dims, self.precision);
        i32::try_from(width).map_err(|_| {
            CribleError::new(
                ErrorCode::Internal,
                format!(
                    "{} dimensions in {} need a vector of {width} bytes, which exceeds the \
                     i32 width of a FIXED_SIZE_BINARY column",
                    self.dims,
                    precision_name(self.precision)
                ),
            )
        })
    }

    /// Reads the document files and returns, per declared document, the next
    /// `ord` expected for it (`None` until its first passage is seen), with the
    /// number of rows read.
    ///
    /// A `doc_id` appears at most once in the documents of a batch, across all
    /// its parts (ADR-0040): a duplicate is refused instead of being merged, so
    /// two `doc_hash` values cannot coexist for one document. The map holds one
    /// entry per declared document, so the memory stays bounded by
    /// `counts.documents`.
    fn read_documents(&self) -> Result<(HashMap<String, Option<u32>>, u64), CribleError> {
        let declared = self.manifest.counts.documents;
        let limit = usize::try_from(declared).unwrap_or(usize::MAX);
        let mut documents: HashMap<String, Option<u32>> = HashMap::new();
        let mut rows = 0_u64;
        for chunk in self.documents() {
            let chunk = chunk?;
            let ids = string_column(&chunk, "doc_id")?;
            for index in 0..chunk.num_rows() {
                let doc_id = string_value(ids, index, "doc_id", Role::Documents, &self.manifest)?;
                if documents.contains_key(doc_id) {
                    return Err(invalid(format!(
                        "the documents files declare the doc_id '{doc_id}' twice; a doc_id \
                         appears at most once in the documents of a batch, across all its parts"
                    )));
                }
                if documents.len() >= limit {
                    return Err(invalid(format!(
                        "the documents files declare more documents than counts.documents (\
                         {declared}); the batch must match its manifest"
                    )));
                }
                documents.insert(doc_id.to_owned(), None);
                rows = rows.saturating_add(1);
            }
        }
        check_count("documents", rows, declared)?;
        Ok((documents, rows))
    }

    /// Reads the passage files, checking control 6 (`ord`s and counts) and
    /// control 7 (finite and normalized vectors) along the way.
    fn read_passages(
        &self,
        documents: &mut HashMap<String, Option<u32>>,
    ) -> Result<u64, CribleError> {
        let declared = self.manifest.counts.passages;
        let mut rows = 0_u64;
        for chunk in self.passages() {
            let chunk = chunk?;
            let ids = string_column(&chunk, "doc_id")?;
            let ords = u32_column(&chunk, "ord")?;
            let vectors = fixed_binary_column(&chunk, "vector")?;
            for index in 0..chunk.num_rows() {
                let doc_id = string_value(ids, index, "doc_id", Role::Passages, &self.manifest)?;
                let ord = ords.value(index);
                let Some(next_ord) = documents.get_mut(doc_id) else {
                    return Err(invalid(format!(
                        "passage {doc_id}#{ord} belongs to a document that the documents files do \
                         not declare; every passage belongs to a document of the batch"
                    )));
                };
                match next_ord {
                    None => {
                        if ord != 0 {
                            return Err(invalid(format!(
                                "the first row of document '{doc_id}' has ord {ord}; the rows \
                                 of a document must arrive in ascending ord order, starting at 0"
                            )));
                        }
                        *next_ord = Some(1);
                    }
                    Some(expected) => {
                        if ord != *expected {
                            return Err(invalid(format!(
                                "passage {doc_id}#{ord} breaks the ord sequence of its document, \
                                 which expects {expected}; the rows of a document must arrive in \
                                 ascending ord order, from 0 without a gap"
                            )));
                        }
                        *next_ord = Some(expected.saturating_add(1));
                    }
                }
                self.check_vector(vectors.value(index), doc_id, ord)?;
                rows = rows.saturating_add(1);
            }
        }
        check_count("passages", rows, declared)?;
        Ok(rows)
    }

    /// Reads the deletion file, when the batch has one.
    ///
    /// A `doc_id` appears at most once in the deletions of a batch (ADR-0040),
    /// so the ids are kept to refuse a duplicate. The set is bounded by
    /// `counts.deletions`: as in `read_documents`, a row beyond the declared
    /// count is refused before its id is stored, so a batch that under-declares
    /// its deletions cannot make the memory grow with the file.
    ///
    /// A `doc_id` that the documents files also declare is refused: the batch
    /// would both add and remove one document, and the result would depend on
    /// the role applied last. The attach semantics of `docs/04` require the
    /// whole batch to be rejected, before any write.
    fn read_deletions(&self, documents: &HashMap<String, Option<u32>>) -> Result<u64, CribleError> {
        let declared = self.manifest.counts.deletions;
        let limit = usize::try_from(declared).unwrap_or(usize::MAX);
        let mut seen: HashSet<String> = HashSet::new();
        let mut rows = 0_u64;
        for chunk in self.deletions() {
            let chunk = chunk?;
            let ids = string_column(&chunk, "doc_id")?;
            for index in 0..chunk.num_rows() {
                let doc_id = string_value(ids, index, "doc_id", Role::Deletions, &self.manifest)?;
                if documents.contains_key(doc_id) {
                    return Err(invalid(format!(
                        "the doc_id '{doc_id}' is declared both in the documents and in the \
                         deletions of batch '{}'; a batch cannot add and remove one document",
                        self.manifest.batch_id
                    )));
                }
                if seen.contains(doc_id) {
                    return Err(invalid(format!(
                        "the deletions file declares the doc_id '{doc_id}' twice; a doc_id \
                         appears at most once in the deletions of a batch"
                    )));
                }
                if seen.len() >= limit {
                    return Err(invalid(format!(
                        "the deletions file declares more deletions than counts.deletions (\
                         {declared}); the batch must match its manifest"
                    )));
                }
                seen.insert(doc_id.to_owned());
                rows = rows.saturating_add(1);
            }
        }
        check_count("deletions", rows, declared)?;
        Ok(rows)
    }

    /// Checks one vector: exact byte length, finite components, and a norm
    /// close to 1 when the batch declares normalized vectors.
    fn check_vector(&self, bytes: &[u8], doc_id: &str, ord: u32) -> Result<(), CribleError> {
        let expected = vector_width(self.dims, self.precision);
        if bytes.len() != expected {
            return Err(invalid(format!(
                "the vector of passage {doc_id}#{ord} is {} bytes, but {} dimensions in {} need \
                 {expected}",
                bytes.len(),
                self.dims,
                precision_name(self.precision)
            )));
        }
        let width = component_width(self.precision);
        let mut sum = 0.0_f64;
        for (component, chunk) in bytes.chunks_exact(width).enumerate() {
            let value = decode_component(chunk, self.precision)?;
            if !value.is_finite() {
                return Err(invalid(format!(
                    "component {component} of the vector of passage {doc_id}#{ord} is not finite \
                     ({value})"
                )));
            }
            sum += f64::from(value) * f64::from(value);
        }
        if self.normalize {
            let norm = sum.sqrt();
            if (norm - 1.0).abs() > NORM_TOLERANCE {
                return Err(invalid(format!(
                    "the vector of passage {doc_id}#{ord} has L2 norm {norm:.6}, but the batch \
                     declares normalized vectors (|norm - 1| must be at most {NORM_TOLERANCE})"
                )));
            }
        }
        Ok(())
    }
}

/// Rows of one role of a batch, handed over as Arrow record batches of bounded
/// size.
///
/// The iterator opens the next Parquet file only once the previous one is
/// exhausted, so a reader that stops early never touches the rest of the batch.
/// An item is an error at most once per failing file; callers that keep reading
/// after an error may see it again.
#[derive(Debug)]
pub struct RowChunks {
    files: std::vec::IntoIter<PathBuf>,
    batch_rows: usize,
    current: Option<ParquetRecordBatchReader>,
    current_path: PathBuf,
}

impl RowChunks {
    fn new(files: Vec<PathBuf>, batch_rows: usize) -> Self {
        Self {
            files: files.into_iter(),
            batch_rows,
            current: None,
            current_path: PathBuf::new(),
        }
    }
}

impl Iterator for RowChunks {
    type Item = Result<RecordBatch, CribleError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(reader) = self.current.as_mut() {
                match reader.next() {
                    Some(Ok(batch)) => return Some(Ok(batch)),
                    Some(Err(error)) => {
                        return Some(Err(arrow_failure(&self.current_path, &error)));
                    }
                    None => self.current = None,
                }
            }
            let path = self.files.next()?;
            match open_batches(&path, self.batch_rows) {
                Ok(reader) => {
                    self.current = Some(reader);
                    self.current_path = path;
                }
                Err(error) => return Some(Err(error)),
            }
        }
    }
}

/// Reads `manifest.json` of a batch directory, and returns the parsed manifest
/// with its raw text.
///
/// The attach digests the raw text, before any deserialization, so the digest
/// identifies the bytes the producer wrote (ADR-0041).
fn read_manifest(batch_dir: &Path) -> Result<(BatchManifest, String), CribleError> {
    let path = batch_dir.join("manifest.json");
    let raw = std::fs::read_to_string(&path).map_err(|error| file_error("read", &path, &error))?;
    let manifest = BatchManifest::from_json_str(&raw)?;
    Ok((manifest, raw))
}

/// Resolves the file names of the manifest inside the batch directory.
fn paths_of(batch_dir: &Path, names: &[String]) -> Vec<PathBuf> {
    names.iter().map(|name| batch_dir.join(name)).collect()
}

/// Checks the digest form and the value of every checksum (control 4).
fn check_checksums(batch_dir: &Path, manifest: &BatchManifest) -> Result<(), CribleError> {
    for (name, declared) in &manifest.files {
        let declared = parse_checksum(declared).map_err(|error| {
            invalid(format!(
                "checksum of '{name}' at $.files is invalid: {}",
                error.message()
            ))
        })?;
        let path = batch_dir.join(name);
        let computed = hash_file(&path)?;
        if computed != declared {
            return Err(invalid(format!(
                "{} does not match its checksum: the manifest declares {declared}, the file \
                 hashes to {computed}",
                path.display()
            )));
        }
    }
    Ok(())
}

/// Streams a file through blake3, so a large file is never held in memory.
fn hash_file(path: &Path) -> Result<blake3::Hash, CribleError> {
    let mut file = File::open(path).map_err(|error| file_error("open", path, &error))?;
    let mut hasher = blake3::Hasher::new();
    std::io::copy(&mut file, &mut hasher).map_err(|error| file_error("read", path, &error))?;
    Ok(hasher.finalize())
}

/// Checks the columns of one Parquet file against the expected ones.
fn check_schema(path: &Path, expected: &[(String, DataType, bool)]) -> Result<(), CribleError> {
    let actual = file_schema(path)?;
    for field in actual.fields() {
        if !expected.iter().any(|(name, _, _)| name == field.name()) {
            return Err(invalid(format!(
                "{} has a column '{}' that the index schema does not declare",
                path.display(),
                field.name()
            )));
        }
    }
    for (name, data_type, nullable) in expected {
        let Some(field) = actual.fields().iter().find(|field| field.name() == name) else {
            return Err(invalid(format!(
                "{} has no column '{name}'; the batch must match the schema of the index",
                path.display()
            )));
        };
        if field.data_type() != data_type {
            return Err(invalid(format!(
                "column '{name}' of {} has type {}, expected {}",
                path.display(),
                field.data_type(),
                data_type
            )));
        }
        if field.is_nullable() != *nullable {
            let expected_form = if *nullable { "nullable" } else { "non-null" };
            return Err(invalid(format!(
                "column '{name}' of {} must be {expected_form}",
                path.display()
            )));
        }
    }
    Ok(())
}

/// Arrow schema of one Parquet file, read from its footer only.
fn file_schema(path: &Path) -> Result<SchemaRef, CribleError> {
    let file = File::open(path).map_err(|error| file_error("open", path, &error))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|error| parquet_failure(path, &error))?;
    Ok(builder.schema().clone())
}

/// Opens one Parquet file for row-by-row reading, at most `batch_rows` rows at
/// a time.
fn open_batches(path: &Path, batch_rows: usize) -> Result<ParquetRecordBatchReader, CribleError> {
    let file = File::open(path).map_err(|error| file_error("open", path, &error))?;
    ParquetRecordBatchReaderBuilder::try_new(file)
        .and_then(|builder| builder.with_batch_size(batch_rows).build())
        .map_err(|error| parquet_failure(path, &error))
}

/// Translates a Parquet failure.
///
/// The checksum of the file has already been verified when this runs, so a
/// file the reader cannot decode is a fault of the batch
/// ([`ErrorCode::InvalidRequest`]), except when the cause is a real I/O
/// failure: a permission error or an EIO describes the disk, not the batch, and
/// is an [`ErrorCode::Internal`] error. A truncated file is a batch fault: its
/// bytes are what the producer wrote.
fn parquet_failure(path: &Path, error: &parquet::errors::ParquetError) -> CribleError {
    match io_cause(error) {
        Some(io) if io.kind() != std::io::ErrorKind::UnexpectedEof => file_error("read", path, io),
        _ => invalid(format!(
            "{} is not a readable Parquet file: {error}",
            path.display()
        )),
    }
}

/// Translates a failure of the Arrow record batch reader.
///
/// arrow-rs keeps the cause when it reports an I/O failure as
/// [`arrow::error::ArrowError::IoError`], and the failure is then an
/// [`ErrorCode::Internal`] error. `ParquetRecordBatchReader` does not take that
/// path: its `impl From<ParquetError> for ArrowError` wraps the error in
/// `ArrowError::ParquetError(String)` and drops the source, so an I/O failure
/// raised while decoding a row group loses its cause and is reported as a fault
/// of the batch. The unit test of this module checks the conversion rule, not a
/// path this reader can reach.
fn arrow_failure(path: &Path, error: &arrow::error::ArrowError) -> CribleError {
    match error {
        arrow::error::ArrowError::IoError(_, io)
            if io.kind() != std::io::ErrorKind::UnexpectedEof =>
        {
            file_error("read", path, io)
        }
        _ => invalid(format!("cannot read {}: {error}", path.display())),
    }
}

/// First real I/O failure in the cause chain of an error, if any.
fn io_cause<'a>(error: &'a (dyn std::error::Error + 'static)) -> Option<&'a std::io::Error> {
    let mut current = Some(error);
    while let Some(cause) = current {
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            return Some(io);
        }
        current = cause.source();
    }
    None
}

/// Arrow type of a declared field (`docs/format-lot-v1.md`, "Colonnes
/// Parquet").
pub(crate) fn parquet_type(field_type: FieldType) -> DataType {
    match field_type {
        FieldType::Keyword => DataType::Utf8,
        FieldType::Integer => DataType::Int64,
        FieldType::Float => DataType::Float64,
        FieldType::Date => DataType::Date32,
        FieldType::Bool => DataType::Boolean,
    }
}

/// Number of bytes of one component of a stored vector.
fn component_width(precision: ReferencePrecision) -> usize {
    match precision {
        ReferencePrecision::Fp16 => 2,
        ReferencePrecision::Fp32 => 4,
    }
}

/// Number of bytes of one stored vector.
pub(crate) fn vector_width(dims: u32, precision: ReferencePrecision) -> usize {
    (dims as usize).saturating_mul(component_width(precision))
}

/// Wire name of a vector precision, for error messages.
///
/// The single definition of the wire name is [`VectorDtype::as_str`], which the
/// batch manifest already writes: the store does not repeat the mapping.
fn precision_name(precision: ReferencePrecision) -> &'static str {
    VectorDtype::from_reference_precision(precision).as_str()
}

/// Decodes one little-endian component; nothing is read through a
/// platform-dependent transmutation.
fn decode_component(bytes: &[u8], precision: ReferencePrecision) -> Result<f32, CribleError> {
    match precision {
        ReferencePrecision::Fp16 => {
            let raw: [u8; 2] = bytes.try_into().map_err(|_| {
                CribleError::new(
                    ErrorCode::Internal,
                    format!("an fp16 component is {} bytes, not 2", bytes.len()),
                )
            })?;
            Ok(f32::from(f16::from_bits(u16::from_le_bytes(raw))))
        }
        ReferencePrecision::Fp32 => {
            let raw: [u8; 4] = bytes.try_into().map_err(|_| {
                CribleError::new(
                    ErrorCode::Internal,
                    format!("an fp32 component is {} bytes, not 4", bytes.len()),
                )
            })?;
            Ok(f32::from_le_bytes(raw))
        }
    }
}

/// Borrows a non-null UTF-8 column of a record batch.
fn string_column<'a>(chunk: &'a RecordBatch, name: &str) -> Result<&'a StringArray, CribleError> {
    let column = chunk.column_by_name(name).ok_or_else(|| {
        CribleError::new(
            ErrorCode::Internal,
            format!("a checked Parquet batch has no column '{name}'"),
        )
    })?;
    column
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| {
            CribleError::new(
                ErrorCode::Internal,
                format!("column '{name}' of a checked Parquet batch is not UTF-8"),
            )
        })
}

/// Borrows a non-null `UInt32` column of a record batch.
fn u32_column<'a>(chunk: &'a RecordBatch, name: &str) -> Result<&'a UInt32Array, CribleError> {
    let column = chunk.column_by_name(name).ok_or_else(|| {
        CribleError::new(
            ErrorCode::Internal,
            format!("a checked Parquet batch has no column '{name}'"),
        )
    })?;
    column
        .as_any()
        .downcast_ref::<UInt32Array>()
        .ok_or_else(|| {
            CribleError::new(
                ErrorCode::Internal,
                format!("column '{name}' of a checked Parquet batch is not a uint32"),
            )
        })
}

/// Borrows a non-null fixed-size binary column of a record batch.
fn fixed_binary_column<'a>(
    chunk: &'a RecordBatch,
    name: &str,
) -> Result<&'a FixedSizeBinaryArray, CribleError> {
    let column = chunk.column_by_name(name).ok_or_else(|| {
        CribleError::new(
            ErrorCode::Internal,
            format!("a checked Parquet batch has no column '{name}'"),
        )
    })?;
    column
        .as_any()
        .downcast_ref::<FixedSizeBinaryArray>()
        .ok_or_else(|| {
            CribleError::new(
                ErrorCode::Internal,
                format!("column '{name}' of a checked Parquet batch is not fixed-size binary"),
            )
        })
}

/// Reads one required non-null string value, naming its file and column on
/// failure.
fn string_value<'a>(
    column: &'a StringArray,
    index: usize,
    column_name: &str,
    role: Role,
    manifest: &BatchManifest,
) -> Result<&'a str, CribleError> {
    if column.is_null(index) {
        return Err(invalid(format!(
            "column '{column_name}' of the {} files of batch '{}' holds null; it is non-null",
            role.as_str(),
            manifest.batch_id
        )));
    }
    let value = column.value(index);
    if value.is_empty() {
        return Err(invalid(format!(
            "column '{column_name}' of the {} files of batch '{}' holds an empty value",
            role.as_str(),
            manifest.batch_id
        )));
    }
    Ok(value)
}

/// Checks that every document of the batch has at least one passage
/// (control 6).
fn check_every_document_has_a_passage(
    documents: &HashMap<String, Option<u32>>,
) -> Result<(), CribleError> {
    if let Some((doc_id, _)) = documents.iter().find(|(_, next_ord)| next_ord.is_none()) {
        return Err(invalid(format!(
            "document '{doc_id}' of the batch has no passage; every document arrives with all its \
             passages and has at least one"
        )));
    }
    Ok(())
}

/// Compares the rows actually read with the count of the manifest (control 6).
fn check_count(role: &str, read: u64, declared: u64) -> Result<(), CribleError> {
    if read != declared {
        return Err(invalid(format!(
            "the manifest declares {declared} {role} but the batch holds {read} rows; counts must \
             match the files"
        )));
    }
    Ok(())
}

/// An I/O failure: a missing batch file is a fault of the batch, anything else
/// is an internal state error.
fn file_error(action: &str, path: &Path, error: &std::io::Error) -> CribleError {
    if error.kind() == std::io::ErrorKind::NotFound {
        return invalid(format!(
            "{} is missing; a batch directory holds every file listed in its manifest",
            path.display()
        ));
    }
    CribleError::new(
        ErrorCode::Internal,
        format!("cannot {action} {}: {error}", path.display()),
    )
}

/// Builds an [`ErrorCode::InvalidRequest`] error for a fault of the batch.
fn invalid(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::InvalidRequest, message)
}

#[cfg(test)]
mod tests {
    use super::{arrow_failure, parquet_failure};
    use crible_core::error::ErrorCode;
    use std::io;
    use std::path::Path;

    #[test]
    fn a_real_io_failure_while_reading_a_parquet_file_is_internal() {
        let path = Path::new("passages.parquet");

        let parquet = parquet_failure(
            path,
            &parquet::errors::ParquetError::from(io::Error::from_raw_os_error(5)),
        );
        assert_eq!(
            parquet.code(),
            ErrorCode::Internal,
            "an EIO is an unexpected state of the disk: {}",
            parquet.message()
        );

        let arrow = arrow_failure(
            path,
            &arrow::error::ArrowError::IoError(
                "read the row group".to_owned(),
                io::Error::from_raw_os_error(13),
            ),
        );
        assert_eq!(
            arrow.code(),
            ErrorCode::Internal,
            "a permission error is an unexpected state of the disk: {}",
            arrow.message()
        );
    }

    #[test]
    fn an_unreadable_or_truncated_parquet_file_is_a_batch_fault() {
        let path = Path::new("passages.parquet");

        let corrupt = parquet_failure(
            path,
            &parquet::errors::ParquetError::General("bad magic".to_owned()),
        );
        assert_eq!(corrupt.code(), ErrorCode::InvalidRequest);

        let truncated = parquet_failure(
            path,
            &parquet::errors::ParquetError::from(io::Error::from(io::ErrorKind::UnexpectedEof)),
        );
        assert_eq!(
            truncated.code(),
            ErrorCode::InvalidRequest,
            "a truncated file is what the producer wrote, so a batch fault: {}",
            truncated.message()
        );

        let arrow = arrow_failure(
            path,
            &arrow::error::ArrowError::ParseError("not a record batch".to_owned()),
        );
        assert_eq!(arrow.code(), ErrorCode::InvalidRequest);
    }
}
