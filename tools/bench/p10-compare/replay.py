#!/usr/bin/env python3
"""Replay the 97 MCP semantic calls against production pgvector (read-only).

Protocol (ADR-0140, decisions 1, 3, 9b and 10)
----------------------------------------------
* the 97 lines of ``mcp.jsonl``, each with its own ``k``, ``dedupliquer``,
  ``partie_exclue``, ``fonds``/``cle_domaine``/``annee_*`` and its perimeter;
* the query vectors are the ones ``encode_queries.py`` computed once with the
  Legifacile encoder -- the encoder is NOT timed here (ADR-0140 decision 3);
* every query is replayed through ``api.recherche_semantique`` by direct SQL,
  exactly as the MCP calls it: ``k`` per corpus for a mono-corpus perimeter,
  ``k x 2`` per corpus plus sort/filter/cut for a multi-corpus one;
* one warm-up pass, then three timed repetitions, strictly serial;
* latency is measured host-side (SQL + fusion, then passage reconstruction),
  indicative only (ADR-0129 decision 7 point 5);
* ``droit_applicable`` uses its applicability filter with the frozen date
  ``2026-11-03`` instead of ``date.today()``: the server's own predicate
  ``outils._applicable_aujourdhui`` receives that date as an argument, so no
  code is changed -- only the date the measurement passes in.

Safety (ADR-0032)
-----------------
* ``$PROD_PGOPTIONS`` must enable a read-only transaction and
  ``SHOW transaction_read_only`` is verified on a fresh connection;
* nothing is written outside the guarded output folder; the report is
  rewritten atomically after every query, so an interrupted campaign keeps
  what it measured;
* the process changes its working directory to the output folder before
  importing the production modules and disables bytecode writing;
* ``serveur_mcp.metriques`` must not be installed: nothing may write to
  ``mcp.appels_outil``;
* queries run one after another; there is no concurrency option.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import sys
import time
from datetime import date, datetime, timezone
from pathlib import Path
from typing import Any, Mapping, Sequence

import mes00lib as lib
import queries as game

SCHEMA_VERSION = 1
FIXED_DATE = date(2026, 11, 3)
STAGES = ("sql", "reconstruction", "total")


def load_entries(path: str | os.PathLike[str]) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    payload = lib.read_json(path)
    if not isinstance(payload, Mapping) or not isinstance(payload.get("queries"), list):
        raise lib.Mes00Error("the input file has no 'queries' array")
    entries: list[dict[str, Any]] = []
    for number, raw in enumerate(payload["queries"], start=1):
        if not isinstance(raw, Mapping):
            raise lib.Mes00Error(f"query #{number} is not an object")
        identifier = raw.get("id")
        perimetre = raw.get("perimetre")
        params = raw.get("params")
        if not isinstance(identifier, str) or not identifier:
            raise lib.Mes00Error(f"query #{number} has no id")
        if perimetre not in game.PERIMETERS:
            raise lib.Mes00Error(f"{identifier}: unknown perimeter")
        if not isinstance(params, Mapping):
            raise lib.Mes00Error(f"{identifier}: no params object")
        entries.append(
            {
                "id": identifier,
                "perimetre": str(perimetre),
                "params": dict(params),
            }
        )
    meta = payload.get("meta", {})
    return (dict(meta) if isinstance(meta, Mapping) else {}), entries


def load_vectors(path: str | os.PathLike[str]) -> tuple[dict[str, Any], dict[str, list[float]]]:
    payload = lib.read_json(path)
    if not isinstance(payload, Mapping) or not isinstance(payload.get("queries"), list):
        raise lib.Mes00Error("the vector file has no 'queries' array")
    vectors: dict[str, list[float]] = {}
    for raw in payload["queries"]:
        if not isinstance(raw, Mapping):
            raise lib.Mes00Error("the vector file has a non-object row")
        identifier = raw.get("id")
        vector = raw.get("vector")
        if not isinstance(identifier, str) or not isinstance(vector, list):
            raise lib.Mes00Error("the vector file has a malformed row")
        vectors[identifier] = [float(value) for value in vector]
    meta = payload.get("meta", {})
    return (dict(meta) if isinstance(meta, Mapping) else {}), vectors


def attach_vectors(
    entries: Sequence[Mapping[str, Any]], vectors: Mapping[str, Sequence[float]]
) -> list[dict[str, Any]]:
    merged: list[dict[str, Any]] = []
    for entry in entries:
        vector = vectors.get(entry["id"])
        if vector is None:
            raise lib.Mes00Error(f"{entry['id']}: no query vector")
        merged.append({**entry, "vector": list(vector)})
    extra = set(vectors) - {entry["id"] for entry in entries}
    if extra:
        raise lib.Mes00Error(f"vectors without a query: {sorted(extra)}")
    return merged


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def timed(call):
    started = time.perf_counter()
    value = call()
    return (time.perf_counter() - started) * 1000.0, value


def replay_one(
    entry: Mapping[str, Any],
    dependencies: Mapping[str, Any],
    fixed_date: date | None,
) -> dict[str, Any]:
    """One replay: SQL/fusion then passage reconstruction, each timed.

    Returns the ranked results and the two durations. The fusion path mirrors
    ``outils._fusionner_semantique``: ``k x 2`` per corpus when the perimeter
    holds several, sort by distance, applicability filter for
    ``perimetres.FILTRE_APPLICABLE``, cut to ``k``.
    """

    base = dependencies["base"]
    perimetres = dependencies["perimetres"]
    outils = dependencies["outils"]
    froid = dependencies["froid"]
    conn = dependencies["conn"]
    k = int(entry["params"].get("k", 10))
    perimetre = entry["perimetre"]
    corpus_list = perimetres.corpus_du_perimetre(perimetre)
    vector = base.Vecteur(entry["vector"])

    def search(corpus: str, limit: int, params: Mapping[str, Any]) -> Mapping[str, Any]:
        return conn.appeler(
            "api.recherche_semantique",
            p_corpus=corpus,
            p_vecteur=vector,
            p_limite=limit,
            **params,
        )

    def work() -> list[dict[str, Any]]:
        rows = game.candidates(corpus_list, k, search, entry["params"])
        applicable = None
        if perimetre in perimetres.FILTRE_APPLICABLE:
            # The server passes date.today(); the game freezes 2026-11-03.
            day = fixed_date or date.today()
            applicable = lambda row, day=day: outils._applicable_aujourdhui(row, day)
        return game.select_ranked(rows, k, applicable)

    ms_sql, results = timed(work)
    ms_reconstruction, _mesure = timed(
        lambda: froid._poste_c(dependencies["ctx"], results)
    )
    return {
        "results": game.ranked_rows(results),
        "sql": round(ms_sql, 3),
        "reconstruction": round(ms_reconstruction, 3),
        "total": round(ms_sql + ms_reconstruction, 3),
    }


def stage_summaries(measurements: Sequence[Mapping[str, Any]]) -> dict[str, Any]:
    """Percentiles per stage over every timed repetition of the given queries.

    The population is declared: the number of queries times the number of
    repetitions, never a mix of warm and cold runs (all are warm here).
    """

    result: dict[str, Any] = {}
    for stage in STAGES:
        values = [
            latency[stage]
            for measurement in measurements
            for latency in (measurement.get("latencies") or {}).values()
            if latency.get(stage) is not None
        ]
        result[stage] = game.summarize(values) if values else {"n": 0}
    return result


def aggregate(measurements: Sequence[Mapping[str, Any]]) -> dict[str, Any]:
    result = stage_summaries(measurements)
    by_perimeter: dict[str, Any] = {}
    for perimeter in sorted({row["perimetre"] for row in measurements}):
        selected = [row for row in measurements if row["perimetre"] == perimeter]
        by_perimeter[perimeter] = {
            "queries": len(selected),
            "total": stage_summaries(selected)["total"],
        }
    result["by_perimeter"] = by_perimeter
    return result


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--queries", required=True, help="output of build_inputs.py")
    parser.add_argument("--vectors", required=True, help="output of encode_queries.py")
    parser.add_argument("--output", required=True, help="full report, under /tmp")
    parser.add_argument(
        "--fixed-date",
        default=FIXED_DATE.isoformat(),
        help="date used by the applicability filter (ADR-0140 decision 9b)",
    )
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--warmup", type=int, default=1)
    parser.add_argument("--project", default=os.environ.get("PROD_REPO", ""))
    parser.add_argument("--allow-root", action="append", default=[])
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        if args.repetitions < 1:
            raise lib.Mes00Error("--repetitions must be positive")
        if args.warmup < 0:
            raise lib.Mes00Error("--warmup must not be negative")
        fixed_date = date.fromisoformat(args.fixed_date)

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
        queries_path = Path(args.queries).expanduser().resolve()
        vectors_path = Path(args.vectors).expanduser().resolve()

        # The copied script must not write bytecode into the checkout, and the
        # production modules must run with their working directory outside it.
        sys.dont_write_bytecode = True
        os.chdir(output.parent)
        os.environ["PGOPTIONS"] = pgoptions
        sys.path.insert(0, str(project))
        sys.path.insert(0, str(project / "scripts" / "db"))

        import psycopg
        from serveur_mcp import base, encodage, metriques, outils, perimetres, rendu
        import froid_decomposition as froid

        if metriques.courant() is not None:
            raise lib.Mes00Error(
                "serveur_mcp.metriques is installed; it would write to mcp.appels_outil"
            )

        # The perimeter table this harness ranks with must be the served one.
        if dict(perimetres.PERIMETRES) != game.PERIMETERS:
            raise lib.Mes00Error(
                "serveur_mcp.perimetres.PERIMETRES differs from the harness table"
            )
        if set(perimetres.FILTRE_APPLICABLE) != set(game.FILTER_APPLICABLE):
            raise lib.Mes00Error(
                "serveur_mcp.perimetres.FILTRE_APPLICABLE differs from the harness table"
            )

        with psycopg.connect(base.DSN, autocommit=True) as probe:
            with probe.cursor() as cur:
                cur.execute("SHOW transaction_read_only")
                if cur.fetchone()[0] != "on":
                    raise lib.Mes00Error(
                        "transaction_read_only is not on; refusing to measure (ADR-0032)"
                    )

        query_meta, entries = load_entries(queries_path)
        vector_meta, vectors = load_vectors(vectors_path)
        if len(vectors) != len(entries):
            raise lib.Mes00Error(
                f"{len(vectors)} vectors for {len(entries)} queries"
            )
        merged = attach_vectors(entries, vectors)

        encodage.charger()
        conn = base.Connexion(application_name="crible_p10_compare")
        ctx = froid.Contexte(
            conn=conn,
            encodage=encodage,
            base=base,
            perimetres=perimetres,
            outils=outils,
            rendu=rendu,
            k=10,
            partie_sql=None,
        )
        dependencies = {
            "base": base,
            "perimetres": perimetres,
            "outils": outils,
            "froid": froid,
            "conn": conn,
            "ctx": ctx,
        }

        started_at = datetime.now(timezone.utc).isoformat()
        measurements: list[dict[str, Any]] = []
        for index, entry in enumerate(merged, start=1):
            if args.warmup:
                for _ in range(args.warmup):
                    replay_one(entry, dependencies, fixed_date)
            measurement: dict[str, Any] = {
                "id": entry["id"],
                "perimetre": entry["perimetre"],
                "results": [],
                "latencies": {},
                "doc_ids_stable": True,
            }
            for repetition in range(1, args.repetitions + 1):
                run = replay_one(entry, dependencies, fixed_date)
                measurement["latencies"][f"rep{repetition}"] = {
                    stage: run[stage] for stage in STAGES
                }
                doc_ids = [row["doc_id"] for row in run["results"]]
                if repetition == 1:
                    measurement["results"] = run["results"]
                elif doc_ids != [row["doc_id"] for row in measurement["results"]]:
                    measurement["doc_ids_stable"] = False

            measurements.append(measurement)
            document = {
                "meta": {
                    "schema_version": SCHEMA_VERSION,
                    "campaign_date": datetime.now(timezone.utc).date().isoformat(),
                    "started_at": started_at,
                    "updated_at": datetime.now(timezone.utc).isoformat(),
                    "count": len(measurements),
                    "expected_count": len(merged),
                    "k": 10,
                    "repetitions": args.repetitions,
                    "warmup": args.warmup,
                    "mode": "serial",
                    "fixed_date": fixed_date.isoformat(),
                    "query_sha256": sha256_of(queries_path),
                    "mcp_sha256": query_meta.get("mcp_sha256"),
                    "vector_sha256": sha256_of(vectors_path),
                    "query_sha256_seen_by_encoder": vector_meta.get("input_sha256"),
                    "encoder": vector_meta.get("encoder"),
                    "model": vector_meta.get("model"),
                    "revision": vector_meta.get("revision"),
                    "storage": vector_meta.get("storage"),
                },
                "measurements": measurements,
                "aggregates": aggregate(measurements),
            }
            lib.write_json(output, document)

            if index % 10 == 0:
                print(
                    f"p10-compare replay: {index}/{len(merged)} queries, "
                    f"{index * args.repetitions} timed replays",
                    file=sys.stderr,
                )
    except lib.Mes00Error as exc:
        print(f"p10-compare replay: {exc}", file=sys.stderr)
        return 2

    print(
        f"p10-compare replay: {len(merged)} queries -> {output}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
