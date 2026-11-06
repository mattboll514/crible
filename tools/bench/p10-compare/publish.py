#!/usr/bin/env python3
"""Assemble the publishable pgvector report (runs on the development machine).

Reads the full replay report and the two count snapshots, and writes the single
publishable form of ADR-0125 decision 16 / ADR-0140 decision 11: one line per
query with its ``id`` and its ``doc_id`` list, the latency percentiles and the
counts. No query text, no filter value, no production path.

The input files stay under ``data/`` (gitignored); only the output may be
committed under ``docs/mesures/``.
"""

from __future__ import annotations

import argparse
import sys

import queries as game


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--replay", required=True, help="full replay report")
    parser.add_argument("--counts-before", required=True)
    parser.add_argument("--counts-after", required=True)
    parser.add_argument("--output", required=True, help="publishable JSON")
    parser.add_argument(
        "--machine",
        default=(
            "Mac mini de production (Apple M1, 8 cœurs, 16 Go), PostgreSQL 17 "
            "dans Docker Desktop ; latence indicative (ADR-0129, décision 7 point 5)"
        ),
    )
    parser.add_argument("--fixed-date", default="2026-11-03")
    parser.add_argument(
        "--vectors",
        help="vector file of encode_queries.py, for the encoder-only latencies",
    )
    args = parser.parse_args(argv)

    try:
        replay = game.read_json(args.replay)
        counts_before = game.read_json(args.counts_before)
        counts_after = game.read_json(args.counts_after)
        encoder = None
        if args.vectors:
            vectors = game.read_json(args.vectors)
            encoder = (vectors.get("meta") or {}).get("encode_ms")
        document = game.build_publishable(
            replay,
            counts_before,
            counts_after,
            machine=args.machine,
            fixed_date=args.fixed_date,
            encoder=encoder,
        )
        with open(args.output, "w", encoding="utf-8") as handle:
            handle.write(_render(document))
    except game.P10Error as exc:
        print(f"p10-compare publish: {exc}", file=sys.stderr)
        return 2

    print(
        f"p10-compare publish: {len(document['queries'])} queries -> {args.output}",
        file=sys.stderr,
    )
    return 0


def _render(document: dict) -> str:
    import json

    return json.dumps(document, ensure_ascii=False, indent=2, sort_keys=False) + "\n"


if __name__ == "__main__":
    raise SystemExit(main())
