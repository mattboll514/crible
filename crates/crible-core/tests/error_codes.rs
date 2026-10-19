// SPDX-License-Identifier: Apache-2.0

//! Correspondence between `ErrorCode` and the HTTP status of the API.
//!
//! Reference: `docs/05-api.md`, section "Erreurs" (the table is normative).

use crible_core::error::{CribleError, ErrorCode};

/// `(variant, HTTP status, wire name)`, copied from `docs/05-api.md`.
const DOCUMENTED: &[(ErrorCode, u16, &str)] = &[
    (ErrorCode::InvalidRequest, 400, "invalid_request"),
    (ErrorCode::InvalidFilter, 400, "invalid_filter"),
    (ErrorCode::Unauthorized, 401, "unauthorized"),
    (ErrorCode::Forbidden, 403, "forbidden"),
    (ErrorCode::UnknownIndex, 404, "unknown_index"),
    (ErrorCode::NotFound, 404, "not_found"),
    (ErrorCode::ImmutableSetting, 409, "immutable_setting"),
    (ErrorCode::BatchConflict, 409, "batch_conflict"),
    (ErrorCode::PayloadTooLarge, 413, "payload_too_large"),
    (ErrorCode::IncompatibleModel, 422, "incompatible_model"),
    (ErrorCode::AcceptanceFailed, 422, "acceptance_failed"),
    (ErrorCode::Overloaded, 503, "overloaded"),
    (ErrorCode::Timeout, 504, "timeout"),
    (ErrorCode::Internal, 500, "internal"),
];

#[test]
fn error_code_http_status_matches_api_doc() {
    for (code, status, _) in DOCUMENTED {
        assert_eq!(
            code.http_status(),
            *status,
            "wrong HTTP status for `{}`",
            code.as_str()
        );
    }
}

#[test]
fn error_code_wire_name_matches_api_doc() {
    for (code, _, name) in DOCUMENTED {
        assert_eq!(code.as_str(), *name, "wrong wire name for {code:?}");
    }
}

#[test]
fn error_code_all_lists_every_documented_code_once() {
    assert_eq!(
        ErrorCode::ALL.len(),
        DOCUMENTED.len(),
        "`ErrorCode::ALL` and `docs/05-api.md` disagree on the number of codes"
    );
    for (code, _, name) in DOCUMENTED {
        let occurrences = ErrorCode::ALL
            .iter()
            .filter(|candidate| *candidate == code)
            .count();
        assert_eq!(
            occurrences, 1,
            "`{name}` must appear exactly once in `ErrorCode::ALL`"
        );
    }
}

#[test]
fn every_error_code_has_a_client_or_server_status() {
    for code in ErrorCode::ALL {
        let status = code.http_status();
        assert!(
            (400..600).contains(&status),
            "`{}` maps to {status}, which is not an error status",
            code.as_str()
        );
    }
}

#[test]
fn crible_error_implements_the_standard_error_trait() {
    fn assert_error<T: std::error::Error + std::fmt::Display + std::fmt::Debug>() {}
    assert_error::<CribleError>();
}
