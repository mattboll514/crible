import json
import sqlite3
from pathlib import Path

import pytest

import mes14


def test_matrix_covers_the_two_indexes_and_both_domain_sizes():
    assert sorted(mes14.CASES) == ["articles", "decisions"]
    for index, cases in mes14.CASES.items():
        names = [case.name for case in cases]
        assert names == ["bitmap", "annee-plein", "date-plein",
                         "annee-reduit", "date-reduit"]
        by_name = {case.name: case for case in cases}
        assert by_name["annee-plein"].filter == {
            "annee": {"gte": mes14.YEAR_LOW, "lte": mes14.YEAR_HIGH}
        }
        assert by_name["annee-reduit"].filter["$and"][0] == {
            by_name["bitmap"].domain[0]: by_name["bitmap"].domain[1]
        }
        assert by_name["annee-plein"].column == by_name["annee-reduit"].column
        assert by_name["annee-plein"].low == by_name["annee-reduit"].low
        assert by_name["annee-plein"].high == by_name["annee-reduit"].high
        assert by_name["date-plein"].column == by_name["date-reduit"].column


def test_game_name_is_stable_and_indexed():
    case = mes14.CASES["decisions"][1]
    assert mes14.game_name("decisions", case) == "mes14-decisions-annee-plein"


def test_build_game_repeats_the_vector_and_numbers_the_ids():
    rows = mes14.build_game([[1.0, 2.0], [3.0, 4.0]], count=5)
    assert [row["id"] for row in rows] == [f"mes14-{n:04d}" for n in range(5)]
    assert rows[0]["vector"] == [1.0, 2.0]
    assert rows[1]["vector"] == [3.0, 4.0]
    assert rows[2]["vector"] == [1.0, 2.0]
    assert "filter" not in rows[0]


def test_write_games_writes_one_file_per_case_with_its_filter(tmp_path):
    written = mes14.write_games(tmp_path, [[0.1, 0.2]], count=2)
    total = sum(len(cases) for cases in mes14.CASES.values())
    assert len(written) == total
    for path, digest in written:
        assert path.is_file()
        assert digest == mes14.hashlib.sha256(path.read_bytes()).hexdigest()
        rows = [json.loads(line) for line in path.read_text().splitlines()]
        assert len(rows) == 2
        index = path.name.split("-")[1]
        case_name = path.stem.split("-", 2)[2]
        case = {
            case.name: case for case in mes14.CASES[index]
        }[case_name]
        assert rows[0]["filter"] == case.filter


def test_filter_and_scan_sql_rebuild_the_same_clause():
    cases = {case.name: case for case in mes14.CASES["decisions"]}
    assert mes14.filter_sql(cases["bitmap"]) == (
        "json_extract(d.fields, '$.juridiction') = 'Conseil d''État'"
    )
    assert mes14.scan_sql(cases["bitmap"]) is None
    assert mes14.filter_sql(cases["annee-plein"]) == (
        "json_extract(d.fields, '$.annee') between 2000 and 2010"
    )
    assert mes14.scan_sql(cases["annee-plein"]) == (
        "json_extract(d.fields, '$.annee') is not null"
    )
    assert mes14.filter_sql(cases["annee-reduit"]) == (
        "json_extract(d.fields, '$.juridiction') = 'Conseil d''État' "
        "and json_extract(d.fields, '$.annee') between 2000 and 2010"
    )
    assert mes14.scan_sql(cases["annee-reduit"]) == (
        "json_extract(d.fields, '$.juridiction') = 'Conseil d''État' "
        "and json_extract(d.fields, '$.annee') is not null"
    )


def document(fields):
    return json.dumps(fields, ensure_ascii=False)


def build_database():
    connection = sqlite3.connect(":memory:")
    connection.execute(
        "create table documents (doc_id text primary key, fields text not null)"
    )
    connection.execute(
        "create table passages (iid integer primary key, doc_id text not null, "
        "dead_since_gen integer)"
    )
    rows = [
        ("d1", {"juridiction": "Conseil d'État", "annee": 2002}),
        ("d2", {"juridiction": "Conseil d'État", "annee": 1990}),
        ("d3", {"juridiction": "Cour de cassation", "annee": 2005}),
        ("d4", {"juridiction": "Conseil d'État"}),
    ]
    for doc_id, fields in rows:
        connection.execute(
            "insert into documents values (?, ?)", (doc_id, document(fields))
        )
    passages = [("d1", None), ("d1", None), ("d2", None), ("d3", None),
                ("d3", 1), ("d4", None)]
    for iid, (doc_id, dead) in enumerate(passages):
        connection.execute(
            "insert into passages values (?, ?, ?)", (iid, doc_id, dead)
        )
    return connection


def test_count_case_counts_live_passages_and_the_scan_domain():
    connection = build_database()
    cases = {case.name: case for case in mes14.CASES["decisions"]}
    assert mes14.count_case(connection, cases["bitmap"]) == {
        "pool_size": 4,
        "scan_iids": None,
    }
    # annee 2000..2010 keeps d1 twice and d3 once; the third d3 passage is dead.
    assert mes14.count_case(connection, cases["annee-plein"]) == {
        "pool_size": 3,
        "scan_iids": 4,
    }
    # Behind the bitmap clause only d1 and d4 have the field; d4 carries no annee.
    assert mes14.count_case(connection, cases["annee-reduit"]) == {
        "pool_size": 2,
        "scan_iids": 3,
    }


def write_report(path: Path, filter_p50: float, total_p50: float):
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "generation": 2,
        "exact_threshold": 5000,
        "modes": {
            "dense": {
                "latency": {
                    "filter": {"p50": filter_p50, "p95": filter_p50 + 1,
                               "p99": filter_p50 + 2},
                    "dense": {"p50": 9.0, "p95": 10.0, "p99": 11.0},
                    "total": {"p50": total_p50, "p95": total_p50 + 1,
                              "p99": total_p50 + 2},
                }
            }
        },
    }
    path.write_text(json.dumps(payload), encoding="utf-8")


def counts_payload():
    return {
        "indexes": {
            "decisions": {
                "live": 3_589_776,
                "snapshot": "decisions-e2-g2",
                "cases": {
                    case.name: {"pool_size": 10, "scan_iids": 20}
                    for case in mes14.CASES["decisions"]
                },
            },
            "articles": {
                "live": 2_476_715,
                "snapshot": "articles-e1-g1",
                "cases": {
                    case.name: {"pool_size": 11, "scan_iids": 21}
                    for case in mes14.CASES["articles"]
                },
            },
        }
    }


def test_aggregate_joins_each_report_with_its_counts(tmp_path):
    runs = tmp_path / "runs"
    for index, cases in mes14.CASES.items():
        for case in cases:
            write_report(
                runs / index
                / f"eval-{mes14.game_name(index, case)}-{mes14.DATE}.json",
                61.5 if case.name == "annee-plein" else 4.5,
                71.3 if case.name == "annee-plein" else 15.0,
            )
    payload = mes14.aggregate(runs, counts_payload())
    assert payload["samples_per_case"] == mes14.REPEAT * mes14.QUERIES_PER_CASE
    decision = payload["indexes"]["decisions"]
    assert decision["live"] == 3_589_776
    assert decision["cases"]["annee-plein"]["filter_ms"]["p50"] == 61.5
    assert decision["cases"]["annee-plein"]["total_ms"]["p50"] == 71.3
    assert decision["cases"]["annee-plein"]["scan_iids"] == 20
    assert payload["indexes"]["articles"]["cases"]["bitmap"]["pool_size"] == 11


def test_aggregate_refuses_a_missing_report(tmp_path):
    with pytest.raises(SystemExit):
        mes14.aggregate(tmp_path / "runs", counts_payload())
