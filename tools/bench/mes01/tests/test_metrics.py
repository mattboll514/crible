# SPDX-License-Identifier: Apache-2.0

"""Tests of the MES-01 / MES-12 metric definitions (ADR-0076, decision 1)."""

from __future__ import annotations

import pytest

import mes01_metrics as metrics


def record(pairs: list[tuple[str, str]]) -> dict:
    """Builds one result list from `(key, doc_id)` pairs."""
    return {
        "results": [
            {"key": key, "doc_id": doc_id, "ord": 0, "score": 1.0} for key, doc_id in pairs
        ]
    }


def reference(count: int = 10) -> dict:
    """Builds an exact top-10 of distinct documents."""
    return record([(f"doc{index}#0", f"doc{index}") for index in range(count)])


def test_recall_is_the_shared_passages_over_ten():
    exact = reference()
    binary = record([(f"doc{index}#0", f"doc{index}") for index in range(7)] + [
        ("other1#0", "other1"),
        ("other2#0", "other2"),
        ("other3#0", "other3"),
    ])
    assert metrics.recall_at_10(binary, exact) == pytest.approx(0.7)


def test_recall_counts_passages_not_documents():
    # Two passages of the same document are two hits at the passage level.
    exact = record([("a#0", "a"), ("a#1", "a"), ("b#0", "b")] + [
        (f"doc{index}#0", f"doc{index}") for index in range(7)
    ])
    binary = record([("a#0", "a"), ("a#1", "a")] + [
        (f"other{index}#0", f"other{index}") for index in range(8)
    ])
    assert metrics.recall_at_10(binary, exact) == pytest.approx(0.2)


def test_recall_refuses_a_short_exact_reference():
    # The denominator of ADR-0076 is the ten exact passages; a short reference
    # would silently divide by another number.
    with pytest.raises(ValueError):
        metrics.recall_at_10(reference(), reference(9))


def test_overlap_uses_distinct_documents_of_the_exact_top_ten():
    exact = record([("a#0", "a"), ("a#1", "a"), ("b#0", "b")] + [
        (f"doc{index}#0", f"doc{index}") for index in range(7)
    ])
    # The exact top-10 holds 9 distinct documents: a, b and doc0…doc6.
    binary = record([("a#7", "a"), ("b#3", "b"), ("zzz#0", "zzz")] + [
        (f"other{index}#0", f"other{index}") for index in range(7)
    ])
    assert metrics.overlap_at_10(binary, exact) == pytest.approx(2 / 9)


def test_overlap_rewards_a_second_passage_of_a_found_document():
    exact = record([("a#0", "a"), ("b#0", "b")] + [
        (f"doc{index}#0", f"doc{index}") for index in range(8)
    ])
    binary = record([("a#0", "a"), ("a#1", "a"), ("a#2", "a")] + [
        (f"other{index}#0", f"other{index}") for index in range(7)
    ])
    assert metrics.overlap_at_10(binary, exact) == pytest.approx(1 / 10)


def test_mean_and_summarise_report_the_query_count_and_the_perfect_ones():
    rows = [{"recall": 1.0}, {"recall": 0.5}, {"recall": 1.0}]
    summary = metrics.summarise(rows, "recall")
    assert summary == {"queries": 3, "mean": pytest.approx(5 / 6), "min": 0.5, "perfect": 2}
    assert metrics.mean([2.0, 4.0]) == pytest.approx(3.0)


def test_mean_of_nothing_is_refused():
    with pytest.raises(ValueError):
        metrics.mean([])
