#!/usr/bin/env python3
"""Merge the per-point JSON reports of the MES-14 bench into one file.

Usage:
    python3 tools/bench/mes14/merge.py <output.json> <input.json>...

Each input is a report written by `pool_filter measure` (one point: one index).
Points are copied as they are, except the index path, replaced by its base name,
and tagged `round` 2 when the file name starts with `round2-` (reproducibility
round). The campaign metadata comes from the first file.

Checksums are read from `checksum-records.json` next to the input files, written
by `run.sh` (`shasum -a 256` of each measured `records.sqlite`); a missing entry
stops the merge with an error rather than leaving the field out silently.
"""

import json
import os
import sys


def main(argv):
    if len(argv) < 3:
        raise SystemExit("usage: merge.py <output.json> <input.json>...")
    output, inputs = argv[1], argv[2:]
    checksum_path = os.path.join(os.path.dirname(inputs[0]), "checksum-records.json")
    with open(checksum_path, encoding="utf-8") as checksum_file:
        checksums = json.load(checksum_file)
    dataset_checksum_path = os.path.join(
        os.path.dirname(inputs[0]), "checksum-synth-small.txt"
    )
    with open(dataset_checksum_path, encoding="utf-8") as dataset_checksum_file:
        dataset_checksum = dataset_checksum_file.read().strip()

    campaign = None
    points = []
    for path in inputs:
        with open(path, encoding="utf-8") as report_file:
            report = json.load(report_file)
        name = os.path.basename(report["index"])
        if name not in checksums:
            raise SystemExit(f"{path}: no checksum for index {name!r}")
        if campaign is None:
            campaign = dict(report["campaign"])
            campaign["dataset"] = "synth-small"
            campaign["dataset_checksum"] = dataset_checksum
        point = dict(report)
        point["index"] = name
        point["records_sqlite_sha256"] = checksums[name]["records_sqlite_sha256"]
        if point.get("campaign", {}).get("round") == 2:
            point["round"] = 2
        points.append(point)
    document = {"campaign": campaign, "points": points}
    with open(output, "w", encoding="utf-8") as output_file:
        json.dump(document, output_file, indent=2, sort_keys=False)
        output_file.write("\n")
    print(f"{output}: {len(points)} points")


if __name__ == "__main__":
    main(sys.argv)
