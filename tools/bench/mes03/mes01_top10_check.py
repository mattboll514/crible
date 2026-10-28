#!/usr/bin/env python3
"""Top-10 control of the MES-01 harness, with the final `REDUIRE-COUT-DES-CLES` build.

The task requires **0 difference** on the top-10 of the MES-01 harness (ADR-0079,
decision 6). This script replays the engine side of the main campaign of MES-01
with a chosen ``fidelity`` binary, on a **copy** of the measurement indexes (the
copy is written, the original stays read-only), and compares, query by query:

- the keys of the top-10, in order;
- the scores, bit for bit;
- ``doc_id`` and ``ord``.

The reference is ``data/mes01/engine/main/<structure>/n<N>.json``, which is what
the MES-01 campaign recorded on 2026-10-27 at commit ``9ca3b07``. A second
comparison runs the same script with the ``base`` binary, so the report can say
whether the difference, if any, comes from the task or from the commits between
MES-01 and `HEAD`.

    python3 tools/bench/mes03/mes01_top10_check.py --variant final \
        --copy data/reduire-cles/mes01-copy --out data/reduire-cles/raw/mes01-final.json
"""

from __future__ import annotations

import argparse
import json
import struct
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
STRUCTURES = ("articles", "decisions", "dole", "jorf", "sarde", "titres")
DEFAULT_NS = (200, 500, 1000, 2000, 5000)


def set_rescore(index_dir: Path, rescore: int) -> None:
    path = index_dir / "index.json"
    config = json.loads(path.read_text(encoding="utf-8"))
    config["search"]["rescore_candidates"] = rescore
    config["search"]["exact_threshold"] = 0
    config["search"]["scan"] = "binary"
    path.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")


def top(record: dict) -> list[tuple]:
    return [
        (row["key"], struct.pack("<f", row["score"]).hex(), row["doc_id"], row["ord"])
        for row in record["results"]
    ]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--variant", required=True)
    parser.add_argument("--copy", default="data/reduire-cles/mes01-copy")
    parser.add_argument("--reference", default="data/mes01")
    parser.add_argument("--bin-dir", default="data/reduire-cles/bin")
    parser.add_argument("--out", required=True)
    parser.add_argument("--ns", default=",".join(str(n) for n in DEFAULT_NS))
    parser.add_argument(
        "--binary", default=None, help="fidelity binary; defaults to <bin-dir>/<variant>"
    )
    arguments = parser.parse_args()

    binary = Path(arguments.binary or Path(arguments.bin_dir) / arguments.variant / "fidelity")
    copy = Path(arguments.copy)
    reference = Path(arguments.reference)
    raw = Path(arguments.out).parent
    raw.mkdir(parents=True, exist_ok=True)
    ns = [int(text) for text in arguments.ns.split(",")]

    summary = {"variant": arguments.variant, "binary": str(binary), "points": []}
    for structure in STRUCTURES:
        queries = reference / "work" / f"main-{structure}.jsonl"
        vectors = reference / "work" / f"main-{structure}.f32"
        for n in ns:
            set_rescore(copy / "indexes" / structure, n)
            out = raw / f"mes01-{arguments.variant}-{structure}-{n}.json"
            command = [
                str(binary),
                "--data",
                str(copy),
                "--index",
                structure,
                "--vectors",
                str(vectors),
                "--queries",
                str(queries),
                "--k",
                "10",
                "--out",
                str(out),
            ]
            completed = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
            if completed.returncode != 0:
                raise SystemExit(
                    f"{structure} N = {n}: fidelity failed with {completed.returncode}: "
                    f"{completed.stderr.strip()}"
                )
            produced = json.loads(out.read_text(encoding="utf-8"))
            recorded = json.loads(
                (reference / "engine" / "main" / structure / f"n{n}.json").read_text(
                    encoding="utf-8"
                )
            )
            produced_by_id = {row["id"]: row for row in produced["results"]}
            differing = []
            for expected in recorded["results"]:
                got = produced_by_id.get(expected["id"])
                if got is None:
                    differing.append(expected["id"])
                    continue
                if top(got) != top(expected):
                    differing.append(expected["id"])
            point = {
                "structure": structure,
                "n": n,
                "queries": len(recorded["results"]),
                "differing": differing,
                "plans": sorted({row["dense_strategy"] for row in produced["results"]}),
                "rescore_candidates": sorted(
                    {row["rescore_candidates"] for row in produced["results"]}
                ),
            }
            summary["points"].append(point)
            print(
                f"{structure} N = {n}: {len(differing)} differing of "
                f"{len(recorded['results'])}, plans {point['plans']}",
                flush=True,
            )
    Path(arguments.out).write_text(
        json.dumps(summary, ensure_ascii=False, indent=1) + "\n", encoding="utf-8"
    )
    total = sum(len(point["differing"]) for point in summary["points"])
    print(f"total differing: {total}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
