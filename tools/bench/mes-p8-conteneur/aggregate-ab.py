#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Builds the artefact of passage A-bis (diagnosis of the engine series' gap).

Reads the raw documents of ``data/mes-p8-conteneur/ab/raw`` and writes one JSON
document: the measured points, the machine states, the binaries' digests, the
residency of the vector pages the candidates touch, and the frames `sample`
charged on the main thread. No value is rounded and no point is dropped: a
missing document is reported as missing, never replaced by a neighbour.

    (cd tools && uv run --no-sync python bench/mes-p8-conteneur/aggregate-ab.py \\
        --raw ../data/mes-p8-conteneur/ab/raw \\
        --out ../docs/mesures/MES-03-diagnostic-moteur.2026-11-03.json)
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path

# The two commits the diagnosis compares: the head of the branch at the moment
# of the measurement, and the commit of the ADR-0080 replay
# (`docs/mesures/MES-03-latence-rescoring.md`, "Lecture groupée des vecteurs").
HEAD = "3fb33439aa3349a31593b8281920af7ce51938bb"
REPLAY = "07a82d773db3245e6fc680b48baabe61fbd4ac21"
# Short form, the name of the extracted tree and of its build directory.
REPLAY_SHORT = REPLAY[:7]

# Frames of interest of the profile, in the order they appear on the main
# thread's call graph.
FRAMES = [
    ("rescore", "DenseSearcher7rescore"),
    ("key_of", "key_of"),
    ("read_passage_key", "read_passage_key"),
    ("btree_table_moveto", "sqlite3BtreeTableMoveto"),
    ("read_db_page", "readDbPage"),
    ("pread", "pread"),
    ("dot", "kernels4neon3dot"),
]


def ab_points(path: Path) -> dict | None:
    """Points of one `rescore_ab measure` document, or None when it is absent."""
    if not path.exists():
        return None
    document = json.loads(path.read_text())
    residency = document.get("resident_fraction")
    return {
        "n": document["n"],
        "k": document["k"],
        "queries": document["queries"],
        "passes": document["passes"],
        "per_pass_p50_ms": [entry["p50"] for entry in document["per_pass"]],
        "per_pass_p95_ms": [entry["p95"] for entry in document["per_pass"]],
        "combined_p50_ms": document["summary"]["p50"],
        "combined_p95_ms": document["summary"]["p95"],
        "combined_p99_ms": document["summary"]["p99"],
        "samples": document["summary"]["n"],
        "resident_fraction": residency,
    }


def dense_points(path: Path) -> dict | None:
    """Points of one `dense_latency rescore` document, or None."""
    if not path.exists():
        return None
    document = json.loads(path.read_text())
    points = []
    for point in document["points"]:
        dot = point.get("dot_only_summary")
        keys = point.get("key_only_summary")
        points.append(
            {
                "n": point["n"],
                "candidates": point["candidates"],
                "per_pass_p50_ms": [entry["p50"] for entry in point["pass_summaries"]],
                "combined_p50_ms": point["summary"]["p50"],
                "combined_p95_ms": point["summary"]["p95"],
                "dot_replica_p50_ms": None if dot is None else dot["p50"],
                "keys_replica_p50_ms": None if keys is None else keys["p50"],
                "samples": point["summary"]["n"],
            }
        )
    return {"points": points, "engine_limit": document.get("engine_limit")}


def machine(path: Path) -> str | None:
    return path.read_text().strip() if path.exists() else None


def digest(path: Path) -> str | None:
    if not path.exists():
        return None
    return hashlib.sha256(path.read_bytes()).hexdigest()


def profile(path: Path) -> dict | None:
    """Main-thread sample counts of the frames of interest."""
    if not path.exists():
        return None
    lines = path.read_text().splitlines()
    start = lines.index("Call graph:")
    end = start + 2
    while end < len(lines) and not re.match(r"^    \d+ Thread_", lines[end]):
        end += 1
    block = lines[start + 2 : end]
    total = None
    counts = {name: None for name, _ in FRAMES}
    for line in block:
        stripped = line.strip().lstrip("!:|+ ")
        match = re.match(r"(\d+)\s", stripped)
        if not match:
            continue
        value = int(match.group(1))
        if total is None:
            total = value
        for name, needle in FRAMES:
            if needle in line and counts[name] is None:
                counts[name] = value
    window = re.search(r"every (\d+) millisecond", "\n".join(lines[:3]))
    return {
        "sampling_period_ms": None if window is None else int(window.group(1)),
        "main_thread_samples": total,
        "main_thread_frames": counts,
        "interpretation": (
            "counts are samples charged on the main thread; a frame appears once, "
            "with the count of its heaviest occurrence"
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--raw", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    arguments = parser.parse_args()
    raw = arguments.raw
    # `--raw` is `data/mes-p8-conteneur/ab/raw`, and the script may run from
    # anywhere: the repository root is four levels up, and every binary path is
    # resolved against it.
    root = raw.resolve().parents[3]

    document = {
        "kind": "mes-03-diagnostic-moteur",
        "date": "2026-11-03",
        "task": "MESURER-EN-CONTENEUR-MAC",
        "passage": "A-bis",
        "protocol": {
            "machine": "MacBook Air M3, 24 Go, macOS, page de 16 Kio, natif",
            "dataset": "data/mes-p8-conteneur/native/indexes/synth-large, "
            "7 740 000 vivants, un segment de 8 600 000 vecteurs f16",
            "queries": "data/mes02/queries.f32, 300 requêtes "
            "(SHA-256 03cb2a2b3bb7951892179ee8d612b0970626f0f041fe6b3e18c2f0393452948b)",
            "series": "une passe d'échauffement jetée, puis une passe mesurée de 300 requêtes",
            "levels": [1000, 5000],
        },
        "commits": {"head": HEAD, "replay": REPLAY},
        "binaries": {
            "head_dense_latency": digest(root / "target/release/examples/dense_latency"),
            "head_rescore_ab": digest(root / "target/release/examples/rescore_ab"),
            "replay_dense_latency": digest(
                root / f"data/mes-p8-conteneur/ab/target-{REPLAY_SHORT}/release/examples/dense_latency"
            ),
            "replay_rescore_ab": digest(
                root / f"data/mes-p8-conteneur/ab/target-{REPLAY_SHORT}/release/examples/rescore_ab"
            ),
        },
        # Only two machine states were recorded: one before the HEAD runs and one
        # after the last run. The intermediate sections of the campaign are not
        # documented by a snapshot, and the report says so.
        "machines": {
            "before": machine(raw / "machine-before.txt"),
            "after": machine(raw / "machine-after.txt"),
        },
        "measured": {
            "head_rescore_ab": {
                str(n): ab_points(raw / f"head-ab-n{n}.json") for n in (1000, 5000)
            },
            "head_dense_engine": {
                str(n): dense_points(raw / f"head-dense-n{n}.json") for n in (1000, 5000)
            },
            "head_dense_engine_limit_k": {
                str(n): dense_points(raw / f"head-dense-limit100-n{n}.json")
                for n in (1000, 5000)
            },
            "replay_rescore_ab": {
                str(n): ab_points(raw / f"{REPLAY_SHORT}-ab-n{n}.json") for n in (1000, 5000)
            },
            "replay_dense_engine": {
                str(n): dense_points(raw / f"{REPLAY_SHORT}-dense-n{n}.json") for n in (1000, 5000)
            },
            "head_residency": ab_points(raw / "head-ab-n1000-residency.json"),
        },
        "profile": profile(raw / "sample-engine-n1000.txt"),
        "published_reference": {
            "source": "docs/mesures/MES-03-latence-rescoring.md, "
            "'Lecture groupée des vecteurs (ADR-0080)', table 'Série complète à chaud'",
            "harness": "rescore_ab, limit = k = 100",
            "grouped_per_pass_p50_ms": {
                "1000": [15.07, 0.76, 0.73],
                "2000": [14.72, 1.02, 0.97],
                "5000": [13.90, 1.97, 1.87],
            },
            "grouped_combined_p50_ms": {"1000": 0.79, "2000": 1.13, "5000": 2.17},
            "base_per_pass_p50_ms": {
                "1000": [39.92, 10.16, 0.89],
                "2000": [30.00, 9.77, 1.38],
                "5000": [56.07, 11.06, 2.73],
            },
        },
    }

    arguments.out.write_text(json.dumps(document, indent=2, ensure_ascii=False) + "\n")
    print(f"wrote {arguments.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
