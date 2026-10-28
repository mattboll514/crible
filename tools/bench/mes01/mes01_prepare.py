# SPDX-License-Identifier: Apache-2.0

"""Prepare the query lists of MES-01 and MES-12 (fidelity of the binary scan).

``data/legifacile-sample/queries.jsonl`` carries the 300 reference queries and
their ``structure``. This script writes, under ``data/mes01/`` (gitignored):

- ``query-texts.jsonl``: the ``{"id", "text"}`` list the encoder harness reads,
  all 300 queries in file order;
- ``queries-main.jsonl``: the main campaign, one ``{"id", "structure", "filter"}``
  per query, without any filter;
- ``queries-filtered.jsonl``: the declared synthetic filtered variant, with
  ``{"etat": "VIGUEUR"}`` for ``articles`` and ``{"partie": "motifs"}`` for
  ``decisions`` (ADR-0076, decision 5). The reference query set is not modified.

No query text is printed. Run from ``tools/``::

    uv run python bench/mes01/mes01_prepare.py
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

#: Repository root, three levels above this file.
ROOT = Path(__file__).resolve().parents[3]
#: Reference queries of the Legifacile sample.
DEFAULT_QUERIES = ROOT / "data/legifacile-sample/queries.jsonl"
#: Working directory of the campaign (gitignored).
DEFAULT_OUT = ROOT / "data/mes01"

#: Filters of the synthetic filtered variant, by structure (ADR-0076).
FILTERS = {
    "articles": {"etat": "VIGUEUR"},
    "decisions": {"partie": "motifs"},
}


def read_queries(path: Path) -> list[dict]:
    """Returns the query records of ``queries.jsonl``, in file order."""
    records = []
    with path.open(encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, start=1):
            if not line.strip():
                continue
            record = json.loads(line)
            missing = {"id", "question", "structure"} - record.keys()
            if missing:
                raise SystemExit(
                    f"line {line_number} lacks {sorted(missing)} in {path}"
                )
            records.append(record)
    if not records:
        raise SystemExit(f"{path} holds no query")
    identifiers = [record["id"] for record in records]
    if len(set(identifiers)) != len(identifiers):
        raise SystemExit(f"{path} repeats a query identifier")
    return records


def write_jsonl(path: Path, rows: list[dict]) -> None:
    """Writes one compact JSON object per line, with a trailing newline."""
    with path.open("w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, ensure_ascii=False) + "\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--queries", type=Path, default=DEFAULT_QUERIES)
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    arguments = parser.parse_args()

    records = read_queries(arguments.queries)
    arguments.out.mkdir(parents=True, exist_ok=True)

    texts = [{"id": record["id"], "text": record["question"]} for record in records]
    write_jsonl(arguments.out / "query-texts.jsonl", texts)

    main_rows = [
        {"id": record["id"], "structure": record["structure"], "filter": None}
        for record in records
    ]
    write_jsonl(arguments.out / "queries-main.jsonl", main_rows)

    filtered_rows = [
        {
            "id": record["id"],
            "structure": record["structure"],
            "filter": FILTERS[record["structure"]],
        }
        for record in records
        if record["structure"] in FILTERS
    ]
    write_jsonl(arguments.out / "queries-filtered.jsonl", filtered_rows)

    by_structure: dict[str, int] = {}
    for record in records:
        by_structure[record["structure"]] = by_structure.get(record["structure"], 0) + 1
    print(f"{len(records)} queries: {json.dumps(by_structure, sort_keys=True)}")
    print(f"{len(filtered_rows)} filtered queries: {json.dumps(FILTERS, sort_keys=True)}")
    print(f"wrote {arguments.out}")


if __name__ == "__main__":
    main()
