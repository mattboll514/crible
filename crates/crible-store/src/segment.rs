// SPDX-License-Identifier: Apache-2.0

//! Immutable vector files of one segment.
//!
//! A segment directory (`segments/<segment_id>/`) holds the reference vectors
//! of a contiguous range of internal identifiers, plus `segment.json`
//! (`docs/02-architecture.md`, "Organisation sur disque"):
//!
//! ```json
//! {
//!   "batch_id": "batch-2026-10-21",
//!   "checksums": { "vectors.f16": "blake3:<hex>" },
//!   "count": 2,
//!   "dims": 2,
//!   "format_version": 1,
//!   "generation": 7,
//!   "iid_start": 4
//! }
//! ```
//!
//! The manifest lists exactly the fields of `docs/02`: `iid_start`, `count`,
//! `dims`, `checksums`, `generation` and `batch_id`, plus the format version
//! (INV-15). Two values are **derived from the disk layout** instead of being
//! repeated: the precision comes from the single `vectors.f16` or `vectors.f32`
//! file present, and the segment identifier from the directory name.
//!
//! The vector file holds `count × dims` components, row-major, the vector of
//! `iid_start + k` starting at component `k × dims`. Each component is
//! little-endian and explicit: 2 bytes for fp16, 4 bytes for fp32. Nothing is
//! read through a platform-dependent transmutation (INV-15).
//!
//! Writing is atomic (`docs/02-architecture.md`, "Générations et point de
//! commit"): the files are built in `segments/tmp-<segment_id>/`, fsynced, then
//! the directory is renamed to its final name. A segment therefore stays
//! invisible until [`SegmentWriter::finish`] returns, and a reader that opens a
//! segment only ever sees complete files.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crible_core::config::ReferencePrecision;
use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::{Generation, Iid};
use half::f16;
use memmap2::Mmap;
use serde_json::{Value, json};

/// Version of the segment on-disk format (INV-15).
pub const FORMAT_VERSION: u32 = 1;

/// Name of the manifest of a segment directory.
const MANIFEST_FILE_NAME: &str = "segment.json";

/// Prefix of the directory a segment is built in before its commit point.
const TEMPORARY_PREFIX: &str = "tmp-";

/// Name of the vector file of a segment of `precision`.
///
/// `segment.json` does not repeat the precision: exactly one of `vectors.f16`
/// and `vectors.f32` is present in the segment directory.
pub(crate) fn vector_file_name(precision: ReferencePrecision) -> &'static str {
    match precision {
        ReferencePrecision::Fp16 => "vectors.f16",
        ReferencePrecision::Fp32 => "vectors.f32",
    }
}

/// Number of bytes of one stored component.
fn bytes_per_component(precision: ReferencePrecision) -> usize {
    match precision {
        ReferencePrecision::Fp16 => 2,
        ReferencePrecision::Fp32 => 4,
    }
}

/// Everything needed to write the vectors of one segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentSpec {
    /// Identifier of the segment, also the name of its directory.
    pub segment_id: String,
    /// Internal identifier of the first vector of the segment.
    pub iid_start: Iid,
    /// Dimensionality of one vector.
    pub dims: u32,
    /// Generation that makes the segment visible.
    pub generation: Generation,
    /// Batch the segment was built from.
    pub batch_id: String,
    /// Precision of the stored vectors.
    pub precision: ReferencePrecision,
}

/// Metadata of a segment, as read back from `segment.json`.
///
/// `segment_id` and `precision` are in memory only, derived from the directory
/// name and from the vector file present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentMeta {
    /// Version of the on-disk format the segment was written with.
    pub format_version: u32,
    /// Identifier of the segment, taken from its directory name.
    pub segment_id: String,
    /// Internal identifier of the first vector of the segment.
    pub iid_start: Iid,
    /// Number of vectors of the segment.
    pub count: u32,
    /// Dimensionality of one vector.
    pub dims: u32,
    /// Generation that made the segment visible.
    pub generation: Generation,
    /// Batch the segment was built from.
    pub batch_id: String,
    /// Precision of the stored vectors, taken from the vector file present.
    pub precision: ReferencePrecision,
    /// Checksums of the files of the segment, `file name -> blake3:<hex>`.
    pub checksums: BTreeMap<String, String>,
}

impl SegmentMeta {
    /// Number of bytes of the vector file of this segment.
    fn vectors_len(&self) -> Result<u64, CribleError> {
        let components = u64::from(self.count)
            .checked_mul(u64::from(self.dims))
            .ok_or_else(|| {
                CribleError::new(
                    ErrorCode::Internal,
                    format!(
                        "segment {} declares count {} and dims {}, \
                         whose product overflows a file length",
                        self.segment_id, self.count, self.dims
                    ),
                )
            })?;
        components
            .checked_mul(bytes_per_component(self.precision) as u64)
            .ok_or_else(|| {
                CribleError::new(
                    ErrorCode::Internal,
                    format!(
                        "segment {} declares count {} and dims {}, \
                         whose byte length overflows",
                        self.segment_id, self.count, self.dims
                    ),
                )
            })
    }
}

/// Writes the vector file and the manifest of one segment.
///
/// Creation builds `segments/tmp-<segment_id>/`; [`SegmentWriter::finish`]
/// fsyncs it and renames it to `segments/<segment_id>/`.
#[derive(Debug)]
pub struct SegmentWriter {
    spec: SegmentSpec,
    temporary_dir: PathBuf,
    final_dir: PathBuf,
    vectors: BufWriter<File>,
    hasher: blake3::Hasher,
    count: u32,
}

impl SegmentWriter {
    /// Starts a segment in `segments/tmp-<segment_id>/`, where `segments_dir` is
    /// the `segments/` directory of an epoch.
    ///
    /// The segment identifier must be unique per attempt: a `tmp-<segment_id>`
    /// directory left by an interrupted attempt is not removed here, so creating
    /// the same segment again fails until the recovery of `GERER-GENERATIONS`
    /// deletes the `tmp-*` directories (`docs/02-architecture.md`, "Reprise au
    /// démarrage"). Removal is the job of that recovery, not of this writer.
    ///
    /// A zero dimensionality is refused with [`ErrorCode::InvalidRequest`]: a
    /// vector file of zero bytes per row cannot be read back by internal
    /// identifier.
    pub fn create(segments_dir: &Path, spec: SegmentSpec) -> Result<Self, CribleError> {
        validate_segment_id(&spec.segment_id)?;
        if spec.dims == 0 {
            return Err(CribleError::new(
                ErrorCode::InvalidRequest,
                format!(
                    "segment {} has zero dimensions; a segment needs at least one",
                    spec.segment_id
                ),
            ));
        }
        let final_dir = segments_dir.join(&spec.segment_id);
        let temporary_dir = segments_dir.join(format!("{TEMPORARY_PREFIX}{}", spec.segment_id));
        fs::create_dir(&temporary_dir).map_err(|error| {
            io_error(
                "create the temporary segment directory",
                &temporary_dir,
                &error,
            )
        })?;

        let vectors_path = temporary_dir.join(vector_file_name(spec.precision));
        let file = File::create(&vectors_path)
            .map_err(|error| io_error("create the vector file", &vectors_path, &error))?;
        Ok(Self {
            spec,
            temporary_dir,
            final_dir,
            vectors: BufWriter::new(file),
            hasher: blake3::Hasher::new(),
            count: 0,
        })
    }

    /// Appends one vector and returns the internal identifier it took.
    ///
    /// A vector whose length is not the dimensionality of the segment is
    /// refused with [`ErrorCode::InvalidRequest`], without consuming an
    /// identifier: at attach time the dimensions come from the batch, so from
    /// the client.
    pub fn append(&mut self, vector: &[f32]) -> Result<Iid, CribleError> {
        let dims = self.spec.dims as usize;
        if vector.len() != dims {
            return Err(CribleError::new(
                ErrorCode::InvalidRequest,
                format!(
                    "a vector of segment {} has {} dimensions, got {}",
                    self.spec.segment_id,
                    self.spec.dims,
                    vector.len()
                ),
            ));
        }
        let offset = u64::from(self.spec.iid_start.get()) + u64::from(self.count);
        let iid = u32::try_from(offset).map_err(|_| {
            CribleError::new(
                ErrorCode::Internal,
                format!(
                    "segment {} would exceed the internal identifier space: \
                     iid_start {} plus {} vectors",
                    self.spec.segment_id,
                    self.spec.iid_start.get(),
                    self.count
                ),
            )
        })?;
        let next_count = self.count.checked_add(1).ok_or_else(|| {
            CribleError::new(
                ErrorCode::Internal,
                format!(
                    "segment {} holds more than u32::MAX vectors",
                    self.spec.segment_id
                ),
            )
        })?;

        let mut encoded = Vec::with_capacity(dims * bytes_per_component(self.spec.precision));
        for value in vector {
            encode_component(&mut encoded, self.spec.precision, *value);
        }
        self.vectors.write_all(&encoded).map_err(|error| {
            io_error(
                "append to the vector file",
                &self
                    .temporary_dir
                    .join(vector_file_name(self.spec.precision)),
                &error,
            )
        })?;
        // The checksum is computed on the bytes actually written, so the hash of
        // a whole fp32 segment never has to be held in memory.
        self.hasher.update(&encoded);
        self.count = next_count;
        Ok(Iid::new(iid))
    }

    /// Completes the segment: fsyncs the vector file, writes `segment.json`,
    /// fsyncs the directory and renames it to `segments/<segment_id>/`.
    pub fn finish(self) -> Result<SegmentMeta, CribleError> {
        let Self {
            spec,
            temporary_dir,
            final_dir,
            vectors,
            hasher,
            count,
        } = self;

        let file = vectors
            .into_inner()
            .map_err(|error| error.into_error())
            .map_err(|error| {
                io_error(
                    "flush the vector file",
                    &temporary_dir.join(vector_file_name(spec.precision)),
                    &error,
                )
            })?;
        file.sync_all().map_err(|error| {
            io_error(
                "fsync the vector file",
                &temporary_dir.join(vector_file_name(spec.precision)),
                &error,
            )
        })?;
        drop(file);

        let checksums = BTreeMap::from([(
            vector_file_name(spec.precision).to_owned(),
            format!("blake3:{}", hasher.finalize().to_hex()),
        )]);
        let meta = SegmentMeta {
            format_version: FORMAT_VERSION,
            segment_id: spec.segment_id,
            iid_start: spec.iid_start,
            count,
            dims: spec.dims,
            generation: spec.generation,
            batch_id: spec.batch_id,
            precision: spec.precision,
            checksums,
        };

        let manifest_path = temporary_dir.join(MANIFEST_FILE_NAME);
        write_file_synced(&manifest_path, manifest_json(&meta)?.as_bytes())?;
        sync_directory(&temporary_dir)?;

        fs::rename(&temporary_dir, &final_dir).map_err(|error| {
            io_error(
                "rename the temporary segment directory",
                &temporary_dir,
                &error,
            )
        })?;
        let segments_dir = final_dir.parent().ok_or_else(|| {
            CribleError::new(
                ErrorCode::Internal,
                format!(
                    "segment directory {} has no parent, so no fsync",
                    final_dir.display()
                ),
            )
        })?;
        sync_directory(segments_dir)?;
        Ok(meta)
    }
}

/// Read-only view of the vectors of one segment, through a memory map.
#[derive(Debug)]
pub struct SegmentReader {
    meta: SegmentMeta,
    data: Option<Mmap>,
}

impl SegmentReader {
    /// Opens a segment directory: checks the format version (INV-15), derives
    /// the precision and the identifier from the disk layout, checks the
    /// declared checksum and the file length, then maps the vector file.
    ///
    /// An unknown or missing `format_version` is an [`ErrorCode::InvalidRequest`]
    /// error, because it describes the caller's view of the format. Everything
    /// else (missing vector file, corrupted or truncated one, zero dimensions)
    /// describes the internal state of an epoch and is an [`ErrorCode::Internal`]
    /// error.
    ///
    /// **Cost of the checksum.** `open` reads the whole vector file once to
    /// verify its blake3 checksum, as the segment task requires ("a corrupted
    /// checksum is detected at open time"). At the scale of Legifacile that is
    /// about 13 GB per open. The policy is unchanged here: the generation
    /// snapshot reads the metadata through `read_meta` and opens no segment,
    /// so it never pays that cost. Deferring the verification itself (once per
    /// generation, or only on recovery) stays an open question for a later
    /// decision.
    pub fn open(segment_dir: &Path) -> Result<Self, CribleError> {
        let meta = read_meta(segment_dir)?;
        let precision = meta.precision;
        let vectors_path = segment_dir.join(vector_file_name(precision));
        if meta.dims == 0 {
            return Err(CribleError::new(
                ErrorCode::Internal,
                format!(
                    "segment {} declares zero dimensions, so its vector file holds \
                     no readable row",
                    meta.segment_id
                ),
            ));
        }
        let expected_len = meta.vectors_len()?;
        let actual_len = fs::metadata(&vectors_path)
            .map_err(|error| io_error("inspect the vector file", &vectors_path, &error))?
            .len();
        if actual_len != expected_len {
            return Err(CribleError::new(
                ErrorCode::Internal,
                format!(
                    "{} is {actual_len} bytes long, but segment {} declares \
                     {expected_len} bytes ({} vectors of {} dimensions)",
                    vectors_path.display(),
                    meta.segment_id,
                    meta.count,
                    meta.dims
                ),
            ));
        }
        check_checksum(&vectors_path, &meta)?;

        let file = File::open(&vectors_path)
            .map_err(|error| io_error("open the vector file", &vectors_path, &error))?;
        // SAFETY: the segment directory is built in `tmp-<id>`, fsynced and only
        // then renamed, and Crible never rewrites or truncates a `vectors.*`
        // file in place: a change creates a new segment. A reader that opened
        // `segments/<segment_id>/` therefore maps bytes no process writes any
        // more. The mapping is read-only, and dropping the file descriptor
        // afterwards does not invalidate it.
        let data = if expected_len == 0 {
            None
        } else {
            Some(
                unsafe { Mmap::map(&file) }
                    .map_err(|error| io_error("map the vector file", &vectors_path, &error))?,
            )
        };
        Ok(Self { meta, data })
    }

    /// Metadata of the segment.
    #[must_use]
    pub fn meta(&self) -> &SegmentMeta {
        &self.meta
    }

    /// True when `iid` belongs to the segment, which covers the half-open range
    /// `[iid_start, iid_start + count)`.
    #[must_use]
    pub fn contains(&self, iid: Iid) -> bool {
        self.index_of(iid).is_some()
    }

    /// Returns the reference vector of `iid`.
    ///
    /// An identifier outside the range of the segment is an
    /// [`ErrorCode::Internal`] error: an internal identifier always comes from
    /// the engine, so a foreign one is a bug, not a bad request (INV-01).
    pub fn vector(&self, iid: Iid) -> Result<Vec<f32>, CribleError> {
        let Some(index) = self.index_of(iid) else {
            let end = u64::from(self.meta.iid_start.get()) + u64::from(self.meta.count);
            return Err(CribleError::new(
                ErrorCode::Internal,
                format!(
                    "iid {} is outside the range [{}, {end}) of segment {}",
                    iid.get(),
                    self.meta.iid_start.get(),
                    self.meta.segment_id
                ),
            ));
        };
        let dims = self.meta.dims as usize;
        let width = bytes_per_component(self.meta.precision);
        let start = index
            .checked_mul(dims)
            .and_then(|offset| offset.checked_mul(width))
            .ok_or_else(|| {
                CribleError::new(
                    ErrorCode::Internal,
                    format!(
                        "offset of iid {} overflows in segment {}",
                        iid.get(),
                        self.meta.segment_id
                    ),
                )
            })?;
        let end = start + dims * width;
        let Some(data) = self.data.as_ref() else {
            return Err(CribleError::new(
                ErrorCode::Internal,
                format!(
                    "segment {} holds no vector file bytes, so no vector for iid {}",
                    self.meta.segment_id,
                    iid.get()
                ),
            ));
        };
        let Some(bytes) = data.get(start..end) else {
            return Err(CribleError::new(
                ErrorCode::Internal,
                format!(
                    "iid {} falls outside the vector file of segment {}",
                    iid.get(),
                    self.meta.segment_id
                ),
            ));
        };
        decode_vector(bytes, self.meta.precision)
    }

    /// Position of `iid` in the vector file, or `None` when it is out of range.
    fn index_of(&self, iid: Iid) -> Option<usize> {
        let offset = u64::from(iid.get()).checked_sub(u64::from(self.meta.iid_start.get()))?;
        if offset >= u64::from(self.meta.count) {
            return None;
        }
        usize::try_from(offset).ok()
    }
}

/// Rejects a segment identifier that would not name exactly one directory of
/// `segments/`, or that the recovery of `tmp-*` directories would delete.
fn validate_segment_id(segment_id: &str) -> Result<(), CribleError> {
    if segment_id.is_empty() {
        return Err(CribleError::new(
            ErrorCode::InvalidRequest,
            "segment_id must not be empty",
        ));
    }
    if segment_id.contains(['/', '\\'])
        || segment_id == "."
        || segment_id == ".."
        || segment_id.starts_with(TEMPORARY_PREFIX)
    {
        return Err(CribleError::new(
            ErrorCode::InvalidRequest,
            format!("segment_id {segment_id:?} is not usable as a directory name"),
        ));
    }
    Ok(())
}

/// Metadata of a segment directory, read from `segment.json` and from the disk
/// layout, **without** reading the vector file.
///
/// The format version is checked (INV-15); `segment_id` and `precision` are
/// derived from the directory name and from the vector file present. The
/// checksum of `vectors.*` is deliberately **not** verified here:
/// [`SegmentReader::open`] does that, and the snapshot of a generation only
/// needs the metadata. Verifying the checksum costs a full read of the vector
/// file (about 13 GB at the scale of Legifacile), which the per-generation
/// snapshot must not pay.
pub(crate) fn read_meta(segment_dir: &Path) -> Result<SegmentMeta, CribleError> {
    let manifest_path = segment_dir.join(MANIFEST_FILE_NAME);
    let raw = fs::read_to_string(&manifest_path)
        .map_err(|error| io_error("read the segment manifest", &manifest_path, &error))?;
    let manifest: Value = serde_json::from_str(&raw).map_err(|error| {
        CribleError::new(
            ErrorCode::Internal,
            format!("{} is not valid JSON: {error}", manifest_path.display()),
        )
    })?;

    Ok(SegmentMeta {
        format_version: read_format_version(&manifest)?,
        segment_id: segment_id_of(segment_dir)?,
        iid_start: Iid::new(manifest_u32(&manifest, "iid_start")?),
        count: manifest_u32(&manifest, "count")?,
        dims: manifest_u32(&manifest, "dims")?,
        generation: Generation::new(manifest_u64(&manifest, "generation")?),
        batch_id: manifest_str(&manifest, "batch_id")?.to_owned(),
        precision: read_precision(segment_dir)?,
        checksums: read_checksums(&manifest)?,
    })
}

/// Reads `format_version` and refuses anything but [`FORMAT_VERSION`] (INV-15).
fn read_format_version(manifest: &Value) -> Result<u32, CribleError> {
    let Some(value) = manifest.get("format_version") else {
        return Err(CribleError::new(
            ErrorCode::InvalidRequest,
            format!(
                "segment.json has no format_version; this build reads \
                 format_version {FORMAT_VERSION}"
            ),
        ));
    };
    let Some(version) = value.as_u64() else {
        return Err(CribleError::new(
            ErrorCode::InvalidRequest,
            format!(
                "segment.json format_version {value} is not an integer; \
                 this build reads format_version {FORMAT_VERSION}"
            ),
        ));
    };
    let version = u32::try_from(version).map_err(|_| {
        CribleError::new(
            ErrorCode::InvalidRequest,
            format!(
                "segment.json format_version {version} is out of range; \
                 this build reads format_version {FORMAT_VERSION}"
            ),
        )
    })?;
    if version != FORMAT_VERSION {
        return Err(CribleError::new(
            ErrorCode::InvalidRequest,
            format!(
                "unsupported segment format_version {version}; \
                 this build reads format_version {FORMAT_VERSION}"
            ),
        ));
    }
    Ok(version)
}

/// Finds the precision from the vector file present in the segment directory.
///
/// `segment.json` does not repeat the precision: exactly one of `vectors.f16`
/// and `vectors.f32` must be there.
fn read_precision(segment_dir: &Path) -> Result<ReferencePrecision, CribleError> {
    let f16_present = segment_dir
        .join(vector_file_name(ReferencePrecision::Fp16))
        .is_file();
    let f32_present = segment_dir
        .join(vector_file_name(ReferencePrecision::Fp32))
        .is_file();
    match (f16_present, f32_present) {
        (true, false) => Ok(ReferencePrecision::Fp16),
        (false, true) => Ok(ReferencePrecision::Fp32),
        (true, true) => Err(CribleError::new(
            ErrorCode::Internal,
            format!(
                "segment directory {} holds both {} and {}; exactly one vector \
                 file is expected",
                segment_dir.display(),
                vector_file_name(ReferencePrecision::Fp16),
                vector_file_name(ReferencePrecision::Fp32)
            ),
        )),
        (false, false) => Err(CribleError::new(
            ErrorCode::Internal,
            format!(
                "segment directory {} holds neither {} nor {}; exactly one \
                 vector file is expected",
                segment_dir.display(),
                vector_file_name(ReferencePrecision::Fp16),
                vector_file_name(ReferencePrecision::Fp32)
            ),
        )),
    }
}

/// Segment identifier of a segment directory: its file name.
fn segment_id_of(segment_dir: &Path) -> Result<String, CribleError> {
    segment_dir
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            CribleError::new(
                ErrorCode::Internal,
                format!(
                    "segment directory {} has no UTF-8 file name, so no segment_id",
                    segment_dir.display()
                ),
            )
        })
}

/// Reads the `checksums` object of a manifest.
fn read_checksums(manifest: &Value) -> Result<BTreeMap<String, String>, CribleError> {
    let Some(object) = manifest.get("checksums").and_then(Value::as_object) else {
        return Err(CribleError::new(
            ErrorCode::Internal,
            "segment.json has no checksums object",
        ));
    };
    let mut checksums = BTreeMap::new();
    for (name, value) in object {
        let Some(declared) = value.as_str() else {
            return Err(CribleError::new(
                ErrorCode::Internal,
                format!("checksum of {name} in segment.json is not a string"),
            ));
        };
        checksums.insert(name.clone(), declared.to_owned());
    }
    Ok(checksums)
}

/// Checks the declared checksum of the vector file against its bytes.
///
/// The file is streamed through the hash so a large segment never has to be
/// held in memory.
fn check_checksum(vectors_path: &Path, meta: &SegmentMeta) -> Result<(), CribleError> {
    let file_name = vector_file_name(meta.precision);
    let Some(declared) = meta.checksums.get(file_name) else {
        return Err(CribleError::new(
            ErrorCode::Internal,
            format!("segment.json declares no checksum for {file_name}"),
        ));
    };
    let mut file = File::open(vectors_path)
        .map_err(|error| io_error("open the vector file", vectors_path, &error))?;
    let mut hasher = blake3::Hasher::new();
    std::io::copy(&mut file, &mut hasher)
        .map_err(|error| io_error("read the vector file", vectors_path, &error))?;
    let computed = format!("blake3:{}", hasher.finalize().to_hex());
    if computed != *declared {
        return Err(CribleError::new(
            ErrorCode::Internal,
            format!(
                "{} does not match its checksum: segment.json declares \
                 {declared}, the file hashes to {computed}",
                vectors_path.display()
            ),
        ));
    }
    Ok(())
}

/// Reads a required unsigned integer field of a manifest.
fn manifest_u64(manifest: &Value, key: &str) -> Result<u64, CribleError> {
    let Some(value) = manifest.get(key) else {
        return Err(CribleError::new(
            ErrorCode::Internal,
            format!("segment.json has no {key}"),
        ));
    };
    value.as_u64().ok_or_else(|| {
        CribleError::new(
            ErrorCode::Internal,
            format!("segment.json {key} {value} is not an unsigned integer"),
        )
    })
}

/// Reads a required 32-bit field of a manifest.
fn manifest_u32(manifest: &Value, key: &str) -> Result<u32, CribleError> {
    let value = manifest_u64(manifest, key)?;
    u32::try_from(value).map_err(|_| {
        CribleError::new(
            ErrorCode::Internal,
            format!("segment.json {key} {value} does not fit in u32"),
        )
    })
}

/// Reads a required string field of a manifest.
fn manifest_str<'a>(manifest: &'a Value, key: &str) -> Result<&'a str, CribleError> {
    let Some(value) = manifest.get(key) else {
        return Err(CribleError::new(
            ErrorCode::Internal,
            format!("segment.json has no {key}"),
        ));
    };
    value.as_str().ok_or_else(|| {
        CribleError::new(
            ErrorCode::Internal,
            format!("segment.json {key} {value} is not a string"),
        )
    })
}

/// Serializes the manifest in canonical JSON: keys sorted, no space around
/// separators (ADR-0019), so the bytes do not depend on the run.
fn manifest_json(meta: &SegmentMeta) -> Result<String, CribleError> {
    let manifest = json!({
        "batch_id": meta.batch_id,
        "checksums": meta.checksums,
        "count": meta.count,
        "dims": meta.dims,
        "format_version": meta.format_version,
        "generation": meta.generation.get(),
        "iid_start": meta.iid_start.get(),
    });
    serde_json::to_string(&manifest).map_err(|error| {
        CribleError::new(
            ErrorCode::Internal,
            format!("cannot serialize segment.json: {error}"),
        )
    })
}

/// Appends one little-endian component to `buffer`.
fn encode_component(buffer: &mut Vec<u8>, precision: ReferencePrecision, value: f32) {
    match precision {
        ReferencePrecision::Fp16 => {
            buffer.extend_from_slice(&f16::from_f32(value).to_bits().to_le_bytes());
        }
        ReferencePrecision::Fp32 => buffer.extend_from_slice(&value.to_le_bytes()),
    }
}

/// Decodes a whole vector from little-endian components.
fn decode_vector(bytes: &[u8], precision: ReferencePrecision) -> Result<Vec<f32>, CribleError> {
    bytes
        .chunks_exact(bytes_per_component(precision))
        .map(|chunk| decode_component(chunk, precision))
        .collect()
}

/// Decodes one little-endian component.
fn decode_component(bytes: &[u8], precision: ReferencePrecision) -> Result<f32, CribleError> {
    match precision {
        ReferencePrecision::Fp16 => {
            let raw: [u8; 2] = bytes.try_into().map_err(|_| {
                CribleError::new(
                    ErrorCode::Internal,
                    format!("a stored fp16 component is {} bytes, not 2", bytes.len()),
                )
            })?;
            Ok(f32::from(f16::from_bits(u16::from_le_bytes(raw))))
        }
        ReferencePrecision::Fp32 => {
            let raw: [u8; 4] = bytes.try_into().map_err(|_| {
                CribleError::new(
                    ErrorCode::Internal,
                    format!("a stored fp32 component is {} bytes, not 4", bytes.len()),
                )
            })?;
            Ok(f32::from_le_bytes(raw))
        }
    }
}

/// Writes a file and fsyncs it before returning.
pub(crate) fn write_file_synced(path: &Path, bytes: &[u8]) -> Result<(), CribleError> {
    let mut file = File::create(path).map_err(|error| io_error("create the file", path, &error))?;
    file.write_all(bytes)
        .map_err(|error| io_error("write the file", path, &error))?;
    file.sync_all()
        .map_err(|error| io_error("fsync the file", path, &error))
}

/// fsyncs a directory, so a rename or a new entry survives a crash.
pub(crate) fn sync_directory(path: &Path) -> Result<(), CribleError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| io_error("fsync the directory", path, &error))
}

/// Wraps an I/O failure on a file of the store, which is an internal state
/// error.
pub(crate) fn io_error(action: &str, path: &Path, error: &std::io::Error) -> CribleError {
    CribleError::new(
        ErrorCode::Internal,
        format!("cannot {action} {}: {error}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::{FORMAT_VERSION, SegmentReader, SegmentSpec, SegmentWriter, validate_segment_id};
    use crible_core::config::ReferencePrecision;
    use crible_core::error::ErrorCode;
    use crible_core::ids::{Generation, Iid};
    use std::fs;
    use std::path::Path;

    /// Builds a specification for the tests of this module.
    fn spec(segment_id: &str, dims: u32) -> SegmentSpec {
        SegmentSpec {
            segment_id: segment_id.to_owned(),
            iid_start: Iid::new(0),
            dims,
            generation: Generation::new(1),
            batch_id: "batch-empty".to_owned(),
            precision: ReferencePrecision::Fp32,
        }
    }

    /// A segment with no vector still opens: an epoch may attach an empty batch.
    #[test]
    fn inv_15_empty_segment_opens_without_mapping() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let segments = dir.path().join("segments");
        fs::create_dir_all(&segments).expect("the segments directory is created");
        let writer = SegmentWriter::create(&segments, spec("seg-empty", 4))
            .expect("the writer starts in a temporary directory");
        let meta = writer.finish().expect("the empty segment is committed");
        assert_eq!(meta.count, 0);

        let reader = SegmentReader::open(&segments.join("seg-empty")).expect("it opens");
        assert_eq!(reader.meta().format_version, FORMAT_VERSION);
        assert_eq!(reader.meta().precision, ReferencePrecision::Fp32);
        assert!(!reader.contains(Iid::new(0)));
    }

    /// A segment identifier must name exactly one directory of `segments/`, and
    /// must not be one the recovery of `tmp-*` would delete.
    #[test]
    fn segment_id_must_be_a_safe_directory_name() {
        for accepted in ["seg-1", "seg-hand-f16", "2026-10-21.1", "a"] {
            validate_segment_id(accepted)
                .unwrap_or_else(|error| panic!("{accepted:?} must be accepted: {error}"));
        }
        for refused in [
            "",
            ".",
            "..",
            "a/b",
            "../escape",
            "/absolute",
            "a\\b",
            "tmp-seg-1",
        ] {
            let error = validate_segment_id(refused)
                .expect_err("a segment_id that is not a plain directory name must be refused");
            assert_eq!(
                error.code(),
                ErrorCode::InvalidRequest,
                "wrong error code for {refused:?}: {}",
                error.message()
            );
        }
    }

    /// A segment of zero dimensions would hold no readable row: refused as a bad
    /// request when the caller asks for it.
    #[test]
    fn create_refuses_zero_dims() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let segments = dir.path().join("segments");
        fs::create_dir_all(&segments).expect("the segments directory is created");

        let error = SegmentWriter::create(&segments, spec("seg-zero", 0))
            .expect_err("a segment of zero dimensions must be refused");
        assert_eq!(
            error.code(),
            ErrorCode::InvalidRequest,
            "wrong error code: {}",
            error.message()
        );
        assert!(
            !segments.join("tmp-seg-zero").exists(),
            "a refused creation must leave no temporary directory"
        );
    }

    /// The same zero dimensionality found on disk is a corrupted segment, not a
    /// bad request: opening it is an internal error.
    #[test]
    fn open_refuses_zero_dims() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let segments = dir.path().join("segments");
        let segment_dir = segments.join("seg-zero");
        fs::create_dir_all(&segment_dir).expect("the segment directory is created");

        let vectors: [u8; 0] = [];
        fs::write(segment_dir.join("vectors.f32"), vectors).expect("an empty vector file");
        let manifest = serde_json::json!({
            "batch_id": "batch-empty",
            "checksums": { "vectors.f32": format!("blake3:{}", blake3::hash(&vectors).to_hex()) },
            "count": 1,
            "dims": 0,
            "format_version": FORMAT_VERSION,
            "generation": 1,
            "iid_start": 0,
        })
        .to_string();
        fs::write(segment_dir.join("segment.json"), manifest).expect("segment.json is written");

        let error = SegmentReader::open(&segment_dir)
            .expect_err("a segment of zero dimensions must not open");
        assert_eq!(
            error.code(),
            ErrorCode::Internal,
            "wrong error code: {}",
            error.message()
        );
        assert!(
            error.message().contains("dimensions"),
            "the message must name the dimensions: {}",
            error.message()
        );
    }

    /// A residual `tmp-<id>` directory blocks a new attempt of the same id until
    /// the recovery of `GERER-GENERATIONS` removes it.
    #[test]
    fn create_fails_on_a_residual_temporary_directory() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let segments = dir.path().join("segments");
        fs::create_dir_all(segments.join("tmp-seg-1")).expect("the residual directory is created");

        let error = SegmentWriter::create(&segments, spec("seg-1", 4))
            .expect_err("a residual temporary directory must be reported");
        assert_eq!(
            error.code(),
            ErrorCode::Internal,
            "wrong error code: {}",
            error.message()
        );
        assert!(
            Path::new(&segments.join("tmp-seg-1")).is_dir(),
            "the writer never deletes a temporary directory: that is recovery's job"
        );
    }
}
