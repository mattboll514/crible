import json
import sqlite3

import pytest

import audit_expected


def fields(**overrides):
    base = {
        "etat": "VIGUEUR",
        "date_debut": "2022-09-01",
        "date_fin": "2999-01-01",
        "annee": 2022,
        "code": "Code du travail",
        "fonds": "LEGI",
        "num": "L1132-1",
        "texte_cid": "LEGITEXT000006072050",
    }
    base.update(overrides)
    return base


def test_state_in_and_nin():
    condition = {"etat": {"in": list(audit_expected.IN_FORCE)}}
    assert audit_expected.filter_admits(fields(), condition)
    assert not audit_expected.filter_admits(fields(etat="MODIFIE"), condition)
    history = {"$or": [{"etat": {"nin": list(audit_expected.IN_FORCE)}}]}
    assert audit_expected.filter_admits(fields(etat="ABROGE"), history)
    assert not audit_expected.filter_admits(fields(), history)


def test_state_exists():
    condition = {"etat": {"exists": False}}
    assert audit_expected.filter_admits(fields(etat=None), condition)
    assert not audit_expected.filter_admits(fields(), condition)


def test_dates_follow_the_campaign_day():
    condition = {"date_debut": {"lte": "2026-11-03"}, "date_fin": {"gt": "2026-11-03"}}
    assert audit_expected.filter_admits(fields(), condition)
    assert audit_expected.filter_admits(
        fields(date_debut="2014-10-01", date_fin="2029-01-01"), condition
    )
    assert not audit_expected.filter_admits(
        fields(date_debut="2026-11-04", date_fin="2999-01-01"), condition
    )
    assert not audit_expected.filter_admits(
        fields(date_debut="2001-01-01", date_fin="2014-01-01"), condition
    )


def test_scalar_and_list_fields():
    assert audit_expected.filter_admits(fields(), {"code": "Code du travail"})
    assert not audit_expected.filter_admits(fields(), {"code": "Code civil"})
    assert audit_expected.filter_admits(fields(), {"fonds": {"in": ["LEGI"]}})
    assert not audit_expected.filter_admits(fields(), {"fonds": {"in": ["JORF"]}})
    assert audit_expected.filter_admits(fields(), {"annee": {"gte": 2020}})
    assert not audit_expected.filter_admits(fields(), {"annee": {"gte": 2023}})


def test_unsupported_field_is_refused():
    with pytest.raises(audit_expected.AuditError):
        audit_expected.filter_admits(fields(), {"unknown": "value"})


def test_partie_is_passage_level_and_not_decided():
    condition = {
        "$or": [
            {"partie": {"nin": ["moyens", "faits"]}},
            {"partie": {"exists": False}},
        ]
    }
    assert audit_expected.filter_admits(fields(), condition)
    assert audit_expected.is_passage_level(condition)
    assert not audit_expected.is_passage_level({"etat": {"in": ["VIGUEUR"]}})


def test_is_current_excludes_future_versions():
    assert audit_expected.is_current(fields())
    assert not audit_expected.is_current(
        fields(etat="VIGUEUR_DIFF", date_debut="2029-01-01", date_fin="2999-01-01")
    )
    assert not audit_expected.is_current(fields(etat="MODIFIE"))


def test_current_versions_keeps_only_the_versions_in_force_on_the_day():
    connection = sqlite3.connect(":memory:")
    connection.execute("CREATE TABLE documents (doc_id TEXT, fields TEXT)")
    rows = [
        ("OLD", fields(etat="MODIFIE", date_debut="2023-04-01", date_fin="2026-08-20")),
        ("NEW", fields(date_debut="2026-08-20", date_fin="2999-01-01")),
        (
            "FUTURE",
            fields(
                etat="VIGUEUR_DIFF",
                date_debut="2029-01-01",
                date_fin="2999-01-01",
            ),
        ),
        ("OTHER", fields(num="L442-2")),
    ]
    connection.executemany(
        "INSERT INTO documents VALUES (?, ?)",
        [(doc_id, json.dumps(value)) for doc_id, value in rows],
    )
    assert audit_expected.current_versions(connection, fields()) == ["NEW"]
    connection.close()


def test_perimeters_rejects_a_line_without_an_id(tmp_path):
    (tmp_path / "mcp.jsonl").write_text(
        '{"perimetre": "droit_en_vigueur"}\n', encoding="utf-8"
    )
    with pytest.raises(audit_expected.AuditError):
        audit_expected.perimeters(tmp_path)


def test_perimeters_rejects_a_line_without_a_perimeter(tmp_path):
    (tmp_path / "mcp.jsonl").write_text('{"id": "Q01"}\n', encoding="utf-8")
    with pytest.raises(audit_expected.AuditError):
        audit_expected.perimeters(tmp_path)


def test_perimeter_admission():
    in_force = {"etat": "VIGUEUR", "date_debut": "2022-09-01", "date_fin": "2999-01-01"}
    assert audit_expected.perimeter_admits("droit_en_vigueur", in_force)
    assert not audit_expected.perimeter_admits("droit_en_vigueur", {"etat": "MODIFIE"})
    # `droit_en_vigueur` is the documented `etat in VIGUEUR*` filter, without a
    # date clause (docs/legifacile/index-legifacile.md:317): a future
    # `VIGUEUR_DIFF` is admitted by the perimeter, even though `is_current`
    # rejects it.
    future = {
        "etat": "VIGUEUR_DIFF",
        "date_debut": "2029-01-01",
        "date_fin": "2999-01-01",
    }
    assert audit_expected.perimeter_admits("droit_en_vigueur", future)
    assert not audit_expected.is_current(future)
    assert audit_expected.perimeter_admits("droit_historique", {"etat": "ABROGE"})
    assert not audit_expected.perimeter_admits("droit_historique", in_force)
    applicable = {"date_debut": "2014-10-01", "date_fin": "2029-01-01"}
    assert audit_expected.perimeter_admits("droit_applicable", applicable)
    assert not audit_expected.perimeter_admits(
        "droit_applicable", {"date_debut": "2001-01-01", "date_fin": "2014-01-01"}
    )
    assert audit_expected.perimeter_admits("jurisprudence", {})
