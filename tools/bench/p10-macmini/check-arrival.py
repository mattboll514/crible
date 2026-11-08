#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Checks the arrival of the snapshots on the Mac mini against `snapshot.json`.

ADR-0064, decision 1: `snapshot.json` lists every file of the snapshot with its
size and its `blake3:<hex>`. This script reads the manifest on the remote host
and reports a missing file or a different size. It needs only the Python
standard library, because the Mac mini has neither `b3sum` nor the `blake3`
module; the blake3 control itself is the one of `crible restore` (ADR-0141,
decision 2).

    ssh … python3 - /Volumes/MattCorp-SSD/crible-exchange/snapshots \
        decisions-e2-g2 articles-e1-g1 < check-arrival.py
"""

import json
import os
import sys


def check(root: str, names: list[str]) -> tuple[int, int, list[str]]:
    problems: list[str] = []
    files = 0
    total = 0
    for name in names:
        directory = os.path.join(root, name)
        manifest = json.load(open(os.path.join(directory, "snapshot.json")))
        local = 0
        for entry in manifest["files"]:
            path = os.path.join(directory, entry["path"])
            files += 1
            if not os.path.exists(path):
                problems.append(f"{name}/{entry['path']}: missing")
                continue
            size = os.path.getsize(path)
            if size != entry["size"]:
                problems.append(f"{name}/{entry['path']}: size {size} != {entry['size']}")
            local += size
            total += size
        print(f"{name}: {len(manifest['files'])} fichiers, {local} octets")
    return files, total, problems


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    files, total, problems = check(argv[0], argv[1:])
    print(f"total: {files} fichiers, {total} octets, {len(problems)} probleme(s)")
    for problem in problems:
        print(f"  {problem}")
    return 0 if not problems else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
