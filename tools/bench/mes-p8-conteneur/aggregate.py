#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Aggregates the raw documents of the container campaign (ADR-0135).

Reads the harness documents of `data/mes-p8-conteneur/raw/` (gitignored) and
writes the three versioned artefacts:

    docs/mesures/MES-02-latence-scan-binaire.<date>.conteneur.json
    docs/mesures/MES-03-latence-rescoring.<date>.conteneur.json
    docs/mesures/MES-09-ram-reelle.<date>.conteneur.json

Every percentile is recomputed from the per-query durations the harness wrote,
by nearest rank (`ceil(ratio * n)`, ADR-0077, decision 4), and the script
**refuses** a figure that differs from the harness summary. No value is rounded,
filtered or extrapolated.

    (cd tools && uv run --no-sync python bench/mes-p8-conteneur/aggregate.py \
        --date 2026-11-03)

The native reference of the same harness, when it exists under
`data/mes-p8-conteneur/native/raw/`, is copied into the artefacts under
`native_reference`, so the comparison is traceable.
"""

from __future__ import annotations

import argparse
import json
import math
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[3]
RAW = ROOT / "data" / "mes-p8-conteneur" / "raw"
NATIVE = ROOT / "data" / "mes-p8-conteneur" / "native" / "raw"
MEASURES = ROOT / "docs" / "mesures"
STAGING = ROOT / "data" / "mes-p8-conteneur" / "staging"

#: Native figures of the last published campaigns, copied here so the artefact
#: carries what it is compared to (ADR-0135, decision 4). Key: (report, section).
PUBLISHED = {
    "mes02": {
        "report": "docs/mesures/MES-02-latence-scan-binaire.md",
        "campaign": "2026-10-27",
        "table": "scan seul, p50 / p95 en ms",
        "note": (
            "The published table uses the dense traversal on contiguous pools; "
            "ADR-0081 removed it. Only the dispersed pools and the full pool "
            "share the sparse traversal of this campaign."
        ),
        "sparse_complement": {
            "report": "docs/mesures/MES-02-parcours-dense-clairseme.md",
            "campaign": "2026-10-27, passes alternées",
            "note": (
                "The only sparse references at N = 1000 for a contiguous pool "
                "and for the full pool; the other contiguous points have none."
            ),
            "points": {
                "1000000:contiguous": {"p50": 5.445, "p95": 6.441},
                "7740000:full": {"p50": 39.719, "p95": 43.204},
            },
        },
        "points": {
            "1000:contiguous": {"p50": 0.055, "p95": 0.089},
            "1000:dispersed": {"p50": 0.063, "p95": 0.097},
            "10000:contiguous": {"p50": 0.375, "p95": 0.431},
            "10000:dispersed": {"p50": 0.400, "p95": 0.467},
            "100000:contiguous": {"p50": 0.978, "p95": 1.061},
            "100000:dispersed": {"p50": 1.261, "p95": 1.399},
            "1000000:contiguous": {"p50": 4.942, "p95": 5.370},
            "1000000:dispersed": {"p50": 11.785, "p95": 14.081},
            "7740000:full": {"p50": 40.037, "p95": 44.132},
        },
    },
    "mes03": {
        "report": "docs/mesures/MES-03-latence-rescoring.md",
        "campaign": "2026-10-28, rejeu d'ADR-0080, variante livrée (option 1, grouped)",
        "table": "série complète à chaud et à froid, p50 / p95 en ms",
        "points": {
            "warm:1000": {"p50": 0.7875, "p95": 14.67},
            "warm:2000": {"p50": 5.28, "p95": 20.605},
            "warm:5000": {"p50": 2.12, "p95": 23.17},
            "cold:1000": {"p50": 22.375, "p95": 38.885, "residency": 0.55},
            "cold:2000": {"p50": 28.955, "p95": 40.63, "residency": 0.685},
            "cold:5000": {"p50": 31.105, "p95": 79.89, "residency": 0.79},
        },
        "note": (
            "p50 and p95 are the means of the two rounds of the 2026-10-28 "
            "replay; the published table gives each round."
        ),
    },
    "mes09": {
        "report": "docs/mesures/MES-09-ram-reelle.md",
        "campaign": "2026-10-27",
        "miB": {
            "open_rss": [1148, 665, 1148],
            "after_pass_rss": [4743, 4948, 4855],
            "after_pass_anon": [852, 852, 861],
            "after_pass_file": [3891, 4096, 3994],
            "footprint": 1126,
            "model_footprint": [2253, 2355],
        },
    },
}

MiB = 1048576


def percentile(sorted_values: list[float], ratio: float) -> float:
    """Nearest rank: the value at `ceil(ratio * n)`, never an interpolation."""
    rank = max(1, math.ceil(ratio * len(sorted_values)))
    return sorted_values[rank - 1]


def summary(values: list[float]) -> dict:
    kept = sorted(value for value in values if not math.isnan(value))
    return {
        "n": len(kept),
        "p50": percentile(kept, 0.50),
        "p95": percentile(kept, 0.95),
        "p99": percentile(kept, 0.99),
        "max": kept[-1],
        "min": kept[0],
        "mean": sum(kept) / len(kept),
    }


def checked(values: list[float], harness: dict, where: str) -> dict:
    """Recomputes the summary and refuses a disagreement with the harness."""
    recomputed = summary(values)
    for key in ("n", "p50", "p95", "p99"):
        left = recomputed[key]
        right = harness[key]
        if key == "n":
            if left != right:
                raise SystemExit(f"{where}: n differs ({left} vs {right})")
        elif not math.isclose(left, right, rel_tol=0, abs_tol=1e-6):
            raise SystemExit(f"{where}: {key} differs ({left} vs {right})")
    return recomputed


def load(name: str, directory: pathlib.Path = RAW) -> dict:
    path = directory / f"{name}.json"
    if not path.is_file():
        raise SystemExit(f"missing raw document: {path}")
    return json.loads(path.read_text(encoding="utf-8"))


def environment() -> dict:
    out = {}
    for name in ("environment.txt", "container-environment.txt"):
        path = STAGING / name
        if path.is_file():
            out[name.removesuffix(".txt")] = path.read_text(encoding="utf-8").splitlines()
    return out


def mes02(date: str) -> dict:
    runs = ("scan-main", "scan-all")
    points = []
    for run in runs:
        for n in (1000, 2000, 5000):
            document = load(f"{run}-n{n}")
            for point in document["points"]:
                durations = point["durations_us"]
                recomputed = checked(durations, point["summary"], f"{run} n={n} size={point['size']} {point['shape']}")
                points.append(
                    {
                        "run": run,
                        "size": point["size"],
                        "shape": point["shape"],
                        "n": point["n"],
                        "traversal": "sparse",
                        "plan": point["plan"],
                        "passes": point["passes"],
                        "queries": point["queries"],
                        "summary_us": recomputed,
                        "pass_summaries_us": point["pass_summaries"],
                    }
                )
    warmup = [f"warmup-scan-n{n}" for n in (1000, 5000)]
    return {
        "kind": "MES-02 container campaign",
        "measure": "MES-02",
        "task": "MESURER-EN-CONTENEUR-MAC",
        "protocol": ["ADR-0077", "ADR-0135"],
        "date": date,
        "environment": environment(),
        "dataset": {
            "name": "synth-large",
            "live_passages": 7740000,
            "dead_passages": 860000,
            "segments": 1,
            "vectors_file_bytes": 13209600000,
            "codes_bytes": 825600000,
            "queries": 300,
            "queries_sha256": "03cb2a2b3bb7951892179ee8d612b0970626f0f041fe6b3e18c2f0393452948b",
        },
        "warmup_discarded": warmup,
        "points": points,
        "native_reference": PUBLISHED["mes02"],
    }


def mes03(date: str) -> dict:
    warm = load("rescore-warm")
    key_only = load("key-only")
    points = []
    for point in warm["points"]:
        n = point["n"]
        entry = {
            "n": n,
            "candidates": point["candidates"],
            "warm_engine_us": checked(point["durations_us"], point["summary"], f"warm engine n={n}"),
            "warm_dot_us": checked(point["dot_only_us"], point["dot_only_summary"], f"warm dot n={n}"),
            "warm_keys_us": checked(point["key_only_us"], point["key_only_summary"], f"warm keys n={n}"),
            "warm_pass_summaries_us": point["pass_summaries"],
        }
        points.append(entry)
    for point in key_only["points"]:
        for entry in points:
            if entry["n"] == point["n"]:
                entry["warm_keys_engine_absent_us"] = checked(
                    point["key_only_us"], point["key_only_summary"], f"key-only n={point['n']}"
                )
    cold = []
    for n in (1000, 2000, 5000):
        engine = load(f"rescore-cold-evict-n{n}")["points"][0]
        dot = load(f"rescore-cold-evict-dot-n{n}")["points"][0]
        keys = load(f"rescore-cold-evict-keys-n{n}")["points"][0]
        cold.append(
            {
                "n": n,
                "engine_us": checked(engine["durations_us"], engine["summary"], f"cold engine n={n}"),
                "engine_residency": engine["resident_fraction"],
                "dot_us": checked(dot["dot_only_us"], dot["dot_only_summary"], f"cold dot n={n}"),
                "dot_residency": dot["resident_fraction"],
                "keys_us": checked(keys["key_only_us"], keys["key_only_summary"], f"cold keys n={n}"),
                "keys_residency": keys["resident_fraction"],
            }
        )
    native = None
    if (NATIVE / "rescore-warm.json").is_file():
        document = load("rescore-warm", NATIVE)
        native = {"protocol": "same harness, same tree, same copy, macOS", "points": []}
        for point in document["points"]:
            native["points"].append(
                {
                    "n": point["n"],
                    "engine_us": checked(point["durations_us"], point["summary"], "native engine"),
                    "dot_us": checked(point["dot_only_us"], point["dot_only_summary"], "native dot"),
                    "keys_us": checked(point["key_only_us"], point["key_only_summary"], "native keys"),
                }
            )
    return {
        "kind": "MES-03 container campaign",
        "measure": "MES-03",
        "task": "MESURER-EN-CONTENEUR-MAC",
        "protocol": ["ADR-0077", "ADR-0080", "ADR-0135"],
        "date": date,
        "environment": environment(),
        "cold_protocol": {
            "eviction": "madvise(MADV_PAGEOUT) on the candidate pages, then posix_fadvise(POSIX_FADV_DONTNEED)",
            "residency": "mincore, published per point",
            "purge": (RAW / "purge.txt").read_text(encoding="utf-8").strip()
            if (RAW / "purge.txt").is_file()
            else None,
        },
        "warmup_discarded": ["warmup-rescore"],
        "points": points,
        "cold": cold,
        "native_reference": native,
        "published_reference": PUBLISHED["mes03"],
    }


def mes09(date: str) -> dict:
    runs = []
    for run in (1, 2, 3):
        document = load(f"rss-base-{run}")
        runs.append({"run": run, "passes": document["passes"], "states": document["states"]})
    models = []
    for run in (1, 2):
        document = load(f"rss-model-cold-{run}")
        models.append(
            {"run": run, "passes": document["passes"], "states": document["states"], "model": document["model"]}
        )
    return {
        "kind": "MES-09 container campaign",
        "measure": "MES-09",
        "task": "MESURER-EN-CONTENEUR-MAC",
        "protocol": ["ADR-0077", "ADR-0135"],
        "date": date,
        "environment": environment(),
        "index": {
            "live_passages": runs[0]["states"]["after_open"] and 7740000,
            "page_size": 4096,
            "memory_limit": "none (`--memory` not set, cgroup memory.max = max)",
        },
        "runs": runs,
        "model_runs": models,
        "native_reference": PUBLISHED["mes09"],
    }


def table_mes02(document: dict) -> str:
    lines = ["| Pool | Forme | N | n | p50 | p95 | p99 | max |", "|---|---|---:|---:|---:|---:|---:|---:|"]
    for point in document["points"]:
        if point["run"] != "scan-main":
            continue
        s = point["summary_us"]
        lines.append(
            "| %d | %s | %d | %d | %.3f | %.3f | %.3f | %.3f |"
            % (
                point["size"],
                point["shape"],
                point["n"],
                s["n"],
                s["p50"] / 1000,
                s["p95"] / 1000,
                s["p99"] / 1000,
                s["max"] / 1000,
            )
        )
    return "\n".join(lines)


def table_mes03(document: dict) -> str:
    lines = [
        "| Série | N | p50 | p95 | p99 | résidence moyenne | résidence max |",
        "|---|---:|---:|---:|---:|---:|---:|",
    ]
    for point in document["points"]:
        for label, key in (
            ("chaud, moteur", "warm_engine_us"),
            ("chaud, dot seul", "warm_dot_us"),
            ("chaud, clés seules", "warm_keys_us"),
        ):
            s = point[key]
            lines.append(
                "| %s | %d | %.3f | %.3f | %.3f | — | — |"
                % (label, point["n"], s["p50"] / 1000, s["p95"] / 1000, s["p99"] / 1000)
            )
    for point in document["cold"]:
        for label, key, residency in (
            ("froid, moteur", "engine_us", "engine_residency"),
            ("froid, dot seul", "dot_us", "dot_residency"),
            ("froid, clés seules", "keys_us", "keys_residency"),
        ):
            s = point[key]
            r = point[residency]
            lines.append(
                "| %s | %d | %.3f | %.3f | %.3f | %.3f | %.3f |"
                % (
                    label,
                    point["n"],
                    s["p50"] / 1000,
                    s["p95"] / 1000,
                    s["p99"] / 1000,
                    r["mean"],
                    max(r["values"]),
                )
            )
    return "\n".join(lines)


def table_mes09(document: dict) -> str:
    lines = [
        "| Exécution | État | RSS | anon | fichiers | `memory.current` | `memory.stat` anon | `memory.stat` file |",
        "|---|---|---:|---:|---:|---:|---:|---:|",
    ]
    for run in document["runs"]:
        for state, values in run["states"].items():
            cgroup = values.get("cgroup") or {}
            stat = cgroup.get("stat") or {}
            lines.append(
                "| `rss-base-%d` | %s | %.1f | %.1f | %.1f | %.1f | %.1f | %.1f |"
                % (
                    run["run"],
                    state,
                    values["resident_bytes"] / MiB,
                    values["anon_bytes"] / MiB,
                    values["file_backed_bytes"] / MiB,
                    (cgroup.get("memory_current_bytes") or 0) / MiB,
                    stat.get("anon", 0) / MiB,
                    stat.get("file", 0) / MiB,
                )
            )
    for run in document["model_runs"]:
        for phase in ("before", "after"):
            values = run["model"][phase]
            cgroup = values.get("cgroup") or {}
            stat = cgroup.get("stat") or {}
            lines.append(
                "| `rss-model-%d` | session fp32 (%s) | %.1f | %.1f | %.1f | %.1f | %.1f | %.1f |"
                % (
                    run["run"],
                    phase,
                    values["resident_bytes"] / MiB,
                    values["anon_bytes"] / MiB,
                    values["file_backed_bytes"] / MiB,
                    (cgroup.get("memory_current_bytes") or 0) / MiB,
                    stat.get("anon", 0) / MiB,
                    stat.get("file", 0) / MiB,
                )
            )
    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--date", default="2026-11-03")
    options = parser.parse_args()
    MEASURES.mkdir(parents=True, exist_ok=True)

    builders = (
        ("MES-02-latence-scan-binaire", mes02, table_mes02),
        ("MES-03-latence-rescoring", mes03, table_mes03),
        ("MES-09-ram-reelle", mes09, table_mes09),
    )
    for stem, build, table in builders:
        document = build(options.date)
        path = MEASURES / f"{stem}.{options.date}.conteneur.json"
        path.write_text(json.dumps(document, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        print(f"wrote {path.relative_to(ROOT)}")
        print(table(document))
        print()
    print("aggregation: every percentile recomputed and checked against the harness")


if __name__ == "__main__":
    sys.exit(main())
