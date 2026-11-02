// SPDX-License-Identifier: Apache-2.0

//! API mode of `crible eval`: one instance, one game, the same metrics.
//!
//! References: ADR-0131 (the interface this module implements), ADR-0125 (the
//! local mode it must stay comparable with), `docs/05-api.md` ("Recherche :
//! requête", "Recherche : réponse", "Erreurs") and `docs/08` (`INV-13`).
//!
//! # What this module owns
//!
//! - the **base URL** of `--url`: plain HTTP/1.1 only, no TLS, and every
//!   component the command does not accept is refused **by its name**, before
//!   any I/O (decision 3);
//! - the **key**: read once in `CRIBLE_EVAL_API_KEY`, sent in
//!   `Authorization: Bearer` and nowhere else, never printed, never logged
//!   (decision 4; `INV-13`);
//! - the **generation**: pinned by `GET /v1/indexes/{index}` and checked on
//!   every answer (decision 10);
//! - the **client**: `hyper` on a `current_thread` tokio runtime, with the two
//!   fixed deadlines of decision 5.
//!
//! # What it does not own
//!
//! The metrics. `eval` computes fidelity and quality from the keys and the
//! documents this module returns, with the functions of the local mode, so the
//! two reports cannot drift apart (decisions 7 and 8).
//!
//! # Confidentiality
//!
//! No message of this module carries a query text, a filter value or the key
//! (`INV-13`): a failure names the instance as `http://host:port`, the `id` of
//! the query when the caller adds it, and the cause. The base URL is the one
//! secret-free form of the instance, as decision 4 asks.

use std::time::{Duration, Instant};

use crible_core::error::{CribleError, ErrorCode};
use crible_search::pipeline::Mode;
use crible_store::online::encode_base64;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Bytes;
use hyper::header::{AUTHORIZATION, CONTENT_TYPE};
use hyper::{Method, Request, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use serde_json::{Map, Value, json};

use crate::commands::{internal, invalid_request};

/// Variable holding the `read` key of the instance (decision 4).
pub(crate) const API_KEY_VARIABLE: &str = "CRIBLE_EVAL_API_KEY";

/// Deadline of one connection, in seconds (decision 5).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Deadline of a whole request, in seconds (decision 5): above the `timeout_ms`
/// ceiling of the route, so the server has time to answer its own `504`.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(35);

/// Bound of a response body, in bytes: the same MiB the server puts on its own
/// bodies, and far above an answer of `k <= 1000` without text nor field
/// (decision 8).
const MAX_BODY_BYTES: usize = 1 << 20;

/// Where the API mode sends its requests.
///
/// Only `http://host[:port]` is accepted: the scheme, the authority and nothing
/// else. Displaying it never shows a credential, and it is the only form of the
/// instance the report carries (decisions 3 and 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Base {
    /// `host` or `host:port`, exactly as `--url` wrote it after the scheme.
    authority: String,
}

impl Base {
    /// Parses `--url`, refusing every component the API mode does not take.
    ///
    /// A refusal names the faulty component and never echoes the URL as it was
    /// received: an accepted URL carries no credential, but a refused one may,
    /// and a message must not print it (decisions 3 and 4).
    pub(crate) fn parse(raw: &str) -> Result<Self, CribleError> {
        let (scheme, rest) = raw.split_once("://").ok_or_else(|| {
            invalid_request(
                "--url carries no scheme: give the base of the instance, such as \
                 http://host:port (ADR-0131, decision 1)",
            )
        })?;
        if !scheme.eq_ignore_ascii_case("http") {
            return Err(invalid_request(format!(
                "--url carries the scheme '{scheme}': the API mode of `crible eval` speaks plain \
                 HTTP/1.1 only, and TLS (https) would need its own ADR (ADR-0131, decision 3)"
            )));
        }
        if rest.contains('?') {
            return Err(invalid_request(
                "--url carries a query string: give the base of the instance only, the command \
                 builds the route (ADR-0131, decision 3)",
            ));
        }
        if rest.contains('#') {
            return Err(invalid_request(
                "--url carries a fragment: give the base of the instance only, the command builds \
                 the route (ADR-0131, decision 3)",
            ));
        }
        let (authority, path) = match rest.split_once('/') {
            Some((authority, path)) => (authority, Some(path)),
            None => (rest, None),
        };
        if let Some(path) = path
            && !path.is_empty()
        {
            return Err(invalid_request(format!(
                "--url carries the path '/{path}': give the base of the instance only, the command \
                 builds the route (ADR-0131, decision 3)"
            )));
        }
        if authority.contains('@') {
            return Err(invalid_request(
                "--url carries a userinfo component ('user:password@'): the API mode takes its \
                 key in CRIBLE_EVAL_API_KEY only, so remove it (ADR-0131, decisions 3 and 4)",
            ));
        }
        let host = if let Some(rest) = authority.strip_prefix('[') {
            let (host, rest) = rest.split_once(']').ok_or_else(|| {
                invalid_request(
                    "--url carries an incomplete IPv6 address: close the brackets of the host \
                     (ADR-0131, decision 3)",
                )
            })?;
            if let Some(port) = rest.strip_prefix(':') {
                check_port(port)?;
            } else if !rest.is_empty() {
                return Err(invalid_request(
                    "--url carries characters after the IPv6 address of the host: give \
                     http://[address]:port (ADR-0131, decision 3)",
                ));
            }
            host
        } else if let Some((host, port)) = authority.rsplit_once(':') {
            check_port(port)?;
            if host.contains(':') {
                return Err(invalid_request(
                    "--url carries an IPv6 address without brackets: write \
                     http://[address]:port (ADR-0131, decision 3)",
                ));
            }
            host
        } else {
            authority
        };
        if host.is_empty() {
            return Err(invalid_request(
                "--url carries no host: give the base of the instance, such as http://host:port \
                 (ADR-0131, decision 1)",
            ));
        }
        if !host
            .chars()
            .all(|component| component.is_ascii_alphanumeric() || ".-_:%".contains(component))
        {
            return Err(invalid_request(
                "--url carries characters that are not part of a host: give http://host:port \
                 (ADR-0131, decision 3)",
            ));
        }
        Ok(Self {
            authority: authority.to_owned(),
        })
    }

    /// The base as the report and every failure print it: `http://host:port`.
    fn display(&self) -> String {
        format!("http://{}", self.authority)
    }

    /// Absolute URI of one route of the instance.
    fn uri(&self, path: &str) -> String {
        format!("http://{}{path}", self.authority)
    }
}

/// `exact_threshold` and N of the evaluated instance (decision 9).
///
/// They are preconditions of the instance, not parameters of the campaign: the
/// JSON report never carries them, and the Markdown names them as read here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Preconditions {
    /// `search.exact_threshold` of the instance.
    pub(crate) exact_threshold: u32,
    /// `search.rescore_candidates` (N) of the instance.
    pub(crate) rescore_candidates: u32,
}

/// One search request of a campaign.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Search<'a> {
    /// Branches to run (`mode` of `docs/05-api.md`).
    pub(crate) mode: Mode,
    /// Query text, for the branches that score one.
    pub(crate) text: Option<&'a str>,
    /// Provided vector, for the dense branch.
    pub(crate) vector: Option<&'a [f32]>,
    /// Filter of the query, in the grammar of `docs/05-api.md`.
    pub(crate) filter: Option<&'a Value>,
    /// Number of results, or of distinct documents under `group_by`.
    pub(crate) k: usize,
    /// Fidelity reference of ADR-0076, decision 1.
    pub(crate) exact: bool,
    /// Quality run: grouped by document, one passage per document.
    pub(crate) grouped: bool,
}

/// What one answer gives the metrics, and how long the client waited.
#[derive(Debug)]
pub(crate) struct Answer {
    /// Keys `doc_id#ord` of the results, best first.
    pub(crate) keys: Vec<String>,
    /// External `doc_id` of the results, best first.
    pub(crate) docs: Vec<String>,
    /// HTTP round trip measured by the client, the only latency of this mode.
    pub(crate) elapsed: Duration,
}

/// State of one generation of the instance, as `GET /v1/indexes/{index}` gives
/// it.
#[derive(Debug, Clone, Copy)]
struct Detail {
    /// Published generation, pinned for the whole campaign.
    generation: u64,
    /// Passages alive in that generation.
    pool_size: u64,
    /// Preconditions of the instance (decision 9).
    preconditions: Preconditions,
}

/// One open connection to the evaluated instance.
pub(crate) struct Session {
    /// Runtime of the client: `current_thread`, the requests are sequential.
    runtime: tokio::runtime::Runtime,
    /// HTTP client, kept for the whole campaign so the connections are reused.
    client: Client<HttpConnector, Full<Bytes>>,
    /// Base of the instance.
    base: Base,
    /// Index the campaign evaluates.
    index: String,
    /// `read` key of the instance; never printed, never logged.
    key: String,
    /// Generation pinned at the first run of the campaign.
    generation: u64,
    /// Passages alive in the pinned generation.
    pool_size: u64,
    /// `exact_threshold` and N of the instance.
    preconditions: Preconditions,
    /// True once the first run has read the state of the index.
    pinned: bool,
}

impl Session {
    /// Reads the key, prepares the client, and returns a session on `base`.
    ///
    /// The generation is pinned by the first run (decision 10), so a failure of
    /// the instance on that first request is reported with the `id` of the
    /// query that could not run (decision 5).
    ///
    /// # Errors
    ///
    /// `invalid_request` when `CRIBLE_EVAL_API_KEY` is absent, empty or not
    /// UTF-8 (decision 4); `internal` when the runtime cannot start.
    pub(crate) fn open(base: Base, index: &str) -> Result<Self, CribleError> {
        let key = api_key()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| {
                internal(format!(
                    "cannot start the HTTP runtime of the API mode: {error}"
                ))
            })?;
        let mut connector = HttpConnector::new();
        connector.set_connect_timeout(Some(CONNECT_TIMEOUT));
        connector.enforce_http(true);
        let client = Client::builder(TokioExecutor::new()).build::<_, Full<Bytes>>(connector);
        Ok(Self {
            runtime,
            client,
            base,
            index: index.to_owned(),
            key,
            generation: 0,
            pool_size: 0,
            preconditions: Preconditions::default(),
            pinned: false,
        })
    }

    /// Generation the campaign is pinned on (decision 10), once a run read it.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// Passages alive in the pinned generation.
    pub(crate) fn pool_size(&self) -> u64 {
        self.pool_size
    }

    /// `exact_threshold` and N of the instance (decision 9).
    pub(crate) fn preconditions(&self) -> Preconditions {
        self.preconditions
    }

    /// The base, as the report prints it: `http://host:port`, without the key.
    pub(crate) fn base(&self) -> String {
        self.base.display()
    }

    /// Pins the generation on the first run of the campaign (decision 10).
    fn ensure_pinned(&mut self) -> Result<(), CribleError> {
        if self.pinned {
            return Ok(());
        }
        let detail = self.detail()?;
        self.generation = detail.generation;
        self.pool_size = detail.pool_size;
        self.preconditions = detail.preconditions;
        self.pinned = true;
        Ok(())
    }

    /// Runs one search and returns what the metrics read from it.
    ///
    /// # Errors
    ///
    /// The code of the instance for a non-2xx answer, `timeout` past the fixed
    /// deadline, `payload_too_large` past the body bound, `internal` for a
    /// transport fault or an answer this command cannot read, and `internal`
    /// when the answer names another generation than the pinned one
    /// (decisions 5 and 10).
    pub(crate) fn search(&mut self, search: &Search<'_>) -> Result<Answer, CribleError> {
        self.ensure_pinned()?;
        let body = body_of(search)?;
        let path = format!("/v1/indexes/{}/search", self.index);
        let started = Instant::now();
        let value = self.send(Method::POST, &path, Some(body))?;
        let elapsed = started.elapsed();
        self.check_generation(&value)?;
        let results = value
            .get("results")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                internal(format!(
                    "the answer of {} carries no `results` array",
                    self.base.display()
                ))
            })?;
        let mut keys = Vec::with_capacity(results.len());
        let mut docs = Vec::with_capacity(results.len());
        for result in results {
            let key = result.get("key").and_then(Value::as_str).ok_or_else(|| {
                internal(format!(
                    "a result of {} carries no `key`",
                    self.base.display()
                ))
            })?;
            let doc_id = result
                .get("doc_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    internal(format!(
                        "a result of {} carries no `doc_id`",
                        self.base.display()
                    ))
                })?;
            keys.push(key.to_owned());
            docs.push(doc_id.to_owned());
        }
        Ok(Answer {
            keys,
            docs,
            elapsed,
        })
    }

    /// Reads the state of the index, and the preconditions of decision 9.
    fn detail(&self) -> Result<Detail, CribleError> {
        let path = format!("/v1/indexes/{}", self.index);
        let value = self.send(Method::GET, &path, None)?;
        let state = value.get("state").ok_or_else(|| {
            internal(format!(
                "the answer of {} carries no `state`",
                self.base.display()
            ))
        })?;
        let generation = number_of(state, "generation", &self.base)?;
        let pool_size = number_of(state, "live_passages", &self.base)?;
        let search = value
            .get("config")
            .and_then(|config| config.get("search"))
            .ok_or_else(|| {
                internal(format!(
                    "the answer of {} carries no `config.search`",
                    self.base.display()
                ))
            })?;
        let exact_threshold = number_of(search, "exact_threshold", &self.base)?;
        let rescore_candidates = number_of(search, "rescore_candidates", &self.base)?;
        Ok(Detail {
            generation,
            pool_size,
            preconditions: Preconditions {
                exact_threshold: narrow(exact_threshold),
                rescore_candidates: narrow(rescore_candidates),
            },
        })
    }

    /// Stops the campaign when an answer names another generation.
    fn check_generation(&self, value: &Value) -> Result<(), CribleError> {
        let received = value
            .get("generation")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                internal(format!(
                    "the answer of {} carries no `generation`",
                    self.base.display()
                ))
            })?;
        if received == self.generation {
            return Ok(());
        }
        // No code of `docs/05-api.md` names a moving index, and decision 5
        // refuses a new one: a campaign that spans two generations is an
        // unexpected state of the instance, which is `internal`.
        Err(CribleError::new(
            ErrorCode::Internal,
            format!(
                "the campaign pinned generation {} of index '{}', and {} answered generation \
                 {received}: the index moved during the campaign, and a report across two \
                 generations measures nothing (ADR-0131, decision 10)",
                self.generation,
                self.index,
                self.base.display()
            ),
        ))
    }

    /// Sends one request and returns the JSON body of a 2xx answer.
    fn send(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value, CribleError> {
        let uri = self.base.uri(path);
        let mut builder = Request::builder()
            .method(method)
            .uri(uri.as_str())
            .header(AUTHORIZATION, format!("Bearer {}", self.key));
        let payload = match body {
            Some(value) => {
                let text = serde_json::to_string(&value).map_err(|error| {
                    internal(format!("cannot serialize a request body: {error}"))
                })?;
                builder = builder.header(CONTENT_TYPE, "application/json");
                Full::new(Bytes::from(text))
            }
            None => Full::new(Bytes::new()),
        };
        let request = builder.body(payload).map_err(|error| {
            internal(format!(
                "cannot build a request for {}: {error}",
                self.base.display()
            ))
        })?;
        let (status, bytes) = self.runtime.block_on(self.exchange(request))?;
        if !status.is_success() {
            return Err(self.failure(status, &bytes));
        }
        serde_json::from_slice(&bytes).map_err(|error| {
            internal(format!(
                "the answer of {} is not JSON: {error}",
                self.base.display()
            ))
        })
    }

    /// Sends a request and reads its body, under the fixed deadline.
    async fn exchange(
        &self,
        request: Request<Full<Bytes>>,
    ) -> Result<(StatusCode, Bytes), CribleError> {
        let url = self.base.display();
        let exchange = async {
            let response = self.client.request(request).await.map_err(|error| {
                internal(format!("cannot reach {url}: {}", cause_chain(&error)))
            })?;
            let status = response.status();
            let body = Limited::new(response.into_body(), MAX_BODY_BYTES)
                .collect()
                .await
                .map_err(|error| {
                    if error.is::<http_body_util::LengthLimitError>() {
                        CribleError::new(
                            ErrorCode::PayloadTooLarge,
                            format!(
                                "the answer of {url} exceeds the {MAX_BODY_BYTES} bytes the API \
                                 mode reads: a search answer is bounded by `k <= 1000`, without \
                                 text and without field (ADR-0131, decision 8)"
                            ),
                        )
                    } else {
                        internal(format!("cannot read the answer of {url}: {error}"))
                    }
                })?;
            Ok::<(StatusCode, Bytes), CribleError>((status, body.to_bytes()))
        };
        match tokio::time::timeout(REQUEST_TIMEOUT, exchange).await {
            Ok(result) => result,
            Err(_) => Err(CribleError::new(
                ErrorCode::Timeout,
                format!(
                    "{url} did not answer within {} s, the fixed deadline of the API mode \
                     (ADR-0131, decision 5)",
                    REQUEST_TIMEOUT.as_secs()
                ),
            )),
        }
    }

    /// Turns a non-2xx answer into the named error of decision 5.
    ///
    /// The code and the message are those of the server body, with its
    /// `request_id`; a body this command cannot read falls back on the code of
    /// the status. The message never carries the key nor a query text: the
    /// server does not put either in its error body (`docs/05-api.md:25-35`).
    fn failure(&self, status: StatusCode, body: &[u8]) -> CribleError {
        let parsed: Option<Value> = serde_json::from_slice(body).ok();
        let error = parsed.as_ref().and_then(|value| value.get("error"));
        let code = error
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let category = ErrorCode::ALL
            .iter()
            .copied()
            .find(|known| known.as_str() == code)
            .unwrap_or_else(|| code_of(status));
        let message = error
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("the answer carries no error body");
        let request_id = error
            .and_then(|error| error.get("request_id"))
            .and_then(Value::as_str)
            .unwrap_or("-");
        CribleError::new(
            category,
            format!(
                "{} answered {} {}: {message} (request_id {request_id})",
                self.base.display(),
                status.as_u16(),
                category.as_str()
            ),
        )
    }
}

/// The key of the instance, from `CRIBLE_EVAL_API_KEY` and nowhere else.
///
/// The message names the variable and never its value, nor the bytes of a
/// value that is not UTF-8 (decision 4).
fn api_key() -> Result<String, CribleError> {
    match std::env::var(API_KEY_VARIABLE) {
        Ok(key) if !key.is_empty() => Ok(key),
        Ok(_) => Err(invalid_request(format!(
            "{API_KEY_VARIABLE} is empty: the API mode sends the `read` key of the instance from \
             that variable, and nowhere else (ADR-0131, decision 4)"
        ))),
        Err(std::env::VarError::NotPresent) => Err(invalid_request(format!(
            "{API_KEY_VARIABLE} is not set: the API mode sends the `read` key of the instance from \
             that variable, and nowhere else (ADR-0131, decision 4)"
        ))),
        Err(std::env::VarError::NotUnicode(_)) => Err(invalid_request(format!(
            "{API_KEY_VARIABLE} is not valid UTF-8: the API mode sends the `read` key of the \
             instance from that variable, and nowhere else (ADR-0131, decision 4)"
        ))),
    }
}

/// Refuses a port that is not a number of the `u16` range.
fn check_port(port: &str) -> Result<(), CribleError> {
    if port.is_empty() {
        return Err(invalid_request(
            "--url carries an empty port: give http://host:port (ADR-0131, decision 1)",
        ));
    }
    if !port.bytes().all(|byte| byte.is_ascii_digit()) || port.parse::<u16>().is_err() {
        return Err(invalid_request(
            "--url carries a port that is not a number from 0 to 65535: give http://host:port \
             (ADR-0131, decision 1)",
        ));
    }
    Ok(())
}

/// Body of one search request (`docs/05-api.md:259-282`).
///
/// The fidelity reference passes `exact: true`, the quality run passes
/// `group_by: "document"`, and neither carries `debug`, so the server writes no
/// debug journal and the answer carries no server stage (decision 8).
fn body_of(search: &Search<'_>) -> Result<Value, CribleError> {
    let mut body = Map::new();
    body.insert("mode".to_owned(), json!(search.mode.as_str()));
    body.insert("k".to_owned(), json!(search.k));
    // No field and no text: the metrics read keys and document identifiers
    // alone, and the answer stays small (decision 8).
    body.insert("return".to_owned(), json!({ "fields": [], "text": false }));
    if search.exact {
        body.insert("exact".to_owned(), json!(true));
    }
    if search.grouped {
        body.insert("group_by".to_owned(), json!("document"));
        body.insert("passages_per_document".to_owned(), json!(1));
    }
    if let Some(filter) = search.filter {
        body.insert("filter".to_owned(), filter.clone());
    }
    match search.mode {
        Mode::Dense => match (search.text, search.vector) {
            (Some(text), None) => {
                body.insert("query".to_owned(), json!(text));
            }
            (None, Some(vector)) => {
                body.insert("vector".to_owned(), json!(vector_base64(vector)));
            }
            (Some(_), Some(_)) => {
                return Err(invalid_request(
                    "a dense run takes exactly one of a `query` text and a `vector`, not both",
                ));
            }
            (None, None) => {
                return Err(invalid_request(
                    "a dense run takes exactly one of a `query` text and a `vector`",
                ));
            }
        },
        Mode::Lexical => {
            let text = search.text.ok_or_else(|| {
                invalid_request("a lexical run takes a `query` text, and no `vector`")
            })?;
            body.insert("query".to_owned(), json!(text));
        }
        Mode::Hybrid => {
            let text = search.text.ok_or_else(|| {
                invalid_request("a hybrid run takes a `query` text, and an optional `vector`")
            })?;
            body.insert("query".to_owned(), json!(text));
            if let Some(vector) = search.vector {
                body.insert("vector".to_owned(), json!(vector_base64(vector)));
            }
        }
    }
    Ok(Value::Object(body))
}

/// Base64 of the little-endian fp32 components of a vector.
///
/// `docs/05-api.md` takes the `vector` of a request in that spelling, and
/// [`encode_base64`] is the encoder written beside the decoder of the same
/// alphabet (decision 8).
fn vector_base64(vector: &[f32]) -> String {
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for component in vector {
        bytes.extend_from_slice(&component.to_le_bytes());
    }
    encode_base64(&bytes)
}

/// One `u64` member of a JSON object, or the fault that names it.
fn number_of(object: &Value, member: &str, base: &Base) -> Result<u64, CribleError> {
    object.get(member).and_then(Value::as_u64).ok_or_else(|| {
        internal(format!(
            "the answer of {} carries no `{member}`",
            base.display()
        ))
    })
}

/// A counter of the instance as a `u32`, saturated: the configuration of an
/// index holds `u32` values, so a larger one is already outside its domain.
fn narrow(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Code of `docs/05-api.md` for a status whose body carried no known code.
fn code_of(status: StatusCode) -> ErrorCode {
    match status.as_u16() {
        400 => ErrorCode::InvalidRequest,
        401 => ErrorCode::Unauthorized,
        403 => ErrorCode::Forbidden,
        404 => ErrorCode::NotFound,
        409 => ErrorCode::ImmutableSetting,
        413 => ErrorCode::PayloadTooLarge,
        422 => ErrorCode::IncompatibleModel,
        503 => ErrorCode::Overloaded,
        504 => ErrorCode::Timeout,
        _ => ErrorCode::Internal,
    }
}

/// The chain of causes of a transport error, so the message names DNS, a
/// refused connection or a truncated answer.
fn cause_chain(error: &hyper_util::client::legacy::Error) -> String {
    let mut text = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        let part = cause.to_string();
        if !text.contains(&part) {
            text.push_str(": ");
            text.push_str(&part);
        }
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    // Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions.
    #![allow(clippy::panic)]

    use super::*;

    /// Message of a refused URL.
    fn refusal(raw: &str) -> String {
        match Base::parse(raw) {
            Ok(base) => panic!("'{raw}' must be refused, got {base:?}"),
            Err(error) => {
                assert_eq!(
                    error.code(),
                    ErrorCode::InvalidRequest,
                    "a refused URL is `invalid_request` (decision 3)"
                );
                error.message().to_owned()
            }
        }
    }

    #[test]
    fn a_plain_http_base_is_accepted() {
        for (raw, authority) in [
            ("http://127.0.0.1:8080", "127.0.0.1:8080"),
            ("http://127.0.0.1:8080/", "127.0.0.1:8080"),
            ("http://example.org", "example.org"),
            ("HTTP://example.org:80", "example.org:80"),
            ("http://[::1]:9000", "[::1]:9000"),
        ] {
            let base = Base::parse(raw).unwrap_or_else(|error| panic!("{raw}: {error}"));
            assert_eq!(base.authority, authority, "{raw}");
        }
        let base =
            Base::parse("http://127.0.0.1:8080/").expect("the trailing slash is a root path");
        assert_eq!(base.display(), "http://127.0.0.1:8080");
        assert_eq!(
            base.uri("/v1/indexes/i/search"),
            "http://127.0.0.1:8080/v1/indexes/i/search"
        );
    }

    #[test]
    fn tls_is_refused_before_any_io() {
        let message = refusal("https://127.0.0.1:1");
        assert!(
            message.contains("https"),
            "the refusal names the scheme: {message}"
        );
        assert!(
            message.to_lowercase().contains("tls"),
            "the refusal says TLS would need an ADR: {message}"
        );
    }

    #[test]
    fn every_component_the_api_mode_does_not_take_is_refused_by_name() {
        let cases = [
            ("ftp://host:1", "ftp"),
            ("host:8080", "scheme"),
            ("http://", "host"),
            ("http://user:pass@host:1", "userinfo"),
            ("http://host:1/path", "path"),
            ("http://host:1?a=b", "query string"),
            ("http://host:1#fragment", "fragment"),
            ("http://host:notaport", "port"),
            ("http://host:", "empty port"),
            ("http://host:+80", "port"),
            ("http://::1:9000", "IPv6"),
            ("http://ho st:1", "host"),
        ];
        for (raw, component) in cases {
            let message = refusal(raw);
            assert!(
                message.contains(component),
                "'{raw}' must name {component:?}: {message}"
            );
            assert!(
                !message.contains("pass@"),
                "the refusal never echoes a credential: {message}"
            );
            // The refusal may quote the base, never a credential: `http://`
            // and `http://host:` are prefixes of the `http://host:port` it
            // suggests, so a no-echo check on those two would be meaningless.
            assert!(
                message.contains("--url"),
                "'{raw}' is a malformed --url, so the refusal says so: {message}"
            );
        }
    }

    #[test]
    fn the_body_of_a_search_carries_only_the_documented_fields() {
        let filter = json!({ "fonds": "LEGI" });
        let search = Search {
            mode: Mode::Dense,
            text: None,
            vector: Some(&[1.0, 0.5]),
            filter: Some(&filter),
            k: 10,
            exact: true,
            grouped: true,
        };
        let body = body_of(&search).unwrap_or_else(|error| panic!("{error}"));
        let members: Vec<&str> = body
            .as_object()
            .unwrap_or_else(|| panic!("the body is an object: {body}"))
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            members,
            vec![
                "exact",
                "filter",
                "group_by",
                "k",
                "mode",
                "passages_per_document",
                "return",
                "vector",
            ],
            "the body is the one of `docs/05-api.md:259-282`: {body}"
        );
        assert_eq!(body["return"], json!({ "fields": [], "text": false }));
        assert!(
            body.get("debug").is_none(),
            "`debug` stays out, so the instance writes no debug journal (decision 8)"
        );
    }

    #[test]
    fn a_vector_is_sent_as_base64_fp32_little_endian() {
        let encoded = vector_base64(&[1.0, 0.0, -2.5]);
        assert_eq!(encoded, "AACAPwAAAAAAACDA");
        let decoded = crible_store::online::decode_base64(&encoded)
            .unwrap_or_else(|error| panic!("the encoder must write valid base64: {error}"));
        let components: Vec<f32> = decoded
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| f32::from_le_bytes(*bytes))
            .collect();
        assert_eq!(components, vec![1.0, 0.0, -2.5]);
    }

    #[test]
    fn a_status_without_a_known_body_code_names_its_code() {
        assert_eq!(code_of(StatusCode::UNAUTHORIZED), ErrorCode::Unauthorized);
        assert_eq!(code_of(StatusCode::NOT_FOUND), ErrorCode::NotFound);
        assert_eq!(
            code_of(StatusCode::SERVICE_UNAVAILABLE),
            ErrorCode::Overloaded
        );
        assert_eq!(code_of(StatusCode::GATEWAY_TIMEOUT), ErrorCode::Timeout);
        assert_eq!(
            code_of(StatusCode::INTERNAL_SERVER_ERROR),
            ErrorCode::Internal
        );
    }
}
