#!/usr/bin/env python3
"""Compact the per-point MES-14 bench reports into one versioned JSON file.

Usage:
    python3 tools/bench/mes14/compact.py <output.json> <before-dir> <after-dir>

`before-dir` and `after-dir` each hold the six files written by
`pool_filter measure` for one campaign: `synth-small.json`, `fixture-1m.json`,
`fixture-8m58.json` and the three `round2-*.json`.

Unlike `merge.py`, this script drops `samples_ns` (1 000 values per case, about
1,4 MB once merged) and keeps the percentiles, the pool size and the scan size.
The timed samples stay in `data/mes14/raw-<date>-<campagne>/`.

`scan_iids` is what the bench measured: the size of the set the **first**
interval clause of the case walks (`present ∩ live ∩ keyword`). A two-bound
range parses as an `And` of two clauses, so the column is walked twice; the
second walk is bounded by what the first kept. The report's "iid lus" column
cumulates both, from a SQL count on the same `records.sqlite`, and that
cumulated value is deliberately **not** written here: it is a derivation, not a
measurement, and the report states where it comes from.

The `controls` block derives the insertion term: the two `annee` controls read
the same set, and only one of them keeps anything, so the difference of their
p50 divided by the kept count is the cost of adding one iid to the result
bitmap. It is arithmetic on two measured p50, labelled `derived`.
"""

import json
import os
import sys

FIXTURES = ("synth-small", "fixture-1m", "fixture-8m58")

# Identity of what was measured, taken from the two campaign runs. The three
# `records.sqlite` digests were checked before each run; `synth-small` was
# re-attached on 2026-10-26, so its digest differs from the 2026-10-22 report,
# but its pool and scan sizes are identical case by case (report section 10).
POOL_RS_SHA256 = "0aa8117cfec84f3653f3c0bd217478cc97de1cc6bd31ae81385f7299d8eeb690"
DATASET_CHECKSUM = (
    "blake3:2d3d8265a8df350d816bccb5732fe338921d275cae5d091a96d0497646e58805"
)
FIXTURE_SHA256 = {
    "synth-small": (
        "75ea8430b0624c2788f18ca4c74c0c25ebbc2a12b3a6018934639510908bf6c8"
    ),
    "fixture-1m": (
        "5ce1be32488c69560c86dfaa7bdad9443c6680211bfa40745ea083cbb142d4f4"
    ),
    "fixture-8m58": (
        "d53184ac6821267a07cdf030aa15370579f9695d729b1d5434b304e809d68c66"
    ),
}


def load(directory, name):
    with open(os.path.join(directory, f"{name}.json"), encoding="utf-8") as source:
        return json.load(source)


def cases_by_label(report):
    return {case["label"]: case for case in report["cases"]}


def strip(case):
    """One case without its timed samples, plus the cost per iid read."""
    kept = {
        key: case[key]
        for key in (
            "label",
            "kind",
            "filter",
            "pool_size",
            "scan_iids",
            "p50_ns",
            "p95_ns",
            "p99_ns",
            "min_ns",
            "max_ns",
        )
    }
    scan = case["scan_iids"]
    kept["ns_per_iid_read_first_clause"] = (
        round(case["p50_ns"] / scan, 3) if scan else None
    )
    return kept


def point(directory, name, round_number):
    report = load(directory, name)
    return {
        "index": os.path.basename(report["index"]),
        "round": round_number,
        "live_passages": report["live_passages"],
        "filter_memory_bytes": report["filter_memory_bytes"],
        "index_build_ns": report["index_build_ns"],
        "records_bytes": report["records_bytes"],
        "cases": [strip(case) for case in report["cases"]],
    }


def comparison(before, after):
    """One row per case, before versus after, plus the correctness check.

    The pool and the scanned set must be the **same** in both runs: the change
    is an optimisation of how the result bitmap is built, so a case whose pool
    or domain size moved means the two runs did not measure the same work, and
    a delta would be meaningless. Raise rather than emit a credible table.
    """
    first_before = cases_by_label(before)
    first_after = cases_by_label(after)
    rows = []
    for label, old in first_before.items():
        new = first_after[label]
        if old["pool_size"] != new["pool_size"]:
            raise SystemExit(
                f"{label}: pool {old['pool_size']} before, {new['pool_size']} after"
                " — the two runs did not measure the same thing"
            )
        if old["scan_iids"] != new["scan_iids"]:
            raise SystemExit(
                f"{label}: scan {old['scan_iids']} before, {new['scan_iids']} after"
                " — the two runs did not measure the same thing"
            )
        rows.append(
            {
                "label": label,
                "kind": old["kind"],
                "pool_size": new["pool_size"],
                "scan_iids": old["scan_iids"],
                "p50_before_ns": old["p50_ns"],
                "p50_after_ns": new["p50_ns"],
                "p95_before_ns": old["p95_ns"],
                "p95_after_ns": new["p95_ns"],
                "delta_p50_percent": round(
                    (new["p50_ns"] / old["p50_ns"] - 1) * 100, 2
                ),
                "delta_p95_percent": round(
                    (new["p95_ns"] / old["p95_ns"] - 1) * 100, 2
                ),
            }
        )
    return rows


def noise(before, before_round2, after, after_round2):
    """Worst round-1 to round-2 gap of either campaign, per case, in percent."""
    rows = []
    b1 = cases_by_label(before)
    b2 = cases_by_label(before_round2)
    a1 = cases_by_label(after)
    a2 = cases_by_label(after_round2)
    for label in b1:
        gap50 = max(
            abs(b2[label]["p50_ns"] / b1[label]["p50_ns"] - 1),
            abs(a2[label]["p50_ns"] / a1[label]["p50_ns"] - 1),
        )
        gap95 = max(
            abs(b2[label]["p95_ns"] / b1[label]["p95_ns"] - 1),
            abs(a2[label]["p95_ns"] / a1[label]["p95_ns"] - 1),
        )
        rows.append(
            {
                "label": label,
                "noise_p50_percent": round(gap50 * 100, 2),
                "noise_p95_percent": round(gap95 * 100, 2),
            }
        )
    return rows


def controls(before, after):
    """Insertion term from the control pair: same reads, none versus all kept."""
    old = cases_by_label(before)
    new = cases_by_label(after)
    none_old = old["controle: annee >= 5000, memes iid lus, aucun garde"]
    none_new = new["controle: annee >= 5000, memes iid lus, aucun garde"]
    all_old = old["controle: annee >= 1900, memes iid lus, tous gardes"]
    all_new = new["controle: annee >= 1900, memes iid lus, tous gardes"]
    reads = none_old["scan_iids"]
    kept = all_old["pool_size"]
    term_old = all_old["p50_ns"] - none_old["p50_ns"]
    term_new = all_new["p50_ns"] - none_new["p50_ns"]
    return {
        "note": (
            "derived: both controls read the same set, only one keeps any iid; "
            "the difference of their p50 divided by the kept count is the cost "
            "of adding one iid to the result bitmap"
        ),
        "reads": reads,
        "kept": kept,
        "read_ns_per_iid_before": round(none_old["p50_ns"] / reads, 3),
        "read_ns_per_iid_after": round(none_new["p50_ns"] / reads, 3),
        "insertion_ns_per_kept_before": round(term_old / kept, 3),
        "insertion_ns_per_kept_after": round(term_new / kept, 3),
        "insertion_term_ms_before": round(term_old / 1e6, 3),
        "insertion_term_ms_after": round(term_new / 1e6, 3),
    }


def main(argv):
    if len(argv) != 4:
        raise SystemExit("usage: compact.py <output.json> <before-dir> <after-dir>")
    output, before_dir, after_dir = argv[1], argv[2], argv[3]

    before = {name: load(before_dir, name) for name in FIXTURES}
    after = {name: load(after_dir, name) for name in FIXTURES}
    before_round2 = {name: load(before_dir, f"round2-{name}") for name in FIXTURES}
    after_round2 = {name: load(after_dir, f"round2-{name}") for name in FIXTURES}

    reference = after["synth-small"]
    document = {
        "campaign": {
            "measure": "MES-14",
            "date": "2026-10-26",
            "task": "INSERER-POOL-EN-ORDRE",
            "before": "releve du 2026-10-26, code au commit 4c7dd6c sans la tache",
            "after": "releve du 2026-10-26, scan_column avec RoaringBitmap::append",
            "commit": "4c7dd6cb794f3f701b1562d15f81f7d5464f7761",
            "pool_rs_sha256": POOL_RS_SHA256,
            "machine": reference["campaign"]["machine"],
            "native": True,
            "profile": "bench",
            "repeats": reference["repeats"],
            "warmup": reference["warmup"],
            "rounds": 2,
            "dataset": "synth-small",
            "dataset_checksum": DATASET_CHECKSUM,
            "percentiles": "nearest rank, no interpolation",
            "cache": "hot, every filter structure in RAM",
        },
        "fixtures": {
            name: {
                "records_sqlite_sha256": FIXTURE_SHA256[name],
                "records_sqlite_bytes": after[name]["records_bytes"],
                "live_passages": after[name]["live_passages"],
                "same_as_2026_10_22": name != "synth-small",
            }
            for name in FIXTURES
        },
        "points": [
            point(after_dir, name, 1) for name in FIXTURES
        ]
        + [point(after_dir, f"round2-{name}", 2) for name in FIXTURES],
        "comparison": {
            name: comparison(before[name], after[name]) for name in FIXTURES
        },
        "noise": {
            name: noise(
                before[name], before_round2[name], after[name], after_round2[name]
            )
            for name in FIXTURES
        },
        "controls": controls(before["fixture-8m58"], after["fixture-8m58"]),
        "controls_1m": controls(before["fixture-1m"], after["fixture-1m"]),
    }
    with open(output, "w", encoding="utf-8") as target:
        json.dump(document, target, indent=1, ensure_ascii=False, sort_keys=False)
        target.write("\n")
    points = len(document["points"])
    print(f"{output}: {points} points, {os.path.getsize(output)} octets")


if __name__ == "__main__":
    main(sys.argv)
