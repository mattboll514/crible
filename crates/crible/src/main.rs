// SPDX-License-Identifier: Apache-2.0

//! Command-line entry point of the Crible engine.
//!
//! The binary drives the store and the search of P1: create an index, attach a
//! batch, search by a provided vector, inspect the state. Every command prints
//! its result on stdout and reports a failure on stderr as
//! `<code>: <message>`, with the [`ErrorCode`](crible_core::error::ErrorCode)
//! of `docs/05-api.md`, so a script reads the category without parsing prose.

mod commands;
mod vector;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use crible_core::error::CribleError;

/// Command-line interface of the Crible engine.
#[derive(Debug, Parser)]
#[command(
    name = "crible",
    version,
    about = "Dense and BM25 search engine for structured data"
)]
struct Cli {
    /// Command to run.
    #[command(subcommand)]
    command: Command,
}

/// Commands of the binary.
#[derive(Debug, Subcommand)]
enum Command {
    /// Manage the indexes of an instance.
    Index {
        /// Index operation to run.
        #[command(subcommand)]
        command: IndexCommand,
    },
    /// Attach a batch to an index.
    Attach {
        /// Root data directory (`<data>/indexes/<index>`).
        #[arg(long)]
        data: PathBuf,
        /// Root models directory (`[paths] models` of `crible.toml`).
        #[arg(long)]
        models: PathBuf,
        /// Index name, as declared in its `index.json`.
        #[arg(long)]
        index: String,
        /// Batch directory holding `manifest.json` and its Parquet files.
        batch: PathBuf,
    },
    /// Search an index with a provided vector.
    Search {
        /// Root data directory (`<data>/indexes/<index>`).
        #[arg(long)]
        data: PathBuf,
        /// Index name, as declared in its `index.json`.
        #[arg(long)]
        index: String,
        /// NumPy `.npy` or JSON file holding the query vector.
        #[arg(long)]
        vector_file: PathBuf,
        /// Filter as a JSON object, in the grammar of `docs/05-api.md`.
        #[arg(long)]
        filter: Option<String>,
        /// Number of results, between 1 and 1000.
        #[arg(long, default_value_t = 10)]
        k: usize,
    },
    /// Report the state of one index or of every index of an instance.
    Inspect {
        /// Root data directory (`<data>/indexes/<index>`).
        #[arg(long)]
        data: PathBuf,
        /// Index name; every index when omitted.
        #[arg(long)]
        index: Option<String>,
    },
}

/// Operations on indexes.
#[derive(Debug, Subcommand)]
enum IndexCommand {
    /// Create the index declared by a configuration file.
    Create {
        /// Root data directory (`<data>/indexes/<index>`).
        #[arg(long)]
        data: PathBuf,
        /// Index configuration (`index.json`) to copy into the index.
        #[arg(long)]
        config: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            report(&error);
            ExitCode::FAILURE
        }
    }
}

/// Runs one command, mapping every engine error to the printed report.
fn run(cli: Cli) -> Result<(), CribleError> {
    match cli.command {
        Command::Index { command } => match command {
            IndexCommand::Create { data, config } => commands::create_index(&data, &config),
        },
        Command::Attach {
            data,
            models,
            index,
            batch,
        } => commands::attach_batch(&data, &models, &index, &batch),
        Command::Search {
            data,
            index,
            vector_file,
            filter,
            k,
        } => commands::search(&data, &index, &vector_file, filter.as_deref(), k),
        Command::Inspect { data, index } => commands::inspect(&data, index.as_deref()),
    }
}

/// Prints a failure as `<code>: <message>` on stderr.
///
/// The category is the wire name of `docs/05-api.md` (`invalid_request`,
/// `immutable_setting`, …), so a caller branches on it without parsing the
/// message, exactly as it would with the HTTP body of the API.
fn report(error: &CribleError) {
    eprintln!("{}: {}", error.code().as_str(), error.message());
}
