// SPDX-License-Identifier: Apache-2.0

//! Regrouping of the candidates of a search: `group_by: "document"`,
//! `passages_per_document` and `dedup: "content"`.
//!
//! References: `docs/03-recherche.md` ("Regroupement", and step 7 of the
//! pipeline table), `docs/05-api.md` (`group_by`, `passages_per_document`,
//! `dedup` of the search request), `docs/phases/P4-recherche-rapide.md`
//! (`REGROUPER-RESULTATS`, phase P4) and ADR-0075 (order, over-sampling layer,
//! shape of the answer, candidates under `BINARY`).
//!
//! # Where it sits
//!
//! The regrouping is step 7 of the pipeline of `docs/03-recherche.md`: it runs
//! after the merge of the branches and before the payload step 8. Its input is
//! therefore a list of scored candidates, best first, and its output the list
//! of candidates the payload step reads. The dense branch is the only branch of
//! P4, so [`group`] is fed by the candidates of that branch alone.
//!
//! # What it does
//!
//! - **`dedup: "content"`** fuses the candidates that share a `passage_hash`
//!   and keeps the best of them (`docs/03-recherche.md:108`). The digest is
//!   read from the store, never recomputed from the text, so the text of a
//!   candidate that the regrouping drops stays unread.
//! - **`group_by: "document"`** keeps the best passage of every document, and
//!   `passages_per_document` (1 by default, 5 at most) adds the other passages
//!   of the same document that matched too (`docs/03-recherche.md:107`).
//!   `k` counts **documents** here: the request returns at most `k` distinct
//!   documents, so at most `k × passages_per_document` entries.
//!
//! # The shape of the answer
//!
//! The answer stays the flat list of passage-level entries of
//! `docs/05-api.md:177-182`, and ADR-0075 decides its order: the entries of one
//! document are **consecutive**, the documents are ranked by their best passage
//! (descending score, then ascending key), and the passages of one document are
//! ranked the same way. A grouped answer is therefore not the global ranking
//! with holes: the entries the grouping keeps are gathered by document, then
//! written one group after the other.
//!
//! # The order of the two steps
//!
//! Deduplication runs **before** the grouping when both options are set, on the
//! whole over-sampled list (ADR-0075, decision 1). The task sheet requires the
//! answer of a grouped search to hold `k` distinct documents
//! (`docs/phases/P4-recherche-rapide.md:79`). Deduplication after the grouping
//! could not honour it: the surviving copy of a duplicated passage may belong
//! to another document than the representative the grouping chose, and the
//! document whose representative it removes would then disappear from the
//! answer. Deduplicating first removes the copies once, and the grouping then
//! takes its `k` documents from the survivors, so it **fills up again** from
//! the rest of the list. When the list runs out before `k` documents, the
//! answer holds the documents it found, without error: a small pool or a
//! selective filter is not a malformed request.
//!
//! # Over-sampling
//!
//! Both steps drop candidates, so the branch must be asked for more of them
//! than the request needs (`docs/03-recherche.md:110-113`): when the request
//! regroups, the branch receives `clamp(k × 20, 200, 1000)` candidates; then
//! `max(k, 100)`. [`candidates_per_branch`] is that rule, and the caller passes
//! its result as the `k` of the dense branch, which is what
//! [`crate::dense::search_grouped`] plans with (ADR-0074, decision 4, and
//! ADR-0075, decision 4: under `BINARY` the scan keeps `max(N, k)`, so the
//! effective rescoring follows the over-sampling without a second rule).
//!
//! The rule lives here, in the shared search crate, and not in the CLI: the
//! server of P6 and the evaluation call the same function, so the candidate
//! count and the `k` of the dense branch cannot diverge (ADR-0075, decision 2).
//! [`crate::dense::search`] keeps an explicit `k` and is not affected.
//!
//! `passages_per_document` is bounded by [`Grouping::MAX_PASSAGES_PER_DOCUMENT`]
//! and by 1 at least, the domain `docs/05-api.md:165` publishes.
//! `invalid_request` names the bound a request leaves.

use std::collections::{HashMap, HashSet};

use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::Iid;

use crate::dense::Scored;

/// Regrouping options of one search request.
///
/// [`Grouping::none`] is the request that regroups nothing: `group_by` and
/// `dedup` null, which `docs/05-api.md` makes the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grouping {
    /// Keep the best passage of every document (`group_by: "document"`).
    pub by_document: bool,
    /// Passages returned per grouped document, from 1 to 5.
    ///
    /// It only acts when [`Grouping::by_document`] is set: `docs/05-api.md`
    /// describes it as "passages renvoyés par document regroupé", so a request
    /// that groups nothing keeps `k` passages whatever this value. Its own
    /// domain is checked either way.
    pub passages_per_document: usize,
    /// Fuse the passages that share a `passage_hash` (`dedup: "content"`).
    pub dedup_content: bool,
}

impl Grouping {
    /// Largest number of passages one grouped document may take
    /// (`docs/05-api.md:165`).
    pub const MAX_PASSAGES_PER_DOCUMENT: usize = 5;

    /// The regrouping of a request that asks for none.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            by_document: false,
            passages_per_document: 1,
            dedup_content: false,
        }
    }

    /// Whether this request drops any candidate, and so needs the branch to be
    /// over-sampled.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.by_document || self.dedup_content
    }

    /// Checks the domain of `passages_per_document`.
    ///
    /// `docs/05-api.md:165` publishes 1 to 5, and `invalid_request` covers a
    /// parameter out of bounds. A higher value is not clamped: the request is
    /// refused, so a caller never believes its option took effect.
    pub fn validate(&self) -> Result<(), CribleError> {
        if self.passages_per_document == 0
            || self.passages_per_document > Self::MAX_PASSAGES_PER_DOCUMENT
        {
            return Err(CribleError::new(
                ErrorCode::InvalidRequest,
                format!(
                    "passages_per_document must be between 1 and {}, got {}",
                    Self::MAX_PASSAGES_PER_DOCUMENT,
                    self.passages_per_document
                ),
            ));
        }
        Ok(())
    }
}

/// Number of candidates the branch must return for a request asking `k`
/// results.
///
/// `docs/03-recherche.md:110-113`: `clamp(k × 20, 200, 1000)` when the request
/// regroups, `max(k, 100)` otherwise. ADR-0075, decision 2 keeps the rule in
/// this shared search crate, where the dense plan is built, so the CLI, the
/// server of P6 and the evaluation cannot each implement it.
///
/// `k` is capped at 1 000 by `docs/05-api.md:162`, so the product stays far
/// below any overflow; the saturation keeps the rule total for a direct caller.
#[must_use]
pub fn candidates_per_branch(k: usize, grouping: &Grouping) -> usize {
    if grouping.is_active() {
        k.saturating_mul(20).clamp(200, 1000)
    } else {
        k.max(100)
    }
}

/// Regroups the candidates of one search.
///
/// `candidates` is ordered best first, as
/// [`crate::dense::Scored`] orders them (descending score, then ascending key).
/// Without `group_by`, the answer is that order truncated to `k`. With
/// `group_by`, the answer follows ADR-0075, decision 3: at most `k` documents,
/// the entries of one document consecutive, the documents ranked by their best
/// passage and the passages of one document by descending score then ascending
/// key.
///
/// `k` is the number of results of a request that does not group, and the
/// number of **documents** of a request that groups.
///
/// `hash_of` reads the `passage_hash` of one passage. It is only called when
/// `dedup_content` is set, and only for the candidates of the list, so the
/// payload step still reads the text of the results alone.
///
/// An empty list, or `k == 0`, returns no result and no error, and so does a
/// list that holds fewer documents than `k` (ADR-0075, decision 1).
pub fn group<F>(
    candidates: Vec<Scored>,
    k: usize,
    grouping: &Grouping,
    mut hash_of: F,
) -> Result<Vec<Scored>, CribleError>
where
    F: FnMut(Iid) -> Result<String, CribleError>,
{
    grouping.validate()?;
    if k == 0 || candidates.is_empty() {
        return Ok(Vec::new());
    }

    let candidates = if grouping.dedup_content {
        dedup_by_content(candidates, &mut hash_of)?
    } else {
        candidates
    };

    if grouping.by_document {
        keep_documents(candidates, k, grouping.passages_per_document)
    } else {
        Ok(candidates.into_iter().take(k).collect())
    }
}

/// Keeps the first candidate of every `passage_hash`.
///
/// The list is ordered best first, so the first occurrence of a digest is its
/// best passage: "on garde le meilleur" (`docs/03-recherche.md:108`) is that
/// first occurrence, in the ranking already in place, and ADR-0075, decision 1
/// introduces no second sort criterion.
fn dedup_by_content<F>(candidates: Vec<Scored>, hash_of: &mut F) -> Result<Vec<Scored>, CribleError>
where
    F: FnMut(Iid) -> Result<String, CribleError>,
{
    let mut seen: HashSet<String> = HashSet::with_capacity(candidates.len());
    let mut kept = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if seen.insert(hash_of(candidate.iid)?) {
            kept.push(candidate);
        }
    }
    Ok(kept)
}

/// Keeps the passages of at most `documents` documents, `per_document` at most
/// for each of them, and writes each document's entries one after the other.
///
/// ADR-0075, decision 3: the answer is a flat list of passage-level entries
/// where the entries of one document are **consecutive**, the documents ranked
/// by their best passage, and the passages of one document ranked by descending
/// score then ascending key.
///
/// The loop walks the candidates once, best first. The first passage of a
/// document opens its group, which is why the groups come out in the order of
/// the best passage of each document; the later passages of a selected document
/// join its group while `per_document` allows it, and the global walk goes on
/// until `documents` groups exist, so the regroupment fills up again from the
/// rest of the list after the deduplication. Since the walk is in descending
/// score then ascending key, each group is in that order too. Flattening the
/// groups at the end is what makes the entries of one document consecutive: a
/// candidate of another document never sits between two of them.
///
/// A document beyond `documents` is dropped, and so is a passage beyond
/// `per_document`; the answer therefore holds at most `documents` distinct
/// documents. A key without the `#` that [`crible_core::ids::PassageKey`] writes
/// can only come from a store that contradicts its own format, so it raises an
/// internal error.
fn keep_documents(
    candidates: Vec<Scored>,
    documents: usize,
    per_document: usize,
) -> Result<Vec<Scored>, CribleError> {
    let mut groups: Vec<Vec<Scored>> = Vec::new();
    let mut selected: HashMap<String, usize> = HashMap::with_capacity(documents);
    for candidate in candidates {
        let doc_id = doc_id_of(&candidate.key)?;
        // Read before the mutable borrow of the match: a document beyond the
        // allowance is dropped without touching the map.
        let room_for_another_document = groups.len() < documents;
        match selected.get(doc_id) {
            Some(&group) => {
                let passages = &mut groups[group];
                if passages.len() < per_document {
                    passages.push(candidate);
                }
            }
            None if room_for_another_document => {
                selected.insert(doc_id.to_owned(), groups.len());
                groups.push(vec![candidate]);
            }
            None => {}
        }
    }
    Ok(groups.into_iter().flatten().collect())
}

/// External identifier of the document a candidate belongs to.
///
/// The key of a [`Scored`] candidate is written by
/// `PassageKey::Display`, `doc_id#ord`; this reads it back the way
/// `PassageKey::from_str` does, at the last `#`.
fn doc_id_of(key: &str) -> Result<&str, CribleError> {
    let (doc_id, _) = key.rsplit_once('#').ok_or_else(|| {
        CribleError::new(
            ErrorCode::Internal,
            "a ranked candidate carries a key without the '#ord' suffix, so it cannot be grouped",
        )
    })?;
    Ok(doc_id)
}
