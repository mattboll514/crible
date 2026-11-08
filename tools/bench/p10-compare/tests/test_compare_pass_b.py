import json
from pathlib import Path

import pytest

import pass_b


def write_jsonl(path: Path, rows):
    path.write_text(
        "".join(json.dumps(row, ensure_ascii=False) + "\n" for row in rows),
        encoding="utf-8",
    )


def test_populations_match_the_reference_game():
    pass_b.check_populations()
    assert len(pass_b.base_ids()) == 57
    assert len(pass_b.satisfiable_ids()) == 56
    assert pass_b.UNSATISFIABLE == "Q30"
    assert pass_b.UNSATISFIABLE not in pass_b.satisfiable_ids()
    assert len(pass_b.corpus_ids("juri_texte_integral")) == 31
    assert len(pass_b.corpus_ids("textes_vigueur")) == 12
    # Q30 is in textes_vigueur but counted apart.
    assert len(pass_b.corpus_ids("textes_vigueur", include_unsatisfiable=True)) == 13
    assert pass_b.question_corpus("Q13") == "juri_sommaires"
    assert pass_b.question_corpus("MES00-q0001", "sarde") == "sarde"
    assert pass_b.is_base_question("Q58")
    assert not pass_b.is_base_question("Q58-VARIANTE")
    assert not pass_b.is_base_question("MES00-q0001")


def reference_directory(tmp_path: Path):
    directory = tmp_path / "reference"
    directory.mkdir()
    for index in ("articles", "decisions", "dole", "jorf", "sarde", "titres"):
        write_jsonl(
            directory / f"{index}.jsonl",
            [
                {
                    "id": f"{index.upper()}-1",
                    "query": f"Question {index}",
                    "filter": {"etat": {"in": ["VIGUEUR"]}},
                    "expected": [f"{index.upper()}DOC"],
                }
            ],
        )
    return directory


def test_load_reference_reads_the_six_files(tmp_path):
    games = pass_b.load_reference(reference_directory(tmp_path))
    assert sorted(games) == ["articles", "decisions", "dole", "jorf", "sarde", "titres"]
    assert games["dole"][0]["id"] == "DOLE-1"


def test_load_reference_rejects_unknown_keys_and_duplicates(tmp_path):
    directory = reference_directory(tmp_path)
    write_jsonl(
        directory / "dole.jsonl",
        [{"id": "X", "query": "q", "extra": 1}],
    )
    with pytest.raises(pass_b.PassBError, match="unknown keys"):
        pass_b.load_reference(directory)

    write_jsonl(
        directory / "dole.jsonl",
        [{"id": "X", "query": "q"}, {"id": "X", "query": "q"}],
    )
    with pytest.raises(pass_b.PassBError, match="duplicate id"):
        pass_b.load_reference(directory)


def test_text_game_keeps_filter_and_expected():
    rows = [
        {
            "id": "Q01",
            "query": "Question",
            "filter": {"etat": {"in": ["VIGUEUR"]}},
            "expected": ["A"],
        }
    ]
    built = pass_b.text_game(rows)
    assert built == [
        {"id": "Q01", "query": "Question", "filter": {"etat": {"in": ["VIGUEUR"]}}, "expected": ["A"]}
    ]
    with pytest.raises(pass_b.PassBError, match="no query text"):
        pass_b.text_game([{"id": "Q02", "vector": [1.0]}])


def test_vector_game_replaces_the_text_and_refuses_a_missing_vector():
    rows = [{"id": "Q01", "query": "Question", "expected": ["A"]}]
    built = pass_b.vector_game(rows, {"Q01": [0.1, 0.2]})
    assert built == [{"id": "Q01", "vector": [0.1, 0.2], "expected": ["A"]}]
    assert "query" not in built[0]
    with pytest.raises(pass_b.PassBError, match="no query vector"):
        pass_b.vector_game([{"id": "Q02", "query": "Question"}], {})


def test_write_jsonl_is_compact_and_hashes(tmp_path):
    path = tmp_path / "game.jsonl"
    digest = pass_b.write_jsonl(path, [{"id": "Q01", "vector": [1.0]}])
    assert len(digest) == 64
    assert path.read_text(encoding="utf-8") == '{"id":"Q01","vector":[1.0]}\n'


def test_load_vectors_reads_the_pass_a_file(tmp_path):
    path = tmp_path / "vectors.json"
    path.write_text(
        json.dumps({"queries": [{"id": "Q01", "vector": [1, 2]}]}),
        encoding="utf-8",
    )
    assert pass_b.load_vectors(path) == {"Q01": [1.0, 2.0]}
    path.write_text(json.dumps({"queries": [{"id": "Q01"}]}), encoding="utf-8")
    with pytest.raises(pass_b.PassBError, match="malformed vector"):
        pass_b.load_vectors(path)


def report(mode: str, rows, *, strategies=None, fidelity=None, latency=None):
    return {
        "modes": {
            mode: {
                "dense_strategy": strategies or {"binary": len(rows)},
                "fidelity": fidelity,
                "queries": rows,
                "latency": latency or {"total": {"p50": 1.0, "p95": 2.0, "p99": 3.0}},
            }
        }
    }


def test_quality_rows_and_summary():
    rows = [
        {"id": "Q01", "quality_recall_at_k": 1.0, "quality_mrr_at_k": 0.5, "quality_found": [0]},
        {"id": "Q02", "quality_recall_at_k": 0.0, "quality_mrr_at_k": 0.0, "quality_found": []},
    ]
    data = report("dense", rows)
    indexed = pass_b.quality_rows(data, "dense-primaire")
    summary = pass_b.quality_summary(indexed, ["Q01", "Q02"])
    assert summary == {"n": 2, "recall_at_10": 0.5, "mrr_at_10": 0.25}
    with pytest.raises(pass_b.PassBError, match="missing report rows"):
        pass_b.quality_summary(indexed, ["Q03"])


def test_quality_rows_refuse_an_absent_mode():
    with pytest.raises(pass_b.PassBError, match="no mode"):
        pass_b.quality_rows({"modes": {}}, "dense-primaire")


def test_found_expected_uses_the_indices_of_expected():
    row = {"quality_found": [0, 2]}
    assert pass_b.found_expected(row, ["A", "B", "C"]) == ["A", "C"]
    with pytest.raises(pass_b.PassBError, match="outside expected"):
        pass_b.found_expected({"quality_found": [3]}, ["A"])


def test_pgvector_rows_compute_recall_and_mrr_at_document():
    replay = {
        "measurements": [
            {
                "id": "Q01",
                "results": [
                    {"doc_id": "B"},
                    {"doc_id": "A"},
                    {"doc_id": "C"},
                ],
            }
        ]
    }
    rows = pass_b.pgvector_rows(replay, {"Q01": ["A", "C"]})
    assert rows["Q01"]["quality_recall_at_k"] == 1.0
    assert rows["Q01"]["quality_mrr_at_k"] == 0.5
    assert rows["Q01"]["doc_ids"] == ["B", "A", "C"]


def test_pgvector_rows_miss_an_absent_expected():
    replay = {"measurements": [{"id": "Q01", "results": [{"doc_id": "X"}]}]}
    rows = pass_b.pgvector_rows(replay, {"Q01": ["A"]})
    assert rows["Q01"]["quality_recall_at_k"] == 0.0
    assert rows["Q01"]["quality_mrr_at_k"] == 0.0


def test_compare_rankings_separates_losses_from_gains():
    primary = {
        "Q01": {"quality_recall_at_k": 1.0, "quality_mrr_at_k": 1.0, "quality_found": [0]},
        "Q02": {"quality_recall_at_k": 1.0, "quality_mrr_at_k": 1.0, "quality_found": [0]},
    }
    pgvector = {
        "Q01": {"quality_recall_at_k": 0.0, "quality_mrr_at_k": 0.0, "doc_ids": ["X"]},
        "Q02": {"quality_recall_at_k": 1.0, "quality_mrr_at_k": 1.0, "doc_ids": ["A"]},
    }
    expected = {"Q01": ["A"], "Q02": ["A"]}
    comparison = pass_b.compare_rankings(primary, pgvector, ["Q01", "Q02"], expected)
    assert comparison["lost"] == []
    assert comparison["gained"] == [{"id": "Q01", "doc_id": "A"}]
    assert comparison["gained_queries"] == 1


def test_compare_rankings_lists_a_loss():
    primary = {
        "Q01": {"quality_recall_at_k": 0.0, "quality_mrr_at_k": 0.0, "quality_found": []}
    }
    pgvector = {
        "Q01": {"quality_recall_at_k": 1.0, "quality_mrr_at_k": 1.0, "doc_ids": ["A"]}
    }
    comparison = pass_b.compare_rankings(
        primary, pgvector, ["Q01"], {"Q01": ["A"]}
    )
    assert comparison["lost"] == [{"id": "Q01", "doc_id": "A"}]
    assert comparison["lost_queries"] == 1


def test_missing_documents_asks_about_expected_and_results():
    expected = {"Q01": ["A", "B"]}
    pgvector = {"Q01": {"doc_ids": ["A", "C"]}}
    seen = set()

    def exists(index, doc_id):
        seen.add((index, doc_id))
        return doc_id not in ("B", "C")

    gaps = pass_b.missing_documents(
        expected, pgvector, exists, {"Q01": "decisions"}
    )
    assert gaps["expected"] == [{"id": "Q01", "doc_id": "B", "index": "decisions"}]
    assert gaps["results"] == [{"id": "Q01", "doc_id": "C", "index": "decisions"}]
    assert seen == {("decisions", "A"), ("decisions", "B"), ("decisions", "C")}


def test_fidelity_summary_checks_the_binary_strategy():
    data = report(
        "dense",
        [{"id": "Q01"}],
        strategies={"binary": 1},
        fidelity={"n": 1, "recall_at_10": 0.99, "overlap_at_10": 1.0},
    )
    data["exact_threshold"] = 0
    data["rescore_candidates"] = 5000
    summary = pass_b.fidelity_summary(data, 1)
    assert summary["recall_at_10"] == 0.99
    assert summary["dense_strategy"] == {"binary": 1}
    assert summary["exact_threshold"] == 0

    data["modes"]["dense"]["dense_strategy"] = {"exact": 1}
    with pytest.raises(pass_b.PassBError, match="not binary"):
        pass_b.fidelity_summary(data, 1)


def test_latency_summary_keeps_percentiles_per_stage():
    data = report(
        "dense",
        [{"id": "Q01"}],
        latency={
            "total": {"p50": 1.0, "p95": 2.0, "p99": 3.0},
            "dense": {"p50": 0.5, "p95": 0.9, "p99": 1.0},
            "latency_basis": "server_stages",
        },
    )
    summary = pass_b.latency_summary(data, "dense-primaire")
    assert summary["total"] == {"p50": 1.0, "p95": 2.0, "p99": 3.0}
    assert "latency_basis" not in summary


def test_parse_inspect_reads_counts_and_ignores_the_rest():
    text = """index dole
  epoch: 1
  generation: 1
  documents: 3600
  passages: 7653 live, 0 dead
  dead ratio: 0.0000
  segments: 1
    seg-dole-full-v1: 7653 vectors
  bitmaps: 139433 bytes (live 8208, keywords 22973, booleans 0, columns 108252)
  codes: 734688 bytes
index sarde
  documents: 210965
  passages: 214578 live, 12 dead
"""
    counts = pass_b.parse_inspect(text)
    assert counts["dole"] == {"documents": 3600, "passages": 7653, "dead": 0}
    assert counts["sarde"] == {"documents": 210965, "passages": 214578, "dead": 12}
    with pytest.raises(pass_b.PassBError, match="no index"):
        pass_b.parse_inspect("nothing")


def test_counts_comparison_reports_the_gap_per_index():
    indexes = ["articles", "decisions", "dole", "jorf", "sarde", "titres"]
    crible = {index: {"documents": 10, "passages": 20} for index in indexes}
    pgvector = {index: {"documents": 11, "passages": 21} for index in indexes}
    comparison = pass_b.counts_comparison(crible, pgvector)
    assert comparison["totals"]["documents"] == {"crible": 60, "pgvector": 66, "gap": -6}
    assert comparison["per_index"]["dole"]["passages"]["gap"] == -1
    with pytest.raises(pass_b.PassBError, match="missing"):
        pass_b.counts_comparison({"dole": {"documents": 1, "passages": 1}}, pgvector)


def test_per_query_rows_pair_both_sides_without_text():
    primary = {
        "Q01": {"quality_recall_at_k": 1.0, "quality_mrr_at_k": 1.0, "quality_found": [1, 0]},
        "Q02": {"quality_recall_at_k": 0.0, "quality_mrr_at_k": 0.0, "quality_found": []},
    }
    pgvector = {
        "Q01": {"quality_recall_at_k": 0.5, "quality_mrr_at_k": 1.0, "doc_ids": ["C", "A"]},
        "Q02": {"quality_recall_at_k": 0.0, "quality_mrr_at_k": 0.0, "doc_ids": ["X"]},
    }
    expected = {"Q01": ["A", "B"], "Q02": ["A"]}
    index_of = {"Q01": "dole", "Q02": "sarde"}
    rows = pass_b.per_query_rows(primary, pgvector, expected, index_of)
    assert [row["id"] for row in rows] == ["Q01", "Q02"]
    assert rows[0] == {
        "id": "Q01",
        "index": "dole",
        "corpus": pass_b.question_corpus("Q01"),
        "expected": ["A", "B"],
        "primary_found": ["B", "A"],
        "pgvector_found": ["A"],
        "primary_recall_at_10": 1.0,
        "pgvector_recall_at_10": 0.5,
    }
    assert rows[1]["primary_found"] == []
    assert rows[1]["pgvector_found"] == []
    assert "query" not in json.dumps(rows)


def test_per_query_rows_refuse_incomplete_input():
    primary = {"Q01": {"quality_recall_at_k": 1.0, "quality_found": [0]}}
    pgvector = {"Q01": {"doc_ids": ["A"]}}
    with pytest.raises(pass_b.PassBError, match="no expected"):
        pass_b.per_query_rows(primary, pgvector, {"Q01": []}, {"Q01": "dole"})
    with pytest.raises(pass_b.PassBError, match="missing on one side"):
        pass_b.per_query_rows(primary, {}, {"Q01": ["A"]}, {"Q01": "dole"})
    with pytest.raises(pass_b.PassBError, match="no index"):
        pass_b.per_query_rows(primary, pgvector, {"Q01": ["A"]}, {})
