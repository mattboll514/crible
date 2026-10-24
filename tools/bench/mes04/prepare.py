# SPDX-License-Identifier: Apache-2.0

"""Prepare the query list of MES-04 for the Rust harness.

``data/legifacile-sample/queries.jsonl`` carries the 300 reference queries with
their metadata (``corpus``, ``structure``, ``expected``…). The Rust harness only
needs an opaque identifier and a text, so this script writes a reduced JSONL to
``data/mes04/`` (gitignored). No query text is printed, and the reduced file
never enters the repository: the measurement reports only aggregates.

Run from ``tools/``::

    uv run python bench/mes04/prepare.py
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

#: Repository root, three levels above this file.
ROOT = Path(__file__).resolve().parents[3]
#: Reference queries of the Legifacile sample.
DEFAULT_QUERIES = ROOT / "data/legifacile-sample/queries.jsonl"
#: Working directory of the campaign (gitignored).
DEFAULT_OUT = ROOT / "data/mes04"


def read_queries(path: Path) -> list[dict]:
    """Returns the 300 query records of ``queries.jsonl``."""
    records = []
    with path.open(encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, start=1):
            if not line.strip():
                continue
            record = json.loads(line)
            missing = {"id", "question"} - record.keys()
            if missing:
                raise ValueError(f"line {line_number} lacks {sorted(missing)}")
            records.append(record)
    if not records:
        raise ValueError(f"{path} holds no query")
    return records


def write_query_texts(records: list[dict], out: Path) -> None:
    """Writes ``{"id", "text"}`` lines, dropping every other field."""
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w", encoding="utf-8") as handle:
        for record in records:
            payload = {"id": record["id"], "text": record["question"]}
            handle.write(json.dumps(payload, ensure_ascii=False) + "\n")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--queries", type=Path, default=DEFAULT_QUERIES)
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    arguments = parser.parse_args(argv)
    try:
        records = read_queries(arguments.queries)
        target = arguments.out / "query-texts.jsonl"
        write_query_texts(records, target)
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    print(f"wrote {len(records)} query texts to {target}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
