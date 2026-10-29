// SPDX-License-Identifier: Apache-2.0

//! Error constructors shared by the modules of the server.
//!
//! Every failure of the server carries a code of
//! [`docs/05-api.md`](docs/05-api.md): a configuration fault is
//! `invalid_request`, an unexpected failure is `internal`. The library never
//! aborts, never unwraps and never expects; the binary turns a returned error
//! into a non-zero exit code.

use crible_core::error::{CribleError, ErrorCode};

/// Builds an [`ErrorCode::InvalidRequest`] error.
pub(crate) fn invalid(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::InvalidRequest, message.into())
}

/// Builds an [`ErrorCode::Internal`] error.
pub(crate) fn internal(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::Internal, message.into())
}
