// SPDX-License-Identifier: Apache-2.0

//! The `memory` journal line of the server (ADR-0151).
//!
//! References: ADR-0151 (the memory figure that counts, its breakdown and the
//! line), `docs/06-observabilite.md` ("Mémoire"), ADR-0077, decision 5 (RSS in
//! three states, anonymous and file-backed memory apart) and ADR-0135,
//! decisions 5 and 7 (`memory.current` and `memory.stat` next to the RSS).
//!
//! **What it reads.** `/proc/self/status` gives `VmRSS`, `RssAnon` and `RssFile`
//! of the process, in `kB`; the cgroup v2 gives `memory.current` and the `anon`
//! and `file` entries of `memory.stat`, in bytes. The cgroup files are the ones
//! of `/sys/fs/cgroup`, with the fallback on the cgroup of the process read in
//! `/proc/self/cgroup` when the container is not at the root. No dependency is
//! added: every file is read as text by `std::fs` and parsed here, as
//! `mounts.rs` does for `/proc/self/mountinfo`.
//!
//! **Absent rather than zero.** A source that cannot be read, or a key a kernel
//! does not write, leaves its field out of the line. It is never an error and
//! never a `0`, which would be an invented state (ADR-0119, decision 3): no
//! startup, no request and no job fails because of this line.
//!
//! **Linux only.** The reading of the files and the emission of the line are
//! under `#[cfg(target_os = "linux")]`; the text parsers stay platform-free, so
//! their tests run on the macOS development machine and in the Linux CI. No
//! line is emitted anywhere else (ADR-0151, decision 2).
//!
//! **Confidentiality.** The line carries numbers, a job identifier, a job kind,
//! an index name and a step, and nothing else. It holds no query text, no filter
//! value and no internal identifier, as every log line of the server (INV-13,
//! ADR-0096).

#[cfg(any(target_os = "linux", test))]
use tracing::info;

/// Step of a `memory` line: the closed list of ADR-0151, decision 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// End of a job, whatever its kind and its outcome.
    JobEnd,
    /// In `delete_index`, just before the directory is renamed to quarantine.
    DeleteBeforeRename,
    /// In `delete_index`, after the quarantined directory was removed.
    DeleteAfterRemoval,
    /// When an ONNX session was loaded, for the index that triggered it.
    SessionLoad,
}

impl Step {
    /// Wire name of the step, as ADR-0151 writes it.
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::JobEnd => "job_end",
            Self::DeleteBeforeRename => "delete_before_rename",
            Self::DeleteAfterRemoval => "delete_after_removal",
            Self::SessionLoad => "session_load",
        }
    }
}

/// What a `memory` line names beside the numbers.
///
/// `job_id`, `kind` and `index` follow ADR-0149; they are optional because a
/// step does not always have all three.
pub(crate) struct Point<'a> {
    step: Step,
    job_id: Option<&'a str>,
    kind: Option<&'a str>,
    index: Option<&'a str>,
}

impl<'a> Point<'a> {
    /// End of a job: its identifier, its kind and its index.
    #[must_use]
    pub(crate) const fn job_end(job_id: &'a str, kind: &'a str, index: &'a str) -> Self {
        Self {
            step: Step::JobEnd,
            job_id: Some(job_id),
            kind: Some(kind),
            index: Some(index),
        }
    }

    /// One of the two steps of a `delete_index` job.
    #[must_use]
    pub(crate) const fn delete(step: Step, job_id: &'a str, kind: &'a str, index: &'a str) -> Self {
        Self {
            step,
            job_id: Some(job_id),
            kind: Some(kind),
            index: Some(index),
        }
    }

    /// Load of an ONNX session, named by the index that triggered the load.
    #[must_use]
    pub(crate) const fn session_load(index: &'a str) -> Self {
        Self {
            step: Step::SessionLoad,
            job_id: None,
            kind: None,
            index: Some(index),
        }
    }
}

/// Bytes read from the process and from its cgroup v2, each absent when its
/// source could not be read.
#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Sample {
    rss_bytes: Option<u64>,
    rss_anon_bytes: Option<u64>,
    rss_file_bytes: Option<u64>,
    cgroup_current_bytes: Option<u64>,
    cgroup_anon_bytes: Option<u64>,
    cgroup_file_bytes: Option<u64>,
}

/// Emits the `memory` line of `point` with the memory of the process and of its
/// cgroup, on Linux only.
///
/// Nothing is read and nothing is emitted on another system: there is no
/// `/proc` and no cgroup v2 in the sense of the Linux kernel (ADR-0151).
pub(crate) fn log(point: Point<'_>) {
    #[cfg(target_os = "linux")]
    {
        emit(&read(), &point);
    }
    #[cfg(not(target_os = "linux"))]
    {
        // The fields are still consumed, so the type keeps one shape on every
        // platform without an unused-field warning; no line is emitted.
        let _ = (point.step.as_str(), point.job_id, point.kind, point.index);
    }
}

/// Emits the line with an already read sample.
///
/// A separate function so the test exercises the line of production instead of
/// a copy: a field added here would reach the test.
#[cfg(any(target_os = "linux", test))]
fn emit(sample: &Sample, point: &Point<'_>) {
    info!(
        rss_bytes = sample.rss_bytes,
        rss_anon_bytes = sample.rss_anon_bytes,
        rss_file_bytes = sample.rss_file_bytes,
        cgroup_current_bytes = sample.cgroup_current_bytes,
        cgroup_anon_bytes = sample.cgroup_anon_bytes,
        cgroup_file_bytes = sample.cgroup_file_bytes,
        job_id = point.job_id,
        kind = point.kind,
        index = point.index,
        step = point.step.as_str(),
        "memory"
    );
}

/// Reads the process and the cgroup of the running process, on Linux.
#[cfg(target_os = "linux")]
fn read() -> Sample {
    let status = std::fs::read_to_string("/proc/self/status").ok();
    let directory = cgroup_directory_of_process();
    let current = std::fs::read_to_string(directory.join("memory.current")).ok();
    let stat = std::fs::read_to_string(directory.join("memory.stat")).ok();
    sample_from(status.as_deref(), current.as_deref(), stat.as_deref())
}

/// Directory of the cgroup v2 files of the process, on Linux.
///
/// **Why `memory.current` is the probe.** The kernel writes `memory.current`
/// only on a cgroup that is not the root of its hierarchy; the root cgroup has
/// none. So its presence in `/sys/fs/cgroup` means the visible root *is* the
/// cgroup of the process, which is what a private cgroup namespace shows
/// (`/proc/self/cgroup` then names `0::/`). Otherwise the process lives below
/// the visible root — a native Linux host without a container, where
/// `/proc/self/cgroup` names `0::/system.slice/<service>`, and a container with
/// `--cgroupns=host`, where it names `0::/docker/<id>` — and the directory
/// follows `/proc/self/cgroup`, under `/sys/fs/cgroup`. `memory.stat`, by
/// contrast, exists on the root cgroup too: it would always be found and cannot
/// serve as the probe.
#[cfg(target_os = "linux")]
fn cgroup_directory_of_process() -> std::path::PathBuf {
    let root = std::path::Path::new("/sys/fs/cgroup");
    if root.join("memory.current").is_file() {
        return root.to_path_buf();
    }
    std::fs::read_to_string("/proc/self/cgroup")
        .ok()
        .and_then(|contents| cgroup_directory(&contents).map(str::to_owned))
        .map(|directory| root.join(directory.trim_start_matches('/')))
        .unwrap_or_else(|| root.to_path_buf())
}

/// Path of the cgroup v2 of the process, from the lines of `/proc/self/cgroup`.
///
/// A line is `hierarchy-id controllers path`; the cgroup v2 line has an empty
/// controller list, as in `0::/crible`. A file without such a line gives
/// `None`.
#[cfg(any(target_os = "linux", test))]
fn cgroup_directory(contents: &str) -> Option<&str> {
    contents
        .lines()
        .find_map(|line| line.strip_prefix("0::").filter(|path| !path.is_empty()))
}

/// Combines the three sources into one sample, each one optional.
#[cfg(any(target_os = "linux", test))]
fn sample_from(status: Option<&str>, current: Option<&str>, stat: Option<&str>) -> Sample {
    let mut sample = status.map_or_else(Sample::default, parse_status);
    if let Some(current) = current {
        sample.cgroup_current_bytes = parse_memory_current(current);
    }
    if let Some(stat) = stat {
        let (anon, file) = parse_memory_stat(stat);
        sample.cgroup_anon_bytes = anon;
        sample.cgroup_file_bytes = file;
    }
    sample
}

/// Reads `VmRSS`, `RssAnon` and `RssFile` from the lines of `/proc/self/status`.
///
/// The kernel writes these values in `kB`; they are converted to bytes. A
/// missing key or a malformed line leaves its field absent.
#[cfg(any(target_os = "linux", test))]
fn parse_status(contents: &str) -> Sample {
    let mut sample = Sample::default();
    for line in contents.lines() {
        let Some((key, _)) = line.split_once(':') else {
            continue;
        };
        match key.trim() {
            "VmRSS" => sample.rss_bytes = kilobytes(line),
            "RssAnon" => sample.rss_anon_bytes = kilobytes(line),
            "RssFile" => sample.rss_file_bytes = kilobytes(line),
            _ => {}
        }
    }
    sample
}

/// Value of a `status` line in bytes, when it is a whole number of `kB`.
///
/// A line whose value is not a number, whose unit is not `kB`, or that carries
/// another token is malformed: it yields `None` rather than a made-up number.
#[cfg(any(target_os = "linux", test))]
fn kilobytes(line: &str) -> Option<u64> {
    let (_, value) = line.split_once(':')?;
    let mut parts = value.split_whitespace();
    let number = parts.next()?.parse::<u64>().ok()?;
    if parts.next() != Some("kB") || parts.next().is_some() {
        return None;
    }
    Some(number.saturating_mul(1024))
}

/// Reads the `anon` and `file` entries of `/sys/fs/cgroup/memory.stat`.
///
/// The kernel writes these values in bytes. A missing key or a malformed line
/// leaves its field absent.
#[cfg(any(target_os = "linux", test))]
fn parse_memory_stat(contents: &str) -> (Option<u64>, Option<u64>) {
    let mut anon = None;
    let mut file = None;
    for line in contents.lines() {
        let mut parts = line.split_whitespace();
        let Some(key) = parts.next() else {
            continue;
        };
        let Some(value) = parts.next().and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        if parts.next().is_some() {
            continue;
        }
        match key {
            "anon" => anon = Some(value),
            "file" => file = Some(value),
            _ => {}
        }
    }
    (anon, file)
}

/// Value of `/sys/fs/cgroup/memory.current`, in bytes.
///
/// The file holds one whole number and a newline; anything else yields `None`.
#[cfg(any(target_os = "linux", test))]
fn parse_memory_current(contents: &str) -> Option<u64> {
    contents.trim().parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::{
        Point, Sample, Step, cgroup_directory, emit, log, parse_memory_current, parse_memory_stat,
        parse_status, sample_from,
    };
    use std::io;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    /// Output of `/proc/self/status` on a Linux container, values in `kB`.
    const STATUS: &str = "\
Name:\tcrible-server
VmPeak:\t 1048576 kB
VmRSS:\t    2048 kB
RssAnon:\t    1536 kB
RssFile:\t     512 kB
RssShmem:\t       0 kB
Threads:\t12
";

    /// Output of `/sys/fs/cgroup/memory.stat`, values in bytes.
    const MEMORY_STAT: &str = "\
anon 1048576
file 2097152
kernel 40960
slab 8192
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
    fn the_status_sample_converts_kilobytes_to_bytes() {
        let sample = parse_status(STATUS);
        assert_eq!(sample.rss_bytes, Some(2048 * 1024));
        assert_eq!(sample.rss_anon_bytes, Some(1536 * 1024));
        assert_eq!(sample.rss_file_bytes, Some(512 * 1024));
        // The cgroup fields do not come from this file.
        assert_eq!(sample.cgroup_current_bytes, None);
        assert_eq!(sample.cgroup_anon_bytes, None);
        assert_eq!(sample.cgroup_file_bytes, None);
    }

    #[test]
    fn a_missing_key_of_status_leaves_its_field_absent() {
        let contents = "Name:\tcrible-server\nVmRSS:\t    2048 kB\nThreads:\t12\n";
        let sample = parse_status(contents);
        assert_eq!(sample.rss_bytes, Some(2048 * 1024));
        assert_eq!(sample.rss_anon_bytes, None);
        assert_eq!(sample.rss_file_bytes, None);
    }

    #[test]
    fn a_malformed_line_of_status_leaves_its_field_absent() {
        // A missing value, a value that is not a number, a unit that is not
        // `kB`, and an extra token: four malformed lines, no invented number.
        let contents = "\
VmRSS:
RssAnon:\t abc kB
RssFile:\t 512 MB
RssShmem:\t 0 kB extra
";
        let sample = parse_status(contents);
        assert_eq!(sample.rss_bytes, None);
        assert_eq!(sample.rss_anon_bytes, None);
        assert_eq!(sample.rss_file_bytes, None);
    }

    #[test]
    fn the_memory_stat_sample_reads_anon_and_file_in_bytes() {
        let (anon, file) = parse_memory_stat(MEMORY_STAT);
        assert_eq!(anon, Some(1_048_576));
        assert_eq!(file, Some(2_097_152));
    }

    #[test]
    fn a_missing_or_malformed_line_of_memory_stat_leaves_its_field_absent() {
        let contents = "\
anon abc
file
kernel 40960
slab 8192 extra
";
        let (anon, file) = parse_memory_stat(contents);
        assert_eq!(anon, None);
        assert_eq!(file, None);
    }

    #[test]
    fn memory_current_is_one_whole_number() {
        assert_eq!(parse_memory_current("4194304\n"), Some(4_194_304));
        assert_eq!(parse_memory_current("  4194304  "), Some(4_194_304));
        assert_eq!(parse_memory_current("4194304 8192"), None);
        assert_eq!(parse_memory_current("max\n"), None);
    }

    #[test]
    fn an_unreadable_source_leaves_its_fields_absent() {
        // No status, no memory.current and no memory.stat: every field is
        // absent, never a zero.
        let sample = sample_from(None, None, None);
        assert_eq!(sample, Sample::default());
        // One unreadable source does not hide the two others.
        let sample = sample_from(Some(STATUS), None, Some(MEMORY_STAT));
        assert_eq!(sample.rss_bytes, Some(2048 * 1024));
        assert_eq!(sample.cgroup_current_bytes, None);
        assert_eq!(sample.cgroup_anon_bytes, Some(1_048_576));
        assert_eq!(sample.cgroup_file_bytes, Some(2_097_152));
    }

    #[test]
    fn the_cgroup_of_the_process_is_read_from_proc_self_cgroup() {
        let contents = "0::/crible\n";
        assert_eq!(cgroup_directory(contents), Some("/crible"));
        assert_eq!(cgroup_directory("0::/\n"), Some("/"));
        assert_eq!(cgroup_directory("2:cpu:/crible\n1:memory:/crible\n"), None);
        assert_eq!(cgroup_directory(""), None);
        assert_eq!(cgroup_directory("0::\n"), None);
    }

    #[test]
    fn the_line_carries_the_sample_and_the_point_of_the_job_end() {
        let sample = Sample {
            rss_bytes: Some(4096),
            rss_anon_bytes: Some(2048),
            rss_file_bytes: Some(2048),
            cgroup_current_bytes: Some(8192),
            cgroup_anon_bytes: Some(4096),
            cgroup_file_bytes: Some(4096),
        };
        let point = Point::job_end("job_0123456789abcdef", "attach_batch", "legifacile");
        let line = captured(&point, &sample);
        assert!(line.contains("\"message\":\"memory\""), "got {line}");
        assert!(line.contains("\"level\":\"INFO\""), "got {line}");
        assert!(line.contains("\"step\":\"job_end\""), "got {line}");
        assert!(
            line.contains("\"job_id\":\"job_0123456789abcdef\""),
            "got {line}"
        );
        assert!(line.contains("\"kind\":\"attach_batch\""), "got {line}");
        assert!(line.contains("\"index\":\"legifacile\""), "got {line}");
        assert!(line.contains("\"rss_bytes\":4096"), "got {line}");
    }

    #[test]
    fn the_four_steps_are_the_closed_list() {
        let steps = [
            Step::JobEnd,
            Step::DeleteBeforeRename,
            Step::DeleteAfterRemoval,
            Step::SessionLoad,
        ];
        let names: Vec<&str> = steps.iter().map(|step| step.as_str()).collect();
        assert_eq!(
            names,
            [
                "job_end",
                "delete_before_rename",
                "delete_after_removal",
                "session_load"
            ]
        );
    }

    #[test]
    fn inv_13_the_memory_line_carries_numbers_and_the_planned_identifiers_only() {
        // INV-13 and ADR-0096: a log line of the server holds no request text
        // and no filter value. The line carries the six numeric fields that
        // were read, the message, and the identifiers of the point, nothing
        // else: an absent field is missing from the object, not rendered, and
        // two of the six numbers are absent here so the shape is exercised.
        let sample = Sample {
            rss_bytes: Some(1024),
            rss_anon_bytes: Some(2048),
            rss_file_bytes: None,
            cgroup_current_bytes: Some(4096),
            cgroup_anon_bytes: None,
            cgroup_file_bytes: Some(8192),
        };
        let point = Point::delete(
            Step::DeleteAfterRemoval,
            "job_0123456789abcdef",
            "delete_index",
            "legifacile",
        );
        let line = captured(&point, &sample);
        let value: serde_json::Value =
            serde_json::from_str(line.trim()).expect("the line is one JSON object");
        let fields = value
            .get("fields")
            .and_then(serde_json::Value::as_object)
            .expect("the line carries its fields");
        let mut keys: Vec<&str> = fields.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "cgroup_current_bytes",
                "cgroup_file_bytes",
                "index",
                "job_id",
                "kind",
                "message",
                "rss_anon_bytes",
                "rss_bytes",
                "step"
            ],
            "got {line}"
        );
        assert_eq!(fields["message"], "memory");
        assert_eq!(fields["rss_bytes"], 1024);
        assert_eq!(fields["rss_anon_bytes"], 2048);
        assert_eq!(fields["cgroup_current_bytes"], 4096);
        assert_eq!(fields["cgroup_file_bytes"], 8192);
        assert_eq!(fields["job_id"], "job_0123456789abcdef");
        assert_eq!(fields["kind"], "delete_index");
        assert_eq!(fields["index"], "legifacile");
        assert_eq!(fields["step"], "delete_after_removal");
        // Nothing of a request or of a filter can appear: they have no field.
        assert!(!line.contains("query"), "got {line}");
        assert!(!line.contains("filter"), "got {line}");
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn no_line_is_emitted_outside_linux() {
        // ADR-0151, decision 2: the module emits nothing where there is no
        // `/proc` and no cgroup v2 of the Linux kernel.
        let sink = Sink::default();
        let subscriber = crate::logging::log_subscriber(sink.clone(), "info")
            .expect("the fixed filter of the test is valid");
        tracing::subscriber::with_default(subscriber, || {
            log(Point::job_end(
                "job_0123456789abcdef",
                "attach_batch",
                "legifacile",
            ));
            log(Point::delete(
                Step::DeleteBeforeRename,
                "job_0123456789abcdef",
                "delete_index",
                "legifacile",
            ));
            log(Point::session_load("legifacile"));
        });
        assert!(sink.contents().is_empty(), "got {}", sink.contents());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_line_is_emitted_on_linux_with_the_read_sample() {
        // The sources of a normal Linux process are readable: the line names
        // the step and its job, and carries the six numeric fields.
        let sink = Sink::default();
        let subscriber = crate::logging::log_subscriber(sink.clone(), "info")
            .expect("the fixed filter of the test is valid");
        tracing::subscriber::with_default(subscriber, || {
            log(Point::job_end(
                "job_0123456789abcdef",
                "compact",
                "legifacile",
            ));
        });
        let line = sink.contents();
        assert!(line.contains("\"message\":\"memory\""), "got {line}");
        assert!(line.contains("\"step\":\"job_end\""), "got {line}");
        assert!(line.contains("\"rss_bytes\":"), "got {line}");
        assert!(line.contains("\"rss_anon_bytes\":"), "got {line}");
        assert!(line.contains("\"rss_file_bytes\":"), "got {line}");
    }

    /// Captures the line `emit` writes for `point` and `sample`.
    fn captured(point: &Point<'_>, sample: &Sample) -> String {
        let sink = Sink::default();
        let subscriber = crate::logging::log_subscriber(sink.clone(), "info")
            .expect("the fixed filter of the test is valid");
        tracing::subscriber::with_default(subscriber, || emit(sample, point));
        sink.contents()
    }
}
