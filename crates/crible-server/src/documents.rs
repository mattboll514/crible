// SPDX-License-Identifier: Apache-2.0

//! Write and read routes of the documents of an index (ADR-0107).
//!
//! Three routes: `POST /v1/indexes/{index}/documents` (an online batch),
//! `PATCH /v1/indexes/{index}/documents` (a metadata patch) and
//! `GET /v1/indexes/{index}/documents/{doc_id}` (a document, its fields and its
//! live passages).
//!
//! Only the **shape** of a body is refused before the job is queued
//! (ADR-0107, decision 3): an unreadable body, an unknown key, a mode other
//! than `upsert`, a `doc_id` both in `documents` and in `deletions`, an entry
//! count above the request limit. Every content fault — `batch_conflict`, an
//! unknown document, a mistyped field, a refused acceptance — is carried by the
//! job, which is the only place where the order of the writer is known.
//!
//! The read runs off the async threads: it opens the `records.sqlite` of the
//! **epoch of the published snapshot** (ADR-0107, decision 6), and filters the
//! passages with the live bitmap of that snapshot, so a concurrent commit in the
//! same epoch cannot add a passage the snapshot does not hold. A vector and an
//! internal id never leave the engine (**INV-01**).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use crible_core::config::{FieldLevel, Schema};
use crible_core::error::{CribleError, ErrorCode};
use crible_core::model::ModelKind;
use crible_core::path::path_component;
use crible_store::generation::epoch_records_path;
use crible_store::jobs::JobKind;
use crible_store::online::{self, MAX_ONLINE_ENTRIES, OnlineDocument, OnlineDocuments};
use crible_store::patch::{self, DocumentPatch, PatchSource};
use crible_store::records::ReadPool;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::AppState;
use crate::error;
use crate::http::{RequestId, accepted};
use crate::jobs::{JobOutcome, log_attach_durations};

/// Body of `POST /v1/indexes/{index}/documents`.
///
/// The envelope refuses unknown members, as `docs/05-api.md` ("Lot en ligne")
/// describes it; each document and each passage is the same type the CLI reads,
/// so the two paths share one format (ADR-0055, ADR-0107).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OnlineBody {
    /// Identifier of the batch, mandatory and chosen by the caller.
    batch_id: String,
    /// Mode of the internal manifest: `upsert` only.
    #[serde(default)]
    mode: crible_core::batch::BatchMode,
    /// Documents to write.
    #[serde(default)]
    documents: Vec<OnlineDocument>,
    /// External identifiers to remove.
    #[serde(default)]
    deletions: Vec<String>,
}

/// Body of `PATCH /v1/indexes/{index}/documents`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PatchBody {
    /// Documents whose fields are replaced.
    documents: Vec<DocumentPatch>,
}

/// `POST /v1/indexes/{index}/documents`: applies an online batch as a job.
///
/// The batch goes through the same internal lot and the same attach as a
/// deposited batch (ADR-0055, ADR-0107, decision 1). A replayed `batch_id` with
/// the same content writes nothing and returns the original report; another
/// content fails the job in `batch_conflict` (decision 3).
#[utoipa::path(
    post,
    path = "/v1/indexes/{index}/documents",
    tag = "documents",
    description = "Apply an online batch as a job. `batch_id` is mandatory; a body with neither \
                   `documents` nor `deletions` is `400 invalid_request`, an unknown key too. A \
                   replay of the same `batch_id` with the same content writes nothing; another \
                   content fails the job in `batch_conflict`. Role: `write`; `admin` includes it \
                   (ADR-0095).",
    params(
        ("index" = String, Path, description = "Name of the index"),
        ("wait" = Option<bool>, Query, description = "Wait for the job under `wait_timeout_ms` (ADR-0067)")
    ),
    request_body = crate::openapi::OnlineBatchRequest,
    responses(
        (status = 200, description = "The job finished within `wait_timeout_ms`", body = crate::openapi::Job),
        (status = 202, description = "The job was accepted", body = crate::openapi::Job),
        (status = 400, description = "Refused body: unknown key, no document nor deletion, mode other than `upsert`", body = crate::openapi::Error),
        (status = 401, description = "Missing, malformed or unknown API key", body = crate::openapi::Error),
        (status = 403, description = "Insufficient role", body = crate::openapi::Error),
        (status = 404, description = "Unknown index", body = crate::openapi::Error),
        (status = 500, description = "Unexpected failure", body = crate::openapi::Error),
        (status = 503, description = "Write queue full, server draining or SQLite lock", body = crate::openapi::Error)
    )
)]
pub async fn post_documents(
    State(state): State<Arc<AppState>>,
    Extension(identifier): Extension<RequestId>,
    AxumPath(index): AxumPath<String>,
    Query(query): Query<HashMap<String, String>>,
    Json(body): Json<Value>,
) -> Response {
    if let Err(failure) = served(&state, &index) {
        return error::rendered(&failure, &identifier.0);
    }
    let body: OnlineBody = match serde_json::from_value(body) {
        Ok(body) => body,
        Err(failure) => {
            return error::rendered(
                &error::invalid(format!(
                    "the body of the online batch is invalid: {failure}"
                )),
                &identifier.0,
            );
        }
    };
    if let Err(failure) = check_online(&body) {
        return error::rendered(&failure, &identifier.0);
    }
    let data_dir = match state.data_root() {
        Some(data) => data.to_path_buf(),
        None => {
            return error::rendered(
                &error::internal("the instance has not loaded its indexes"),
                &identifier.0,
            );
        }
    };
    let holder = Arc::clone(&state);
    let name = index.clone();
    let OnlineBody {
        batch_id,
        documents,
        deletions,
        ..
    } = body;
    let work: crate::jobs::JobWork = Box::new(move |store, progress, writer| {
        let index_config = writer.configuration()?;
        let report = ingest(
            store,
            &holder,
            &name,
            &index_config,
            &data_dir,
            &batch_id,
            &documents,
            &deletions,
        )?;
        log_attach_durations(
            progress.job_id(),
            JobKind::OnlineBatch,
            &name,
            report.durations,
        );
        Ok(JobOutcome {
            generation: Some(report.generation.get()),
            result: Some(json!({ "report": report.to_json() })),
        })
    });
    submit(
        &state,
        &index,
        JobKind::OnlineBatch,
        work,
        wait_requested(&query),
        &identifier,
    )
    .await
}

/// `PATCH /v1/indexes/{index}/documents`: patches the fields of documents.
///
/// The report `{ updated, unchanged, generation }` is kept in memory for the
/// life of the job and never persisted (ADR-0060, ADR-0103). A document unknown
/// or removed fails the whole job in `not_found`, and no document of the body is
/// applied (ADR-0060, ADR-0107).
#[utoipa::path(
    patch,
    path = "/v1/indexes/{index}/documents",
    tag = "documents",
    description = "Patch the document fields of the index without re-encoding. A key outside the \
                   document fields of the schema is `400 invalid_request`; an unknown or removed \
                   document fails the job in `not_found` and no document of the body is applied. \
                   Role: `write`; `admin` includes it (ADR-0095).",
    params(
        ("index" = String, Path, description = "Name of the index"),
        ("wait" = Option<bool>, Query, description = "Wait for the job under `wait_timeout_ms` (ADR-0067)")
    ),
    request_body = crate::openapi::MetadataPatchRequest,
    responses(
        (status = 200, description = "The job finished within `wait_timeout_ms`", body = crate::openapi::Job),
        (status = 202, description = "The job was accepted", body = crate::openapi::Job),
        (status = 400, description = "Unknown key, passage field, or mistyped value", body = crate::openapi::Error),
        (status = 401, description = "Missing, malformed or unknown API key", body = crate::openapi::Error),
        (status = 403, description = "Insufficient role", body = crate::openapi::Error),
        (status = 404, description = "Unknown index", body = crate::openapi::Error),
        (status = 500, description = "Unexpected failure", body = crate::openapi::Error),
        (status = 503, description = "Write queue full, server draining or SQLite lock", body = crate::openapi::Error)
    )
)]
pub async fn patch_documents(
    State(state): State<Arc<AppState>>,
    Extension(identifier): Extension<RequestId>,
    AxumPath(index): AxumPath<String>,
    Query(query): Query<HashMap<String, String>>,
    Json(body): Json<Value>,
) -> Response {
    if let Err(failure) = served(&state, &index) {
        return error::rendered(&failure, &identifier.0);
    }
    let body: PatchBody = match serde_json::from_value(body) {
        Ok(body) => body,
        Err(failure) => {
            return error::rendered(
                &error::invalid(format!("the body of the patch is invalid: {failure}")),
                &identifier.0,
            );
        }
    };
    if body.documents.len() > patch::MAX_ENTRIES {
        return error::rendered(
            &CribleError::new(
                ErrorCode::PayloadTooLarge,
                format!(
                    "the patch holds {} documents, above the {} documents of one request \
                     (docs/05-api.md, \"Patch de métadonnées\")",
                    body.documents.len(),
                    patch::MAX_ENTRIES
                ),
            ),
            &identifier.0,
        );
    }
    let patches = body.documents;
    let work: crate::jobs::JobWork = Box::new(move |store, _progress, writer| {
        let index_config = writer.configuration()?;
        let report = patch::apply(store, &index_config.schema, PatchSource::Body, &patches)?;
        Ok(JobOutcome {
            generation: Some(report.generation.get()),
            result: Some(json!({
                "report": {
                    "updated": report.updated,
                    "unchanged": report.unchanged,
                    "generation": report.generation.get(),
                }
            })),
        })
    });
    submit(
        &state,
        &index,
        JobKind::PatchDocuments,
        work,
        wait_requested(&query),
        &identifier,
    )
    .await
}

/// `GET /v1/indexes/{index}/documents/{doc_id}`: one document of the published
/// generation.
///
/// `?text=true` adds the text of each passage; without it, a passage carries its
/// `ord` and its fields only (ADR-0107, decision 6). Only the fields the schema
/// declares `returned` are rendered.
#[utoipa::path(
    get,
    path = "/v1/indexes/{index}/documents/{doc_id}",
    tag = "documents",
    description = "One document, its fields and its live passages, read in the base of the \
                   epoch of the published snapshot; `?text=true` adds the text of each passage. \
                   No vector nor internal id ever appears. Role: `read`; `write` and `admin` \
                   include it (ADR-0095).",
    params(
        ("index" = String, Path, description = "Name of the index"),
        ("doc_id" = String, Path, description = "External identifier of the document"),
        ("text" = Option<bool>, Query, description = "Include the text of each passage")
    ),
    responses(
        (status = 200, description = "The document and its live passages", body = crate::openapi::DocumentDetail),
        (status = 401, description = "Missing, malformed or unknown API key", body = crate::openapi::Error),
        (status = 403, description = "Insufficient role", body = crate::openapi::Error),
        (status = 404, description = "Unknown index, or unknown or removed document", body = crate::openapi::Error),
        (status = 500, description = "Unexpected failure", body = crate::openapi::Error)
    )
)]
pub async fn get_document(
    State(state): State<Arc<AppState>>,
    Extension(identifier): Extension<RequestId>,
    AxumPath((index, doc_id)): AxumPath<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let include_text = query.get("text").is_some_and(|value| value == "true");
    let holder = Arc::clone(&state);
    match blocking(move || document(&holder, &index, &doc_id, include_text)).await {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(failure) => error::rendered(&failure, &identifier.0),
    }
}

/// Checks that the index is served, before any body is read.
fn served(state: &AppState, index: &str) -> Result<(), CribleError> {
    match state.index(index)? {
        Some(_) => Ok(()),
        None => Err(CribleError::new(
            ErrorCode::UnknownIndex,
            format!("unknown index '{index}'"),
        )),
    }
}

/// Checks the shape of an online body, before it is queued (ADR-0107,
/// decision 3).
fn check_online(body: &OnlineBody) -> Result<(), CribleError> {
    // A lot that changes nothing would commit an empty generation: the file
    // path refuses an empty file (`online::survey_file`), and the route refuses
    // an empty body the same way (complement of 2026-10-29 to ADR-0107,
    // decision 1). A body with `deletions` only stays valid.
    if body.documents.is_empty() && body.deletions.is_empty() {
        return Err(error::invalid(
            "the online lot carries neither 'documents' nor 'deletions'; a lot that changes nothing \
             is not attached (ADR-0107)",
        ));
    }
    if body.mode != crible_core::batch::BatchMode::Upsert {
        return Err(error::invalid(format!(
            "the online lot accepts mode 'upsert' only, got '{}': at most 10 000 entries per \
             request make 'replace_all' unusable in line (ADR-0055)",
            body.mode.as_str()
        )));
    }
    path_component(&body.batch_id, "batch_id")?;
    let entries = body
        .documents
        .iter()
        .map(|document| u64::try_from(document.passages.len()).unwrap_or(u64::MAX))
        .fold(0_u64, u64::saturating_add)
        .saturating_add(u64::try_from(body.deletions.len()).unwrap_or(u64::MAX));
    if entries > MAX_ONLINE_ENTRIES {
        return Err(CribleError::new(
            ErrorCode::PayloadTooLarge,
            format!(
                "the online lot holds {entries} entries, above the {MAX_ONLINE_ENTRIES} entries \
                 of one request; passages described and deletions count together (ADR-0107)"
            ),
        ));
    }
    let documents: HashSet<&str> = body
        .documents
        .iter()
        .map(|document| document.doc_id.as_str())
        .collect();
    for deletion in &body.deletions {
        if documents.contains(deletion.as_str()) {
            return Err(error::invalid(format!(
                "the doc_id '{deletion}' is declared both in the documents and in the deletions \
                 of the online lot; a batch cannot add and remove one document (ADR-0040)"
            )));
        }
    }
    Ok(())
}

/// Runs one online batch on the writer of the index.
///
/// The model is resolved here, not at submission: its configuration is read once
/// and an `onnx` graph is loaded on the blocking thread of the job (ADR-0054).
#[allow(clippy::too_many_arguments)]
fn ingest(
    store: &mut crible_store::generation::GenerationStore,
    state: &AppState,
    index: &str,
    index_config: &crible_core::config::IndexConfig,
    data_dir: &Path,
    batch_id: &str,
    documents: &[OnlineDocument],
    deletions: &[String],
) -> Result<crible_store::attach::AttachReport, CribleError> {
    let model = state.model(&index_config.model)?;
    let token_counter = match model.config.kind {
        ModelKind::Onnx => Some(model.token_counter()?),
        ModelKind::External => None,
    };
    let encoder = || model.encoder();
    online::ingest_documents(
        store,
        &OnlineDocuments {
            batch_id,
            documents,
            deletions,
            data_dir,
            index_config,
            model_config: &model.config,
            token_counter: token_counter.as_ref(),
            document_encoder: Some(&encoder),
        },
    )
    .map_err(|failure| named(failure, index))
}

/// Prefixes the message of `failure` with the index, keeping its code.
fn named(failure: CribleError, index: &str) -> CribleError {
    CribleError::new(
        failure.code(),
        format!("index '{index}': {}", failure.message()),
    )
}

/// Reads one document in the epoch of the published snapshot.
fn document(
    state: &AppState,
    index: &str,
    doc_id: &str,
    include_text: bool,
) -> Result<Value, CribleError> {
    let handle = state.index(index)?.ok_or_else(|| {
        CribleError::new(ErrorCode::UnknownIndex, format!("unknown index '{index}'"))
    })?;
    let published = handle.published()?;
    let generation = published.snapshot.generation();
    let schema = &published.config.schema;
    let document_fields = returned_fields(schema, FieldLevel::Document);
    let passage_fields = returned_fields(schema, FieldLevel::Passage);
    let path = epoch_records_path(&state.index_directory(index)?, published.snapshot.epoch());
    // One connection per request, never a pool: the read route opens the
    // `records.sqlite` of one epoch for one document, closes it with the pool,
    // and the process never holds a connection it does not use. The cost is one
    // open per GET and it is measured in P7 (MES-04), where a pool per epoch can
    // replace this line if the measurement asks for it.
    let pool = ReadPool::new(&path, 1)?;
    let Some(record) = pool.read_document(doc_id)? else {
        return Err(document_not_found());
    };
    // A row written after the snapshot describes a generation the request did
    // not see. The epoch database is shared by every commit of the epoch, so the
    // generation is checked before the row is rendered.
    if record.updated_generation > generation {
        return Err(document_not_found());
    }
    let fields = visible(&record.fields, &document_fields);
    let live = published.snapshot.filters().live();
    let mut passages = Vec::new();
    for passage in pool.passages_of_document(doc_id)? {
        if !live.contains(passage.iid.get()) {
            continue;
        }
        let fields = visible(&passage.fields, &passage_fields);
        let entry = if include_text {
            json!({ "ord": passage.ord, "fields": fields, "text": passage.text })
        } else {
            json!({ "ord": passage.ord, "fields": fields })
        };
        passages.push(entry);
    }
    Ok(json!({
        "doc_id": record.doc_id,
        "generation": generation.get(),
        "doc_hash": record.doc_hash,
        "fields": fields,
        "passages": passages,
    }))
}

/// Names of the schema fields of `level` the API may render.
fn returned_fields(schema: &Schema, level: FieldLevel) -> BTreeSet<String> {
    schema
        .fields()
        .filter(|(_, field_level, spec)| *field_level == level && spec.returned)
        .map(|(name, _, _)| name.to_owned())
        .collect()
}

/// Keeps the members of `fields` whose name is `returned`, and no other.
fn visible(fields: &Value, returned: &BTreeSet<String>) -> Value {
    let stored = fields.as_object().cloned().unwrap_or_default();
    let mut rendered = Map::new();
    for (name, value) in stored {
        if returned.contains(&name) {
            rendered.insert(name, value);
        }
    }
    Value::Object(rendered)
}

/// The `404 not_found` of a document unknown at the generation of the snapshot.
fn document_not_found() -> CribleError {
    CribleError::new(ErrorCode::NotFound, "document not found")
}

/// Queues `work` and renders the answer of the mutation.
async fn submit(
    state: &Arc<AppState>,
    index: &str,
    kind: JobKind,
    work: crate::jobs::JobWork,
    wait: bool,
    identifier: &RequestId,
) -> Response {
    match state.submit_job(index, kind, work).await {
        Ok(job) => accepted(state, job, wait).await,
        Err(failure) => error::rendered(&failure, &identifier.0),
    }
}

/// Runs `work` on a blocking thread, so the async threads keep serving.
async fn blocking<T, F>(work: F) -> Result<T, CribleError>
where
    F: FnOnce() -> Result<T, CribleError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work).await.map_err(|join| {
        error::internal(format!(
            "a blocking task of a document route failed: {join}"
        ))
    })?
}

/// Whether the request asked for `?wait=true` (ADR-0067).
fn wait_requested(query: &HashMap<String, String>) -> bool {
    query.get("wait").is_some_and(|value| value == "true")
}
