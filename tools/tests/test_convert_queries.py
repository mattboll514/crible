# SPDX-License-Identifier: Apache-2.0
"""Tests of the sample-to-eval converter (ADR-0125, decisions 3, 4 and 16).

The converter is the only Python part of `crible eval`: it maps `question` to
`query`, `expected[].src_id` to `expected` and `id` to `id`, and writes one game
per structure under `data/`. These tests pin that mapping, the closed shape of
the output, the refusals with their line number, and the confidentiality rule of
decision 16.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from bench import convert_queries

SAMPLE = Path(__file__).resolve().parents[2] / "data" / "legifacile-sample" / "queries.jsonl"


def question(identifier: str, text: str = "une question", structure: str = "articles", expected=None):
    """One row of `queries.jsonl`, with the members the converter ignores."""
    row = {
        "id": identifier,
        "question": text,
        "corpus": "textes_vigueur",
        "structure": structure,
        "domaine": "droit civil",
        "satisfiable": True,
        "expected": expected if expected is not None else [],
        "source": "test",
    }
    return row


def write_queries(path: Path, rows: list[dict]) -> Path:
    path.write_text(
        "".join(json.dumps(row, ensure_ascii=False) + "\n" for row in rows), encoding="utf-8"
    )
    return path


def test_the_mapping_takes_the_question_the_src_ids_and_the_id():
    rows = [(1, question("Q01", "Un texte ?", expected=[{"src_id": "LEGI1"}, {"src_id": "LEGI2"}]))]
    games = convert_queries.convert(rows)
    assert games == {
        "articles": [{"id": "Q01", "query": "Un texte ?", "expected": ["LEGI1", "LEGI2"]}]
    }


def test_one_game_is_written_per_structure_in_file_order():
    rows = [
        (1, question("Q01", structure="articles")),
        (2, question("Q02", structure="decisions")),
        (3, question("Q03", structure="articles")),
    ]
    games = convert_queries.convert(rows)
    assert sorted(games) == ["articles", "decisions"]
    assert [line["id"] for line in games["articles"]] == ["Q01", "Q03"]
    assert [line["id"] for line in games["decisions"]] == ["Q02"]


def test_a_question_without_expected_writes_no_expected_member():
    games = convert_queries.convert([(1, question("Q01"))])
    assert games["articles"][0] == {"id": "Q01", "query": "une question"}


def test_a_repeated_src_id_is_kept_once():
    games = convert_queries.convert(
        [(1, question("Q01", expected=[{"src_id": "LEGI1"}, {"src_id": "LEGI1"}]))]
    )
    assert games["articles"][0]["expected"] == ["LEGI1"]


def test_only_the_requested_structures_are_kept():
    rows = [
        (1, question("Q01", structure="articles")),
        (2, question("Q02", structure="decisions")),
    ]
    games = convert_queries.convert(rows, ["decisions"])
    assert sorted(games) == ["decisions"]


@pytest.mark.parametrize(
    ("row", "message"),
    [
        (question(""), "`id` is missing"),
        ({"id": "Q01", "question": "t"}, "`structure` is missing"),
        ({"id": "Q01", "structure": "articles"}, "`question` is missing"),
        (question("Q01", expected="LEGI1"), "`expected` must be a list"),
        (question("Q01", expected=["LEGI1"]), "an `expected` entry must be an object"),
        (question("Q01", expected=[{"num": "1"}]), "has no `src_id`"),
    ],
)
def test_a_malformed_question_names_its_line(row, message):
    with pytest.raises(SystemExit) as failure:
        convert_queries.convert([(1, question("Q0A")), (2, row)])
    assert "line 2" in str(failure.value)
    assert message in str(failure.value)


def test_a_duplicate_id_names_its_line():
    with pytest.raises(SystemExit) as failure:
        convert_queries.convert([(1, question("Q01")), (2, question("Q01"))])
    assert "line 2" in str(failure.value)
    assert "not unique" in str(failure.value)


def test_a_refusal_never_repeats_the_question_text():
    secret = "question-confidentielle-2c9f"
    with pytest.raises(SystemExit) as failure:
        convert_queries.convert([(1, question("Q01", text=secret, expected="LEGI1"))])
    assert secret not in str(failure.value)


def test_invalid_json_names_its_line(tmp_path: Path):
    path = tmp_path / "queries.jsonl"
    path.write_text('{"id": "Q01"}\nnot json\n', encoding="utf-8")
    with pytest.raises(SystemExit) as failure:
        convert_queries.read_rows(path)
    assert "line 2" in str(failure.value)


def test_the_games_are_written_under_the_output_directory(tmp_path: Path):
    rows = [
        (1, question("Q01", structure="articles")),
        (2, question("Q02", structure="titres")),
    ]
    games = convert_queries.convert(rows)
    out = tmp_path / "data" / "eval-queries"
    written = convert_queries.write_games(games, out)
    assert [path.name for path in written] == ["articles.jsonl", "titres.jsonl"]
    assert all(path.parent == out for path in written)
    assert sorted(path.name for path in tmp_path.iterdir()) == ["data"]


def test_the_sample_of_the_repository_converts_to_one_index_per_structure():
    """The real sample: one game per structure, and no other member."""
    if not SAMPLE.is_file():
        pytest.skip("data/legifacile-sample/queries.jsonl is absent")
    games = convert_queries.convert(convert_queries.read_rows(SAMPLE))
    assert sorted(games) == ["articles", "decisions", "dole", "jorf", "sarde", "titres"]
    assert sum(len(lines) for lines in games.values()) == 300
    for lines in games.values():
        for line in lines:
            assert set(line) <= {"id", "query", "expected"}
            assert isinstance(line["query"], str) and line["query"]
            assert line["id"]
