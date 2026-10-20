#!/usr/bin/env python3
"""Select a deterministic subset of an existing MES-00 query set.

Used to build the perimeter campaign: the multi-corpus perimeters are measured
on the first ``--per-corpus`` queries of each selected corpus, in the order of
the input file. The input file was already shuffled by ``gen_requetes.py`` with
a fixed seed, so the selection is reproducible without another random draw.

The output keeps the source fingerprint and records the selection rule. Query
texts stay in the file (they are titles), but are never written by the latency
harness itself.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Mapping, Sequence

import mes00lib as lib

SCHEMA_VERSION = 1


def load_queries(path: str | os.PathLike[str]) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    payload = lib.read_json(path)
    if isinstance(payload, Mapping):
        queries = payload.get("queries", payload.get("requetes"))
        meta = payload.get("meta", {})
    else:
        queries, meta = payload, {}
    if not isinstance(queries, list) or not queries:
        raise lib.Mes00Error("the input file has no 'queries' array")

    normalised: list[dict[str, Any]] = []
    for index, raw in enumerate(queries, start=1):
        if not isinstance(raw, Mapping):
            raise lib.Mes00Error(f"query #{index} is not an object")
        text = raw.get("text", raw.get("texte"))
        corpus = raw.get("corpus")
        if not isinstance(text, str) or not text.strip():
            raise lib.Mes00Error(f"query #{index} has no text")
        if not isinstance(corpus, str) or not corpus.strip():
            raise lib.Mes00Error(f"query #{index} has no corpus")
        query_id = raw.get("query_id", raw.get("requete_id", f"q{index:04d}"))
        normalised.append(
            {
                "query_id": str(query_id),
                "corpus": corpus.strip(),
                "text": text,
                "source_id": raw.get("source_id"),
            }
        )
    return (dict(meta) if isinstance(meta, Mapping) else {}), normalised


def select(
    queries: Sequence[Mapping[str, Any]],
    *,
    corpora: Sequence[str],
    per_corpus: int | None,
) -> list[dict[str, Any]]:
    """Keep the first ``per_corpus`` queries of each selected corpus."""

    if not corpora:
        raise lib.Mes00Error("give at least one --corpus")
    if per_corpus is not None and per_corpus <= 0:
        raise lib.Mes00Error("--per-corpus must be positive")
    wanted = set(corpora)
    counts: dict[str, int] = {}
    selected: list[dict[str, Any]] = []
    for query in queries:
        corpus = str(query["corpus"])
        if corpus not in wanted:
            continue
        if per_corpus is not None and counts.get(corpus, 0) >= per_corpus:
            continue
        counts[corpus] = counts.get(corpus, 0) + 1
        selected.append(dict(query))
    if not selected:
        raise lib.Mes00Error("no query selected")
    return selected


def fingerprint(queries: Sequence[Mapping[str, Any]]) -> str:
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
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def build_document(
    meta: Mapping[str, Any],
    queries: Sequence[Mapping[str, Any]],
    *,
    corpora: Sequence[str],
    per_corpus: int | None,
) -> dict[str, Any]:
    counts: dict[str, int] = {}
    for query in queries:
        counts[str(query["corpus"])] = counts.get(str(query["corpus"]), 0) + 1
    return {
        "meta": {
            "schema_version": SCHEMA_VERSION,
            "date": datetime.now(timezone.utc).isoformat(),
            "seed": meta.get("seed", meta.get("graine")),
            "source_sha256": meta.get("sha256", meta.get("empreinte_sha256")),
            "count": len(queries),
            "corpora": dict(sorted(counts.items())),
            "selection": {
                "corpora": list(corpora),
                "per_corpus": per_corpus,
                "order": "input file order",
            },
            "sha256": fingerprint(queries),
        },
        "queries": list(queries),
    }


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, help="source query JSON")
    parser.add_argument("--output", required=True, help="output JSON, under /tmp")
    parser.add_argument("--corpus", action="append", default=[])
    parser.add_argument("--per-corpus", type=int)
    parser.add_argument(
        "--allow-root", action="append", default=[], help="additional output root"
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = _build_parser().parse_args(argv)
    try:
        protected = [root for root in (lib.find_repo_root(),) if root is not None]
        prod_repo = os.environ.get("PROD_REPO", "").strip()
        if prod_repo:
            protected.append(lib.resolve_project_dir(prod_repo))
        output = lib.check_output_path(
            args.output, allow_roots=args.allow_root, forbidden_roots=protected
        )
        meta, queries = load_queries(args.input)
        selected = select(
            queries, corpora=args.corpus, per_corpus=args.per_corpus
        )
        lib.write_json(
            output,
            build_document(
                meta,
                selected,
                corpora=args.corpus,
                per_corpus=args.per_corpus,
            ),
        )
    except lib.Mes00Error as exc:
        print(f"mes00 filter_queries: {exc}", file=sys.stderr)
        return 2

    print(
        f"mes00 filter_queries: {len(selected)} queries -> {output}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
