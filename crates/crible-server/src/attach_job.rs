// SPDX-License-Identifier: Apache-2.0

//! Attach of a write job, with the holds of the `failpoints` test build.
//!
//! A deposited batch (`src/batches.rs`) and an online batch (`src/documents.rs`)
//! both end in the attach of `crible-store`, which writes one generation and
//! publishes it (ADR-0016). INV-02 needs a test to hold that write at a precise
//! point while a search reads the published generation. A crash failpoint
//! panics, so it cannot hold anything: the holds are
//! [`crible_store::attach::attach_with_observer`] and
//! [`crible_store::online::ingest_documents_with_observer`], both behind the
//! `failpoints` feature, and this module is the single place that chooses
//! between the plain call and the observed one (ADR-0113, decision 3).
//!
//! Without the feature, or without `CRIBLE_TEST_BARRIER`, the two helpers are
//! the plain calls of the store, so the production behaviour is unchanged.

use std::path::Path;
use std::sync::Arc;

use crible_core::config::IndexConfig;
use crible_core::error::CribleError;
use crible_core::model::ModelConfig;
use crible_encode::{Model, TokenCounter};
use crible_store::attach::AttachReport;
use crible_store::generation::GenerationStore;
use crible_store::online::OnlineDocuments;

/// Attaches a deposited batch, holding it at the barriers of a test when the
/// `failpoints` build is armed.
#[cfg(feature = "failpoints")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn attach_batch(
    store: &mut GenerationStore,
    batch_dir: &Path,
    index_config: &IndexConfig,
    model_config: &ModelConfig,
    token_counter: Option<&TokenCounter>,
    document_encoder: Option<&dyn Fn() -> Result<Arc<Model>, CribleError>>,
) -> Result<AttachReport, CribleError> {
    if std::env::var(crate::barriers::BARRIER_ENV).is_err() {
        return crible_store::attach::attach(
            store,
            batch_dir,
            index_config,
            model_config,
            token_counter,
            document_encoder,
        );
    }
    crible_store::attach::attach_with_observer(
        store,
        batch_dir,
        index_config,
        model_config,
        token_counter,
        document_encoder,
        crate::barriers::hold_attach,
    )
}

/// Attaches a deposited batch; a normal build carries no barrier.
#[cfg(not(feature = "failpoints"))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn attach_batch(
    store: &mut GenerationStore,
    batch_dir: &Path,
    index_config: &IndexConfig,
    model_config: &ModelConfig,
    token_counter: Option<&TokenCounter>,
    document_encoder: Option<&dyn Fn() -> Result<Arc<Model>, CribleError>>,
) -> Result<AttachReport, CribleError> {
    crible_store::attach::attach(
        store,
        batch_dir,
        index_config,
        model_config,
        token_counter,
        document_encoder,
    )
}

/// Ingests an online batch, holding it at the barriers of a test when the
/// `failpoints` build is armed.
#[cfg(feature = "failpoints")]
pub(crate) fn ingest_documents(
    store: &mut GenerationStore,
    request: &OnlineDocuments<'_>,
) -> Result<AttachReport, CribleError> {
    if std::env::var(crate::barriers::BARRIER_ENV).is_err() {
        return crible_store::online::ingest_documents(store, request);
    }
    crible_store::online::ingest_documents_with_observer(
        store,
        request,
        crate::barriers::hold_attach,
    )
}

/// Ingests an online batch; a normal build carries no barrier.
#[cfg(not(feature = "failpoints"))]
pub(crate) fn ingest_documents(
    store: &mut GenerationStore,
    request: &OnlineDocuments<'_>,
) -> Result<AttachReport, CribleError> {
    crible_store::online::ingest_documents(store, request)
}
