#!/usr/bin/env python3
"""MES-00: encode a small query sample for the HNSW fidelity measurement.

Reads the query set produced by ``gen_requetes.py``, keeps the first
``--per-corpus`` queries of every corpus (deterministic: the query file is
already shuffled by a fixed seed), encodes them with the production encoder,
and writes ``{query_id, corpus, vector}``. No query text reaches the output:
only vectors, which the exact scan needs.

The encoder is the one production uses (``serveur_mcp.encodage``), loaded from
the Legifacile checkout given by ``$PROD_REPO`` or ``--project``. The process
changes its working directory to the guarded output folder and disables
bytecode writing before importing the production module.
"""

from __future__ import annotations

import argparse
import os
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Mapping, Sequence

import mes00lib as lib

SCHEMA_VERSION = 1


def select(queries: Sequence[Mapping[str, Any]], per_corpus: int) -> list[dict[str, str]]:
    counts: dict[str, int] = {}
    selected: list[dict[str, str]] = []
    for raw in queries:
        if not isinstance(raw, Mapping):
            raise lib.Mes00Error("a query is not an object")
        corpus = raw.get("corpus")
        text = raw.get("text", raw.get("texte"))
        if not isinstance(corpus, str) or not corpus.strip():
            raise lib.Mes00Error("a query has no corpus")
        if not isinstance(text, str) or not text.strip():
            raise lib.Mes00Error("a query has no text")
        corpus = corpus.strip()
        if counts.get(corpus, 0) >= per_corpus:
            continue
        counts[corpus] = counts.get(corpus, 0) + 1
        selected.append(
            {
                "query_id": str(raw.get("query_id", raw.get("requete_id", ""))),
                "corpus": corpus,
                "text": text,
            }
        )
    return selected


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--queries", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--per-corpus", type=int, default=3)
    parser.add_argument("--project", default=os.environ.get("PROD_REPO", ""))
    parser.add_argument("--allow-root", action="append", default=[])
    args = parser.parse_args(argv)

    try:
        pgoptions = lib.require_prod_pgoptions()
        project = lib.resolve_project_dir(
            args.project or lib.require_env("PROD_REPO")
        )
        forbidden = [project]
        repo_root = lib.find_repo_root()
        if repo_root is not None:
            forbidden.append(repo_root)
        output = lib.check_output_path(
            args.output, allow_roots=args.allow_root, forbidden_roots=forbidden
        )
        lib.prepare_output(output)

        payload = lib.read_json(args.queries)
        queries = (
            payload.get("queries", payload.get("requetes"))
            if isinstance(payload, Mapping)
            else payload
        )
        if not isinstance(queries, list) or not queries:
            raise lib.Mes00Error("the query file has no 'queries' array")
        meta = payload.get("meta", {}) if isinstance(payload, Mapping) else {}

        if args.per_corpus <= 0:
            raise lib.Mes00Error("--per-corpus must be positive")
        selected = select(queries, args.per_corpus)
        if not selected:
            raise lib.Mes00Error("no query selected")

        sys.dont_write_bytecode = True
        os.chdir(output.parent)
        os.environ["PGOPTIONS"] = pgoptions
        sys.path.insert(0, str(project))
        from serveur_mcp import encodage

        encodage.charger()
        document = {
            "meta": {
                "schema_version": SCHEMA_VERSION,
                "date": datetime.now(timezone.utc).isoformat(),
                "seed": meta.get("seed", meta.get("graine")),
                "query_sha256": meta.get(
                    "source_sha256",
                    meta.get("sha256", meta.get("empreinte_sha256")),
                ),
                "per_corpus": args.per_corpus,
                "count": len(selected),
            },
            "queries": [],
        }
        for query in selected:
            vector = encodage.encoder_requete(query["text"])
            document["queries"].append(
                {
                    "query_id": query["query_id"],
                    "corpus": query["corpus"],
                    "vector": vector,
                }
            )
        lib.write_json(output, document)
    except lib.Mes00Error as exc:
        print(f"mes00 vectors: {exc}", file=sys.stderr)
        return 2

    print(f"mes00 vectors: {len(selected)} vectors -> {output}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
