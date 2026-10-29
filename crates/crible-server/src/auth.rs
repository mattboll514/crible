// SPDX-License-Identifier: Apache-2.0

//! API keys, roles and the `Bearer` middleware.
//!
//! References: `docs/05-api.md` ("Conventions", roles, `401` and `403`),
//! `docs/07-stack-et-deploiement.md` (`[[auth.keys]]`), ADR-0100 (format of a key
//! and of its digest, validation, analysis of the header, comparison,
//! responses, request context and logs), ADR-0095 (the middleware and the three
//! roles are proved on a test router) and ADR-0099 (`/health` and `/ready` need
//! no key).
//!
//! A key is 32 random bytes written as 64 lowercase hexadecimal characters. The
//! server never holds it in clear: `crible.toml` carries the SHA-256 of the text
//! sent in the header, and the middleware hashes the received token before
//! comparing it to **every** declared digest with `subtle`. Nothing is cached:
//! the whole list is walked on each request, so the time does not depend on the
//! position of the matching key.
//!
//! Neither the `Authorization` header, nor the key, nor a digest is ever
//! journalised, at any level (ADR-0096, ADR-0100). The name of a recognised key
//! may appear, in the `client` field.

use std::sync::Arc;

use axum::Router;
use axum::extract::Request;
use axum::http::header::{AUTHORIZATION, HeaderValue, WWW_AUTHENTICATE};
use axum::middleware::{self, Next};
use axum::response::Response;
use crible_core::error::{CribleError, ErrorCode};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::config::{ApiKey, Role};
use crate::error;
use crate::http::RequestId;

/// Number of random bytes of a generated key.
pub const KEY_BYTES: usize = 32;

/// Length of a key and of a digest, in hexadecimal characters.
pub const HEX_LENGTH: usize = 64;

/// Message of every `401`.
///
/// The same text whether the key is absent, malformed or unknown: no response
/// tells an attacker which case it hit (ADR-0100, decision 8).
pub const UNAUTHORIZED_MESSAGE: &str = "missing or invalid API key";

/// Message of every `403`: the key is valid, its role is not enough.
pub const FORBIDDEN_MESSAGE: &str = "the API key does not grant the required role";

/// Writes bytes as lowercase hexadecimal, without prefix (ADR-0100).
#[must_use]
pub fn hex_encode(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(nibble(byte >> 4));
        text.push(nibble(byte & 0x0f));
    }
    text
}

/// One hexadecimal character, lowercase.
const fn nibble(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        _ => (b'a' + value - 10) as char,
    }
}

/// Parses a digest of 64 lowercase hexadecimal characters.
///
/// `None` on any other length, an uppercase letter or a character outside
/// `[0-9a-f]`: the format of ADR-0100 is the only accepted one.
#[must_use]
pub fn parse_digest(text: &str) -> Option<[u8; 32]> {
    let bytes = text.as_bytes();
    if bytes.len() != HEX_LENGTH {
        return None;
    }
    let mut digest = [0u8; KEY_BYTES];
    let pairs = bytes.as_chunks::<2>().0;
    for (index, pair) in pairs.iter().enumerate() {
        digest[index] = (hex_digit(pair[0])? << 4) | hex_digit(pair[1])?;
    }
    Some(digest)
}

/// One hexadecimal digit, from `0` to `15`.
fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

/// SHA-256 of the text sent in the header, as 64 lowercase hexadecimal
/// characters.
///
/// The hashed bytes are the UTF-8 bytes of the 64 characters of the key, as
/// the caller transmits them (ADR-0100, decision 2).
#[must_use]
pub fn sha256_hex(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    hex_encode(&digest[..])
}

/// Draws a key and returns it with its digest, for `crible key generate`.
///
/// # Errors
///
/// [`ErrorCode::Internal`] when the system random source fails.
pub fn generate() -> Result<(String, String), CribleError> {
    let mut bytes = [0u8; KEY_BYTES];
    getrandom::fill(&mut bytes)
        .map_err(|failure| error::internal(format!("cannot draw a random API key: {failure}")))?;
    let key = hex_encode(&bytes);
    let digest = sha256_hex(&key);
    Ok((key, digest))
}

/// Caller recognised by the middleware, attached to the request.
///
/// It feeds the `client` dimension of the analytics (`docs/06-observabilite.md`)
/// and the debug journal; it never leaves the engine as an identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Client {
    /// Name of the key that authenticated the request.
    pub name: String,
    /// Role of that key.
    pub role: Role,
}

/// Authenticator of an instance: the declared keys with their roles.
///
/// Immutable once built: no cache and no state changes on a request
/// (ADR-0100, decision 7).
pub struct Auth {
    credentials: Vec<Credential>,
}

/// One declared key, with its digest decoded once at startup.
struct Credential {
    name: String,
    role: Role,
    digest: [u8; 32],
}

impl Auth {
    /// Builds the authenticator of the declared keys.
    ///
    /// An empty list is accepted: the instance starts, and every authenticated
    /// route answers `401` (ADR-0100, decision 4).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidRequest`] when a digest is not 64 lowercase
    /// hexadecimal characters. [`Config`](crate::config::Config) refuses that
    /// case at startup; the constructor keeps the invariant of the type on its
    /// own, so no other caller can build an authenticator that would silently
    /// never match.
    pub fn new(keys: &[ApiKey]) -> Result<Self, CribleError> {
        let mut credentials = Vec::with_capacity(keys.len());
        for key in keys {
            let digest = parse_digest(&key.sha256).ok_or_else(|| {
                error::invalid(format!(
                    "key '{}': 'sha256' must be 64 lowercase hexadecimal characters \
                     (ADR-0100)",
                    key.name
                ))
            })?;
            credentials.push(Credential {
                name: key.name.clone(),
                role: key.role,
                digest,
            });
        }
        Ok(Self { credentials })
    }

    /// Recognises `token`, or returns `None` when no digest matches.
    ///
    /// Every digest is compared, without stopping at the first match: the walk
    /// does not reveal the position of the key that matched (ADR-0100,
    /// decision 7).
    fn identify(&self, token: &str) -> Option<Client> {
        let received = sha256_of(token);
        let mut matched: Option<&Credential> = None;
        for credential in &self.credentials {
            // No `break` and no early return in this loop: that absence is the
            // property ADR-0100 asks for, and a reviewer checks it here.
            if bool::from(credential.digest[..].ct_eq(&received[..])) {
                matched = Some(credential);
            }
        }
        matched.map(|credential| Client {
            name: credential.name.clone(),
            role: credential.role,
        })
    }
}

/// Journals the closed-by-default instance once (ADR-0100, decision 4).
///
/// An empty `[[auth.keys]]` is accepted: the instance starts and every
/// authenticated route answers `401`. The startup calls this function, so an
/// operator reads the consequence instead of diagnosing silent refusals. The
/// line names the table to fill and holds no secret.
pub fn warn_if_empty(keys: &[ApiKey]) {
    if keys.is_empty() {
        warn!("no [[auth.keys]] is declared: every authenticated route answers 401 (ADR-0100)");
    }
}

/// Raw SHA-256 of a token, for the comparison only.
fn sha256_of(token: &str) -> [u8; 32] {
    let digest = Sha256::digest(token.as_bytes());
    let mut bytes = [0u8; KEY_BYTES];
    bytes.copy_from_slice(&digest[..]);
    bytes
}

/// Extracts the token of one `Authorization` value.
///
/// The scheme `Bearer` is recognised in any case, then exactly one space, then
/// a non-empty token without whitespace (ADR-0100, decision 6). Anything else
/// yields `None`, which the caller renders as `401`.
#[must_use]
pub fn bearer_token(value: &HeaderValue) -> Option<&str> {
    let text = value.to_str().ok()?;
    let (scheme, token) = text.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return None;
    }
    if token.is_empty() || token.chars().any(char::is_whitespace) {
        return None;
    }
    Some(token)
}

/// Identifies the caller of `request`.
///
/// `None` for an absent header, a repeated one, a malformed value and an
/// unknown key: the four cases share the `401` of ADR-0100.
fn identified(auth: &Auth, request: &Request) -> Option<Client> {
    let mut values = request.headers().get_all(AUTHORIZATION).iter();
    let first = values.next()?;
    if values.next().is_some() {
        return None;
    }
    let token = bearer_token(first)?;
    auth.identify(token)
}

/// Protects every route added so far with at least `role`.
///
/// Mount the call after the routes of one level:
/// `protect(router.route("/v1/search", post(search)), auth, Role::Read)`. The
/// layer identifies the caller, attaches its [`Client`] in the request
/// extensions, then compares the role. `/health`, `/ready`, `/openapi.json` and
/// `/docs` are never wrapped: `docs/05-api.md` gives them the role "—"
/// (ADR-0099).
pub fn protect<S>(router: Router<S>, auth: Arc<Auth>, role: Role) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router.route_layer(middleware::from_fn(move |request: Request, next: Next| {
        let auth = Arc::clone(&auth);
        async move { authorize(&auth, role, request, next).await }
    }))
}

/// Runs the middleware: identify, attach, then check the role.
async fn authorize(auth: &Auth, required: Role, mut request: Request, next: Next) -> Response {
    let identifier = identifier_of(&request);
    let Some(client) = identified(auth, &request) else {
        return refusal(ErrorCode::Unauthorized, UNAUTHORIZED_MESSAGE, &identifier);
    };
    // The name is the one `docs/06` allows in the `client` field; neither the
    // header nor the key nor the digest reaches a field of this line.
    debug!(client = %client.name, "the request is authenticated");
    if !client.role.grants(required) {
        return refusal(ErrorCode::Forbidden, FORBIDDEN_MESSAGE, &identifier);
    }
    request.extensions_mut().insert(client);
    next.run(request).await
}

/// Identifier of the request, or a fresh one when the identifier layer is not
/// mounted: the body and the header never disagree (ADR-0098).
fn identifier_of(request: &Request) -> String {
    request
        .extensions()
        .get::<RequestId>()
        .map_or_else(|| Uuid::new_v4().to_string(), |id| id.0.clone())
}

/// Builds an error response of the middleware.
///
/// The body is the one of `docs/05-api.md`, identical for every cause of the
/// same code. `401` carries `WWW-Authenticate: Bearer`; `403` does not, because
/// the caller is authenticated and asking for credentials again would change
/// nothing (ADR-0100, decision 8).
fn refusal(code: ErrorCode, message: &str, identifier: &str) -> Response {
    let mut response = error::response(code, message, Value::Object(Map::new()), identifier);
    if code == ErrorCode::Unauthorized {
        response
            .headers_mut()
            .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::{
        Auth, Client, HEX_LENGTH, Role, bearer_token, hex_encode, parse_digest, protect, sha256_hex,
    };
    use crate::config::ApiKey;
    use crate::http::with_common_layers;
    use axum::Router;
    use axum::body::Body;
    use axum::extract::Request;
    use axum::http::header::{AUTHORIZATION, HeaderValue};
    use axum::http::{StatusCode, header::WWW_AUTHENTICATE};
    use axum::response::Response;
    use axum::routing::get;
    use std::sync::Arc;
    use tower::ServiceExt;

    const READ_KEY: &str = "0000000000000000000000000000000000000000000000000000000000000001";
    const WRITE_KEY: &str = "0000000000000000000000000000000000000000000000000000000000000002";
    const ADMIN_KEY: &str = "0000000000000000000000000000000000000000000000000000000000000003";

    fn key(name: &str, role: Role, token: &str) -> ApiKey {
        ApiKey {
            name: name.to_owned(),
            role,
            sha256: sha256_hex(token),
        }
    }

    fn auth() -> Arc<Auth> {
        Arc::new(
            Auth::new(&[
                key("reader", Role::Read, READ_KEY),
                key("writer", Role::Write, WRITE_KEY),
                key("administrator", Role::Admin, ADMIN_KEY),
            ])
            .expect("the three keys hold a valid digest"),
        )
    }

    #[test]
    fn a_key_is_hexadecimal_of_the_documented_length() {
        // ADR-0100, decision 2: 32 bytes, 64 lowercase characters, no prefix.
        let (key, digest) = super::generate().expect("the system random source answers");
        assert_eq!(key.len(), HEX_LENGTH);
        assert_eq!(digest.len(), HEX_LENGTH);
        assert!(
            key.bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "got {key}"
        );
        assert_eq!(digest, sha256_hex(&key), "the digest is the one of the key");
        let (other, _) = super::generate().expect("the system random source answers");
        assert_ne!(key, other, "two keys are drawn independently");
    }

    #[test]
    fn only_the_documented_digest_format_parses() {
        assert_eq!(
            parse_digest(&"ab".repeat(32)).map(|digest| digest[0]),
            Some(0xab)
        );
        for refused in [
            "ab".repeat(31),
            "ab".repeat(33),
            "AB".repeat(32),
            format!("{}g", "ab".repeat(31)),
            format!("0x{}", "ab".repeat(31)),
            " key".to_owned(),
            String::new(),
        ] {
            assert!(parse_digest(&refused).is_none(), "refused: {refused:?}");
        }
        assert_eq!(hex_encode(&[0x00, 0x0f, 0xa5]), "000fa5");
    }

    #[test]
    fn the_header_is_analysed_as_adr_0100_says() {
        // Decision 6: scheme insensitive to case, one single space, a
        // non-empty token without whitespace.
        for accepted in ["Bearer abc", "bearer abc", "BEARER abc", "BeArEr abc"] {
            let header = HeaderValue::from_str(accepted).expect("the value is a header");
            assert_eq!(bearer_token(&header), Some("abc"), "accepted: {accepted:?}");
        }
        for refused in [
            "Bearer",
            "Bearer ",
            "Bearer  abc",
            "Bearer\tabc",
            "Basic abc",
            "abc",
            "Bearer abc def",
            "Bearer abc\tdef",
        ] {
            let header = HeaderValue::from_str(refused).expect("the value is a header");
            assert!(bearer_token(&header).is_none(), "refused: {refused:?}");
        }
    }

    #[test]
    fn every_declared_key_authenticates_in_turn() {
        // ADR-0100, decision 7: the walk covers the whole list, so the first,
        // the middle and the last declared key are all recognised.
        let auth = auth();
        for token in [READ_KEY, WRITE_KEY, ADMIN_KEY] {
            let client = auth
                .identify(token)
                .unwrap_or_else(|| panic!("key {token} must be recognised"));
            assert_eq!(client, expected_client(token));
        }
        assert!(auth.identify("unknown").is_none());
        Auth::new(&[]).expect("an empty list is accepted");
    }

    #[test]
    fn a_duplicate_digest_does_not_stop_the_walk() {
        // ADR-0100, decision 7: every digest is compared. Two keys sharing a
        // digest are refused by `Config` but accepted by the constructor; the
        // second must win, which a `break` after the first match would prevent.
        let auth = Auth::new(&[
            key("first", Role::Read, READ_KEY),
            key("second", Role::Write, READ_KEY),
        ])
        .expect("the constructor keeps no uniqueness rule");
        let client = auth.identify(READ_KEY).expect("the digest is declared");
        assert_eq!(client.name, "second");
        assert_eq!(client.role, Role::Write);
    }

    #[test]
    fn a_digest_outside_the_format_is_refused_by_the_constructor() {
        let bad = ApiKey {
            name: "broken".to_owned(),
            role: Role::Read,
            sha256: "not-a-digest".to_owned(),
        };
        let failure = Auth::new(&[bad]).err().expect("the digest is refused");
        assert_eq!(
            failure.code(),
            crible_core::error::ErrorCode::InvalidRequest
        );
        assert!(failure.message().contains("broken"), "names the key");
    }

    fn expected_client(token: &str) -> Client {
        match token {
            READ_KEY => Client {
                name: "reader".to_owned(),
                role: Role::Read,
            },
            WRITE_KEY => Client {
                name: "writer".to_owned(),
                role: Role::Write,
            },
            _ => Client {
                name: "administrator".to_owned(),
                role: Role::Admin,
            },
        }
    }

    async fn handler(request: Request) -> Response {
        match request.extensions().get::<Client>().cloned() {
            Some(client) => Response::builder()
                .status(StatusCode::OK)
                .body(Body::from(format!(
                    "{} {}",
                    client.name,
                    client.role.as_str()
                )))
                .unwrap_or_else(|error| panic!("the test response is well formed: {error}")),
            None => Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::empty())
                .unwrap_or_else(|error| panic!("the test response is well formed: {error}")),
        }
    }

    fn router() -> Router {
        let auth = auth();
        let read = protect(
            Router::new().route("/read", get(handler)),
            Arc::clone(&auth),
            Role::Read,
        );
        let write = protect(
            Router::new().route("/write", get(handler)),
            Arc::clone(&auth),
            Role::Write,
        );
        let admin = protect(
            Router::new().route("/admin", get(handler)),
            auth,
            Role::Admin,
        );
        with_common_layers(read.merge(write).merge(admin))
    }

    async fn call(path: &str, authorization: Option<&str>) -> Response {
        let mut request = Request::builder()
            .uri(path)
            .header(crate::http::REQUEST_ID_HEADER, "test-1");
        if let Some(value) = authorization {
            request = request.header(AUTHORIZATION, value);
        }
        router()
            .oneshot(
                request
                    .body(Body::empty())
                    .expect("the request is well formed"),
            )
            .await
            .expect("the router answers")
    }

    async fn json_body(response: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("the body is readable");
        serde_json::from_slice(&bytes).expect("the body is JSON")
    }

    #[tokio::test]
    async fn each_role_reaches_its_level() {
        // `write` contains `read`, `admin` contains `write` (`docs/05-api.md`).
        for token in [READ_KEY, WRITE_KEY, ADMIN_KEY] {
            let response = call("/read", Some(&format!("Bearer {token}"))).await;
            assert_eq!(response.status(), StatusCode::OK, "read with {token}");
        }
        for token in [WRITE_KEY, ADMIN_KEY] {
            let response = call("/write", Some(&format!("Bearer {token}"))).await;
            assert_eq!(response.status(), StatusCode::OK, "write with {token}");
        }
        let response = call("/admin", Some(&format!("Bearer {ADMIN_KEY}"))).await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "admin with the admin key"
        );

        // A lower role receives `403` above its level, never a `401`.
        for (path, token) in [
            ("/write", READ_KEY),
            ("/admin", READ_KEY),
            ("/admin", WRITE_KEY),
        ] {
            let response = call(path, Some(&format!("Bearer {token}"))).await;
            assert_eq!(
                response.status(),
                StatusCode::FORBIDDEN,
                "{token} on {path}"
            );
            assert!(
                response.headers().get(WWW_AUTHENTICATE).is_none(),
                "a 403 asks for no credential (ADR-0100)"
            );
            let body = json_body(response).await;
            assert_eq!(body["error"]["code"], "forbidden");
            assert_eq!(body["error"]["request_id"], "test-1");
            assert!(body["error"]["details"].is_object());
        }
    }

    #[tokio::test]
    async fn the_client_is_attached_to_the_request() {
        let response = call("/write", Some(&format!("Bearer {WRITE_KEY}"))).await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("the body is readable");
        assert_eq!(String::from_utf8_lossy(&bytes), "writer write");
    }

    #[tokio::test]
    async fn the_three_causes_of_a_401_share_one_body() {
        // ADR-0100, decision 8: absent, malformed and unknown give the same
        // body; the request identifier is pinned so the comparison is exact.
        let mut bodies = Vec::new();
        for authorization in [
            None,
            Some("Basic abc"),
            Some("Bearer"),
            Some("Bearer "),
            Some("Bearer  abc"),
            Some("Bearer unknown"),
        ] {
            let response = call("/read", authorization).await;
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{authorization:?}"
            );
            assert_eq!(
                response.headers().get(WWW_AUTHENTICATE),
                Some(&HeaderValue::from_static("Bearer")),
                "a 401 asks for a Bearer credential"
            );
            assert_eq!(
                response
                    .headers()
                    .get(crate::http::REQUEST_ID_HEADER)
                    .and_then(|value| value.to_str().ok()),
                Some("test-1")
            );
            bodies.push(json_body(response).await);
        }
        assert_eq!(bodies[0], bodies[1]);
        assert_eq!(bodies[1], bodies[2]);
        assert_eq!(bodies[2], bodies[3]);
        assert_eq!(bodies[3], bodies[4]);
        assert_eq!(bodies[4], bodies[5]);
        assert_eq!(bodies[0]["error"]["code"], "unauthorized");
        assert_eq!(bodies[0]["error"]["request_id"], "test-1");
        assert!(bodies[0]["error"]["details"].is_object());
    }

    #[tokio::test]
    async fn a_repeated_header_is_refused() {
        // Decision 6: an `Authorization` present several times yields `401`.
        let request = Request::builder()
            .uri("/read")
            .header(AUTHORIZATION, format!("Bearer {READ_KEY}"))
            .header(AUTHORIZATION, format!("Bearer {WRITE_KEY}"))
            .body(Body::empty())
            .expect("the request is well formed");
        let router = protect(
            Router::new().route("/read", get(handler)),
            auth(),
            Role::Read,
        );
        let response = router.oneshot(request).await.expect("the router answers");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn an_empty_key_list_refuses_every_key() {
        // ADR-0100, decision 4: the closed-by-default instance starts and every
        // authenticated route answers `401`.
        let router = protect(
            Router::new().route("/read", get(handler)),
            Arc::new(Auth::new(&[]).expect("an empty list is accepted")),
            Role::Read,
        );
        let request = Request::builder()
            .uri("/read")
            .header(AUTHORIZATION, format!("Bearer {READ_KEY}"))
            .body(Body::empty())
            .expect("the request is well formed");
        let response = router.oneshot(request).await.expect("the router answers");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn a_role_grants_its_level_and_below_only() {
        // `admin` contains `write`, which contains `read` (`docs/05-api.md`).
        assert!(Role::Read.grants(Role::Read));
        assert!(!Role::Read.grants(Role::Write));
        assert!(!Role::Read.grants(Role::Admin));
        assert!(Role::Write.grants(Role::Read));
        assert!(Role::Write.grants(Role::Write));
        assert!(!Role::Write.grants(Role::Admin));
        assert!(Role::Admin.grants(Role::Read));
        assert!(Role::Admin.grants(Role::Write));
        assert!(Role::Admin.grants(Role::Admin));
    }
}
