// SPDX-License-Identifier: Apache-2.0

// Test helpers may panic; `allow-panic-in-tests` covers `#[test]` functions only.
#![allow(clippy::panic)]

//! `INSERER-POOL-EN-ORDRE`: the bitmap a range clause builds equals the one a
//! one-by-one insertion of the kept iids builds.
//!
//! References: `docs/phases/P4-recherche-rapide.md` (`INSERER-POOL-EN-ORDRE`),
//! `docs/mesures/MES-14-cout-filtres-intervalles.md` (§ 2, § 3, § 4.2 and § 5),
//! ADR-0018 (interval filters on value columns), ADR-0046 (the cost of a range
//! clause) and `docs/03-recherche.md` ("Filtres → pool").
//!
//! # API under test
//!
//! ```text
//! crible_search::pool::Pool::evaluate(filters: &FilterIndex, filter: &Filter)
//!     -> Result<Pool, CribleError>;
//! Pool::bitmap(&self) -> &RoaringBitmap;
//! Pool::len(&self) -> u32;
//! Pool::is_empty(&self) -> bool;
//! ```
//!
//! # What the test states
//!
//! The task rewrites `scan_column` (`crates/crible-search/src/pool.rs`) so that
//! the result bitmap is built in ascending iid order instead of one
//! `RoaringBitmap::insert` per kept iid. The observable contract does not
//! change: for every case of MES-14, the bitmap of the pool is **equal** to the
//! set this file computes from the same rows by walking the iids in ascending
//! order and inserting each kept one, which is the construction `scan_column`
//! used before the task.
//!
//! Equality of two `RoaringBitmap`s is equality of the two sets of iids,
//! whatever the containers were filled in, so the assertion is free of any
//! detail of the ordered construction. It fails exactly when the new
//! construction drops an iid, adds one, or mixes two columns.
//!
//! # The cases of MES-14
//!
//! A full domain (`annee 2000..2010`, `date_publication 2000-01-01..2003-12-31`,
//! `score > 0.6`), a domain reduced by a keyword clause (`partie=corps` and
//! `categorie=cat-03`), one bound and two bounds, a case that keeps no iid
//! (`annee >= 5000`) and a case that keeps every iid carrying a value
//! (`annee >= 1900`). The rows are **not** the fixtures of the measure: this
//! file generates its own index, of test size, with the schema and the
//! distributions MES-14 § 2 describes (one document per five passages, 5 % of
//! nulls on the integer, float and date fields of the document, 10 % on the two
//! keyword or date fields of the passage, 16 categories, 5 parties, years and
//! dates 1990 to 2025, drawn by splitmix64 from the iid).
//!
//! # Determinism
//!
//! Every value is drawn from a fixed seed, so a run always builds the same
//! index. Nothing here reads the clock, the network or an environment variable.

mod common;

use crible_core::config::Schema;
use crible_core::filter::Filter;
use crible_store::filters_index::date_days;
use roaring::RoaringBitmap;
use serde_json::{Map, Value, json};

use common::{Fixture, document, passage, sorted, sql_matches};

/// JSON schema of the fixture: the fields of the MES-14 fixtures.
const SCHEMA_JSON: &str = r#"{
    "document_fields": {
        "categorie":        { "type": "keyword" },
        "annee":            { "type": "integer" },
        "score":            { "type": "float" },
        "date_publication": { "type": "date" }
    },
    "passage_fields": {
        "partie":     { "type": "keyword" },
        "date_effet": { "type": "date" }
    }
}"#;

/// Number of documents of the fixture, so 6 000 passages: enough for the
/// `categorie=cat-03` case to keep rows in a four-year window of publication
/// dates, which a thousand rows did not.
const DOCUMENT_COUNT: usize = 1200;
/// Passages per document, as in the MES-14 fixtures.
const PASSAGES_PER_DOCUMENT: usize = 5;
/// Seed of the value generator, fixed so a run is reproducible.
const SEED: u64 = 0x5EED_0014;
/// Number of distinct categories of `categorie`, as in `synth-small`.
const CATEGORIES: u64 = 16;
/// Parties of `partie`, as in `synth-small`.
const PARTIES: [&str; 5] = ["corps", "moyens", "faits", "annexe", "dispositif"];

/// The deterministic generator of the fixture values: splitmix64, the same
/// family MES-14 § 2 names.
struct Draw(u64);

impl Draw {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    /// `None` once out of `one_in`, a drawn value otherwise: the null rate of
    /// MES-14 § 2.
    fn maybe<T>(&mut self, one_in: u64, value: impl FnOnce(&mut Self) -> T) -> Option<T> {
        if self.below(one_in) == 0 {
            None
        } else {
            Some(value(self))
        }
    }

    /// A date of 1990 to 2025, zero-padded so that a text order is a
    /// chronological order in the SQL oracle too.
    fn date(&mut self) -> String {
        let year = 1990 + self.below(36);
        let month = 1 + self.below(12);
        let day = 1 + self.below(28);
        format!("{year:04}-{month:02}-{day:02}")
    }
}

/// The rows of the fixture, one entry per iid, so the reference set is computed
/// independently of the index the pool reads.
struct Data {
    /// Document-level `categorie`, expanded to its passages.
    categorie: Vec<Option<String>>,
    /// Document-level `annee`, expanded to its passages.
    annee: Vec<Option<i64>>,
    /// Document-level `score`, expanded to its passages.
    score: Vec<Option<f64>>,
    /// Document-level `date_publication`, in days, expanded to its passages.
    date_publication: Vec<Option<i32>>,
    /// Passage-level `partie`.
    partie: Vec<Option<String>>,
    /// Passage-level `date_effet`, in days.
    date_effet: Vec<Option<i32>>,
}

impl Data {
    /// Number of passages of the fixture.
    fn len(&self) -> u32 {
        u32::try_from(self.annee.len()).unwrap_or_else(|error| panic!("{error}"))
    }
}

/// Builds the fixture index and keeps the rows it was built from.
fn fixture() -> (Fixture, Data) {
    let schema = Schema::from_json_str(SCHEMA_JSON)
        .unwrap_or_else(|error| panic!("the fixture schema must be valid: {error}"));
    let mut fixture = Fixture::create(schema);

    let mut data = Data {
        categorie: Vec::new(),
        annee: Vec::new(),
        score: Vec::new(),
        date_publication: Vec::new(),
        partie: Vec::new(),
        date_effet: Vec::new(),
    };
    let mut documents = Vec::new();
    let mut passages = Vec::new();

    for doc in 0..DOCUMENT_COUNT {
        let doc_id = format!("doc-{doc:03}");
        let mut draw = Draw::new(SEED ^ (doc as u64 + 1));

        // MES-14 § 2: `categorie` is always present, the three other document
        // fields are null 5 % of the time.
        let categorie = format!("cat-{:02}", draw.below(CATEGORIES));
        let annee = draw.maybe(20, |draw| 1990 + draw.below(36) as i64);
        let score = draw.maybe(20, |draw| draw.below(1_000) as f64 / 100.0);
        let date_publication = draw.maybe(20, Draw::date);
        let publication_days = date_publication.as_deref().map(days);

        documents.push(document(
            &doc_id,
            document_fields(&categorie, annee, score, date_publication.as_deref()),
            1,
        ));

        for ord in 0..PASSAGES_PER_DOCUMENT {
            let iid = (doc * PASSAGES_PER_DOCUMENT + ord) as u32;
            let mut draw = Draw::new(SEED ^ (0x1_0000_0000 + u64::from(iid)));

            // MES-14 § 2: the two passage fields are null 10 % of the time.
            let partie = draw.maybe(10, |draw| {
                PARTIES[draw.below(PARTIES.len() as u64) as usize].to_owned()
            });
            let date_effet = draw.maybe(10, Draw::date);

            passages.push(passage(
                iid,
                &doc_id,
                ord as u32,
                passage_fields(partie.as_deref(), date_effet.as_deref()),
                1,
            ));

            data.categorie.push(Some(categorie.clone()));
            data.annee.push(annee);
            data.score.push(score);
            data.date_publication.push(publication_days);
            data.partie.push(partie);
            data.date_effet.push(date_effet.as_deref().map(days));
        }
    }

    fixture.commit(documents, passages, Vec::new());
    (fixture, data)
}

/// The document row of the fixture; a null value is an explicit JSON null, the
/// state ADR-0035 settles.
fn document_fields(
    categorie: &str,
    annee: Option<i64>,
    score: Option<f64>,
    date_publication: Option<&str>,
) -> Value {
    let mut fields = Map::new();
    fields.insert("categorie".to_owned(), json!(categorie));
    fields.insert(
        "annee".to_owned(),
        annee.map_or(Value::Null, |year| json!(year)),
    );
    fields.insert(
        "score".to_owned(),
        score.map_or(Value::Null, |value| json!(value)),
    );
    fields.insert(
        "date_publication".to_owned(),
        date_publication.map_or(Value::Null, |text| json!(text)),
    );
    Value::Object(fields)
}

/// The passage row of the fixture.
fn passage_fields(partie: Option<&str>, date_effet: Option<&str>) -> Value {
    let mut fields = Map::new();
    fields.insert(
        "partie".to_owned(),
        partie.map_or(Value::Null, |text| json!(text)),
    );
    fields.insert(
        "date_effet".to_owned(),
        date_effet.map_or(Value::Null, |text| json!(text)),
    );
    Value::Object(fields)
}

/// Day count of a `YYYY-MM-DD` date, the unit the column holds.
fn days(text: &str) -> i32 {
    date_days(text).unwrap_or_else(|| panic!("'{text}' must be a date in YYYY-MM-DD"))
}

// --- The reference construction ---

/// The set `scan_column` built before the task: walk the ids in ascending order
/// and insert each kept one.
///
/// `values` is the column indexed by iid, `None` being a null (ADR-0035). When a
/// `domain` is given, only its ids are read, exactly as the scan of a reduced
/// domain does (ADR-0018).
fn reference_scan<T>(
    values: &[Option<T>],
    domain: Option<&RoaringBitmap>,
    keep: impl Fn(&T) -> bool,
) -> RoaringBitmap {
    let kept = |iid: u32| match &values[iid as usize] {
        Some(value) => keep(value),
        None => false,
    };
    let mut result = RoaringBitmap::new();
    match domain {
        Some(domain) => {
            for iid in domain.iter() {
                if kept(iid) {
                    result.insert(iid);
                }
            }
        }
        None => {
            for iid in 0..u32::try_from(values.len()).unwrap_or_else(|error| panic!("{error}")) {
                if kept(iid) {
                    result.insert(iid);
                }
            }
        }
    }
    result
}

/// The set of a range clause with inclusive bounds: both are optional, so one
/// call states a one-bound and a two-bound range alike.
fn reference_range<T: Copy + PartialOrd>(
    values: &[Option<T>],
    domain: Option<&RoaringBitmap>,
    low: Option<T>,
    high: Option<T>,
) -> RoaringBitmap {
    reference_scan(values, domain, |value| {
        low.is_none_or(|low| *value >= low) && high.is_none_or(|high| *value <= high)
    })
}

/// The set of a `keyword` condition, the bitmap clause MES-14 puts in front of a
/// range clause to reduce its domain.
fn reference_keyword(values: &[Option<String>], expected: &str) -> RoaringBitmap {
    reference_scan(values, None, |value| value.as_str() == expected)
}

/// Reference of an `$and` of one `keyword` clause and one or two bounds of a
/// column, read in the order `evaluate_and` runs them: the keyword clause first,
/// then each bound over the domain the previous clause kept.
fn reference_keyword_then_range<T: Copy + PartialOrd>(
    keyword: (&[Option<String>], &str),
    values: &[Option<T>],
    bounds: (Option<T>, Option<T>),
) -> RoaringBitmap {
    let (texts, expected) = keyword;
    let mut domain = reference_keyword(texts, expected);
    if let Some(low) = bounds.0 {
        let scan = reference_range(values, Some(&domain), Some(low), None);
        domain &= scan;
    }
    if let Some(high) = bounds.1 {
        let scan = reference_range(values, Some(&domain), None, Some(high));
        domain &= scan;
    }
    domain
}

/// Every passage of the fixture, which the single generation leaves alive.
fn live(data: &Data) -> RoaringBitmap {
    (0..data.len()).collect()
}

// --- The assertion ---

/// Asserts the pool of `raw` is exactly `expected`, as a bitmap.
///
/// The expected set is also compared to the SQL oracle of INV-07
/// (`common::sql_matches`), so a reference that mirrored a wrong engine could
/// not pass: the two agree or the case fails.
fn check(fixture: &Fixture, raw: &str, expected: &RoaringBitmap) {
    let schema = fixture.schema();
    let filter = Filter::from_json_str(raw)
        .unwrap_or_else(|error| panic!("the filter {raw} must parse: {error}"));
    filter
        .validate(schema)
        .unwrap_or_else(|error| panic!("the filter {raw} must validate: {error}"));

    let snapshot = fixture.snapshot();
    let pool = crible_search::pool::Pool::evaluate(snapshot.filters(), &filter)
        .unwrap_or_else(|error| panic!("the filter {raw} must evaluate: {error}"));

    assert_eq!(
        pool.bitmap(),
        expected,
        "the bitmap of {raw} must equal the one a one-by-one insertion builds: \
         got {:?}, expected {:?}",
        sorted(pool.bitmap()),
        sorted(expected)
    );
    assert_eq!(
        u64::from(pool.len()),
        pool.bitmap().len(),
        "the size the pool of {raw} reports must be the size of its bitmap"
    );
    assert_eq!(
        pool.is_empty(),
        expected.is_empty(),
        "the pool of {raw} must report emptiness consistently"
    );
    assert!(
        pool.bitmap().is_subset(snapshot.filters().live()),
        "the pool of {raw} must never hold a tombstoned passage"
    );

    let records = fixture.records();
    assert_eq!(
        sql_matches(&records, &filter, schema),
        sorted(expected),
        "the SQL oracle of {raw} must read the same set as the reference"
    );
}

// --- The cases of MES-14 ---

#[test]
fn range_clause_bitmap_equals_reference_on_a_full_domain() {
    let (fixture, data) = fixture();
    assert_eq!(
        fixture.snapshot().filters().live().len(),
        u64::from(data.len()),
        "the fixture must leave every passage alive, or the expected sets would drift"
    );

    // `integer` alone, `annee 2000..2010`: two bounds, full domain (MES-14 § 3).
    let expected = reference_range(&data.annee, None, Some(2000), Some(2010));
    assert!(
        !expected.is_empty() && expected.len() < u64::from(data.len()),
        "the case must keep some passages and drop some others: {} kept of {}",
        expected.len(),
        data.len()
    );
    check(
        &fixture,
        r#"{"$and":[{"annee":{"gte":2000}},{"annee":{"lte":2010}}]}"#,
        &expected,
    );

    // `date` alone, `date_publication 2000-01-01..2003-12-31`: two bounds.
    let expected = reference_range(
        &data.date_publication,
        None,
        Some(days("2000-01-01")),
        Some(days("2003-12-31")),
    );
    assert!(
        !expected.is_empty(),
        "the case must keep at least one publication date"
    );
    check(
        &fixture,
        r#"{"$and":[{"date_publication":{"gte":"2000-01-01"}},{"date_publication":{"lte":"2003-12-31"}}]}"#,
        &expected,
    );

    // `float` alone, `score > 0.6`: one strict bound.
    let expected = reference_scan(&data.score, None, |value| *value > 0.6);
    assert!(
        !expected.is_empty(),
        "the case must keep at least one score"
    );
    check(&fixture, r#"{"score":{"gt":0.6}}"#, &expected);

    // A `date` of a passage, one bound: the same scan on the passage level.
    let expected = reference_range(&data.date_effet, None, None, Some(days("2000-12-31")));
    assert!(expected.is_empty() || expected.len() < u64::from(data.len()));
    check(
        &fixture,
        r#"{"date_effet":{"lte":"2000-12-31"}}"#,
        &expected,
    );
}

#[test]
fn range_clause_bitmap_equals_reference_on_a_domain_reduced_by_partie() {
    let (fixture, data) = fixture();
    let full = reference_range(&data.annee, None, Some(2000), Some(2010));

    // `partie=corps` ET `annee 2000..2010` (MES-14 § 3): the range scans the
    // domain the keyword clause kept.
    let expected = reference_keyword_then_range(
        (&data.partie, "corps"),
        &data.annee,
        (Some(2000), Some(2010)),
    );
    assert!(
        !expected.is_empty() && expected.is_subset(&full),
        "the reduced domain must keep some passages and stay inside the full-domain range"
    );
    check(
        &fixture,
        r#"{"$and":[{"partie":"corps"},{"annee":{"gte":2000}},{"annee":{"lte":2010}}]}"#,
        &expected,
    );

    // `partie=corps` ET `date_publication 2000-01-01..2003-12-31`.
    let expected = reference_keyword_then_range(
        (&data.partie, "corps"),
        &data.date_publication,
        (Some(days("2000-01-01")), Some(days("2003-12-31"))),
    );
    assert!(
        !expected.is_empty(),
        "the reduced date case must keep at least one passage"
    );
    check(
        &fixture,
        r#"{"$and":[{"partie":"corps"},{"date_publication":{"gte":"2000-01-01"}},{"date_publication":{"lte":"2003-12-31"}}]}"#,
        &expected,
    );

    // A one-bound range behind the same keyword clause.
    let expected =
        reference_keyword_then_range((&data.partie, "corps"), &data.score, (None, Some(0.5)));
    check(
        &fixture,
        r#"{"$and":[{"partie":"corps"},{"score":{"lte":0.5}}]}"#,
        &expected,
    );
}

#[test]
fn range_clause_bitmap_equals_reference_on_a_domain_reduced_by_categorie() {
    let (fixture, data) = fixture();
    let full = reference_range(&data.annee, None, Some(2000), Some(2010));

    // `categorie=cat-03` ET `annee 2000..2010`: the most selective keyword
    // clause of MES-14 § 3.
    let expected = reference_keyword_then_range(
        (&data.categorie, "cat-03"),
        &data.annee,
        (Some(2000), Some(2010)),
    );
    assert!(
        !expected.is_empty() && expected.is_subset(&full),
        "the category must keep some passages inside the full-domain range"
    );
    assert!(
        expected.len() < full.len(),
        "the category must really narrow the domain: {} kept against {}",
        expected.len(),
        full.len()
    );
    check(
        &fixture,
        r#"{"$and":[{"categorie":"cat-03"},{"annee":{"gte":2000}},{"annee":{"lte":2010}}]}"#,
        &expected,
    );

    // `categorie=cat-03` ET `date_publication 2000-01-01..2003-12-31`.
    let expected = reference_keyword_then_range(
        (&data.categorie, "cat-03"),
        &data.date_publication,
        (Some(days("2000-01-01")), Some(days("2003-12-31"))),
    );
    assert!(
        !expected.is_empty(),
        "the reduced date case must keep at least one passage"
    );
    check(
        &fixture,
        r#"{"$and":[{"categorie":"cat-03"},{"date_publication":{"gte":"2000-01-01"}},{"date_publication":{"lte":"2003-12-31"}}]}"#,
        &expected,
    );
}

#[test]
fn range_clause_bitmap_is_empty_when_no_iid_is_kept() {
    let (fixture, data) = fixture();

    // The `annee >= 5000` control of MES-14 § 3: the whole column is read and
    // nothing is kept, so the result bitmap stays empty.
    let expected = reference_range(&data.annee, None, Some(5000), None);
    assert!(
        expected.is_empty(),
        "the fixture must carry no year at or above 5000"
    );
    check(&fixture, r#"{"annee":{"gte":5000}}"#, &expected);

    // The same control behind a reduced domain.
    let expected =
        reference_keyword_then_range((&data.partie, "corps"), &data.annee, (Some(5000), None));
    assert!(expected.is_empty());
    check(
        &fixture,
        r#"{"$and":[{"partie":"corps"},{"annee":{"gte":5000}}]}"#,
        &expected,
    );

    // An empty result on a date column, both bounds above every stored date.
    let expected = reference_range(
        &data.date_publication,
        None,
        Some(days("2030-01-01")),
        Some(days("2040-12-31")),
    );
    assert!(expected.is_empty());
    check(
        &fixture,
        r#"{"$and":[{"date_publication":{"gte":"2030-01-01"}},{"date_publication":{"lte":"2040-12-31"}}]}"#,
        &expected,
    );
}

#[test]
fn range_clause_bitmap_equals_reference_when_every_iid_is_kept() {
    let (fixture, data) = fixture();

    // The `annee >= 1900` control of MES-14 § 3: every row carrying a year is
    // kept, so the result is the present bitmap of the column, and the null
    // rows stay out (ADR-0035).
    let expected = reference_range(&data.annee, None, Some(1900), None);
    assert_eq!(
        expected.len(),
        data.annee.iter().flatten().count() as u64,
        "every non-null year must be kept"
    );
    assert!(
        expected.len() < u64::from(data.len()),
        "the fixture must hold null years, or the case would not tell presence from the live set"
    );
    check(&fixture, r#"{"annee":{"gte":1900}}"#, &expected);

    // Every passage keeps a `date_effet` at or before 2100-01-01.
    let expected = reference_range(&data.date_effet, None, None, Some(days("2100-01-01")));
    assert_eq!(
        expected.len(),
        data.date_effet.iter().flatten().count() as u64,
        "every non-null effect date must be kept"
    );
    check(
        &fixture,
        r#"{"date_effet":{"lte":"2100-01-01"}}"#,
        &expected,
    );
}

/// The scan of a column never receives a domain that was filled in ascending
/// order only: an `$and` behind a `$not` hands it a complement, built by
/// removing iids from the live set.
///
/// The column is still read in ascending iid order there, because
/// `RoaringBitmap::iter` is ordered, so a construction that appends each kept
/// iid is exact. This case pins the bitmap equality on such a domain: dropping
/// it would leave the ordered construction checked on plain ascending domains
/// only.
#[test]
fn range_clause_bitmap_equals_reference_under_a_complement_domain() {
    let (fixture, data) = fixture();

    // `$not` keeps the passages without a `partie` (nulls included, ADR-0035);
    // the range clause that follows reads that complement.
    let mut domain = live(&data);
    domain -= reference_keyword(&data.partie, "corps");
    let scan = reference_range(&data.annee, Some(&domain), Some(2000), Some(2010));
    let expected = &domain & &scan;
    assert!(
        !expected.is_empty(),
        "the complement must hand the scan a non-empty domain"
    );
    check(
        &fixture,
        r#"{"$and":[{"$not":{"partie":"corps"}},{"annee":{"gte":2000}},{"annee":{"lte":2010}}]}"#,
        &expected,
    );

    // The same shape with a `$not` whose child is a boolean-ish bitmap clause
    // over the document level, so the complement is not a single-keyword one.
    let mut domain = live(&data);
    domain -= reference_keyword(&data.categorie, "cat-03");
    let scan = reference_range(
        &data.date_publication,
        Some(&domain),
        None,
        Some(days("2001-12-31")),
    );
    let expected = &domain & &scan;
    check(
        &fixture,
        r#"{"$and":[{"$not":{"categorie":"cat-03"}},{"date_publication":{"lte":"2001-12-31"}}]}"#,
        &expected,
    );
}
