// SPDX-License-Identifier: Apache-2.0

//! INV-15 — every on-disk format is explicitly little-endian and versioned.
//!
//! For a segment (`segments/<segment_id>/segment.json` + `vectors.f16` or
//! `vectors.f32`), the invariant is proved two ways:
//!
//! - bytes are written **by hand** in the test, with known IEEE-754 values, and
//!   the reader must return those values for the documented ids;
//! - the writer must produce exactly the same bytes, so a file written by
//!   Crible is readable by anyone who follows `docs/02-architecture.md`;
//! - `segment.json` carries a `format_version`, is refused when that version is
//!   unknown or missing, and its bytes do not depend on the run.
//!
//! References: `docs/08-invariants-et-tests.md` (INV-15),
//! `docs/02-architecture.md` ("Organisation sur disque") and the
//! `ECRIRE-SEGMENTS-VECTEURS` task of
//! `docs/phases/P1-stockage-et-recherche-exacte.md`.
//!
//! Assumed API of the future `crible_store::segment` module (the tests do not
//! compile until `@dev` provides it):
//!
//! ```ignore
//! pub const FORMAT_VERSION: u32 = 1;
//!
//! // `precision` is the `ReferencePrecision` of the index configuration, the
//! // only place that names the precision of the reference vectors. The names of
//! // the vector files stay private to `crible_store::segment`.
//!
//! pub struct SegmentSpec {
//!     pub segment_id: String,
//!     pub iid_start: Iid,
//!     pub dims: u32,
//!     pub generation: Generation,
//!     pub batch_id: String,
//!     pub precision: ReferencePrecision,
//! }
//!
//! pub struct SegmentMeta {
//!     pub format_version: u32,
//!     pub segment_id: String,
//!     pub iid_start: Iid,
//!     pub count: u32,
//!     pub dims: u32,
//!     pub generation: Generation,
//!     pub batch_id: String,
//!     pub precision: ReferencePrecision,
//!     pub checksums: std::collections::BTreeMap<String, String>,
//! }
//!
//! pub struct SegmentWriter;
//! impl SegmentWriter {
//!     pub fn create(segments_dir: &Path, spec: SegmentSpec) -> Result<Self, CribleError>;
//!     pub fn append(&mut self, vector: &[f32]) -> Result<Iid, CribleError>;
//!     pub fn finish(self) -> Result<SegmentMeta, CribleError>;
//! }
//!
//! pub struct SegmentReader;
//! impl SegmentReader {
//!     pub fn open(segment_dir: &Path) -> Result<Self, CribleError>;
//!     pub fn meta(&self) -> &SegmentMeta;
//!     pub fn contains(&self, iid: Iid) -> bool;
//!     pub fn vector(&self, iid: Iid) -> Result<Vec<f32>, CribleError>;
//! }
//! ```
//!
//! `segment.json` holds exactly the fields of `docs/02-architecture.md`, plus
//! the format version (keys in canonical order, one space of indentation or
//! none, both accepted):
//!
//! ```json
//! {
//!   "batch_id": "batch-2026-10-21",
//!   "checksums": { "vectors.f16": "blake3:<64 lowercase hex digits>" },
//!   "count": 2,
//!   "dims": 2,
//!   "format_version": 1,
//!   "generation": 7,
//!   "iid_start": 4
//! }
//! ```
//!
//! Neither the precision nor the segment identifier is written: the precision
//! follows from the single `vectors.f16` or `vectors.f32` file present, and the
//! identifier from the name of the segment directory. Both stay available in
//! memory through `SegmentMeta`.
//!
//! `vectors.f16` is `count × dims × 2` bytes, fp16 little-endian, row-major:
//! the vector of `iid_start + k` starts at offset `k × dims × 2`.

use std::fs;
use std::path::{Path, PathBuf};

use crible_core::config::ReferencePrecision;
use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::{Generation, Iid};
use crible_store::segment::{FORMAT_VERSION, SegmentReader, SegmentSpec, SegmentWriter};
use serde_json::{Value, json};

/// Identifier of the segment used by the hand-written fixtures.
const SEGMENT_ID: &str = "seg-hand";

/// Name of the vector file of a segment of `precision`, as documented in
/// `docs/02-architecture.md` ("Organisation sur disque").
fn vectors_file_name(precision: ReferencePrecision) -> &'static str {
    match precision {
        ReferencePrecision::Fp16 => "vectors.f16",
        ReferencePrecision::Fp32 => "vectors.f32",
    }
}

/// `blake3:<hex>` form of the digest of `bytes`, the shape used by every
/// checksum of the project (`docs/format-lot-v1.md`, "Fichiers et checksums").
fn blake3_hex(bytes: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(bytes).to_hex())
}

/// `segment.json` of a hand-written segment, with the checksums of `vectors`.
///
/// The precision and the segment identifier are absent on purpose: they are
/// derived from the vector file present and from the directory name.
fn handwritten_manifest(
    iid_start: u32,
    count: u32,
    dims: u32,
    precision: ReferencePrecision,
    vectors: &[u8],
    format_version: Option<u32>,
) -> String {
    let mut manifest = json!({
        "iid_start": iid_start,
        "count": count,
        "dims": dims,
        "generation": 7,
        "batch_id": "batch-2026-10-21",
        "checksums": { vectors_file_name(precision): blake3_hex(vectors) },
    });
    if let Some(version) = format_version {
        manifest["format_version"] = json!(version);
    }
    manifest.to_string()
}

/// Writes `segment.json` and the vector file of a hand-written segment and
/// returns its directory.
fn handwritten_segment(
    root: &Path,
    segment_id: &str,
    precision: ReferencePrecision,
    vectors: &[u8],
    manifest: &str,
) -> PathBuf {
    let segment_dir = root.join("segments").join(segment_id);
    fs::create_dir_all(&segment_dir)
        .unwrap_or_else(|error| panic!("the segment directory is created: {error}"));
    fs::write(segment_dir.join(vectors_file_name(precision)), vectors)
        .unwrap_or_else(|error| panic!("the vector file is written byte by byte: {error}"));
    fs::write(segment_dir.join("segment.json"), manifest)
        .unwrap_or_else(|error| panic!("segment.json is written: {error}"));
    segment_dir
}

fn spec(segment_id: &str, iid_start: u32, dims: u32, precision: ReferencePrecision) -> SegmentSpec {
    SegmentSpec {
        segment_id: segment_id.to_owned(),
        iid_start: Iid::new(iid_start),
        dims,
        generation: Generation::new(7),
        batch_id: "batch-2026-10-21".to_owned(),
        precision,
    }
}

/// Asserts that opening `segment_dir` is refused with `expected` and that the
/// message contains every needle.
fn assert_open_rejected(segment_dir: &Path, expected: ErrorCode, needles: &[&str]) -> CribleError {
    let error = match SegmentReader::open(segment_dir) {
        Ok(_) => panic!("the segment at {segment_dir:?} must be refused"),
        Err(error) => error,
    };
    assert_eq!(
        error.code(),
        expected,
        "wrong error code for {segment_dir:?}: {}",
        error.message()
    );
    for needle in needles {
        assert!(
            error.message().contains(needle),
            "message {:?} must contain {needle:?}",
            error.message()
        );
    }
    error
}

/// Hand-written `vectors.f16` of two vectors of two dimensions:
/// `[1.0, -2.0]` then `[0.5, 0.0]`.
///
/// Each fp16 is two little-endian bytes, low byte first: 1.0 = `0x3C00` →
/// `00 3C`, -2.0 = `0xC000` → `00 C0`, 0.5 = `0x3800` → `00 38`, 0.0 = `0x0000`
/// → `00 00`.
const F16_KNOWN_VECTORS: [u8; 8] = [0x00, 0x3C, 0x00, 0xC0, 0x00, 0x38, 0x00, 0x00];

/// Hand-written `vectors.f32` of the same two vectors, four little-endian
/// bytes per component.
const F32_KNOWN_VECTORS: [u8; 16] = [
    0x00, 0x00, 0x80, 0x3F, // 1.0
    0x00, 0x00, 0x00, 0xC0, // -2.0
    0x00, 0x00, 0x00, 0x3F, // 0.5
    0x00, 0x00, 0x00, 0x00, // 0.0
];

#[test]
fn inv_15_handwritten_f16_segment_reads_known_values() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let manifest = handwritten_manifest(
        4,
        2,
        2,
        ReferencePrecision::Fp16,
        &F16_KNOWN_VECTORS,
        Some(FORMAT_VERSION),
    );
    let segment_dir = handwritten_segment(
        dir.path(),
        SEGMENT_ID,
        ReferencePrecision::Fp16,
        &F16_KNOWN_VECTORS,
        &manifest,
    );

    let reader = SegmentReader::open(&segment_dir)
        .expect("a segment whose bytes follow the documented format opens");

    let meta = reader.meta();
    assert_eq!(meta.format_version, FORMAT_VERSION);
    assert_eq!(meta.segment_id, SEGMENT_ID);
    assert_eq!(meta.iid_start, Iid::new(4));
    assert_eq!(meta.count, 2);
    assert_eq!(meta.dims, 2);
    assert_eq!(meta.generation, Generation::new(7));
    assert_eq!(meta.batch_id, "batch-2026-10-21");
    assert_eq!(meta.precision, ReferencePrecision::Fp16);

    assert_eq!(
        reader.vector(Iid::new(4)).expect("first vector"),
        vec![1.0_f32, -2.0],
        "fp16 0x3C00 is 1.0 and 0xC000 is -2.0, little-endian"
    );
    assert_eq!(
        reader.vector(Iid::new(5)).expect("second vector"),
        vec![0.5_f32, 0.0],
        "fp16 0x3800 is 0.5 and 0x0000 is 0.0, little-endian"
    );
}

#[test]
fn inv_15_handwritten_f32_segment_reads_known_values() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let manifest = handwritten_manifest(
        4,
        2,
        2,
        ReferencePrecision::Fp32,
        &F32_KNOWN_VECTORS,
        Some(FORMAT_VERSION),
    );
    let segment_dir = handwritten_segment(
        dir.path(),
        SEGMENT_ID,
        ReferencePrecision::Fp32,
        &F32_KNOWN_VECTORS,
        &manifest,
    );

    let reader = SegmentReader::open(&segment_dir).expect("the f32 segment opens");
    assert_eq!(reader.meta().precision, ReferencePrecision::Fp32);
    assert_eq!(
        reader.vector(Iid::new(4)).expect("first vector"),
        vec![1.0_f32, -2.0]
    );
    assert_eq!(
        reader.vector(Iid::new(5)).expect("second vector"),
        vec![0.5_f32, 0.0]
    );
}

#[test]
fn inv_15_writer_writes_expected_f16_bytes() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let segments = dir.path().join("segments");
    fs::create_dir_all(&segments).expect("the segments directory is created");

    let mut writer =
        SegmentWriter::create(&segments, spec(SEGMENT_ID, 0, 2, ReferencePrecision::Fp16))
            .expect("the writer starts in a temporary directory");
    assert_eq!(
        writer.append(&[1.0, -2.0]).expect("first vector"),
        Iid::new(0)
    );
    assert_eq!(
        writer.append(&[0.5, 0.0]).expect("second vector"),
        Iid::new(1)
    );
    let meta = writer.finish().expect("the segment is committed");

    let raw = fs::read(segments.join(SEGMENT_ID).join("vectors.f16"))
        .expect("vectors.f16 exists after finish");
    assert_eq!(
        raw,
        F16_KNOWN_VECTORS.to_vec(),
        "the writer must emit fp16 little-endian components, low byte first"
    );
    assert_eq!(
        meta.checksums.get("vectors.f16"),
        Some(&blake3_hex(&raw)),
        "the checksum of segment.json is the blake3 of the bytes on disk"
    );
}

#[test]
fn inv_15_writer_writes_expected_f32_bytes() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let segments = dir.path().join("segments");
    fs::create_dir_all(&segments).expect("the segments directory is created");

    let mut writer =
        SegmentWriter::create(&segments, spec(SEGMENT_ID, 0, 2, ReferencePrecision::Fp32))
            .expect("the writer starts in a temporary directory");
    writer.append(&[1.0, -2.0]).expect("first vector");
    writer.append(&[0.5, 0.0]).expect("second vector");
    writer.finish().expect("the segment is committed");

    let raw = fs::read(segments.join(SEGMENT_ID).join("vectors.f32"))
        .expect("vectors.f32 exists after finish");
    assert_eq!(
        raw,
        F32_KNOWN_VECTORS.to_vec(),
        "the writer must emit fp32 little-endian components, low byte first"
    );
}

#[test]
fn inv_15_segment_json_records_current_format_version() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let segments = dir.path().join("segments");
    fs::create_dir_all(&segments).expect("the segments directory is created");

    let mut writer =
        SegmentWriter::create(&segments, spec(SEGMENT_ID, 4, 2, ReferencePrecision::Fp16))
            .expect("the writer starts in a temporary directory");
    writer.append(&[1.0, -2.0]).expect("first vector");
    writer.append(&[0.5, 0.0]).expect("second vector");
    writer.finish().expect("the segment is committed");

    let raw = fs::read_to_string(segments.join(SEGMENT_ID).join("segment.json"))
        .expect("segment.json exists after finish");
    let manifest: Value = serde_json::from_str(&raw).expect("segment.json is valid JSON");

    assert_eq!(
        manifest["format_version"],
        json!(FORMAT_VERSION),
        "segment.json must carry the version of this format"
    );
    assert_eq!(manifest["iid_start"], json!(4));
    assert_eq!(manifest["count"], json!(2));
    assert_eq!(manifest["dims"], json!(2));
    assert_eq!(manifest["generation"], json!(7));
    assert_eq!(manifest["batch_id"], json!("batch-2026-10-21"));
    assert!(
        manifest.get("precision").is_none(),
        "the precision follows from the vector file present, segment.json must not repeat it"
    );
    assert!(
        manifest.get("segment_id").is_none(),
        "the segment identifier follows from the directory name, segment.json must not repeat it"
    );

    let checksums = manifest["checksums"]
        .as_object()
        .expect("checksums is an object keyed by file name");
    let declared = checksums
        .get("vectors.f16")
        .and_then(Value::as_str)
        .expect("the vectors file has a checksum");
    assert!(
        declared.starts_with("blake3:") && declared.len() == "blake3:".len() + 64,
        "checksums use the blake3:<hex> form of the project, got {declared:?}"
    );

    // The canonical form is exact, not just stable across runs: keys sorted,
    // no space around separators, no trailing newline (ADR-0019). Comparing to
    // a literal proves the writer sorts the keys; comparing two runs would only
    // prove it is deterministic.
    let vectors = fs::read(segments.join(SEGMENT_ID).join("vectors.f16"))
        .expect("vectors.f16 exists after finish");
    let expected = format!(
        concat!(
            r#"{{"batch_id":"batch-2026-10-21","#,
            r#""checksums":{{"vectors.f16":"{}"}},"#,
            r#""count":2,"dims":2,"format_version":{},"generation":7,"iid_start":4}}"#,
        ),
        blake3_hex(&vectors),
        FORMAT_VERSION
    );
    assert_eq!(
        raw, expected,
        "segment.json must be canonical JSON: sorted keys, no space, no newline"
    );
}

#[test]
fn inv_15_writer_output_bytes_are_identical_across_runs() {
    let first = tempfile::tempdir().expect("temporary directory");
    let second = tempfile::tempdir().expect("temporary directory");

    let mut manifests = Vec::new();
    let mut vectors = Vec::new();
    for root in [first.path(), second.path()] {
        let segments = root.join("segments");
        fs::create_dir_all(&segments).expect("the segments directory is created");
        let mut writer =
            SegmentWriter::create(&segments, spec(SEGMENT_ID, 4, 2, ReferencePrecision::Fp16))
                .expect("the writer starts in a temporary directory");
        writer.append(&[1.0, -2.0]).expect("first vector");
        writer.append(&[0.5, 0.0]).expect("second vector");
        writer.finish().expect("the segment is committed");
        manifests.push(
            fs::read(segments.join(SEGMENT_ID).join("segment.json"))
                .expect("segment.json exists after finish"),
        );
        vectors.push(
            fs::read(segments.join(SEGMENT_ID).join("vectors.f16"))
                .expect("vectors.f16 exists after finish"),
        );
    }

    assert_eq!(
        manifests[0], manifests[1],
        "segment.json must be deterministic: no timestamp, no run-dependent key order"
    );
    assert_eq!(
        vectors[0], vectors[1],
        "the vector file is deterministic too"
    );
}

#[test]
fn inv_15_unknown_format_version_is_rejected_on_open() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let manifest = handwritten_manifest(
        4,
        2,
        2,
        ReferencePrecision::Fp16,
        &F16_KNOWN_VECTORS,
        Some(999),
    );
    let segment_dir = handwritten_segment(
        dir.path(),
        SEGMENT_ID,
        ReferencePrecision::Fp16,
        &F16_KNOWN_VECTORS,
        &manifest,
    );

    assert_open_rejected(
        &segment_dir,
        ErrorCode::InvalidRequest,
        &["999", &FORMAT_VERSION.to_string()],
    );
}

#[test]
fn inv_15_missing_format_version_is_rejected_on_open() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let manifest =
        handwritten_manifest(4, 2, 2, ReferencePrecision::Fp16, &F16_KNOWN_VECTORS, None);
    let segment_dir = handwritten_segment(
        dir.path(),
        SEGMENT_ID,
        ReferencePrecision::Fp16,
        &F16_KNOWN_VECTORS,
        &manifest,
    );

    assert_open_rejected(&segment_dir, ErrorCode::InvalidRequest, &["format_version"]);
}
