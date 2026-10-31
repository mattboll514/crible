# SPDX-License-Identifier: Apache-2.0

"""Tests of the MES-16 harness (ADR-0113).

The campaign itself is local and belongs to `@mesureur`; these tests pin the
logic the verdict rests on, so a change of the aggregation is caught in CI:

- the nearest-rank percentile of ADR-0077, without interpolation;
- the phase split, which drops the warm-up;
- the criterion of decision 1, per request, and the p99 of a write phase;
- an empty write phase, which measured nothing and cannot pass;
- the counting of statuses, and the `5xx` rules of decision 6 (a `503` fails a
  search, never a write by itself; a `5xx` fails the campaign in every measured
  phase, the rest included);
- the warm-up, kept out of the criterion and sliced by 10 s for the report;
- `pool_size` and the mode mix, kept per phase and per sample;
- the set-up order of a campaign: the base batch is attached before the load.
"""

from __future__ import annotations

import pytest

import mes16_charge as harness


def sample(
    phase: str,
    wall_ms: float,
    status: int | None = 200,
    pool_size: int | None = 20000,
    at_s: float = 0.0,
    mode: str | None = "dense",
) -> harness.Sample:
    """One sample with a trivial `took_ms`."""
    return harness.Sample(
        phase=phase,
        wall_ms=wall_ms,
        took_ms=wall_ms,
        status=status,
        mode=mode,
        pool_size=pool_size,
        at_s=at_s,
    )


def test_nearest_rank_is_the_value_at_ceil_rank() -> None:
    series = [float(value) for value in range(1, 101)]
    assert harness.nearest_rank(series, 0.50) == 50.0
    assert harness.nearest_rank(series, 0.95) == 95.0
    assert harness.nearest_rank(series, 0.99) == 99.0


def test_nearest_rank_interpolates_nothing() -> None:
    # Two samples, ratio 0.5: rank ceil(1) = 1, so the smaller value, never 1.5.
    assert harness.nearest_rank([1.0, 2.0], 0.5) == 1.0
    # Nine samples, ratio 0.99: rank ceil(8.91) = 9, the maximum.
    assert harness.nearest_rank([float(value) for value in range(1, 10)], 0.99) == 9.0


def test_nearest_rank_refuses_an_empty_series() -> None:
    with pytest.raises(ValueError):
        harness.nearest_rank([], 0.99)


def test_split_phases_drops_the_warm_up() -> None:
    samples = [
        sample("warmup", 1000.0),
        sample("rest", 10.0),
        sample("attach", 20.0),
        sample("compaction", 30.0),
        sample("done", 40.0),
    ]
    phases = harness.split_phases(samples)
    assert sorted(phases) == ["attach", "compaction", "rest"]
    assert [entry.wall_ms for entry in phases["rest"]] == [10.0]
    assert [entry.wall_ms for entry in phases["attach"]] == [20.0]
    assert [entry.wall_ms for entry in phases["compaction"]] == [30.0]


def test_warm_up_slices_are_p99_per_ten_seconds() -> None:
    samples = [
        sample("warmup", 10.0, at_s=0.5),
        sample("warmup", 30.0, at_s=9.9),
        sample("warmup", 8.0, at_s=10.1),
        sample("rest", 1.0, at_s=20.5),
        sample("warmup", 9.0, at_s=25.0),
    ]
    slices = harness.warmup_slices(samples)
    assert [(entry["from_s"], entry["to_s"], entry["n"]) for entry in slices] == [
        (0.0, 10.0, 2),
        (10.0, 20.0, 1),
        (20.0, 30.0, 1),
    ]
    assert slices[0]["p99_ms"] == 30.0
    assert [entry["n"] for entry in harness.warmup_slices(samples, 5.0)] == [1, 1, 1, 1]


def test_warm_up_slices_refuse_a_non_positive_width() -> None:
    with pytest.raises(ValueError):
        harness.warmup_slices([sample("warmup", 1.0)], 0.0)


def test_sample_from_keeps_pool_size_and_took() -> None:
    entry = harness.sample_from(
        {"took_ms": 12.5, "pool_size": 20000},
        "attach",
        20.0,
        3.5,
        "hybrid",
    )
    assert entry.status == 200
    assert entry.took_ms == 12.5
    assert entry.pool_size == 20000
    assert entry.at_s == 3.5
    assert entry.mode == "hybrid"


def test_sample_from_tolerates_an_answer_without_pool_size() -> None:
    entry = harness.sample_from({}, "rest", 1.0, 0.0, "dense")
    assert entry.took_ms is None
    assert entry.pool_size is None


def test_count_modes_and_pool_range() -> None:
    samples = [
        sample("attach", 1.0, pool_size=20000),
        sample("attach", 2.0, pool_size=20400),
        harness.Sample("attach", 3.0, 3.0, 200, mode="hybrid", pool_size=None),
    ]
    assert harness.count_modes(samples) == {"dense": 2, "hybrid": 1}
    assert harness.pool_range(samples) == {"min": 20000, "max": 20400}
    assert harness.pool_range([sample("rest", 1.0, pool_size=None)]) is None


def test_phase_stats_keeps_pool_size_and_modes() -> None:
    stats = harness.phase_stats(
        [sample("rest", 10.0, pool_size=20000), sample("rest", 20.0, pool_size=20000)]
    )
    assert stats["n"] == 2
    assert stats["wall_ms"]["p50"] == 10.0
    assert stats["wall_ms"]["p99"] == 20.0
    assert stats["wall_ms"]["max"] == 20.0
    assert stats["pool_size"] == {"min": 20000, "max": 20000}
    assert stats["modes"] == {"dense": 2}
    assert stats["samples"][0]["pool_size"] == 20000


def test_phase_stats_of_an_empty_phase_has_no_statistics() -> None:
    stats = harness.phase_stats([])
    assert stats["n"] == 0
    assert stats["wall_ms"] is None
    assert stats["pool_size"] is None
    assert stats["samples"] == []


def test_count_statuses_names_a_transport_failure() -> None:
    samples = [
        sample("rest", 1.0, 200),
        sample("rest", 2.0, 200),
        sample("attach", 3.0, 503),
        sample("attach", 4.0, None),
    ]
    assert harness.count_statuses(samples) == {"200": 2, "503": 1, "transport": 1}


def test_search_failures_include_503_and_transport() -> None:
    failures = harness.search_failures(
        [
            sample("attach", 1.0, 200),
            sample("attach", 2.0, 503),
            sample("attach", 3.0, 500),
            sample("attach", 4.0, None),
        ]
    )
    assert [entry.status for entry in failures] == [503, 500, None]


def test_criterion_is_strict_per_request() -> None:
    entry = harness.criterion(10.0, [sample("attach", 29.999), sample("attach", 30.0)])
    assert entry["limit_ms"] == 30.0
    assert entry["over"] == 1
    assert entry["worst_ms"] == 30.0
    assert entry["passed"] is False


def test_criterion_passes_under_the_limit() -> None:
    entry = harness.criterion(10.0, [sample("attach", 5.0), sample("attach", 29.0)])
    assert entry["over"] == 0
    assert entry["passed"] is True
    assert entry["p99_passed"] is True


def test_criterion_of_an_empty_phase_fails() -> None:
    # A write phase that recorded no search measured nothing (decision 1).
    entry = harness.criterion(10.0, [])
    assert entry["n"] == 0
    assert entry["passed"] is False
    assert entry["p99_passed"] is False
    assert entry["p99_ms"] is None


def test_verdict_needs_every_write_phase_and_no_search_failure() -> None:
    phases = {
        "rest": [sample("rest", 10.0) for _ in range(100)],
        "attach": [sample("attach", 20.0) for _ in range(50)],
        "compaction": [sample("compaction", 21.0) for _ in range(50)],
    }
    writes = [
        harness.Write("attach", "batch-1", 202, None, 1200.0, 6, "succeeded"),
        harness.Write("compaction", None, 202, None, 900.0, 7, "succeeded"),
    ]
    decision = harness.verdict(10.0, phases, writes)
    assert decision["passed"] is True
    assert decision["per_request"]["attach"]["passed"] is True
    assert decision["searches"]["failures"] == 0


def test_verdict_fails_on_one_search_over_the_limit() -> None:
    phases = {
        "rest": [sample("rest", 10.0) for _ in range(100)],
        "attach": [sample("attach", 20.0) for _ in range(50)] + [sample("attach", 31.0)],
        "compaction": [sample("compaction", 21.0)],
    }
    decision = harness.verdict(10.0, phases, [])
    assert decision["passed"] is False
    assert decision["per_request"]["attach"]["over"] == 1


def test_verdict_counts_a_write_503_without_failing() -> None:
    phases = {
        "rest": [sample("rest", 10.0) for _ in range(100)],
        "attach": [sample("attach", 20.0)],
        "compaction": [sample("compaction", 21.0)],
    }
    writes = [harness.Write("attach", "batch-1", 503, 1.0, 15.0, None, "failed")]
    decision = harness.verdict(10.0, phases, writes)
    assert decision["passed"] is True
    assert decision["writes"]["statuses"] == {"503": 1}
    assert decision["writes"]["explained_503"] == [
        {"kind": "attach", "batch_id": "batch-1", "retry_after": 1.0}
    ]


def test_verdict_fails_on_a_write_5xx_other_than_503() -> None:
    phases = {
        "rest": [sample("rest", 10.0) for _ in range(100)],
        "attach": [sample("attach", 20.0)],
        "compaction": [sample("compaction", 21.0)],
    }
    writes = [harness.Write("attach", "batch-1", 500, None, 15.0, None, "failed")]
    decision = harness.verdict(10.0, phases, writes)
    assert decision["passed"] is False
    assert decision["writes"]["failures"] == [
        {"kind": "attach", "batch_id": "batch-1", "status": 500}
    ]


def test_verdict_fails_on_a_5xx_during_the_rest() -> None:
    # Decision 6: any `5xx` of a search fails the campaign, the reference
    # included, or a failure of the rest phase would pass unseen.
    phases = {
        "rest": [sample("rest", 10.0) for _ in range(99)] + [sample("rest", 11.0, 500)],
        "attach": [sample("attach", 20.0)],
        "compaction": [sample("compaction", 21.0)],
    }
    decision = harness.verdict(10.0, phases, [])
    assert decision["per_request"]["attach"]["passed"] is True
    assert decision["searches"]["failures"] == 1
    assert decision["searches"]["by_phase"]["rest"] == {"200": 99, "500": 1}
    assert decision["passed"] is False


def test_verdict_fails_on_an_empty_write_phase() -> None:
    phases = {
        "rest": [sample("rest", 10.0) for _ in range(100)],
        "attach": [sample("attach", 20.0)],
        "compaction": [],
    }
    writes = [harness.Write("compaction", None, 202, None, 900.0, 7, "succeeded")]
    decision = harness.verdict(10.0, phases, writes)
    assert decision["per_request"]["compaction"]["n"] == 0
    assert decision["per_request"]["compaction"]["passed"] is False
    assert decision["passed"] is False


def test_verdict_reports_the_statuses_of_every_measured_phase() -> None:
    phases = {
        "rest": [sample("rest", 10.0)],
        "attach": [sample("attach", 20.0)],
        "compaction": [sample("compaction", 21.0)],
    }
    decision = harness.verdict(10.0, phases, [])
    assert decision["searches"]["by_phase"] == {
        "rest": {"200": 1},
        "attach": {"200": 1},
        "compaction": {"200": 1},
    }


def test_verdict_ignores_the_warm_up() -> None:
    # Decision 5: the warm-up is measured, never judged. A `5xx` or an outlier
    # there must not change the verdict, and its statuses stay out of `by_phase`.
    phases = {
        "warmup": [sample("warmup", 5000.0, 500)],
        "rest": [sample("rest", 10.0) for _ in range(10)],
        "attach": [sample("attach", 20.0)],
        "compaction": [sample("compaction", 21.0)],
    }
    decision = harness.verdict(10.0, phases, [])
    assert decision["passed"] is True
    assert decision["searches"]["failures"] == 0
    assert "warmup" not in decision["searches"]["by_phase"]
    assert decision["searches"]["n"] == 12


def test_query_set_alternates_dense_and_hybrid() -> None:
    queries = harness.query_set(4, 8)
    assert [entry["mode"] for entry in queries] == ["dense", "hybrid", "dense", "hybrid"]
    assert "query" not in queries[0] and "vector" in queries[0]
    assert "query" in queries[1] and "vector" in queries[1]
    assert queries[0]["vector"].shape == (8,)


def test_query_set_is_deterministic() -> None:
    first = harness.query_set(3, 8)
    second = harness.query_set(3, 8)
    for left, right in zip(first, second):
        assert left["mode"] == right["mode"]
        assert (left["vector"] == right["vector"]).all()


class FakeServer:
    """A server that is never started: the campaign only needs its address."""

    url = "http://127.0.0.1:1"

    def __init__(self, *arguments: object, **keywords: object) -> None:
        pass

    def __enter__(self) -> FakeServer:
        return self

    def __exit__(self, *exc_info: object) -> None:
        return None

    def wait_ready(self, timeout: float = 120.0) -> None:
        pass


def test_campaign_attaches_the_base_before_the_load(monkeypatch, tmp_path) -> None:
    # Decision 1 compares the rest with the write phases: the base batch must be
    # the index the reference measures, so it is attached as the set-up, before
    # the first search of the load. The server and the load are replaced here: no
    # binary is needed to pin the order.
    events: list[str] = []

    def fake_generate(inbox, options):
        events.append("generate")
        return harness.Batches(base="mes16-base", updates=("mes16-update-1",))

    async def fake_create_index(url, admin_key):
        events.append("create_index")
        return 1

    async def fake_attach_base(url, key, batch_id, options):
        events.append(f"attach_base:{batch_id}")
        return harness.Write("attach", batch_id, 202, None, 5.0, 2, "succeeded")

    async def fake_run_load(root, url, keys, batch_ids, options, progress):
        events.append("load")
        assert tuple(batch_ids) == ("mes16-update-1",)
        phases = {
            "warmup": [
                sample("warmup", 40.0, at_s=1.0),
                sample("warmup", 20.0, at_s=11.0),
            ],
            "rest": [sample("rest", 10.0) for _ in range(10)],
            "attach": [sample("attach", 20.0)],
            "compaction": [sample("compaction", 21.0)],
        }
        samples = [entry for entries in phases.values() for entry in entries]
        writes = [
            harness.Write("attach", "mes16-update-1", 202, None, 50.0, 3, "succeeded"),
            harness.Write("compaction", None, 202, None, 40.0, 4, "succeeded"),
        ]
        return samples, writes

    monkeypatch.setattr(harness, "Server", FakeServer)
    monkeypatch.setattr(harness, "generate", fake_generate)
    monkeypatch.setattr(harness, "_create_index", fake_create_index)
    monkeypatch.setattr(harness, "_attach_base", fake_attach_base)
    monkeypatch.setattr(harness, "_run_load", fake_run_load)

    document = harness.campaign(harness.Options(out=tmp_path), 1)
    assert events == ["generate", "create_index", "attach_base:mes16-base", "load"]
    assert document["base"] == {
        "batch_id": "mes16-base",
        "generation": 2,
        "duration_ms": 5.0,
    }
    # The warm-up is kept apart: its slices show the convergence, and it is not
    # one of the phases the criterion reads (decision 5).
    assert sorted(document["phases"]) == ["attach", "compaction", "rest"]
    assert document["warmup"]["n"] == 2
    assert [entry["n"] for entry in document["warmup"]["slices"]] == [1, 1]
    assert document["criterion"]["searches"]["n"] == 12
    assert document["phases"]["rest"]["pool_size"] == {"min": 20000, "max": 20000}
    assert document["phases"]["rest"]["samples"][0]["pool_size"] == 20000

