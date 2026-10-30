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
use crible_search::group::Grouping;
use crible_search::pipeline::{Mode, RankedHit, ScoreType};
use serde_json::{Map, Value, json};

use crate::jobs::Published;
use crate::search::{
    self, IndexOutcome, IndexParams, Inputs, MULTI_BODY_FIELDS, ReturnSpec, SearchRequest,
    json_object, optional_string, reject_unknown,
};
use crate::{AppState, error, http::RequestId, limits};

/// Constant of the reciprocal rank fusion, shared with the fusion of the two
/// branches of one index (`docs/03-recherche.md:101`, ADR-0110, decision 9).
const RRF_CONSTANT: f32 = 60.0;

/// `POST /v1/search`: one search over the indexes of `targets`.
///
/// The route is mounted under the `read` role: `write` and `admin` include it,
/// so the three roles reach it (ADR-0095).
pub async fn search(
    State(state): State<Arc<AppState>>,
    Extension(identifier): Extension<RequestId>,
    Json(body): Json<Value>,
) -> Response {
    let plan = match MultiRequest::parse(&body).and_then(|request| validate(&state, request)) {
        Ok(plan) => plan,
        Err(failure) => return error::rendered(&failure, &identifier.0),
    };
    let limit = match state.search_limit() {
        Ok(limit) => limit,
        Err(failure) => return error::rendered(&failure, &identifier.0),
    };

    let holder = Arc::clone(&state);
    let outcome = limit
        .run_blocking(plan.deadline, move || {
            let started = Instant::now();
            // A test holds the work here to prove the single token of the whole
            // request (ADR-0110, decision 8; ADR-0102, decision 2).
            #[cfg(feature = "failpoints")]
            crate::barriers::wait("search-multi");
            run_multi(&holder, plan).map(|outcome| outcome.with_start(started))
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
    /// Deadline of the whole request (ADR-0102, decision 3).
    deadline: Duration,
}

/// One target of the request, with its published state and its filter.
struct ResolvedTarget {
    /// Name of the index, as the request names it.
    index: String,
    /// Triplet `(config, snapshot, summary)` of the index.
    published: Published,
    /// Filter of this target; the empty filter matches every living passage.
    filter: Filter,
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
        targets.push(TargetRequest { index, filter });
    }
    Ok(targets)
}

/// Applies the order of ADR-0110, decision 7, and builds the plan.
fn validate(state: &AppState, request: MultiRequest) -> Result<Plan, CribleError> {
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
        });
    }
    // (c) `return.fields` against the whole set of targets, once their schemas
    // are known: `"all"` needs no check.
    if common.selection.names().is_some() {
        check_return_names(&common.selection, &targets)?;
    }
    let selection = common.selection.selection();

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
        deadline,
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
        .model(&first.published.config.model)?
        .config
        .fingerprint();
    for target in &targets[1..] {
        let access = state.model(&target.published.config.model)?;
        if access.config.fingerprint() != fingerprint {
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
fn run_multi(state: &AppState, plan: Plan) -> Result<MultiOutcome, CribleError> {
    let Plan {
        targets,
        mode,
        inputs,
        grouping,
        fusion,
        selection,
        k,
        exact,
        ..
    } = plan;

    // The single encoding point of the request, whatever the number of targets
    // (ADR-0110, decisions 8 and 11).
    let first = targets
        .first()
        .ok_or_else(|| error::invalid("'$.targets' must name at least one index"))?;
    let prepared = inputs.prepare(|query| search::encode_query(state, &first.published, query))?;
    let vector = match &prepared.vector {
        Some(vector) => Some(search::check_vector(
            state,
            &first.published,
            vector.clone(),
        )?),
        None => None,
    };

    let mut outcomes: Vec<(&ResolvedTarget, IndexOutcome)> = Vec::with_capacity(targets.len());
    for target in &targets {
        let per_index_fusion = fusion.unwrap_or(target.published.config.search.fusion);
        let outcome = search::search_index(
            state,
            &target.index,
            &target.published,
            &IndexParams {
                mode,
                vector: vector.as_deref(),
                text: prepared.text.as_deref(),
                filter: &target.filter,
                grouping: &grouping,
                fusion: per_index_fusion,
                k,
                exact,
            },
        )?;
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
    let entries = fuse(outcomes, mode, &grouping, k);
    let results = render_entries(entries, &targets, score_type, &selection)?;
    Ok(MultiOutcome {
        results,
        targets: summaries,
        truncated_query: prepared.truncated_query,
        took_ms: 0.0,
    })
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
    if rrf {
        for (position, unit) in units.iter_mut().enumerate() {
            // The rank is the position of the unit in the final list of its
            // index, from 1; every entry of a document carries its score.
            let rank = f32::from(u16::try_from(position + 1).unwrap_or(u16::MAX));
            let score = 1.0 / (RRF_CONSTANT + rank);
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
) -> Result<Vec<Value>, CribleError> {
    let mut rendered = Vec::with_capacity(entries.len());
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
        rendered.push(value);
    }
    Ok(rendered)
}

/// One answer of the multi-index search, before its envelope.
struct MultiOutcome {
    results: Vec<Value>,
    targets: Vec<Value>,
    truncated_query: bool,
    took_ms: f64,
}

impl MultiOutcome {
    /// Sets the duration measured from the token acquisition.
    fn with_start(mut self, started: Instant) -> Self {
        self.took_ms = started.elapsed().as_secs_f64() * 1_000.0;
        self
    }

    /// Renders the body of the `200`, with the request identifier of ADR-0098.
    fn render(self, request_id: &str) -> Value {
        json!({
            "results": self.results,
            "targets": self.targets,
            "took_ms": self.took_ms,
            "truncated_query": self.truncated_query,
            "request_id": request_id,
        })
    }
}
