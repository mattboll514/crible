// SPDX-License-Identifier: Apache-2.0

//! Text encoding for the Crible engine.
//!
//! [`Model`] loads one ONNX graph of a model described by `model.json`
//! (`docs/04-donnees-et-ingestion.md`, section "Le modèle") and turns a text
//! into one vector: prompt, tokenization, CLS pooling and L2 normalization.
//! [`TokenCounter`] loads the tokenizer alone, without ONNX Runtime, so the
//! attach counts the tokens of every passage and refuses an over-long one
//! (**INV-11**).
//! [`ModelRegistry`] keeps one session per `(fingerprint, precision)`, so a
//! process that serves several indexes loads each graph once (ADR-0051).
//! [`QueryEncoder`] encodes queries in INT8 by default, caches their vectors in
//! RAM, and reports truncation.
//!
//! ONNX Runtime is loaded dynamically and named by `ORT_DYLIB_PATH`
//! (`docs/07-stack-et-deploiement.md`); `tools/README.md` documents the
//! installation on a Mac.

pub mod model;
pub mod query;
pub mod registry;

pub use model::{EncodedText, Model, Precision, Side, TokenCounter};
pub use query::{CacheStats, DEFAULT_QUERY_CACHE_ENTRIES, EncodedQuery, QueryEncoder};
pub use registry::ModelRegistry;
