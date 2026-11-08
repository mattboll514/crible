#!/usr/bin/env python3
"""Replay MES-14 on the real Legifacile indexes (pass C of P10).

ADR-0140, decision 8, asks for the p50 and p95 of requests carrying an interval
filter on the two largest indexes, ``decisions`` and ``articles``, in two
situations: a **full domain** (an interval clause alone, so ``scan_column``
walks every live passage that carries the field) and a **reduced domain** (the
same clause behind a bitmap clause). The generated games stay under ``data/``
(ADR-0140, decision 11); every file this module writes is gitignored.

What is measured:

- ``crible eval`` gives the **end-to-end** ``total`` stage and the isolated
  ``filter`` stage, which is the ``Pool::evaluate`` call itself. The second
  number is the MES-14 quantity, read from the same structure the bench reads,
  and it is reported next to the bench reference;
- the SQL counts give ``scan_iids``, the size of ``present ∩ live ∩ keyword``
  that the interval clause receives, and ``pool_size``, the size of the final
  pool. Without them a duration would not be comparable between cases.

The bound values deliberately repeat MES-14 (`annee 2000..2010`,
`2000-01-01..2003-12-31`) so the two campaigns can be put side by side. The
bitmap clause is a keyword field of the index: ``juridiction`` for ``decisions``
and ``etat`` for ``articles``, both named by ADR-0140, decision 8.

The game files carry filter values: they stay under ``data/`` and never enter a
committed report. The aggregate keeps case names, counts and percentiles only.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sqlite3
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable, Sequence

#: Number of query lines per case. With ``--repeat 50`` each case carries 1 000
#: filter samples, the count MES-14 uses for one case.
QUERIES_PER_CASE = 20
REPEAT = 50


def _write_text_atomic(target: Path, text: str) -> None:
    """Write ``text`` to ``target`` through a temporary file and ``os.replace``."""

    target.parent.mkdir(parents=True, exist_ok=True)
    handle = tempfile.NamedTemporaryFile(
        mode="w",
        encoding="utf-8",
        newline="\n",
        dir=target.parent,
        prefix=f".{target.name}.",
        suffix=".tmp",
        delete=False,
    )
    temporary = Path(handle.name)
    try:
        with handle:
            handle.write(text)
        os.replace(temporary, target)
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise

DATE = "2026-11-06"

#: Bounds of MES-14, reused verbatim so the two campaigns compare.
YEAR_LOW, YEAR_HIGH = 2000, 2010
DATE_LOW, DATE_HIGH = "2000-01-01", "2003-12-31"


@dataclass(frozen=True)
class Case:
    """One filter of the matrix.

    ``filter`` is the JSON object ``crible eval`` receives. ``column``,
    ``domain``, ``low`` and ``high`` are the same clause in a form the count
    query can rebuild, so the filter and the SQL cannot drift apart.
    """

    name: str
    kind: str
    filter: dict[str, Any]
    column: str | None = None
    domain: tuple[str, str] | None = None
    low: int | str | None = None
    high: int | str | None = None


def _keyword(domain: tuple[str, str] | None) -> dict[str, Any] | None:
    if domain is None:
        return None
    field, value = domain
    return {field: value}


def _interval(column: str | None, low: int | str | None, high: int | str | None):
    if column is None:
        return None
    return {column: {"gte": low, "lte": high}}


def _and(*clauses: dict[str, Any] | None) -> dict[str, Any]:
    kept = [clause for clause in clauses if clause is not None]
    if len(kept) == 1:
        return kept[0]
    return {"$and": kept}


DECISIONS_DOMAIN = ("juridiction", "Conseil d'État")
ARTICLES_DOMAIN = ("etat", "VIGUEUR_ETEN")


def _cases(domain: tuple[str, str]) -> tuple[Case, ...]:
    keyword = _keyword(domain)
    annee = _interval("annee", YEAR_LOW, YEAR_HIGH)
    date = _interval("date_debut" if domain[0] == "etat" else "date_decision",
                     DATE_LOW, DATE_HIGH)
    return (
        Case("bitmap", "bitmap", keyword, domain=domain),
        Case("annee-plein", "integer", annee, column="annee",
             low=YEAR_LOW, high=YEAR_HIGH),
        Case("date-plein", "date", date,
             column="date_debut" if domain[0] == "etat" else "date_decision",
             low=DATE_LOW, high=DATE_HIGH),
        Case("annee-reduit", "integer", _and(keyword, annee), column="annee",
             domain=domain, low=YEAR_LOW, high=YEAR_HIGH),
        Case("date-reduit", "date", _and(keyword, date),
             column="date_debut" if domain[0] == "etat" else "date_decision",
             domain=domain, low=DATE_LOW, high=DATE_HIGH),
    )


#: Matrix of ADR-0140, decision 8: the two largest indexes, both interval kinds
#: and both domain sizes.
CASES: dict[str, tuple[Case, ...]] = {
    "decisions": _cases(DECISIONS_DOMAIN),
    "articles": _cases(ARTICLES_DOMAIN),
}


def game_name(index: str, case: Case) -> str:
    """Name of the game, and of the eval report it produces."""
    return f"mes14-{index}-{case.name}"


def build_game(vectors: Sequence[Sequence[float]], count: int = QUERIES_PER_CASE):
    """Rows of one case, with distinct ids and a cycled query vector.

    The vector drives the dense branch only; it never touches the filter, so
    cycling the pass-A vectors does not bias the measured stage. It keeps the
    end-to-end latency realistic without encoding a text.
    """
    if not vectors:
        raise ValueError("no query vector to build a game with")
    return [
        {
            "id": f"mes14-{number:04d}",
            "vector": list(vectors[number % len(vectors)]),
        }
        for number in range(count)
    ]


def write_games(
    output_dir: Path,
    vectors: Sequence[Sequence[float]],
    count: int = QUERIES_PER_CASE,
) -> list[tuple[Path, str]]:
    """Write every case of every index, and return path and SHA-256."""
    output_dir.mkdir(parents=True, exist_ok=True)
    written = []
    for index, cases in CASES.items():
        for case in cases:
            rows = build_game(vectors, count)
            for row in rows:
                row["filter"] = case.filter
            path = output_dir / f"{game_name(index, case)}.jsonl"
            text = "".join(
                json.dumps(row, ensure_ascii=False, separators=(",", ":")) + "\n"
                for row in rows
            )
            _write_text_atomic(path, text)
            written.append((path, hashlib.sha256(text.encode("utf-8")).hexdigest()))
    return written


def _literal(value: int | str) -> str:
    if isinstance(value, str):
        return "'" + value.replace("'", "''") + "'"
    return str(int(value))


def _field(name: str) -> str:
    return f"json_extract(d.fields, '$.{name}')"


def filter_sql(case: Case) -> str:
    """SQL predicate of the whole filter, over ``passages p`` and ``documents d``."""
    parts = []
    if case.domain is not None:
        parts.append(f"{_field(case.domain[0])} = {_literal(case.domain[1])}")
    if case.column is not None:
        parts.append(
            f"{_field(case.column)} between {_literal(case.low)} "
            f"and {_literal(case.high)}"
        )
    return " and ".join(parts)


def scan_sql(case: Case) -> str | None:
    """SQL predicate of the set the interval clause walks, or ``None``."""
    if case.column is None:
        return None
    parts = []
    if case.domain is not None:
        parts.append(f"{_field(case.domain[0])} = {_literal(case.domain[1])}")
    parts.append(f"{_field(case.column)} is not null")
    return " and ".join(parts)


def count_case(connection: sqlite3.Connection, case: Case) -> dict[str, int]:
    """Return ``pool_size`` and, when an interval runs, ``scan_iids``."""
    base = (
        "select count(*) from passages p join documents d on d.doc_id = p.doc_id "
        "where p.dead_since_gen is null and "
    )
    pool = connection.execute(base + filter_sql(case)).fetchone()[0]
    scan = None
    clause = scan_sql(case)
    if clause is not None:
        scan = connection.execute(base + clause).fetchone()[0]
    return {"pool_size": int(pool), "scan_iids": None if scan is None else int(scan)}


def count_all(snapshot_dir: Path) -> dict[str, Any]:
    """Count every case on the snapshot of each index, read-only."""
    counts: dict[str, Any] = {"format_version": 1, "date": DATE, "indexes": {}}
    for index, cases in CASES.items():
        snapshots = sorted(snapshot_dir.glob(f"{index}-*"))
        if len(snapshots) != 1:
            raise SystemExit(
                f"expected one snapshot for {index!r} under {snapshot_dir}, "
                f"found {len(snapshots)}"
            )
        records = snapshots[0] / "records.sqlite"
        uri = f"file:{records}?mode=ro&immutable=1"
        connection = sqlite3.connect(uri, uri=True)
        try:
            counts["indexes"][index] = {
                "snapshot": snapshots[0].name,
                "live": connection.execute(
                    "select count(*) from passages where dead_since_gen is null"
                ).fetchone()[0],
                "cases": {
                    case.name: count_case(connection, case) for case in cases
                },
            }
        finally:
            connection.close()
    return counts


def _latency(report: dict[str, Any], mode: str) -> dict[str, Any]:
    modes = report.get("modes") or {}
    if mode not in modes:
        raise SystemExit(f"report carries no {mode!r} mode")
    latency = modes[mode].get("latency") or {}
    kept = {}
    for stage in ("filter", "dense", "lexical", "total"):
        if stage in latency:
            kept[stage] = {
                key: latency[stage].get(key) for key in ("p50", "p95", "p99")
            }
    return kept


def aggregate(
    runs_dir: Path,
    counts: dict[str, Any],
    date: str = DATE,
    repeat: int = REPEAT,
    queries: int = QUERIES_PER_CASE,
) -> dict[str, Any]:
    """Read the eval reports and join them with the SQL counts."""
    result: dict[str, Any] = {
        "format_version": 1,
        "measure": "MES-14",
        "context": "COMPARER-AVEC-PGVECTOR, pass C",
        "date": date,
        "repeat": repeat,
        "queries_per_case": queries,
        "samples_per_case": repeat * queries,
        "method": (
            "crible eval, dense mode, k=10, exact_threshold=5000; "
            "the filter stage is Pool::evaluate; counts from the same "
            "generation snapshot, read-only"
        ),
        "indexes": {},
    }
    for index, cases in CASES.items():
        report_dir = runs_dir / index
        entry: dict[str, Any] = {
            "live": counts["indexes"][index]["live"],
            "snapshot": counts["indexes"][index]["snapshot"],
            "cases": {},
        }
        for case in cases:
            path = report_dir / f"eval-{game_name(index, case)}-{date}.json"
            if not path.is_file():
                raise SystemExit(f"missing eval report {path}")
            report = json.loads(path.read_text(encoding="utf-8"))
            measured = _latency(report, "dense")
            counted = counts["indexes"][index]["cases"][case.name]
            entry["cases"][case.name] = {
                "kind": case.kind,
                "pool_size": counted["pool_size"],
                "scan_iids": counted["scan_iids"],
                "generation": report.get("generation"),
                "exact_threshold": report.get("exact_threshold"),
                "filter_ms": measured.get("filter"),
                "dense_ms": measured.get("dense"),
                "total_ms": measured.get("total"),
            }
        result["indexes"][index] = entry
    return result


def _load_vectors(path: Path) -> list[list[float]]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    queries = payload["queries"] if isinstance(payload, dict) else payload
    return [query["vector"] for query in queries]


def main(argv: Iterable[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    games = sub.add_parser("games", help="write the generated filtered games")
    games.add_argument("--vectors", type=Path,
                       default=Path("data/p10-compare/query-vectors.json"))
    games.add_argument("--output-dir", type=Path,
                       default=Path("data/p10-compare/mes14/games"))
    games.add_argument("--count", type=int, default=QUERIES_PER_CASE)

    counts = sub.add_parser("counts", help="count scan and pool sizes")
    counts.add_argument("--snapshot-dir", type=Path,
                        default=Path("data/p10-exchange/snapshots"))
    counts.add_argument("--output", type=Path,
                        default=Path("data/p10-compare/mes14/counts.json"))

    join = sub.add_parser("aggregate", help="join eval reports and counts")
    join.add_argument("--runs", type=Path,
                      default=Path("data/p10-compare/mes14/runs"))
    join.add_argument("--counts", type=Path,
                      default=Path("data/p10-compare/mes14/counts.json"))
    join.add_argument("--date", default=DATE)
    join.add_argument("--repeat", type=int, default=REPEAT)
    join.add_argument("--queries", type=int, default=QUERIES_PER_CASE)
    join.add_argument("--output", type=Path,
                      default=Path("data/p10-compare/mes14/aggregate.json"))
    join.add_argument("--overwrite", action="store_true",
                      help="replace an existing dated aggregate")

    args = parser.parse_args(list(argv) if argv is not None else None)

    if args.command == "games":
        written = write_games(args.output_dir, _load_vectors(args.vectors), args.count)
        for path, digest in written:
            print(f"{path} {digest}")
        return 0

    if args.command == "counts":
        payload = count_all(args.snapshot_dir)
        _write_text_atomic(
            args.output,
            json.dumps(payload, indent=1, ensure_ascii=False) + "\n",
        )
        print(args.output)
        return 0

    counts_payload = json.loads(args.counts.read_text(encoding="utf-8"))
    if args.output.exists() and not args.overwrite:
        print(
            f"{args.output} exists; pass --overwrite to replace a dated aggregate",
            file=sys.stderr,
        )
        return 2
    payload = aggregate(args.runs, counts_payload, args.date, args.repeat,
                        args.queries)
    _write_text_atomic(
        args.output,
        json.dumps(payload, indent=1, ensure_ascii=False) + "\n",
    )
    print(args.output)
    return 0


if __name__ == "__main__":
    sys.exit(main())
