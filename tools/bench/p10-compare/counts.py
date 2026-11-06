#!/usr/bin/env python3
"""Count pgvector documents and passages per corpus (read-only, on the host).

Protocol (ADR-0140 decision 10): the counts are taken BEFORE and AFTER the
replay, with the date and the time, so a data drift can be told apart from a
loss of ranking. Documents are rows of ``vec.sources``; passages are rows of
``vec.passages``. The per-index sums follow ``queries.INDEX_CORPORA``
(``articles`` and ``decisions`` merge two partitions each), which is the shape
the import report compares against.

Safety (ADR-0032): the session must be read-only, verified by
``SHOW transaction_read_only``; nothing is written but the guarded JSON file.
"""

from __future__ import annotations

import argparse
import os
import sys
from datetime import datetime, timezone
from typing import Any, Mapping, Sequence

import mes00lib as lib
import queries as game

SCHEMA_VERSION = 1

CORPUS_SQL = """
SELECT s.corpus,
       count(*) AS documents,
       coalesce(sum(s.n_passages), 0) AS passages_declares
FROM vec.sources AS s
GROUP BY s.corpus
ORDER BY s.corpus
"""

PASSAGES_SQL = """
SELECT p.corpus,
       count(*) AS passages,
       count(DISTINCT p.src_id) AS documents_dans_passages
FROM vec.passages AS p
GROUP BY p.corpus
ORDER BY p.corpus
"""

VERSION_SQL = """
SELECT (SELECT version()) AS postgres,
       (SELECT extversion FROM pg_extension WHERE extname = 'vector') AS pgvector
"""


def merge_counts(
    sources: Sequence[Mapping[str, Any]], passages: Sequence[Mapping[str, Any]]
) -> dict[str, dict[str, int]]:
    """Join the two counts by corpus; every corpus seen must appear in both."""

    per_corpus: dict[str, dict[str, int]] = {}
    for row in sources:
        per_corpus.setdefault(str(row["corpus"]), {})["documents"] = int(row["documents"])
        per_corpus[str(row["corpus"])]["passages_declares"] = int(row["passages_declares"])
    for row in passages:
        corpus = str(row["corpus"])
        if corpus not in per_corpus:
            raise game.P10Error(f"{corpus}: passages without a source row")
        per_corpus[corpus]["passages"] = int(row["passages"])
        per_corpus[corpus]["documents_dans_passages"] = int(
            row["documents_dans_passages"]
        )
    missing = [c for c, row in per_corpus.items() if "passages" not in row]
    if missing:
        raise game.P10Error(f"corpora without passages: {sorted(missing)}")
    return per_corpus


def snapshot(per_corpus: Mapping[str, Mapping[str, int]], versions: Mapping[str, Any]) -> dict[str, Any]:
    documents = sum(int(row["documents"]) for row in per_corpus.values())
    passages = sum(int(row["passages"]) for row in per_corpus.values())
    return {
        "schema_version": SCHEMA_VERSION,
        "taken_at": datetime.now(timezone.utc).isoformat(),
        "server": dict(versions),
        "documents": documents,
        "passages": passages,
        "per_corpus": {
            corpus: {
                "documents": int(row["documents"]),
                "passages": int(row["passages"]),
                "passages_declares": int(row.get("passages_declares", 0)),
                "documents_dans_passages": int(row.get("documents_dans_passages", 0)),
            }
            for corpus, row in sorted(per_corpus.items())
        },
        "per_index": game.aggregate_indexes(per_corpus),
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, help="count JSON, under /tmp")
    parser.add_argument("--label", default="counts", help="free label (before/after)")
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

        sys.dont_write_bytecode = True
        os.chdir(output.parent)
        os.environ["PGOPTIONS"] = pgoptions
        sys.path.insert(0, str(project))

        import psycopg
        from serveur_mcp import base

        with psycopg.connect(base.DSN, autocommit=True) as connection:
            with connection.cursor() as cursor:
                cursor.execute("SHOW transaction_read_only")
                if cursor.fetchone()[0] != "on":
                    raise lib.Mes00Error(
                        "transaction_read_only is not on; refusing to measure (ADR-0032)"
                    )
                cursor.execute(VERSION_SQL)
                version_row = cursor.fetchone()
                versions = {"postgres": version_row[0], "pgvector": version_row[1]}
                cursor.execute(CORPUS_SQL)
                sources = [
                    {
                        "corpus": corpus,
                        "documents": documents,
                        "passages_declares": passages_declares,
                    }
                    for corpus, documents, passages_declares in cursor.fetchall()
                ]
                cursor.execute(PASSAGES_SQL)
                passages = [
                    {
                        "corpus": corpus,
                        "passages": total,
                        "documents_dans_passages": documents,
                    }
                    for corpus, total, documents in cursor.fetchall()
                ]

        document = snapshot(merge_counts(sources, passages), versions)
        document["label"] = args.label
        lib.write_json(output, document)
    except (lib.Mes00Error, game.P10Error) as exc:
        print(f"p10-compare counts: {exc}", file=sys.stderr)
        return 2

    print(
        f"p10-compare counts: {document['documents']} documents, "
        f"{document['passages']} passages -> {output}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
