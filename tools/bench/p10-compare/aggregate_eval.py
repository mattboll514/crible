#!/usr/bin/env python3
"""Aggregate the six ``crible eval`` campaigns of pass B (P10).

Reads the raw reports left by ``run_eval.sh`` under
``data/p10-compare/eval/runs/<index>/<pass>/``, the pass A replay, the two
``crible inspect`` snapshots and the pgvector counts, then writes one JSON:

``data/p10-compare/eval/aggregates.json``

It carries the three quality passes on the 56 satisfiable base questions and on
the two constrained corpus (ADR-0140, decision 4), the fidelity of the six
indexes with ``dense_strategy`` checked, the latency percentiles of
``debug.timings_ms``, the per-query comparison of the dense primary pass against
pgvector, and the data gaps of decision 10.

No query text and no filter value enters the output: only ``id`` and
``doc_id``, with counts (ADR-0125, decision 16).
"""

from __future__ import annotations

import argparse
import sqlite3
import sys
from pathlib import Path
from typing import Any, Mapping

import pass_b
import queries as game


def read_report(path: Path) -> dict[str, Any]:
    payload = game.read_json(path)
    if not isinstance(payload, dict):
        raise pass_b.PassBError(f"{path.name}: not a JSON object")
    return payload


def find_report(root: Path, index: str, pass_name: str) -> Path:
    matches = sorted((root / index / pass_name).glob(f"eval-{index}-{pass_name}-*.json"))
    if len(matches) != 1:
        raise pass_b.PassBError(
            f"{index}/{pass_name}: {len(matches)} reports found under {root}"
        )
    return matches[0]


def snapshot_database(root: Path, index: str) -> Path:
    matches = sorted(root.glob(f"{index}-e*-g*"))
    if len(matches) != 1:
        raise pass_b.PassBError(
            f"{index}: {len(matches)} snapshots found under {root}"
        )
    database = matches[0] / "records.sqlite"
    if not database.is_file():
        raise pass_b.PassBError(f"{database}: missing")
    return database


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference-dir", default="docs/legifacile/reference")
    parser.add_argument("--replay", default="data/p10-compare/replay.json")
    parser.add_argument("--runs", default="data/p10-compare/eval/runs")
    parser.add_argument("--counts-before", default="data/p10-compare/eval/counts-before.txt")
    parser.add_argument("--counts-after", default="data/p10-compare/eval/counts-after.txt")
    parser.add_argument(
        "--pgvector-counts", default="data/p10-compare/counts-before.json"
    )
    parser.add_argument(
        "--snapshots", default="data/p10-exchange/snapshots"
    )
    parser.add_argument("--output", default="data/p10-compare/eval/aggregates.json")
    args = parser.parse_args(argv)

    try:
        pass_b.check_populations()
        reference = pass_b.load_reference(args.reference_dir)
        expected = pass_b.expected_by_id(reference)
        index_of = pass_b.index_of_question(reference)
        runs = Path(args.runs)

        reports: dict[str, dict[str, dict[str, Any]]] = {
            pass_name: {} for pass_name in pass_b.QUALITY_PASSES
        }
        fidelity: dict[str, Any] = {}
        latency: dict[str, Any] = {}
        for index in game.INDEXES:
            for pass_name in pass_b.PASSES:
                report = read_report(find_report(runs, index, pass_name))
                expected_queries = len(reference[index])
                if pass_name == "fidelite":
                    fidelity[index] = pass_b.fidelity_summary(report, expected_queries)
                else:
                    reports[pass_name][index] = report
                    latency.setdefault(index, {})[pass_name] = pass_b.latency_summary(
                        report, pass_name
                    )

        passes = pass_b.aggregate_passes(reports)

        replay = game.read_json(args.replay)
        if not isinstance(replay, Mapping):
            raise pass_b.PassBError("the replay is not a JSON object")
        pgvector = pass_b.pgvector_rows(replay, expected)
        pgvector_aggregates = pass_b.pgvector_pass(pgvector)

        primary = {}
        for index, report in reports["dense-primaire"].items():
            primary.update(pass_b.quality_rows(report, "dense-primaire"))
        comparison = pass_b.compare_rankings(
            primary, pgvector, sorted(expected), expected
        )

        connections: dict[str, sqlite3.Connection] = {}

        def exists(index: str, doc_id: str) -> bool:
            if index not in connections:
                database = snapshot_database(Path(args.snapshots), index)
                connections[index] = sqlite3.connect(
                    f"file:{database}?mode=ro", uri=True
                )
            row = connections[index].execute(
                "SELECT 1 FROM documents WHERE doc_id = ?", (doc_id,)
            ).fetchone()
            return row is not None

        gaps = pass_b.missing_documents(expected, pgvector, exists, index_of)

        crible_before = pass_b.parse_inspect(Path(args.counts_before).read_text())
        crible_after = pass_b.parse_inspect(Path(args.counts_after).read_text())
        pgvector_counts = game.read_json(args.pgvector_counts)
        if not isinstance(pgvector_counts, Mapping):
            raise pass_b.PassBError("the pgvector counts are not a JSON object")

        fidelity_global = {
            "n": sum(row["n"] for row in fidelity.values()),
            "recall_at_10": round(
                sum(row["recall_at_10"] * row["n"] for row in fidelity.values())
                / sum(row["n"] for row in fidelity.values()),
                6,
            ),
        }

        document = {
            "format_version": pass_b.SCHEMA_VERSION,
            "measure": "COMPARER-AVEC-PGVECTOR",
            "pass": "B",
            "date": "2026-11-06",
            "k": 10,
            "repetitions": 3,
            "warmup": 1,
            "populations": {
                "base": len(pass_b.base_ids()),
                "global": len(pass_b.satisfiable_ids()),
                "juri_texte_integral": len(pass_b.corpus_ids("juri_texte_integral")),
                "textes_vigueur": len(pass_b.corpus_ids("textes_vigueur")),
                "unsatisfiable": pass_b.UNSATISFIABLE,
            },
            "passes": {**passes, "pgvector": pgvector_aggregates},
            "fidelity": {"per_index": fidelity, "global": fidelity_global},
            "latency_ms": latency,
            "comparison_dense_primaire_vs_pgvector": comparison,
            "per_query": pass_b.per_query_rows(
                primary, pgvector, expected, index_of
            ),
            "data_gaps": gaps,
            "counts": {
                "crible_before": crible_before,
                "crible_after": crible_after,
                "crible_vs_pgvector": pass_b.counts_comparison(
                    crible_before, pgvector_counts.get("per_index") or {}
                ),
                "crible_drift_before_after": {
                    index: {
                        field: crible_after[index][field] - crible_before[index][field]
                        for field in ("documents", "passages")
                    }
                    for index in game.INDEXES
                },
            },
        }
        output = Path(args.output)
        output.parent.mkdir(parents=True, exist_ok=True)
        pass_b.write_json(output, document)
    except (pass_b.PassBError, game.P10Error) as exc:
        print(f"p10-compare pass B aggregate: {exc}", file=sys.stderr)
        return 2

    for pass_name, aggregates in document["passes"].items():
        global_row = aggregates["global"]
        print(
            f"{pass_name:16s} global {global_row['recall_at_10']:.4f} "
            f"(n={global_row['n']}) "
            f"juri {aggregates['juri_texte_integral']['recall_at_10']:.4f} "
            f"textes {aggregates['textes_vigueur']['recall_at_10']:.4f}"
        )
    print(f"fidelity global {fidelity_global['recall_at_10']:.4f} (n={fidelity_global['n']})")
    print(f"lost {comparison['lost_queries']} queries, gained {comparison['gained_queries']}")
    print(f"data gaps: {len(gaps['expected'])} expected, {len(gaps['results'])} results")
    print(f"aggregates -> {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
