// SPDX-License-Identifier: Apache-2.0

//! Journal of the server: JSON lines on stdout, honoring `RUST_LOG`.
//!
//! References: `docs/06-observabilite.md` ("Santé et logs"), `docs/07` ("Logs",
//! `tracing-subscriber` JSON) and ADR-0096. The subscriber is JSON, its writer
//! is stdout (Docker reads it), and its filter comes from `RUST_LOG`, with
//! `info` when the variable is absent.
//!
//! **Confidentiality does not depend on the level.** Whatever `RUST_LOG` says,
//! no log call of the server carries a query text or a filter value: no such
//! field is ever journalised, at any level (INV-13, ADR-0096). The filter only
//! decides which lines are printed. The dependency is held to the same rule:
//! the rejection journal of axum, which quotes the body it refused, is
//! silenced by a directive forced after the one of `RUST_LOG`
//! (ADR-0101, decision 4).

use std::io;

use crible_core::error::{CribleError, ErrorCode};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::Directive;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt;

/// Filter used when `RUST_LOG` is absent or empty (ADR-0096).
pub const DEFAULT_FILTER: &str = "info";

/// Target on which axum journals the rejection of an extractor, body text
/// included.
pub const REJECTION_TARGET: &str = "axum::rejection";

/// Directive that silences [`REJECTION_TARGET`], forced after the one of
/// `RUST_LOG`.
pub const SILENCED_REJECTION_DIRECTIVE: &str = "axum::rejection=off";

/// Installs the journal of the server on stdout.
///
/// # Errors
///
/// [`ErrorCode::InvalidRequest`] when `RUST_LOG` is not a valid filter, and
/// [`ErrorCode::Internal`] when a subscriber is already installed.
pub fn init_logging() -> Result<(), CribleError> {
    let filter = effective_filter(std::env::var("RUST_LOG").ok().as_deref());
    let subscriber = log_subscriber(io::stdout, &filter)?;
    subscriber.try_init().map_err(|error| {
        CribleError::new(
            ErrorCode::Internal,
            format!("cannot install the log subscriber of the server: {error}"),
        )
    })
}

/// Filter a subscriber uses, from the raw value of `RUST_LOG`.
///
/// A present but blank variable is the absent one: an empty filter would print
/// nothing, and an operator who exports `RUST_LOG=` expects the default.
#[must_use]
pub fn effective_filter(raw: Option<&str>) -> String {
    match raw {
        Some(value) if !value.trim().is_empty() => value.to_owned(),
        _ => DEFAULT_FILTER.to_owned(),
    }
}

/// Builds the JSON subscriber of the server over `writer`.
///
/// The writer is a parameter so a test captures a line; the server passes
/// stdout.
///
/// The filter of `RUST_LOG` is applied, then [`SILENCED_REJECTION_DIRECTIVE`]
/// is added after it: a directive on a target wins over a bare level, and a
/// later directive on the same target replaces the earlier one, so no operator
/// can turn the rejection journal back on. axum journals the body text of an
/// extractor rejection on that target at `TRACE`, and that text quotes the
/// value the client sent (`invalid type: string "<value>"`): a filter of
/// `trace` or `axum::rejection=trace` would print a request field, which
/// `docs/06` and ADR-0096 forbid at any level (ADR-0101, decision 4).
///
/// # Errors
///
/// [`ErrorCode::InvalidRequest`] when `filter` is not a valid `RUST_LOG`
/// directive. The message never quotes the rejected directive in full: it is
/// configuration, not request content, but a filter is operator input like any
/// other and the diagnosis names the variable, not its value.
pub fn log_subscriber<W>(
    writer: W,
    filter: &str,
) -> Result<impl tracing::Subscriber + Send + Sync + 'static, CribleError>
where
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    let filter = EnvFilter::try_new(filter).map_err(|error| {
        CribleError::new(
            ErrorCode::InvalidRequest,
            format!("RUST_LOG does not hold a valid filter: {error}"),
        )
    })?;
    let filter = filter.add_directive(silenced_directive()?);
    Ok(tracing_subscriber::fmt()
        .json()
        .with_writer(writer)
        .with_env_filter(filter)
        .finish())
}

/// Parses the directive that silences the rejection journal of axum.
///
/// The text is a constant of the code: a failure to parse it is a bug of the
/// server, not a fault of the configuration, hence [`ErrorCode::Internal`].
fn silenced_directive() -> Result<Directive, CribleError> {
    SILENCED_REJECTION_DIRECTIVE.parse().map_err(|error| {
        CribleError::new(
            ErrorCode::Internal,
            format!("the fixed log directive of the server is malformed: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_FILTER, effective_filter, log_subscriber};
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
    fn the_absent_variable_means_info() {
        // ADR-0096: `RUST_LOG` absent (or blank) lets the default `info`
        // through; an empty filter would silence the journal entirely.
        assert_eq!(effective_filter(None), DEFAULT_FILTER);
        assert_eq!(effective_filter(Some("")), DEFAULT_FILTER);
        assert_eq!(effective_filter(Some("   ")), DEFAULT_FILTER);
        assert_eq!(effective_filter(Some("warn")), "warn");
    }

    #[test]
    fn the_journal_is_json_and_honors_rust_log() {
        // ADR-0096: with `info`, an `info!` reaches the writer, and each line is
        // one JSON object.
        let sink = Sink::default();
        let subscriber = log_subscriber(sink.clone(), "info").expect("a valid filter is accepted");
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(index = "alpha", "the index is published");
        });
        let text = String::from_utf8(sink.contents()).expect("the journal is UTF-8");
        let line = text
            .lines()
            .next()
            .expect("an info line is printed at level info");
        let value: serde_json::Value =
            serde_json::from_str(line).expect("a journal line is a JSON object");
        assert_eq!(value["level"], "INFO");
        assert_eq!(value["fields"]["index"], "alpha");
    }

    #[test]
    fn a_narrower_filter_silences_info() {
        let sink = Sink::default();
        let subscriber = log_subscriber(sink.clone(), "warn").expect("a valid filter is accepted");
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("this line must not be printed");
        });
        assert!(
            sink.contents().is_empty(),
            "the filter comes from RUST_LOG: {:?}",
            String::from_utf8_lossy(&sink.contents())
        );
    }

    #[test]
    fn the_rejection_journal_of_axum_is_silenced_whatever_rust_log_says() {
        // ADR-0096, ADR-0101 decision 4: axum journals the body text of an
        // extractor rejection on the target `axum::rejection` at `TRACE`, and
        // that text quotes the value the client sent. The forced directive cuts
        // the target under a bare `trace` and under a directive that names it,
        // and the later directive replaces the earlier one.
        for filter in [
            "trace",
            "debug",
            "axum::rejection=trace",
            "info,axum::rejection=trace",
            "trace,axum=trace",
        ] {
            let sink = Sink::default();
            let subscriber =
                log_subscriber(sink.clone(), filter).expect("a valid filter is accepted");
            tracing::subscriber::with_default(subscriber, || {
                tracing::event!(
                    target: "axum::rejection",
                    tracing::Level::TRACE,
                    body = "zz-rejected-value-zz",
                    "Failed to deserialize the JSON body into the target type"
                );
            });
            assert!(
                sink.contents().is_empty(),
                "a rejected value reached the journal under {filter}: {:?}",
                String::from_utf8_lossy(&sink.contents())
            );
        }
    }

    #[test]
    fn the_forced_directive_does_not_silence_the_server() {
        // The directive names one target: the lines of the server keep coming
        // under the same filters (ADR-0096).
        let sink = Sink::default();
        let subscriber =
            log_subscriber(sink.clone(), "trace,axum::rejection=trace").expect("a valid filter");
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(index = "alpha", "the index is published");
        });
        assert!(
            !sink.contents().is_empty(),
            "the lines of the server are still printed"
        );
    }

    #[test]
    fn an_invalid_filter_is_refused() {
        let error = log_subscriber(Sink::default(), "info,,,=x").err();
        let error = error.expect("an invalid filter must be refused");
        assert_eq!(error.code(), crible_core::error::ErrorCode::InvalidRequest);
    }
}
