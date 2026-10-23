// SPDX-License-Identifier: Apache-2.0

//! Cache of loaded encoders, keyed by model fingerprint.
//!
//! Loading an ONNX graph and hashing its files costs seconds and hundreds of
//! megabytes. A process loads one encoder per fingerprint and shares it:
//! `docs/04-donnees-et-ingestion.md` makes the fingerprint the identity of the
//! frozen part of a model, so two model directories with the same fingerprint
//! describe the same encoder (ADR-0028).

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crible_core::error::{CribleError, ErrorCode};

use crate::model::{Model, read_config};

/// Encoders loaded by this process, one per fingerprint.
#[derive(Debug, Default)]
pub struct ModelRegistry {
    models: Mutex<HashMap<String, Arc<Model>>>,
    loads: AtomicUsize,
}

impl ModelRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the encoder of `directory`, loading it on first use.
    ///
    /// The `model.json` of the directory is read on every call to compute the
    /// fingerprint, which is cheap next to loading the graph. The first caller
    /// for a fingerprint loads the model while holding the lock, so concurrent
    /// callers wait instead of loading the same encoder twice.
    ///
    /// # Errors
    ///
    /// The errors of [`Model::load`], plus [`ErrorCode::Internal`] when the
    /// registry lock is poisoned.
    pub fn get_or_load(&self, directory: impl AsRef<Path>) -> Result<Arc<Model>, CribleError> {
        let directory = directory.as_ref();
        let config = read_config(directory)?;
        let fingerprint = config.fingerprint();
        let mut models = self.models.lock().map_err(|_| {
            CribleError::new(
                ErrorCode::Internal,
                "the model registry is poisoned by a previous panic",
            )
        })?;
        if let Some(model) = models.get(&fingerprint) {
            return Ok(Arc::clone(model));
        }
        let model = Arc::new(Model::load_with_config(directory, config)?);
        self.loads.fetch_add(1, Ordering::Relaxed);
        models.insert(fingerprint, Arc::clone(&model));
        Ok(model)
    }

    /// Number of encoders this registry loaded, cached or not.
    ///
    /// It counts actual loads, so a fingerprint served from the cache does not
    /// increment it. A poisoned registry still reports the number of loads that
    /// happened before it was poisoned.
    #[must_use]
    pub fn loads(&self) -> usize {
        self.loads.load(Ordering::Relaxed)
    }

    /// Number of encoders loaded so far.
    ///
    /// A poisoned registry reports 0: it can no longer serve models, because
    /// [`Self::get_or_load`] returns [`ErrorCode::Internal`].
    #[must_use]
    pub fn len(&self) -> usize {
        self.models.lock().map_or(0, |models| models.len())
    }

    /// Whether no encoder is loaded.
    ///
    /// A poisoned registry reports `true`, for the same reason as [`Self::len`].
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
