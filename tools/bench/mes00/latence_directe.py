#!/usr/bin/env python3
"""MES-00: per-query latency of the production semantic search, in three stages.

Why this script exists
----------------------
Legifacile's ``scripts/db/froid_decomposition.py`` is a *campaign* harness: it
picks its own questions from ``docs/mcp/evaluation/questions.yaml``, waits
``--attente-min`` before each (series, perimeter) round, and its ``--json``
option expects a **file path**. It has no per-query command line, so the
1 000-query p99 demanded by MES-00 cannot go through it.

This script therefore calls the SAME production functions as
``froid_decomposition.py``, one query at a time, serially:

* ``serveur_mcp.encodage.encoder_requete`` -- stage A (encoder);
* ``api.recherche_semantique`` -- stage B (SQL + HNSW);
* ``api.source_vectorielle`` + ``encodage.passage_exact`` +
  ``rendu.plafonner_passage`` -- stage C (passage reconstruction), reused
  through ``froid_decomposition._poste_c`` so no logic is duplicated.

Both views are measured for every query:

* ``corpus``: one ``api.recherche_semantique`` call on the query's corpus;
* ``perimeter``: the multi-corpus fusion path, through
  ``froid_decomposition._poste_b`` (same ``k x 2`` rule, same sort, same
  applicability filter).

A production user call pays one of the two, never both; stage C is therefore
paid twice per query here. The two views are reported separately.

Safety (ADR-0032)
-----------------
* ``$PROD_PGOPTIONS`` is required and must enable a read-only transaction; the
  harness verifies ``SHOW transaction_read_only`` on a fresh connection.
* Nothing is written outside the guarded output path under ``/tmp``; the
  output is rewritten atomically after every query, so an interrupted campaign
  keeps what it measured.
* The process changes its working directory to the guarded output folder
  **before** importing the production modules, and disables bytecode writing,
  so a copied script cannot create files in ``$PROD_REPO``.
* Queries run strictly one after another; there is no concurrency option.
* ``serveur_mcp.metriques`` must not be installed: nothing may write to
  ``mcp.appels_outil``.

Cold is what this script can honestly produce: a controlled idle period
(``--idle-minutes-per-corpus``) before the first query of each corpus. It never
drops a cache on production.
"""

from __future__ import annotations

import argparse
import os
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Mapping, Sequence

import mes00lib as lib

SCHEMA_VERSION = 1
STAGES = ("encoder", "sql", "reconstruction", "total")
STATES = ("cold", "warm")


def timed(fn):
    """Return (milliseconds, value, censored) for one synchronous call.

    A ``statement_timeout`` cancellation is a measurement, not a crash: it means
    "at least base.TIMEOUT_MS". It is flagged and excluded from percentiles
    rather than averaged as if it were the cap (same rule as
    ``froid_decomposition._chrono``).
    """

    import psycopg

    started = time.perf_counter()
    try:
        value = fn()
    except psycopg.errors.QueryCanceled:
        return (time.perf_counter() - started) * 1000.0, None, True
    return (time.perf_counter() - started) * 1000.0, value, False


def wrap_vector(base_module: Any, vector: Sequence[float] | None) -> Any:
    """Wrap a raw query vector for the direct search path only."""

    return base_module.Vecteur(vector) if vector is not None else None


def search_corpus(
    conn: Any,
    corpus: str,
    vector: Any,
    k: int,
    excluded_part: Sequence[str] | None,
) -> Any:
    """Call ``api.recherche_semantique`` with an already wrapped vector."""

    return conn.appeler(
        "api.recherche_semantique",
        p_corpus=corpus,
        p_vecteur=vector,
        p_limite=k,
        p_fonds=None,
        p_etat=None,
        p_annee_min=None,
        p_annee_max=None,
        p_cle_domaine=None,
        p_dedupliquer="document",
        p_partie=(list(excluded_part) if excluded_part else None),
    )


def search_perimeter(
    froid: Any,
    ctx: Any,
    perimeter: str,
    vector: Sequence[float],
) -> Any:
    """Call the fusion path with the raw vector, which it wraps itself."""

    return froid._poste_b(ctx, perimeter, vector, None)


def load_queries(path: str | os.PathLike[str]) -> tuple[dict[str, Any], list[dict[str, str]]]:
    payload = lib.read_json(path)
    if isinstance(payload, Mapping):
        queries = payload.get("queries", payload.get("requetes"))
        meta = payload.get("meta", {})
    else:
        queries, meta = payload, {}
    if not isinstance(queries, list) or not queries:
        raise lib.Mes00Error("the query file has no 'queries' array")
    normalised: list[dict[str, str]] = []
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
            }
        )
    return (dict(meta) if isinstance(meta, Mapping) else {}), normalised


def run_state(index: int, idle_minutes: float) -> str:
    """Return the thermal state of the first query in a corpus."""

    return "cold" if index == 0 and idle_minutes > 0 else "warm"


def failed_run(
    query: Mapping[str, str], corpus: str, perimeter: str, state: str
) -> dict[str, Any]:
    """Record an encoding failure without turning a cold run into a warm one."""

    empty = {"encoder": None, "sql": None, "reconstruction": None, "total": None}
    return {
        "query_id": query["query_id"],
        "corpus": corpus,
        "perimeter": perimeter,
        "corpus_state": state,
        "perimeter_state": state,
        "corpus_path": dict(empty),
        "perimeter_path": dict(empty),
        "not_run": {"corpus": [], "perimeter": []},
        "failed": True,
    }


def group_by_corpus(
    queries: Sequence[Mapping[str, str]],
) -> list[tuple[str, list[dict[str, str]]]]:
    """Group queries by corpus, in sorted corpus order, file order inside a corpus."""

    buckets: dict[str, list[dict[str, str]]] = {}
    for query in queries:
        buckets.setdefault(str(query["corpus"]), []).append(dict(query))
    return [(corpus, buckets[corpus]) for corpus in sorted(buckets)]


def measure_corpus(
    conn: Any,
    corpus: str,
    vector: Any,
    k: int,
    excluded_part: Sequence[str] | None,
    ctx: Any,
    froid: Any,
) -> tuple[float | None, float | None, bool, bool]:
    """Measure one mono-corpus call and the reconstruction stage that follows it.

    A ``statement_timeout`` cancellation returns no results, so the
    reconstruction stage never ran. It is reported as ``None`` and flagged
    censored: writing ``0.0`` there would enter the reconstruction series as a
    real duration and bias it.
    """

    ms_sql, raw, censored_sql = timed(
        lambda: search_corpus(conn, corpus, vector, k, excluded_part)
    )
    if raw is None:
        return ms_sql, None, censored_sql, True
    results = (raw.get("resultats") or [])[:k]
    for row in results:
        row.setdefault("corpus", corpus)
    ms_c, _, censored_c = timed(lambda: froid._poste_c(ctx, results).ms)
    return ms_sql, ms_c, censored_sql, censored_c


def measure_perimeter(
    froid: Any, ctx: Any, perimeter: str, raw_vector: Sequence[float]
) -> tuple[float | None, float | None, bool, bool]:
    """Measure the fusion path of a corpus that belongs to a perimeter.

    A corpus outside every perimeter has no fusion path: both durations are
    ``None`` because nothing was measured, and ``summarize_runs`` keeps such a
    run out of the perimeter aggregates instead of counting fabricated zeros.
    """

    if not perimeter:
        return None, None, False, False
    ms_sql, fusion, censored_sql = timed(
        lambda: search_perimeter(froid, ctx, perimeter, raw_vector)
    )
    if fusion is None:
        return ms_sql, None, censored_sql, True
    results = fusion[1]
    if not results:
        # The fusion returned nothing to reconstruct: no duration was measured,
        # so report None instead of a fabricated 0 ms (same rule as censoring).
        return ms_sql, None, censored_sql, False
    ms_c, _, censored_c = timed(lambda: froid._poste_c(ctx, results).ms)
    return ms_sql, ms_c, censored_sql, censored_c


def stage_value(value: float | None, censored: bool) -> float | None:
    """Return a rounded duration, or ``None`` when it was censored or not run."""

    if censored or value is None:
        return None
    return round(value, 3)


def not_run_stages(
    ms_reconstruction: float | None, censored_reconstruction: bool
) -> list[str]:
    """Stages that did not run at all, as opposed to being censored.

    The only case today is an empty fusion result: the SQL call succeeded but
    returned no passage, so there was nothing to reconstruct and no total to
    add up. The durations are ``None`` and must not be counted as a
    ``statement_timeout`` cancellation.
    """

    if ms_reconstruction is None and not censored_reconstruction:
        return ["reconstruction", "total"]
    return []


def path_record(
    ms_encoder: float | None,
    censored_encoder: bool,
    ms_sql: float | None,
    censored_sql: bool,
    ms_reconstruction: float | None,
    censored_reconstruction: bool,
) -> dict[str, float | None]:
    """Build one view record; the total is ``None`` as soon as a stage is missing."""

    encoder = stage_value(ms_encoder, censored_encoder)
    sql = stage_value(ms_sql, censored_sql)
    reconstruction = stage_value(ms_reconstruction, censored_reconstruction)
    total = None
    if (
        ms_encoder is not None
        and ms_sql is not None
        and ms_reconstruction is not None
        and not censored_encoder
        and not censored_sql
        and not censored_reconstruction
    ):
        total = round(ms_encoder + ms_sql + ms_reconstruction, 3)
    return {
        "encoder": encoder,
        "sql": sql,
        "reconstruction": reconstruction,
        "total": total,
    }


def summarize_runs(runs: Sequence[Mapping[str, Any]], view: str) -> dict[str, Any]:
    """Percentiles per stage for one view, split into cold and warm runs.

    A stage that was censored (``None``) is skipped and counted, never imputed.
    A stage that did not run at all (empty fusion: nothing to reconstruct) is
    counted in ``not_run``, apart from ``censored`` so the latter keeps meaning
    "cancelled by ``statement_timeout``". A run that failed during encoding is
    counted in ``failed``, which is neither a cancellation nor a skipped stage.
    A run whose corpus belongs to no perimeter has no fusion pass at all; it is
    dropped from the perimeter view so it cannot add zeros to those aggregates.
    """

    path = f"{view}_path"
    applicable = [
        run for run in runs if view != "perimeter" or run.get("perimeter")
    ]
    result: dict[str, Any] = {}
    for state in STATES:
        selected = [run for run in applicable if run[f"{view}_state"] == state]
        if not selected:
            continue
        stages: dict[str, Any] = {}
        for stage in STAGES:
            values = [
                run[path][stage]
                for run in selected
                if run[path][stage] is not None
            ]
            not_run = sum(
                1
                for run in selected
                if stage in (run.get("not_run") or {}).get(view, ())
            )
            failed = sum(1 for run in selected if run.get("failed"))
            censored = len(selected) - len(values) - not_run - failed
            if not values:
                stages[stage] = {
                    "n": 0,
                    "censored": censored,
                    "not_run": not_run,
                    "failed": failed,
                }
                continue
            stages[stage] = {
                **lib.summarize(values),
                "censored": censored,
                "not_run": not_run,
                "failed": failed,
            }
        result[state] = stages
    return result


def aggregate(runs: Sequence[Mapping[str, Any]]) -> dict[str, Any]:
    """Global, per-corpus and per-perimeter aggregates for both views."""

    def bucket(key) -> dict[str, Any]:
        selected = [run for run in runs if key(run)]
        return {
            "n": len(selected),
            "corpus": summarize_runs(selected, "corpus"),
            "perimeter": summarize_runs(selected, "perimeter"),
        }

    by_corpus: dict[str, Any] = {}
    for corpus in sorted({run["corpus"] for run in runs}):
        by_corpus[corpus] = bucket(lambda run, corpus=corpus: run["corpus"] == corpus)

    by_perimeter: dict[str, Any] = {}
    for perimeter in sorted({run["perimeter"] for run in runs if run["perimeter"]}):
        by_perimeter[perimeter] = bucket(
            lambda run, perimeter=perimeter: run["perimeter"] == perimeter
        )

    return {
        "global": bucket(lambda run: True),
        "by_corpus": by_corpus,
        "by_perimeter": by_perimeter,
    }


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--queries", required=True, help="generator output JSON")
    parser.add_argument("--output", required=True, help="output JSON, under /tmp")
    parser.add_argument(
        "--project",
        default=os.environ.get("PROD_REPO", ""),
        help="Legifacile checkout (default: $PROD_REPO)",
    )
    parser.add_argument("--k", type=int, default=10)
    parser.add_argument(
        "--idle-minutes-per-corpus",
        type=float,
        default=10.0,
        help="idle minutes before the first query of each corpus (0 = no cold run)",
    )
    parser.add_argument("--limit", type=int, help="stop after N queries (smoke test)")
    parser.add_argument(
        "--perimeter-first",
        action="store_true",
        help=(
            "measure the perimeter (fusion) path first, then the corpus path. The "
            "second call on the same graph finds the pages the first one loaded, "
            "so the order decides which view looks faster; the default (corpus "
            "first) is the production order for a mono-corpus call."
        ),
    )
    parser.add_argument(
        "--allow-root",
        action="append",
        default=[],
        help="additional output root",
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = _build_parser().parse_args(argv)
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
        queries_path = Path(args.queries).expanduser().resolve()

        # The copied script must not write bytecode into $PROD_REPO, and the
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

        meta, queries = load_queries(queries_path)
        if args.limit:
            if args.limit <= 0:
                raise lib.Mes00Error("--limit must be positive")
            queries = queries[: args.limit]

        excluded_part = tuple(getattr(outils, "PARTIE_EXCLUE_DEFAUT", ()) or ())

        # Verify the read-only guard on a dedicated connection before anything else.
        with psycopg.connect(base.DSN, autocommit=True) as probe:
            with probe.cursor() as cur:
                cur.execute("SHOW transaction_read_only")
                if cur.fetchone()[0] != "on":
                    raise lib.Mes00Error(
                        "transaction_read_only is not on; refusing to measure (ADR-0032)"
                    )

        encodage.charger()
        conn = base.Connexion(application_name="crible_mes00_latence")
        ctx = froid.Contexte(
            conn=conn, encodage=encodage, base=base, perimetres=perimetres,
            outils=outils, rendu=rendu, k=args.k,
            partie_sql=(list(excluded_part) if excluded_part else None),
        )

        def perimeter_of(corpus: str) -> str:
            for name, corpora in perimetres.PERIMETRES.items():
                if corpus in corpora:
                    return name
            return ""

        runs: list[dict[str, Any]] = []
        started_at = datetime.now(timezone.utc).isoformat()
        total = len(queries)

        for corpus, group in group_by_corpus(queries):
            perimeter = perimeter_of(corpus)
            if args.idle_minutes_per_corpus > 0:
                print(
                    f"mes00 latence: idle {args.idle_minutes_per_corpus:g} min before "
                    f"{corpus} ({len(group)} queries)",
                    file=sys.stderr,
                )
                time.sleep(args.idle_minutes_per_corpus * 60.0)

            for index, query in enumerate(group):
                state = run_state(index, args.idle_minutes_per_corpus)
                ms_encoder, vector, censored_a = timed(
                    lambda: encodage.encoder_requete(query["text"])
                )
                vector_value = wrap_vector(base, vector)
                if vector_value is None:
                    runs.append(failed_run(query, corpus, perimeter, state))
                    continue

                # Order matters: the second call on the same HNSW graph finds the
                # pages the first one loaded. ``--perimeter-first`` swaps the
                # order so the fusion path can be measured without that gift.
                # Both helpers receive the current query's vector explicitly, so
                # a loop reassignment cannot leak into the wrong call.
                if args.perimeter_first:
                    ms_sql_perimeter, ms_c_perimeter, censored_bp, censored_cp = (
                        measure_perimeter(froid, ctx, perimeter, vector)
                    )
                    ms_sql_corpus, ms_c_corpus, censored_bc, censored_cc = (
                        measure_corpus(
                            conn,
                            corpus,
                            vector_value,
                            args.k,
                            excluded_part,
                            ctx,
                            froid,
                        )
                    )
                else:
                    ms_sql_corpus, ms_c_corpus, censored_bc, censored_cc = (
                        measure_corpus(
                            conn,
                            corpus,
                            vector_value,
                            args.k,
                            excluded_part,
                            ctx,
                            froid,
                        )
                    )
                    ms_sql_perimeter, ms_c_perimeter, censored_bp, censored_cp = (
                        measure_perimeter(froid, ctx, perimeter, vector)
                    )

                runs.append(
                    {
                        "query_id": query["query_id"],
                        "corpus": corpus,
                        "perimeter": perimeter,
                        "corpus_state": state,
                        "perimeter_state": state,
                        "corpus_path": path_record(
                            ms_encoder,
                            censored_a,
                            ms_sql_corpus,
                            censored_bc,
                            ms_c_corpus,
                            censored_cc,
                        ),
                        "perimeter_path": path_record(
                            ms_encoder,
                            censored_a,
                            ms_sql_perimeter,
                            censored_bp,
                            ms_c_perimeter,
                            censored_cp,
                        ),
                        "not_run": {
                            "corpus": not_run_stages(ms_c_corpus, censored_cc),
                            "perimeter": not_run_stages(
                                ms_c_perimeter, censored_cp
                            ),
                        },
                        "failed": False,
                    }
                )

                document = {
                    "meta": {
                        "schema_version": SCHEMA_VERSION,
                        "started_at": started_at,
                        "updated_at": datetime.now(timezone.utc).isoformat(),
                        "count": len(runs),
                        "expected_count": total,
                        "seed": meta.get("seed", meta.get("graine")),
                        "query_sha256": meta.get(
                            "source_sha256",
                            meta.get("sha256", meta.get("empreinte_sha256")),
                        ),
                        "k": args.k,
                        "idle_minutes_per_corpus": args.idle_minutes_per_corpus,
                        "mode": "serial",
                        "default_excluded_part": bool(excluded_part),
                        "views": ["corpus (one SQL call)", "perimeter (fusion)"],
                    },
                    "measurements": runs,
                    "aggregates": aggregate(runs),
                }
                lib.write_json(output, document)

                if len(runs) % 50 == 0:
                    print(
                        f"mes00 latence: {len(runs)}/{total} queries measured",
                        file=sys.stderr,
                    )

        print(f"mes00 latence: {len(runs)} runs -> {output}", file=sys.stderr)
    except lib.Mes00Error as exc:
        print(f"mes00 latence: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
