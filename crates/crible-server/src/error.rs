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
//! once, for every code of the table. `AUTHENTIFIER-CLES` uses it for the `401`
//! and the `403`; `FORMAT-ERREURS-ET-LIMITES` continues it for a [`CribleError`]
//! and for the responses that no route renders: an unknown route, a method that
//! no route of the path accepts, a rejection of an extractor, a body above the
//! bound of `docs/05-api.md` and a panic (ADR-0101, ADR-0102).
//!
//! Two rules hold for every body built here. The message of an `internal` is a
//! fixed generic text, never the message of the failure: an unexpected failure
//! names a server path or an internal structure. That cause is not lost: it
//! travels in the extension of the response, as a [`Cause`], and the line of
//! the `500` journals it under the `request_id` (ADR-0101, decision 4). And no
//! message of a `4xx` is journalised: it may quote a value the client sent,
//! which `docs/06` and ADR-0096 forbid in a log at any level.

use std::any::Any;

use axum::body::Body;
use axum::extract::Request;
use axum::http::header::RETRY_AFTER;
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use crible_core::error::{CribleError, ErrorCode};
use serde_json::{Value, json};
use tracing::{error, warn};
use uuid::Uuid;

use crate::auth::{FORBIDDEN_MESSAGE, UNAUTHORIZED_MESSAGE};
use crate::http::{REQUEST_ID_HEADER, RequestId};
use crate::limits::{OVERLOADED_MESSAGE, RETRY_AFTER_SECONDS};

/// Fixed message of every `internal` response (ADR-0101, decision 2).
pub const INTERNAL_MESSAGE: &str = "internal error; see the server log for this request_id";

/// Message of a body that could not be read or parsed (ADR-0101, decision 6).
pub const INVALID_REQUEST_MESSAGE: &str = "the request body or its parameters are invalid";

/// Message of a body above the bound of the instance (ADR-0102, decision 1).
///
/// The bound itself is [`crate::limits::MAX_BODY_BYTES`] and `docs/05-api.md`;
/// the message does not repeat the number, so it stays true when a test mounts
/// the layers with a smaller bound.
pub const PAYLOAD_TOO_LARGE_MESSAGE: &str =
    "the request body exceeds the size limit of the instance";

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
///
/// Every `overloaded` carries `Retry-After: 1`, whatever produced it: the search
/// limit, the write queue, a server in drainage, a SQLite lock, the lexical
/// branch (ADR-0103, decision 9). The rule is here and nowhere else, so a new
/// producer of `overloaded` inherits it without a decision; the `503` of
/// `/ready` is not an `overloaded` and never comes through this function
/// (ADR-0099).
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
    response.extensions_mut().insert(Rendered(code));
    if code == ErrorCode::Overloaded {
        response.headers_mut().insert(
            RETRY_AFTER,
            axum::http::HeaderValue::from(RETRY_AFTER_SECONDS),
        );
    }
    if let Ok(value) = axum::http::HeaderValue::from_str(request_id) {
        response.headers_mut().insert(
            axum::http::HeaderName::from_static(REQUEST_ID_HEADER),
            value,
        );
    }
    response
}

/// Renders a [`CribleError`] in the format of `docs/05-api.md`.
///
/// The message is the one of the error, except for `internal`, which keeps
/// [`INTERNAL_MESSAGE`]: an unexpected failure names a server path or an
/// internal structure (ADR-0101, decision 2). `details` is empty, except
/// `path` for the `invalid_filter` error that carries one: `docs/05` documents
/// that key for that code only (decision 3, ADR-0109, ADR-0110).
///
/// The message of an `internal` is not lost: it travels as a [`Cause`] in the
/// extension of the response, and [`render_layer_errors`] journals it under the
/// `request_id` (decision 4). The client receives the generic text and the
/// identifier; the operator receives the cause.
pub(crate) fn rendered(failure: &CribleError, request_id: &str) -> Response {
    let code = failure.code();
    let message = match code {
        ErrorCode::Internal => INTERNAL_MESSAGE,
        _ => failure.message(),
    };
    let details = match (code, failure.path()) {
        (ErrorCode::InvalidFilter, Some(path)) => json!({ "path": path }),
        _ => json!({}),
    };
    let mut answer = response(code, message, details, request_id);
    if code == ErrorCode::Internal {
        answer
            .extensions_mut()
            .insert(Cause(failure.message().to_owned()));
    }
    answer
}

/// Fallback of every path the router does not know (ADR-0101, decision 5).
///
/// A method that no route of an existing path accepts renders the same body:
/// axum answers a `405` without a body, and [`render_layer_errors`] turns it
/// into the same `404 not_found`, so no `405` ever leaves the server.
pub(crate) async fn unknown_route(request: Request) -> Response {
    let identifier = identifier(&request);
    let message = no_route(request.method(), request.uri().path());
    response(ErrorCode::NotFound, &message, json!({}), &identifier)
}

/// Marks a response whose body the formatting layer must not touch.
///
/// The `503` of `/ready` is the one response of the server whose body is not an
/// error of `docs/05-api.md` (ADR-0099, ADR-0101 decision 1). It is also not an
/// error for the line of decision 4: a not-ready instance answers its state,
/// and the probe is not a refusal.
pub(crate) fn preserved(mut answer: Response) -> Response {
    answer.extensions_mut().insert(Preserved);
    answer
}

/// Turns the response of another layer into the format of `docs/05-api.md`.
///
/// The body bound (`413`), a rejection of an extractor (`400`, `415`, `422`)
/// and a panic (`500`) are rendered outside the routes that know the format
/// (ADR-0101, decisions 6 and 7; ADR-0102, decision 1). A response that already
/// carries [`Rendered`] keeps its body: the line is the only thing added.
///
/// Every error response produces exactly one line, with its code, its status
/// and its `request_id`. The message is journalised for `internal` only, and it
/// comes from [`Cause`]: [`rendered`] attaches the message of an `internal`
/// there, [`panic_response`] attaches the payload of a panic. The text of any
/// other layer body is dropped without being read (ADR-0101, decision 4).
pub(crate) async fn render_layer_errors(request: Request, next: Next) -> Response {
    let identifier = identifier(&request);
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let answer = next.run(request).await;
    let status = answer.status();
    if answer.extensions().get::<Preserved>().is_some()
        || (!status.is_client_error() && !status.is_server_error())
    {
        return answer;
    }

    let cause = answer
        .extensions()
        .get::<Cause>()
        .map(|cause| cause.0.clone());
    if let Some(Rendered(code)) = answer.extensions().get::<Rendered>().copied() {
        log_refusal(code, status, &identifier, cause.as_deref());
        return answer;
    }

    // The body of this intermediate response is dropped without being read:
    // it may quote a value the client sent, which `docs/06` and ADR-0096
    // forbid in a log at any level. Only the cause of an `internal` is kept, in
    // the extension of the response (ADR-0101, decision 4).
    let (code, message) = from_status(status, &method, &path);
    let cause = if code == ErrorCode::Internal {
        cause
    } else {
        None
    };
    log_refusal(code, status, &identifier, cause.as_deref());
    response(code, &message, json!({}), &identifier)
}

/// Renders the `500` of a panicking handler (ADR-0101, decision 7).
///
/// The cause is attached to this intermediate response as a [`Cause`] and
/// never written in its body: [`render_layer_errors`] journalises it under the
/// `request_id` of the request, then replaces the whole response with the
/// generic `500` of [`INTERNAL_MESSAGE`]. Without that layer the cause is
/// journalised nowhere and the client reads an empty `500`.
pub(crate) fn panic_response(cause: Box<dyn Any + Send + 'static>) -> Response {
    let mut answer = Response::new(Body::empty());
    *answer.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    answer
        .extensions_mut()
        .insert(Cause(cause_text(cause.as_ref())));
    answer
}

/// Identifies the request, or draws one when the identifier layer is absent.
fn identifier(request: &Request) -> String {
    request
        .extensions()
        .get::<RequestId>()
        .map_or_else(|| Uuid::new_v4().to_string(), |id| id.0.clone())
}

/// The message ADR-0101, decision 5, fixes for an unknown route.
fn no_route(method: &Method, path: &str) -> String {
    format!("no route for {method} {path}")
}

/// Code and message of a response that no route rendered.
///
/// The statuses are the ones axum renders by itself: `400`, `415` and `422` of
/// an extractor are all `invalid_request`, because `docs/05` has no code for
/// `415` and gives `422` another meaning (ADR-0101, decision 6). Any other
/// client error is `invalid_request` too. A `503` that no route rendered is
/// `overloaded`, because `docs/05` gives that status to a transient contention
/// and an overload must not become an `internal`; the `Retry-After: 1` of
/// ADR-0103 is added by [`response`], like for every producer of `overloaded`;
/// any other server error is `internal`: no route produces them through this
/// path, which is a last resort that keeps the format instead of leaking a
/// body.
fn from_status(status: StatusCode, method: &Method, path: &str) -> (ErrorCode, String) {
    match status {
        StatusCode::BAD_REQUEST
        | StatusCode::UNSUPPORTED_MEDIA_TYPE
        | StatusCode::UNPROCESSABLE_ENTITY => (
            ErrorCode::InvalidRequest,
            INVALID_REQUEST_MESSAGE.to_owned(),
        ),
        StatusCode::UNAUTHORIZED => (ErrorCode::Unauthorized, UNAUTHORIZED_MESSAGE.to_owned()),
        StatusCode::FORBIDDEN => (ErrorCode::Forbidden, FORBIDDEN_MESSAGE.to_owned()),
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED => {
            (ErrorCode::NotFound, no_route(method, path))
        }
        StatusCode::PAYLOAD_TOO_LARGE => (
            ErrorCode::PayloadTooLarge,
            PAYLOAD_TOO_LARGE_MESSAGE.to_owned(),
        ),
        StatusCode::SERVICE_UNAVAILABLE => (ErrorCode::Overloaded, OVERLOADED_MESSAGE.to_owned()),
        _ if status.is_server_error() => (ErrorCode::Internal, INTERNAL_MESSAGE.to_owned()),
        _ => (
            ErrorCode::InvalidRequest,
            INVALID_REQUEST_MESSAGE.to_owned(),
        ),
    }
}

/// Writes the one line of an error response (ADR-0101, decision 4).
fn log_refusal(code: ErrorCode, status: StatusCode, identifier: &str, cause: Option<&str>) {
    if code == ErrorCode::Internal {
        error!(
            code = code.as_str(),
            status = status.as_u16(),
            request_id = %identifier,
            message = cause.unwrap_or(INTERNAL_MESSAGE),
            "request failed"
        );
    } else {
        warn!(
            code = code.as_str(),
            status = status.as_u16(),
            request_id = %identifier,
            "request refused"
        );
    }
}

/// Text of a panic payload, for the line of an `internal`.
fn cause_text(cause: &(dyn Any + Send)) -> String {
    if let Some(text) = cause.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = cause.downcast_ref::<String>() {
        text.clone()
    } else {
        "a handler panicked with a payload that is not a text".to_owned()
    }
}

/// Code of a response already written in the format of `docs/05-api.md`.
///
/// The formatting layer reads it: such a response keeps its body, and only gets
/// its line (ADR-0101, decision 4).
#[derive(Debug, Clone, Copy)]
struct Rendered(ErrorCode);

/// Cause of an `internal`, kept for the line of the journal and never sent to
/// the client.
///
/// Two producers attach it: [`rendered`], with the message of a
/// [`CribleError`] whose code is [`ErrorCode::Internal`], and
/// [`panic_response`], with the payload of the panic (ADR-0101, decisions 4
/// and 7). It lives in the extension of the response: without that layer it is
/// dropped with the response, and no body ever carries it.
#[derive(Debug, Clone)]
struct Cause(String);

/// Marker of a response the formatting layer leaves untouched.
///
/// Only the `503` of `/ready` carries it (ADR-0101, decision 1).
#[derive(Debug, Clone, Copy)]
struct Preserved;

#[cfg(test)]
mod tests {
    use super::{INTERNAL_MESSAGE, Rendered, rendered};
    use axum::response::Response;
    use crible_core::error::{CribleError, ErrorCode};
    use serde_json::{Value, json};

    /// Decodes the JSON body of a response.
    async fn json_body(response: Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("the body is readable");
        serde_json::from_slice(&bytes).expect("the body is JSON")
    }

    #[tokio::test]
    async fn every_code_of_the_table_renders_its_status_and_body() {
        // One test per reachable code of `docs/05-api.md`: the status of the
        // table, the wire name, the four keys, an empty `details` and the
        // generic message of an `internal` (ADR-0101, decisions 1 to 3).
        let mut names = Vec::new();
        for code in ErrorCode::ALL {
            let failure = CribleError::new(code, "a message of the failure");
            let answer = rendered(&failure, "request-1");

            assert_eq!(
                answer.status().as_u16(),
                code.http_status(),
                "{} has the status of the table",
                code.as_str()
            );
            assert_eq!(
                answer
                    .headers()
                    .get(super::REQUEST_ID_HEADER)
                    .and_then(|value| value.to_str().ok()),
                Some("request-1"),
                "{} carries the identifier in its header",
                code.as_str()
            );
            assert!(
                answer.extensions().get::<Rendered>().is_some(),
                "{} is marked as rendered",
                code.as_str()
            );

            let body = json_body(answer).await;
            let object = body["error"]
                .as_object()
                .unwrap_or_else(|| panic!("{} names an error: {body}", code.as_str()));
            let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
            keys.sort_unstable();
            assert_eq!(
                keys,
                ["code", "details", "message", "request_id"],
                "{} has the four keys of the format",
                code.as_str()
            );
            assert_eq!(body["error"]["code"], code.as_str());
            assert_eq!(body["error"]["request_id"], "request-1");
            assert_eq!(body["error"]["details"], json!({}));
            let expected = if code == ErrorCode::Internal {
                INTERNAL_MESSAGE
            } else {
                failure.message()
            };
            assert_eq!(
                body["error"]["message"],
                expected,
                "{} renders its message",
                code.as_str()
            );
            assert!(
                !names.contains(&code.as_str()),
                "{} is the first code to render that name",
                code.as_str()
            );
            names.push(code.as_str());
        }
        assert_eq!(names.len(), 14, "the table holds fourteen codes");
        assert!(
            !INTERNAL_MESSAGE.contains("a message of the failure"),
            "the generic message hides the cause"
        );
    }
}
