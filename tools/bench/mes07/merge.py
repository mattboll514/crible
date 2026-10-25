#!/usr/bin/env python3
"""Merge the per-index JSON reports of the MES-07 bench into one file.

Usage:
    python3 tools/bench/mes07/merge.py <output.json> <input.json>...

Each input is a report written by `compact_cost measure` (one index). The
campaign metadata comes from the first file. The `.time` file written by
`/usr/bin/time -l` next to the report (same name, `.time` extension) gives the
peak resident set size of the process, added to the report. A missing `.time`
file, or one without the `maximum resident set size` line, stops the merge with
an error: the value is never guessed.
"""

import json
import os
import sys

CAMPAIGN_KEYS = ("measure", "commit", "machine", "date", "repeats")


def process_max_rss(time_path):
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
    reports = []
    for path in inputs:
        with open(path, encoding="utf-8") as report_file:
            report = json.load(report_file)
        if campaign is None:
            campaign = {key: report[key] for key in CAMPAIGN_KEYS if key in report}
        time_path = path[: -len(".json")] + ".time" if path.endswith(".json") else path + ".time"
        if not os.path.exists(time_path):
            raise SystemExit(f"{path}: no .time file, process peak RSS unknown")
        report["process_max_rss_bytes"] = process_max_rss(time_path)
        reports.append(report)
    with open(output, "w", encoding="utf-8") as output_file:
        json.dump({"campaign": campaign, "indexes": reports}, output_file, indent=2)
        output_file.write("\n")


if __name__ == "__main__":
    main(sys.argv)
