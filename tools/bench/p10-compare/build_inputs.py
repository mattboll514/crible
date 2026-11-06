#!/usr/bin/env python3
"""Build the 97-entry input of the pgvector replay (runs on the development machine).

Joins ``mcp.jsonl`` (the 97 MCP calls) with their texts: the six versioned
``<index>.jsonl`` files for the 73 ``Q*`` lines, and ``data/mes00/queries.json``
for the 24 ``MES00-qNNNN`` lines. Writes one JSON document under ``data/``
(gitignored, ADR-0125 decision 16) and prints its SHA-256.

The output carries query texts and filter values, so it is an input file, never
a report: only ``id`` and ``doc_id`` may leave for ``docs/mesures/``.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

import queries as game


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--reference-dir",
        default="docs/legifacile/reference",
        help="folder holding the six <index>.jsonl files and mcp.jsonl",
    )
    parser.add_argument(
        "--mes00-queries",
        default="data/mes00/queries.json",
        help="MES-00 query file, source of the 24 MES00-qNNNN texts",
    )
    parser.add_argument(
        "--output",
        default="data/p10-compare/queries.json",
        help="input file to write (under data/, gitignored)",
    )
    args = parser.parse_args(argv)

    try:
        reference = Path(args.reference_dir)
        built = game.build_queries(
            reference_dir=reference,
            mcp_path=reference / "mcp.jsonl",
            mes00_path=Path(args.mes00_queries),
        )
        output = Path(args.output)
        output.parent.mkdir(parents=True, exist_ok=True)
        document = {
            "meta": {
                "schema_version": game.SCHEMA_VERSION,
                "built_at": datetime.now(timezone.utc).isoformat(),
                "count": len(built),
                "mcp_sha256": sha256_of(reference / "mcp.jsonl"),
                "source_counts": {
                    source: sum(1 for row in built if row["text_source"] == source)
                    for source in ("reference", "mes00")
                },
            },
            "queries": built,
        }
        with open(output, "w", encoding="utf-8") as handle:
            json.dump(document, handle, ensure_ascii=False, indent=2)
            handle.write("\n")
    except game.P10Error as exc:
        print(f"p10-compare inputs: {exc}", file=sys.stderr)
        return 2

    print(f"p10-compare inputs: {len(built)} queries -> {output}", file=sys.stderr)
    print(f"sha256 {sha256_of(output)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
