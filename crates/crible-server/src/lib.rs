// SPDX-License-Identifier: Apache-2.0

//! HTTP server for the Crible engine.
//!
//! References: `docs/05-api.md` (conventions), `docs/06-observabilite.md`
//! ("Santé et logs"), `docs/07-stack-et-deploiement.md` ("Configuration de
//! l'instance", "Docker"), and ADR-0096 to ADR-0099.
//!
//! This crate carries the skeleton of the server: the instance configuration
//! with its `CRIBLE_*` overrides ([`config`]), the startup that loads the
//! models and opens the indexes for writing with their recovery ([`state`]),
//! the probes and the request identifier ([`http`]), the API keys and their
//! roles ([`auth`]), the JSON journal ([`logging`]) and the lifecycle with its
//! bounded drainage ([`lifecycle`]).
//!
//! The `/v1` routes, the general error conversion and the write jobs arrive
//! with the tasks that create them; this crate only provides the state, the
//! router they mount on and the middleware they wrap their routes with.

pub mod auth;
// Test barriers of the `failpoints` build only: they hold a job at a precise
// point while a test reads the instance (ADR-0107, decisions 5 and 6).
#[cfg(feature = "failpoints")]
mod barriers;
mod batches;
pub mod config;
mod documents;
mod error;
pub mod http;
mod indexes;
pub mod jobs;
pub mod lifecycle;
pub mod limits;
pub mod logging;
mod maintenance;
pub mod state;

pub use auth::{Auth, Client, protect};
pub use config::Config;
pub use http::{REQUEST_ID_HEADER, RequestId, accepted, build_router};
pub use lifecycle::{Phase, Shutdown, StopReason, run_server, serve, serve_blocking, serve_bound};
pub use logging::{DEFAULT_FILTER, effective_filter, init_logging, log_subscriber};
pub use state::{AppState, IndexHandle, Loaded};
