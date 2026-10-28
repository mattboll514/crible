# SPDX-License-Identifier: Apache-2.0
"""Assemble the versioned artifact of the MES-03 sampling profile.

References: `docs/mesures/MES-03-latence-rescoring.md`, ADR-0079 decision 1,
ADR-0077 (measurement protocol).

The campaign writes its raw documents under `data/mes03/` (gitignored). This
script gathers them into one versioned JSON: the cold series, the warm series,
the candidate-order readings and the parameters, so the report can be read
without the 30 MiB of raw text reports of `/usr/bin/sample`.

A point is warm when its run document records at least one warm-up pass
(`warmup_passes > 0`): ADR-0077 asks for untimed warm-up passes before a hot
measurement, and that count says which state the point describes.
"""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path

REPORT_NAME = "MES-03-profil-rescoring.2026-10-27.json"
TARGET = f"docs/mesures/{REPORT_NAME}"

# The series the campaign keeps, in the order the report reads them. The first
# element of a tuple is the state written in the artifact, the second the file
# written by the harness run and the third a one-line description.
SERIES = (
    ("froid", "profile-1000.json", "cold state, 300 queries, one pass over the queries"),
    ("froid", "profile-5000.json", "cold state, 300 queries, one pass over the queries"),
    (
        "chaud, campagne (300 requêtes)",
        "profile-warm-1000.json",
        "warm-up passes then a sampled loop, 300 queries as in ADR-0077",
    ),
    (
        "chaud, campagne (300 requêtes)",
        "profile-warm-5000.json",
        "warm-up passes then a sampled loop, 300 queries as in ADR-0077",
    ),
    (
        "chaud, résidente (40 requêtes)",
        "profile-warm-1000-q40.json",
        "working set small enough to stay resident, same queries at every iteration",
    ),
)

BASE_QUERIES = 300


def order_path(directory: Path, run: dict) -> Path | None:
    """Order document of a series: `order-N.json`, or `order-N-qQ.json`.

    The order reading is taken over the same queries as the profile, so a series
    that keeps only the first `Q` queries uses its own file.
    """
    n = run.get("n")
    queries = run.get("queries")
    if n is None or queries is None:
        return None
    if queries == BASE_QUERIES:
        return directory / f"order-{n}.json"
    return directory / f"order-{n}-q{queries}.json"


def build_point(directory: Path, state: str, name: str, description: str) -> dict | None:
    """One point of the artifact, or None when its run document is absent."""
    path = directory / name
    if not path.exists():
        return None
    profile = json.loads(path.read_text(encoding="utf-8"))
    run = profile["run"]
    order = None
    order_file = order_path(directory, run)
    if order_file is not None and order_file.exists():
        order = json.loads(order_file.read_text(encoding="utf-8"))["order"]
        order = {key: value for key, value in order.items() if key != "per_query"}
    return {
        "state": state,
        "description": description,
        "source": name,
        "n": run.get("n"),
        "run": run,
        "total_samples": profile.get("total_samples"),
        "unattributed_samples": profile.get("unattributed_samples"),
        "seconds_sampled": 20,
        "sample_interval_ms": 1,
        "posts": profile.get("posts"),
        "groups": profile.get("groups"),
        "order": order,
    }


def build(directory: Path, commit: str) -> dict:
    """The versioned document, with the parameters and every series found."""
    points = []
    for state, name, description in SERIES:
        point = build_point(directory, state, name, description)
        if point is not None:
            points.append(point)
    return {
        "kind": "MES-03-profile",
        "task": "REDUIRE-COUT-DES-CLES",
        "date": "2026-10-27",
        "commit": commit,
        "tool": "/usr/bin/sample, 20 s at 1 ms, on crates/crible/examples/rescore_profile.rs",
        "build": "CARGO_PROFILE_RELEASE_DEBUG=true cargo build --release --example rescore_profile",
        "data": (
            "data/mes02/indexes/synth-large, generation 2, epoch 1, "
            "7740000 live passages, fp16, 768 dims"
        ),
        "raw": "data/mes03/profile-*.json, sample-rescore-*.txt, order-*.json (gitignored)",
        "warm_method": (
            "untimed warm-up passes over the same queries, then one sampled loop; "
            "a point is warm when its run records warmup_passes > 0"
        ),
        "points": points,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dir", default="data/mes03", type=Path, help="raw documents")
    parser.add_argument("--out", default=Path(TARGET), type=Path, help="versioned artifact")
    arguments = parser.parse_args()
    commit = subprocess.run(
        ["git", "rev-parse", "HEAD"], capture_output=True, text=True, check=True
    ).stdout.strip()
    document = build(arguments.dir, commit)
    arguments.out.parent.mkdir(parents=True, exist_ok=True)
    arguments.out.write_text(
        json.dumps(document, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    kept = ", ".join(str(point["source"]) for point in document["points"])
    print(f"wrote {arguments.out}: {len(document['points'])} points ({kept})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
