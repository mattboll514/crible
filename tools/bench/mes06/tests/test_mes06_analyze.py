# SPDX-License-Identifier: Apache-2.0

"""Tests of the aggregation of MES-06 (ADR-0091).

The aggregator recomputes every percentile from the per-query durations and
refuses a document whose figure does not match the one the harness recorded.
These tests pin the percentile definition (nearest rank, no interpolation), the
extrapolation of ADR-0091 decision 1 (a range, never a single figure), the
subtraction of decision 4, and the way the campaign gathers its documents.
"""

from __future__ import annotations

import json

import pytest

import mes06_analyze as analyze


def series(count: int) -> list[float]:
    """`count` increasing values, so the nearest rank is readable by eye."""
    return [float(index + 1) for index in range(count)]


def size_document(structure: str, positions: bool, bytes_: int, passages: int) -> dict:
    return {
        "index": structure,
        "positions": positions,
        "live_passages": passages,
        "dead_passages": 0,
        "bytes": bytes_,
        "bytes_per_passage": bytes_ / passages,
        "segments": 1,
        "files": 10,
        "bytes_by_extension": {},
    }


def token_document(structure: str, passages: int, tokens: int) -> dict:
    return {
        "index": structure,
        "passages": passages,
        "tokens": tokens,
        "tokens_per_passage": tokens / passages,
        "characters": passages * 100,
        "characters_per_passage": 100.0,
        "distinct_terms": 1000,
        "longest_passage": 500,
        "shortest_passage": 4,
    }


def test_nearest_rank_takes_the_value_of_rank_ceiling_ratio_times_n():
    values = series(100)
    assert analyze.nearest_rank(values, 0.50) == 50.0
    assert analyze.nearest_rank(values, 0.95) == 95.0
    assert analyze.nearest_rank(values, 0.99) == 99.0


def test_nearest_rank_never_interpolates_between_two_samples():
    # n = 20: rank 19 of 20, no average of 19 and 20.
    values = series(20)
    assert analyze.nearest_rank(values, 0.95) == 19.0
    assert analyze.nearest_rank(values, 1.0) == 20.0


def test_nearest_rank_refuses_an_empty_series_or_a_ratio_out_of_range():
    with pytest.raises(ValueError):
        analyze.nearest_rank([], 0.5)
    with pytest.raises(ValueError):
        analyze.nearest_rank([1.0], 0.0)


def test_summarise_counts_every_sample_of_every_pass():
    summary = analyze.summarise([1.0, 2.0, 3.0, 4.0])
    assert summary["n"] == 4
    assert summary["p50"] == 2.0
    assert summary["p95"] == 4.0
    assert summary["min"] == 1.0
    assert summary["max"] == 4.0


def test_check_summary_accepts_the_figure_the_harness_recorded():
    passes = [[1.0, 2.0], [3.0, 4.0]]
    recorded = analyze.summarise([value for one_pass in passes for value in one_pass])
    analyze.check_summary(recorded, passes)


def test_check_summary_refuses_a_drifted_figure():
    passes = [[1.0, 2.0], [3.0, 4.0]]
    recorded = {"n": 4, "p50": 2.0, "p95": 99.0, "p99": 4.0}
    with pytest.raises(ValueError):
        analyze.check_summary(recorded, passes)


def test_weighted_mean_weighs_a_rate_by_its_population():
    rows = [
        {"tokens_per_passage": 10.0, "passages": 1},
        {"tokens_per_passage": 100.0, "passages": 9},
    ]
    assert analyze.weighted_mean(rows, "tokens_per_passage", "passages") == 91.0


def test_extrapolate_gives_a_range_and_a_passage_ratio():
    result = analyze.extrapolate([10.0, 30.0], target_passages=100)
    assert result["low"] == 1000.0
    assert result["central"] == 2000.0
    assert result["high"] == 3000.0
    assert result["ratio"] == 3.0


def test_extrapolate_defaults_to_the_legifacile_target():
    result = analyze.extrapolate([2.0])
    assert result["target_passages"] == 8_580_000
    assert result["central"] == 17_160_000.0


def test_budget_subtracts_every_cited_term_and_keeps_its_source():
    remainder = analyze.budget()
    assert remainder["budget_ms"] == 698.0
    assert remainder["spent_ms"] == pytest.approx(132.1872, abs=1e-4)
    assert remainder["remaining_ms"] == pytest.approx(565.8128, abs=1e-4)
    assert all(term["source"] for term in remainder["terms"])


def test_budget_filter_term_is_the_keyword_p95_of_mes14():
    term = next(
        term for term in analyze.BUDGET_TERMS if term["term"].startswith("filtre")
    )
    assert "keyword" in term["term"]
    assert "p95" in term["term"]
    assert term["ms"] == pytest.approx(0.0522)
    assert "MES-14-cout-filtres-intervalles.md:182" in term["source"]


def test_budget_rescoring_term_is_the_delivered_group_at_n_5000():
    term = next(
        term for term in analyze.BUDGET_TERMS if term["term"].startswith("rescoring")
    )
    assert "N = 5000" in term["term"]
    assert term["ms"] == pytest.approx(31.24)
    assert "MES-03-latence-rescoring.md:1418" in term["source"]


def test_sensitivity_replaces_the_filter_term_with_the_interval_case():
    line = analyze.sensitivity()
    assert line["term"]["ms"] == pytest.approx(216.9)
    assert "MES-14-cout-filtres-intervalles.md:183" in line["term"]["source"]
    assert line["spent_ms"] == pytest.approx(349.035, abs=1e-9)
    assert line["remaining_ms"] == pytest.approx(348.965, abs=1e-9)


def test_display_path_is_repository_relative_inside_the_tree():
    assert analyze.display_path(analyze.ROOT / "docs" / "x.json") == "docs/x.json"


def test_display_path_falls_back_to_the_absolute_path_outside_the_tree(tmp_path):
    # `--measures` may point outside the repository: printing the artefact must
    # not raise after the write.
    outside = tmp_path / "MES-06.json"
    shown = analyze.display_path(outside)
    assert shown == str(outside)
    assert not shown.startswith(str(analyze.ROOT))


def test_alert_verdicts_judge_every_point_of_both_campaigns():
    latency = [
        {
            "structure": "articles",
            "positions": False,
            "pool": "target-1000",
            "pool_size": 1027,
            "k": 10,
            "summary": {"p95": 837.0},
        },
        {
            "structure": "decisions",
            "positions": True,
            "pool": "target-50000",
            "pool_size": 49998,
            "k": 100,
            "summary": {"p95": 1_091.0},
        },
    ]
    synth = {
        "warm": [
            {
                "pool": "target-7740000",
                "pool_size": 7_740_000,
                "k": 10,
                "summary": {"p95": 21_617_472.0},
            }
        ],
        "cold": {
            "pool": "target-7740000",
            "pool_size": 7_740_000,
            "k": 10,
            "summary": {"p95": 22_840_205.0},
        },
    }

    rows = analyze.alert_verdicts(latency, synth, remaining_ms=565.8128)

    assert len(rows) == 4
    assert [row["verdict"] for row in rows] == [
        "sous le reste",
        "sous le reste",
        "ALERTE",
        "ALERTE",
    ]
    assert all(row["remaining_ms"] == pytest.approx(565.8128) for row in rows)
    assert rows[0]["campaign"] == "echantillon"
    assert rows[3]["campaign"].startswith("synth-large froid")


def test_verdict_alerts_only_above_the_remainder():
    remainder = analyze.budget()["remaining_ms"]
    assert analyze.verdict(remainder - 1.0, remainder) == "sous le reste"
    assert analyze.verdict(remainder + 1.0, remainder) == "ALERTE"


def test_size_and_token_rows_read_one_document_per_structure(tmp_path, monkeypatch):
    monkeypatch.setattr(analyze, "STRUCTURES", ("articles",))
    for positions in (False, True):
        name = f"size-articles-{'true' if positions else 'false'}.json"
        (tmp_path / name).write_text(
            json.dumps(size_document("articles", positions, 1_000 if positions else 500, 50)),
            encoding="utf-8",
        )
    (tmp_path / "tokens-articles-false.json").write_text(
        json.dumps(token_document("articles", 50, 5_000)), encoding="utf-8"
    )

    sizes = analyze.size_rows(tmp_path)
    tokens = analyze.token_rows(tmp_path)

    assert [row["positions"] for row in sizes] == [False, True]
    assert sizes[0]["bytes_per_passage"] == 10.0
    assert sizes[1]["bytes_per_passage"] == 20.0
    assert tokens[0]["tokens_per_passage"] == 100.0


def test_latency_pooled_gathers_the_samples_of_every_structure(tmp_path, monkeypatch):
    monkeypatch.setattr(analyze, "STRUCTURES", ("articles", "decisions"))
    for structure, values in (("articles", [1.0, 2.0]), ("decisions", [3.0, 4.0])):
        document = {
            "index": structure,
            "positions": False,
            "measured": [
                {
                    "pool": "target-1000",
                    "filter": None,
                    "pool_size": 1000,
                    "pool_millis": 1.0,
                    "k": 10,
                    "queries": len(values),
                    "passes": [values],
                    "summary": analyze.summarise(values),
                    "results_in_first_pass": 10,
                }
            ],
        }
        (tmp_path / f"latency-{structure}-false.json").write_text(
            json.dumps(document), encoding="utf-8"
        )

    pooled = analyze.latency_pooled(tmp_path)

    assert len(pooled) == 1
    assert pooled[0]["structures"] == 2
    assert pooled[0]["summary"]["n"] == 4
    assert pooled[0]["summary"]["p50"] == 2.0
