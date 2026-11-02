// SPDX-License-Identifier: Apache-2.0

//! The capture of the branch rankings a debug request asks for (ADR-0119,
//! decision 8).
//!
//! References: `docs/adr/0119-objet-debug-de-la-reponse-de-recherche-forme-rangs-temps-et-capture.md`
//! (decision 8), `docs/06-observabilite.md:74-84` (the journal keeps the top 20
//! of each branch) and `docs/02-architecture.md:53` (INV-01, no internal id
//! leaves the engine).
//!
//! # What it holds, and what it never holds
//!
//! One [`DebugCapture`] keeps, per branch, at most `top_n` entries
//! `(key, rank, score)`, where the key is the external `doc_id#ord` of
//! [`crate::dense::Scored`]. It never keeps an [`Iid`](crible_core::ids::Iid):
//! the key is enough to join a captured entry and a result, and an internal id
//! would leave the engine (INV-01).
//!
//! # Why it is built only when `debug` is asked
//!
//! The capture costs one bounded vector per branch. The caller builds it from
//! [`DebugCapture::new`] only for a request that asks for `debug`, and passes
//! `None` otherwise: a search without `debug` allocates nothing here
//! (ADR-0119, decision 8).
//!
//! The capture is **not** the source of the ranks of `per_result`: those come
//! from the branch candidate lists, and a result may carry a rank above
//! `top_n`. The capture is what the journal of JOURNAL-DE-DEBUG will consume.
//!
//! ```text
//! cargo test -p crible-search debug::tests
//! ```

use crate::dense::Scored;

/// Default number of entries kept per branch, the top 20 of the debug journal
/// (`docs/06-observabilite.md:82`).
pub const DEFAULT_TOP_N: usize = 20;

/// One captured entry of a branch: the external key, the rank and the score.
///
/// The type carries no internal id, by construction (INV-01).
#[derive(Debug, Clone, PartialEq)]
pub struct CapturedEntry {
    /// External key `doc_id#ord`.
    pub key: String,
    /// Rank in the branch, from 1.
    pub rank: u32,
    /// Score the branch gave the passage.
    pub score: f32,
}

/// Capture of the top `top_n` candidates of each branch of one search.
///
/// Empty until `capture_dense` or `capture_lexical` fills it; the pipeline
/// fills it after a branch ran and before the regrouping consumes the list.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DebugCapture {
    top_n: usize,
    dense: Vec<CapturedEntry>,
    lexical: Vec<CapturedEntry>,
}

impl DebugCapture {
    /// A capture that keeps at most `top_n` entries per branch.
    #[must_use]
    pub fn new(top_n: usize) -> Self {
        Self {
            top_n,
            dense: Vec::new(),
            lexical: Vec::new(),
        }
    }

    /// Number of entries a branch may keep.
    #[must_use]
    pub fn top_n(&self) -> usize {
        self.top_n
    }

    /// Entries captured from the dense branch, in its order.
    #[must_use]
    pub fn dense(&self) -> &[CapturedEntry] {
        &self.dense
    }

    /// Entries captured from the lexical branch, in its order.
    #[must_use]
    pub fn lexical(&self) -> &[CapturedEntry] {
        &self.lexical
    }

    /// Records the top `top_n` candidates of the dense branch.
    pub(crate) fn capture_dense(&mut self, branch: &[Scored]) {
        self.dense = top_entries(branch, self.top_n);
    }

    /// Records the top `top_n` candidates of the lexical branch.
    pub(crate) fn capture_lexical(&mut self, branch: &[Scored]) {
        self.lexical = top_entries(branch, self.top_n);
    }
}

/// Keeps the first `top_n` entries of a branch, with their rank from 1.
fn top_entries(branch: &[Scored], top_n: usize) -> Vec<CapturedEntry> {
    branch
        .iter()
        .take(top_n)
        .enumerate()
        .map(|(position, candidate)| CapturedEntry {
            key: candidate.key.clone(),
            rank: u32::try_from(position + 1).unwrap_or(u32::MAX),
            score: candidate.score,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{CapturedEntry, DEFAULT_TOP_N, DebugCapture};
    use crate::dense::Scored;
    use crible_core::ids::Iid;

    /// A branch of `count` candidates, best first, with keys `doc-N#0`.
    fn branch(count: u32) -> Vec<Scored> {
        (0..count)
            .map(|position| Scored {
                iid: Iid::new(position),
                key: format!("doc-{position:03}#0"),
                score: 1.0 - f32::from(u16::try_from(position).unwrap_or(0)) / 100.0,
            })
            .collect()
    }

    #[test]
    fn the_capture_keeps_at_most_top_n_entries_per_branch() {
        // ADR-0119, decision 8: the cost is bounded by `top_n`, not by the
        // length of the branch.
        let mut capture = DebugCapture::new(3);
        capture.capture_dense(&branch(10));
        capture.capture_lexical(&branch(2));
        assert_eq!(capture.dense().len(), 3);
        assert_eq!(capture.lexical().len(), 2);
        assert_eq!(capture.top_n(), 3);
        assert_eq!(capture.dense()[0].key, "doc-000#0");
        assert_eq!(capture.dense()[0].rank, 1);
        assert_eq!(capture.dense()[2].rank, 3);
    }

    #[test]
    fn the_capture_carries_keys_ranks_and_scores_only() {
        // INV-01: the captured entry names the external key, never the internal
        // identifier the branch sorted.
        let mut capture = DebugCapture::new(DEFAULT_TOP_N);
        capture.capture_dense(&branch(4));
        let captured: &[CapturedEntry] = capture.dense();
        assert_eq!(captured.len(), 4);
        for (position, entry) in captured.iter().enumerate() {
            assert_eq!(entry.rank, u32::try_from(position + 1).unwrap_or(u32::MAX));
            assert!(entry.key.ends_with("#0"), "{}", entry.key);
        }
        // The only members of a captured entry are the three of the decision.
        assert_eq!(
            format!("{:?}", captured[0]),
            "CapturedEntry { key: \"doc-000#0\", rank: 1, score: 1.0 }"
        );
    }

    #[test]
    fn a_capture_left_untouched_holds_nothing() {
        // A search without `debug` builds no capture; one built and never
        // filled holds no entry, so a journal cannot read a stale branch.
        let capture = DebugCapture::new(DEFAULT_TOP_N);
        assert!(capture.dense().is_empty());
        assert!(capture.lexical().is_empty());
        assert_eq!(capture.top_n(), DEFAULT_TOP_N);
    }
}
