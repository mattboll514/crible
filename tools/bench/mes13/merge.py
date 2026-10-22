#!/usr/bin/env python3
"""Merges the per-point JSON of MES-13 into one raw-data file.

Usage:
    python3 tools/bench/mes13/merge.py <out.json> <point.json>...

Each input is the document `startup_cost` writes for one invocation (one point,
one cache state). The output groups them by label and keeps the campaign
metadata of the first input.
"""

import json
import sys
from pathlib import Path


def main(argv: list[str]) -> int:
    if len(argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    out = Path(argv[1])
    points = [json.loads(Path(path).read_text(encoding="utf-8")) for path in argv[2:]]
    first = points[0]
    document = {
        "measure": "MES-13",
        "date": first.get("date"),
        "commit": first.get("commit"),
        "machine": first.get("machine"),
        "points": points,
    }
    for point in points:
        for key in ("measure", "date", "commit", "machine"):
            if point.get(key) != first.get(key):
                raise SystemExit(
                    f"{key} differs between inputs: {point.get(key)!r} against "
                    f"{first.get(key)!r}"
                )
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
