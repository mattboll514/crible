// SPDX-License-Identifier: Apache-2.0

//! Cache of loaded encoders, keyed by `(model fingerprint, precision)`.
//!
//! Loading an ONNX graph and hashing its files costs seconds and hundreds of
//! megabytes. A process loads one session per `(fingerprint, precision)` and
//! shares it: `docs/04-donnees-et-ingestion.md` makes the fingerprint the
//! identity of the frozen part of a model, so two model directories with the
//! same fingerprint describe the same encoder (ADR-0028). The INT8 graph is
//! outside the fingerprint, so the precision is part of the key: the two graphs
//! of one model stay distinct (ADR-0051).
//!
//! Sessions are loaded on first use of their precision, so an instance that
//! only serves queries loads the fp32 graph only (ADR-0052).

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crible_core::error::{CribleError, ErrorCode};

use crate::model::{Model, Precision, read_config};

/// Encoders loaded by this process, one per `(fingerprint, precision)`.
#[derive(Debug, Default)]
pub struct ModelRegistry {
    models: Mutex<HashMap<(String, Precision), Arc<Model>>>,
    loads: AtomicUsize,
}

impl ModelRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the fp32 encoder of `directory`, loading it on first use.
    ///
    /// This is the precision of the acceptance check, of the online document
    /// encoding, and the default of queries
    /// ([`crate::DEFAULT_QUERY_PRECISION`], ADR-0052); an explicit INT8 query
    /// goes through [`Self::get_or_load_precision`].
    ///
    /// # Errors
    ///
    /// The errors of [`Model::load_precision`], plus [`ErrorCode::Internal`]
    /// when the registry lock is poisoned.
    pub fn get_or_load(&self, directory: impl AsRef<Path>) -> Result<Arc<Model>, CribleError> {
        self.get_or_load_precision(directory, Precision::Fp32)
    }

    /// Returns the encoder of `directory` for `precision`, loading it on first
    /// use.
    ///
    /// The `model.json` of the directory is read on every call to compute the
    /// fingerprint, which is cheap next to loading the graph. The first caller
    /// for a `(fingerprint, precision)` loads the session while holding the
    /// lock, so concurrent callers wait instead of loading it twice. The two
    /// precisions of one model are separate entries and separate sessions
    /// (ADR-0051).
    ///
    /// # Errors
    ///
    /// The errors of [`Model::load_precision`], plus [`ErrorCode::Internal`]
    /// when the registry lock is poisoned.
    pub fn get_or_load_precision(
        &self,
        directory: impl AsRef<Path>,
        precision: Precision,
    ) -> Result<Arc<Model>, CribleError> {
        let directory = directory.as_ref();
        let config = read_config(directory)?;
        let key = (config.fingerprint(), precision);
        let mut models = self.models.lock().map_err(|_| {
            CribleError::new(
                ErrorCode::Internal,
                "the model registry is poisoned by a previous panic",
            )
        })?;
        if let Some(model) = models.get(&key) {
            return Ok(Arc::clone(model));
        }
        let model = Arc::new(Model::load_with_config(directory, config, precision)?);
        self.loads.fetch_add(1, Ordering::Relaxed);
        models.insert(key, Arc::clone(&model));
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
