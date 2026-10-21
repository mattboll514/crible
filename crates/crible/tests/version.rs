// SPDX-License-Identifier: Apache-2.0

//! Checks the contract of the `--version` flag.

use std::process::Command;

#[test]
fn version_flag_prints_package_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_crible"))
        .arg("--version")
        .output()
        .expect("run crible --version");
    assert!(output.status.success(), "crible --version failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "unexpected output: {stdout}"
    );
}
