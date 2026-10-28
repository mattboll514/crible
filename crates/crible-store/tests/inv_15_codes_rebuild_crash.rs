// SPDX-License-Identifier: Apache-2.0

//! INV-15 — a rebuild of `codes.bin` interrupted before its rename leaves no
//! residue once the index is reopened.
//!
//! `ADR-0069` announces a crash test of the reconstruction: a rebuild writes
//! `tmp-codes.bin`, fsyncs it, renames it, then fsyncs the segment directory. A
//! process death between the write and the rename leaves the temporary file
//! inside the segment directory, which the sweep of `tmp-*` entries of
//! `segments/` does not enter. The recovery of the epoch removes it for every
//! referenced segment, so the next open leaves neither the temporary file nor a
//! missing `codes.bin`.
//!
//! The crash is simulated by panicking inside `rebuild_codes` at the failpoint
//! `codes_rebuild_before_rename`; the store is then reopened exactly as a
//! restart does. Run it with:
//!
//! ```text
//! cargo test -p crible-store --features failpoints --test inv_15_codes_rebuild_crash
//! ```

mod common;

#[cfg(feature = "failpoints")]
use std::fs;

#[cfg(feature = "failpoints")]
use crible_store::codes::{CODES_FILE_NAME, codes_len};

/// Step of `codes::rebuild_codes` at which the rebuild stops, once the
/// temporary file is written and before its rename.
#[cfg(feature = "failpoints")]
const FAILPOINT: &str = "codes_rebuild_before_rename";

/// `ADR-0069`: a rebuild interrupted after its write and before its rename is
/// cleaned by the next open, and the codes are rebuilt from the vectors.
#[cfg(feature = "failpoints")]
#[test]
fn inv_15_interrupted_codes_rebuild_leaves_no_temporary_file_after_recovery() {
    let (_temp, index) = common::temp_index();
    let mut store = common::create_store(&index);
    let segment = common::write_segment(
        &index,
        "seg-a",
        0,
        1,
        &[&[0.0, 1.0, 0.0, 0.0], &[1.0, 0.0, 0.0, 0.0]],
    );
    let change = common::change(
        vec![segment],
        vec![common::document("doc-a", 1)],
        vec![
            common::passage(0, "doc-a", 0, "seg-a", 1),
            common::passage(1, "doc-a", 0, "seg-a", 1),
        ],
        Vec::new(),
    );
    common::commit(&mut store, change);
    drop(store);

    // A restored snapshot carries no `codes.bin` (ADR-0064, decision 4), so the
    // next writer open rebuilds it.
    let segment_dir = common::segments_dir(&index).join("seg-a");
    let codes_path = segment_dir.join(CODES_FILE_NAME);
    let temporary = segment_dir.join(format!("tmp-{CODES_FILE_NAME}"));
    fs::remove_file(&codes_path).expect("the derived codes file is removed");

    let armed = common::FailpointGuard::arm(FAILPOINT);
    let outcome =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| common::open_store(&index)));
    drop(armed);
    assert!(
        outcome.is_err(),
        "the armed failpoint must stop the rebuild before its rename"
    );
    assert!(
        temporary.is_file(),
        "an interrupted rebuild must leave its temporary file behind"
    );

    let reopened = common::open_store(&index);
    assert!(
        !temporary.exists(),
        "the recovery of the epoch must remove {}",
        temporary.display()
    );
    assert!(
        codes_path.is_file(),
        "the same open must persist the rebuilt codes"
    );
    assert_eq!(
        fs::metadata(&codes_path)
            .expect("the rebuilt codes file is readable")
            .len(),
        codes_len(2, common::DIMS),
        "the rebuilt file must hold two codes of {} dimensions",
        common::DIMS
    );
    let snapshot = reopened.snapshot();
    let codes = snapshot
        .codes_of("seg-a")
        .expect("the snapshot holds the codes of its segment");
    assert_eq!(
        codes.len(),
        2,
        "the rebuilt array must hold one code per passage"
    );
}
