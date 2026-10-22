// SPDX-License-Identifier: Apache-2.0

//! MES-13 bench: startup cost, the filter structures rebuilt from
//! `records.sqlite`, and the parts that make it up.
//!
//! References: `docs/08-invariants-et-tests.md` (MES-13), `docs/02-architecture.md`
//! ("Reprise au démarrage"), `docs/phases/P3-cycle-de-vie.md` (the 60 s startup
//! criterion), `crates/crible-store/src/filters_index.rs`,
//! `crates/crible-store/src/generation.rs` (`GenerationStore::open`,
//! `build_snapshot`).
//!
//! `GenerationStore::open` publishes the snapshot of the committed generation.
//! On the read path it does four things, and only the last one grows with the
//! number of passages:
//!
//! 1. read `CURRENT_EPOCH`, open SQLite, read `meta` and the segment rows;
//! 2. run the recovery of INV-04 (remove `tmp-*` and orphan segments);
//! 3. `FilterIndex::from_records`: one `SELECT` over every row of `passages`
//!    joined to its `documents` row, parse the canonical JSON of each row, then
//!    insert every value into a bitmap or a value column;
//! 4. write the manifest when it is missing.
//!
//! The bench measures the whole `open`, then replays the three parts of step 3
//! on the same database: the SQL read alone, the SQL read plus the JSON decode,
//! and the full build. The difference is the insertion into the bitmaps and the
//! columns. Each replica is labelled as such in the report: a replica measures
//! the cost of a copy of the production code, not of the production code
//! itself.
//!
//! # Modes
//!
//! ```text
//! startup_cost measure --index <dir> --label <name> --repeats <n>
//!                      [--warmup <n>] --commit <sha> --machine <label>
//!                      --date <date> --out <file.json>
//! startup_cost cold --index <dir> --label <name> --work <dir>
//!                   --commit <sha> --machine <label> --date <date>
//!                   --out <file.json>
//! ```
//!
//! `measure` opens the index `--warmup` times without timing, then times the
//! four phases `--repeats` times. `cold` copies the index (`cp -cR`, APFS clone,
//! falling back to `cp -R`) once per phase and times that phase on the fresh
//! copy, so the first read of the first copy is as cold as this bench can make
//! it without `root`: the page cache of macOS is not purged here, and the report
//! says so.
//!
//! # Platform
//!
//! The `cold` mode runs on macOS only (it relies on `cp -c`); `measure` is
//! portable and needs no other tool.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::time::{Duration, Instant};

use crible_core::config::Schema;
use crible_store::filters_index::FilterIndex;
use crible_store::generation::GenerationStore;
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};

/// Query of `FilterIndex::from_records`, copied here because the function takes
/// the connection and keeps the SQL private.
const FILTER_QUERY: &str = "SELECT p.iid, p.fields, d.fields, p.dead_since_gen
     FROM passages p LEFT JOIN documents d ON d.doc_id = p.doc_id
     ORDER BY p.iid";

/// Lock wait every production connection applies (ADR-0038); the replicas set
/// the same one so they read under the same regime.
const BUSY_TIMEOUT_MS: u64 = 1000;

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse(std::env::args().skip(1))?;
    match args.mode.as_str() {
        "measure" => measure(&args),
        "cold" => cold(&args),
        other => Err(fail(format!(
            "unknown mode {other:?}: expected `measure` or `cold`"
        ))),
    }
}

/// Typed failure of the bench, reported as a plain message.
fn fail(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        message.into(),
    ))
}

// ---------------------------------------------------------------------------
// Command line
// ---------------------------------------------------------------------------

/// Parsed command line: a mode and `--key value` options.
struct Args {
    mode: String,
    values: BTreeMap<String, Vec<String>>,
}

impl Args {
    fn parse(raw: impl Iterator<Item = String>) -> Result<Self, Box<dyn Error>> {
        let mut mode = None;
        let mut values: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut tokens = raw;
        while let Some(token) = tokens.next() {
            let Some(key) = token.strip_prefix("--") else {
                if mode.is_some() {
                    return Err(fail(format!("unexpected argument {token:?}")));
                }
                mode = Some(token);
                continue;
            };
            // `cargo bench` appends `--bench` (and `cargo test --benches`
            // appends `--test`): they are flags of the harness protocol, not
            // options of the bench.
            if matches!(key, "bench" | "test") {
                continue;
            }
            let value = tokens
                .next()
                .ok_or_else(|| fail(format!("--{key} expects a value")))?;
            values.entry(key.to_owned()).or_default().push(value);
        }
        let mode = mode.ok_or_else(|| fail("missing mode: `measure` or `cold`"))?;
        Ok(Self { mode, values })
    }

    fn one(&self, key: &str) -> Result<&str, Box<dyn Error>> {
        self.values
            .get(key)
            .and_then(|values| values.first())
            .map(String::as_str)
            .ok_or_else(|| fail(format!("--{key} is required")))
    }

    fn number<T: FromStr>(&self, key: &str) -> Result<T, Box<dyn Error>>
    where
        T::Err: std::fmt::Display,
    {
        let raw = self.one(key)?;
        raw.parse::<T>()
            .map_err(|error| fail(format!("--{key} {raw:?}: {error}")))
    }

    fn number_or<T: FromStr>(&self, key: &str, fallback: T) -> Result<T, Box<dyn Error>>
    where
        T::Err: std::fmt::Display,
    {
        match self.values.get(key).and_then(|values| values.first()) {
            Some(raw) => raw
                .parse::<T>()
                .map_err(|error| fail(format!("--{key} {raw:?}: {error}"))),
            None => Ok(fallback),
        }
    }
}

// ---------------------------------------------------------------------------
// Measurement
// ---------------------------------------------------------------------------

/// One measured phase and its durations, one per repetition, in nanoseconds.
#[derive(Default)]
struct Samples {
    label: &'static str,
    durations: Vec<u64>,
    memory: Option<Memory>,
}

/// Estimated heap size of the filter structures, as `crible inspect` reports it.
#[derive(Default, Clone, Copy)]
struct Memory {
    live: usize,
    keywords: usize,
    booleans: usize,
    columns: usize,
}

impl Memory {
    fn total(self) -> usize {
        self.live + self.keywords + self.booleans + self.columns
    }

    fn to_json(self) -> Value {
        json!({
            "live": self.live,
            "keywords": self.keywords,
            "booleans": self.booleans,
            "columns": self.columns,
            "total": self.total(),
        })
    }
}

/// Warm measurement: `--warmup` untimed opens, then `--repeats` timed passes of
/// every phase.
fn measure(args: &Args) -> Result<(), Box<dyn Error>> {
    let index = PathBuf::from(args.one("index")?);
    let label = args.one("label")?;
    let repeats: usize = args.number("repeats")?;
    let warmup: usize = args.number_or("warmup", 1)?;
    if repeats == 0 {
        return Err(fail("--repeats must be positive"));
    }
    let schema = read_schema(&index)?;
    let records = records_path(&index)?;
    let bytes = fs::metadata(&records)?.len();

    for _ in 0..warmup {
        let started = Instant::now();
        let store = GenerationStore::open(&index, &schema)?;
        drop(store);
        eprintln!(
            "{label}: warm-up open {:.3} s",
            started.elapsed().as_secs_f64()
        );
    }

    let mut sql_read = Samples {
        label: "sql_read",
        ..Samples::default()
    };
    let mut json_decode = Samples {
        label: "json_decode",
        ..Samples::default()
    };
    let mut build = Samples {
        label: "build",
        ..Samples::default()
    };
    let mut open = Samples {
        label: "open",
        ..Samples::default()
    };
    let mut live = 0usize;

    for repetition in 0..repeats {
        let started = Instant::now();
        let scanned = sql_read_phase(&records)?;
        sql_read.durations.push(duration_ns(started.elapsed()));

        let started = Instant::now();
        let parsed = json_decode_phase(&records)?;
        json_decode.durations.push(duration_ns(started.elapsed()));
        if parsed != scanned {
            return Err(fail(format!(
                "the read replica saw {scanned:?} and the decode replica {parsed:?}: \
                 the two passes do not see the same rows"
            )));
        }

        let started = Instant::now();
        let connection = open_records(&records)?;
        let filters = FilterIndex::from_records(&connection, &schema)?;
        let memory = filters.memory();
        drop(filters);
        drop(connection);
        build.durations.push(duration_ns(started.elapsed()));
        build.memory = Some(Memory {
            live: memory.live,
            keywords: memory.keywords,
            booleans: memory.booleans,
            columns: memory.columns,
        });

        let started = Instant::now();
        let store = GenerationStore::open(&index, &schema)?;
        let snapshot = store.snapshot();
        let memory = snapshot.filter_memory();
        live = snapshot.live_iids().count();
        let segments = snapshot.segments().len();
        drop(snapshot);
        drop(store);
        open.durations.push(duration_ns(started.elapsed()));
        open.memory = Some(Memory {
            live: memory.live,
            keywords: memory.keywords,
            booleans: memory.booleans,
            columns: memory.columns,
        });

        eprintln!(
            "{label}: repetition {}/{repeats} sql {:.3} s, decode {:.3} s, build {:.3} s, \
             open {:.3} s ({segments} segments)",
            repetition + 1,
            ns_seconds(sql_read.durations[repetition]),
            ns_seconds(json_decode.durations[repetition]),
            ns_seconds(build.durations[repetition]),
            ns_seconds(open.durations[repetition]),
        );
    }

    let document = json!({
        "label": label,
        "index": index.display().to_string(),
        "records_sqlite": records.display().to_string(),
        "records_sqlite_bytes": bytes,
        "live_passages": live,
        "repeats": repeats,
        "warmup": warmup,
        "cache": "warm",
        "phases": [
            samples_json(&sql_read),
            samples_json(&json_decode),
            samples_json(&build),
            samples_json(&open),
        ],
        "filter_memory": build.memory.unwrap_or_default().to_json(),
    });
    write_out(args, document)
}

/// Cold measurement: one pass of each phase, each on a fresh copy of the index.
///
/// Copying is what the bench can do without `root`: the page cache of macOS is
/// not purged, so the first read of a clone is treated as an approximation of a
/// cold start, not as a controlled cold state.
fn cold(args: &Args) -> Result<(), Box<dyn Error>> {
    let index = PathBuf::from(args.one("index")?);
    let label = args.one("label")?;
    let work = PathBuf::from(args.one("work")?);
    fs::create_dir_all(&work)?;
    let schema = read_schema(&index)?;
    let records = records_path(&index)?;
    let bytes = fs::metadata(&records)?.len();

    let mut phases = Vec::new();
    let mut memory = Memory::default();
    let mut live = 0usize;

    for (phase, run) in [
        ("sql_read", 0usize),
        ("json_decode", 1),
        ("build", 2),
        ("open", 3),
    ] {
        let copy = work.join(format!("cold-{label}-{phase}"));
        if copy.exists() {
            fs::remove_dir_all(&copy)?;
        }
        clone_directory(&index, &copy)?;
        let copied_records = records_path(&copy)?;
        let started = Instant::now();
        match run {
            0 => {
                sql_read_phase(&copied_records)?;
            }
            1 => {
                json_decode_phase(&copied_records)?;
            }
            2 => {
                let connection = open_records(&copied_records)?;
                let filters = FilterIndex::from_records(&connection, &schema)?;
                let measured = filters.memory();
                memory = Memory {
                    live: measured.live,
                    keywords: measured.keywords,
                    booleans: measured.booleans,
                    columns: measured.columns,
                };
                drop(filters);
                drop(connection);
            }
            _ => {
                let store = GenerationStore::open(&copy, &schema)?;
                let snapshot = store.snapshot();
                let measured = snapshot.filter_memory();
                memory = Memory {
                    live: measured.live,
                    keywords: measured.keywords,
                    booleans: measured.booleans,
                    columns: measured.columns,
                };
                live = snapshot.live_iids().count();
                drop(snapshot);
                drop(store);
            }
        }
        let elapsed = duration_ns(started.elapsed());
        eprintln!("{label}: cold {phase} {:.3} s", ns_seconds(elapsed));
        phases.push(json!({
            "phase": phase,
            "first_access_ns": elapsed,
            "copy": copy.display().to_string(),
        }));
        fs::remove_dir_all(&copy)?;
    }

    let document = json!({
        "label": label,
        "index": index.display().to_string(),
        "records_sqlite_bytes": bytes,
        "live_passages": live,
        "cache": "cold (first access of a fresh clone; page cache not purged)",
        "phases": phases,
        "filter_memory": memory.to_json(),
    });
    write_out(args, document)
}

/// Times the SQL read alone: every row is materialised, then dropped.
///
/// The returned pair is the row count and a checksum of the iids and of the
/// lengths read, so the compiler cannot drop the read and a later phase can
/// check that both passes saw the same rows.
fn sql_read_phase(records: &Path) -> Result<(usize, u64), Box<dyn Error>> {
    let connection = open_records(records)?;
    let mut statement = connection.prepare(FILTER_QUERY)?;
    let mut rows = statement.query([])?;
    let mut seen = 0usize;
    let mut checksum = 0u64;
    while let Some(row) = rows.next()? {
        let iid: i64 = row.get(0)?;
        let passage: String = row.get(1)?;
        let document: Option<String> = row.get(2)?;
        let dead: Option<i64> = row.get(3)?;
        checksum = checksum
            .wrapping_add(iid as u64)
            .wrapping_add(passage.len() as u64)
            .wrapping_add(document.as_ref().map_or(0, String::len) as u64)
            .wrapping_add(u64::from(dead.is_some()));
        seen += 1;
    }
    Ok((seen, checksum))
}

/// Times the SQL read and the JSON decode, without any insertion.
fn json_decode_phase(records: &Path) -> Result<(usize, u64), Box<dyn Error>> {
    let connection = open_records(records)?;
    let mut statement = connection.prepare(FILTER_QUERY)?;
    let mut rows = statement.query([])?;
    let mut seen = 0usize;
    let mut checksum = 0u64;
    while let Some(row) = rows.next()? {
        let iid: i64 = row.get(0)?;
        let passage: String = row.get(1)?;
        let document: Option<String> = row.get(2)?;
        let dead: Option<i64> = row.get(3)?;
        let passage_len = passage.len() as u64;
        let document_len = document.as_ref().map_or(0, String::len) as u64;
        let passage: Value = serde_json::from_str(&passage)?;
        let document: Option<Value> = match document {
            Some(raw) => Some(serde_json::from_str(&raw)?),
            None => None,
        };
        // Both replicas add the same raw lengths, so their checksums are
        // comparable; `black_box` keeps the decoded values alive.
        std::hint::black_box((&passage, &document));
        checksum = checksum
            .wrapping_add(iid as u64)
            .wrapping_add(passage_len)
            .wrapping_add(document_len)
            .wrapping_add(u64::from(dead.is_some()));
        seen += 1;
    }
    Ok((seen, checksum))
}

/// Opens the records database the way a production read does (read-write, WAL,
/// busy timeout) without writing anything.
fn open_records(path: &Path) -> Result<Connection, Box<dyn Error>> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    connection.busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))?;
    Ok(connection)
}

/// Schema `index.json` declares, the argument `GenerationStore::open` takes.
fn read_schema(index: &Path) -> Result<Schema, Box<dyn Error>> {
    let raw = fs::read_to_string(index.join("index.json"))?;
    let value: Value = serde_json::from_str(&raw)?;
    let schema = value
        .get("schema")
        .ok_or_else(|| fail("index.json declares no schema"))?;
    Ok(Schema::from_json_str(&schema.to_string())?)
}

/// Path of `records.sqlite` of the current epoch.
fn records_path(index: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let epoch = fs::read_to_string(index.join("CURRENT_EPOCH"))?;
    let epoch = epoch.trim();
    if epoch.is_empty() {
        return Err(fail("CURRENT_EPOCH is empty"));
    }
    Ok(index.join("epochs").join(epoch).join("records.sqlite"))
}

/// Copies an index directory, with an APFS clone when the platform offers it.
fn clone_directory(source: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
    let cloned = Command::new("cp")
        .arg("-cR")
        .arg(source)
        .arg(destination)
        .status()?;
    if cloned.success() {
        return Ok(());
    }
    let copied = Command::new("cp")
        .arg("-R")
        .arg(source)
        .arg(destination)
        .status()?;
    if copied.success() {
        Ok(())
    } else {
        Err(fail(format!(
            "cannot copy {} to {}",
            source.display(),
            destination.display()
        )))
    }
}

/// Writes the JSON document of one invocation.
fn write_out(args: &Args, mut document: Value) -> Result<(), Box<dyn Error>> {
    let out = PathBuf::from(args.one("out")?);
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(object) = document.as_object_mut() {
        object.insert("measure".to_owned(), json!("MES-13"));
        object.insert("commit".to_owned(), json!(args.one("commit")?));
        object.insert("machine".to_owned(), json!(args.one("machine")?));
        object.insert("date".to_owned(), json!(args.one("date")?));
        object.insert(
            "generated_at".to_owned(),
            json!(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |elapsed| elapsed.as_secs())
            ),
        );
    }
    let file = File::create(&out)?;
    serde_json::to_writer_pretty(file, &document)?;
    eprintln!("wrote {}", out.display());
    Ok(())
}

/// JSON of one phase: percentiles by nearest rank, minimum, maximum and every
/// raw value.
fn samples_json(samples: &Samples) -> Value {
    let mut sorted = samples.durations.clone();
    sorted.sort_unstable();
    json!({
        "phase": samples.label,
        "p50_ns": percentile(&sorted, 50),
        "p95_ns": percentile(&sorted, 95),
        "p99_ns": percentile(&sorted, 99),
        "min_ns": sorted.first().copied(),
        "max_ns": sorted.last().copied(),
        "values_ns": samples.durations,
    })
}

/// Nearest-rank percentile: the smallest sample at or above `percent` percent.
fn percentile(sorted: &[u64], percent: usize) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (percent * sorted.len()).div_ceil(100);
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted.get(index).copied()
}

fn duration_ns(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX)
}

fn ns_seconds(nanoseconds: u64) -> f64 {
    nanoseconds as f64 / 1_000_000_000.0
}
