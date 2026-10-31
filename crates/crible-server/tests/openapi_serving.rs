// SPDX-License-Identifier: Apache-2.0

// Tests may panic; `allow-panic-in-tests` covers `#[test]` functions only, and
// the helpers of this file are assertions.
#![allow(clippy::expect_used, clippy::panic)]

//! The specification and Swagger UI are served by the instance, without a key.
//!
//! References: `docs/05-api.md` (the two publication routes and the security
//! scheme), ADR-0015 (the specification is generated from the code), ADR-0100
//! (the API key) and ADR-0111, decisions 2, 3 and 4 (`GET /openapi.json`,
//! `GET /docs`, OpenAPI 3.1.0 and the absence of `servers`).

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use crible_server::http::build_router;
use serde_json::{Value, json};
use tower::ServiceExt;

/// Sends one request without any `Authorization` header.
async fn get(router: &axum::Router, path: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .body(Body::empty())
                .expect("the request is valid"),
        )
        .await
        .expect("the router answers");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 8 << 20)
        .await
        .expect("the body is readable")
        .to_vec();
    (status, headers, bytes)
}

#[tokio::test]
async fn the_specification_is_served_without_a_key() {
    let instance = common::Instance::new();
    let router = build_router(instance.state());
    let (status, headers, bytes) = get(&router, "/openapi.json").await;
    assert_eq!(status, StatusCode::OK, "`GET /openapi.json` answers 200");
    assert!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("application/json")),
        "the specification is JSON"
    );
    let specification: Value =
        serde_json::from_slice(&bytes).expect("the served specification is JSON");

    assert_eq!(specification["openapi"], "3.1.0", "OpenAPI 3.1.0");
    assert_eq!(
        specification["info"]["version"],
        env!("CARGO_PKG_VERSION"),
        "the version of the package"
    );
    assert!(
        specification.get("servers").is_none(),
        "the document declares no server (ADR-0111)"
    );
    assert_eq!(
        specification["components"]["securitySchemes"]["bearerAuth"]["type"], "http",
        "the scheme is an HTTP authentication"
    );
    assert_eq!(
        specification["components"]["securitySchemes"]["bearerAuth"]["scheme"], "bearer",
        "the scheme is `bearer`"
    );
    assert_eq!(
        specification["security"],
        json!([{ "bearerAuth": [] }]),
        "the whole document requires the key"
    );
    // The operation cancels the global requirement; `utoipa` renders
    // `security(())` as `[{}]`, one requirement that names no scheme, the shape
    // ADR-0111 keeps.
    for path in ["/health", "/ready"] {
        let requirements = specification["paths"][path]["get"]["security"]
            .as_array()
            .unwrap_or_else(|| panic!("`GET {path}` cancels the global requirement"));
        assert!(!requirements.is_empty(), "`GET {path}` needs no key");
        for requirement in requirements {
            assert_eq!(requirement, &json!({}), "`GET {path}` requires no scheme");
        }
    }
    assert!(
        specification["paths"]["/v1/indexes"]["post"].is_object(),
        "the routes are in the specification"
    );
}

#[tokio::test]
async fn swagger_ui_is_served_without_a_key() {
    let instance = common::Instance::new();
    let router = build_router(instance.state());

    // The crate mounts the page on `/docs/` and redirects `/docs` to it.
    let (status, headers, _) = get(&router, "/docs").await;
    assert!(
        status.is_redirection(),
        "`GET /docs` redirects to the page, without a key: {status}"
    );
    assert_eq!(
        headers
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/docs/"),
        "the redirect targets the page"
    );

    let (status, headers, bytes) = get(&router, "/docs/").await;
    assert_eq!(status, StatusCode::OK, "`GET /docs/` answers 200");
    assert!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/html")),
        "the page is HTML"
    );
    let page = String::from_utf8(bytes).expect("the page is UTF-8");
    assert!(
        page.contains("swagger-ui"),
        "the page is the Swagger UI bundle"
    );

    // The page reads the specification through its initialiser, which the
    // crate builds from the URL given to `SwaggerUi::url`.
    let (status, _, bytes) = get(&router, "/docs/swagger-initializer.js").await;
    assert_eq!(status, StatusCode::OK, "the initialiser is served");
    let initialiser = String::from_utf8(bytes).expect("the initialiser is UTF-8");
    assert!(
        initialiser.contains("/openapi.json"),
        "the page reads the served specification: {initialiser}"
    );
}
