// SPDX-License-Identifier: Apache-2.0

//! Exclusive file locks shared by the presence lock of an online batch and the
//! writer lock of an index.
//!
//! References: ADR-0056 (presence lock and the identity check after locking),
//! ADR-0116 (decision 2: `WRITER_LOCK`, exclusive and non-blocking), `docs/02`
//! ("Concurrence").
//!
//! The lock is taken with [`std::fs::File::try_lock`]: exclusive, and never
//! blocking. A lock is not enough on its own, because the path may have been
//! unlinked and recreated between the open and the `try_lock`, so the handle
//! would hold a lock on a file no name points at any more, and two writers would
//! each hold a lock they believe exclusive (ADR-0056). After a successful
//! `try_lock`, the identity of the open file is therefore compared with the
//! identity of the path; on a mismatch the handle is dropped and the open is
//! retried.
//!
//! Both callers describe the file they lock in their own words (`what`), so a
//! failure names it: "open the writer lock of index 'alpha'", "the presence lock
//! of the online batch". Neither message carries a server path (ADR-0101).

use std::fs::{self, File, TryLockError};
use std::path::Path;

use crible_core::error::{CribleError, ErrorCode};

use crate::segment::io_error;

/// Highest number of times the lock is reopened when the path is replaced under
/// the caller. A single replacement is expected; more means something keeps
/// deleting the lock file, and guessing is worse than failing.
const LOCK_ATTEMPTS: usize = 8;

/// Outcome of [`lock_exclusive`] on one lock path.
#[derive(Debug)]
pub(crate) enum LockAttempt {
    /// The exclusive lock is held on the file the path currently names.
    Locked(File),
    /// Another writer holds the lock: the caller must refuse or retry.
    Busy,
}

/// Opens `path` (creating it) and takes an exclusive lock on the **current**
/// file of that path.
///
/// `what` names the lock in the messages, for example `writer lock of index
/// 'alpha'`. A lock held by another process is not a failure: it is
/// [`LockAttempt::Busy`], and the caller decides what the contention means
/// (ADR-0056, ADR-0116).
///
/// # Errors
///
/// [`ErrorCode::Internal`] when the lock file cannot be opened or locked, or
/// when the path keeps being replaced.
pub(crate) fn lock_exclusive(path: &Path, what: &str) -> Result<LockAttempt, CribleError> {
    lock_exclusive_with(path, what, &|| {})
}

/// Body of [`lock_exclusive`], with a hook between the open and the `try_lock`.
///
/// `between` runs after the file is opened and before the lock is attempted. It
/// exists so a test can open the window ADR-0056 describes deterministically: a
/// path replaced between the open and the lock must not yield two writers. The
/// production call passes an empty closure; the hook is the only difference
/// between the two entry points.
fn lock_exclusive_with(
    path: &Path,
    what: &str,
    between: &dyn Fn(),
) -> Result<LockAttempt, CribleError> {
    for _ in 0..LOCK_ATTEMPTS {
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|error| io_error(&format!("open the {what}"), path, &error))?;
        between();
        match file.try_lock() {
            Ok(()) => {
                if names_same_file(&file, path) {
                    return Ok(LockAttempt::Locked(file));
                }
                // The path was replaced under this handle: retry, and let the
                // handle drop at the end of the iteration.
            }
            Err(TryLockError::WouldBlock) => return Ok(LockAttempt::Busy),
            Err(TryLockError::Error(error)) => {
                return Err(CribleError::new(
                    ErrorCode::Internal,
                    format!("cannot lock the {what} {}: {error}", path.display()),
                ));
            }
        }
    }
    Err(CribleError::new(
        ErrorCode::Internal,
        format!(
            "the {what} {} was replaced {LOCK_ATTEMPTS} times while it was being taken; \
             refusing to guess which file it protects",
            path.display()
        ),
    ))
}

/// Tells whether `file` is still the file `path` names.
///
/// The identity is the pair (device, inode) of the two metadata reads, which is
/// how a Unix kernel designates a file. `false` on any fault, including a path
/// that no longer exists, so the caller retries instead of locking a vanished
/// file.
#[cfg(unix)]
pub(crate) fn names_same_file(file: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    let (Ok(held), Ok(current)) = (file.metadata(), fs::metadata(path)) else {
        return false;
    };
    held.dev() == current.dev() && held.ino() == current.ino()
}

/// On a platform without a portable file identity, the check is skipped.
///
/// Crible is deployed on Linux (`docs/07-stack-et-deploiement.md`, Docker), and
/// its tests run on Linux and macOS, where the Unix check above applies.
#[cfg(not(unix))]
pub(crate) fn names_same_file(_file: &File, _path: &Path) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::{LockAttempt, lock_exclusive, lock_exclusive_with, names_same_file};
    use std::fs;

    /// Names a lock in the test messages.
    const WHAT: &str = "writer lock of index 'alpha'";

    #[test]
    fn a_lock_file_recreated_between_the_open_and_the_lock_names_the_current_file() {
        // ADR-0056, ADR-0116 decision 2: "un fichier recréé entre l'ouverture et
        // le verrouillage ne doit pas donner deux verrous". The open and the
        // `try_lock` are not one instant: a handle opened before the path was
        // recreated holds an inode no name points at, and the lock it takes
        // proves nothing. The identity check makes the caller retry, so the lock
        // it returns is always the lock of the file the path names.
        let temp = tempfile::tempdir().expect("a temporary directory is creatable");
        let path = temp.path().join("WRITER_LOCK");
        fs::write(&path, "").expect("the lock file is creatable");
        let replaced = std::cell::Cell::new(false);

        let outcome = lock_exclusive_with(&path, WHAT, &|| {
            // A cleanup that replaced the lock file while the caller had it
            // open, without holding its lock. It happens once: a path replaced at
            // every attempt is the other test.
            if replaced.replace(true) {
                return;
            }
            fs::remove_file(&path).expect("the lock file is removable");
            fs::write(&path, "").expect("a new lock file is creatable");
        })
        .expect("the lock is taken");
        let LockAttempt::Locked(file) = outcome else {
            panic!("the file the path names is free, so the lock is taken");
        };
        assert!(
            names_same_file(&file, &path),
            "the returned lock is held on the file the path names, never on a replaced inode"
        );

        // The stale handle the first attempt opened names no file any more: its
        // lock would have been a second writer pretending to hold the index.
        // The lock the caller keeps is the one of the current file, so a second
        // caller meets it instead of locking a file no name points at.
        assert!(
            matches!(
                lock_exclusive(&path, WHAT).expect("a contention is not an error"),
                LockAttempt::Busy
            ),
            "the current file is held, so no second writer takes it"
        );
    }

    #[test]
    fn a_lock_replaced_once_more_than_the_attempts_is_an_internal_failure() {
        // The retry is bounded: a path that keeps being replaced is a broken
        // file system, not a lock to guess.
        let temp = tempfile::tempdir().expect("a temporary directory is creatable");
        let path = temp.path().join("WRITER_LOCK");
        fs::write(&path, "").expect("the lock file is creatable");
        let failure = lock_exclusive_with(&path, WHAT, &|| {
            fs::remove_file(&path).expect("the lock file is removable");
            fs::write(&path, "").expect("a new lock file is creatable");
        })
        .expect_err("a path replaced at every attempt is refused");
        assert_eq!(
            failure.code(),
            crible_core::error::ErrorCode::Internal,
            "a lock that cannot be trusted is not a contention"
        );
    }

    #[test]
    fn a_held_lock_is_reported_as_busy() {
        let temp = tempfile::tempdir().expect("a temporary directory is creatable");
        let path = temp.path().join("WRITER_LOCK");
        let first = lock_exclusive(&path, WHAT).expect("the first lock is taken");
        assert!(
            matches!(first, LockAttempt::Locked(_)),
            "a free path yields a lock"
        );
        let second = lock_exclusive(&path, WHAT).expect("a contention is not an error");
        assert!(
            matches!(second, LockAttempt::Busy),
            "a held lock is reported as busy, without waiting"
        );
    }
}
