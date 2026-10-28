# SPDX-License-Identifier: Apache-2.0
"""Tests of the artifact assembler of MES-03 (ADR-0079, decision 1)."""

from __future__ import annotations

import json
from pathlib import Path

from mes03_profile_artifact import BASE_QUERIES, build, build_point, order_path

RUN = {
    "n": 1000,
    "k": 100,
    "queries": BASE_QUERIES,
    "iterations": 180,
    "ms_p50": 50.0,
    "ms_p95": 56.0,
}


def write_profile(directory: Path, name: str, run: dict | None = None) -> None:
    document = {
        "threads": 8,
        "main_thread": 0,
        "total_samples": 1000,
        "attributed_samples": 1000,
        "unattributed_samples": 0,
        "posts": [{"post": "key read: page reads (b-tree, pread)", "samples": 600}],
        "groups": {"key read": {"samples": 600, "share": 0.6}},
        "run": dict(RUN if run is None else run),
        "sample_file": "sample.txt",
        "run_file": "run.json",
    }
    (directory / name).write_text(json.dumps(document), encoding="utf-8")


def write_order(directory: Path, name: str, distinct: int, queries: int) -> None:
    order = {
        "queries": queries,
        "pairs": queries * (BASE_QUERIES - 1),
        "distinct_iids": distinct,
        "per_query": [{"query": 0}],
    }
    (directory / name).write_text(json.dumps({"order": order}), encoding="utf-8")


def test_order_path_uses_the_series_own_query_count() -> None:
    directory = Path("/tmp")

    assert order_path(directory, {"n": 1000, "queries": BASE_QUERIES}) == directory / "order-1000.json"
    assert order_path(directory, {"n": 1000, "queries": 40}) == directory / "order-1000-q40.json"
    assert order_path(directory, {"n": 5000, "queries": 40}) == directory / "order-5000-q40.json"


def test_order_path_needs_both_parameters() -> None:
    assert order_path(Path("/tmp"), {"n": None, "queries": 300}) is None
    assert order_path(Path("/tmp"), {"n": 1000, "queries": None}) is None


def test_point_carries_the_state_and_the_warm_up_passes(tmp_path: Path) -> None:
    write_profile(tmp_path, "profile-a.json", {**RUN, "warmup_passes": 4, "warmup_pass_ms": [60.0]})
    point = build_point(tmp_path, "chaud", "profile-a.json", "four warm-up passes")

    assert point is not None
    assert point["state"] == "chaud"
    assert point["n"] == 1000
    assert point["run"]["warmup_passes"] == 4
    assert point["run"]["warmup_pass_ms"] == [60.0]
    assert point["description"] == "four warm-up passes"


def test_point_drops_the_per_query_detail_of_the_order_reading(tmp_path: Path) -> None:
    write_profile(tmp_path, "profile-a.json")
    write_order(tmp_path, "order-1000.json", 294_449, BASE_QUERIES)
    point = build_point(tmp_path, "froid", "profile-a.json", "cold")

    assert point is not None
    assert point["order"] is not None
    assert "per_query" not in point["order"]
    assert point["order"]["distinct_iids"] == 294_449


def test_point_without_order_file_keeps_a_null_order(tmp_path: Path) -> None:
    write_profile(tmp_path, "profile-a.json")
    point = build_point(tmp_path, "froid", "profile-a.json", "cold")

    assert point is not None
    assert point["order"] is None


def test_missing_run_document_is_not_a_point(tmp_path: Path) -> None:
    assert build_point(tmp_path, "froid", "absent.json", "cold") is None


def test_build_keeps_only_the_series_present(tmp_path: Path) -> None:
    write_profile(tmp_path, "profile-1000.json")
    write_order(tmp_path, "order-1000.json", 294_449, BASE_QUERIES)
    document = build(tmp_path, "0" * 40)

    sources = [point["source"] for point in document["points"]]
    assert sources == ["profile-1000.json"]
    assert document["kind"] == "MES-03-profile"
    assert document["task"] == "REDUIRE-COUT-DES-CLES"
    assert document["commit"] == "0" * 40
