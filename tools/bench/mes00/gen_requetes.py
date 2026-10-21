#!/usr/bin/env python3
"""Generate the MES-00 latency query set from article and summary titles.

The query set is deterministic for a given seed, input rows and limit. It only
contains titles read from the corpus; it never contains a user question.

Three sources are supported:

* ``--from-db`` reads titles through a read-only ``SELECT``/``WITH`` query
  executed with ``docker exec`` and ``PGOPTIONS=$PROD_PGOPTIONS``. The query
  must return the columns ``id``, ``corpus`` and ``text`` (``texte`` is
  accepted for legacy exports).
* ``--from-csv`` reads the production CSV export with those columns.
* ``--from-json`` reads an existing JSON or JSONL export with the same
  columns. It is intended for tests and for re-running the generator offline.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import io
import json
import os
import random
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable, Mapping, Sequence

import mes00lib as lib

SCHEMA_VERSION = 1
DEFAULT_SEED = 20261019
DEFAULT_COUNT = 1000
DEFAULT_MAX_LENGTH = 300

_REQUIRED_COLUMNS = ("id", "corpus", "text")
_SUPPORTED_TEXT_COLUMNS = ("text", "texte", "titre", "title")
_FORBIDDEN_SQL = re.compile(
    r"\b(insert|update|delete|drop|alter|create|grant|revoke|truncate|copy|"
    r"vacuum|analyze|reindex|set|reset|call|do|listen|notify|lock)\b",
    re.IGNORECASE,
)


def validate_readonly_sql(sql: str) -> str:
    """Refuse anything that is not a single SELECT or WITH statement."""

    stripped = sql.strip()
    if not stripped:
        raise lib.Mes00Error("the title query is empty")
    core = stripped[:-1] if stripped.endswith(";") else stripped
    if ";" in core:
        raise lib.Mes00Error("the title query must be a single statement")
    first = core.split(None, 1)[0].upper()
    if first not in {"SELECT", "WITH"}:
        raise lib.Mes00Error("the title query must start with SELECT or WITH")
    match = _FORBIDDEN_SQL.search(core)
    if match:
        raise lib.Mes00Error(
            f"the title query contains a forbidden keyword: {match.group(1)}"
        )
    return core


def _pick(raw: Mapping[str, Any], *names: str) -> str:
    for name in names:
        value = raw.get(name)
        if value is not None:
            return str(value).strip()
    return ""


def _normalise_row(raw: Mapping[str, Any], *, origin: str) -> dict[str, str] | None:
    identifier = _pick(raw, "id", "source_id")
    corpus = _pick(raw, "corpus")
    text = _pick(raw, *_SUPPORTED_TEXT_COLUMNS)
    if not corpus or not text:
        return None
    if not identifier:
        # A stable synthetic id makes the fingerprint reproducible without
        # inventing a corpus identifier.
        identifier = hashlib.sha256(f"{corpus}\0{text}".encode("utf-8")).hexdigest()[:16]
    return {"id": identifier, "corpus": corpus, "text": text, "origin": origin}


def _rows_from_payload(payload: Any, *, origin: str) -> list[dict[str, str]]:
    if isinstance(payload, Mapping):
        payload = payload.get("rows", payload)
    if not isinstance(payload, list):
        raise lib.Mes00Error("the input must contain a JSON array of rows")
    rows: list[dict[str, str]] = []
    for raw in payload:
        if not isinstance(raw, Mapping):
            continue
        row = _normalise_row(raw, origin=origin)
        if row is not None:
            rows.append(row)
    return rows


def read_rows_json(path: str | os.PathLike[str]) -> list[dict[str, str]]:
    """Read a JSON array, a JSON object with ``rows`` or a JSONL file."""

    source = Path(path)
    try:
        if source.suffix.lower() == ".jsonl":
            raw_rows = [
                json.loads(line)
                for line in source.read_text(encoding="utf-8").splitlines()
                if line.strip()
            ]
            rows = _rows_from_payload(raw_rows, origin="json")
        else:
            payload = json.loads(source.read_text(encoding="utf-8"))
            rows = _rows_from_payload(payload, origin="json")
    except OSError as exc:
        raise lib.Mes00Error(f"cannot read {source.name}: {exc}") from exc
    except json.JSONDecodeError as exc:
        raise lib.Mes00Error(f"invalid JSON in {source.name}: {exc}") from exc
    if not rows:
        raise lib.Mes00Error(f"{source.name} contains no usable title row")
    return rows


def read_rows_csv(path: str | os.PathLike[str]) -> list[dict[str, str]]:
    """Read the production CSV export (``id,corpus,text`` header)."""

    source = Path(path)
    try:
        with source.open("r", encoding="utf-8", newline="") as handle:
            reader = csv.DictReader(handle)
            if reader.fieldnames is None:
                raise lib.Mes00Error(f"{source.name} has no CSV header")
            header = {name.strip().lower(): name for name in reader.fieldnames}
            text_column = next(
                (name for name in _SUPPORTED_TEXT_COLUMNS if name in header), None
            )
            if "id" not in header or "corpus" not in header or text_column is None:
                raise lib.Mes00Error(
                    f"{source.name} needs the columns id, corpus and text"
                )
            rows: list[dict[str, str]] = []
            for raw in reader:
                mapped = {
                    "id": raw.get(header["id"], ""),
                    "corpus": raw.get(header["corpus"], ""),
                    "text": raw.get(header[text_column], ""),
                }
                row = _normalise_row(mapped, origin="csv")
                if row is not None:
                    rows.append(row)
    except OSError as exc:
        raise lib.Mes00Error(f"cannot read {source.name}: {exc}") from exc
    if not rows:
        raise lib.Mes00Error(f"{source.name} contains no usable title row")
    return rows


def build_psql_argv(container: str, sql: str) -> list[str]:
    """Build the read-only ``docker exec`` command, with an explicit user."""

    core = validate_readonly_sql(sql)
    command = f"COPY ({core}) TO STDOUT WITH (FORMAT csv, HEADER true)"
    return [
        "docker",
        "exec",
        "-i",
        "-e",
        "PGOPTIONS",
        container,
        "psql",
        "-X",
        "-q",
        "-U",
        "postgres",
        "-v",
        "ON_ERROR_STOP=1",
        "-c",
        command,
    ]


def rows_from_db(sql: str, *, container: str, pgoptions: str) -> list[dict[str, str]]:
    """Run a read-only COPY of the title query and parse it as CSV."""

    argv = build_psql_argv(container, sql)
    try:
        completed = subprocess.run(
            argv,
            capture_output=True,
            text=True,
            env=lib.child_env(pgoptions),
            check=False,
            timeout=300,
        )
    except FileNotFoundError as exc:
        raise lib.Mes00Error("docker is not available on this machine") from exc
    except subprocess.TimeoutExpired as exc:
        raise lib.Mes00Error("the title query timed out") from exc
    if completed.returncode != 0:
        tail = " ".join(completed.stderr.strip().splitlines()[-1:])
        raise lib.Mes00Error(
            f"the title query failed with exit code {completed.returncode}: {tail[:300]}"
        )
    return _rows_from_csv_text(completed.stdout)


def _rows_from_csv_text(text: str) -> list[dict[str, str]]:
    reader = csv.DictReader(io.StringIO(text))
    if reader.fieldnames is None:
        raise lib.Mes00Error("the title query returned no header")
    header = {name.strip().lower(): name for name in reader.fieldnames}
    text_column = next(
        (name for name in _SUPPORTED_TEXT_COLUMNS if name in header), None
    )
    if "id" not in header or "corpus" not in header or text_column is None:
        raise lib.Mes00Error(
            "the title query must return columns id, corpus, text; "
            "missing one of them"
        )
    rows: list[dict[str, str]] = []
    for raw in reader:
        mapped = {
            "id": raw.get(header["id"], ""),
            "corpus": raw.get(header["corpus"], ""),
            "text": raw.get(header[text_column], ""),
        }
        row = _normalise_row(mapped, origin="db")
        if row is not None:
            rows.append(row)
    if not rows:
        raise lib.Mes00Error("the title query returned no usable row")
    return rows


def shorten(text: str, max_length: int) -> str:
    """Trim a title on a word boundary without changing its wording."""

    collapsed = " ".join(text.split())
    if max_length <= 0 or len(collapsed) <= max_length:
        return collapsed
    cut = collapsed[:max_length]
    boundary = cut.rfind(" ")
    if boundary >= max_length // 2:
        cut = cut[:boundary]
    return cut.rstrip(" ,;:-")


def deduplicate(rows: Iterable[Mapping[str, str]]) -> list[dict[str, str]]:
    """Drop duplicates and sort so the seed always samples the same pool."""

    seen: set[tuple[str, str]] = set()
    unique: list[dict[str, str]] = []
    for row in rows:
        key = (str(row["corpus"]), str(row["id"]))
        if key in seen:
            continue
        seen.add(key)
        unique.append(
            {
                "id": str(row["id"]),
                "corpus": str(row["corpus"]),
                "text": str(row["text"]),
                "origin": str(row.get("origin", "json")),
            }
        )
    unique.sort(key=lambda row: (row["corpus"], row["id"]))
    return unique


def build_queries(
    rows: Sequence[Mapping[str, str]],
    *,
    count: int,
    seed: int,
    max_length: int = DEFAULT_MAX_LENGTH,
    date: str | None = None,
) -> dict[str, Any]:
    """Build the deterministic query document (pure function)."""

    if count <= 0:
        raise lib.Mes00Error("--count must be positive")
    pool = deduplicate(rows)
    if not pool:
        raise lib.Mes00Error("no title row to generate queries from")

    prepared = [
        {
            "source_id": row["id"],
            "corpus": row["corpus"],
            "text": shorten(row["text"], max_length),
        }
        for row in pool
    ]
    prepared = [row for row in prepared if row["text"]]
    if not prepared:
        raise lib.Mes00Error("every title is empty after trimming")

    generator = random.Random(seed)
    selected = generator.sample(prepared, min(count, len(prepared)))
    generator.shuffle(selected)

    queries = [
        {
            "query_id": f"q{index:04d}",
            "corpus": row["corpus"],
            "text": row["text"],
            "source_id": row["source_id"],
        }
        for index, row in enumerate(selected, start=1)
    ]

    canonical = json.dumps(
        [
            {
                "query_id": query["query_id"],
                "corpus": query["corpus"],
                "text": query["text"],
            }
            for query in queries
        ],
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    )
    counts: dict[str, int] = {}
    for query in queries:
        counts[query["corpus"]] = counts.get(query["corpus"], 0) + 1

    return {
        "meta": {
            "schema_version": SCHEMA_VERSION,
            "seed": seed,
            "date": date or datetime.now(timezone.utc).isoformat(),
            "count": len(queries),
            "max_length": max_length,
            "corpora": dict(sorted(counts.items())),
            "sha256": hashlib.sha256(canonical.encode("utf-8")).hexdigest(),
            "origin": "article and summary titles, never a user question",
        },
        "queries": queries,
    }


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, help="output JSON, under /tmp")
    parser.add_argument("--count", type=int, default=DEFAULT_COUNT)
    parser.add_argument("--seed", type=int, default=DEFAULT_SEED)
    parser.add_argument("--max-length", type=int, default=DEFAULT_MAX_LENGTH)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--from-json", help="JSON or JSONL title export")
    source.add_argument("--from-csv", help="CSV title export with id,corpus,text")
    source.add_argument(
        "--from-db",
        action="store_true",
        help="read titles from $PROD_DB_CONTAINER with $PROD_PGOPTIONS",
    )
    parser.add_argument("--sql", help="read-only title query (SELECT or WITH)")
    parser.add_argument("--sql-file", help="file containing the title query")
    parser.add_argument(
        "--allow-root",
        action="append",
        default=[],
        help="additional explicitly allowed output root",
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = _build_parser().parse_args(argv)
    try:
        pgoptions = lib.require_prod_pgoptions()
        forbidden = [root for root in (lib.find_repo_root(),) if root is not None]
        prod_repo = os.environ.get("PROD_REPO", "").strip()
        if prod_repo:
            forbidden.append(lib.resolve_project_dir(prod_repo))
        output = lib.check_output_path(
            args.output,
            allow_roots=args.allow_root,
            forbidden_roots=forbidden,
        )

        if args.from_db:
            if bool(args.sql) == bool(args.sql_file):
                raise lib.Mes00Error(
                    "give exactly one of --sql or --sql-file with --from-db"
                )
            if args.sql_file:
                try:
                    sql = Path(args.sql_file).read_text(encoding="utf-8")
                except OSError as exc:
                    raise lib.Mes00Error(
                        f"cannot read the SQL file: {exc}"
                    ) from exc
            else:
                sql = args.sql or ""
            container = lib.require_env("PROD_DB_CONTAINER")
            rows = rows_from_db(sql, container=container, pgoptions=pgoptions)
        elif args.from_csv:
            rows = read_rows_csv(args.from_csv)
        else:
            rows = read_rows_json(args.from_json)

        document = build_queries(
            rows,
            count=args.count,
            seed=args.seed,
            max_length=args.max_length,
        )
        lib.write_json(output, document)
    except lib.Mes00Error as exc:
        print(f"mes00 gen_requetes: {exc}", file=sys.stderr)
        return 2

    print(
        f"mes00 gen_requetes: {document['meta']['count']} queries -> {output}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
