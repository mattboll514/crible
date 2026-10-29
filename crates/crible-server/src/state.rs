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
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crible_core::config::IndexConfig;
use crible_core::error::CribleError;
use crible_core::model::{ModelConfig, ModelKind};
use crible_core::path::path_component;
use crible_encode::ModelRegistry;
use crible_store::generation::GenerationStore;
use crible_store::jobs::{JOBS_FILE_NAME, JobKind, JobState};
use crible_store::online;
use tracing::{info, warn};

use crate::auth::Auth;
use crate::config::Config;
use crate::error::{internal, invalid};
use crate::jobs::{IndexWriter, JobBoard, JobView, JobWork, SHUTTING_DOWN_MESSAGE};
use crate::lifecycle::Shutdown;

/// One index opened by the server.
pub struct IndexHandle {
    /// Configuration of the index, as its `index.json` declares it.
    pub config: IndexConfig,
    /// Single writer task of the index, with its bounded queue and its job
    /// board (ADR-0103). It owns the writable store, so every write of the index
    /// is serialised and no request holds that store.
    pub writer: Arc<IndexWriter>,
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
    auth: OnceLock<Arc<Auth>>,
    jobs: OnceLock<Arc<JobBoard>>,
    shutdown: OnceLock<Shutdown>,
    wait_timeout: OnceLock<Duration>,
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

    /// Publishes the authenticator of the instance.
    ///
    /// The keys come from `[[auth.keys]]` and never change during the life of
    /// the process (ADR-0100): the routes read this one, and no request can
    /// widen it. It is shared as an [`Arc`] because [`protect`](crate::auth::protect)
    /// moves it into a `'static` layer.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when an authenticator is already published: the
    /// startup sets it once, before the router is served.
    pub fn set_auth(&self, auth: Arc<Auth>) -> Result<(), CribleError> {
        self.auth
            .set(auth)
            .map_err(|_| internal("the authenticator of the instance is already published"))
    }

    /// Authenticator of the instance, once the startup has published it.
    #[must_use]
    pub fn auth(&self) -> Option<Arc<Auth>> {
        self.auth.get().cloned()
    }

    /// Publishes the shutdown trigger, so a submission sees the drainage
    /// (ADR-0104).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when a trigger is already published: the startup
    /// sets it once, before the router is served.
    pub fn set_shutdown(&self, shutdown: Shutdown) -> Result<(), CribleError> {
        self.shutdown
            .set(shutdown)
            .map_err(|_| internal("the shutdown trigger of the instance is already published"))
    }

    /// Shutdown trigger of the instance, once the startup has published it.
    #[must_use]
    pub fn shutdown(&self) -> Option<&Shutdown> {
        self.shutdown.get()
    }

    /// Starts the writer task of every opened index.
    ///
    /// The writers are tokio tasks; a caller outside a runtime leaves them
    /// dormant, and a submission starts its own writer. Called by
    /// `serve_bound` once the startup has succeeded.
    pub fn start_writers(&self) {
        if let Some(loaded) = self.loaded.get() {
            for index in loaded.indexes() {
                index.writer.ensure_started();
            }
        }
    }

    /// Job board of the instance, once the startup has opened it.
    #[must_use]
    pub fn jobs(&self) -> Option<&Arc<JobBoard>> {
        self.jobs.get()
    }

    /// Reads one retained job, if the instance serves one.
    #[must_use]
    pub fn job(&self, id: &str) -> Option<JobView> {
        self.jobs.get()?.job(id)
    }

    /// Lists the retained jobs, filtered by `index` and `state`.
    #[must_use]
    pub fn list_jobs(&self, index: Option<&str>, state: Option<JobState>) -> Vec<JobView> {
        self.jobs
            .get()
            .map_or_else(Vec::new, |board| board.list(index, state))
    }

    /// Submits a job to the writer of `index`, the internal API the write routes
    /// will call.
    ///
    /// The work receives the exclusive access to the store of the index and its
    /// progression handle; it runs on a blocking thread (ADR-0103, decision 8).
    /// From the start of the drainage, the submission is refused with
    /// [`ErrorCode::Overloaded`] and [`SHUTTING_DOWN_MESSAGE`] (ADR-0104,
    /// decision 1).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Overloaded`] when the server drains or the queue is full,
    /// [`ErrorCode::UnknownIndex`] when `index` is not served, and the errors of
    /// the job table.
    pub async fn submit_job(
        &self,
        index: &str,
        kind: JobKind,
        work: JobWork,
    ) -> Result<JobView, CribleError> {
        if let Some(shutdown) = self.shutdown.get()
            && shutdown.is_draining()
        {
            return Err(CribleError::new(
                crible_core::error::ErrorCode::Overloaded,
                SHUTTING_DOWN_MESSAGE,
            ));
        }
        let loaded = self
            .loaded
            .get()
            .ok_or_else(|| internal("the instance has not loaded its indexes"))?;
        let writer = loaded
            .indexes()
            .iter()
            .find(|handle| handle.config.name == index)
            .map(|handle| Arc::clone(&handle.writer))
            .ok_or_else(|| {
                CribleError::new(
                    crible_core::error::ErrorCode::UnknownIndex,
                    format!("unknown index '{index}'"),
                )
            })?;
        writer.ensure_started();
        writer.submit(kind, work)
    }

    /// Waits for `id` under `?wait=true`: until it is terminal, the deadline of
    /// `[limits].wait_timeout_ms`, or the start of the drainage (ADR-0067,
    /// ADR-0104).
    #[must_use]
    pub async fn wait_for_job(&self, id: &str) -> Option<JobView> {
        let board = self.jobs.get()?;
        let Some(shutdown) = self.shutdown.get() else {
            return board.job(id);
        };
        let timeout = self
            .wait_timeout
            .get()
            .copied()
            .unwrap_or(Duration::from_secs(60));
        board.wait_for(id, timeout, shutdown).await
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
    ///   of an index has no readable `model.json`, when the store refuses an
    ///   index for a configuration reason, and when `jobs.sqlite` is of another
    ///   format;
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
        // ADR-0103, decision 5: the job table is opened before any writer, the
        // jobs a restart interrupted become `failed`, and the expired ones are
        // purged. The table is at the root of `<data>`, one per instance.
        let jobs = Arc::new(JobBoard::open(&config.paths.data.join(JOBS_FILE_NAME))?);
        // ADR-0099 and `docs/02-architecture.md`: the write open purges the
        // `online-*` leftovers of `<data>/tmp` whose presence lock is free. The
        // server is the writer of every index, so it runs it once, for the
        // whole instance, before opening any index (ADR-0056, decision 4).
        online::remove_stale_batches(&config.paths.data)?;
        self.wait_timeout
            .get_or_init(|| Duration::from_millis(config.limits.wait_timeout_ms));
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
            let writer = Arc::new(IndexWriter::new(
                index.name.clone(),
                store,
                config.limits.writer_queue,
                Arc::clone(&jobs),
            ));
            indexes.push(IndexHandle {
                config: index,
                writer,
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
        self.jobs
            .set(jobs)
            .map_err(|_| internal("the job board of the instance was opened twice"))?;
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
