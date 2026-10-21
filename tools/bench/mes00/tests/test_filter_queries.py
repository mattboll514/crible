import json
import tempfile
from pathlib import Path

import pytest

import filter_queries
import mes00lib as lib


def make_queries():
    return [
        {"query_id": "q0001", "corpus": "juri_sommaires", "text": "A"},
        {"query_id": "q0002", "corpus": "titres_textes", "text": "B"},
        {"query_id": "q0003", "corpus": "juri_sommaires", "text": "C"},
        {"query_id": "q0004", "corpus": "textes_vigueur", "text": "D"},
        {"query_id": "q0005", "corpus": "juri_sommaires", "text": "E"},
        {"query_id": "q0006", "corpus": "textes_vigueur", "text": "F"},
    ]


def test_select_keeps_input_order_and_caps_per_corpus():
    selected = filter_queries.select(
        make_queries(),
        corpora=["juri_sommaires", "textes_vigueur"],
        per_corpus=2,
    )
    assert [query["query_id"] for query in selected] == ["q0001", "q0003", "q0004", "q0006"]


def test_select_without_cap_keeps_every_matching_query():
    selected = filter_queries.select(
        make_queries(), corpora=["juri_sommaires"], per_corpus=None
    )
    assert [query["query_id"] for query in selected] == ["q0001", "q0003", "q0005"]


def test_select_rejects_empty_selection_and_bad_cap():
    with pytest.raises(lib.Mes00Error, match="at least one"):
        filter_queries.select(make_queries(), corpora=[], per_corpus=1)
    with pytest.raises(lib.Mes00Error, match="positive"):
        filter_queries.select(make_queries(), corpora=["juri"], per_corpus=0)
    with pytest.raises(lib.Mes00Error, match="no query selected"):
        filter_queries.select(make_queries(), corpora=["unknown"], per_corpus=1)


def test_build_document_records_the_selection_rule_and_fingerprint():
    selected = filter_queries.select(
        make_queries(), corpora=["juri_sommaires"], per_corpus=2
    )
    document = filter_queries.build_document(
        {"seed": 20261019, "sha256": "abc"},
        selected,
        corpora=["juri_sommaires"],
        per_corpus=2,
    )
    assert document["meta"]["source_sha256"] == "abc"
    assert document["meta"]["count"] == 2
    assert document["meta"]["selection"] == {
        "corpora": ["juri_sommaires"],
        "per_corpus": 2,
        "order": "input file order",
    }
    assert len(document["meta"]["sha256"]) == 64


def test_load_queries_accepts_legacy_names():
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    path = root / "legacy.json"
    path.write_text(
        json.dumps(
            {
                "meta": {"graine": 1, "empreinte_sha256": "abc"},
                "requetes": [
                    {"requete_id": "q1", "corpus": "c", "texte": "Titre"}
                ],
            }
        ),
        encoding="utf-8",
    )
    meta, queries = filter_queries.load_queries(path)
    assert meta["graine"] == 1
    assert queries[0]["query_id"] == "q1"
    assert queries[0]["text"] == "Titre"


def test_cli_writes_a_guarded_file(monkeypatch):
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    source = root / "queries.json"
    source.write_text(
        json.dumps(
            {
                "meta": {"seed": 1, "sha256": "abc"},
                "queries": make_queries(),
            }
        ),
        encoding="utf-8",
    )
    output = root / "selected.json"
    assert (
        filter_queries.main(
            [
                "--input",
                str(source),
                "--output",
                str(output),
                "--corpus",
                "juri_sommaires",
                "--per-corpus",
                "2",
            ]
        )
        == 0
    )
    document = json.loads(output.read_text(encoding="utf-8"))
    assert document["meta"]["count"] == 2
    assert [q["query_id"] for q in document["queries"]] == ["q0001", "q0003"]


def test_cli_refuses_output_outside_tmp():
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    source = root / "queries.json"
    source.write_text(json.dumps({"queries": make_queries()}), encoding="utf-8")
    assert (
        filter_queries.main(
            [
                "--input",
                str(source),
                "--output",
                str(Path(__file__).resolve().parents[3] / "interdit.json"),
                "--corpus",
                "juri_sommaires",
            ]
        )
        == 2
    )
