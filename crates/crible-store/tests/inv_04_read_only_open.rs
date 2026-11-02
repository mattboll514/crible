// SPDX-License-Identifier: Apache-2.0

//! INV-04 — an open for **read** writes nothing under the index directory.
//!
//! References: `docs/02-architecture.md` ("Organisation sur disque", "Reprise au
//! démarrage"), `docs/08-invariants-et-tests.md` (INV-04), `ADR-0116` (decision 1)
//! and the `OUVRIR-EN-LECTURE-SANS-ECRIRE` task of `docs/phases/P8-docker-et-deploiement.md`.
//!
//! The invariant says the state after a restart is the last committed
//! generation, and everything else is rebuilt or cleaned. ADR-0116 fixes
//! **when**: the reconstructions and the cleanup belong to the open for
//! **write**; an open for read leaves the index directory untouched, with the
//! single exception of the files SQLite manages itself in WAL mode (`-wal` and
//! `-shm`, ADR-0038).
//!
//! The fixture is an index that a crash left "dirty":
//!
//! - a `tmp-*` file at the root of the index, as an interrupted creation
//!   leaves one;
//! - a segment directory that the `segments` table does not reference, as a
//!   segment written and renamed just before a crash leaves one;
//! - a missing manifest, as a crash after the SQLite commit and before
//!   `manifests/<G>.json` leaves one.
//!
//! Before ADR-0116, decision 1, `recover` and `ensure_manifest` removed and
//! rewrote all three **in both modes**; the tests below pin the split the ADR
//! decides: the repairs belong to the open for write alone.
//!
//! ```text
//! cargo test -p crible-store --test inv_04_read_only_open
//! ```

// The fixtures of this file call `panic!` outside the `#[test]` functions, as
// the other integration tests do; see `tests/common/mod.rs`.
#![allow(clippy::panic)]

mod common;

use std::fs;
use std::path::Path;

use crible_core::ids::Generation;
use crible_store::generation::GenerationStore;

/// Segment directory the `segments` table does not reference.
const ORPHAN_SEGMENT: &str = "seg-orphan";

/// Leftover of an interrupted creation, at the root of the index.
const TEMPORARY_FILE: &str = "tmp-CURRENT_EPOCH";

/// One entry of the recursive inventory of an index directory.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    /// Path relative to the root of the index, with `/` separators.
    path: String,
    /// Size in bytes: `0` for a directory.
    size: u64,
    /// `blake3:<hex>` of the bytes of a file; `<directory>` for a directory.
    digest: String,
}

/// Recursive inventory of `root`: every path, its size and its blake3, sorted
/// by path.
///
/// The files SQLite manages itself in WAL mode are left out: ADR-0116 makes
/// them the only exception to "a read writes nothing".
fn inventory(root: &Path) -> Vec<Entry> {
    let mut entries = Vec::new();
    collect(root, root, &mut entries);
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    entries
}

/// Recursive body of [`inventory`].
fn collect(root: &Path, directory: &Path, entries: &mut Vec<Entry>) {
    let listing = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("{} must be listable: {error}", directory.display()));
    for entry in listing {
        let entry = entry.unwrap_or_else(|error| {
            panic!(
                "an entry of {} must be readable: {error}",
                directory.display()
            )
        });
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let relative = path
            .strip_prefix(root)
            .unwrap_or_else(|error| {
                panic!(
                    "{} must be under {}: {error}",
                    path.display(),
                    root.display()
                )
            })
            .to_string_lossy()
            .replace('\\', "/");
        if is_wal_file(&name) {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)
            .unwrap_or_else(|error| panic!("{} must be inspectable: {error}", path.display()));
        if metadata.is_dir() {
            entries.push(Entry {
                path: relative,
                size: 0,
                digest: "<directory>".to_owned(),
            });
            collect(root, &path, entries);
            continue;
        }
        let bytes = fs::read(&path)
            .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
        entries.push(Entry {
            path: relative,
            size: metadata.len(),
            digest: format!("blake3:{}", blake3::hash(&bytes).to_hex()),
        });
    }
}

/// True for the files SQLite manages itself in WAL mode (ADR-0038).
fn is_wal_file(name: &str) -> bool {
    name.ends_with("-wal") || name.ends_with("-shm")
}

/// Builds an index whose committed generation is 1, then dirties it the way an
/// interrupted creation or commit leaves it.
///
/// The three leftovers are the ones `recover` removes and `ensure_manifest`
/// rewrites: a `tmp-*` file at the root, a segment directory absent from the
/// `segments` table, and a missing manifest.
fn dirty_index(index: &Path) {
    let mut store = common::create_store(index);
    let segment = common::write_segment(index, "seg-a", 0, 1, &[&[1.0, 0.0, 0.0, 0.0]]);
    common::commit(
        &mut store,
        common::change(
            vec![segment],
            vec![common::document("doc-a", 1)],
            vec![common::passage(0, "doc-a", 0, "seg-a", 1)],
            Vec::new(),
        ),
    );
    drop(store);

    // An interrupted creation writes `tmp-CURRENT_EPOCH` and renames it; a
    // crash before the rename leaves the temporary file behind.
    fs::write(index.join(TEMPORARY_FILE), b"1\n")
        .unwrap_or_else(|error| panic!("the tmp-* fixture file must be writable: {error}"));

    // A segment written and renamed, but never referenced by SQLite because the
    // commit that would have named it never happened.
    common::write_segment(index, ORPHAN_SEGMENT, 1, 2, &[&[0.0, 1.0, 0.0, 0.0]]);

    // A crash after the SQLite commit and before the manifest leaves the
    // derived view of generation 1 missing.
    let manifest = common::manifest_path(index, Generation::new(1));
    fs::remove_file(&manifest)
        .unwrap_or_else(|error| panic!("the manifest fixture must be removable: {error}"));
}

/// Asserts that the index still holds exactly the three leftovers of
/// [`dirty_index`], so a read that cleaned one of them is caught by name.
fn assert_dirty(index: &Path, after: &str) {
    assert!(
        index.join(TEMPORARY_FILE).exists(),
        "{after} removed the tmp-* file at the root of the index"
    );
    assert!(
        common::segments_dir(index).join(ORPHAN_SEGMENT).exists(),
        "{after} removed the segment SQLite does not reference"
    );
    assert!(
        !common::manifest_path(index, Generation::new(1)).exists(),
        "{after} rewrote the missing manifest of generation 1"
    );
}

/// ADR-0116, decision 1: opening the index for read leaves it identical byte
/// for byte, `-wal` and `-shm` excepted, and still publishes the committed
/// generation.
#[test]
fn inv_04_read_only_open_leaves_the_index_directory_untouched() {
    // The failpoint variable is process-global: every committing test of this
    // binary takes the same guard, armed or not.
    let _serial = common::SerialGuard::acquire();
    let (_temp, index) = common::temp_index();
    dirty_index(&index);

    let before = inventory(&index);
    let store = GenerationStore::open_read_only(&index, &common::fixture_schema()).unwrap_or_else(
        |error| {
            panic!(
                "a read-only open must succeed on an index whose leftovers are \
                 not repaired: {error}"
            )
        },
    );
    // The read is not vacuous: it still publishes the committed generation,
    // its segment and its live iid.
    common::assert_view(&store.snapshot(), 1, &["seg-a"], &[0]);
    drop(store);

    let after = inventory(&index);
    assert_dirty(&index, "the read-only open");
    assert_eq!(
        after, before,
        "a read-only open must not touch the index directory (ADR-0116, decision 1): \
         the inventory of paths, sizes and blake3 must be identical, -wal and -shm \
         excepted"
    );
}

/// The repair still happens, at the next open for **write**: the `tmp-*` file
/// and the unreferenced segment disappear, and the missing manifest is written.
#[test]
fn opening_for_write_after_a_read_only_open_repairs_the_index() {
    // The failpoint variable is process-global: every committing test of this
    // binary takes the same guard, armed or not.
    let _serial = common::SerialGuard::acquire();
    let (_temp, index) = common::temp_index();
    dirty_index(&index);

    {
        let read = GenerationStore::open_read_only(&index, &common::fixture_schema())
            .unwrap_or_else(|error| panic!("the read-only open must succeed: {error}"));
        common::assert_view(&read.snapshot(), 1, &["seg-a"], &[0]);
    }

    let written = common::open_store(&index);
    common::assert_view(&written.snapshot(), 1, &["seg-a"], &[0]);
    drop(written);

    assert!(
        !index.join(TEMPORARY_FILE).exists(),
        "the write open must remove the tmp-* file at the root of the index"
    );
    common::assert_segment_directories(&index, common::EPOCH, &["seg-a"]);
    common::assert_manifest_describes(&index, 1, &["seg-a"]);
}
