// SPDX-License-Identifier: Apache-2.0

// `clippy::panic` is allowed on the fixture helpers that stop their test, not on
// the whole file: `clippy.toml` covers `#[test]` functions only.

//! INV-15 — `codes.bin` is explicitly little-endian and versioned.
//!
//! For a segment the invariant was proved on the vectors by
//! `tests/inv_15_segment_vectors.rs`. This file proves it for the derived file
//! `codes.bin` of the same segment directory, as `ADR-0069` fixes its format and
//! its reconstruction:
//!
//! - bits are written **by hand** in the test, with known values, and the
//!   reader must return exactly those `u64` words;
//! - the writer must produce exactly those bytes, low byte first, so a
//!   `codes.bin` written by Crible is readable by anyone who follows
//!   `docs/02-architecture.md` and `ADR-0069`;
//! - a vector whose dimensions are not a multiple of 64 keeps zero bits above
//!   its dimensions, so no garbage can reach a popcount;
//! - a truncated or oversized `codes.bin` is refused with `Internal`, naming the
//!   file: a wrong length is a corruption, never a derived file to rebuild;
//! - an **absent** `codes.bin` is not an error (`ADR-0069`, decision 3). The
//!   writer open rebuilds it from the reference vectors, persists it atomically
//!   and byte for byte identical to what the attach wrote, without a new
//!   generation and without a new epoch (decision 1). A read-only open builds
//!   the missing codes in RAM and writes nothing under `epochs/` (decision 2).
//!   A restored snapshot, which never carries `codes.bin` (ADR-0064, decision 4),
//!   therefore rebuilds the same bytes at the first write open.
//!
//! `codes.bin` carries **no header of its own**: the version that governs it is
//! the `format_version` of `segment.json`, exactly as `docs/02-architecture.md`
//! describes the file and as `ADR-0069` decision 6 states. An unknown version is
//! therefore refused once, at the segment, never twice with two values that
//! could disagree.
//!
//! API of `crible_store::codes` used by these tests:
//!
//! ```ignore
//! /// Name of the binary-code file of a segment directory.
//! pub const CODES_FILE_NAME: &str = "codes.bin";
//!
//! /// Number of `u64` words of one code: `ceil(dims / 64)`.
//! pub fn words_per_code(dims: u32) -> usize;
//!
//! /// Byte length of a `codes.bin` holding `count` codes of `dims` dimensions,
//! /// `count × words_per_code(dims) × 8` (ADR-0069, decision 6). Two `u32`
//! /// operands cannot overflow a `u64`, so the function is total.
//! pub fn codes_len(count: u32, dims: u32) -> u64;
//!
//! /// Code of one vector: bit `i` is 1 when `vector[i] > 0`, the bits
//! /// `dims..64 × words` stay zero (ADR-0004, ADR-0069 decision 6).
//! pub fn quantize(vector: &[f32]) -> Vec<u64>;
//!
//! /// Reads a whole `codes.bin`: `count` codes of `dims` dimensions, one
//! /// little-endian `u64` per word, code after code. A length that is not
//! /// exactly `codes_len(count, dims)` is an `ErrorCode::Internal` error naming
//! /// the file and both lengths (ADR-0069, decision 3).
//! pub fn read_codes_bin(path: &Path, count: u32, dims: u32) -> Result<Vec<u64>, CribleError>;
//! ```
//!
//! And on the existing reader and writer:
//!
//! ```ignore
//! impl SegmentReader {
//!     /// Opens the read-only view of one segment. When `codes.bin` is absent,
//!     /// the codes are quantized from `vectors.f16` or `vectors.f32` **in RAM**
//!     /// and nothing is written under the epoch (ADR-0069, decision 2). A
//!     /// `codes.bin` of the wrong length is still an `Internal` error.
//!     pub fn open(segment_dir: &Path) -> Result<Self, CribleError>;
//!
//!     /// Code of `iid`, in RAM: from `codes.bin` when it is present, from the
//!     /// reference vectors otherwise (`docs/02-architecture.md`, "Concurrence").
//!     pub fn codes(&self, iid: Iid) -> Result<&[u64], CribleError>;
//! }
//! ```
//!
//! The writer open is the recovery path of the index: `GenerationStore::open`
//! rebuilds every missing `codes.bin` of the published epoch, atomically, and
//! creates neither a generation nor an epoch (ADR-0069, decision 1). It may use
//! an internal `codes::rebuild_codes(&Path) -> Result<u64, CribleError>`; the
//! tests below never call it directly, they go through the open.
//!
//! `codes.bin` has **no checksum** of its own in `segment.json`: it is a derived
//! file and its byte-exact length is what the reader checks (ADR-0069,
//! decision 6). The vector files keep theirs.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use crible_core::config::{IndexConfig, ReferencePrecision};
use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::{Generation, Iid};
use crible_store::codes::{CODES_FILE_NAME, codes_len, quantize, read_codes_bin, words_per_code};
use crible_store::segment::{FORMAT_VERSION, SegmentReader, SegmentSpec, SegmentWriter};
use crible_store::snapshot;
use serde_json::json;

/// Identifier of the segment used by the hand-written fixtures.
const SEGMENT_ID: &str = "seg-codes";

/// Dimensionality of the hand-written fixtures: not a multiple of 64, so the
/// padding of the last word is part of every assertion.
const DIMS: u32 = 3;

/// First internal identifier of the hand-written fixtures.
const IID_START: u32 = 4;

/// `blake3:<hex>` form of the digest of `bytes`, the shape every checksum of the
/// project uses.
fn blake3_hex(bytes: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(bytes).to_hex())
}

/// `segment.json` of a hand-written segment, with the checksum of its vector
/// file. `codes.bin` gets no entry: it is derived (ADR-0069, decision 6).
fn handwritten_manifest(
    iid_start: u32,
    count: u32,
    dims: u32,
    vectors: &[u8],
    format_version: Option<u32>,
) -> String {
    let mut manifest = json!({
        "iid_start": iid_start,
        "count": count,
        "dims": dims,
        "generation": 7,
        "batch_id": "batch-2026-10-26",
        "checksums": { "vectors.f16": blake3_hex(vectors) },
    });
    if let Some(version) = format_version {
        manifest["format_version"] = json!(version);
    }
    manifest.to_string()
}

/// Writes a hand-written segment: `segment.json`, `vectors.f16`, and
/// `codes.bin` when `codes` is given.
///
/// A fixture helper stops the calling test on failure: `clippy::panic` is allowed
/// here, one helper at a time, because `clippy.toml` covers `#[test]` functions
/// only.
#[allow(clippy::panic)]
fn handwritten_segment(
    root: &Path,
    dims: u32,
    count: u32,
    vectors: &[u8],
    codes: Option<&[u8]>,
    format_version: Option<u32>,
) -> PathBuf {
    let segment_dir = root.join("segments").join(SEGMENT_ID);
    fs::create_dir_all(&segment_dir)
        .unwrap_or_else(|error| panic!("the segment directory is created: {error}"));
    fs::write(segment_dir.join("vectors.f16"), vectors)
        .unwrap_or_else(|error| panic!("the vector file is written byte by byte: {error}"));
    if let Some(codes) = codes {
        fs::write(segment_dir.join(CODES_FILE_NAME), codes)
            .unwrap_or_else(|error| panic!("codes.bin is written byte by byte: {error}"));
    }
    let manifest = handwritten_manifest(IID_START, count, dims, vectors, format_version);
    fs::write(segment_dir.join("segment.json"), manifest)
        .unwrap_or_else(|error| panic!("segment.json is written: {error}"));
    segment_dir
}

fn spec(dims: u32) -> SegmentSpec {
    SegmentSpec {
        segment_id: SEGMENT_ID.to_owned(),
        iid_start: Iid::new(IID_START),
        dims,
        generation: Generation::new(7),
        batch_id: "batch-2026-10-26".to_owned(),
        precision: ReferencePrecision::Fp16,
    }
}

/// Starts a segment writer in `<root>/segments/` for `dims` dimensions.
///
/// The helper returns its error instead of stopping: `clippy::panic` is allowed
/// in `#[test]` functions only (`clippy.toml`), so the calling test is where a
/// failure is reported.
fn writer(root: &Path, dims: u32) -> Result<SegmentWriter, CribleError> {
    let segments = root.join("segments");
    fs::create_dir_all(&segments).map_err(|error| {
        CribleError::new(
            ErrorCode::Internal,
            format!(
                "the segments directory {} is created: {error}",
                segments.display()
            ),
        )
    })?;
    SegmentWriter::create(&segments, spec(dims))
}

/// Sorted names of the entries of `dir`.
///
/// The helpers of this file allow `clippy::panic` one by one: `clippy.toml`
/// covers `#[test]` functions only, and a helper that cannot read its fixture
/// must stop the test that called it.
#[allow(clippy::panic)]
fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", dir.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("an entry must be readable: {error}"))
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort_unstable();
    names
}

/// Asserts that opening `segment_dir` is refused with `expected` and that the
/// message contains every needle.
///
/// Same reason as [`listing`] for allowing `clippy::panic`: the helper must stop
/// the calling test when the open unexpectedly succeeds.
#[allow(clippy::panic)]
fn assert_open_rejected(segment_dir: &Path, expected: ErrorCode, needles: &[&str]) {
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
}

/// Hand-written `vectors.f16` of two vectors of three dimensions:
/// `[1.0, -2.0, 0.0]` then `[0.0, 0.5, -1.0]`.
///
/// Each fp16 is two little-endian bytes: 1.0 = `0x3C00` → `00 3C`,
/// -2.0 = `0xC000` → `00 C0`, 0.0 = `0x0000` → `00 00`, 0.5 = `0x3800` → `00 38`,
/// -1.0 = `0xBC00` → `00 BC`.
const F16_KNOWN_VECTORS: [u8; 12] = [
    0x00, 0x3C, 0x00, 0xC0, 0x00, 0x00, // [1.0, -2.0, 0.0]
    0x00, 0x00, 0x00, 0x38, 0x00, 0xBC, // [0.0, 0.5, -1.0]
];

/// Hand-written `codes.bin` of those two vectors: one `u64` per code, eight
/// little-endian bytes.
///
/// First vector: only 1.0 is positive, so bit 0 is set → word `1`.
/// Second vector: only 0.5 is positive, so bit 1 is set → word `2`.
/// Neither 0.0 nor -2.0 nor -1.0 is `> 0`.
const CODES_KNOWN_WORDS: [u8; 16] = [
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // code of [1.0, -2.0, 0.0]
    0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // code of [0.0, 0.5, -1.0]
];

#[test]
fn inv_15_codes_length_rounds_up_to_whole_words() {
    assert_eq!(words_per_code(1), 1, "one word holds 64 dimensions");
    assert_eq!(words_per_code(64), 1, "64 dimensions fit one word exactly");
    assert_eq!(
        words_per_code(65),
        2,
        "65 dimensions need a second word, never a partial byte"
    );
    assert_eq!(
        words_per_code(768),
        12,
        "768 dimensions are 96 bytes, the d/8 of docs/02:72"
    );
    assert_eq!(codes_len(0, DIMS), 0, "a segment may hold no code");
    assert_eq!(
        codes_len(2, DIMS),
        16,
        "two codes of three dimensions are two whole words"
    );
    assert_eq!(
        codes_len(3, 65),
        48,
        "three codes of 65 dimensions are three times two words"
    );
}

#[test]
fn inv_15_handwritten_codes_read_known_bits() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let segment_dir = handwritten_segment(
        dir.path(),
        DIMS,
        2,
        &F16_KNOWN_VECTORS,
        Some(&CODES_KNOWN_WORDS),
        Some(FORMAT_VERSION),
    );

    assert_eq!(
        read_codes_bin(&segment_dir.join(CODES_FILE_NAME), 2, DIMS)
            .expect("a codes.bin whose bytes follow the documented format reads"),
        vec![1_u64, 2],
        "bit i is 1 when x_i > 0, and the words are little-endian"
    );

    let reader = SegmentReader::open(&segment_dir)
        .expect("a segment whose codes.bin follows the documented format opens");
    assert_eq!(
        reader.codes(Iid::new(IID_START)).expect("first code"),
        &[1_u64],
        "the first vector of the segment has exactly one positive component"
    );
    assert_eq!(
        reader.codes(Iid::new(IID_START + 1)).expect("second code"),
        &[2_u64],
        "the second vector has exactly one positive component, at index 1"
    );
    assert_eq!(
        fs::metadata(segment_dir.join(CODES_FILE_NAME))
            .expect("codes.bin exists")
            .len(),
        codes_len(2, DIMS),
        "codes.bin is exactly count × words_per_code × 8 bytes"
    );
}

#[test]
fn inv_15_writer_packs_known_vectors_into_expected_bytes() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let mut writer =
        writer(dir.path(), DIMS).expect("the fixture writer starts in a temporary directory");
    assert_eq!(
        writer.append(&[1.0, -2.0, 0.0]).expect("first vector"),
        Iid::new(IID_START)
    );
    assert_eq!(
        writer.append(&[0.0, 0.5, -1.0]).expect("second vector"),
        Iid::new(IID_START + 1)
    );
    writer.finish().expect("the segment is committed");

    let segment_dir = dir.path().join("segments").join(SEGMENT_ID);
    let raw = fs::read(segment_dir.join(CODES_FILE_NAME)).expect("codes.bin exists after finish");
    assert_eq!(
        raw,
        CODES_KNOWN_WORDS.to_vec(),
        "the writer must emit one little-endian u64 per code, low byte first, \
         bit i set when x_i > 0"
    );
    assert_eq!(
        raw.len() as u64,
        codes_len(2, DIMS),
        "the writer must not pad the file beyond the whole words"
    );
}

#[test]
fn inv_15_codes_second_word_is_little_endian() {
    const WIDE_DIMS: u32 = 70;
    let dir = tempfile::tempdir().expect("temporary directory");
    let mut writer =
        writer(dir.path(), WIDE_DIMS).expect("the fixture writer starts in a temporary directory");

    // Dimensions 0 and 64 are positive: one bit in the first word, one in the
    // second. A big-endian writer would emit `00 … 01` at each word.
    let mut vector = vec![0.0_f32; WIDE_DIMS as usize];
    vector[0] = 1.0;
    vector[64] = 2.0;
    writer.append(&vector).expect("the wide vector");

    // Everything negative: the words are all zero.
    let zero = vec![-1.0_f32; WIDE_DIMS as usize];
    writer.append(&zero).expect("the negative vector");
    writer.finish().expect("the segment is committed");

    let raw = fs::read(
        dir.path()
            .join("segments")
            .join(SEGMENT_ID)
            .join(CODES_FILE_NAME),
    )
    .expect("codes.bin exists after finish");
    let expected: [u8; 32] = [
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // word 0: bit 0
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // word 1: bit 64
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // word 0: nothing positive
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // word 1: nothing positive
    ];
    assert_eq!(
        raw,
        expected.to_vec(),
        "each u64 word is little-endian, and a negative or null component clears its bit"
    );
}

#[test]
fn inv_15_codes_padding_bits_above_dims_are_zero() {
    const PADDED_DIMS: u32 = 65;
    let dir = tempfile::tempdir().expect("temporary directory");
    let mut writer = writer(dir.path(), PADDED_DIMS)
        .expect("the fixture writer starts in a temporary directory");

    let mut vector = vec![0.0_f32; PADDED_DIMS as usize];
    vector[0] = 1.0;
    writer.append(&vector).expect("the padded vector");
    writer.finish().expect("the segment is committed");

    let raw = fs::read(
        dir.path()
            .join("segments")
            .join(SEGMENT_ID)
            .join(CODES_FILE_NAME),
    )
    .expect("codes.bin exists after finish");
    let expected: [u8; 16] = [
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // word 0: bit 0 only
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // word 1: dimension 64 only
    ];
    assert_eq!(
        raw,
        expected.to_vec(),
        "the bits dims..64 × words must stay zero: a popcount over them would count garbage"
    );
    assert_eq!(
        read_codes_bin(
            &dir.path()
                .join("segments")
                .join(SEGMENT_ID)
                .join(CODES_FILE_NAME),
            1,
            PADDED_DIMS,
        )
        .expect("the padded code reads back"),
        vec![1_u64, 0],
        "reading back the padded code gives the same single bit, in the two words \
         `words_per_code(65)` declares"
    );
}

#[test]
fn inv_15_attach_quantizes_the_stored_value() {
    // 1e-9 is positive as an `f32` but underflows to +0 once stored in fp16
    // (subnormal minimum 2^-25 ≈ 2.98e-08). The bit must stay clear: a bit the
    // reader never sees would make the rebuild from the stored vectors write
    // another `codes.bin` than the attach did (ADR-0069, decision 6).
    let dir = tempfile::tempdir().expect("temporary directory");
    let mut writer =
        writer(dir.path(), DIMS).expect("the fixture writer starts in a temporary directory");
    writer
        .append(&[1.0e-9, 0.0, -1.0e-9])
        .expect("the underflowing vector");
    writer.finish().expect("the segment is committed");

    let segment_dir = dir.path().join("segments").join(SEGMENT_ID);
    let raw = fs::read(segment_dir.join(CODES_FILE_NAME)).expect("codes.bin exists after finish");
    assert_eq!(
        raw,
        vec![0_u8; 8],
        "both components round to zero in fp16: no bit may be set for them"
    );

    let reader = SegmentReader::open(&segment_dir).expect("the segment opens");
    let stored = reader
        .vector(Iid::new(IID_START))
        .expect("the stored vector");
    assert_eq!(
        reader.codes(Iid::new(IID_START)).expect("the stored code"),
        quantize(&stored).as_slice(),
        "the writer quantizes the bytes it writes, so the code of the \
         reference vector it stores is the code of the attach"
    );
}

#[test]
fn inv_15_truncated_codes_file_is_rejected() {
    let dir = tempfile::tempdir().expect("temporary directory");
    // One code of three dimensions is eight bytes; seven is one short.
    let truncated = &CODES_KNOWN_WORDS[..7];
    let segment_dir = handwritten_segment(
        dir.path(),
        DIMS,
        1,
        &F16_KNOWN_VECTORS[..6],
        Some(truncated),
        Some(FORMAT_VERSION),
    );

    let error = read_codes_bin(&segment_dir.join(CODES_FILE_NAME), 1, DIMS)
        .expect_err("a truncated codes.bin must be refused");
    assert_eq!(
        error.code(),
        ErrorCode::Internal,
        "a truncated derived file describes the state of an epoch: {}",
        error.message()
    );
    assert!(
        error.message().contains("codes.bin"),
        "the message must name the file: {}",
        error.message()
    );

    assert_open_rejected(&segment_dir, ErrorCode::Internal, &["codes.bin"]);
}

#[test]
fn inv_15_oversized_codes_file_is_rejected() {
    let dir = tempfile::tempdir().expect("temporary directory");
    // Two codes where the manifest declares one: the length is exact, not a
    // minimum, so a stale codes.bin from a longer segment cannot be read.
    let segment_dir = handwritten_segment(
        dir.path(),
        DIMS,
        1,
        &F16_KNOWN_VECTORS[..6],
        Some(&CODES_KNOWN_WORDS),
        Some(FORMAT_VERSION),
    );

    assert_open_rejected(&segment_dir, ErrorCode::Internal, &["codes.bin"]);
}

#[test]
fn inv_15_unknown_format_version_refuses_segment_carrying_codes() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let segment_dir = handwritten_segment(
        dir.path(),
        DIMS,
        2,
        &F16_KNOWN_VECTORS,
        Some(&CODES_KNOWN_WORDS),
        Some(999),
    );

    assert_open_rejected(
        &segment_dir,
        ErrorCode::InvalidRequest,
        &["999", &FORMAT_VERSION.to_string()],
    );
}

#[test]
fn inv_15_read_only_open_builds_codes_in_ram_without_writing() {
    let dir = tempfile::tempdir().expect("temporary directory");
    // An index built before QUANTIFIER-BINAIRE, or a restored one: no codes.bin.
    let segment_dir = handwritten_segment(
        dir.path(),
        DIMS,
        2,
        &F16_KNOWN_VECTORS,
        None,
        Some(FORMAT_VERSION),
    );
    let entries_before = listing(&segment_dir);

    let reader = SegmentReader::open(&segment_dir)
        .expect("a segment without codes.bin opens in read-only mode");

    assert_eq!(
        reader.codes(Iid::new(IID_START)).expect("first code"),
        &[1_u64],
        "the missing code is quantized from the reference vector, in RAM"
    );
    assert_eq!(
        reader.codes(Iid::new(IID_START + 1)).expect("second code"),
        &[2_u64],
        "the missing code of the second iid is quantized the same way"
    );
    for iid in [Iid::new(IID_START), Iid::new(IID_START + 1)] {
        assert_eq!(
            reader
                .codes(iid)
                .expect("the code of the ram-built segment"),
            quantize(&reader.vector(iid).expect("the reference vector")),
            "a code built in RAM is the quantization of its reference vector"
        );
    }

    assert!(
        !segment_dir.join(CODES_FILE_NAME).exists(),
        "a read-only open must never write codes.bin into the epoch (ADR-0069, decision 2)"
    );
    assert_eq!(
        listing(&segment_dir),
        entries_before,
        "a read-only open leaves the segment directory untouched, temporary files included"
    );
}

#[test]
fn inv_15_write_open_rebuilds_codes_identical_to_attach() {
    let (_temp, index) = common::temp_index();
    let mut store = common::create_store(&index);
    let segment = common::write_segment(
        &index,
        "seg-1",
        0,
        1,
        &[&[1.0_f32, -2.0, 0.0, 0.0][..], &[0.0_f32, 0.5, -1.0, 0.0]],
    );
    common::commit(
        &mut store,
        common::change(
            vec![segment],
            vec![common::document("D1", 1), common::document("D2", 1)],
            vec![
                common::passage(0, "D1", 0, "seg-1", 1),
                common::passage(1, "D2", 0, "seg-1", 1),
            ],
            Vec::new(),
        ),
    );

    let segment_dir = common::segments_dir(&index).join("seg-1");
    let codes_path = segment_dir.join(CODES_FILE_NAME);
    let written = fs::read(&codes_path).expect("the attach writes codes.bin");
    assert_eq!(
        written.len() as u64,
        codes_len(2, common::DIMS),
        "the codes of the attach have the documented length"
    );

    let epoch_before = store.epoch();
    let generation_before = store.generation();
    let pointer_before = fs::read(common::current_epoch_path(&index))
        .expect("CURRENT_EPOCH is readable before the removal");
    let epochs_before = listing(&index.join("epochs"));
    fs::remove_file(&codes_path).expect("codes.bin is removed, as on an index built before P4");
    drop(store);

    // Opening the index in write mode runs the recovery: codes.bin is rebuilt.
    let reopened = common::open_store(&index);

    let rebuilt = fs::read(&codes_path).expect("the writer open rebuilds codes.bin");
    assert_eq!(
        rebuilt, written,
        "the codes rebuilt from the reference vectors are identical to those written at attach"
    );
    assert_eq!(
        reopened.epoch(),
        epoch_before,
        "rebuilding codes.bin must not create a new epoch (ADR-0069, decision 1)"
    );
    assert_eq!(
        reopened.generation(),
        generation_before,
        "rebuilding codes.bin must not create a new generation"
    );
    assert_eq!(
        fs::read(common::current_epoch_path(&index))
            .expect("CURRENT_EPOCH is readable after the open"),
        pointer_before,
        "the pointer of the epoch must be untouched"
    );
    assert_eq!(
        listing(&index.join("epochs")),
        epochs_before,
        "no epoch directory may appear or disappear"
    );

    let reader = SegmentReader::open(&segment_dir).expect("the rebuilt segment opens");
    for iid in [Iid::new(0), Iid::new(1)] {
        assert_eq!(
            reader.codes(iid).expect("the code of the rebuilt segment"),
            quantize(&reader.vector(iid).expect("the reference vector")),
            "the persisted code of every iid is the quantization of its reference vector"
        );
    }
}

#[test]
fn inv_15_restored_snapshot_rebuilds_identical_codes_on_write_open() {
    let (temp, index) = common::temp_index();
    let exchange = temp.path().join("exchange");
    let mut store = common::create_store(&index);
    let segment = common::write_segment(&index, "seg-1", 0, 1, &[&[1.0_f32, -2.0, 0.0, 0.0][..]]);
    common::commit(
        &mut store,
        common::change(
            vec![segment],
            vec![common::document("D1", 1)],
            vec![common::passage(0, "D1", 0, "seg-1", 1)],
            Vec::new(),
        ),
    );
    let original = fs::read(
        common::segments_dir(&index)
            .join("seg-1")
            .join(CODES_FILE_NAME),
    )
    .expect("the attach writes codes.bin");

    let config =
        IndexConfig::from_json_str(&common::test_config("test", &common::fixture_schema()))
            .unwrap_or_else(|error| panic!("the fixture configuration must be valid: {error}"));
    let report = snapshot::create(&mut store, &config, &exchange)
        .unwrap_or_else(|error| panic!("the snapshot must be taken: {error}"));
    drop(store);

    // Another data root, so the restore creates the index from the snapshot.
    let target = tempfile::tempdir().expect("temporary directory");
    let target_data = target.path().join("data");
    let restored = snapshot::restore(&target_data, &report.directory)
        .unwrap_or_else(|error| panic!("the snapshot must be restored: {error}"));
    let restored_index = target_data.join("indexes").join("test");
    let restored_segment = restored_index
        .join("epochs")
        .join(restored.epoch.get().to_string())
        .join("segments")
        .join("seg-1");
    assert!(
        !report
            .directory
            .join("segments")
            .join("seg-1")
            .join(CODES_FILE_NAME)
            .exists(),
        "a snapshot never carries codes.bin, it is derived (ADR-0064, decision 4)"
    );

    // `snapshot::restore` ends with the write open of `GenerationStore::open`
    // (INV-04), and that write open is the reconstruction of `ADR-0069`,
    // decision 1: the restored index therefore already holds the rebuilt codes,
    // and they must be the bytes the attach wrote.
    let rebuilt = fs::read(restored_segment.join(CODES_FILE_NAME))
        .expect("the write open of the restore rebuilds the restored codes.bin");
    assert_eq!(
        rebuilt, original,
        "a restored index rebuilds the same codes as the attach wrote, from the same vectors"
    );

    // The next write open changes neither the generation nor the epoch: a
    // rebuild is invisible.
    let reopened = common::open_store(&restored_index);
    assert_eq!(
        reopened.generation(),
        restored.generation,
        "the rebuild keeps the restored generation"
    );
    assert_eq!(
        reopened.epoch(),
        restored.epoch,
        "the rebuild keeps the restored epoch"
    );
}

#[test]
fn inv_15_commit_reuses_the_codes_of_unchanged_segments() {
    let (_temp, index) = common::temp_index();
    let mut store = common::create_store(&index);
    let segment = common::write_segment(&index, "seg-1", 0, 1, &[&[1.0_f32, -2.0, 0.0, 0.0][..]]);
    common::commit(
        &mut store,
        common::change(
            vec![segment],
            vec![common::document("D1", 1)],
            vec![common::passage(0, "D1", 0, "seg-1", 1)],
            Vec::new(),
        ),
    );

    // A segment is immutable, so the second snapshot must take its codes from
    // the first one. `codes.bin` is overwritten with the right length and the
    // wrong bits: a commit that read it again would report another code.
    let segment_dir = common::segments_dir(&index).join("seg-1");
    fs::write(segment_dir.join(CODES_FILE_NAME), [0xff_u8; 8])
        .expect("the codes file is overwritten with other bits");

    common::commit(
        &mut store,
        common::change(
            Vec::new(),
            vec![common::document("D1", 2)],
            Vec::new(),
            Vec::new(),
        ),
    );

    let snapshot = store.snapshot();
    assert_eq!(
        snapshot.codes_of("seg-1"),
        Some(&[1_u64][..]),
        "the commit shares the codes of the segment it keeps instead of reading \
         them again"
    );
    assert_eq!(
        snapshot.codes_memory(),
        8,
        "one code of four dimensions is one word, and the snapshot reports it \
         whatever its origin"
    );
}

#[test]
fn inv_15_codes_bytes_are_identical_across_runs() {
    let first = tempfile::tempdir().expect("temporary directory");
    let second = tempfile::tempdir().expect("temporary directory");

    let mut written = Vec::new();
    for root in [first.path(), second.path()] {
        let mut writer =
            writer(root, DIMS).expect("the fixture writer starts in a temporary directory");
        writer.append(&[1.0, -2.0, 0.0]).expect("first vector");
        writer.append(&[0.0, 0.5, -1.0]).expect("second vector");
        writer.finish().expect("the segment is committed");
        written.push(
            fs::read(root.join("segments").join(SEGMENT_ID).join(CODES_FILE_NAME))
                .expect("codes.bin exists after finish"),
        );
    }

    assert_eq!(
        written[0], written[1],
        "codes.bin must be deterministic: no timestamp, no run-dependent order"
    );
}
