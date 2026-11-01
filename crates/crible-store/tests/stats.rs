// SPDX-License-Identifier: Apache-2.0

// Tests may panic; `allow-panic-in-tests` covers `#[test]` functions only, and
// every helper of this file is a fixture or an assertion.
#![allow(clippy::expect_used, clippy::panic)]

//! The aggregated read of `analytics.sqlite`, `ROUTE-STATS`
//! (`docs/phases/P7-observabilite.md`).
//!
//! References: ADR-0124 (the response of `GET /v1/stats`, the window, the
//! buckets, the percentiles, the rate and the two rankings) and ADR-0117,
//! decisions 2 to 4 (the buckets and the nearest rank).
//!
//! The traffic of the fixtures is chosen so that every expected number is
//! computed from the bucket bounds, never copied from an output (ADR-0124,
//! decision 15). The rows are written by the store itself, then their minute and
//! day are aged by SQL, so a bucket can be controlled without a clock.

use std::path::{Path, PathBuf};

use crible_core::error::ErrorCode;
use crible_store::analytics::{
    ANALYTICS_FILE_NAME, AnalyticsStore, DayIndex, MinuteEvent, ShapeCount, Stats, StatsFilter,
    StatsGroupBy, latency_bucket_upper_ms, read_stats,
};

/// One written fixture: the temporary directory, the file and the statistics.
struct Fixture {
    /// The directory holding the file.
    _directory: tempfile::TempDir,
    /// Path of `analytics.sqlite`.
    path: PathBuf,
    /// The rows a test reads back.
    events: Vec<MinuteEvent>,
    /// The shapes a test reads back.
    shapes: Vec<ShapeCount>,
    /// The daily rows a test reads back.
    days: Vec<DayIndex>,
}

impl Fixture {
    /// A fixture that has not written anything yet.
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory is creatable");
        let path = directory.path().join(ANALYTICS_FILE_NAME);
        let store = AnalyticsStore::open_or_create(&path).expect("the file opens");
        store.write(&[], &[], &[]).expect("the empty flush writes");
        Self {
            _directory: directory,
            path,
            events: Vec::new(),
            shapes: Vec::new(),
            days: Vec::new(),
        }
    }

    /// A fixture holding `events`, `shapes` and `days`, not written yet.
    fn with_rows(events: Vec<MinuteEvent>, shapes: Vec<ShapeCount>, days: Vec<DayIndex>) -> Self {
        Self {
            events,
            shapes,
            days,
            ..Self::new()
        }
    }

    /// Flushes the rows, then ages their minute and writes their day by SQL.
    ///
    /// `minutes` gives the minute of each event by its client; a daily row
    /// carries its day in [`DayIndex::day`], since the store always stamps the
    /// current one.
    fn write(self, minutes: &[(&str, &str)]) -> Self {
        let store = AnalyticsStore::open_or_create(&self.path).expect("the file opens");
        store
            .write(&self.events, &self.shapes, &[])
            .expect("the flush writes");
        drop(store);
        let connection = rusqlite::Connection::open(&self.path).expect("the file is open");
        for (client, minute) in minutes {
            let updated = connection
                .execute(
                    "UPDATE minute_events SET minute = ?1 WHERE client = ?2",
                    rusqlite::params![minute, client],
                )
                .expect("the minute is writable");
            assert_eq!(updated, 1, "one row carries the client {client}");
        }
        for day in &self.days {
            connection
                .execute(
                    "INSERT INTO day_indexes
                       (day, \"index\", documents, live_passages, dead_passages, disk_bytes,
                        generation, batches)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![
                        day.day,
                        day.index,
                        i64::try_from(day.documents).expect("a count"),
                        i64::try_from(day.live_passages).expect("a count"),
                        i64::try_from(day.dead_passages).expect("a count"),
                        i64::try_from(day.disk_bytes).expect("a count"),
                        i64::try_from(day.generation).expect("a count"),
                        i64::try_from(day.batches).expect("a count"),
                    ],
                )
                .expect("the daily row is writable");
        }
        drop(connection);
        self
    }

    /// Reads the statistics over `filter`.
    fn read(&self, filter: &StatsFilter) -> Stats {
        read_stats(&self.path, filter).unwrap_or_else(|error| {
            panic!(
                "the statistics are read: {}: {}",
                error.code().as_str(),
                error.message()
            )
        })
    }
}

/// A row with `client`, `index`, `operation` and `status`.
fn event(
    client: &str,
    index: &str,
    operation: &str,
    status: &str,
    count: u64,
    zero_results: u64,
) -> MinuteEvent {
    let mut event = MinuteEvent::empty(index, operation, status);
    event.client = client.to_owned();
    event.mode = "dense".to_owned();
    event.dense_strategy = "exact".to_owned();
    event.count = count;
    event.zero_results = zero_results;
    event
}

/// Adds `times` samples of `stage` to `bucket`.
fn sample(event: &mut MinuteEvent, stage: usize, bucket: usize, times: u64) {
    for _ in 0..times {
        event.add_latency(stage, bucket);
    }
}

/// The filter of a test, with every parameter absent.
fn filter() -> StatsFilter {
    StatsFilter::parse(None, None, None, None, None).expect("the default filter")
}

/// The five rows of the traffic of the known-numbers test.
fn traffic() -> Fixture {
    let mut rows = vec![
        event("c1", "alpha", "search", "ok", 3, 1),
        event("c2", "alpha", "search", "invalid_filter", 2, 0),
        event("c3", "beta", "search_multi", "ok", 4, 2),
        event("c4", "beta", "search_multi", "timeout", 1, 0),
        event("c5", "alpha", "compact", "ok", 5, 0),
    ];
    // `total` has one sample per event, `validate` only on the first row, and
    // `dense` never: an unsampled stage is omitted from `latency_ms`.
    sample(&mut rows[0], 0, 7, 3);
    sample(&mut rows[0], 8, 3, 3);
    sample(&mut rows[1], 8, 10, 2);
    sample(&mut rows[2], 8, 5, 4);
    sample(&mut rows[3], 8, 20, 1);
    sample(&mut rows[4], 8, 0, 5);
    let shapes = vec![
        ShapeCount {
            index: "alpha".to_owned(),
            shape: "fonds:eq".to_owned(),
            count: 3,
        },
        ShapeCount {
            index: "beta".to_owned(),
            shape: "date_debut:lte,fonds:eq".to_owned(),
            count: 4,
        },
    ];
    let days = vec![DayIndex {
        day: "2026-10-31".to_owned(),
        index: "alpha".to_owned(),
        documents: 2,
        live_passages: 8,
        dead_passages: 2,
        disk_bytes: 1_024,
        generation: 7,
        batches: 3,
    }];
    let fixture = Fixture::with_rows(rows, shapes, days);
    fixture.write(&[
        ("c1", "2026-10-31T08:05:00Z"),
        ("c2", "2026-10-31T08:07:00Z"),
        ("c3", "2026-10-31T08:40:00Z"),
        ("c4", "2026-10-31T09:10:00Z"),
        ("c5", "2026-10-31T09:20:00Z"),
    ])
}

#[test]
fn the_counts_and_the_errors_of_a_known_traffic_are_exact() {
    // ADR-0124, decisions 1 and 5: the counts are summed per bucket, the error
    // codes counted, and the rate divides the zero results of the successful
    // searches by their counts.
    let fixture = traffic();
    let stats = fixture.read(&filter());
    assert_eq!(
        stats.from.as_deref(),
        Some("2026-10-31T08:05:00Z"),
        "the default window starts at the oldest row"
    );
    assert_eq!(
        stats.to.as_deref(),
        Some("2026-10-31T09:20:00Z"),
        "and ends at the newest one"
    );
    assert_eq!(stats.group_by, StatsGroupBy::Hour, "hour is the default");
    assert_eq!(stats.buckets.len(), 2, "two hours hold an event");
    let morning = &stats.buckets[0];
    assert_eq!(morning.start, "2026-10-31T08:00:00Z");
    assert_eq!(morning.count, 9, "3 + 2 + 4 events");
    assert_eq!(morning.errors.get("invalid_filter"), Some(&2));
    let rate = morning.zero_results_rate.expect("two successful searches");
    assert!((rate - 3.0 / 7.0).abs() < f64::EPSILON, "3 of 7: {rate}");
    let evening = &stats.buckets[1];
    assert_eq!(evening.start, "2026-10-31T09:00:00Z");
    assert_eq!(evening.count, 6, "1 timeout + 5 compacts");
    assert_eq!(evening.errors.get("timeout"), Some(&1));
    assert!(
        evening.zero_results_rate.is_none(),
        "no successful search ran in the bucket, so the rate is omitted"
    );
    let totals = stats.totals.as_ref().expect("the window holds events");
    assert_eq!(totals.count, 15);
    assert_eq!(totals.errors.get("invalid_filter"), Some(&2));
    assert_eq!(totals.errors.get("timeout"), Some(&1));
    let rate = totals.zero_results_rate.expect("two successful searches");
    assert!((rate - 3.0 / 7.0).abs() < f64::EPSILON, "3 of 7: {rate}");
}

#[test]
fn the_percentiles_are_the_upper_bounds_of_the_nearest_ranks() {
    // ADR-0124, decision 4 and ADR-0117, decision 4: `r = ceil(p x n / 100)`,
    // and the value is the upper bound of the bucket that contains `r`.
    let fixture = traffic();
    let stats = fixture.read(&filter());
    let totals = stats.totals.as_ref().expect("the window holds events");
    // 15 samples: bucket 0 holds 5, bucket 3 holds 3, bucket 5 holds 4, bucket
    // 10 holds 2 and bucket 20 holds 1.
    let total = totals.latency_ms.total.expect("every event closes a total");
    assert!(
        (total.p50 - latency_bucket_upper_ms(3)).abs() < f64::EPSILON,
        "rank 8 falls in bucket 3: {}",
        total.p50
    );
    assert!(
        (total.p95 - latency_bucket_upper_ms(20)).abs() < f64::EPSILON,
        "rank 15 falls in bucket 20: {}",
        total.p95
    );
    assert!(
        (total.p99 - latency_bucket_upper_ms(20)).abs() < f64::EPSILON,
        "rank 15 is also the 99th: {}",
        total.p99
    );
    let validate = totals
        .latency_ms
        .validate
        .expect("three validation samples");
    assert!(
        (validate.p50 - latency_bucket_upper_ms(7)).abs() < f64::EPSILON,
        "one bucket holds every validation sample: {}",
        validate.p50
    );
    assert!(
        totals.latency_ms.dense.is_none(),
        "a stage without a sample is omitted, never rendered as a zero"
    );
    // Each bucket carries its own percentiles, merged from its rows only.
    let morning = &stats.buckets[0];
    let total = morning.latency_ms.total.expect("the morning holds totals");
    assert!(
        (total.p50 - latency_bucket_upper_ms(5)).abs() < f64::EPSILON,
        "9 samples at 08: rank 5 reaches bucket 5: {}",
        total.p50
    );
}

#[test]
fn the_percentiles_reach_the_first_and_the_last_bucket() {
    // ADR-0124, decision 15: the proof covers a duration below 0.05 ms, which
    // falls in bucket 0, and a duration beyond bucket 62, which falls in bucket
    // 63. With two samples, rank 1 is bucket 0 and ranks 2 are bucket 63.
    let mut row = event("c1", "alpha", "search", "ok", 2, 0);
    sample(&mut row, 8, 0, 1);
    sample(&mut row, 8, 63, 1);
    let fixture = Fixture::with_rows(vec![row], Vec::new(), Vec::new())
        .write(&[("c1", "2026-10-31T08:05:00Z")]);
    let stats = fixture.read(&filter());
    let totals = stats.totals.as_ref().expect("the window holds an event");
    let total = totals.latency_ms.total.expect("both events close a total");
    assert!(
        (total.p50 - latency_bucket_upper_ms(0)).abs() < f64::EPSILON,
        "rank 1 of 2 falls in bucket 0: {}",
        total.p50
    );
    assert!(
        (total.p95 - latency_bucket_upper_ms(63)).abs() < f64::EPSILON,
        "rank 2 of 2 falls in the overflow bucket: {}",
        total.p95
    );
    assert!(
        (total.p99 - latency_bucket_upper_ms(63)).abs() < f64::EPSILON,
        "rank 2 is also the 99th: {}",
        total.p99
    );
}

#[test]
fn a_minute_bucket_aggregates_every_key_of_its_minute() {
    // ADR-0124, decision 2: `minute` is the width of the stored rows, and a
    // bucket sums the keys of the same minute.
    let fixture = traffic();
    let filter =
        StatsFilter::parse(None, None, None, None, Some("minute")).expect("the filter is valid");
    let stats = fixture.read(&filter);
    assert_eq!(stats.group_by, StatsGroupBy::Minute);
    let starts: Vec<&str> = stats
        .buckets
        .iter()
        .map(|bucket| bucket.start.as_str())
        .collect();
    assert_eq!(
        starts,
        vec![
            "2026-10-31T08:05:00Z",
            "2026-10-31T08:07:00Z",
            "2026-10-31T08:40:00Z",
            "2026-10-31T09:10:00Z",
            "2026-10-31T09:20:00Z",
        ],
        "one bucket per stored minute, chronological"
    );
}

#[test]
fn a_day_bucket_starts_at_midnight() {
    // ADR-0124, decision 2: a day bucket starts at `00:00:00` UTC.
    let fixture = traffic();
    let filter =
        StatsFilter::parse(None, None, None, None, Some("day")).expect("the filter is valid");
    let stats = fixture.read(&filter);
    assert_eq!(stats.buckets.len(), 1, "one day holds the whole traffic");
    assert_eq!(stats.buckets[0].start, "2026-10-31T00:00:00Z");
    assert_eq!(stats.buckets[0].count, 15);
}

#[test]
fn the_two_bounds_are_included_and_floored_to_their_minute() {
    // ADR-0124, decision 2 and ADR-0122, decision 9: both bounds are included,
    // a date inside a minute selects that minute, and the echo is normalised in
    // UTC.
    let fixture = traffic();
    let filter = StatsFilter::parse(
        Some("2026-10-31T10:07:30+02:00"),
        Some("2026-10-31T08:40:00Z"),
        None,
        None,
        Some("minute"),
    )
    .expect("the filter is valid");
    let stats = fixture.read(&filter);
    assert_eq!(
        stats.from.as_deref(),
        Some("2026-10-31T08:07:00Z"),
        "10:07:30+02:00 is 08:07:30Z, floored to its minute"
    );
    let starts: Vec<&str> = stats
        .buckets
        .iter()
        .map(|bucket| bucket.start.as_str())
        .collect();
    assert_eq!(
        starts,
        vec!["2026-10-31T08:07:00Z", "2026-10-31T08:40:00Z"],
        "the lower bound includes its minute and the upper one too"
    );
    assert_eq!(
        stats.totals.as_ref().map(|totals| totals.count),
        Some(6),
        "2 invalid filters and 4 multi-index searches"
    );
}

#[test]
fn the_two_filters_select_their_events() {
    // ADR-0124, decision 2: `index` and `operation` are equality filters, and
    // `operation` reaches only the events, never the daily state.
    let fixture = traffic();
    let filter = StatsFilter::parse(None, None, Some("beta"), Some("search_multi"), None)
        .expect("the filter is valid");
    let stats = fixture.read(&filter);
    let totals = stats.totals.as_ref().expect("beta has events");
    assert_eq!(totals.count, 5, "4 successful and 1 timeout");
    assert_eq!(
        stats.top_indexes.len(),
        1,
        "only beta appears: {:?}",
        stats.top_indexes
    );
    assert!(
        stats.indexes.is_empty(),
        "the daily state of beta is outside the traffic window"
    );
}

#[test]
fn an_index_without_an_event_renders_empty_lists() {
    // ADR-0124, decision 2: a valid name with no row is a filter, not a
    // resource: the lists are empty and no member is invented.
    let fixture = traffic();
    let filter =
        StatsFilter::parse(None, None, Some("nowhere"), None, None).expect("the filter is valid");
    let stats = fixture.read(&filter);
    assert!(stats.buckets.is_empty());
    assert!(stats.totals.is_none(), "no event, so no total");
    assert!(stats.top_indexes.is_empty());
    assert!(stats.top_filter_shapes.is_empty());
    assert!(stats.indexes.is_empty());
    assert_eq!(
        stats.from.as_deref(),
        Some("2026-10-31T08:05:00Z"),
        "the window is the retention, even when the filter selects nothing"
    );
}

#[test]
fn a_window_beyond_the_retention_renders_no_value() {
    // ADR-0124, decisions 1 and 2: an empty window keeps the four lists and
    // omits the members without a value, including `totals`.
    let fixture = traffic();
    let filter = StatsFilter::parse(
        Some("2030-01-01T00:00:00Z"),
        Some("2030-01-02T00:00:00Z"),
        None,
        None,
        None,
    )
    .expect("the filter is valid");
    let stats = fixture.read(&filter);
    assert!(stats.buckets.is_empty());
    assert!(stats.totals.is_none());
    assert!(stats.top_indexes.is_empty());
    assert!(stats.top_filter_shapes.is_empty());
    assert!(stats.indexes.is_empty());
    assert_eq!(stats.from.as_deref(), Some("2030-01-01T00:00:00Z"));
    assert_eq!(stats.to.as_deref(), Some("2030-01-02T00:00:00Z"));
}

#[test]
fn the_rankings_are_bounded_and_ordered_by_count_then_name() {
    // ADR-0124, decisions 6 and 7: ten entries at most, count descending, the
    // tie broken by the name or the form, ascending.
    let mut events: Vec<MinuteEvent> = Vec::new();
    for position in 0..12_u64 {
        let index = format!("index-{position:02}");
        events.push(event("c", &index, "search", "ok", position + 1, 0));
    }
    events.push(event("tie-a", "tie-a", "search", "ok", 1, 0));
    events.push(event("tie-b", "tie-b", "search", "ok", 1, 0));
    let mut shapes = Vec::new();
    for (position, name) in ["c:eq", "a:eq", "b:eq", "d:eq"].iter().enumerate() {
        shapes.push(ShapeCount {
            index: "index-11".to_owned(),
            shape: (*name).to_owned(),
            count: u64::try_from(4 - position).expect("a count"),
        });
    }
    shapes.push(ShapeCount {
        index: "tie-a".to_owned(),
        shape: "z:eq".to_owned(),
        count: 1,
    });
    shapes.push(ShapeCount {
        index: "tie-b".to_owned(),
        shape: "y:eq".to_owned(),
        count: 1,
    });
    let fixture = Fixture::with_rows(events, shapes, Vec::new()).write(&[]);
    let stats = fixture.read(&filter());
    assert_eq!(stats.top_indexes.len(), 10, "the ranking is bounded to ten");
    let counts: Vec<u64> = stats.top_indexes.iter().map(|entry| entry.count).collect();
    assert_eq!(
        counts,
        vec![12, 11, 10, 9, 8, 7, 6, 5, 4, 3],
        "count descending, the highest ten"
    );
    let last = &stats.top_indexes[9];
    assert_eq!(last.index, "index-02");
    // The two indexes of one event each are outside the ten, so the tie test
    // reads the shapes instead.
    let forms: Vec<&str> = stats
        .top_filter_shapes
        .iter()
        .map(|entry| entry.shape.as_str())
        .collect();
    assert_eq!(
        forms,
        vec!["c:eq", "a:eq", "b:eq", "d:eq", "y:eq", "z:eq"],
        "count descending, then the smaller form first"
    );
}

#[test]
fn a_tie_of_counts_is_broken_by_the_smaller_name() {
    // ADR-0124, decision 7: the tie is broken by the index name, ascending, so
    // the answer never depends on an insertion order.
    let fixture = Fixture::with_rows(
        vec![
            event("c1", "beta", "search", "ok", 5, 0),
            event("c2", "alpha", "search", "ok", 5, 0),
        ],
        Vec::new(),
        Vec::new(),
    )
    .write(&[]);
    let stats = fixture.read(&filter());
    let names: Vec<&str> = stats
        .top_indexes
        .iter()
        .map(|entry| entry.index.as_str())
        .collect();
    assert_eq!(names, vec!["alpha", "beta"]);
}

#[test]
fn the_daily_state_is_the_last_row_of_the_window() {
    // ADR-0124, decision 8 and ADR-0118, decision 5: the last `day_indexes` row
    // of each index in the window, no gap filled, one entry per index.
    let days = vec![
        DayIndex {
            day: "2026-10-30".to_owned(),
            index: "alpha".to_owned(),
            documents: 1,
            live_passages: 10,
            dead_passages: 0,
            disk_bytes: 100,
            generation: 1,
            batches: 1,
        },
        DayIndex {
            day: "2026-10-31".to_owned(),
            index: "alpha".to_owned(),
            documents: 2,
            live_passages: 8,
            dead_passages: 2,
            disk_bytes: 200,
            generation: 2,
            batches: 2,
        },
        DayIndex {
            day: "2026-10-29".to_owned(),
            index: "gamma".to_owned(),
            documents: 9,
            ..DayIndex::default()
        },
    ];
    let fixture = Fixture::with_rows(
        vec![event("c1", "alpha", "search", "ok", 1, 0)],
        Vec::new(),
        days,
    )
    .write(&[("c1", "2026-10-31T08:05:00Z")]);
    let filter = StatsFilter::parse(
        Some("2026-10-30T00:00:00Z"),
        Some("2026-10-31T23:59:00Z"),
        None,
        None,
        None,
    )
    .expect("the filter is valid");
    let stats = fixture.read(&filter);
    assert_eq!(stats.indexes.len(), 1, "gamma is outside the window");
    let alpha = &stats.indexes[0];
    assert_eq!(alpha.index, "alpha");
    assert_eq!(alpha.day, "2026-10-31", "the last row of the window");
    assert_eq!(alpha.documents, 2);
    assert_eq!(alpha.live_passages, 8);
    assert_eq!(alpha.dead_passages, 2);
    assert!(
        (alpha.dead_ratio - 0.2).abs() < f64::EPSILON,
        "2 of 10 dead: {}",
        alpha.dead_ratio
    );
    assert_eq!(alpha.disk_bytes, 200);
    assert_eq!(alpha.generation, 2);
    assert_eq!(alpha.batches, 2);
}

#[test]
fn the_parameters_are_refused_by_name() {
    // ADR-0124, decision 2: each refusal is `invalid_request` and names the
    // parameter, exactly as the route and the CLI report it.
    // The five parameters of `StatsFilter::parse`, then the name the message
    // must hold.
    type Case<'a> = (
        Option<&'a str>,
        Option<&'a str>,
        Option<&'a str>,
        Option<&'a str>,
        Option<&'a str>,
        &'a str,
    );
    let cases: Vec<Case<'_>> = vec![
        (Some("2026-10-31"), None, None, None, None, "'from'"),
        (None, Some("not a date"), None, None, None, "'to'"),
        (
            Some("2026-10-31T10:00:00Z"),
            Some("2026-10-31T09:00:00Z"),
            None,
            None,
            None,
            "'from'",
        ),
        (None, None, Some("a/b"), None, None, "'index'"),
        (None, None, Some(".."), None, None, "'index'"),
        (None, None, None, Some("purge"), None, "'operation'"),
        (None, None, None, None, Some("week"), "'group_by'"),
    ];
    for (from, to, index, operation, group_by, named) in cases {
        let failure = StatsFilter::parse(from, to, index, operation, group_by)
            .expect_err("{from:?} {to:?} {index:?} {operation:?} {group_by:?} is refused");
        assert_eq!(failure.code(), ErrorCode::InvalidRequest);
        assert!(
            failure.message().contains(named),
            "the refusal names {named}: {}",
            failure.message()
        );
    }
}

#[test]
fn a_reading_writes_nothing() {
    // ADR-0124, decision 12: the read opens the file read-only and writes no
    // byte, so it can run beside the writer of a live instance.
    let fixture = traffic();
    let before = std::fs::read(&fixture.path).expect("the file is readable");
    let stats = fixture.read(&filter());
    assert_eq!(stats.totals.as_ref().map(|totals| totals.count), Some(15));
    let after = std::fs::read(&fixture.path).expect("the file is readable");
    assert_eq!(before, after, "the file is unchanged by the read");
}

/// Reads a statistics answer from a path, for a test that needs the error.
fn read_failure(path: &Path, filter: &StatsFilter) -> crible_core::error::CribleError {
    match read_stats(path, filter) {
        Ok(stats) => panic!("the read is refused: {stats:?}"),
        Err(failure) => failure,
    }
}

#[test]
fn a_file_of_another_format_is_refused() {
    // INV-15 through ADR-0117, decision 1: the version is checked before any
    // row is read.
    let fixture = Fixture::new();
    let connection = rusqlite::Connection::open(&fixture.path).expect("the file is open");
    connection
        .execute("UPDATE meta SET format_version = 99", [])
        .expect("the version is writable");
    drop(connection);
    let failure = read_failure(&fixture.path, &filter());
    assert_eq!(failure.code(), ErrorCode::InvalidRequest);
    assert!(
        failure.message().contains("format_version"),
        "the refusal names the version: {}",
        failure.message()
    );
}

#[test]
fn the_answer_serialises_the_members_of_the_contract() {
    // ADR-0124, decision 1: the members are the fixed list, `totals` is omitted
    // when the window holds no event, and the four lists are always present.
    let fixture = traffic();
    let stats = fixture.read(&filter());
    let value = serde_json::to_value(&stats).expect("the answer is serialisable");
    let object = value.as_object().expect("the answer is an object");
    for member in [
        "from",
        "to",
        "group_by",
        "buckets",
        "totals",
        "top_indexes",
        "top_filter_shapes",
        "indexes",
    ] {
        assert!(
            object.contains_key(member),
            "the member {member} is present"
        );
    }
    assert!(
        !object.contains_key("analytics_dropped"),
        "the counter of the process belongs to the route, not to the file"
    );
    let totals = object["totals"].as_object().expect("totals is an object");
    assert!(
        !totals.contains_key("start"),
        "the total has the shape of a bucket without `start`"
    );
    let filter = StatsFilter::parse(
        Some("2030-01-01T00:00:00Z"),
        Some("2030-01-02T00:00:00Z"),
        None,
        None,
        None,
    )
    .expect("the filter is valid");
    let empty = serde_json::to_value(fixture.read(&filter)).expect("the answer serialises");
    assert!(
        empty.get("totals").is_none(),
        "an empty window omits `totals`: {empty}"
    );
    for member in ["buckets", "top_indexes", "top_filter_shapes", "indexes"] {
        assert!(
            empty[member].as_array().is_some_and(Vec::is_empty),
            "the list {member} is present and empty: {empty}"
        );
    }
}
