# SPDX-License-Identifier: Apache-2.0

"""Tests of the aggregation of MES-10 and MES-11 (ADR-0092).

The aggregator recomputes recall@10, MRR@10, the median rank and the paired
bootstrap from the raw documents of the harness. These tests pin the metric
definitions of decision 2, the sign convention and the reproducibility of the
bootstrap of decision 6, the decision rule (lower bound, secondary set,
guardrail corpora, tie-breaks), and the reading of `queries.jsonl`.
"""

from __future__ import annotations

import json

import pytest

import aggregate


def question(identifier: str, expected: list[str], top: list[str]) -> dict:
    """One row as the aggregator reads it, with a single method."""
    return {"id": identifier, "expected": expected, "top": {"rrf": top}}


def test_question_scores_counts_the_share_of_expected_documents_found():
    recall, inverse, rank = aggregate.question_scores(
        ["a", "b"], ["x", "a", "b"]
    )
    assert recall == 1.0
    assert inverse == 0.5
    assert rank == 2


def test_question_scores_uses_the_first_expected_document_for_the_rank():
    recall, inverse, rank = aggregate.question_scores(
        ["a", "b"], ["x", "b", "a"]
    )
    assert recall == 1.0
    assert rank == 2
    assert inverse == 0.5


def test_question_scores_returns_zero_and_no_rank_when_nothing_is_found():
    recall, inverse, rank = aggregate.question_scores(["a"], ["x", "y"])
    assert recall == 0.0
    assert inverse == 0.0
    assert rank is None


def test_question_scores_ignores_an_empty_expected_list():
    recall, inverse, rank = aggregate.question_scores([], ["x"])
    assert (recall, inverse, rank) == (0.0, 0.0, None)


def test_summarise_averages_the_recall_and_the_reciprocal_rank():
    rows = [
        question("Q1", ["a"], ["a"]),
        question("Q2", ["a"], ["x", "a"]),
        question("Q3", ["a"], ["x"]),
    ]
    summary = aggregate.summarise(rows, "rrf")
    assert summary["n"] == 3
    assert summary["recall"] == pytest.approx(2 / 3)
    assert summary["mrr"] == pytest.approx((1.0 + 0.5) / 3)
    assert summary["found"] == 2
    assert summary["median_rank"] == 1.5


def test_bootstrap_is_reproducible_with_its_fixed_seed():
    reference = [1.0, 0.0, 1.0, 0.0]
    other = [0.0, 0.0, 1.0, 0.0]
    first = aggregate.bootstrap(reference, other, resamples=500, seed=7)
    second = aggregate.bootstrap(reference, other, resamples=500, seed=7)
    assert first == second


def test_bootstrap_computes_other_minus_reference():
    interval = aggregate.bootstrap(
        [1.0] * 20, [0.0] * 20, resamples=200, seed=7
    )
    assert interval["difference"] == pytest.approx(1.0)
    assert interval["low"] > 0.0


def test_bootstrap_negative_when_the_other_method_is_worse():
    interval = aggregate.bootstrap(
        [0.0] * 20, [1.0] * 20, resamples=200, seed=7
    )
    assert interval["difference"] == pytest.approx(-1.0)
    assert interval["high"] < 0.0


def test_bootstrap_refuses_two_series_of_different_lengths():
    with pytest.raises(ValueError):
        aggregate.bootstrap([1.0], [1.0, 0.0], resamples=10, seed=7)


def test_alpha_of_reads_the_harness_names():
    assert aggregate.alpha_of("relative_0") == 0.0
    assert aggregate.alpha_of("relative_0.3") == 0.3
    assert aggregate.alpha_of("relative_0.75") == 0.75
    assert aggregate.alpha_of("relative_1") == 1.0
    assert aggregate.alpha_of("dense") == 0.0


def test_no_detectable_regression_requires_an_upper_bound_at_or_above_zero():
    assert aggregate.no_detectable_regression({"low": -0.2, "high": 0.1})
    assert aggregate.no_detectable_regression({"low": 0.01, "high": 0.2})
    assert aggregate.no_detectable_regression({"low": -0.2, "high": 0.0})
    assert not aggregate.no_detectable_regression({"low": -1.0, "high": -0.5})


def test_decide_keeps_the_reference_when_no_lower_bound_is_above_zero():
    methods = {
        "rrf": {"recall": 0.9, "mrr": 0.7},
        "relative_0.3": {"recall": 0.95, "mrr": 0.8},
    }
    intervals = {"relative_0.3": {"low": 0.0, "high": 0.1}}
    decision = aggregate.decide(methods, intervals)
    assert decision["recommended"] == "rrf"
    assert decision["rejected"][0]["method"] == "relative_0.3"


def test_decide_never_names_a_method_that_is_worse():
    methods = {
        "rrf": {"recall": 0.9, "mrr": 0.7},
        "relative_0.3": {"recall": 0.8, "mrr": 0.6},
    }
    intervals = {"relative_0.3": {"low": -0.1, "high": -0.01}}
    decision = aggregate.decide(methods, intervals)
    assert decision["recommended"] == "rrf"
    assert decision["rejected"][0]["method"] == "relative_0.3"


def test_decide_names_a_method_whose_lower_bound_is_above_zero():
    methods = {
        "rrf": {"recall": 0.9, "mrr": 0.7},
        "relative_0.3": {"recall": 0.95, "mrr": 0.8},
    }
    intervals = {"relative_0.3": {"low": 0.01, "high": 0.1}}
    decision = aggregate.decide(methods, intervals)
    assert decision["recommended"] == "relative_0.3"


def test_decide_rejects_a_gain_that_regresses_on_the_secondary_set():
    methods = {
        "rrf": {"recall": 0.9, "mrr": 0.7},
        "relative_0.3": {"recall": 0.95, "mrr": 0.8},
    }
    intervals = {"relative_0.3": {"low": 0.01, "high": 0.1}}
    secondary = {"relative_0.3": {"low": -0.1, "high": -0.02}}
    decision = aggregate.decide(methods, intervals, secondary=secondary)
    assert decision["recommended"] == "rrf"
    assert decision["rejected"][0]["reason"] == "regression on the secondary set"


def test_decide_accepts_a_gain_that_also_gains_on_the_secondary_set():
    methods = {
        "rrf": {"recall": 0.9, "mrr": 0.7},
        "relative_0.3": {"recall": 0.95, "mrr": 0.8},
    }
    intervals = {"relative_0.3": {"low": 0.01, "high": 0.1}}
    secondary = {"relative_0.3": {"low": 0.01, "high": 0.05}}
    decision = aggregate.decide(methods, intervals, secondary=secondary)
    assert decision["recommended"] == "relative_0.3"


def test_decide_rejects_a_gain_that_loses_on_a_guardrail_corpus():
    methods = {
        "rrf": {"recall": 0.9, "mrr": 0.7},
        "relative_0.3": {"recall": 0.95, "mrr": 0.8},
    }
    intervals = {"relative_0.3": {"low": 0.01, "high": 0.1}}
    secondary = {"relative_0.3": {"low": 0.01, "high": 0.05}}
    guardrails = {
        "relative_0.3": {
            "juri_texte_integral": {"low": -0.2, "high": -0.01},
            "textes_vigueur": {"low": -0.1, "high": 0.1},
        }
    }
    decision = aggregate.decide(
        methods, intervals, secondary=secondary, guardrails=guardrails
    )
    assert decision["recommended"] == "rrf"
    assert (
        decision["rejected"][0]["reason"]
        == "regression beyond noise on a guardrail corpus"
    )


def test_decide_breaks_a_tie_on_mrr_then_on_the_alpha_closer_to_one_half():
    methods = {
        "rrf": {"recall": 0.9, "mrr": 0.7},
        "relative_0.75": {"recall": 0.95, "mrr": 0.85},
        "relative_0.9": {"recall": 0.95, "mrr": 0.85},
    }
    intervals = {
        "relative_0.75": {"low": 0.01, "high": 0.1},
        "relative_0.9": {"low": 0.01, "high": 0.1},
    }
    assert aggregate.decide(methods, intervals)["recommended"] == "relative_0.75"


def test_decide_compares_the_mes_11_variants_against_v0():
    methods = {
        "V0": {"recall": 0.9, "mrr": 0.7},
        "V2": {"recall": 0.95, "mrr": 0.8},
    }
    intervals = {"V2": {"low": 0.02, "high": 0.1}}
    decision = aggregate.decide(methods, intervals, reference="V0")
    assert decision["recommended"] == "V2"


def test_load_questions_keeps_the_labelled_flag_and_the_expected_ids(tmp_path):
    path = tmp_path / "queries.jsonl"
    rows = [
        {
            "id": "Q01",
            "corpus": "textes_vigueur",
            "structure": "articles",
            "satisfiable": True,
            "expected": [{"src_id": "LEGIARTI1"}],
            "expected_in_sample": False,
        },
        {
            "id": "R01",
            "corpus": "dole",
            "structure": "dole",
            "satisfiable": True,
            "expected": [{"src_id": "JURITEXT1"}],
            "expected_in_sample": True,
        },
    ]
    path.write_text(
        "\n".join(json.dumps(row) for row in rows) + "\n", encoding="utf-8"
    )
    questions = aggregate.load_questions(path)
    assert questions["Q01"]["labelled"] is True
    assert questions["Q01"]["expected"] == ["LEGIARTI1"]
    assert questions["R01"]["labelled"] is False
    assert questions["R01"]["expected_in_sample"] is True


def test_load_questions_skips_blank_lines(tmp_path):
    path = tmp_path / "queries.jsonl"
    path.write_text('{"id": "Q01", "expected": []}\n\n', encoding="utf-8")
    assert list(aggregate.load_questions(path)) == ["Q01"]


def test_markdown_table_formats_floats_and_missing_values():
    table = aggregate.markdown_table(
        [{"name": "rrf", "value": 0.5, "rank": None}],
        [("name", "Mode"), ("value", "Valeur"), ("rank", "Rang")],
    )
    assert table.splitlines()[0] == "| Mode | Valeur | Rang |"
    assert table.splitlines()[2] == "| rrf | 0.5000 | — |"


def test_the_reports_use_the_two_slugs_of_decision_7():
    assert aggregate.SLUG_10 == "MES-10-qualite-fusion"
    assert aggregate.SLUG_11 == "MES-11-analyse-francaise"


def test_the_six_structures_are_the_ones_of_the_sample():
    assert aggregate.STRUCTURES == (
        "articles",
        "decisions",
        "jorf",
        "sarde",
        "dole",
        "titres",
    )


def test_the_question_excluded_from_every_mean_is_q30():
    assert aggregate.UNSATISFIABLE == "Q30"
