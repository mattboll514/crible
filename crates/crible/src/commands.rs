// SPDX-License-Identifier: Apache-2.0

//! Implementations of the first Crible commands.
//!
//! References: `docs/phases/P1-stockage-et-recherche-exacte.md` (task
//! `CLI-PREMIERES-COMMANDES`), `docs/02-architecture.md` ("Organisation sur
//! disque"), `docs/05-api.md` ("Recherche : réponse"), `docs/07` (the binary
//! `crible`).
//!
//! # Layout the commands read and write
//!
//! ```text
//! <data>/indexes/<index>/index.json        configuration of the index
//! <data>/indexes/<index>/CURRENT_EPOCH     written by `index create`
//! <models>/<model>/model.json              resolved model configuration
//! ```
//!
//! The index directory is the one of `docs/02-architecture.md`, rooted at
//! `--data`: `index create` copies the configuration to `index.json` before the
//! store opens the index, so the store can cross-check the frozen schema it
//! builds the filter structures from.
//!
//! The model configuration is the `<models>/<name>/model.json` file of
//! `docs/04-donnees-et-ingestion.md`. Models live on their own volume
//! (`[paths] models = "/models"` of `docs/07-stack-et-deploiement.md`), a
//! sibling of the data volume, so every command that reads one takes its root
//! as `--models`. P1 has no model of its own; a project that brings its vectors
//! writes an `external` model configuration (`dims` and `normalize`), which
//! `crible attach` reads.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crible_core::batch::BatchMode;
use crible_core::config::IndexConfig;
use crible_core::error::{CribleError, ErrorCode};
use crible_core::filter::Filter;
use crible_core::model::{ModelConfig, ModelKind};
use crible_encode::{
    DEFAULT_QUERY_CACHE_ENTRIES, DEFAULT_QUERY_PRECISION, Model, ModelRegistry, Precision,
    QueryEncoder, TokenCounter,
};
use crible_search::dense::exact::{DenseSearcher, Hit};
use crible_search::dense::{DenseQuery, search_grouped as dense_search_grouped};
use crible_search::group::Grouping;
use crible_search::pool::Pool;
use crible_search::scan_pool::ScanPool;
use crible_store::attach::{AttachReport, attach};
use crible_store::generation::{
    CURRENT_EPOCH_FILE_NAME, GenerationStore, INDEX_CONFIG_FILE_NAME, INDEXES_DIRECTORY,
    INITIAL_EPOCH, IndexState, write_index_config,
};
use crible_store::online;
use crible_store::patch::{self, PatchSource};
use serde_json::{Value, json};

use crate::vector;

/// Highest `k` accepted by a search (`docs/05-api.md`, "Recherche : requête").
const MAX_K: usize = 1_000;

/// Number of characters of a passage shown by the readable output of `search`.
///
/// The full text stays in the JSON response (`docs/05-api.md`, "Recherche :
/// réponse"); the excerpt is a display choice, so a demo reads one line per
/// passage instead of a page of body text.
const EXCERPT_CHARS: usize = 160;

/// Output format of `crible search`.
///
/// `json` is the default and the contract of `docs/05-api.md`: the CLI tests and
/// the scripts parse it. `text` is a human-readable report for the J2 demo; it
/// carries the same facts, with the passage text cut to [`EXCERPT_CHARS`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum SearchFormat {
    /// JSON response of `docs/05-api.md`.
    #[default]
    Json,
    /// Human-readable report: key, score, fields and a text excerpt.
    Text,
}

/// `crible index create --data <dir> --config <json>`.
pub fn create_index(data: &Path, config_path: &Path) -> Result<(), CribleError> {
    let raw = fs::read_to_string(config_path).map_err(|error| {
        invalid_request(format!(
            "cannot read the index configuration {}: {error}",
            config_path.display()
        ))
    })?;
    let config = IndexConfig::from_json_str(&raw)?;
    let name = path_component(&config.name, "index name")?;
    let index_dir = index_directory(data, name);

    // `write_index_config` decides alone: it refuses an index that already
    // holds an epoch, and replaces a residual `index.json` left by a creation
    // that stopped before its first epoch. The store then cross-checks the
    // schema it builds the filter structures from.
    write_index_config(&index_dir, &raw)?;

    let store = GenerationStore::create(&index_dir, INITIAL_EPOCH, &config.schema)?;
    println!(
        "index '{}' created at {} (epoch {}, generation {})",
        name,
        index_dir.display(),
        store.epoch().get(),
        store.generation().get()
    );
    Ok(())
}

/// `crible attach --data <dir> --models <dir> --index <i> <batch-dir>`.
pub fn attach_batch(
    data: &Path,
    models: &Path,
    index: &str,
    batch: &Path,
) -> Result<(), CribleError> {
    let name = path_component(index, "index name")?;
    let (index_dir, config) = load_index(data, name)?;
    let model = load_model(models, &config.model)?;
    let model_dir = model_directory(models, &config.model)?;
    // Control 8 of `docs/04` counts the tokens of every passage with the
    // tokenizer of an `onnx` model, without loading ONNX Runtime (INV-11). An
    // `external` model has no tokenizer and is out of that control. The
    // configuration was read once by `load_model`, so the counter reuses it
    // instead of reading `model.json` again.
    let token_counter = match model.kind {
        ModelKind::External => None,
        ModelKind::Onnx => Some(TokenCounter::load_with_config(&model_dir, &model)?),
    };
    // Control 9 of `docs/04` re-encodes a sample in fp32 (INV-12). The graph is
    // loaded lazily, by this closure, at the point the attach reaches the
    // check: a batch refused earlier (another model, a bad checksum, an
    // over-long passage) never pays the ONNX Runtime session (ADR-0053).
    let load_encoder = || Model::load_precision(&model_dir, Precision::Fp32).map(Arc::new);
    let document_encoder: Option<&dyn Fn() -> Result<Arc<Model>, CribleError>> = match model.kind {
        ModelKind::External => None,
        ModelKind::Onnx => Some(&load_encoder),
    };
    // Write opening: the leftover online batches of `<data>/tmp` are removed
    // here, under the presence lock that protects a live lot (ADR-0056).
    let mut store = open_store_for_write(data, &index_dir, &config.schema)?;
    let report = attach(
        &mut store,
        batch,
        &config,
        &model,
        token_counter.as_ref(),
        document_encoder,
    )?;
    print_report(name, &report);
    Ok(())
}

/// `crible ingest --data <dir> --models <dir> --index <i> <documents.jsonl>`.
///
/// One JSON document per line, in the shape of the `documents` member of
/// `docs/05-api.md`; the passages without a vector are encoded in fp32 by the
/// model of the index (ADR-0055). The command writes the internal batch under
/// `<data>/tmp`, applies it with `attach`, and prints the same report.
pub fn ingest_documents(
    data: &Path,
    models: &Path,
    index: &str,
    documents: &Path,
) -> Result<(), CribleError> {
    let name = path_component(index, "index name")?;
    let (index_dir, config) = load_index(data, name)?;
    let model = load_model(models, &config.model)?;
    let model_dir = model_directory(models, &config.model)?;
    let token_counter = match model.kind {
        ModelKind::External => None,
        ModelKind::Onnx => Some(TokenCounter::load_with_config(&model_dir, &model)?),
    };
    // Encoded in fp32, stored at the reference precision of the index: the
    // session is loaded lazily, once, by the online ingestion, and handed to
    // the acceptance check of the attach (ADR-0054, ADR-0055).
    let load_encoder = || Model::load_precision(&model_dir, Precision::Fp32).map(Arc::new);
    let document_encoder: Option<&dyn Fn() -> Result<Arc<Model>, CribleError>> = match model.kind {
        ModelKind::External => None,
        ModelKind::Onnx => Some(&load_encoder),
    };
    // Write opening: same cleanup as `attach`, and the ingestion then takes the
    // presence lock of its own batch (ADR-0056).
    let mut store = open_store_for_write(data, &index_dir, &config.schema)?;
    let report = online::ingest(
        &mut store,
        &online::OnlineIngest {
            documents,
            data_dir: data,
            index_config: &config,
            model_config: &model,
            token_counter: token_counter.as_ref(),
            document_encoder,
            mode: BatchMode::Upsert,
        },
    )?;
    print_report(name, &report);
    Ok(())
}

/// `crible patch --data <dir> --index <i> <fichier>.jsonl`.
///
/// Reads a JSONL patch and applies it, as ADR-0060 defines it: the fields of
/// each named document are merged, `doc_hash` does not change, one generation
/// G+1 is committed when at least one document changes, and no generation is
/// created otherwise. No model is loaded and no passage is encoded, so the
/// command takes no `--models`.
///
/// The file is parsed before the writer opens: a file above the bounds of
/// ADR-0060, or one that is not a list of `{doc_id, fields}`, is refused without
/// touching the index. The JSONL file is a positional argument, as the batch
/// directory of `crible attach` is; `--data` keeps its meaning of data root.
pub fn patch_documents(data: &Path, index: &str, file: &Path) -> Result<(), CribleError> {
    let name = path_component(index, "index name")?;
    let (index_dir, config) = load_index(data, name)?;
    let patches = patch::read_file(file)?;
    // Write opening: same recovery and same cleanup of the stale online batches
    // as `attach` and `ingest`.
    let mut store = open_store_for_write(data, &index_dir, &config.schema)?;
    let report = patch::apply(&mut store, &config.schema, PatchSource::File, &patches)?;
    println!(
        "{}",
        compact(&json!({
            "updated": report.updated,
            "unchanged": report.unchanged,
            "generation": report.generation.get(),
        }))?
    );
    Ok(())
}

/// `crible compact --data <dir> --index <i>`.
///
/// Compacts an index into the next epoch without its dead passages
/// (`docs/02-architecture.md`, "Suppressions, pierres tombales et compaction";
/// ADR-0006, ADR-0017). The command ignores the `dead_ratio` of the
/// configuration that triggers an automatic compaction: the operator asks for
/// it, so an index that holds no dead passage is rewritten and renumbered too.
///
/// The report gives the state before the compaction, so a log keeps the dead
/// ratio that justified it, then what the compaction moved.
pub fn compact_index(data: &Path, index: &str) -> Result<(), CribleError> {
    let name = path_component(index, "index name")?;
    let (index_dir, config) = load_index(data, name)?;
    // Write opening: same recovery and same cleanup of the stale online batches
    // as the other write commands; the compaction suspends the writer, which is
    // this exclusive `&mut`, and the readers keep their snapshots (ADR-0017).
    let mut store = open_store_for_write(data, &index_dir, &config.schema)?;
    let before = store.state()?;
    println!(
        "index '{}': epoch {}, generation {}, {} live and {} dead passages (dead ratio {:.4})",
        name,
        before.epoch.get(),
        before.generation.get(),
        before.live_passages,
        before.dead_passages,
        before.dead_ratio(),
    );
    let report = crible_store::compact::compact(&mut store, &config)?;
    println!(
        "index '{}' compacted: epoch {} -> {}, generation {}, {} live passages, \
         {} removed",
        name,
        report.from_epoch.get(),
        report.to_epoch.get(),
        report.generation.get(),
        report.live_passages,
        report.removed_passages,
    );
    Ok(())
}

/// `crible rebuild-lexical --data <dir> --index <i>`.
///
/// Rebuilds the lexical index of the committed epoch from the texts
/// `records.sqlite` stores (`RECONSTRUIRE-LEXICAL`, `docs/03`, "Branche lexicale",
/// "Reconstruction"). The job exists because a change of the analysis chain —
/// `lexical.language` or `lexical.positions` — changes no text: the generations
/// and the payload of the last tantivy commit stay such that `M ≤ Gt ≤ G`, so the
/// recovery of a write open has nothing to repair (`ADR-0088`, decision 2), and
/// only an explicit rebuild applies the new chain.
///
/// The command opens the index for **writing**, as every reconstruction of a
/// derived directory does (`ADR-0069`, `ADR-0087`), then rebuilds. It publishes
/// no generation, no epoch and no manifest: the reference state is unchanged, and
/// the rebuilt index carries the same generation as its payload. Running it twice
/// leaves the same index.
///
/// An epoch that never carried a passage has nothing to rebuild and gains no
/// `lexical/` directory (`docs/02`, step 4): the command says so instead of
/// reporting a rebuild that wrote nothing.
pub fn rebuild_lexical(data: &Path, index: &str) -> Result<(), CribleError> {
    let name = path_component(index, "index name")?;
    let (index_dir, config) = load_index(data, name)?;
    let mut store = open_store_for_write(data, &index_dir, &config.schema)?;
    let state = store.state()?;
    let generation = crible_store::generation::rebuild_lexical(&mut store)?;
    if state.live_passages == 0 {
        println!(
            "index '{}': nothing to rebuild, the reference state at generation {} holds no live \
             passage",
            name,
            generation.get(),
        );
    } else {
        println!(
            "index '{}': lexical index rebuilt from the reference state at generation {}",
            name,
            generation.get(),
        );
    }
    Ok(())
}

/// `crible export --data <dir> --models <dir> --exchange <dir> --index <i>`.
///
/// Writes a `replace_all` batch of the live data of the published generation
/// into `<exchange>/outbox/<index>-g<G>/` (`docs/04`, "Export, instantanés,
/// restauration"). The export reads the published snapshot through its own
/// read-only connection, so it never suspends the writer. The model
/// configuration is read because the manifest of the batch declares it, with
/// `dims` and `normalize`, which the index configuration does not carry.
pub fn export_index(
    data: &Path,
    models: &Path,
    exchange: &Path,
    index: &str,
) -> Result<(), CribleError> {
    let name = path_component(index, "index name")?;
    let (index_dir, config) = load_index(data, name)?;
    let model = load_model(models, &config.model)?;
    // The write open runs the recovery of INV-04 and rebuilds a missing
    // `codes.bin`, whose array the snapshot then loads; `<data>/tmp` is left
    // alone, as for a read command.
    let store = GenerationStore::open(&index_dir, &config.schema)?;
    let report = crible_store::export::export(&store, &config, &model, exchange)?;
    println!(
        "index '{}' exported to {}: generation {}, {} document(s), {} passage(s)",
        name,
        report.directory.display(),
        report.generation.get(),
        report.documents,
        report.passages,
    );
    Ok(())
}

/// `crible snapshot --data <dir> --exchange <dir> --index <i>`.
///
/// Takes a coherent copy of the current epoch into
/// `<exchange>/snapshots/<index>-e<E>-g<G>/` (`docs/04`, "Règles de
/// l'instantané"; ADR-0064, ADR-0065). The command is **offline**: the operator
/// stops the server first, because a process cannot suspend the writer of
/// another. Inside this process, the write opening holds the single writer, so
/// no commit can run while the copy is taken.
///
/// The command encodes nothing and takes no `--models`: neither the copy nor the
/// restore loads the model (ADR-0065, decision 6).
pub fn snapshot_index(data: &Path, exchange: &Path, index: &str) -> Result<(), CribleError> {
    let name = path_component(index, "index name")?;
    let (index_dir, config) = load_index(data, name)?;
    let mut store = open_store_for_write(data, &index_dir, &config.schema)?;
    let report = crible_store::snapshot::create(&mut store, &config, exchange)?;
    println!(
        "index '{}' snapshot to {}: epoch {}, generation {}, {} document(s), {} passage(s), \
         {} bytes",
        report.index,
        report.directory.display(),
        report.epoch.get(),
        report.generation.get(),
        report.documents,
        report.passages,
        report.bytes,
    );
    Ok(())
}

/// `crible restore --data <dir> --snapshot <dir>`.
///
/// Installs a snapshot as the epoch `E+1` of the index it names, the generation
/// left at the snapshot's G, then switches `CURRENT_EPOCH` (`docs/04`, "Règles
/// de la restauration"; ADR-0065). The command is offline, encodes nothing and
/// takes no `--models`. The replaced epoch is **kept** on disk and named in the
/// report: it may hold passages no valid copy carries (INV-14), so only the
/// operator removes it.
pub fn restore_index(data: &Path, snapshot: &Path) -> Result<(), CribleError> {
    // The stale online batches of `<data>/tmp` are removed inside the restore,
    // once the snapshot and the target have been checked (ADR-0056, ADR-0065).
    let report = crible_store::snapshot::restore(data, snapshot)?;
    let replaced = match (report.replaced_epoch, report.replaced_generation) {
        (Some(epoch), Some(generation)) => format!(
            "replaced epoch {} (generation {})",
            epoch.get(),
            generation.get()
        ),
        _ => "no epoch replaced".to_owned(),
    };
    println!(
        "index '{}' restored from {}: {}, new epoch {}, generation {}, {} document(s), \
         {} passage(s)",
        report.index,
        snapshot.display(),
        replaced,
        report.epoch.get(),
        report.generation.get(),
        report.documents,
        report.passages,
    );
    Ok(())
}

/// Prints the report of one attach, as `crible attach` and `crible ingest` do.
fn print_report(name: &str, report: &AttachReport) {
    println!(
        "batch '{}' attached to index '{}': generation {} (added {}, replaced {}, \
         unchanged {}, metadata_updated {}, deleted {}, ignored {})",
        report.batch_id,
        name,
        report.generation.get(),
        report.added,
        report.replaced,
        report.unchanged,
        report.metadata_updated,
        report.deleted,
        report.ignored,
    );
    if let Some(acceptance) = report.acceptance {
        println!(
            "  acceptance: {} passage(s) re-encoded, min cosine {:.6}, p1 {:.6}, p50 {:.6}",
            acceptance.sample, acceptance.min_cosine, acceptance.p1, acceptance.p50
        );
    }
}

/// Opens an index for a **write** command: the leftover online batches of
/// `<data>/tmp` are removed first, under the presence lock that protects a live
/// lot (ADR-0056, decision 4), then the store runs its recovery (INV-04).
///
/// A read command calls [`GenerationStore::open_read_only`] directly: `search`
/// and `inspect` never modify `<data>/tmp`, so they cannot delete a lot another
/// index is ingesting, and they never write `codes.bin` (ADR-0056,
/// `ADR-0069`, decision 2).
fn open_store_for_write(
    data: &Path,
    index_dir: &Path,
    schema: &crible_core::config::Schema,
) -> Result<GenerationStore, CribleError> {
    online::remove_stale_batches(data)?;
    GenerationStore::open(index_dir, schema)
}

/// One `crible search` invocation.
///
/// The query comes either from a file of a provided vector (`--vector-file`) or
/// from text (`--query`); the two are mutually exclusive and one is required.
/// A text query needs `--models`, and an `onnx` encoder: the model of the index
/// by default, or the one named by `--model`, which is what lets `synth-small`
/// (an `external` index) be searched by text with a test encoder.
#[derive(Clone, Copy)]
pub struct SearchRequest<'a> {
    /// Root data directory (`<data>/indexes/<index>`).
    pub data: &'a Path,
    /// Index name, as declared in its `index.json`.
    pub index: &'a str,
    /// Vector file, when the caller provides the vector.
    pub vector_file: Option<&'a Path>,
    /// Text query, encoded by the server.
    pub query: Option<&'a str>,
    /// Root models directory; required with `query`.
    pub models: Option<&'a Path>,
    /// Encoder to use for `query`; the model of the index when omitted.
    pub model: Option<&'a str>,
    /// Filter as a JSON object.
    pub filter: Option<&'a str>,
    /// Number of results, between 1 and 1000.
    pub k: usize,
    /// Force the exact dense calculation (`exact` of the search request in
    /// `docs/05-api.md`).
    pub exact: bool,
    /// Regrouping by document (`group_by` of `docs/05-api.md`): `document`, or
    /// none.
    pub group_by: Option<&'a str>,
    /// Passages returned per grouped document, from 1 to 5
    /// (`passages_per_document` of `docs/05-api.md`).
    pub passages_per_document: usize,
    /// Fusion of identical passages (`dedup` of `docs/05-api.md`): `content`,
    /// or none.
    pub dedup: Option<&'a str>,
    /// Output format.
    pub format: SearchFormat,
}

impl std::fmt::Debug for SearchRequest<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A query and a filter value must never reach a log (AGENTS.md,
        // "Confidentialité"), so their presence is shown, never their text.
        formatter
            .debug_struct("SearchRequest")
            .field("data", &self.data)
            .field("index", &self.index)
            .field("vector_file", &self.vector_file)
            .field("query", &self.query.map(|_| "<redacted>"))
            .field("models", &self.models)
            .field("model", &self.model)
            .field("filter", &self.filter.map(|_| "<redacted>"))
            .field("k", &self.k)
            .field("exact", &self.exact)
            .field("group_by", &self.group_by)
            .field("passages_per_document", &self.passages_per_document)
            .field("dedup", &self.dedup)
            .field("format", &self.format)
            .finish()
    }
}

/// Regrouping options of one `crible search`, from its flags.
///
/// `docs/05-api.md` publishes `group_by: "document"` and `dedup: "content"` as
/// the only non-null values, `passages_per_document` from 1 to 5, and
/// `invalid_request` for a parameter out of bounds. The message names the value
/// the flag accepts, never the text of a query or a filter value.
///
/// `passages_per_document` only acts on grouped results (`docs/05-api.md`, "Passages
/// renvoyés par document regroupé"), so it has no effect without `--group-by`;
/// its own domain is checked either way, so a request that leaves it is refused
/// even when nothing is grouped.
fn grouping_of(request: &SearchRequest<'_>) -> Result<Grouping, CribleError> {
    let by_document = match request.group_by {
        None => false,
        Some("document") => true,
        Some(other) => {
            return Err(CribleError::new(
                ErrorCode::InvalidRequest,
                format!("group_by must be \"document\" or absent, got {other:?}"),
            ));
        }
    };
    let dedup_content = match request.dedup {
        None => false,
        Some("content") => true,
        Some(other) => {
            return Err(CribleError::new(
                ErrorCode::InvalidRequest,
                format!("dedup must be \"content\" or absent, got {other:?}"),
            ));
        }
    };
    let grouping = Grouping {
        by_document,
        passages_per_document: request.passages_per_document,
        dedup_content,
    };
    grouping.validate()?;
    Ok(grouping)
}

/// `crible search --data <dir> --index <i> (--vector-file <file> | --query <text>)`.
///
/// The response follows the shape of `docs/05-api.md` ("Recherche : réponse"),
/// with the fields P1 and `ENCODER-REQUETES` can fill: `results`, `pool_size`,
/// `generation`, `took_ms` and `truncated_query`. The dense branch runs the
/// strategy [`crible_search::dense::plan`] chooses from `search.*`, the pool
/// and `--exact` (ADR-0074), but both strategies score with the same cosine, so
/// `score_type` is `"cosine"` either way. `took_ms` is the total, from the
/// entry of the command: it includes reading the index configuration, the
/// vector or the text encoding, the filter, evaluating the pool, opening the
/// readers and the scan itself.
///
/// `truncated_query` is the wire name of the `truncated` flag of
/// `crible_encode::EncodedQuery` (ADR-0051); the HTTP route of P6 keeps that
/// mapping.
///
/// `--group-by`, `--passages-per-document` and `--dedup` are the regrouping of
/// `docs/03-recherche.md` ("Regroupement") and `REGROUPER-RESULTATS`: the dense
/// branch is asked for over-sampled candidates, the regrouping runs before the
/// payload step, and `k` counts documents when `--group-by document` is set. A
/// grouped document may then take up to `--passages-per-document` results in
/// the flat `results` list of `docs/05-api.md`.
///
/// `--format json` (the default) prints that response; `--format text` prints
/// the readable report of [`print_search_text`] instead, for the J2 demo.
pub fn search(request: &SearchRequest<'_>) -> Result<(), CribleError> {
    let started = Instant::now();
    if request.k == 0 || request.k > MAX_K {
        return Err(invalid_request(format!(
            "k must be between 1 and {MAX_K}, got {}",
            request.k
        )));
    }
    // The regrouping options are validated before the index is opened, the
    // model loaded and the pool evaluated: a request the command can refuse
    // pays for none of that.
    let grouping = grouping_of(request)?;
    let name = path_component(request.index, "index name")?;
    let (index_dir, config) = load_index(request.data, name)?;
    let store = GenerationStore::open_read_only(&index_dir, &config.schema)?;
    let (vector, truncated_query) = match (request.vector_file, request.query) {
        (Some(path), None) => (vector::load(path)?, false),
        (None, Some(text)) => encode_text_query(request.models, &config, request.model, text)?,
        (Some(_), Some(_)) => {
            return Err(invalid_request(
                "--vector-file and --query are mutually exclusive; pass one of them",
            ));
        }
        (None, None) => {
            return Err(invalid_request(
                "search needs --vector-file <file> or --query <text>",
            ));
        }
    };
    let filter = match request.filter {
        Some(raw) => {
            let filter = Filter::from_json_str(raw)?;
            filter.validate(&config.schema)?;
            filter
        }
        None => Filter::And(Vec::new()),
    };

    let snapshot = store.snapshot();
    let pool = Pool::evaluate(snapshot.filters(), &filter)?;
    let scan = ScanPool::new(0)?;
    let segments_directory = store.segments_directory();
    let records_path = store.records_path();
    let segments = snapshot.segments().to_vec();
    let searcher =
        scan.install(|| DenseSearcher::open(&segments_directory, &segments, &records_path))?;
    let hits = scan.install(|| {
        dense_search_grouped(
            &snapshot,
            &searcher,
            &pool,
            &DenseQuery {
                vector: &vector,
                k: request.k,
                exact: request.exact,
                grouping: &grouping,
            },
            &config.search,
        )
    })?;
    let took_ms = started.elapsed().as_secs_f64() * 1_000.0;

    match request.format {
        SearchFormat::Json => {
            let results: Vec<Value> = hits
                .iter()
                .map(|hit| {
                    json!({
                        "doc_id": hit.doc_id,
                        "passage": hit.ord,
                        "key": hit.key,
                        "score": hit.score,
                        "score_type": "cosine",
                        "fields": hit.fields,
                        "text": hit.text,
                    })
                })
                .collect();
            let response = json!({
                "results": results,
                "pool_size": pool.len(),
                "generation": snapshot.generation().get(),
                "took_ms": took_ms,
                "truncated_query": truncated_query,
            });
            println!("{}", pretty(&response)?);
        }
        SearchFormat::Text => print_search_text(
            name,
            &hits,
            pool.len(),
            snapshot.generation().get(),
            took_ms,
            truncated_query,
        )?,
    }
    Ok(())
}

/// Prints the readable report of one search: key, score, fields and a text
/// excerpt per result, then the counts of the JSON response.
fn print_search_text(
    index: &str,
    hits: &[Hit],
    pool_size: u32,
    generation: u64,
    took_ms: f64,
    truncated_query: bool,
) -> Result<(), CribleError> {
    println!(
        "index '{index}': {pool_size} passages in the pool, generation {generation}, \
         {} result(s), {took_ms:.1} ms",
        hits.len()
    );
    if truncated_query {
        println!("note: the query was truncated to the max_tokens of the model");
    }
    for (position, hit) in hits.iter().enumerate() {
        println!(
            "{}. {}  score {:.6} (cosine)",
            position + 1,
            hit.key,
            hit.score
        );
        println!("   fields: {}", compact(&hit.fields)?);
        println!("   text: {}", excerpt(&hit.text, EXCERPT_CHARS));
    }
    Ok(())
}

/// Returns one line of at most `limit` characters, cut on a character boundary.
///
/// Whitespace and line breaks are folded into single spaces, so a passage of
/// several paragraphs still takes one line. An ellipsis marks a cut text.
fn excerpt(text: &str, limit: usize) -> String {
    let flattened = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut characters = flattened.chars();
    let head: String = characters.by_ref().take(limit).collect();
    if characters.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

/// Encodes a text query with the encoder named by `model`, or with the model of
/// the index.
///
/// The query cache and the default precision are those of
/// `docs/03-recherche.md` and ADR-0052 (`fp32`). The instance configuration of
/// P6 will pass `[limits].query_cache_entries` and
/// `[encoder].query_precision`; the CLI has no `crible.toml` loader yet, so it
/// uses the documented defaults.
fn encode_text_query(
    models: Option<&Path>,
    index: &IndexConfig,
    model: Option<&str>,
    text: &str,
) -> Result<(Vec<f32>, bool), CribleError> {
    let models = models.ok_or_else(|| {
        invalid_request(
            "--query needs --models <dir>, the [paths] models root of crible.toml holding \
             <model>/model.json",
        )
    })?;
    let name = model.unwrap_or(&index.model);
    let directory = model_directory(models, name)?;
    let config = load_model(models, name)?;
    if config.kind != ModelKind::Onnx {
        return Err(invalid_request(format!(
            "index '{}' uses model '{}' of kind external, which has no encoder; --query needs an \
             onnx model, pass --model <name>",
            index.name, name
        )));
    }

    let encoder = QueryEncoder::new(
        &directory,
        Arc::new(ModelRegistry::new()),
        DEFAULT_QUERY_CACHE_ENTRIES,
    );
    let encoded = encoder.encode_query(text, DEFAULT_QUERY_PRECISION)?;
    Ok((encoded.vector, encoded.truncated))
}

/// `crible inspect --data <dir> [--index <i>]`.
///
/// Like every read of the store, opening the index runs its recovery
/// (`docs/02-architecture.md`, "Reprise") and takes the SQLite write lock:
/// `inspect` removes leftover `tmp-*` entries and segments SQLite does not
/// reference, and rewrites a missing manifest. It therefore needs a writable
/// mount, not a read-only one. It leaves the online batches of `<data>/tmp`
/// alone, and it never rebuilds a missing `codes.bin`: the codes are quantized
/// in RAM (ADR-0056; ADR-0069, decision 2).
pub fn inspect(data: &Path, index: Option<&str>) -> Result<(), CribleError> {
    let names = match index {
        Some(name) => vec![path_component(name, "index name")?.to_owned()],
        None => list_indexes(data)?,
    };
    if names.is_empty() {
        return Err(invalid_request(format!(
            "no index found under {}",
            indexes_directory(data).display()
        )));
    }
    for name in names {
        let (index_dir, config) = load_index(data, &name)?;
        let store = GenerationStore::open_read_only(&index_dir, &config.schema)?;
        let state = store.state()?;
        print_state(&name, &state);
    }
    Ok(())
}

/// Prints one index state, as `crible inspect` reports it.
fn print_state(name: &str, state: &IndexState) {
    println!("index {name}");
    println!("  epoch: {}", state.epoch.get());
    println!("  generation: {}", state.generation.get());
    println!("  documents: {}", state.documents);
    println!(
        "  passages: {} live, {} dead",
        state.live_passages, state.dead_passages
    );
    println!("  dead ratio: {:.4}", state.dead_ratio());
    println!("  segments: {}", state.segments.len());
    for segment in &state.segments {
        println!("    {}: {} vectors", segment.segment_id, segment.count);
    }
    let memory = state.filter_memory;
    println!(
        "  bitmaps: {} bytes (live {}, keywords {}, booleans {}, columns {})",
        memory.total(),
        memory.live,
        memory.keywords,
        memory.booleans,
        memory.columns
    );
    println!("  codes: {} bytes", state.codes_memory);
}

/// Directory holding every index of an instance.
fn indexes_directory(data: &Path) -> PathBuf {
    data.join(INDEXES_DIRECTORY)
}

/// Directory of one index.
fn index_directory(data: &Path, name: &str) -> PathBuf {
    indexes_directory(data).join(name)
}

/// Reads and validates the configuration of an index.
///
/// The directory name and the `name` of the configuration must agree: a command
/// that names `synth-small` must not open another index.
fn load_index(data: &Path, name: &str) -> Result<(PathBuf, IndexConfig), CribleError> {
    let index_dir = index_directory(data, name);
    let path = index_dir.join(INDEX_CONFIG_FILE_NAME);
    let raw = fs::read_to_string(&path).map_err(|error| {
        CribleError::new(
            ErrorCode::UnknownIndex,
            format!(
                "cannot read the configuration of index '{name}' at {}: {error}",
                path.display()
            ),
        )
    })?;
    let config = IndexConfig::from_json_str(&raw)?;
    // The export writes under `<exchange>/outbox/<index name>-g<G>/`, so the
    // name an index directory holds must be one path component, exactly as the
    // name the caller asks for (`export.rs` therefore keeps no copy of this
    // check).
    path_component(&config.name, "index name")?;
    if config.name != name {
        return Err(invalid_request(format!(
            "index directory '{}' holds a configuration for index '{}'",
            index_dir.display(),
            config.name
        )));
    }
    Ok((index_dir, config))
}

/// Reads the resolved model configuration of `model` under the models root.
fn load_model(models: &Path, model: &str) -> Result<ModelConfig, CribleError> {
    let path = model_directory(models, model)?.join("model.json");
    let raw = fs::read_to_string(&path).map_err(|error| {
        invalid_request(format!(
            "cannot read the model configuration of '{model}' at {}: {error}; pass \
             --models <dir>, the [paths] models root of crible.toml, holding \
             <model>/model.json",
            path.display()
        ))
    })?;
    ModelConfig::from_json_str(&raw)
}

/// Directory of one model under the models root.
fn model_directory(models: &Path, model: &str) -> Result<PathBuf, CribleError> {
    let model = path_component(model, "model name")?;
    Ok(models.join(model))
}

/// Lists the index directories that carry a `CURRENT_EPOCH`, sorted by name.
fn list_indexes(data: &Path) -> Result<Vec<String>, CribleError> {
    let directory = indexes_directory(data);
    let entries = fs::read_dir(&directory).map_err(|error| {
        invalid_request(format!(
            "cannot list the indexes under {}: {error}",
            directory.display()
        ))
    })?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            internal(format!(
                "cannot read an entry of {}: {error}",
                directory.display()
            ))
        })?;
        let path = entry.path();
        if !path.is_dir() || !path.join(CURRENT_EPOCH_FILE_NAME).is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            return Err(internal(format!(
                "{} has no UTF-8 name, so it cannot be an index name",
                path.display()
            )));
        };
        names.push(name);
    }
    names.sort();
    Ok(names)
}

/// Accepts `name` only as one path component.
fn path_component<'a>(name: &'a str, what: &str) -> Result<&'a str, CribleError> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        return Err(invalid_request(format!(
            "{what} '{name}' must be a single path component"
        )));
    }
    Ok(name)
}

/// Serializes a response, or reports the serialization fault.
fn pretty(value: &Value) -> Result<String, CribleError> {
    serde_json::to_string_pretty(value)
        .map_err(|error| internal(format!("cannot serialize the response as JSON: {error}")))
}

/// Serializes one JSON value on a single line, or reports the fault.
fn compact(value: &Value) -> Result<String, CribleError> {
    serde_json::to_string(value)
        .map_err(|error| internal(format!("cannot serialize a value as JSON: {error}")))
}

/// Error raised when the request itself is malformed.
fn invalid_request(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::InvalidRequest, message)
}

/// Error raised when the process cannot carry out the request.
fn internal(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::Internal, message)
}

#[cfg(test)]
mod tests {
    use super::{EXCERPT_CHARS, SearchFormat, SearchRequest, excerpt};
    use std::path::Path;

    #[test]
    fn debug_never_prints_the_query_text_nor_a_filter_value() {
        // A log line must not carry a query or a filter value (AGENTS.md,
        // "Confidentialité"): the derived `Debug` is replaced by a masked one,
        // which shows their presence only.
        let request = SearchRequest {
            data: Path::new("/data"),
            index: "legifacile",
            vector_file: None,
            query: Some("responsabilité du fait des choses"),
            models: None,
            model: None,
            filter: Some(r#"{"categorie": "SECRET-VALUE"}"#),
            k: 10,
            exact: false,
            group_by: None,
            passages_per_document: 1,
            dedup: None,
            format: SearchFormat::Json,
        };
        let rendered = format!("{request:?}");
        assert!(
            !rendered.contains("responsabilité"),
            "the query text must not be printed: {rendered}"
        );
        assert!(
            !rendered.contains("SECRET-VALUE"),
            "the filter value must not be printed: {rendered}"
        );
        assert!(
            rendered.contains("<redacted>"),
            "the presence of the query and the filter is shown: {rendered}"
        );
    }

    #[test]
    fn excerpt_keeps_a_short_text_whole() {
        assert_eq!(
            excerpt("LOI n° 2024-42 du 26 janvier 2024", 160),
            "LOI n° 2024-42 du 26 janvier 2024"
        );
    }

    #[test]
    fn excerpt_folds_whitespace_and_line_breaks() {
        assert_eq!(
            excerpt(
                "LOI n° 2024-42\n\n  pour contrôler   l'immigration\tet améliorer l'intégration",
                160
            ),
            "LOI n° 2024-42 pour contrôler l'immigration et améliorer l'intégration"
        );
    }

    #[test]
    fn excerpt_cuts_a_long_text_and_marks_the_cut() {
        let text = "droit ".repeat(100);
        let excerpt = excerpt(&text, 20);
        assert_eq!(excerpt, "droit droit droit dr…");
        assert_eq!(
            excerpt.chars().count(),
            21,
            "20 characters plus the ellipsis"
        );
    }

    #[test]
    fn excerpt_cuts_on_a_character_boundary() {
        let text = "é".repeat(EXCERPT_CHARS + 10);
        let excerpt = excerpt(&text, EXCERPT_CHARS);
        assert_eq!(excerpt.chars().count(), EXCERPT_CHARS + 1);
        assert!(excerpt.ends_with('…'));
    }
}
