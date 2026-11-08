# SPDX-License-Identifier: Apache-2.0
"""Tests of the pure helpers of pass C of the Mac mini campaign."""

import json

import pytest

import aggregate_pass_c
import macmini


def test_summary_uses_the_nearest_rank_and_keeps_every_sample() -> None:
    stats = aggregate_pass_c.summary(range(1, 101))
    assert stats["n"] == 100
    assert stats["p50"] == 50
    assert stats["p95"] == 95
    assert stats["p99"] == 99
    assert stats["min"] == 1
    assert stats["max"] == 100


def test_check_summary_refuses_a_figure_that_differs() -> None:
    computed = {"n": 10, "p50": 1.5, "p95": 2.0, "p99": 2.0, "min": 1.0, "max": 2.0}
    aggregate_pass_c.check_summary(computed, computed, "same")
    with pytest.raises(macmini.MacMiniError):
        aggregate_pass_c.check_summary({**computed, "p50": 1.6}, computed, "different")


def test_parse_state_reads_the_container_counters(tmp_path) -> None:
    path = tmp_path / "rest.txt"
    path.write_text(
        "2026-11-07T21:21:47Z\n"
        "VmRSS:\t 3731692 kB\n"
        "RssAnon:\t 3700120 kB\n"
        "RssFile:\t   31572 kB\n"
        "VmSwap:\t       0 kB\n"
        "memory.current\n"
        "9714925568\n"
        "anon 3789197312\n"
        "file 5878091776\n"
        "file_mapped 20746240\n"
        "inactive_file 5744574464\n"
        "active_file 133517312\n"
        "crible-p10 3.696GiB / 9.703GiB 38.09%\n"
        "vm.swapusage: total = 3072.00M  used = 2309.38M  free = 762.62M\n",
        encoding="utf-8",
    )
    state = aggregate_pass_c.parse_state(path)
    assert state["vmrss_kb"] == 3731692
    assert state["vmswap_kb"] == 0
    assert state["memory_current"] == 9714925568
    assert state["anon"] == 3789197312
    assert state["file"] == 5878091776
    assert state["docker_mem"] == "3.696GiB"
    assert state["host_swap_used_m"] == 2309.38


def test_encode_points_pools_two_runs_and_refuses_a_cache(tmp_path) -> None:
    for run in (1, 2):
        (tmp_path / f"mes05-encode-int8-{run}.json").write_text(
            json.dumps(
                {
                    "precision": "int8",
                    "precision_effective": "int8",
                    "cache_capacity": 0,
                    "cache_hits": 0,
                    "cache_misses": 1301,
                    "calls_us": [10_000] * 1000,
                    "count": 1000,
                    "first_call_ms": 1000.0,
                    "threads": {"available_parallelism": 8, "process_threads": 8},
                }
            ),
            encoding="utf-8",
        )
    points = aggregate_pass_c.encode_points(tmp_path)
    assert len(points) == 1
    assert points[0]["precision"] == "int8"
    assert points[0]["pooled"]["n"] == 2000
    assert points[0]["pooled"]["p50"] == 10.0
    assert len(points[0]["runs"]) == 2

    document = json.loads((tmp_path / "mes05-encode-int8-1.json").read_text())
    document["cache_capacity"] = 1024
    (tmp_path / "mes05-encode-int8-1.json").write_text(json.dumps(document))
    with pytest.raises(macmini.MacMiniError):
        aggregate_pass_c.encode_points(tmp_path)


def test_direct_latency_keeps_the_unique_series_without_text(tmp_path) -> None:
    payload = {
        "method": "one POST search per query",
        "queries": 2,
        "unique_queries": 1,
        "cache_hits": 1,
        "warmup_ms": 500.0,
        "failures": [],
        "summary_ms": {"total": {"p50": 100.0, "p95": 100.0, "p99": 100.0, "n": 2}},
        "summary_ms_unique": {"total": {"p50": 120.0, "p95": 120.0, "p99": 120.0, "n": 1}},
        "samples": [
            {"id": "Q1", "index": "dole", "status": 200, "total": 120.0},
            {"id": "Q2", "index": "dole", "status": 200, "total": 80.0},
        ],
    }
    (tmp_path / "direct-latency.json").write_text(json.dumps(payload), encoding="utf-8")
    result = aggregate_pass_c.direct_latency(tmp_path)
    assert result["unique_queries"] == 1
    assert result["cache_hits"] == 1
    assert result["unique_queries_only"]["total"]["p50"] == 120.0
    assert all("query" not in sample for sample in result["samples"])
