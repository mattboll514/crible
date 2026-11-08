import json
from pathlib import Path

import pytest

import replay


def write_json(path: Path, payload):
    path.write_text(json.dumps(payload, ensure_ascii=False), encoding="utf-8")
    return path


def test_load_entries_rejects_an_unknown_perimeter(tmp_path):
    path = write_json(
        tmp_path / "queries.json",
        {
            "meta": {},
            "queries": [
                {
                    "id": "Q01",
                    "perimetre": "inconnu",
                    "params": {"k": 10},
                    "text": "Question",
                }
            ],
        },
    )
    with pytest.raises(replay.lib.Mes00Error, match="unknown perimeter"):
        replay.load_entries(path)


def test_attach_vectors_requires_one_vector_per_query():
    entries = [{"id": "Q01", "perimetre": "titres", "params": {}}]
    merged = replay.attach_vectors(entries, {"Q01": [0.1, 0.2]})
    assert merged[0]["vector"] == [0.1, 0.2]
    with pytest.raises(replay.lib.Mes00Error, match="no query vector"):
        replay.attach_vectors(entries, {})
    with pytest.raises(replay.lib.Mes00Error, match="vectors without a query"):
        replay.attach_vectors(entries, {"Q01": [0.1], "Q02": [0.2]})


class FakeMesure:
    def __init__(self, ms):
        self.ms = ms


def fake_dependencies(rows_by_corpus, reconstruction_ms=1.0, calls=None):
    """Minimal stand-ins for the production modules used by replay_one."""

    class Vecteur:
        def __init__(self, values):
            self.values = values

    class Base:
        pass

    Base.Vecteur = Vecteur

    class Perimetres:
        PERIMETERS = {}
        FILTRE_APPLICABLE = {"droit_applicable"}

        @staticmethod
        def corpus_du_perimetre(name):
            return Perimetres.PERIMETRES[name]

    class Outils:
        @staticmethod
        def _applicable_aujourdhui(row, day):
            return row.get("applicable", True)

    class Froid:
        @staticmethod
        def _poste_c(ctx, results):
            return FakeMesure(reconstruction_ms)

    class Conn:
        @staticmethod
        def appeler(fonction, **kwargs):
            if calls is not None:
                calls.append((fonction, kwargs["p_corpus"], kwargs["p_limite"]))
            return {"resultats": rows_by_corpus.get(kwargs["p_corpus"], [])}

    return {
        "base": Base,
        "perimetres": Perimetres,
        "outils": Outils,
        "froid": Froid,
        "conn": Conn,
        "ctx": None,
    }


def fixed_timed(durations):
    """Stand-in for ``replay.timed``: fixed durations, the call still runs."""

    remaining = list(durations)

    def timed(call):
        return remaining.pop(0), call()

    return timed


def test_replay_one_asks_k_per_corpus_for_a_mono_perimeter(monkeypatch):
    # The durations are fixed: the three fields are rounded independently, and
    # a real clock made this test fail about once in four runs (2026-11-08).
    monkeypatch.setattr(replay, "timed", fixed_timed([0.0004999, 2.0004999]))
    calls = []
    dependencies = fake_dependencies(
        {"titres_textes": [{"src_id": "T1", "distance": 0.2}]}, calls=calls
    )
    dependencies["perimetres"].PERIMETRES = {"titres": ("titres_textes",)}
    entry = {
        "id": "Q01",
        "perimetre": "titres",
        "params": {"k": 10, "partie_exclue": []},
        "vector": [0.0],
    }
    run = replay.replay_one(entry, dependencies, None)
    assert calls == [("api.recherche_semantique", "titres_textes", 10)]
    assert [row["doc_id"] for row in run["results"]] == ["T1"]
    assert run["sql"] == 0.0
    assert run["reconstruction"] == 2.0
    assert run["total"] == 2.001
    # Each field is rounded to 0.001 ms independently, so the sum can differ by
    # up to 0.0015 ms.
    assert run["total"] == pytest.approx(run["sql"] + run["reconstruction"], abs=0.002)


def test_replay_one_ranks_a_fusion_and_applies_the_frozen_date():
    dependencies = fake_dependencies(
        {
            "textes_vigueur": [
                {"src_id": "V1", "distance": 0.1, "applicable": True},
                {"src_id": "V2", "distance": 0.3, "applicable": False},
            ],
            "textes_hors_vigueur": [
                {"src_id": "H1", "distance": 0.2, "applicable": True}
            ],
        }
    )
    dependencies["perimetres"].PERIMETRES = {
        "droit_applicable": ("textes_vigueur", "textes_hors_vigueur")
    }
    days = []

    class Outils:
        @staticmethod
        def _applicable_aujourdhui(row, day):
            days.append(day)
            return row.get("applicable", True)

    dependencies["outils"] = Outils
    entry = {
        "id": "Q02-APPLICABLE",
        "perimetre": "droit_applicable",
        "params": {"k": 2, "partie_exclue": []},
        "vector": [0.0],
    }
    run = replay.replay_one(entry, dependencies, replay.FIXED_DATE)
    assert [row["doc_id"] for row in run["results"]] == ["V1", "H1"]
    assert set(days) == {replay.FIXED_DATE}


def test_aggregate_declares_n_and_returns_percentiles():
    measurements = [
        {
            "id": "Q01",
            "perimetre": "titres",
            "latencies": {
                "rep1": {"sql": 1.0, "reconstruction": 2.0, "total": 3.0},
                "rep2": {"sql": 3.0, "reconstruction": 4.0, "total": 7.0},
            },
        },
        {
            "id": "Q02",
            "perimetre": "titres",
            "latencies": {
                "rep1": {"sql": 5.0, "reconstruction": 6.0, "total": 11.0},
                "rep2": {"sql": 7.0, "reconstruction": 8.0, "total": 15.0},
            },
        },
    ]
    aggregates = replay.aggregate(measurements)
    assert aggregates["sql"]["n"] == 4
    assert aggregates["sql"]["p50"] == 4.0
    assert aggregates["total"]["max"] == 15.0
    assert aggregates["by_perimeter"]["titres"]["queries"] == 2
