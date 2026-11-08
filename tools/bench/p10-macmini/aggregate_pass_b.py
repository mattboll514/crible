#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Aggregate the pass B reports of `MESURER-SUR-MAC-MINI` (ADR-0141).

Reads the raw `crible eval` reports of `data/p10-macmini/runs/`, groups the
latencies by series, recomputes the recall of the textual dense pass on the
three populations of the constraint, and compares the top-10 of the dense
primary pass with the development machine. It also compares the per-query
fidelity and quality controls with the versioned eval reports of 2026-11-06.

The output carries no query text and no filter value, only identifiers
(ADR-0125, decision 16). It is the versioned companion of the raw reports.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import macmini

INDEXES = ("articles", "decisions", "dole", "jorf", "sarde", "titres")


def find_report(root: Path, index: str, pass_name: str) -> Path:
    folder = root / index / pass_name
    candidates = sorted(folder.glob("eval-*.json"))
    if len(candidates) != 1:
        raise macmini.MacMiniError(
            f"{index}/{pass_name}: expected one report under {folder}, found {len(candidates)}"
        )
    return candidates[0]


def read_top10(root: Path, index: str) -> dict[str, list[str]]:
    folder = root / index
    dump: dict[str, list[str]] = {}
    for path in sorted(folder.glob("*.json")):
        payload = macmini.read_json(path)
        if isinstance(payload, list):
            continue
        dump[path.stem] = macmini.doc_ids(payload)
    return dump


def read_eval_queries(path: Path) -> dict[str, dict]:
    """Per-query controls of one dense eval report, keyed by query id."""

    report = macmini.read_json(path)
    queries = report.get("modes", {}).get("dense", {}).get("queries")
    if not isinstance(queries, list) or not queries:
        raise macmini.MacMiniError(f"{path}: no dense per-query controls")
    return {query["id"]: query for query in queries}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runs", default="data/p10-macmini/runs")
    parser.add_argument("--top10", default="data/p10-macmini/top10")
    parser.add_argument("--eval-dir", default="docs/mesures")
    parser.add_argument("--output", default="docs/mesures/eval-macmini-2026-11-07.aggregates.json")
    parser.add_argument("--vm-bytes", type=int, default=10_418_937_856)
    parser.add_argument("--vm-cpus", type=int, default=8)
    args = parser.parse_args(argv)

    try:
        root = Path(args.runs)
        population_names = macmini.populations()

        series: dict[str, dict] = {}
        reports: dict[str, dict[str, dict]] = {}
        for pass_name in macmini.PASSES:
            summaries: dict[str, dict] = {}
            raw: dict[str, dict] = {}
            for index in INDEXES:
                report = macmini.read_json(find_report(root, index, pass_name))
                raw[index] = report
                summaries[index] = macmini.summarise_report(report, pass_name)
            series[pass_name] = {
                "mode": macmini.PASS_MODE[pass_name],
                "indexes": summaries,
                "grouped": macmini.grouped_percentiles(list(summaries.values())),
            }
            reports[pass_name] = raw

        recall = macmini.recall_by_population(
            reports["dense-texte"], population_names
        )

        reference = Path(args.top10) / "local"
        candidate = Path(args.top10) / "remote"
        per_index = {}
        for index in INDEXES:
            left = read_top10(reference, index)
            right = read_top10(candidate, index)
            per_index[index] = macmini.compare_top10(left, right)
        total_compared = sum(entry["compared"] for entry in per_index.values())
        total_identical = sum(entry["identical"] for entry in per_index.values())
        top10 = {
            "per_index": per_index,
            "compared": total_compared,
            "identical": total_identical,
        }

        versioned_per_index = {}
        quality_differing = []
        for index in INDEXES:
            reference_path = Path(args.eval_dir) / f"eval-{index}-dense-primaire-2026-11-06.json"
            reference = read_eval_queries(reference_path)
            candidate = read_eval_queries(find_report(root, index, "dense-vecteur"))
            versioned_per_index[index] = macmini.compare_eval_queries(reference, candidate)
            quality_differing.extend(
                {"index": index, "id": identifier}
                for identifier in versioned_per_index[index]["quality_differing"]
            )
        versioned_eval = {
            "reference": "docs/mesures/eval-<index>-dense-primaire-2026-11-06.json",
            "document_ids_available": False,
            "note": (
                "the versioned eval reports carry no doc_id, so the top-10 identity is "
                "established by the local and remote search dumps, not by these files"
            ),
            "per_index": versioned_per_index,
            "compared": sum(entry["compared"] for entry in versioned_per_index.values()),
            "fidelity_identical": sum(
                entry["fidelity_identical"] for entry in versioned_per_index.values()
            ),
            "quality_identical": sum(
                entry["quality_identical"] for entry in versioned_per_index.values()
            ),
            "quality_differing": quality_differing,
        }

        text_grouped = series["dense-texte"]["grouped"]
        document = {
            "format_version": 1,
            "measure": "MESURER-SUR-MAC-MINI",
            "pass": "B",
            "date": "2026-11-07",
            "machine": "Mac mini M1 (Macmini9,1), aarch64",
            "vm": {"cpus": args.vm_cpus, "bytes": args.vm_bytes},
            "criterion": {
                "decision": "ADR-0141, decision 4",
                "measure": "total chaud de crible eval local, 97 requêtes en texte",
                "value_ms": text_grouped["p95_bound_max_index"],
                "scope": "plus grand p95 par index",
                "encoding_note": (
                    "the query cache serves the repeated texts (ADR-0051), so the total "
                    "barely includes encoding; this is a fact of measurement against the "
                    "assumption of decision 4"
                ),
            },
            "protocol": {
                "mode": "local",
                "repeat": 3,
                "warmup": 1,
                "exact_threshold": 5000,
                "rescore_candidates": 5000,
                "percentile": "nearest rank on raw durations (ADR-0077, decision 4)",
                "grouped_samples_expected": {
                    "dense-texte": 97 * 3,
                    "mes00-1000-titres": 1000 * 3,
                },
            },
            "series": series,
            "recall": recall,
            "top10": top10,
            "versioned_eval": versioned_eval,
        }
        macmini.write_json(args.output, document)
    except macmini.MacMiniError as exc:
        print(f"p10-macmini pass B aggregate: {exc}", file=sys.stderr)
        return 2

    for pass_name, entry in series.items():
        grouped = entry["grouped"]
        print(
            f"{pass_name:20s} samples {grouped['samples']:5d} "
            f"p95 bound {grouped['p95_bound_max_index']:.1f} ms"
        )
    for name, value in recall.items():
        print(f"recall {name:20s} {value['recall_at_10']:.6f} (n={value['n']})")
    print(f"top10 identical {top10['identical']}/{top10['compared']}")
    print(
        "versioned eval "
        f"fidelity {versioned_eval['fidelity_identical']}/{versioned_eval['compared']}, "
        f"quality {versioned_eval['quality_identical']}/{versioned_eval['compared']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
