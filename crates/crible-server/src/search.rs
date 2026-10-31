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
//! `invalid_filter` code). `debug` and `explain_ids` are named as not supported
//! yet: P7 opens them (`REPONSE-DEBUG`, `EXPLIQUER-DOCUMENTS`). No field is
//! ignored in silence. A query text or a filter value never reaches a log
//! (AGENTS.md, "Confidentialité").

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Instant;

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::response::{IntoResponse, Response};
use crible_core::config::{FusionConfig, check_alpha};
use crible_core::error::{CribleError, ErrorCode};
use crible_core::filter::Filter;
use crible_core::model::ModelKind;
use crible_search::api::{self, ReturnSelection};
use crible_search::dense::exact::DenseSearcher;
use crible_search::group::Grouping;
use crible_search::pipeline::{
    self, Answer, Mode, RankedHit, Request as PipelineRequest, ScoreType,
};
use crible_search::pool::Pool;
use crible_store::generation::{epoch_records_path, epoch_segments_directory};
use serde_json::{Map, Value, json};

use crate::AppState;
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
    tag = "search",
    description = "Search one index, dense, lexical or hybrid, and fuse the two rankings. An \
                   unknown field of the body is `400 invalid_request`; an unknown index is `404 \
                   unknown_index`; an insufficient role `403`; a saturated concurrency limit \
                   `503` with `Retry-After: 1`; a deadline `504 timeout`. `debug` and \
                   `explain_ids` are refused until P7. Role: `read`; `write` and `admin` include \
                   it (ADR-0095).",
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
    Path(index): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let handle = match state.index(&index) {
        Ok(Some(handle)) => handle,
        Ok(None) => return error::rendered(&unknown_index(&index), &identifier.0),
        Err(failure) => return error::rendered(&failure, &identifier.0),
    };
    let request = match SearchRequest::parse(&body) {
        Ok(request) => request,
        Err(failure) => return error::rendered(&failure, &identifier.0),
    };
    let published = match handle.published() {
        Ok(published) => published,
        Err(failure) => return error::rendered(&failure, &identifier.0),
    };
    // `timeout_ms` is read and bounded before any other work, so a fault of the
    // deadline never hides behind a missing input (ADR-0102, decision 3).
    let default_timeout_ms = match state.default_timeout_ms() {
        Ok(value) => value,
        Err(failure) => return error::rendered(&failure, &identifier.0),
    };
    let deadline = match limits::timeout(request.timeout_ms, default_timeout_ms) {
        Ok(deadline) => deadline,
        Err(failure) => return error::rendered(&failure, &identifier.0),
    };
    let mode = request
        .mode
        .unwrap_or_else(|| Mode::default_for(published.config.lexical.enabled));
    if mode.uses_lexical() && !published.config.lexical.enabled {
        return error::rendered(
            &error::invalid("lexical branch is disabled for this index"),
            &identifier.0,
        );
    }
    let inputs = match request.inputs(mode) {
        Ok(inputs) => inputs,
        Err(failure) => return error::rendered(&failure, &identifier.0),
    };
    // A permanent refusal of ADR-0109, decision 4, before the token: an
    // `external` model cannot encode, so a mode that would encode needs its
    // `vector` (ADR-0110, decision 7).
    if let Err(failure) = check_model_inputs(&state, &published, mode, &inputs) {
        return error::rendered(&failure, &identifier.0);
    }
    let fusion = request.fusion.unwrap_or(published.config.search.fusion);
    let grouping = match api::grouping(
        request.by_document,
        request.passages_per_document,
        request.dedup_content,
    ) {
        Ok(grouping) => grouping,
        Err(failure) => return error::rendered(&failure, &identifier.0),
    };
    let selection = match request.selection.resolve(&published.config.schema) {
        Ok(selection) => selection,
        Err(failure) => return error::rendered(&failure, &identifier.0),
    };
    let filter = match request.filter {
        Some(filter) => match filter.validate(&published.config.schema) {
            Ok(()) => filter,
            Err(failure) => return error::rendered(&failure, &identifier.0),
        },
        None => Filter::And(Vec::new()),
    };
    let limit = match state.search_limit() {
        Ok(limit) => limit,
        Err(failure) => return error::rendered(&failure, &identifier.0),
    };

    let holder = Arc::clone(&state);
    let name = index.clone();
    // `took_ms` starts at the token acquisition (ADR-0109, decision 2). The
    // token is taken without waiting, so this instant is that acquisition.
    let started = Instant::now();
    // A test holds the route here to prove that `took_ms` starts before the
    // token, not at the entry of the blocking task (ADR-0109, decision 2).
    #[cfg(feature = "failpoints")]
    crate::barriers::wait_search_start();
    let outcome = limit
        .run_blocking(deadline, move || {
            // A test holds the work here to prove that a `504` keeps the token
            // until the computation really ends (ADR-0102, decision 3).
            #[cfg(feature = "failpoints")]
            crate::barriers::wait("search");
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
            )
            .map(|outcome| outcome.with_start(started))
        })
        .await;

    match outcome {
        Ok(Ok(outcome)) => (
            axum::http::StatusCode::OK,
            Json(outcome.render(&identifier.0)),
        )
            .into_response(),
        Ok(Err(failure)) => limits::refusal(&failure, &identifier.0),
        Err(failure) => limits::refusal(&failure, &identifier.0),
    }
}

/// Runs the pipeline on the blocking thread, with the token held.
///
/// The caller read `started` just before taking the token, which is the start of
/// the duration `took_ms` reports (ADR-0109, decision 2).
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
) -> Result<SearchOutcome, CribleError> {
    // The encoding point of the server: the closure below runs at most once,
    // even though the pipeline has two branches (ADR-0109, ADR-0051).
    let prepared = inputs.prepare(|query| encode_query(state, published, query))?;
    let vector = match &prepared.vector {
        Some(vector) => Some(check_vector(state, published, vector.clone())?),
        None => None,
    };
    let outcome = search_index(
        state,
        index,
        published,
        &IndexParams {
            mode,
            vector: vector.as_deref(),
            text: prepared.text.as_deref(),
            filter,
            grouping,
            fusion,
            k,
            exact,
        },
    )?;
    let results = api::results_json(
        &Answer {
            score_type: outcome.score_type,
            results: outcome.hits,
        },
        &published.config.schema,
        selection,
    );
    Ok(SearchOutcome {
        results,
        pool_size: outcome.pool_size,
        generation: outcome.generation,
        truncated_query: prepared.truncated_query,
        took_ms: 0.0,
    })
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
}

/// Runs the pipeline of one index on the blocking thread.
pub(crate) fn search_index(
    state: &AppState,
    index: &str,
    published: &Published,
    params: &IndexParams<'_>,
) -> Result<IndexOutcome, CribleError> {
    let loaded = state
        .loaded()
        .ok_or_else(|| error::internal("the instance has not loaded its indexes"))?;
    let snapshot = Arc::clone(&published.snapshot);
    let pool = Pool::evaluate(snapshot.filters(), params.filter)?;
    let index_dir = loaded.index_directory(index);
    let segments_directory = epoch_segments_directory(&index_dir, snapshot.epoch());
    let records_path = epoch_records_path(&index_dir, snapshot.epoch());
    let segments = snapshot.segments().to_vec();
    let scan = loaded.scan_pool();
    let searcher =
        scan.install(|| DenseSearcher::open(&segments_directory, &segments, &records_path))?;
    let answer = scan.install(|| {
        pipeline::search(
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
        )
    })?;
    Ok(IndexOutcome {
        hits: answer.results,
        score_type: answer.score_type,
        pool_size: pool.len(),
        generation: snapshot.generation().get(),
    })
}

/// Refuses a dense mode that would have to encode on an `external` model.
///
/// ADR-0109, decision 4: `dense` on an `external` index requires `vector`, and
/// `hybrid` requires `query` **and** `vector`. The message names the model, and
/// the refusal comes before the concurrency token (ADR-0110, decision 7), so a
/// saturated instance answers `400` rather than `503`.
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
    let access = state.model(&published.config.model)?;
    if access.config.kind != ModelKind::External {
        return Ok(());
    }
    Err(error::invalid(format!(
        "index '{}' uses model '{}' of kind external, which has no encoder; mode {} needs a \
         'vector' in the request",
        published.config.name,
        access.config.name,
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
struct SearchOutcome {
    results: Vec<Value>,
    pool_size: u32,
    generation: u64,
    truncated_query: bool,
    took_ms: f64,
}

impl SearchOutcome {
    /// Sets the duration measured from the token acquisition.
    fn with_start(mut self, started: Instant) -> Self {
        self.took_ms = started.elapsed().as_secs_f64() * 1_000.0;
        self
    }

    /// Renders the body of the `200`, with the request identifier of ADR-0098.
    fn render(self, request_id: &str) -> Value {
        json!({
            "results": self.results,
            "pool_size": self.pool_size,
            "generation": self.generation,
            "took_ms": self.took_ms,
            "truncated_query": self.truncated_query,
            "request_id": request_id,
        })
    }
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
    pub(crate) k: usize,
    pub(crate) fusion: Option<FusionConfig>,
    pub(crate) by_document: bool,
    pub(crate) passages_per_document: usize,
    pub(crate) dedup_content: bool,
    pub(crate) selection: ReturnSpec,
    pub(crate) exact: bool,
    pub(crate) timeout_ms: Option<u64>,
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
        for opened in ["debug", "explain_ids"] {
            if object.contains_key(opened) {
                return Err(error::invalid(format!(
                    "the field '$.{opened}' is not supported yet; it opens in P7"
                )));
            }
        }
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
        let timeout_ms = optional_unsigned(object, "$", "timeout_ms")?;
        Ok(Self {
            query,
            vector,
            mode,
            filter,
            k,
            fusion,
            by_document,
            passages_per_document,
            dedup_content,
            selection,
            exact,
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
    use super::{BODY_FIELDS, SearchRequest};
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
    fn debug_and_explain_are_refused_as_not_supported_yet() {
        for body in [json!({ "debug": true }), json!({ "explain_ids": [] })] {
            let failure = parse(body);
            assert!(
                failure.message().contains("not supported yet"),
                "{}",
                failure.message()
            );
        }
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
