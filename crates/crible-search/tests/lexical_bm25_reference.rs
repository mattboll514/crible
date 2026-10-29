// SPDX-License-Identifier: Apache-2.0

// Test fixtures may panic; `allow-panic-in-tests` covers `#[test]` functions
// only, and the fixtures of this file build an index.
#![allow(clippy::panic)]

//! Acceptance criterion of `RECHERCHER-BM25-DANS-LE-POOL`: the scores are those
//! of a BM25 reference calculated here.
//!
//! References: `docs/phases/P5-lexical-et-hybride.md`
//! (`RECHERCHER-BM25-DANS-LE-POOL`, "les résultats sont égaux à un calcul BM25 de
//! référence sur un petit corpus"), `docs/03-recherche.md` ("Branche lexicale"),
//! `docs/02-architecture.md` ("Suppressions, pierres tombales et compaction") and
//! tantivy 0.27 (`src/query/bm25.rs`, `src/fieldnorm/code.rs`).
//!
//! # The formula the reference reproduces, and where it comes from
//!
//! tantivy 0.27, `Bm25Weight::score`:
//!
//! ```text
//! score(doc, term) = idf * (1 + k1) * tf / (tf + k1 * (1 - b + b * dl / avgdl))
//! idf              = ln(1 + (N - n + 0.5) / (n + 0.5))
//! k1               = 1.2, b = 0.75
//! ```
//!
//! with, for the field `text` of the whole searcher:
//!
//! - `N`, `total_num_docs`, the sum of `SegmentReader::max_doc()` over the
//!   segments — **not** `num_docs`, so a document tantivy deleted still counts
//!   until the segments are merged;
//! - `avgdl`, `total_num_tokens / N`, where `total_num_tokens` is the token
//!   count recorded **when the segment was written**: it counts the tokens of
//!   the documents a later generation tombstoned, too;
//! - `n = doc_freq(term)`, read from the postings, which also describe the
//!   deleted documents;
//! - `dl`, the field norm of the document, stored on one byte
//!   (`FieldNormReader::id_to_fieldnorm`). The scale is the identity up to 41
//!   tokens (`src/fieldnorm/code.rs`, `FIELD_NORMS_TABLE`), and every fixture
//!   text below stays well under that bound, so `dl` is the exact token count.
//!
//! A bag of terms — `docs/03`, "sac de termes (OU)" — is scored as the sum of
//! the per-term scores: tantivy combines the `Should` clauses with `SumCombiner`
//! and applies no coordination factor.
//!
//! # Why the second test exists
//!
//! The statistics of a tombstoned passage keep moving every score of the epoch
//! until the compaction writes an index that never held it. The first test pins
//! the formula on an index without tombstone; the second pins what `N`, `n` and
//! `avgdl` are worth **when a document was deleted**, which is the part of the
//! formula a re-implementation gets wrong.
//!
//! ```text
//! cargo test -p crible-search --test lexical_bm25_reference
//! ```

mod common;

use common::lexical::{LexicalFixture, doc, match_all};
use crible_core::ids::Iid;
use crible_search::dense::Scored;
use crible_search::lexical;

/// `k1` of tantivy's BM25 (`src/query/bm25.rs`).
const K1: f64 = 1.2;

/// `b` of tantivy's BM25 (`src/query/bm25.rs`).
const B: f64 = 0.75;

/// Tolerance on a score: the reference accumulates in `f64` and tantivy in
/// `f32`, so the two orders of summation differ in the last digits.
const SCORE_TOLERANCE: f32 = 1e-5;

/// Corpus of the two tests: five documents, one passage each, five to seven
/// tokens analysed. Every text stays under the 41-token identity range of the
/// field norm, so `dl` is exact.
///
/// `civil-1240ter` holds `responsabilite` twice: a term frequency above one is
/// what tells the saturation term of the formula from a plain count.
const CORPUS: [(&str, &str); 5] = [
    ("civil-1240", "article 1240 code civil responsabilite"),
    ("civil-1241", "article 1241 code civil garde"),
    (
        "civil-1240ter",
        "responsabilite article 1240 responsabilite code civil",
    ),
    ("civil-1134", "contrat article 1134 code civil force"),
    ("civil-1582", "vente article 1582 code civil propriete"),
];

/// Query of the two tests: `articl`, `1240` and `responsabil` once analysed.
const QUERY: &str = "article 1240 responsabilite";

/// Documents of the fixture, each with a non-zero vector so that the segment of
/// the index is well formed. The dense branch is not read here, so the vectors
/// only have to exist and to be non-zero: [`common::lexical::DIMS`] components
/// per passage, and five passages for four dimensions.
fn documents() -> Vec<common::lexical::Doc> {
    CORPUS
        .iter()
        .enumerate()
        .map(|(position, (doc_id, text))| {
            let mut vector = [0.0_f32; common::lexical::DIMS as usize];
            vector[position % common::lexical::DIMS as usize] = 1.0;
            doc(doc_id, text, vector)
        })
        .collect()
}

/// Key `doc_id#ord` of each document of [`CORPUS`], in insertion order: the
/// fixture writes one passage per document, at `ord = 0`.
fn keys_of_corpus() -> Vec<String> {
    CORPUS
        .iter()
        .map(|(doc_id, _)| format!("{doc_id}#0"))
        .collect()
}

/// Tokens the analysis chain of `code` produces for `text`, in order.
fn tokens(code: &str, text: &str) -> Vec<String> {
    let mut analyzer = crible_store::lexical::analyzer::analyzer(code).unwrap_or_else(|error| {
        panic!(
            "the analysis chain of {code} must build: {}",
            error.message()
        )
    });
    let mut stream = analyzer.token_stream(text);
    let mut tokens = Vec::new();
    while let Some(token) = stream.next() {
        tokens.push(token.text.clone());
    }
    tokens
}

/// BM25 scores of `scored`, computed from the token streams `statistics`.
///
/// `statistics` is what tantivy counts: one token stream per document the
/// segment holds, **tombstoned ones included**, and `keys` holds the key
/// `doc_id#ord` of each of them, in the same order. `scored` names the documents
/// to return, by their position in `statistics`, so a deleted document can feed
/// the statistics without being a candidate.
///
/// The result is sorted as the branch returns it: descending score, then
/// ascending **textual** key `doc_id#ord` (ADR-0045, ADR-0079). The fixture hands
/// the identifiers out in insertion order, so the key of identifier `n` is
/// `keys[n]`.
fn reference(
    statistics: &[Vec<String>],
    keys: &[String],
    scored: &[Iid],
    query_terms: &[String],
) -> Vec<(Iid, f32)> {
    #[allow(clippy::cast_precision_loss)]
    let total_docs = statistics.len() as f64;
    #[allow(clippy::cast_precision_loss)]
    let total_tokens: f64 = statistics.iter().map(|tokens| tokens.len() as f64).sum();
    let average_length = total_tokens / total_docs;

    let mut scores: Vec<(Iid, f32)> = Vec::with_capacity(scored.len());
    for iid in scored {
        let Some(document) = statistics.get(iid.get() as usize) else {
            panic!("iid {} has no token stream", iid.get());
        };
        #[allow(clippy::cast_precision_loss)]
        let length = document.len() as f64;
        let mut score = 0.0_f64;
        for term in query_terms {
            #[allow(clippy::cast_precision_loss)]
            let frequency = document.iter().filter(|token| *token == term).count() as f64;
            if frequency == 0.0 {
                continue;
            }
            #[allow(clippy::cast_precision_loss)]
            let document_frequency = statistics
                .iter()
                .filter(|document| document.iter().any(|token| token == term))
                .count() as f64;
            let idf =
                (1.0 + (total_docs - document_frequency + 0.5) / (document_frequency + 0.5)).ln();
            let norm = K1 * (1.0 - B + B * length / average_length);
            score += idf * (1.0 + K1) * frequency / (frequency + norm);
        }
        #[allow(clippy::cast_possible_truncation)]
        scores.push((*iid, score as f32));
    }
    scores.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| keys[left.0.get() as usize].cmp(&keys[right.0.get() as usize]))
    });
    scores
}

/// Asserts that the branch returned `expected`, in that order, within the
/// tolerance.
fn assert_equal_to_reference(context: &str, expected: &[(Iid, f32)], actual: &[Scored]) {
    let expected_iids: Vec<u32> = expected.iter().map(|(iid, _)| iid.get()).collect();
    let actual_iids: Vec<u32> = actual.iter().map(|hit| hit.iid.get()).collect();
    assert_eq!(
        actual_iids, expected_iids,
        "{context}: the branch must return the passages of the reference, in the same order \
         (descending score, then ascending key)"
    );
    for ((iid, expected), hit) in expected.iter().zip(actual) {
        let gap = (hit.score - expected).abs();
        assert!(
            gap <= SCORE_TOLERANCE,
            "{context}: iid {} scored {} against {} in the reference, a gap of {gap:e} above \
             {SCORE_TOLERANCE:e}",
            iid.get(),
            hit.score,
            expected
        );
    }
}

/// Acceptance criterion, without tombstone: the branch returns exactly what the
/// formula gives, for the whole pool and for a truncated `k`.
#[test]
fn bm25_scores_equal_a_reference_calculation_on_a_small_corpus() {
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&documents());
    let snapshot = fixture.snapshot();
    let pool = crible_search::pool::Pool::evaluate(snapshot.filters(), &match_all())
        .unwrap_or_else(|error| panic!("the empty conjunction must evaluate: {error}"));

    let statistics: Vec<Vec<String>> = CORPUS.iter().map(|(_, text)| tokens("fr", text)).collect();
    let keys = keys_of_corpus();
    let mut query_terms = tokens("fr", QUERY);
    query_terms.sort_unstable();
    query_terms.dedup();
    assert!(
        query_terms.len() > 1,
        "the query must analyse to a bag of terms, got {query_terms:?}"
    );
    let scored: Vec<Iid> = (0..CORPUS.len())
        .map(|position| Iid::new(u32::try_from(position).unwrap_or(u32::MAX)))
        .collect();
    let expected = reference(&statistics, &keys, &scored, &query_terms);
    assert_eq!(
        expected.len(),
        CORPUS.len(),
        "every fixture document holds a query term, otherwise the reference would compare a \
         short list with a full one"
    );

    let hits = lexical::search(&snapshot, &pool, QUERY, CORPUS.len())
        .unwrap_or_else(|error| panic!("the lexical search must run: {}", error.message()));
    assert_equal_to_reference("the whole pool", &expected, &hits);

    let top = lexical::search(&snapshot, &pool, QUERY, 3)
        .unwrap_or_else(|error| panic!("the lexical search must run: {}", error.message()));
    assert_equal_to_reference("the three best", &expected[..3], &top);
}

/// Acceptance criterion, with a tombstone: the statistics tantivy uses still
/// count the document the generation removed, until the segments are merged
/// (`docs/02`), and the branch must score like them.
///
/// The assertion is decisive: a reference built from the live documents alone
/// gives other scores, and the test checks that the branch does **not** follow
/// it.
#[test]
fn bm25_statistics_count_a_deleted_document_until_the_segments_are_merged() {
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&documents());
    fixture.remove_document("civil-1582");

    let snapshot = fixture.snapshot();
    let pool = crible_search::pool::Pool::evaluate(snapshot.filters(), &match_all())
        .unwrap_or_else(|error| panic!("the empty conjunction must evaluate: {error}"));
    assert_eq!(
        common::sorted(pool.bitmap()),
        vec![0, 1, 2, 3],
        "the tombstoned passage must leave the pool"
    );

    let all_documents: Vec<Vec<String>> =
        CORPUS.iter().map(|(_, text)| tokens("fr", text)).collect();
    let keys = keys_of_corpus();
    let live_documents: Vec<Vec<String>> = all_documents[..4].to_vec();
    let mut query_terms = tokens("fr", QUERY);
    query_terms.sort_unstable();
    query_terms.dedup();
    let scored: Vec<Iid> = (0..4).map(Iid::new).collect();

    let hits = lexical::search(&snapshot, &pool, QUERY, 10)
        .unwrap_or_else(|error| panic!("the lexical search must run: {}", error.message()));

    // The reference tantivy computes: the deleted document still counts in N,
    // in the document frequencies and in the average length.
    let counting_the_deleted = reference(&all_documents, &keys, &scored, &query_terms);
    assert_equal_to_reference(
        "statistics that count the deleted document (tantivy until a merge)",
        &counting_the_deleted,
        &hits,
    );

    // The other hypothesis, which the branch must not follow: a reference built
    // from the live passages alone.
    let live_only = reference(&live_documents, &keys, &scored, &query_terms);
    let differs = live_only.iter().zip(&hits).any(|((iid, expected), hit)| {
        assert_eq!(
            iid.get(),
            hit.iid.get(),
            "the comparison must oppose the same passage on both sides"
        );
        (hit.score - expected).abs() > 1e-3
    });
    assert!(
        differs,
        "the fixture must move the scores when the deleted document leaves the statistics, \
         otherwise the test cannot tell the two hypotheses apart: {live_only:?} against {:?}",
        hits.iter()
            .map(|hit| (hit.iid.get(), hit.score))
            .collect::<Vec<_>>()
    );
}

/// A query that analyses to no term returns no result and no error.
///
/// It is the boundary of the bag of terms: tantivy asserts that a BM25 weight
/// has at least one term, so the branch must answer an empty bag before it
/// builds one.
#[test]
fn bm25_a_query_without_terms_returns_no_result() {
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&documents());
    let snapshot = fixture.snapshot();
    let pool = crible_search::pool::Pool::evaluate(snapshot.filters(), &match_all())
        .unwrap_or_else(|error| panic!("the empty conjunction must evaluate: {error}"));

    assert!(
        tokens("fr", "du de la").is_empty(),
        "the fixture query must analyse to no token: it is made of French stop words only"
    );
    let hits = lexical::search(&snapshot, &pool, "du de la", 10).unwrap_or_else(|error| {
        panic!("an empty bag of terms is not an error: {}", error.message())
    });
    assert!(
        hits.is_empty(),
        "an empty bag of terms matches nothing, got {:?}",
        hits.iter().map(|hit| hit.iid.get()).collect::<Vec<_>>()
    );
}

/// An empty `k` returns no result, as the dense branch does, and does not touch
/// the index.
#[test]
fn bm25_a_zero_k_returns_no_result() {
    let mut fixture = LexicalFixture::create(true);
    fixture.add_generation(&documents());
    let snapshot = fixture.snapshot();
    let pool = crible_search::pool::Pool::evaluate(snapshot.filters(), &match_all())
        .unwrap_or_else(|error| panic!("the empty conjunction must evaluate: {error}"));

    let hits = lexical::search(&snapshot, &pool, QUERY, 0)
        .unwrap_or_else(|error| panic!("k = 0 is not an error: {}", error.message()));
    assert!(hits.is_empty(), "k = 0 returns no result, got {hits:?}");
}
