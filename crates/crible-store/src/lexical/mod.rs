// SPDX-License-Identifier: Apache-2.0

//! Lexical branch of an index: the tantivy side of the storage.
//!
//! References: `docs/03-recherche.md` ("Branche lexicale (tantivy)"),
//! `docs/07-stack-et-deploiement.md` (tantivy 0.27), ADR-0007 (tantivy for
//! BM25) and ADR-0086 (`lexical.language` and the analysis chain).

pub mod analyzer;
