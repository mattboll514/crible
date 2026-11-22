#!/usr/bin/env python3
"""Aggregate the two passes of MES-11 (T14) over the 97 reference questions.

Reads the twelve `crible eval` reports of the two passes (six indexes each, the
game of `docs/legifacile/reference/` unchanged), then:

- means recall@10 and MRR@10 per branch (`lexical`, `hybrid`) over the 97
  questions, and per index;
- the paired bootstrap of ADR-0092, decision 6: 10 000 resamplings of the query
  indices, fixed seed, difference `v1 - none`, percentile interval at 95 %.

The convention is ADR-0092's: the difference is `variant - reference`, so a
negative interval means a regression of `v1`.

Usage:

    python3 tools/bench/t14/aggregate.py --none data/t14-mes11/out \
        --v1 data/t14-mes11/out-v1 --out data/t14-mes11/raw/eval-summary.json

The script reads only the reports and writes only `--out`.
"""

import argparse
import json
import os
import random

INDEXES = ["dole", "sarde", "titres", "jorf", "articles", "decisions"]
BRANCHES = ["lexical", "hybrid"]
SEED = 20261111
RESAMPLES = 10000


def read_pass(root):
    """Per index, then per branch, the per-query rows of one pass."""
    out = {}
    for index in INDEXES:
        directory = os.path.join(root, index)
        if not os.path.isdir(directory):
            # The pass 1 reports were written into `none-<index>` directories.
            directory = os.path.join(root, f"none-{index}")
        files = [
            name
            for name in os.listdir(directory)
            if name.startswith(f"eval-{index}-") and name.endswith(".json")
        ]
        if len(files) != 1:
            raise SystemExit(f"expected one report in {directory}, found {files}")
        report = json.load(open(os.path.join(directory, files[0]), encoding="utf-8"))
        out[index] = {
            branch: report["modes"][branch]["queries"] for branch in BRANCHES
        }
    return out


def samples(pass_data, branch):
    """The 97 rows of one branch, in a stable order (index, then game order)."""
    rows = []
    for index in INDEXES:
        rows.extend(pass_data[index][branch])
    return rows


def mean(values):
    return sum(values) / len(values) if values else None


def percentile(sorted_values, fraction):
    """Nearest-rank percentile, as `docs/08` and ADR-0117 read them."""
    if not sorted_values:
        return None
    rank = max(1, round(fraction * len(sorted_values)))
    return sorted_values[min(rank, len(sorted_values)) - 1]


def bootstrap(differences):
    rng = random.Random(SEED)
    n = len(differences)
    means = []
    for _ in range(RESAMPLES):
        total = 0.0
        for _ in range(n):
            total += differences[rng.randrange(n)]
        means.append(total / n)
    means.sort()
    return {
        "resamples": RESAMPLES,
        "seed": SEED,
        "point": mean(differences),
        "ci95": [percentile(means, 0.025), percentile(means, 0.975)],
        "negative": sum(1 for value in means if value < 0) / len(means),
        "positive": sum(1 for value in means if value > 0) / len(means),
        "zero": sum(1 for value in means if value == 0) / len(means),
    }


def summarise(none_data, v1_data):
    summary = {"n": sum(len(none_data[i]["lexical"]) for i in INDEXES), "branches": {}}
    for branch in BRANCHES:
        before = samples(none_data, branch)
        after = samples(v1_data, branch)
        if [row["id"] for row in before] != [row["id"] for row in after]:
            raise SystemExit("the two passes do not carry the same queries")
        recalls_none = [row["quality_recall_at_k"] for row in before]
        recalls_v1 = [row["quality_recall_at_k"] for row in after]
        mrr_none = [row["quality_mrr_at_k"] for row in before]
        mrr_v1 = [row["quality_mrr_at_k"] for row in after]
        differences = [after - before for before, after in zip(recalls_none, recalls_v1)]
        moved = [
            row["id"]
            for row, delta in zip(before, differences)
            if delta != 0
        ]
        summary["branches"][branch] = {
            "none": {"recall_at_10": mean(recalls_none), "mrr_at_10": mean(mrr_none)},
            "v1": {"recall_at_10": mean(recalls_v1), "mrr_at_10": mean(mrr_v1)},
            "delta_recall_at_10": bootstrap(differences),
            "delta_mrr_at_10": {
                "point": mean([a - b for b, a in zip(mrr_none, mrr_v1)]),
            },
            "queries_with_a_different_recall": moved,
        }
    summary["per_index"] = {
        index: {
            branch: {
                "none": mean(
                    [row["quality_recall_at_k"] for row in none_data[index][branch]]
                ),
                "v1": mean(
                    [row["quality_recall_at_k"] for row in v1_data[index][branch]]
                ),
            }
            for branch in BRANCHES
        }
        for index in INDEXES
    }
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--none", required=True)
    parser.add_argument("--v1", required=True)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    summary = summarise(read_pass(args.none), read_pass(args.v1))
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump(summary, handle, ensure_ascii=False, indent=1)
        handle.write("\n")
    print(json.dumps(summary, ensure_ascii=False, indent=1))


if __name__ == "__main__":
    main()
