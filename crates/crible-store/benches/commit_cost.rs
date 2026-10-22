// SPDX-License-Identifier: Apache-2.0

//! MES-15 bench: cost of a generation commit as a function of the number of
//! live passages `N` and of the number of segments `S`.
//!
//! References: `docs/08-invariants-et-tests.md` (MES-15), `docs/02-architecture.md`
//! ("Générations et point de commit"), `ADR-0016`, `crates/crible-store/src/
//! generation.rs` (`GenerationStore::commit`, `build_snapshot`).
//!
//! `commit` does three things whose cost grows with the size of the index:
//!
//! 1. one SQLite transaction writes the change (the commit point, ADR-0016);
//! 2. `build_snapshot_after_commit` derives the filter structures of the new
//!    generation in copy on write, reads the live iids back from the bitmap of
//!    the live passages and re-reads the `segment.json` of **every segment**
//!    (the task `CONSTRUIRE-BITMAPS`);
//! 3. `manifests/<G>.json` is written, fsynced and renamed.
//!
//! The bench measures (1)+(2)+(3) end to end, then isolates each part on the
//! same on-disk state:
//!
//! - the transaction is replayed with the very SQL of `records.rs`, on a copy of
//!   the index (the real commit is then measured on another copy);
//! - the live-iid read is replayed with the query of `RecordStore::live_iids`;
//! - the `segment.json` read is replayed with the logic of `segment::read_meta`
//!   (that function is `pub(crate)`, so the bench cannot call it);
//! - the manifest write is replayed with the atomic write of `generation.rs`.
//!
//! Every replica is labelled as such in the report: a replica measures the cost
//! of a copy of the production code, not of the production code itself. The
//! `N` and `S` sweeps are the second, independent decomposition: the slope in
//! `N` at fixed `S` is the per-passage part, the slope in `S` at fixed `N` is
//! the per-segment part.
//!
//! # Modes
//!
//! ```text
//! commit_cost prepare --index <dir> --passages <N> --segments <S>
//!                     [--dims 768] [--vector-files sparse|empty] [--overwrite]
//! commit_cost measure --index <dir> [--index <dir> ...] --work <dir>
//!                     --repeats <n> [--new 2000] [--tombstones 50]
//!                     --commit <sha> --machine <label> --out <file.json>
//! ```
//!
//! # Why the reference state is loaded by the bench, not by `commit`
//!
//! Preparing the base index with `commit` would cost O(N) per chunk: building
//! 1 000 segments at 8,58 M passages would rebuild the snapshot 1 000 times.
//! The bench therefore writes the rows of the reference state directly, with the
//! schema, the canonical JSON and the zstd frames of `records.rs`, then verifies
//! them through `GenerationStore::open`. The base index is exactly what a commit
//! of that content would leave on disk; only the preparation path differs.
//!
//! # Why the commits run on copies
//!
//! A commit adds passages and tombstones, so repeating it on the same index
//! changes `N` between repetitions. Each repetition therefore clones the base
//! index (APFS `cp -c`, falling back to `cp -R`) and commits once, so every
//! repetition sees the same `N` and the same `S`.
//!
//! # Overwriting a base index
//!
//! `prepare` refuses a non-empty `--index` target unless `--overwrite` is given,
//! as `tools/gen_synth.py` does for its output directory: a typo in the path must
//! not delete an arbitrary directory.
//!
//! # Platform
//!
//! This bench runs on macOS only. It relies on `cp -c` (APFS clone), on
//! `ps -o rss=`, and the driver `tools/bench/mes15/run.sh` relies on
//! `/usr/bin/time -l`. The crate compiles it on any target, and CI does compile
//! it, but no test runs it: it needs several GB of disk and several minutes.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::time::{Duration, Instant};

use crible_core::config::{ReferencePrecision, Schema};
use crible_core::ids::{Epoch, Generation, Iid};
use crible_store::generation::{GenerationChange, GenerationStore, write_index_config};
use crible_store::records::{DocumentRecord, PassageRecord};
use crible_store::segment::{SegmentSpec, SegmentWriter};
use rusqlite::{Connection, OpenFlags, params};
use serde_json::{Value, json};

/// Number of warm inner runs of a replicated read; the median of them is
/// reported. Each replica also runs once more, before these, and that first run
/// is reported on its own as the first-access cost.
const REPLICA_RUNS: usize = 3;

/// Size of the pool of passage texts used by the bulk loader. The loader reuses
/// compressed frames so that preparing 8,58 M passages does not pay one zstd
/// call per row; the frames have the size of a real article passage.
const TEXT_POOL: usize = 1024;

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse(std::env::args().skip(1))?;
    match args.mode.as_str() {
        "prepare" => prepare(&args),
        "measure" => measure(&args),
        other => Err(fail(format!(
            "unknown mode {other:?}: expected `prepare` or `measure`"
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

/// Options that carry no value: their presence enables them.
const FLAGS: [&str; 1] = ["overwrite"];

/// Parsed command line: a mode and `--key value` options, values kept in order
/// so `--index` can be repeated.
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
            let (key, value) = match key.split_once('=') {
                Some((key, value)) => (key.to_owned(), Some(value.to_owned())),
                None if FLAGS.contains(&key) => (key.to_owned(), None),
                None => {
                    let value = tokens
                        .next()
                        .ok_or_else(|| fail(format!("--{key} expects a value")))?;
                    (key.to_owned(), Some(value))
                }
            };
            if FLAGS.contains(&key.as_str()) && value.is_some() {
                return Err(fail(format!("--{key} takes no value")));
            }
            let entry = values.entry(key).or_default();
            if let Some(value) = value {
                entry.push(value);
            }
        }
        let mode = mode.ok_or_else(|| fail("missing mode: `prepare` or `measure`"))?;
        Ok(Self { mode, values })
    }

    fn flag(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    fn one(&self, key: &str) -> Result<&str, Box<dyn Error>> {
        let values = self
            .values
            .get(key)
            .ok_or_else(|| fail(format!("--{key} is required")))?;
        values
            .first()
            .map(String::as_str)
            .ok_or_else(|| fail(format!("--{key} is required")))
    }

    fn one_or<'a>(&'a self, key: &str, fallback: &'a str) -> &'a str {
        self.values
            .get(key)
            .and_then(|values| values.first())
            .map_or(fallback, String::as_str)
    }

    fn all(&self, key: &str) -> Vec<&str> {
        self.values
            .get(key)
            .map(|values| values.iter().map(String::as_str).collect())
            .unwrap_or_default()
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
// Preparation of the base index
// ---------------------------------------------------------------------------

/// Writes the base index: `N` live passages spread over `S` segments, at
/// generation 1, ready for the measured commit to publish generation 2.
fn prepare(args: &Args) -> Result<(), Box<dyn Error>> {
    let index = PathBuf::from(args.one("index")?);
    let passages: u32 = args.number("passages")?;
    let segments: u32 = args.number("segments")?;
    let dims: u32 = args.number_or("dims", 768)?;
    let vector_files = args.one_or("vector-files", "sparse");
    if passages == 0 || segments == 0 {
        return Err(fail("--passages and --segments must be positive"));
    }
    if segments > passages {
        return Err(fail("--segments must not exceed --passages"));
    }
    let sparse = match vector_files {
        "sparse" => true,
        "empty" => false,
        other => {
            return Err(fail(format!(
                "--vector-files {other:?}: expected `sparse` or `empty`"
            )));
        }
    };

    if index.exists() {
        let non_empty = fs::read_dir(&index)?.next().is_some();
        if non_empty && !args.flag("overwrite") {
            return Err(fail(format!(
                "{}: target is not empty, pass --overwrite to replace it",
                index.display()
            )));
        }
        fs::remove_dir_all(&index)?;
    }
    let started = Instant::now();
    let schema = bench_schema()?;
    write_index_config(&index, &bench_config("commit-cost", &schema))?;
    drop(GenerationStore::create(&index, Epoch::new(1), &schema)?);

    let epoch_dir = epoch_dir_of(&index);
    let segments_dir = epoch_dir.join("segments");
    let counts = split_counts(passages, segments);
    let mut metas = Vec::with_capacity(counts.len());
    let mut iid_start = 0u32;
    for (position, count) in counts.iter().copied().enumerate() {
        let segment_id = format!("seg-{position:04}");
        write_segment_files(
            &segments_dir.join(&segment_id),
            &segment_id,
            iid_start,
            count,
            dims,
            sparse,
        )?;
        iid_start = iid_start
            .checked_add(count)
            .ok_or_else(|| fail("the segment layout overflows the iid space"))?;
        metas.push((segment_id, count));
    }

    let records = epoch_dir.join("records.sqlite");
    let load_started = Instant::now();
    bulk_load(&records, passages, &metas, dims)?;
    let load_ns = duration_ns(load_started.elapsed());

    let store = GenerationStore::open(&index, &bench_schema()?)?;
    let snapshot = store.snapshot();
    let live = snapshot.live_iids().count();
    let found_segments = snapshot.segments().len();
    let max_iid = snapshot.live_iids().last().map_or(0, |iid| iid.get());
    drop(snapshot);
    drop(store);
    if live != usize::try_from(passages)? {
        return Err(fail(format!(
            "the prepared index has {live} live passages, expected {passages}"
        )));
    }
    if found_segments != usize::try_from(segments)? {
        return Err(fail(format!(
            "the prepared index has {found_segments} segments, expected {segments}"
        )));
    }

    let report = json!({
        "index": index.display().to_string(),
        "passages": passages,
        "segments": segments,
        "dims": dims,
        "vector_files": vector_files,
        "max_live_iid": max_iid,
        "prepare_seconds": seconds(started.elapsed()),
        "bulk_load_seconds": ns_to_seconds(load_ns),
        "records_bytes": file_bytes(&records),
        "apparent_vector_bytes": apparent_vector_bytes(&segments_dir)?,
        "physical_vector_bytes": physical_bytes(&segments_dir),
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

/// Splits `passages` into `segments` counts, the first segments taking the
/// remainder: a compacted index holds full segments and one shorter tail.
fn split_counts(passages: u32, segments: u32) -> Vec<u32> {
    let base = passages / segments;
    let remainder = passages % segments;
    (0..segments)
        .map(|position| base + u32::from(position < remainder))
        .collect()
}

/// Writes one segment directory as `SegmentWriter::finish` would leave it: a
/// `vectors.f16` file and `segment.json`.
///
/// `sparse` gives the vector file its full expected length through
/// `File::set_len`, which allocates no block on APFS: the control point of the
/// measure is that the commit never reads those bytes. In that case the
/// checksum stored in `segment.json` is a placeholder, because hashing 13 GB per
/// segment would dominate the preparation; `segment::read_meta` parses it as a
/// string and never verifies it, which is exactly the property under test.
fn write_segment_files(
    dir: &Path,
    segment_id: &str,
    iid_start: u32,
    count: u32,
    dims: u32,
    sparse: bool,
) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(dir)?;
    let vectors = dir.join("vectors.f16");
    let file = File::create(&vectors)?;
    let checksum = if sparse {
        let components = u64::from(count) * u64::from(dims);
        file.set_len(components * 2)?;
        "blake3:unverified-control-point".to_owned()
    } else {
        format!("blake3:{}", blake3::hash(&[]).to_hex())
    };
    drop(file);

    let manifest = json!({
        "batch_id": format!("batch-{segment_id}"),
        "checksums": { "vectors.f16": checksum },
        "count": count,
        "dims": dims,
        "format_version": 1,
        "generation": 1,
        "iid_start": iid_start,
    });
    let bytes = serde_json::to_string_pretty(&manifest)?;
    fs::write(dir.join("segment.json"), bytes)?;
    Ok(())
}

/// Writes the reference state of the base index in one SQLite transaction: the
/// `segments` rows, the `documents` rows, the `passages` rows, and the `meta`
/// row at generation 1.
///
/// The rows carry the encoding of `records.rs`: canonical JSON in `fields`, a
/// zstd frame in `text`, and one document per two passages.
fn bulk_load(
    records: &Path,
    passages: u32,
    segments: &[(String, u32)],
    dims: u32,
) -> Result<(), Box<dyn Error>> {
    let mut connection = Connection::open_with_flags(records, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    connection.busy_timeout(Duration::from_millis(1_000))?;
    let frames = text_frames()?;
    let transaction = connection.transaction()?;
    {
        let mut statement = transaction.prepare(
            "INSERT INTO segments (segment_id, iid_start, count, generation, batch_id, checksums)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        let mut iid_start = 0u32;
        for (segment_id, count) in segments {
            let checksums = json!({
                "vectors.f16": "blake3:unverified-control-point",
            });
            statement.execute(params![
                segment_id.as_str(),
                i64::from(iid_start),
                i64::from(*count),
                1i64,
                format!("batch-{segment_id}"),
                serde_json::to_string(&checksums)?,
            ])?;
            iid_start += *count;
        }
        drop(statement);

        let mut statement = transaction.prepare(
            "INSERT INTO documents (doc_id, doc_hash, fields, updated_generation)
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for document in 0..passages.div_ceil(2) {
            let doc_id = format!("doc-{document}");
            statement.execute(params![
                doc_id.as_str(),
                format!("doc-hash-{document:016x}"),
                document_fields(document, dims),
                1i64,
            ])?;
        }
        drop(statement);

        let mut statement = transaction.prepare(
            "INSERT INTO passages (iid, doc_id, ord, text, passage_hash, fields,
                                   segment_id, created_generation, dead_since_gen)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)",
        )?;
        for iid in 0..passages {
            let doc_id = format!("doc-{}", iid / 2);
            statement.execute(params![
                i64::from(iid),
                doc_id.as_str(),
                i64::from(iid % 2),
                frames[(iid as usize) % TEXT_POOL].as_slice(),
                format!("passage-hash-{iid:016x}"),
                passage_fields(iid, dims),
                segment_of(segments, iid),
                1i64,
            ])?;
        }
        drop(statement);

        transaction.execute(
            "UPDATE meta SET generation = 1, next_iid = ?1 WHERE id = 1",
            params![passages],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

/// Segment identifier holding `iid`, in the layout of `split_counts`.
fn segment_of(segments: &[(String, u32)], iid: u32) -> String {
    let mut start = 0u32;
    for (segment_id, count) in segments {
        if iid < start + count {
            return segment_id.clone();
        }
        start += count;
    }
    String::from("seg-missing")
}

/// Fields of a document row, as canonical JSON: keys sorted, no space.
fn document_fields(document: u32, dims: u32) -> String {
    format!(
        "{{\"dims\":{dims},\"fonds\":\"vigueur\",\"num\":\"L{}-\",\"source\":\"mes15\"}}",
        document % 1_000
    )
}

/// Schema of the index the bench prepares: the fields its rows carry.
///
/// `dims` is declared at document level only, though a passage row carries it
/// too: one name cannot be declared at both levels (ADR-0030), and an undeclared
/// field of a row is simply not indexed.
fn bench_schema() -> Result<Schema, Box<dyn Error>> {
    Ok(Schema::from_json_str(
        r#"{
            "document_fields": {
                "dims": { "type": "integer" },
                "fonds": { "type": "keyword" },
                "num": { "type": "keyword" },
                "source": { "type": "keyword" }
            },
            "passage_fields": {
                "partie": { "type": "keyword" },
                "rang": { "type": "integer" }
            }
        }"#,
    )?)
}

/// Index configuration the benchmark creates, declaring `schema`.
fn bench_config(name: &str, schema: &Schema) -> String {
    format!(
        r#"{{"name":"{name}","model":"m","reference_precision":"fp16","schema":{},"cluster_by":[],"lexical":{{"enabled":false,"language":"fr","positions":false}},"search":{{"scan":"none","rescore_candidates":10,"exact_threshold":10,"candidates_per_branch":10,"fusion":{{"method":"rrf"}}}},"ingest":{{"acceptance_sample":1,"acceptance_min_cosine":0.9}},"compaction":{{"dead_ratio":0.1}}}}"#,
        schema.to_json_string()
    )
}

/// Fields of a passage row, as canonical JSON.
fn passage_fields(iid: u32, dims: u32) -> String {
    format!(
        "{{\"dims\":{dims},\"partie\":\"corps\",\"rang\":{}}}",
        iid % 7
    )
}

/// A pool of `TEXT_POOL` compressed passage texts, the size of an article
/// passage of Legifacile.
fn text_frames() -> Result<Vec<Vec<u8>>, Box<dyn Error>> {
    let mut frames = Vec::with_capacity(TEXT_POOL);
    for position in 0..TEXT_POOL {
        let text = format!(
            "Article {position} : le présent article dispose que la présente mesure \
             s'applique aux articles suivants du présent code, dans les conditions \
             prévues à l'article {}.",
            position + 1
        );
        frames.push(zstd::stream::encode_all(
            text.as_bytes(),
            zstd::DEFAULT_COMPRESSION_LEVEL,
        )?);
    }
    Ok(frames)
}

// ---------------------------------------------------------------------------
// Measurement
// ---------------------------------------------------------------------------

/// Parameters shared by every measured point.
struct MeasureOptions {
    repeats: usize,
    new_passages: u32,
    tombstones: u32,
}

/// Costs of one replicated transaction, in nanoseconds.
struct ReplicaTransaction {
    body_ns: u64,
    commit_ns: u64,
    documents_ns: u64,
    passage_encode_ns: u64,
    passage_insert_ns: u64,
    tombstone_ns: u64,
    meta_ns: u64,
}

/// Measures every `--index` point and writes the raw samples as JSON.
fn measure(args: &Args) -> Result<(), Box<dyn Error>> {
    let indexes = args.all("index");
    if indexes.is_empty() {
        return Err(fail("at least one --index is required"));
    }
    let work = PathBuf::from(args.one("work")?);
    let out = PathBuf::from(args.one("out")?);
    let options = MeasureOptions {
        repeats: args.number("repeats")?,
        new_passages: args.number_or("new", 2_000)?,
        tombstones: args.number_or("tombstones", 50)?,
    };
    if options.repeats < 5 {
        return Err(fail("--repeats must be at least 5 for percentiles"));
    }
    fs::create_dir_all(&work)?;

    let mut points = Vec::with_capacity(indexes.len());
    for index in &indexes {
        let point = measure_point(Path::new(index), &work, &options)?;
        println!("{}", serde_json::to_string(&point)?);
        points.push(point);
    }

    let report = json!({
        "measure": "MES-15",
        "commit": args.one("commit")?,
        "machine": args.one("machine")?,
        "date": args.one("date")?,
        "repeats": options.repeats,
        "new_passages": options.new_passages,
        "tombstones": options.tombstones,
        "replica_runs": REPLICA_RUNS,
        "points": points,
    });
    fs::write(&out, serde_json::to_string_pretty(&report)?)?;
    Ok(())
}

/// Measures one `(N, S)` point by repeating a full commit on a fresh copy.
fn measure_point(
    index: &Path,
    work: &Path,
    options: &MeasureOptions,
) -> Result<Value, Box<dyn Error>> {
    let label = index
        .file_name()
        .and_then(|name| name.to_str())
        .map_or_else(|| index.display().to_string(), str::to_owned);
    let base_epoch = epoch_dir_of(index);

    let (n, s, next_iid) = {
        let store = GenerationStore::open(index, &bench_schema()?)?;
        let snapshot = store.snapshot();
        let live = snapshot.live_iids().count();
        let next_iid = snapshot
            .live_iids()
            .last()
            .map_or(0, |iid| iid.get())
            .checked_add(1)
            .ok_or_else(|| fail("the base index exhausts the iid space"))?;
        (live, snapshot.segments().len(), next_iid)
    };
    let n = u32::try_from(n)?;
    let s = u32::try_from(s)?;
    let deletions = tombstone_iids(n, options.tombstones)?;
    let texts = new_passage_texts(options.new_passages)?;

    let mut commit_ns = Vec::with_capacity(options.repeats);
    let mut live_iids_ns = Vec::with_capacity(options.repeats);
    let mut live_iids_first_ns = Vec::with_capacity(options.repeats);
    let mut segment_meta_first_ns = Vec::with_capacity(options.repeats);
    let mut manifest_first_ns = Vec::with_capacity(options.repeats);
    let mut segment_meta_ns = Vec::with_capacity(options.repeats);
    let mut manifest_ns = Vec::with_capacity(options.repeats);
    let mut transaction_body_ns = Vec::with_capacity(options.repeats);
    let mut transaction_commit_ns = Vec::with_capacity(options.repeats);
    let mut transaction_documents_ns = Vec::with_capacity(options.repeats);
    let mut transaction_encode_ns = Vec::with_capacity(options.repeats);
    let mut transaction_insert_ns = Vec::with_capacity(options.repeats);
    let mut transaction_tombstone_ns = Vec::with_capacity(options.repeats);
    let mut transaction_meta_ns = Vec::with_capacity(options.repeats);
    let mut segment_write_ns = Vec::with_capacity(options.repeats);
    let mut rss_bytes = Vec::with_capacity(options.repeats);
    let mut warm_up_ns = Vec::with_capacity(options.repeats);
    let mut encode_reused_ns = Vec::with_capacity(options.repeats);
    let mut clone_kinds = Vec::with_capacity(options.repeats);
    let mut live_after = 0usize;
    let mut segments_after = 0usize;

    for repetition in 0..options.repeats {
        let clone_commit = work.join(format!("{label}-commit-{repetition}"));
        let clone_replica = work.join(format!("{label}-replica-{repetition}"));
        for clone in [&clone_commit, &clone_replica] {
            if clone.exists() {
                fs::remove_dir_all(clone)?;
            }
        }
        clone_kinds.push(clone_tree(index, &clone_commit)?);
        clone_tree(index, &clone_replica)?;

        let commit_epoch = epoch_dir_of(&clone_commit);
        let replica_epoch = epoch_dir_of(&clone_replica);
        let commit_records = commit_epoch.join("records.sqlite");
        let replica_records = replica_epoch.join("records.sqlite");

        // Warming: the copy starts with whatever page cache state the OS gave
        // it, and that state cannot be purged without root. Reading the whole
        // reference database once makes every repetition start from the same
        // state, the one of a running index; the duration of that read is the
        // measured cold-I/O cost of the file.
        let warm_up = timed(|| warm_up(&commit_records))?;
        warm_up_ns.push(warm_up.1);

        // Replicated reads, on the state the commit will read. The first run of
        // each replica is kept apart as a control of the warm-up; the reported
        // value is the median of the following runs.
        let mut runs = Vec::with_capacity(REPLICA_RUNS);
        let mut first = 0u64;
        for run in 0..=REPLICA_RUNS {
            let (_, elapsed) = timed(|| replica_live_iids(&commit_records))?;
            if run == 0 {
                first = elapsed;
            } else {
                runs.push(elapsed);
            }
        }
        live_iids_first_ns.push(first);
        live_iids_ns.push(median(runs));

        let mut segment_runs = Vec::with_capacity(REPLICA_RUNS);
        let mut segment_first = 0u64;
        let mut metas = Vec::new();
        for run in 0..=REPLICA_RUNS {
            let (found, elapsed) =
                timed(|| replica_segment_meta(&commit_records, &commit_epoch.join("segments")))?;
            metas = found;
            if run == 0 {
                segment_first = elapsed;
            } else {
                segment_runs.push(elapsed);
            }
        }
        segment_meta_first_ns.push(segment_first);
        segment_meta_ns.push(median(segment_runs));

        let mut manifest_runs = Vec::with_capacity(REPLICA_RUNS);
        let mut manifest_first = 0u64;
        for run in 0..=REPLICA_RUNS {
            let (_, elapsed) = timed(|| replica_manifest(&commit_epoch, 2, &metas))?;
            if run == 0 {
                manifest_first = elapsed;
            } else {
                manifest_runs.push(elapsed);
            }
        }
        manifest_first_ns.push(manifest_first);
        manifest_ns.push(median(manifest_runs));

        // The measured commit.
        let mut store = GenerationStore::open(&clone_commit, &bench_schema()?)?;
        let generation = Generation::new(
            store
                .generation()
                .get()
                .checked_add(1)
                .ok_or_else(|| fail("the generation counter is exhausted"))?,
        );
        // Control point: the same texts compressed with one reused zstd context,
        // which bounds the avoidable part of the transaction.
        encode_reused_ns.push(reused_context_compression(&texts)?);

        let (change, write_ns) =
            build_change(&commit_epoch, generation, next_iid, &texts, &deletions)?;
        segment_write_ns.push(write_ns);
        let started = Instant::now();
        store.commit(change)?;
        commit_ns.push(duration_ns(started.elapsed()));

        let snapshot = store.snapshot();
        live_after = snapshot.live_iids().count();
        segments_after = snapshot.segments().len();
        rss_bytes.push(sample_rss_bytes()?);
        drop(snapshot);
        drop(store);

        // The replicated transaction, on the second copy: the real commit has
        // already changed the first one.
        let replica = replica_transaction(
            &replica_records,
            generation,
            next_iid,
            &deletions,
            &texts,
            options,
        )?;
        transaction_body_ns.push(replica.body_ns);
        transaction_commit_ns.push(replica.commit_ns);
        transaction_documents_ns.push(replica.documents_ns);
        transaction_encode_ns.push(replica.passage_encode_ns);
        transaction_insert_ns.push(replica.passage_insert_ns);
        transaction_tombstone_ns.push(replica.tombstone_ns);
        transaction_meta_ns.push(replica.meta_ns);

        for clone in [&clone_commit, &clone_replica] {
            fs::remove_dir_all(clone)?;
        }
    }

    let expected_live = usize::try_from(n + options.new_passages - options.tombstones)?;
    if live_after != expected_live {
        return Err(fail(format!(
            "{label}: the commit left {live_after} live passages, expected {expected_live}"
        )));
    }

    Ok(json!({
        "label": label,
        "index": index.display().to_string(),
        "n": n,
        "s": s,
        "repeats": options.repeats,
        "clone_kind": clone_kinds,
        "live_after": live_after,
        "segments_after": segments_after,
        "commit_ns": stats(&commit_ns),
        "live_iids_ns": stats(&live_iids_ns),
        "live_iids_first_ns": stats(&live_iids_first_ns),
        "segment_meta_first_ns": stats(&segment_meta_first_ns),
        "manifest_first_ns": stats(&manifest_first_ns),
        "segment_meta_ns": stats(&segment_meta_ns),
        "manifest_ns": stats(&manifest_ns),
        "transaction_body_ns": stats(&transaction_body_ns),
        "transaction_commit_ns": stats(&transaction_commit_ns),
        "transaction_documents_ns": stats(&transaction_documents_ns),
        "transaction_encode_ns": stats(&transaction_encode_ns),
        "transaction_insert_ns": stats(&transaction_insert_ns),
        "transaction_tombstone_ns": stats(&transaction_tombstone_ns),
        "transaction_meta_ns": stats(&transaction_meta_ns),
        "segment_write_ns": stats(&segment_write_ns),
        "rss_bytes": stats(&rss_bytes),
        "warm_up_ns": stats(&warm_up_ns),
        "encode_reused_ns": stats(&encode_reused_ns),
        "base_records_bytes": file_bytes(&base_epoch.join("records.sqlite")),
        "snapshot_bytes_computed": u64::from(n) * 4 + u64::from(s) * 152,
    }))
}

/// Builds the change of a daily attach: one new segment of `new_passages`
/// passages, `deletions` tombstones, one document per two passages.
///
/// Returns the change and the time spent writing the segment on disk, which is
/// **outside** the commit (the attach prepares it before calling `commit`).
fn build_change(
    epoch: &Path,
    generation: Generation,
    next_iid: u32,
    texts: &[String],
    deletions: &[Iid],
) -> Result<(GenerationChange, u64), Box<dyn Error>> {
    let new_passages = u32::try_from(texts.len())?;
    let started = Instant::now();
    let spec = SegmentSpec {
        segment_id: format!("seg-g{}", generation.get()),
        iid_start: Iid::new(next_iid),
        dims: 768,
        generation,
        batch_id: format!("batch-g{}", generation.get()),
        precision: ReferencePrecision::Fp16,
    };
    let mut writer = SegmentWriter::create(&epoch.join("segments"), spec)?;
    let vector = vec![0.0f32; 768];
    for _ in 0..new_passages {
        writer.append(&vector)?;
    }
    let meta = writer.finish()?;
    let write_ns = duration_ns(started.elapsed());

    let mut documents = Vec::with_capacity(usize::try_from(new_passages.div_ceil(2))?);
    for document in 0..new_passages.div_ceil(2) {
        documents.push(DocumentRecord {
            doc_id: format!("doc-new-{document}"),
            doc_hash: format!("doc-hash-new-{document:016x}"),
            fields: json!({"fonds": "vigueur", "source": "mes15"}),
            updated_generation: generation,
        });
    }
    let mut passages = Vec::with_capacity(usize::try_from(new_passages)?);
    for position in 0..new_passages {
        let iid = next_iid
            .checked_add(position)
            .ok_or_else(|| fail("the new segment overflows the iid space"))?;
        passages.push(PassageRecord {
            iid: Iid::new(iid),
            doc_id: format!("doc-new-{}", position / 2),
            ord: position % 2,
            text: texts[usize::try_from(position)?].clone(),
            passage_hash: format!("passage-hash-new-{iid:016x}"),
            fields: json!({"partie": "corps", "rang": position % 7}),
            segment_id: meta.segment_id.clone(),
            created_generation: generation,
            dead_since_gen: None,
        });
    }

    Ok((
        GenerationChange {
            segments: vec![meta],
            documents,
            passages,
            deletions: deletions.to_vec(),
        },
        write_ns,
    ))
}

/// Texts of the passages of the measured attach, one per passage: the exact
/// string `commit` compresses, and the input of the reused-context control.
fn new_passage_texts(count: u32) -> Result<Vec<String>, Box<dyn Error>> {
    let mut texts = Vec::with_capacity(usize::try_from(count)?);
    for position in 0..count {
        texts.push(format!(
            "Article nouveau {position} : le présent article dispose que la présente \
             mesure s'applique aux articles suivants du présent code."
        ));
    }
    Ok(texts)
}

/// Compresses the same texts with **one** zstd context, as a control point:
/// `records::encode_text` calls `encode_all`, which builds a context per row.
/// This measurement bounds the part of the transaction that a reused context
/// would remove; it changes no production code.
fn reused_context_compression(texts: &[String]) -> Result<u64, Box<dyn Error>> {
    let mut compressor = zstd::bulk::Compressor::new(zstd::DEFAULT_COMPRESSION_LEVEL)?;
    let started = Instant::now();
    for text in texts {
        let frame = compressor.compress(text.as_bytes())?;
        std::hint::black_box(frame);
    }
    Ok(duration_ns(started.elapsed()))
}

/// `count` distinct live iids of the base index, spread over its range.
fn tombstone_iids(n: u32, count: u32) -> Result<Vec<Iid>, Box<dyn Error>> {
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut iids = Vec::with_capacity(usize::try_from(count)?);
    for position in 0..u64::from(count) {
        let iid = position * 97 % u64::from(n);
        iids.push(Iid::new(u32::try_from(iid)?));
    }
    Ok(iids)
}

// ---------------------------------------------------------------------------
// Replicas of the production reads and writes
// ---------------------------------------------------------------------------

/// Replays `RecordStore::live_iids`: every live iid, in ascending order, read
/// from a read-only connection and materialised in a `Vec`.
fn replica_live_iids(records: &Path) -> Result<usize, Box<dyn Error>> {
    let connection = open_read_only(records)?;
    let mut statement =
        connection.prepare("SELECT iid FROM passages WHERE dead_since_gen IS NULL ORDER BY iid")?;
    let rows = statement
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let iids = rows
        .into_iter()
        .map(u32::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(iids.len())
}

/// One segment as `segment::read_meta` rebuilds it.
#[derive(Debug, Clone)]
struct RepSegment {
    format_version: u32,
    segment_id: String,
    iid_start: u32,
    count: u32,
    dims: u32,
    generation: u64,
    batch_id: String,
    checksums: BTreeMap<String, String>,
}

/// Replays `build_snapshot`'s segment loop: the segment ids of SQLite, then one
/// `segment.json` read and two `is_file` checks per segment, then the sort by
/// `iid_start` and identifier.
fn replica_segment_meta(
    records: &Path,
    segments_dir: &Path,
) -> Result<Vec<RepSegment>, Box<dyn Error>> {
    let connection = open_read_only(records)?;
    let mut statement =
        connection.prepare("SELECT segment_id FROM segments ORDER BY segment_id")?;
    let ids = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    drop(connection);

    let mut metas = Vec::with_capacity(ids.len());
    for segment_id in ids {
        let dir = segments_dir.join(&segment_id);
        let raw = fs::read_to_string(dir.join("segment.json"))?;
        let manifest: Value = serde_json::from_str(&raw)?;
        let f16_present = dir.join("vectors.f16").is_file();
        let f32_present = dir.join("vectors.f32").is_file();
        if f16_present == f32_present {
            return Err(fail(format!(
                "segment {segment_id} must hold exactly one vector file"
            )));
        }
        let format_version = manifest
            .get("format_version")
            .and_then(Value::as_u64)
            .ok_or_else(|| fail(format!("segment {segment_id} has no format_version")))?;
        let format_version = u32::try_from(format_version)?;
        if format_version != 1 {
            return Err(fail(format!(
                "segment {segment_id} has format_version {format_version}"
            )));
        }
        let mut checksums = BTreeMap::new();
        if let Some(object) = manifest.get("checksums").and_then(Value::as_object) {
            for (name, value) in object {
                let declared = value
                    .as_str()
                    .ok_or_else(|| fail(format!("checksum {name} is not a string")))?;
                checksums.insert(name.clone(), declared.to_owned());
            }
        }
        metas.push(RepSegment {
            format_version,
            segment_id,
            iid_start: field_u32(&manifest, "iid_start")?,
            count: field_u32(&manifest, "count")?,
            dims: field_u32(&manifest, "dims")?,
            generation: manifest
                .get("generation")
                .and_then(Value::as_u64)
                .ok_or_else(|| fail("segment.json has no generation"))?,
            batch_id: manifest
                .get("batch_id")
                .and_then(Value::as_str)
                .ok_or_else(|| fail("segment.json has no batch_id"))?
                .to_owned(),
            checksums,
        });
    }
    metas.sort_by(|left, right| {
        left.iid_start
            .cmp(&right.iid_start)
            .then_with(|| left.segment_id.cmp(&right.segment_id))
    });
    Ok(metas)
}

fn field_u32(manifest: &Value, key: &str) -> Result<u32, Box<dyn Error>> {
    let value = manifest
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| fail(format!("segment.json has no {key}")))?;
    Ok(u32::try_from(value)?)
}

/// Replays `write_manifest`: the same canonical JSON, written to a temporary
/// file, fsynced, renamed, then the directory fsynced.
fn replica_manifest(
    epoch: &Path,
    generation: u64,
    metas: &[RepSegment],
) -> Result<(), Box<dyn Error>> {
    let segments: Vec<Value> = metas
        .iter()
        .map(|meta| {
            json!({
                "batch_id": meta.batch_id,
                "checksums": meta.checksums,
                "count": meta.count,
                "dims": meta.dims,
                "format_version": meta.format_version,
                "generation": meta.generation,
                "iid_start": meta.iid_start,
                "segment_id": meta.segment_id,
            })
        })
        .collect();
    let manifest = json!({
        "epoch": 1,
        "format_version": 1,
        "generation": generation,
        "segments": segments,
    });
    let bytes = serde_json::to_string(&manifest)?;
    let directory = epoch.join("manifests");
    write_atomically(&directory.join("replica.json"), bytes.as_bytes())?;
    Ok(())
}

/// Replays the transaction of `GenerationStore::commit`: the 2 000 passage rows,
/// the 1 000 document rows, the tombstones and `meta`, then measures the body
/// and the SQLite `COMMIT` separately.
fn replica_transaction(
    records: &Path,
    generation: Generation,
    next_iid: u32,
    deletions: &[Iid],
    texts: &[String],
    options: &MeasureOptions,
) -> Result<ReplicaTransaction, Box<dyn Error>> {
    let mut connection = Connection::open_with_flags(records, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    connection.busy_timeout(Duration::from_millis(1_000))?;
    let started = Instant::now();
    let transaction = connection.transaction()?;
    let mut passage_encode_ns = 0u64;
    let mut passage_insert_ns = 0u64;
    let documents_ns;
    let tombstone_ns;
    let meta_ns;
    {
        let block = Instant::now();
        let mut documents = transaction.prepare(
            "INSERT INTO documents (doc_id, doc_hash, fields, updated_generation)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(doc_id) DO UPDATE SET
                 doc_hash = excluded.doc_hash,
                 fields = excluded.fields,
                 updated_generation = excluded.updated_generation",
        )?;
        for document in 0..options.new_passages.div_ceil(2) {
            documents.execute(params![
                format!("doc-new-{document}"),
                format!("doc-hash-new-{document:016x}"),
                canonical_json(&json!({"fonds": "vigueur", "source": "mes15"}))?,
                i64::try_from(generation.get())?,
            ])?;
        }
        drop(documents);
        documents_ns = duration_ns(block.elapsed());

        let mut passages = transaction.prepare(
            "INSERT INTO passages (iid, doc_id, ord, text, passage_hash, fields,
                                   segment_id, created_generation, dead_since_gen)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;
        for position in 0..options.new_passages {
            let iid = next_iid + position;
            let text = &texts[usize::try_from(position)?];
            let encoded = Instant::now();
            let frame = zstd::stream::encode_all(text.as_bytes(), zstd::DEFAULT_COMPRESSION_LEVEL)?;
            let fields = canonical_json(&json!({"partie": "corps", "rang": position % 7}))?;
            passage_encode_ns += duration_ns(encoded.elapsed());
            let inserted = Instant::now();
            passages.execute(params![
                i64::from(iid),
                format!("doc-new-{}", position / 2),
                i64::from(position % 2),
                frame,
                format!("passage-hash-new-{iid:016x}"),
                fields,
                format!("seg-g{}", generation.get()),
                i64::try_from(generation.get())?,
                Option::<i64>::None,
            ])?;
            passage_insert_ns += duration_ns(inserted.elapsed());
        }
        drop(passages);

        let block = Instant::now();
        for iid in deletions {
            let changed = transaction.execute(
                "UPDATE passages SET dead_since_gen = ?2
                 WHERE iid = ?1 AND dead_since_gen IS NULL",
                params![i64::from(iid.get()), i64::try_from(generation.get())?],
            )?;
            if changed != 1 {
                return Err(fail(format!(
                    "replica: iid {} is not live, so it cannot be tombstoned",
                    iid.get()
                )));
            }
        }
        tombstone_ns = duration_ns(block.elapsed());

        let block = Instant::now();
        transaction.execute(
            "UPDATE meta SET generation = ?1, next_iid = ?2 WHERE id = 1",
            params![
                i64::try_from(generation.get())?,
                next_iid + options.new_passages
            ],
        )?;
        meta_ns = duration_ns(block.elapsed());
    }
    let body_ns = duration_ns(started.elapsed());
    let started = Instant::now();
    transaction.commit()?;
    let commit_ns = duration_ns(started.elapsed());
    Ok(ReplicaTransaction {
        body_ns,
        commit_ns,
        documents_ns,
        passage_encode_ns,
        passage_insert_ns,
        tombstone_ns,
        meta_ns,
    })
}

/// Replays `records::canonical_json`: keys sorted at every level, no space
/// around separators (ADR-0019).
///
/// The function is private in `records.rs`, so the replica carries its own copy:
/// the transaction writes canonical JSON, and skipping it would make the
/// replicated transaction cheaper than the production one.
fn canonical_json(value: &Value) -> Result<String, Box<dyn Error>> {
    Ok(serde_json::to_string(&canonical_value(value))?)
}

fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries: Vec<(&str, &Value)> = object
                .iter()
                .map(|(key, child)| (key.as_str(), child))
                .collect();
            entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
            let mut canonical = serde_json::Map::new();
            for (key, child) in entries {
                canonical.insert(key.to_owned(), canonical_value(child));
            }
            Value::Object(canonical)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_value).collect()),
        scalar => scalar.clone(),
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn epoch_dir_of(index: &Path) -> PathBuf {
    index.join("epochs").join("1")
}

/// Reads `records` once, in 8 MB chunks, so its pages are in the page cache
/// before the measured commit. Returns the number of bytes read.
fn warm_up(records: &Path) -> Result<u64, Box<dyn Error>> {
    let mut file = File::open(records)?;
    let mut buffer = vec![0u8; 8 * 1024 * 1024];
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok(total);
        }
        total += read as u64;
    }
}

/// Opens a database read-only, as `ReadPool` does.
fn open_read_only(records: &Path) -> Result<Connection, Box<dyn Error>> {
    let connection = Connection::open_with_flags(records, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(Duration::from_millis(1_000))?;
    Ok(connection)
}

/// Writes a file atomically: temporary file, fsync, rename, then fsync of the
/// directory, as `generation::write_atomically` does.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    let parent = path
        .parent()
        .ok_or_else(|| fail(format!("{} has no parent directory", path.display())))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| fail(format!("{} has no UTF-8 name", path.display())))?;
    let temporary = parent.join(format!("tmp-{name}"));
    let mut file = File::create(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

/// Clones one directory tree: APFS `cp -c` when available, `cp -R` otherwise.
fn clone_tree(from: &Path, to: &Path) -> Result<&'static str, Box<dyn Error>> {
    let cloned = Command::new("cp").arg("-cR").arg(from).arg(to).status()?;
    if cloned.success() {
        return Ok("clonefile");
    }
    let copied = Command::new("cp").arg("-R").arg(from).arg(to).status()?;
    if !copied.success() {
        return Err(fail(format!(
            "cannot copy {} to {}: cp -R failed",
            from.display(),
            to.display()
        )));
    }
    Ok("copy")
}

/// Runs `run` and returns its value with its duration in nanoseconds.
fn timed<T>(run: impl FnOnce() -> Result<T, Box<dyn Error>>) -> Result<(T, u64), Box<dyn Error>> {
    let started = Instant::now();
    let value = run()?;
    Ok((value, duration_ns(started.elapsed())))
}

fn duration_ns(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX)
}

/// Median of a small sample.
fn median(mut values: Vec<u64>) -> u64 {
    values.sort_unstable();
    values[values.len() / 2]
}

/// Percentiles of a sample, by nearest rank: `p50`, `p95`, `p99`, the minimum,
/// the maximum, and the raw values.
fn stats(values: &[u64]) -> Value {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    json!({
        "p50": percentile(&sorted, 50),
        "p95": percentile(&sorted, 95),
        "p99": percentile(&sorted, 99),
        "min": sorted.first().copied(),
        "max": sorted.last().copied(),
        "values": values,
    })
}

/// Nearest-rank percentile: the smallest sample at or above `p` percent.
fn percentile(sorted: &[u64], percent: usize) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (percent * sorted.len()).div_ceil(100);
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted.get(index).copied()
}

/// Resident set size of this process, in bytes, read through `ps`.
///
/// A point sample, taken with the new snapshot held: the bench does not follow
/// the peak, which the report states.
fn sample_rss_bytes() -> Result<u64, Box<dyn Error>> {
    let pid = std::process::id().to_string();
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let kib = text.trim().parse::<u64>()?;
    Ok(kib * 1024)
}

fn file_bytes(path: &Path) -> u64 {
    fs::metadata(path).map_or(0, |metadata| metadata.len())
}

/// Apparent length of the vector files of `segments_dir`.
fn apparent_vector_bytes(segments_dir: &Path) -> Result<u64, Box<dyn Error>> {
    let mut total = 0u64;
    for entry in fs::read_dir(segments_dir)? {
        let directory = entry?.path();
        for name in ["vectors.f16", "vectors.f32"] {
            total += file_bytes(&directory.join(name));
        }
    }
    Ok(total)
}

/// Bytes the vector files actually occupy, through `du -sk`.
fn physical_bytes(segments_dir: &Path) -> u64 {
    let output = Command::new("du").arg("-sk").arg(segments_dir).output();
    let Ok(output) = output else {
        return 0;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    text.split_whitespace()
        .next()
        .and_then(|kib| kib.parse::<u64>().ok())
        .map_or(0, |kib| kib * 1024)
}

fn seconds(elapsed: Duration) -> f64 {
    elapsed.as_secs_f64()
}

fn ns_to_seconds(nanoseconds: u64) -> f64 {
    nanoseconds as f64 / 1_000_000_000.0
}
