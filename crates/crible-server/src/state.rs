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

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use crible_core::config::IndexConfig;
use crible_core::error::{CribleError, ErrorCode};
use crible_core::model::{ModelConfig, ModelKind};
use crible_core::path::path_component;
use crible_encode::{ModelRegistry, Precision, QueryEncoder};
use crible_search::scan_pool::ScanPool;
use crible_store::generation::{GenerationStore, INITIAL_EPOCH, write_index_config};
use crible_store::jobs::{JOBS_FILE_NAME, JobKind, JobState};
use crible_store::online;
use tracing::{info, warn};
use uuid::Uuid;

use crate::auth::Auth;
use crate::config::Config;
use crate::error::{internal, invalid};
use crate::jobs::{IndexWriter, JobBoard, JobView, JobWork, Published, SHUTTING_DOWN_MESSAGE};
use crate::lifecycle::Shutdown;
use crate::limits::SearchLimit;

/// One index opened or created by the server.
pub struct IndexHandle {
    /// Name of the index, frozen at its creation.
    pub name: String,
    /// Single writer task of the index, with its bounded queue and its job
    /// board (ADR-0103). It owns the writable store, so every write of the index
    /// is serialised and no request holds that store. It also owns the published
    /// triplet `(config, snapshot, summary)`, so a `PATCH` job replaces the
    /// configuration and the snapshot together (ADR-0105, decision 4).
    pub writer: Arc<IndexWriter>,
}

impl IndexHandle {
    /// The triplet `(config, snapshot, summary)` of the index, read under one
    /// guard so a request never sees a mixture (ADR-0105, decision 4).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the
    /// lock is poisoned.
    pub fn published(&self) -> Result<Published, CribleError> {
        self.writer.published()
    }

    /// Configuration currently published for the index.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the
    /// lock is poisoned.
    pub fn configuration(&self) -> Result<IndexConfig, CribleError> {
        self.writer.configuration()
    }
}

/// Everything the startup of an instance loads, and the registry the
/// administration routes mutate (ADR-0105, decision 1).
pub struct Loaded {
    /// Encoders loaded at startup or at a creation, one session per referenced
    /// fingerprint. Never unloaded before the restart (ADR-0105, decision 2).
    /// Shared as an [`Arc`] so a write job can load the fp32 graph of an `onnx`
    /// model in its blocking task (ADR-0054, ADR-0107).
    pub models: Arc<ModelRegistry>,
    indexes: RwLock<Vec<Arc<IndexHandle>>>,
    /// Root of `<data>`, for the creation and the deletion of an index.
    data: PathBuf,
    /// Root of `[paths].models`.
    models_root: PathBuf,
    /// Root of `[paths].exchange`, for the batch, export and snapshot routes.
    exchange: PathBuf,
    /// Depth of the write queue of a new index.
    writer_queue: usize,
    /// Serialises the creations, so two of them never write the same name nor
    /// load the same model twice (ADR-0105, decision 1).
    creation: Mutex<()>,
    /// Semaphore of the searches, one per instance (ADR-0102, ADR-0109).
    search_limit: Arc<SearchLimit>,
    /// Dedicated rayon pool of the scans, never the tokio threads
    /// (`docs/02-architecture.md`, ADR-0109).
    scan_pool: Arc<ScanPool>,
    /// Default `timeout_ms` of a search (`[limits].default_timeout_ms`).
    default_timeout_ms: u64,
    /// Entries of the query cache (`[limits].query_cache_entries`, ADR-0051).
    query_cache_entries: usize,
    /// Precision of the query encoding (`[encoder].query_precision`, ADR-0052).
    query_precision: Precision,
    /// One query encoder per model directory, shared by every request so the
    /// cache of ADR-0051 is the one of the instance and not of a request.
    query_encoders: Mutex<HashMap<PathBuf, Arc<QueryEncoder>>>,
}

/// One model of the instance, as a write job reads it.
///
/// The configuration and the directory are read once per job, from
/// `<models>/<model>/model.json`; the registry is shared with the startup, so an
/// `onnx` graph loaded by a job is the one the next job reuses (ADR-0105,
/// decision 2).
pub struct ModelAccess {
    /// Resolved configuration, with `dims`, `normalize` and the encoder fields.
    pub config: ModelConfig,
    /// Directory holding `model.json` and the graph files.
    pub directory: PathBuf,
    registry: Arc<ModelRegistry>,
}

impl ModelAccess {
    /// Returns the fp32 encoder of the model, loading it on first use.
    ///
    /// # Errors
    ///
    /// The errors of [`ModelRegistry::get_or_load`], plus
    /// [`ErrorCode::Internal`] when the registry is poisoned.
    pub fn encoder(&self) -> Result<Arc<crible_encode::Model>, CribleError> {
        self.registry.get_or_load(&self.directory)
    }

    /// Loads the token counter of an `onnx` model.
    ///
    /// # Errors
    ///
    /// The errors of [`crible_encode::TokenCounter::load_with_config`], plus
    /// [`ErrorCode::Internal`] when the model is not an `onnx` one.
    pub fn token_counter(&self) -> Result<crible_encode::TokenCounter, CribleError> {
        crible_encode::TokenCounter::load_with_config(&self.directory, &self.config)
    }
}

impl Loaded {
    /// Indexes in the order of their names.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the registry is poisoned: an empty list
    /// would answer `200` with no index while the instance still serves them.
    pub fn indexes(&self) -> Result<Vec<Arc<IndexHandle>>, CribleError> {
        self.indexes
            .read()
            .map(|indexes| indexes.clone())
            .map_err(|_| internal("the index registry is poisoned"))
    }

    /// The index named `name`, when it is served.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the registry is poisoned: `None` would
    /// render `404` for an index the instance still serves.
    pub fn index(&self, name: &str) -> Result<Option<Arc<IndexHandle>>, CribleError> {
        let indexes = self
            .indexes
            .read()
            .map_err(|_| internal("the index registry is poisoned"))?;
        Ok(indexes.iter().find(|handle| handle.name == name).cloned())
    }

    /// Inserts `handle` and keeps the registry sorted by name.
    fn insert(&self, handle: Arc<IndexHandle>) -> Result<(), CribleError> {
        let mut indexes = self
            .indexes
            .write()
            .map_err(|_| internal("the index registry is poisoned"))?;
        indexes.push(handle);
        indexes.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(())
    }

    /// Removes the index named `name`, for `delete_index`.
    fn remove(&self, name: &str) -> Result<Option<Arc<IndexHandle>>, CribleError> {
        let mut indexes = self
            .indexes
            .write()
            .map_err(|_| internal("the index registry is poisoned"))?;
        let position = indexes.iter().position(|handle| handle.name == name);
        Ok(position.map(|position| indexes.remove(position)))
    }

    /// Root of `[paths].exchange`, for the batch, export and snapshot routes.
    #[must_use]
    pub fn exchange(&self) -> &Path {
        &self.exchange
    }

    /// Directory of the index named `name`: `<data>/indexes/<name>`.
    #[must_use]
    pub fn index_directory(&self, name: &str) -> PathBuf {
        self.data.join("indexes").join(name)
    }

    /// Semaphore of the concurrent searches of the instance.
    #[must_use]
    pub fn search_limit(&self) -> Arc<SearchLimit> {
        Arc::clone(&self.search_limit)
    }

    /// Dedicated rayon pool of the scans.
    #[must_use]
    pub fn scan_pool(&self) -> Arc<ScanPool> {
        Arc::clone(&self.scan_pool)
    }

    /// Default `timeout_ms` of a search.
    #[must_use]
    pub fn default_timeout_ms(&self) -> u64 {
        self.default_timeout_ms
    }

    /// Precision of the query encoding.
    #[must_use]
    pub fn query_precision(&self) -> Precision {
        self.query_precision
    }

    /// Query encoder of the model directory of `access`, built once per
    /// directory and reused by every request.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the cache is poisoned.
    pub fn query_encoder(&self, access: &ModelAccess) -> Result<Arc<QueryEncoder>, CribleError> {
        let mut encoders = self
            .query_encoders
            .lock()
            .map_err(|_| internal("the query encoder cache of the instance is poisoned"))?;
        if let Some(encoder) = encoders.get(&access.directory) {
            return Ok(Arc::clone(encoder));
        }
        let encoder = Arc::new(QueryEncoder::new(
            &access.directory,
            Arc::clone(&self.models),
            self.query_cache_entries,
        ));
        encoders.insert(access.directory.clone(), Arc::clone(&encoder));
        Ok(encoder)
    }

    /// Configuration and encoder directory of the model `name`.
    ///
    /// The name comes from `index.json`, not from a request, and is checked as
    /// one path component so a `..` in a configuration never reads outside
    /// `[paths].models` (ADR-0105, decision 2).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidRequest`] for a name that is not one path component,
    /// a missing or unreadable `model.json`, and the errors of the model
    /// configuration; the errors of the registry when the model is `onnx`.
    pub fn model(&self, name: &str) -> Result<ModelAccess, CribleError> {
        let model = path_component(name, "model of an index")?;
        let directory = self.models_root.join(model);
        let path = directory.join("model.json");
        let raw = fs::read_to_string(&path).map_err(|error| {
            invalid(format!(
                "cannot read the model configuration {}: {error}",
                path.display()
            ))
        })?;
        let config = ModelConfig::from_json_str(&raw).map_err(|error| {
            named(
                error,
                &format!("the model '{}' at {}", model, directory.display()),
            )
        })?;
        let registry = Arc::clone(&self.models);
        let access = ModelAccess {
            config,
            directory,
            registry,
        };
        if access.config.kind == ModelKind::Onnx {
            // The startup already loaded the graph; this keeps a model created
            // after the startup reachable by the first write job (ADR-0105).
            access.encoder()?;
        }
        Ok(access)
    }
}

/// Removes the directory of a creation that failed after its first write.
///
/// ADR-0105, decision 2, asks for the same quarantine as the deletion of
/// decision 6: the half-created directory is renamed to
/// `<data>/tmp/deleted-<name>-<id>`, in the same file system, then removed. A
/// failure to do so is logged and never turned into a second error: the caller
/// is already reporting the failure that armed the guard.
///
/// The guard is armed only once the creation has established that the directory
/// does not exist, so it never removes an index it did not create.
struct CreationQuarantine<'a> {
    data: &'a Path,
    name: &'a str,
    armed: bool,
}

impl<'a> CreationQuarantine<'a> {
    /// Arms the guard for the creation of `name` under `<data>/indexes`.
    fn new(data: &'a Path, name: &'a str) -> Self {
        Self {
            data,
            name,
            armed: true,
        }
    }

    /// Disarms the guard: the index is fully created and registered.
    fn complete(mut self) {
        self.armed = false;
    }
}

impl Drop for CreationQuarantine<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let directory = self.data.join("indexes").join(self.name);
        if !directory.exists() {
            return;
        }
        let tmp = self.data.join("tmp");
        let quarantine = tmp.join(format!("deleted-{}-{}", self.name, Uuid::new_v4().simple()));
        if let Err(error) =
            fs::create_dir_all(&tmp).and_then(|()| fs::rename(&directory, &quarantine))
        {
            warn!(
                index = %self.name,
                cause = %error,
                "a failed creation left its directory in place"
            );
            return;
        }
        if let Err(error) = fs::remove_dir_all(&quarantine) {
            warn!(
                index = %self.name,
                cause = %error,
                "a quarantined creation could not be removed"
            );
        }
    }
}

/// State shared by every request.
///
/// It is created at the bind, before any index is opened, so `/health` can
/// answer while the loader runs. `loaded` is filled once; its registry then
/// changes at runtime, when an index is created or deleted (ADR-0105).
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

    /// Semaphore of the concurrent searches.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the instance has not loaded its
    /// configuration.
    pub fn search_limit(&self) -> Result<Arc<SearchLimit>, CribleError> {
        self.loaded
            .get()
            .map(Loaded::search_limit)
            .ok_or_else(|| internal("the instance has not loaded its indexes"))
    }

    /// Dedicated rayon pool of the scans.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the instance has not loaded its
    /// configuration.
    pub fn scan_pool(&self) -> Result<Arc<ScanPool>, CribleError> {
        self.loaded
            .get()
            .map(Loaded::scan_pool)
            .ok_or_else(|| internal("the instance has not loaded its indexes"))
    }

    /// Default `timeout_ms` of a search.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the instance has not loaded its
    /// configuration.
    pub fn default_timeout_ms(&self) -> Result<u64, CribleError> {
        self.loaded
            .get()
            .map(Loaded::default_timeout_ms)
            .ok_or_else(|| internal("the instance has not loaded its indexes"))
    }

    /// Query encoder of a model, shared by every request.
    ///
    /// # Errors
    ///
    /// The errors of [`Loaded::query_encoder`], and [`ErrorCode::Internal`] when
    /// the instance has not loaded its configuration.
    pub fn query_encoder(&self, access: &ModelAccess) -> Result<Arc<QueryEncoder>, CribleError> {
        self.loaded
            .get()
            .ok_or_else(|| internal("the instance has not loaded its indexes"))?
            .query_encoder(access)
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
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the registry is poisoned.
    pub fn start_writers(&self) -> Result<(), CribleError> {
        if let Some(loaded) = self.loaded.get() {
            for index in loaded.indexes()? {
                index.writer.ensure_started();
            }
        }
        Ok(())
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
                ErrorCode::Overloaded,
                SHUTTING_DOWN_MESSAGE,
            ));
        }
        let loaded = self
            .loaded
            .get()
            .ok_or_else(|| internal("the instance has not loaded its indexes"))?;
        let writer = loaded
            .index(index)?
            .map(|handle| Arc::clone(&handle.writer))
            .ok_or_else(|| {
                CribleError::new(ErrorCode::UnknownIndex, format!("unknown index '{index}'"))
            })?;
        writer.ensure_started();
        writer.submit(kind, work)
    }

    /// One served index, for the administration routes.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the registry is poisoned.
    pub fn index(&self, name: &str) -> Result<Option<Arc<IndexHandle>>, CribleError> {
        match self.loaded.get() {
            Some(loaded) => loaded.index(name),
            None => Ok(None),
        }
    }

    /// Every served index, sorted by name.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the registry is poisoned.
    pub fn indexes(&self) -> Result<Vec<Arc<IndexHandle>>, CribleError> {
        match self.loaded.get() {
            Some(loaded) => loaded.indexes(),
            None => Ok(Vec::new()),
        }
    }

    /// Root of `<data>`, once the startup has published its configuration.
    #[must_use]
    pub fn data_root(&self) -> Option<&Path> {
        self.loaded.get().map(|loaded| loaded.data.as_path())
    }

    /// Root of `[paths].exchange`, once the startup has published its
    /// configuration.
    #[must_use]
    pub fn exchange_root(&self) -> Option<&Path> {
        self.loaded.get().map(|loaded| loaded.exchange.as_path())
    }

    /// Directory of the served index named `name`.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the instance has not loaded its
    /// configuration.
    pub fn index_directory(&self, name: &str) -> Result<PathBuf, CribleError> {
        self.loaded
            .get()
            .map(|loaded| loaded.index_directory(name))
            .ok_or_else(|| internal("the instance has not loaded its indexes"))
    }

    /// Model of a served index, read from its configuration.
    ///
    /// # Errors
    ///
    /// The errors of [`Loaded::model`], and [`ErrorCode::Internal`] when the
    /// instance has not loaded its configuration.
    pub fn model(&self, name: &str) -> Result<ModelAccess, CribleError> {
        self.loaded
            .get()
            .ok_or_else(|| internal("the instance has not loaded its indexes"))?
            .model(name)
    }

    /// Removes `name` from the registry, for the execution of `delete_index`
    /// (ADR-0105, decision 6).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the registry is poisoned.
    pub fn remove_index(&self, name: &str) -> Result<Option<Arc<IndexHandle>>, CribleError> {
        let loaded = self
            .loaded
            .get()
            .ok_or_else(|| internal("the instance has not loaded its indexes"))?;
        loaded.remove(name)
    }

    /// Creates an index and registers it, without a restart (ADR-0105,
    /// decisions 1 and 2).
    ///
    /// Blocking: the caller runs it off the async threads, because it hashes a
    /// model, loads its session and writes the index. The order is the one of
    /// ADR-0105: name, model, configuration write, store creation, writer
    /// construction, registry insertion. A failure after the beginning of the
    /// write quarantines the directory, so a name is never left half created.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidRequest`] for a refused name, a name already taken, an
    /// absent or unreadable model and a configuration the store refuses, and the
    /// errors of the store and of the job table otherwise.
    pub fn create_index(&self, config: IndexConfig) -> Result<Arc<IndexHandle>, CribleError> {
        let loaded = self
            .loaded
            .get()
            .ok_or_else(|| internal("the instance has not loaded its indexes"))?;
        let _creation = loaded
            .creation
            .lock()
            .map_err(|_| internal("the creation lock of the instance is poisoned"))?;
        let name = path_component(&config.name, "index name")?.to_owned();
        if loaded.index(&name)?.is_some() {
            return Err(invalid(format!("index already exists: '{name}'")));
        }
        let directory = loaded.data.join("indexes").join(&name);
        if directory.exists() {
            return Err(invalid(format!("index already exists: '{name}'")));
        }
        // ADR-0105, decision 2: the model is read before any write.
        load_model(&config, &loaded.models_root, &loaded.models)?;
        let quarantine = CreationQuarantine::new(&loaded.data, &name);
        let text = config.to_json_string();
        write_index_config(&directory, &text)?;
        let store = GenerationStore::create(&directory, INITIAL_EPOCH, &config.schema)
            .map_err(|error| named(error, &format!("index '{name}'")))?;
        let jobs = self
            .jobs
            .get()
            .ok_or_else(|| internal("the job board of the instance is not open"))?;
        let writer = Arc::new(IndexWriter::new(
            name.clone(),
            store,
            config,
            loaded.writer_queue,
            Arc::clone(jobs),
        )?);
        let handle = Arc::new(IndexHandle {
            name: name.clone(),
            writer,
        });
        loaded.insert(Arc::clone(&handle))?;
        quarantine.complete();
        Ok(handle)
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
        let models = ModelRegistry::new();
        let mut indexes = Vec::new();
        for directory in index_directories(&config.paths.data.join("indexes"))? {
            let index = read_index_config(&directory)?;
            let store = open_index(&directory, &index, &config.paths.models, &models)?;
            info!(
                index = %index.name,
                epoch = store.epoch().get(),
                generation = store.generation().get(),
                "index opened for writing"
            );
            let name = index.name.clone();
            let writer = Arc::new(IndexWriter::new(
                name.clone(),
                store,
                index,
                config.limits.writer_queue,
                Arc::clone(&jobs),
            )?);
            indexes.push(Arc::new(IndexHandle { name, writer }));
        }
        let loaded = Loaded {
            models: Arc::new(models),
            indexes: RwLock::new(indexes),
            data: config.paths.data.clone(),
            models_root: config.paths.models.clone(),
            exchange: config.paths.exchange.clone(),
            writer_queue: config.limits.writer_queue,
            creation: Mutex::new(()),
            search_limit: Arc::new(SearchLimit::new(config.server.max_concurrent_searches)),
            scan_pool: Arc::new(ScanPool::new(config.server.search_threads)?),
            default_timeout_ms: config.limits.default_timeout_ms,
            query_cache_entries: config.limits.query_cache_entries,
            query_precision: config.encoder.query_precision,
            query_encoders: Mutex::new(HashMap::new()),
        };
        let opened = loaded.indexes()?;
        if opened.is_empty() {
            warn!("the instance serves no index");
        }
        info!(
            indexes = opened.len(),
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
///
/// The models root is passed instead of the whole [`Config`], so the creation of
/// an index (`POST /v1/indexes`) uses the same reading as the startup (ADR-0105,
/// decision 2).
fn load_model(
    index: &IndexConfig,
    models_root: &Path,
    models: &ModelRegistry,
) -> Result<(), CribleError> {
    // `index.json` is a file of the index, not a trusted input: a `model` of
    // `..` or an absolute path would read outside `[paths].models`.
    let model = path_component(&index.model, &format!("model of index '{}'", index.name))?;
    let directory = models_root.join(model);
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
    models_root: &Path,
    models: &ModelRegistry,
) -> Result<GenerationStore, CribleError> {
    load_model(index, models_root, models)?;
    GenerationStore::open(directory, &index.schema)
        .map_err(|error| named(error, &format!("index '{}'", index.name)))
}

/// Prefixes the message of `error` with `context`, keeping its code.
fn named(error: CribleError, context: &str) -> CribleError {
    CribleError::new(error.code(), format!("{context}: {}", error.message()))
}

#[cfg(test)]
mod tests {
    use super::CreationQuarantine;
    use std::fs;
    use std::path::Path;

    /// Names left under `<data>/tmp`.
    fn quarantined(data: &Path) -> Vec<String> {
        fs::read_dir(data.join("tmp"))
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter_map(|entry| entry.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn a_failed_creation_quarantines_then_removes_its_directory() {
        // ADR-0105, decision 2: a failure after the beginning of the write
        // removes the directory the creation made, so a name is never left half
        // created. The guard runs when the caller drops it still armed.
        let temp = tempfile::tempdir().expect("a temporary directory is creatable");
        let data = temp.path();
        let index = data.join("indexes").join("ghost");
        fs::create_dir_all(&index).expect("the half-created directory exists");
        fs::write(index.join("index.json"), b"{}").expect("the configuration is written");

        drop(CreationQuarantine::new(data, "ghost"));

        assert!(
            !index.exists(),
            "the half-created directory must not stay under <data>/indexes"
        );
        assert!(
            quarantined(data)
                .iter()
                .all(|name| !name.starts_with("deleted-")),
            "the quarantine must be removed: {:?}",
            quarantined(data)
        );
    }

    #[test]
    fn a_completed_creation_keeps_its_directory() {
        // The other half of decision 2: a creation that reaches its insertion
        // into the registry disarms the guard.
        let temp = tempfile::tempdir().expect("a temporary directory is creatable");
        let data = temp.path();
        let index = data.join("indexes").join("kept");
        fs::create_dir_all(&index).expect("the created directory exists");
        fs::write(index.join("index.json"), b"{}").expect("the configuration is written");

        CreationQuarantine::new(data, "kept").complete();

        assert!(
            index.join("index.json").is_file(),
            "a completed creation keeps its directory"
        );
    }
}
