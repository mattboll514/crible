// SPDX-License-Identifier: Apache-2.0

//! `INV-07`: every operator and every null case, pinned by a hand-written
//! dataset.
//!
//! References: `docs/08-invariants-et-tests.md` (INV-07), task `EVALUER-FILTRES`
//! of `docs/phases/P1-stockage-et-recherche-exacte.md`, `docs/05-api.md`
//! ("Grammaire des filtres"), `docs/03-recherche.md` ("Filtres → pool") and
//! ADR-0035 (the semantics of null values).
//!
//! # Assumed API
//!
//! Same minimal surface as `inv_07_filter_bitmap_matches_sql.rs`:
//!
//! ```text
//! crible_search::pool::Pool::evaluate(filters: &FilterIndex, filter: &Filter)
//!     -> Result<Pool, CribleError>;
//! Pool::bitmap(&self) -> &RoaringBitmap;
//! Pool::len(&self) -> u32;
//! Pool::is_empty(&self) -> bool;
//! ```
//!
//! # Why a second test
//!
//! The property test compares the engine to a SQL oracle. If both were wrong in
//! the same way, it would pass. This file states the expected sets by hand, from
//! ADR-0035 alone, and checks that the engine **and** the oracle reach them. It
//! therefore fixes the reading of every operator on a state that ADR-0035 was
//! written for: a value, an explicit null, and a key the row does not carry.
//!
//! # The dataset
//!
//! Six passages, six rows of the reference database, one of them tombstoned in a
//! second generation so that "the pool is a subset of the live passages" is
//! checked on data too. `iid` is the passage index, from 0.
//!
//! | iid | fonds | annee | score | vigueur | archive | partie | rang | poids | jour | principal |
//! |---|---|---|---|---|---|---|---|---|---|---|
//! | 0 | LEGI | 2020 | 1.25 | 2020-01-01 | false | corps | 0 | 1.25 | 2020-01-01 | true |
//! | 1 | LEGI | 2020 | 1.25 | 2020-01-01 | false | null | 1 | null | null | false |
//! | 2 | null | 2021 | null | null | true | absent | 2 | 1.25 | absent | true |
//! | 3 | null | 2021 | null | null | true | moyens | null | null | absent | null |
//! | 4 | JORF | null | absent | absent | null | faits | absent | absent | absent | absent |
//! | 5 | dead since generation 2, so never in a pool |
//!
//! "absent" is a key the JSON object does not carry; "null" is an explicit
//! `null`. ADR-0035 makes the two cases identical for `exists`, `ne` and `$not`,
//! and this table is where that equality is checked.

mod common;

use crible_core::filter::Filter;
use crible_core::ids::Iid;
use serde_json::json;

use common::{Fixture, document, fixture_schema, json_object, passage, sorted, sql_matches};

/// Builds the dataset of the table above: one generation, then a tombstone.
fn small_index() -> Fixture {
    let mut fixture = Fixture::create(fixture_schema());

    let documents = vec![
        document(
            "doc-a",
            json_object(vec![
                ("fonds", json!("LEGI")),
                ("annee", json!(2020)),
                ("score", json!(1.25)),
                ("vigueur", json!("2020-01-01")),
                ("archive", json!(false)),
            ]),
            1,
        ),
        document(
            "doc-b",
            // `fonds`, `score` and `vigueur` are explicit nulls here.
            json_object(vec![
                ("fonds", json!(null)),
                ("annee", json!(2021)),
                ("score", json!(null)),
                ("vigueur", json!(null)),
                ("archive", json!(true)),
            ]),
            1,
        ),
        document(
            "doc-c",
            // `annee` and `archive` are nulls, `score` and `vigueur` are absent.
            json_object(vec![
                ("fonds", json!("JORF")),
                ("annee", json!(null)),
                ("archive", json!(null)),
            ]),
            1,
        ),
    ];

    let passages = vec![
        passage(
            0,
            "doc-a",
            0,
            json_object(vec![
                ("partie", json!("corps")),
                ("rang", json!(0)),
                ("poids", json!(1.25)),
                ("jour", json!("2020-01-01")),
                ("principal", json!(true)),
            ]),
            1,
        ),
        passage(
            1,
            "doc-a",
            1,
            // `partie`, `poids` and `jour` are explicit nulls.
            json_object(vec![
                ("partie", json!(null)),
                ("rang", json!(1)),
                ("poids", json!(null)),
                ("jour", json!(null)),
                ("principal", json!(false)),
            ]),
            1,
        ),
        passage(
            2,
            "doc-b",
            0,
            // `partie` and `jour` are absent.
            json_object(vec![
                ("rang", json!(2)),
                ("poids", json!(1.25)),
                ("principal", json!(true)),
            ]),
            1,
        ),
        passage(
            3,
            "doc-b",
            1,
            json_object(vec![
                ("partie", json!("moyens")),
                ("rang", json!(null)),
                ("poids", json!(null)),
                ("principal", json!(null)),
            ]),
            1,
        ),
        passage(
            4,
            "doc-c",
            0,
            // Only `partie` is carried: every other passage field is absent.
            json_object(vec![("partie", json!("faits"))]),
            1,
        ),
        passage(
            5,
            "doc-c",
            1,
            json_object(vec![
                ("partie", json!("corps")),
                ("rang", json!(5)),
                ("poids", json!(1.25)),
                ("jour", json!("1990-01-01")),
                ("principal", json!(false)),
            ]),
            1,
        ),
    ];

    fixture.commit(documents, passages, Vec::new());
    // A dead passage stays in `records.sqlite` and in the structures, and only
    // the live bitmap hides it: this is what "pool ⊆ vivants" checks.
    fixture.commit(Vec::new(), Vec::new(), vec![Iid::new(5)]);

    fixture
}

/// Asserts the pool of `raw`, the size it reports and the SQL oracle.
fn check(fixture: &Fixture, raw: &str, expected: &[u32]) {
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
        sorted(pool.bitmap()),
        expected,
        "the pool of {raw} must hold exactly the expected passages"
    );
    assert_eq!(
        u64::from(pool.len()),
        pool.bitmap().len(),
        "the size of the pool of {raw} must be the size of its bitmap"
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

    // The oracle is checked against the same hand-written set: a property test
    // that compared the engine to a wrong oracle would pass for the wrong
    // reason, and this line is what rules that out operator by operator.
    let records = fixture.records();
    assert_eq!(
        sql_matches(&records, &filter, schema),
        expected,
        "the SQL oracle of {raw} must read the same hand-written set"
    );
}

#[test]
fn inv_07_each_operator_and_null_follows_adr_0035() {
    let fixture = small_index();

    // The live set the expectations are drawn from.
    let live: Vec<u32> = fixture.snapshot().live_iids().map(Iid::get).collect();
    assert_eq!(live, vec![0, 1, 2, 3, 4], "iid 5 is tombstoned");

    // --- keyword, document level ---
    check(&fixture, r#"{"fonds":"LEGI"}"#, &[0, 1]);
    check(&fixture, r#"{"fonds":{"eq":"JORF"}}"#, &[4]);
    check(&fixture, r#"{"fonds":{"ne":"LEGI"}}"#, &[4]);
    check(&fixture, r#"{"fonds":{"in":["JORF","CNIL"]}}"#, &[4]);
    check(&fixture, r#"{"fonds":{"nin":["LEGI"]}}"#, &[4]);
    check(&fixture, r#"{"fonds":{"nin":["LEGI","JORF"]}}"#, &[]);
    check(&fixture, r#"{"fonds":{"exists":true}}"#, &[0, 1, 4]);
    check(&fixture, r#"{"fonds":{"exists":false}}"#, &[2, 3]);
    check(&fixture, r#"{"fonds":{"ne":"LEGI","exists":true}}"#, &[4]);
    // ADR-0035: `$not` complements over the live passages, nulls included, so it
    // differs from `ne` (which keeps `fonds` present only).
    check(&fixture, r#"{"$not":{"fonds":{"eq":"LEGI"}}}"#, &[2, 3, 4]);

    // --- integer, document level ---
    check(&fixture, r#"{"annee":2020}"#, &[0, 1]);
    check(&fixture, r#"{"annee":{"ne":2020}}"#, &[2, 3]);
    check(&fixture, r#"{"annee":{"lt":2021}}"#, &[0, 1]);
    check(&fixture, r#"{"annee":{"lte":2020}}"#, &[0, 1]);
    check(&fixture, r#"{"annee":{"gt":2020}}"#, &[2, 3]);
    check(&fixture, r#"{"annee":{"gte":2021}}"#, &[2, 3]);
    check(&fixture, r#"{"annee":{"in":[2020,1990]}}"#, &[0, 1]);
    check(&fixture, r#"{"annee":{"exists":false}}"#, &[4]);
    check(&fixture, r#"{"$not":{"annee":{"gt":2020}}}"#, &[0, 1, 4]);

    // --- float, document level ---
    check(&fixture, r#"{"score":1.25}"#, &[0, 1]);
    check(&fixture, r#"{"score":{"ne":1.25}}"#, &[]);
    check(&fixture, r#"{"score":{"gt":1.0}}"#, &[0, 1]);
    check(&fixture, r#"{"score":{"exists":false}}"#, &[2, 3, 4]);

    // --- date, document level ---
    check(&fixture, r#"{"vigueur":{"eq":"2020-01-01"}}"#, &[0, 1]);
    check(&fixture, r#"{"vigueur":{"lt":"2020-01-02"}}"#, &[0, 1]);
    check(&fixture, r#"{"vigueur":{"gt":"2020-01-01"}}"#, &[]);
    check(&fixture, r#"{"vigueur":{"exists":true}}"#, &[0, 1]);
    // A null and an absent key are both absent for `exists` (ADR-0035).
    check(&fixture, r#"{"vigueur":{"exists":false}}"#, &[2, 3, 4]);
    check(
        &fixture,
        r#"{"$not":{"vigueur":{"exists":true}}}"#,
        &[2, 3, 4],
    );

    // --- bool, document level ---
    check(&fixture, r#"{"archive":false}"#, &[0, 1]);
    check(&fixture, r#"{"archive":{"eq":true}}"#, &[2, 3]);
    check(&fixture, r#"{"archive":{"exists":true}}"#, &[0, 1, 2, 3]);
    check(&fixture, r#"{"archive":{"exists":false}}"#, &[4]);
    check(&fixture, r#"{"$not":{"archive":false}}"#, &[2, 3, 4]);

    // --- keyword, passage level ---
    check(&fixture, r#"{"partie":"corps"}"#, &[0]);
    check(&fixture, r#"{"partie":{"ne":"corps"}}"#, &[3, 4]);
    check(&fixture, r#"{"partie":{"in":["moyens","faits"]}}"#, &[3, 4]);
    check(&fixture, r#"{"partie":{"nin":["corps","faits"]}}"#, &[3]);
    check(&fixture, r#"{"partie":{"exists":true}}"#, &[0, 3, 4]);
    check(&fixture, r#"{"partie":{"exists":false}}"#, &[1, 2]);
    check(
        &fixture,
        r#"{"$not":{"partie":{"eq":"corps"}}}"#,
        &[1, 2, 3, 4],
    );

    // --- integer, passage level ---
    check(&fixture, r#"{"rang":2}"#, &[2]);
    check(&fixture, r#"{"rang":{"ne":1}}"#, &[0, 2]);
    check(&fixture, r#"{"rang":{"lt":2}}"#, &[0, 1]);
    check(&fixture, r#"{"rang":{"gte":2}}"#, &[2]);
    check(&fixture, r#"{"rang":{"in":[0,2]}}"#, &[0, 2]);
    check(&fixture, r#"{"rang":{"exists":false}}"#, &[3, 4]);

    // --- float, passage level ---
    check(&fixture, r#"{"poids":1.25}"#, &[0, 2]);
    check(&fixture, r#"{"poids":{"exists":false}}"#, &[1, 3, 4]);

    // --- date, passage level ---
    check(&fixture, r#"{"jour":{"eq":"2020-01-01"}}"#, &[0]);
    check(&fixture, r#"{"jour":{"exists":false}}"#, &[1, 2, 3, 4]);

    // --- bool, passage level ---
    check(&fixture, r#"{"principal":true}"#, &[0, 2]);
    check(&fixture, r#"{"principal":{"exists":false}}"#, &[3, 4]);

    // --- composition: implicit AND, `$and`, `$or`, `$not` ---
    check(
        &fixture,
        r#"{"fonds":"LEGI","annee":{"gte":2020}}"#,
        &[0, 1],
    );
    check(
        &fixture,
        r#"{"$or":[{"fonds":{"eq":"JORF"}},{"partie":{"eq":"corps"}}]}"#,
        &[0, 4],
    );
    check(
        &fixture,
        r#"{"$and":[{"partie":{"exists":true}},{"rang":{"exists":true}}]}"#,
        &[0],
    );
    check(&fixture, r#"{"$not":{"annee":{"gte":2021}}}"#, &[0, 1, 4]);
    // A `$not` nested in a `$not`: both complements are taken on the live set.
    check(&fixture, r#"{"$not":{"$not":{"fonds":"LEGI"}}}"#, &[0, 1]);
    check(
        &fixture,
        r#"{"$or":[{"partie":{"ne":"corps"}},{"$not":{"rang":{"exists":true}}}]}"#,
        &[3, 4],
    );
    // A field condition next to a logical key: the object is an implicit AND.
    check(
        &fixture,
        r#"{"fonds":{"exists":true},"$not":{"annee":{"eq":2020}}}"#,
        &[4],
    );
    // An empty object is the empty conjunction: every live passage. Its dual,
    // an empty disjunction, is the empty pool.
    check(&fixture, r#"{}"#, &[0, 1, 2, 3, 4]);
    check(&fixture, r#"{"$or":[]}"#, &[]);

    // The validation accepts an empty list (`docs/05-api.md` caps it at 1 000
    // values and does not require one), and ADR-0035 settles its reading:
    // `in: []` is an empty union, so the empty pool; `nin: []` is
    // `present ∩ ¬empty`, so the present set, nulls excluded.
    check(&fixture, r#"{"fonds":{"in":[]}}"#, &[]);
    check(&fixture, r#"{"fonds":{"nin":[]}}"#, &[0, 1, 4]);
    check(&fixture, r#"{"annee":{"in":[]}}"#, &[]);
    check(&fixture, r#"{"partie":{"nin":[]}}"#, &[0, 3, 4]);
}

/// A filter the validation refuses must not silently evaluate to an empty or a
/// full pool: [`Pool::evaluate`] repeats the checks and reports
/// `invalid_filter`, the code the API uses for a filter the schema rejects.
#[test]
fn pool_evaluate_refuses_a_filter_the_schema_rejects() {
    let fixture = small_index();
    let schema = fixture.schema();
    let filter = Filter::from_json_str(r#"{"fonds":{"lt":"LEGI"}}"#)
        .unwrap_or_else(|error| panic!("the filter must parse: {error}"));
    assert!(
        filter.validate(schema).is_err(),
        "`lt` is not allowed on a keyword field"
    );

    let snapshot = fixture.snapshot();
    let error = crible_search::pool::Pool::evaluate(snapshot.filters(), &filter)
        .expect_err("the evaluator must refuse a filter the schema rejects");
    assert_eq!(
        error.code(),
        crible_core::error::ErrorCode::InvalidFilter,
        "a bad filter is an invalid_filter, like the validation itself"
    );
}

/// A clause that empties the pool must not hide a malformed clause that comes
/// later in the conjunction: the evaluator reports the error, it does not
/// return a plausible empty pool.
#[test]
fn pool_evaluate_checks_every_clause_even_when_the_pool_is_empty() {
    let fixture = small_index();
    let schema = fixture.schema();
    // The keyword clause keeps nothing (`INCONNU` matches no row), and the
    // column clause is ill-typed (`gte` expects an integer).
    let filter =
        Filter::from_json_str(r#"{"$and":[{"fonds":"INCONNU"},{"annee":{"gte":"2020-01-01"}}]}"#)
            .unwrap_or_else(|error| panic!("the filter must parse: {error}"));
    assert!(
        filter.validate(schema).is_err(),
        "the second clause is ill-typed"
    );

    let snapshot = fixture.snapshot();
    let error = crible_search::pool::Pool::evaluate(snapshot.filters(), &filter)
        .expect_err("the empty first clause must not hide the malformed second one");
    assert_eq!(
        error.code(),
        crible_core::error::ErrorCode::InvalidFilter,
        "the evaluator must report the ill-typed clause whatever the order"
    );
}

#[test]
fn inv_07_pool_never_holds_a_tombstoned_passage() {
    let fixture = small_index();
    let snapshot = fixture.snapshot();

    // `rang` is present on iid 5, which the second generation tombstoned: every
    // other passage of the dataset has a `rang` too, so the filter alone would
    // keep the six rows of `records.sqlite`.
    assert_eq!(
        snapshot.filters().live_iids(),
        vec![
            Iid::new(0),
            Iid::new(1),
            Iid::new(2),
            Iid::new(3),
            Iid::new(4)
        ],
        "the live bitmap must be the five passages the tombstone left alive"
    );
    check(&fixture, r#"{"rang":{"gte":0}}"#, &[0, 1, 2]);
    // The complement of an unsatisfiable condition is every live passage, and
    // never the tombstone.
    check(
        &fixture,
        r#"{"$not":{"rang":{"eq":9999}}}"#,
        &[0, 1, 2, 3, 4],
    );
}
