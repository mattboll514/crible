// SPDX-License-Identifier: Apache-2.0

//! Acceptance tests of `crible_store::segment`: a segment written by the writer
//! is read back with the same values, a corrupted or truncated vector file is
//! refused at open, an id outside `[iid_start, iid_start + count)` is refused,
//! and the segment directory only appears under its final name once committed.
//!
//! References: `docs/02-architecture.md` ("Organisation sur disque": `segments/
//! <segment_id>/segment.json` and `vectors.f16`, atomic write through
//! `tmp-<id>`, fsync, rename) and the `ECRIRE-SEGMENTS-VECTEURS` task of
//! `docs/phases/P1-stockage-et-recherche-exacte.md`.
//!
//! Assumed API: see the module comment of `inv_15_segment_vectors.rs`, which
//! documents the exact names and signatures `@dev` must provide.

use std::fs;
use std::path::{Path, PathBuf};

use crible_core::config::ReferencePrecision;
use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::{Generation, Iid};
use crible_store::segment::{SegmentReader, SegmentSpec, SegmentWriter};

const SEGMENT_ID: &str = "seg-roundtrip";

fn spec(segment_id: &str, iid_start: u32, dims: u32) -> SegmentSpec {
    SegmentSpec {
        segment_id: segment_id.to_owned(),
        iid_start: Iid::new(iid_start),
        dims,
        generation: Generation::new(12),
        batch_id: "batch-2026-10-21".to_owned(),
        precision: ReferencePrecision::Fp16,
    }
}

/// Writes a four-dimension segment of two vectors and returns the segments
/// directory. The values are exactly representable in fp16, so the round-trip
/// is bit-exact.
fn write_known_segment(root: &Path) -> PathBuf {
    let segments = root.join("segments");
    fs::create_dir_all(&segments)
        .unwrap_or_else(|error| panic!("the segments directory is created: {error}"));
    let mut writer = SegmentWriter::create(&segments, spec(SEGMENT_ID, 100, 4))
        .unwrap_or_else(|error| panic!("the writer starts in a temporary directory: {error}"));
    assert_eq!(
        writer
            .append(&[0.0, 1.0, -2.0, 0.5])
            .unwrap_or_else(|error| panic!("first vector: {error}")),
        Iid::new(100)
    );
    assert_eq!(
        writer
            .append(&[16.0, -0.5, 3.0, 1024.0])
            .unwrap_or_else(|error| panic!("second vector: {error}")),
        Iid::new(101)
    );
    writer
        .finish()
        .unwrap_or_else(|error| panic!("the segment is committed: {error}"));
    segments
}

fn segment_dir(segments: &Path) -> PathBuf {
    segments.join(SEGMENT_ID)
}

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

#[test]
fn roundtrip_known_segment_returns_same_vectors_and_meta() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let segments = write_known_segment(dir.path());

    let reader = SegmentReader::open(&segment_dir(&segments)).expect("the segment opens");
    let meta = reader.meta();
    assert_eq!(meta.segment_id, SEGMENT_ID);
    assert_eq!(meta.iid_start, Iid::new(100));
    assert_eq!(meta.count, 2);
    assert_eq!(meta.dims, 4);
    assert_eq!(meta.generation, Generation::new(12));
    assert_eq!(meta.batch_id, "batch-2026-10-21");
    assert_eq!(meta.precision, ReferencePrecision::Fp16);

    assert_eq!(
        reader.vector(Iid::new(100)).expect("first vector"),
        vec![0.0_f32, 1.0, -2.0, 0.5]
    );
    assert_eq!(
        reader.vector(Iid::new(101)).expect("second vector"),
        vec![16.0_f32, -0.5, 3.0, 1024.0]
    );
}

#[test]
fn writer_assigns_contiguous_iids_from_iid_start() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let segments = dir.path().join("segments");
    fs::create_dir_all(&segments).expect("the segments directory is created");

    let mut writer = SegmentWriter::create(&segments, spec(SEGMENT_ID, 7, 1))
        .expect("the writer starts in a temporary directory");
    for step in 0..3_u32 {
        let iid = writer
            .append(&[step as f32])
            .unwrap_or_else(|error| panic!("append {step} must succeed: {error}"));
        assert_eq!(
            iid,
            Iid::new(7 + step),
            "the n-th vector of a segment takes iid_start + n"
        );
    }
    let meta = writer.finish().expect("the segment is committed");
    assert_eq!(meta.count, 3);
    assert_eq!(meta.iid_start, Iid::new(7));
}

#[test]
fn append_with_wrong_dims_is_rejected() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let segments = dir.path().join("segments");
    fs::create_dir_all(&segments).expect("the segments directory is created");

    let mut writer = SegmentWriter::create(&segments, spec(SEGMENT_ID, 0, 4))
        .expect("the writer starts in a temporary directory");
    let error = writer
        .append(&[1.0, 2.0])
        .expect_err("a vector of the wrong length must be refused");
    assert_eq!(
        error.code(),
        ErrorCode::InvalidRequest,
        "wrong error code: {}",
        error.message()
    );

    // The failed append must not have moved the write cursor.
    assert_eq!(
        writer.append(&[1.0, 2.0, 3.0, 4.0]).expect("valid vector"),
        Iid::new(0),
        "a rejected vector must not consume an iid"
    );
}

#[test]
fn corrupted_vectors_byte_is_detected_on_open() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let segments = write_known_segment(dir.path());
    let vectors = segment_dir(&segments).join("vectors.f16");

    let mut bytes = fs::read(&vectors).expect("the vector file is readable");
    bytes[1] ^= 0xFF;
    fs::write(&vectors, &bytes).expect("the corrupted file is written back");

    assert_open_rejected(
        &segment_dir(&segments),
        ErrorCode::Internal,
        &["vectors.f16"],
    );
}

#[test]
fn truncated_vectors_file_is_detected_on_open() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let segments = write_known_segment(dir.path());
    let vectors = segment_dir(&segments).join("vectors.f16");

    let bytes = fs::read(&vectors).expect("the vector file is readable");
    fs::write(&vectors, &bytes[..bytes.len() - 2]).expect("the file is truncated");

    assert_open_rejected(
        &segment_dir(&segments),
        ErrorCode::Internal,
        &["vectors.f16"],
    );
}

#[test]
fn iid_below_segment_range_is_rejected() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let segments = write_known_segment(dir.path());
    let reader = SegmentReader::open(&segment_dir(&segments)).expect("the segment opens");

    assert!(
        !reader.contains(Iid::new(99)),
        "an iid below iid_start is not part of the segment"
    );
    let error = reader
        .vector(Iid::new(99))
        .expect_err("an iid below iid_start must be refused");
    assert_eq!(
        error.code(),
        ErrorCode::Internal,
        "an iid always comes from the engine, a foreign one is a bug (INV-01): {}",
        error.message()
    );
}

#[test]
fn iid_at_end_of_segment_range_is_rejected() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let segments = write_known_segment(dir.path());
    let reader = SegmentReader::open(&segment_dir(&segments)).expect("the segment opens");

    let past_end = Iid::new(100 + 2);
    assert!(
        !reader.contains(past_end),
        "the range is half-open: [iid_start, iid_start + count)"
    );
    let error = reader
        .vector(past_end)
        .expect_err("an iid at iid_start + count must be refused");
    assert_eq!(
        error.code(),
        ErrorCode::Internal,
        "an iid always comes from the engine, a foreign one is a bug (INV-01): {}",
        error.message()
    );
}

#[test]
fn finish_renames_temporary_directory_to_final_segment_id() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let segments = dir.path().join("segments");
    fs::create_dir_all(&segments).expect("the segments directory is created");

    let mut writer = SegmentWriter::create(&segments, spec(SEGMENT_ID, 0, 1))
        .expect("the writer starts in a temporary directory");
    let temporary = segments.join(format!("tmp-{SEGMENT_ID}"));
    assert!(
        temporary.is_dir(),
        "an unfinished segment lives in segments/tmp-<id>"
    );
    assert!(
        !segment_dir(&segments).exists(),
        "the final directory must not exist before the commit point"
    );

    writer.append(&[1.0]).expect("a vector");
    writer.finish().expect("the segment is committed");

    assert!(segment_dir(&segments).is_dir(), "the segment is renamed");
    assert!(
        !temporary.exists(),
        "no temporary directory survives a successful finish"
    );
    let leftovers: Vec<_> = fs::read_dir(&segments)
        .expect("the segments directory is readable")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("tmp-"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "no tmp-* directory may survive the commit, found {leftovers:?}"
    );
}
