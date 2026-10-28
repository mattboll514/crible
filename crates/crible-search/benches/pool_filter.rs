// SPDX-License-Identifier: Apache-2.0

//! MES-14 bench: cost of `Pool::evaluate` for an interval filter on a value
//! column, as a function of the domain the interval clause receives.
//!
//! References: `docs/08-invariants-et-tests.md` (MES-14), `docs/03-recherche.md`
//! ("budget: filtre (bitmaps) < 1 ms"), `docs/phases/P1-stockage-et-recherche-
//! exacte.md` (`EVALUER-FILTRES`), ADR-0018 (a value column is scanned only over
//! the domain the enclosing conjunction kept) and
//! `crates/crible-search/src/pool.rs` (`Pool::evaluate`, `evaluate_and`,
//! `scan_column`).
//!
//! # What is measured
//!
//! One call to `Pool::evaluate`, end to end: the parse of the filter is done
//! once, outside the timed loop, and the loop times the evaluation only. Each
//! case is a filter of the closed grammar of `docs/05-api.md`:
//!
//! - `keyword alone` — one bitmap clause, the reference cost;
//! - `column alone` — one interval clause whose domain is every live passage
//!   (`scan_column` then walks `present ∩ live`);
//! - `keyword AND column` — the keyword clause runs first, so the interval
//!   clause walks `present ∩ live ∩ keyword` only (ADR-0018).
//!
//! The bench reports, for each case, the **scan size** it actually gave the
//! interval clause, computed with the same structures the evaluation reads
//! (`present ∩ live ∩ keyword`). Without that number a "cost of an interval
//! filter" would not be comparable between cases.
//!
//! # Modes
//!
//! ```text
//! pool_filter prepare --index <dir> --passages <N> [--overwrite]
//! pool_filter measure --index <dir> [--index <dir> ...] --out <dir>
//!                     --repeats 1000 --warmup 200 --commit <sha>
//!                     --machine <text> --date <YYYY-MM-DD> [--round 1]
//! ```
//!
//! `prepare` writes an index whose `records.sqlite` carries `N` live passages
//! with the schema and the value distributions of `synth-small`. It is a
//! measurement fixture, not an ingestible index: `Pool::evaluate` reads only
//! the filter structures, so no segment row and no vector file is written. The
//! report says so, and never pretends such an index is a `crible attach`
//! result.
//!
//! # Protocole
//!
//! The harness is `harness = false`: the measure is a case matrix, not a
//! criterion function, and `criterion` is not added for it (same choice as
//! MES-15). Each case runs `--warmup` evaluations, discarded, then `--repeats`
//! timed evaluations, one `Instant` pair per evaluation. Percentiles are
//! **nearest rank** on the sorted samples, never an interpolation: on 1 000
//! samples p95 is the 950th value and p99 the 990th.
//!
//! Cache: hot. Every structure is in RAM (the filter index is built by
//! `FilterIndex::from_records` and lives on the heap), so the page cache does
//! not enter the measured loop; the one-off build is timed separately and
//! reported as `index_build_ns`.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Instant;

use crible_core::config::{IndexConfig, Schema};
use crible_core::filter::Filter;
use crible_core::ids::Epoch;
use crible_search::pool::Pool;
use crible_store::filters_index::FilterIndex;
use crible_store::generation::GenerationStore;
use rusqlite::{Connection, OpenFlags, params};
use serde_json::{Value, json};

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
    fn parse(mut raw: impl Iterator<Item = String>) -> Result<Self, Box<dyn Error>> {
        let mut mode = None;
        let mut values: BTreeMap<String, Vec<String>> = BTreeMap::new();
        while let Some(token) = raw.next() {
            let Some(key) = token.strip_prefix("--") else {
                if mode.is_some() {
                    return Err(fail(format!("unexpected argument {token:?}")));
                }
                mode = Some(token);
                continue;
            };
            // `cargo bench` appends `--bench` (and `cargo test --benches`
            // appends `--test`): flags of the harness protocol, not options.
            if matches!(key, "bench" | "test") {
                continue;
            }
            let (key, value) = match key.split_once('=') {
                Some((key, value)) => (key.to_owned(), Some(value.to_owned())),
                None if FLAGS.contains(&key) => (key.to_owned(), None),
                None => {
                    let value = raw
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
        self.values
            .get(key)
            .and_then(|values| values.first())
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
// Preparation of a fixture at scale
// ---------------------------------------------------------------------------

/// Schema of `synth-small`, copied from `tools/gen_synth.py`.
const FIXTURE_SCHEMA: &str = r#"{
    "document_fields": {
        "annee": { "type": "integer" },
        "categorie": { "type": "keyword" },
        "date_publication": { "type": "date" },
        "publie": { "type": "bool" },
        "score": { "type": "float" },
        "source": { "type": "keyword" }
    },
    "passage_fields": {
        "date_effet": { "type": "date" },
        "partie": { "type": "keyword" },
        "poids": { "type": "float" },
        "principal": { "type": "bool" },
        "rang": { "type": "integer" }
    }
}"#;

/// Configuration of the fixture index: same schema and same shape as
/// `synth-small`, with a name of its own.
fn fixture_config(name: &str, schema: &Schema) -> String {
    format!(
        r#"{{"name":"{name}","model":"synthetic-external","reference_precision":"fp16","schema":{},"cluster_by":["categorie"],"lexical":{{"enabled":false,"language":"fr","positions":false}},"search":{{"scan":"none","rescore_candidates":5000,"exact_threshold":5000,"candidates_per_branch":100,"fusion":{{"method":"rrf"}}}},"ingest":{{"acceptance_sample":200,"acceptance_min_cosine":0.999}},"compaction":{{"dead_ratio":0.1}}}}"#,
        schema.to_json_string()
    )
}

/// Sources, parts, clusters and date span of `synth-small`.
const SOURCES: [&str; 4] = ["journal", "code", "decret", "constitution"];
const PARTS: [&str; 5] = ["titre", "corps", "motifs", "dispositif", "annexe"];
const CLUSTERS: u64 = 16;
const PASSAGES_PER_DOCUMENT: u32 = 5;
const YEAR_FIRST: u64 = 1990;
const YEAR_COUNT: u64 = 36;
const DAY_COUNT: u64 = 13_149;

/// splitmix64 finaliser: a deterministic draw from `(iid, salt)`.
fn draw(iid: u64, salt: u64) -> u64 {
    let mut z = iid
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(salt.wrapping_mul(0xD1B5_4A32_D192_ED03));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// `YYYY-MM-DD` of a day offset from 1990-01-01, the epoch of the generator.
///
/// The civil-from-days algorithm of Howard Hinnant, so the fixture does not add
/// a date crate to the bench.
fn civil_date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{day:02}")
}

/// `Some(day offset)` unless the field is null, at the rate of `synth-small`.
fn day_value(iid: u64, salt: u64, null_percent: u64) -> Option<i64> {
    if draw(iid, salt) % 100 < null_percent {
        return None;
    }
    Some(i64::try_from(draw(iid, salt + 1) % DAY_COUNT).unwrap_or(0))
}

/// Document-level fields of one document, canonical JSON (keys sorted, no
/// space), with the nulls of `synth-small` written as `null`.
fn document_fields(document: u64) -> String {
    let year = if draw(document, 20) % 100 < 5 {
        "null".to_owned()
    } else {
        let value = YEAR_FIRST + draw(document, 21) % YEAR_COUNT;
        value.to_string()
    };
    let publication = match day_value(document, 30, 5) {
        Some(days) => format!("\"{}\"", civil_date(days)),
        None => "null".to_owned(),
    };
    let published = if draw(document, 40) % 100 < 5 {
        "null".to_owned()
    } else {
        (draw(document, 41) % 100 < 80).to_string()
    };
    let score = if draw(document, 50) % 100 < 5 {
        "null".to_owned()
    } else {
        format!(
            "{:.6}",
            (draw(document, 51) % 1_000_000) as f64 / 1_000_000.0
        )
    };
    let source = if draw(document, 60) % 100 < 10 {
        "null".to_owned()
    } else {
        let index = usize::try_from(draw(document, 61) % 4).unwrap_or(0);
        format!("\"{}\"", SOURCES[index])
    };
    format!(
        "{{\"annee\":{year},\"categorie\":\"cat-{:02}\",\"date_publication\":{publication},\
         \"publie\":{published},\"score\":{score},\"source\":{source}}}",
        document % CLUSTERS
    )
}

/// Passage-level fields of one passage, canonical JSON.
fn passage_fields(iid: u64, ord: u32) -> String {
    let effect = match day_value(iid, 70, 10) {
        Some(days) => format!("\"{}\"", civil_date(days)),
        None => "null".to_owned(),
    };
    let part = if draw(iid, 80) % 100 < 10 {
        "null".to_owned()
    } else {
        let index = usize::try_from(draw(iid, 81) % 5).unwrap_or(0);
        format!("\"{}\"", PARTS[index])
    };
    let weight = if draw(iid, 90) % 100 < 5 {
        "null".to_owned()
    } else {
        format!("{:.6}", (draw(iid, 91) % 1_000_000) as f64 / 1_000_000.0)
    };
    let principal = if draw(iid, 100) % 100 < 5 {
        "null".to_owned()
    } else {
        (draw(iid, 101) % 100 < 60).to_string()
    };
    let rank = if draw(iid, 110) % 100 < 5 {
        "null".to_owned()
    } else {
        ord.to_string()
    };
    format!(
        "{{\"date_effet\":{effect},\"partie\":{part},\"poids\":{weight},\
         \"principal\":{principal},\"rang\":{rank}}}"
    )
}

/// Writes an index whose `records.sqlite` holds `passages` live passages, one
/// document per five passages, at generation 0.
///
/// The index is a **measurement fixture**: it carries no segment row and no
/// vector file, because `Pool::evaluate` reads only the filter structures.
/// `GenerationStore::create` writes the schema of the tables and the index
/// directory; the rows are then inserted by this bench, not by `crible attach`.
fn prepare(args: &Args) -> Result<(), Box<dyn Error>> {
    let index = PathBuf::from(args.one("index")?);
    let passages: u32 = args.number("passages")?;
    let name = args.one_or("name", "mes14-fixture");
    if passages == 0 {
        return Err(fail("--passages must be positive"));
    }
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
    let schema = Schema::from_json_str(FIXTURE_SCHEMA)?;
    crible_store::generation::write_index_config(&index, &fixture_config(name, &schema))?;
    let store = GenerationStore::create(&index, Epoch::new(1), &schema)?;
    let epoch_dir = store.epoch_directory();
    drop(store);

    let records = epoch_dir.join("records.sqlite");
    let load_started = Instant::now();
    let mut connection = Connection::open_with_flags(&records, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    {
        let transaction = connection.transaction()?;
        let documents = u64::from(passages).div_ceil(u64::from(PASSAGES_PER_DOCUMENT));
        {
            let mut statement = transaction.prepare(
                "INSERT INTO documents (doc_id, doc_hash, fields, updated_generation)
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for document in 0..documents {
                statement.execute(params![
                    format!("doc-{document:08}"),
                    "doc-hash",
                    document_fields(document),
                    0i64,
                ])?;
            }
        }
        {
            let mut statement = transaction.prepare(
                "INSERT INTO passages (iid, doc_id, ord, text, passage_hash, fields,
                                       segment_id, created_generation, dead_since_gen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)",
            )?;
            for iid in 0..u64::from(passages) {
                let ord = u32::try_from(iid % u64::from(PASSAGES_PER_DOCUMENT))?;
                statement.execute(params![
                    i64::try_from(iid)?,
                    format!("doc-{:08}", iid / u64::from(PASSAGES_PER_DOCUMENT)),
                    i64::from(ord),
                    &[][..],
                    "passage-hash",
                    passage_fields(iid, ord),
                    "seg-fixture",
                    0i64,
                ])?;
            }
        }
        transaction.commit()?;
    }
    drop(connection);
    let load_ns = duration_ns(load_started.elapsed());

    let bytes = fs::metadata(&records)?.len();
    let report = json!({
        "index": index.display().to_string(),
        "passages": passages,
        "documents": u64::from(passages).div_ceil(u64::from(PASSAGES_PER_DOCUMENT)),
        "records_bytes": bytes,
        "prepare_seconds": seconds(started.elapsed()),
        "load_seconds": ns_to_seconds(load_ns),
        "note": "measurement fixture: no segment row, no vector file, rows written by the bench",
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// One filter of the matrix, with the clause whose scan it isolates.
struct Case {
    label: &'static str,
    /// `bitmap` (keyword, reference), `integer`, `float` or `date`.
    kind: &'static str,
    filter: &'static str,
    /// Field of the value column the case scans, `None` for a bitmap case.
    column: Option<&'static str>,
    /// `(field, value)` of the keyword clause that reduces the domain first.
    domain: Option<(&'static str, &'static str)>,
}

/// The matrix. The same filters run on `synth-small` and on the fixture at
/// scale, because the fixture copies the distributions of `synth-small`.
///
/// The last two cases are **controls**: same scan size as `annee 2000..2010`,
/// but one keeps no passage and the other keeps every present one. Their
/// difference bounds what the insertion into the result bitmap costs, and what
/// the scan alone costs.
const CASES: [Case; 11] = [
    Case {
        label: "keyword seul, categorie=cat-03",
        kind: "bitmap",
        filter: r#"{"categorie":"cat-03"}"#,
        column: None,
        domain: None,
    },
    Case {
        label: "keyword seul, partie=corps",
        kind: "bitmap",
        filter: r#"{"partie":"corps"}"#,
        column: None,
        domain: None,
    },
    Case {
        label: "integer seul, annee 2000..2010, domaine = vivants",
        kind: "integer",
        filter: r#"{"annee":{"gte":2000,"lte":2010}}"#,
        column: Some("annee"),
        domain: None,
    },
    Case {
        label: "date seul, date_publication 2000-01-01..2003-12-31, domaine = vivants",
        kind: "date",
        filter: r#"{"date_publication":{"gte":"2000-01-01","lte":"2003-12-31"}}"#,
        column: Some("date_publication"),
        domain: None,
    },
    Case {
        label: "float seul, score > 0.6, domaine = vivants",
        kind: "float",
        filter: r#"{"score":{"gt":0.6}}"#,
        column: Some("score"),
        domain: None,
    },
    Case {
        label: "partie=corps ET annee 2000..2010",
        kind: "integer",
        filter: r#"{"$and":[{"partie":"corps"},{"annee":{"gte":2000,"lte":2010}}]}"#,
        column: Some("annee"),
        domain: Some(("partie", "corps")),
    },
    Case {
        label: "partie=corps ET date_publication 2000-01-01..2003-12-31",
        kind: "date",
        filter: r#"{"$and":[{"partie":"corps"},{"date_publication":{"gte":"2000-01-01","lte":"2003-12-31"}}]}"#,
        column: Some("date_publication"),
        domain: Some(("partie", "corps")),
    },
    Case {
        label: "categorie=cat-03 ET annee 2000..2010",
        kind: "integer",
        filter: r#"{"$and":[{"categorie":"cat-03"},{"annee":{"gte":2000,"lte":2010}}]}"#,
        column: Some("annee"),
        domain: Some(("categorie", "cat-03")),
    },
    Case {
        label: "categorie=cat-03 ET date_publication 2000-01-01..2003-12-31",
        kind: "date",
        filter: r#"{"$and":[{"categorie":"cat-03"},{"date_publication":{"gte":"2000-01-01","lte":"2003-12-31"}}]}"#,
        column: Some("date_publication"),
        domain: Some(("categorie", "cat-03")),
    },
    Case {
        label: "controle: annee >= 5000, memes iid lus, aucun garde",
        kind: "control",
        filter: r#"{"annee":{"gte":5000}}"#,
        column: Some("annee"),
        domain: None,
    },
    Case {
        label: "controle: annee >= 1900, memes iid lus, tous gardes",
        kind: "control",
        filter: r#"{"annee":{"gte":1900}}"#,
        column: Some("annee"),
        domain: None,
    },
];

// ---------------------------------------------------------------------------
// Measurement
// ---------------------------------------------------------------------------

/// Statistics of one case: `n` timed samples, nearest-rank percentiles.
struct Stats {
    p50: u64,
    p95: u64,
    p99: u64,
    min: u64,
    max: u64,
}

fn stats(samples: &[u64]) -> Result<Stats, Box<dyn Error>> {
    if samples.is_empty() {
        return Err(fail("a case has no sample"));
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let rank = |percent: usize| -> Result<u64, Box<dyn Error>> {
        let n = sorted.len();
        let index = (percent * n).div_ceil(100).saturating_sub(1).min(n - 1);
        sorted
            .get(index)
            .copied()
            .ok_or_else(|| fail("percentile out of the samples"))
    };
    Ok(Stats {
        p50: rank(50)?,
        p95: rank(95)?,
        p99: rank(99)?,
        min: sorted.first().copied().unwrap_or(0),
        max: sorted.last().copied().unwrap_or(0),
    })
}

fn duration_ns(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn ns_to_seconds(nanoseconds: u64) -> f64 {
    nanoseconds as f64 / 1_000_000_000.0
}

fn seconds(duration: std::time::Duration) -> f64 {
    duration.as_secs_f64()
}

/// Reads the current epoch of an index directory.
fn current_epoch(index: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let raw = fs::read_to_string(index.join("CURRENT_EPOCH"))?;
    let epoch: u32 = raw
        .trim()
        .parse()
        .map_err(|error| fail(format!("{}: bad CURRENT_EPOCH: {error}", index.display())))?;
    Ok(index.join("epochs").join(epoch.to_string()))
}

/// Size of `present ∩ live ∩ keyword`, the set `scan_column` walks.
fn scan_size(
    filters: &FilterIndex,
    column: Option<&str>,
    domain: Option<(&str, &str)>,
) -> Result<u64, Box<dyn Error>> {
    let Some(field) = column else {
        return Ok(0);
    };
    let column = filters
        .column(field)
        .ok_or_else(|| fail(format!("field {field:?} has no value column")))?;
    let mut scanned = column.present().clone();
    scanned &= filters.live();
    if let Some((field, value)) = domain {
        let keyword = filters
            .keyword(field)
            .and_then(|keyword| keyword.bitmap(value))
            .ok_or_else(|| fail(format!("field {field:?} has no bitmap for {value:?}")))?;
        scanned &= keyword;
    }
    Ok(scanned.len())
}

/// Measures one index over the whole matrix.
fn measure_index(
    index: &Path,
    repeats: usize,
    warmup: usize,
    campaign: &Value,
) -> Result<Value, Box<dyn Error>> {
    let config = IndexConfig::from_json_str(&fs::read_to_string(index.join("index.json"))?)?;
    let epoch_dir = current_epoch(index)?;
    let records = epoch_dir.join("records.sqlite");
    let connection = Connection::open_with_flags(&records, OpenFlags::SQLITE_OPEN_READ_ONLY)?;

    let build_started = Instant::now();
    let filters = FilterIndex::from_records(&connection, &config.schema)?;
    let build_ns = duration_ns(build_started.elapsed());
    drop(connection);

    let live = filters.live().len();
    let memory = filters.memory();
    let mut cases = Vec::with_capacity(CASES.len());
    for case in &CASES {
        let filter = Filter::from_json_str(case.filter)?;
        // Warm-up: the same evaluation, discarded, so the first timed call
        // never pays a cold allocation.
        for _ in 0..warmup {
            black_box(Pool::evaluate(&filters, &filter)?.len());
        }
        let mut samples = Vec::with_capacity(repeats);
        let mut pool_size = 0u32;
        for _ in 0..repeats {
            let started = Instant::now();
            let pool = Pool::evaluate(&filters, &filter)?;
            let elapsed = duration_ns(started.elapsed());
            pool_size = pool.len();
            samples.push(elapsed);
        }
        let stats = stats(&samples)?;
        let scan = scan_size(&filters, case.column, case.domain)?;
        cases.push(json!({
            "label": case.label,
            "kind": case.kind,
            "filter": case.filter,
            "pool_size": pool_size,
            "scan_iids": scan,
            "p50_ns": stats.p50,
            "p95_ns": stats.p95,
            "p99_ns": stats.p99,
            "min_ns": stats.min,
            "max_ns": stats.max,
            "samples_ns": samples,
        }));
    }

    Ok(json!({
        "campaign": campaign,
        "index": index.display().to_string(),
        "records_bytes": fs::metadata(&records)?.len(),
        "live_passages": live,
        "filter_memory_bytes": memory.total(),
        "index_build_ns": build_ns,
        "repeats": repeats,
        "warmup": warmup,
        "cases": cases,
    }))
}

fn measure(args: &Args) -> Result<(), Box<dyn Error>> {
    let indexes = args.all("index");
    if indexes.is_empty() {
        return Err(fail("--index is required (repeatable)"));
    }
    let out = PathBuf::from(args.one("out")?);
    let repeats = args.number_or("repeats", 1_000usize)?;
    let warmup = args.number_or("warmup", 200usize)?;
    let round = args.number_or("round", 1u32)?;
    if repeats == 0 {
        return Err(fail("--repeats must be positive"));
    }
    fs::create_dir_all(&out)?;
    let campaign = json!({
        "measure": "MES-14",
        "commit": args.one("commit")?,
        "machine": args.one("machine")?,
        "date": args.one("date")?,
        "repeats": repeats,
        "warmup": warmup,
        "round": round,
    });

    for path in indexes {
        let index = PathBuf::from(path);
        let report = measure_index(&index, repeats, warmup, &campaign)?;
        let name = match args.values.get("label").and_then(|values| values.first()) {
            Some(label) => label.clone(),
            None => index
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .ok_or_else(|| fail(format!("{} has no UTF-8 name", index.display())))?
                .to_owned(),
        };
        let target = out.join(format!("{name}.json"));
        fs::write(&target, serde_json::to_string_pretty(&report)?)?;
        println!("{}", target.display());
    }
    Ok(())
}
