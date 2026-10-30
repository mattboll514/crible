// SPDX-License-Identifier: Apache-2.0

//! `POST /v1/indexes/{index}/batches`: attaches a batch deposited in the inbox
//! (ADR-0107, decision 5).
//!
//! The body is `{ "directory": "<name>" }`, and `<name>` is one path component
//! below `<exchange>/inbox`. The checks run **before** the job is queued:
//!
//! 1. `path_component` refuses the empty string, `.`, `..` and the separators
//!    `/` and `\`;
//! 2. the canonical directory must stay under the canonical inbox;
//! 3. `<name>` itself and every entry below it, at any depth, must not be a
//!    symbolic link, even one that stays in the inbox;
//! 4. every entry must be a file or a directory.
//!
//! A refusal is `400 invalid_request` and names the field `directory`, never an
//! absolute path of the server (ADR-0101). An absent directory is
//! `404 not_found`.
//!
//! The operator controls the inbox and the job waits in the writer queue after
//! the route answered, so the job **revalidates** the directory with the same
//! function just before it reads it (complement of 2026-10-29 to ADR-0107,
//! decision 5): a symbolic link or a special entry that appears after the `202`
//! makes the job fail in `invalid_request` and nothing is attached.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path as AxumPath, Query, State};
use axum::response::Response;
use crible_core::error::{CribleError, ErrorCode};
use crible_core::path::path_component;
use crible_store::jobs::JobKind;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;
use crate::error;
use crate::http::{RequestId, accepted};
use crate::jobs::{JobOutcome, log_attach_durations};

/// Body of `POST /v1/indexes/{index}/batches`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchBody {
    /// One path component below the inbox.
    directory: String,
}

/// `POST /v1/indexes/{index}/batches`: validates the inbox path, then queues the
/// attach as an `attach_batch` job.
#[utoipa::path(
    post,
    path = "/v1/indexes/{index}/batches",
    tag = "batches",
    description = "Attach a batch already written in `<exchange>/inbox/<name>`. `directory` is \
                   one path component, without symbolic link nor special entry; a missing \
                   directory is `404 not_found`, a refused one `400 invalid_request`. Role: \
                   `write`; `admin` includes it (ADR-0095).",
    params(
        ("index" = String, Path, description = "Name of the index"),
        ("wait" = Option<bool>, Query, description = "Wait for the job under `wait_timeout_ms` (ADR-0067)")
    ),
    request_body = crate::openapi::DepositedBatchRequest,
    responses(
        (status = 200, description = "The job finished within `wait_timeout_ms`", body = crate::openapi::Job),
        (status = 202, description = "The job was accepted", body = crate::openapi::Job),
        (status = 400, description = "Refused directory: several components, symbolic link or special entry", body = crate::openapi::Error),
        (status = 401, description = "Missing, malformed or unknown API key", body = crate::openapi::Error),
        (status = 403, description = "Insufficient role", body = crate::openapi::Error),
        (status = 404, description = "Unknown index, or absent directory", body = crate::openapi::Error),
        (status = 500, description = "Unexpected failure", body = crate::openapi::Error),
        (status = 503, description = "Write queue full, server draining or SQLite lock", body = crate::openapi::Error)
    )
)]
pub async fn post_batch(
    State(state): State<Arc<AppState>>,
    Extension(identifier): Extension<RequestId>,
    AxumPath(index): AxumPath<String>,
    Query(query): Query<HashMap<String, String>>,
    Json(body): Json<Value>,
) -> Response {
    if let Err(failure) = served(&state, &index) {
        return error::rendered(&failure, &identifier.0);
    }
    let body: BatchBody = match serde_json::from_value(body) {
        Ok(body) => body,
        Err(failure) => {
            return error::rendered(
                &error::invalid(format!("the body of the batch route is invalid: {failure}")),
                &identifier.0,
            );
        }
    };
    let exchange = match state.exchange_root() {
        Some(exchange) => exchange.to_path_buf(),
        None => {
            return error::rendered(
                &error::internal("the instance has not loaded its indexes"),
                &identifier.0,
            );
        }
    };
    // The canonicalisation and the recursive walk cost a syscall per entry, so
    // they run on a blocking thread, never on a thread of the reactor (ADR-0054,
    // ADR-0107).
    let exchange_for_check = exchange.clone();
    let name_for_check = body.directory.clone();
    let checked =
        tokio::task::spawn_blocking(move || inbox_directory(&exchange_for_check, &name_for_check))
            .await;
    match checked {
        Ok(Ok(_canonical)) => {}
        Ok(Err(failure)) => return error::rendered(&failure, &identifier.0),
        Err(join) => {
            return error::rendered(
                &error::internal(format!("the check of the batch directory failed: {join}")),
                &identifier.0,
            );
        }
    }
    let holder = Arc::clone(&state);
    let name = index.clone();
    let directory = body.directory;
    let work: crate::jobs::JobWork = Box::new(move |store, progress, writer| {
        // The route checked the directory before the `202`; the operator can
        // still change the inbox while the job waits in the queue, so the job
        // revalidates it with the same function just before reading it
        // (complement of 2026-10-29 to ADR-0107, decision 5). A test of the
        // `failpoints` build may hold the job here to change the directory.
        #[cfg(feature = "failpoints")]
        crate::barriers::wait("batch");
        let directory = inbox_directory(&exchange, &directory)?;
        let index_config = writer.configuration()?;
        let model = holder.model(&index_config.model)?;
        let token_counter = match model.config.kind {
            crible_core::model::ModelKind::Onnx => Some(model.token_counter()?),
            crible_core::model::ModelKind::External => None,
        };
        let encoder = || model.encoder();
        let report = crate::attach_job::attach_batch(
            store,
            &directory,
            &index_config,
            &model.config,
            token_counter.as_ref(),
            Some(&encoder),
        )
        .map_err(|failure| {
            CribleError::new(
                failure.code(),
                format!("index '{name}': {}", failure.message()),
            )
        })?;
        log_attach_durations(
            progress.job_id(),
            JobKind::AttachBatch,
            &name,
            report.durations,
        );
        Ok(JobOutcome {
            generation: Some(report.generation.get()),
            result: Some(json!({ "report": report.to_json() })),
        })
    });
    match state.submit_job(&index, JobKind::AttachBatch, work).await {
        Ok(job) => accepted(&state, job, wait_requested(&query)).await,
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

/// Resolves `<exchange>/inbox/<name>` and applies the four rules of ADR-0107,
/// decision 5.
///
/// The returned path is canonical, so the job reads the directory that was
/// checked, not a path a link could redirect later.
fn inbox_directory(exchange: &Path, name: &str) -> Result<PathBuf, CribleError> {
    if path_component(name, "directory").is_err() {
        return Err(refusal(
            "the 'directory' of the batch must be a single path component",
        ));
    }
    let inbox = exchange.join("inbox");
    let root = match fs::canonicalize(&inbox) {
        Ok(root) => root,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(absent()),
        Err(error) => {
            return Err(error::internal(format!(
                "cannot canonicalise the batch inbox: {error}"
            )));
        }
    };
    let candidate = inbox.join(name);
    let metadata = match fs::symlink_metadata(&candidate) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(absent()),
        Err(error) => {
            return Err(error::internal(format!(
                "cannot read the metadata of the batch directory: {error}"
            )));
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(refusal(
            "the 'directory' of the batch names a symbolic link, which an attach refuses \
             (ADR-0107)",
        ));
    }
    if !metadata.is_dir() {
        return Err(refusal("the 'directory' of the batch is not a directory"));
    }
    let canonical = fs::canonicalize(&candidate).map_err(|error| {
        error::internal(format!("cannot canonicalise the batch directory: {error}"))
    })?;
    if !canonical.starts_with(&root) {
        return Err(refusal(
            "the 'directory' of the batch must stay under the inbox",
        ));
    }
    check_entries(&canonical)?;
    Ok(canonical)
}

/// Refuses a symbolic link at any depth and an entry that is neither a file nor
/// a directory (ADR-0107, decision 5, rules 3 and 4).
fn check_entries(directory: &Path) -> Result<(), CribleError> {
    let entries = fs::read_dir(directory)
        .map_err(|error| error::internal(format!("cannot list the batch directory: {error}")))?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            error::internal(format!(
                "cannot read an entry of the batch directory: {error}"
            ))
        })?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            error::internal(format!(
                "cannot read the metadata of a batch entry: {error}"
            ))
        })?;
        let kind = metadata.file_type();
        if kind.is_symlink() {
            return Err(refusal(
                "the batch named by 'directory' holds a symbolic link; an attach refuses one at \
                 any depth (ADR-0107)",
            ));
        }
        if kind.is_dir() {
            check_entries(&path)?;
        } else if !kind.is_file() {
            return Err(refusal(
                "the batch named by 'directory' holds an entry that is neither a file nor a \
                 directory (ADR-0107)",
            ));
        }
    }
    Ok(())
}

/// `400 invalid_request` naming the field `directory`.
fn refusal(message: &str) -> CribleError {
    error::invalid(message)
}

/// `404 not_found` of an absent batch directory.
fn absent() -> CribleError {
    CribleError::new(
        ErrorCode::NotFound,
        "the 'directory' of the batch is not found in the inbox",
    )
}

/// Whether the request asked for `?wait=true` (ADR-0067).
fn wait_requested(query: &HashMap<String, String>) -> bool {
    query.get("wait").is_some_and(|value| value == "true")
}
