// SPDX-License-Identifier: Apache-2.0

//! Command-line entry point of the Crible engine.

use clap::Parser;

/// Command-line interface of the Crible engine.
#[derive(Debug, Parser)]
#[command(
    name = "crible",
    version,
    about = "Dense and BM25 search engine for structured data"
)]
struct Cli {}

fn main() {
    // Clap handles `--help` and `--version` and exits on invalid input.
    let _ = Cli::parse();
}
