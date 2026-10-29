// SPDX-License-Identifier: Apache-2.0

//! Error constructors shared by the modules of the server.
//!
//! Every failure of the server carries a code of
//! [`docs/05-api.md`](docs/05-api.md): a configuration fault is
//! `invalid_request`, an unexpected failure is `internal`. The library never
//! aborts, never unwraps and never expects; the binary turns a returned error
//! into a non-zero exit code.
//!
//! The JSON body of an error has one shape (`docs/05-api.md`, "Erreurs"):
//! `{"error": {"code", "message", "details", "request_id"}}`. It is built here,
//! once, so `FORMAT-ERREURS-ET-LIMITES` continues it for every code instead of
//! writing a second one. `AUTHENTIFIER-CLES` is its first user, for the `401`
//! and the `403` only.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use crible_core::error::{CribleError, ErrorCode};
use serde_json::{Value, json};

use crate::http::REQUEST_ID_HEADER;

/// Builds an [`ErrorCode::InvalidRequest`] error.
pub(crate) fn invalid(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::InvalidRequest, message.into())
}

/// Builds an [`ErrorCode::Internal`] error.
pub(crate) fn internal(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::Internal, message.into())
}

/// Builds the JSON body of an error, in the format of `docs/05-api.md`.
///
/// `details` is an object, empty when the code has nothing to add: the format
/// names the key in every case, and a caller reads it without guessing whether
/// it is present.
pub(crate) fn body(code: ErrorCode, message: &str, details: Value, request_id: &str) -> Value {
    json!({
        "error": {
            "code": code.as_str(),
            "message": message,
            "details": details,
            "request_id": request_id,
        }
    })
}

/// Builds the HTTP response of an error: the status of its code and its body.
///
/// The `x-request-id` header is set here as well: an error rendered inside the
/// router must carry it even when it is produced outside the request-id layer
/// (ADR-0098). The value is the one of the body, so the two never disagree.
pub(crate) fn response(
    code: ErrorCode,
    message: &str,
    details: Value,
    request_id: &str,
) -> Response {
    let status =
        StatusCode::from_u16(code.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response =
        (status, axum::Json(body(code, message, details, request_id))).into_response();
    if let Ok(value) = axum::http::HeaderValue::from_str(request_id) {
        response.headers_mut().insert(
            axum::http::HeaderName::from_static(REQUEST_ID_HEADER),
            value,
        );
    }
    response
}
