// SPDX-License-Identifier: Apache-2.0

//! Lifecycle of the server: bind, load, serve, drain.
//!
//! References: ADR-0099 (order of the startup, `/health`, `/ready`, write open
//! of the indexes, models by fingerprint, stop on SIGTERM and SIGINT bounded by
//! `[limits].default_timeout_ms` + 1 000 ms, second signal) and `docs/07`
//! ("Docker", clean stop).
//!
//! The order is the one ADR-0099 fixes, and it matters:
//!
//! 1. the listener is bound, so `/health` answers `200` during a long recovery;
//! 2. the router is served while the loader opens the indexes in a blocking
//!    task, so `/ready` answers `503` until it is done;
//! 3. the loader marks the state ready, or its failure stops the startup with a
//!    non-zero exit code: there is no partial service (ADR-0099);
//! 4. the drainage withdraws `/ready` before it stops accepting new
//!    connections (in `run_server`, so a signal and a test take the same path),
//!    drains the requests in flight for at most `default_timeout_ms` + 1 000 ms,
//!    then exits;
//! 5. a second signal exits at once with a non-zero code: the binary hands the
//!    runtime to `shutdown_background`, so a recovery still in progress in a
//!    blocking task cannot delay the exit.
//!
//! The write jobs do not exist yet: `FILE-D-ECRITURE-ET-JOBS` owns their wait
//! (ADR-0095), and this task has none to drain.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use crible_core::error::CribleError;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tracing::{error, info, warn};

use crate::auth::{Auth, warn_if_empty};
use crate::config::Config;
use crate::error::{internal, invalid};
use crate::http::build_router;
use crate::state::AppState;

/// Milliseconds added to `[limits].default_timeout_ms` for the drainage bound
/// (ADR-0099). It is not a configuration key.
const DRAIN_MARGIN_MS: u64 = 1_000;

/// Phase of the shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The server accepts requests.
    Running,
    /// The server refuses new connections and drains the requests in flight.
    Draining,
    /// The drainage is abandoned: the server exits now.
    Forced,
}

/// How the server stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// Every request in flight ended within the bound.
    Drained,
    /// The bound was reached; the remaining requests were cut and logged.
    BoundReached,
    /// A second signal forced the exit.
    Forced,
}

/// Trigger of the shutdown, shared by the signal listener, the readiness and
/// the server loop.
///
/// It carries a `watch` channel rather than a `Notify` because two tasks await
/// the same transition, and one notification would wake only one of them.
#[derive(Debug, Clone)]
pub struct Shutdown {
    phase: watch::Sender<Phase>,
}

impl Shutdown {
    /// Creates a trigger in [`Phase::Running`].
    #[must_use]
    pub fn new() -> Self {
        let (phase, _receiver) = watch::channel(Phase::Running);
        Self { phase }
    }

    /// Asks for a graceful stop.
    pub fn drain(&self) {
        self.set(Phase::Draining);
    }

    /// Asks for an immediate exit.
    pub fn force(&self) {
        self.set(Phase::Forced);
    }

    /// Current phase of the shutdown.
    #[must_use]
    pub fn phase(&self) -> Phase {
        *self.phase.borrow()
    }

    /// Whether the server has begun to drain: no new job is accepted from that
    /// moment (ADR-0104, decision 1).
    #[must_use]
    pub fn is_draining(&self) -> bool {
        self.phase() != Phase::Running
    }

    /// Waits until the server leaves [`Phase::Running`].
    pub async fn wait_draining(&self) {
        if self.is_draining() {
            return;
        }
        let mut phase = self.receiver();
        let _ = phase.wait_for(|phase| *phase != Phase::Running).await;
    }

    fn set(&self, phase: Phase) {
        self.phase.send_if_modified(|current| {
            // `Forced` is final: a later `drain` cannot reopen the drainage.
            if *current == Phase::Forced || *current == phase {
                return false;
            }
            *current = phase;
            true
        });
    }

    fn receiver(&self) -> watch::Receiver<Phase> {
        self.phase.subscribe()
    }
}

impl Default for Shutdown {
    fn default() -> Self {
        Self::new()
    }
}

/// Serves a bound listener until `shutdown` asks it to stop.
///
/// The listener is bound by the caller, so a test knows the port. From the
/// first non-running phase, the state stops being ready: `/ready` answers `503`
/// while the connections already open are drained (ADR-0099, "Arrêt", step 1).
/// The drain is bounded by `drain`: when the bound is reached, the connections
/// still open are cut, which is logged, and the function returns
/// [`StopReason::BoundReached`].
///
/// The write jobs are drained in parallel, bounded by `jobs` counted from the
/// same signal (`[limits].wait_timeout_ms`, ADR-0104). Jobs accepted before the
/// signal finish; a submission after it is refused by [`AppState`]. At the job
/// bound, the function logs the abandoned jobs, with their identifier, kind and
/// index, and returns [`StopReason::BoundReached`] as well.
///
/// # Errors
///
/// [`ErrorCode::Internal`](crible_core::error::ErrorCode::Internal) when the
/// server task fails or the trigger disappears.
pub async fn run_server(
    listener: TcpListener,
    router: Router,
    state: Arc<AppState>,
    shutdown: Shutdown,
    drain: Duration,
    jobs: Duration,
) -> Result<StopReason, CribleError> {
    let mut stopping = shutdown.receiver();
    let graceful = shutdown.clone();
    let readiness = Arc::clone(&state);
    let server = axum::serve(listener, router).with_graceful_shutdown(async move {
        let mut serving = graceful.receiver();
        let _ = serving.wait_for(|phase| *phase != Phase::Running).await;
        // ADR-0099, "Arrêt", step 1: the readiness is withdrawn before the
        // acceptance stops, in the same task, so a probe never reads "ready" on
        // a listener that no longer accepts a connection.
        readiness.mark_not_ready();
    });
    let mut handle = tokio::spawn(async move { server.await });

    if stopping
        .wait_for(|phase| *phase != Phase::Running)
        .await
        .is_err()
    {
        handle.abort();
        return Err(internal(
            "the shutdown trigger disappeared before the server stopped",
        ));
    }
    // The drainage begins here, whether a signal or a test triggered it: the
    // readiness was withdrawn by the graceful shutdown of the server task, just
    // before it stopped accepting (ADR-0099, "Arrêt", step 1).
    if *stopping.borrow_and_update() == Phase::Forced {
        handle.abort();
        return Ok(StopReason::Forced);
    }

    let mut forced = shutdown.receiver();
    let forced_wait = async move {
        let _ = forced.wait_for(|phase| *phase == Phase::Forced).await;
    };
    let drain_both = async {
        let requests = tokio::time::timeout(drain, &mut handle);
        let writes = async {
            match state.jobs() {
                Some(board) => tokio::time::timeout(jobs, board.drained()).await.is_ok(),
                None => true,
            }
        };
        tokio::join!(requests, writes)
    };
    tokio::select! {
        () = forced_wait => {
            warn!("a second signal cuts the drainage and the process exits now");
            handle.abort();
            Ok(StopReason::Forced)
        }
        (requests, writes) = drain_both => {
            let mut reason = StopReason::Drained;
            match requests {
                Ok(joined) => {
                    let outcome = joined.map_err(join_failure)?;
                    outcome.map_err(|error| {
                        internal(format!("the HTTP server stopped on an I/O failure: {error}"))
                    })?;
                }
                Err(_) => {
                    warn!(
                        bound_ms = drain.as_millis(),
                        "the drainage bound is reached; the requests still in flight are cut"
                    );
                    reason = StopReason::BoundReached;
                    handle.abort();
                }
            }
            if !writes {
                let abandoned = state
                    .jobs()
                    .map_or_else(Vec::new, |board| board.unfinished());
                warn!(
                    abandoned = abandoned.len(),
                    bound_ms = jobs.as_millis(),
                    "the job bound is reached; the write jobs still unfinished are abandoned"
                );
                for job in &abandoned {
                    warn!(
                        job_id = %job.id,
                        kind = job.kind.as_str(),
                        index = %job.index,
                        "abandoned write job"
                    );
                }
                reason = StopReason::BoundReached;
            }
            Ok(reason)
        }
    }
}

/// Binds the configured address and serves the instance until a signal.
///
/// # Errors
///
/// [`ErrorCode::InvalidRequest`](crible_core::error::ErrorCode::InvalidRequest)
/// when the address cannot be bound, and every error of [`AppState::load`] when
/// the startup cannot complete (ADR-0099: no partial service).
///
/// The returned [`StopReason`] is [`StopReason::Forced`] when a second signal
/// cut the drainage.
pub async fn serve(config: Config) -> Result<StopReason, CribleError> {
    let listener = TcpListener::bind(config.server.bind.as_str())
        .await
        .map_err(|error| invalid(format!("cannot bind to '{}': {error}", config.server.bind)))?;
    serve_bound(listener, config).await
}

/// Same as [`serve`], on a listener the caller already bound.
///
/// # Errors
///
/// The errors of [`serve`], and those of the signal registration.
pub async fn serve_bound(listener: TcpListener, config: Config) -> Result<StopReason, CribleError> {
    let address = listener
        .local_addr()
        .map_err(|error| internal(format!("cannot read the bound address: {error}")))?;
    info!(address = %address, "the server is bound");
    let state = Arc::new(AppState::new());
    let shutdown = Shutdown::new();
    state.set_shutdown(shutdown.clone())?;
    // ADR-0100, decision 4: an empty `[[auth.keys]]` is the closed-by-default
    // instance. It starts, and says so once instead of failing every request
    // silently.
    warn_if_empty(&config.auth.keys);
    state.set_auth(Arc::new(Auth::new(&config.auth.keys)?))?;
    let router = build_router(Arc::clone(&state));
    let drain = Duration::from_millis(
        config
            .limits
            .default_timeout_ms
            .saturating_add(DRAIN_MARGIN_MS),
    );
    // ADR-0104, decision 3: the wait of the write jobs is bounded by
    // `[limits].wait_timeout_ms`, counted from the signal, in parallel with the
    // request drainage above.
    let jobs_drain = Duration::from_millis(config.limits.wait_timeout_ms);
    install_signal_handlers(shutdown.clone())?;

    let mut server = tokio::spawn(run_server(
        listener,
        router,
        Arc::clone(&state),
        shutdown.clone(),
        drain,
        jobs_drain,
    ));
    let loading = {
        let state = Arc::clone(&state);
        let config = config.clone();
        tokio::task::spawn_blocking(move || state.load(&config))
    };
    tokio::pin!(loading);
    tokio::select! {
        loaded = &mut loading => {
            match loaded {
                Ok(Ok(())) => {
                    if let Err(failure) = state.start_writers() {
                        error!(
                            code = failure.code().as_str(),
                            message = failure.message(),
                            "the startup failed; the instance exits"
                        );
                        shutdown.force();
                        let _ = server.await;
                        return Err(failure);
                    }
                    if publish_ready(&state, &shutdown) {
                        let indexes = state
                            .loaded()
                            .and_then(|loaded| loaded.indexes().ok())
                            .map_or(0, |indexes| indexes.len());
                        info!(indexes, "the instance is ready");
                    } else {
                        info!(
                            "the startup finished during the drainage; the instance stays not ready"
                        );
                    }
                }
                Ok(Err(failure)) => {
                    error!(
                        code = failure.code().as_str(),
                        message = failure.message(),
                        "the startup failed; the instance exits"
                    );
                    shutdown.force();
                    let _ = server.await;
                    return Err(failure);
                }
                Err(join) => {
                    error!(message = %join, "the startup task failed; the instance exits");
                    shutdown.force();
                    let _ = server.await;
                    return Err(join_failure(join));
                }
            }
        }
        stopped = &mut server => {
            return stopped.map_err(join_failure)?;
        }
    }

    server.await.map_err(join_failure)?
}

/// Loads the configuration of `path` and serves the instance until a signal.
///
/// The binary entry point: it owns the tokio runtime, because every other
/// command of `crible` is synchronous.
///
/// On [`StopReason::Forced`], the runtime is shut down in the background before
/// the error is returned. Dropping it would wait for the `spawn_blocking` task
/// of the loader, whose recovery cannot be cancelled, and ADR-0099 requires the
/// second signal to exit at once. The same holds when the drainage ends while
/// the loader still runs, so every exit uses `shutdown_background`: a recovered
/// index needs no join, and the process must not wait for an uncancellable task.
///
/// # Errors
///
/// The errors of [`Config::load`] and of [`serve`], plus an
/// [`ErrorCode::Internal`](crible_core::error::ErrorCode::Internal) when the
/// runtime cannot start or when a second signal forced the exit.
pub fn serve_blocking(path: &Path) -> Result<(), CribleError> {
    let config = Config::load(path)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| internal(format!("cannot start the runtime of the server: {error}")))?;
    let outcome = runtime.block_on(serve(config));
    runtime.shutdown_background();
    match outcome {
        Ok(StopReason::Forced) => Err(forced()),
        Ok(StopReason::Drained | StopReason::BoundReached) => Ok(()),
        Err(error) => Err(error),
    }
}

/// Registers the signal listeners: the first stop is graceful, the second one
/// immediate (ADR-0099).
#[cfg(unix)]
fn install_signal_handlers(shutdown: Shutdown) -> Result<(), CribleError> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate())
        .map_err(|error| internal(format!("cannot listen to SIGTERM: {error}")))?;
    let mut interrupt = signal(SignalKind::interrupt())
        .map_err(|error| internal(format!("cannot listen to SIGINT: {error}")))?;
    drop(tokio::spawn(async move {
        tokio::select! {
            _ = terminate.recv() => info!("received SIGTERM"),
            _ = interrupt.recv() => info!("received SIGINT"),
        }
        // The drainage withdraws the readiness in `run_server`, the single
        // place that serves it, so a test that drains goes through the same
        // step (ADR-0099, step 1).
        shutdown.drain();
        info!("stopping: new requests are refused and the requests in flight are drained");
        tokio::select! {
            _ = terminate.recv() => warn!("received a second signal"),
            _ = interrupt.recv() => warn!("received a second signal"),
        }
        shutdown.force();
    }));
    Ok(())
}

/// Registers `Ctrl-C`, the only signal of a non-Unix target.
#[cfg(not(unix))]
fn install_signal_handlers(shutdown: Shutdown) -> Result<(), CribleError> {
    drop(tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_err() {
            return;
        }
        shutdown.drain();
        info!("stopping: new requests are refused and the requests in flight are drained");
        if tokio::signal::ctrl_c().await.is_ok() {
            warn!("received a second interrupt");
            shutdown.force();
        }
    }));
    Ok(())
}

/// Publishes the readiness of a finished startup, unless the drainage has begun.
///
/// The loader can finish after `run_server` withdrew the readiness. The phase is
/// read only after [`AppState::mark_ready`], so the window closes: if the read
/// sees [`Phase::Running`], the withdrawal of the drainage happens strictly after
/// this `mark_ready` and leaves the state not ready; otherwise the readiness is
/// withdrawn again here (ADR-0099, "Arrêt", step 1).
///
/// Returns whether the instance stays ready.
fn publish_ready(state: &AppState, shutdown: &Shutdown) -> bool {
    state.mark_ready();
    if shutdown.is_draining() {
        state.mark_not_ready();
        return false;
    }
    true
}

/// Turns a failed join into an [`ErrorCode::Internal`] error.
fn join_failure(error: tokio::task::JoinError) -> CribleError {
    internal(format!("a task of the server failed: {error}"))
}

/// Error of a shutdown forced by a second signal.
fn forced() -> CribleError {
    internal("a second shutdown signal forced the exit before the drainage ended")
}

#[cfg(test)]
mod tests {
    use super::{Phase, Shutdown, publish_ready};
    use crate::state::AppState;

    #[test]
    fn a_startup_that_finishes_during_the_drainage_stays_not_ready() {
        // The loader can mark the state ready after the drainage withdrew the
        // readiness. The test forces that order without a race: the drain and
        // its withdrawal happen first, then the startup publishes.
        let state = AppState::new();
        let shutdown = Shutdown::new();
        shutdown.drain();
        state.mark_not_ready();
        assert!(
            !publish_ready(&state, &shutdown),
            "the startup does not publish a ready instance during the drainage"
        );
        assert!(
            !state.is_ready(),
            "`mark_ready` must not reopen `/ready` during the drainage (ADR-0099)"
        );
    }

    #[test]
    fn a_startup_that_finishes_while_running_stays_ready() {
        let state = AppState::new();
        let shutdown = Shutdown::new();
        assert!(publish_ready(&state, &shutdown));
        assert!(state.is_ready());
    }

    #[test]
    fn the_phase_only_moves_forward() {
        // A `drain` after a `force` must not reopen the drainage: the second
        // signal is the operator's escape hatch (ADR-0099).
        let shutdown = Shutdown::new();
        assert_eq!(*shutdown.receiver().borrow(), Phase::Running);
        shutdown.drain();
        assert_eq!(*shutdown.receiver().borrow(), Phase::Draining);
        shutdown.force();
        assert_eq!(*shutdown.receiver().borrow(), Phase::Forced);
        shutdown.drain();
        assert_eq!(*shutdown.receiver().borrow(), Phase::Forced);
    }

    #[tokio::test]
    async fn a_second_receiver_sees_the_transition() {
        // Two tasks await the same transition; a `Notify` would wake only one.
        let shutdown = Shutdown::new();
        let mut first = shutdown.receiver();
        let mut second = shutdown.receiver();
        shutdown.drain();
        assert!(
            first
                .wait_for(|phase| *phase != Phase::Running)
                .await
                .is_ok(),
            "the first receiver sees the stop"
        );
        assert!(
            second
                .wait_for(|phase| *phase != Phase::Running)
                .await
                .is_ok(),
            "the second receiver sees the stop"
        );
    }
}
