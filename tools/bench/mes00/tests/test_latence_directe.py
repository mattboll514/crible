import json
import tempfile
from pathlib import Path

import pytest

import latence_directe as direct
import mes00lib as lib


def run(corpus, perimeter, state, sql, encoder=1.0, reconstruction=2.0):
    total = encoder + sql + reconstruction
    return {
        "query_id": f"{corpus}-{sql}",
        "corpus": corpus,
        "perimeter": perimeter,
        "corpus_state": state,
        "perimeter_state": state,
        "corpus_path": {
            "encoder": encoder,
            "sql": sql,
            "reconstruction": reconstruction,
            "total": total,
        },
        "perimeter_path": {
            "encoder": encoder,
            "sql": sql,
            "reconstruction": reconstruction,
            "total": total,
        },
    }


class FakeBase:
    class Vecteur:
        def __init__(self, values):
            self.values = list(values)


class FakeConn:
    def __init__(self):
        self.name = None
        self.vector = None

    def appeler(self, name, **kwargs):
        self.name = name
        self.vector = kwargs["p_vecteur"]
        return {"resultats": []}


class FakeFroid:
    def __init__(self):
        self.vector = None
        self.extra = "not-called"

    def _poste_b(self, ctx, perimeter, vector, extra):
        self.vector = vector
        self.extra = extra
        return ("fusion", [])


def test_direct_search_wraps_vectors_but_fusion_receives_the_raw_list():
    raw = [0.1, 0.2]
    wrapped = direct.wrap_vector(FakeBase, raw)
    assert isinstance(wrapped, FakeBase.Vecteur)
    assert direct.wrap_vector(FakeBase, None) is None

    conn = FakeConn()
    direct.search_corpus(conn, "juri", wrapped, 10, None)
    assert conn.name == "api.recherche_semantique"
    assert conn.vector is wrapped

    froid = FakeFroid()
    direct.search_perimeter(froid, object(), "jurisprudence", raw)
    assert froid.vector is raw
    assert isinstance(froid.vector, list)
    assert not isinstance(froid.vector, FakeBase.Vecteur)
    assert froid.extra is None


def test_run_state_keeps_the_cold_label_when_encoding_fails():
    assert direct.run_state(0, 10) == "cold"
    assert direct.run_state(1, 10) == "warm"
    assert direct.run_state(0, 0) == "warm"

    failed = direct.failed_run(
        {"query_id": "q1"}, "juri", "jurisprudence", direct.run_state(0, 10)
    )
    assert failed["corpus_state"] == "cold"
    assert failed["corpus_path"]["encoder"] is None
    warm = run("juri", "jurisprudence", "warm", 20.0)
    summary = direct.summarize_runs([failed, warm], "corpus")
    assert summary["cold"]["encoder"] == {
        "n": 0,
        "censored": 0,
        "not_run": 0,
        "failed": 1,
    }
    assert summary["warm"]["encoder"]["n"] == 1
    assert summary["warm"]["encoder"]["failed"] == 0


def test_group_by_corpus_sorts_corpora_and_keeps_file_order():
    queries = [
        {"query_id": "1", "corpus": "jorf", "text": "b"},
        {"query_id": "2", "corpus": "juri", "text": "a"},
        {"query_id": "3", "corpus": "juri", "text": "c"},
    ]
    grouped = direct.group_by_corpus(queries)
    assert [corpus for corpus, _ in grouped] == ["jorf", "juri"]
    assert [query["query_id"] for query in grouped[1][1]] == ["2", "3"]


def test_summarize_runs_separates_states_and_counts_censored():
    runs = [
        run("juri", "jurisprudence", "cold", 10.0),
        run("juri", "jurisprudence", "warm", 20.0),
        run("juri", "jurisprudence", "warm", 30.0),
    ]
    runs[2]["corpus_path"]["sql"] = None
    runs[2]["corpus_path"]["total"] = None

    summary = direct.summarize_runs(runs, "corpus")
    assert summary["cold"]["sql"]["n"] == 1
    assert summary["warm"]["sql"]["n"] == 1
    assert summary["warm"]["sql"]["censored"] == 1
    assert summary["warm"]["sql"]["mean"] == 20.0
    assert summary["warm"]["total"]["n"] == 1
    assert summary["warm"]["total"]["censored"] == 1


def test_summarize_runs_all_censored_has_explicit_zero():
    runs = [run("juri", "jurisprudence", "warm", 20.0)]
    runs[0]["corpus_path"]["sql"] = None
    summary = direct.summarize_runs(runs, "corpus")
    assert summary["warm"]["sql"] == {
        "n": 0,
        "censored": 1,
        "not_run": 0,
        "failed": 0,
    }


def test_measure_corpus_does_not_impute_a_zero_after_a_censored_call(monkeypatch):
    def fake_timed(fn):
        return 8_000.0, None, True

    monkeypatch.setattr(direct, "timed", fake_timed)
    ms_sql, ms_reconstruction, censored_sql, censored_reconstruction = (
        direct.measure_corpus(
            FakeConn(),
            "juri",
            FakeBase.Vecteur([0.1, 0.2]),
            10,
            None,
            object(),
            FakeFroid(),
        )
    )
    assert ms_sql == 8_000.0
    assert censored_sql is True
    # The reconstruction stage never ran: no 0 ms may enter its series.
    assert ms_reconstruction is None
    assert censored_reconstruction is True
    record = direct.path_record(
        1.0, False, ms_sql, censored_sql, ms_reconstruction, censored_reconstruction
    )
    assert record == {
        "encoder": 1.0,
        "sql": None,
        "reconstruction": None,
        "total": None,
    }


def test_measure_perimeter_without_a_perimeter_measures_nothing():
    assert direct.measure_perimeter(
        FakeFroid(), object(), "", [0.1, 0.2]
    ) == (None, None, False, False)


def test_measure_perimeter_does_not_impute_a_zero_when_the_fusion_is_empty(
    monkeypatch,
):
    def fake_timed(fn):
        return 120.0, ("fusion", []), False

    monkeypatch.setattr(direct, "timed", fake_timed)
    ms_sql, ms_reconstruction, censored_sql, censored_reconstruction = (
        direct.measure_perimeter(
            FakeFroid(), object(), "jurisprudence", [0.1, 0.2]
        )
    )
    assert ms_sql == 120.0
    assert ms_reconstruction is None
    assert censored_sql is False
    assert censored_reconstruction is False
    assert direct.not_run_stages(
        ms_reconstruction, censored_reconstruction
    ) == ["reconstruction", "total"]


def test_summarize_runs_counts_not_run_apart_from_censored():
    first = run("juri", "jurisprudence", "warm", 20.0)
    empty = run("jorf", "journal_officiel", "warm", 30.0)
    empty["perimeter_path"]["reconstruction"] = None
    empty["perimeter_path"]["total"] = None
    empty["not_run"] = {"corpus": [], "perimeter": ["reconstruction", "total"]}

    summary = direct.summarize_runs([first, empty], "perimeter")
    assert summary["warm"]["sql"]["n"] == 2
    reconstruction = summary["warm"]["reconstruction"]
    assert reconstruction["n"] == 1
    assert reconstruction["censored"] == 0
    assert reconstruction["not_run"] == 1
    assert summary["warm"]["total"]["censored"] == 0
    assert summary["warm"]["total"]["not_run"] == 1


def test_summarize_runs_keeps_runs_without_a_perimeter_out_of_the_perimeter_view():
    runs = [
        run("juri", "jurisprudence", "warm", 20.0),
        run("isole", "", "warm", 0.0, reconstruction=0.0),
    ]
    runs[1]["perimeter_path"] = {
        "encoder": 1.0,
        "sql": None,
        "reconstruction": None,
        "total": None,
    }
    summary = direct.summarize_runs(runs, "perimeter")
    assert summary["warm"]["sql"]["n"] == 1
    assert summary["warm"]["sql"]["min"] == 20.0
    assert summary["warm"]["reconstruction"]["n"] == 1
    assert direct.aggregate(runs)["global"]["perimeter"]["warm"]["sql"]["n"] == 1
    assert direct.aggregate(runs)["global"]["n"] == 2


def test_aggregate_groups_global_corpus_and_perimeter():
    runs = [
        run("juri", "jurisprudence", "warm", 10.0),
        run("juri", "jurisprudence", "warm", 20.0),
        run("jorf", "journal_officiel", "warm", 30.0),
    ]
    aggregates = direct.aggregate(runs)
    assert aggregates["global"]["n"] == 3
    assert aggregates["by_corpus"]["juri"]["n"] == 2
    assert aggregates["by_corpus"]["juri"]["corpus"]["warm"]["sql"]["mean"] == 15.0
    assert aggregates["by_perimeter"]["journal_officiel"]["n"] == 1


def write_queries(path: Path, queries):
    path.write_text(json.dumps({"queries": queries}), encoding="utf-8")
    return path


def test_load_queries_accepts_english_and_legacy_keys():
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    current = write_queries(
        root / "current.json",
        [{"query_id": "q1", "corpus": "juri", "text": "Titre"}],
    )
    legacy = root / "legacy.json"
    legacy.write_text(
        json.dumps(
            {"requetes": [{"requete_id": "q2", "corpus": "jorf", "texte": "Ancien"}]}
        ),
        encoding="utf-8",
    )
    assert direct.load_queries(current)[1][0]["query_id"] == "q1"
    assert direct.load_queries(legacy)[1][0]["text"] == "Ancien"


def test_load_queries_rejects_missing_corpus():
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    path = write_queries(root / "bad.json", [{"query_id": "q1", "text": "Titre"}])
    with pytest.raises(lib.Mes00Error):
        direct.load_queries(path)
