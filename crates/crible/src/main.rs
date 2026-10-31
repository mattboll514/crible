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
use tracing_subscriber::util::SubscriberInitExt;

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
    /// Ingest a JSONL file of documents, encoding the passages that carry no
    /// vector (ADR-0055).
    Ingest {
        /// Root data directory (`<data>/indexes/<index>`).
        #[arg(long)]
        data: PathBuf,
        /// Root models directory (`[paths] models` of `crible.toml`).
        #[arg(long)]
        models: PathBuf,
        /// Index name, as declared in its `index.json`.
        #[arg(long)]
        index: String,
        /// JSONL file holding one document per line (`docs/05-api.md`).
        documents: PathBuf,
    },
    /// Compact an index into the next epoch, without its dead passages
    /// (ADR-0006, ADR-0017).
    Compact {
        /// Root data directory (`<data>/indexes/<index>`).
        #[arg(long)]
        data: PathBuf,
        /// Index name, as declared in its `index.json`.
        #[arg(long)]
        index: String,
    },
    /// Rebuild the lexical index of the committed epoch from the stored texts
    /// (`RECONSTRUIRE-LEXICAL`): the job to run when the analysis chain of
    /// `lexical.language` or `lexical.positions` changes.
    RebuildLexical {
        /// Root data directory (`<data>/indexes/<index>`).
        #[arg(long)]
        data: PathBuf,
        /// Index name, as declared in its `index.json`.
        #[arg(long)]
        index: String,
    },
    /// Take a coherent snapshot of an epoch into the exchange root (ADR-0064,
    /// ADR-0065). Offline: stop the server first.
    Snapshot {
        /// Root data directory (`<data>/indexes/<index>`).
        #[arg(long)]
        data: PathBuf,
        /// Root exchange directory (`[paths] exchange` of `crible.toml`); the
        /// snapshot is written under its `snapshots/`.
        #[arg(long)]
        exchange: PathBuf,
        /// Index name, as declared in its `index.json`.
        #[arg(long)]
        index: String,
    },
    /// Restore a snapshot into the index it names, as a new epoch (ADR-0065).
    /// Offline: stop the server first.
    Restore {
        /// Root data directory (`<data>/indexes/<index>`).
        #[arg(long)]
        data: PathBuf,
        /// Snapshot directory, holding `snapshot.json` and the copy.
        #[arg(long)]
        snapshot: PathBuf,
    },
    /// Export the live data of an index as a portable `replace_all` batch.
    Export {
        /// Root data directory (`<data>/indexes/<index>`).
        #[arg(long)]
        data: PathBuf,
        /// Root models directory (`[paths] models` of `crible.toml`).
        #[arg(long)]
        models: PathBuf,
        /// Root exchange directory (`[paths] exchange` of `crible.toml`); the
        /// batch is written under its `outbox/`.
        #[arg(long)]
        exchange: PathBuf,
        /// Index name, as declared in its `index.json`.
        #[arg(long)]
        index: String,
    },
    /// Update the fields of documents without re-encoding anything (ADR-0060).
    Patch {
        /// Root data directory (`<data>/indexes/<index>`).
        #[arg(long)]
        data: PathBuf,
        /// Index name, as declared in its `index.json`.
        #[arg(long)]
        index: String,
        /// JSONL file holding one `{ "doc_id": …, "fields": { … } }` per line.
        file: PathBuf,
    },
    /// Search an index with a provided vector or a text query.
    Search {
        /// Root data directory (`<data>/indexes/<index>`).
        #[arg(long)]
        data: PathBuf,
        /// Index name, as declared in its `index.json`.
        #[arg(long)]
        index: String,
        /// NumPy `.npy` or JSON file holding the query vector; one of
        /// `--vector-file` and `--query` is required.
        #[arg(long)]
        vector_file: Option<PathBuf>,
        /// Text query, encoded by the model of the index or by `--model`.
        #[arg(long)]
        query: Option<String>,
        /// Root models directory (`[paths] models` of `crible.toml`); required
        /// with `--query`.
        #[arg(long)]
        models: Option<PathBuf>,
        /// ONNX encoder to use for `--query`; defaults to the model of the
        /// index, which must be an `onnx` model.
        #[arg(long)]
        model: Option<String>,
        /// Filter as a JSON object, in the grammar of `docs/05-api.md`.
        #[arg(long)]
        filter: Option<String>,
        /// Number of results, between 1 and 1000.
        #[arg(long, default_value_t = 10)]
        k: usize,
        /// Force the exact dense search, whatever the size of the pool
        /// (`exact` of the search request in `docs/05-api.md`; evaluation and
        /// debug).
        #[arg(long)]
        exact: bool,
        /// Regroup the results by document: `document`
        /// (`group_by` of `docs/05-api.md`). `k` then counts documents.
        #[arg(long, value_name = "document")]
        group_by: Option<String>,
        /// Passages returned per grouped document, from 1 to 5
        /// (`passages_per_document` of `docs/05-api.md`); only acts with
        /// `--group-by document`.
        #[arg(long, default_value_t = 1)]
        passages_per_document: usize,
        /// Fuse the passages of identical content: `content`
        /// (`dedup` of `docs/05-api.md`).
        #[arg(long, value_name = "content")]
        dedup: Option<String>,
        /// Branches to run: `dense`, `lexical` or `hybrid` (`mode` of
        /// `docs/05-api.md`); defaults to the mode of the index, `hybrid` when
        /// the lexical branch is enabled and `dense` otherwise.
        #[arg(long, value_enum)]
        mode: Option<commands::SearchMode>,
        /// Fusion of the two branches of `--mode hybrid`: `rrf` or
        /// `relative_score` (`fusion` of `docs/05-api.md`); defaults to the
        /// fusion of the index.
        #[arg(long, value_enum)]
        fusion: Option<commands::SearchFusion>,
        /// Weight of the dense branch with `--fusion relative_score`, in `[0, 1]`
        /// bounds included, on a finite value (`docs/03-recherche.md`, "Fusion",
        /// ADR-0090); required with that fusion. `allow_hyphen_values` lets a
        /// negative value reach the domain control, so it is refused in
        /// `invalid_request` like any other entry point, rather than as a clap
        /// usage error.
        #[arg(long, allow_hyphen_values = true)]
        alpha: Option<f64>,
        /// Output format: `json` (the response of `docs/05-api.md`, default) or
        /// `text` (readable report with a text excerpt).
        #[arg(long, value_enum, default_value_t = commands::SearchFormat::Json)]
        format: commands::SearchFormat,
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
    /// Serve the HTTP API of an instance until SIGTERM or SIGINT (ADR-0099).
    Serve {
        /// Path of the instance configuration file (ADR-0097).
        #[arg(long, default_value = "/etc/crible/crible.toml")]
        config: PathBuf,
    },
    /// Manage the API keys of an instance (ADR-0100).
    Key {
        /// Key operation to run.
        #[command(subcommand)]
        command: KeyCommand,
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

/// Operations on API keys.
#[derive(Debug, Subcommand)]
enum KeyCommand {
    /// Draw a key and print it with its SHA-256 digest, for `[[auth.keys]]`.
    ///
    /// No option: the command writes `key: <key>` then `sha256: <digest>` on
    /// stdout, and reads and writes no file (ADR-0100).
    Generate,
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
    // The server journals on stdout, in JSON, at the level `RUST_LOG` asks for
    // (ADR-0096): its subscriber replaces the one of the commands, which stays
    // on stderr.
    if matches!(cli.command, Command::Serve { .. }) {
        crible_server::init_logging()?;
    } else {
        init_logging()?;
    }
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
        Command::Ingest {
            data,
            models,
            index,
            documents,
        } => commands::ingest_documents(&data, &models, &index, &documents),
        Command::Compact { data, index } => commands::compact_index(&data, &index),
        Command::RebuildLexical { data, index } => commands::rebuild_lexical(&data, &index),
        Command::Snapshot {
            data,
            exchange,
            index,
        } => commands::snapshot_index(&data, &exchange, &index),
        Command::Restore { data, snapshot } => commands::restore_index(&data, &snapshot),
        Command::Export {
            data,
            models,
            exchange,
            index,
        } => commands::export_index(&data, &models, &exchange, &index),
        Command::Patch { data, index, file } => commands::patch_documents(&data, &index, &file),
        Command::Search {
            data,
            index,
            vector_file,
            query,
            models,
            model,
            filter,
            k,
            exact,
            group_by,
            passages_per_document,
            dedup,
            mode,
            fusion,
            alpha,
            format,
        } => commands::search(&commands::SearchRequest {
            data: &data,
            index: &index,
            vector_file: vector_file.as_deref(),
            query: query.as_deref(),
            models: models.as_deref(),
            model: model.as_deref(),
            filter: filter.as_deref(),
            k,
            exact,
            group_by: group_by.as_deref(),
            passages_per_document,
            dedup: dedup.as_deref(),
            mode,
            fusion,
            alpha,
            format,
        }),
        Command::Inspect { data, index } => commands::inspect(&data, index.as_deref()),
        Command::Serve { config } => crible_server::serve_blocking(&config),
        Command::Key { command } => match command {
            KeyCommand::Generate => commands::generate_key(),
        },
    }
}

/// Installs the log subscriber of a command: JSON lines on stderr, filtered by
/// `RUST_LOG` through the same [`EnvFilter`](tracing_subscriber::EnvFilter) as
/// the server (ADR-0096).
///
/// The journal never goes to stdout: a command prints its result there (JSON of
/// `search`, report of `attach`), and a script parses it. Every stderr line of
/// the journal is one JSON object, as `docs/07-stack-et-deploiement.md` lists
/// `tracing-subscriber` "(JSON)". An absent or blank `RUST_LOG` keeps the level
/// of P2, warnings and above, because the warnings that matter are those of the
/// online-batch cleanup (ADR-0056, decision 3); a present value replaces that
/// level, as `crible serve` already did.
///
/// `try_init` rather than `init`: a second subscriber is not a reason to abort
/// a command. A `RUST_LOG` that is not a valid filter is one, though: it is
/// configuration, and the command reports it like any other error.
///
/// # Errors
///
/// [`ErrorCode::InvalidRequest`](crible_core::error::ErrorCode::InvalidRequest)
/// when `RUST_LOG` is not a valid filter.
fn init_logging() -> Result<(), CribleError> {
    let raw = std::env::var("RUST_LOG").ok();
    let filter = effective_filter(raw.as_deref());
    let subscriber = crible_server::log_subscriber(std::io::stderr, &filter)?;
    let _ = subscriber.try_init();
    Ok(())
}

/// Level a command journals at when `RUST_LOG` is absent or blank.
///
/// The server defaults to `info` (ADR-0096); a command keeps `warn`, the level
/// of P2, so `crible search > result.json` stays free of lines a caller did not
/// ask for.
const DEFAULT_COMMAND_FILTER: &str = "warn";

/// Filter a command uses, from the raw value of `RUST_LOG`.
///
/// A present but blank variable is the absent one, as in the server.
#[must_use]
fn effective_filter(raw: Option<&str>) -> String {
    match raw {
        Some(value) if !value.trim().is_empty() => value.to_owned(),
        _ => DEFAULT_COMMAND_FILTER.to_owned(),
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

#[cfg(test)]
mod tests {
    use super::{DEFAULT_COMMAND_FILTER, effective_filter};
    use std::io;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    /// In-memory writer that captures the journal of a test.
    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Sink {
        fn contents(&self) -> Vec<u8> {
            self.0.lock().map(|bytes| bytes.clone()).unwrap_or_default()
        }
    }

    /// One write handle of the [`Sink`].
    struct SinkWriter(Sink);

    impl io::Write for SinkWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            let shared = &self.0.0;
            let mut bytes = shared
                .lock()
                .map_err(|_| io::Error::other("the journal sink is poisoned"))?;
            bytes.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Sink {
        type Writer = SinkWriter;

        fn make_writer(&'a self) -> Self::Writer {
            SinkWriter(self.clone())
        }
    }

    #[test]
    fn the_installed_journal_writes_one_json_object_per_line() {
        // `docs/07` lists `tracing-subscriber` "(JSON)": a line of the journal
        // is a JSON object a tool can parse, not free text.
        let sink = Sink::default();
        let subscriber = crible_server::log_subscriber(sink.clone(), DEFAULT_COMMAND_FILTER)
            .expect("the default filter is valid");
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(
                path = "/data/tmp/online-x.lock",
                "cannot take the presence lock of an online batch"
            );
        });

        let text = String::from_utf8(sink.contents()).expect("the journal is UTF-8");
        let line = text
            .lines()
            .next()
            .expect("a warning writes at least one line");
        let value: serde_json::Value =
            serde_json::from_str(line).expect("a journal line is a JSON object");
        assert_eq!(value["level"], "WARN");
        assert_eq!(
            value["fields"]["message"],
            "cannot take the presence lock of an online batch"
        );
        assert_eq!(value["fields"]["path"], "/data/tmp/online-x.lock");
    }

    #[test]
    fn the_journal_stays_silent_below_warn_without_rust_log() {
        // The default of a command is `WARN`: an `info!` must not reach stderr,
        // so a caller's stderr stays free of noise.
        let sink = Sink::default();
        let subscriber = crible_server::log_subscriber(sink.clone(), DEFAULT_COMMAND_FILTER)
            .expect("the default filter is valid");
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("a message below the level of the journal");
            tracing::debug!("another one");
        });
        assert!(
            sink.contents().is_empty(),
            "a level below WARN writes nothing: {:?}",
            String::from_utf8_lossy(&sink.contents())
        );
    }

    #[test]
    fn rust_log_raises_the_level_of_a_command() {
        // ADR-0096: a command honors `RUST_LOG` through the same `EnvFilter` as
        // the server, so an operator can ask for the `info` lines of a command
        // without changing the binary.
        assert_eq!(effective_filter(None), "warn");
        assert_eq!(effective_filter(Some("")), "warn");
        assert_eq!(effective_filter(Some("  ")), "warn");
        assert_eq!(effective_filter(Some("info")), "info");

        let sink = Sink::default();
        let filter = effective_filter(Some("info"));
        let subscriber =
            crible_server::log_subscriber(sink.clone(), &filter).expect("info is valid");
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("an info line an operator asked for");
        });
        let text = String::from_utf8(sink.contents()).expect("the journal is UTF-8");
        assert!(
            text.contains("an info line an operator asked for"),
            "`RUST_LOG=info` lets the info lines through: {text:?}"
        );
    }

    #[test]
    fn an_invalid_rust_log_is_reported() {
        // The filter is configuration: a malformed value is a command error of
        // the category `invalid_request`, not a silent fallback.
        let failure = crible_server::log_subscriber(Sink::default(), "info,,,=x")
            .err()
            .expect("an invalid filter is refused");
        assert_eq!(
            failure.code(),
            crible_core::error::ErrorCode::InvalidRequest
        );
    }
}
