// SPDX-License-Identifier: Apache-2.0

//! Router of the server: the two probes and the request identifier.
//!
//! References: ADR-0098 (`x-request-id`), ADR-0099 (`/health`, `/ready`) and
//! `docs/05-api.md` (conventions). The `/v1` routes arrive with the tasks that
//! create them and mount on this router; nothing here anticipates their shape.
//!
//! `/health` answers `200` from the bind, whatever the loader is doing, and
//! `/ready` answers `503` until the models are loaded and every index has
//! published its snapshot. Neither probe asks for authentication
//! (`docs/05-api.md`, role "—"), and neither body carries an index name, a
//! field name, a path or a detailed count (ADR-0099): the body only tells
//! "alive" from "ready".
//!
//! Every response, including a `404` rendered before any route, carries
//! `x-request-id`. The value is the caller's header when it holds at most 128
//! bytes of `[A-Za-z0-9._:-]`; otherwise a UUID v4 is generated and the refused
//! value is never journalised (ADR-0098).

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, Query, Request, State};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Extension, Json, Router};
use crible_core::error::{CribleError, ErrorCode};
use crible_store::debug_journal::JournalFilter;
use crible_store::jobs::JobState;
use serde_json::{Value, json};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::{Instrument, Level, error, info_span};
use utoipa::OpenApi as _;
use uuid::Uuid;

use crate::AppState;
use crate::auth::{Auth, protect};
use crate::config::Role;
use crate::error;
use crate::indexes;
use crate::jobs::JobView;
use crate::limits::MAX_BODY_BYTES;
use crate::{batches, documents, maintenance, search, search_multi};

/// Header carrying the request identifier on every response (ADR-0098).
pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// Longest request identifier accepted from the caller, in bytes (ADR-0098).
const MAX_REQUEST_ID_BYTES: usize = 128;

/// Identifier of the request being served, for the error body and the logs.
///
/// `AUTHENTIFIER-CLES` and `FORMAT-ERREURS-ET-LIMITES` read it; the probes of
/// this task do not need it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestId(pub String);

/// Builds the router of an instance.
///
/// The state is the one created at the bind: the router answers `503` on
/// `/ready` until [`AppState::mark_ready`] is called.
///
/// The job routes of `docs/05-api.md` are mounted here with the `write` role
/// (ADR-0095, ADR-0103). When no authenticator is published yet — a test router,
/// or the bind before the startup — the closed-by-default one refuses every
/// request with `401`, as an empty `[[auth.keys]]` does (ADR-0100).
pub fn build_router(state: Arc<AppState>) -> Router {
    let auth = state.auth().unwrap_or_else(|| Arc::new(Auth::closed()));
    let write = protect(writes_routes(), Arc::clone(&auth), Role::Write);
    let jobs = protect(jobs_routes(), Arc::clone(&auth), Role::Write);
    let admin = protect(indexes_admin_routes(), Arc::clone(&auth), Role::Admin);
    let debug = protect(debug_routes(), Arc::clone(&auth), Role::Admin);
    let read = protect(indexes_read_routes(), auth, Role::Read);
    // The specification and Swagger UI are served by the instance, without a
    // key: their paths are the two exceptions of `docs/05-api.md` and never ask
    // for authentication (ADR-0111, decision 4). The UI reads the specification
    // from `/openapi.json`, mounted here as well.
    let documentation = Router::from(
        utoipa_swagger_ui::SwaggerUi::new("/docs")
            .url("/openapi.json", crate::openapi::ApiDoc::openapi()),
    );
    with_common_layers(
        Router::new()
            .route("/health", get(health))
            .route("/ready", get(ready))
            .merge(documentation)
            .merge(write)
            .merge(jobs)
            .merge(admin)
            .merge(debug)
            .merge(read),
    )
    .with_state(state)
}

/// The write routes of documents and deposited batches, `write` only
/// (ADR-0107, decision 8).
fn writes_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/v1/indexes/{index}/documents",
            post(documents::post_documents).patch(documents::patch_documents),
        )
        .route("/v1/indexes/{index}/batches", post(batches::post_batch))
}

/// The administration routes of `/v1/indexes`, `admin` only (ADR-0105,
/// decision 7).
fn indexes_admin_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/indexes", post(indexes::create_index))
        .route(
            "/v1/indexes/{index}",
            patch(indexes::patch_index).delete(indexes::delete_index),
        )
        .route("/v1/indexes/{index}/compact", post(maintenance::compact))
        .route("/v1/indexes/{index}/exports", post(maintenance::exports))
        .route(
            "/v1/indexes/{index}/snapshots",
            post(maintenance::snapshots),
        )
}

/// The two reads of `/v1/indexes`, open to `read` (ADR-0105, decision 7).
fn indexes_read_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/indexes", get(indexes::list_indexes))
        .route("/v1/indexes/{index}", get(indexes::get_index))
        .route(
            "/v1/indexes/{index}/documents/{doc_id}",
            get(documents::get_document),
        )
        // The search is open to `read`, and `write` and `admin` include it, so
        // the three roles reach the route (ADR-0095, ADR-0109).
        .route("/v1/indexes/{index}/search", post(search::search))
        // The multi-index search is open to the same role, for the same reason
        // (ADR-0110, ADR-0095).
        .route("/v1/search", post(search_multi::search))
}

/// The debug routes of the instance: the journal, `admin` only (ADR-0122,
/// decision 9).
fn debug_routes() -> Router<Arc<AppState>> {
    Router::new().route("/v1/debug/queries", get(debug_queries))
}

/// The job routes of the instance: the list and the detail, `write` only.
///
/// No write route exists yet (`ROUTES-INDEX`, `ROUTES-DOCUMENTS-ET-LOTS`); the
/// routes that create a job will call [`AppState::submit_job`] and
/// [`accepted`], and mount beside these.
fn jobs_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/jobs", get(list_jobs))
        .route("/v1/jobs/{job_id}", get(get_job))
}

/// `GET /v1/jobs`: the retained jobs, filtered by `index` and `state`.
///
/// The order is `created_at` descending then `id`, without pagination, and a
/// `state` outside the four of `docs/05-api.md` is `400 invalid_request`
/// (ADR-0103, decision 7).
#[utoipa::path(
    get,
    path = "/v1/jobs",
    tag = "jobs",
    description = "The retained jobs, `created_at` descending then `id`, without pagination. \
                   Role: `write`; `admin` includes it (ADR-0095).",
    params(
        ("index" = Option<String>, Query, description = "Only the jobs of that index"),
        ("state" = Option<String>, Query, description = "Only the jobs in that state: `queued`, `running`, `succeeded` or `failed`")
    ),
    responses(
        (status = 200, description = "The retained jobs", body = crate::openapi::JobList),
        (status = 400, description = "`state` outside the four states", body = crate::openapi::Error),
        (status = 401, description = "Missing, malformed or unknown API key", body = crate::openapi::Error),
        (status = 403, description = "Insufficient role", body = crate::openapi::Error),
        (status = 500, description = "Unexpected failure", body = crate::openapi::Error)
    )
)]
async fn list_jobs(
    State(state): State<Arc<AppState>>,
    Extension(identifier): Extension<RequestId>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let wanted = match query.get("state") {
        Some(raw) => match JobState::parse(raw) {
            Some(state) => Some(state),
            None => {
                let failure =
                    error::invalid("'state' must be one of queued, running, succeeded or failed");
                return error::rendered(&failure, &identifier.0);
            }
        },
        None => None,
    };
    let jobs = state.list_jobs(query.get("index").map(String::as_str), wanted);
    Json(json!({
        "jobs": jobs.iter().map(JobView::to_json).collect::<Vec<_>>()
    }))
    .into_response()
}

/// `GET /v1/jobs/{job_id}`: one retained job, or `404 not_found`.
#[utoipa::path(
    get,
    path = "/v1/jobs/{job_id}",
    tag = "jobs",
    description = "One retained job. An unknown or purged job is `404 not_found`. Role: `write`; \
                   `admin` includes it (ADR-0095).",
    params(("job_id" = String, Path, description = "Identifier of the job")),
    responses(
        (status = 200, description = "The job", body = crate::openapi::Job),
        (status = 401, description = "Missing, malformed or unknown API key", body = crate::openapi::Error),
        (status = 403, description = "Insufficient role", body = crate::openapi::Error),
        (status = 404, description = "Unknown or purged job", body = crate::openapi::Error),
        (status = 500, description = "Unexpected failure", body = crate::openapi::Error)
    )
)]
async fn get_job(
    State(state): State<Arc<AppState>>,
    Extension(identifier): Extension<RequestId>,
    Path(job_id): Path<String>,
) -> Response {
    match state.job(&job_id) {
        Some(job) => (StatusCode::OK, Json(job.to_json())).into_response(),
        None => {
            let failure = CribleError::new(ErrorCode::NotFound, "job not found");
            error::rendered(&failure, &identifier.0)
        }
    }
}

/// `GET /v1/debug/queries`: one page of the debug journal, `admin` only
/// (ADR-0122, decision 9).
///
/// The parameters are checked **before** the journal is read, so a refused
/// parameter is a `400` even when the journal is disabled; a disabled journal
/// answers `200` with `enabled: false` and opens no file (ADR-0121, decision 1).
/// The payload of each row is parsed and rendered as an object; an unreadable
/// payload is an `internal` failure of the route (ADR-0122, decision 9).
#[utoipa::path(
    get,
    path = "/v1/debug/queries",
    tag = "debug",
    operation_id = "debug_queries",
    description = "One page of the debug journal, `created_at` descending then `id`. When the \
                   journal is disabled, `200` with `enabled: false` and an empty list, without \
                   opening the file. Role: `admin` (ADR-0095, ADR-0122).",
    params(
        ("index" = Option<String>, Query, description = "Name of the index; a multi-index row is retained when it targets that index"),
        ("min_total_ms" = Option<f64>, Query, description = "Keep the rows whose `total_ms` is at least this value; a `null` total is excluded"),
        ("from" = Option<String>, Query, description = "Lower bound, included, of `created_at`, ISO 8601 in UTC"),
        ("to" = Option<String>, Query, description = "Upper bound, included, of `created_at`, ISO 8601 in UTC"),
        ("limit" = Option<usize>, Query, description = "Number of rows, from 1 to 1 000; 100 by default"),
        ("before_id" = Option<i64>, Query, description = "Keyset cursor: only the rows of a strictly lower `id`")
    ),
    responses(
        (status = 200, description = "One page of the journal", body = crate::openapi::DebugQueryList),
        (status = 400, description = "Unknown parameter, or a value outside its domain, naming the parameter", body = crate::openapi::Error),
        (status = 401, description = "Missing, malformed or unknown API key", body = crate::openapi::Error),
        (status = 403, description = "Insufficient role", body = crate::openapi::Error),
        (status = 500, description = "Unexpected failure, or a payload the route cannot render", body = crate::openapi::Error)
    )
)]
async fn debug_queries(
    State(state): State<Arc<AppState>>,
    Extension(identifier): Extension<RequestId>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let filter = match journal_filter(&query) {
        Ok(filter) => filter,
        Err(failure) => return error::rendered(&failure, &identifier.0),
    };
    let enabled = state
        .loaded()
        .is_some_and(|loaded| loaded.debug_journal_config().enabled);
    if !enabled {
        let list = crate::openapi::DebugQueryList {
            enabled: false,
            dropped: crate::debug_journal::dropped(),
            queries: Vec::new(),
        };
        return Json(list).into_response();
    }
    let Some(root) = state.data_root() else {
        let failure = error::internal("the instance has not loaded its configuration");
        return error::rendered(&failure, &identifier.0);
    };
    let rows = match crate::debug_journal::page(root, filter).await {
        Ok(rows) => rows,
        Err(failure) => return error::rendered(&failure, &identifier.0),
    };
    let mut queries = Vec::with_capacity(rows.len());
    for row in rows {
        let payload = match serde_json::from_str::<crate::openapi::FreeObject>(&row.payload) {
            Ok(payload) => payload,
            Err(_) => {
                // ADR-0122, decision 9: an unreadable payload is an error of the
                // route, never an opaque string; the request id is logged, the
                // payload is not.
                let failure = error::internal(format!(
                    "the payload of the row '{}' of the debug journal is not a JSON object",
                    row.request_id
                ));
                return error::rendered(&failure, &identifier.0);
            }
        };
        queries.push(crate::openapi::DebugQuery {
            id: row.id,
            created_at: row.created_at,
            request_id: row.request_id,
            index: row.index,
            client: row.client,
            mode: row.mode,
            status: row.status,
            total_ms: row.total_ms,
            payload,
        });
    }
    let list = crate::openapi::DebugQueryList {
        enabled: true,
        dropped: crate::debug_journal::dropped(),
        queries,
    };
    Json(list).into_response()
}

/// The parameters of `GET /v1/debug/queries`, checked and normalised
/// (ADR-0122, decision 9).
///
/// Every refusal names the parameter and its JSON path, and none reads the file
/// or the configuration: a `400` is decided on the query string alone.
fn journal_filter(query: &HashMap<String, String>) -> Result<JournalFilter, CribleError> {
    const ALLOWED: [&str; 6] = ["index", "min_total_ms", "from", "to", "limit", "before_id"];
    for parameter in query.keys() {
        if !ALLOWED.contains(&parameter.as_str()) {
            return Err(error::invalid(format!(
                "'{parameter}' is not a parameter of this route; the parameters are `index`, \
                 `min_total_ms`, `from`, `to`, `limit` and `before_id`"
            )));
        }
    }
    let limit = match query.get("limit") {
        Some(raw) => match raw.parse::<usize>() {
            Ok(limit) if (1..=crate::debug_journal::MAX_PAGE).contains(&limit) => limit,
            _ => {
                return Err(error::invalid(format!(
                    "'limit' must be an integer from 1 to {}",
                    crate::debug_journal::MAX_PAGE
                )));
            }
        },
        None => crate::debug_journal::DEFAULT_PAGE,
    };
    let before_id = match query.get("before_id") {
        Some(raw) => match raw.parse::<i64>() {
            Ok(id) if id >= 0 => Some(id),
            _ => {
                return Err(error::invalid("'before_id' must be a non-negative integer"));
            }
        },
        None => None,
    };
    let min_total_ms = match query.get("min_total_ms") {
        Some(raw) => match raw.parse::<f64>() {
            Ok(value) if value.is_finite() && value >= 0.0 => Some(value),
            _ => {
                return Err(error::invalid(
                    "'min_total_ms' must be a non-negative number",
                ));
            }
        },
        None => None,
    };
    let from = match query.get("from") {
        Some(raw) => Some(journal_timestamp(raw, "from")?),
        None => None,
    };
    let to = match query.get("to") {
        Some(raw) => Some(journal_timestamp(raw, "to")?),
        None => None,
    };
    Ok(JournalFilter {
        index: query.get("index").cloned(),
        min_total_ms,
        from,
        to,
        limit,
        before_id,
    })
}

/// Normalises an ISO 8601 date of the query string to the millisecond form
/// SQLite writes, `YYYY-MM-DDTHH:MM:SS.mmmZ`.
///
/// The comparison of the file is a string comparison, so the bound takes the
/// exact shape of `created_at` (ADR-0121, decision 3). The date may carry the `Z`
/// suffix or a `±HH:MM` offset, which is applied to read the instant in UTC; a
/// calendar day that does not exist, a leap second and a malformed offset are
/// refused, with a message naming `parameter` (ADR-0122, decision 9).
fn journal_timestamp(raw: &str, parameter: &str) -> Result<String, CribleError> {
    let refused = || {
        error::invalid(format!(
            "'{parameter}' must be an ISO 8601 date, such as \
             2026-10-31T10:00:00.123Z or 2026-10-31T12:00:00+02:00"
        ))
    };
    let (body, offset) = split_offset(raw).ok_or_else(refused)?;
    let bytes = body.as_bytes();
    if bytes.len() < 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return Err(refused());
    }
    let year = i64::from(ascii_digits(body, 0, 4).ok_or_else(refused)?);
    let month = ascii_digits(body, 5, 7).ok_or_else(refused)?;
    let day = ascii_digits(body, 8, 10).ok_or_else(refused)?;
    let hour = ascii_digits(body, 11, 13).ok_or_else(refused)?;
    let minute = ascii_digits(body, 14, 16).ok_or_else(refused)?;
    let second = ascii_digits(body, 17, 19).ok_or_else(refused)?;
    if !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err(refused());
    }
    let fraction = body.get(19..).ok_or_else(refused)?;
    let millis = if fraction.is_empty() {
        0
    } else {
        let digits = fraction.strip_prefix('.').ok_or_else(refused)?;
        if digits.is_empty()
            || digits.len() > 9
            || !digits.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(refused());
        }
        let truncated = digits.get(..digits.len().min(3)).ok_or_else(refused)?;
        let mut padded = truncated.to_owned();
        while padded.len() < 3 {
            padded.push('0');
        }
        padded.parse::<u32>().map_err(|_| refused())?
    };
    // The offset states the local reading; UTC is that reading minus it, so a
    // date east of Greenwich is earlier in UTC (ADR-0122, decision 9).
    let local = days_from_civil(year, month, day) * 86_400
        + i64::from(hour) * 3_600
        + i64::from(minute) * 60
        + i64::from(second);
    let utc = local - i64::from(offset);
    let days = utc.div_euclid(86_400);
    let remaining = utc.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    if !(0..=9_999).contains(&year) {
        return Err(refused());
    }
    let hour = remaining / 3_600;
    let minute = remaining % 3_600 / 60;
    let second = remaining % 60;
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z"
    ))
}

/// Splits an ISO 8601 date into its local reading and its offset in seconds east
/// of UTC: `Z` is zero, `+HH:MM` and `-HH:MM` are read.
///
/// The sign is searched after the time, so the dashes of the calendar date are
/// never read as an offset. A suffix of any other shape yields `None`.
fn split_offset(raw: &str) -> Option<(&str, i32)> {
    if let Some(body) = raw.strip_suffix('Z') {
        return Some((body, 0));
    }
    let digits = raw.get(19..)?;
    let position = 19 + digits.find(['+', '-'])?;
    let sign = match raw.as_bytes().get(position)? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    if raw.as_bytes().get(position + 3) != Some(&b':') || raw.len() != position + 6 {
        return None;
    }
    let hours = ascii_digits(raw, position + 1, position + 3)?;
    let minutes = ascii_digits(raw, position + 4, position + 6)?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    let offset = i32::try_from(hours * 3_600 + minutes * 60).ok()?;
    Some((raw.get(..position)?, sign * offset))
}

/// Number of days from 1970-01-01 to `year-month-day`, Gregorian calendar.
///
/// Howard Hinnant's `days_from_civil`, with the year starting in March so that
/// the leap day is the last day of the year.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted_month = i64::from((month + 9) % 12);
    let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The Gregorian date `year-month-day` of a number of days from 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (
        year,
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
    )
}

/// Reads the four ASCII digits of `raw` in `[start, end)`.
fn ascii_digits(raw: &str, start: usize, end: usize) -> Option<u32> {
    let slice = raw.get(start..end)?;
    if !slice.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    slice.parse().ok()
}

/// Number of days of `month` in `year`, on the Gregorian calendar.
fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

/// Renders the answer to a job submission, the helper every write route uses.
///
/// Without `?wait=true`, the answer is `202 Accepted` with the job as accepted.
/// With it, the call waits for the job under `[limits].wait_timeout_ms`: a job
/// that finishes in the delay is rendered terminal, in `200`; at the deadline,
/// and as soon as the server begins to drain, the answer is `202 Accepted` with
/// the job in its current state, never `504` (ADR-0067, ADR-0104).
pub async fn accepted(state: &AppState, job: JobView, wait: bool) -> Response {
    if wait && let Some(current) = state.wait_for_job(&job.id).await {
        let status = if current.state.is_terminal() {
            StatusCode::OK
        } else {
            StatusCode::ACCEPTED
        };
        return (status, Json(current.to_json())).into_response();
    }
    (StatusCode::ACCEPTED, Json(job.to_json())).into_response()
}

/// Applies the layers every router of the server shares.
///
/// The trace layer and the request identifier are mounted here so a route
/// added by another task, or a test router of the middleware, carries them the
/// same way: a `401` rendered before a route still names its request (ADR-0098),
/// and no log line records the path with its query string (ADR-0096).
///
/// The same holds for the limits and the error format of
/// `FORMAT-ERREURS-ET-LIMITES`, which are mounted here rather than in
/// [`build_router`]: a test router meets the `400`, `404`, `413`, `500` and
/// `503` of `docs/05-api.md` exactly as the real routes do (ADR-0095,
/// ADR-0101, ADR-0102).
///
/// The function is applied once, at the root of a router, but a second
/// application is harmless: the request identifier already assigned is reused
/// (ADR-0098).
pub fn with_common_layers<S>(router: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    with_body_limit(router, MAX_BODY_BYTES)
}

/// Applies the layers of [`with_common_layers`] with an explicit body bound.
///
/// The bound is a parameter so a test proves the path without a declared
/// `Content-Length` with one kilobyte instead of 32 MiB; the server passes
/// [`MAX_BODY_BYTES`], the constant of ADR-0102, and no configuration key
/// reaches this function (ADR-0097).
fn with_body_limit<S>(router: Router<S>, body_limit: usize) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router
        // The fallback is added before the layers so they wrap it as well: an
        // unknown route carries the identifier, follows the body bound and gets
        // the format of ADR-0101.
        .fallback(error::unknown_route)
        // One body bound for the whole instance, a constant of the code
        // (ADR-0102, decision 1). axum's own bound is removed first, so the one
        // of the code is the only bound a client meets.
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(body_limit))
        // A panic becomes a `500` instead of a dropped connection (ADR-0101,
        // decision 7); the formatting layer below writes its body.
        .layer(CatchPanicLayer::custom(error::panic_response))
        // Every response of another layer takes the format of `docs/05-api.md`,
        // and every error response gets its one line (ADR-0101).
        .layer(middleware::from_fn(error::render_layer_errors))
        .layer(
            TraceLayer::new_for_http()
                // The path only: `docs/06` forbids a query text and a filter
                // value in a log at any level, and `RUST_LOG` makes `debug`
                // reachable. The default span of `TraceLayer` records the whole
                // URI, query string included.
                .make_span_with(|request: &Request<Body>| {
                    info_span!(
                        "http",
                        method = %request.method(),
                        path = %request.uri().path()
                    )
                })
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(middleware::from_fn(assign_request_id))
}

/// `/health`: the process is alive.
///
/// Fixed body, independent of the indexes and of the models: it answers during
/// a long recovery, which is the reason the server binds before loading
/// (ADR-0099).
#[utoipa::path(
    get,
    path = "/health",
    tag = "probes",
    description = "The process is alive. The body is fixed and independent of the indexes and \
                   of the models, so the probe answers during a long recovery. Role: none; the \
                   route needs no API key (ADR-0099).",
    responses((status = 200, description = "The process is alive", body = crate::openapi::Health)),
    security(())
)]
async fn health() -> (StatusCode, Json<Value>) {
    (StatusCode::OK, Json(json!({ "status": "ok" })))
}

/// `/ready`: models loaded, every index published, no recovery in progress.
///
/// `503` with `{"status":"not_ready"}` until the loader has finished, `200` with
/// `{"status":"ready"}` after. The two bodies tell "alive" from "ready" and
/// reveal nothing else (ADR-0099). They are the one body the error format of
/// ADR-0101 leaves alone, which [`error::preserved`] marks.
#[utoipa::path(
    get,
    path = "/ready",
    tag = "probes",
    description = "Ready: the models are loaded, every index has published its snapshot and no \
                   recovery is in progress. `503` until then, `200` after; a not-ready instance \
                   answers its state, not an error of ADR-0101. Role: none; the route needs no \
                   API key (ADR-0099).",
    responses(
        (status = 200, description = "Every index has published its snapshot", body = crate::openapi::Ready),
        (status = 503, description = "Not ready yet; the body is the one of ADR-0099, not the error format", body = crate::openapi::NotReady)
    ),
    security(())
)]
async fn ready(State(state): State<Arc<AppState>>) -> Response {
    let (status, body) = if state.is_ready() {
        (StatusCode::OK, json!({ "status": "ready" }))
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "status": "not_ready" }),
        )
    };
    error::preserved((status, Json(body)).into_response())
}

/// Assigns the request identifier and returns it on the response.
///
/// The layer is the outermost one: a `404`, a `405` or a rejection rendered
/// before any route still carries the header (ADR-0098). The span carries the
/// identifier so every line logged during the request names it.
///
/// The layer is idempotent: a router already carrying `with_common_layers` keeps
/// the identifier of the outer application, so wrapping it again cannot give the
/// header and the body of one error two different identifiers (ADR-0098).
async fn assign_request_id(mut request: Request, next: Next) -> Response {
    let identifier = match request.extensions().get::<RequestId>() {
        Some(existing) => existing.0.clone(),
        None => request
            .headers()
            .get(REQUEST_ID_HEADER)
            .and_then(accepted_request_id)
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
    };
    request
        .extensions_mut()
        .insert(RequestId(identifier.clone()));
    let span = info_span!("request", request_id = %identifier);
    let mut response = next.run(request).instrument(span).await;
    match HeaderValue::from_str(&identifier) {
        Ok(value) => {
            response
                .headers_mut()
                .insert(HeaderName::from_static(REQUEST_ID_HEADER), value);
        }
        Err(_) => {
            // Unreachable: the identifier is either a validated ASCII value or
            // a UUID. Logged rather than ignored, because a response without
            // the header would break the trace with no diagnostic.
            error!("the request identifier cannot be sent back as a header value");
        }
    }
    response
}

/// Returns the caller's identifier when it is acceptable, otherwise `None`.
///
/// ADR-0098: at most 128 bytes, and only `[A-Za-z0-9._:-]`. A refused value is
/// dropped without a trace: its length and its content are never journalised.
fn accepted_request_id(value: &HeaderValue) -> Option<String> {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_REQUEST_ID_BYTES {
        return None;
    }
    let accepted = bytes.iter().all(|byte| {
        matches!(
            byte,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b':' | b'-'
        )
    });
    if !accepted {
        return None;
    }
    std::str::from_utf8(bytes).ok().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{REQUEST_ID_HEADER, RequestId, build_router};
    use crate::state::AppState;
    use axum::Json;
    use axum::Router;
    use axum::body::Body;
    use axum::extract::Extension;
    use axum::extract::Request;
    use axum::http::StatusCode;
    use axum::response::Response;
    use std::sync::Arc;
    use tower::ServiceExt;

    /// Sends one `GET`, with an optional caller identifier, through the router.
    async fn get(router: &Router, path: &str, identifier: Option<&str>) -> Response {
        let mut request = Request::builder().uri(path);
        if let Some(identifier) = identifier {
            request = request.header(REQUEST_ID_HEADER, identifier);
        }
        let request = request
            .body(Body::empty())
            .expect("the request is well formed");
        router
            .clone()
            .oneshot(request)
            .await
            .expect("the router always answers")
    }

    fn identifier_of(response: &Response) -> String {
        let value = response
            .headers()
            .get(REQUEST_ID_HEADER)
            .unwrap_or_else(|| panic!("every response carries {REQUEST_ID_HEADER}"));
        value.to_str().expect("the identifier is ASCII").to_owned()
    }

    /// Whether `value` has the canonical form of a UUID v4.
    fn is_uuid_v4(value: &str) -> bool {
        let bytes = value.as_bytes();
        bytes.len() == 36
            && bytes.iter().enumerate().all(|(index, byte)| match index {
                8 | 13 | 18 | 23 => *byte == b'-',
                14 => *byte == b'4',
                _ => byte.is_ascii_digit() || (b'a'..=b'f').contains(byte),
            })
    }

    #[tokio::test]
    async fn a_valid_incoming_identifier_is_echoed() {
        // First state of ADR-0098: reprise of the caller's header.
        let router = build_router(Arc::new(AppState::new()));
        let response = get(&router, "/health", Some("mcp-42:retry.1_abc-DEF")).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(identifier_of(&response), "mcp-42:retry.1_abc-DEF");
    }

    #[tokio::test]
    async fn an_absent_identifier_is_generated() {
        let router = build_router(Arc::new(AppState::new()));
        let response = get(&router, "/health", None).await;
        let identifier = identifier_of(&response);
        assert!(is_uuid_v4(&identifier), "got {identifier}");
    }

    #[tokio::test]
    async fn an_identifier_longer_than_128_bytes_is_replaced() {
        let router = build_router(Arc::new(AppState::new()));
        let too_long = "a".repeat(129);
        let response = get(&router, "/health", Some(&too_long)).await;
        let identifier = identifier_of(&response);
        assert_ne!(identifier, too_long);
        assert!(is_uuid_v4(&identifier), "got {identifier}");
    }

    #[tokio::test]
    async fn an_identifier_with_a_forbidden_character_is_replaced() {
        // ADR-0098: a space or any character outside `[A-Za-z0-9._:-]` makes
        // the value refused, never journalised. A value with a control
        // character never reaches the router: `HeaderValue` refuses to build it.
        let router = build_router(Arc::new(AppState::new()));
        for refused in ["two words", "a/b", "a+b"] {
            let response = get(&router, "/health", Some(refused)).await;
            let identifier = identifier_of(&response);
            assert_ne!(identifier, refused);
            assert!(is_uuid_v4(&identifier), "got {identifier}");
        }
    }

    #[tokio::test]
    async fn a_router_wrapped_twice_keeps_one_identifier() {
        // ADR-0098: the layers are applied once, at the root of the router. A
        // router that carries them again must keep the identifier of the outer
        // application — otherwise the header and the body of one error would
        // name two different requests.
        let router = super::with_common_layers(super::with_common_layers(
            Router::new().route("/identifier", axum::routing::get(current_identifier)),
        ));
        let response = get(&router, "/identifier", None).await;
        let header = identifier_of(&response);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("the body is readable");
        assert_eq!(
            String::from_utf8_lossy(&body),
            header,
            "the handler and the header name the same request (ADR-0098)"
        );
    }

    /// Echoes the identifier of the request, for the idempotence test.
    async fn current_identifier(Extension(identifier): Extension<RequestId>) -> String {
        identifier.0
    }

    #[tokio::test]
    async fn a_response_before_any_route_carries_the_identifier() {
        // ADR-0098: a `404` rendered by the fallback still carries the header,
        // because the layer is the outermost one.
        let router = build_router(Arc::new(AppState::new()));
        let response = get(&router, "/no-such-route", None).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(is_uuid_v4(&identifier_of(&response)));
    }

    #[tokio::test]
    async fn the_probes_tell_alive_from_ready() {
        // `docs/06` and ADR-0099: two bodies, two meanings. The state is not
        // ready until the loader publishes it.
        let state = Arc::new(AppState::new());
        let router = build_router(Arc::clone(&state));

        let health = get(&router, "/health", None).await;
        assert_eq!(health.status(), StatusCode::OK);
        let ready = get(&router, "/ready", None).await;
        assert_eq!(ready.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            body_json(ready).await["status"],
            "not_ready",
            "the body of a not-ready instance names its state"
        );

        state.mark_ready();
        let ready = get(&router, "/ready", None).await;
        assert_eq!(ready.status(), StatusCode::OK);
        assert_eq!(body_json(ready).await["status"], "ready");
        assert_eq!(
            body_json(get(&router, "/health", None).await).await["status"],
            "ok",
            "the two bodies stay distinct"
        );
    }

    #[tokio::test]
    async fn the_probes_need_no_authentication() {
        // `docs/05-api.md` gives the two probes the role "—": no `Authorization`
        // header is sent here, and both answer on their own status.
        let state = Arc::new(AppState::new());
        state.mark_ready();
        let router = build_router(state);
        assert_ne!(
            get(&router, "/health", None).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_ne!(
            get(&router, "/ready", None).await.status(),
            StatusCode::UNAUTHORIZED
        );
    }

    /// Decodes the JSON body of a response.
    async fn body_json(response: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("the body is readable");
        serde_json::from_slice(&bytes).expect("the body of a probe is JSON")
    }

    /// Echoes its body, for the test of the body bound.
    async fn echo(Json(value): Json<serde_json::Value>) -> Json<serde_json::Value> {
        Json(value)
    }

    #[tokio::test]
    async fn a_body_without_a_declared_length_above_the_bound_is_a_payload_too_large() {
        // ADR-0102, decision 1, second path of the bound: without a
        // `Content-Length` header, the body is read and the limit fails while it
        // streams; axum turns that `http_body_util::LengthLimitError` into a
        // `413`, which the formatting layer renders. The bound is a parameter of
        // this module, so the test sends four kilobytes: the server passes the
        // constant of the ADR, and no configuration key reaches it.
        let router = super::with_body_limit(
            Router::new().route("/echo", axum::routing::post(echo)),
            1_024,
        );
        let payload = format!(r#"{{"value":"{}"}}"#, "a".repeat(4_096));
        let request = Request::builder()
            .method("POST")
            .uri("/echo")
            .header(REQUEST_ID_HEADER, "body-bound-test")
            .header("content-type", "application/json")
            .body(Body::from(payload))
            .expect("the request is well formed");
        let response = router
            .oneshot(request)
            .await
            .expect("the router always answers");
        assert_eq!(
            response.status(),
            StatusCode::PAYLOAD_TOO_LARGE,
            "a streamed body above the bound is refused"
        );
        assert_eq!(identifier_of(&response), "body-bound-test");
        let body = body_json(response).await;
        assert_eq!(body["error"]["code"], "payload_too_large");
        assert_eq!(body["error"]["request_id"], "body-bound-test");
    }

    #[test]
    fn the_identifier_header_is_the_documented_one() {
        assert_eq!(REQUEST_ID_HEADER, "x-request-id");
    }

    #[test]
    fn only_the_documented_characters_are_accepted() {
        for value in ["abc", "A-Z.a_z:0"] {
            let header = value.parse().expect("the test value is a header");
            assert_eq!(
                super::accepted_request_id(&header).as_deref(),
                Some(value),
                "ADR-0098 accepts {value}"
            );
        }
        for value in ["", " ", "a b", "+"] {
            let header = value.parse().expect("the test value is a header");
            assert!(
                super::accepted_request_id(&header).is_none(),
                "ADR-0098 refuses {value:?}"
            );
        }
    }

    #[test]
    fn a_journal_bound_reads_zulu_and_a_signed_offset_as_utc() {
        // ADR-0122, decision 9: the bound is an ISO 8601 date. The offset is
        // applied, because the file stores UTC and the comparison is literal.
        for (raw, expected) in [
            ("2026-10-31T12:00:00Z", "2026-10-31T12:00:00.000Z"),
            ("2026-10-31T12:00:00.123Z", "2026-10-31T12:00:00.123Z"),
            ("2026-10-31T12:00:00+02:00", "2026-10-31T10:00:00.000Z"),
            ("2026-10-31T12:00:00-05:30", "2026-10-31T17:30:00.000Z"),
            ("2026-01-01T01:00:00+02:00", "2025-12-31T23:00:00.000Z"),
        ] {
            assert_eq!(
                super::journal_timestamp(raw, "from").expect("the bound is accepted"),
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn a_malformed_journal_bound_names_its_parameter() {
        for raw in [
            "2026-10-31T12:00:00",
            "2026-10-31T12:00:00+2:00",
            "2026-10-31T12:00:00+24:00",
            "2026-10-31T12:00:00+02:60",
            "2026-10-31T12:00:00+0200",
            "2026-02-30T12:00:00Z",
        ] {
            let failure = super::journal_timestamp(raw, "to").expect_err(raw);
            assert!(failure.message().contains("'to'"), "{raw}: {failure:?}");
        }
    }
}
