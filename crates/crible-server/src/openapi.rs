// SPDX-License-Identifier: Apache-2.0

//! The OpenAPI specification of the server, generated from the code.
//!
//! References: `docs/05-api.md` (the routes, the bodies and the error format),
//! ADR-0015 (the specification is generated from the code), ADR-0095 (roles),
//! ADR-0099 (the two probes and the `503` of `/ready`), ADR-0100 and ADR-0101
//! (the key and the error body), ADR-0105 (the administration of the indexes),
//! ADR-0107 (the write routes), ADR-0109 (the search body), ADR-0110 (the
//! multi-index search) and ADR-0111 (the publication of the specification).
//!
//! # What is described here
//!
//! The routes of `docs/05-api.md` are annotated where they are mounted, with
//! [`utoipa::path`]; the bodies and the answers are described by the types of
//! this module. The routes themselves build their JSON with `serde_json::json!`
//! (a `Job` is assembled from a row of `jobs.sqlite`, a search answer from the
//! pipeline), so the types below mirror those bodies for the specification: they
//! are never serialised back by the server. `PUBLIER-OPENAPI` adds them without
//! changing one line of the routes.
//!
//! Every object whose keys the server refuses is closed (`additionalProperties:
//! false`, through `deny_unknown_fields`), as ADR-0031, ADR-0109 and ADR-0110
//! require: the specification must not accept a body the server rejects. The
//! three free objects of ADR-0111, decision 7 (`filter`, `return.fields` and the
//! schema of an index), are described by their grammar instead of being frozen
//! into a list of known fields.
//!
//! # The three maintenance routes
//!
//! `POST /v1/indexes/{index}/compact`, `…/exports` and `…/snapshots` take **no
//! body** (`docs/05-api.md`). Their handlers read a `Bytes` extractor to refuse
//! a non-empty body, and `axum_extras` would turn that argument into a request
//! body in the specification. Their paths are therefore annotated on the
//! documentation stubs of this module, which describe the routes without their
//! body, and the handlers of `crate::maintenance` stay untouched.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::openapi::content::Content;
use utoipa::openapi::response::ResponseBuilder;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::openapi::{OpenApi, Ref, RefOr};
use utoipa::{Modify, OpenApi as OpenApiDerive, ToSchema};

/// A JSON object whose keys and values the server does not fix in advance.
///
/// It describes `details` of an error (ADR-0101), the `fields` of a document or
/// of a result, `filter`, and the `result` of a job. The grammar, when there is
/// one, is in the `description` of the field.
pub type FreeObject = BTreeMap<String, Value>;

/// The body of every error of the API, except the `503` of `/ready`.
///
/// One component, `Error`, for the whole document (ADR-0111, decision 6): a
/// generated client reads one type of error, whatever the code of
/// `docs/05-api.md`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Error {
    /// The error itself, in the format of ADR-0101.
    pub error: ErrorBody,
}

/// The members of one error (`docs/05-api.md`, "Erreurs").
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorBody {
    /// Code of the table of `docs/05-api.md`, also the category of the analytics.
    #[schema(example = "invalid_filter")]
    pub code: String,
    /// Message of the error; generic and fixed for `internal` (ADR-0101).
    pub message: String,
    /// Keyed explanation; empty, except the documented keys of a code.
    ///
    /// Only `path` is documented today, for `invalid_filter` (ADR-0101,
    /// decision 3). A message never carries an internal id (INV-01) nor an
    /// absolute path of the server.
    #[schema(value_type = Object)]
    pub details: FreeObject,
    /// Identifier of the request, the same as the `x-request-id` header (ADR-0098).
    pub request_id: String,
}

/// The body of `GET /health` (ADR-0099).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Health {
    /// Always `ok`: the process is alive.
    #[schema(example = "ok")]
    pub status: String,
}

/// The body of the `200` of `GET /ready` (ADR-0099).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Ready {
    /// Always `ready`: the models are loaded and every index has published.
    #[schema(example = "ready")]
    pub status: String,
}

/// The body of the `503` of `GET /ready` (ADR-0099).
///
/// It is the one body the error format of ADR-0101 leaves alone, and it is
/// declared on that operation only (ADR-0111, decision 6).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NotReady {
    /// Always `not_ready`.
    #[schema(example = "not_ready")]
    pub status: String,
}

/// Type of a field declared by the schema of an index (`docs/04`).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    /// Opaque string.
    Keyword,
    /// Signed integer.
    Integer,
    /// Floating-point number.
    Float,
    /// Calendar date, `YYYY-MM-DD`.
    Date,
    /// Boolean.
    Bool,
}

/// One field of the schema of an index (`docs/04`).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexField {
    /// Type of the values of the field.
    #[serde(rename = "type")]
    pub field_type: FieldType,
    /// Whether the field may appear in a filter; `true` when absent.
    #[schema(nullable = false)]
    pub filterable: Option<bool>,
    /// Whether the field is returned in a result; `true` when absent.
    #[schema(nullable = false)]
    pub returned: Option<bool>,
}

/// Schema of an index: the fields of the document and of the passage.
///
/// The **names** are free, because they belong to each index (ADR-0111,
/// decision 7); a name declared at both levels is refused (ADR-0030).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexSchema {
    /// Fields carried by the document and inherited by its passages.
    pub document_fields: BTreeMap<String, IndexField>,
    /// Fields carried by a single passage.
    pub passage_fields: BTreeMap<String, IndexField>,
}

/// Precision of the reference vectors, frozen at the creation of the index.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ReferencePrecision {
    /// Half precision.
    Fp16,
    /// Single precision.
    Fp32,
}

/// Scan strategy of the dense branch.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScanStrategy {
    /// Binary-code scan followed by exact rescoring.
    Binary,
    /// Always exact.
    None,
}

/// Fusion of the dense and lexical rankings (`docs/03`).
///
/// The body is `{"method": "rrf"}` or
/// `{"method": "relative_score", "alpha": a}`: `alpha` is required with
/// `relative_score`, lies in `[0, 1]`, bounds included, and is finite; it is
/// refused with `rrf`, and an unknown member of the object is refused too
/// (ADR-0090). The two variants are written as closed objects, so that the
/// schema refuses what the server refuses.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum Fusion {
    /// Reciprocal rank fusion.
    Rrf(RrfFusion),
    /// Relative score, with the weight of the dense branch.
    RelativeScore(RelativeScoreFusion),
}

/// The `rrf` form of a fusion: its only member is the method.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RrfFusion {
    /// The method, `rrf`.
    pub method: RrfMethod,
}

/// The method of the `rrf` form.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RrfMethod {
    /// Reciprocal rank fusion.
    Rrf,
}

/// The `relative_score` form of a fusion: the method and the weight.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RelativeScoreFusion {
    /// The method, `relative_score`.
    pub method: RelativeScoreMethod,
    /// Weight of the dense branch, in `[0, 1]`, bounds included, and finite.
    #[schema(minimum = 0, maximum = 1)]
    pub alpha: f64,
}

/// The method of the `relative_score` form.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RelativeScoreMethod {
    /// Relative score, with the weight of the dense branch.
    RelativeScore,
}

/// Lexical branch settings of an index.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LexicalConfig {
    /// Whether the lexical branch is built and queried.
    pub enabled: bool,
    /// Language of the analyzer, one of the codes of ADR-0086.
    #[schema(example = "fr")]
    pub language: String,
    /// Whether term positions are stored.
    pub positions: bool,
}

/// Dense search settings of an index.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchConfig {
    /// Scan strategy of the dense branch.
    pub scan: ScanStrategy,
    /// Number of candidates re-scored exactly.
    pub rescore_candidates: u32,
    /// Pool size up to which (inclusive) the search is exact (ADR-0074).
    #[schema(nullable = false)]
    pub exact_threshold: Option<u32>,
    /// Candidates kept by each branch before fusion.
    pub candidates_per_branch: u32,
    /// Fusion of the two rankings.
    pub fusion: Fusion,
}

/// Ingestion settings of an index, acceptance check included.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct IngestConfig {
    /// Passages re-encoded by the acceptance check.
    pub acceptance_sample: u32,
    /// Minimum cosine similarity accepted, in `]0, 1]` (ADR-0054).
    pub acceptance_min_cosine: f64,
}

/// Compaction settings of an index.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CompactionConfig {
    /// Fraction of dead passages that triggers a compaction.
    pub dead_ratio: f64,
}

/// Configuration of an index, in the form of `index.json` (`docs/04`).
///
/// It is the body of `POST /v1/indexes` and the `config` of
/// `GET /v1/indexes/{index}`. `exact_threshold` is optional at creation and
/// materialised by the creation (ADR-0074).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexConfig {
    /// Name of the index, frozen.
    pub name: String,
    /// Name of the model directory, frozen.
    pub model: String,
    /// Precision of the reference vectors, frozen.
    pub reference_precision: ReferencePrecision,
    /// Schema of the index.
    pub schema: IndexSchema,
    /// Fields driving the clustering, applied at the next compaction.
    pub cluster_by: Vec<String>,
    /// Lexical branch settings.
    pub lexical: LexicalConfig,
    /// Dense search settings.
    pub search: SearchConfig,
    /// Ingestion settings.
    pub ingest: IngestConfig,
    /// Compaction settings.
    pub compaction: CompactionConfig,
}

/// One field of a `PATCH` of configuration; every member is optional.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexFieldPatch {
    /// New type of the field; required when the field is new.
    #[serde(rename = "type")]
    #[schema(nullable = false)]
    pub field_type: Option<FieldType>,
    /// New value of `filterable`.
    #[schema(nullable = false)]
    pub filterable: Option<bool>,
    /// New value of `returned`.
    #[schema(nullable = false)]
    pub returned: Option<bool>,
}

/// Schema part of a `PATCH` of configuration.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexSchemaPatch {
    /// Fields of the document to add or to amend.
    #[schema(nullable = false)]
    pub document_fields: Option<BTreeMap<String, IndexFieldPatch>>,
    /// Fields of the passage to add or to amend.
    #[schema(nullable = false)]
    pub passage_fields: Option<BTreeMap<String, IndexFieldPatch>>,
}

/// Lexical part of a `PATCH` of configuration.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LexicalPatch {
    /// New state of the branch.
    #[schema(nullable = false)]
    pub enabled: Option<bool>,
    /// New language of the analyzer.
    #[schema(nullable = false)]
    pub language: Option<String>,
    /// New state of the positions.
    #[schema(nullable = false)]
    pub positions: Option<bool>,
}

/// Dense part of a `PATCH` of configuration.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchPatch {
    /// New scan strategy.
    #[schema(nullable = false)]
    pub scan: Option<ScanStrategy>,
    /// New number of re-scored candidates.
    #[schema(nullable = false)]
    pub rescore_candidates: Option<u32>,
    /// New exact threshold.
    #[schema(nullable = false)]
    pub exact_threshold: Option<u32>,
    /// New number of candidates per branch.
    #[schema(nullable = false)]
    pub candidates_per_branch: Option<u32>,
    /// New fusion.
    #[schema(nullable = false)]
    pub fusion: Option<Fusion>,
}

/// Ingestion part of a `PATCH` of configuration.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct IngestPatch {
    /// New size of the acceptance sample.
    #[schema(nullable = false)]
    pub acceptance_sample: Option<u32>,
    /// New minimum cosine of the acceptance check.
    #[schema(nullable = false)]
    pub acceptance_min_cosine: Option<f64>,
}

/// Compaction part of a `PATCH` of configuration.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CompactionPatch {
    /// New fraction of dead passages that triggers a compaction.
    #[schema(nullable = false)]
    pub dead_ratio: Option<f64>,
}

/// Body of `PATCH /v1/indexes/{index}`: the settings to change.
///
/// A key outside this list is `400 invalid_request` at every level (ADR-0031);
/// a frozen setting named here is `409 immutable_setting` (INV-16).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexPatch {
    /// Name of the index, frozen.
    #[schema(nullable = false)]
    pub name: Option<String>,
    /// Model of the index, frozen.
    #[schema(nullable = false)]
    pub model: Option<String>,
    /// Precision of the reference vectors, frozen.
    #[schema(nullable = false)]
    pub reference_precision: Option<ReferencePrecision>,
    /// Schema fields to add or to amend.
    #[schema(nullable = false)]
    pub schema: Option<IndexSchemaPatch>,
    /// Fields driving the clustering.
    #[schema(nullable = false)]
    pub cluster_by: Option<Vec<String>>,
    /// Lexical branch settings.
    #[schema(nullable = false)]
    pub lexical: Option<LexicalPatch>,
    /// Dense search settings.
    #[schema(nullable = false)]
    pub search: Option<SearchPatch>,
    /// Ingestion settings.
    #[schema(nullable = false)]
    pub ingest: Option<IngestPatch>,
    /// Compaction settings.
    #[schema(nullable = false)]
    pub compaction: Option<CompactionPatch>,
}

/// One entry of `GET /v1/indexes` (ADR-0105).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexListEntry {
    /// Name of the index.
    pub name: String,
    /// Name of the model directory.
    pub model: String,
    /// Published generation.
    pub generation: u64,
    /// Documents the published generation holds.
    pub documents: u64,
    /// Live passages at this generation.
    pub live_passages: u64,
}

/// Body of `GET /v1/indexes`: one key, the plural name of the resource.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexList {
    /// The served indexes, sorted by name.
    pub indexes: Vec<IndexListEntry>,
}

/// Estimated sizes of the published state of an index.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexSizes {
    /// Size of the index directory on disk.
    pub disk_bytes: u64,
    /// Estimated heap size of the filter structures.
    pub filter_memory_bytes: u64,
    /// Bytes of RAM the binary codes hold.
    pub codes_memory_bytes: u64,
}

/// Last batch applied to the epoch, `null` when none was attached.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LastBatch {
    /// Identifier of the batch.
    pub batch_id: String,
    /// Date the batch was applied, RFC 3339 UTC.
    pub applied_at: String,
    /// Generation the attach published.
    pub generation: u64,
}

/// State of the published generation of an index (ADR-0105).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexState {
    /// Epoch directory holding the reference state.
    pub epoch: u64,
    /// Published generation.
    pub generation: u64,
    /// Documents the published generation holds.
    pub documents: u64,
    /// Live passages at this generation.
    pub live_passages: u64,
    /// Tombstoned passages.
    pub dead_passages: u64,
    /// Ratio of dead passages among all the passages.
    pub dead_ratio: f64,
    /// Number of segments; the list is never exposed (INV-01).
    pub segments: u64,
    /// Estimated sizes of the state.
    pub sizes: IndexSizes,
    /// Last applied batch.
    pub last_batch: Option<LastBatch>,
}

/// Body of `GET /v1/indexes/{index}` and of the `201` of `POST /v1/indexes`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexDetail {
    /// Name of the index.
    pub name: String,
    /// Materialised configuration, in the form of `index.json`.
    pub config: IndexConfig,
    /// State of the published generation.
    pub state: IndexState,
}

/// The four states of a job (`docs/05-api.md`, "Jobs").
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// Accepted, waiting for its writer.
    Queued,
    /// Running on the writer of its index.
    Running,
    /// Finished without an error.
    Succeeded,
    /// Finished with an error.
    Failed,
}

/// The nine kinds of a job (`docs/05-api.md`, "Jobs").
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// Attach a batch deposited in the inbox.
    AttachBatch,
    /// Apply an online batch.
    OnlineBatch,
    /// Update the metadata of documents.
    PatchDocuments,
    /// Compact an epoch.
    Compact,
    /// Export the live data of an index.
    Export,
    /// Snapshot an epoch.
    Snapshot,
    /// Rebuild the lexical branch.
    RebuildLexical,
    /// Apply a configuration patch.
    PatchConfig,
    /// Delete an index.
    DeleteIndex,
}

/// Fine progression of a job.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct JobProgress {
    /// Step the job reached.
    #[schema(example = "commit")]
    pub step: String,
    /// Units already treated.
    pub done: u64,
    /// Units to treat.
    pub total: u64,
}

/// Error of a failed job: `{ "code", "message", "details" }`, without a
/// `request_id` (ADR-0103).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct JobError {
    /// Code of the table of `docs/05-api.md`.
    pub code: String,
    /// Message of the error.
    pub message: String,
    /// Keyed explanation, empty unless a code documents a key.
    #[schema(value_type = Object)]
    pub details: FreeObject,
}

/// One job as the API renders it (`docs/05-api.md`, "Jobs").
///
/// Every absent member is `null`, so a client reads the same keys on every job.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Job {
    /// `job_` followed by 32 hexadecimal characters.
    pub id: String,
    /// Kind of work.
    pub kind: JobKind,
    /// Index the job writes.
    pub index: String,
    /// Current state.
    pub state: JobState,
    /// Creation, RFC 3339 UTC.
    pub created_at: String,
    /// Start of the run.
    pub started_at: Option<String>,
    /// End of the run.
    pub finished_at: Option<String>,
    /// Generation the job published, when it has one.
    pub generation: Option<u64>,
    /// Fine progression.
    pub progress: Option<JobProgress>,
    /// Report of the job, when its kind carries one. Its form depends on the
    /// kind: `{ "report": … }` for an attach, a metadata patch or an online
    /// batch.
    #[schema(value_type = Option<Object>)]
    pub result: Option<FreeObject>,
    /// Error of a failed job.
    pub error: Option<JobError>,
}

/// Body of `GET /v1/jobs`: one key, the plural name of the resource.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct JobList {
    /// The retained jobs, `created_at` descending then `id`.
    pub jobs: Vec<Job>,
}

/// Mode of an online batch: `upsert` only (ADR-0055).
///
/// `replace_all` is the mode of a deposited batch; the online route refuses it
/// in `400 invalid_request` (`docs/05-api.md`, "Lot en ligne"), so the schema
/// does not publish it.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BatchMode {
    /// Create, replace or update the listed documents; leave the others alone.
    Upsert,
}

/// One passage of an online batch (ADR-0055, ADR-0107).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OnlinePassage {
    /// Exact text indexed in BM25 and encoded when no vector is provided.
    pub text: String,
    /// Passage fields, names of the schema of the index; `{}` when absent.
    #[schema(value_type = Option<Object>, nullable = false)]
    pub fields: Option<FreeObject>,
    /// Vector of the passage, base64 of the little-endian fp32 components,
    /// instead of the encoding of `text`.
    #[schema(nullable = false)]
    pub vector: Option<String>,
}

/// One document of an online batch.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OnlineDocument {
    /// External identifier of the document.
    pub doc_id: String,
    /// Producer-provided digest; the blake3 of the canonical passages when
    /// absent (ADR-0055).
    #[schema(nullable = false)]
    pub doc_hash: Option<String>,
    /// Document fields, names of the schema of the index; `{}` when absent.
    #[schema(value_type = Option<Object>, nullable = false)]
    pub fields: Option<FreeObject>,
    /// Passages of the document, in `ord` order; at least one.
    pub passages: Vec<OnlinePassage>,
}

/// Body of `POST /v1/indexes/{index}/documents`: one online batch.
///
/// A body with neither `documents` nor `deletions` is `400 invalid_request`
/// (ADR-0107).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OnlineBatchRequest {
    /// Identifier of the batch, mandatory and chosen by the caller.
    pub batch_id: String,
    /// Mode of the internal manifest: `upsert` only.
    #[schema(nullable = false)]
    pub mode: Option<BatchMode>,
    /// Documents to write.
    #[schema(nullable = false)]
    pub documents: Option<Vec<OnlineDocument>>,
    /// External identifiers to remove.
    #[schema(nullable = false)]
    pub deletions: Option<Vec<String>>,
}

/// One entry of a metadata patch (ADR-0060).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentPatch {
    /// External identifier of the document to patch; a patch never creates one.
    pub doc_id: String,
    /// Document fields to write, merged into the stored ones.
    #[schema(value_type = Object)]
    pub fields: FreeObject,
}

/// Body of `PATCH /v1/indexes/{index}/documents`: the documents to patch.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MetadataPatchRequest {
    /// Documents whose fields are replaced.
    pub documents: Vec<DocumentPatch>,
}

/// Body of `POST /v1/indexes/{index}/batches`: the directory of the batch.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DepositedBatchRequest {
    /// One path component below `<exchange>/inbox` (ADR-0107).
    pub directory: String,
}

/// One passage of a document read by `GET …/documents/{doc_id}`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentPassage {
    /// Order of the passage in its document.
    pub ord: u32,
    /// Passage fields the schema declares `returned`.
    #[schema(value_type = Object)]
    pub fields: FreeObject,
    /// Text of the passage, present with `?text=true` only.
    #[schema(nullable = false)]
    pub text: Option<String>,
}

/// Body of `GET /v1/indexes/{index}/documents/{doc_id}` (ADR-0107).
///
/// Only the fields the schema declares `returned` are rendered, and no vector
/// nor internal id ever appears (INV-01).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentDetail {
    /// External identifier of the document.
    pub doc_id: String,
    /// Generation of the snapshot the document was read in.
    pub generation: u64,
    /// Digest of the passages of the document.
    pub doc_hash: String,
    /// Document fields the schema declares `returned`.
    #[schema(value_type = Object)]
    pub fields: FreeObject,
    /// Live passages, `ord` ascending.
    pub passages: Vec<DocumentPassage>,
}

/// Mode of a search: the branches to run (`docs/05-api.md`).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    /// Dense branch only.
    Dense,
    /// Lexical branch only.
    Lexical,
    /// Both branches, fused.
    Hybrid,
}

/// Regroupment of a search: `document` or absent.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GroupBy {
    /// Best passages of each document, in a flat list.
    Document,
}

/// Deduplication of a search: `content` or absent.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Dedup {
    /// Identical passages fused before the regroupment.
    Content,
}

/// Filter of a search: an object whose grammar is `docs/05-api.md`.
///
/// A key is a field of the schema of the index, or `$and`, `$or`, `$not`. The
/// value of a field is a scalar (which means `eq`) or an object of operators,
/// according to the type of the field. A clause outside the grammar is
/// `invalid_filter`, with `details.path`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Filter {
    /// The clauses of the filter, by field name or by logical operator.
    #[serde(flatten)]
    #[schema(value_type = Object)]
    pub clauses: FreeObject,
}

/// Selection of what a result renders (`docs/05-api.md`, "Recherche : requête").
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchReturn {
    /// `"all"`, or the list of the document field names to render. A name the
    /// schema declares `returned: false` is refused (ADR-0109).
    #[schema(nullable = false)]
    pub fields: Option<Value>,
    /// Whether the text of the passage is rendered.
    #[schema(nullable = false)]
    pub text: Option<bool>,
}

/// Body of `POST /v1/indexes/{index}/search` (ADR-0109).
///
/// `debug` and `explain_ids` are not members: P6 refuses them in
/// `invalid_request`, and the specification describes only the body the server
/// accepts. P7 opens them (`REPONSE-DEBUG`, `EXPLIQUER-DOCUMENTS`).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchRequest {
    /// Text of the query, encoded by the server; required by `lexical` and
    /// `hybrid`, optional in `dense`.
    #[schema(nullable = false)]
    pub query: Option<String>,
    /// Vector provided, base64 of the little-endian fp32 components; it
    /// replaces the encoding of `query` in the dense branch, and is mandatory
    /// for a model of kind `external`.
    #[schema(nullable = false)]
    pub vector: Option<String>,
    /// Branches to run; `hybrid` when the lexical branch is enabled, otherwise
    /// `dense`.
    #[schema(nullable = false)]
    pub mode: Option<SearchMode>,
    /// Filter of the request; its grammar is above.
    #[schema(nullable = false)]
    pub filter: Option<Filter>,
    /// Number of results, 1 to 1000; under `group_by`, number of documents.
    #[schema(minimum = 1, maximum = 1000)]
    #[schema(nullable = false)]
    pub k: Option<u32>,
    /// Fusion of the two rankings; the configured one of the index when absent.
    #[schema(nullable = false)]
    pub fusion: Option<Fusion>,
    /// Regroupment by document.
    #[schema(nullable = false)]
    pub group_by: Option<GroupBy>,
    /// Passages rendered per regrouped document, 1 to 5.
    #[schema(minimum = 1, maximum = 5)]
    #[schema(nullable = false)]
    pub passages_per_document: Option<u32>,
    /// Fusion of the identical passages.
    #[schema(nullable = false)]
    pub dedup: Option<Dedup>,
    /// Content rendered in a result.
    #[serde(rename = "return")]
    #[schema(nullable = false)]
    pub return_selection: Option<SearchReturn>,
    /// Forces the exact computation (eval, debug).
    #[schema(nullable = false)]
    pub exact: Option<bool>,
    /// Deadline of the request, 1 to 30000 milliseconds.
    #[schema(minimum = 1, maximum = 30000)]
    #[schema(nullable = false)]
    pub timeout_ms: Option<u32>,
}

/// One target of a multi-index search (ADR-0110).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchTarget {
    /// Name of the index of the target.
    pub index: String,
    /// Filter applied to this target only; no filter when absent.
    #[schema(nullable = false)]
    pub filter: Option<Filter>,
}

/// Body of `POST /v1/search` (ADR-0110).
///
/// Every field of [`SearchRequest`] except `filter` is common, at the first
/// level; `filter` becomes a member of a target.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchMultiRequest {
    /// Text of the query, encoded once for every target.
    #[schema(nullable = false)]
    pub query: Option<String>,
    /// Vector provided, base64 of the little-endian fp32 components.
    #[schema(nullable = false)]
    pub vector: Option<String>,
    /// Branches to run on every target; `hybrid` when all targets enable the
    /// lexical branch, otherwise `dense`.
    #[schema(nullable = false)]
    pub mode: Option<SearchMode>,
    /// Number of results, or of documents under `group_by`.
    #[schema(minimum = 1, maximum = 1000)]
    #[schema(nullable = false)]
    pub k: Option<u32>,
    /// Fusion applied to every target; the configured one of each index when
    /// absent.
    #[schema(nullable = false)]
    pub fusion: Option<Fusion>,
    /// Regroupment by document.
    #[schema(nullable = false)]
    pub group_by: Option<GroupBy>,
    /// Passages rendered per regrouped document, 1 to 5.
    #[schema(minimum = 1, maximum = 5)]
    #[schema(nullable = false)]
    pub passages_per_document: Option<u32>,
    /// Fusion of the identical passages.
    #[schema(nullable = false)]
    pub dedup: Option<Dedup>,
    /// Content rendered in a result.
    #[serde(rename = "return")]
    #[schema(nullable = false)]
    pub return_selection: Option<SearchReturn>,
    /// Forces the exact computation.
    #[schema(nullable = false)]
    pub exact: Option<bool>,
    /// Deadline of the whole request, 1 to 30000 milliseconds.
    #[schema(minimum = 1, maximum = 30000)]
    #[schema(nullable = false)]
    pub timeout_ms: Option<u32>,
    /// Indexes searched, in the order of the fusion; at least one, no
    /// duplicate (ADR-0110).
    pub targets: Vec<SearchTarget>,
}

/// How the score of a result was computed (`docs/03-recherche.md`).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScoreType {
    /// Cosine similarity, mode `dense`.
    Cosine,
    /// BM25 score, mode `lexical`.
    Bm25,
    /// Reciprocal rank fusion, mode `hybrid` with `rrf`.
    Rrf,
    /// Relative score, mode `hybrid` with `relative_score`.
    Relative,
}

/// How the score of a multi-index result was computed (ADR-0110).
///
/// The fusion between indexes is either the cosine of `dense` or the RRF of
/// `lexical` and `hybrid`: `bm25` and `relative` never name a multi-index
/// score.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MultiScoreType {
    /// Cosine similarity, mode `dense`.
    Cosine,
    /// Reciprocal rank fusion, modes `lexical` and `hybrid`.
    Rrf,
}

/// One result of a search.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchResult {
    /// External identifier of the document of the passage.
    pub doc_id: String,
    /// Order of the passage in its document.
    pub passage: u32,
    /// `doc_id#ord`.
    pub key: String,
    /// Score of the result.
    pub score: f64,
    /// How the score was computed.
    pub score_type: ScoreType,
    /// Document fields the schema declares `returned` and the request asks
    /// for; absent when the request asks for no field (ADR-0043).
    #[schema(value_type = Option<Object>, nullable = false)]
    pub fields: Option<FreeObject>,
    /// Text of the passage; absent when the request sets `return.text` to
    /// `false`.
    #[schema(nullable = false)]
    pub text: Option<String>,
}

/// Body of the `200` of `POST /v1/indexes/{index}/search`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchResponse {
    /// Results, best first.
    pub results: Vec<SearchResult>,
    /// Size of the pool the request ran on.
    pub pool_size: u32,
    /// Generation read by the request.
    pub generation: u64,
    /// Duration of the whole treatment, in milliseconds.
    pub took_ms: f64,
    /// Whether the query text was truncated to `max_tokens`.
    pub truncated_query: bool,
    /// Identifier of the request (ADR-0098).
    pub request_id: String,
}

/// One result of a multi-index search: a result, with its index.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchMultiResult {
    /// Index the result comes from.
    pub index: String,
    /// External identifier of the document of the passage.
    pub doc_id: String,
    /// Order of the passage in its document.
    pub passage: u32,
    /// `doc_id#ord`.
    pub key: String,
    /// Score of the result, in the fusion between indexes.
    pub score: f64,
    /// How the score was computed.
    pub score_type: MultiScoreType,
    /// Document fields this index declares `returned` and the request asks for.
    #[schema(value_type = Option<Object>, nullable = false)]
    pub fields: Option<FreeObject>,
    /// Text of the passage.
    #[schema(nullable = false)]
    pub text: Option<String>,
}

/// State of one target of a multi-index search (ADR-0110).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchTargetState {
    /// Name of the index.
    pub index: String,
    /// Generation the request read in that index.
    pub generation: u64,
    /// Size of the pool the request ran on in that index.
    pub pool_size: u32,
}

/// Body of the `200` of `POST /v1/search`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchMultiResponse {
    /// Results of every target, fused and best first.
    pub results: Vec<SearchMultiResult>,
    /// State of each target, in the order of the request.
    pub targets: Vec<SearchTargetState>,
    /// Duration of the whole request, in milliseconds.
    pub took_ms: f64,
    /// Whether the query text was truncated to `max_tokens`.
    pub truncated_query: bool,
    /// Identifier of the request (ADR-0098).
    pub request_id: String,
}

/// Adds the `413` of the body bound to every operation.
///
/// The bound is a layer of the whole instance (`crate::http::with_body_limit`),
/// so every route of `docs/05-api.md` may answer `413 payload_too_large`,
/// probes included (ADR-0102, decision 1: "the same bound holds for every
/// request"). The response is declared once, here, rather than route by route.
struct BodyBound;

impl Modify for BodyBound {
    fn modify(&self, openapi: &mut OpenApi) {
        let response = ResponseBuilder::new()
            .description(
                "Body of the request above the bound of the instance, 32 MiB \
                 (ADR-0102, decision 1)",
            )
            .content(
                "application/json",
                Content::new(Some(RefOr::Ref(Ref::from_schema_name("Error")))),
            )
            .build();
        for item in openapi.paths.paths.values_mut() {
            for operation in [
                &mut item.get,
                &mut item.put,
                &mut item.post,
                &mut item.delete,
                &mut item.options,
                &mut item.head,
                &mut item.patch,
                &mut item.trace,
            ]
            .into_iter()
            .flatten()
            {
                operation
                    .responses
                    .responses
                    .insert("413".to_owned(), RefOr::T(response.clone()));
            }
        }
    }
}

/// Adds the `bearerAuth` scheme to the generated specification.
///
/// The scheme describes the API key of ADR-0100: an HTTP `Authorization`
/// header, scheme `Bearer`. The requirement is set at the level of the document
/// by the `#[openapi(security(...))]` attribute of [`ApiDoc`], so a route added
/// without `security(())` inherits it (ADR-0111, decision 4).
struct BearerAuth;

impl Modify for BearerAuth {
    fn modify(&self, openapi: &mut OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearerAuth",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some(
                        "API key of the instance, declared in `crible.toml` as a SHA-256 digest \
                         with a name and a role (ADR-0100). The header is `Authorization: Bearer \
                         <key>`.",
                    ))
                    .build(),
            ),
        );
    }
}

/// `POST /v1/indexes/{index}/compact` takes no body (ADR-0107).
///
/// The annotation lives here, and not on the handler: the handler reads a
/// `Bytes` extractor to refuse a non-empty body, which `axum_extras` would
/// describe as a request body.
#[utoipa::path(
    post,
    path = "/v1/indexes/{index}/compact",
    tag = "maintenance",
    description = "Compact the index into its next epoch. The route takes no body; a non-empty \
                   body is `400 invalid_request`. Role: `admin` only (ADR-0095).",
    params(("index" = String, Path, description = "Name of the index"), ("wait" = Option<bool>, Query, description = "Wait for the job under `wait_timeout_ms` (ADR-0067)")),
    responses(
        (status = 200, description = "The job finished within `wait_timeout_ms`", body = Job),
        (status = 202, description = "The job was accepted", body = Job),
        (status = 400, description = "Non-empty body", body = Error),
        (status = 401, description = "Missing, malformed or unknown API key", body = Error),
        (status = 403, description = "Insufficient role", body = Error),
        (status = 404, description = "Unknown index", body = Error),
        (status = 500, description = "Unexpected failure", body = Error),
        (status = 503, description = "Write queue full, server draining or SQLite lock", body = Error)
    )
)]
pub fn compact_doc() {}

/// `POST /v1/indexes/{index}/exports` takes no body (ADR-0107).
#[utoipa::path(
    post,
    path = "/v1/indexes/{index}/exports",
    tag = "maintenance",
    description = "Export the live data of the index to `exchange/outbox`. The route takes no \
                   body; a non-empty body is `400 invalid_request`. Role: `admin` only \
                   (ADR-0095).",
    params(("index" = String, Path, description = "Name of the index"), ("wait" = Option<bool>, Query, description = "Wait for the job under `wait_timeout_ms` (ADR-0067)")),
    responses(
        (status = 200, description = "The job finished within `wait_timeout_ms`", body = Job),
        (status = 202, description = "The job was accepted", body = Job),
        (status = 400, description = "Non-empty body, or destination already occupied", body = Error),
        (status = 401, description = "Missing, malformed or unknown API key", body = Error),
        (status = 403, description = "Insufficient role", body = Error),
        (status = 404, description = "Unknown index", body = Error),
        (status = 500, description = "Unexpected failure", body = Error),
        (status = 503, description = "Write queue full, generation race or SQLite lock", body = Error)
    )
)]
pub fn exports_doc() {}

/// `POST /v1/indexes/{index}/snapshots` takes no body (ADR-0107).
#[utoipa::path(
    post,
    path = "/v1/indexes/{index}/snapshots",
    tag = "maintenance",
    description = "Snapshot the epoch of the index to `exchange/snapshots`; the writer is \
                   suspended during the copy and the reads continue. The route takes no body; a \
                   non-empty body is `400 invalid_request`. Role: `admin` only (ADR-0095).",
    params(("index" = String, Path, description = "Name of the index"), ("wait" = Option<bool>, Query, description = "Wait for the job under `wait_timeout_ms` (ADR-0067)")),
    responses(
        (status = 200, description = "The job finished within `wait_timeout_ms`", body = Job),
        (status = 202, description = "The job was accepted", body = Job),
        (status = 400, description = "Non-empty body, or destination already occupied", body = Error),
        (status = 401, description = "Missing, malformed or unknown API key", body = Error),
        (status = 403, description = "Insufficient role", body = Error),
        (status = 404, description = "Unknown index", body = Error),
        (status = 500, description = "Unexpected failure", body = Error),
        (status = 503, description = "Write queue full or SQLite lock", body = Error)
    )
)]
pub fn snapshots_doc() {}

/// The specification of the API, served by `GET /openapi.json`.
///
/// OpenAPI 3.1.0, `info.version` at the package version, no `servers` array
/// (ADR-0111, decisions 3 and 4). `security` at the level of the document
/// requires `bearerAuth` on every operation; only the two probes declare
/// `security(())`, and `/openapi.json` and `/docs` stay outside the
/// specification (ADR-0111, decision 5).
#[derive(OpenApiDerive)]
#[openapi(
    version = "3.1.0",
    info(
        title = "Crible API",
        version = env!("CARGO_PKG_VERSION"),
        description = "Dense and BM25 search over structured data. The routes, the bodies and the \
                       error format are documented in `docs/05-api.md`, which is the reference of \
                       the list of the routes (ADR-0111)."
    ),
    paths(
        crate::http::health,
        crate::http::ready,
        crate::http::list_jobs,
        crate::http::get_job,
        crate::indexes::create_index,
        crate::indexes::list_indexes,
        crate::indexes::get_index,
        crate::indexes::patch_index,
        crate::indexes::delete_index,
        crate::documents::post_documents,
        crate::documents::patch_documents,
        crate::documents::get_document,
        crate::batches::post_batch,
        crate::search::search,
        crate::search_multi::search,
        compact_doc,
        exports_doc,
        snapshots_doc,
    ),
    components(schemas(
        Error,
        ErrorBody,
        Health,
        Ready,
        NotReady,
        IndexField,
        FieldType,
        IndexSchema,
        ReferencePrecision,
        ScanStrategy,
        Fusion,
        RrfFusion,
        RrfMethod,
        RelativeScoreFusion,
        RelativeScoreMethod,
        LexicalConfig,
        SearchConfig,
        IngestConfig,
        CompactionConfig,
        IndexConfig,
        IndexFieldPatch,
        IndexSchemaPatch,
        LexicalPatch,
        SearchPatch,
        IngestPatch,
        CompactionPatch,
        IndexPatch,
        IndexListEntry,
        IndexList,
        IndexSizes,
        LastBatch,
        IndexState,
        IndexDetail,
        JobState,
        JobKind,
        JobProgress,
        JobError,
        Job,
        JobList,
        BatchMode,
        OnlinePassage,
        OnlineDocument,
        OnlineBatchRequest,
        DocumentPatch,
        MetadataPatchRequest,
        DepositedBatchRequest,
        DocumentPassage,
        DocumentDetail,
        SearchMode,
        GroupBy,
        Dedup,
        Filter,
        SearchReturn,
        SearchRequest,
        SearchTarget,
        SearchMultiRequest,
        ScoreType,
        MultiScoreType,
        SearchResult,
        SearchResponse,
        SearchMultiResult,
        SearchTargetState,
        SearchMultiResponse,
    )),
    modifiers(&BearerAuth, &BodyBound),
    security(("bearerAuth" = []))
)]
pub struct ApiDoc;

/// The specification as a JSON value, for the tests and the publication.
///
/// # Errors
///
/// The error of the serialisation of the generated document, which cannot
/// happen: the document is built from serialisable types.
pub fn specification() -> Result<Value, serde_json::Error> {
    serde_json::to_value(ApiDoc::openapi())
}
