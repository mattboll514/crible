#!/usr/bin/env python3
"""Audit the ``expected`` documents of the reference game (ACTUALISER-JEU-DE-REFERENCE).

For every line of ``docs/legifacile/reference/<index>.jsonl`` and every
``expected`` document it names, the script reads the document fields from the
``records.sqlite`` of the evaluated epoch (read-only connection) and reports:

- the document state and version dates (``etat``, ``date_debut``, ``date_fin``);
- whether the filter of its own line admits the document, using the same
  grammar as ``docs/05-api.md`` for the operators the game actually carries;
- whether the perimeter of its own MCP line admits the document
  (``droit_en_vigueur``, ``droit_historique``, ``droit_applicable``);
- for an ``articles`` document that is not in force, the documents of the same
  ``(texte_cid, num)`` that are in force, so a stale version can be replaced
  by its current one.

The output carries ``id`` and ``doc_id`` only: no query text and no filter
value (ADR-0125, decision 16). ``partie`` filters act on passages, not on the
document, and are reported as ``passage_level`` instead of being decided.
"""

from __future__ import annotations

import argparse
import contextlib
import json
import sqlite3
import sys
from collections.abc import Iterable, Mapping
from pathlib import Path
from typing import Any

INDEXES = ("articles", "jorf", "decisions", "titres", "sarde", "dole")
IN_FORCE = ("VIGUEUR", "VIGUEUR_DIFF", "VIGUEUR_ETEN", "VIGUEUR_NON_ETEN")
CAMPAIGN_DAY = "2026-11-03"
SCALAR_FIELDS = (
    "code",
    "fonds",
    "section",
    "juridiction",
    "theme",
    "type",
    "nature",
)
RANGE_FIELDS = ("date_debut", "date_fin", "annee")
PASSAGE_FIELDS = ("partie",)


class AuditError(Exception):
    """A game file or a database cannot be read as the audit expects."""


def _matches_scalar(value: Any, condition: Any) -> bool:
    if isinstance(condition, Mapping):
        if "in" in condition:
            return value in condition["in"]
        if "nin" in condition:
            return value not in condition["nin"]
        if "exists" in condition:
            return (value is not None) is bool(condition["exists"])
        raise AuditError(f"unsupported scalar condition: {sorted(condition)}")
    return value == condition


def _matches_range(value: Any, condition: Mapping[str, Any]) -> bool:
    for operator, bound in condition.items():
        if value is None:
            return False
        if operator == "lte" and not value <= bound:
            return False
        if operator == "gt" and not value > bound:
            return False
        if operator == "gte" and not value >= bound:
            return False
        if operator == "lt" and not value < bound:
            return False
        if operator not in ("lte", "gt", "gte", "lt"):
            raise AuditError(f"unsupported range operator: {operator}")
    return True


def filter_admits(fields: Mapping[str, Any], condition: Mapping[str, Any]) -> bool:
    """Whether a document filter admits one document, from its fields alone.

    ``partie`` acts on passages and is ignored here; the caller reports it as a
    passage-level condition. ``$or`` is the only combinator the game carries.
    """

    if "$or" in condition:
        return any(filter_admits(fields, part) for part in condition["$or"])
    for key, expected in condition.items():
        if key in PASSAGE_FIELDS:
            continue
        value = fields.get(key)
        if key == "etat":
            if not _matches_scalar(value, expected):
                return False
        elif key in RANGE_FIELDS:
            if not isinstance(expected, Mapping):
                raise AuditError(f"{key}: a range must be an object")
            if not _matches_range(value, expected):
                return False
        elif key in SCALAR_FIELDS:
            if not _matches_scalar(value, expected):
                return False
        else:
            raise AuditError(f"unsupported filter field: {key}")
    return True


def is_passage_level(condition: Mapping[str, Any]) -> bool:
    if "$or" in condition:
        return any(is_passage_level(part) for part in condition["$or"])
    return any(key in PASSAGE_FIELDS for key in condition)


def perimeter_admits(perimeter: str, fields: Mapping[str, Any]) -> bool:
    """Whether the MCP perimeter of the line admits the document.

    `droit_en_vigueur` and `droit_historique` follow the documented state
    filters (`docs/legifacile/index-legifacile.md:317-318`), which carry no date
    clause; `is_current` is the in-force test, used by `current_versions`.
    """

    state = fields.get("etat")
    if perimeter == "droit_en_vigueur":
        return state in IN_FORCE
    if perimeter == "droit_historique":
        return state not in IN_FORCE
    if perimeter == "droit_applicable":
        start = fields.get("date_debut")
        end = fields.get("date_fin")
        return start is not None and end is not None and start <= CAMPAIGN_DAY < end
    return True


def is_current(fields: Mapping[str, Any]) -> bool:
    """Whether a document is in force on the campaign day.

    The state must be one of the four ``VIGUEUR*`` values *and* the version
    interval must contain the day: a ``VIGUEUR_DIFF`` version that starts in
    the future is not in force yet.
    """

    start = fields.get("date_debut")
    end = fields.get("date_fin")
    return (
        fields.get("etat") in IN_FORCE
        and start is not None
        and end is not None
        and start <= CAMPAIGN_DAY < end
    )


def current_versions(
    connection: sqlite3.Connection, fields: Mapping[str, Any]
) -> list[str]:
    """Documents of the same ``(texte_cid, num)`` that are in force on the day."""

    text = fields.get("texte_cid")
    number = fields.get("num")
    if text is None or number is None:
        return []
    rows = connection.execute(
        "SELECT doc_id, fields FROM documents "
        "WHERE json_extract(fields, '$.texte_cid') = ? "
        "AND json_extract(fields, '$.num') = ? "
        "ORDER BY json_extract(fields, '$.date_debut')",
        (text, number),
    )
    found: list[str] = []
    for doc_id, raw in rows:
        candidate = json.loads(raw)
        if is_current(candidate):
            found.append(doc_id)
    return found


def read_jsonl(path: Path) -> Iterable[dict[str, Any]]:
    try:
        with path.open(encoding="utf-8") as handle:
            for line in handle:
                if line.strip():
                    yield json.loads(line)
    except (OSError, json.JSONDecodeError) as exc:
        raise AuditError(f"{path}: {exc}") from exc


def perimeters(reference_dir: Path) -> dict[str, str]:
    mapping: dict[str, str] = {}
    path = reference_dir / "mcp.jsonl"
    for row in read_jsonl(path):
        identifier = row.get("id")
        perimeter = row.get("perimetre")
        if not isinstance(identifier, str) or not isinstance(perimeter, str):
            raise AuditError(f"{path}: a line lacks a string id or perimeter")
        mapping[identifier] = perimeter
    return mapping


def audit(reference_dir: Path, data: Path) -> list[dict[str, Any]]:
    perimeter_of = perimeters(reference_dir)
    rows: list[dict[str, Any]] = []
    for index in INDEXES:
        epoch = (data / "indexes" / index / "CURRENT_EPOCH").read_text().strip()
        database = data / "indexes" / index / "epochs" / epoch / "records.sqlite"
        if not database.is_file():
            raise AuditError(f"{database}: missing")
        game = reference_dir / f"{index}.jsonl"
        with contextlib.closing(
            sqlite3.connect(f"file:{database}?mode=ro", uri=True)
        ) as connection:
            for line in read_jsonl(game):
                identifier = line.get("id")
                expected = line.get("expected")
                if (
                    not isinstance(identifier, str)
                    or not isinstance(expected, list)
                    or not expected
                ):
                    raise AuditError(
                        f"{game}: a line lacks a string id or a non-empty expected list"
                    )
                condition = line.get("filter")
                for doc_id in expected:
                    found = connection.execute(
                        "SELECT fields FROM documents WHERE doc_id = ?", (doc_id,)
                    ).fetchone()
                    if found is None:
                        rows.append(
                            {
                                "id": identifier,
                                "index": index,
                                "doc_id": doc_id,
                                "present": False,
                            }
                        )
                        continue
                    fields = json.loads(found[0])
                    perimeter = perimeter_of.get(identifier)
                    rows.append(
                        {
                            "id": identifier,
                            "index": index,
                            "doc_id": doc_id,
                            "present": True,
                            "etat": fields.get("etat"),
                            "date_debut": fields.get("date_debut"),
                            "date_fin": fields.get("date_fin"),
                            "filter_admitted": True
                            if condition is None
                            else filter_admits(fields, condition),
                            "passage_level": False
                            if condition is None
                            else is_passage_level(condition),
                            "perimeter": perimeter,
                            "perimeter_admitted": True
                            if perimeter is None
                            else perimeter_admits(perimeter, fields),
                            "current_versions": current_versions(connection, fields)
                            if index == "articles"
                            and fields.get("etat") not in IN_FORCE
                            else [],
                        }
                    )
    return rows


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--reference-dir", default="docs/legifacile/reference"
    )
    parser.add_argument("--data", default="/data")
    parser.add_argument("--output", required=True)
    args = parser.parse_args(argv)

    try:
        rows = audit(Path(args.reference_dir), Path(args.data))
    except (AuditError, OSError, sqlite3.Error) as exc:
        print(f"p10-compare audit: {exc}", file=sys.stderr)
        return 2
    payload = {
        "format_version": 1,
        "campaign_day": CAMPAIGN_DAY,
        "rows": rows,
        "filter_not_admitted": [
            {"id": row["id"], "doc_id": row["doc_id"]}
            for row in rows
            if row.get("present") and not row.get("filter_admitted", True)
        ],
        "perimeter_not_admitted": [
            {"id": row["id"], "doc_id": row["doc_id"]}
            for row in rows
            if row.get("present") and not row.get("perimeter_admitted", True)
        ],
        "absent": [
            {"id": row["id"], "doc_id": row["doc_id"]}
            for row in rows
            if not row.get("present", True)
        ],
    }
    Path(args.output).write_text(
        json.dumps(payload, ensure_ascii=False, indent=1) + "\n", encoding="utf-8"
    )
    print(
        "audit: "
        f"{len(rows)} expected documents, "
        f"{len(payload['absent'])} absent, "
        f"{len(payload['filter_not_admitted'])} not admitted by their filter, "
        f"{len(payload['perimeter_not_admitted'])} not admitted by their perimeter"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
