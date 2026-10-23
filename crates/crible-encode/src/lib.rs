// SPDX-License-Identifier: Apache-2.0

//! Text encoding for the Crible engine.
//!
//! [`Model`] loads an ONNX encoder described by `model.json`
//! (`docs/04-donnees-et-ingestion.md`, section "Le modèle") and turns a text
//! into one vector: prompt, tokenization, CLS pooling and L2 normalization.
//! [`ModelRegistry`] keeps one encoder per fingerprint, so a process that
//! serves several indexes loads each model once.
//!
//! ONNX Runtime is loaded dynamically and named by `ORT_DYLIB_PATH`
//! (`docs/07-stack-et-deploiement.md`); `tools/README.md` documents the
//! installation on a Mac.

pub mod model;
pub mod registry;

pub use model::{Model, Side};
pub use registry::ModelRegistry;
