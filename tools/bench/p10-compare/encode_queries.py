#!/usr/bin/env python3
"""Encode the 97 reference queries with the production encoder (runs on the host).

Same path as MES-00 (``tools/bench/mes00/vecteurs_requetes.py``): the encoder
is ``serveur_mcp.encodage`` imported from the Legifacile checkout, loaded once,
and every query goes through ``encodage.encoder_requete`` -- the very function
the MCP calls for ``api.recherche_semantique``'s ``query:`` prompt.

Read-only (ADR-0032): the model is loaded from local files, nothing is fetched
from the network on purpose, nothing is written except the guarded output. The
output holds no query text, only vectors, and records the model, the revision,
the device and the precision actually used.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Mapping, Sequence

import mes00lib as lib
import queries as game

SCHEMA_VERSION = 1


def summarize_encode(rows: Sequence[Mapping[str, Any]]) -> dict[str, Any]:
    """Encoder-only distribution; the encoder is not part of the replay timings."""

    values = [float(row["encode_ms"]) for row in rows if row.get("encode_ms") is not None]
    return game.summarize(values) if values else {"n": 0}


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def read_entries(path: str | os.PathLike[str]) -> tuple[dict[str, Any], list[dict[str, str]]]:
    payload = lib.read_json(path)
    if not isinstance(payload, Mapping) or not isinstance(payload.get("queries"), list):
        raise lib.Mes00Error("the input file has no 'queries' array")
    entries: list[dict[str, str]] = []
    for number, raw in enumerate(payload["queries"], start=1):
        if not isinstance(raw, Mapping):
            raise lib.Mes00Error(f"query #{number} is not an object")
        identifier = raw.get("id")
        text = raw.get("text")
        if not isinstance(identifier, str) or not identifier:
            raise lib.Mes00Error(f"query #{number} has no id")
        if not isinstance(text, str) or not text.strip():
            raise lib.Mes00Error(f"query {identifier} has no text")
        entries.append({"id": identifier, "text": text})
    if not entries:
        raise lib.Mes00Error("the input file has no query")
    meta = payload.get("meta", {})
    return (dict(meta) if isinstance(meta, Mapping) else {}), entries


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--queries", required=True, help="output of build_inputs.py")
    parser.add_argument("--output", required=True, help="vector file, under /tmp")
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

        input_path = Path(args.queries).expanduser().resolve()
        meta, entries = read_entries(input_path)

        # The production modules must not write bytecode into the checkout, and
        # they run with a working directory outside it.
        sys.dont_write_bytecode = True
        os.chdir(output.parent)
        os.environ["PGOPTIONS"] = pgoptions
        sys.path.insert(0, str(project))
        from serveur_mcp import encodage
        import sync_vecteurs

        encodage.charger()
        _tok, _model, device, torch = encodage._encodeur()._assurer()

        document: dict[str, Any] = {
            "meta": {
                "schema_version": SCHEMA_VERSION,
                "date": datetime.now(timezone.utc).isoformat(),
                "input_sha256": sha256_of(input_path),
                "count": len(entries),
                "model": sync_vecteurs.MODELE,
                "revision": sync_vecteurs.REVISION,
                "dimension": sync_vecteurs.DIMENSION,
                "query_prompt": sync_vecteurs.INVITE_REQUETE,
                "device": device,
                "compute_dtype": str(torch.get_default_dtype()),
                "storage": "halfvec(768) (fp16) on the pgvector side",
                "encoder": "serveur_mcp.encodage.encoder_requete",
            },
            "queries": [],
        }
        for entry in entries:
            started = time.perf_counter()
            vector = encodage.encoder_requete(entry["text"])
            elapsed = (time.perf_counter() - started) * 1000.0
            if len(vector) != sync_vecteurs.DIMENSION:
                raise lib.Mes00Error(
                    f"{entry['id']}: encoder returned {len(vector)} components"
                )
            document["queries"].append(
                {"id": entry["id"], "vector": vector, "encode_ms": round(elapsed, 3)}
            )
            if len(document["queries"]) % 20 == 0:
                print(
                    f"p10-compare encode: {len(document['queries'])}/{len(entries)}",
                    file=sys.stderr,
                )
        document["meta"]["encode_ms"] = summarize_encode(document["queries"])
        document["meta"]["input_meta"] = {
            "mcp_sha256": meta.get("mcp_sha256"),
            "count": meta.get("count"),
        }
        lib.write_json(output, document)
    except lib.Mes00Error as exc:
        print(f"p10-compare encode: {exc}", file=sys.stderr)
        return 2

    print(f"p10-compare encode: {len(entries)} vectors -> {output}", file=sys.stderr)
    print(f"sha256 {sha256_of(output)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
