# SPDX-License-Identifier: Apache-2.0

"""Tests of the reducer of MES-16 (`resume_mes16.py`).

The summary JSON is the only numbered piece the report cites without the raw
files, so its reduction is pinned here: the nearest rank of ADR-0077 imported
from the harness, the samples selected beyond the limit, and an empty phase,
which the campaign would have recorded as `n = 0`.
"""

from __future__ import annotations

import json

import mes16_charge as harness
import resume_mes16 as reducer


def sample(
    wall_ms: float,
    at_s: float = 0.0,
    took_ms: float | None = None,
    mode: str = "dense",
    pool_size: int = 50000,
    status: int | None = 200,
) -> dict[str, object]:
    """One raw sample, as the harness writes it."""
    return {
        "mode": mode,
        "wall_ms": wall_ms,
        "took_ms": wall_ms if took_ms is None else took_ms,
        "status": status,
        "pool_size": pool_size,
        "at_s": at_s,
    }


def campaign(
    rest: list[dict[str, object]],
    attach: list[dict[str, object]],
    compaction: list[dict[str, object]],
) -> dict[str, object]:
    """A minimal raw campaign, with the keys the reducer reads."""
    limit = 3.0 * harness.nearest_rank([entry["wall_ms"] for entry in rest], 0.99)
    return {
        "campaign": 1,
        "date": "2026-10-30T00:00:00Z",
        "duration_s": 12.5,
        "machine": {"cores": 8},
        "protocol": {"clients": 4},
        "base": {"batch_id": "mes16-base", "duration_ms": 1800.0, "generation": 1},
        "warmup": {"samples": [sample(wall) for wall in (10.0, 20.0)]},
        "phases": {
            "rest": {"samples": rest},
            "attach": {"samples": attach},
            "compaction": {"samples": compaction},
        },
        "writes": [{"kind": "compaction", "batch_id": None, "status": 202}],
        "criterion": {
            "reference": {"phase": "rest", "p99_ms": limit / 3.0, "limit_ms": limit},
            "per_request": {},
            "searches": {"n": 0},
            "writes": {"n": 1},
            "passed": True,
        },
    }


def test_nearest_rank_comes_from_the_harness() -> None:
    """The reducer imports the percentile, it does not recompute it."""
    assert reducer.harness.nearest_rank is harness.nearest_rank


def test_summary_uses_the_nearest_rank_without_interpolation() -> None:
    # Ten values: the p99 is the tenth, never a value between the ninth and it.
    entries = [sample(float(value)) for value in range(1, 11)]
    stats = reducer.summary([entry["wall_ms"] for entry in entries])
    assert stats["n"] == 10
    assert stats["p50"] == 5.0
    assert stats["p95"] == 10.0
    assert stats["p99"] == 10.0
    assert stats["max"] == 10.0


def test_slices_follow_the_harness_buckets() -> None:
    entries = [sample(10.0, at_s=1.0), sample(20.0, at_s=11.0), sample(30.0, at_s=12.0)]
    buckets = reducer.slices(entries, "rest")
    assert [(bucket["from_s"], bucket["n"]) for bucket in buckets] == [(0.0, 1), (10.0, 2)]
    assert buckets[1]["p50_ms"] == 20.0
    assert buckets[1]["p99_ms"] == 30.0


def test_samples_beyond_the_limit_are_selected() -> None:
    rest = [sample(value) for value in (10.0, 10.0, 10.0, 10.0)]
    attach = [sample(20.0), sample(25.0), sample(31.0, at_s=0.5)]
    document = reducer.reduce_campaign(campaign(rest, attach, []))
    assert document["criterion"]["limit_ms"] == 30.0
    assert [entry["wall_ms"] for entry in document["criterion"]["over_limit_samples"]] == [
        31.0
    ]
    assert document["criterion"]["over_limit_samples"][0]["phase"] == "attach"


def test_sample_at_the_limit_is_counted_as_beyond() -> None:
    """The criterion is strict: a search exactly at the limit is over it."""
    rest = [sample(value) for value in (10.0, 10.0, 10.0, 10.0)]
    document = reducer.reduce_campaign(campaign(rest, [sample(30.0)], []))
    assert len(document["criterion"]["over_limit_samples"]) == 1


def test_empty_phase_is_reported_without_raising() -> None:
    rest = [sample(10.0)]
    document = reducer.reduce_campaign(campaign(rest, [], []))
    assert document["phases"]["attach"] == {"n": 0, "slices": []}
    assert document["phases"]["compaction"] == {"n": 0, "slices": []}


def test_phase_statistics_drop_the_samples() -> None:
    rest = [sample(10.0), sample(20.0), sample(30.0)]
    document = reducer.reduce_campaign(campaign(rest, [], []))
    assert "samples" not in document["phases"]["rest"]
    assert document["phases"]["rest"]["n"] == 3
    assert document["phases"]["rest"]["wall_ms"]["p99"] == 30.0


def test_warmup_is_reduced_from_its_own_key() -> None:
    rest = [sample(10.0)]
    document = reducer.reduce_campaign(campaign(rest, [], []))
    assert document["phases"]["warmup"]["n"] == 2
    assert document["phases"]["warmup"]["wall_ms"]["p50"] == 10.0


def test_summary_rounds_to_the_millisecond_fraction() -> None:
    entries = [sample(1.23456), sample(2.0)]
    document = reducer.reduce_campaign(campaign(entries, [], []))
    assert document["phases"]["rest"]["wall_ms"]["min"] == 1.235
    assert document["criterion"]["limit_ms"] == round(3.0 * 2.0, 3)


def test_main_writes_the_summary_for_one_directory(tmp_path) -> None:
    raw = tmp_path / "campaign-1"
    raw.mkdir()
    (raw / "campaign-1.json").write_text(
        json.dumps(campaign([sample(10.0), sample(20.0)], [], [])), encoding="utf-8"
    )
    out = tmp_path / "summary.json"
    arguments = ["--raw", str(raw), "--out", str(out)]
    import sys

    previous = sys.argv
    sys.argv = ["resume_mes16.py", *arguments]
    try:
        assert reducer.main() == 0
    finally:
        sys.argv = previous
    written = json.loads(out.read_text(encoding="utf-8"))
    assert written["campaigns"][0]["campaign"] == 1
    assert written["campaigns"][0]["phases"]["rest"]["wall_ms"]["p99"] == 20.0
