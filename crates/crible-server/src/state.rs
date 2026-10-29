// SPDX-License-Identifier: Apache-2.0

//! Startup state of the server: the opened indexes and the loaded encoders.
//!
//! References: ADR-0099 ("Démarrage", "Index", "Modèles"). The server binds
//! first, serves `/health` and a `503` `/ready`, then opens every index of
//! `<data>/indexes/*` **for writing** with the recovery of
//! `docs/02-architecture.md`, and loads one session per fingerprint the opened
//! indexes reference. The state becomes ready only when that work is over.
//!
//! The indexes are opened by [`GenerationStore::open`], which is the write open:
//! the server is the writer of every index (`docs/02-architecture.md`,
//! "Concurrence"), so the startup purges the leftover `online-*` batches of
//! `<data>/tmp` with [`online::remove_stale_batches`] (ADR-0056, ADR-0099),
//! cleans `segments/tmp-*`, repairs the lexical branch and rebuilds a missing
//! `codes.bin`. No read-only open exists in the server path in P6.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use crible_core::config::IndexConfig;
use crible_core::error::CribleError;
use crible_core::model::{ModelConfig, ModelKind};
use crible_core::path::path_component;
use crible_encode::ModelRegistry;
use crible_store::generation::GenerationStore;
use crible_store::online;
use tracing::{info, warn};

use crate::config::Config;
use crate::error::{internal, invalid};

/// One index opened by the server.
pub struct IndexHandle {
    /// Configuration of the index, as its `index.json` declares it.
    pub config: IndexConfig,
    /// Writable store of the index, with the snapshot of its committed
    /// generation.
    ///
    /// The store is behind a mutex until `FILE-D-ECRITURE-ET-JOBS` moves it into
    /// the single writer task of the index: `GenerationStore` owns a writable
    /// SQLite connection, which is not `Sync`, and reading a snapshot does not
    /// need the lock.
    pub store: Mutex<GenerationStore>,
}

/// Everything the startup of an instance loads.
pub struct Loaded {
    /// Encoders loaded at startup, one session per referenced fingerprint.
    pub models: ModelRegistry,
    indexes: Vec<IndexHandle>,
}

impl Loaded {
    /// Indexes opened at startup, in the order of their directory names.
    #[must_use]
    pub fn indexes(&self) -> &[IndexHandle] {
        &self.indexes
    }
}

/// State shared by every request.
///
/// It is created at the bind, before any index is opened, so `/health` can
/// answer while the loader runs. `loaded` is filled once; the state never
/// reloads its indexes in P6.
#[derive(Default)]
pub struct AppState {
    ready: AtomicBool,
    loaded: OnceLock<Loaded>,
}

impl AppState {
    /// Creates the state of an instance that is not ready yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether models, indexes and recovery are done.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    /// Publishes the readiness of the instance.
    pub fn mark_ready(&self) {
        self.ready.store(true, Ordering::Release);
    }

    /// Withdraws the readiness of the instance.
    ///
    /// The shutdown calls it before it stops accepting: `/ready` renders `503`
    /// from that moment (ADR-0099, "Arrêt").
    pub fn mark_not_ready(&self) {
        self.ready.store(false, Ordering::Release);
    }

    /// Everything loaded at startup, once the loader has finished.
    #[must_use]
    pub fn loaded(&self) -> Option<&Loaded> {
        self.loaded.get()
    }

    /// Loads the models and opens the indexes of `config`.
    ///
    /// Blocking: the caller runs it off the async threads
    /// (`tokio::task::spawn_blocking`), because the recovery reads and writes
    /// SQLite and hashes model files.
    ///
    /// # Errors
    ///
    /// - [`ErrorCode::InvalidRequest`] when `<data>` does not exist, when an
    ///   entry of `<data>/indexes` has no readable `index.json`, when the model
    ///   of an index has no readable `model.json`, and when the store refuses an
    ///   index for a configuration reason;
    /// - the error of the store or of the encoder otherwise, with the index or
    ///   the model named in front of it, and that of the purge of `<data>/tmp`
    ///   when a leftover whose presence lock is free cannot be removed.
    ///
    /// A model that is required and missing stops the startup (ADR-0099): the
    /// instance answers no `/ready` `200` it could not serve.
    pub fn load(&self, config: &Config) -> Result<(), CribleError> {
        if !config.paths.data.is_dir() {
            return Err(invalid(format!(
                "the data root {} of [paths].data is not a directory",
                config.paths.data.display()
            )));
        }
        // ADR-0099 and `docs/02-architecture.md`: the write open purges the
        // `online-*` leftovers of `<data>/tmp` whose presence lock is free. The
        // server is the writer of every index, so it runs it once, for the
        // whole instance, before opening any index (ADR-0056, decision 4).
        online::remove_stale_batches(&config.paths.data)?;
        let mut models = ModelRegistry::new();
        let mut indexes = Vec::new();
        for directory in index_directories(&config.paths.data.join("indexes"))? {
            let index = read_index_config(&directory)?;
            let store = open_index(&directory, &index, config, &mut models)?;
            info!(
                index = %index.name,
                epoch = store.epoch().get(),
                generation = store.generation().get(),
                "index opened for writing"
            );
            indexes.push(IndexHandle {
                config: index,
                store: Mutex::new(store),
            });
        }
        let loaded = Loaded { models, indexes };
        if loaded.indexes.is_empty() {
            warn!("the instance serves no index");
        }
        info!(
            indexes = loaded.indexes.len(),
            encoders = loaded.models.len(),
            "models and indexes loaded"
        );
        self.loaded
            .set(loaded)
            .map_err(|_| internal("the instance configuration was loaded twice"))
    }
}

/// Directories of `<data>/indexes`, sorted by name.
///
/// An absent `indexes` directory is an instance that serves no index, which is
/// ready (ADR-0099). An absent `<data>` root is not: it names a path the
/// operator expected to exist.
fn index_directories(root: &Path) -> Result<Vec<PathBuf>, CribleError> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(invalid(format!(
                "cannot list the indexes of {}: {error}",
                root.display()
            )));
        }
    };
    let mut directories = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            invalid(format!(
                "cannot read an entry of {}: {error}",
                root.display()
            ))
        })?;
        let kind = entry.file_type().map_err(|error| {
            invalid(format!(
                "cannot read the type of {}: {error}",
                entry.path().display()
            ))
        })?;
        if kind.is_dir() {
            directories.push(entry.path());
        }
    }
    directories.sort();
    Ok(directories)
}

/// Reads the `index.json` of an index directory.
fn read_index_config(directory: &Path) -> Result<IndexConfig, CribleError> {
    let path = directory.join("index.json");
    let raw = fs::read_to_string(&path).map_err(|error| {
        invalid(format!(
            "cannot read the configuration {} of an index: {error}",
            path.display()
        ))
    })?;
    IndexConfig::from_json_str(&raw)
        .map_err(|error| named(error, &format!("the index at {}", directory.display())))
}

/// Reads the model of `index` and loads its session when it needs one.
///
/// An `external` index asks for no encoder: its vectors come with the batch.
/// An `onnx` index loads the fp32 graph, the precision of the acceptance check,
/// of the online encoding and the query default (ADR-0052); the INT8 graph
/// stays loaded on its first use (ADR-0051).
fn load_model(
    index: &IndexConfig,
    config: &Config,
    models: &mut ModelRegistry,
) -> Result<(), CribleError> {
    // `index.json` is a file of the index, not a trusted input: a `model` of
    // `..` or an absolute path would read outside `[paths].models`.
    let model = path_component(&index.model, &format!("model of index '{}'", index.name))?;
    let directory = config.paths.models.join(model);
    let path = directory.join("model.json");
    let raw = fs::read_to_string(&path).map_err(|error| {
        invalid(format!(
            "cannot read the model configuration {} of index '{}': {error}",
            path.display(),
            index.name
        ))
    })?;
    let model = ModelConfig::from_json_str(&raw).map_err(|error| {
        named(
            error,
            &format!("the model '{}' of index '{}'", index.model, index.name),
        )
    })?;
    if model.kind == ModelKind::Onnx {
        models.get_or_load(&directory).map_err(|error| {
            named(
                error,
                &format!(
                    "the model '{}' of index '{}' at {}",
                    index.model,
                    index.name,
                    directory.display()
                ),
            )
        })?;
    }
    Ok(())
}

/// Opens one index for writing, after loading the model it needs.
fn open_index(
    directory: &Path,
    index: &IndexConfig,
    config: &Config,
    models: &mut ModelRegistry,
) -> Result<GenerationStore, CribleError> {
    load_model(index, config, models)?;
    GenerationStore::open(directory, &index.schema)
        .map_err(|error| named(error, &format!("index '{}'", index.name)))
}

/// Prefixes the message of `error` with `context`, keeping its code.
fn named(error: CribleError, context: &str) -> CribleError {
    CribleError::new(error.code(), format!("{context}: {}", error.message()))
}
