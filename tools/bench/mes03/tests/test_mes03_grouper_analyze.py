# SPDX-License-Identifier: Apache-2.0
"""Tests of the campaign reducer of MES-03 (``mes03_grouper_analyze.py``)."""

from __future__ import annotations

import json
from pathlib import Path

from mes03_grouper_analyze import gap, noise_verdict, point, series


def write_document(
    raw: Path, name: str, p50: float, p95: float, passes: int = 3
) -> None:
    """Writes a minimal ``rescore_ab`` document, with only the read fields."""
    document = {
        "summary": {"p50": p50, "p95": p95, "p99": p95 * 1.2, "n": 300 * passes},
        "per_pass": [
            {"p50": p50, "p95": p95, "p99": p95 * 1.2} for _ in range(passes)
        ],
    }
    (raw / name).write_text(json.dumps(document), encoding="utf-8")


def test_a_variant_faster_than_base_has_a_negative_gap():
    assert gap(10.0, 2.0) == -80.0
    assert gap(10.0, 12.0) == 20.0


def test_a_gap_inside_its_own_amplitude_is_within_noise():
    verdict = noise_verdict([-1.0, 2.0])
    assert verdict["amplitude"] == 3.0
    assert verdict["verdict"] == "within_noise"


def test_a_gap_beyond_its_own_amplitude_keeps_its_direction():
    assert noise_verdict([-80.0, -82.0])["verdict"] == "faster"
    assert noise_verdict([+6.0, +9.0])["verdict"] == "slower"


def test_a_single_round_declares_the_amplitude_unknown():
    verdict = noise_verdict([-58.0])
    assert verdict["amplitude"] is None
    assert verdict["verdict"] == "single_round"


def test_a_point_compares_every_variant_against_base_of_the_same_round(tmp_path: Path):
    write_document(tmp_path, "screen-r1-base-1000.json", 10.0, 20.0)
    write_document(tmp_path, "screen-r2-base-1000.json", 8.0, 16.0)
    write_document(tmp_path, "screen-r1-grouped-1000.json", 1.0, 2.0)
    write_document(tmp_path, "screen-r2-grouped-1000.json", 0.8, 1.6)

    reduced = point(tmp_path, "screen", (1, 2), 1000, ("base", "grouped"))

    gaps = reduced["against_base"]["grouped"]["gaps"]
    assert gaps["gaps"] == [-90.0, -90.0]
    assert gaps["verdict"] == "faster"
    assert reduced["measured"]["base"]["r1"]["samples"] == 900


def test_a_missing_document_is_reported_and_never_replaced(tmp_path: Path):
    write_document(tmp_path, "screen-r1-base-1000.json", 10.0, 20.0)
    write_document(tmp_path, "screen-r1-grouped-1000.json", 1.0, 2.0)

    reduced = series(tmp_path, "screen", (1, 2), (1000,), ("base", "grouped"))

    assert reduced["missing"] == [
        "screen-r2-base-1000.json",
        "screen-r2-grouped-1000.json",
    ]
    # Only one round is measured, so the point keeps its single gap and says so.
    assert reduced["points"][0]["against_base"]["grouped"]["gaps"]["verdict"] == (
        "single_round"
    )
