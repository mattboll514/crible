// SPDX-License-Identifier: Apache-2.0

//! Administration routes of an index: `POST /compact`, `/exports` and
//! `/snapshots` (ADR-0107, decision 7).
//!
//! The three routes take **no body**: a non-empty body is `400 invalid_request`.
//! Each renders `202` with a job (`compact`, `export`, `snapshot`) and honours
//! `?wait=true`. The destinations are derived from the published generation
//! (`docs/04-donnees-et-ingestion.md`, "Export, instantanés, restauration",
//! ADR-0061, ADR-0064).
//!
//! The job runs in the writer of the index, so the writer is suspended during the
//! copy and the readers keep their `Arc<Snapshot>` (ADR-0017, ADR-0065). A failed
//! compaction follows ADR-0108: the writer reopens the index from disk before its
//! next job.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, Path as AxumPath, Query, State};
use axum::response::Response;
use crible_core::config::IndexConfig;
use crible_core::error::CribleError;
use crible_store::compact::CompactReport;
use crible_store::compact::compact as compact_store;
use crible_store::export::export;
use crible_store::generation::GenerationStore;
use crible_store::jobs::JobKind;
use crible_store::snapshot;
use serde_json::json;

use crate::AppState;
use crate::error;
use crate::http::{RequestId, accepted};
use crate::jobs::JobOutcome;

/// Message of the `400` a non-empty body receives on the three routes.
const NO_BODY_MESSAGE: &str = "this route takes no body; the destination and the bounds come from the published generation \
     (ADR-0107)";

/// Compacts the index, holding the job after the epoch switch when a test asks
/// for it.
///
/// With the `failpoints` feature and `CRIBLE_TEST_BARRIER` set, the job stops
/// after `CURRENT_EPOCH` names E+1 and before the snapshot of E+1 is published:
/// a test reads the document of the index at that exact point and proves that
/// the read route uses the epoch of the published snapshot (ADR-0107,
/// decision 6). Without the variable the call is [`compact_store`].
#[cfg(feature = "failpoints")]
fn compact_for_job(
    store: &mut GenerationStore,
    index_config: &IndexConfig,
) -> Result<CompactReport, CribleError> {
    if std::env::var(crate::barriers::BARRIER_ENV).is_err() {
        return compact_store(store, index_config);
    }
    crible_store::compact::compact_with_observer(store, index_config, |step| {
        if matches!(step, crible_store::compact::CompactStep::Switched) {
            crate::barriers::wait("compaction");
        }
    })
}

/// Compacts the index, without the test barrier of the `failpoints` build.
#[cfg(not(feature = "failpoints"))]
fn compact_for_job(
    store: &mut GenerationStore,
    index_config: &IndexConfig,
) -> Result<CompactReport, CribleError> {
    compact_store(store, index_config)
}

/// `POST /v1/indexes/{index}/compact`: compacts the index into its next epoch.
pub async fn compact(
    State(state): State<Arc<AppState>>,
    Extension(identifier): Extension<RequestId>,
    AxumPath(index): AxumPath<String>,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    if let Err(failure) = served(&state, &index).and_then(|()| no_body(&body)) {
        return error::rendered(&failure, &identifier.0);
    }
    let work: crate::jobs::JobWork = Box::new(move |store, _progress, writer| {
        let index_config = writer.configuration()?;
        let report = compact_for_job(store, &index_config)?;
        Ok(JobOutcome {
            generation: Some(report.generation.get()),
            result: Some(json!({
                "report": {
                    "from_epoch": report.from_epoch.get(),
                    "to_epoch": report.to_epoch.get(),
                    "generation": report.generation.get(),
                    "live_passages": report.live_passages,
                    "removed_passages": report.removed_passages,
                }
            })),
        })
    });
    submit(&state, &index, JobKind::Compact, work, &query, &identifier).await
}

/// `POST /v1/indexes/{index}/exports`: exports the live data to
/// `exchange/outbox/<index>-g<G>/`.
pub async fn exports(
    State(state): State<Arc<AppState>>,
    Extension(identifier): Extension<RequestId>,
    AxumPath(index): AxumPath<String>,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    if let Err(failure) = served(&state, &index).and_then(|()| no_body(&body)) {
        return error::rendered(&failure, &identifier.0);
    }
    let exchange = match exchange_root(&state) {
        Ok(exchange) => exchange,
        Err(failure) => return error::rendered(&failure, &identifier.0),
    };
    let holder = Arc::clone(&state);
    let destination = exchange.clone();
    let work: crate::jobs::JobWork = Box::new(move |store, _progress, writer| {
        let index_config = writer.configuration()?;
        let model = holder.model(&index_config.model)?;
        let report = export(store, &index_config, &model.config, &destination)?;
        Ok(JobOutcome {
            generation: Some(report.generation.get()),
            result: Some(json!({
                "report": {
                    "generation": report.generation.get(),
                    "directory": relative(&destination, &report.directory),
                    "documents": report.documents,
                    "passages": report.passages,
                }
            })),
        })
    });
    submit(&state, &index, JobKind::Export, work, &query, &identifier).await
}

/// `POST /v1/indexes/{index}/snapshots`: copies the published epoch to
/// `exchange/snapshots/<index>-e<E>-g<G>/`.
pub async fn snapshots(
    State(state): State<Arc<AppState>>,
    Extension(identifier): Extension<RequestId>,
    AxumPath(index): AxumPath<String>,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    if let Err(failure) = served(&state, &index).and_then(|()| no_body(&body)) {
        return error::rendered(&failure, &identifier.0);
    }
    let exchange = match exchange_root(&state) {
        Ok(exchange) => exchange,
        Err(failure) => return error::rendered(&failure, &identifier.0),
    };
    let destination = exchange.clone();
    let work: crate::jobs::JobWork = Box::new(move |store, _progress, writer| {
        let index_config = writer.configuration()?;
        let report = snapshot::create(store, &index_config, &destination)?;
        Ok(JobOutcome {
            generation: Some(report.generation.get()),
            result: Some(json!({
                "report": {
                    "index": report.index,
                    "directory": relative(&destination, &report.directory),
                    "epoch": report.epoch.get(),
                    "generation": report.generation.get(),
                    "documents": report.documents,
                    "passages": report.passages,
                    "bytes": report.bytes,
                }
            })),
        })
    });
    submit(&state, &index, JobKind::Snapshot, work, &query, &identifier).await
}

/// Checks that the index is served, before anything else.
fn served(state: &AppState, index: &str) -> Result<(), CribleError> {
    match state.index(index)? {
        Some(_) => Ok(()),
        None => Err(CribleError::new(
            crible_core::error::ErrorCode::UnknownIndex,
            format!("unknown index '{index}'"),
        )),
    }
}

/// Refuses a non-empty body on the three routes (ADR-0107, decision 7).
fn no_body(body: &Bytes) -> Result<(), CribleError> {
    if body.is_empty() {
        Ok(())
    } else {
        Err(error::invalid(NO_BODY_MESSAGE))
    }
}

/// Root of `[paths].exchange`, once the instance has loaded it.
fn exchange_root(state: &AppState) -> Result<std::path::PathBuf, CribleError> {
    state
        .exchange_root()
        .map(Path::to_path_buf)
        .ok_or_else(|| error::internal("the instance has not loaded its indexes"))
}

/// Renders a destination relative to the exchange root, so a job body never
/// carries an absolute path of the server.
fn relative(exchange: &Path, directory: &Path) -> String {
    match directory.strip_prefix(exchange) {
        Ok(relative) => relative.to_string_lossy().into_owned(),
        Err(_) => directory
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

/// Queues `work` and renders the answer of the mutation. `?wait=true` is the
/// only query parameter the three routes honour (ADR-0067).
async fn submit(
    state: &Arc<AppState>,
    index: &str,
    kind: JobKind,
    work: crate::jobs::JobWork,
    query: &HashMap<String, String>,
    identifier: &RequestId,
) -> Response {
    let wait = query.get("wait").is_some_and(|value| value == "true");
    match state.submit_job(index, kind, work).await {
        Ok(job) => accepted(state, job, wait).await,
        Err(failure) => error::rendered(&failure, &identifier.0),
    }
}
