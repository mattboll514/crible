// SPDX-License-Identifier: Apache-2.0

//! `crible key generate`: two lines on stdout, the key then its digest.
//!
//! Acceptance criterion of `AUTHENTIFIER-CLES`
//! (`docs/phases/P6-serveur-et-api.md`): the command prints the key and its
//! hash. ADR-0100, decision 3: no option, `key: <clé>` then `sha256: <haché>`,
//! 64 lowercase hexadecimal characters each, and nothing else on stdout.
//!
//! The digest is recomputed here with `sha2` rather than with the helper of the
//! server, so the test checks the convention of ADR-0100 and not a function
//! against itself.

use std::process::Command;

use sha2::{Digest, Sha256};

/// Path of the binary built for this integration test.
fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_crible")
}

/// Lowercase hexadecimal of a digest.
fn hex(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The 64 characters of `line`, after `prefix`, when they are lowercase
/// hexadecimal.
fn value_of(line: &str, prefix: &str) -> Option<String> {
    let value = line.strip_prefix(prefix)?;
    let bytes = value.as_bytes();
    let hexadecimal = bytes.len() == 64
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte));
    hexadecimal.then(|| value.to_owned())
}

#[test]
fn key_generate_prints_the_key_then_its_digest() {
    let output = Command::new(binary())
        .args(["key", "generate"])
        .output()
        .expect("run crible key generate");
    assert!(
        output.status.success(),
        "crible key generate failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "exactly two lines, got {stdout:?}");

    let key = value_of(lines[0], "key: ").unwrap_or_else(|| {
        panic!("the first line holds a key of 64 lowercase characters: {stdout:?}")
    });
    let digest = value_of(lines[1], "sha256: ").unwrap_or_else(|| {
        panic!("the second line holds a digest of 64 lowercase characters: {stdout:?}")
    });
    assert_eq!(
        digest,
        hex(&Sha256::digest(key.as_bytes())),
        "the printed digest is the SHA-256 of the 64 characters of the key"
    );
}

#[test]
fn two_runs_print_two_keys() {
    // ADR-0100, decision 2: 32 bytes drawn from the system source, so a second
    // call almost surely prints another key. A fixed key would be a defect.
    let first = Command::new(binary())
        .args(["key", "generate"])
        .output()
        .expect("run crible key generate");
    let second = Command::new(binary())
        .args(["key", "generate"])
        .output()
        .expect("run crible key generate");
    assert_ne!(first.stdout, second.stdout);
}

#[test]
fn key_generate_takes_no_option() {
    // ADR-0100, decision 3: the command has no option, and refuses an unknown
    // one instead of ignoring it.
    let output = Command::new(binary())
        .args(["key", "generate", "--length", "16"])
        .output()
        .expect("run crible key generate --length 16");
    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "a refused option prints no key: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
}
