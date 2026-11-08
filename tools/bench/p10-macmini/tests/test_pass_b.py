# SPDX-License-Identifier: Apache-2.0
"""Tests of the pure helpers of pass B of the Mac mini campaign."""

import pytest

import macmini


def test_titles_game_splits_by_index_and_filters_articles() -> None:
    rows = [
        {"query_id": "q1", "corpus": "textes_vigueur", "text": "titre vivant"},
        {"query_id": "q2", "corpus": "textes_hors_vigueur", "text": "titre mort"},
        {"query_id": "q3", "corpus": "juri_sommaires", "text": "sommaire"},
        {"query_id": "q4", "corpus": "dole", "text": "dossier"},
    ]
    games = macmini.titles_game(rows)
    assert sorted(games) == ["articles", "decisions", "dole"]
    assert [row["id"] for row in games["articles"]] == ["q1", "q2"]
    assert games["articles"][0]["filter"] == {
        "etat": {"in": ["VIGUEUR", "VIGUEUR_DIFF", "VIGUEUR_ETEN", "VIGUEUR_NON_ETEN"]}
    }
    assert games["articles"][1]["filter"] == {
        "$or": [
            {"etat": {"nin": ["VIGUEUR", "VIGUEUR_DIFF", "VIGUEUR_ETEN", "VIGUEUR_NON_ETEN"]}},
            {"etat": {"exists": False}},
        ]
    }
    assert "filter" not in games["decisions"][0]
    assert "filter" not in games["dole"][0]


def test_titles_game_refuses_unknown_corpus_and_empty_text() -> None:
    with pytest.raises(macmini.MacMiniError):
        macmini.titles_game([{"query_id": "q1", "corpus": "inconnu", "text": "x"}])
    with pytest.raises(macmini.MacMiniError):
        macmini.titles_game([{"query_id": "q1", "corpus": "dole", "text": "  "}])


def test_titles_game_refuses_repeated_identifier() -> None:
    rows = [
        {"query_id": "q1", "corpus": "dole", "text": "a"},
        {"query_id": "q1", "corpus": "sarde", "text": "b"},
    ]
    with pytest.raises(macmini.MacMiniError):
        macmini.titles_game(rows)


def test_percentile_uses_the_nearest_rank() -> None:
    samples = list(range(1, 101))
    assert macmini.percentile(samples, 50.0) == 50
    assert macmini.percentile(samples, 95.0) == 95
    assert macmini.percentile(samples, 99.0) == 99
    assert macmini.percentile([], 95.0) is None
    assert macmini.percentile([3.0], 95.0) == 3.0


def test_summarise_report_reads_latency_and_encoding_share() -> None:
    report = {
        "index": "dole",
        "generation": 1,
        "queries": 3,
        "repeat": 3,
        "exact_threshold": 5000,
        "rescore_candidates": 5000,
        "modes": {
            "dense": {
                "latency": {
                    "latency_basis": "server_stages",
                    "total": {"p50": 100.0, "p95": 200.0, "p99": 300.0},
                    "encode": {"p50": 10.0, "p95": 20.0, "p99": 30.0},
                }
            }
        },
    }
    summary = macmini.summarise_report(report, "dense-texte")
    assert summary["samples"] == 9
    assert summary["total"] == {"p50": 100.0, "p95": 200.0, "p99": 300.0}
    assert summary["encode_share_at_p95"] == 0.1


def test_grouped_percentiles_refuse_to_invent_a_grouped_value() -> None:
    summaries = [
        {"index": "a", "samples": 6, "total": {"p50": 10.0, "p95": 20.0, "p99": 30.0}},
        {"index": "b", "samples": 9, "total": {"p50": 5.0, "p95": 50.0, "p99": 60.0}},
    ]
    grouped = macmini.grouped_percentiles(summaries)
    assert grouped["available"] is False
    assert grouped["samples"] == 15
    assert grouped["p95_bound_max_index"] == 50.0
    assert grouped["p50_range"] == [5.0, 10.0]
    assert grouped["per_index_p95"] == {"a": 20.0, "b": 50.0}


def test_compare_top10_reports_identical_queries() -> None:
    reference = {"Q1": ["a", "b"], "Q2": ["c"]}
    candidate = {"Q1": ["a", "b"], "Q2": ["d"], "Q3": ["e"]}
    result = macmini.compare_top10(reference, candidate)
    assert result["compared"] == 2
    assert result["identical"] == 1
    assert result["differing"] == ["Q2"]
    assert result["extra"] == ["Q3"]


def test_compare_eval_queries_separates_fidelity_and_quality() -> None:
    reference = {
        "Q1": {
            "fidelity_overlap_at_10": 1.0,
            "fidelity_recall_at_10": 1.0,
            "quality_found": [0],
            "quality_mrr_at_k": 0.5,
            "quality_recall_at_k": 1.0,
        },
        "Q2": {
            "fidelity_overlap_at_10": 0.9,
            "fidelity_recall_at_10": 0.8,
            "quality_found": [],
            "quality_mrr_at_k": 0.0,
            "quality_recall_at_k": 0.0,
        },
    }
    candidate = {
        "Q1": dict(reference["Q1"]),
        "Q2": {
            **reference["Q2"],
            "quality_found": [1],
            "quality_mrr_at_k": 0.25,
            "quality_recall_at_k": 0.5,
        },
        "Q3": dict(reference["Q1"]),
    }
    result = macmini.compare_eval_queries(reference, candidate)
    assert result["compared"] == 2
    assert result["fidelity_identical"] == 2
    assert result["fidelity_differing"] == []
    assert result["quality_identical"] == 1
    assert result["quality_differing"] == ["Q2"]
    assert result["extra"] == ["Q3"]


def test_populations_carry_the_expected_sizes() -> None:
    names = macmini.populations()
    assert len(names["global"]) == 56
    assert len(names["juri_texte_integral"]) == 31
    assert len(names["textes_vigueur"]) == 12
    assert "Q30" not in names["global"]


def test_recall_by_population_reads_each_query_once() -> None:
    reports = {
        "articles": {
            "modes": {
                "dense": {
                    "queries": [
                        {"id": "Q01", "quality_recall_at_k": 1.0},
                        {"id": "Q02", "quality_recall_at_k": 0.0},
                    ]
                }
            }
        }
    }
    result = macmini.recall_by_population(
        reports, {"textes_vigueur": ["Q01", "Q02"]}
    )
    assert result["textes_vigueur"] == {"n": 2, "recall_at_10": 0.5}


def test_recall_by_population_refuses_a_missing_query() -> None:
    with pytest.raises(macmini.MacMiniError):
        macmini.recall_by_population(
            {"articles": {"modes": {"dense": {"queries": []}}}},
            {"textes_vigueur": ["Q01"]},
        )
