#!/usr/bin/env python3
"""Merges the per-point JSON of MES-13 into one raw-data file.

Usage:
    python3 tools/bench/mes13/merge.py <out.json> <point.json>...

Each input is the document `startup_cost` writes for one invocation (one point,
one cache state). The output groups them by label and keeps the campaign
metadata of the first input.

The `index`, `records_sqlite` and `copy` paths are normalised to paths relative
to the repository root, as the artefacts of MES-02, MES-03, MES-07 and MES-08
do: the bench receives absolute paths from `run.sh` (cargo runs it from the
package root), and the merged file is versioned, so it never carries a local
path.
"""

import json
import sys
from pathlib import Path

#: Repository root, three levels above this file.
ROOT = Path(__file__).resolve().parents[3]


def relative(path):
    """Returns `path` relative to the repository root.

    A path outside the repository is refused: the artefact is versioned and
    must not name a directory of the measurement machine.
    """
    if path is None:
        return None
    candidate = Path(path)
    if not candidate.is_absolute():
        return candidate.as_posix()
    try:
        return candidate.relative_to(ROOT).as_posix()
    except ValueError as error:
        raise SystemExit(
            f"{path} is outside the repository: an artefact must not carry a local path"
        ) from error


def normalise(point, source):
    """Makes the paths of one point relative, refusing a missing `index`."""
    index = point.get("index")
    if index is None:
        raise SystemExit(f"{source}: no index in the point, cannot make it relative")
    point["index"] = relative(index)
    if "records_sqlite" in point:
        point["records_sqlite"] = relative(point["records_sqlite"])
    for phase in point.get("phases", ()):
        copy = phase.get("copy")
        if copy is None:
            raise SystemExit(f"{source}: a phase has no copy, cannot make it relative")
        phase["copy"] = relative(copy)


def main(argv: list[str]) -> int:
    if len(argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    out = Path(argv[1])
    points = [json.loads(Path(path).read_text(encoding="utf-8")) for path in argv[2:]]
    for path, point in zip(argv[2:], points):
        normalise(point, path)
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
