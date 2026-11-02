# SPDX-License-Identifier: Apache-2.0
"""Convert the Legifacile sample questions into the input of `crible eval`.

ADR-0125, decision 3: the input of `crible eval` is a closed JSONL file, one
query per line, `{"id", "query" | "vector", "filter"?, "expected"?}`. The sample
file `data/legifacile-sample/queries.jsonl` carries another shape — `id`,
`question`, `corpus`, `structure`, `satisfiable` and an `expected` list of
objects — so this harness converts it, and nothing else.

Mapping (ADR-0125, decision 3):

- `id` for `id`, the identifier of the query set;
- `question` for `query`, the text `crible eval` encodes with the model of the
  index;
- `expected[].src_id` for `expected`, the document identifiers the query set
  expects (decision 4: the quality is judged at the document).

One output file per structure, because an index serves one structure (ADR-0003):
`<out>/<structure>.jsonl`. The file name is the game of the report, so
`articles.jsonl` produces `eval-articles-<AAAA-MM-JJ>.json`.

Usage, from the repository root::

    (cd tools && uv run --no-sync python bench/convert_queries.py \\
        --queries ../data/legifacile-sample/queries.jsonl \\
        --out ../data/eval-queries)

Neither stdout nor the files carry more than identifiers, counts and paths: the
harness prints the games and their size, never a question text, and the
converted files stay under `data/`, which is gitignored (ADR-0125, decision 16).
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]


def convert_line(row: Any, number: int) -> tuple[str, dict[str, Any]]:
    """One eval line and its structure, from one sample question.

    `number` is the 1-based line of the question in the input file, named by
    every refusal. The message never repeats the text of a question.
    """
    if not isinstance(row, dict):
        raise SystemExit(f"line {number}: a question must be a JSON object")
    identifier = row.get("id")
    if not isinstance(identifier, str) or not identifier:
        raise SystemExit(f"line {number}: `id` is missing or is not a non-empty string")
    structure = row.get("structure")
    if not isinstance(structure, str) or not structure:
        raise SystemExit(f"line {number} ({identifier}): `structure` is missing")
    question = row.get("question")
    if not isinstance(question, str) or not question:
        raise SystemExit(f"line {number} ({identifier}): `question` is missing")
    expected: list[str] = []
    raw_expected = row.get("expected")
    if raw_expected is not None:
        if not isinstance(raw_expected, list):
            raise SystemExit(f"line {number} ({identifier}): `expected` must be a list")
        for entry in raw_expected:
            if not isinstance(entry, dict):
                raise SystemExit(
                    f"line {number} ({identifier}): an `expected` entry must be an object"
                )
            source = entry.get("src_id")
            if not isinstance(source, str) or not source:
                raise SystemExit(
                    f"line {number} ({identifier}): an `expected` entry has no `src_id`"
                )
            if source not in expected:
                expected.append(source)
    line: dict[str, Any] = {"id": identifier, "query": question}
    if expected:
        line["expected"] = expected
    return structure, line


def read_rows(path: Path) -> list[tuple[int, Any]]:
    """The questions of `path`, with their 1-based line, blanks skipped."""
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as error:
        raise SystemExit(f"cannot read {path}: {error}") from error
    rows = []
    for number, raw in enumerate(text.splitlines(), start=1):
        if not raw.strip():
            continue
        try:
            rows.append((number, json.loads(raw)))
        except json.JSONDecodeError as error:
            raise SystemExit(f"line {number}: invalid JSON: {error.msg}") from error
    return rows


def convert(
    rows: list[tuple[int, Any]], structures: list[str] | None = None
) -> dict[str, list[dict[str, Any]]]:
    """The eval lines of every kept structure, in file order, keyed by structure."""
    games: dict[str, list[dict[str, Any]]] = {}
    present: set[str] = set()
    seen: set[str] = set()
    for number, row in rows:
        structure, line = convert_line(row, number)
        present.add(structure)
        if line["id"] in seen:
            raise SystemExit(f"line {number} ({line['id']}): `id` is not unique")
        seen.add(line["id"])
        if structures is not None and structure not in structures:
            continue
        games.setdefault(structure, []).append(line)
    if structures is not None:
        missing = sorted(set(structures) - present)
        if missing:
            raise SystemExit(f"no question of structure {', '.join(missing)}")
    return games


def write_games(games: dict[str, list[dict[str, Any]]], out: Path) -> list[Path]:
    """Writes one `<structure>.jsonl` per game and returns the paths written."""
    out.mkdir(parents=True, exist_ok=True)
    written = []
    for structure in sorted(games):
        path = out / f"{structure}.jsonl"
        body = "".join(json.dumps(line, ensure_ascii=False) + "\n" for line in games[structure])
        path.write_text(body, encoding="utf-8")
        written.append(path)
    return written


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--queries",
        type=Path,
        default=ROOT / "data" / "legifacile-sample" / "queries.jsonl",
        help="sample questions, one JSON object per line",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=ROOT / "data" / "eval-queries",
        help="directory of the converted games (stays under data/)",
    )
    parser.add_argument(
        "--structure",
        action="append",
        default=None,
        help="convert that structure only; the flag is repeatable",
    )
    options = parser.parse_args()

    games = convert(read_rows(options.queries), options.structure)
    written = write_games(games, options.out)
    total = sum(len(lines) for lines in games.values())
    print(f"{total} queries in {len(games)} game(s), written under {options.out}")
    for structure in sorted(games):
        print(f"  {structure}: {len(games[structure])}")
    for path in written:
        print(f"  {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
