// SPDX-License-Identifier: Apache-2.0

//! `POST /v1/indexes/{index}/search`: the search route of P6.
//!
//! References: `docs/phases/P6-serveur-et-api.md` (`ROUTE-RECHERCHE`),
//! `docs/05-api.md` ("Recherche : requête", "Recherche : réponse",
//! "Limites par défaut"), ADR-0109 (unknown fields, deadline and the proof
//! "API = CLI"), ADR-0102 (the token of `max_concurrent_searches`, the
//! deadline), ADR-0095 (the three roles reach this route) and ADR-0074,
//! ADR-0075, ADR-0089, ADR-0090 (mode, regrouping, lexical availability and
//! fusion).
//!
//! # What the route shares with the CLI
//!
//! The body is parsed here, because it names a JSON path on every fault; the
//! pipeline, the regroupment, the candidate count and the shape of a result come
//! from [`crible_search`] and [`crible_search::api`]. The route therefore serves
//! exactly the answer `crible search` prints (ADR-0109, decision 3).
//!
//! # Where the work runs
//!
//! The search is CPU-bound. It runs inside one blocking task that holds the
//! token of the instance and installs the dedicated rayon pool
//! ([`ScanPool`]): never on a tokio thread (`docs/02-architecture.md`,
//! "Concurrence"). The token is taken before any work and stays taken after a
//! `504`, until the computation really ends (ADR-0102, decision 3; ADR-0109,
//! decision 2).
//!
//! # What the body refuses
//!
//! A field outside the fourteen of `docs/05-api.md` is an `invalid_request` that
//! names its JSON path, at every level (`fusion`, `return`, `filter` keeps its
//! `invalid_filter` code). `explain_ids` is open since `EXPLIQUER-DOCUMENTS`
//! (P7, ADR-0123): the route parses it, and the explanation runs after
//! `took_ms`, inside the same blocking task, under the deadline. `debug` is
//! accepted since P7 (ADR-0119). No field is ignored in silence. A query text or
//! a filter value never reaches a log (AGENTS.md, "Confidentialité").

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::response::{IntoResponse, Response};
use crible_core::config::{FusionConfig, check_alpha};
use crible_core::error::{CribleError, ErrorCode};
use crible_core::filter::Filter;
use crible_core::model::ModelKind;
use crible_search::api::{self, ReturnSelection};
use crible_search::debug::DebugCapture;
use crible_search::dense::Strategy;
use crible_search::dense::exact::DenseSearcher;
use crible_search::explain::{self, ExplainEntry, ExplainId, MAX_IDS, Survivor};
use crible_search::group::{Grouping, candidates_per_branch};
use crible_search::pipeline::{
    self, Answer, Mode, RankedHit, Request as PipelineRequest, ScoreType, Stages,
};
use crible_search::pool::Pool;
use crible_store::generation::{epoch_records_path, epoch_segments_directory};
use serde_json::{Map, Value, json};

use crate::AppState;
use crate::analytics::{self, Operation, Record, SearchEvent, StageDurations};
use crate::auth::Client;
use crate::debug_journal::{self, Deposit, RequestView};
use crate::error;
use crate::http::RequestId;
use crate::jobs::Published;
use crate::limits;

/// Fields of the search body of `docs/05-api.md`.
pub(crate) const BODY_FIELDS: [&str; 14] = [
    "query",
    "vector",
    "mode",
    "filter",
    "k",
    "fusion",
    "group_by",
    "passages_per_document",
    "dedup",
    "return",
    "exact",
    "debug",
    "explain_ids",
    "timeout_ms",
];

/// Fields of the multi-index body of `docs/05-api.md`, "Recherche multi-index".
///
/// It is the set of the simple route with `targets` in place of `filter`
/// (ADR-0110, decision 1): every other field is common, at the first level.
pub(crate) const MULTI_BODY_FIELDS: [&str; 14] = [
    "query",
    "vector",
    "mode",
    "k",
    "fusion",
    "group_by",
    "passages_per_document",
    "dedup",
    "return",
    "exact",
    "debug",
    "explain_ids",
    "timeout_ms",
    "targets",
];

/// Default `k` of a search (`docs/05-api.md`).
pub(crate) const DEFAULT_K: usize = 10;

/// `POST /v1/indexes/{index}/search`: one search over one index.
///
/// The roles are checked by the `read` layer the route is mounted under: `write`
/// and `admin` include `read`, so the three reach it (ADR-0095). An unknown index
/// is `404 unknown_index`; the body is validated before any token is taken, and
/// `timeout_ms` before the work starts.
#[utoipa::path(
    post,
    path = "/v1/indexes/{index}/search",
    operation_id = "search_index",
    tag = "search",
    description = "Search one index, dense, lexical or hybrid, and fuse the two rankings. An \
                   unknown field of the body is `400 invalid_request`; an unknown index is `404 \
                   unknown_index`; an insufficient role `403`; a saturated concurrency limit \
                   `503` with `Retry-After: 1`; a deadline `504 timeout`. `debug` adds the plan, the timings and the branch ranks \
                   (ADR-0119); `explain_ids` adds the explanation of the requested ids (ADR-0123). Role: \
                   `read`; `write` and `admin` include it (ADR-0095).",
    params(("index" = String, Path, description = "Name of the index")),
    request_body = crate::openapi::SearchRequest,
    responses(
        (status = 200, description = "The fused answer", body = crate::openapi::SearchResponse),
        (status = 400, description = "Unknown field, value outside its bounds or inputs of the mode", body = crate::openapi::Error),
        (status = 401, description = "Missing, malformed or unknown API key", body = crate::openapi::Error),
        (status = 403, description = "Insufficient role", body = crate::openapi::Error),
        (status = 404, description = "Unknown index", body = crate::openapi::Error),
        (status = 500, description = "Unexpected failure", body = crate::openapi::Error),
        (status = 503, description = "Search limit reached, server draining or lexical branch unavailable", body = crate::openapi::Error),
        (status = 504, description = "`timeout_ms` reached", body = crate::openapi::Error)
    )
)]
pub async fn search(
    State(state): State<Arc<AppState>>,
    Extension(identifier): Extension<RequestId>,
    Extension(client): Extension<Client>,
    Path(index): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    // ADR-0122, decision 1: the draw happens at the entry of the route, before
    // the concurrency token, so it costs no token and never extends `took_ms`.
    let sampled = debug_journal::sampled(&state);
    let (answer, records, deposit) =
        search_inner(Arc::clone(&state), &identifier, index, body, sampled).await;
    analytics::record(&state, records, &client);
    if let Some(deposit) = deposit {
        debug_journal::deposit(&state, deposit, &client, &identifier.0);
    }
    answer
}

/// The work of [`search`], with the events it records (ADR-0117, decision 5)
/// and the row of the debug journal, when the request is recorded (ADR-0122).
///
/// The route records nothing itself: this function builds the record and the
/// row beside the response, so an early refusal and a successful answer take
/// the same path and no exit of the route can forget its event.
async fn search_inner(
    state: Arc<AppState>,
    identifier: &RequestId,
    index: String,
    body: Value,
    sampled: bool,
) -> (Response, Vec<Record>, Option<Deposit>) {
    let mut observed = Observed::new(index.clone(), Operation::Search);
    // ADR-0122, decision 5: a sampled request opens its row at the entry, so a
    // body refused before it is parsed is still recorded, with its code alone.
    if sampled {
        observed.open_journal(None);
    }
    let handle = match state.index(&index) {
        Ok(Some(handle)) => handle,
        Ok(None) => return observed.refused(&unknown_index(&index), &identifier.0),
        Err(failure) => return observed.refused(&failure, &identifier.0),
    };
    let request = match SearchRequest::parse(&body) {
        Ok(request) => request,
        Err(failure) => return observed.refused(&failure, &identifier.0),
    };
    let published = match handle.published() {
        Ok(published) => published,
        Err(failure) => return observed.refused(&failure, &identifier.0),
    };
    // `timeout_ms` is read and bounded before any other work, so a fault of the
    // deadline never hides behind a missing input (ADR-0102, decision 3).
    let default_timeout_ms = match state.default_timeout_ms() {
        Ok(value) => value,
        Err(failure) => return observed.refused(&failure, &identifier.0),
    };
    let deadline = match limits::timeout(request.timeout_ms, default_timeout_ms) {
        Ok(deadline) => deadline,
        Err(failure) => return observed.refused(&failure, &identifier.0),
    };
    let mode = request
        .mode
        .unwrap_or_else(|| Mode::default_for(published.config.lexical.enabled));
    if mode.uses_lexical() && !published.config.lexical.enabled {
        return observed.refused(
            &error::invalid("lexical branch is disabled for this index"),
            &identifier.0,
        );
    }
    observed.mode(mode);
    let inputs = match request.inputs(mode) {
        Ok(inputs) => inputs,
        Err(failure) => return observed.refused(&failure, &identifier.0),
    };
    // A permanent refusal of ADR-0109, decision 4, before the token: an
    // `external` model cannot encode, so a mode that would encode needs its
    // `vector` (ADR-0110, decision 7).
    if let Err(failure) = check_model_inputs(&state, &published, mode, &inputs) {
        return observed.refused(&failure, &identifier.0);
    }
    let fusion = request.fusion.unwrap_or(published.config.search.fusion);
    let grouping = match api::grouping(
        request.by_document,
        request.passages_per_document,
        request.dedup_content,
    ) {
        Ok(grouping) => grouping,
        Err(failure) => return observed.refused(&failure, &identifier.0),
    };
    let selection = match request.selection.resolve(&published.config.schema) {
        Ok(selection) => selection,
        Err(failure) => return observed.refused(&failure, &identifier.0),
    };
    let filter = match request.filter {
        Some(filter) => match filter.validate(&published.config.schema) {
            Ok(()) => filter,
            Err(failure) => return observed.refused(&failure, &identifier.0),
        },
        None => Filter::And(Vec::new()),
    };
    // ADR-0117, decision 6: the shape is computed from the validated filter, so
    // a refused filter produces no row.
    observed.shape(analytics::filter_shape(&filter));
    // ADR-0122, decision 1: a `debug` request is always recorded when the
    // journal is on, and the draw records the others. ADR-0122, decision 3: the
    // plan and the capture are built when the row is recorded, and the `debug`
    // member of the answer stays conditioned on `debug`. A journal that is off
    // records nothing, so the payload is not built (ADR-0122, decision 8).
    let recording = state.debug_journal().is_some() && (sampled || request.debug);
    if recording {
        let view = RequestView {
            body: &body,
            mode,
            k: request.k,
            exact: request.exact,
            fusion,
            by_document: request.by_document,
            passages_per_document: request.passages_per_document,
            dedup_content: request.dedup_content,
            with_filter: true,
            dimension: inputs.vector_dimension(),
        };
        observed.open_journal(Some(debug_journal::request_payload(&view)));
    }
    let limit = match state.search_limit() {
        Ok(limit) => limit,
        Err(failure) => return observed.refused(&failure, &identifier.0),
    };

    // The validation ends here, and its duration is the first sample of the
    // event (ADR-0117, decision 2).
    observed.validate_now();
    let holder = Arc::clone(&state);
    let name = index.clone();
    // ADR-0123: the ids move into the blocking task, which runs the explanation
    // after `fetch`, under the deadline. A request without `explain_ids` carries
    // an empty list and allocates nothing for it.
    let explained = request.explain;
    // `took_ms` starts at the acquisition of the concurrency token (ADR-0109,
    // decision 2). This instant is read just before the token, so the
    // acquisition and everything after it are covered.
    let started = Instant::now();
    // A test holds the route here to prove that `took_ms` starts before the
    // token, not at the entry of the blocking task (ADR-0109, decision 2).
    #[cfg(feature = "failpoints")]
    crate::barriers::wait_search_start();
    let attempt = limit
        .run_blocking(deadline, move || {
            // A test holds the work here to prove that a `504` keeps the token
            // until the computation really ends (ADR-0102, decision 3).
            #[cfg(feature = "failpoints")]
            crate::barriers::wait_search();
            run(
                &holder,
                &name,
                &published,
                mode,
                inputs,
                &filter,
                &grouping,
                fusion,
                &selection,
                request.k,
                request.exact,
                request.debug,
                recording,
                &explained,
                started,
            )
        })
        .await;

    let outcome = match attempt {
        Ok(attempt) => observed.absorb(attempt),
        Err(failure) => return observed.refused(&failure, &identifier.0),
    };
    match outcome {
        Ok(outcome) => {
            // ADR-0120, decision 1: the total is stopped at the end of `fetch`,
            // before the JSON rendering; it serves the `debug` member and the
            // analytics column alike (ADR-0119, decision 7). The fallback only
            // covers an attempt that closed no total.
            let total = observed.stages.total.unwrap_or_else(|| started.elapsed());
            let answer = (
                axum::http::StatusCode::OK,
                Json(outcome.render(&identifier.0, total, &observed.stages)),
            )
                .into_response();
            observed.answered(answer)
        }
        Err(failure) => observed.refused(&failure, &identifier.0),
    }
}

/// What one search of a route records, beside its answer (ADR-0117, decision 5).
///
/// The route fills it as the request advances, so a refusal records the steps
/// that really ran and no sample for the steps that did not (ADR-0117,
/// decision 2): a `400` of the body carries its validation and its total, never
/// an empty histogram of the dense branch.
#[derive(Debug, Clone)]
pub(crate) struct Observed {
    index: String,
    operation: Operation,
    entry: Instant,
    stages: StageDurations,
    mode: Option<Mode>,
    strategy: Option<Strategy>,
    pool: u32,
    zero_results: bool,
    truncated: bool,
    contributions: Contributions,
    shape: Option<String>,
    /// Row of the debug journal, when this request is recorded (ADR-0122).
    journal: Option<JournalDraft>,
}

/// The parts of the payload of one journal row, filled as the request advances
/// (ADR-0121, decision 4).
#[derive(Debug, Clone)]
pub(crate) struct JournalDraft {
    /// The `request` member, as received; absent when the body was not parsed.
    request: Option<Value>,
    /// The closed `plan` object, built by the same code as the `debug` member.
    plan: Option<Value>,
    /// The capture of the branch tops, rendered for the payload.
    capture: Option<Value>,
    /// `targets[]` of a multi-index request, in the order of the request.
    targets: Option<Vec<Value>>,
    /// Generation of the index; a multi-index row carries it per target.
    generation: Option<u64>,
}

impl JournalDraft {
    /// Renders the payload of a request that answered.
    fn success(&self, stages: &StageDurations) -> Value {
        let mut rendered = Map::new();
        if let Some(request) = &self.request {
            rendered.insert("request".to_owned(), request.clone());
        }
        if let Some(plan) = &self.plan {
            rendered.insert("plan".to_owned(), plan.clone());
        }
        rendered.insert(
            "timings_ms".to_owned(),
            debug_journal::timings_payload(stages),
        );
        if let Some(capture) = &self.capture {
            rendered.insert("capture".to_owned(), capture.clone());
        }
        if let Some(targets) = &self.targets {
            rendered.insert("targets".to_owned(), Value::Array(targets.clone()));
        }
        if let Some(generation) = self.generation {
            rendered.insert("generation".to_owned(), json!(generation));
        }
        Value::Object(rendered)
    }

    /// Renders the payload of a refused request: the code alone, never its
    /// message nor its details, and the steps that were really measured
    /// (ADR-0121, decision 4; ADR-0122, decision 5).
    fn failure(&self, code: &str, stages: &StageDurations) -> Value {
        let mut rendered = Map::new();
        if let Some(request) = &self.request {
            rendered.insert("request".to_owned(), request.clone());
        }
        if let Some(targets) = &self.targets {
            rendered.insert("targets".to_owned(), Value::Array(targets.clone()));
        }
        rendered.insert(
            "timings_ms".to_owned(),
            debug_journal::measured_timings_payload(stages),
        );
        rendered.insert("error".to_owned(), json!({ "code": code }));
        Value::Object(rendered)
    }
}

/// Results each branch of a `hybrid` search brought, alone or together
/// (ADR-0117, decision 5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Contributions {
    /// A result the dense branch alone found.
    pub(crate) dense_only: bool,
    /// A result the lexical branch alone found.
    pub(crate) lexical_only: bool,
    /// A result both branches found.
    pub(crate) both: bool,
}

impl Contributions {
    /// Reads the contributions of the returned results.
    ///
    /// The ranks each branch gave travel on every [`RankedHit`], so the three
    /// facts are read from the answer and not recomputed: a result both branches
    /// found carries two ranks, a result one branch alone found carries one.
    fn of(hits: &[RankedHit]) -> Self {
        Self {
            dense_only: hits
                .iter()
                .any(|hit| hit.dense.is_some() && hit.lexical.is_none()),
            lexical_only: hits
                .iter()
                .any(|hit| hit.dense.is_none() && hit.lexical.is_some()),
            both: hits
                .iter()
                .any(|hit| hit.dense.is_some() && hit.lexical.is_some()),
        }
    }
}

impl Observed {
    /// Starts recording a search of `index` at the entry of its route.
    pub(crate) fn new(index: impl Into<String>, operation: Operation) -> Self {
        Self::new_at(index, operation, Instant::now())
    }

    /// Starts recording a search that entered its route at `entry`.
    pub(crate) fn new_at(index: impl Into<String>, operation: Operation, entry: Instant) -> Self {
        Self {
            index: index.into(),
            operation,
            entry,
            stages: StageDurations::default(),
            mode: None,
            strategy: None,
            pool: 0,
            zero_results: false,
            truncated: false,
            contributions: Contributions::default(),
            shape: None,
            journal: None,
        }
    }

    /// Opens the journal row of this request with its `request` member, or
    /// without it when the body has not been parsed yet (ADR-0122, decision 5).
    pub(crate) fn open_journal(&mut self, request: Option<Value>) {
        self.journal = Some(JournalDraft {
            request,
            plan: None,
            capture: None,
            targets: None,
            generation: None,
        });
    }

    /// Records the mode the request resolved to.
    pub(crate) fn mode(&mut self, mode: Mode) {
        self.mode = Some(mode);
    }

    /// Records the canonical form of the filter (ADR-0117, decision 6).
    pub(crate) fn shape(&mut self, shape: Option<String>) {
        self.shape = shape;
    }

    /// Closes the validation step just before the concurrency token.
    pub(crate) fn validate_now(&mut self) {
        self.stages.validate = Some(self.entry.elapsed());
    }

    /// Folds the attempt of the blocking thread and returns its answer.
    pub(crate) fn absorb(&mut self, attempt: Attempt) -> Result<SearchOutcome, CribleError> {
        self.stages.encode = attempt.stages.encode;
        self.stages.filter = attempt.stages.filter;
        self.stages.dense = attempt.stages.dense;
        self.stages.lexical = attempt.stages.lexical;
        self.stages.fusion = attempt.stages.fusion;
        self.stages.group = attempt.stages.group;
        self.stages.fetch = attempt.stages.fetch;
        self.stages.total = attempt.total;
        self.strategy = attempt.strategy;
        self.pool = attempt.pool;
        self.contributions = attempt.contributions;
        if let Ok(outcome) = &attempt.result {
            self.zero_results = outcome.zero_results;
            self.truncated = outcome.truncated_query;
        }
        // ADR-0122, decision 3: the plan and the capture built for the journal
        // are folded into the row; the answer keeps its own `debug` member.
        if let Some(draft) = &mut self.journal {
            draft.plan.clone_from(&attempt.plan);
            draft.capture = attempt.capture.as_ref().map(debug_journal::capture_payload);
            if let Ok(outcome) = &attempt.result {
                draft.generation = Some(outcome.generation);
            }
        }
        attempt.result
    }

    /// Records the duration of the encoding, step 2 of `docs/03-recherche.md`.
    pub(crate) fn encoded(&mut self, duration: Duration) {
        self.stages.encode = Some(duration);
    }

    /// Folds the outcome of one target of a multi-index search.
    ///
    /// The encoding and the validation belong to the request, not to the target,
    /// and are recorded once on the first target: summing them per target would
    /// count one encoding several times (ADR-0117, decision 5).
    pub(crate) fn target(&mut self, outcome: &IndexOutcome, truncated: bool) {
        let mode = self.mode.unwrap_or(Mode::Dense);
        let measured = measure(mode, outcome);
        self.stages.filter = measured.filter;
        self.stages.dense = measured.dense;
        self.stages.lexical = measured.lexical;
        self.stages.fusion = measured.fusion;
        self.stages.group = measured.group;
        self.stages.fetch = measured.fetch;
        self.strategy = outcome.strategy;
        self.pool = outcome.pool_size;
        self.zero_results = outcome.hits.is_empty();
        self.truncated = truncated;
        self.contributions = contributions(mode, &outcome.hits);
    }

    /// Closes the total stage of one target of a multi-index request.
    ///
    /// Each target measures its own work: closing the total at the entry of the
    /// request would count the whole request once per target, so three targets
    /// would add three times its duration to `latency_total` (ADR-0117,
    /// decision 2).
    pub(crate) fn finish_at(&mut self, started: Instant) {
        self.stages.total = Some(started.elapsed());
    }

    /// Closes the total stage with a duration already measured.
    ///
    /// The caller reads the instant once and hands the same duration to the
    /// event and to the `debug` member (ADR-0119, decision 7).
    pub(crate) fn close_total(&mut self, total: Duration) {
        self.stages.total = Some(total);
    }

    /// Validation and encoding durations, measured once for the request.
    ///
    /// The multi-index route renders them in its global `timings_ms` and
    /// `0.0` in each target (ADR-0119, decision 9).
    pub(crate) fn request_stages(&self) -> (Option<Duration>, Option<Duration>) {
        (self.stages.validate, self.stages.encode)
    }

    /// Renders `failure` and yields its event and its journal row.
    pub(crate) fn refused(
        self,
        failure: &CribleError,
        request_id: &str,
    ) -> (Response, Vec<Record>, Option<Deposit>) {
        let code = failure.code().as_str().to_owned();
        let deposit = self.deposit(&code);
        let answer = error::rendered(failure, request_id);
        let status = analytics::status(&answer);
        let records = self.events(status);
        (answer, records, deposit)
    }

    /// Yields `answer`, its event and its journal row.
    pub(crate) fn answered(self, answer: Response) -> (Response, Vec<Record>, Option<Deposit>) {
        let status = analytics::status(&answer);
        let deposit = self.deposit(&status);
        let records = self.events(status);
        (answer, records, deposit)
    }

    /// Builds the row of the journal, if this request is recorded.
    ///
    /// The total is read **before** [`Self::events`] falls back to the entry of
    /// the route: a `503` taken at the token, a `400` of the body and a `504` of
    /// the deadline keep `NULL` (ADR-0121, decision 3).
    fn deposit(&self, status: &str) -> Option<Deposit> {
        let draft = self.journal.as_ref()?;
        let payload = if status == "ok" {
            draft.success(&self.stages)
        } else {
            draft.failure(status, &self.stages)
        };
        Some(Deposit {
            request_id: String::new(),
            index: match self.operation {
                Operation::Search => Some(self.index.clone()),
                _ => None,
            },
            client: String::new(),
            mode: self
                .mode
                .map_or_else(String::new, |mode| mode.as_str().to_owned()),
            status: status.to_owned(),
            total_ms: self.stages.total.map(|total| total.as_secs_f64() * 1_000.0),
            payload,
        })
    }

    /// Builds the one event of this search.
    pub(crate) fn events(mut self, status: String) -> Vec<Record> {
        // A successful search closes its total at the end of `fetch`
        // (ADR-0120, decision 1); a target of a multi-index request closes its
        // own. The fallback covers a refusal, which keeps the route duration.
        if self.stages.total.is_none() {
            self.stages.total = Some(self.entry.elapsed());
        }
        vec![Record::Search {
            event: SearchEvent {
                minute: String::new(),
                index: self.index,
                operation: self.operation,
                status,
                // The name of the key is filled by the route, which alone reads
                // the `Client` (ADR-0118, decision 6).
                client: String::new(),
                mode: self.mode,
                dense_strategy: self.strategy,
                stages: self.stages,
                pool: self.pool,
                zero_results: self.zero_results,
                truncated: self.truncated,
                dense_only: self.contributions.dense_only,
                lexical_only: self.contributions.lexical_only,
                both: self.contributions.both,
            },
            shape: self.shape,
        }]
    }
}

/// One attempt at a search on a blocking thread, successful or not.
///
/// The durations travel beside the result so a failure of the encoding, of the
/// pool or of a branch still records the steps that ran.
pub(crate) struct Attempt {
    /// Answer of the pipeline, or the failure the route renders.
    pub(crate) result: Result<SearchOutcome, CribleError>,
    /// Durations of the steps that ran on the blocking thread.
    pub(crate) stages: StageDurations,
    /// Duration from the token acquisition to the end of `fetch`, read once
    /// (ADR-0120, decision 1); `None` when the attempt stopped before it.
    pub(crate) total: Option<Duration>,
    /// Dense strategy that ran, when a dense branch ran.
    pub(crate) strategy: Option<Strategy>,
    /// Size of the pool the filter kept, `0` when the filter never ran.
    pub(crate) pool: u32,
    /// Results each branch of a `hybrid` search brought.
    pub(crate) contributions: Contributions,
    /// Capture of the branch tops, built for `debug` or for the journal
    /// (ADR-0122, decision 3).
    pub(crate) capture: Option<DebugCapture>,
    /// The closed `plan` object, built for `debug` or for the journal.
    pub(crate) plan: Option<Value>,
}

impl Attempt {
    /// An attempt that stopped at `failure`, with the steps that ran before it.
    fn failed(failure: CribleError, stages: StageDurations) -> Self {
        Self {
            result: Err(failure),
            stages,
            total: None,
            strategy: None,
            pool: 0,
            contributions: Contributions::default(),
            capture: None,
            plan: None,
        }
    }
}

/// Runs the pipeline on the blocking thread, with the token held.
///
/// `debug` asks for the `debug` member of ADR-0119 and `recording` for the row
/// of the journal (ADR-0122, decision 3). Either one builds the capture and the
/// plan; only `debug` renders them in the answer, so a sampled request never
/// gains a `debug` member.
#[allow(clippy::too_many_arguments)]
fn run(
    state: &AppState,
    index: &str,
    published: &Published,
    mode: Mode,
    inputs: Inputs,
    filter: &Filter,
    grouping: &Grouping,
    fusion: FusionConfig,
    selection: &ReturnSelection,
    k: usize,
    exact: bool,
    debug: bool,
    recording: bool,
    explain_targets: &[ExplainTarget],
    started: Instant,
) -> Attempt {
    let mut stages = StageDurations::default();
    // The encoding point of the server: the closure below runs at most once,
    // even though the pipeline has two branches (ADR-0109, ADR-0051).
    let encodes = inputs.encodes();
    let encoding = Instant::now();
    let prepared = match inputs.prepare(|query| encode_query(state, published, query)) {
        Ok(prepared) => prepared,
        Err(failure) => return Attempt::failed(failure, stages),
    };
    if encodes {
        stages.encode = Some(encoding.elapsed());
    }
    let vector = match &prepared.vector {
        Some(vector) => match check_vector(state, published, vector.clone()) {
            Ok(vector) => Some(vector),
            Err(failure) => return Attempt::failed(failure, stages),
        },
        None => None,
    };
    // ADR-0119, decision 8, and ADR-0122, decision 3: the capture exists for a
    // request that asks for `debug` or is sampled; without either, no vector is
    // allocated on the search path. One source for its depth: the default of
    // the engine (`docs/06-observabilite.md:82`).
    let collecting = debug || recording;
    let mut capture = collecting.then(|| DebugCapture::new(crible_search::debug::DEFAULT_TOP_N));
    let outcome = match search_index(
        state,
        index,
        published,
        IndexParams {
            mode,
            vector: vector.as_deref(),
            text: prepared.text.as_deref(),
            filter,
            grouping,
            fusion,
            k,
            exact,
            capture: capture.as_mut(),
            explain: !explain_targets.is_empty(),
        },
    ) {
        Ok(outcome) => outcome,
        Err(failure) => return Attempt::failed(failure, stages),
    };
    // ADR-0120, decision 1: the total stops at the end of `fetch`, before the
    // JSON rendering of the results. The deadline and the token still cover the
    // whole request.
    let total = started.elapsed();
    // ADR-0123, decision 4: the explanation runs after `fetch` and out of
    // `took_ms`, but under the deadline and the token, which stay taken until the
    // blocking task really ends.
    let explain = match outcome.explain.as_ref() {
        Some(context) if !explain_targets.is_empty() => {
            let ids: Vec<ExplainId> = explain_targets
                .iter()
                .map(|target| target.id.clone())
                .collect();
            match context.explain(
                &published.snapshot,
                vector.as_deref(),
                prepared.text.as_deref(),
                &published.config.search,
                &ids,
            ) {
                Ok(entries) => Some(entries),
                Err(failure) => {
                    return Attempt::failed(
                        failure,
                        StageDurations {
                            encode: stages.encode,
                            ..measure(mode, &outcome)
                        },
                    );
                }
            }
        }
        _ => None,
    };
    // Step 3 to 8 of `docs/03-recherche.md`, read from the outcome: a step the
    // mode did not run produces no sample (ADR-0117, decision 2).
    let measured = measure(mode, &outcome);
    let contributions = contributions(mode, &outcome.hits);
    let zero_results = outcome.hits.is_empty();
    // ADR-0121, decision 4: the `plan` member is built by the same code as the
    // `debug` member, then consumed by the answer or by the journal.
    let plan_value = collecting.then(|| {
        crate::debug::plan(
            mode,
            outcome.strategy,
            &published.config.search,
            outcome.pool_size,
            k,
            exact,
            grouping,
            fusion,
        )
    });
    let debug = match (&plan_value, debug) {
        (Some(plan), true) => Some(crate::debug::SearchDebug {
            plan: plan.clone(),
            per_result: outcome
                .hits
                .iter()
                .map(crate::debug::result_entry)
                .collect(),
        }),
        _ => None,
    };
    let results = api::results_json(
        &Answer {
            score_type: outcome.score_type,
            results: outcome.hits,
            stages: outcome.stages,
            strategy: outcome.strategy,
            pre_group_ranks: Vec::new(),
        },
        &published.config.schema,
        selection,
    );
    Attempt {
        result: Ok(SearchOutcome {
            results,
            debug,
            explain,
            pool_size: outcome.pool_size,
            generation: outcome.generation,
            truncated_query: prepared.truncated_query,
            zero_results,
        }),
        stages: StageDurations {
            encode: stages.encode,
            ..measured
        },
        total: Some(total),
        strategy: outcome.strategy,
        pool: outcome.pool_size,
        contributions,
        capture,
        plan: plan_value,
    }
}

/// Durations of steps 3 to 8 of `docs/03-recherche.md`, read from the pipeline.
///
/// Step 2, `encode`, and step 1, `validate`, belong to the caller and are added
/// beside these (ADR-0117, decision 2).
pub(crate) fn measure(mode: Mode, outcome: &IndexOutcome) -> StageDurations {
    StageDurations {
        filter: Some(outcome.filter),
        dense: mode.uses_dense().then_some(outcome.stages.dense),
        lexical: mode.uses_lexical().then_some(outcome.stages.lexical),
        fusion: (mode == Mode::Hybrid).then_some(outcome.stages.fusion),
        group: Some(outcome.stages.group),
        fetch: Some(outcome.stages.fetch),
        ..StageDurations::default()
    }
}

/// Contributions of the two branches, read from the returned results.
///
/// Only `hybrid` fills them: `dense` and `lexical` leave the three counters at
/// zero (ADR-0117, decision 5).
fn contributions(mode: Mode, hits: &[RankedHit]) -> Contributions {
    if mode == Mode::Hybrid {
        Contributions::of(hits)
    } else {
        Contributions::default()
    }
}

/// One search over one index, with its inputs already prepared.
///
/// Both search routes of P6 call this function: the simple route of
/// `ROUTE-RECHERCHE` and each target of `POST /v1/search` (ADR-0110,
/// decision 8). The caller owns the pool inputs, the token and the deadline; the
/// per-index pipeline — pool, branches, fusion, `dedup` then `group_by`, top-`k`
/// — is the one of `docs/03-recherche.md`.
pub(crate) struct IndexParams<'a> {
    /// Branches to run.
    pub(crate) mode: Mode,
    /// Query vector, checked against the model by the caller.
    pub(crate) vector: Option<&'a [f32]>,
    /// Query text for the lexical branch.
    pub(crate) text: Option<&'a str>,
    /// Filter of this index, validated against its schema.
    pub(crate) filter: &'a Filter,
    /// Regrouping options.
    pub(crate) grouping: &'a Grouping,
    /// Fusion of the two branches in `hybrid`.
    pub(crate) fusion: FusionConfig,
    /// Results, or documents under `group_by`.
    pub(crate) k: usize,
    /// `exact: true` forces the exact dense calculation.
    pub(crate) exact: bool,
    /// Capture of the branch tops, built only for a `debug` request
    /// (ADR-0119, decision 8).
    pub(crate) capture: Option<&'a mut DebugCapture>,
    /// Whether the request asks for `explain_ids` (ADR-0123). It makes the
    /// pipeline keep its ranking before the regrouping, and the outcome carry
    /// what the explanation reads after `fetch`.
    pub(crate) explain: bool,
}

/// What the explanation of one index reads after `took_ms`, kept beside the
/// answer by [`search_index`] (ADR-0123, decision 4).
///
/// It holds what the pipeline already computed — the pool, the searcher, the
/// ranking before the regrouping, the survivors — so the explanation runs after
/// `fetch` without reopening a reader or running a branch again.
pub(crate) struct ExplainContext {
    pub(crate) pool: Pool,
    pub(crate) searcher: DenseSearcher,
    pub(crate) filter: Filter,
    pub(crate) mode: Mode,
    pub(crate) requested: usize,
    pub(crate) exact: bool,
    pub(crate) strategy: Option<Strategy>,
    pub(crate) pre_group_ranks: Vec<(String, u32)>,
    /// Results that survived the regrouping, without their content digest: the
    /// digest reads happen after `took_ms`, in [`ExplainContext::explain`].
    pub(crate) survivors: Vec<Survivor>,
    /// Whether the request deduplicates on content, so the digests are needed.
    pub(crate) dedup: bool,
}

impl ExplainContext {
    /// Explains `ids` against this context (ADR-0123, decisions 1 and 4).
    pub(crate) fn explain(
        &self,
        snapshot: &crible_store::generation::Snapshot,
        vector: Option<&[f32]>,
        text: Option<&str>,
        search: &crible_core::config::SearchConfig,
        ids: &[ExplainId],
    ) -> Result<Vec<ExplainEntry>, CribleError> {
        // ADR-0123, complement of 2026-11-01: the digest reads of `dedup` are
        // part of the explanation, so they happen here, after the total.
        let mut survivors = self.survivors.clone();
        if self.dedup {
            explain::fill_hashes(&self.searcher, &mut survivors)?;
        }
        explain::explain(
            snapshot,
            &explain::ExplainInput {
                pool: &self.pool,
                searcher: &self.searcher,
                filter: &self.filter,
                mode: self.mode,
                vector,
                text,
                search,
                requested: self.requested,
                exact: self.exact,
                strategy: self.strategy,
                pre_group_ranks: &self.pre_group_ranks,
                survivors: &survivors,
            },
            ids,
        )
    }
}

/// Answer of one index, before the envelope of a search route.
pub(crate) struct IndexOutcome {
    /// Results, best first, regrouping applied.
    pub(crate) hits: Vec<RankedHit>,
    /// Type of the score every result carries.
    pub(crate) score_type: ScoreType,
    /// Size of the pool this index evaluated.
    pub(crate) pool_size: u32,
    /// Generation of the snapshot the search read.
    pub(crate) generation: u64,
    /// Durations the pipeline measured, steps 4 to 8 of `docs/03-recherche.md`.
    pub(crate) stages: Stages,
    /// Dense strategy that actually ran, `None` without a dense branch.
    pub(crate) strategy: Option<Strategy>,
    /// Duration of the pool, step 3, measured here and not by the pipeline.
    pub(crate) filter: Duration,
    /// What the explanation reads, when the request asks for one.
    pub(crate) explain: Option<ExplainContext>,
}

/// Runs the pipeline of one index on the blocking thread.
pub(crate) fn search_index(
    state: &AppState,
    index: &str,
    published: &Published,
    params: IndexParams<'_>,
) -> Result<IndexOutcome, CribleError> {
    let loaded = state
        .loaded()
        .ok_or_else(|| error::internal("the instance has not loaded its indexes"))?;
    let snapshot = Arc::clone(&published.snapshot);
    // Step 3 of `docs/03-recherche.md`: the pool is computed once per index and
    // shared by the two branches (INV-05); its duration is a sample of its own.
    let filtering = Instant::now();
    let pool = Pool::evaluate(snapshot.filters(), params.filter)?;
    let filter = filtering.elapsed();
    let index_dir = loaded.index_directory(index);
    let segments_directory = epoch_segments_directory(&index_dir, snapshot.epoch());
    let records_path = epoch_records_path(&index_dir, snapshot.epoch());
    let segments = snapshot.segments().to_vec();
    let scan = loaded.scan_pool();
    // ADR-0120, decision 2: the opening of the segment and payload readers
    // stays in `took_ms` and belongs to the `fetch` key, in every mode, until
    // the per-generation cache replaces it.
    let opening = Instant::now();
    let searcher =
        scan.install(|| DenseSearcher::open(&segments_directory, &segments, &records_path))?;
    let open = opening.elapsed();
    let answer = scan.install(|| {
        pipeline::search_captured(
            &snapshot,
            &searcher,
            &pool,
            &PipelineRequest {
                mode: params.mode,
                vector: params.vector,
                text: params.text,
                k: params.k,
                exact: params.exact,
                grouping: params.grouping,
                fusion: params.fusion,
            },
            &published.config.search,
            params.capture,
            params.explain,
        )
    })?;
    let mut stages = answer.stages;
    stages.fetch += open;
    // ADR-0123, decision 4, and its complement of 2026-11-01: the only work of
    // the explanation inside `took_ms` is the capture of the ranks before the
    // regrouping, which the pipeline does. The context keeps the results by
    // their key and document; the digest reads of `dedup` happen after the
    // total, in `ExplainContext::explain`.
    let pool_size = pool.len();
    let explanation = if params.explain {
        let survivors = explain::survivors(&searcher, &answer.results, false)?;
        Some(ExplainContext {
            pool,
            searcher,
            filter: params.filter.clone(),
            mode: params.mode,
            requested: candidates_per_branch(params.k, params.grouping),
            exact: params.exact,
            strategy: answer.strategy,
            pre_group_ranks: answer.pre_group_ranks,
            survivors,
            dedup: params.grouping.dedup_content,
        })
    } else {
        None
    };
    Ok(IndexOutcome {
        hits: answer.results,
        score_type: answer.score_type,
        pool_size,
        generation: snapshot.generation().get(),
        stages,
        strategy: answer.strategy,
        filter,
        explain: explanation,
    })
}

/// Refuses a dense mode that would have to encode on an `external` model.
///
/// ADR-0109, decision 4: `dense` on an `external` index requires `vector`, and
/// `hybrid` requires `query` **and** `vector`. The message names the model, and
/// the refusal comes before the concurrency token (ADR-0110, decision 7), so a
/// saturated instance answers `400` rather than `503`. Only the configuration
/// of the model is read here, never its graph: an `onnx` load would block the
/// async runtime, outside the blocking task of the search (ADR-0107).
pub(crate) fn check_model_inputs(
    state: &AppState,
    published: &Published,
    mode: Mode,
    inputs: &Inputs,
) -> Result<(), CribleError> {
    if !mode.uses_dense() || inputs.has_vector() {
        return Ok(());
    }
    // A `failpoints` test may stand a double at the encoding point; it answers
    // the encoding this check is about, so the refusal does not apply
    // (ADR-0110, decision 11).
    #[cfg(feature = "failpoints")]
    if crate::test_encoding::armed() {
        return Ok(());
    }
    let config = state.model_config(&published.config.model)?;
    if config.kind != ModelKind::External {
        return Ok(());
    }
    Err(error::invalid(format!(
        "index '{}' uses model '{}' of kind external, which has no encoder; mode {} needs a \
         'vector' in the request",
        published.config.name,
        config.name,
        mode.as_str()
    )))
}

/// Checks a provided vector against the dimensions of the model of the index.
pub(crate) fn check_vector(
    state: &AppState,
    published: &Published,
    vector: Vec<f32>,
) -> Result<Vec<f32>, CribleError> {
    let access = state.model(&published.config.model)?;
    check_dimensions(&vector, access.config.dims)?;
    Ok(vector)
}

/// Encodes the text query with the model of the index, through the instance
/// cache.
///
/// This is the single encoding point of the server. Under the `failpoints`
/// feature a test may stand a double here, which counts the real calls and
/// returns a fixed vector without ONNX Runtime (ADR-0110, decision 11).
pub(crate) fn encode_query(
    state: &AppState,
    published: &Published,
    query: &str,
) -> Result<(Vec<f32>, bool), CribleError> {
    let access = state.model(&published.config.model)?;
    #[cfg(feature = "failpoints")]
    if let Some(encoded) = crate::test_encoding::encode(access.config.dims) {
        return encoded;
    }
    if access.config.kind != ModelKind::Onnx {
        return Err(error::invalid(format!(
            "index '{}' uses model '{}' of kind external, which has no encoder; pass a vector \
             in the request",
            published.config.name, access.config.name
        )));
    }
    let precision = state
        .loaded()
        .ok_or_else(|| error::internal("the instance has not loaded its indexes"))?
        .query_precision();
    let encoder = state.query_encoder(&access)?;
    let encoded = encoder.encode_query(query, precision)?;
    Ok((encoded.vector, encoded.truncated))
}

/// One answer of the search, before the envelope of `docs/05-api.md`.
pub(crate) struct SearchOutcome {
    results: Vec<Value>,
    /// Plan and ranks of a `debug` request; absent without `debug`.
    debug: Option<crate::debug::SearchDebug>,
    /// Entries of `explain_ids`, in the order of the request; absent without
    /// `explain_ids` (ADR-0123, decision 1).
    explain: Option<Vec<ExplainEntry>>,
    pool_size: u32,
    generation: u64,
    truncated_query: bool,
    /// Whether the answer carries no result at all (ADR-0117, decision 5).
    zero_results: bool,
}

impl SearchOutcome {
    /// Renders the body of the `200`, with the request identifier of ADR-0098.
    ///
    /// `total` is the duration `took_ms` reports, read once by the route; the
    /// `debug` member renders it as its `total` too (ADR-0119, decision 7).
    fn render(self, request_id: &str, total: Duration, stages: &StageDurations) -> Value {
        let mut body = json!({
            "results": self.results,
            "pool_size": self.pool_size,
            "generation": self.generation,
            "took_ms": total.as_secs_f64() * 1_000.0,
            "truncated_query": self.truncated_query,
            "request_id": request_id,
        });
        if let Some(debug) = self.debug {
            body["debug"] = debug.render(stages);
        }
        // ADR-0123, decision 1: `explain` is a sibling of `results`, present only
        // when `explain_ids` was present and non-empty, never in an error body.
        if let Some(explain) = self.explain {
            body["explain"] = Value::Array(render_explain(&explain, None));
        }
        body
    }
}

/// Renders one entry of `explain`, omitting every member without a value
/// (ADR-0119, division 3; ADR-0123, decision 1).
///
/// `index` is set on the multi-index route only (ADR-0123, decision 2).
pub(crate) fn render_explain_entry(entry: &ExplainEntry, index: Option<&str>) -> Value {
    let mut object = Map::new();
    if let Some(index) = index {
        object.insert("index".to_owned(), json!(index));
    }
    object.insert("id".to_owned(), json!(entry.id));
    object.insert("doc_id".to_owned(), json!(entry.doc_id));
    object.insert("exists".to_owned(), json!(entry.exists));
    if let Some(alive) = entry.alive {
        object.insert("alive".to_owned(), json!(alive));
    }
    object.insert("in_pool".to_owned(), json!(entry.in_pool));
    if !entry.excluded_by.is_empty() {
        object.insert("excluded_by".to_owned(), json!(entry.excluded_by));
    }
    if let Some(dense) = &entry.dense {
        let mut branch = Map::new();
        branch.insert("key".to_owned(), json!(dense.key));
        branch.insert("exact_rank".to_owned(), json!(dense.exact_rank));
        if let Some(rank) = dense.binary_rank {
            branch.insert("binary_rank".to_owned(), json!(rank));
        }
        if let Some(kept) = dense.in_binary_candidates {
            branch.insert("in_binary_candidates".to_owned(), json!(kept));
        }
        object.insert("dense".to_owned(), Value::Object(branch));
    }
    if let Some(lexical) = &entry.lexical {
        let mut branch = Map::new();
        branch.insert("key".to_owned(), json!(lexical.key));
        branch.insert("rank".to_owned(), json!(lexical.rank));
        branch.insert("score".to_owned(), json!(lexical.score));
        branch.insert("terms".to_owned(), json!(render_terms(&lexical.terms)));
        object.insert("lexical".to_owned(), Value::Object(branch));
    }
    if let Some(fused) = entry.fused {
        object.insert("fused".to_owned(), json!({ "rank": fused.rank }));
    }
    if let Some(under) = &entry.grouped_under {
        object.insert("grouped_under".to_owned(), json!(under));
    }
    Value::Object(object)
}

/// Renders the per-term contributions of the lexical branch, a closed list of
/// `{ term, tf, score }` (ADR-0123, decision 6).
fn render_terms(terms: &[crible_search::lexical::TermScore]) -> Vec<Value> {
    terms
        .iter()
        .map(|term| json!({ "term": term.term, "tf": term.tf, "score": term.score }))
        .collect()
}

/// Renders a whole `explain` list, in the order of the request.
fn render_explain(entries: &[ExplainEntry], index: Option<&str>) -> Vec<Value> {
    entries
        .iter()
        .map(|entry| render_explain_entry(entry, index))
        .collect()
}

/// Inputs the mode runs on, already parsed from the body.
pub(crate) enum Inputs {
    /// Lexical branch alone, on the query text.
    Lexical { query: String },
    /// Dense branch on a provided vector.
    Dense { vector: Vec<f32> },
    /// Dense branch on the encoded query text.
    DenseText { query: String },
    /// Both branches; the vector replaces the encoded text when provided.
    Hybrid {
        query: String,
        vector: Option<Vec<f32>>,
    },
}

impl Inputs {
    /// Whether the inputs bring the vector the dense branch runs on.
    fn has_vector(&self) -> bool {
        matches!(
            self,
            Self::Dense { .. }
                | Self::Hybrid {
                    vector: Some(_),
                    ..
                }
        )
    }

    /// Whether preparing the inputs calls the encoding point at all.
    /// It is false for a lexical search and for a vector the client provided:
    /// step 2 of `docs/03-recherche.md` did not run, so it produces no sample
    /// (ADR-0117, decision 2). It is read **before** [`Inputs::prepare`]
    /// consumes the inputs.
    pub(crate) fn encodes(&self) -> bool {
        matches!(
            self,
            Self::DenseText { .. } | Self::Hybrid { vector: None, .. }
        )
    }

    /// Dimension of the vector the request carries, when it carries one.
    ///
    /// The journal records the presence and the dimension of the vector, never
    /// its bytes (ADR-0121, decision 4). It is read **before**
    /// [`Inputs::prepare`] consumes the inputs.
    pub(crate) fn vector_dimension(&self) -> Option<usize> {
        match self {
            Self::Dense { vector } => Some(vector.len()),
            Self::Hybrid { vector, .. } => vector.as_ref().map(Vec::len),
            Self::Lexical { .. } | Self::DenseText { .. } => None,
        }
    }

    /// Prepares the inputs of a request, calling `encode` **at most once**.
    ///
    /// `encode` is the encoding point of the server: the real one of
    /// [`encode_query`], or a test double that counts its calls (ADR-0110,
    /// decision 11). A multi-index request prepares its inputs once and serves
    /// the vector to every target (ADR-0110, decision 8), so the closure runs
    /// once whatever the number of targets.
    pub(crate) fn prepare<F>(self, encode: F) -> Result<Prepared, CribleError>
    where
        F: FnOnce(&str) -> Result<(Vec<f32>, bool), CribleError>,
    {
        match self {
            Self::Lexical { query } => Ok(Prepared {
                vector: None,
                text: Some(query),
                truncated_query: false,
            }),
            Self::Dense { vector } => Ok(Prepared {
                vector: Some(vector),
                text: None,
                truncated_query: false,
            }),
            Self::DenseText { query } => {
                let (vector, truncated_query) = encode(&query)?;
                Ok(Prepared {
                    vector: Some(vector),
                    text: None,
                    truncated_query,
                })
            }
            Self::Hybrid { query, vector } => match vector {
                Some(vector) => Ok(Prepared {
                    vector: Some(vector),
                    text: Some(query),
                    truncated_query: false,
                }),
                None => {
                    let (vector, truncated_query) = encode(&query)?;
                    Ok(Prepared {
                        vector: Some(vector),
                        text: Some(query),
                        truncated_query,
                    })
                }
            },
        }
    }
}

/// Inputs of a request after decoding and at most one encoding.
pub(crate) struct Prepared {
    /// Query vector, when a dense branch runs.
    pub(crate) vector: Option<Vec<f32>>,
    /// Query text, when a lexical branch runs.
    pub(crate) text: Option<String>,
    /// Whether the encoded query lost its tail (`truncated_query`).
    pub(crate) truncated_query: bool,
}

/// Body of a search, parsed and checked against the closed list of fields.
///
/// The same type serves `POST /v1/indexes/{index}/search` and the common part of
/// `POST /v1/search`: ADR-0110, decision 1 makes the multi-index body an
/// extension of the simple one, so the two routes share the parser and every
/// bound. Only `filter` and `targets` are exclusive to one of them.
#[derive(Debug)]
pub(crate) struct SearchRequest {
    pub(crate) query: Option<String>,
    pub(crate) vector: Option<String>,
    pub(crate) mode: Option<Mode>,
    filter: Option<Filter>,
    /// Ids of `explain_ids`, deduplicated in the order of the request.
    pub(crate) explain: Vec<ExplainTarget>,
    pub(crate) k: usize,
    pub(crate) fusion: Option<FusionConfig>,
    pub(crate) by_document: bool,
    pub(crate) passages_per_document: usize,
    pub(crate) dedup_content: bool,
    pub(crate) selection: ReturnSpec,
    pub(crate) exact: bool,
    pub(crate) debug: bool,
    pub(crate) timeout_ms: Option<u64>,
}

/// One id of `explain_ids`, with the index it names in the multi-index form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExplainTarget {
    /// Target of the entry; absent on the simple route (ADR-0123, decision 2).
    pub(crate) index: Option<String>,
    /// The id itself, `doc_id` or passage key.
    pub(crate) id: ExplainId,
    /// Identifier as requested, rendered back in `explain[].id`.
    pub(crate) raw: String,
}

impl SearchRequest {
    /// Checks that every entry of the multi-index form names a target
    /// (ADR-0123, decision 2), before the existence of the indexes is read.
    pub(crate) fn check_explain_targets(
        &self,
        targets: &BTreeSet<String>,
    ) -> Result<(), CribleError> {
        for (position, target) in self.explain.iter().enumerate() {
            let Some(index) = &target.index else {
                continue;
            };
            if !targets.contains(index) {
                return Err(error::invalid(format!(
                    "'$.explain_ids[{position}].index' names an index which is not one of the targets"
                )));
            }
        }
        Ok(())
    }
}

/// Parses `explain_ids`, in the simple form or in the multi-index form.
///
/// Duplicates are merged keeping the first occurrence, and the bound of
/// [`MAX_IDS`] applies after that merge (ADR-0123, decision 3). An empty id, a
/// malformed key or a wrong shape is an `invalid_request` that names the JSON
/// path of the fault and never the value of the id.
fn parse_explain(
    object: &Map<String, Value>,
    multi: bool,
) -> Result<Vec<ExplainTarget>, CribleError> {
    let Some(value) = object.get("explain_ids") else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let items = value
        .as_array()
        .ok_or_else(|| error::invalid("'$.explain_ids' must be an array of ids"))?;
    let mut seen: BTreeSet<(Option<String>, String)> = BTreeSet::new();
    let mut parsed = Vec::with_capacity(items.len());
    for (position, item) in items.iter().enumerate() {
        let path = format!("$.explain_ids[{position}]");
        let (index, raw) = if multi {
            let entry = json_object(item, &path)?;
            reject_unknown(entry, &path, &["index", "id"])?;
            let index = optional_string(entry, &path, "index")?
                .ok_or_else(|| error::invalid(format!("'{path}.index' is required")))?;
            let raw = optional_string(entry, &path, "id")?
                .ok_or_else(|| error::invalid(format!("'{path}.id' is required")))?;
            (Some(index), raw)
        } else {
            let Value::String(raw) = item else {
                return Err(error::invalid(format!(
                    "'{path}' must be a string, a doc_id or a passage key"
                )));
            };
            (None, raw.clone())
        };
        let id = ExplainId::parse_at(&raw, &path)?;
        if seen.insert((index.clone(), raw.clone())) {
            parsed.push(ExplainTarget { index, id, raw });
        }
    }
    if parsed.len() > MAX_IDS {
        return Err(error::invalid(format!(
            "'$.explain_ids' asks for {} ids, above the bound of {MAX_IDS}",
            parsed.len()
        )));
    }
    Ok(parsed)
}

/// `return` of the request.
#[derive(Debug)]
pub(crate) enum ReturnSpec {
    /// Every field the schema declares `returned`.
    All { text: bool },
    /// The named fields.
    Fields { names: Vec<String>, text: bool },
}

impl ReturnSpec {
    /// The requested names, or `None` for `"all"`.
    pub(crate) fn names(&self) -> Option<&[String]> {
        match self {
            Self::All { .. } => None,
            Self::Fields { names, .. } => Some(names),
        }
    }

    /// Whether the results carry their text.
    pub(crate) fn text(&self) -> bool {
        match self {
            Self::All { text } | Self::Fields { text, .. } => *text,
        }
    }

    /// Builds the selection once the names have been checked against the
    /// schemas of the targets (ADR-0110, decision 6).
    ///
    /// The selection still keeps only the fields each schema declares
    /// `returned`, through [`api::result_json`], so one selection serves
    /// heterogeneous schemas without a second check.
    pub(crate) fn selection(&self) -> ReturnSelection {
        let names = self
            .names()
            .map(|names| names.iter().cloned().collect::<BTreeSet<_>>());
        ReturnSelection::new(names, self.text())
    }

    /// Builds the selection, refusing a name the schema does not declare.
    fn resolve(
        &self,
        schema: &crible_core::config::Schema,
    ) -> Result<ReturnSelection, CribleError> {
        match self {
            Self::All { text } => Ok(ReturnSelection::new(None, *text)),
            Self::Fields { names, text } => {
                let mut selected = BTreeSet::new();
                for (position, name) in names.iter().enumerate() {
                    let Some((field, _level)) = schema.field(name) else {
                        return Err(error::invalid(format!(
                            "the field '{}' of '$.return.fields[{position}]' is not declared by \
                             the schema of the index",
                            name
                        )));
                    };
                    if !field.returned {
                        // ADR-0109, decision 5: a declared but never rendered
                        // field is refused, never accepted for nothing.
                        return Err(error::invalid(format!(
                            "the field '{}' of '$.return.fields[{position}]' is declared \
                             returned: false by the schema of the index",
                            name
                        )));
                    }
                    selected.insert(name.clone());
                }
                Ok(ReturnSelection::new(Some(selected), *text))
            }
        }
    }
}

impl SearchRequest {
    /// Parses the body of the simple route, naming the JSON path of every fault.
    pub(crate) fn parse(body: &Value) -> Result<Self, CribleError> {
        Self::parse_with(body, &BODY_FIELDS)
    }

    /// Parses a search body whose closed list of fields is `allowed`.
    ///
    /// The multi-index route passes [`MULTI_BODY_FIELDS`]: `filter` is then not
    /// a member of the first level, and its presence is an unknown field named
    /// `$.filter` (ADR-0110, decisions 1 and 2). The rest of the body keeps the
    /// semantics and the bounds of the simple route.
    pub(crate) fn parse_with(body: &Value, allowed: &[&str]) -> Result<Self, CribleError> {
        let object = json_object(body, "$")?;
        reject_unknown(object, "$", allowed)?;
        // `explain_ids` opened with EXPLIQUER-DOCUMENTS (ADR-0123). Its form is
        // checked with the body, before any index is read (ADR-0110, decision
        // 2): the multi-index route names each entry `{ "index", "id" }`.
        let explain = parse_explain(object, allowed.contains(&"targets"))?;
        let query = optional_string(object, "$", "query")?;
        let vector = optional_string(object, "$", "vector")?;
        let mode = match optional_string(object, "$", "mode")? {
            Some(name) => Some(parse_mode(&name)?),
            None => None,
        };
        let filter = if allowed.contains(&"filter") {
            match object.get("filter") {
                None => None,
                Some(Value::Null) => None,
                Some(value) => {
                    let _ = json_object(value, "$.filter")?;
                    // The path of the filter in the body, so `details.path` and
                    // the message both name `$.filter.…` (docs/05, ADR-0109).
                    Some(Filter::from_json_str_at(&value.to_string(), "$.filter")?)
                }
            }
        } else {
            None
        };
        let k = optional_unsigned(object, "$", "k")?.map_or(DEFAULT_K, as_usize);
        api::check_k(k)?;
        let fusion = match object.get("fusion") {
            None | Some(Value::Null) => None,
            Some(value) => Some(parse_fusion(value)?),
        };
        let by_document = api::group_by(optional_string(object, "$", "group_by")?.as_deref())?;
        let passages_per_document =
            optional_unsigned(object, "$", "passages_per_document")?.map_or(1, as_usize);
        let dedup_content = api::dedup(optional_string(object, "$", "dedup")?.as_deref())?;
        let selection = match object.get("return") {
            None | Some(Value::Null) => ReturnSpec::All { text: true },
            Some(value) => parse_return(value)?,
        };
        let exact = optional_bool(object, "$", "exact")?.unwrap_or(false);
        let debug = optional_bool(object, "$", "debug")?.unwrap_or(false);
        let timeout_ms = optional_unsigned(object, "$", "timeout_ms")?;
        Ok(Self {
            query,
            vector,
            mode,
            filter,
            explain,
            k,
            fusion,
            by_document,
            passages_per_document,
            dedup_content,
            selection,
            exact,
            debug,
            timeout_ms,
        })
    }

    /// Checks which inputs `mode` needs and decodes the vector.
    pub(crate) fn inputs(&self, mode: Mode) -> Result<Inputs, CribleError> {
        match mode {
            Mode::Dense => match (&self.query, &self.vector) {
                (Some(_), Some(_)) => Err(error::invalid(
                    "'query' and 'vector' are mutually exclusive in mode dense; pass one of them",
                )),
                (None, None) => Err(error::invalid(
                    "a search needs 'query' or 'vector'; mode dense takes exactly one",
                )),
                (Some(query), None) => Ok(Inputs::DenseText {
                    query: query.clone(),
                }),
                (None, Some(vector)) => Ok(Inputs::Dense {
                    vector: decode_vector(vector)?,
                }),
            },
            Mode::Lexical => {
                if self.vector.is_some() {
                    return Err(error::invalid(
                        "'vector' cannot be used in mode lexical, which scores the query text",
                    ));
                }
                let query = self.query.as_ref().ok_or_else(|| {
                    error::invalid("mode lexical needs 'query', the text it scores")
                })?;
                Ok(Inputs::Lexical {
                    query: query.clone(),
                })
            }
            Mode::Hybrid => {
                let query = self.query.as_ref().ok_or_else(|| {
                    error::invalid("mode hybrid needs 'query' for the lexical branch")
                })?;
                let vector = match &self.vector {
                    Some(vector) => Some(decode_vector(vector)?),
                    None => None,
                };
                Ok(Inputs::Hybrid {
                    query: query.clone(),
                    vector,
                })
            }
        }
    }
}

/// Parses `mode` (`docs/05-api.md`).
fn parse_mode(name: &str) -> Result<Mode, CribleError> {
    match name {
        "dense" => Ok(Mode::Dense),
        "lexical" => Ok(Mode::Lexical),
        "hybrid" => Ok(Mode::Hybrid),
        other => Err(error::invalid(format!(
            "'$.mode' must be dense, lexical or hybrid, got {other:?}"
        ))),
    }
}

/// Parses `fusion` (`docs/05-api.md`, ADR-0090).
fn parse_fusion(value: &Value) -> Result<FusionConfig, CribleError> {
    let object = json_object(value, "$.fusion")?;
    reject_unknown(object, "$.fusion", &["method", "alpha"])?;
    let method = object
        .get("method")
        .ok_or_else(|| error::invalid("'$.fusion.method' is required"))?;
    let method = method
        .as_str()
        .ok_or_else(|| error::invalid("'$.fusion.method' must be a string"))?;
    match method {
        "rrf" => {
            if object.contains_key("alpha") {
                return Err(error::invalid(
                    "'$.fusion.alpha' is not allowed with rrf, as in the fusion of index.json",
                ));
            }
            Ok(FusionConfig::Rrf)
        }
        "relative_score" => {
            let alpha = object
                .get("alpha")
                .ok_or_else(|| error::invalid("'$.fusion.alpha' is required with relative_score"))?
                .as_f64()
                .ok_or_else(|| error::invalid("'$.fusion.alpha' must be a number"))?;
            check_alpha(alpha, "$.fusion.alpha")?;
            Ok(FusionConfig::RelativeScore { alpha })
        }
        other => Err(error::invalid(format!(
            "'$.fusion.method' must be rrf or relative_score, got {other:?}"
        ))),
    }
}

/// Parses `return` (`docs/05-api.md`).
fn parse_return(value: &Value) -> Result<ReturnSpec, CribleError> {
    let object = json_object(value, "$.return")?;
    reject_unknown(object, "$.return", &["fields", "text"])?;
    let text = match object.get("text") {
        None => true,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| error::invalid("'$.return.text' must be a boolean"))?,
    };
    match object.get("fields") {
        None => Ok(ReturnSpec::All { text }),
        Some(Value::String(all)) if all == "all" => Ok(ReturnSpec::All { text }),
        Some(Value::String(other)) => Err(error::invalid(format!(
            "'$.return.fields' must be \"all\" or an array, got {other:?}"
        ))),
        Some(Value::Array(items)) => {
            let mut names = Vec::with_capacity(items.len());
            for (position, item) in items.iter().enumerate() {
                let name = item.as_str().ok_or_else(|| {
                    error::invalid(format!(
                        "'$.return.fields[{position}]' must be a field name"
                    ))
                })?;
                names.push(name.to_owned());
            }
            Ok(ReturnSpec::Fields { names, text })
        }
        Some(_) => Err(error::invalid(
            "'$.return.fields' must be \"all\" or an array of field names",
        )),
    }
}

/// Decodes the base64 `vector` of a request and checks its length against the
/// model of the index.
fn decode_vector(text: &str) -> Result<Vec<f32>, CribleError> {
    let bytes = crible_store::online::decode_base64(text)
        .map_err(|cause| error::invalid(format!("'$.vector' {cause}")))?;
    if !bytes.len().is_multiple_of(4) {
        return Err(error::invalid(format!(
            "'$.vector' holds {} bytes, not a whole number of fp32 components",
            bytes.len()
        )));
    }
    let vector: Vec<f32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect();
    Ok(vector)
}

/// Checks the length of a decoded vector against the dimensions of the model.
fn check_dimensions(vector: &[f32], dims: u32) -> Result<(), CribleError> {
    let expected = as_usize(u64::from(dims));
    if vector.len() != expected {
        return Err(error::invalid(format!(
            "'$.vector' holds {} components where the model of the index declares {expected}",
            vector.len()
        )));
    }
    Ok(())
}

/// A JSON object at `path`, or `invalid_request`.
pub(crate) fn json_object<'a>(
    value: &'a Value,
    path: &str,
) -> Result<&'a Map<String, Value>, CribleError> {
    value
        .as_object()
        .ok_or_else(|| error::invalid(format!("expected a JSON object at {path}")))
}

/// Refuses a key outside `allowed`, naming its JSON path (ADR-0109, decision 1).
pub(crate) fn reject_unknown(
    object: &Map<String, Value>,
    path: &str,
    allowed: &[&str],
) -> Result<(), CribleError> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(error::invalid(format!(
                "unknown field '{path}.{key}' in the search request"
            )));
        }
    }
    Ok(())
}

/// An optional string member.
pub(crate) fn optional_string(
    object: &Map<String, Value>,
    path: &str,
    name: &str,
) -> Result<Option<String>, CribleError> {
    match object.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(error::invalid(format!(
            "the field '{path}.{name}' must be a string"
        ))),
    }
}

/// An optional non-negative integer member.
fn optional_unsigned(
    object: &Map<String, Value>,
    path: &str,
    name: &str,
) -> Result<Option<u64>, CribleError> {
    match object.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| {
            error::invalid(format!(
                "the field '{path}.{name}' must be a non-negative integer"
            ))
        }),
    }
}

/// An optional boolean member.
fn optional_bool(
    object: &Map<String, Value>,
    path: &str,
    name: &str,
) -> Result<Option<bool>, CribleError> {
    match object.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| error::invalid(format!("the field '{path}.{name}' must be a boolean"))),
    }
}

/// Converts a JSON unsigned value to `usize`, saturating above the platform
/// maximum so the range checks of the caller speak last.
fn as_usize(value: u64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// The `404 unknown_index` of a name the instance does not serve.
///
/// Both search routes refuse an unknown index with this error, so the two
/// cannot spell it differently (ADR-0110, decision 5).
pub(crate) fn unknown_index(index: &str) -> CribleError {
    CribleError::new(ErrorCode::UnknownIndex, format!("unknown index '{index}'"))
}

#[cfg(test)]
mod tests {
    use super::{BODY_FIELDS, MAX_IDS, SearchRequest};
    use crible_search::pipeline::Mode;
    use serde_json::json;

    fn parse(body: serde_json::Value) -> crible_core::error::CribleError {
        SearchRequest::parse(&body).expect_err("the body is refused")
    }

    #[test]
    fn an_unknown_field_names_its_json_path_at_every_level() {
        let failure = parse(json!({ "filtre": {} }));
        assert!(
            failure.message().contains("$.filtre"),
            "{}",
            failure.message()
        );
        let failure = parse(json!({ "return": { "field": "all" } }));
        assert!(
            failure.message().contains("$.return.field"),
            "{}",
            failure.message()
        );
        let failure = parse(json!({ "fusion": { "method": "rrf", "alpah": 0.3 } }));
        assert!(
            failure.message().contains("$.fusion.alpah"),
            "{}",
            failure.message()
        );
    }

    #[test]
    fn explain_ids_is_parsed_with_its_bounds() {
        // ADR-0123, decisions 1 to 3: an absent or null list explains nothing,
        // the entries keep the order of the request, duplicates are merged, a
        // malformed id and a list above 100 entries are refused.
        let absent = SearchRequest::parse(&json!({})).expect("the body parses");
        assert!(absent.explain.is_empty());
        let null = SearchRequest::parse(&json!({ "explain_ids": null })).expect("null is absent");
        assert!(null.explain.is_empty());
        let parsed = SearchRequest::parse(&json!({
            "explain_ids": ["A", "A#2", "A", "B"]
        }))
        .expect("the list parses");
        assert_eq!(parsed.explain.len(), 3, "the duplicate is merged");
        assert_eq!(parsed.explain[0].raw, "A");
        assert_eq!(parsed.explain[1].raw, "A#2");
        assert_eq!(parsed.explain[2].raw, "B");
        assert!(parsed.explain[0].index.is_none());

        let not_a_list = parse(json!({ "explain_ids": "A" }));
        assert!(
            not_a_list.message().contains("$.explain_ids"),
            "{}",
            not_a_list.message()
        );
        for body in [
            json!({ "explain_ids": [""] }),
            json!({ "explain_ids": ["#3"] }),
            json!({ "explain_ids": [1] }),
        ] {
            let failure = parse(body);
            assert!(
                failure.message().contains("$.explain_ids[0]"),
                "{}",
                failure.message()
            );
        }
        // ADR-0123, complement of 2026-11-01: `A#x` is a `doc_id`, not a key.
        let hash_doc =
            SearchRequest::parse(&json!({ "explain_ids": ["A#x"] })).expect("A#x is a doc_id");
        assert_eq!(hash_doc.explain[0].raw, "A#x");
        let too_many: Vec<String> = (0..=MAX_IDS).map(|index| format!("D{index}")).collect();
        let failure = parse(json!({ "explain_ids": too_many }));
        assert!(
            failure.message().contains("$.explain_ids"),
            "{}",
            failure.message()
        );
    }

    #[test]
    fn debug_is_parsed_and_refused_outside_a_boolean() {
        // ADR-0119, decision 10: `debug` is accepted since P7 and is independent
        // of `explain_ids`.
        let asked = SearchRequest::parse(&json!({ "debug": true })).expect("debug is accepted");
        assert!(asked.debug);
        let omitted = SearchRequest::parse(&json!({})).expect("the body parses");
        assert!(!omitted.debug);
        let refused =
            SearchRequest::parse(&json!({ "debug": 1 })).expect_err("a number is refused");
        assert!(refused.message().contains("debug"), "{}", refused.message());
    }

    #[test]
    fn the_body_field_list_is_the_one_of_docs_05() {
        assert_eq!(BODY_FIELDS.len(), 14);
        for name in ["query", "vector", "timeout_ms", "debug", "explain_ids"] {
            assert!(BODY_FIELDS.contains(&name), "{name}");
        }
    }

    #[test]
    fn a_dense_request_needs_exactly_one_input() {
        let none = SearchRequest::parse(&json!({ "mode": "dense" })).expect("the body parses");
        assert!(none.inputs(Mode::Dense).is_err());
        let both = SearchRequest::parse(&json!({ "mode": "dense", "query": "x", "vector": "" }))
            .expect("the body parses");
        assert!(both.inputs(Mode::Dense).is_err());
    }

    #[test]
    fn a_timeout_outside_the_documented_type_is_refused_by_the_body() {
        for body in [
            json!({ "timeout_ms": -1 }),
            json!({ "timeout_ms": "10" }),
            json!({ "timeout_ms": 1.5 }),
        ] {
            let failure = parse(body);
            assert!(
                failure.message().contains("timeout_ms"),
                "{}",
                failure.message()
            );
        }
    }
}
