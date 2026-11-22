// SPDX-License-Identifier: Apache-2.0

//! The trim of the allocator at the end of a job, and its `trim` line
//! (ADR-0160).
//!
//! The module holds the production items of the task `T05-CORRIGER-MALLOC-TRIM`
//! and the tests that fix their contract. ADR-0160, decisions 1 and 4, fixes
//! them:
//!
//! - `pub(crate) fn trim_allocator(pad: usize) -> bool`, the single function the
//!   calling code knows. Under `cfg(all(target_os = "linux", target_env =
//!   "gnu"))` it calls `malloc_trim` and renders a non-zero return as `true`;
//!   everywhere else it calls nothing and returns `false` (decision 1).
//! - `pub(crate) fn malloc_trim(pad: usize) -> core::ffi::c_int`, declared in an
//!   `unsafe extern "C"` block of this module under the same `cfg`, with a
//!   `// SAFETY:` comment, as `crates/crible/examples/rescore_ab.rs:914-925`
//!   does for `mincore`. The tests read the raw `0`/`1` domain of glibc here,
//!   which no other test reaches.
//! - `pub(crate) struct Point<'a>` with `Point::job_end(job_id: &'a str, kind:
//!   &'a str, index: &'a str) -> Self`, the job whose end triggered the call,
//!   on the model of `crate::memory::Point` (decision 4).
//! - `pub(crate) struct Sample` with the public fields `pad: usize`,
//!   `trimmed: bool` and `duration_us: u64`, the three numbers of the line.
//! - `pub(crate) fn log(point: Point<'_>, sample: &Sample)`, a `debug` line whose
//!   message is `trim` and whose fields are exactly those of decision 4.
//! - `pub(crate) fn log_call(job_id: &str, kind: &str, index: &str, trimmed:
//!   bool, duration_us: u64)`, the only item `jobs.rs` calls for the line: on
//!   glibc it renders [`log`], and elsewhere it writes nothing because no call
//!   was made, on the model of `crate::memory::log` (decisions 1 and 4).
//! - `pub(crate) mod seam`, the test bench of decision 5, under `#[cfg(test)]`:
//!   a counter of the trims a board admitted and a pause that holds the one
//!   which owns the guard, so two concurrent ends of job are counted
//!   deterministically. A production build compiles none of it.
//!
//! Nothing here is emitted elsewhere than a glibc build: a line says a call was
//! made, and off glibc no call is made (ADR-0160, decisions 1 and 4).
//!
//! **Confidentiality.** The line carries numbers, a job identifier, a job kind
//! and an index name, and nothing else: no query text, no filter value, no
//! internal identifier (INV-13, ADR-0096).

#[cfg(any(all(target_os = "linux", target_env = "gnu"), test))]
use tracing::debug;

// `malloc_trim` of glibc, declared locally: one symbol, one target, so no
// `libc` dependency and no line in `docs/07` (ADR-0160, decision 1).
//
// On another system the symbol does not exist and the block is not compiled:
// `trim_allocator` then has a second definition that calls nothing. Rustdoc
// generates no documentation for an extern block, so these are plain comments.
// SAFETY: `malloc_trim` is stateless, takes a `usize` by value, returns a
// `c_int`, and signals its only failure by the value `0`; it neither reads nor
// writes a pointer the caller owns. The declaration matches glibc's ABI and
// `trim_allocator` is the single caller.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
unsafe extern "C" {
    // Releases the free pages at the top of the heap, or the padding asked.
    pub(crate) fn malloc_trim(pad: usize) -> core::ffi::c_int;
}

/// Calls the allocator to give back the free pages of its heap, and renders
/// whether it released some (ADR-0160, decision 1).
///
/// The calling code knows this function only, whatever the target: on glibc a
/// non-zero return of `malloc_trim` means pages were released, everywhere else
/// there is no allocator to call and the answer is `false`.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub(crate) fn trim_allocator(pad: usize) -> bool {
    // SAFETY: the declaration above takes a `usize` by value and returns a
    // `c_int`; it holds no state the call could corrupt, and `pad` is the
    // padding glibc documents, `0` at every call of the server.
    unsafe { malloc_trim(pad) != 0 }
}

/// The no-op wrapper of the targets without glibc: nothing is called, so
/// nothing is released (ADR-0160, decision 1).
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub(crate) fn trim_allocator(_pad: usize) -> bool {
    false
}

/// What a `trim` line names beside the numbers: the job whose end triggered the
/// call, under the names and shapes already employed (ADR-0104, ADR-0149,
/// ADR-0151).
#[cfg(any(all(target_os = "linux", target_env = "gnu"), test))]
pub(crate) struct Point<'a> {
    job_id: &'a str,
    kind: &'a str,
    index: &'a str,
}

#[cfg(any(all(target_os = "linux", target_env = "gnu"), test))]
impl<'a> Point<'a> {
    /// End of a job: its identifier, its kind and its index.
    #[must_use]
    pub(crate) const fn job_end(job_id: &'a str, kind: &'a str, index: &'a str) -> Self {
        Self {
            job_id,
            kind,
            index,
        }
    }
}

/// The three numbers of a `trim` line: the argument, what the call released and
/// how long it took (ADR-0160, decision 4).
#[cfg(any(all(target_os = "linux", target_env = "gnu"), test))]
pub(crate) struct Sample {
    /// The argument passed to `malloc_trim`, so `0`.
    pub(crate) pad: usize,
    /// `true` when the call rendered non-zero, so pages were released.
    pub(crate) trimmed: bool,
    /// Duration of the call, in microseconds.
    pub(crate) duration_us: u64,
}

/// Emits the `trim` line of a made call, at level `debug`.
///
/// It is emitted for a call that **was made** only: a skipped trim has nothing
/// to measure and writes no line, so `trimmed = 0` keeps a single sense, that of
/// a call which released nothing (ADR-0160, decision 4). The line is silent
/// under the default `info` filter.
#[cfg(any(all(target_os = "linux", target_env = "gnu"), test))]
pub(crate) fn log(point: Point<'_>, sample: &Sample) {
    debug!(
        pad = sample.pad,
        trimmed = u8::from(sample.trimmed),
        duration_us = sample.duration_us,
        job_id = point.job_id,
        kind = point.kind,
        index = point.index,
        "trim"
    );
}

/// Emits the `trim` line of the call that just ended, and nothing off glibc,
/// where no call was made.
///
/// The shape follows [`crate::memory::log`]: the calling code names the job and
/// the numbers, and the target decides whether a line exists (ADR-0160,
/// decisions 1 and 4).
pub(crate) fn log_call(job_id: &str, kind: &str, index: &str, trimmed: bool, duration_us: u64) {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        log(
            Point::job_end(job_id, kind, index),
            &Sample {
                // The server passes `0` at every call: the allocator returns
                // everything it can (ADR-0160, decision 4).
                pad: 0,
                trimmed,
                duration_us,
            },
        );
    }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        // No allocator was called, so no line is written; the fields are still
        // consumed, so the type keeps one shape on every target.
        let _ = (job_id, kind, index, trimmed, duration_us);
    }
}

/// Counting and pause seam of the unit tests of ADR-0160, decision 5.
///
/// It exists in a test build only: a production build compiles neither the
/// counter nor the pause, so the code that runs in service is unchanged.
#[cfg(test)]
pub(crate) mod seam {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{Receiver, Sender};

    /// Number of trims one job board admitted, counted for the test of
    /// decision 5.
    #[derive(Debug, Default)]
    pub(crate) struct Calls(AtomicUsize);

    impl Calls {
        /// Counts one admitted trim.
        pub(crate) fn record(&self) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }

        /// Number of trims admitted since the board opened.
        pub(crate) fn count(&self) -> usize {
            self.0.load(Ordering::Acquire)
        }
    }

    /// Rendezvous of the test of decision 5: the admitted trim announces itself
    /// on `reached` and waits for `release` **while it still holds the guard**,
    /// so the other attempt is deterministically skipped.
    pub(crate) struct Pause {
        reached: Sender<()>,
        release: Receiver<()>,
    }

    impl Pause {
        /// Arms a pause that announces itself and waits for its release.
        #[must_use]
        pub(crate) fn new(reached: Sender<()>, release: Receiver<()>) -> Self {
            Self { reached, release }
        }

        /// Announces the admitted trim and waits for its release.
        pub(crate) fn hold(self) {
            let _ = self.reached.send(());
            let _ = self.release.recv();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Point, Sample, log, trim_allocator};
    use crate::test_sink::Sink;

    /// Identifier of the job the `trim` line of the tests names.
    const JOB_ID: &str = "job_0123456789abcdef";

    /// Name of the index the `trim` line of the tests names.
    const INDEX: &str = "alpha";

    /// Captures the line `log` writes for `sample`.
    fn captured(sample: &Sample) -> String {
        let sink = Sink::default();
        let subscriber = crate::logging::log_subscriber(sink.clone(), "debug")
            .expect("the fixed filter of the test is valid");
        tracing::subscriber::with_default(subscriber, || {
            log(Point::job_end(JOB_ID, "compact", INDEX), sample);
        });
        sink.contents()
    }

    /// Off glibc, the wrapper does nothing and says so with its return value:
    /// no symbol is looked up, no page is released (ADR-0160, decision 1).
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    #[test]
    fn trim_is_a_no_op_off_glibc() {
        // Any padding is refused in the same way: the argument never reaches an
        // allocator, because there is no glibc to receive it.
        assert!(
            !trim_allocator(0),
            "off glibc the wrapper renders false, as nothing was released"
        );
        assert!(
            !trim_allocator(4096),
            "off glibc the wrapper renders false whatever the padding"
        );
    }

    /// On glibc, an allocation is freed, the wrapper is called, and the value of
    /// the C function is `0` or `1`: the test fails at the **link** step when the
    /// symbol is missing, which nothing else proves (ADR-0160, decision 5, test
    /// 2).
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn trim_allocator_on_a_freed_allocation_returns_zero_or_one() {
        // A block large enough that its pages go back to the kernel when the
        // allocator returns them; every page is touched, so the mapping exists.
        let mut block: Vec<u8> = vec![0; 8 * 1024 * 1024];
        for offset in (0..block.len()).step_by(4096) {
            block[offset] = 1;
        }
        drop(block);

        // The wrapper is the function the calling code knows; calling it here is
        // what proves that `malloc_trim` linked. Its value is not asserted: the
        // heap of the test process may have nothing left to give back, and both
        // `0` and `1` are legitimate.
        let _trimmed = trim_allocator(0);

        // The `{0, 1}` domain is the contract of glibc's `malloc_trim`, which
        // `bool` keeps: `1` released pages, `0` released none, and any other
        // value would mean the symbol is not the one ADR-0160 declares.
        // SAFETY: `malloc_trim` takes a `usize` by value, returns a `c_int`, and
        // has no state of its own to corrupt; it signals its failure by the
        // value `0` alone. The padding is `0` here, as the ADR prescribes.
        let code = unsafe { super::malloc_trim(0) };
        assert!(
            code == 0 || code == 1,
            "glibc's malloc_trim returns 1 when it released pages and 0 otherwise: {code}"
        );
    }

    /// INV-13 and ADR-0160, decision 4: the `trim` line holds exactly the six
    /// planned fields, and every one of them is a number or a name. No query
    /// text, no filter value and no internal identifier can reach it, because
    /// no field of the line can hold one.
    #[test]
    fn inv_13_trim_line_carries_only_numbers_and_names() {
        let line = captured(&Sample {
            pad: 0,
            trimmed: true,
            duration_us: 1_234,
        });
        let value: serde_json::Value =
            serde_json::from_str(line.trim()).expect("the line is one JSON object");
        assert_eq!(
            value["level"], "DEBUG",
            "the line is of level debug: {line}"
        );
        let fields = value
            .get("fields")
            .and_then(serde_json::Value::as_object)
            .expect("the line carries its fields");
        let mut names: Vec<&str> = fields.keys().map(String::as_str).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "duration_us",
                "index",
                "job_id",
                "kind",
                "message",
                "pad",
                "trimmed"
            ],
            "the line carries these fields only, so no free field can hold a \
             text or a value (INV-13): {line}"
        );
        assert_eq!(fields["message"], "trim", "{line}");
        assert_eq!(fields["job_id"], JOB_ID, "{line}");
        assert_eq!(fields["kind"], "compact", "{line}");
        assert_eq!(fields["index"], INDEX, "{line}");
        // The three numbers are numbers: `trimmed` is `1`, never `true`, and the
        // exact set of names above already rules out a free field.
        assert_eq!(
            fields["pad"], 0,
            "the padding is the argument, so `0`: {line}"
        );
        assert_eq!(
            fields["trimmed"], 1,
            "a call that released pages renders `1`: {line}"
        );
        assert_eq!(fields["duration_us"], 1_234, "{line}");
        for name in ["pad", "trimmed", "duration_us"] {
            assert!(
                fields[name].as_u64().is_some(),
                "`{name}` is a non-negative integer: {line}"
            );
        }
        assert!(!line.contains("query"), "got {line}");
        assert!(!line.contains("filter"), "got {line}");

        // A call that released nothing renders `0`, which then keeps a single
        // sense: a call was made and returned no page (ADR-0160, decision 4).
        let line = captured(&Sample {
            pad: 0,
            trimmed: false,
            duration_us: 0,
        });
        let value: serde_json::Value =
            serde_json::from_str(line.trim()).expect("the line is one JSON object");
        assert_eq!(
            value["fields"]["trimmed"], 0,
            "a call that released nothing renders `0`, never `false`: {line}"
        );
        assert_eq!(value["fields"]["duration_us"], 0, "{line}");
    }
}
