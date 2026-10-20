#!/usr/bin/env python3
"""Measure HNSW fidelity against exact search for MES-00.

The script compares, for each query and corpus, the ids returned by pgvector's
HNSW index with the ids returned by an exact scan, and reports recall@k.

Two modes for the exact side:

``export`` (preferred)
    Uses a one-off export of the vectors produced outside production by
    Legifacile's ``scripts/db/exporter_vecteurs.py``. The export directory
    contains one ``<corpus>.npz`` per corpus (arrays ``ids`` and ``vectors``)
    or one ``<corpus>/`` directory per corpus with ``vectors.npy`` and
    ``ids.jsonl``. The vectors are read with numpy, in chunks, so a corpus
    larger than RAM can be scanned.

``sql`` (fallback)
    Runs an exact SQL scan at night on a small sample of queries. The exact
    query must disable the HNSW index (for example with
    ``SET LOCAL enable_indexscan=off``) and is capped by ``--max-queries``.

In both modes the HNSW side is queried on production through a command
template. Every command runs serially, with ``PGOPTIONS=$PROD_PGOPTIONS``.

Input: a JSON array (or an object with a ``queries`` array) of objects with
``query_id``, ``corpus`` and ``vector`` (a list of floats). Query texts are
never needed and never written to the output.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import re
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Mapping, Sequence

import mes00lib as lib

SCHEMA_VERSION = 1
DEFAULT_K = 10
DEFAULT_MAX_SQL_QUERIES = 25
HARD_MAX_SQL_QUERIES = 100
DEFAULT_CHUNK_SIZE = 65_536


# ---------------------------------------------------------------------------
# Pure computations
# ---------------------------------------------------------------------------


def recall_at_k(exact_ids: Sequence[str], approx_ids: Sequence[str], k: int) -> float:
    """Recall of the approximate top-k against the exact top-k."""

    if k <= 0:
        raise lib.Mes00Error("k must be positive")
    exact = list(dict.fromkeys(str(identifier) for identifier in exact_ids))[:k]
    if not exact:
        raise lib.Mes00Error("the exact result is empty; recall is undefined")
    approximate = set(str(identifier) for identifier in approx_ids[:k])
    return len(set(exact) & approximate) / len(exact)


def normalise_vector(raw: Any, *, label: str) -> list[float]:
    """Validate a query vector before it reaches a SQL literal."""

    if not isinstance(raw, list) or not raw:
        raise lib.Mes00Error(f"{label}: 'vector' must be a non-empty array")
    vector: list[float] = []
    for value in raw:
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise lib.Mes00Error(f"{label}: vector components must be numbers")
        number = float(value)
        if not math.isfinite(number):
            raise lib.Mes00Error(f"{label}: vector components must be finite")
        vector.append(number)
    if not any(component != 0.0 for component in vector):
        raise lib.Mes00Error(f"{label}: the zero vector cannot be searched")
    return vector


def vector_literal(vector: Sequence[float]) -> str:
    """Render a vector for a pgvector SQL literal (no quotes, no cast)."""

    return "[" + ",".join(f"{value:.8g}" for value in vector) + "]"


_CORPUS_NAME = re.compile(r"^[a-z0-9_]+$")
_EF_SEARCH = re.compile(r"hnsw\.ef_search\s*=\s*(\d+)", re.IGNORECASE)


def ef_search_of(template: Sequence[str]) -> int | None:
    """Return the ``SET LOCAL hnsw.ef_search`` value of a command template.

    The value lives in the template, never in an environment variable, so it is
    safe to record it in the output metadata.
    """

    for element in template:
        match = _EF_SEARCH.search(element)
        if match:
            return int(match.group(1))
    return None


def template_fingerprint(template: Sequence[str]) -> str:
    """Return the SHA-256 of an argv template, with placeholders left in place.

    This distinguishes two fidelity runs that only differ by ``ef_search`` or by
    the exact-scan settings, even when their output file names are similar. Only
    the template is hashed: no environment value is ever included.
    """

    payload = json.dumps(list(template), ensure_ascii=True, separators=(",", ":"))
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def _safe_corpus(corpus: str) -> str:
    """Accept only corpus names that are safe in a path and in SQL."""

    if not _CORPUS_NAME.fullmatch(corpus):
        raise lib.Mes00Error(f"invalid corpus name in query file: {corpus!r}")
    return corpus


def load_query_vectors(path: str | os.PathLike[str]) -> list[dict[str, Any]]:
    payload = lib.read_json(path)
    if isinstance(payload, Mapping):
        queries = payload.get("queries", payload.get("requetes"))
    else:
        queries = payload
    if not isinstance(queries, list) or not queries:
        raise lib.Mes00Error("the query file has no 'queries' array")

    result: list[dict[str, Any]] = []
    for index, raw in enumerate(queries, start=1):
        if not isinstance(raw, Mapping):
            raise lib.Mes00Error(f"query #{index} is not an object")
        corpus = raw.get("corpus")
        if not isinstance(corpus, str) or not corpus.strip():
            raise lib.Mes00Error(f"query #{index} has no corpus")
        label = f"query {raw.get('query_id', raw.get('requete_id', index))}"
        result.append(
            {
                "query_id": str(
                    raw.get("query_id", raw.get("requete_id", f"q{index:04d}"))
                ),
                "corpus": _safe_corpus(corpus.strip()),
                "vector": normalise_vector(
                    raw.get("vector", raw.get("vecteur")), label=label
                ),
            }
        )
    return result


def _as_id(value: Any) -> str:
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="strict")
    return str(value)


def _ids_from_payload(payload: Any, *, label: str) -> list[str]:
    if isinstance(payload, Mapping):
        payload = payload.get("ids", payload.get("results", payload.get("resultats")))
    if not isinstance(payload, list):
        raise lib.Mes00Error(f"{label}: expected a JSON array or an object with 'ids'")
    identifiers: list[str] = []
    for item in payload:
        if isinstance(item, Mapping):
            value = item.get("id", item.get("doc_id"))
        else:
            value = item
        if value is None or isinstance(value, (dict, list, bool)):
            raise lib.Mes00Error(f"{label}: each result must carry an id")
        identifiers.append(_as_id(value))
    return identifiers


# ---------------------------------------------------------------------------
# Exact search over an export
# ---------------------------------------------------------------------------


def _read_ids_file(path: Path) -> list[str]:
    if path.suffix.lower() == ".jsonl":
        try:
            return [
                _as_id(json.loads(line))
                for line in path.read_text(encoding="utf-8").splitlines()
                if line.strip()
            ]
        except (OSError, json.JSONDecodeError) as exc:
            raise lib.Mes00Error(f"cannot read {path.name}: {exc}") from exc
    payload = lib.read_json(path)
    if not isinstance(payload, list):
        raise lib.Mes00Error(f"{path.name} must contain a JSON array of ids")
    return [_as_id(item) for item in payload]


def _load_corpus_export(export_dir: Path, corpus: str) -> tuple[list[str], Any]:
    """Return ``(ids, vectors)`` for one corpus, without loading it eagerly."""

    import numpy as np  # imported lazily: the SQL mode does not need numpy

    name = _safe_corpus(corpus)
    directory = export_dir / name
    npz_path = export_dir / f"{name}.npz"

    if directory.is_dir():
        vector_path = directory / "vectors.npy"
        if not vector_path.is_file():
            raise lib.Mes00Error(f"missing vectors.npy for corpus {name}")
        ids_path = directory / "ids.jsonl"
        if not ids_path.is_file():
            ids_path = directory / "ids.json"
        if not ids_path.is_file():
            raise lib.Mes00Error(f"missing ids.jsonl for corpus {name}")
        identifiers = _read_ids_file(ids_path)
        vectors = np.load(vector_path, mmap_mode="r")
    elif npz_path.is_file():
        try:
            archive = np.load(npz_path, allow_pickle=False)
            identifiers = [_as_id(item) for item in archive["ids"]]
            vectors = archive["vectors"] if "vectors" in archive else archive["vecteurs"]
        except (OSError, KeyError, ValueError) as exc:
            raise lib.Mes00Error(f"cannot read {npz_path.name}: {exc}") from exc
    else:
        raise lib.Mes00Error(
            f"no export for corpus {name} under the export directory "
            "(expected <corpus>.npz or <corpus>/vectors.npy)"
        )

    if vectors.ndim != 2:
        raise lib.Mes00Error(f"vectors for corpus {name} must be a 2-D array")
    if len(identifiers) != vectors.shape[0]:
        raise lib.Mes00Error(
            f"corpus {name}: {len(identifiers)} ids for {vectors.shape[0]} vectors"
        )
    return identifiers, vectors


def exact_topk_from_vectors(
    query: Sequence[float],
    identifiers: Sequence[str],
    vectors: Any,
    *,
    k: int,
    chunk_size: int = DEFAULT_CHUNK_SIZE,
) -> list[str]:
    """Cosine exact top-k, scanned in chunks to bound memory use."""

    import numpy as np

    if k <= 0:
        raise lib.Mes00Error("k must be positive")
    if chunk_size <= 0:
        raise lib.Mes00Error("chunk size must be positive")
    if vectors.shape[0] != len(identifiers):
        raise lib.Mes00Error("the id list and the vector array disagree")
    if vectors.shape[1] != len(query):
        raise lib.Mes00Error(
            f"query dimension {len(query)} does not match vectors "
            f"({vectors.shape[1]})"
        )

    query_array = np.asarray(query, dtype=np.float32)
    query_norm = float(np.linalg.norm(query_array))
    if query_norm == 0.0:
        raise lib.Mes00Error("the zero vector cannot be searched")

    best_scores = np.empty(0, dtype=np.float32)
    best_indices = np.empty(0, dtype=np.int64)
    for start in range(0, vectors.shape[0], chunk_size):
        stop = min(start + chunk_size, vectors.shape[0])
        block = np.asarray(vectors[start:stop], dtype=np.float32)
        norms = np.linalg.norm(block, axis=1)
        scores = np.divide(
            block @ query_array,
            norms * query_norm,
            out=np.full(block.shape[0], -np.inf, dtype=np.float32),
            where=norms > 0.0,
        )
        local_k = min(k, scores.shape[0])
        local = np.argpartition(scores, -local_k)[-local_k:]
        merged_scores = np.concatenate((best_scores, scores[local]))
        merged_indices = np.concatenate(
            (best_indices, local.astype(np.int64) + start)
        )
        keep = min(k, merged_scores.shape[0])
        order = np.argpartition(merged_scores, -keep)[-keep:]
        order = order[np.argsort(merged_scores[order])[::-1]]
        best_scores = merged_scores[order]
        best_indices = merged_indices[order]

    return [str(identifiers[int(index)]) for index in best_indices]


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--queries", required=True, help="query vectors JSON")
    parser.add_argument("--output", required=True, help="output JSON, under /tmp")
    parser.add_argument("--mode", choices=("export", "sql"), required=True)
    parser.add_argument("--export-dir", help="vector export directory (export mode)")
    parser.add_argument(
        "--hnsw-command-json",
        required=True,
        help=(
            "JSON argv template for the HNSW query; placeholders "
            "{vector_literal}, {corpus}, {query_id}, {container}"
        ),
    )
    parser.add_argument(
        "--exact-command-json",
        help="JSON argv template for the exact SQL scan (sql mode)",
    )
    parser.add_argument("--k", type=int, default=DEFAULT_K)
    parser.add_argument("--chunk-size", type=int, default=DEFAULT_CHUNK_SIZE)
    parser.add_argument("--max-queries", type=int, default=DEFAULT_MAX_SQL_QUERIES)
    parser.add_argument("--timeout", type=float, default=300.0)
    parser.add_argument(
        "--allow-root", action="append", default=[], help="additional output root"
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
        lib.prepare_output(output)

        queries = load_query_vectors(args.queries)
        if args.mode == "sql":
            if not args.exact_command_json:
                raise lib.Mes00Error(
                    "--mode sql requires --exact-command-json"
                )
            if args.max_queries > HARD_MAX_SQL_QUERIES:
                raise lib.Mes00Error(
                    f"--max-queries must not exceed {HARD_MAX_SQL_QUERIES} "
                    "for an exact scan on production"
                )
            if len(queries) > args.max_queries:
                raise lib.Mes00Error(
                    f"{len(queries)} queries given for an exact SQL scan; "
                    f"reduce to {args.max_queries} or fewer"
                )
            exact_template = lib.command_from_json(args.exact_command_json)
        else:
            if not args.export_dir:
                raise lib.Mes00Error("--mode export requires --export-dir")
            export_dir = Path(args.export_dir)
            if not export_dir.is_dir():
                raise lib.Mes00Error("--export-dir must be an existing directory")
            exact_template = None

        hnsw_template = lib.command_from_json(args.hnsw_command_json)
        used_templates = hnsw_template + list(exact_template or [])
        container = os.environ.get("PROD_DB_CONTAINER", "").strip()
        if any("{container}" in element for element in used_templates):
            container = lib.require_env("PROD_DB_CONTAINER")
        env = lib.child_env(pgoptions)
        exports: dict[str, tuple[list[str], Any]] = {}
        details: list[dict[str, Any]] = []

        for index, query in enumerate(queries, start=1):
            corpus = query["corpus"]
            values = {
                "vector_literal": vector_literal(query["vector"]),
                "vecteur_litteral": vector_literal(query["vector"]),
                "corpus": corpus,
                "query_id": query["query_id"],
                "requete_id": query["query_id"],
                "container": container,
                "conteneur": container,
            }
            hnsw_payload = lib.run_json_command(
                lib.render_command(hnsw_template, values),
                label=f"HNSW query {query['query_id']}",
                timeout=args.timeout,
                cwd=output.parent,
                env=env,
            )
            hnsw_ids = _ids_from_payload(
                hnsw_payload, label=f"HNSW query {query['query_id']}"
            )

            if args.mode == "export":
                if corpus not in exports:
                    exports[corpus] = _load_corpus_export(
                        Path(args.export_dir), corpus
                    )
                identifiers, vectors = exports[corpus]
                exact_ids = exact_topk_from_vectors(
                    query["vector"],
                    identifiers,
                    vectors,
                    k=args.k,
                    chunk_size=args.chunk_size,
                )
            else:
                exact_payload = lib.run_json_command(
                    lib.render_command(exact_template or [], values),
                    label=f"exact query {query['query_id']}",
                    timeout=args.timeout,
                    cwd=output.parent,
                    env=env,
                )
                exact_ids = _ids_from_payload(
                    exact_payload, label=f"exact query {query['query_id']}"
                )

            details.append(
                {
                    "query_id": query["query_id"],
                    "corpus": corpus,
                    "recall": round(
                        recall_at_k(exact_ids, hnsw_ids, args.k), 6
                    ),
                    "hnsw_ids": hnsw_ids[: args.k],
                    "exact_ids": exact_ids[: args.k],
                }
            )
            if index % 100 == 0:
                print(
                    f"mes00 fidelity: {index}/{len(queries)} queries measured",
                    file=sys.stderr,
                )

        by_corpus: dict[str, list[float]] = {}
        for detail in details:
            by_corpus.setdefault(detail["corpus"], []).append(detail["recall"])
        per_corpus = {
            corpus: lib.summarize(values)
            for corpus, values in sorted(by_corpus.items())
        }
        document = {
            "meta": {
                "schema_version": SCHEMA_VERSION,
                "date": datetime.now(timezone.utc).isoformat(),
                "exact_mode": args.mode,
                "k": args.k,
                "count": len(details),
                "ef_search": ef_search_of(hnsw_template),
                "hnsw_template_sha256": template_fingerprint(hnsw_template),
                "exact_template_sha256": (
                    template_fingerprint(exact_template) if exact_template else None
                ),
                "recall_definition": "|exact_top_k inter hnsw_top_k| / |exact_top_k|",
            },
            "global": lib.summarize([detail["recall"] for detail in details]),
            "by_corpus": per_corpus,
            "details": details,
        }
        lib.write_json(output, document)
    except lib.Mes00Error as exc:
        print(f"mes00 fidelity: {exc}", file=sys.stderr)
        return 2

    print(f"mes00 fidelity: {len(details)} queries -> {output}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
