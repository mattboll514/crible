// SPDX-License-Identifier: Apache-2.0

//! Identifiers shared by every crate.
//!
//! References: `docs/02-architecture.md`, section "Identifiants".

use std::fmt;
use std::str::FromStr;

use crate::error::{CribleError, ErrorCode};

/// External identifier of a document, provided by the producer.
///
/// A valid identifier is a non-empty UTF-8 string of at most
/// [`DocId::MAX_BYTES`] bytes. This is the only identifier visible outside the
/// engine.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocId(String);

impl DocId {
    /// Maximum length of a document identifier, in bytes of its UTF-8 encoding.
    pub const MAX_BYTES: usize = 512;

    /// Validates and wraps a producer-supplied identifier.
    ///
    /// Empty identifiers and identifiers longer than [`DocId::MAX_BYTES`] are
    /// rejected with [`ErrorCode::InvalidRequest`].
    pub fn new(raw: impl Into<String>) -> Result<Self, CribleError> {
        let raw = raw.into();
        if raw.is_empty() {
            return Err(CribleError::new(
                ErrorCode::InvalidRequest,
                "doc_id must not be empty",
            ));
        }
        if raw.len() > Self::MAX_BYTES {
            return Err(CribleError::new(
                ErrorCode::InvalidRequest,
                format!(
                    "doc_id must be at most {} bytes, got {}",
                    Self::MAX_BYTES,
                    raw.len()
                ),
            ));
        }
        Ok(Self(raw))
    }

    /// Borrows the identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DocId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Key of a passage: the document identifier and the position of the passage
/// inside that document.
///
/// The textual form is `doc_id#ord`. A document identifier may itself contain
/// `#`, so parsing splits on the **last** separator.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PassageKey {
    doc_id: DocId,
    ord: u32,
}

impl PassageKey {
    /// Builds a passage key from its parts.
    #[must_use]
    pub fn new(doc_id: DocId, ord: u32) -> Self {
        Self { doc_id, ord }
    }

    /// Identifier of the document the passage belongs to.
    #[must_use]
    pub fn doc_id(&self) -> &DocId {
        &self.doc_id
    }

    /// Position of the passage inside its document, starting at 0.
    #[must_use]
    pub fn ord(&self) -> u32 {
        self.ord
    }
}

impl fmt::Display for PassageKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}#{}", self.doc_id.as_str(), self.ord)
    }
}

impl FromStr for PassageKey {
    type Err = CribleError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let Some((doc_id, ord)) = raw.rsplit_once('#') else {
            return Err(CribleError::new(
                ErrorCode::InvalidRequest,
                "passage key must use the form doc_id#ord",
            ));
        };
        let doc_id = DocId::new(doc_id)?;
        let ord = ord.parse::<u32>().map_err(|_| {
            CribleError::new(ErrorCode::InvalidRequest, "ord must be a decimal u32")
        })?;
        Ok(Self::new(doc_id, ord))
    }
}

/// Dense internal identifier of a passage, stable within an epoch.
///
/// INV-01: an `Iid` never leaves the engine. This type deliberately does not
/// derive or implement `serde::Serialize`, and this crate does not depend on
/// `serde`, so no response, log record, analytics row or export can carry one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Iid(u32);

impl Iid {
    /// Wraps a dense internal identifier.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the wrapped identifier.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Visible generation of an index, incremented at each validated change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Generation(u64);

impl Generation {
    /// Wraps a generation number.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the wrapped generation number.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Set of index files between two compactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Epoch(u32);

impl Epoch {
    /// Wraps an epoch number.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the wrapped epoch number.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}
