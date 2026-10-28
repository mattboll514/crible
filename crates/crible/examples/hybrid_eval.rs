// SPDX-License-Identifier: Apache-2.0

//! Measurement harness of MES-10 (quality of the fusion) and MES-11 (French
//! analysis: elisions and identifiers).
//!
//! References: `docs/08-invariants-et-tests.md` (MES-10, MES-11), the task sheet
//! `docs/phases/P5-lexical-et-hybride.md` (`MESURER-HYBRIDE`), and **ADR-0092**
//! (protocol: the 57 labelled questions on the enriched set `data/mes10/`,
//! recall@10 and MRR@10 at the document level with `group_by: "document"` and
//! `k = 10`, the production defaults, the five variants of the « article 1240 »
//! case, the four analysis variants of MES-11, and the decision rule).
//!
//! This example is a harness, not production code, and it changes nothing in
//! `crates/*/src/`. It calls the **public** API only:
//!
//! - `pipeline::search` runs one request (pool, branches, fusion, regrouping,
//!   payload) exactly as the CLI does, so the figures are the engine's;
//! - `lexical::search` is used by `analyze` as the anchor of the replica;
//! - `crible_store::lexical::analyzer::analyzer` is the V0 chain of MES-11.
//!
//! What the harness replicates, and why it says so:
//!
//! - the analysis chain of the four variants of MES-11 (V1 adds an elision
//!   filter, V2 an identifier tokenizer, V3 both), because ADR-0092, decision 5,
//!   asks for a replica **outside the engine**: one tantivy measurement index per
//!   variant, built from the same passage texts;
//! - `rrf` over the replica's lexical ranking and the engine's dense ranking
//!   (secondary series of MES-11), because the engine cannot run a fusion over a
//!   ranking it did not produce.
//!
//! Both replicas are anchored to the engine: `analyze` compares the V0 replica's
//! document top-10 with the one `lexical::search` returns, query by query, and
//! the report gives that agreement rate.
//!
//! Modes:
//!
//! ```text
//! hybrid_eval encode  --models M --queries Q --out O
//! hybrid_eval eval    --data D --index I --column C --queries Q --vectors V --out O
//!                     [--only dense,lexical,rrf,relative_0.5] [--limit N]
//!                     [--case-1240 EXPECTED_ID]
//! hybrid_eval analyze --data D --index I --queries Q --vectors V --out O
//!                     [--limit N]
//! ```

use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crible_core::config::{FusionConfig, IndexConfig};
use crible_core::filter::Filter;
use crible_encode::{ModelRegistry, Precision, QueryEncoder};
use crible_search::dense::exact::DenseSearcher;
use crible_search::group::Grouping;
use crible_search::lexical;
use crible_search::pipeline::{self, Mode, Request};
use crible_search::pool::Pool;
use crible_search::scan_pool::ScanPool;
use crible_store::generation::GenerationStore;
use serde_json::{Map, Value, json};
use tantivy::collector::{Collector, SegmentCollector};
use tantivy::fastfield::Column;
use tantivy::query::{BooleanQuery, Occur, Query, TermQuery};
use tantivy::schema::{IndexRecordOption, Schema, TextFieldIndexing, TextOptions};
use tantivy::tokenizer::{
    AsciiFoldingFilter, Language, LowerCaser, RemoveLongFilter, SimpleTokenizer, Stemmer,
    StopWordFilter, TextAnalyzer, Token, TokenFilter, TokenStream, Tokenizer,
};
use tantivy::{DocId, Index, Score, SegmentReader, TantivyDocument, Term};

/// Field of the internal identifier in the replica, kept as a fast field.
const REPLICA_IID: &str = "iid";
/// Field of the passage text in the replica.
const REPLICA_TEXT: &str = "text";
/// Name the replica registers its chain under.
const REPLICA_TOKENIZER: &str = "mes10";
/// Longest token kept by the production chain (`RemoveLongFilter(40)`).
const MAX_TOKEN_LENGTH: usize = 40;
/// Constant of `rrf` (`docs/03-recherche.md`).
const RRF_K: f64 = 60.0;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().collect();
    let mode = arguments
        .get(1)
        .cloned()
        .ok_or("usage: hybrid_eval <encode|eval|analyze> …")?;
    match mode.as_str() {
        "encode" => encode_mode(&arguments),
        "eval" => eval_mode(&arguments),
        "analyze" => analyze_mode(&arguments),
        other => Err(format!("unknown mode '{other}'").into()),
    }
}

// --------------------------------------------------------------- opening -----

/// One index of the campaign, opened read-only once per process.
struct Opened {
    /// Configuration read from `index.json`.
    config: IndexConfig,
    /// The store the snapshot comes from.
    store: GenerationStore,
}

impl Opened {
    /// Snapshot the campaign holds for the whole run.
    fn snapshot(&self) -> Arc<crible_store::generation::Snapshot> {
        self.store.snapshot()
    }

    /// Opens the index `index` under `data` for reading.
    fn read(data: &Path, index: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let directory = data.join("indexes").join(index);
        let raw = fs::read_to_string(directory.join("index.json"))?;
        let config = IndexConfig::from_json_str(&raw)?;
        let store = GenerationStore::open_read_only(&directory, &config.schema)?;
        Ok(Self { config, store })
    }
}

// ---------------------------------------------------------------- encode -----

/// Encodes every text the campaign needs in fp32 and archives the vectors.
///
/// ADR-0092, decision 3: the query vectors are fp32 (ADR-0052), encoded from
/// `data/models/mdenseon/`, and archived with their SHA-256 — the digest is
/// written by the aggregator, which reads the file this mode writes.
fn encode_mode(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let models = PathBuf::from(required(arguments, "--models")?);
    let queries_path = PathBuf::from(required(arguments, "--queries")?);
    let out = PathBuf::from(required(arguments, "--out")?);

    let questions = read_questions(&queries_path)?;
    let encoder = QueryEncoder::new(models.join("mdenseon"), Arc::new(ModelRegistry::new()), 0);

    let mut vectors: BTreeMap<String, Vec<f32>> = BTreeMap::new();
    let mut truncated: Vec<String> = Vec::new();
    let mut texts: Vec<(String, String)> = Vec::new();
    for question in &questions {
        texts.push((question.id.clone(), question.text.clone()));
    }
    for (name, text) in case_1240_variants() {
        texts.push((name.to_owned(), text.to_owned()));
    }
    for (name, text) in probes() {
        texts.push((name.to_owned(), text.to_owned()));
    }

    for (key, text) in &texts {
        let encoded = encoder.encode_query(text, Precision::Fp32)?;
        if encoded.truncated {
            truncated.push(key.clone());
        }
        vectors.insert(key.clone(), encoded.vector);
    }

    let document = json!({
        "model": "mdenseon",
        "precision": "fp32",
        "dims": vectors.values().next().map(Vec::len).unwrap_or(0),
        "texts": texts.len(),
        "truncated": truncated,
        "vectors": vectors,
    });
    write_json(&out, &document)?;
    println!(
        "{} text(s) encoded in fp32, {} truncated",
        texts.len(),
        document["truncated"].as_array().map(Vec::len).unwrap_or(0)
    );
    Ok(())
}

// ------------------------------------------------------------------ eval -----

/// One measured configuration of `pipeline::search`.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Method {
    /// `mode: dense`, the scan and the rescoring the configuration plans.
    Dense,
    /// `mode: lexical`, BM25 over the same pool.
    Lexical,
    /// `mode: hybrid`, `rrf` at 60.
    Rrf,
    /// `mode: hybrid`, `relative_score` with this `alpha`.
    Relative(f64),
}

impl Method {
    /// Name of the method in the raw document and in the report.
    fn name(self) -> String {
        match self {
            Self::Dense => "dense".to_owned(),
            Self::Lexical => "lexical".to_owned(),
            Self::Rrf => "rrf".to_owned(),
            Self::Relative(alpha) => format!("relative_{alpha}"),
        }
    }

    /// Fusion the request carries; the two other modes ignore it.
    const fn fusion(self) -> FusionConfig {
        match self {
            Self::Relative(alpha) => FusionConfig::RelativeScore { alpha },
            _ => FusionConfig::Rrf,
        }
    }

    /// Pipeline mode of the method.
    const fn mode(self) -> Mode {
        match self {
            Self::Dense => Mode::Dense,
            Self::Lexical => Mode::Lexical,
            Self::Rrf | Self::Relative(_) => Mode::Hybrid,
        }
    }
}

/// The nine measured points: three single branches, `rrf`, five `alpha`.
///
/// ADR-0092, decision 3: `alpha = 0` and `alpha = 1` are coherence controls, not
/// preferences; they must reproduce the lexical branch alone and the dense one.
fn methods() -> Vec<Method> {
    let mut methods = vec![Method::Dense, Method::Lexical, Method::Rrf];
    for alpha in [0.0, 0.3, 0.5, 0.75, 1.0] {
        methods.push(Method::Relative(alpha));
    }
    methods
}

/// Runs the campaign of one index and writes its raw document.
///
/// The document is per index and per column; the aggregator merges the six
/// indexes of the binary column and the six of the exact column, so that a gap
/// can be attributed to the fusion rather than to the scan (ADR-0092, decision
/// 3). Every question of a structure is measured on that structure's index,
/// because a question names its structure in `queries.jsonl`; this is the rule
/// of MES-06, decision 3, reused here.
fn eval_mode(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let data = PathBuf::from(required(arguments, "--data")?);
    let index = required(arguments, "--index")?;
    let column = required(arguments, "--column")?;
    let queries_path = PathBuf::from(required(arguments, "--queries")?);
    let vectors_path = PathBuf::from(required(arguments, "--vectors")?);
    let out = PathBuf::from(required(arguments, "--out")?);
    let limit = optional(arguments, "--limit")
        .map(|value| value.parse::<usize>())
        .transpose()?;
    let case_1240 = optional(arguments, "--case-1240");
    let only = optional(arguments, "--only").map(|text| {
        text.split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_owned)
            .collect::<Vec<String>>()
    });

    // The exact column reads a clone of the index named `<structure>-exact`;
    // the questions stay the ones of the structure.
    let structure = optional(arguments, "--structure")
        .unwrap_or_else(|| index.trim_end_matches("-exact").to_owned());
    let vectors = read_vectors(&vectors_path)?;
    let questions = read_questions(&queries_path)?;
    let mut selected: Vec<&Question> = questions
        .iter()
        .filter(|question| question.structure == structure)
        .filter(|question| question.labelled || question.expected_in_sample)
        .collect();
    if let Some(limit) = limit {
        selected.truncate(limit);
    }

    let opened = Opened::read(&data, &index)?;
    let snapshot = opened.snapshot();
    let pool = Pool::evaluate(snapshot.filters(), &Filter::from_json_str("{}")?)?;
    let scan = ScanPool::new(0)?;
    let segments_directory = opened.store.segments_directory();
    let records_path = opened.store.records_path();
    let segments = snapshot.segments().to_vec();
    let searcher =
        scan.install(|| DenseSearcher::open(&segments_directory, &segments, &records_path))?;
    let grouping = Grouping {
        by_document: true,
        passages_per_document: 1,
        dedup_content: false,
    };
    let depth = crible_search::group::candidates_per_branch(10, &grouping);

    let mut measured: Vec<Method> = methods();
    if let Some(only) = &only {
        measured.retain(|method| only.iter().any(|name| *name == method.name()));
    }

    let started = std::time::Instant::now();
    let mut rows: Vec<Value> = Vec::new();
    for question in &selected {
        let Some(vector) = vectors.get(&question.id) else {
            return Err(format!("no archived vector for '{}'", question.id).into());
        };
        let mut top: Map<String, Value> = Map::new();
        for method in &measured {
            let answer = pipeline::search(
                &snapshot,
                &searcher,
                &pool,
                &Request {
                    mode: method.mode(),
                    vector: Some(vector),
                    text: Some(&question.text),
                    k: 10,
                    exact: false,
                    grouping: &grouping,
                    fusion: method.fusion(),
                },
                &opened.config.search,
            )?;
            let documents: Vec<String> = answer
                .results
                .iter()
                .map(|result| result.hit.doc_id.clone())
                .collect();
            top.insert(method.name(), json!(documents));
        }
        rows.push(json!({
            "id": question.id,
            "corpus": question.corpus,
            "satisfiable": question.satisfiable,
            "labelled": question.labelled,
            "expected_in_sample": question.expected_in_sample,
            "expected": question.expected,
            "top": top,
        }));
        if rows.len().is_multiple_of(20) {
            println!(
                "{index}: {} question(s) measured, {:.1} s",
                rows.len(),
                started.elapsed().as_secs_f64()
            );
        }
    }

    let case = match &case_1240 {
        Some(expected) if structure == "articles" => {
            let mut variants: Vec<Value> = Vec::new();
            for (name, text) in case_1240_variants() {
                let Some(vector) = vectors.get(name) else {
                    return Err(format!("no archived vector for '{name}'").into());
                };
                let mut ranks: Map<String, Value> = Map::new();
                for method in &measured {
                    let answer = pipeline::search(
                        &snapshot,
                        &searcher,
                        &pool,
                        &Request {
                            mode: method.mode(),
                            vector: Some(vector),
                            text: Some(text),
                            k: 10,
                            exact: false,
                            grouping: &grouping,
                            fusion: method.fusion(),
                        },
                        &opened.config.search,
                    )?;
                    let rank = answer
                        .results
                        .iter()
                        .position(|result| &result.hit.doc_id == expected)
                        .map(|position| position + 1);
                    ranks.insert(method.name(), json!(rank));
                }
                variants.push(json!({"name": name, "text": text, "rank": ranks}));
            }
            json!({"expected": expected, "variants": variants})
        }
        Some(expected) => json!({"expected": expected, "skipped": "not the articles index"}),
        None => Value::Null,
    };

    let document = json!({
        "index": index,
        "structure": structure,
        "column": column,
        "generation": snapshot.generation().get(),
        "pool_size": pool.len(),
        "scan": match opened.config.search.scan {
            crible_core::config::ScanStrategy::Binary => "binary",
            crible_core::config::ScanStrategy::None => "none",
        },
        "rescore_candidates": opened.config.search.rescore_candidates,
        "exact_threshold": opened.config.search.exact_threshold,
        "lexical": {
            "enabled": opened.config.lexical.enabled,
            "language": opened.config.lexical.language,
            "positions": opened.config.lexical.positions,
        },
        "k": 10,
        "grouping": "document",
        "passages_per_document": 1,
        "candidates_per_branch": depth,
        "methods": measured.iter().map(|method| method.name()).collect::<Vec<String>>(),
        "questions": rows,
        "case_1240": case,
    });
    write_json(&out, &document)?;
    println!(
        "{index} ({column}): {} question(s) measured on a pool of {} passages in {:.1} s",
        document["questions"].as_array().map(Vec::len).unwrap_or(0),
        pool.len(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

// --------------------------------------------------------------- analyze -----

/// One analysis variant of MES-11 (ADR-0092, decision 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Variant {
    /// The production chain.
    V0,
    /// The production chain plus an elision filter.
    V1,
    /// The production chain with identifiers kept as one token.
    V2,
    /// V1 and V2 together.
    V3,
}

impl Variant {
    /// Name of the variant in the raw document and in the report.
    const fn name(self) -> &'static str {
        match self {
            Self::V0 => "V0",
            Self::V1 => "V1",
            Self::V2 => "V2",
            Self::V3 => "V3",
        }
    }

    /// Whether the variant rewrites identifiers before analysis.
    const fn rewrites_identifiers(self) -> bool {
        matches!(self, Self::V2 | Self::V3)
    }

    /// Whether the variant drops elided forms.
    const fn drops_elisions(self) -> bool {
        matches!(self, Self::V1 | Self::V3)
    }

    /// Analysis chain of the variant, in the order of `docs/03-recherche.md`.
    fn analyzer(self) -> TextAnalyzer {
        let stop_words = StopWordFilter::new(Language::French)
            .unwrap_or_else(|| StopWordFilter::remove(Vec::new()));
        let builder = if self.rewrites_identifiers() {
            TextAnalyzer::builder(IdentifierTokenizer::default()).dynamic()
        } else {
            TextAnalyzer::builder(SimpleTokenizer::default()).dynamic()
        };
        let builder = if self.drops_elisions() {
            builder.filter_dynamic(ElisionFilter)
        } else {
            builder
        };
        builder
            .filter_dynamic(RemoveLongFilter::limit(MAX_TOKEN_LENGTH))
            .filter_dynamic(LowerCaser)
            .filter_dynamic(AsciiFoldingFilter)
            .filter_dynamic(stop_words)
            .filter_dynamic(Stemmer::new(Language::French))
            .build()
    }

    /// Text the variant analyses: V2 and V3 rewrite identifiers first.
    fn prepare(self, text: &str) -> String {
        if self.rewrites_identifiers() {
            rewrite_identifiers(text)
        } else {
            text.to_owned()
        }
    }
}

/// Elision filter of V1: drops a token that is an elided French form.
///
/// `SimpleTokenizer` splits on the apostrophe, so `qu'il` reaches the filter as
/// `qu` then `il`, and `l'article` as `l` then `article`. The French stop list of
/// tantivy already holds the single letters (`c d j l m n s t qu`), so this
/// filter only adds the compound forms; the report measures that, and does not
/// assume it.
struct ElisionFilter;

/// Forms the filter drops, after the tokenizer and before the lowercaser.
///
/// `qu` is also in the French stop list, so naming it here changes no token; it
/// keeps the filter and the question detection of [`has_elision`] on the same
/// list.
const ELISIONS: [&str; 9] = ["c", "d", "j", "l", "m", "n", "s", "t", "qu"];
/// Compound forms, which the stop list does **not** hold.
const COMPOUND_ELISIONS: [&str; 4] = ["jusqu", "lorsqu", "puisqu", "quoiqu"];

impl ElisionFilter {
    /// Whether `text` is an elided form the filter removes.
    ///
    /// The filter runs before the lowercaser, so the comparison folds the case:
    /// `Jusqu'à` and `jusqu'à` must be treated alike.
    fn is_elision(text: &str) -> bool {
        let lower = text.to_lowercase();
        ELISIONS.contains(&lower.as_str()) || COMPOUND_ELISIONS.contains(&lower.as_str())
    }
}

impl TokenFilter for ElisionFilter {
    type Tokenizer<T: Tokenizer> = ElisionWrapper<T>;

    fn transform<T: Tokenizer>(self, tokenizer: T) -> ElisionWrapper<T> {
        ElisionWrapper { inner: tokenizer }
    }
}

/// Wrapper of [`ElisionFilter`] around any tokenizer.
#[derive(Clone)]
struct ElisionWrapper<T: Tokenizer> {
    inner: T,
}

impl<T: Tokenizer> Tokenizer for ElisionWrapper<T> {
    type TokenStream<'a> = ElisionStream<T::TokenStream<'a>>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        ElisionStream {
            tail: self.inner.token_stream(text),
        }
    }
}

/// Stream of [`ElisionWrapper`], skipping the elided forms.
struct ElisionStream<T> {
    tail: T,
}

impl<T: TokenStream> TokenStream for ElisionStream<T> {
    fn advance(&mut self) -> bool {
        while self.tail.advance() {
            if !ElisionFilter::is_elision(self.tail.token().text.as_str()) {
                return true;
            }
        }
        false
    }

    fn token(&self) -> &Token {
        self.tail.token()
    }

    fn token_mut(&mut self) -> &mut Token {
        self.tail.token_mut()
    }
}

/// Tokenizer of V2: an identifier is one token, everything else is a word.
///
/// `SimpleTokenizer` splits on every character that is not alphanumeric, so
/// `L. 1132-1` becomes `l`, `1132` and `1`. This tokenizer recognises an
/// identifier first (`L. 1132-1`, `L1132-1`, `R. 123-4`) and emits its normalised
/// form without the dot and the space, the hyphen kept: `l1132-1`. `L. 1132-11`
/// gives `l1132-11`, which is a different token, as ADR-0092, decision 5,
/// requires. Any other character that is not alphanumeric is a separator,
/// exactly as `SimpleTokenizer` does.
#[derive(Clone, Default)]
struct IdentifierTokenizer {
    tokens: Vec<Token>,
    cursor: usize,
}

impl Tokenizer for IdentifierTokenizer {
    type TokenStream<'a> = IdentifierStream;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        self.tokens.clear();
        self.cursor = 0;
        let mut position = 0_usize;
        let mut index = 0_usize;
        while index < text.len() {
            let Some(character) = text[index..].chars().next() else {
                break;
            };
            if let Some((end, normalised)) = identifier_at(text, index) {
                self.tokens.push(Token {
                    offset_from: index,
                    offset_to: end,
                    position,
                    text: normalised,
                    position_length: 1,
                });
                position += 1;
                index = end;
                continue;
            }
            if character.is_alphanumeric() {
                let start = index;
                index += character.len_utf8();
                while index < text.len() {
                    let Some(next) = text[index..].chars().next() else {
                        break;
                    };
                    if !next.is_alphanumeric() {
                        break;
                    }
                    index += next.len_utf8();
                }
                self.tokens.push(Token {
                    offset_from: start,
                    offset_to: index,
                    position,
                    text: text[start..index].to_owned(),
                    position_length: 1,
                });
                position += 1;
                continue;
            }
            index += character.len_utf8();
        }
        IdentifierStream {
            tokens: self.tokens.clone(),
            cursor: 0,
        }
    }
}

/// Stream of [`IdentifierTokenizer`].
#[derive(Clone)]
struct IdentifierStream {
    tokens: Vec<Token>,
    cursor: usize,
}

impl TokenStream for IdentifierStream {
    fn advance(&mut self) -> bool {
        if self.cursor >= self.tokens.len() {
            return false;
        }
        self.cursor += 1;
        true
    }

    fn token(&self) -> &Token {
        &self.tokens[self.cursor - 1]
    }

    fn token_mut(&mut self) -> &mut Token {
        &mut self.tokens[self.cursor - 1]
    }
}

/// Identifier starting at `index`, with its normalised form, if any.
///
/// The pattern is a letter, an optional dot and spaces, digits, then optional
/// `-digits` groups: `L. 1132-1`, `L1132-1`, `R. 123-4`. The normalised form is
/// the same in every spelling, so `L. 1132-1` and `L1132-1` meet, and the digit
/// groups separate `1132-1` from `1132-11`.
fn identifier_at(text: &str, index: usize) -> Option<(usize, String)> {
    let bytes = text.as_bytes();
    let previous = text[..index].chars().next_back();
    if previous.is_some_and(|character| character.is_alphanumeric()) {
        return None;
    }
    let mut cursor = index;
    let letter = text[cursor..].chars().next()?;
    if !letter.is_ascii_alphabetic() {
        return None;
    }
    cursor += letter.len_utf8();
    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
    }
    let spaces = text[cursor..]
        .chars()
        .take_while(|character| character.is_whitespace())
        .count();
    cursor += spaces;
    let digits_start = cursor;
    while cursor < text.len() && bytes[cursor].is_ascii_digit() {
        cursor += 1;
    }
    if cursor == digits_start {
        return None;
    }
    loop {
        if bytes.get(cursor) != Some(&b'-') {
            break;
        }
        let after = cursor + 1;
        let mut group = after;
        while group < text.len() && bytes[group].is_ascii_digit() {
            group += 1;
        }
        if group == after {
            break;
        }
        cursor = group;
    }
    let normalised = format!("{}{}", letter, &text[digits_start..cursor]);
    Some((cursor, normalised.to_lowercase()))
}

/// Rewrites the identifiers of `text` so that the chain sees one word per
/// identifier, as V2 requires.
fn rewrite_identifiers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut index = 0_usize;
    while index < text.len() {
        let Some(character) = text[index..].chars().next() else {
            break;
        };
        if let Some((end, normalised)) = identifier_at(text, index) {
            out.push_str(&normalised);
            index = end;
            continue;
        }
        out.push(character);
        index += character.len_utf8();
    }
    out
}

/// One passage of the replica: internal identifier, key and text.
struct Passage {
    iid: u32,
    key: String,
    doc_id: String,
    text: String,
}

/// The `analyze` mode: the four variants of MES-11 on one index.
///
/// For every question of the index it records the lexical top-10 (principal
/// series), the `rrf` top-10 over the replica's lexical ranking and the engine's
/// dense ranking (secondary series), the tokens the four chains produce, and the
/// rank of the expected document. It also anchors V0 to the engine: the replica's
/// document top-10 must be the one `lexical::search` returns.
fn analyze_mode(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let data = PathBuf::from(required(arguments, "--data")?);
    let index = required(arguments, "--index")?;
    let queries_path = PathBuf::from(required(arguments, "--queries")?);
    let vectors_path = PathBuf::from(required(arguments, "--vectors")?);
    let out = PathBuf::from(required(arguments, "--out")?);
    let limit = optional(arguments, "--limit")
        .map(|value| value.parse::<usize>())
        .transpose()?;

    // The exact column reads a clone of the index named `<structure>-exact`;
    // the questions stay the ones of the structure.
    let structure = optional(arguments, "--structure")
        .unwrap_or_else(|| index.trim_end_matches("-exact").to_owned());
    let vectors = read_vectors(&vectors_path)?;
    let questions = read_questions(&queries_path)?;
    let mut selected: Vec<&Question> = questions
        .iter()
        .filter(|question| question.structure == structure)
        .filter(|question| question.labelled)
        .collect();
    if let Some(limit) = limit {
        selected.truncate(limit);
    }

    let opened = Opened::read(&data, &index)?;
    let snapshot = opened.snapshot();
    let pool = Pool::evaluate(snapshot.filters(), &Filter::from_json_str("{}")?)?;
    let scan = ScanPool::new(0)?;
    let segments_directory = opened.store.segments_directory();
    let records_path = opened.store.records_path();
    let segments = snapshot.segments().to_vec();
    let searcher =
        scan.install(|| DenseSearcher::open(&segments_directory, &segments, &records_path))?;

    let started = std::time::Instant::now();
    let passages = read_passages(&opened)?;
    println!(
        "{index}: {} passage(s) of replica, {:.1} s",
        passages.len(),
        started.elapsed().as_secs_f64()
    );
    let keys: HashMap<u32, String> = passages
        .iter()
        .map(|passage| (passage.iid, passage.key.clone()))
        .collect();
    let by_iid: HashMap<u32, &Passage> = passages
        .iter()
        .map(|passage| (passage.iid, passage))
        .collect();

    // Dense ranking of every question, once: the secondary `rrf` series fuses
    // it with the replica's lexical ranking.
    let mut dense_ranks: HashMap<String, Vec<(String, u32)>> = HashMap::new();
    let mut engine_lexical: HashMap<String, Vec<String>> = HashMap::new();
    for question in &selected {
        let Some(vector) = vectors.get(&question.id) else {
            return Err(format!("no archived vector for '{}'", question.id).into());
        };
        let dense = pipeline::search(
            &snapshot,
            &searcher,
            &pool,
            &Request {
                mode: Mode::Dense,
                vector: Some(vector),
                text: None,
                k: 200,
                exact: false,
                grouping: &Grouping::none(),
                fusion: FusionConfig::Rrf,
            },
            &opened.config.search,
        )?;
        let ranks: Vec<(String, u32)> = dense
            .results
            .iter()
            .enumerate()
            .map(|(position, result)| {
                (
                    result.hit.key.clone(),
                    u32::try_from(position + 1).unwrap_or(u32::MAX),
                )
            })
            .collect();
        dense_ranks.insert(question.id.clone(), ranks);
        let engine = lexical::search(&snapshot, &pool, &question.text, 200)?;
        engine_lexical.insert(
            question.id.clone(),
            documents_of_ranking(engine.iter().map(|scored| scored.key.as_str())),
        );
    }

    let mut variant_documents: HashMap<&'static str, HashMap<String, Vec<String>>> = HashMap::new();
    let mut variant_rrf: HashMap<&'static str, HashMap<String, Vec<String>>> = HashMap::new();
    let mut variant_stats: Vec<Value> = Vec::new();
    let mut variant_tokens: HashMap<&'static str, HashMap<String, Vec<String>>> = HashMap::new();
    let mut example_differences: Vec<Value> = Vec::new();

    let signatures = token_statistics(&passages);
    // The four replicas are built once and used by the questions and by the
    // probes: rebuilding an index per probe would multiply the build by sixteen.
    let mut replicas: HashMap<&'static str, Index> = HashMap::new();
    for variant in [Variant::V0, Variant::V1, Variant::V2, Variant::V3] {
        replicas.insert(variant.name(), build_replica(variant, &passages)?);
    }
    for variant in [Variant::V0, Variant::V1, Variant::V2, Variant::V3] {
        let replica = replicas.get(variant.name()).ok_or("a replica is missing")?;
        let mut documents: HashMap<String, Vec<String>> = HashMap::new();
        let mut fused: HashMap<String, Vec<String>> = HashMap::new();
        let mut tokens: HashMap<String, Vec<String>> = HashMap::new();
        for question in &selected {
            let ranking = replica_search(replica, &keys, &question.text, variant, 200)?;
            let document_top = documents_of_ranking(ranking.iter().map(|(key, _)| key.as_str()));
            documents.insert(question.id.clone(), document_top);
            let lexical_ranks: Vec<(String, u32)> = ranking
                .iter()
                .enumerate()
                .map(|(position, (key, _))| {
                    (key.clone(), u32::try_from(position + 1).unwrap_or(u32::MAX))
                })
                .collect();
            let dense = dense_ranks
                .get(&question.id)
                .ok_or("the dense ranking of a question is missing")?;
            fused.insert(
                question.id.clone(),
                rrf_documents(dense, &lexical_ranks, 10),
            );
            tokens.insert(question.id.clone(), analyse_tokens(variant, &question.text));
        }
        variant_documents.insert(variant.name(), documents);
        variant_rrf.insert(variant.name(), fused);
        variant_tokens.insert(variant.name(), tokens);
        let summary = signatures
            .iter()
            .find(|entry| entry.0 == variant.name())
            .map(|entry| entry.1.clone())
            .unwrap_or(Value::Null);
        variant_stats.push(json!({"variant": variant.name(), "stats": summary}));
        if variant != Variant::V0 && example_differences.len() < 40 {
            let mut found = 0;
            for passage in passages.iter() {
                if found >= 4 {
                    break;
                }
                let mut reference_chain = Variant::V0.analyzer();
                let mut other = variant.analyzer();
                let left = tokens_of(&mut reference_chain, &passage.text);
                let right = tokens_of(&mut other, &variant.prepare(&passage.text));
                if left != right {
                    found += 1;
                    // Show a window **around the first difference**: the first
                    // twelve tokens are often identical and would hide the
                    // reason the passage is listed.
                    let first = (0..left.len().max(right.len()))
                        .find(|position| left.get(*position) != right.get(*position))
                        .unwrap_or(0);
                    let start = first.saturating_sub(2);
                    let left_end = (start + 12).min(left.len());
                    let right_end = (start + 12).min(right.len());
                    example_differences.push(json!({
                        "variant": variant.name(),
                        "doc_id": passage.doc_id,
                        "key": passage.key,
                        "first_difference": first,
                        "v0": left.get(start..left_end).unwrap_or(&[]),
                        "variant_tokens": right.get(start..right_end).unwrap_or(&[]),
                    }));
                }
            }
        }
        println!(
            "{index}: {} replica built and measured, {:.1} s",
            variant.name(),
            started.elapsed().as_secs_f64()
        );
    }

    let mut rows: Vec<Value> = Vec::new();
    let mut anchored = 0_u32;
    for question in &selected {
        let mut lexical: Map<String, Value> = Map::new();
        let mut fused: Map<String, Value> = Map::new();
        let mut tokens: Map<String, Value> = Map::new();
        for variant in [Variant::V0, Variant::V1, Variant::V2, Variant::V3] {
            let name = variant.name();
            lexical.insert(
                name.to_owned(),
                json!(
                    variant_documents
                        .get(name)
                        .and_then(|map| map.get(&question.id))
                        .cloned()
                        .unwrap_or_default()
                ),
            );
            fused.insert(
                name.to_owned(),
                json!(
                    variant_rrf
                        .get(name)
                        .and_then(|map| map.get(&question.id))
                        .cloned()
                        .unwrap_or_default()
                ),
            );
            tokens.insert(
                name.to_owned(),
                json!(
                    variant_tokens
                        .get(name)
                        .and_then(|map| map.get(&question.id))
                        .cloned()
                        .unwrap_or_default()
                ),
            );
        }
        let reference = variant_documents
            .get("V0")
            .and_then(|map| map.get(&question.id))
            .cloned()
            .unwrap_or_default();
        if engine_lexical.get(&question.id) == Some(&reference) {
            anchored += 1;
        }
        rows.push(json!({
            "id": question.id,
            "corpus": question.corpus,
            "expected": question.expected,
            "elision": question.elision,
            "identifier": question.identifier,
            "lexical": lexical,
            "rrf": fused,
            "tokens": tokens,
        }));
    }

    let mut probes_rows: Vec<Value> = Vec::new();
    for (name, text) in probes() {
        let Some(vector) = vectors.get(name) else {
            return Err(format!("no archived vector for '{name}'").into());
        };
        let mut lexical: Map<String, Value> = Map::new();
        let mut fused: Map<String, Value> = Map::new();
        let mut tokens: Map<String, Value> = Map::new();
        for variant in [Variant::V0, Variant::V1, Variant::V2, Variant::V3] {
            let name = variant.name();
            let replica = replicas.get(name).ok_or("a replica is missing")?;
            let ranking = replica_search(replica, &keys, text, variant, 10)?;
            lexical.insert(
                name.to_owned(),
                json!(documents_of_ranking(
                    ranking.iter().map(|(key, _)| key.as_str())
                )),
            );
            let dense = pipeline::search(
                &snapshot,
                &searcher,
                &pool,
                &Request {
                    mode: Mode::Dense,
                    vector: Some(vector),
                    text: None,
                    k: 200,
                    exact: false,
                    grouping: &Grouping::none(),
                    fusion: FusionConfig::Rrf,
                },
                &opened.config.search,
            )?;
            let dense_ranks: Vec<(String, u32)> = dense
                .results
                .iter()
                .enumerate()
                .map(|(position, result)| {
                    (
                        result.hit.key.clone(),
                        u32::try_from(position + 1).unwrap_or(u32::MAX),
                    )
                })
                .collect();
            let lexical_ranks: Vec<(String, u32)> = ranking
                .iter()
                .enumerate()
                .map(|(position, (key, _))| {
                    (key.clone(), u32::try_from(position + 1).unwrap_or(u32::MAX))
                })
                .collect();
            fused.insert(
                name.to_owned(),
                json!(rrf_documents(&dense_ranks, &lexical_ranks, 10)),
            );
            tokens.insert(name.to_owned(), json!(analyse_tokens(variant, text)));
        }
        let expected = probe_expected(name);
        let mut lexical_rank: Map<String, Value> = Map::new();
        for variant in [Variant::V0, Variant::V1, Variant::V2, Variant::V3] {
            let top = lexical
                .get(variant.name())
                .and_then(Value::as_array)
                .map(|documents| {
                    documents
                        .iter()
                        .position(|document| {
                            document.as_str() == Some(expected.unwrap_or_default())
                        })
                        .map(|position| position + 1)
                })
                .unwrap_or(None);
            lexical_rank.insert(variant.name().to_owned(), json!(top));
        }
        probes_rows.push(json!({
            "name": name,
            "text": text,
            "expected": expected,
            "lexical": lexical,
            "lexical_rank": lexical_rank,
            "rrf": fused,
            "tokens": tokens,
        }));
    }

    let document = json!({
        "index": index,
        "questions": rows,
        "probes": probes_rows,
        "variants": variant_stats,
        "differences": example_differences,
        "anchor": {
            "v0_replica_equals_engine_documents": anchored,
            "questions": selected.len(),
        },
        "passages": passages.len(),
        "by_iid": by_iid.len(),
    });
    write_json(&out, &document)?;
    println!(
        "{index}: V0 replica reproduces the engine's document top-10 for {anchored} of {} question(s)",
        selected.len()
    );
    Ok(())
}

/// Documents of a passage ranking: the first passage of each document, in order.
fn documents_of_ranking<'a>(keys: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for key in keys {
        let document = key.split('#').next().unwrap_or_default().to_owned();
        if seen.contains(&document) {
            continue;
        }
        seen.push(document);
        if seen.len() == 10 {
            break;
        }
    }
    seen
}

/// `rrf` over two passage rankings, then the document top-10 of the fusion.
///
/// The fusion of `crates/crible-search/src/fusion.rs` sums `1 / (60 + rank)` over
/// the branches a passage appears in and orders the list by descending score,
/// then ascending key; the regrouping keeps the best passage of each document.
/// The sum is computed in `f64` here, where the engine rounds once to `f32`.
fn rrf_documents(dense: &[(String, u32)], lexical: &[(String, u32)], k: usize) -> Vec<String> {
    let mut scores: BTreeMap<&str, f64> = BTreeMap::new();
    for (key, rank) in dense {
        *scores.entry(key.as_str()).or_insert(0.0) += 1.0 / (RRF_K + f64::from(*rank));
    }
    for (key, rank) in lexical {
        *scores.entry(key.as_str()).or_insert(0.0) += 1.0 / (RRF_K + f64::from(*rank));
    }
    let mut ordered: Vec<(&str, f64)> = scores.into_iter().collect();
    ordered.sort_by(|left, right| right.1.total_cmp(&left.1).then_with(|| left.0.cmp(right.0)));
    let mut documents: Vec<String> = Vec::new();
    for (key, _) in ordered {
        let document = key.split('#').next().unwrap_or_default().to_owned();
        if documents.contains(&document) {
            continue;
        }
        documents.push(document);
        if documents.len() == k {
            break;
        }
    }
    documents
}

// -------------------------------------------------------------- replica ------

/// Reads every live passage of the index: identifier, key, document and text.
///
/// The text is the one `records.sqlite` stores — the encoder input, which is also
/// the text the lexical branch indexes (ADR-0020) — so the replica analyses
/// exactly what the engine analyses.
fn read_passages(opened: &Opened) -> Result<Vec<Passage>, Box<dyn std::error::Error>> {
    let connection = rusqlite::Connection::open_with_flags(
        opened.store.records_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut statement = connection.prepare(
        "SELECT iid, doc_id, ord, text FROM passages WHERE dead_since_gen IS NULL ORDER BY iid",
    )?;
    let mut rows = statement.query([])?;
    let mut passages: Vec<Passage> = Vec::new();
    while let Some(row) = rows.next()? {
        let iid: i64 = row.get(0)?;
        let doc_id: String = row.get(1)?;
        let ord: i64 = row.get(2)?;
        let frame: Vec<u8> = row.get(3)?;
        let bytes = zstd::stream::decode_all(&frame[..])?;
        passages.push(Passage {
            iid: u32::try_from(iid)?,
            key: format!("{doc_id}#{ord}"),
            doc_id,
            text: String::from_utf8(bytes)?,
        });
    }
    Ok(passages)
}

/// Builds the tantivy measurement index of one variant, in RAM.
fn build_replica(
    variant: Variant,
    passages: &[Passage],
) -> Result<Index, Box<dyn std::error::Error>> {
    let mut builder = Schema::builder();
    let iid = builder.add_u64_field(
        REPLICA_IID,
        tantivy::schema::NumericOptions::default().set_fast(),
    );
    let text = builder.add_text_field(
        REPLICA_TEXT,
        TextOptions::default().set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(REPLICA_TOKENIZER)
                .set_index_option(IndexRecordOption::WithFreqs),
        ),
    );
    let schema = builder.build();
    let index = Index::create_in_ram(schema);
    index
        .tokenizers()
        .register(REPLICA_TOKENIZER, variant.analyzer());
    let mut writer = index.writer_with_num_threads(1, 128 * 1024 * 1024)?;
    for passage in passages {
        let prepared = variant.prepare(&passage.text);
        let mut document = TantivyDocument::default();
        document.add_u64(iid, u64::from(passage.iid));
        document.add_text(text, &prepared);
        writer.add_document(document)?;
    }
    writer.commit()?;
    index.load_metas()?;
    Ok(index)
}

/// The `k` best passages of the replica for `text`, by descending BM25, ties
/// broken on the ascending key.
fn replica_search(
    index: &Index,
    keys: &HashMap<u32, String>,
    text: &str,
    variant: Variant,
    k: usize,
) -> Result<Vec<(String, Score)>, Box<dyn std::error::Error>> {
    let searcher = index.reader()?.searcher();
    let schema = searcher.schema();
    let field = schema.get_field(REPLICA_TEXT)?;
    let mut analyzer = variant.analyzer();
    let prepared = variant.prepare(text);
    let mut stream = analyzer.token_stream(&prepared);
    let mut terms: Vec<String> = Vec::new();
    while let Some(token) = stream.next() {
        if !terms.iter().any(|term| term == &token.text) {
            terms.push(token.text.clone());
        }
    }
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let clauses: Vec<(Occur, Box<dyn Query>)> = terms
        .into_iter()
        .map(|term| {
            let query: Box<dyn Query> = Box::new(TermQuery::new(
                Term::from_field_text(field, &term),
                IndexRecordOption::WithFreqs,
            ));
            (Occur::Should, query)
        })
        .collect();
    let query = BooleanQuery::new(clauses);
    let found = searcher.search(&query, &AllScored)?;
    let mut scored: Vec<(String, Score)> = found
        .into_iter()
        .filter_map(|(score, iid)| keys.get(&iid).map(|key| (key.clone(), score)))
        .collect();
    scored.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    scored.truncate(k);
    Ok(scored)
}

/// Collector that keeps every scored document, so that the boundary tie of the
/// `k`-th score can be broken on the key as the engine does.
struct AllScored;

impl Collector for AllScored {
    type Fruit = Vec<(Score, u32)>;
    type Child = AllScoredSegment;

    fn for_segment(
        &self,
        _segment_ord: u32,
        segment: &SegmentReader,
    ) -> tantivy::Result<Self::Child> {
        Ok(AllScoredSegment {
            iid: segment.fast_fields().u64(REPLICA_IID)?,
            found: Vec::new(),
        })
    }

    fn requires_scoring(&self) -> bool {
        true
    }

    fn merge_fruits(&self, fruits: Vec<Vec<(Score, u32)>>) -> tantivy::Result<Vec<(Score, u32)>> {
        Ok(fruits.into_iter().flatten().collect())
    }
}

/// One segment of [`AllScored`].
struct AllScoredSegment {
    iid: Column<u64>,
    found: Vec<(Score, u32)>,
}

impl SegmentCollector for AllScoredSegment {
    type Fruit = Vec<(Score, u32)>;

    fn collect(&mut self, doc: DocId, score: Score) {
        if let Some(value) = self.iid.first(doc)
            && let Ok(iid) = u32::try_from(value)
        {
            self.found.push((score, iid));
        }
    }

    fn harvest(self) -> Self::Fruit {
        self.found
    }
}

/// Tokens of `text` under `analyzer`.
fn tokens_of(analyzer: &mut TextAnalyzer, text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut stream = analyzer.token_stream(text);
    while let Some(token) = stream.next() {
        tokens.push(token.text.clone());
    }
    tokens
}

/// Tokens of `text` under `variant`.
fn analyse_tokens(variant: Variant, text: &str) -> Vec<String> {
    let mut analyzer = variant.analyzer();
    tokens_of(&mut analyzer, &variant.prepare(text))
}

/// Per-variant counts over the replica: passages, tokens, passages that differ
/// from V0 (by a hash of their token list), and the tokens of a few examples.
fn token_statistics(passages: &[Passage]) -> Vec<(&'static str, Value)> {
    let mut signature_v0: HashMap<u32, u64> = HashMap::new();
    let mut results: Vec<(&'static str, Value)> = Vec::new();
    for variant in [Variant::V0, Variant::V1, Variant::V2, Variant::V3] {
        let mut analyzer = variant.analyzer();
        let mut tokens = 0_u64;
        let mut differing = 0_u64;
        for passage in passages {
            let produced = tokens_of(&mut analyzer, &variant.prepare(&passage.text));
            tokens += u64::try_from(produced.len()).unwrap_or(0);
            let mut hasher = DefaultHasher::new();
            produced.hash(&mut hasher);
            let signature = hasher.finish();
            if variant == Variant::V0 {
                signature_v0.insert(passage.iid, signature);
            } else if signature_v0.get(&passage.iid) != Some(&signature) {
                differing += 1;
            }
        }
        results.push((
            variant.name(),
            json!({
                "passages": passages.len(),
                "tokens": tokens,
                "tokens_per_passage": if passages.is_empty() {
                    0.0
                } else {
                    tokens as f64 / passages.len() as f64
                },
                "passages_differing_from_v0": differing,
            }),
        ));
    }
    results
}

// ---------------------------------------------------------------- inputs -----

/// One question of `queries.jsonl`.
struct Question {
    id: String,
    text: String,
    structure: String,
    corpus: String,
    satisfiable: bool,
    labelled: bool,
    expected_in_sample: bool,
    expected: Vec<String>,
    elision: bool,
    identifier: bool,
}

/// Reads `queries.jsonl`, keeping every field the campaign uses.
fn read_questions(path: &Path) -> Result<Vec<Question>, Box<dyn std::error::Error>> {
    let raw = fs::read_to_string(path)?;
    let mut questions = Vec::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let row: Value = serde_json::from_str(line)?;
        let id = row
            .get("id")
            .and_then(Value::as_str)
            .ok_or("a query line has no id")?
            .to_owned();
        let text = row
            .get("question")
            .and_then(Value::as_str)
            .ok_or("a query line has no question")?
            .to_owned();
        let expected: Vec<String> = row
            .get("expected")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| entry.get("src_id").and_then(Value::as_str))
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        questions.push(Question {
            labelled: id.starts_with('Q'),
            elision: has_elision(&text),
            identifier: has_identifier(&text),
            id,
            text,
            structure: row
                .get("structure")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            corpus: row
                .get("corpus")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            satisfiable: row
                .get("satisfiable")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            expected_in_sample: row
                .get("expected_in_sample")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            expected,
        });
    }
    Ok(questions)
}

/// Whether `text` holds a French elision (`l'`, `d'`, `qu'` …).
///
/// The forms come from the filter's own lists, so the detection cannot drift
/// from what V1 and V3 remove.
fn has_elision(text: &str) -> bool {
    let lowered = text.to_lowercase();
    ELISIONS
        .iter()
        .chain(COMPOUND_ELISIONS.iter())
        .any(|form| lowered.contains(&format!("{form}'")) || lowered.contains(&format!("{form}’")))
}

/// Whether `text` holds a legal identifier (`L. 1132-1`, `L1132-1`, `R. 123-4`).
fn has_identifier(text: &str) -> bool {
    for (index, _) in text.char_indices() {
        let previous = text[..index].chars().next_back();
        if previous.is_some_and(|character| character.is_alphanumeric()) {
            continue;
        }
        if let Some((_, normalised)) = identifier_at(text, index)
            && normalised.len() > 2
            && normalised
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_alphabetic())
        {
            return true;
        }
    }
    false
}

/// The five variants of the « article 1240 » case (ADR-0092, decision 4).
fn case_1240_variants() -> [(&'static str, &'static str); 5] {
    [
        ("V1", "l'article 1240 du code civil"),
        ("V2", "article 1240 du code civil"),
        ("V3", "article 1240"),
        ("V4", "art. 1240 code civil"),
        ("V5", "Code civil, article 1240"),
    ]
}

/// Declared probes of MES-11, with the document they target when one exists.
fn probes() -> [(&'static str, &'static str); 4] {
    [
        ("P1", "L. 1132-1"),
        ("P2", "L1132-1"),
        ("P3", "L. 1132-11"),
        ("P4", "qu'il"),
    ]
}

/// Expected document of a probe, when the probe targets one.
fn probe_expected(name: &str) -> Option<&'static str> {
    match name {
        // `Q01` targets the article L. 1132-1 of the Code du travail, which the
        // enriched set holds; `P3` is the negative control, `P4` has no target.
        "P1" | "P2" => Some("LEGIARTI000045391841"),
        _ => None,
    }
}

/// Query vectors archived by `encode`.
fn read_vectors(path: &Path) -> Result<HashMap<String, Vec<f32>>, Box<dyn std::error::Error>> {
    let raw = fs::read_to_string(path)?;
    let document: Value = serde_json::from_str(&raw)?;
    let vectors = document
        .get("vectors")
        .and_then(Value::as_object)
        .ok_or("the vectors file has no 'vectors' object")?;
    let mut out: HashMap<String, Vec<f32>> = HashMap::new();
    for (key, value) in vectors {
        let components: Vec<f32> = value
            .as_array()
            .ok_or("a vector is not an array")?
            .iter()
            .map(|component| {
                component
                    .as_f64()
                    .map(|value| value as f32)
                    .ok_or("a component is not a number")
            })
            .collect::<Result<Vec<f32>, _>>()?;
        out.insert(key.clone(), components);
    }
    Ok(out)
}

// --------------------------------------------------------------- helpers -----

fn required(arguments: &[String], flag: &str) -> Result<String, Box<dyn std::error::Error>> {
    optional(arguments, flag).ok_or_else(|| format!("missing {flag}").into())
}

fn optional(arguments: &[String], flag: &str) -> Option<String> {
    arguments
        .iter()
        .position(|argument| argument == flag)
        .and_then(|position| arguments.get(position + 1))
        .cloned()
}

/// Writes `document` as pretty JSON, creating the parent directory.
fn write_json(path: &Path, document: &Value) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = File::create(path)?;
    file.write_all(serde_json::to_string_pretty(document)?.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}
