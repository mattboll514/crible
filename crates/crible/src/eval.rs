// SPDX-License-Identifier: Apache-2.0

//! `crible eval`: the owner's validation instrument (phases P7 and P8).
//!
//! References: `docs/phases/P7-observabilite.md` (task `COMMANDE-EVAL` and its
//! two dated notes), `docs/phases/P8-docker-et-deploiement.md` (task
//! `EVALUER-PAR-API` and its note of 2026-11-02), `docs/06-observabilite.md`
//! ("Éval"), `docs/03-recherche.md` (the pipeline and its nine timing keys),
//! `docs/mesures/README.md` (the report model), ADR-0125, whose seventeen
//! decisions this module applies in local mode, and ADR-0131, which adds the
//! API mode without duplicating a metric.
//!
//! # What one campaign does
//!
//! One invocation evaluates **one index** against **one game of queries**, in
//! **local mode** by default and in **API mode** when `--url` names a running
//! instance (ADR-0131, decision 1).
//!
//! In local mode the index is opened read-only, exactly as `crible search` opens
//! it (ADR-0116), so a campaign runs beside a live server and writes nothing
//! under the data root. In API mode every request goes to
//! `POST /v1/indexes/{index}/search` of the instance, with the key read in
//! `CRIBLE_EVAL_API_KEY` and the generation pinned at the start
//! (`crate::eval_api`).
//!
//! Per mode, per query, the module runs the same work a search route runs,
//! three times:
//!
//! - a **flat** run, `k = 10`, the timed one: its durations feed the latency
//!   table and its keys feed the fidelity;
//! - the same flat run with `exact: true`, the **fidelity reference**
//!   (ADR-0076, decision 1);
//! - a run **grouped by document**, `k` documents, the **quality** run
//!   (ADR-0092, decision 2).
//!
//! The three runs come from a [`Runner`]: the local pipeline, or the route of
//! an instance. The metrics below are computed once for both modes, so the two
//! reports cannot drift apart (ADR-0131, decision 8).
//!
//! One **warmup pass** precedes the `--repeat` counted repetitions and enters no
//! percentile (ADR-0125, decision 9). Only the first counted repetition pays the
//! exact and grouped runs: they produce figures, not latency samples.
//!
//! # Metrics
//!
//! - **fidelity**: recall@10 at the passage, `|normal ∩ exact| / |exact|`, and
//!   overlap@10 at the document, against the same pipeline in `exact: true`;
//! - **quality**: recall@k and MRR@k at the document, on the grouped answer,
//!   over the queries that carry a non-empty `expected`;
//! - **latency**: p50, p95 and p99 per stage and for `total`, at the nearest
//!   rank (ADR-0117, decision 4), in milliseconds. In API mode only `total` is
//!   measured, and it is the HTTP round trip seen by the client (ADR-0131,
//!   decision 7). They are indicative and never change the exit code;
//! - **baseline**: the mean drops of quality and fidelity of a previous report,
//!   and the queries that lost an expected document.
//!
//! A query whose exact answer is empty carries no fidelity figure: nothing was
//! lost, and nothing is invented. A mode whose fidelity has no sample omits the
//! member, as an absent stage omits its column (ADR-0119, decision 3).
//!
//! # The report
//!
//! Two files, named `eval-<game>-<AAAA-MM-JJ>`, written into `--out` and never
//! over an existing file (ADR-0125, decisions 11 and 13): a closed JSON of
//! `format_version: 3`, and a Markdown that follows the model of
//! `docs/mesures/README.md`. Version 2 added `source` (the mode, and the URL in
//! API mode) and a `latency_basis` per mode, so a local and an API campaign
//! cannot be confused (ADR-0131, decision 7). Version 3 adds the replayable
//! top-k of ADR-0155: every per-query row of every mode carries `doc_ids`, the
//! ordered external `doc_id`s of the grouped run, always present and `[]` when
//! the run rendered nothing; the local header carries `model_fingerprint`
//! (omitted for an `external` model, and by a campaign given no `--models`),
//! `query_precision` (`"fp32"`, only when a dense text was encoded) and
//! `queries_fingerprint`; the API header carries `queries_fingerprint` alone.
//! `--baseline` reads versions 1, 2 and 3, compares the top-k of two campaigns
//! and declares what its differences can be attributed to, without ever
//! changing the exit code. Neither report carries the text of a query, the
//! value of a filter, an internal id or a `doc_id#ord` key: a query is
//! identified by its `id` alone (ADR-0125, decision 16), and a result by the
//! external `doc_id`s of ADR-0155, decision 8.
//!
//! ```text
//! cargo test -p crible --test eval_cli
//! cargo test -p crible --test eval_api_cli
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crible_core::config::IndexConfig;
use crible_core::error::CribleError;
use crible_core::filter::Filter;
use crible_core::model::ModelKind;
use crible_core::path::path_component;
use crible_search::api;
use crible_search::dense::Strategy;
use crible_search::dense::exact::DenseSearcher;
use crible_search::group::Grouping;
use crible_search::pipeline::{self, Mode, Request as PipelineRequest};
use crible_search::pool::Pool;
use crible_search::scan_pool::ScanPool;
use crible_server::analytics::StageDurations;
use crible_server::{TIMING_KEYS, round_ms};
use crible_store::generation::{GenerationStore, Snapshot};
use serde_json::{Map, Value, json};

use crate::commands::{self, SearchMode, TextEncoder};
use crate::eval_api::{self, Preconditions};

/// Number of results compared by the fidelity runs (ADR-0076, decision 1).
const FIDELITY_K: usize = 10;

/// Number of counted repetitions when `--repeat` is absent (ADR-0125,
/// decision 9).
pub const DEFAULT_REPEAT: usize = 3;

/// Largest absolute drop of a mean that still passes `--baseline`
/// (ADR-0125, decision 12).
const REGRESSION_THRESHOLD: f64 = 0.005;

/// Version of the report format (ADR-0125, decision 11; ADR-0131, decision 7;
/// ADR-0155, decision 1).
const FORMAT_VERSION: u64 = 3;

/// Versions of a reference report `--baseline` reads (ADR-0131, decision 7;
/// ADR-0155, decision 1).
///
/// Version 1 carries no `latency_basis`, and versions 1 and 2 carry no
/// `doc_ids` and no fingerprint: their latencies are read, and both the
/// latency and the top-k comparisons say what they could not compare.
const BASELINE_VERSIONS: [u64; 3] = [1, 2, 3];

/// How a campaign measured its figures (ADR-0131, decision 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Basis {
    /// Local mode: the stages of the pipeline, as `debug.timings_ms` carries
    /// them.
    ServerStages,
    /// API mode: the HTTP round trip, measured by the client.
    ClientRoundTrip,
}

impl Basis {
    /// Value of the `latency_basis` member of a mode.
    const fn as_str(self) -> &'static str {
        match self {
            Self::ServerStages => "server_stages",
            Self::ClientRoundTrip => "client_round_trip",
        }
    }
}

/// Seconds in a day, for the date of a campaign.
const SECONDS_PER_DAY: u64 = 86_400;

/// One `crible eval` invocation.
#[derive(Clone, Copy)]
pub struct EvalRequest<'a> {
    /// Root data directory (`<data>/indexes/<index>`); required without `url`
    /// (ADR-0131, decision 6).
    pub data: Option<&'a Path>,
    /// Base of a running instance, `http://host:port`; the mode is API when it
    /// is present (ADR-0131, decision 1).
    pub url: Option<&'a str>,
    /// Index name, as declared in its `index.json`.
    pub index: &'a str,
    /// JSONL game of queries (ADR-0125, decision 3).
    pub queries: &'a Path,
    /// Root models directory; required when a `dense` or `hybrid` mode has to
    /// encode a textual query of the game (ADR-0125, decision 2), and refused
    /// in API mode (ADR-0131, decision 6).
    pub models: Option<&'a Path>,
    /// Branches to evaluate; `dense` when absent.
    pub modes: &'a [SearchMode],
    /// Number of documents of the quality runs.
    pub k: usize,
    /// Directory the two reports are written into; never under the data root.
    pub out: &'a Path,
    /// In-memory replacement of `search.exact_threshold`; `0` when absent, and
    /// refused in API mode (ADR-0131, decisions 6 and 9).
    pub exact_threshold: Option<u32>,
    /// In-memory replacement of `search.rescore_candidates`, refused in API mode
    /// (ADR-0131, decisions 6 and 9).
    pub rescore_candidates: Option<u32>,
    /// Number of counted repetitions; [`DEFAULT_REPEAT`] when absent.
    pub repeat: usize,
    /// Reference report to compare against.
    pub baseline: Option<&'a Path>,
}

/// One query of a game, parsed and validated.
///
/// The filter stays as its JSON text: step 1 of the pipeline parses and
/// validates it on every run, so its duration is a real `validate` sample
/// (ADR-0125, decision 9).
#[derive(Debug, Clone)]
struct Query {
    /// Identifier of the query in its game; the only name the report carries.
    id: String,
    /// Text of the query, encoded by the model of the index.
    text: Option<String>,
    /// Provided vector, used as it is (ADR-0125, decision 2).
    vector: Option<Vec<f32>>,
    /// Filter, in the grammar of `docs/05-api.md`, kept as JSON text.
    filter: Option<String>,
    /// Documents the query expects, deduplicated and in file order.
    expected: Vec<String>,
}

/// Answer of one pipeline run, cut to what the metrics and the report need.
struct Run {
    /// Keys `doc_id#ord` of the results, best first.
    keys: Vec<String>,
    /// External `doc_id` of the results, best first.
    docs: Vec<String>,
    /// Durations of the run, per stage of `docs/03-recherche.md`.
    stages: StageDurations,
    /// Strategy the dense branch ran, when a dense branch ran.
    strategy: Option<Strategy>,
}

/// Figures of one query in one mode.
#[derive(Default)]
struct QueryOutcome {
    /// Identifier of the query.
    id: String,
    /// External `doc_id`s of the grouped run, best first, as the run rendered
    /// them (ADR-0155, decision 2).
    doc_ids: Vec<String>,
    /// recall@10 at the passage, overlap@10 at the document, and the number of
    /// keys of the exact top-10, which is their denominator (ADR-0125,
    /// complément du 2026-11-01, point C1).
    fidelity: Option<(f64, f64, usize)>,
    /// recall@k, MRR@k, and the indices of `expected` the top-k documents hold.
    quality: Option<(f64, f64, Vec<usize>)>,
}

/// Figures and samples of one mode.
#[derive(Default)]
struct ModeReport {
    /// One row per query, in file order.
    rows: Vec<QueryOutcome>,
    /// Fidelity samples: recall@10 then overlap@10.
    fidelity: Vec<(f64, f64)>,
    /// Queries whose exact answer is empty: they carry no fidelity figure and
    /// are counted apart (ADR-0125, complément du 2026-11-01, point C1).
    fidelity_excluded: Vec<String>,
    /// Quality samples: recall@k then MRR@k.
    quality: Vec<(f64, f64)>,
    /// Number of queries the plan sent to each dense strategy.
    strategies: BTreeMap<&'static str, u64>,
    /// Latency samples per stage, in milliseconds.
    latencies: BTreeMap<&'static str, Vec<f64>>,
}

/// What a local campaign can say about its encoder (ADR-0155, decision 3).
///
/// The API mode builds none: the encoder and its precision are preconditions of
/// the evaluated instance, and the campaign reads no value of them
/// (ADR-0131, decision 9).
#[derive(Default)]
struct EncoderHeader {
    /// `ModelConfig::fingerprint` of an `onnx` model; `None` for an `external`
    /// one, whose configuration says nothing about the encoder, for a campaign
    /// given no models root, which read no configuration at all, and for a
    /// campaign that ran no dense branch, which read no encoder either.
    model_fingerprint: Option<String>,
    /// True when the campaign encoded a dense query text.
    encoded_text: bool,
}

/// State of one local campaign: what every run reads.
struct Campaign<'a> {
    /// Configuration of the index, with the in-memory overrides applied.
    config: IndexConfig,
    /// Published generation the whole campaign reads.
    snapshot: &'a Snapshot,
    /// Readers of the segments, opened once for the campaign.
    searcher: &'a DenseSearcher,
    /// Thread pool of the dense scan, as the search command installs it.
    scan: &'a ScanPool,
    /// Root models directory, for a game with textual queries.
    models: Option<&'a Path>,
    /// Encoder, built on the first textual query and reused after (ADR-0051).
    encoder: Option<TextEncoder>,
}

/// What one campaign can run: the local pipeline, or the route of an instance
/// (ADR-0131, decision 1).
///
/// The metrics of [`evaluate`] read runs through this trait, so the two modes
/// share them instead of carrying two copies (ADR-0131, decision 8).
trait Runner {
    /// Runs `plan` once for `query` in `mode`.
    fn run(&mut self, mode: Mode, query: &Query, plan: &Plan) -> Result<Run, CribleError>;
}

/// One run of a campaign, as the metrics ask for it.
#[derive(Debug, Clone, Copy)]
struct Plan {
    /// Fidelity reference of ADR-0076, decision 1.
    exact: bool,
    /// Quality run: grouped by document, one passage per document.
    grouped: bool,
    /// Number of results, or of documents under `grouped`.
    k: usize,
}

/// `crible eval --data <dir> …` or `crible eval --url <base> …`.
///
/// Returns [`ExitCode::SUCCESS`], or `1` when `--baseline` detects a regression
/// of a mean (ADR-0125, decision 12). The two reports are written before the
/// code is decided, so a regression leaves its trace.
///
/// # Errors
///
/// [`ErrorCode::InvalidRequest`](crible_core::error::ErrorCode::InvalidRequest)
/// for a malformed game, an unusable mode, an option the mode does not take, a
/// refused URL, a missing key, a report that would be overwritten or a refused
/// baseline; the store, search and HTTP errors of the campaign otherwise.
pub fn run(request: &EvalRequest<'_>) -> Result<ExitCode, CribleError> {
    let target = where_of(request)?;
    api::check_k(request.k)?;
    if request.repeat == 0 {
        return Err(commands::invalid_request(
            "--repeat must be at least 1: a campaign counts at least one repetition of its game",
        ));
    }
    let name = path_component(request.index, "index name")?;
    let modes = modes_of(request.modes)?;
    let queries = read_queries(request.queries)?;
    if queries.is_empty() {
        return Err(commands::invalid_request(format!(
            "the game {} holds no query",
            request.queries.display()
        )));
    }
    let game = game_of(request.queries)?;
    let date = today()?;
    let (json_path, markdown_path) = report_paths(request.out, &game, &date)?;

    let (mut report, preconditions) = match &target {
        Where::Local(data) => (
            local_campaign(request, data, name, &modes, &queries, &game, &date)?,
            None,
        ),
        Where::Api(base) => {
            let (report, preconditions) =
                api_campaign(request, base, name, &modes, &queries, &game, &date)?;
            (report, Some(preconditions))
        }
    };

    let regression = match request.baseline {
        Some(path) => {
            let baseline = read_baseline(path)?;
            let (comparison, regression) = compare(&baseline, &report, request.k)?;
            report["baseline"] = comparison;
            regression
        }
        None => false,
    };

    let markdown = markdown(&report, preconditions.as_ref());
    fs::write(&json_path, format!("{}\n", pretty(&report)?)).map_err(|error| {
        commands::internal(format!("cannot write {}: {error}", json_path.display()))
    })?;
    fs::write(&markdown_path, markdown).map_err(|error| {
        commands::internal(format!("cannot write {}: {error}", markdown_path.display()))
    })?;
    println!("json: {}", json_path.display());
    println!("markdown: {}", markdown_path.display());
    println!(
        "regression: {}",
        if regression { "detected" } else { "none" }
    );
    Ok(if regression {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// Where the campaign reads its figures, decided before any I/O.
enum Where<'a> {
    /// The index under `<data>/indexes/<index>`.
    Local(&'a Path),
    /// A running instance, from the base `--url` gives.
    Api(eval_api::Base),
}

/// Decides the mode, and refuses what the command does not take (ADR-0131,
/// decisions 3 and 6).
///
/// `--data` and `--url` are exclusive and one of them is required; the three
/// overrides of the local mode are refused by name in API mode. Every refusal
/// is `invalid_request`, which is the code of every other entry point, and none
/// of them touches the network or the data root.
fn where_of<'a>(request: &EvalRequest<'a>) -> Result<Where<'a>, CribleError> {
    match (request.data, request.url) {
        (Some(_), Some(_)) => Err(commands::invalid_request(
            "--data and --url are exclusive: --data evaluates a local index, --url a running \
             instance, and a campaign measures one of the two (ADR-0131, decision 6)",
        )),
        (None, None) => Err(commands::invalid_request(
            "--data is required without --url: give the data root of the index to evaluate, or \
             --url <base> to evaluate a running instance (ADR-0131, decision 6)",
        )),
        (Some(data), None) => Ok(Where::Local(data)),
        (None, Some(url)) => {
            let refused = [
                (
                    "--models",
                    request.models.is_some(),
                    "the server encodes the queries of the game itself",
                ),
                (
                    "--exact-threshold",
                    request.exact_threshold.is_some(),
                    "`exact_threshold` is a precondition of the evaluated instance, not a field of \
                     a request: change it with the administration `PATCH` or in the configuration",
                ),
                (
                    "--rescore-candidates",
                    request.rescore_candidates.is_some(),
                    "N (rescoring) is a precondition of the evaluated instance, not a field of a \
                     request: change it with the administration `PATCH` or in the configuration",
                ),
            ];
            for (option, given, reason) in refused {
                if given {
                    return Err(commands::invalid_request(format!(
                        "{option} has no meaning in API mode: {reason} (ADR-0131, decisions 6 \
                         and 9)"
                    )));
                }
            }
            Ok(Where::Api(eval_api::Base::parse(url)?))
        }
    }
}

/// One local campaign: the index opened read-only, the pipeline of
/// `crible search`, the report of the local mode.
fn local_campaign(
    request: &EvalRequest<'_>,
    data: &Path,
    name: &str,
    modes: &[Mode],
    queries: &[Query],
    game: &str,
    date: &str,
) -> Result<Value, CribleError> {
    let (index_dir, mut config) = commands::load_index(data, name)?;
    for mode in modes {
        if mode.uses_lexical() && !config.lexical.enabled {
            return Err(commands::invalid_request(format!(
                "--modes {}: the lexical branch is disabled for index '{}'",
                mode.as_str(),
                name
            )));
        }
    }
    // The overrides live in memory only (ADR-0125, decision 7): the
    // configuration file is read, never written back.
    config.search.exact_threshold = request.exact_threshold.unwrap_or(0);
    if let Some(candidates) = request.rescore_candidates {
        config.search.rescore_candidates = candidates;
    }
    // A textual query needs the encoder of the index only when a dense branch
    // has to encode it; a vector-only dense run and a lexical run do not
    // (ADR-0125, decision 2). `synth-small` has no encoder at all.
    let textual = queries.iter().any(|query| query.text.is_some());
    let needs_encoder = textual
        && modes.iter().any(|mode| mode.uses_dense())
        && queries.iter().any(|query| query.vector.is_none());
    if needs_encoder && request.models.is_none() {
        return Err(commands::invalid_request(
            "this game holds textual queries: pass --models <dir>, the [paths] models root of \
             crible.toml, so the model of the index can encode them",
        ));
    }

    let store = GenerationStore::open_read_only(&index_dir, &config.schema)?;
    let state = store.state()?;
    let snapshot = store.snapshot();
    let segments = snapshot.segments().to_vec();
    let segments_directory = store.segments_directory();
    let records_path = store.records_path();
    let scan = ScanPool::new(0)?;
    let searcher = DenseSearcher::open_in(&scan, &segments_directory, &segments, &records_path)?;

    let mut campaign = Campaign {
        config,
        snapshot: &snapshot,
        searcher: &searcher,
        scan: &scan,
        models: request.models,
        encoder: None,
    };
    let mut reports: BTreeMap<&'static str, ModeReport> = BTreeMap::new();
    for mode in modes {
        reports.insert(
            mode.as_str(),
            evaluate(&mut campaign, *mode, queries, request.k, request.repeat)?,
        );
    }

    let mut report = header(
        request,
        name,
        date,
        game,
        state.generation.get(),
        state.live_passages,
        queries,
        modes,
        Some(&EncoderHeader {
            model_fingerprint: if modes.iter().any(|mode| mode.uses_dense()) {
                model_fingerprint_of(request.models, &campaign.config)?
            } else {
                // A purely lexical campaign reads no encoder configuration:
                // `--models` stays without effect there, as before ADR-0155
                // (ADR-0143).
                None
            },
            encoded_text: campaign.encoder.is_some(),
        }),
        json!({ "mode": "local" }),
    )?;
    report.insert(
        "exact_threshold".to_owned(),
        json!(campaign.config.search.exact_threshold),
    );
    report.insert(
        "rescore_candidates".to_owned(),
        json!(campaign.config.search.rescore_candidates),
    );
    report.insert(
        "rescore_candidates_source".to_owned(),
        json!(if request.rescore_candidates.is_some() {
            "--rescore-candidates"
        } else {
            "configuration"
        }),
    );
    report.insert(
        "modes".to_owned(),
        modes_value(&reports, modes, Basis::ServerStages),
    );
    Ok(Value::Object(report))
}

/// One campaign against a running instance (ADR-0131).
///
/// The generation is pinned before the first search, the requests are
/// sequential, and the report carries neither the server stages nor
/// `dense_strategy`: decision 8 keeps `debug` out of every request.
fn api_campaign(
    request: &EvalRequest<'_>,
    base: &eval_api::Base,
    name: &str,
    modes: &[Mode],
    queries: &[Query],
    game: &str,
    date: &str,
) -> Result<(Value, Preconditions), CribleError> {
    let mut session = eval_api::Session::open(base.clone(), name)?;
    let mut reports: BTreeMap<&'static str, ModeReport> = BTreeMap::new();
    for mode in modes {
        reports.insert(
            mode.as_str(),
            evaluate(&mut session, *mode, queries, request.k, request.repeat)?,
        );
    }
    let mut report = header(
        request,
        name,
        date,
        game,
        session.generation(),
        session.pool_size(),
        queries,
        modes,
        None,
        json!({ "mode": "api", "url": session.base() }),
    )?;
    report.insert(
        "modes".to_owned(),
        modes_value(&reports, modes, Basis::ClientRoundTrip),
    );
    Ok((Value::Object(report), session.preconditions()))
}

/// The members both modes carry, with the `source` of the campaign.
///
/// `exact_threshold` and N stay out of it in API mode: decision 9 of ADR-0131
/// makes them preconditions of the instance, and a report that carried them
/// would let a reader believe the campaign applied them.
///
/// `queries_fingerprint` describes the game alone, so both modes write it; the
/// two members that describe the encoder come from `encoder`, which the API
/// mode passes as `None` (ADR-0155, decisions 3 and 4).
#[allow(clippy::too_many_arguments)]
fn header(
    request: &EvalRequest<'_>,
    name: &str,
    date: &str,
    game: &str,
    generation: u64,
    pool_size: u64,
    queries: &[Query],
    modes: &[Mode],
    encoder: Option<&EncoderHeader>,
    source: Value,
) -> Result<Map<String, Value>, CribleError> {
    let queries_with_expected = queries
        .iter()
        .filter(|query| !query.expected.is_empty())
        .count();
    let mut report = Map::new();
    report.insert("format_version".to_owned(), json!(FORMAT_VERSION));
    report.insert(
        "command".to_owned(),
        json!(command_of(request, modes, request.url)),
    );
    report.insert("source".to_owned(), source);
    report.insert("date".to_owned(), json!(date));
    report.insert("game".to_owned(), json!(game));
    report.insert("index".to_owned(), json!(name));
    report.insert("generation".to_owned(), json!(generation));
    report.insert("pool_size".to_owned(), json!(pool_size));
    report.insert("version".to_owned(), json!(env!("CARGO_PKG_VERSION")));
    report.insert("k".to_owned(), json!(request.k));
    report.insert("repeat".to_owned(), json!(request.repeat));
    report.insert("queries".to_owned(), json!(queries.len()));
    report.insert(
        "queries_with_expected".to_owned(),
        json!(queries_with_expected),
    );
    report.insert(
        "queries_fingerprint".to_owned(),
        json!(queries_fingerprint(queries)?),
    );
    if let Some(encoder) = encoder {
        // An `external` model has no encoder to describe, so its configuration
        // is not hashed: the member is omitted rather than misleading
        // (ADR-0155, decision 3).
        if let Some(fingerprint) = &encoder.model_fingerprint {
            report.insert("model_fingerprint".to_owned(), json!(fingerprint));
        }
        // A campaign that encoded no dense text encoded no query, and writing
        // `"fp32"` there would be false (ADR-0155, decision 3).
        if encoder.encoded_text {
            report.insert("query_precision".to_owned(), json!("fp32"));
        }
    }
    Ok(report)
}

/// `ModelConfig::fingerprint` of the model of `config`, when it has one.
///
/// The model configuration is read under the models root `--models` gives, as
/// the encoder reads it. An `onnx` model carries its identifier, its revision
/// and the digests of its files, so its fingerprint says which encoder ran; an
/// `external` model carries neither, so no fingerprint is written
/// (ADR-0155, decision 3). A campaign given no models root read no
/// configuration and writes none either. The caller asks only when the campaign
/// runs a dense branch: a purely lexical campaign reads no encoder
/// configuration, and `--models` keeps having no effect on it (ADR-0143).
fn model_fingerprint_of(
    models: Option<&Path>,
    config: &IndexConfig,
) -> Result<Option<String>, CribleError> {
    let Some(models) = models else {
        return Ok(None);
    };
    let model = commands::load_model(models, &config.model)?;
    Ok(match model.kind {
        ModelKind::Onnx => Some(model.fingerprint()),
        ModelKind::External => None,
    })
}

/// blake3 of the canonical form of the parsed game (ADR-0155, decision 3).
///
/// One line per query, in the file order; keys sorted, compact JSON, non-ASCII
/// characters left as UTF-8, which is the form of ADR-0028. The members a query
/// does not carry are not written, and the hash covers the **parsed** game, not
/// the bytes: respacing a file or reordering its keys keeps the fingerprint.
fn queries_fingerprint(queries: &[Query]) -> Result<String, CribleError> {
    let mut hasher = blake3::Hasher::new();
    for query in queries {
        let mut line = Map::new();
        line.insert("id".to_owned(), json!(query.id));
        if let Some(text) = &query.text {
            line.insert("query".to_owned(), json!(text));
        }
        if let Some(vector) = &query.vector {
            line.insert("vector".to_owned(), json!(vector));
        }
        if let Some(raw) = &query.filter {
            // The filter is kept as text for the timed `validate` step; the
            // canonical form writes the object, whose keys are sorted here as
            // every level of the form is.
            // The game is validated before it is run, so this path is not
            // reachable in practice; it stays an invalid request rather than an
            // internal fault, because the text comes from the caller's file.
            let filter: Value = serde_json::from_str(raw).map_err(|error| {
                commands::invalid_request(format!(
                    "query '{}': the filter of the game is not JSON: {error}",
                    query.id
                ))
            })?;
            line.insert("filter".to_owned(), filter);
        }
        if !query.expected.is_empty() {
            line.insert("expected".to_owned(), json!(query.expected));
        }
        hasher.update(Value::Object(line).to_string().as_bytes());
        hasher.update(b"\n");
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// The command line of the campaign, as the reader can replay it.
///
/// The report carries the paths such as the command received them (ADR-0125,
/// decision 11), so the line is complete without any secret nor any query text.
/// In API mode the line carries `--url` and never `--data` (ADR-0131,
/// decision 6).
fn command_of(request: &EvalRequest<'_>, modes: &[Mode], url: Option<&str>) -> String {
    let mut parts = vec!["crible eval".to_owned()];
    match (url, request.data) {
        (Some(base), _) => parts.push(format!("--url {base}")),
        (None, Some(data)) => parts.push(format!("--data {}", data.display())),
        (None, None) => {}
    }
    parts.push(format!("--index {}", request.index));
    parts.push(format!("--queries {}", request.queries.display()));
    if let Some(models) = request.models {
        parts.push(format!("--models {}", models.display()));
    }
    parts.push(format!(
        "--modes {}",
        modes
            .iter()
            .map(|mode| mode.as_str())
            .collect::<Vec<_>>()
            .join(",")
    ));
    parts.push(format!("--k {}", request.k));
    parts.push(format!("--out {}", request.out.display()));
    if let Some(threshold) = request.exact_threshold {
        parts.push(format!("--exact-threshold {threshold}"));
    }
    if let Some(candidates) = request.rescore_candidates {
        parts.push(format!("--rescore-candidates {candidates}"));
    }
    parts.push(format!("--repeat {}", request.repeat));
    if let Some(baseline) = request.baseline {
        parts.push(format!("--baseline {}", baseline.display()));
    }
    parts.join(" ")
}

/// Evaluates one mode and returns its figures.
///
/// Pass 0 is the uncounted warmup; passes `1..=repeat` are the counted
/// repetitions of ADR-0125, decision 9. Only the first counted pass pays the
/// exact and the grouped run. The metrics come from [`fidelity`] and
/// [`quality`] whatever the [`Runner`], so the local and the API reports share
/// them (ADR-0131, decision 8).
fn evaluate<R: Runner>(
    runner: &mut R,
    mode: Mode,
    queries: &[Query],
    k: usize,
    repeat: usize,
) -> Result<ModeReport, CribleError> {
    let mut report = ModeReport::default();
    for pass in 0..=repeat {
        for query in queries {
            let normal = runner.run(
                mode,
                query,
                &Plan {
                    exact: false,
                    grouped: false,
                    k: FIDELITY_K,
                },
            )?;
            if pass > 0 {
                record_latencies(&mut report.latencies, &normal.stages);
            }
            if pass != 1 {
                continue;
            }
            let strategy = normal.strategy.map(Strategy::as_str);
            if let Some(strategy) = strategy {
                *report.strategies.entry(strategy).or_insert(0) += 1;
            }
            let mut outcome = QueryOutcome {
                id: query.id.clone(),
                ..QueryOutcome::default()
            };
            if mode.uses_dense() {
                let exact = runner.run(
                    mode,
                    query,
                    &Plan {
                        exact: true,
                        grouped: false,
                        k: FIDELITY_K,
                    },
                )?;
                if let Some(fidelity) = fidelity(&normal, &exact) {
                    report.fidelity.push((fidelity.0, fidelity.1));
                    outcome.fidelity = Some(fidelity);
                } else {
                    report.fidelity_excluded.push(query.id.clone());
                }
            }
            let quality_run = runner.run(
                mode,
                query,
                &Plan {
                    exact: false,
                    grouped: true,
                    k,
                },
            )?;
            if let Some(quality) = quality(&quality_run.docs, &query.expected) {
                report.quality.push((quality.0, quality.1));
                outcome.quality = Some(quality);
            }
            // Decision 2: the row keeps the ordered external `doc_id`s of the
            // grouped run, whether or not the query carries an `expected`.
            outcome.doc_ids = quality_run.docs;
            report.rows.push(outcome);
        }
    }
    Ok(report)
}

impl Runner for Campaign<'_> {
    /// Runs the local pipeline once for `query`, in `mode`.
    fn run(&mut self, mode: Mode, query: &Query, plan: &Plan) -> Result<Run, CribleError> {
        self.run_once(mode, query, plan)
    }
}

impl Runner for eval_api::Session {
    /// Runs the request once against the instance, and reads what the metrics
    /// need from the answer.
    ///
    /// The filter of the line travels as it was parsed, and no `debug` flag is
    /// sent: the answer carries no server stage, so `total` alone feeds the
    /// latency table (ADR-0131, decision 8).
    fn run(&mut self, mode: Mode, query: &Query, plan: &Plan) -> Result<Run, CribleError> {
        let filter = match &query.filter {
            Some(raw) => Some(serde_json::from_str::<Value>(raw).map_err(|error| {
                commands::internal(format!(
                    "query '{}': the filter of the game is not JSON: {error}",
                    query.id
                ))
            })?),
            None => None,
        };
        let text = match mode {
            Mode::Dense => query.text.as_deref(),
            Mode::Lexical | Mode::Hybrid => Some(text_of(query, mode)?),
        };
        let answer = self
            .search(&eval_api::Search {
                mode,
                text,
                vector: query.vector.as_deref(),
                filter: filter.as_ref(),
                k: plan.k,
                exact: plan.exact,
                grouped: plan.grouped,
            })
            .map_err(|error| {
                // Decision 5: the first error names the `id` of the query of
                // the game, never its text, and keeps the code of the failure.
                CribleError::new(
                    error.code(),
                    format!("query '{}': {}", query.id, error.message()),
                )
            })?;
        let stages = StageDurations {
            total: Some(answer.elapsed),
            ..StageDurations::default()
        };
        Ok(Run {
            keys: answer.keys,
            docs: answer.docs,
            stages,
            strategy: None,
        })
    }
}

impl Campaign<'_> {
    /// Runs the pipeline once for `query`, in `mode`, under the plan.
    fn run_once(&mut self, mode: Mode, query: &Query, plan: &Plan) -> Result<Run, CribleError> {
        // The grouped run of the quality metrics is the one of `crible search`
        // and of the route, built and validated by the same constructor.
        let grouping = if plan.grouped {
            api::grouping(true, 1, false)?
        } else {
            Grouping::none()
        };
        let (exact, k) = (plan.exact, plan.k);
        let started = Instant::now();
        // Step 1: the filter is parsed and validated on every run, as a route
        // does for every request, so `validate` is a measured sample.
        let validation = Instant::now();
        let filter = filter_of(query, &self.config)?;
        let validate = validation.elapsed();
        // Step 2: a text query is encoded, once per run, by the cache of the
        // encoder; a provided vector is used as it is.
        let encoding = Instant::now();
        let mut encoded = false;
        let (vector, text) = match mode {
            Mode::Dense => match (&query.vector, query.text.as_deref()) {
                (Some(vector), _) => (Some(vector.clone()), None),
                (None, Some(text)) => {
                    encoded = true;
                    (Some(self.encode(text)?), None)
                }
                (None, None) => {
                    return Err(commands::invalid_request(format!(
                        "query '{}': a dense run needs a query text or a vector",
                        query.id
                    )));
                }
            },
            Mode::Lexical => (None, Some(text_of(query, mode)?)),
            Mode::Hybrid => {
                let text = text_of(query, mode)?;
                let vector = match &query.vector {
                    Some(vector) => Some(vector.clone()),
                    None => {
                        encoded = true;
                        Some(self.encode(text)?)
                    }
                };
                (vector, Some(text))
            }
        };
        let encode = encoded.then(|| encoding.elapsed());
        // Step 3: the pool, computed once and shared by the branches (INV-05).
        let pooling = Instant::now();
        let pool = Pool::evaluate(self.snapshot.filters(), &filter)?;
        let filter_duration = pooling.elapsed();
        let snapshot = self.snapshot;
        let searcher = self.searcher;
        let search = &self.config.search;
        let answer = self.scan.install(|| {
            pipeline::search(
                snapshot,
                searcher,
                &pool,
                &PipelineRequest {
                    mode,
                    vector: vector.as_deref(),
                    text,
                    k,
                    exact,
                    grouping: &grouping,
                    fusion: search.fusion,
                },
                search,
            )
        })?;
        let mut stages = StageDurations {
            validate: Some(validate),
            encode,
            filter: Some(filter_duration),
            ..StageDurations::default()
        };
        if mode.uses_dense() {
            stages.dense = Some(answer.stages.dense);
        }
        if mode.uses_lexical() {
            stages.lexical = Some(answer.stages.lexical);
        }
        if mode == Mode::Hybrid {
            stages.fusion = Some(answer.stages.fusion);
        }
        stages.group = Some(answer.stages.group);
        stages.fetch = Some(answer.stages.fetch);
        stages.total = Some(started.elapsed());
        let mut keys = Vec::with_capacity(answer.results.len());
        let mut docs = Vec::with_capacity(answer.results.len());
        for result in answer.results {
            keys.push(result.hit.key);
            docs.push(result.hit.doc_id);
        }
        Ok(Run {
            keys,
            docs,
            stages,
            strategy: answer.strategy,
        })
    }

    /// Vector of a textual query, encoded by the model of the index.
    fn encode(&mut self, text: &str) -> Result<Vec<f32>, CribleError> {
        if self.encoder.is_none() {
            let encoder = commands::text_encoder(self.models, &self.config, None)?;
            self.encoder = Some(encoder);
        }
        let encoder = self
            .encoder
            .as_ref()
            .ok_or_else(|| commands::internal("the encoder of the campaign was not built"))?;
        let (vector, _truncated) = encoder.encode(text)?;
        Ok(vector)
    }
}

/// Modes to evaluate, in the order given, without duplicates.
fn modes_of(modes: &[SearchMode]) -> Result<Vec<Mode>, CribleError> {
    let mut kept: Vec<Mode> = Vec::new();
    for mode in modes {
        let mode = Mode::from(*mode);
        if !kept.contains(&mode) {
            kept.push(mode);
        }
    }
    if kept.is_empty() {
        kept.push(Mode::Dense);
    }
    Ok(kept)
}

/// Filter of one query, parsed and validated against the schema of the index.
fn filter_of(query: &Query, config: &IndexConfig) -> Result<Filter, CribleError> {
    match &query.filter {
        Some(raw) => {
            let filter = Filter::from_json_str(raw)?;
            filter.validate(&config.schema)?;
            Ok(filter)
        }
        None => Ok(Filter::And(Vec::new())),
    }
}

/// Text a mode needs, refused with the `id` of the query when absent.
///
/// The message names the query of the game, never its text nor a filter value
/// (ADR-0125, decision 16).
fn text_of(query: &Query, mode: Mode) -> Result<&str, CribleError> {
    query.text.as_deref().ok_or_else(|| {
        commands::invalid_request(format!(
            "query '{}': mode {} scores a text, and this line carries a vector only; give it a \
             `query` text, or evaluate another mode",
            query.id,
            mode.as_str()
        ))
    })
}

/// Fidelity of one flat run against its exact reference.
///
/// recall@10 is `|normal ∩ exact| / |exact|` at the passage, overlap@10 the
/// same ratio at the document (ADR-0076, decision 1; ADR-0125, complément du
/// 2026-11-01, point C1). `|exact|` is `10` as soon as the pool holds ten
/// passages, so the figure is the one of ADR-0076 there; on a smaller pool it
/// stays just instead of being inflated by a fixed divisor. An empty exact
/// answer has no denominator: the query carries no figure and is counted apart
/// by the report.
fn fidelity(normal: &Run, exact: &Run) -> Option<(f64, f64, usize)> {
    let exact_keys: BTreeSet<&String> = exact.keys.iter().collect();
    let exact_docs: BTreeSet<&String> = exact.docs.iter().collect();
    if exact_keys.is_empty() || exact_docs.is_empty() {
        return None;
    }
    let normal_keys: BTreeSet<&String> = normal.keys.iter().collect();
    let kept = exact_keys.intersection(&normal_keys).count();
    let recall = kept as f64 / exact_keys.len() as f64;
    let normal_docs: BTreeSet<&String> = normal.docs.iter().collect();
    let overlap = exact_docs.intersection(&normal_docs).count() as f64 / exact_docs.len() as f64;
    Some((recall, overlap, exact_keys.len()))
}

/// Quality of one grouped answer: recall@k, MRR@k, and the indices of
/// `expected` the answer holds.
///
/// A query without `expected` carries no figure (ADR-0125, decision 4).
fn quality(docs: &[String], expected: &[String]) -> Option<(f64, f64, Vec<usize>)> {
    if expected.is_empty() {
        return None;
    }
    let mut found = Vec::new();
    let mut reciprocal = 0.0;
    for (index, wanted) in expected.iter().enumerate() {
        if let Some(position) = docs.iter().position(|doc| doc == wanted) {
            found.push(index);
            if reciprocal == 0.0 {
                reciprocal = 1.0 / (position as f64 + 1.0);
            }
        }
    }
    let recall = found.len() as f64 / expected.len() as f64;
    Some((recall, reciprocal, found))
}

/// Adds the durations of one run to the samples of its mode.
///
/// The keys are [`crible_server::TIMING_KEYS`], so a column of the report and a
/// key of `debug.timings_ms` cannot drift apart (ADR-0125, decision 9). A stage
/// the run did not execute has no sample and no column.
fn record_latencies(samples: &mut BTreeMap<&'static str, Vec<f64>>, stages: &StageDurations) {
    for (key, duration) in TIMING_KEYS.iter().zip(stages.stages()) {
        if let Some(duration) = duration {
            samples
                .entry(*key)
                .or_default()
                .push(round_ms(duration.as_secs_f64() * 1_000.0));
        }
    }
}

/// Percentile at the nearest rank of ADR-0117, decision 4.
fn percentile(samples: &[f64], percent: f64) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = ((percent / 100.0) * sorted.len() as f64).ceil();
    // A percentile is bounded by `100`, so the rank fits a `usize`; the cast
    // saturates rather than wrapping if it ever did not.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let rank = rank.max(1.0) as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted.get(index).copied()
}

/// Mean of one side of the samples, with its `n`.
fn mean(samples: impl Iterator<Item = f64>, n: usize) -> f64 {
    if n == 0 {
        return 0.0;
    }
    let total: f64 = samples.sum();
    total / n as f64
}

/// The `modes` member of the report: one object per mode, in evaluation order.
fn modes_value(
    reports: &BTreeMap<&'static str, ModeReport>,
    modes: &[Mode],
    basis: Basis,
) -> Value {
    let mut object = Map::new();
    for mode in modes {
        if let Some(report) = reports.get(mode.as_str()) {
            object.insert(mode.as_str().to_owned(), mode_value(report, *mode, basis));
        }
    }
    Value::Object(object)
}

/// One mode of the report: strategies, means, latency, per-query rows.
///
/// `dense_strategy` is a member of the local mode only: the API mode keeps
/// `debug` out of its requests, so the answer carries no plan and no stage
/// (ADR-0131, decision 8). `latency_basis` names what the latency column
/// measures, and keeps a local report from being read as an API one (decision
/// 7).
fn mode_value(report: &ModeReport, mode: Mode, basis: Basis) -> Value {
    let mut object = Map::new();
    if mode.uses_dense() {
        if basis == Basis::ServerStages {
            let counts: Map<String, Value> = report
                .strategies
                .iter()
                .map(|(strategy, count)| ((*strategy).to_owned(), json!(count)))
                .collect();
            object.insert("dense_strategy".to_owned(), Value::Object(counts));
        }
        object.insert(
            "fidelity_excluded".to_owned(),
            json!(report.fidelity_excluded),
        );
    }
    if !report.fidelity.is_empty() {
        let n = report.fidelity.len();
        object.insert(
            "fidelity".to_owned(),
            json!({
                "recall_at_10": mean(report.fidelity.iter().map(|sample| sample.0), n),
                "overlap_at_10": mean(report.fidelity.iter().map(|sample| sample.1), n),
                "n": n,
            }),
        );
    }
    let n = report.quality.len();
    let mut quality = Map::new();
    if n > 0 {
        quality.insert(
            "recall_at_k".to_owned(),
            json!(mean(report.quality.iter().map(|sample| sample.0), n)),
        );
        quality.insert(
            "mrr_at_k".to_owned(),
            json!(mean(report.quality.iter().map(|sample| sample.1), n)),
        );
    }
    quality.insert("n".to_owned(), json!(n));
    object.insert("quality".to_owned(), Value::Object(quality));
    let mut latency = Map::new();
    latency.insert("latency_basis".to_owned(), json!(basis.as_str()));
    for key in TIMING_KEYS {
        let Some(samples) = report.latencies.get(key) else {
            continue;
        };
        if let (Some(p50), Some(p95), Some(p99)) = (
            percentile(samples, 50.0),
            percentile(samples, 95.0),
            percentile(samples, 99.0),
        ) {
            latency.insert(
                (*key).to_owned(),
                json!({ "p50": p50, "p95": p95, "p99": p99 }),
            );
        }
    }
    object.insert("latency".to_owned(), Value::Object(latency));
    let rows: Vec<Value> = report
        .rows
        .iter()
        .map(|outcome| {
            let mut row = Map::new();
            row.insert("id".to_owned(), json!(outcome.id));
            // Decision 2: always present, and `[]` is a measured top-k, not an
            // absent measurement.
            row.insert("doc_ids".to_owned(), json!(outcome.doc_ids));
            if let Some((recall, overlap, denominator)) = outcome.fidelity {
                row.insert("fidelity_recall_at_10".to_owned(), json!(recall));
                row.insert("fidelity_overlap_at_10".to_owned(), json!(overlap));
                if denominator != FIDELITY_K {
                    row.insert("fidelity_denominator".to_owned(), json!(denominator));
                }
            }
            if let Some((recall, reciprocal, found)) = &outcome.quality {
                row.insert("quality_recall_at_k".to_owned(), json!(recall));
                row.insert("quality_mrr_at_k".to_owned(), json!(reciprocal));
                row.insert("quality_found".to_owned(), json!(found));
            }
            Value::Object(row)
        })
        .collect();
    object.insert("queries".to_owned(), Value::Array(rows));
    Value::Object(object)
}

/// Reads and validates a game of queries (ADR-0125, decision 3).
///
/// The format is closed: an unknown member, a line without exactly one of
/// `query` and `vector`, a malformed JSON or a repeated `id` stops the command
/// with the **line number** and the cause. No message repeats a query text nor a
/// filter value.
fn read_queries(path: &Path) -> Result<Vec<Query>, CribleError> {
    let text = fs::read_to_string(path).map_err(|error| {
        commands::invalid_request(format!("cannot read the game {}: {error}", path.display()))
    })?;
    let mut queries: Vec<Query> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (position, raw) in text.lines().enumerate() {
        let number = position + 1;
        if raw.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(raw)
            .map_err(|error| commands::invalid_request(format!("line {number}: {error}")))?;
        let query = parse_query(&value, number)?;
        if !seen.insert(query.id.clone()) {
            return Err(commands::invalid_request(format!(
                "line {number}: the id '{}' is not unique in the game",
                query.id
            )));
        }
        queries.push(query);
    }
    Ok(queries)
}

/// One query, from one line of the game.
fn parse_query(value: &Value, number: usize) -> Result<Query, CribleError> {
    let object = value.as_object().ok_or_else(|| {
        commands::invalid_request(format!("line {number}: a query must be a JSON object"))
    })?;
    for member in object.keys() {
        if !matches!(
            member.as_str(),
            "id" | "query" | "vector" | "filter" | "expected"
        ) {
            return Err(commands::invalid_request(format!(
                "line {number}: unknown member '{member}'; a query carries id, query or vector, \
                 filter and expected"
            )));
        }
    }
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            commands::invalid_request(format!(
                "line {number}: `id` is missing or is not a non-empty string"
            ))
        })?
        .to_owned();
    let text = match object.get("query") {
        Some(value) => Some(
            value
                .as_str()
                .filter(|text| !text.is_empty())
                .ok_or_else(|| {
                    commands::invalid_request(format!(
                        "line {number} ({id}): `query` must be a non-empty string"
                    ))
                })?
                .to_owned(),
        ),
        None => None,
    };
    let vector = match object.get("vector") {
        Some(value) => Some(parse_vector(value, number, &id)?),
        None => None,
    };
    match (text.is_some(), vector.is_some()) {
        (true, true) => {
            return Err(commands::invalid_request(format!(
                "line {number} ({id}): exactly one of `query` and `vector` is allowed"
            )));
        }
        (false, false) => {
            return Err(commands::invalid_request(format!(
                "line {number} ({id}): one of `query` and `vector` is required"
            )));
        }
        _ => {}
    }
    let filter = match object.get("filter") {
        Some(value) if value.is_object() => Some(value.to_string()),
        Some(_) => {
            return Err(commands::invalid_request(format!(
                "line {number} ({id}): `filter` must be a JSON object"
            )));
        }
        None => None,
    };
    let expected = match object.get("expected") {
        Some(value) => parse_expected(value, number, &id)?,
        None => Vec::new(),
    };
    Ok(Query {
        id,
        text,
        vector,
        filter,
        expected,
    })
}

/// Vector of one query, finite numbers only.
fn parse_vector(value: &Value, number: usize, id: &str) -> Result<Vec<f32>, CribleError> {
    let entries = value
        .as_array()
        .filter(|entries| !entries.is_empty())
        .ok_or_else(|| {
            commands::invalid_request(format!(
                "line {number} ({id}): `vector` must be a non-empty array of numbers"
            ))
        })?;
    let mut vector = Vec::with_capacity(entries.len());
    for entry in entries {
        let number_value = entry.as_f64().ok_or_else(|| {
            commands::invalid_request(format!(
                "line {number} ({id}): `vector` holds a value that is not a number"
            ))
        })?;
        let component = number_value as f32;
        if !component.is_finite() {
            return Err(commands::invalid_request(format!(
                "line {number} ({id}): `vector` holds a value outside the range of a 32-bit float"
            )));
        }
        vector.push(component);
    }
    Ok(vector)
}

/// `expected` of one query: document identifiers, deduplicated, in order.
fn parse_expected(value: &Value, number: usize, id: &str) -> Result<Vec<String>, CribleError> {
    let entries = value.as_array().ok_or_else(|| {
        commands::invalid_request(format!(
            "line {number} ({id}): `expected` must be an array of document ids"
        ))
    })?;
    let mut expected: Vec<String> = Vec::new();
    for entry in entries {
        let doc_id = entry
            .as_str()
            .filter(|doc_id| !doc_id.is_empty())
            .ok_or_else(|| {
                commands::invalid_request(format!(
                    "line {number} ({id}): an `expected` entry must be a non-empty document id"
                ))
            })?;
        if !expected.iter().any(|kept| kept == doc_id) {
            expected.push(doc_id.to_owned());
        }
    }
    Ok(expected)
}

/// Slug of a game: the stem of its file, which names the reports.
fn game_of(path: &Path) -> Result<String, CribleError> {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| {
            commands::invalid_request(format!(
                "the game {} has no file name to name its reports",
                path.display()
            ))
        })?;
    Ok(path_component(stem, "game name")?.to_owned())
}

/// The two report paths, refusing to overwrite an existing file.
///
/// Both paths are checked before anything is written, so a refusal leaves the
/// directory untouched (ADR-0125, decision 13).
fn report_paths(out: &Path, game: &str, date: &str) -> Result<(PathBuf, PathBuf), CribleError> {
    if out.exists() && !out.is_dir() {
        return Err(commands::invalid_request(format!(
            "--out {} exists and is not a directory",
            out.display()
        )));
    }
    fs::create_dir_all(out)
        .map_err(|error| commands::internal(format!("cannot create {}: {error}", out.display())))?;
    let stem = out.join(format!("eval-{game}-{date}"));
    let json_path = stem.with_extension("json");
    let markdown_path = stem.with_extension("md");
    for path in [&json_path, &markdown_path] {
        if path.exists() {
            return Err(commands::invalid_request(format!(
                "{} already exists: a dated campaign is never overwritten, choose another --out or \
                 another day",
                path.display()
            )));
        }
    }
    Ok((json_path, markdown_path))
}

/// Today's date, `AAAA-MM-JJ`, in UTC.
fn today() -> Result<String, CribleError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| commands::internal(format!("the system clock is before 1970: {error}")))?;
    let days = i64::try_from(elapsed.as_secs() / SECONDS_PER_DAY)
        .map_err(|error| commands::internal(format!("the date is out of range: {error}")))?;
    Ok(civil_date(days))
}

/// `AAAA-MM-JJ` of a day offset from 1970-01-01.
///
/// Howard Hinnant's civil-from-days algorithm, so the date costs no dependency
/// (ADR-0125, decision 17).
fn civil_date(days: i64) -> String {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_position = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_position + 2) / 5 + 1;
    let month = if month_position < 10 {
        month_position + 3
    } else {
        month_position - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}")
}

/// Reads a reference report, refusing a version this command does not know.
///
/// Versions 1, 2 and 3 are read (ADR-0131, decision 7; ADR-0155, decision 1):
/// version 1 carries no `latency_basis` and versions 1 and 2 carry no
/// `doc_ids` and no fingerprint, so their latencies and their top-k are said
/// non-comparable rather than invented.
fn read_baseline(path: &Path) -> Result<Value, CribleError> {
    let raw = fs::read_to_string(path).map_err(|error| {
        commands::invalid_request(format!(
            "cannot read the baseline {}: {error}",
            path.display()
        ))
    })?;
    let value: Value = serde_json::from_str(&raw).map_err(|error| {
        commands::invalid_request(format!(
            "the baseline {} is not JSON: {error}",
            path.display()
        ))
    })?;
    let version = value.get("format_version").and_then(Value::as_u64);
    if !version.is_some_and(|version| BASELINE_VERSIONS.contains(&version)) {
        let read: Vec<String> = BASELINE_VERSIONS.iter().map(u64::to_string).collect();
        return Err(commands::invalid_request(format!(
            "the baseline {} carries format_version {}, and this command reads {} only",
            path.display(),
            version.map_or_else(|| "none".to_owned(), |version| version.to_string()),
            read.join(", ")
        )));
    }
    Ok(value)
}

/// Compares the current report to a baseline: means, lost queries, top-k,
/// latencies, attribution.
///
/// A regression is a mean of quality or fidelity that drops by more than
/// [`REGRESSION_THRESHOLD`] in absolute value (ADR-0125, decision 12). Latencies
/// and top-k are compared and reported, and neither ever changes the verdict
/// (ADR-0131, decision 7; ADR-0155, decision 5). A mode the baseline does not
/// carry is not compared, and the report says so.
fn compare(baseline: &Value, current: &Value, k: usize) -> Result<(Value, bool), CribleError> {
    let baseline_k = baseline.get("k").and_then(Value::as_u64);
    if baseline_k != Some(k as u64) {
        return Err(commands::invalid_request(format!(
            "the baseline was measured at k = {}, and this campaign at k = {}: the means are not \
             comparable",
            baseline_k.map_or_else(|| "none".to_owned(), |k| k.to_string()),
            k
        )));
    }
    let reference_version = baseline.get("format_version").and_then(Value::as_u64);
    let mut regression = false;
    let mut modes = Map::new();
    let mut not_compared: Vec<String> = Vec::new();
    let current_modes = current.get("modes").and_then(Value::as_object);
    let baseline_modes = baseline.get("modes").and_then(Value::as_object);
    for (name, mode) in current_modes.into_iter().flatten() {
        let Some(reference) = baseline_modes.and_then(|modes| modes.get(name)) else {
            not_compared.push(name.clone());
            continue;
        };
        let mut entry = Map::new();
        // Quality and fidelity carry the means the threshold applies to.
        for family in ["fidelity", "quality"] {
            let mut compared = Map::new();
            for metric in ["recall_at_10", "overlap_at_10", "recall_at_k", "mrr_at_k"] {
                let (Some(previous), Some(now)) = (
                    mean_of(reference, family, metric),
                    mean_of(mode, family, metric),
                ) else {
                    continue;
                };
                let delta = now - previous;
                if metric != "overlap_at_10" && delta < -REGRESSION_THRESHOLD {
                    regression = true;
                }
                compared.insert(
                    metric.to_owned(),
                    json!({ "reference": previous, "current": now, "delta": delta }),
                );
            }
            if !compared.is_empty() {
                entry.insert(family.to_owned(), Value::Object(compared));
            }
        }
        // The queries that lost an expected document the reference found.
        let lost = lost_queries(mode, reference);
        entry.insert("lost".to_owned(), json!(lost));
        // The top-k comparison is informative, and a version 1 or 2 reference
        // has no `doc_ids` to compare (ADR-0155, decisions 1 and 5).
        match top_k_comparison(mode, reference) {
            Some(top_k) => {
                entry.insert("top_k".to_owned(), top_k);
            }
            None => {
                let mut skipped = Map::new();
                if let Some(version) = reference_version {
                    skipped.insert("reference_format_version".to_owned(), json!(version));
                }
                entry.insert("top_k_not_compared".to_owned(), Value::Object(skipped));
            }
        }
        // Latencies: reported, never blocking, and compared only on the same
        // basis (ADR-0131, decision 7).
        match latency_comparison(mode, reference) {
            Ok(latency) => {
                entry.insert("latency".to_owned(), latency);
            }
            Err(current_basis) => {
                let mut uncomparable = Map::new();
                if let Some(basis) = basis_of(reference) {
                    uncomparable.insert("reference_basis".to_owned(), json!(basis));
                }
                if let Some(basis) = current_basis {
                    uncomparable.insert("current_basis".to_owned(), json!(basis));
                }
                entry.insert(
                    "latency_not_compared".to_owned(),
                    Value::Object(uncomparable),
                );
            }
        }
        modes.insert(name.clone(), Value::Object(entry));
    }
    let mut comparison = Map::new();
    comparison.insert("regression".to_owned(), json!(regression));
    comparison.insert("modes".to_owned(), Value::Object(modes));
    comparison.insert("attribution".to_owned(), attribution(baseline, current));
    if !not_compared.is_empty() {
        comparison.insert("modes_not_compared".to_owned(), json!(not_compared));
    }
    Ok((Value::Object(comparison), regression))
}

/// Top-k overlap of one mode: its mean, its `n`, and the queries that moved.
///
/// `overlap_at_k` divides by the size of the **reference** list, so a shorter
/// reference does not dilute the figure; a query whose reference list is empty
/// is excluded from the mean and counted apart, with its `id`; a query present
/// in one report only is counted apart too. The ordered lists are compared, not
/// their sets, so a rank exchange counts as a change (ADR-0155, decision 5).
///
/// Returns `None` when the reference carries no `doc_ids`: a version 1 or 2
/// report cannot be compared that way.
fn top_k_comparison(current: &Value, reference: &Value) -> Option<Value> {
    let before = reference.get("queries")?.as_array()?;
    if !before
        .iter()
        .any(|row| row.get("doc_ids").is_some_and(Value::is_array))
    {
        return None;
    }
    let after: BTreeMap<&str, &Value> = current
        .get("queries")?
        .as_array()?
        .iter()
        .filter_map(|row| row.get("id").and_then(Value::as_str).map(|id| (id, row)))
        .collect();
    let mut samples: Vec<f64> = Vec::new();
    let mut changed: Vec<&str> = Vec::new();
    let mut excluded: Vec<&str> = Vec::new();
    let mut not_compared: Vec<&str> = Vec::new();
    let mut before_ids: Vec<&str> = Vec::new();
    for row in before {
        let Some(id) = row.get("id").and_then(Value::as_str) else {
            continue;
        };
        before_ids.push(id);
        let Some(reference_ids) = doc_ids_of(row) else {
            continue;
        };
        let Some(current_row) = after.get(id).copied() else {
            not_compared.push(id);
            continue;
        };
        let Some(current_ids) = doc_ids_of(current_row) else {
            continue;
        };
        if reference_ids.is_empty() {
            excluded.push(id);
            continue;
        }
        let kept = reference_ids
            .iter()
            .filter(|doc_id| current_ids.contains(doc_id))
            .count();
        samples.push(kept as f64 / reference_ids.len() as f64);
        if reference_ids != current_ids {
            changed.push(id);
        }
    }
    // A query the current campaign carries and the reference does not is
    // counted apart too: decision 5 says a query present in one report only is,
    // whichever report it comes from.
    for id in after.keys() {
        if !before_ids.contains(id) {
            not_compared.push(id);
        }
    }
    let mut top_k = Map::new();
    if !samples.is_empty() {
        top_k.insert(
            "overlap_at_k".to_owned(),
            json!(mean(samples.iter().copied(), samples.len())),
        );
    }
    top_k.insert("n".to_owned(), json!(samples.len()));
    top_k.insert("changed".to_owned(), json!(changed));
    top_k.insert("excluded".to_owned(), json!(excluded));
    if !not_compared.is_empty() {
        top_k.insert("not_compared".to_owned(), json!(not_compared));
    }
    Some(Value::Object(top_k))
}

/// External `doc_id`s of one per-query row, or `None` when it carries none.
fn doc_ids_of(row: &Value) -> Option<Vec<&str>> {
    row.get("doc_ids")?
        .as_array()
        .map(|ids| ids.iter().filter_map(Value::as_str).collect())
}

/// What a comparison can say about the cause of a difference (ADR-0155,
/// decision 6).
///
/// A generation, a model fingerprint or a game fingerprint that differs makes
/// the cause ambiguous, never the figures false: the block is written, and the
/// reference is never refused for it.
fn attribution(baseline: &Value, current: &Value) -> Value {
    let mut generation = Map::new();
    if let Some(reference) = baseline.get("generation") {
        generation.insert("reference".to_owned(), reference.clone());
    }
    if let Some(now) = current.get("generation") {
        generation.insert("current".to_owned(), now.clone());
    }
    let mut attribution = Map::new();
    attribution.insert("generation".to_owned(), Value::Object(generation));
    for member in ["model_fingerprint", "queries_fingerprint"] {
        attribution.insert(
            member.to_owned(),
            json!(fingerprint_state(baseline, current, member)),
        );
    }
    Value::Object(attribution)
}

/// `same`, `differs` or `unknown` for one fingerprint member of two reports.
///
/// `unknown` covers a reference of version 1 or 2, which carries no
/// fingerprint, and the API mode, which writes none for the model: neither
/// side then names what to compare.
fn fingerprint_state(baseline: &Value, current: &Value, member: &str) -> &'static str {
    match (
        baseline.get(member).and_then(Value::as_str),
        current.get(member).and_then(Value::as_str),
    ) {
        (Some(reference), Some(now)) if reference == now => "same",
        (Some(_), Some(_)) => "differs",
        _ => "unknown",
    }
}

/// One mean of one family of a mode, when the report carries it.
fn mean_of(mode: &Value, family: &str, metric: &str) -> Option<f64> {
    mode.get(family)?.get(metric)?.as_f64()
}

/// Ids of the queries whose top-k lost an expected document the reference found.
fn lost_queries(current: &Value, reference: &Value) -> Vec<String> {
    let Some(before) = reference.get("queries").and_then(Value::as_array) else {
        return Vec::new();
    };
    let after: BTreeMap<&str, &Value> = current
        .get("queries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| row.get("id").and_then(Value::as_str).map(|id| (id, row)))
        .collect();
    let mut lost = Vec::new();
    for row in before {
        let Some(id) = row.get("id").and_then(Value::as_str) else {
            continue;
        };
        let found_before = indices(row, "quality_found");
        let found_after = after
            .get(id)
            .map_or_else(Vec::new, |row| indices(row, "quality_found"));
        if found_before
            .iter()
            .any(|index| !found_after.contains(index))
        {
            lost.push(id.to_owned());
        }
    }
    lost
}

/// Indices of the `expected` a per-query row reports as found.
fn indices(row: &Value, member: &str) -> Vec<u64> {
    row.get(member)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .collect()
}

/// Relative gap of the p50 and p95 of every stage both reports carry.
///
/// The comparison happens only when both reports name the same `latency_basis`
/// (ADR-0131, decision 7): the stages of the server and an HTTP round trip are
/// not the same measure, and a version 1 report names no basis at all. The two
/// bases of a refused comparison are returned for the report to say so.
fn latency_comparison<'a>(
    current: &'a Value,
    reference: &'a Value,
) -> Result<Value, Option<&'a str>> {
    let current_basis = basis_of(current);
    if current_basis.is_none() || current_basis != basis_of(reference) {
        return Err(current_basis);
    }
    let before = reference.get("latency").and_then(Value::as_object);
    let after = current.get("latency").and_then(Value::as_object);
    let (Some(before), Some(after)) = (before, after) else {
        return Err(current_basis);
    };
    let mut stages = Map::new();
    for (stage, entry) in after {
        let Some(previous) = before.get(stage) else {
            continue;
        };
        let mut compared = Map::new();
        for percentile in ["p50", "p95"] {
            let (Some(previous), Some(now)) = (
                previous.get(percentile).and_then(Value::as_f64),
                entry.get(percentile).and_then(Value::as_f64),
            ) else {
                continue;
            };
            let relative = if previous > 0.0 {
                (now - previous) / previous
            } else {
                0.0
            };
            compared.insert(
                percentile.to_owned(),
                json!({ "reference": previous, "current": now, "relative": relative }),
            );
        }
        if !compared.is_empty() {
            stages.insert(stage.clone(), Value::Object(compared));
        }
    }
    Ok(Value::Object(stages))
}

/// `latency_basis` of one mode of a report, when it carries one.
fn basis_of(mode: &Value) -> Option<&str> {
    mode.get("latency")?.get("latency_basis")?.as_str()
}
/// Serializes the report as pretty JSON, or reports the fault.
fn pretty(value: &Value) -> Result<String, CribleError> {
    serde_json::to_string_pretty(value)
        .map_err(|error| commands::internal(format!("cannot serialize the report: {error}")))
}

/// Renders the Markdown report of a campaign.
///
/// It follows the model of `docs/mesures/README.md` — goal, machine, data,
/// parameters, results, reading — and carries no text of a query nor a filter
/// value (ADR-0125, decisions 11 and 16).
fn markdown(report: &Value, preconditions: Option<&Preconditions>) -> String {
    let text = |pointer: &str| {
        report
            .pointer(pointer)
            .and_then(Value::as_str)
            .unwrap_or("-")
            .to_owned()
    };
    let number = |pointer: &str| report.pointer(pointer).and_then(Value::as_u64);
    let mut out = String::new();
    out.push_str(&format!(
        "# Éval — {} — {}\n\n",
        text("/game"),
        text("/date")
    ));
    out.push_str("## But\n\n");
    out.push_str(&format!(
        "Mesurer, sur l'index évalué, la fidélité de la recherche rapide contre la recherche \
         exacte, la qualité des résultats contre les documents attendus, et les latences {} \
         Campagne produite par `crible eval` (ADR-0125, ADR-0131).\n\n",
        if preconditions.is_some() {
            "de l'aller-retour HTTP vu par le client. "
        } else {
            "par étape. "
        },
    ));
    out.push_str("## Machine\n\n");
    out.push_str(&format!(
        "- Système : {} {}\n- Cœurs disponibles : {}\n- Version de Crible : {}\n\n",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism()
            .map(|cores| cores.get())
            .unwrap_or(0),
        text("/version"),
    ));
    out.push_str("## Données\n\n");
    out.push_str(&format!(
        "- Index : `{}`\n- Génération : {}\n- Passages vivants : {}\n- Jeu de requêtes : `{}` \
         ({} requêtes, dont {} avec des documents attendus)\n\n",
        text("/index"),
        number("/generation").unwrap_or(0),
        number("/pool_size").unwrap_or(0),
        text("/game"),
        number("/queries").unwrap_or(0),
        number("/queries_with_expected").unwrap_or(0),
    ));
    if preconditions.is_some() {
        out.push_str(&format!("- Instance : {}\n\n", text("/source/url")));
    }
    out.push_str("## Paramètres\n\n");
    out.push_str(&format!("- Commande : `{}`\n", text("/command")));
    out.push_str(&format!(
        "- Modes : {}\n- `k` de la qualité : {}\n",
        report
            .get("modes")
            .and_then(Value::as_object)
            .map(|modes| modes.keys().cloned().collect::<Vec<_>>().join(", "))
            .unwrap_or_else(|| "-".to_owned()),
        number("/k").unwrap_or(0),
    ));
    match preconditions {
        // API mode: the two settings are preconditions of the instance, read
        // from it and never changed by the campaign (ADR-0131, decision 9).
        Some(preconditions) => {
            out.push_str(&format!(
                "- `exact_threshold` : {} (précondition de l'instance, lue sur \
                 `GET /v1/indexes/{}` ; 0 force la stratégie binaire en mémoire, ADR-0076)\n\
                 - `rescore_candidates` (N) : {} (précondition de l'instance, lue sur \
                 `GET /v1/indexes/{}`)\n",
                preconditions.exact_threshold,
                text("/index"),
                preconditions.rescore_candidates,
                text("/index"),
            ));
        }
        None => {
            out.push_str(&format!(
                "- `exact_threshold` : {} (0 force la stratégie binaire en mémoire, ADR-0076)\n\
                 - `rescore_candidates` (N) : {} (valeur de `search.rescore_candidates` de \
                 l'index ; la campagne la remplace quand `--rescore-candidates` est donné). \
                 Source de N : `{}`\n",
                number("/exact_threshold").unwrap_or(0),
                number("/rescore_candidates").unwrap_or(0),
                text("/rescore_candidates_source"),
            ));
        }
    }
    out.push_str(&format!(
        "- Répétitions comptées : {} (une passe d'échauffement non comptée précède)\n",
        number("/repeat").unwrap_or(0),
    ));
    out.push_str("\n## Résultats\n\n");
    if let Some(modes) = report.get("modes").and_then(Value::as_object) {
        for (mode, entry) in modes {
            out.push_str(&format!("### Mode `{mode}`\n\n"));
            if let Some(strategies) = entry.get("dense_strategy").and_then(Value::as_object) {
                let strategies: Vec<String> = strategies
                    .iter()
                    .map(|(name, count)| format!("{name} × {}", count.as_u64().unwrap_or(0)))
                    .collect();
                out.push_str(&format!(
                    "- Stratégie dense du plan : {}\n",
                    strategies.join(", ")
                ));
            }
            if let Some(fidelity) = entry.get("fidelity") {
                out.push_str(&format!(
                    "- Fidélité : recall@10 au passage {:.4}, recouvrement@10 au document {:.4} \
                     (n = {})\n",
                    fidelity
                        .get("recall_at_10")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0),
                    fidelity
                        .get("overlap_at_10")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0),
                    fidelity.get("n").and_then(Value::as_u64).unwrap_or(0),
                ));
            }
            if let Some(quality) = entry.get("quality") {
                let n = quality.get("n").and_then(Value::as_u64).unwrap_or(0);
                if n > 0 {
                    out.push_str(&format!(
                        "- Qualité : recall@k {:.4}, MRR@k {:.4} (n = {n})\n",
                        quality
                            .get("recall_at_k")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0),
                        quality
                            .get("mrr_at_k")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0),
                    ));
                } else {
                    out.push_str(
                        "- Qualité : aucune requête avec des documents attendus (n = 0)\n",
                    );
                }
            }
            if entry.get("fidelity_excluded").is_some() && entry.get("fidelity").is_none() {
                out.push_str("- Fidélité : non calculée, aucune liste exacte non vide\n");
            }
            if let Some(excluded) = entry.get("fidelity_excluded").and_then(Value::as_array)
                && !excluded.is_empty()
            {
                let ids: Vec<String> = excluded
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|id| format!("`{id}`"))
                    .collect();
                out.push_str(&format!(
                    "- Fidélité : {} requête(s) exclue(s), liste exacte vide — {}\n",
                    ids.len(),
                    ids.join(", ")
                ));
            }
            out.push('\n');
            if let Some(latency) = entry.get("latency").and_then(Value::as_object) {
                if preconditions.is_some() {
                    // API mode: the only latency is the client round trip
                    // (ADR-0131, decision 7), so the table says so.
                    out.push_str("Latence côté client (aller-retour HTTP) :\n\n");
                }
                out.push_str("| Étape | p50 (ms) | p95 (ms) | p99 (ms) |\n|---|---|---|---|\n");
                for (stage, values) in latency {
                    // `latency_basis` is a string, not a stage.
                    if !values.is_object() {
                        continue;
                    }
                    out.push_str(&format!(
                        "| `{stage}` | {:.3} | {:.3} | {:.3} |\n",
                        values.get("p50").and_then(Value::as_f64).unwrap_or(0.0),
                        values.get("p95").and_then(Value::as_f64).unwrap_or(0.0),
                        values.get("p99").and_then(Value::as_f64).unwrap_or(0.0),
                    ));
                }
                out.push('\n');
            }
        }
    }
    if let Some(baseline) = report.get("baseline") {
        out.push_str("## Comparaison à la référence\n\n");
        let regression = baseline
            .get("regression")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        out.push_str(&format!(
            "- Régression détectée : {}\n",
            if regression { "oui" } else { "non" }
        ));
        if let Some(modes) = baseline.get("modes").and_then(Value::as_object) {
            for (mode, entry) in modes {
                if let Some(families) = entry.as_object() {
                    for (family, metrics) in families {
                        if !matches!(family.as_str(), "fidelity" | "quality") {
                            continue;
                        }
                        if let Some(metrics) = metrics.as_object() {
                            for (metric, values) in metrics {
                                out.push_str(&format!(
                                    "- `{mode}` {family}.{metric} : {:.4} → {:.4} ({:+.4})\n",
                                    values
                                        .get("reference")
                                        .and_then(Value::as_f64)
                                        .unwrap_or(0.0),
                                    values.get("current").and_then(Value::as_f64).unwrap_or(0.0),
                                    values.get("delta").and_then(Value::as_f64).unwrap_or(0.0),
                                ));
                            }
                        }
                    }
                }
                if let Some(lost) = entry.get("lost").and_then(Value::as_array)
                    && !lost.is_empty()
                {
                    let ids: Vec<String> = lost
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|id| format!("`{id}`"))
                        .collect();
                    out.push_str(&format!(
                        "- `{mode}` : requêtes ayant perdu un attendu — {}\n",
                        ids.join(", ")
                    ));
                }
                if let Some(uncomparable) = entry.get("latency_not_compared") {
                    let reference = uncomparable
                        .get("reference_basis")
                        .and_then(Value::as_str)
                        .unwrap_or("inconnue");
                    let current = uncomparable
                        .get("current_basis")
                        .and_then(Value::as_str)
                        .unwrap_or("inconnue");
                    out.push_str(&format!(
                        "- `{mode}` : latences non comparées, référence en `{reference}` et \
                         campagne en `{current}` (ADR-0131, décision 7)\n"
                    ));
                }
            }
        }
        if let Some(skipped) = baseline.get("modes_not_compared").and_then(Value::as_array)
            && !skipped.is_empty()
        {
            let modes: Vec<String> = skipped
                .iter()
                .filter_map(Value::as_str)
                .map(|mode| format!("`{mode}`"))
                .collect();
            out.push_str(&format!(
                "- Modes absents de la référence, non comparés : {}\n",
                modes.join(", ")
            ));
        }
        out.push('\n');
        top_k_markdown(&mut out, baseline);
    }
    out.push_str("## Lecture\n\n");
    if preconditions.is_some() {
        out.push_str(
            "- En mode API, la seule latence est l'aller-retour HTTP mesuré par le client \
             (`total`) : elle inclut le réseau, la file du serveur et son encodage, et ne se \
             compare pas aux étapes du mode local (`server_stages`). Le serveur encode les \
             requêtes du jeu ; son `exact_threshold` et son N sont des préconditions lues au \
             début de la campagne (ADR-0131, décisions 7 et 9).\n\
             - Le mode API vise la route simple `POST /v1/indexes/{index}/search` : le top-k \
             servi par la route multi-index `POST /v1/search`, celui de Legifacile, n'est pas \
             enregistré ici (ADR-0110, décision 1 ; ADR-0131, décision 1 ; ADR-0155, \
             décision 4).\n\
             - Les requêtes partent l'une après l'autre, sur la génération épinglée au début de \
             la campagne : une génération qui change en cours de route arrête la campagne \
             (ADR-0131, décision 10).\n",
        );
    }
    if report.get("model_fingerprint").is_none() {
        out.push_str(
            "- Le rapport ne porte pas `model_fingerprint` : ni un modèle `external`, qui ne \
             décrit pas son encodeur, ni une campagne sans `--models`, qui n'a lu aucune \
             configuration, ni une campagne sans mode dense, qui n'a lu aucune configuration \
             d'encodeur, ni le mode API, qui n'impose pas l'encodeur de l'instance, ne \
             permettent de l'écrire. Un écart de top-k entre deux campagnes n'est donc pas \
             attribuable à l'encodeur par ce rapport seul (ADR-0155, décisions 3, 4 et 6).\n",
        );
    }
    out.push_str(
        "- La cible de fidélité est 0,99 de recall@10 au passage, **au N par défaut de 5000** et \
         en moyenne sur les requêtes dont la liste exacte n'est pas vide (ADR-0076, ADR-0078, \
         ADR-0125 complément C1). Une campagne à un autre N \
         éclaire ce réglage : elle ne se compare pas directement à la cible. Le recouvrement@10 \
         n'a pas de cible, il dépend du nombre de passages par document.\n\
         - Le dénominateur du recall@10 de fidélité est la taille du top-10 EXACT : il vaut 10 \
         dès que le pool filtré compte au moins dix passages, et le rapport écrit sa valeur par \
         requête (`fidelity_denominator`) quand elle diffère de 10. Une requête dont la liste \
         exacte est vide est exclue de la moyenne et comptée à part, avec son `id` (ADR-0125, \
         complément du 2026-11-01, point C1).\n\
         - La qualité ne porte que sur les requêtes qui portent des documents attendus : `n` la \
         borne, et un `n` faible rend chaque requête plus lourde que le seuil de régression de \
         0,005. Un attendu absent du jeu mesuré compte simplement comme non trouvé\n\
         (ADR-0125, décision 4).\n",
    );
    // The two bullets below are the ones of the local mode: in API mode the
    // latencies are not `debug.timings_ms`, and two bases that differ are not
    // compared at all (ADR-0131, decision 7).
    if preconditions.is_none() {
        out.push_str(
            "- Les latences sont **indicatives** : elles viennent de `debug.timings_ms`, ne \
             valent pas une mesure MES-xx et ne changent jamais le code de sortie. La passe \
             d'échauffement charge lecteurs, modèle et cache de requêtes (ADR-0051), donc \
             `encode` mesure le chemin d'un cache déjà chaud, pas le premier encodage.\n",
        );
    }
    out.push_str(
        "- La machine est décrite par son système, son architecture et ses cœurs : un rapport de \
         mesure MES-xx, lui, nomme le processeur et la RAM.\n\
         - Les chiffres portent sur cette génération et ce jeu : ils ne valent pas preuve à \
         8,58 M passages, où la fidélité se confirme en P9 (ADR-0076).\n",
    );
    if preconditions.is_none() {
        out.push_str(
            "- Une régression de `--baseline` est une baisse de plus de 0,005 en valeur absolue \
             d'une moyenne de qualité ou de fidélité ; le code de sortie vaut alors 1. Les \
             latences sont comparées et rapportées, jamais bloquantes.\n",
        );
    }
    out
}

/// The top-k section of the Markdown report (ADR-0155, decision 7).
///
/// The full `doc_ids` lists stay in the JSON: a campaign report has to read
/// without its context, and 97 × 10 identifiers do not. The section gives the
/// mean and its `n`, the `id`s whose ranking moved, the empty references
/// counted apart, and the attribution; it says when the comparison was not made
/// instead of disappearing, because an absent section would read as a zero.
fn top_k_markdown(out: &mut String, baseline: &Value) {
    out.push_str("### Recouvrement de top-k\n\n");
    if let Some(modes) = baseline.get("modes").and_then(Value::as_object) {
        for (mode, entry) in modes {
            if let Some(top_k) = entry.get("top_k") {
                let n = top_k.get("n").and_then(Value::as_u64).unwrap_or(0);
                match top_k.get("overlap_at_k").and_then(Value::as_f64) {
                    Some(overlap) => out.push_str(&format!(
                        "- `{mode}` : recouvrement de top-k {overlap:.4} (n = {n})\n"
                    )),
                    None => out.push_str(&format!(
                        "- `{mode}` : recouvrement de top-k sans échantillon (n = 0)\n"
                    )),
                }
                for (member, label) in [
                    ("changed", "classement modifié"),
                    ("excluded", "exclue(s) de la moyenne, référence vide"),
                    (
                        "not_compared",
                        "non comparée(s), absente(s) d'un des deux rapports",
                    ),
                ] {
                    let ids = quoted_ids(top_k.get(member));
                    if !ids.is_empty() {
                        out.push_str(&format!(
                            "- `{mode}` : {} requête(s) {label} — {}\n",
                            ids.len(),
                            ids.join(", ")
                        ));
                    }
                }
            } else if let Some(skipped) = entry.get("top_k_not_compared") {
                let version = skipped
                    .get("reference_format_version")
                    .and_then(Value::as_u64)
                    .map_or_else(String::new, |version| {
                        format!("(format_version {version}) ")
                    });
                out.push_str(&format!(
                    "- `{mode}` : recouvrement de top-k non comparé, la référence {version}ne \
                     porte pas de `doc_ids` (ADR-0155, décision 5)\n"
                ));
            }
        }
    }
    // A mode the reference does not carry has no entry in `modes`, so its
    // top-k gets its own line: decision 7 asks the section to say it rather
    // than to disappear with the mode.
    if let Some(skipped) = baseline.get("modes_not_compared").and_then(Value::as_array) {
        for mode in skipped.iter().filter_map(Value::as_str) {
            out.push_str(&format!(
                "- `{mode}` : recouvrement de top-k non comparé, mode absent de la référence \
                 (ADR-0155, décision 5)\n"
            ));
        }
    }
    out.push_str(
        "- La comparaison de top-k est **informative** : elle ne change pas le code de sortie, \
         qui reste celui des seules moyennes de qualité et de fidélité (ADR-0155, décision 5).\n",
    );
    if let Some(attribution) = baseline.get("attribution") {
        let generation = attribution.get("generation");
        let reference = generation
            .and_then(|generation| generation.get("reference"))
            .map_or_else(|| "?".to_owned(), Value::to_string);
        let current = generation
            .and_then(|generation| generation.get("current"))
            .map_or_else(|| "?".to_owned(), Value::to_string);
        let state = |member: &str| {
            attribution
                .get(member)
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned()
        };
        out.push_str(&format!(
            "- Attribution : génération {reference} → {current} ; empreinte du modèle « {} » ; \
             empreinte du jeu « {} » (ADR-0155, décision 6)\n",
            state("model_fingerprint"),
            state("queries_fingerprint"),
        ));
    }
    out.push('\n');
}

/// The `id`s of one member of a comparison, quoted, in report order.
fn quoted_ids(member: Option<&Value>) -> Vec<String> {
    member
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|id| format!("`{id}`"))
        .collect()
}

#[cfg(test)]
mod tests {
    use crible_core::error::ErrorCode;

    use super::*;

    /// One minimal request, with `data` and `url` as the mode tests need them.
    fn request<'a>(data: Option<&'a Path>, url: Option<&'a str>) -> EvalRequest<'a> {
        EvalRequest {
            data,
            url,
            index: "fixture",
            queries: Path::new("game.jsonl"),
            models: None,
            modes: &[],
            k: 10,
            out: Path::new("out"),
            exact_threshold: None,
            rescore_candidates: None,
            repeat: 1,
            baseline: None,
        }
    }

    /// The refusal of `where_of`, checked as `invalid_request`.
    fn refusal(request: &EvalRequest<'_>) -> String {
        let error = match where_of(request) {
            Ok(_) => panic!("the request must be refused"),
            Err(error) => error,
        };
        assert_eq!(
            error.code(),
            ErrorCode::InvalidRequest,
            "a refused option is a request fault: {error}"
        );
        error.message().to_owned()
    }

    #[test]
    fn api_mode_refuses_the_three_local_overrides_by_name() {
        // ADR-0131, decisions 6 and 9: the mode decides before any I/O, and
        // each override is named in its own refusal.
        let base = "http://127.0.0.1:8080";
        let mut models = request(None, Some(base));
        models.models = Some(Path::new("models"));
        let message = refusal(&models);
        assert!(
            message.contains("--models") && message.contains("API mode"),
            "the refusal names --models: {message}"
        );
        let mut threshold = request(None, Some(base));
        threshold.exact_threshold = Some(0);
        let message = refusal(&threshold);
        assert!(
            message.contains("--exact-threshold") && message.contains("precondition"),
            "the refusal names --exact-threshold and its reason: {message}"
        );
        let mut candidates = request(None, Some(base));
        candidates.rescore_candidates = Some(5_000);
        let message = refusal(&candidates);
        assert!(
            message.contains("--rescore-candidates") && message.contains("precondition"),
            "the refusal names --rescore-candidates and its reason: {message}"
        );
    }

    #[test]
    fn one_target_is_required_and_the_two_are_exclusive() {
        // ADR-0131, decision 6: `--data` and `--url` pick the mode, and the
        // command takes exactly one of them.
        let message = refusal(&request(Some(Path::new("data")), Some("http://host:1")));
        assert!(
            message.contains("--data")
                && message.contains("--url")
                && message.contains("exclusive"),
            "the refusal names both options: {message}"
        );
        let message = refusal(&request(None, None));
        assert!(
            message.contains("--data") && message.contains("--url"),
            "the refusal says one of the two is required: {message}"
        );
    }
}

/// The reference game of Legifacile (`CONSTITUER-JEU-DE-REFERENCE`, P10).
///
/// The six `<index>.jsonl` files of `docs/legifacile/reference/` are read by
/// [`read_queries`], the parser of `crible eval` itself (ADR-0125, decision 3),
/// so the game is accepted by the command that will run it. Every `filter` is
/// then parsed and validated against the schema of its index by
/// `Filter::from_json_str` and `Filter::validate`, the path of step 1 of the
/// pipeline (ADR-0029, ADR-0035). `mcp.jsonl` is checked as the other side of
/// the same game: one line per `id`, one `id` per line, the four members, the
/// eight perimeters and the parameter names of `recherche_semantique`.
#[cfg(test)]
mod reference_game {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};

    use crible_core::config::IndexConfig;
    use crible_core::filter::Filter;
    use serde_json::Value;

    use super::Query;
    use super::read_queries;

    /// The six indexes of `docs/legifacile/configs/`.
    const INDEXES: [&str; 6] = ["articles", "jorf", "decisions", "titres", "sarde", "dole"];

    /// The eight perimeters of the MCP (`serveur_mcp/perimetres.py`).
    const PERIMETERS: [&str; 8] = [
        "droit_applicable",
        "droit_en_vigueur",
        "droit_historique",
        "jurisprudence",
        "journal_officiel",
        "index_thematique",
        "dossiers_legislatifs",
        "titres",
    ];

    /// The filter parameter names `recherche_semantique` accepts
    /// (`serveur_mcp/outils.py`), `corpus` left out: `mcp.jsonl` describes the
    /// call by perimeter only.
    const FILTER_PARAMS: [&str; 8] = [
        "k",
        "dedupliquer",
        "partie_exclue",
        "fonds",
        "etat",
        "annee_min",
        "annee_max",
        "cle_domaine",
    ];

    /// Root of the repository, from this crate.
    fn docs() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("docs")
    }

    /// One game, read by the parser of the command.
    fn game(name: &str) -> Vec<Query> {
        let path = docs()
            .join("legifacile/reference")
            .join(format!("{name}.jsonl"));
        read_queries(&path).unwrap_or_else(|error| {
            panic!(
                "{} must be a game `crible eval` accepts: {error}",
                path.display()
            )
        })
    }

    /// The configuration of one index, read as `crible-store` reads it.
    fn config(name: &str) -> IndexConfig {
        let path = docs()
            .join("legifacile/configs")
            .join(format!("{name}.json"));
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
        IndexConfig::from_json_str(&raw).unwrap_or_else(|error| {
            panic!("index {name}: the configuration must be valid: {error}")
        })
    }

    #[test]
    fn every_reference_game_parses_and_every_filter_validates_against_its_index() {
        for name in INDEXES {
            let queries = game(name);
            assert!(
                !queries.is_empty(),
                "index {name}: the reference game must hold at least one query"
            );
            let config = config(name);
            for query in &queries {
                assert!(
                    query.text.is_some(),
                    "index {name} ({}): a reference query carries its text",
                    query.id
                );
                assert!(
                    !query.expected.is_empty(),
                    "index {name} ({}): a reference query names the documents it expects",
                    query.id
                );
                let Some(raw) = query.filter.as_deref() else {
                    continue;
                };
                let filter = Filter::from_json_str(raw).unwrap_or_else(|error| {
                    panic!(
                        "index {name} ({}): the filter is outside the grammar: {error}",
                        query.id
                    )
                });
                filter.validate(&config.schema).unwrap_or_else(|error| {
                    panic!(
                        "index {name} ({}): the filter does not validate against the schema: \
                         {error}",
                        query.id
                    )
                });
            }
        }
    }

    #[test]
    fn every_id_has_exactly_one_mcp_line_and_the_converse() {
        let mut ids: BTreeMap<String, &str> = BTreeMap::new();
        for name in INDEXES {
            for query in game(name) {
                if let Some(previous) = ids.insert(query.id.clone(), name) {
                    panic!(
                        "the id '{}' appears in {previous} and in {name}: the game numbers its \
                         queries once",
                        query.id
                    );
                }
            }
        }

        let path = docs().join("legifacile/reference/mcp.jsonl");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for (position, line) in text.lines().enumerate() {
            let number = position + 1;
            let value: Value = serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("mcp.jsonl line {number}: invalid JSON: {error}"));
            let object = value
                .as_object()
                .unwrap_or_else(|| panic!("mcp.jsonl line {number}: a line must be an object"));
            let members: BTreeSet<&str> = object.keys().map(String::as_str).collect();
            let expected: BTreeSet<&str> =
                ["id", "tool", "perimetre", "params"].into_iter().collect();
            assert_eq!(
                members, expected,
                "mcp.jsonl line {number}: the line is closed to these four members"
            );
            let id = object["id"]
                .as_str()
                .unwrap_or_else(|| panic!("mcp.jsonl line {number}: `id` must be a string"))
                .to_owned();
            assert_eq!(
                object["tool"].as_str(),
                Some("recherche_semantique"),
                "mcp.jsonl line {number}: the game compares the semantic branch"
            );
            let perimetre = object["perimetre"]
                .as_str()
                .unwrap_or_else(|| panic!("mcp.jsonl line {number}: `perimetre` must be a string"));
            assert!(
                PERIMETERS.contains(&perimetre),
                "mcp.jsonl line {number}: '{perimetre}' is not one of the eight perimeters"
            );
            let params = object["params"]
                .as_object()
                .unwrap_or_else(|| panic!("mcp.jsonl line {number}: `params` must be an object"));
            for key in params.keys() {
                assert!(
                    FILTER_PARAMS.contains(&key.as_str()),
                    "mcp.jsonl line {number}: '{key}' is not a parameter of `recherche_semantique`"
                );
            }
            assert!(
                ids.contains_key(&id),
                "mcp.jsonl line {number}: the id '{id}' is in no index game"
            );
            assert!(
                seen.insert(id.clone()),
                "mcp.jsonl line {number}: the id '{id}' has more than one line"
            );
        }
        assert_eq!(
            seen.len(),
            ids.len(),
            "mcp.jsonl: {} lines for {} ids of the six games",
            seen.len(),
            ids.len()
        );
    }
}
