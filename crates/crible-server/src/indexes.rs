// SPDX-License-Identifier: Apache-2.0

//! Administration routes of `/v1/indexes` (ADR-0105).
//!
//! Five routes, all of them described by ADR-0105: the creation at runtime
//! (`POST`, `201` with the detail body), the two reads (the list and the detail),
//! the patch (validated before any queueing, then applied by the writer as a job)
//! and the deletion (with its `confirm` parameter and its quarantine).
//!
//! The heavy work never runs on a tokio thread: the creation hashes a model,
//! loads its session and writes the index; the detail walks the directory tree
//! to measure `sizes.disk_bytes`. Both go through a blocking task
//! (`docs/02-architecture.md`, "Concurrence").
//!
//! The registry, the stores and the job table are the ones of the startup: a
//! created index is served without a restart (ADR-0105, decision 1), and a
//! deleted one leaves its job history behind (decision 6).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use crible_core::config::{IndexConfig, IndexPatch, apply_patch};
use crible_core::error::{CribleError, ErrorCode};
use crible_store::generation::{GenerationStore, rewrite_index_config};
use crible_store::jobs::JobKind;
use serde_json::{Value, json};
use tracing::warn;

use crate::AppState;
use crate::error;
use crate::http::{RequestId, accepted};
use crate::jobs::{IndexSummary, IndexWriter, JobOutcome, JobProgress};
use crate::state::IndexHandle;

/// `POST /v1/indexes`: creates an index and serves it without a restart.
///
/// The configuration is validated by [`IndexConfig::from_value`], the same path
/// as the CLI (`docs/04`, ADR-0105 decision 2). A name already taken, an absent
/// model and a refused configuration render `400 invalid_request`; the body of
/// the `201` is the one of `GET /v1/indexes/{index}`.
pub async fn create_index(
    State(state): State<Arc<AppState>>,
    Extension(identifier): Extension<RequestId>,
    Json(body): Json<Value>,
) -> Response {
    let config = match IndexConfig::from_value(&body) {
        Ok(config) => config,
        Err(failure) => return error::rendered(&failure, &identifier.0),
    };
    let holder = Arc::clone(&state);
    let outcome = blocking(move || {
        let handle = holder.create_index(config)?;
        detail(&holder, &handle)
    })
    .await;
    match outcome {
        Ok(value) => (StatusCode::CREATED, Json(value)).into_response(),
        Err(failure) => error::rendered(&failure, &identifier.0),
    }
}

/// `GET /v1/indexes`: the served indexes, sorted by name.
///
/// The envelope is the one of ADR-0103: one key, the plural name of the
/// resource. The counts come from the summary the writer refreshed after its
/// last job, never from a read of `records.sqlite` (ADR-0105, decision 3).
pub async fn list_indexes(
    State(state): State<Arc<AppState>>,
    Extension(identifier): Extension<RequestId>,
) -> Response {
    let indexes = match state.indexes() {
        Ok(indexes) => indexes,
        Err(failure) => return error::rendered(&failure, &identifier.0),
    };
    let mut entries = Vec::new();
    for handle in indexes {
        match list_entry(&handle) {
            Ok(entry) => entries.push(entry),
            Err(failure) => return error::rendered(&failure, &identifier.0),
        }
    }
    entries.sort_by(|left, right| {
        left["name"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["name"].as_str().unwrap_or_default())
    });
    Json(json!({ "indexes": entries })).into_response()
}

/// `GET /v1/indexes/{index}`: the configuration and the state of the index.
///
/// `segments` is a **count**, never the list of `SegmentMeta`, whose bounds are
/// iid and never leave the engine (INV-01). `sizes.disk_bytes` walks the index
/// directory, so the call runs off the async threads.
pub async fn get_index(
    State(state): State<Arc<AppState>>,
    Extension(identifier): Extension<RequestId>,
    AxumPath(index): AxumPath<String>,
) -> Response {
    let handle = match state.index(&index) {
        Ok(Some(handle)) => handle,
        Ok(None) => return unknown(&index, &identifier.0),
        Err(failure) => return error::rendered(&failure, &identifier.0),
    };
    let holder = Arc::clone(&state);
    match blocking(move || detail(&holder, &handle)).await {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(failure) => error::rendered(&failure, &identifier.0),
    }
}

/// `PATCH /v1/indexes/{index}`: validates the patch, then queues its job.
///
/// The validation happens **before any queueing** (ADR-0105, decision 4): an
/// unknown key is `400` (ADR-0031), a frozen setting is `409 immutable_setting`
/// (INV-16), a value outside its domain is `400`. The job kind is
/// `rebuild_lexical` when the analysis settings changed and the branch stays
/// enabled, `patch_config` otherwise.
pub async fn patch_index(
    State(state): State<Arc<AppState>>,
    Extension(identifier): Extension<RequestId>,
    AxumPath(index): AxumPath<String>,
    Query(query): Query<HashMap<String, String>>,
    Json(body): Json<Value>,
) -> Response {
    let handle = match state.index(&index) {
        Ok(Some(handle)) => handle,
        Ok(None) => return unknown(&index, &identifier.0),
        Err(failure) => return error::rendered(&failure, &identifier.0),
    };
    let patch = match IndexPatch::from_value(&body) {
        Ok(patch) => patch,
        Err(failure) => return error::rendered(&failure, &identifier.0),
    };
    let current = match handle.configuration() {
        Ok(current) => current,
        Err(failure) => return error::rendered(&failure, &identifier.0),
    };
    // The validation runs on the configuration published now; the job applies
    // the patch again to the one published when it starts, so two patches queued
    // before the first job ran compose in the order of their arrival instead of
    // the second erasing the first (ADR-0105, decision 4).
    let mut candidate = current.clone();
    if let Err(failure) = apply_patch(&mut candidate, &patch) {
        return error::rendered(&failure, &identifier.0);
    }
    let kind = if candidate.lexical != current.lexical && candidate.lexical.enabled {
        JobKind::RebuildLexical
    } else {
        JobKind::PatchConfig
    };
    let directory = match index_directory(&state, &index) {
        Ok(directory) => directory,
        Err(failure) => return error::rendered(&failure, &identifier.0),
    };
    let work = Box::new(
        move |store: &mut GenerationStore, _progress: &JobProgress, writer: &IndexWriter| {
            let mut candidate = writer.configuration()?;
            apply_patch(&mut candidate, &patch)?;
            rewrite_index_config(&directory, &candidate.to_json_string())?;
            // `reconfigure` builds the snapshot without publishing it, so the
            // configuration, the snapshot and the counts are swapped under one
            // write lock: a request reads the old triplet or the new one, never a
            // configuration beside a snapshot of another generation (ADR-0105,
            // decision 4).
            let snapshot = store.reconfigure(&candidate)?;
            store.adopt(Arc::clone(&snapshot));
            let summary = IndexSummary::of_snapshot(store, &snapshot)?;
            writer.publish(candidate, snapshot, summary)?;
            Ok(JobOutcome {
                generation: Some(store.generation().get()),
                result: None,
            })
        },
    );
    match state.submit_job(&index, kind, work).await {
        Ok(job) => accepted(&state, job, wait_requested(&query)).await,
        Err(failure) => error::rendered(&failure, &identifier.0),
    }
}

/// `DELETE /v1/indexes/{index}?confirm={index}`: quarantines, then removes the
/// index.
///
/// A `confirm` absent or different from the name renders `400 invalid_request`
/// and the message names the parameter (ADR-0105, decision 5). The job runs in
/// the queue of the index, so the jobs accepted before it finish first; it then
/// removes the index from the registry, marks the writer deleted, renames the
/// directory to `<data>/tmp/deleted-<index>-<job_id>` and removes it
/// (decision 6). The jobs still queued behind it fail in `unknown_index`.
pub async fn delete_index(
    State(state): State<Arc<AppState>>,
    Extension(identifier): Extension<RequestId>,
    AxumPath(index): AxumPath<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    match state.index(&index) {
        Ok(Some(_)) => {}
        Ok(None) => return unknown(&index, &identifier.0),
        Err(failure) => return error::rendered(&failure, &identifier.0),
    }
    if query.get("confirm").map(String::as_str) != Some(index.as_str()) {
        let failure = error::invalid(
            "the 'confirm' parameter must be present and repeat the name of the index",
        );
        return error::rendered(&failure, &identifier.0);
    }
    let data = match state.data_root().map(Path::to_path_buf) {
        Some(data) => data,
        None => {
            let failure = error::internal("the instance has not loaded its indexes");
            return error::rendered(&failure, &identifier.0);
        }
    };
    let holder = Arc::clone(&state);
    let name = index.clone();
    let work = Box::new(
        move |_store: &mut GenerationStore, progress: &JobProgress, writer: &IndexWriter| {
            let source = data.join("indexes").join(&name);
            let tmp = data.join("tmp");
            let target = tmp.join(format!("deleted-{name}-{}", progress.job_id()));
            // ADR-0105, decision 6: the rename comes first. A failure leaves the
            // index both registered and on disk, so the job fails and the index
            // stays intact; deregistering first would hide a served directory
            // from the registry until the next restart. Between the rename and
            // the removal of the registry entry, a search that already holds its
            // snapshot finishes on it: the files it mapped stay valid.
            rename_to_quarantine(&source, &target)?;
            if holder.remove_index(&name)?.is_none() {
                return Err(CribleError::new(
                    ErrorCode::UnknownIndex,
                    format!("unknown index '{name}'"),
                ));
            }
            writer.mark_deleted();
            remove_quarantined(&target);
            Ok(JobOutcome {
                generation: None,
                result: None,
            })
        },
    );
    match state.submit_job(&index, JobKind::DeleteIndex, work).await {
        Ok(job) => accepted(&state, job, wait_requested(&query)).await,
        Err(failure) => error::rendered(&failure, &identifier.0),
    }
}

/// One entry of `GET /v1/indexes`.
fn list_entry(handle: &IndexHandle) -> Result<Value, CribleError> {
    let published = handle.published()?;
    let config = published.config;
    let summary = published.summary;
    Ok(json!({
        "name": handle.name,
        "model": config.model,
        "generation": summary.generation,
        "documents": summary.documents,
        "live_passages": summary.live_passages,
    }))
}

/// The body shared by `POST /v1/indexes` and `GET /v1/indexes/{index}`.
///
/// Blocking: it walks the index directory for `sizes.disk_bytes`. The other
/// numbers come from the published snapshot (ADR-0105, decision 3), and
/// `last_batch` is the last row of `batches`, `null` when none was attached.
fn detail(state: &AppState, handle: &IndexHandle) -> Result<Value, CribleError> {
    // One read guard for the three parts: the configuration, the snapshot the
    // counts come from and the counts are published together (ADR-0105,
    // decision 4).
    let published = handle.published()?;
    let config = published.config;
    let summary = published.summary;
    let directory = index_directory(state, &handle.name)?;
    let disk_bytes = directory_size(&directory)?;
    Ok(json!({
        "name": handle.name,
        "config": config.to_value(),
        "state": {
            "epoch": summary.epoch,
            "generation": summary.generation,
            "documents": summary.documents,
            "live_passages": summary.live_passages,
            "dead_passages": summary.dead_passages,
            "dead_ratio": summary.dead_ratio(),
            "segments": summary.segments,
            "sizes": {
                "disk_bytes": disk_bytes,
                "filter_memory_bytes": summary.filter_memory_bytes,
                "codes_memory_bytes": summary.codes_memory_bytes,
            },
            "last_batch": summary.last_batch.as_ref().map(|batch| json!({
                "batch_id": batch.batch_id,
                "applied_at": batch.applied_at,
                "generation": batch.generation,
            })),
        },
    }))
}

/// `<data>/indexes/<name>`, the directory of the index.
fn index_directory(state: &AppState, name: &str) -> Result<PathBuf, CribleError> {
    let data = state
        .data_root()
        .ok_or_else(|| error::internal("the instance has not loaded its indexes"))?;
    Ok(data.join("indexes").join(name))
}

/// Size of a directory tree, in bytes.
///
/// Symbolic links are skipped: following one would count a file outside
/// `<data>`, and a loop would never end. The value is the disk space the index
/// itself occupies, never a link target (ADR-0105, "À surveiller").
fn directory_size(directory: &Path) -> Result<u64, CribleError> {
    let mut total = 0_u64;
    let entries = fs::read_dir(directory).map_err(|error| {
        error::internal(format!(
            "cannot measure the index directory {}: {error}",
            directory.display()
        ))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            error::internal(format!(
                "cannot read an entry of {}: {error}",
                directory.display()
            ))
        })?;
        // `symlink_metadata` describes the link itself, so the walk never
        // descends into one nor counts its target.
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
            error::internal(format!(
                "cannot read the metadata of {}: {error}",
                entry.path().display()
            ))
        })?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            total = total.saturating_add(directory_size(&entry.path())?);
        } else {
            total = total.saturating_add(metadata.len());
        }
    }
    Ok(total)
}

/// Renames `source` to `target`, before the index leaves the registry
/// (ADR-0105, decision 6).
///
/// The rename happens in the same file system, so a crash leaves either the
/// index in place or a `deleted-*` directory the startup purges (ADR-0056). A
/// failure leaves the source untouched and fails the job, with the index still
/// registered and served.
fn rename_to_quarantine(source: &Path, target: &Path) -> Result<(), CribleError> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            error::internal(format!(
                "cannot create the quarantine directory {}: {error}",
                parent.display()
            ))
        })?;
    }
    fs::rename(source, target).map_err(|error| {
        error::internal(format!(
            "cannot quarantine the index directory {}: {error}",
            source.display()
        ))
    })
}

/// Removes a directory the job already renamed out of `<data>/indexes`.
///
/// A failure is logged and does not fail the job: the index is gone from the
/// registry, and the startup purges the `deleted-*` directory it finds.
fn remove_quarantined(target: &Path) {
    if let Err(error) = fs::remove_dir_all(target) {
        warn!(
            directory = %target.display(),
            cause = %error,
            "a quarantined index directory could not be removed"
        );
    }
}

/// Runs `work` on a blocking thread, so the async threads keep serving.
async fn blocking<T, F>(work: F) -> Result<T, CribleError>
where
    F: FnOnce() -> Result<T, CribleError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work).await.map_err(|join| {
        error::internal(format!("a blocking task of an index route failed: {join}"))
    })?
}

/// Renders the `404 unknown_index` of an index the registry does not serve.
fn unknown(index: &str, identifier: &str) -> Response {
    let failure = CribleError::new(ErrorCode::UnknownIndex, format!("unknown index '{index}'"));
    error::rendered(&failure, identifier)
}

/// Whether the request asked for `?wait=true` (ADR-0067).
fn wait_requested(query: &HashMap<String, String>) -> bool {
    query.get("wait").is_some_and(|value| value == "true")
}
