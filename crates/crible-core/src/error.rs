// SPDX-License-Identifier: Apache-2.0

//! Error codes of the API and the engine error type.
//!
//! The wire names and HTTP statuses follow the normative table of
//! `docs/05-api.md`, section "Erreurs".

use thiserror::Error;

/// Category of a [`CribleError`], as exposed by the API and the analytics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ErrorCode {
    /// Malformed body or out-of-range parameter.
    InvalidRequest,
    /// Unknown field, operator forbidden for the field type, or ill-typed value.
    InvalidFilter,
    /// Missing or unknown API key.
    Unauthorized,
    /// Insufficient role.
    Forbidden,
    /// Unknown index.
    UnknownIndex,
    /// Unknown document, job or batch.
    NotFound,
    /// Attempt to change an immutable setting.
    ImmutableSetting,
    /// Same `batch_id`, different manifest.
    BatchConflict,
    /// Body above the configured limits.
    PayloadTooLarge,
    /// Incompatible model between a batch and the index, or between indexes.
    IncompatibleModel,
    /// Rejected acceptance check (recette).
    AcceptanceFailed,
    /// Write queue or concurrent search limit reached.
    Overloaded,
    /// Request took too long.
    Timeout,
    /// Unexpected error, logged with the request id.
    Internal,
}

impl ErrorCode {
    /// Every code of `docs/05-api.md`, in the order of the table.
    pub const ALL: [Self; 14] = [
        Self::InvalidRequest,
        Self::InvalidFilter,
        Self::Unauthorized,
        Self::Forbidden,
        Self::UnknownIndex,
        Self::NotFound,
        Self::ImmutableSetting,
        Self::BatchConflict,
        Self::PayloadTooLarge,
        Self::IncompatibleModel,
        Self::AcceptanceFailed,
        Self::Overloaded,
        Self::Timeout,
        Self::Internal,
    ];

    /// Wire name used in the JSON error body and in the analytics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::InvalidFilter => "invalid_filter",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::UnknownIndex => "unknown_index",
            Self::NotFound => "not_found",
            Self::ImmutableSetting => "immutable_setting",
            Self::BatchConflict => "batch_conflict",
            Self::PayloadTooLarge => "payload_too_large",
            Self::IncompatibleModel => "incompatible_model",
            Self::AcceptanceFailed => "acceptance_failed",
            Self::Overloaded => "overloaded",
            Self::Timeout => "timeout",
            Self::Internal => "internal",
        }
    }

    /// HTTP status returned by the API for this code.
    #[must_use]
    pub const fn http_status(self) -> u16 {
        match self {
            Self::InvalidRequest | Self::InvalidFilter => 400,
            Self::Unauthorized => 401,
            Self::Forbidden => 403,
            Self::UnknownIndex | Self::NotFound => 404,
            Self::ImmutableSetting | Self::BatchConflict => 409,
            Self::PayloadTooLarge => 413,
            Self::IncompatibleModel | Self::AcceptanceFailed => 422,
            Self::Internal => 500,
            Self::Overloaded => 503,
            Self::Timeout => 504,
        }
    }
}

/// Typed error carried by every fallible public function of the engine.
///
/// The category is a [`ErrorCode`], so callers can branch on it without
/// parsing the message.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct CribleError {
    code: ErrorCode,
    message: String,
}

impl CribleError {
    /// Builds an error from its category and a human-readable message.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Category of the error.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        self.code
    }

    /// Human-readable message, suitable for the `error.message` field.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}
