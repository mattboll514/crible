# SPDX-License-Identifier: Apache-2.0
"""Build the measurement indexes of MES-01 and MES-12.

Copies the provisional configuration of each structure of the Legifacile sample,
forces ``search.exact_threshold`` to ``0`` (ADR-0076, decision 4: BINARY is kept
for every non-empty pool), and builds a fresh index under ``data/mes01/indexes``
from the sample batches. The sample batches and the sample indexes are never
modified.

Run from the repository root, after ``cargo build --release``::

    uv run --no-sync --project tools python tools/bench/mes01/mes01_build_indexes.py
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

#: Repository root, three levels above this file.
ROOT = Path(__file__).resolve().parents[3]
#: Structures of the sample, in the order of ``docs/08``.
STRUCTURES = ("articles", "decisions", "dole", "jorf", "sarde", "titres")


def patched_config(structure: str, rescore: int) -> dict:
    """Returns the sample configuration with the measurement settings."""
    source = ROOT / "data/legifacile-sample/configs" / f"{structure}.json"
    config = json.loads(source.read_text(encoding="utf-8"))
    config["search"]["scan"] = "binary"
    config["search"]["rescore_candidates"] = rescore
    # ADR-0076: a pool of at most N passages would otherwise be searched
    # exactly, and the comparison would be 1.0 by construction.
    config["search"]["exact_threshold"] = 0
    return config


def run(command: list[str]) -> None:
    """Runs one command, echoing it, and stops on a non-zero exit."""
    print("+", " ".join(command), flush=True)
    completed = subprocess.run(command, cwd=ROOT)
    if completed.returncode != 0:
        raise SystemExit(f"command failed with {completed.returncode}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", type=Path, default=ROOT / "data/mes01")
    parser.add_argument("--rescore", type=int, default=1000)
    parser.add_argument(
        "--structures",
        nargs="+",
        choices=STRUCTURES,
        default=list(STRUCTURES),
    )
    arguments = parser.parse_args()

    binary = ROOT / "target/release/crible"
    if not binary.exists():
        raise SystemExit(f"{binary} is missing; run `cargo build --release` first")
    configs = arguments.data / "configs"
    configs.mkdir(parents=True, exist_ok=True)

    for structure in arguments.structures:
        config_path = configs / f"{structure}.json"
        config_path.write_text(
            json.dumps(patched_config(structure, arguments.rescore), indent=2) + "\n",
            encoding="utf-8",
        )
        index_dir = arguments.data / "indexes" / structure
        if not (index_dir / "CURRENT_EPOCH").exists():
            run([str(binary), "index", "create", "--data", str(arguments.data), "--config", str(config_path)])
        run(
            [
                str(binary),
                "attach",
                "--data",
                str(arguments.data),
                "--models",
                str(ROOT / "data/models"),
                "--index",
                structure,
                str(
                    ROOT
                    / "data/legifacile-sample/batches"
                    / f"legifacile-sample-{structure}-v1"
                ),
            ]
        )

    print("measurement indexes under", arguments.data / "indexes")


if __name__ == "__main__":
    sys.exit(main())
