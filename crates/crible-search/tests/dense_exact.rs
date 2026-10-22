// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `RECHERCHER-DENSE-EXACT`: the properties the numpy reference does not
//! guarantee by itself, on constructed pools.
//!
//! References: `docs/phases/P1-stockage-et-recherche-exacte.md` (task
//! `RECHERCHER-DENSE-EXACT`), `docs/03-recherche.md` ("Branche dense"),
//! `docs/05-api.md` ("Recherche : réponse") and ADR-0030 (a field name
//! designates one level).
//!
//! # Assumed API
//!
//! Same surface as `inv_06_dense_exact_synth.rs`, repeated here so the file
//! reads on its own:
//!
//! ```text
//! crible_search::dense::exact::ExactSearcher::open(
//!     segments_dir: &Path, segments: &[SegmentMeta], records_path: &Path)
//!     -> Result<Self, CribleError>;
//! ExactSearcher::top_k(&self, query: &[f32], pool: &Pool, k: usize)
//!     -> Result<Vec<Hit>, CribleError>;
//!
//! pub struct Hit {
//!     pub doc_id: String,
//!     pub ord: u32,
//!     pub key: String,
//!     pub score: f32,
//!     pub fields: serde_json::Value,
//!     pub text: String,
//! }
//! ```
//!
//! The search parallelises by contiguous iid slices and merges one heap per
//! thread (`docs/03-recherche.md`), so the tests run it inside rayon thread
//! pools of different sizes through `ThreadPool::install` and require the same
//! answer.
//!
//! # What each test pins
//!
//! | Test | Criterion |
//! |---|---|
//! | `inv_06_ties_are_broken_by_ascending_key` | "départage stable par clé", on a constructed exact tie |
//! | `the_kth_place_boundary_tie_can_only_be_replaced_within_the_tolerance` | "à la frontière du k-ième résultat, une clé peut être remplacée par une autre dont le score est égal à 1e-5 près" — the case is exercised, not only tolerated |
//! | `an_empty_pool_returns_no_result` | empty pool |
//! | `a_k_larger_than_the_pool_returns_the_whole_pool` | `k` above the pool size |
//! | `a_filter_narrows_the_result_to_its_pool` | the search scores the pool, not the live set |
//! | `the_payload_of_each_result_carries_fields_and_text` | "la lecture des payloads (champs, texte) pour les k résultats", across two segments |
//! | `the_score_is_the_cosine_of_a_non_unit_query` | `docs/03`: "score = produit scalaire = cosinus" |
//! | `the_result_does_not_depend_on_the_rayon_thread_count` | "parallélisé avec rayon par tranches, puis fusion des tas" |
//! | `a_query_of_the_wrong_dimensionality_is_rejected` | the request is validated before the search |
//! | `a_query_of_zero_norm_is_rejected` | no cosine is defined for the zero vector |
//! | `the_query_is_validated_even_over_an_empty_pool` | validation does not depend on the size of the pool |
//! | `a_pool_without_segment_is_an_internal_error` | an inconsistent epoch is an `internal` error |
//! | `a_non_finite_score_is_an_internal_error` | a non-finite cosine never silently wins the top-k |
//! | `a_non_finite_query_is_rejected` | a malformed query is an `invalid_request`, not an `internal` error |
//! | `an_empty_generation_accepts_any_query_and_returns_no_result` | the empty generation of `crible index create` |
//! | `opening_refuses_a_missing_or_foreign_records_database` | `open` fails on an unusable `records.sqlite` |
//! | `missing_document_row_renders_passage_fields` | ADR-0043: text and passage fields, no document field, no error |

mod common;

use crible_core::error::ErrorCode;
use crible_core::filter::Filter;
use crible_search::dense::exact::{ExactSearcher, Hit};
use crible_search::pool::Pool;
use serde_json::json;

use common::dense::{
    DenseFixture, ExpectedResult, SCORE_TOLERANCE, Sample, compare_to_reference, match_all,
    merged_fields,
};

/// Builds a filter from its JSON form, panicking with the JSON on a failure.
fn filter(raw: &str) -> Filter {
    Filter::from_json_str(raw)
        .unwrap_or_else(|error| panic!("the filter {raw} must parse: {error}"))
}

/// Keys of a result list, in order.
fn keys(hits: &[Hit]) -> Vec<&str> {
    hits.iter().map(|hit| hit.key.as_str()).collect()
}

/// Three passages of equal score, whose iid order is not the key order: the
/// fixture hands out iids in the sample order, `doc-b`, `doc-a`, `doc-c`.
fn tie_fixture() -> DenseFixture {
    let mut fixture = DenseFixture::create(3);
    let vector = vec![1.0, 0.0, 0.0];
    fixture.add_generation(&[
        Sample::new("doc-b", 0, vector.clone()),
        Sample::new("doc-a", 0, vector.clone()),
        Sample::new("doc-c", 0, vector.clone()),
    ]);
    fixture
}

#[test]
fn inv_06_ties_are_broken_by_ascending_key() {
    let fixture = tie_fixture();
    let hits = fixture.search(&[1.0, 0.0, 0.0], &match_all(), 3);
    assert_eq!(
        keys(&hits),
        ["doc-a#0", "doc-b#0", "doc-c#0"],
        "equal scores must be ordered by ascending key doc_id#ord, not by iid"
    );
    for hit in &hits {
        assert_eq!(
            hit.score, 1.0,
            "the cosine of a unit vector with itself is exactly 1 in f32"
        );
    }
}

/// The k-th place boundary is where the reference may hold a different key than
/// the engine, because two equal scores end at the cut.
///
/// The comparison of the task sheet is used as the oracle: it accepts the other
/// key of the tie, and it rejects a key whose score is not within the tolerance.
#[test]
fn the_kth_place_boundary_tie_can_only_be_replaced_within_the_tolerance() {
    let fixture = tie_fixture();
    let hits = fixture.search(&[1.0, 0.0, 0.0], &match_all(), 2);
    assert_eq!(
        keys(&hits),
        ["doc-a#0", "doc-b#0"],
        "the two best keys of an exact tie are the two smallest keys"
    );

    let other_key_of_the_tie = [
        ExpectedResult::new("doc-a#0", 1.0),
        ExpectedResult::new("doc-c#0", 1.0),
    ];
    let summary = compare_to_reference(1, &other_key_of_the_tie, &hits).unwrap_or_else(|message| {
        panic!(
            "the k-th place is a tie, so the other key of the tie is a legitimate reference: \
             {message}"
        )
    });
    assert_eq!(
        summary.boundary_replacements, 1,
        "the comparison must report the replaced k-th place, not only tolerate it"
    );

    let lower_score = [
        ExpectedResult::new("doc-a#0", 1.0),
        ExpectedResult::new("doc-d#0", 0.0),
    ];
    assert!(
        compare_to_reference(1, &lower_score, &hits).is_err(),
        "the k-th place may only be replaced by a key whose score is within {SCORE_TOLERANCE}, \
         so replacing doc-b#0 (score 1) with doc-d#0 (score 0) must fail"
    );
}

#[test]
fn an_empty_pool_returns_no_result() {
    let fixture = tie_fixture();
    let empty = filter(r#"{"fonds":"absent"}"#);
    assert!(
        fixture.pool(&empty).is_empty(),
        "the fixture filter must select no passage"
    );
    let hits = fixture.search(&[1.0, 0.0, 0.0], &empty, 5);
    assert!(
        hits.is_empty(),
        "an empty pool must return no result, not an error and not a stale passage"
    );
}

#[test]
fn a_k_larger_than_the_pool_returns_the_whole_pool() {
    // The stored vectors are normalised, as the segment reader requires: the
    // scores are the cosines 1, 1/sqrt(2) and 0 with the query, so the expected
    // order is strict and does not rest on a tie.
    let mut fixture = DenseFixture::create(3);
    let half = std::f32::consts::FRAC_1_SQRT_2;
    fixture.add_generation(&[
        Sample::new("doc-a", 0, vec![1.0, 0.0, 0.0]),
        Sample::new("doc-b", 0, vec![half, half, 0.0]),
        Sample::new("doc-c", 0, vec![0.0, 1.0, 0.0]),
    ]);
    let hits = fixture.search(&[1.0, 0.0, 0.0], &match_all(), 10);
    assert_eq!(
        keys(&hits),
        ["doc-a#0", "doc-b#0", "doc-c#0"],
        "k above the pool size must return the whole pool, in score order"
    );
}

#[test]
fn a_filter_narrows_the_result_to_its_pool() {
    let mut fixture = DenseFixture::create(3);
    fixture.add_generation(&[
        Sample::new("doc-a", 0, vec![1.0, 0.0, 0.0]).with_doc_fields(json!({ "fonds": "JORF" })),
        Sample::new("doc-b", 0, vec![0.0, 1.0, 0.0]).with_doc_fields(json!({ "fonds": "LEGI" })),
    ]);
    let hits = fixture.search(&[1.0, 0.0, 0.0], &filter(r#"{"fonds":"LEGI"}"#), 10);
    assert_eq!(
        keys(&hits),
        ["doc-b#0"],
        "the search must score the pool only: doc-a#0 scores 1 but its document is out of the filter"
    );
    assert_eq!(hits[0].score, 0.0, "the cosine of orthogonal vectors is 0");
}

#[test]
fn the_payload_of_each_result_carries_fields_and_text() {
    // Two generations, so two segments: the iid ranges of the results span both
    // segments, and the payload must follow the iid to its own row.
    let mut fixture = DenseFixture::create(3);
    fixture.add_generation(&[
        Sample::new("doc-1", 0, vec![1.0, 0.0, 0.0])
            .with_doc_fields(json!({ "fonds": "LEGI" }))
            .with_passage_fields(json!({ "rang": 0 }))
            .with_text("Code civil > Article 1240"),
        Sample::new("doc-1", 1, vec![0.5, 0.0, 0.0])
            .with_doc_fields(json!({ "fonds": "LEGI" }))
            .with_passage_fields(json!({ "rang": 1 }))
            .with_text("Code civil > Article 1241"),
    ]);
    fixture.add_generation(&[Sample::new("doc-2", 0, vec![0.0, 1.0, 0.0])
        .with_doc_fields(json!({ "fonds": "JORF" }))
        .with_passage_fields(json!({ "rang": 0 }))
        .with_text("Journal officiel")]);

    let hits = fixture.search(&[1.0, 0.0, 0.0], &match_all(), 3);
    assert_eq!(keys(&hits), ["doc-1#0", "doc-1#1", "doc-2#0"]);

    let expected: [(&str, u32, &str, &str, u32); 3] = [
        ("doc-1", 0, "Code civil > Article 1240", "LEGI", 0),
        ("doc-1", 1, "Code civil > Article 1241", "LEGI", 1),
        ("doc-2", 0, "Journal officiel", "JORF", 0),
    ];
    for (hit, (doc_id, ord, text, fonds, rang)) in hits.iter().zip(expected) {
        assert_eq!(hit.doc_id, doc_id);
        assert_eq!(hit.ord, ord);
        assert_eq!(hit.key, format!("{doc_id}#{ord}"));
        assert_eq!(
            hit.text, text,
            "the text of {doc_id}#{ord} must be the stored one"
        );
        assert_eq!(
            hit.fields,
            merged_fields(&json!({ "fonds": fonds }), &json!({ "rang": rang })),
            "the fields of {doc_id}#{ord} must merge the document and passage fields"
        );
    }
    assert_eq!(
        hits[0].fields.get("fonds"),
        hits[1].fields.get("fonds"),
        "the two passages of doc-1 must inherit the same document fields"
    );
    assert_eq!(
        hits[0].fields.get("fonds").and_then(|value| value.as_str()),
        Some("LEGI"),
        "the inherited document field must be the one of the document row"
    );
}

#[test]
fn the_score_is_the_cosine_of_a_non_unit_query() {
    let mut fixture = DenseFixture::create(3);
    fixture.add_generation(&[
        Sample::new("doc-a", 0, vec![1.0, 0.0, 0.0]),
        Sample::new("doc-b", 0, vec![0.0, 1.0, 0.0]),
    ]);
    // A query of norm 3: a plain dot product would score 3 and 0, the cosine
    // scores 1 and 0 (`docs/03-recherche.md`: the vectors are normalised, so the
    // score is the cosine; the reference normalises the query in f32).
    let hits = fixture.search(&[3.0, 0.0, 0.0], &match_all(), 2);
    assert_eq!(keys(&hits), ["doc-a#0", "doc-b#0"]);
    assert!(
        (hits[0].score - 1.0).abs() <= 1e-6,
        "a parallel passage must score 1 whatever the norm of the query, got {}",
        hits[0].score
    );
    assert!(
        hits[1].score.abs() <= 1e-6,
        "an orthogonal passage must score 0, got {}",
        hits[1].score
    );
}

#[test]
fn the_result_does_not_depend_on_the_rayon_thread_count() {
    // 40 passages in one segment, with distinct scores: query e1 against
    // vectors [1, i/100, 0, 0] scores i/100, so the expected order is the
    // descending one of i and does not depend on how the iids are sliced.
    let mut fixture = DenseFixture::create(4);
    let samples: Vec<Sample> = (0..40)
        .map(|i| {
            Sample::new(
                &format!("doc-{i:02}"),
                0,
                vec![1.0, i as f32 / 100.0, 0.0, 0.0],
            )
        })
        .collect();
    fixture.add_generation(&samples);

    let query = [0.0, 1.0, 0.0, 0.0];
    let pool = fixture.pool(&match_all());
    let searcher = fixture.searcher();

    let one = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap_or_else(|error| panic!("a one-thread rayon pool must build: {error}"));
    let several = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap_or_else(|error| panic!("a four-thread rayon pool must build: {error}"));

    let sequential = one
        .install(|| searcher.top_k(&query, &pool, 10))
        .unwrap_or_else(|error| panic!("the one-thread search must succeed: {error}"));
    let parallel = several
        .install(|| searcher.top_k(&query, &pool, 10))
        .unwrap_or_else(|error| panic!("the four-thread search must succeed: {error}"));
    // A searcher opened inside the one-thread pool sizes its read pool on a
    // single connection, while the search below runs on four workers: the
    // capacity smaller than the worker count is exercised in that direction too.
    // The fixture itself is not `Send`, so its parts are taken outside the pool.
    let segments_dir = fixture.segments_dir();
    let records_path = fixture.records_path();
    let snapshot = fixture.snapshot();
    let narrow = one
        .install(|| ExactSearcher::open(&segments_dir, snapshot.segments(), &records_path))
        .unwrap_or_else(|error| panic!("a searcher must open inside a one-thread pool: {error}"));
    let narrow_wide = several
        .install(|| narrow.top_k(&query, &pool, 10))
        .unwrap_or_else(|error| panic!("the narrow searcher must still answer: {error}"));

    assert_eq!(
        keys(&sequential),
        keys(&parallel),
        "the slice partitioning must not change the keys, nor their order"
    );
    assert_eq!(
        keys(&sequential),
        keys(&narrow_wide),
        "a read pool smaller than the worker count must not change the answer"
    );
    for ((left, right), narrow) in sequential.iter().zip(&parallel).zip(&narrow_wide) {
        assert_eq!(
            left.score.to_bits(),
            right.score.to_bits(),
            "the score of {} must not depend on the partitioning, got {} and {}",
            left.key,
            left.score,
            right.score
        );
        assert_eq!(
            left.score.to_bits(),
            narrow.score.to_bits(),
            "the score of {} must not depend on the capacity of the read pool",
            left.key
        );
    }

    let worst = [
        "doc-39#0", "doc-38#0", "doc-37#0", "doc-36#0", "doc-35#0", "doc-34#0", "doc-33#0",
        "doc-32#0", "doc-31#0", "doc-30#0",
    ];
    assert_eq!(
        keys(&sequential),
        worst,
        "the ten best scores must be the ten largest i/100"
    );
}

/// The request is validated before the search runs, so a malformed vector is
/// refused even when the filter keeps a passage: a cosine needs as many
/// components as the index has dimensions.
#[test]
fn a_query_of_the_wrong_dimensionality_is_rejected() {
    let fixture = tie_fixture();
    let error = fixture
        .searcher()
        .top_k(&[1.0, 0.0], &fixture.pool(&match_all()), 3)
        .err()
        .unwrap_or_else(|| {
            panic!("a query of two components over a three-dimensional index must fail")
        });
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
}

/// A zero vector has no cosine: every passage would score zero, which is not an
/// answer. The query is refused, as `tools/reference_topk.py` refuses it.
#[test]
fn a_query_of_zero_norm_is_rejected() {
    let fixture = tie_fixture();
    let error = fixture
        .searcher()
        .top_k(&[0.0, 0.0, 0.0], &fixture.pool(&match_all()), 3)
        .err()
        .unwrap_or_else(|| panic!("a query of zero norm must fail"));
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
}

/// The empty pool is a short-circuit of the search, not of the validation: a
/// filter that keeps nothing must not turn a malformed query into an empty
/// answer. Both parts of the validation are exercised, the norm and the
/// dimensionality.
#[test]
fn the_query_is_validated_even_over_an_empty_pool() {
    let fixture = tie_fixture();
    let empty = filter(r#"{"fonds":"absent"}"#);
    let pool = fixture.pool(&empty);
    assert!(pool.is_empty(), "the fixture filter must select no passage");
    let searcher = fixture.searcher();
    let error = searcher
        .top_k(&[0.0, 0.0, 0.0], &pool, 3)
        .err()
        .unwrap_or_else(|| panic!("a zero-norm query over an empty pool must fail"));
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
    let error = searcher
        .top_k(&[1.0, 0.0], &pool, 3)
        .err()
        .unwrap_or_else(|| {
            panic!("a query of the wrong dimensionality over an empty pool must fail")
        });
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
}

/// A pool that holds a passage while no segment is open describes an
/// inconsistent epoch: the fault is the engine's, not the caller's.
#[test]
fn a_pool_without_segment_is_an_internal_error() {
    let mut fixture = DenseFixture::create(3);
    fixture.add_generation(&[Sample::new("doc-a", 0, vec![1.0, 0.0, 0.0])]);
    let pool = fixture.pool(&match_all());
    let searcher = ExactSearcher::open(&fixture.segments_dir(), &[], &fixture.records_path())
        .unwrap_or_else(|error| panic!("a searcher over no segment must open: {error}"));
    let error = searcher
        .top_k(&[1.0, 0.0, 0.0], &pool, 3)
        .err()
        .unwrap_or_else(|| panic!("a non-empty pool without a segment must fail"));
    assert_eq!(error.code(), ErrorCode::Internal);
}

/// A vector that holds a NaN cannot produce a cosine. Its score must stop the
/// search with an `internal` error instead of ranking above every finite score
/// through [`f32::total_cmp`].
#[test]
fn a_non_finite_score_is_an_internal_error() {
    let mut fixture = DenseFixture::create(3);
    fixture.add_generation(&[Sample::new("doc-a", 0, vec![f32::NAN, 0.0, 0.0])]);
    let error = fixture
        .searcher()
        .top_k(&[1.0, 0.0, 0.0], &fixture.pool(&match_all()), 3)
        .err()
        .unwrap_or_else(|| panic!("a non-finite cosine must fail"));
    assert_eq!(error.code(), ErrorCode::Internal);
}

/// A non-finite query, or finite components whose squared norm overflows `f32`,
/// is a bad request: the fault is the caller's, not the stored data's. Without
/// this check the norm would be infinite and the normalised query would turn
/// into the zero vector in silence, scoring the whole pool at zero.
#[test]
fn a_non_finite_query_is_rejected() {
    let fixture = tie_fixture();
    let searcher = fixture.searcher();
    let pool = fixture.pool(&match_all());
    let queries = [
        vec![f32::NAN, 0.0, 0.0],
        vec![f32::INFINITY, 0.0, 0.0],
        vec![f32::MAX, f32::MAX, f32::MAX],
    ];
    for query in queries {
        let error = searcher
            .top_k(&query, &pool, 3)
            .err()
            .unwrap_or_else(|| panic!("the query {query:?} must be refused"));
        assert_eq!(
            error.code(),
            ErrorCode::InvalidRequest,
            "the query {query:?} is malformed by the caller, not inconsistent stored data"
        );
    }
}

/// A generation without a segment is the state of a freshly created index: its
/// pool is empty, and there is no dimensionality to compare a query against, so
/// any query returns no result and no error. The zero vector stays refused,
/// because no cosine exists whatever the generation holds.
#[test]
fn an_empty_generation_accepts_any_query_and_returns_no_result() {
    let fixture = DenseFixture::create(3);
    let searcher = fixture.searcher();
    let pool = fixture.pool(&match_all());
    assert!(pool.is_empty(), "a fresh index has no live passage");
    for (query, k) in [
        (&[1.0, 0.0, 0.0][..], 10_usize),
        (&[1.0][..], 10),
        (&[1.0, 0.0, 0.0][..], 0),
    ] {
        let hits = searcher.top_k(query, &pool, k).unwrap_or_else(|error| {
            panic!("a query {query:?} with k = {k} over an empty generation must succeed: {error}")
        });
        assert!(
            hits.is_empty(),
            "an empty generation must return no result for {query:?} with k = {k}"
        );
    }
    let error = searcher
        .top_k(&[0.0], &pool, 10)
        .err()
        .unwrap_or_else(|| panic!("the zero vector must stay refused over an empty generation"));
    assert_eq!(error.code(), ErrorCode::InvalidRequest);
}

/// `open` must refuse a records database it cannot read, at once: a missing file
/// or a foreign file would otherwise surface as a failure of the first search,
/// far from the fault.
#[test]
fn opening_refuses_a_missing_or_foreign_records_database() {
    let mut fixture = DenseFixture::create(3);
    fixture.add_generation(&[Sample::new("doc-a", 0, vec![1.0, 0.0, 0.0])]);
    let snapshot = fixture.snapshot();
    let temp = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));

    let missing = temp.path().join("absent.sqlite");
    let error = ExactSearcher::open(&fixture.segments_dir(), snapshot.segments(), &missing)
        .err()
        .unwrap_or_else(|| panic!("a missing records database must be refused at open"));
    assert_eq!(
        error.code(),
        ErrorCode::Internal,
        "a missing file is an engine-side failure, not a caller error"
    );

    let foreign = temp.path().join("foreign.sqlite");
    std::fs::write(&foreign, b"not a SQLite database at all\n")
        .unwrap_or_else(|error| panic!("the foreign file must be written: {error}"));
    let error = ExactSearcher::open(&fixture.segments_dir(), snapshot.segments(), &foreign)
        .err()
        .unwrap_or_else(|| panic!("a foreign file must be refused at open"));
    assert_eq!(
        error.code(),
        ErrorCode::Internal,
        "SQLite does not validate the header when a connection opens, so the \
         trivial query of `open` is what refuses the file"
    );
}

/// ADR-0043, option A, without marker: a document removed after the snapshot was
/// taken leaves the retained result with its text and its passage fields, no
/// document field and no error. `internal` is reserved for a real inconsistency,
/// and a missing `documents` row is not one.
#[test]
fn missing_document_row_renders_passage_fields() {
    let mut fixture = DenseFixture::create(3);
    fixture.add_generation(&[
        Sample::new("doc-a", 0, vec![1.0, 0.0, 0.0])
            .with_doc_fields(json!({ "fonds": "LEGI" }))
            .with_passage_fields(json!({ "rang": 0 }))
            .with_text("article 1240"),
        Sample::new("doc-b", 0, vec![0.0, 1.0, 0.0])
            .with_doc_fields(json!({ "fonds": "JORF" }))
            .with_passage_fields(json!({ "rang": 0 }))
            .with_text("journal officiel"),
    ]);

    // The snapshot and the pool are taken before the removal, as one request
    // does, and the searcher keeps the records database of that epoch.
    let snapshot = fixture.snapshot();
    let pool = Pool::evaluate(snapshot.filters(), &match_all())
        .unwrap_or_else(|error| panic!("the fixture filter must evaluate: {error}"));
    let searcher = ExactSearcher::open(
        &fixture.segments_dir(),
        snapshot.segments(),
        &fixture.records_path(),
    )
    .unwrap_or_else(|error| panic!("the fixture searcher must open: {error}"));

    // The next generation removes doc-a, the best result of the query: its
    // passage is tombstoned and its document row deleted (ADR-0041).
    fixture.remove_document("doc-a");

    let hits = searcher
        .top_k(&[1.0, 0.0, 0.0], &pool, 2)
        .unwrap_or_else(|error| {
            panic!("a snapshot taken before the removal must still answer: {error}")
        });
    assert_eq!(
        keys(&hits),
        ["doc-a#0", "doc-b#0"],
        "the snapshot keeps the passage the newer generation removed"
    );
    assert_eq!(
        hits[0].text, "article 1240",
        "the text of the removed document's passage stays available"
    );
    assert_eq!(
        hits[0].fields,
        json!({ "rang": 0 }),
        "a result without a document row carries its passage fields only, no marker"
    );
    assert_eq!(
        hits[1].fields,
        json!({ "fonds": "JORF", "rang": 0 }),
        "a result whose document row is present still merges both levels"
    );
}
