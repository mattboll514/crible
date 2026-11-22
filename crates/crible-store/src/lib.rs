// SPDX-License-Identifier: Apache-2.0

//! Persistent storage for the Crible engine.

pub mod analytics;
pub mod attach;
pub mod batch_reader;
pub mod codes;
pub mod compact;
pub mod debug_journal;
pub mod document_ordinals;
pub mod export;
pub(crate) mod file_lock;
pub mod filters_index;
pub mod generation;
pub mod jobs;
pub mod lexical;
pub mod online;
pub mod patch;
pub mod readers;
pub mod records;
pub mod segment;
pub mod snapshot;
mod sqlite;
pub(crate) mod writer_lock;
// The observable hook of the segment readers of ADR-0126, decisions 2 and 6. It
// is compiled only in the `failpoints` build, so a production binary carries
// neither the module nor the calls that feed it.
#[cfg(feature = "failpoints")]
pub mod testhooks;
