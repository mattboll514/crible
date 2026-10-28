# SPDX-License-Identifier: Apache-2.0
"""Tests of the A/B analysis of MES-03 (``REDUIRE-COUT-DES-CLES``)."""

from __future__ import annotations

import math

import pytest

from mes03_ab_analyze import (
    CONFIRMATION_NS,
    CONFIRMATION_VARIANTS,
    comparison,
    gain,
    nearest_rank,
    paired_dispersion,
    point,
    replay_verdicts,
    verdict,
)


def test_nearest_rank_uses_the_rank_from_adr_0077():
    values = [5.0, 1.0, 3.0, 2.0, 4.0]
    assert nearest_rank(values, 0.50) == 3.0
    assert nearest_rank(values, 0.95) == 5.0
    assert nearest_rank(values, 0.99) == 5.0
    assert nearest_rank(values, 0.10) == 1.0
    assert math.isnan(nearest_rank([], 0.50))


def test_paired_dispersion_is_the_largest_round_to_round_difference():
    rounds = [[10.0, 20.0, 30.0], [11.0, 24.0, 29.0]]
    amplitude, percent = paired_dispersion(rounds)
    assert amplitude == 4.0
    assert percent == pytest.approx(100.0 * 4.0 / 20.0)


def test_paired_dispersion_of_a_single_round_is_zero():
    assert paired_dispersion([[10.0, 20.0]]) == (0.0, 0.0)


def test_verdict_is_a_tie_inside_the_dispersion():
    assert verdict(-2.0, 2.0) == "nul"
    assert verdict(2.0, 2.0) == "nul"
    assert verdict(0.0, 0.0) == "nul"


def test_verdict_beyond_the_dispersion_follows_the_sign():
    assert verdict(-10.0, 4.0) == "gain"
    assert verdict(10.0, 4.0) == "perte"


def document(n, passes, durations):
    """A minimal ``rescore_ab`` document, with one entry per pass."""
    return {
        "n": n,
        "queries": 2,
        "candidates_min": n,
        "candidates_max": n,
        "distinct_iids": 10,
        "k": 100,
        "limit": 100,
        "per_pass": [
            {"p50": value, "p95": value} for value in passes
        ],
        "durations_ms": durations,
    }


def test_point_keeps_every_pass_of_every_round():
    first = document(1000, [10.0, 8.0], [[10.0, 10.0], [8.0, 8.0]])
    second = document(1000, [9.0, 7.0], [[9.0, 9.0], [7.0, 7.0]])
    summary = point([first, second])
    assert [entry["p50"] for entry in summary["passes"]] == [10.0, 8.0, 9.0, 7.0]
    assert summary["n_samples"] == 8
    # Nearest rank on 8 samples: ceil(0.50 × 8) = 4, one-based, so the fourth
    # value of the sorted list is the p50.
    assert summary["p50"] == 8.0
    assert summary["dispersion_ms"] == 1.0


def test_gain_is_relative_and_signed():
    assert gain({"p50": 100.0}, {"p50": 50.0}, "p50") == pytest.approx(-50.0)
    assert gain({"p50": 100.0}, {"p50": 120.0}, "p50") == pytest.approx(20.0)


def test_comparison_applies_the_noise_rule_of_the_candidate():
    reference = {"p50": 100.0, "p95": 200.0, "dispersion_percent": 1.0}
    candidate = {"p50": 10.0, "p95": 120.0, "dispersion_percent": 3.0}
    entry = comparison(reference, candidate)
    assert entry["delta_percent"] == pytest.approx(-90.0)
    assert entry["delta_percent_p95"] == pytest.approx(-40.0)
    assert entry["p50_reference"] == 100.0
    assert entry["p95_reference"] == 200.0
    assert entry["dispersion_percent"] == 3.0
    assert entry["verdict"] == "gain"


def test_comparison_calls_a_small_difference_a_tie():
    reference = {"p50": 100.0, "p95": 200.0, "dispersion_percent": 1.0}
    candidate = {"p50": 101.0, "p95": 205.0, "dispersion_percent": 3.0}
    assert comparison(reference, candidate)["verdict"] == "nul"


def test_replay_applies_the_noise_rule_of_adr_0082():
    # A large negative delta that stays inside the replay point's own
    # dispersion (the cold first pass widens it) is a tie, not a gain.
    entry = {"gain_percent": -80.0, "gain_percent_p95": -50.0, "dispersion_percent": 90.0}
    replay_verdicts(entry)
    assert entry["verdict"] == "nul"
    assert "verdict_p95" not in entry


def test_replay_beyond_its_dispersion_is_a_gain():
    entry = {"gain_percent": -70.0, "gain_percent_p95": -64.0, "dispersion_percent": 11.0}
    replay_verdicts(entry)
    assert entry["verdict"] == "gain"
    assert "verdict_p95" not in entry


def test_confirmation_series_covers_the_delivered_variant_against_base_and_final():
    assert CONFIRMATION_VARIANTS == ("base", "final", "b-only")
    assert CONFIRMATION_NS == (1000, 2000, 5000)
