// SPDX-License-Identifier: Apache-2.0

//! INV-01 — an internal id never leaves the engine.
//!
//! The invariant is enforced at the type level: `Iid` must not be serializable,
//! so that no response, log record, analytics row or export can carry one.
//!
//! `trybuild` is not a declared test dependency (`docs/07-stack-et-deploiement.md`),
//! so this is a static source check instead of a compile-fail test. It scans the
//! `src/` trees of every crate and rejects two forms:
//!
//! - `#[derive(… Serialize …)]` on the `Iid` declaration;
//! - `impl … Serialize for Iid …`.
//!
//! The check also fails when the `Iid` declaration cannot be found, so a rename
//! cannot silently make it vacuous.

use std::fs;
use std::path::{Path, PathBuf};

/// Directories never scanned, relative name only.
const SKIPPED_DIRS: [&str; 1] = ["target"];

fn workspace_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    match manifest_dir.ancestors().nth(2) {
        Some(root) => root.to_path_buf(),
        None => panic!("cannot locate the workspace root from {manifest_dir:?}"),
    }
}

fn collect_rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let skipped = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| SKIPPED_DIRS.contains(&name));
            if !skipped {
                collect_rust_files(&path, files);
            }
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

/// Reads the production sources: the `src/` trees of every crate.
fn production_sources() -> Vec<(PathBuf, String)> {
    let crates_dir = workspace_root().join("crates");
    let mut files = Vec::new();
    collect_rust_files(&crates_dir, &mut files);

    let mut sources = Vec::new();
    for path in files {
        let in_src_tree = path
            .components()
            .any(|component| component.as_os_str().to_str() == Some("src"));
        if !in_src_tree {
            continue;
        }
        match fs::read_to_string(&path) {
            Ok(text) => sources.push((path, text)),
            Err(error) => panic!("cannot read {path:?}: {error}"),
        }
    }
    sources
}

/// True when the line declares the `Iid` newtype.
fn declares_iid(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.starts_with("//") || trimmed.starts_with('*') {
        return false;
    }
    ["struct Iid(", "struct Iid {", "struct Iid;"]
        .iter()
        .any(|pattern| trimmed.contains(pattern))
}

/// True when the line contains an `impl … Serialize for Iid` (but not `IidView`).
fn implements_serialize_for_iid(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") || trimmed.starts_with('*') || trimmed.starts_with("/*") {
        return false;
    }
    if !line.contains("impl") || !line.contains("Serialize") {
        return false;
    }
    let Some(position) = line.find("for Iid") else {
        return false;
    };
    let rest = &line[position + "for Iid".len()..];
    !rest
        .chars()
        .next()
        .is_some_and(|character| character.is_alphanumeric() || character == '_')
}

/// True when the attributes above the declaration at `index` derive `Serialize`.
fn derives_serialize(lines: &[&str], index: usize) -> bool {
    let mut cursor = index;
    while cursor > 0 {
        cursor -= 1;
        let line = lines[cursor].trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("#[") {
            if line.contains("derive") && line.contains("Serialize") {
                return true;
            }
            continue;
        }
        break;
    }
    false
}

#[test]
fn inv_01_iid_is_declared_in_the_crates() {
    let sources = production_sources();
    assert!(
        sources.len() >= 6,
        "expected at least the sources of the six crates, found {} files",
        sources.len()
    );

    let declarations: Vec<PathBuf> = sources
        .iter()
        .filter(|(_, text)| text.lines().any(declares_iid))
        .map(|(path, _)| path.clone())
        .collect();

    assert_eq!(
        declarations.len(),
        1,
        "expected exactly one `Iid` declaration, found {declarations:?}"
    );
}

#[test]
fn inv_01_iid_does_not_implement_serialize() {
    let mut violations = Vec::new();

    for (path, text) in production_sources() {
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            let serializable = implements_serialize_for_iid(line)
                || (declares_iid(line) && derives_serialize(&lines, index));
            if serializable {
                violations.push(format!("{path:?}:{}: {}", index + 1, line.trim()));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "INV-01 violated: `Iid` must not be serializable, found:\n{}",
        violations.join("\n")
    );
}

#[test]
fn inv_01_detector_flags_a_serializable_iid() {
    // Guards the check itself: a broken detector would let INV-01 pass silently.
    let derived = [
        "#[derive(Debug, Serialize)]",
        "pub struct Iid(pub(crate) u32);",
    ];
    assert!(declares_iid(derived[1]));
    assert!(derives_serialize(&derived, 1));

    assert!(implements_serialize_for_iid("impl Serialize for Iid {"));
    assert!(implements_serialize_for_iid(
        "impl serde::Serialize for Iid {"
    ));
    assert!(implements_serialize_for_iid("impl<'a> Serialize for Iid {"));

    assert!(!implements_serialize_for_iid(
        "impl Serialize for IidView {"
    ));
    assert!(!implements_serialize_for_iid("impl Serialize for DocId {"));
    assert!(!implements_serialize_for_iid("// impl Serialize for Iid {"));

    let not_derived = [
        "#[derive(Debug, Clone, Copy)]",
        "pub struct Iid(pub(crate) u32);",
    ];
    assert!(!derives_serialize(&not_derived, 1));
}
