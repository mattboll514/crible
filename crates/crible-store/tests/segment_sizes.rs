// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixtures of this file are helpers.
#![allow(clippy::expect_used, clippy::panic)]

//! The two sizes of a segment that `live_bytes` sums, and what a compaction
//! does to them (ADR-0152, decisions 2, 3 and 4).
//!
//! `SegmentMeta::vectors_bytes` and `SegmentMeta::manifest_bytes` are computed
//! from the metadata alone. The first test is the proof that the computation
//! gives the bytes of the files: it may `stat`, the code under test may not.
//!
//! | Test | Criterion of the task |
//! |---|---|
//! | `segment_sizes_match_the_files_on_disk` | the computed size of each segment equals the size of its `segment.json` and `vectors.*` |
//! | `compaction_keeps_the_vector_bytes_and_drops_the_merged_manifests` | a compaction leaves the sum of the vectors unchanged and removes the `segment.json` of the merged segments (ADR-0152, decision 4) |
//!
//! ```text
//! cargo test -p crible-store --test segment_sizes
//! ```

mod common;

use std::fs;
use std::path::Path;

use crible_core::config::{IndexConfig, ReferencePrecision};
use crible_core::ids::Epoch;
use crible_store::generation::GenerationStore;
use crible_store::segment::SegmentMeta;

/// Writes the fixture configuration and creates the index from it.
fn create_index(index: &Path, raw: &str) -> GenerationStore {
    let schema = common::fixture_schema();
    crible_store::generation::write_index_config(index, raw)
        .expect("the fixture index configuration is writable");
    GenerationStore::create(index, Epoch::new(common::EPOCH), &schema)
        .expect("the fixture index is creatable")
}

/// Name of the vector file of a segment of `precision`.
fn vector_file_name(precision: ReferencePrecision) -> &'static str {
    match precision {
        ReferencePrecision::Fp16 => "vectors.f16",
        ReferencePrecision::Fp32 => "vectors.f32",
    }
}

/// Asserts that the two computed sizes of `segment` are the sizes of its files.
fn assert_sizes_match_files(index: &Path, segment: &SegmentMeta) {
    let directory = common::segments_dir(index).join(&segment.segment_id);
    let vectors_path = directory.join(vector_file_name(segment.precision));
    let vectors = fs::metadata(&vectors_path)
        .unwrap_or_else(|error| panic!("{} is statable: {error}", vectors_path.display()))
        .len();
    assert_eq!(
        segment
            .vectors_bytes()
            .expect("the byte length is computable"),
        vectors,
        "the computed length of {} must be the length of the file",
        vectors_path.display()
    );

    let manifest_path = directory.join("segment.json");
    let manifest = fs::metadata(&manifest_path)
        .unwrap_or_else(|error| panic!("{} is statable: {error}", manifest_path.display()))
        .len();
    assert_eq!(
        segment
            .manifest_bytes()
            .expect("the manifest length is computable"),
        manifest,
        "the computed length of {} must be the length of the file, because the \
         same canonical serialisation writes it (ADR-0152, decision 3)",
        manifest_path.display()
    );
}

/// `vectors_bytes` and `manifest_bytes` of a committed segment equal the sizes
/// of its files on disk.
///
/// The test is the only place that reads the directory for these sizes: the
/// route computes them from the published state (ADR-0152, decision 3).
#[test]
fn segment_sizes_match_the_files_on_disk() {
    let (_temp, index) = common::temp_index();
    let raw = common::test_config("sizes", &common::fixture_schema());
    let mut store = create_index(&index, &raw);

    let segment = common::write_segment(
        &index,
        "seg-a",
        0,
        1,
        &[
            &[0.0, 1.0, 0.0, 0.0],
            &[1.0, 0.0, 0.0, 0.0],
            &[0.0, 0.0, 1.0, 0.0],
        ],
    );
    common::commit(
        &mut store,
        common::change(
            vec![segment],
            vec![common::document("doc-a", 1), common::document("doc-b", 1)],
            vec![
                common::passage(0, "doc-a", 0, "seg-a", 1),
                common::passage(1, "doc-a", 1, "seg-a", 1),
                common::passage(2, "doc-b", 0, "seg-a", 1),
            ],
            Vec::new(),
        ),
    );

    let snapshot = store.snapshot();
    assert_eq!(
        snapshot.segments().len(),
        1,
        "the fixture publishes exactly one segment"
    );
    let segment = &snapshot.segments()[0];
    assert_eq!(
        segment
            .vectors_bytes()
            .expect("the byte length is computable"),
        3 * 4 * 2,
        "three fp16 vectors of four dimensions hold 24 bytes"
    );
    assert_sizes_match_files(&index, segment);
}

/// A compaction keeps the live passages with their dimension and their
/// precision, so the sum of the vector bytes does not change; each merged
/// segment takes its `segment.json` out of the sum (ADR-0152, decision 4).
#[test]
fn compaction_keeps_the_vector_bytes_and_drops_the_merged_manifests() {
    let (_temp, index) = common::temp_index();
    let raw = common::test_config("sizes", &common::fixture_schema());
    let config = IndexConfig::from_json_str(&raw)
        .unwrap_or_else(|error| panic!("the fixture configuration is valid: {error}"));
    let mut store = create_index(&index, &raw);

    // Generation 1: four live passages, no tombstone, in seg-a.
    let first = common::write_segment(
        &index,
        "seg-a",
        0,
        1,
        &[
            &[0.0, 1.0, 0.0, 0.0],
            &[1.0, 0.0, 0.0, 0.0],
            &[0.0, 0.0, 1.0, 0.0],
            &[0.0, 0.0, 0.0, 1.0],
        ],
    );
    common::commit(
        &mut store,
        common::change(
            vec![first],
            vec![common::document("doc-a", 1), common::document("doc-b", 1)],
            vec![
                common::passage(0, "doc-a", 0, "seg-a", 1),
                common::passage(1, "doc-a", 1, "seg-a", 1),
                common::passage(2, "doc-b", 0, "seg-a", 1),
                common::passage(3, "doc-b", 1, "seg-a", 1),
            ],
            Vec::new(),
        ),
    );

    // Generation 2: two live passages, no tombstone, in seg-b.
    let second = common::write_segment(
        &index,
        "seg-b",
        4,
        2,
        &[&[0.5, 0.5, 0.5, 0.5], &[0.5, 0.5, 0.5, 0.5]],
    );
    common::commit(
        &mut store,
        common::change(
            vec![second],
            vec![common::document("doc-c", 2)],
            vec![
                common::passage(4, "doc-c", 0, "seg-b", 2),
                common::passage(5, "doc-c", 1, "seg-b", 2),
            ],
            Vec::new(),
        ),
    );

    let before = store.snapshot();
    assert_eq!(
        before.segments().len(),
        2,
        "the fixture publishes two segments before the compaction"
    );
    let (vectors_before, manifests_before) = sums(before.segments());
    assert_eq!(
        vectors_before,
        (4 + 2) * 4 * 2,
        "six fp16 vectors of four dimensions hold 48 bytes"
    );
    assert!(
        manifests_before > 0,
        "each published segment carries its segment.json"
    );

    let report = crible_store::compact::compact(&mut store, &config)
        .unwrap_or_else(|error| panic!("the fixture compaction succeeds: {error}"));
    assert_eq!(
        report.removed_passages, 0,
        "the fixture holds no tombstone, so the compaction removes nothing"
    );

    let after = store.snapshot();
    assert_eq!(
        after.segments().len(),
        1,
        "six passages fit in one segment of at most 1 000 000 passages"
    );
    let (vectors_after, manifests_after) = sums(after.segments());
    assert_eq!(
        vectors_after, vectors_before,
        "the live passages keep their dimension and their precision, so the \
         vector bytes are invariant (ADR-0152, decision 4)"
    );
    assert!(
        manifests_after < manifests_before,
        "the compaction merged two segments into one, so one segment.json left \
         the sum: {manifests_after} < {manifests_before}"
    );
    assert_eq!(
        manifests_after,
        after.segments()[0]
            .manifest_bytes()
            .expect("the manifest length is computable"),
        "the manifest term follows the number of published segments, and only it"
    );
}

/// Sum of the vector bytes and of the manifest bytes of `segments`.
fn sums(segments: &[SegmentMeta]) -> (u64, u64) {
    let vectors = segments
        .iter()
        .map(|segment| {
            segment
                .vectors_bytes()
                .unwrap_or_else(|error| panic!("the vector length is computable: {error}"))
        })
        .sum();
    (vectors, manifests_total(segments))
}

/// Sum of the `segment.json` lengths of `segments`.
fn manifests_total(segments: &[SegmentMeta]) -> u64 {
    segments
        .iter()
        .map(|segment| {
            segment
                .manifest_bytes()
                .unwrap_or_else(|error| panic!("the manifest length is computable: {error}"))
        })
        .sum()
}
