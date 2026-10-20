import json
import tempfile
from pathlib import Path

import pytest

import gen_requetes
import mes00lib as lib
import pgoptions_values as pgvalues


def make_rows(count=50):
    return [
        {
            "id": f"id-{index:03d}",
            "corpus": "juri" if index % 2 == 0 else "jorf",
            "text": f"Article {index} sur la responsabilité civile",
            "origin": "json",
        }
        for index in range(count)
    ]


def test_build_queries_is_deterministic():
    rows = make_rows()
    first = gen_requetes.build_queries(
        rows, count=20, seed=42, date="2026-10-19T00:00:00+00:00"
    )
    second = gen_requetes.build_queries(
        list(reversed(rows)), count=20, seed=42, date="2026-10-19T00:00:00+00:00"
    )
    assert first == second
    assert first["meta"]["seed"] == 42
    assert first["meta"]["count"] == 20
    assert len(first["queries"]) == 20
    assert len(first["meta"]["sha256"]) == 64
    assert sum(first["meta"]["corpora"].values()) == 20


def test_build_queries_changes_with_seed():
    rows = make_rows()
    first = gen_requetes.build_queries(rows, count=30, seed=1)
    second = gen_requetes.build_queries(rows, count=30, seed=2)
    assert [q["text"] for q in first["queries"]] != [
        q["text"] for q in second["queries"]
    ]


def test_build_queries_has_only_english_keys():
    document = gen_requetes.build_queries(make_rows(), count=10, seed=1)
    assert all(
        set(query) == {"query_id", "corpus", "text", "source_id"}
        for query in document["queries"]
    )
    assert all(query["source_id"] for query in document["queries"])


def test_build_queries_caps_at_pool_size():
    document = gen_requetes.build_queries(make_rows(5), count=100, seed=1)
    assert len(document["queries"]) == 5


def test_deduplicate_sorts_and_drops_duplicate_ids():
    rows = [
        {"id": "b", "corpus": "x", "text": "B"},
        {"id": "a", "corpus": "x", "text": "A"},
        {"id": "b", "corpus": "x", "text": "B bis"},
    ]
    unique = gen_requetes.deduplicate(rows)
    assert [row["id"] for row in unique] == ["a", "b"]
    assert unique[1]["text"] == "B"


def test_shorten_keeps_whole_words_and_hard_cuts_long_tokens():
    assert gen_requetes.shorten("  Article   1240  ", 100) == "Article 1240"
    assert gen_requetes.shorten("mot " * 50, 12) == "mot mot mot"
    assert gen_requetes.shorten("x" * 400, 300) == "x" * 300


def test_validate_readonly_sql_accepts_select_and_with():
    assert gen_requetes.validate_readonly_sql("SELECT id FROM t") == "SELECT id FROM t"
    assert gen_requetes.validate_readonly_sql("WITH t AS (SELECT 1) SELECT * FROM t;")
    with pytest.raises(lib.Mes00Error):
        gen_requetes.validate_readonly_sql("SELECT 1; DROP TABLE t")
    with pytest.raises(lib.Mes00Error):
        gen_requetes.validate_readonly_sql("UPDATE t SET a = 1")
    with pytest.raises(lib.Mes00Error):
        gen_requetes.validate_readonly_sql("SELECT * FROM t WHERE x = 'set'")


def test_read_rows_json_and_jsonl_accept_legacy_text_key():
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    rows = [{"id": "1", "corpus": "c", "text": "Titre"}]
    json_path = root / "rows.json"
    json_path.write_text(json.dumps({"rows": rows}), encoding="utf-8")
    jsonl_path = root / "rows.jsonl"
    jsonl_path.write_text(json.dumps(rows[0]) + "\n", encoding="utf-8")
    legacy = root / "legacy.json"
    legacy.write_text(
        json.dumps([{"id": "2", "corpus": "c", "texte": "Ancien"}]), encoding="utf-8"
    )
    assert gen_requetes.read_rows_json(json_path)[0]["id"] == "1"
    assert gen_requetes.read_rows_json(jsonl_path)[0]["text"] == "Titre"
    assert gen_requetes.read_rows_json(legacy)[0]["text"] == "Ancien"


def test_read_rows_csv_accepts_both_text_headers():
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    current = root / "titres.csv"
    current.write_text("id,corpus,text\n1,juri,Titre\n", encoding="utf-8")
    legacy = root / "titres-legacy.csv"
    legacy.write_text("id,corpus,texte\n2,jorf,Ancien\n", encoding="utf-8")
    assert gen_requetes.read_rows_csv(current)[0]["text"] == "Titre"
    assert gen_requetes.read_rows_csv(legacy)[0]["text"] == "Ancien"


def test_read_rows_rejects_empty_document():
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    empty = root / "empty.json"
    empty.write_text("[]", encoding="utf-8")
    with pytest.raises(lib.Mes00Error):
        gen_requetes.read_rows_json(empty)


def test_build_psql_argv_uses_the_postgres_user():
    argv = gen_requetes.build_psql_argv("container", "SELECT id, corpus, text FROM t")
    assert argv[:1] == ["docker"]
    assert "-U" in argv
    assert argv[argv.index("-U") + 1] == "postgres"
    assert argv[argv.index("-c") + 1].startswith("COPY (")


def test_cli_writes_a_guarded_file(monkeypatch):
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    source = root / "rows.json"
    source.write_text(json.dumps(make_rows(10)), encoding="utf-8")
    output = root / "queries.json"
    monkeypatch.setenv("PROD_PGOPTIONS", pgvalues.VALID)

    assert (
        gen_requetes.main(
            [
                "--from-json",
                str(source),
                "--output",
                str(output),
                "--count",
                "5",
                "--seed",
                "7",
            ]
        )
        == 0
    )
    document = json.loads(output.read_text(encoding="utf-8"))
    assert document["meta"]["seed"] == 7
    assert len(document["queries"]) == 5


def test_cli_accepts_csv(monkeypatch):
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    source = root / "titres.csv"
    source.write_text("id,corpus,text\n1,juri,Titre\n2,jorf,Titre 2\n", encoding="utf-8")
    output = root / "queries.json"
    monkeypatch.setenv("PROD_PGOPTIONS", pgvalues.VALID)
    assert (
        gen_requetes.main(
            ["--from-csv", str(source), "--output", str(output)]
        )
        == 0
    )
    assert json.loads(output.read_text(encoding="utf-8"))["meta"]["count"] == 2


def test_cli_refuses_output_outside_tmp(monkeypatch):
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    source = root / "rows.json"
    source.write_text(json.dumps(make_rows(3)), encoding="utf-8")
    monkeypatch.setenv("PROD_PGOPTIONS", pgvalues.VALID)
    forbidden = Path(__file__).resolve().parents[3] / "interdit.json"
    assert (
        gen_requetes.main(
            ["--from-json", str(source), "--output", str(forbidden)]
        )
        == 2
    )


def test_cli_refuses_without_prod_pgoptions(monkeypatch, tmp_path):
    monkeypatch.delenv("PROD_PGOPTIONS", raising=False)
    source = tmp_path / "rows.json"
    source.write_text(json.dumps(make_rows(3)), encoding="utf-8")
    assert (
        gen_requetes.main(
            ["--from-json", str(source), "--output", "/tmp/mes00-queries.json"]
        )
        == 2
    )
