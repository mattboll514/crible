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

/// Why a request is refused, as the closed vocabulary of `details.reason`.
///
/// `docs/05-api.md` documents one value per documented case, so a client reads
/// the discriminant instead of the English message (ADR-0145, decision 2). A
/// new value is documented in `docs/05` and decided by its own ADR, in the same
/// change as the code that produces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorReason {
    /// The requested branch is disabled by the configuration of the index.
    LexicalBranchDisabled,
    /// A `delete_index` job of the name is not terminal, so the name is held
    /// until it is (ADR-0150, decision 1).
    IndexDeleting,
}

impl ErrorReason {
    /// Wire value used in `details.reason`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LexicalBranchDisabled => "lexical_branch_disabled",
            Self::IndexDeleting => "index_deleting",
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
    path: Option<String>,
    reason: Option<ErrorReason>,
    job_id: Option<String>,
}

impl CribleError {
    /// Builds an error from its category and a human-readable message.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            path: None,
            reason: None,
            job_id: None,
        }
    }

    /// Attaches the JSON path of the fault, for `details.path`.
    ///
    /// `invalid_filter` publishes that key alone; an `invalid_request`
    /// publishes it only together with a [`ErrorReason`] (ADR-0145, decision
    /// 3). The message keeps naming the path as well, so a CLI without a body
    /// still reads it.
    #[must_use]
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Attaches the reason of the refusal, for `details.reason`.
    ///
    /// A [`ErrorReason`] says why the request is refused; it is the stable
    /// discriminant a client reads, where the message is prose (ADR-0145,
    /// decisions 2 and 3). The renderer publishes that key only together with
    /// its locator: a [`with_path`](Self::with_path) for an `invalid_request`
    /// whose field is faulty, and a [`with_job_id`](Self::with_job_id) for
    /// `index_deleting` (ADR-0150, decision 1). An `invalid_request` that
    /// carries a reason without its locator publishes nothing.
    #[must_use]
    pub fn with_reason(mut self, reason: ErrorReason) -> Self {
        self.reason = Some(reason);
        self
    }

    /// Attaches the public identifier of the job a refusal refers to, for
    /// `details.job_id`.
    ///
    /// It is the `job_…` identifier of ADR-0103, never an iid (INV-01), and the
    /// renderer publishes it only with its reason: an `invalid_request` that
    /// carries a job without `index_deleting` publishes neither (ADR-0150,
    /// decision 1).
    #[must_use]
    pub fn with_job_id(mut self, job_id: impl Into<String>) -> Self {
        self.job_id = Some(job_id.into());
        self
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

    /// JSON path of the fault, when the error carries one.
    #[must_use]
    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// Reason of the refusal, when the error carries one.
    #[must_use]
    pub const fn reason(&self) -> Option<ErrorReason> {
        self.reason
    }

    /// Public identifier of the job the refusal refers to, when it carries one.
    #[must_use]
    pub fn job_id(&self) -> Option<&str> {
        self.job_id.as_deref()
    }
}
