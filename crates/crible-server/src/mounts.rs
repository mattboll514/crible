// SPDX-License-Identifier: Apache-2.0

//! Warning when the data root is a directory shared from the host.
//!
//! References: `docs/07-stack-et-deploiement.md` ("Docker", volumes) and the
//! task `DEFINIR-VOLUMES-ET-CONFIG` (`docs/phases/P8-docker-et-deploiement.md`).
//! `/data` is a named volume because a memory map through the file sharing of
//! macOS into the Docker VM is slow. The startup cannot refuse such a mount, an
//! operator may have reasons, but it says so once so nobody measures a latency
//! caused by the volume type without knowing it.
//!
//! **Detection.** On Linux, `/proc/self/mountinfo` lists the mount points of
//! the container with their filesystem type. Docker Desktop for Mac mounts a
//! host directory with `virtiofs`, `fakeowner`, `grpcfuse`, or another `fuse.*`
//! type; a named volume is the filesystem of the VM (`ext4`). The mount that
//! decides is the deepest one whose mount point contains the data root, which is
//! how the kernel resolves a path. Every other system returns no warning: the
//! question belongs to the Linux container, and the same rule would be wrong on
//! a native Linux host, where a bind mount is an ordinary `ext4` mount.
//!
//! No dependency is added: the file is read as text and its lines are parsed
//! here. The decision is a pure function of the mountinfo lines, tested below on
//! samples.
//!
//! **Confidentiality.** The warning carries the data root and the filesystem
//! type only. It holds no query text and no filter value, as every log line of
//! the server (INV-13, ADR-0096).

use std::path::{Path, PathBuf};

/// Filesystem types with which Docker Desktop for Mac shares a host directory.
///
/// `fuse.*` covers `fuse.grpcfuse`, `fuse.osxfs` and the other FUSE-based
/// shares; the three exact names are the ones a current Docker Desktop reports.
const HOST_SHARE_TYPES: [&str; 3] = ["virtiofs", "fakeowner", "grpcfuse"];

/// Warns when the data root is a directory shared from the host.
///
/// Does nothing on a system other than Linux, and when `/proc/self/mountinfo`
/// cannot be read: a missing file is not a startup fault.
pub(crate) fn warn_if_host_share(data: &Path) {
    let Some(contents) = mountinfo() else {
        return;
    };
    if let Some((mount_point, filesystem)) = host_share(data, &contents) {
        warn_shared(data, &mount_point, &filesystem);
    }
}

/// Emits the warning of a host share.
///
/// A separate function so the test exercises the line of production instead of
/// a copy: a field added here would reach the test.
fn warn_shared(data: &Path, mount_point: &Path, filesystem: &str) {
    tracing::warn!(
        data = %data.display(),
        mount = %mount_point.display(),
        filesystem = %filesystem,
        "the data root is a directory shared from the host; its memory maps will be slow. \
         Mount /data as a named volume (docs/07-stack-et-deploiement.md, section Docker)"
    );
}

/// Contents of the mount table of the process, on Linux.
#[cfg(target_os = "linux")]
fn mountinfo() -> Option<String> {
    std::fs::read_to_string("/proc/self/mountinfo").ok()
}

/// No mount table to read outside a Linux container.
#[cfg(not(target_os = "linux"))]
fn mountinfo() -> Option<String> {
    None
}

/// Decides from the lines of a `mountinfo` file.
///
/// Returns the mount point and the filesystem type of the deepest mount that
/// contains `data`, when that type is a host share of Docker Desktop. A mount
/// point contains `data` when it is a path prefix of it in the sense of
/// [`Path::starts_with`], which compares whole components: `/data-two` is not a
/// parent of `/data`.
fn host_share(data: &Path, mountinfo: &str) -> Option<(PathBuf, String)> {
    let mut deepest: Option<(PathBuf, String)> = None;
    for line in mountinfo.lines() {
        // A line is: id parent major:minor root mount_point options [optional
        // fields] - filesystem source super_options. The separator ` - ` is the
        // only field a mount option could imitate; the left side is split by
        // whitespace before it.
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        let Some(mount_point) = left.split_whitespace().nth(4) else {
            continue;
        };
        let Some(filesystem) = right.split_whitespace().next() else {
            continue;
        };
        if !is_host_share(filesystem) {
            continue;
        }
        let mount_point = PathBuf::from(unescape(mount_point));
        if !data.starts_with(&mount_point) {
            continue;
        }
        let deeper = deepest.as_ref().is_none_or(|(current, _)| {
            mount_point.components().count() > current.components().count()
        });
        if deeper {
            deepest = Some((mount_point, filesystem.to_owned()));
        }
    }
    deepest
}

/// Whether a filesystem type names a host share of Docker Desktop for Mac.
fn is_host_share(filesystem: &str) -> bool {
    HOST_SHARE_TYPES.contains(&filesystem) || filesystem.starts_with("fuse.")
}

/// Decodes the octal escapes of a `mountinfo` field.
///
/// The kernel writes a space, a tab, a newline and a backslash as `\040`,
/// `\011`, `\012` and `\134`; every other byte stands for itself. A sequence
/// whose octal value does not fit a byte (`\400` and above) stays literal: the
/// kernel never writes one, and a panic would turn a malformed table into a
/// startup fault. The result stays valid UTF-8, because it is built from a
/// `str` and ASCII replacements.
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut position = 0;
    while position < bytes.len() {
        if let Some(value) = octal_escape(bytes, position) {
            decoded.push(value);
            position += 4;
        } else {
            if let Some(byte) = bytes.get(position) {
                decoded.push(*byte);
            }
            position += 1;
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// Value of the `\ooo` escape that starts at `position`, when it fits a byte.
///
/// Only the digits `0` to `7` form an escape; a value above 255 is not one.
fn octal_escape(bytes: &[u8], position: usize) -> Option<u8> {
    if bytes.get(position) != Some(&b'\\') {
        return None;
    }
    let digits = bytes.get(position + 1..position + 4)?;
    if !digits.iter().all(|digit| (b'0'..=b'7').contains(digit)) {
        return None;
    }
    let value = digits
        .iter()
        .fold(0_u16, |value, digit| value * 8 + u16::from(digit - b'0'));
    u8::try_from(value).ok()
}

#[cfg(test)]
mod tests {
    use super::{host_share, is_host_share, unescape, warn_shared};
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    /// Mountinfo of a Docker Desktop container: the named volume `ext4`, the
    /// host share `fakeowner`, and a second named volume under `/srv`.
    const MOUNTINFO: &str = "\
22 1 254:1 / / rw,relatime - ext4 /dev/vda1 rw,discard
180 171 254:1 /docker/volumes/crible-data/_data /data rw,relatime master:1 - ext4 /dev/vda1 rw,discard
181 171 0:45 /Users/matt/data /models rw,nosuid,nodev,relatime - fakeowner /run/host_mark/Users rw,fakeowner
190 171 0:46 /Users/matt/exchange /exchange rw,nosuid,nodev,relatime - virtiofs osxfs rw
200 171 254:1 /srv /srv rw,relatime - fuse.grpcfuse grpcfuse rw
";

    /// In-memory writer that captures the journal of a test.
    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Sink {
        fn contents(&self) -> String {
            self.0
                .lock()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .unwrap_or_default()
        }
    }

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
    fn a_named_volume_is_not_a_host_share() {
        // The `/data` line of the fixture is an `ext4` named volume: the kernel
        // resolves it before the root mount.
        assert_eq!(host_share(Path::new("/data"), MOUNTINFO), None);
        assert_eq!(
            host_share(Path::new("/data/indexes/alpha"), MOUNTINFO),
            None
        );
    }

    #[test]
    fn a_host_share_of_docker_desktop_is_detected() {
        // The three types the Mac container reports, and the deepest mount
        // point, which is not the root mount.
        assert_eq!(
            host_share(Path::new("/models"), MOUNTINFO),
            Some((PathBuf::from("/models"), "fakeowner".to_owned()))
        );
        assert_eq!(
            host_share(Path::new("/models/mdenseon"), MOUNTINFO),
            Some((PathBuf::from("/models"), "fakeowner".to_owned()))
        );
        assert_eq!(
            host_share(Path::new("/exchange/inbox"), MOUNTINFO),
            Some((PathBuf::from("/exchange"), "virtiofs".to_owned()))
        );
        assert_eq!(
            host_share(Path::new("/srv/batches"), MOUNTINFO),
            Some((PathBuf::from("/srv"), "fuse.grpcfuse".to_owned()))
        );
    }

    #[test]
    fn the_deepest_mount_wins() {
        // A host share mounted inside an `ext4` volume decides for the paths
        // under it, and not for the rest of the volume.
        let contents = "\
22 1 254:1 / / rw - ext4 /dev/vda1 rw
180 171 254:1 /docker/volumes/crible-data/_data /data rw - ext4 /dev/vda1 rw
181 180 0:45 /Users/matt /data/import rw - virtiofs Users rw
";
        assert_eq!(
            host_share(Path::new("/data/import/batch-1"), contents),
            Some((PathBuf::from("/data/import"), "virtiofs".to_owned()))
        );
        assert_eq!(host_share(Path::new("/data/indexes"), contents), None);
    }

    #[test]
    fn a_path_prefix_that_is_not_a_parent_is_not_a_mount() {
        // `/data-two` shares three characters with `/data`, not a component.
        assert_eq!(host_share(Path::new("/data-two"), MOUNTINFO), None);
        assert_eq!(host_share(Path::new("/data"), MOUNTINFO), None);
    }

    #[test]
    fn a_root_share_is_detected() {
        let contents = "22 1 0:45 / / rw - virtiofs Users rw\n";
        assert_eq!(
            host_share(Path::new("/data"), contents),
            Some((PathBuf::from("/"), "virtiofs".to_owned()))
        );
    }

    #[test]
    fn a_line_the_kernel_did_not_write_is_ignored() {
        // A truncated line, a line without the separator, and a line whose
        // left side is too short must not stop the scan.
        let contents = "\
not a mountinfo line
180 171 254:1 /data rw - ext4 /dev/vda1 rw
181 171
";
        assert_eq!(host_share(Path::new("/data"), contents), None);
    }

    #[test]
    fn every_host_share_type_is_recognized() {
        for filesystem in ["virtiofs", "fakeowner", "grpcfuse", "fuse.osxfs"] {
            assert!(is_host_share(filesystem), "{filesystem} is a host share");
        }
        for filesystem in ["ext4", "xfs", "overlay", "tmpfs", "fusefs"] {
            assert!(
                !is_host_share(filesystem),
                "{filesystem} is not a host share"
            );
        }
    }

    #[test]
    fn an_escaped_mount_point_is_decoded() {
        // The kernel escapes a space as `\040`; the comparison is on the real
        // path, so `/data/my\040indexes` is `/data/my indexes`.
        assert_eq!(unescape("/data/my\\040indexes"), "/data/my indexes");
        assert_eq!(unescape("/data\\134name"), "/data\\name");
        assert_eq!(unescape("/data"), "/data");
        let contents = "180 171 254:1 /x /data/my\\040indexes rw - virtiofs Users rw\n";
        assert_eq!(
            host_share(Path::new("/data/my indexes/alpha"), contents),
            Some((PathBuf::from("/data/my indexes"), "virtiofs".to_owned()))
        );
    }

    #[test]
    fn a_sequence_that_does_not_fit_a_byte_stays_literal() {
        // Only `\000` to `\377` are escapes. `\777` would be 511 and `\400`
        // would be 256: both stay as the four characters the field holds, so a
        // malformed table cannot panic the startup in the debug profile.
        for field in ["/data\\777name", "/data\\400name", "/data\\999name"] {
            assert_eq!(unescape(field), field);
        }
        // A sequence the kernel does not write but that fits a byte is decoded.
        assert_eq!(unescape("/data\\101name"), "/dataAname");
    }

    #[test]
    fn the_warning_carries_the_path_and_the_filesystem_only() {
        // INV-13 and ADR-0096: a log line of the server holds no request text
        // and no filter value. The two fields allowed here are the data root
        // and the filesystem type. The line comes from `warn_shared`, the
        // function `warn_if_host_share` calls, so a field added to production is
        // seen here.
        let contents = "180 171 254:1 /Users/matt/data /data rw - fakeowner Users rw\n";
        let (mount_point, filesystem) =
            host_share(Path::new("/data/indexes"), contents).expect("the fixture is a host share");
        let sink = Sink::default();
        let subscriber = crate::logging::log_subscriber(sink.clone(), "warn")
            .expect("the fixed filter of the test is valid");
        tracing::subscriber::with_default(subscriber, || {
            warn_shared(Path::new("/data"), &mount_point, &filesystem);
        });
        let line = sink.contents();
        assert!(line.contains("\"data\":\"/data\""), "got {line}");
        assert!(line.contains("\"filesystem\":\"fakeowner\""), "got {line}");
        assert!(!line.contains("query"), "got {line}");
        assert!(!line.contains("filter"), "got {line}");
    }
}
