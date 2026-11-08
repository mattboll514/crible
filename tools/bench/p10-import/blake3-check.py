#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Recomputes the blake3 of every file of a snapshot and compares it to `snapshot.json`.

ADR-0064, decision 1: `snapshot.json` lists every copied file with its size and
its `blake3:<hex>`. The CLI checks that at restore; this script checks it on the
host, without Docker, with the `blake3` module of `tools/pyproject.toml`.

    cd tools && uv run --no-sync python ../tools/bench/p10-import/blake3-check.py \\
        ../data/p10-exchange/snapshots/dole-e1-g1

The script prints one line per snapshot and exits non-zero on the first mismatch.
"""

import json
import pathlib
import sys

from blake3 import blake3


def check(directory: pathlib.Path) -> tuple[list[str], int]:
    manifest = json.loads((directory / "snapshot.json").read_text(encoding="utf-8"))
    problems = []
    checked = 0
    for entry in manifest["files"]:
        path = directory / entry["path"]
        if not path.exists():
            problems.append(f"{entry['path']}: missing")
            continue
        hasher = blake3()
        with path.open("rb") as source:
            for block in iter(lambda: source.read(1 << 20), b""):
                hasher.update(block)
        digest = hasher.hexdigest()
        expected = entry["blake3"].removeprefix("blake3:")
        if digest != expected:
            problems.append(f"{entry['path']}: blake3 {digest} != {expected}")
        if path.stat().st_size != entry["size"]:
            problems.append(f"{entry['path']}: size {path.stat().st_size} != {entry['size']}")
        checked += 1
    return problems, checked


def main(argv: list[str]) -> int:
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    failed = 0
    for argument in argv:
        directory = pathlib.Path(argument)
        problems, checked = check(directory)
        print(f"{directory.name}: {checked} fichiers, "
              f"{'blake3 conforme' if not problems else f'{len(problems)} problème(s)'}")
        for problem in problems:
            print(f"  {problem}")
        failed += len(problems)
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
