// SPDX-License-Identifier: Apache-2.0

//! `POST /v1/indexes/{index}/facets`: the frequent values of `keyword` fields
//! and the two counts of a filtered pool.
//!
//! References: ADR-0154 (decisions 1 to 12) and `docs/05-api.md`, "Facettes".
//! The route is not a search: it encodes nothing, ranks nothing and runs neither
//! the dense nor the lexical branch. It evaluates the filter into the same pool
//! as a search (INV-07), counts its passages, counts its distinct documents
//! through the derived table of the snapshot, and intersects the bitmap of each
//! value with that pool.
//!
//! # Where the work runs
//!
//! The work is CPU-bound, so it runs inside one blocking task that holds a token
//! of the same semaphore as the searches (`[server].max_concurrent_searches`,
//! ADR-0154, decision 8). The token is taken **without waiting**: none free means
//! `503 overloaded` at once, before anything is computed.
//!
//! # Deadline
//!
//! The deadline of `timeout_ms` starts at the acquisition of the token and is
//! checked between two values of a field and during the count of the documents.
//! At the deadline the answer is `504 timeout`, never a truncated result and
//! never a wrong `other` (ADR-0154, decision 8): a partial answer would be read
//! as a complete one.
//!
//! # Observability
//!
//! The call records one `OpEvent` of operation `facets` and nothing else: no
//! `filter_shapes` row, no debug journal row (ADR-0154, decision 9). No field
//! value and no filter value reaches the analytics (INV-13).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::response::{IntoResponse, Response};
use crible_core::config::{FieldType, Schema};
use crible_core::error::{CribleError, ErrorCode};
use crible_core::filter::Filter;
use crible_search::pool::Pool;
use crible_store::filters_index::KeywordField;
use crible_store::generation::Snapshot;
use serde_json::{Map as JsonMap, Value, json};

use crate::AppState;
use crate::analytics::{self, Operation};
use crate::auth::Client;
use crate::error;
use crate::http::RequestId;
use crate::limits;
use crate::search::{as_usize, json_object, optional_unsigned, unknown_index};

/// Members of the facets body of `docs/05-api.md`, "Facettes".
pub(crate) const BODY_FIELDS: [&str; 4] = ["fields", "filter", "limit", "timeout_ms"];

/// Highest number of field names of one call (ADR-0154, decision 2).
pub(crate) const MAX_FIELDS: usize = 10;

/// Default `limit` of one field (ADR-0154, decision 2).
pub(crate) const DEFAULT_LIMIT: u64 = 20;

/// Highest `limit` of one field, a constant of the code (ADR-0154, decision 2).
pub(crate) const MAX_LIMIT: u64 = 100;

/// Message of the `504` this route renders at its own deadline.
///
/// It names the route, not a search: a facet is not a search (ADR-0154,
/// decision 8).
pub(crate) const TIMEOUT_MESSAGE: &str = "the facets request exceeded its deadline (timeout_ms)";

/// Number of iids between two readings of the clock during the document count.
///
/// Reading the clock for every iid would dominate the count of a pool of
/// millions of passages; one reading every 4096 iids still bounds the overshoot
/// at the deadline far below its millisecond granularity.
const CHECK_EVERY: u32 = 4096;

/// `POST /v1/indexes/{index}/facets`: the facets of one index.
///
/// The role `read` is enforced by the layer the route is mounted under
/// (ADR-0095). An unknown index is `404 unknown_index`; a body of the wrong
/// shape or a field outside `keyword`, `filterable` and declared is `400
/// invalid_request`; a filter outside the grammar keeps `400 invalid_filter`.
#[utoipa::path(
    post,
    path = "/v1/indexes/{index}/facets",
    operation_id = "facets_index",
    tag = "facets",
    description = "The frequent values of one to ten `keyword` filterable fields and the counts \
                   of passages and documents of the filtered pool (ADR-0154). The route runs no \
                   search: it encodes nothing and ranks nothing. An unknown field of the body or \
                   a field outside `keyword` is `400 invalid_request`; an invalid filter is `400 \
                   invalid_filter`; an unknown index is `404 unknown_index`; an insufficient role \
                   `403`; a saturated concurrency limit `503` with `Retry-After: 1`; a deadline \
                   `504 timeout`. Role: `read`; `write` and `admin` include it (ADR-0095).",
    params(("index" = String, Path, description = "Name of the index")),
    request_body = crate::openapi::FacetsRequest,
    responses(
        (status = 200, description = "The counts of the pool and the values of each requested field", body = crate::openapi::FacetsResponse),
        (status = 400, description = "Unknown or refused member of the body, value outside its bounds, or a field outside `keyword`, `filterable` and declared", body = crate::openapi::Error),
        (status = 401, description = "Missing, malformed or unknown API key", body = crate::openapi::Error),
        (status = 403, description = "Insufficient role", body = crate::openapi::Error),
        (status = 404, description = "Unknown index", body = crate::openapi::Error),
        (status = 413, description = "Body above 32 Mio (ADR-0102)", body = crate::openapi::Error),
        (status = 500, description = "Unexpected failure", body = crate::openapi::Error),
        (status = 503, description = "Search limit reached, server draining, or the fixed message \"the server is starting\" before the registry is published (ADR-0146)", body = crate::openapi::Error),
        (status = 504, description = "`timeout_ms` reached", body = crate::openapi::Error)
    )
)]
pub async fn facets(
    State(state): State<Arc<AppState>>,
    Extension(identifier): Extension<RequestId>,
    Extension(client): Extension<Client>,
    Path(index): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    // The total of the event covers the whole route, refusals included
    // (ADR-0117, decision 5).
    let started = Instant::now();
    let answer = inner(&state, &index, &body, &identifier.0).await;
    analytics::record_operation(&state, Operation::Facets, &index, &client, started, &answer);
    answer
}

/// The work of [`facets`], from the body to the response.
async fn inner(state: &Arc<AppState>, index: &str, body: &Value, request_id: &str) -> Response {
    let request = match FacetsRequest::parse(body) {
        Ok(request) => request,
        Err(failure) => return error::rendered(&failure, request_id),
    };
    // A fault of an explicit `timeout_ms` is a `400` before the registry is
    // read (ADR-0110, decision 7; ADR-0146, decision 3).
    if let Some(milliseconds) = request.timeout_ms
        && let Err(failure) = limits::bounded(milliseconds)
    {
        return error::rendered(&failure, request_id);
    }
    let handle = match state.index(index) {
        Ok(Some(handle)) => handle,
        Ok(None) => return error::rendered(&unknown_index(index), request_id),
        Err(failure) => return error::rendered(&failure, request_id),
    };
    let published = match handle.published() {
        Ok(published) => published,
        Err(failure) => return error::rendered(&failure, request_id),
    };
    // The acceptance of a field needs the published schema, so it follows the
    // registry access (ADR-0154, decision 7).
    if let Err(failure) = check_fields(&published.config.schema, &request.fields) {
        return error::rendered(&failure, request_id);
    }
    let filter = match &request.filter {
        Some(filter) => match filter.validate(&published.config.schema) {
            Ok(()) => filter.clone(),
            Err(failure) => return error::rendered(&failure, request_id),
        },
        None => Filter::And(Vec::new()),
    };
    let default_timeout_ms = match state.default_timeout_ms() {
        Ok(value) => value,
        Err(failure) => return error::rendered(&failure, request_id),
    };
    let deadline = match limits::timeout(request.timeout_ms, default_timeout_ms) {
        Ok(deadline) => deadline,
        Err(failure) => return error::rendered(&failure, request_id),
    };
    let limit = match state.search_limit() {
        Ok(limit) => limit,
        Err(failure) => return error::rendered(&failure, request_id),
    };
    let snapshot = Arc::clone(&published.snapshot);
    let fields = request.fields;
    let limit_value = request.limit;
    // The deadline starts here, at the acquisition of the token (ADR-0102,
    // decision 3; ADR-0154, decision 8).
    let started = Instant::now();
    let attempt = limit
        .run_blocking(deadline, move || {
            // A test holds the work here to prove that the deadline of a facets
            // request answers `504` with no truncated result (ADR-0154,
            // decision 8).
            #[cfg(feature = "failpoints")]
            crate::barriers::wait_facets();
            compute(
                &snapshot,
                &fields,
                &filter,
                limit_value,
                Deadline {
                    started,
                    budget: deadline,
                },
            )
        })
        .await;
    let answer = match attempt {
        Ok(Ok(answer)) => answer,
        Ok(Err(failure)) => return error::rendered(&failure, request_id),
        // The token was held, and the computation keeps running with it: the
        // deadline stops the wait, never the work (ADR-0102, decision 3). Only
        // the message of a deadline is replaced; an `overloaded` keeps the one
        // of `limits::OVERLOADED_MESSAGE`, with its `Retry-After`.
        Err(failure) => {
            let failure = if failure.code() == ErrorCode::Timeout {
                CribleError::new(ErrorCode::Timeout, TIMEOUT_MESSAGE)
            } else {
                failure
            };
            return error::rendered(&failure, request_id);
        }
    };
    (axum::http::StatusCode::OK, Json(answer.render())).into_response()
}

/// Parsed body of `docs/05-api.md`, "Facettes".
#[derive(Debug, Clone)]
struct FacetsRequest {
    /// Field names, from one to [`MAX_FIELDS`], distinct.
    fields: Vec<String>,
    /// Filter of the pool; `None` is the empty conjunction.
    filter: Option<Filter>,
    /// Values rendered per field, from 1 to [`MAX_LIMIT`].
    limit: usize,
    /// Deadline of the request, bounded by [`limits::bounded`].
    timeout_ms: Option<u64>,
}

impl FacetsRequest {
    /// Parses the closed body, naming the JSON path of every fault.
    fn parse(body: &Value) -> Result<Self, CribleError> {
        let object = json_object(body, "$")?;
        reject_unknown(object)?;
        let fields = parse_fields(object)?;
        let filter = match object.get("filter") {
            None | Some(Value::Null) => None,
            Some(value) => {
                let _ = json_object(value, "$.filter")?;
                // The path of the filter in the body, so `details.path` names
                // `$.filter.…` (ADR-0101, decision 3).
                Some(Filter::from_json_str_at(&value.to_string(), "$.filter")?)
            }
        };
        let limit = match optional_unsigned(object, "$", "limit")? {
            None => DEFAULT_LIMIT,
            Some(value) if (1..=MAX_LIMIT).contains(&value) => value,
            Some(value) => {
                return Err(error::invalid(format!(
                    "the field '$.limit' must be between 1 and {MAX_LIMIT}, not {value}"
                )));
            }
        };
        let timeout_ms = optional_unsigned(object, "$", "timeout_ms")?;
        Ok(Self {
            fields,
            filter,
            limit: as_usize(limit),
            timeout_ms,
        })
    }
}

/// Refuses a key outside [`BODY_FIELDS`], naming its JSON path.
fn reject_unknown(object: &JsonMap<String, Value>) -> Result<(), CribleError> {
    for key in object.keys() {
        if !BODY_FIELDS.contains(&key.as_str()) {
            return Err(error::invalid(format!(
                "unknown field '$.{key}' in the facets request"
            )));
        }
    }
    Ok(())
}

/// Parses `fields`: from one to [`MAX_FIELDS`] distinct textual names.
fn parse_fields(object: &JsonMap<String, Value>) -> Result<Vec<String>, CribleError> {
    let Some(value) = object.get("fields") else {
        return Err(error::invalid("the field '$.fields' is mandatory"));
    };
    let items = value
        .as_array()
        .ok_or_else(|| error::invalid("the field '$.fields' must be an array of field names"))?;
    if items.is_empty() {
        return Err(error::invalid(
            "the field '$.fields' must name at least one field",
        ));
    }
    if items.len() > MAX_FIELDS {
        return Err(error::invalid(format!(
            "the field '$.fields' names {} fields, above the bound of {MAX_FIELDS}",
            items.len()
        )));
    }
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut names = Vec::with_capacity(items.len());
    for (position, item) in items.iter().enumerate() {
        let Value::String(name) = item else {
            return Err(error::invalid(format!(
                "the field '$.fields[{position}]' must be a string"
            )));
        };
        if !seen.insert(name.as_str()) {
            return Err(error::invalid(format!(
                "the field '$.fields' names '{name}' twice"
            )));
        }
        names.push(name.clone());
    }
    Ok(names)
}

/// Accepts a field declared `keyword` and `filterable` by the schema.
///
/// A field of another type is refused with its type in the message, because the
/// caller has to know that numeric and date facets are outside the 1.1
/// (ADR-0154, decision 3).
fn check_fields(schema: &Schema, names: &[String]) -> Result<(), CribleError> {
    for name in names {
        let (spec, _) = schema.field(name).ok_or_else(|| {
            error::invalid(format!(
                "the field '{name}' is not declared by the schema of the index"
            ))
        })?;
        if !spec.filterable {
            return Err(error::invalid(format!(
                "the field '{name}' is not filterable"
            )));
        }
        if spec.field_type != FieldType::Keyword {
            return Err(error::invalid(format!(
                "the field '{name}' is a {} field; the facets of 1.1 accept `keyword` fields only",
                spec.field_type.as_str()
            )));
        }
    }
    Ok(())
}

/// Deadline of the blocking task, read between two units of work.
#[derive(Debug, Clone, Copy)]
struct Deadline {
    /// Instant the token was acquired.
    started: Instant,
    /// Budget of `timeout_ms`.
    budget: Duration,
}

impl Deadline {
    /// Refuses the request when the budget is spent.
    fn check(&self) -> Result<(), CribleError> {
        if self.started.elapsed() >= self.budget {
            return Err(CribleError::new(ErrorCode::Timeout, TIMEOUT_MESSAGE));
        }
        Ok(())
    }
}

/// The counts and the values one call renders.
#[derive(Debug, Clone)]
struct FacetsAnswer {
    /// Generation of the snapshot that served the whole request.
    generation: u64,
    /// Passages of the pool.
    passages: u64,
    /// Distinct documents of the pool.
    documents: u64,
    /// One entry per requested field, in the order of the request.
    fields: Vec<(String, Facet)>,
}

/// The rendered values of one field and the count they leave out.
#[derive(Debug, Clone, Default)]
struct Facet {
    /// `(value, passages)` pairs, count descending then value ascending.
    values: Vec<(String, u64)>,
    /// Pool passages carrying a value, minus the rendered counts.
    other: u64,
}

impl FacetsAnswer {
    /// Renders the body of the `200` of `docs/05-api.md`, "Facettes".
    fn render(self) -> Value {
        let mut fields = JsonMap::new();
        for (name, facet) in self.fields {
            let values: Vec<Value> = facet
                .values
                .into_iter()
                .map(|(value, count)| json!({ "value": value, "count": count }))
                .collect();
            fields.insert(name, json!({ "values": values, "other": facet.other }));
        }
        json!({
            "generation": self.generation,
            "count": { "passages": self.passages, "documents": self.documents },
            "fields": fields,
        })
    }
}

/// Computes the answer of one snapshot, on the blocking thread.
fn compute(
    snapshot: &Snapshot,
    fields: &[String],
    filter: &Filter,
    limit: usize,
    deadline: Deadline,
) -> Result<FacetsAnswer, CribleError> {
    // The same pool as a search with the same filter (INV-07).
    let pool = Pool::evaluate(snapshot.filters(), filter)?;
    deadline.check()?;
    let passages = u64::from(pool.len());
    let mut ticks: u32 = 0;
    let documents = snapshot.documents_ordinals().count_documents(
        pool.bitmap(),
        // One reading of the clock every [`CHECK_EVERY`] iids: see the constant.
        || {
            ticks = ticks.wrapping_add(1);
            if ticks.is_multiple_of(CHECK_EVERY) {
                deadline.check()
            } else {
                Ok(())
            }
        },
    )?;
    let mut rendered = Vec::with_capacity(fields.len());
    for name in fields {
        // The schema accepted the field, and `FilterIndex::empty` builds one
        // structure per declared `keyword` field: a miss here is a divergence.
        let field = snapshot.filters().keyword(name).ok_or_else(|| {
            error::internal(format!(
                "the filter structures hold no keyword field '{name}'"
            ))
        })?;
        rendered.push((name.clone(), facet_of(field, &pool, limit, deadline)?));
        deadline.check()?;
    }
    Ok(FacetsAnswer {
        generation: snapshot.generation().get(),
        passages,
        documents,
        fields: rendered,
    })
}

/// Counts the values of one field under `pool`.
fn facet_of(
    field: &KeywordField,
    pool: &Pool,
    limit: usize,
    deadline: Deadline,
) -> Result<Facet, CribleError> {
    let mut counted: Vec<(String, u64)> = Vec::new();
    for (value, _) in field.values() {
        let count = field.count_in(value, pool.bitmap());
        // A value no pool passage carries enters neither `values` nor `other`
        // (ADR-0154, decision 4).
        if count > 0 {
            counted.push((value.to_owned(), count));
        }
        deadline.check()?;
    }
    // Count descending, then value ascending in UTF-8 bytes: a total order, so
    // two identical calls render the same answer (ADR-0154, decision 4).
    counted.sort_unstable_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    counted.truncate(limit);
    let rendered: u64 = counted.iter().map(|(_, count)| count).sum();
    let present = field.present().intersection_len(pool.bitmap());
    Ok(Facet {
        values: counted,
        other: present.saturating_sub(rendered),
    })
}

#[cfg(test)]
mod tests {
    use super::{FacetsRequest, MAX_FIELDS, MAX_LIMIT, parse_fields};
    use crible_core::error::ErrorCode;
    use serde_json::json;

    /// Parses a body and returns the error code of a refusal.
    fn refused(body: serde_json::Value) -> ErrorCode {
        FacetsRequest::parse(&body)
            .expect_err("the body is refused")
            .code()
    }

    #[test]
    fn the_body_is_closed_and_carries_only_its_four_members() {
        for unknown in ["query", "vector", "mode", "k", "return", "debug"] {
            let body = json!({ "fields": ["fonds"], unknown: "x" });
            let failure = FacetsRequest::parse(&body).expect_err("the key is unknown");
            assert!(
                failure.message().contains(&format!("$.{unknown}")),
                "{}",
                failure.message()
            );
        }
        let request =
            FacetsRequest::parse(&json!({ "fields": ["fonds"], "limit": 5 })).expect("valid");
        assert_eq!(request.limit, 5);
        assert_eq!(request.timeout_ms, None);
    }

    #[test]
    fn the_fields_are_one_to_ten_distinct_textual_names() {
        assert!(FacetsRequest::parse(&json!({ "limit": 20 })).is_err());
        assert_eq!(refused(json!({ "fields": [] })), ErrorCode::InvalidRequest);
        assert_eq!(
            refused(json!({ "fields": "fonds" })),
            ErrorCode::InvalidRequest
        );
        assert_eq!(refused(json!({ "fields": [1] })), ErrorCode::InvalidRequest);
        assert_eq!(
            refused(json!({ "fields": ["fonds", "fonds"] })),
            ErrorCode::InvalidRequest
        );
        let eleven: Vec<String> = (1..=11).map(|number| format!("f{number}")).collect();
        assert_eq!(
            refused(json!({ "fields": eleven })),
            ErrorCode::InvalidRequest
        );
        let ten: Vec<String> = (1..=10).map(|number| format!("f{number}")).collect();
        let request = FacetsRequest::parse(&json!({ "fields": ten })).expect("ten names");
        assert_eq!(request.fields.len(), MAX_FIELDS);
    }

    #[test]
    fn the_limit_is_one_to_one_hundred() {
        for limit in [1_u64, MAX_LIMIT] {
            let request = FacetsRequest::parse(&json!({ "fields": ["fonds"], "limit": limit }))
                .expect("inside the bounds");
            assert_eq!(request.limit as u64, limit);
        }
        for limit in [0_u64, MAX_LIMIT + 1] {
            assert_eq!(
                refused(json!({ "fields": ["fonds"], "limit": limit })),
                ErrorCode::InvalidRequest
            );
        }
    }

    #[test]
    fn a_duplicate_names_the_faulty_field_in_its_message() {
        let object = json!({ "fields": ["fonds", "etat", "fonds"] });
        let object = object.as_object().expect("an object");
        let failure = parse_fields(object).expect_err("the name is duplicated");
        assert!(failure.message().contains("fonds"), "{}", failure.message());
    }
}
