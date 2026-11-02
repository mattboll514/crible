// SPDX-License-Identifier: Apache-2.0

//! Explanation of documents (`explain_ids`), for the diagnostic route of P7.
//!
//! References: `docs/05-api.md` ("Recherche : requête", `explain_ids`),
//! `docs/06-observabilite.md` (explain), the fiche `EXPLIQUER-DOCUMENTS` of
//! `docs/phases/P7-observabilite.md`, and ADR-0123 (closed shape, bounds, cost,
//! ranks, `grouped_under`, no `iid`).
//!
//! # What an explanation answers
//!
//! For one requested id — a `doc_id` or a passage key `doc_id#ord` — the
//! explanation says whether the document exists and is alive, whether the id is
//! in the filtered pool and, when it is not, which leaves of the filter exclude
//! it (each leaf evaluated on its own, ADR-0123, decision 7). It then describes
//! the branches that ran: the exact dense rank over the whole pool, the binary
//! rank and its membership among the rescorable candidates, the BM25 rank, score
//! and per-term contributions, the fused rank before the regrouping, and the key
//! of the result that absorbed the passage.
//!
//! # Cost
//!
//! The explanation is computed **once per request**, after `fetch`: the caller
//! reads `took_ms` first, then calls [`explain`]. The exact scan of the pool and
//! the lexical rank of the targets are shared by every id of the call, so the
//! cost does not grow with the number of ids (ADR-0123, decision 4). The lexical
//! rank counts the passages that precede a target instead of ranking the whole
//! pool (complement of 2026-11-01, "À surveiller"). A request without
//! `explain_ids` never reaches this module and allocates nothing for it.
//!
//! # Confidentiality
//!
//! An internal identifier never leaves the engine (INV-01): every object carries
//! a key, never an `iid`. The rank of the binary selection departs from an order
//! that breaks ties on the identifier, but only the rank is rendered
//! (ADR-0123, decision 9).

use std::collections::HashMap;

use crible_core::config::SearchConfig;
use crible_core::error::{CribleError, ErrorCode};
use crible_core::filter::{Condition, Filter};
use crible_core::ids::{DocId, Iid, PassageKey};
use crible_store::codes::quantize;
use crible_store::filters_index::FilterIndex;
use crible_store::generation::Snapshot;

use crate::dense::exact::DenseSearcher;
use crate::dense::{self, Strategy};
use crate::lexical::{self, TermScore};
use crate::pipeline::Mode;
use crate::pool::{Pool, condition_holds};

/// Maximum number of ids one request may ask for, duplicates merged
/// (ADR-0123, decision 3). A fixed bound, not a configuration key.
pub const MAX_IDS: usize = 100;

/// One id of `explain_ids`, resolved from its textual form.
///
/// An id is a passage key when it ends with `#` followed by a decimal integer
/// valid as an `ord`, the analysis cutting at the **last** `#`, as
/// [`PassageKey`] does. Every other id is a document identifier, even when it
/// contains `#`: `A#x` is a `doc_id`, never a refusal (ADR-0123, complement of
/// 2026-11-01). Only an empty id, or a key whose document part is empty or too
/// long, is refused (decision 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExplainId {
    /// A document identifier, naming every live passage of the document.
    Doc(DocId),
    /// A passage key `doc_id#ord`, naming one passage.
    Passage(PassageKey),
}

impl ExplainId {
    /// Parses one element of `explain_ids`, naming its JSON path on a fault.
    pub fn parse_at(raw: &str, path: &str) -> Result<Self, CribleError> {
        if key_form(raw) {
            raw.parse::<PassageKey>()
                .map(Self::Passage)
                .map_err(|error| invalid_at(path, error.message()))
        } else {
            DocId::new(raw)
                .map(Self::Doc)
                .map_err(|error| invalid_at(path, error.message()))
        }
    }

    /// External identifier of the document the id names.
    #[must_use]
    pub fn doc_id(&self) -> &DocId {
        match self {
            Self::Doc(doc_id) => doc_id,
            Self::Passage(key) => key.doc_id(),
        }
    }
}

/// Explanation of one requested id, in the shape of ADR-0123, decision 1.
///
/// A member without value is left `None` and omitted by the renderer, never
/// written as `null` or `0` (ADR-0119, decision 3).
#[derive(Debug, Clone, PartialEq)]
pub struct ExplainEntry {
    /// Identifier as requested, `doc_id` or key.
    pub id: String,
    /// Document the id belongs to.
    pub doc_id: String,
    /// Whether the reference state holds the `documents` row (ADR-0041).
    pub exists: bool,
    /// Whether the document has at least one living passage; absent when
    /// `exists` is false.
    pub alive: Option<bool>,
    /// Whether the id is in the filtered pool.
    pub in_pool: bool,
    /// Leaves of the filter that exclude the id, by their full JSON path, in the
    /// order of the filter; empty when the id is in the pool or when nothing
    /// excludes it.
    pub excluded_by: Vec<String>,
    /// Dense branch, absent when the mode does not run it or the id is not in the
    /// pool.
    pub dense: Option<DenseExplanation>,
    /// Lexical branch, absent when the mode does not run it or the id is not in
    /// the pool.
    pub lexical: Option<LexicalExplanation>,
    /// Rank in the fused list before the regrouping, absent when the id is not in
    /// that list.
    pub fused: Option<FusedExplanation>,
    /// Key of the result that absorbed the passage, absent when it is not
    /// absorbed.
    pub grouped_under: Option<String>,
}

/// Dense branch of one explanation (ADR-0123, decision 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenseExplanation {
    /// Key of the passage the branch describes.
    pub key: String,
    /// Rank over the whole filtered pool, independent of `k`.
    pub exact_rank: u32,
    /// Rank in the order of the binary selection; absent under `exact`.
    pub binary_rank: Option<u32>,
    /// Whether the passage is among the rescorable candidates; absent under
    /// `exact`.
    pub in_binary_candidates: Option<bool>,
}

/// Lexical branch of one explanation (ADR-0123, decision 6).
#[derive(Debug, Clone, PartialEq)]
pub struct LexicalExplanation {
    /// Key of the passage the branch describes.
    pub key: String,
    /// Rank among the pool passages that have a BM25 score, independent of `k`.
    pub rank: u32,
    /// BM25 score of the passage.
    pub score: f32,
    /// Contribution of each analysed term of the query present in the passage.
    pub terms: Vec<TermScore>,
}

/// Fused branch of one explanation: the rank before the regrouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FusedExplanation {
    /// Rank in the fused list, from 1.
    pub rank: u32,
}

/// One result that survived the regrouping, as `grouped_under` needs it.
///
/// `hash` is the content digest of the passage, read only when the request
/// deduplicates on content; `None` otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Survivor {
    /// Key of the result.
    pub key: String,
    /// Document of the result.
    pub doc_id: String,
    /// Content digest, when `dedup: "content"` may have merged passages.
    pub hash: Option<String>,
}

/// Everything [`explain`] needs besides the snapshot and the ids.
pub struct ExplainInput<'a> {
    /// Pool of the request, shared with the two branches.
    pub pool: &'a Pool,
    /// Searcher of the generation, for the payload and the identity columns.
    pub searcher: &'a DenseSearcher,
    /// Validated filter of the request.
    pub filter: &'a Filter,
    /// Branches the request ran.
    pub mode: Mode,
    /// Query vector, required by a mode that runs the dense branch.
    pub vector: Option<&'a [f32]>,
    /// Query text, required by a mode that runs the lexical branch.
    pub text: Option<&'a str>,
    /// Search configuration of the index, for the strategy and its candidates.
    pub search: &'a SearchConfig,
    /// Candidates each branch was asked for,
    /// `candidates_per_branch(k, grouping)`; it gives the effective number of
    /// binary candidates with `N` (ADR-0074, ADR-0075).
    pub requested: usize,
    /// Whether the request forces the exact dense strategy.
    pub exact: bool,
    /// Strategy the dense branch actually ran.
    pub strategy: Option<Strategy>,
    /// Ranking before the regrouping, of the pipeline (ADR-0123, decision 5).
    pub pre_group_ranks: &'a [(String, u32)],
    /// Results that survived the regrouping, for `grouped_under`.
    pub survivors: &'a [Survivor],
}

/// Builds the survivor list `grouped_under` reads, from the results of the
/// pipeline.
///
/// The list keeps the order of `results`, so the first result of a document or of
/// a digest is the one the answer names. The digest is read only when `dedup:
/// "content"` may have merged passages; [`fill_hashes`] does that work, so a
/// caller that must keep the hash reads outside `took_ms` can build the list
/// without them and fill them later (ADR-0123, complement of 2026-11-01).
pub fn survivors(
    searcher: &DenseSearcher,
    results: &[crate::pipeline::RankedHit],
    dedup: bool,
) -> Result<Vec<Survivor>, CribleError> {
    let mut survivors: Vec<Survivor> = results
        .iter()
        .map(|result| Survivor {
            key: result.hit.key.clone(),
            doc_id: result.hit.doc_id.clone(),
            hash: None,
        })
        .collect();
    if dedup {
        fill_hashes(searcher, &mut survivors)?;
    }
    Ok(survivors)
}

/// Reads the content digest of every survivor, for `dedup: "content"`.
///
/// A survivor whose passage the reference state no longer holds keeps no digest.
pub fn fill_hashes(
    searcher: &DenseSearcher,
    survivors: &mut [Survivor],
) -> Result<(), CribleError> {
    for survivor in survivors {
        let key = survivor.key.parse::<PassageKey>()?;
        survivor.hash = match searcher
            .passages_of(key.doc_id().as_str())?
            .into_iter()
            .find(|passage| passage.ord == key.ord())
        {
            Some(passage) => Some(searcher.passage_hash_of(passage.iid)?),
            None => None,
        };
    }
    Ok(())
}

/// Explanation of every id of `ids`, in the order of the slice.
pub fn explain(
    snapshot: &Snapshot,
    input: &ExplainInput<'_>,
    ids: &[ExplainId],
) -> Result<Vec<ExplainEntry>, CribleError> {
    let facts = document_facts(snapshot, input, ids)?;
    let dense = dense_ranks(snapshot, input, &facts)?;
    let lexical = lexical_ranks(snapshot, input, &facts)?;
    let mut entries = Vec::with_capacity(ids.len());
    for (position, id) in ids.iter().enumerate() {
        let entry = entry_of(snapshot, input, id, &facts[position], &dense, &lexical)?;
        entries.push(entry);
    }
    Ok(entries)
}

/// Whether `raw` ends with `#` followed by a decimal integer valid as an `ord`.
///
/// The split is the one of [`PassageKey::from_str`], at the **last** `#`; only
/// the shape of the suffix decides, so `#3` is a key with an empty document part
/// (refused by the parser) and `A#x` is a document identifier (ADR-0123,
/// complement of 2026-11-01).
fn key_form(raw: &str) -> bool {
    raw.rsplit_once('#')
        .is_some_and(|(_doc_id, ord)| ord.parse::<u32>().is_ok())
}

/// One resolved id: its document, its passage when it names one, and the facts
/// read from the state.
struct Facts {
    doc_id: String,
    exists: bool,
    /// Passage of a key id; `None` for a `doc_id`, or when the state holds no
    /// such passage.
    iid: Option<Iid>,
    /// Living passages of the document.
    live: Vec<Iid>,
    /// Living passages of the document the filter keeps.
    in_pool: Vec<Iid>,
}

impl Facts {
    /// Passages the members of an entry describe.
    ///
    /// For a key, the passage itself and never its document nor a sibling; for a
    /// `doc_id`, the whole pooled document (ADR-0123, complement of 2026-11-01).
    fn described(&self, id: &ExplainId) -> Vec<Iid> {
        match id {
            ExplainId::Doc(_) => self.in_pool.clone(),
            ExplainId::Passage(_) => self
                .iid
                .filter(|iid| self.in_pool.contains(iid))
                .into_iter()
                .collect(),
        }
    }
}

/// Reads the facts of every id, once per document.
fn document_facts(
    snapshot: &Snapshot,
    input: &ExplainInput<'_>,
    ids: &[ExplainId],
) -> Result<Vec<Facts>, CribleError> {
    let mut cache: HashMap<String, (bool, Vec<Iid>)> = HashMap::new();
    let mut facts = Vec::with_capacity(ids.len());
    for id in ids {
        let doc_id = id.doc_id().to_string();
        let (exists, live) = match cache.get(&doc_id) {
            Some(cached) => cached.clone(),
            None => {
                let record = input.searcher.document_record(&doc_id)?;
                let exists = record.is_some();
                let live = if exists {
                    input
                        .searcher
                        .passages_of(&doc_id)?
                        .into_iter()
                        .map(|passage| passage.iid)
                        .filter(|iid| snapshot.filters().live().contains(iid.get()))
                        .collect()
                } else {
                    Vec::new()
                };
                cache.insert(doc_id.clone(), (exists, live.clone()));
                (exists, live)
            }
        };
        let in_pool = live
            .iter()
            .copied()
            .filter(|iid| input.pool.bitmap().contains(iid.get()))
            .collect();
        let iid = match id {
            ExplainId::Doc(_) => None,
            ExplainId::Passage(key) => input
                .searcher
                .passages_of(&doc_id)?
                .into_iter()
                .find(|passage| passage.ord == key.ord())
                .map(|passage| passage.iid),
        };
        facts.push(Facts {
            doc_id,
            exists,
            iid,
            live,
            in_pool,
        });
    }
    Ok(facts)
}

/// Dense ranks of the passages the explanation describes, keyed by identifier.
struct DenseRanks {
    /// Exact rank over the pool, absent when the mode has no dense branch.
    exact: HashMap<Iid, u32>,
    /// Rank in the binary selection, empty under `exact` or without a dense
    /// branch.
    binary: HashMap<Iid, u32>,
    /// Effective number of rescorable candidates, for `in_binary_candidates`.
    candidates: u32,
}

fn dense_ranks(
    snapshot: &Snapshot,
    input: &ExplainInput<'_>,
    facts: &[Facts],
) -> Result<DenseRanks, CribleError> {
    let empty = DenseRanks {
        exact: HashMap::new(),
        binary: HashMap::new(),
        candidates: 0,
    };
    if !input.mode.uses_dense() || input.pool.is_empty() {
        return Ok(empty);
    }
    let Some(vector) = input.vector else {
        return Err(invalid(
            "mode dense or hybrid runs the dense branch, so explain needs the query vector",
        ));
    };
    let query = input.searcher.normalised_query(vector, false)?;
    let mut targets: Vec<(Iid, String)> = Vec::new();
    let mut ids: Vec<Iid> = Vec::new();
    for facts in facts {
        for iid in &facts.in_pool {
            if ids.contains(iid) {
                continue;
            }
            ids.push(*iid);
            targets.push((*iid, input.searcher.key_of(*iid)?));
        }
    }
    let exact = input.searcher.exact_ranks(&query, input.pool, &targets)?;
    let planned = dense::plan(input.search, input.pool.len(), input.requested, input.exact);
    let binary = if input.strategy == Some(Strategy::Binary) {
        let code = quantize(&query);
        dense::binary::ranks(snapshot, input.pool, &code, &ids)?
    } else {
        HashMap::new()
    };
    Ok(DenseRanks {
        exact,
        binary,
        candidates: planned.rescore_candidates,
    })
}

/// Lexical rank and score of every pool passage that the query scores.
struct LexicalRanks {
    /// `(rank, score)` keyed by identifier, empty without a lexical branch.
    scores: HashMap<Iid, (u32, f32)>,
}

fn lexical_ranks(
    snapshot: &Snapshot,
    input: &ExplainInput<'_>,
    facts: &[Facts],
) -> Result<LexicalRanks, CribleError> {
    if !input.mode.uses_lexical() || input.pool.is_empty() {
        return Ok(LexicalRanks {
            scores: HashMap::new(),
        });
    }
    let Some(text) = input.text else {
        return Err(invalid(
            "mode lexical or hybrid runs the lexical branch, so explain needs the query text",
        ));
    };
    // Every pooled passage of a requested document may be the one its branch
    // describes; the rank of each is computed in one scan of the pool.
    let mut targets: Vec<Iid> = Vec::new();
    for facts in facts {
        for iid in &facts.in_pool {
            if !targets.contains(iid) {
                targets.push(*iid);
            }
        }
    }
    let scores = lexical::ranks(snapshot, input.pool, text, &targets)?;
    Ok(LexicalRanks { scores })
}

/// Builds one entry from the facts and the branch ranks.
fn entry_of(
    snapshot: &Snapshot,
    input: &ExplainInput<'_>,
    id: &ExplainId,
    facts: &Facts,
    dense: &DenseRanks,
    lexical: &LexicalRanks,
) -> Result<ExplainEntry, CribleError> {
    // The members of an entry describe the passage itself for a key, and the
    // document for a `doc_id` (ADR-0123, complement of 2026-11-01).
    let described = facts.described(id);
    let in_pool = !described.is_empty();
    let excluded_by = if in_pool {
        Vec::new()
    } else {
        excluded_passages(id, facts, snapshot, input.filter)?
    };
    let dense_object = dense_object(input, &described, dense)?;
    let lexical_object = lexical_object(snapshot, input, &described, lexical)?;
    let fused = fused_rank(input, &described)?;
    let grouped_under = grouped_under(input, id, facts)?;
    Ok(ExplainEntry {
        id: raw_of(id),
        doc_id: facts.doc_id.clone(),
        exists: facts.exists,
        alive: facts.exists.then_some(!facts.live.is_empty()),
        in_pool,
        excluded_by,
        dense: dense_object,
        lexical: lexical_object,
        fused,
        grouped_under,
    })
}

/// Passages whose leaves an id out of the pool reports.
///
/// A `doc_id` reports the leaves that fail for at least one of its living
/// passages; a key reports those that fail for its own passage, and only when
/// that passage is living (ADR-0123, complement of 2026-11-01).
fn excluded_passages(
    id: &ExplainId,
    facts: &Facts,
    snapshot: &Snapshot,
    filter: &Filter,
) -> Result<Vec<String>, CribleError> {
    match id {
        ExplainId::Doc(_) => excluded_by(snapshot.filters(), filter, &facts.live),
        ExplainId::Passage(_) => match facts.iid {
            Some(iid) if facts.live.contains(&iid) => {
                excluded_by(snapshot.filters(), filter, &[iid])
            }
            _ => Ok(Vec::new()),
        },
    }
}

/// Textual form of an id, as it is rendered back in the entry.
fn raw_of(id: &ExplainId) -> String {
    match id {
        ExplainId::Doc(doc_id) => doc_id.as_str().to_owned(),
        ExplainId::Passage(key) => key.to_string(),
    }
}

/// Passage the dense branch describes, or `None` when no described passage
/// carries a dense rank.
fn dense_passage(input: &ExplainInput<'_>, described: &[Iid], dense: &DenseRanks) -> Option<Iid> {
    if !input.mode.uses_dense() {
        return None;
    }
    described
        .iter()
        .copied()
        .filter(|iid| dense.exact.contains_key(iid))
        .min_by_key(|iid| dense.exact.get(iid).copied())
}

fn dense_object(
    input: &ExplainInput<'_>,
    described: &[Iid],
    dense: &DenseRanks,
) -> Result<Option<DenseExplanation>, CribleError> {
    let Some(iid) = dense_passage(input, described, dense) else {
        return Ok(None);
    };
    let Some(exact_rank) = dense.exact.get(&iid).copied() else {
        return Ok(None);
    };
    let binary_rank = dense.binary.get(&iid).copied();
    let in_binary_candidates = binary_rank.map(|rank| rank <= dense.candidates);
    Ok(Some(DenseExplanation {
        key: input.searcher.key_of(iid)?,
        exact_rank,
        binary_rank,
        in_binary_candidates,
    }))
}

fn lexical_object(
    snapshot: &Snapshot,
    input: &ExplainInput<'_>,
    described: &[Iid],
    lexical: &LexicalRanks,
) -> Result<Option<LexicalExplanation>, CribleError> {
    if !input.mode.uses_lexical() {
        return Ok(None);
    }
    let Some(iid) = described
        .iter()
        .copied()
        .filter(|iid| lexical.scores.contains_key(iid))
        .min_by_key(|iid| lexical.scores.get(iid).map(|(rank, _)| *rank))
    else {
        return Ok(None);
    };
    let Some((rank, score)) = lexical.scores.get(&iid).copied() else {
        return Ok(None);
    };
    let key = input.searcher.key_of(iid)?;
    let passage_key = key.parse::<PassageKey>()?;
    let text = input.searcher.passage_text(iid)?;
    let terms = match input.text {
        Some(query) => {
            lexical::term_scores(snapshot, &passage_key, query, &text)?.unwrap_or_default()
        }
        None => Vec::new(),
    };
    Ok(Some(LexicalExplanation {
        key,
        rank,
        score,
        terms,
    }))
}

/// Best fused rank of the described passages before the regrouping.
fn fused_rank(
    input: &ExplainInput<'_>,
    described: &[Iid],
) -> Result<Option<FusedExplanation>, CribleError> {
    let keys: Vec<String> = described
        .iter()
        .map(|iid| input.searcher.key_of(*iid))
        .collect::<Result<Vec<_>, CribleError>>()?;
    let rank = input
        .pre_group_ranks
        .iter()
        .filter(|(key, _)| keys.iter().any(|own| own == key))
        .map(|(_, rank)| *rank)
        .min();
    Ok(rank.map(|rank| FusedExplanation { rank }))
}

/// Key of the result that absorbed the passage, when it was absorbed.
///
/// Under `dedup: "content"`, the rendered passage of the **same digest** comes
/// first; the rendered passage of the same document is the fallback of
/// `group_by: "document"` (ADR-0123, complement of 2026-11-01).
fn grouped_under(
    input: &ExplainInput<'_>,
    id: &ExplainId,
    facts: &Facts,
) -> Result<Option<String>, CribleError> {
    let ExplainId::Passage(_) = id else {
        return Ok(None);
    };
    let Some(iid) = facts.iid.filter(|iid| facts.in_pool.contains(iid)) else {
        return Ok(None);
    };
    let key = input.searcher.key_of(iid)?;
    if input.survivors.iter().any(|survivor| survivor.key == key) {
        return Ok(None);
    }
    if input
        .survivors
        .iter()
        .any(|survivor| survivor.hash.is_some())
    {
        let hash = input.searcher.passage_hash_of(iid)?;
        if let Some(survivor) = input
            .survivors
            .iter()
            .find(|survivor| survivor.hash.as_deref() == Some(hash.as_str()))
        {
            return Ok(Some(survivor.key.clone()));
        }
    }
    if let Some(survivor) = input
        .survivors
        .iter()
        .find(|survivor| survivor.doc_id == facts.doc_id)
    {
        return Ok(Some(survivor.key.clone()));
    }
    Ok(None)
}

/// Leaves of `filter` that fail for at least one passage of `ids`, by JSON path.
///
/// The leaves are read in the order of the filter, never their combinatory
/// nodes, and each one is evaluated on its own (ADR-0123, decision 7). For a
/// `doc_id`, that is the union of the leaves that fail for at least one living
/// passage, so the list is never empty when a living passage is out of the pool
/// (complement of 2026-11-01). A document without a living passage reports
/// nothing: nothing excluded it.
fn excluded_by(
    filters: &FilterIndex,
    filter: &Filter,
    ids: &[Iid],
) -> Result<Vec<String>, CribleError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut failing = Vec::new();
    collect_failing(filters, filter, ids, &mut failing)?;
    Ok(failing)
}

fn collect_failing(
    filters: &FilterIndex,
    filter: &Filter,
    ids: &[Iid],
    failing: &mut Vec<String>,
) -> Result<(), CribleError> {
    match filter {
        Filter::Condition(condition) => {
            if fails_for_some(filters, condition, ids)? {
                failing.push(condition.path().to_owned());
            }
            Ok(())
        }
        Filter::And(children) | Filter::Or(children) => {
            for child in children {
                collect_failing(filters, child, ids, failing)?;
            }
            Ok(())
        }
        Filter::Not(child) => collect_failing(filters, child, ids, failing),
    }
}

/// Whether `condition` fails for at least one of `ids`.
fn fails_for_some(
    filters: &FilterIndex,
    condition: &Condition,
    ids: &[Iid],
) -> Result<bool, CribleError> {
    for iid in ids {
        if !condition_holds(filters, condition, *iid)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn invalid(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::InvalidRequest, message)
}

fn invalid_at(path: &str, message: &str) -> CribleError {
    CribleError::new(ErrorCode::InvalidRequest, format!("{message} at {path}"))
}
