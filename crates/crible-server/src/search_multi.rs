// SPDX-License-Identifier: Apache-2.0

//! `POST /v1/search`: one search over several indexes (ADR-0110).
//!
//! References: `docs/phases/P6-serveur-et-api.md` (`RECHERCHE-MULTI-INDEX`),
//! `docs/05-api.md` ("Recherche multi-index"), `docs/03-recherche.md`
//! ("Recherche multi-index"), ADR-0110 (body, unknown fields, common model,
//! default mode, targets, `return.fields`, order of the validations, execution,
//! fusion between indexes, answer and the proof of the single encoding),
//! ADR-0109 (inputs according to the mode, deadline) and ADR-0102 (one token,
//! `Retry-After: 1`).
//!
//! # What the route shares with the simple route
//!
//! The body is the one of `POST /v1/indexes/{index}/search`, parsed by
//! [`SearchRequest::parse_with`]: the same fields, the same bounds and the same
//! words for a fault. Only `filter` becomes a member of a target and `targets`
//! appears (ADR-0110, decision 1).
//!
//! The request is encoded **once** by [`Inputs::prepare`], at the single
//! encoding point of the server ([`encode_query`](crate::search::encode_query)),
//! and the vector serves every target. Each target then runs the pipeline of the
//! simple route through [`search_index`](crate::search::search_index): pool,
//! branches, fusion, `dedup` then `group_by`, top-`k`.
//!
//! # Order of the validations
//!
//! ADR-0110, decision 7 fixes the first fault rendered: `400` (body, unknown
//! fields, bounds, inputs of the mode, `targets`, duplicates), then `404` for
//! the existence of each index, then the `400` of each target (lexical branch
//! disabled, then its filter) and of `return.fields`, then `422` for the common
//! model. The concurrency token is taken only once all of them passed, so a
//! permanent refusal holds no capacity (ADR-0102, decision 2).
//!
//! # Fusion between indexes
//!
//! ADR-0110, decision 9: `dense` sorts the units by descending cosine, then by
//! ascending index name and key, and the answer equals the global sort of the
//! top-`k` of each index; `lexical` and `hybrid` score each unit by
//! `1 / (60 + rank)` on the rank of the final list of its index, and all the
//! entries of a regrouped document carry the score of its document. The final
//! truncation keeps `k` units: `k` results, or `k` distinct documents and at
//! most `k × passages_per_document` entries. No `dedup` and no regrouping ever
//! crosses an index boundary (ADR-0110, decision 8).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Extension, State};
use axum::response::{IntoResponse, Response};
use crible_core::config::FusionConfig;
use crible_core::error::{CribleError, ErrorCode};
use crible_core::filter::Filter;
use crible_search::api::{self, ReturnSelection};
use crible_search::debug::DebugCapture;
use crible_search::group::Grouping;
use crible_search::pipeline::{Mode, RankedHit, ScoreType};
use serde_json::{Map, Value, json};

use crate::AppState;
use crate::analytics::{self, Operation, Record, StageDurations};
use crate::auth::Client;
use crate::debug_journal::{self, Deposit, RequestView};
use crate::error;
use crate::http::RequestId;
use crate::jobs::Published;
use crate::limits;
use crate::search::{
    self, IndexOutcome, IndexParams, Inputs, MULTI_BODY_FIELDS, Observed, ReturnSpec,
    SearchRequest, json_object, optional_string, reject_unknown,
};

/// Constant of the reciprocal rank fusion, shared with the fusion of the two
/// branches of one index (`docs/03-recherche.md:101`, ADR-0110, decision 9).
const RRF_CONSTANT: f32 = 60.0;

/// `POST /v1/search`: one search over the indexes of `targets`.
///
/// The route is mounted under the `read` role: `write` and `admin` include it,
/// so the three roles reach it (ADR-0095).
#[utoipa::path(
    post,
    path = "/v1/search",
    operation_id = "search_multi",
    tag = "search",
    description = "Search several indexes in one request, with one encoding and one deadline, \
                   then fuse their rankings. `targets` is mandatory and holds no duplicate; an \
                   unknown target is `404 not_found`; a common model mismatch is `422 \
                   incompatible_model`; a saturated concurrency limit `503`; the deadline `504 \
                   timeout`. Role: `read`; `write` and `admin` include it (ADR-0095).",
    request_body = crate::openapi::SearchMultiRequest,
    responses(
        (status = 200, description = "The fused answer of every target", body = crate::openapi::SearchMultiResponse),
        (status = 400, description = "Unknown field, bounds, inputs of the mode, empty or duplicate targets, lexical branch disabled or filter against a schema", body = crate::openapi::Error),
        (status = 401, description = "Missing, malformed or unknown API key", body = crate::openapi::Error),
        (status = 403, description = "Insufficient role", body = crate::openapi::Error),
        (status = 404, description = "Unknown target index", body = crate::openapi::Error),
        (status = 422, description = "Targets with different model fingerprints", body = crate::openapi::Error),
        (status = 500, description = "Unexpected failure", body = crate::openapi::Error),
        (status = 503, description = "Search limit reached, server draining or lexical branch unavailable", body = crate::openapi::Error),
        (status = 504, description = "`timeout_ms` reached", body = crate::openapi::Error)
    )
)]
pub async fn search(
    State(state): State<Arc<AppState>>,
    Extension(identifier): Extension<RequestId>,
    Extension(client): Extension<Client>,
    Json(body): Json<Value>,
) -> Response {
    // ADR-0122, decision 1: the draw happens at the entry of the route, before
    // the concurrency token.
    let sampled = debug_journal::sampled(&state);
    let (answer, records, deposit) =
        search_inner(Arc::clone(&state), &identifier, body, sampled).await;
    analytics::record(&state, records, &client);
    if let Some(deposit) = deposit {
        debug_journal::deposit(&state, deposit, &client, &identifier.0);
    }
    answer
}

/// The work of [`search`], with the events it records (ADR-0117, decision 5)
/// and the row of the debug journal, when the request is recorded (ADR-0122).
///
/// A multi-index search produces **one event per target**, never a consolidated
/// row: the pool, the strategy and the contributions belong to one target, and a
/// global row could only average them (ADR-0117, decision 5). The journal, on the
/// other hand, writes **one row per request** (ADR-0122, decision 4).
async fn search_inner(
    state: Arc<AppState>,
    identifier: &RequestId,
    body: Value,
    sampled: bool,
) -> (Response, Vec<Record>, Option<Deposit>) {
    let entry = Instant::now();
    // A request refused before its targets are resolved carries the first name
    // the body declares, and an empty index when it declares none: the name of
    // a target is never invented (ADR-0118, decision 6).
    let claimed = first_target_name(&body);
    let plan =
        match MultiRequest::parse(&body).and_then(|request| validate(&state, request, sampled)) {
            Ok(plan) => plan,
            Err(failure) => {
                let answered = Observed::new_at(claimed, Operation::SearchMulti, entry)
                    .refused(&failure, &identifier.0);
                let deposit = sampled.then(|| failure_deposit(&failure, None, &identifier.0));
                return (answered.0, answered.1, deposit);
            }
        };
    // ADR-0122, decisions 1 and 3: a `debug` request is recorded when the
    // journal is on, and the draw records the others; the capture and the plan
    // are built only for a recorded request.
    let recording = plan.recording;
    let request = recording.then(|| request_payload_of(&body, &plan));
    let limit = match state.search_limit() {
        Ok(limit) => limit,
        Err(failure) => {
            let answered = Observed::new_at(claimed, Operation::SearchMulti, entry)
                .refused(&failure, &identifier.0);
            let deposit = recording.then(|| failure_deposit(&failure, request, &identifier.0));
            return (answered.0, answered.1, deposit);
        }
    };
    let index = plan
        .targets
        .first()
        .map_or_else(String::new, |target| target.index.clone());
    let mut first = Observed::new_at(index, Operation::SearchMulti, entry);
    first.mode(plan.mode);
    first.shape(
        plan.targets
            .first()
            .and_then(|t| analytics::filter_shape(&t.filter)),
    );
    first.validate_now();

    let holder = Arc::clone(&state);
    // `took_ms` starts at the acquisition of the concurrency token (ADR-0109,
    // decision 2). This instant is read just before the token, so the
    // acquisition and everything after it are covered.
    let started = Instant::now();
    let reported = first.clone();
    let mode = plan.mode;
    let journal_request = request.clone();
    let outcome = limit
        .run_blocking(plan.deadline, move || {
            // A test holds the work here to prove the single token of the whole
            // request (ADR-0110, decision 8; ADR-0102, decision 2).
            #[cfg(feature = "failpoints")]
            crate::barriers::wait_search_multi();
            run_multi(&holder, plan, first, started, journal_request)
        })
        .await;

    match outcome {
        Ok((Ok(outcome), observers, journal)) => {
            let took_ms = Some(outcome.took_ms);
            let payload = journal;
            let answer = (
                axum::http::StatusCode::OK,
                Json(outcome.render(&identifier.0)),
            )
                .into_response();
            let deposit =
                payload.map(|payload| multi_deposit(&identifier.0, mode, "ok", took_ms, payload));
            (answer, events_of(observers, "ok"), deposit)
        }
        Ok((Err(failure), observers, journal)) => {
            let answer = error::rendered(&failure, &identifier.0);
            let status = analytics::status(&answer);
            let payload = journal.or_else(|| failure_payload(&failure, request));
            let deposit =
                payload.map(|payload| multi_deposit(&identifier.0, mode, &status, None, payload));
            (answer, events_of(observers, &status), deposit)
        }
        // The deadline expired before the blocking thread answered: no target
        // reported anything, so the single event carries the first target the
        // plan resolved, without inventing a faulty one.
        Err(failure) => {
            let answered = reported.refused(&failure, &identifier.0);
            let deposit = recording.then(|| failure_deposit(&failure, request, &identifier.0));
            (answered.0, answered.1, deposit)
        }
    }
}

/// Builds the journal row of a multi-index answer (ADR-0122, decision 4).
///
/// `index` stays `NULL` for **every** request of this route, even when it names
/// one target: the column says which route wrote the row, not how many targets
/// it had.
fn multi_deposit(
    request_id: &str,
    mode: Mode,
    status: &str,
    total_ms: Option<f64>,
    payload: Value,
) -> Deposit {
    Deposit {
        request_id: request_id.to_owned(),
        index: None,
        client: String::new(),
        mode: mode.as_str().to_owned(),
        status: status.to_owned(),
        total_ms,
        payload,
    }
}

/// The payload of a refused request whose `request` member is already known.
fn failure_payload(failure: &CribleError, request: Option<Value>) -> Option<Value> {
    request.map(|request| {
        json!({
            "request": request,
            "error": { "code": failure.code().as_str() },
        })
    })
}

/// The row of a refusal recorded before the `request` member could be built.
fn failure_deposit(failure: &CribleError, request: Option<Value>, request_id: &str) -> Deposit {
    Deposit {
        request_id: request_id.to_owned(),
        index: None,
        client: String::new(),
        mode: String::new(),
        status: failure.code().as_str().to_owned(),
        total_ms: None,
        payload: failure_payload(failure, request)
            .unwrap_or_else(|| json!({ "error": { "code": failure.code().as_str() } })),
    }
}

/// Builds the `request` member of a multi-index journal row.
///
/// `filter` is absent: each target carries its own in `targets[]`
/// (ADR-0121, decision 4). The rest of the body is common to the request.
fn request_payload_of(body: &Value, plan: &Plan) -> Value {
    let fusion = plan.fusion.unwrap_or_else(|| {
        plan.targets.first().map_or(FusionConfig::Rrf, |target| {
            target.published.config.search.fusion
        })
    });
    let view = RequestView {
        body,
        mode: plan.mode,
        k: plan.k,
        exact: plan.exact,
        fusion,
        by_document: plan.grouping.by_document,
        passages_per_document: plan.grouping.passages_per_document,
        dedup_content: plan.grouping.dedup_content,
        with_filter: false,
        dimension: plan.dimension,
    };
    debug_journal::request_payload(&view)
}

/// One event per target that ran, the last one with `last_status` and the
/// earlier ones `ok` (ADR-0117, decision 5).
///
/// The callers hand the observers in the order of the request: on a failure, the
/// faulty target is the last of them, because `run_multi` truncates the list
/// there.
fn events_of(observers: Vec<Observed>, last_status: &str) -> Vec<Record> {
    let mut records = Vec::with_capacity(observers.len());
    let count = observers.len();
    for (position, observed) in observers.into_iter().enumerate() {
        let status = if position + 1 == count {
            last_status.to_owned()
        } else {
            "ok".to_owned()
        };
        records.extend(observed.events(status));
    }
    records
}

/// Name the body claims as its first target, without validating the body.
///
/// It is read only for the event of a request refused before its targets are
/// resolved; the route never uses it to serve anything.
fn first_target_name(body: &Value) -> String {
    body.get("targets")
        .and_then(Value::as_array)
        .and_then(|targets| targets.first())
        .and_then(|target| target.get("index"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// A multi-index request, validated far enough to take the token.
struct Plan {
    /// Targets in the order of the request.
    targets: Vec<ResolvedTarget>,
    /// Branches every target runs.
    mode: Mode,
    /// Inputs of the mode, decoded once.
    inputs: Inputs,
    /// Regrouping options, shared by every target.
    grouping: Grouping,
    /// `fusion` of the body; absent means the configured fusion of each index.
    fusion: Option<FusionConfig>,
    /// `return`, checked against every schema.
    selection: ReturnSelection,
    /// Results, or documents under `group_by`.
    k: usize,
    /// `exact: true` forces the exact dense calculation.
    exact: bool,
    /// `debug` asks for the `debug` member of ADR-0119.
    debug: bool,
    /// Deadline of the whole request (ADR-0102, decision 3).
    deadline: Duration,
    /// Whether the request is recorded by the debug journal (ADR-0122).
    recording: bool,
    /// Dimension of the vector the body carries, when it carries one, for the
    /// `request` member of the journal payload.
    dimension: Option<usize>,
}

/// One target of the request, with its published state and its filter.
struct ResolvedTarget {
    /// Name of the index, as the request names it.
    index: String,
    /// Triplet `(config, snapshot, summary)` of the index.
    published: Published,
    /// Filter of this target; the empty filter matches every living passage.
    filter: Filter,
    /// Filter of this target as it was received, for the journal payload.
    filter_json: Value,
}

/// Body of the multi-index request: the common fields and the list of targets.
struct MultiRequest {
    /// Every field of the simple body but `filter` (ADR-0110, decision 1).
    common: SearchRequest,
    /// Targets, already free of duplicates.
    targets: Vec<TargetRequest>,
}

/// One element of `targets`.
struct TargetRequest {
    /// Name of the index.
    index: String,
    /// `filter` of the target, structurally parsed with its full JSON path.
    filter: Option<Filter>,
    /// `filter` of the target as received, `{}` when absent.
    filter_json: Value,
}

impl MultiRequest {
    /// Parses the body, naming the JSON path of every fault.
    fn parse(body: &Value) -> Result<Self, CribleError> {
        let common = SearchRequest::parse_with(body, &MULTI_BODY_FIELDS)?;
        let object = json_object(body, "$")?;
        let targets = parse_targets(object)?;
        Ok(Self { common, targets })
    }
}

/// Parses `targets`, refusing an absent list, an empty list and a duplicate
/// index (ADR-0110, decision 5).
fn parse_targets(object: &Map<String, Value>) -> Result<Vec<TargetRequest>, CribleError> {
    let Some(value) = object.get("targets") else {
        return Err(error::invalid(
            "'$.targets' is required and must name at least one index",
        ));
    };
    let items = value
        .as_array()
        .ok_or_else(|| error::invalid("'$.targets' must be an array of targets"))?;
    if items.is_empty() {
        return Err(error::invalid("'$.targets' must name at least one index"));
    }
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut targets = Vec::with_capacity(items.len());
    for (position, item) in items.iter().enumerate() {
        let path = format!("$.targets[{position}]");
        let target = json_object(item, &path)?;
        reject_unknown(target, &path, &["index", "filter"])?;
        let index = optional_string(target, &path, "index")?
            .ok_or_else(|| error::invalid(format!("'{path}.index' is required")))?;
        if !seen.insert(index.clone()) {
            // The same index twice would add two ranks in the RRF of
            // ADR-0110, decision 9.
            return Err(error::invalid(format!(
                "the index '{index}' appears more than once in '$.targets'; '{path}.index' \
                 repeats an earlier target"
            )));
        }
        let filter = match target.get("filter") {
            None | Some(Value::Null) => None,
            Some(value) => {
                let filter_path = format!("{path}.filter");
                let _ = json_object(value, &filter_path)?;
                Some(Filter::from_json_str_at(&value.to_string(), &filter_path)?)
            }
        };
        let filter_json = match target.get("filter") {
            Some(value) if !value.is_null() => value.clone(),
            _ => json!({}),
        };
        targets.push(TargetRequest {
            index,
            filter,
            filter_json,
        });
    }
    Ok(targets)
}

/// Applies the order of ADR-0110, decision 7, and builds the plan.
fn validate(state: &AppState, request: MultiRequest, sampled: bool) -> Result<Plan, CribleError> {
    let common = request.common;
    // (a) bounds of the body, and the entries of an explicit mode, before any
    // index is read. `targets` was already checked by the parser.
    let deadline = limits::timeout(common.timeout_ms, state.default_timeout_ms()?)?;
    let grouping = api::grouping(
        common.by_document,
        common.passages_per_document,
        common.dedup_content,
    )?;
    let named_inputs = match common.mode {
        Some(mode) => Some(common.inputs(mode)?),
        None => None,
    };

    // (b) existence of each index, in the order of `targets`: no partial
    // result, a single `404` naming the first unknown index.
    let mut published_targets = Vec::with_capacity(request.targets.len());
    for target in request.targets {
        match state.index(&target.index)? {
            Some(handle) => published_targets.push((target, handle.published()?)),
            None => return Err(search::unknown_index(&target.index)),
        }
    }

    // The default mode needs every target (ADR-0110, decision 4): `hybrid` only
    // when all of them enable the lexical branch, so it can never produce a
    // `400` by itself. Without an explicit mode, the entries are checked here;
    // they need that mode to be known.
    let mode = common.mode.unwrap_or_else(|| {
        if published_targets
            .iter()
            .all(|(_, published)| published.config.lexical.enabled)
        {
            Mode::Hybrid
        } else {
            Mode::Dense
        }
    });
    let inputs = match named_inputs {
        Some(inputs) => inputs,
        None => common.inputs(mode)?,
    };

    // (b') an `external` model cannot encode: a mode that would encode needs the
    // `vector` (ADR-0109, decision 4). It is a permanent `400` of the inputs,
    // rendered before the token (ADR-0110, decision 7), so a saturated instance
    // answers `400` rather than `503`. It needs the published configuration of
    // each target, so it follows the existence step `(b)` — the `404` of an
    // unknown index still comes first — and it still precedes the token.
    for (_, published) in &published_targets {
        search::check_model_inputs(state, published, mode, &inputs)?;
    }

    // (c) per target, in the order of `targets`: the lexical branch, then the
    // filter against the schema of the index.
    let mut targets = Vec::with_capacity(published_targets.len());
    for (target, published) in published_targets {
        if mode.uses_lexical() && !published.config.lexical.enabled {
            return Err(error::invalid(format!(
                "lexical branch is disabled for this index: '{}'",
                target.index
            )));
        }
        let filter = match target.filter {
            None => Filter::And(Vec::new()),
            Some(filter) => {
                filter.validate(&published.config.schema)?;
                filter
            }
        };
        targets.push(ResolvedTarget {
            index: target.index,
            published,
            filter,
            filter_json: target.filter_json,
        });
    }
    // (c) `return.fields` against the whole set of targets, once their schemas
    // are known: `"all"` needs no check.
    if common.selection.names().is_some() {
        check_return_names(&common.selection, &targets)?;
    }
    let selection = common.selection.selection();
    let dimension = inputs.vector_dimension();

    // (d) common model of a mode that compares vectors (ADR-0110, decision 3).
    if mode.uses_dense() {
        check_common_model(state, &targets)?;
    }

    Ok(Plan {
        targets,
        mode,
        inputs,
        grouping,
        fusion: common.fusion,
        selection,
        k: common.k,
        exact: common.exact,
        debug: common.debug,
        deadline,
        // ADR-0122, decision 1: a `debug` request is always recorded when the
        // journal is on; the draw records the others.
        recording: state.debug_journal().is_some() && (sampled || common.debug),
        dimension,
    })
}

/// Checks `return.fields` against every schema (ADR-0110, decision 6).
///
/// A name is accepted when at least one target declares it; a name that one
/// target declares `returned: false` is refused, and the message names the
/// field, its path, the index and the declaration.
fn check_return_names(spec: &ReturnSpec, targets: &[ResolvedTarget]) -> Result<(), CribleError> {
    let names = spec.names().unwrap_or_default();
    for (position, name) in names.iter().enumerate() {
        let mut declared = false;
        for target in targets {
            let Some((field, _level)) = target.published.config.schema.field(name) else {
                continue;
            };
            declared = true;
            if !field.returned {
                return Err(error::invalid(format!(
                    "the field '{name}' of '$.return.fields[{position}]' is declared \
                     returned: false by the index '{}'",
                    target.index
                )));
            }
        }
        if !declared {
            return Err(error::invalid(format!(
                "the field '{name}' of '$.return.fields[{position}]' is not declared by any \
                 target"
            )));
        }
    }
    Ok(())
}

/// Checks that every target shares the fingerprint of the first one
/// (ADR-0110, decision 3).
///
/// The message names the first target and the first one that differs, so a
/// client knows which two indexes it mixed.
fn check_common_model(state: &AppState, targets: &[ResolvedTarget]) -> Result<(), CribleError> {
    let first = targets
        .first()
        .ok_or_else(|| error::invalid("'$.targets' must name at least one index"))?;
    let fingerprint = state
        .model_config(&first.published.config.model)?
        .fingerprint();
    for target in &targets[1..] {
        let config = state.model_config(&target.published.config.model)?;
        if config.fingerprint() != fingerprint {
            return Err(CribleError::new(
                ErrorCode::IncompatibleModel,
                format!(
                    "the indexes '{}' and '{}' have different model fingerprints; one search \
                     cannot compare their vectors",
                    first.index, target.index
                ),
            ));
        }
    }
    Ok(())
}

/// Runs every target, one after the other, then fuses their answers.
///
/// It returns the events of the targets **with** the answer, successful or not:
/// one event per target that ran, never a consolidated row (ADR-0117,
/// decision 5). The first target carries the validation and the single encoding
/// of the request, which happen once and must not be counted again on the
/// others. Each target closes its own total, so a request of three targets does
/// not add its whole duration three times (decision 2).
///
/// A failure after the targets were resolved keeps the observers of the targets
/// that ran, the last one being the faulty target: the caller gives it the code
/// of the answer and the earlier ones `ok`.
fn run_multi(
    state: &AppState,
    plan: Plan,
    first: Observed,
    started: Instant,
    journal_request: Option<Value>,
) -> (
    Result<MultiOutcome, CribleError>,
    Vec<Observed>,
    Option<Value>,
) {
    let Plan {
        targets,
        mode,
        inputs,
        grouping,
        fusion,
        selection,
        k,
        exact,
        debug,
        recording,
        ..
    } = plan;

    // The single encoding point of the request, whatever the number of targets
    // (ADR-0110, decisions 8 and 11).
    let first_target = match targets.first() {
        Some(target) => target,
        None => {
            let failure = error::invalid("'$.targets' must name at least one index");
            let payload = failure_payload(&failure, journal_request);
            return (Err(failure), vec![first], payload);
        }
    };
    let encodes = inputs.encodes();
    let encoding = Instant::now();
    let prepared =
        match inputs.prepare(|query| search::encode_query(state, &first_target.published, query)) {
            Ok(prepared) => prepared,
            // No target ran: the one event of the request is the first target the
            // plan resolved (decision 5).
            Err(failure) => {
                let payload = failure_payload(&failure, journal_request);
                return (Err(failure), vec![first], payload);
            }
        };
    let encoding = encodes.then(|| encoding.elapsed());
    let vector = match &prepared.vector {
        Some(vector) => {
            match search::check_vector(state, &first_target.published, vector.clone()) {
                Ok(vector) => Some(vector),
                Err(failure) => {
                    let payload = failure_payload(&failure, journal_request);
                    return (Err(failure), vec![first], payload);
                }
            }
        }
        None => None,
    };

    // The validation and the single encoding of the request belong to the
    // global `timings_ms` of `debug`; each target renders `0.0` for them
    // (ADR-0119, decision 9).
    // The encoding is measured once, on the observer of the first target and
    // here; the validation happened before it (ADR-0119, decision 9).
    let request_validate = first.request_stages().0;
    let request_encode = encoding;
    let mut observers: Vec<Observed> = targets
        .iter()
        .enumerate()
        .map(|(position, target)| {
            let mut observed = if position == 0 {
                first.clone()
            } else {
                Observed::new(target.index.clone(), Operation::SearchMulti)
            };
            observed.mode(mode);
            observed.shape(analytics::filter_shape(&target.filter));
            if let Some(encoding) = encoding
                && position == 0
            {
                observed.encoded(encoding);
            }
            observed
        })
        .collect();

    let mut outcomes: Vec<(&ResolvedTarget, IndexOutcome)> = Vec::with_capacity(targets.len());
    let mut target_debug: Vec<Value> = Vec::new();
    let mut target_journal: Vec<Value> = Vec::new();
    let mut filter_total = Duration::ZERO;
    let mut dense_total = Duration::ZERO;
    let mut lexical_total = Duration::ZERO;
    let mut fusion_total = Duration::ZERO;
    let mut group_total = Duration::ZERO;
    let mut fetch_total = Duration::ZERO;
    for (position, target) in targets.iter().enumerate() {
        let per_index_fusion = fusion.unwrap_or(target.published.config.search.fusion);
        let target_started = Instant::now();
        // ADR-0122, decision 3: the capture exists for a request that asks for
        // `debug` or is recorded; without either, no vector is allocated.
        let mut capture =
            (debug || recording).then(|| DebugCapture::new(crible_search::debug::DEFAULT_TOP_N));
        let outcome = match search::search_index(
            state,
            &target.index,
            &target.published,
            IndexParams {
                mode,
                vector: vector.as_deref(),
                text: prepared.text.as_deref(),
                filter: &target.filter,
                grouping: &grouping,
                fusion: per_index_fusion,
                k,
                exact,
                capture: capture.as_mut(),
            },
        ) {
            Ok(outcome) => outcome,
            Err(failure) => {
                // One event for every target that ran, the faulty one included:
                // the targets that answered keep their `ok` and their own
                // durations, and the targets that never ran produce nothing
                // (ADR-0117, decision 5).
                if let Some(observed) = observers.get_mut(position) {
                    observed.finish_at(target_started);
                }
                observers.truncate(position + 1);
                let payload = failure_payload(&failure, journal_request);
                return (Err(failure), observers, payload);
            }
        };
        // One reading of the target's total, shared by the event and by the
        // target's `timings_ms` (ADR-0119, decision 7).
        let target_total = target_started.elapsed();
        if let Some(observed) = observers.get_mut(position) {
            observed.target(&outcome, prepared.truncated_query);
            observed.close_total(target_total);
        }
        if debug {
            target_debug.push(json!({
                "index": target.index,
                "plan": crate::debug::plan(
                    mode,
                    outcome.strategy,
                    &target.published.config.search,
                    outcome.pool_size,
                    k,
                    exact,
                    &grouping,
                    per_index_fusion,
                ),
                "timings_ms": crate::debug::timings(&target_stages(mode, &outcome, target_total)),
            }));
        }
        if recording {
            // ADR-0122, decision 4: one entry per target, in the order of the
            // request, with the filter as received and its own generation.
            // ADR-0119, decision 3: `capture` is omitted when no capture was
            // built, never written as an empty object.
            let mut entry = Map::new();
            entry.insert("index".to_owned(), json!(target.index));
            entry.insert("filter".to_owned(), target.filter_json.clone());
            entry.insert("generation".to_owned(), json!(outcome.generation));
            if let Some(capture) = capture.as_ref() {
                entry.insert(
                    "capture".to_owned(),
                    debug_journal::capture_payload(capture),
                );
            }
            target_journal.push(Value::Object(entry));
        }
        filter_total += outcome.filter;
        dense_total += outcome.stages.dense;
        lexical_total += outcome.stages.lexical;
        fusion_total += outcome.stages.fusion;
        group_total += outcome.stages.group;
        fetch_total += outcome.stages.fetch;
        outcomes.push((target, outcome));
    }

    let summaries: Vec<Value> = outcomes
        .iter()
        .map(|(target, outcome)| {
            json!({
                "index": target.index,
                "generation": outcome.generation,
                "pool_size": outcome.pool_size,
            })
        })
        .collect();
    let score_type = if mode == Mode::Dense {
        ScoreType::Cosine
    } else {
        ScoreType::Rrf
    };
    let fusing = Instant::now();
    let entries = fuse(outcomes, mode, &grouping, k);
    // The fusion between indexes belongs to the global `fusion` of `debug` and
    // of the journal; it is timed only for a request that needs it
    // (ADR-0119, decision 9).
    let exchange_fusion = (debug || recording).then(|| fusing.elapsed());
    // ADR-0120, decision 1: the global total stops at the end of `fetch`, after
    // the fusion between indexes and before the JSON rendering of the results.
    // It is read once and shared with the `debug` member (ADR-0119, decisions 7
    // and 9) and with the journal (ADR-0121, decision 4).
    let total = started.elapsed();
    let (results, per_result) =
        match render_entries(entries, &targets, score_type, &selection, debug) {
            Ok(rendered) => rendered,
            // Every target ran: all of them keep their event, and the last one
            // carries the code of the failure (ADR-0117, decision 5).
            Err(failure) => {
                let payload = failure_payload(&failure, journal_request);
                return (Err(failure), observers, payload);
            }
        };
    let stages = StageDurations {
        validate: request_validate,
        encode: request_encode,
        filter: Some(filter_total),
        dense: mode.uses_dense().then_some(dense_total),
        lexical: mode.uses_lexical().then_some(lexical_total),
        fusion: (mode == Mode::Hybrid)
            .then_some(fusion_total + exchange_fusion.unwrap_or_default()),
        group: Some(group_total),
        fetch: Some(fetch_total),
        total: Some(total),
    };
    let debug = debug.then(|| {
        json!({
            "targets": target_debug,
            "timings_ms": crate::debug::timings(&stages),
            "per_result": per_result,
        })
    });
    // ADR-0122, decision 4: one row for the request, the text once, and one
    // entry per target in `targets[]`.
    let journal = recording.then(|| {
        let mut payload = Map::new();
        if let Some(request) = journal_request {
            payload.insert("request".to_owned(), request);
        }
        payload.insert(
            "timings_ms".to_owned(),
            debug_journal::timings_payload(&stages),
        );
        payload.insert("targets".to_owned(), Value::Array(target_journal));
        Value::Object(payload)
    });
    (
        Ok(MultiOutcome {
            results,
            debug,
            targets: summaries,
            truncated_query: prepared.truncated_query,
            took_ms: total.as_secs_f64() * 1_000.0,
        }),
        observers,
        journal,
    )
}

/// Durations of one target, in the shape of `debug`.
///
/// `validate` and `encode` happened once for the request, so they are `0.0`
/// here; the other steps come from the target's own outcome, and its `total` is
/// the duration read once by the route (ADR-0119, decision 9).
fn target_stages(mode: Mode, outcome: &IndexOutcome, total: Duration) -> StageDurations {
    StageDurations {
        filter: Some(outcome.filter),
        dense: mode.uses_dense().then_some(outcome.stages.dense),
        lexical: mode.uses_lexical().then_some(outcome.stages.lexical),
        fusion: (mode == Mode::Hybrid).then_some(outcome.stages.fusion),
        group: Some(outcome.stages.group),
        fetch: Some(outcome.stages.fetch),
        total: Some(total),
        ..StageDurations::default()
    }
}

/// One unit of the fusion between indexes: a passage, or a document under
/// `group_by` (ADR-0110, decision 9).
struct MergedUnit {
    /// Index the unit belongs to, the first tie-break.
    index: String,
    /// External identifier of its document, which keeps its entries together.
    doc_id: String,
    /// Key of its best entry, the last tie-break.
    key: String,
    /// Score the unit is ranked and rendered with.
    score: f32,
    /// Entries the answer renders, in the order of the index.
    entries: Vec<MergedEntry>,
}

/// One entry of a merged unit, with the score the answer renders.
struct MergedEntry {
    /// Index of the entry.
    index: String,
    /// Result and its payload.
    hit: RankedHit,
    /// Score of the answer: the cosine of the entry in `dense`, the score of the
    /// document in `lexical` and `hybrid`.
    score: f32,
    /// Rank of the unit in the final list of its index, from 1 (ADR-0119,
    /// decision 9).
    target_rank: u32,
}

/// Fuses the answers of the targets and truncates to `k` units.
fn fuse(
    outcomes: Vec<(&ResolvedTarget, IndexOutcome)>,
    mode: Mode,
    grouping: &Grouping,
    k: usize,
) -> Vec<MergedEntry> {
    let rrf = mode != Mode::Dense;
    let mut units: Vec<MergedUnit> = Vec::new();
    for (target, outcome) in outcomes {
        units.extend(units_of(&target.index, outcome.hits, grouping, rrf));
    }
    // A total order: score, then index name, then key (ADR-0110, decision 9),
    // so the answer never depends on the order of a hash table.
    units.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.index.cmp(&right.index))
            .then_with(|| left.key.cmp(&right.key))
    });
    units.truncate(k);
    units.into_iter().flat_map(|unit| unit.entries).collect()
}

/// Splits the final list of one index into units, scoring them by mode.
fn units_of(index: &str, hits: Vec<RankedHit>, grouping: &Grouping, rrf: bool) -> Vec<MergedUnit> {
    let mut units = group_hits(index, hits, grouping);
    for (position, unit) in units.iter_mut().enumerate() {
        // The rank is the position of the unit in the final list of its index,
        // from 1; every entry of a document carries its rank and its score
        // (ADR-0110, decision 9; ADR-0119, decision 9).
        let rank = u32::try_from(position + 1).unwrap_or(u32::MAX);
        for entry in &mut unit.entries {
            entry.target_rank = rank;
        }
        if rrf {
            let position = f32::from(u16::try_from(position + 1).unwrap_or(u16::MAX));
            let score = 1.0 / (RRF_CONSTANT + position);
            unit.score = score;
            for entry in &mut unit.entries {
                entry.score = score;
            }
        }
    }
    units
}

/// Groups consecutive entries of one document when the request groups.
///
/// The pipeline wrote the entries of a document consecutively, in the order of
/// the index, so the walk of the list is enough. Without `group_by`, every
/// entry is a unit of its own.
fn group_hits(index: &str, hits: Vec<RankedHit>, grouping: &Grouping) -> Vec<MergedUnit> {
    let mut units: Vec<MergedUnit> = Vec::new();
    for hit in hits {
        let entry_index = index.to_owned();
        if grouping.by_document
            && let Some(last) = units.last_mut()
            && last.doc_id == hit.hit.doc_id
        {
            last.entries.push(MergedEntry {
                index: entry_index,
                score: hit.hit.score,
                hit,
                target_rank: 0,
            });
            continue;
        }
        units.push(MergedUnit {
            index: index.to_owned(),
            doc_id: hit.hit.doc_id.clone(),
            key: hit.hit.key.clone(),
            score: hit.hit.score,
            entries: vec![MergedEntry {
                index: entry_index,
                score: hit.hit.score,
                hit,
                target_rank: 0,
            }],
        });
    }
    units
}

/// Renders the fused entries with the schema of the index of each one.
///
/// A name of `return.fields` that an index does not declare is simply absent
/// from its results (ADR-0110, decision 6), which is why each entry is rendered
/// against its own schema.
fn render_entries(
    entries: Vec<MergedEntry>,
    targets: &[ResolvedTarget],
    score_type: ScoreType,
    selection: &ReturnSelection,
    debug: bool,
) -> Result<(Vec<Value>, Vec<Value>), CribleError> {
    let mut rendered = Vec::with_capacity(entries.len());
    let mut per_result = Vec::new();
    for entry in entries {
        let target = targets
            .iter()
            .find(|target| target.index == entry.index)
            .ok_or_else(|| {
                error::internal(
                    "a fused entry names an index the request did not target, so its schema is \
                     unknown",
                )
            })?;
        let mut hit = entry.hit;
        hit.hit.score = entry.score;
        let mut value =
            api::result_json(&hit, score_type, &target.published.config.schema, selection);
        if let Some(object) = value.as_object_mut() {
            object.insert("index".to_owned(), json!(entry.index));
        }
        if debug {
            per_result.push(crate::debug::multi_result_entry(
                &entry.index,
                &hit,
                entry.target_rank,
            ));
        }
        rendered.push(value);
    }
    Ok((rendered, per_result))
}

/// One answer of the multi-index search, before its envelope.
struct MultiOutcome {
    results: Vec<Value>,
    /// `targets[]`, `timings_ms` and `per_result` of a `debug` request; absent
    /// without `debug` (ADR-0119, decision 9).
    debug: Option<Value>,
    targets: Vec<Value>,
    truncated_query: bool,
    took_ms: f64,
}

impl MultiOutcome {
    /// Renders the body of the `200`, with the request identifier of ADR-0098.
    fn render(self, request_id: &str) -> Value {
        let mut body = json!({
            "results": self.results,
            "targets": self.targets,
            "took_ms": self.took_ms,
            "truncated_query": self.truncated_query,
            "request_id": request_id,
        });
        if let Some(debug) = self.debug {
            body["debug"] = debug;
        }
        body
    }
}

#[cfg(test)]
mod tests {
    use super::{Observed, Operation, events_of};
    use crate::analytics::{Record, SearchEvent};
    use std::time::Instant;

    /// An observer of a target that ran, closed on its own duration.
    fn ran(index: &str) -> Observed {
        let started = Instant::now();
        let mut observed = Observed::new_at(index, Operation::SearchMulti, Instant::now());
        observed.finish_at(started);
        observed
    }

    /// The events of the records, in their order.
    fn events(records: Vec<Record>) -> Vec<SearchEvent> {
        records
            .into_iter()
            .filter_map(|record| match record {
                Record::Search { event, .. } => Some(event),
                Record::Operation(_) => None,
            })
            .collect()
    }

    #[test]
    fn a_failure_after_the_first_target_leaves_one_event_per_target_that_ran() {
        // ADR-0117, decision 5: one event per target, never a consolidated row.
        // The targets that answered keep `ok`, the faulty one carries the code,
        // and a target that never ran produces no event.
        let written = events(events_of(vec![ran("alpha"), ran("beta")], "timeout"));
        assert_eq!(
            written.len(),
            2,
            "one event per target that ran: {written:?}"
        );
        assert_eq!(written[0].index, "alpha");
        assert_eq!(written[0].status, "ok");
        assert_eq!(written[1].index, "beta");
        assert_eq!(written[1].status, "timeout");
    }

    #[test]
    fn a_request_that_failed_before_any_target_keeps_one_event() {
        // No target ran: the single event is the first target the plan resolved,
        // and it carries the code of the failure.
        let written = events(events_of(vec![ran("alpha")], "invalid_request"));
        assert_eq!(written.len(), 1, "{written:?}");
        assert_eq!(written[0].index, "alpha");
        assert_eq!(written[0].status, "invalid_request");
    }
}
