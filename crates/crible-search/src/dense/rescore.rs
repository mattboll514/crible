// SPDX-License-Identifier: Apache-2.0

//! Rescoring of the candidates of the binary scan: their exact cosine.
//!
//! References: `docs/phases/P4-recherche-rapide.md` (`RESCORER-FP16`, phase P4),
//! `docs/03-recherche.md` ("Branche dense", strategy BINARY: "scan de Hamming
//! sur le pool → top-N → rescoring fp16 → tri"), ADR-0072 (one calculation of
//! the dot product for the exact strategy and for the rescoring) and ADR-0070
//! (the `f16` kernel).
//!
//! # What the rescoring computes
//!
//! The binary scan ranks the pool by Hamming distance, which is cheap and
//! approximate; the rescoring takes the `N` passages it kept and computes their
//! exact cosine, then sorts them. The score comes from
//! `cosine`, the one function the dense branch uses (ADR-0072):
//! the same components, the same kernel, the same rounding as the exact
//! strategy.
//!
//! The vectors are read from the memory map of the segment that holds each
//! passage, in the precision the segment stores. The binary scan's copy of the
//! codes is not read here: the rescoring needs the reference vectors, not their
//! signs.
//!
//! # Identity with the exact strategy
//!
//! The acceptance criterion of the task sheet is: when `N` is at least the size
//! of the pool, the rescoring returns **the same keys, in the same order, with
//! bit-identical scores** as `EXACT` (ADR-0072, decision 3).
//!
//! This module meets it by construction, in three places:
//!
//! - the candidate list then covers the whole pool, because the scan returns
//!   the whole pool when `N` exceeds it;
//! - both strategies score through the same function, and normalise the query
//!   through the searcher's own validation, so a passage receives the same bits
//!   from either of them;
//! - both rank with [`Scored`], the single carrier of the order "descending
//!   score, then ascending textual key", so a tie cannot be broken one way by a
//!   heap and another way by a sort.
//!
//! # Parallelism and cost
//!
//! The loop is sequential. `N` is bounded by `search.rescore_candidates` (1 000
//! by default, `docs/03-recherche.md`), so it is a few thousand products plus
//! the random reads of the retained vectors, while the scan it follows has
//! already walked millions of codes. MES-03 measures that step; a parallel loop
//! is added only if the measurement asks for one, and it would not change the
//! answer, since the ranking is total and built after the loop.
//!
//! **Nesting rule.** As with [`ExactSearcher::top_k`], a caller must not hold a
//! connection of the searcher's read pool when it calls this method: the key
//! reads would wait for it, and the pool would run out of connections.

use crible_core::error::CribleError;

use super::binary::Candidate;
use super::exact::ExactSearcher;
use super::{Scored, cosine, internal};

impl ExactSearcher {
    /// Scores the `candidates` of the binary scan and returns them best first,
    /// by descending cosine and, at equal score, by ascending key `doc_id#ord`.
    ///
    /// `candidates` is the result of one scan of the pool
    /// ([`binary::top_k`](super::binary::top_k)): distinct identifiers, taken
    /// from the generation this searcher opened. The Hamming distance they
    /// carry is ignored: it ranked the candidates, it does not score them.
    ///
    /// The query is validated and normalised exactly as the exact strategy does
    /// it, before an empty candidate list short-circuits: a malformed query is
    /// refused whatever the scan kept, and a query with a zero norm has no
    /// cosine to compute. An empty list returns no result and no error, which is
    /// the state of a pool the filter emptied or of a generation without a
    /// segment.
    ///
    /// A stored vector that produces a non-finite cosine stops the rescoring
    /// with an [`ErrorCode::Internal`](crible_core::error::ErrorCode::Internal)
    /// error, as in the exact strategy: the vectors are written normalised and
    /// their checksums are verified when a segment opens, so a non-finite score
    /// means the stored data contradicts the format, not a bad request.
    ///
    /// A candidate whose passage row is absent is an `internal` error, here and
    /// in the exact strategy: the key is read before the payload, and a passage
    /// of the pool always has a row. A document row may be absent, which is a
    /// normal state of the database and is handled by the payload step
    /// (ADR-0043).
    pub fn rescore(
        &self,
        query: &[f32],
        candidates: &[Candidate],
    ) -> Result<Vec<Scored>, CribleError> {
        let empty = candidates.is_empty();
        let query = self.normalised_query(query, empty)?;
        if empty {
            return Ok(Vec::new());
        }

        let mut scored = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let iid = candidate.iid;
            let score = cosine(self.reader(iid)?, iid, &query)?;
            if !score.is_finite() {
                return Err(internal(
                    "a stored vector produces a non-finite cosine, so the ranking is undefined",
                ));
            }
            scored.push(Scored {
                iid,
                key: self.key_of(iid)?,
                score,
            });
        }
        scored.sort_unstable_by(|left, right| right.cmp(left));
        Ok(scored)
    }
}

#[cfg(test)]
mod tests {
    use crible_core::ids::Iid;

    use super::super::Scored;

    /// The order the rescoring sorts with is total, and a tie is broken by the
    /// textual key, not by the internal identifier.
    ///
    /// `doc#10` sorts **before** `doc#2` as text, while the `Ord` of
    /// [`crible_core::ids::PassageKey`] would order them the other way: the test
    /// pins the textual order that `tools/reference_topk.py` and the exact
    /// strategy use (ADR-0072, decision 3).
    #[test]
    fn a_tie_is_broken_by_the_ascending_textual_key() {
        let mut scored = [
            Scored {
                iid: Iid::new(2),
                key: "doc#2".to_owned(),
                score: 0.5,
            },
            Scored {
                iid: Iid::new(10),
                key: "doc#10".to_owned(),
                score: 0.5,
            },
            Scored {
                iid: Iid::new(4),
                key: "doc#0".to_owned(),
                score: 0.75,
            },
        ];
        scored.sort_unstable_by(|left, right| right.cmp(left));

        let keys: Vec<&str> = scored
            .iter()
            .map(|candidate| candidate.key.as_str())
            .collect();
        assert_eq!(
            keys,
            vec!["doc#0", "doc#10", "doc#2"],
            "descending score first, then the ascending textual key"
        );
    }
}
