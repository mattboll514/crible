#!/usr/bin/env python3
"""Merge the per-point JSON reports of the MES-15 bench into one file.

Usage:
    python3 tools/bench/mes15/merge.py <output.json> <input.json>...

Each input is a report written by `commit_cost measure` (one point). Points are
copied as they are, without the absolute index path, and tagged `round` 2 when
the file name starts with `round2-` (reproducibility round). The campaign
metadata comes from the first file.

The `.time` file written by `/usr/bin/time -l` next to the report (same name,
`.time` extension) gives the peak resident set size of the process, added to the
point as `process_max_rss_bytes`. A missing `.time` file, or a `.time` file
without the `maximum resident set size` line, stops the merge with an error: the
value is never guessed.
"""

import json
import os
import sys

CAMPAIGN_KEYS = (
    "measure",
    "commit",
    "machine",
    "date",
    "repeats",
    "new_passages",
    "tombstones",
    "replica_runs",
)


def process_max_rss(time_path):
    """Peak resident set size of the process, read from a `/usr/bin/time -l` file."""
    with open(time_path, encoding="utf-8") as time_file:
        for line in time_file:
            if "maximum resident set size" in line:
                return int(line.split()[0])
    raise SystemExit(f"{time_path}: no `maximum resident set size` line")


def main(argv):
    if len(argv) < 3:
        raise SystemExit("usage: merge.py <output.json> <input.json>...")
    output, inputs = argv[1], argv[2:]
    campaign = None
    points = []
    for path in inputs:
        with open(path, encoding="utf-8") as report_file:
            report = json.load(report_file)
        if campaign is None:
            campaign = {key: report[key] for key in CAMPAIGN_KEYS if key in report}
            campaign["vector_files_control"] = "sparse (except i-1000000-s10-empty)"
        if path.endswith(".json"):
            time_path = path[: -len(".json")] + ".time"
        else:
            time_path = path + ".time"
        if not os.path.exists(time_path):
            raise SystemExit(f"{path}: no .time file, process peak RSS unknown")
        peak = process_max_rss(time_path)
        for point in report["points"]:
            point.pop("index", None)
            point["round"] = 2 if os.path.basename(path).startswith("round2-") else 1
            point["process_max_rss_bytes"] = peak
            points.append(point)
    with open(output, "w", encoding="utf-8") as output_file:
        json.dump({"campaign": campaign, "points": points}, output_file, indent=2)
        output_file.write("\n")


if __name__ == "__main__":
    main(sys.argv)
