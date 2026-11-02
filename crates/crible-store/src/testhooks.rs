// SPDX-License-Identifier: Apache-2.0

//! Observable hook of the segment readers, `failpoints` build only.
//!
//! References: ADR-0126, decisions 2 and 6. The cache of the dense readers
//! (module [`crate::readers`]) exists to remove the opening of the readers from
//! the path of a query, and a test must prove that absence **on the observable
//! path**, never by asserting that the code does not call `open`. This module is
//! that path: with the `failpoints` feature, and only with it,
//! [`SegmentReader::open`](crate::segment::SegmentReader::open) appends one
//! `open` line, one `seal` line around the blake3 check, and the reader appends
//! one `drop` line when its last owner releases it.
//!
//! The log is a file whose path [`SEGMENT_READERS_ENV`] names. A test sets the
//! variable to a temporary file, runs its scenario, and counts the lines with
//! [`read_events`]. An absent variable, or a write that fails, changes nothing:
//! the hook is silent and the store behaves exactly as without it.
//!
//! One line is `open <segment_id>`, `seal <segment_id>` or `drop <segment_id>`,
//! written in append mode, so two writers never overwrite each other's lines.
//!
//! ```text
//! cargo test -p crible-store --features failpoints
//! ```

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Environment variable naming the log file of the hook.
///
/// Absent, the hook writes nothing. Set, it names the file the events are
/// appended to; the file is created on the first event.
pub const SEGMENT_READERS_ENV: &str = "CRIBLE_TEST_SEGMENT_READERS";

/// One event the hook recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReaderEvent {
    /// A reader of `segment_id` was opened.
    Open {
        /// Identifier of the segment, from its directory name.
        segment_id: String,
    },
    /// The blake3 checksum of `segment_id` was verified.
    Seal {
        /// Identifier of the segment, from its directory name.
        segment_id: String,
    },
    /// The last owner of the reader of `segment_id` released it.
    Drop {
        /// Identifier of the segment, from its directory name.
        segment_id: String,
    },
}

impl ReaderEvent {
    /// Segment the event names.
    #[must_use]
    pub fn segment_id(&self) -> &str {
        match self {
            Self::Open { segment_id } | Self::Seal { segment_id } | Self::Drop { segment_id } => {
                segment_id
            }
        }
    }
}

/// Kind of an event, as [`record`] writes it.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Kind {
    /// A reader was opened.
    Open,
    /// A checksum was verified.
    Seal,
    /// A reader was released.
    Drop,
}

impl Kind {
    /// Token the log line starts with.
    fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Seal => "seal",
            Self::Drop => "drop",
        }
    }
}

/// Path of the log armed now, or `None` when the hook is off.
///
/// [`SegmentReader::open`](crate::segment::SegmentReader::open) captures it, so
/// every event of one reader lands in the log that was armed when it opened: a
/// reader released after a test removed the variable still writes to the log of
/// its own test, never to the log of the next one.
#[must_use]
pub(crate) fn armed_log() -> Option<PathBuf> {
    std::env::var_os(SEGMENT_READERS_ENV).map(PathBuf::from)
}

/// Appends one event line to `log`, silently.
///
/// An uncreatable file or a failed write leaves the caller with no error and no
/// behaviour change: the hook observes, it never decides.
pub(crate) fn record_to(log: &Path, kind: Kind, segment_id: &str) {
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log) else {
        return;
    };
    let _ = writeln!(file, "{} {segment_id}", kind.as_str());
}

/// Reads the events of the log at `path`.
///
/// An absent file is an empty log: a test that never opened a reader sees no
/// event rather than an error.
///
/// # Errors
///
/// The errors of reading the file, except [`io::ErrorKind::NotFound`].
pub fn read_events(path: &Path) -> io::Result<Vec<ReaderEvent>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut events = Vec::new();
    for line in text.lines() {
        let Some((kind, segment_id)) = line.split_once(' ') else {
            continue;
        };
        let segment_id = segment_id.trim();
        if segment_id.is_empty() {
            continue;
        }
        let segment_id = segment_id.to_owned();
        let event = match kind {
            "open" => ReaderEvent::Open { segment_id },
            "seal" => ReaderEvent::Seal { segment_id },
            "drop" => ReaderEvent::Drop { segment_id },
            _ => continue,
        };
        events.push(event);
    }
    Ok(events)
}

/// Forgets the events recorded so far, by removing the log at `path`.
///
/// An absent file is an empty log, so removing it twice is not an error.
///
/// # Errors
///
/// The errors of removing the file, except [`io::ErrorKind::NotFound`].
pub fn reset_events(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Kind, ReaderEvent, SEGMENT_READERS_ENV, armed_log, read_events, record_to, reset_events,
    };
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard};

    /// Serialises the access to the process-global variable of the hook.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// A log path in a temporary directory, with the variable armed.
    ///
    /// The caller keeps the guard: the variable is process-global, so the tests
    /// of this module run one at a time.
    fn armed_log_file() -> (MutexGuard<'static, ()>, tempfile::TempDir, PathBuf) {
        let serial = SERIAL.lock().expect("the hook slot is not poisoned");
        let directory = tempfile::tempdir().expect("a temporary directory is creatable");
        let path = directory.path().join("segment-readers.log");
        // SAFETY: the guard is held, and no other test of this binary reads the
        // variable meanwhile; the guard outlives the removal below.
        unsafe { std::env::set_var(SEGMENT_READERS_ENV, &path) };
        (serial, directory, path)
    }

    #[test]
    fn the_hook_reads_back_the_events_it_writes() {
        let (serial, _directory, path) = armed_log_file();
        let log = armed_log().expect("the hook names its log");
        record_to(&log, Kind::Open, "c0-0");
        record_to(&log, Kind::Seal, "c0-0");
        record_to(&log, Kind::Drop, "c0-0");
        let events = read_events(&path).expect("the log is readable");
        assert_eq!(
            events,
            vec![
                ReaderEvent::Open {
                    segment_id: "c0-0".to_owned()
                },
                ReaderEvent::Seal {
                    segment_id: "c0-0".to_owned()
                },
                ReaderEvent::Drop {
                    segment_id: "c0-0".to_owned()
                },
            ]
        );
        // SAFETY: the guard is held, so no other test of this binary reads the
        // variable while it is removed.
        unsafe { std::env::remove_var(SEGMENT_READERS_ENV) };
        drop(serial);
    }

    #[test]
    fn an_absent_log_is_empty_and_reset_is_idempotent() {
        let _serial = SERIAL.lock().expect("the hook slot is not poisoned");
        let directory = tempfile::tempdir().expect("a temporary directory is creatable");
        let path = directory.path().join("missing.log");
        assert!(
            read_events(&path)
                .expect("an absent log is readable")
                .is_empty()
        );
        reset_events(&path).expect("resetting an absent log succeeds");
    }

    #[test]
    fn an_absent_variable_arms_no_log() {
        let _serial = SERIAL.lock().expect("the hook slot is not poisoned");
        // SAFETY: the guard is held, so no other test of this binary reads the
        // variable while it is removed.
        unsafe { std::env::remove_var(SEGMENT_READERS_ENV) };
        assert!(
            armed_log().is_none(),
            "the hook names no log without its variable"
        );
    }
}
