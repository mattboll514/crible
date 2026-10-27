# SPDX-License-Identifier: Apache-2.0
"""Two controls the protocol of MES-01 relies on, made replayable.

The report claims that the measurement is deterministic and that the example
returns what the shipped CLI returns. Both claims were true when the campaign
ran; this script keeps them reproducible instead of leaving them to a shell
session:

1. **Determinism.** The example is run twice on ``articles`` at ``N = 2000`` and
   the two files are compared byte for byte.
2. **CLI equivalence.** The same vector is passed to ``crible search --k 10`` and
   to the example; the keys and the scores must match, in the same order. This
   is what justifies ``k = 10`` against the ``candidates_per_branch = 100`` the
   CLI plans.

Run from the repository root, after ``mes01_campaign.py``::

    (cd tools && uv run --no-sync python bench/mes01/mes01_controls.py)
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

import numpy as np

from mes01_campaign import set_rescore

#: Repository root, three levels above this file.
ROOT = Path(__file__).resolve().parents[3]
#: Structure and ``N`` the two controls use.
CONTROL_STRUCTURE = "articles"
CONTROL_N = 2000
#: Query the CLI comparison replays.
CONTROL_QUERY = "Q01"


def sha256(path: Path) -> str:
    """SHA-256 of a file, lowercase hexadecimal."""
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def run_example(data: Path, work: Path, out: Path) -> None:
    """Runs the example on the control slice."""
    command = [
        str(ROOT / "target/release/examples/fidelity"),
        "--data",
        str(data),
        "--index",
        CONTROL_STRUCTURE,
        "--vectors",
        str(work / f"main-{CONTROL_STRUCTURE}.f32"),
        "--queries",
        str(work / f"main-{CONTROL_STRUCTURE}.jsonl"),
        "--k",
        "10",
        "--out",
        str(out),
    ]
    print("+", " ".join(command), flush=True)
    completed = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
    if completed.returncode != 0:
        raise SystemExit(
            f"the example failed with {completed.returncode}: {completed.stderr.strip()}"
        )


def query_vector(data: Path, query_id: str, out: Path) -> np.ndarray:
    """Writes the fp32 vector of one query as a ``.npy`` file and returns it."""
    texts = [
        json.loads(line)
        for line in (data / "query-texts.jsonl").read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    row = next(index for index, record in enumerate(texts) if record["id"] == query_id)
    matrix = np.fromfile(data / "vectors-fp32.f32", dtype="<f4").reshape(-1, 768)
    vector = matrix[row]
    np.save(out, vector)
    return vector


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", type=Path, default=ROOT / "data/mes01")
    arguments = parser.parse_args()

    work = arguments.data / "work"
    if not (work / f"main-{CONTROL_STRUCTURE}.jsonl").exists():
        raise SystemExit(f"{work} holds no query slice; run mes01_campaign.py first")

    set_rescore(arguments.data / "indexes" / CONTROL_STRUCTURE, CONTROL_N)

    first = work / "controls-determinism-a.json"
    second = work / "controls-determinism-b.json"
    run_example(arguments.data, work, first)
    run_example(arguments.data, work, second)
    digest_a = sha256(first)
    digest_b = sha256(second)
    if digest_a != digest_b:
        raise SystemExit(
            f"the two runs of {CONTROL_STRUCTURE} at N = {CONTROL_N} differ; "
            "the measurement is not deterministic"
        )

    example = json.loads(first.read_text(encoding="utf-8"))
    record = next(item for item in example["results"] if item["id"] == CONTROL_QUERY)
    vector_file = work / f"controls-{CONTROL_QUERY.lower()}.npy"
    query_vector(arguments.data, CONTROL_QUERY, vector_file)
    command = [
        str(ROOT / "target/release/crible"),
        "search",
        "--data",
        str(arguments.data),
        "--index",
        CONTROL_STRUCTURE,
        "--vector-file",
        str(vector_file),
        "--k",
        "10",
    ]
    print("+", " ".join(command), flush=True)
    completed = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
    if completed.returncode != 0:
        raise SystemExit(
            f"crible search failed with {completed.returncode}: {completed.stderr.strip()}"
        )
    cli = json.loads(completed.stdout)

    example_keys = [result["key"] for result in record["results"]]
    example_scores = [result["score"] for result in record["results"]]
    cli_keys = [result["key"] for result in cli["results"]]
    cli_scores = [result["score"] for result in cli["results"]]
    if example_keys != cli_keys:
        raise SystemExit(
            f"the CLI and the example disagree on the top-10 of {CONTROL_QUERY}"
        )
    if example_scores != cli_scores:
        raise SystemExit(
            f"the CLI and the example agree on the keys but not on the scores of "
            f"{CONTROL_QUERY}"
        )

    report = {
        "determinism": {
            "index": CONTROL_STRUCTURE,
            "n": CONTROL_N,
            "queries": example["queries"],
            "sha256_first": digest_a,
            "sha256_second": digest_b,
            "identical": True,
        },
        "cli_equivalence": {
            "index": CONTROL_STRUCTURE,
            "n": CONTROL_N,
            "query": CONTROL_QUERY,
            "k": 10,
            "results": len(cli_keys),
            "keys_equal": True,
            "scores_equal": True,
            "command": (
                "crible search --data data/mes01 --index articles "
                "--vector-file data/mes01/work/controls-q01.npy --k 10"
            ),
        },
    }
    (arguments.data / "controls.json").write_text(
        json.dumps(report, indent=2) + "\n", encoding="utf-8"
    )
    print(
        f"determinism ok ({digest_a[:16]}), CLI and example agree on the top-10 "
        f"of {CONTROL_QUERY}"
    )


if __name__ == "__main__":
    sys.exit(main())
