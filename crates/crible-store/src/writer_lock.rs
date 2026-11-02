// SPDX-License-Identifier: Apache-2.0

//! Writer lock of one index: `WRITER_LOCK` at the root of the index directory.
//!
//! References: ADR-0116 (decision 2), ADR-0056 (identity check after the lock),
//! `docs/02-architecture.md` ("Organisation sur disque", "Concurrence"), and
//! `docs/05-api.md` (code `overloaded`).
//!
//! One process writes a given index. The server takes the lock when it opens an
//! index and holds it while it serves it; each command of the CLI that opens an
//! existing index for writing takes it for the whole duration of the command.
//! A read command never takes it.
//!
//! The lock is a file of the index directory, not of an epoch: it is excluded
//! from the inventory of an epoch, from a snapshot and from an export. A write
//! opening never removes it, because a file removed under a live holder would
//! let a second writer lock a new inode (the ABA defect of ADR-0056). The kernel
//! releases the lock when the process dies, so a leftover file blocks nobody
//! (INV-04).

use std::fs::File;
use std::path::Path;

use crible_core::error::{CribleError, ErrorCode};

use crate::file_lock::{LockAttempt, lock_exclusive};

/// Name of the writer lock, at the root of the index directory (ADR-0116).
pub(crate) const WRITER_LOCK_FILE_NAME: &str = "WRITER_LOCK";

/// Exclusive writer lock of one index, held as long as this value lives.
///
/// Dropping the value releases the lock: the file stays in place, and only the
/// kernel lock goes away.
pub(crate) struct WriterLock {
    /// Handle holding the lock; dropping it releases the lock.
    _file: File,
}

impl WriterLock {
    /// Takes the writer lock of `index`, whose directory is `index_dir`.
    ///
    /// The lock is exclusive and non-blocking: a held lock is a refusal, never a
    /// wait (ADR-0116, decision 2). The file is created when absent and never
    /// removed.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Overloaded`] when another process holds the lock of this
    /// index; the message names the index and says that another process writes
    /// it, and carries no path of the server (ADR-0101). [`ErrorCode::Internal`]
    /// when the lock file cannot be opened or locked.
    pub(crate) fn acquire(index_dir: &Path, index: &str) -> Result<Self, CribleError> {
        let path = index_dir.join(WRITER_LOCK_FILE_NAME);
        let what = format!("writer lock of index '{index}'");
        match lock_exclusive(&path, &what)? {
            LockAttempt::Locked(file) => Ok(Self { _file: file }),
            LockAttempt::Busy => Err(CribleError::new(
                ErrorCode::Overloaded,
                format!(
                    "another process writes the index '{index}' (its {WRITER_LOCK_FILE_NAME} is \
                     held); stop the instance that serves this index before writing to it"
                ),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    // Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions
    // only, and this module writes its helpers inline.
    use super::{WRITER_LOCK_FILE_NAME, WriterLock};
    use crible_core::error::{CribleError, ErrorCode};
    use std::sync::{Arc, Barrier};

    /// Name of the index the tests lock.
    const INDEX: &str = "alpha";

    #[test]
    fn two_acquires_of_the_same_index_give_one_writer() {
        // ADR-0116, decision 2: "un seul processus écrit un index donné". The
        // lock is exclusive and non-blocking, so of two writers racing on one
        // index exactly one proceeds and the other is refused at once in
        // `overloaded`, with a message that names the index and says another
        // process writ[es] it, and no server path (ADR-0101).
        for _ in 0..16 {
            let temp = tempfile::tempdir().expect("a temporary directory is creatable");
            let directory = temp.path().to_path_buf();
            let barrier = Arc::new(Barrier::new(2));
            let starts: Vec<_> = (0..2)
                .map(|_| {
                    let directory = directory.clone();
                    let barrier = Arc::clone(&barrier);
                    std::thread::spawn(move || {
                        barrier.wait();
                        WriterLock::acquire(&directory, INDEX)
                    })
                })
                .collect();
            let outcomes: Vec<Result<WriterLock, CribleError>> = starts
                .into_iter()
                .map(|start| start.join().expect("the thread finishes"))
                .collect();
            assert_eq!(
                outcomes.iter().filter(|outcome| outcome.is_ok()).count(),
                1,
                "exactly one writer takes the lock of the same index"
            );
            let loser = outcomes
                .iter()
                .find_map(|outcome| outcome.as_ref().err())
                .expect("the loser reports its refusal");
            assert_eq!(
                loser.code(),
                ErrorCode::Overloaded,
                "a held writer lock is `overloaded` (`docs/05-api.md`)"
            );
            let message = loser.message();
            assert!(
                message.contains(INDEX),
                "the message names the index, got {message:?}"
            );
            assert!(
                message.contains("another process") && message.to_lowercase().contains("writ"),
                "the message says that another process writes the index, got {message:?}"
            );
            assert!(
                !message.contains(directory.to_string_lossy().as_ref()),
                "the message carries no server path (ADR-0101), got {message:?}"
            );
        }
    }

    #[test]
    fn the_lock_of_a_dead_holder_is_free_again() {
        // INV-04: the kernel drops the lock with the process, and dropping the
        // value in this one. A leftover `WRITER_LOCK` file therefore blocks no
        // next writer, and the file itself stays in place (ADR-0116).
        let temp = tempfile::tempdir().expect("a temporary directory is creatable");
        let directory = temp.path().to_path_buf();
        let first =
            WriterLock::acquire(&directory, INDEX).expect("the first writer takes the lock");
        assert!(
            directory.join(WRITER_LOCK_FILE_NAME).is_file(),
            "the lock file lives at the root of the index directory"
        );
        drop(first);
        assert!(
            WriterLock::acquire(&directory, INDEX).is_ok(),
            "a released lock is free, and a leftover file does not refuse the next writer"
        );
    }

    #[test]
    fn a_second_open_of_the_same_index_in_one_process_is_refused() {
        // `flock` is held per open file description, so a second acquisition in
        // the same process is refused exactly like one from another process: the
        // rule is "one writer per index", not "one writer per process".
        let temp = tempfile::tempdir().expect("a temporary directory is creatable");
        let directory = temp.path().to_path_buf();
        let _held =
            WriterLock::acquire(&directory, INDEX).expect("the first writer takes the lock");
        let second = WriterLock::acquire(&directory, INDEX)
            .err()
            .expect("the second acquisition is refused");
        assert_eq!(second.code(), ErrorCode::Overloaded);
    }
}
