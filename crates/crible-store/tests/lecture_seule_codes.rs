// SPDX-License-Identifier: Apache-2.0

//! A read opening of the store never writes a derived **segment** file under the
//! epoch.
//!
//! `docs/02-architecture.md:143` and `ADR-0069`, decision 2 require a read-only
//! open to build a missing `codes.bin` in RAM and to write nothing; `ADR-0056`
//! states that a read no longer modifies the disk. `GenerationStore::open` is
//! the write open: it rebuilds every missing `codes.bin`
//! (`ADR-0069`, decision 1). `GenerationStore::open_read_only` skips that
//! rebuild, which `crible search` and `crible inspect` use. It still rewrites a
//! missing manifest, as recovery always did; that file belongs to the
//! generation, not to a segment.
//!
//! The test removes `codes.bin` of a committed segment, opens the index in read
//! mode, and checks three things: the file is still absent, the snapshot serves
//! the same codes as the index that had the file, and a later write open
//! persists it without changing a code.

mod common;

use std::fs;

use crible_store::codes::CODES_FILE_NAME;
use crible_store::generation::GenerationStore;

#[test]
fn read_only_open_quantizes_missing_codes_without_writing_them() {
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
    let written = store
        .snapshot()
        .codes_of("seg-a")
        .expect("the snapshot holds the codes it just loaded")
        .to_vec();
    drop(store);

    let codes_path = common::segments_dir(&index)
        .join("seg-a")
        .join(CODES_FILE_NAME);
    fs::remove_file(&codes_path).expect("the derived codes file is removed");
    assert!(
        !codes_path.exists(),
        "the fixture must start without a codes.bin"
    );

    let read = GenerationStore::open_read_only(&index, &common::fixture_schema())
        .expect("the index opens for a read");
    assert!(
        !codes_path.exists(),
        "a read opening must not persist the missing codes"
    );
    assert_eq!(
        read.snapshot()
            .codes_of("seg-a")
            .expect("the read snapshot holds the codes of the segment"),
        written.as_slice(),
        "the codes quantized in RAM must be the codes the file held"
    );

    let writer = common::open_store(&index);
    assert!(
        codes_path.is_file(),
        "the write open must rebuild the missing codes"
    );
    assert_eq!(
        writer
            .snapshot()
            .codes_of("seg-a")
            .expect("the write snapshot holds the codes of the segment"),
        written.as_slice(),
        "the rebuilt file must not change a single code"
    );
}
