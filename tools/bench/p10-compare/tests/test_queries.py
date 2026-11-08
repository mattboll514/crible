import json
from pathlib import Path

import pytest

import queries as game


def write_jsonl(path: Path, rows):
    path.write_text(
        "".join(json.dumps(row, ensure_ascii=False) + "\n" for row in rows),
        encoding="utf-8",
    )


def reference_dir(tmp_path: Path, extra_rows=None, mes00_rows=None):
    """A minimal version of docs/legifacile/reference, one row per index."""

    directory = tmp_path / "reference"
    directory.mkdir()
    texts = {}
    for index in game.INDEXES:
        identifier = f"{index.upper()}-1"
        text = f"Question {index}"
        texts[identifier] = text
        write_jsonl(
            directory / f"{index}.jsonl",
            [{"id": identifier, "query": text, "expected": ["X"]}],
        )
    rows = [
        {
            "id": identifier,
            "tool": "recherche_semantique",
            "perimetre": "titres",
            "params": {
                "k": 10,
                "dedupliquer": "document",
                "partie_exclue": [],
            },
        }
        for identifier in texts
    ]
    rows.extend(extra_rows or [])
    write_jsonl(directory / "mcp.jsonl", rows)
    mes00 = tmp_path / "queries.json"
    mes00.write_text(
        json.dumps({"queries": mes00_rows or []}, ensure_ascii=False),
        encoding="utf-8",
    )
    return directory, mes00


def test_build_queries_joins_the_two_text_sources(tmp_path):
    mes00_rows = [{"query_id": "q0001", "corpus": "sarde", "text": "Titre MES-00"}]
    extra = [
        {
            "id": "MES00-q0001",
            "tool": "recherche_semantique",
            "perimetre": "index_thematique",
            "params": {"k": 10, "dedupliquer": "document", "partie_exclue": []},
        }
    ]
    directory, mes00 = reference_dir(tmp_path, extra, mes00_rows)
    built = game.build_queries(directory, directory / "mcp.jsonl", mes00)
    assert len(built) == 7
    by_id = {row["id"]: row for row in built}
    assert by_id["ARTICLES-1"]["text"] == "Question articles"
    assert by_id["ARTICLES-1"]["text_source"] == "reference"
    assert by_id["MES00-q0001"]["text"] == "Titre MES-00"
    assert by_id["MES00-q0001"]["source_corpus"] == "sarde"


def test_build_queries_rejects_a_missing_mes00_text(tmp_path):
    extra = [
        {
            "id": "MES00-q9999",
            "tool": "recherche_semantique",
            "perimetre": "titres",
            "params": {"k": 10},
        }
    ]
    directory, mes00 = reference_dir(tmp_path, extra)
    with pytest.raises(game.P10Error, match="MES-00 query"):
        game.build_queries(directory, directory / "mcp.jsonl", mes00)


def test_build_queries_rejects_an_id_without_a_text(tmp_path):
    extra = [
        {
            "id": "Q99",
            "tool": "recherche_semantique",
            "perimetre": "titres",
            "params": {"k": 10},
        }
    ]
    directory, mes00 = reference_dir(tmp_path, extra)
    with pytest.raises(game.P10Error, match="six reference files"):
        game.build_queries(directory, directory / "mcp.jsonl", mes00)


def test_load_mcp_rejects_unknown_keys_and_params(tmp_path):
    directory = tmp_path / "reference"
    directory.mkdir()
    write_jsonl(
        directory / "mcp.jsonl",
        [
            {
                "id": "Q01",
                "tool": "recherche_semantique",
                "perimetre": "titres",
                "params": {"k": 10},
                "extra": 1,
            }
        ],
    )
    with pytest.raises(game.P10Error, match="unexpected keys"):
        game.load_mcp(directory / "mcp.jsonl")

    write_jsonl(
        directory / "mcp.jsonl",
        [
            {
                "id": "Q01",
                "tool": "recherche_semantique",
                "perimetre": "titres",
                "params": {"k": 10, "corpus": "sarde"},
            }
        ],
    )
    with pytest.raises(game.P10Error, match="unknown params"):
        game.load_mcp(directory / "mcp.jsonl")


def test_load_mcp_rejects_a_duplicate_id(tmp_path):
    path = tmp_path / "mcp.jsonl"
    row = {
        "id": "Q01",
        "tool": "recherche_semantique",
        "perimetre": "titres",
        "params": {"k": 10},
    }
    write_jsonl(path, [row, row])
    with pytest.raises(game.P10Error, match="duplicate id"):
        game.load_mcp(path)


def test_sql_params_maps_names_and_neutralises_an_empty_exclusion():
    mapped = game.sql_params(
        {
            "k": 10,
            "dedupliquer": "document",
            "partie_exclue": [],
            "fonds": ["CASS"],
            "cle_domaine": ["Code du travail"],
            "annee_min": 2020,
        }
    )
    assert mapped["p_fonds"] == ["CASS"]
    assert mapped["p_cle_domaine"] == ["Code du travail"]
    assert mapped["p_annee_min"] == 2020
    assert mapped["p_annee_max"] is None
    assert mapped["p_etat"] is None
    assert mapped["p_dedupliquer"] == "document"
    # The SQL function applies no exclusion at all on NULL: the MCP default is
    # a decision of serveur_mcp.outils, never of the database.
    assert mapped["p_partie"] is None


def test_sql_params_forwards_a_real_exclusion():
    mapped = game.sql_params({"partie_exclue": ["moyens", "faits"]})
    assert mapped["p_partie"] == ["moyens", "faits"]


def test_fetch_limit_follows_the_fusion_rule():
    assert game.fetch_limit(10, 1) == 10
    assert game.fetch_limit(10, 2) == 20
    with pytest.raises(game.P10Error, match="positive"):
        game.fetch_limit(0, 1)
    with pytest.raises(game.P10Error, match="at least one corpus"):
        game.fetch_limit(10, 0)


def test_candidates_asks_ktimes_two_per_corpus_and_tags_each_row():
    calls = []

    def search(corpus, limit, params):
        calls.append((corpus, limit, params["p_partie"]))
        return {"resultats": [{"src_id": f"{corpus}-1", "distance": 0.1}]}

    rows = game.candidates(
        ["textes_vigueur", "textes_hors_vigueur"],
        10,
        search,
        {"dedupliquer": "document", "partie_exclue": []},
    )
    assert calls == [
        ("textes_vigueur", 20, None),
        ("textes_hors_vigueur", 20, None),
    ]
    assert [row["corpus"] for row in rows] == ["textes_vigueur", "textes_hors_vigueur"]


def test_select_ranked_sorts_filters_and_cuts():
    rows = [
        {"src_id": "c", "distance": 0.3},
        {"src_id": "a", "distance": 0.1},
        {"src_id": "b", "distance": 0.2},
    ]
    assert [r["src_id"] for r in game.select_ranked(rows, 2)] == ["a", "b"]
    assert [
        r["src_id"]
        for r in game.select_ranked(rows, 3, applicable=lambda r: r["src_id"] != "a")
    ] == ["b", "c"]


def test_ranked_rows_numbers_ranks_and_drops_nan():
    rows = game.ranked_rows(
        [
            {"src_id": "X1", "corpus": "dole", "score": 0.9, "distance": 0.1},
            {"src_id": "X2", "corpus": "dole", "score": "NaN", "distance": "NaN"},
        ]
    )
    assert [row["rank"] for row in rows] == [1, 2]
    assert rows[0]["doc_id"] == "X1"
    assert rows[1]["score"] is None
    assert rows[1]["distance"] is None


def test_percentiles_match_the_mes00_definition():
    assert game.percentile([1.0, 2.0, 3.0, 4.0], 0) == 1.0
    assert game.percentile([1.0, 2.0, 3.0, 4.0], 50) == 2.5
    assert game.percentile([1.0, 2.0, 3.0, 4.0], 100) == 4.0
    summary = game.summarize([4.0, 1.0, 3.0, 2.0])
    assert summary["n"] == 4
    assert summary["p50"] == 2.5
    with pytest.raises(game.P10Error, match="empty"):
        game.summarize([])


def test_aggregate_indexes_merges_the_two_partitions():
    per_corpus = {
        "textes_vigueur": {"documents": 3, "passages": 5},
        "textes_hors_vigueur": {"documents": 2, "passages": 4},
        "dole": {"documents": 1, "passages": 2},
    }
    totals = game.aggregate_indexes(per_corpus)
    assert totals["articles"] == {"documents": 5, "passages": 9}
    assert totals["dole"] == {"documents": 1, "passages": 2}
    assert totals["decisions"] == {"documents": 0, "passages": 0}


def test_drift_separates_data_movement_from_ranking():
    before = {
        "documents": 10,
        "passages": 20,
        "per_corpus": {"dole": {"documents": 10, "passages": 20}},
    }
    after = {
        "documents": 11,
        "passages": 23,
        "per_corpus": {"dole": {"documents": 11, "passages": 23}},
    }
    moved = game.drift(before, after)
    assert moved["totals"] == {"documents": 1, "passages": 3}
    assert moved["per_corpus"]["dole"] == {"documents": 1, "passages": 3}


def test_build_publishable_carries_ids_not_texts():
    replay = {
        "meta": {
            "campaign_date": "2026-11-06",
            "k": 10,
            "repetitions": 3,
            "warmup": 1,
            "query_sha256": "abc",
            "vector_sha256": "def",
        },
        "measurements": [
            {
                "id": "Q01",
                "perimetre": "droit_en_vigueur",
                "results": [
                    {"rank": 1, "doc_id": "LEGIARTI1", "corpus": "textes_vigueur"},
                    {"rank": 2, "doc_id": "LEGIARTI2", "corpus": "textes_vigueur"},
                ],
            }
        ],
        "aggregates": {
            "sql": {"n": 3, "p50": 1.0, "p95": 2.0, "p99": 3.0},
            "reconstruction": {"n": 3},
            "total": {"n": 3},
        },
    }
    counts = {"taken_at": "T", "documents": 1, "passages": 2, "per_corpus": {}}
    published = game.build_publishable(
        replay, counts, counts, machine="Mac mini", fixed_date="2026-11-03"
    )
    assert published["queries"] == [
        {"id": "Q01", "doc_ids": ["LEGIARTI1", "LEGIARTI2"]}
    ]
    assert published["fixed_date"] == "2026-11-03"
    assert published["latency_ms"]["sql"]["p95"] == 2.0
    assert published["counts"]["drift"]["totals"] == {"documents": 0, "passages": 0}
    # The publishable form never carries a text or a filter value.
    forbidden_keys = {
        "text",
        "query",
        "params",
        "partie_exclue",
        "fonds",
        "cle_domaine",
        "annee_min",
        "annee_max",
        "perimetre",
    }

    def walk(node):
        if isinstance(node, dict):
            assert not (set(node) & forbidden_keys), sorted(set(node) & forbidden_keys)
            for value in node.values():
                walk(value)
        elif isinstance(node, list):
            for item in node:
                walk(item)

    walk(published)
