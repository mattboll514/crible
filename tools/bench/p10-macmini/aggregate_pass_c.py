#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Aggregate pass C of `MESURER-SUR-MAC-MINI` (P10, ADR-0141).

Pass C redoes MES-02, MES-03, MES-05 and MES-09 in the container of the Mac
mini, replays the INV-12 recipe on `dole`, and reads the latency of the served
instance through the HTTP API with a cold query cache. This script recomputes every percentile from
the raw durations, **refuses** a figure that differs from the summary written by
the harness, and writes the versioned companions of the reports. It never reads
or writes a query text: the raw documents carry identifiers and timings only
(ADR-0125, decision 16).

    python3 tools/bench/p10-macmini/aggregate_pass_c.py

The raw files are under `data/p10-macmini/pass-c/raw/` and
`data/p10-macmini/pass-c/mes09/` (gitignored). The four outputs go to
`docs/mesures/`.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any, Iterable, Mapping

import macmini

SCAN_FILES = {
    "decisions": {
        "main": "scan-main-n{n}.json",
        "all": "scan-all-n{n}.json",
    },
    "articles": {
        "main": "scan-articles-main-n{n}.json",
        "all": "scan-articles-all-n{n}.json",
    },
}
SIZES = (1000, 2000, 5000)

DATE = "2026-11-07"
COMMIT = "439d946bf28d63214e64aa7cb2e09e458f81a45d"
VM_BYTES = 10_418_937_856
VM_CPUS = 8
VM_PAGE = 4096


def fail(message: str) -> None:
    raise macmini.MacMiniError(message)


def check(condition: bool, message: str) -> None:
    if not condition:
        fail(message)


def close(left: float, right: float, *, tol: float = 1e-6) -> bool:
    return abs(float(left) - float(right)) <= tol * max(1.0, abs(float(right)))


def summary(durations: Iterable[float]) -> dict[str, Any]:
    values = list(durations)
    check(bool(values), "empty duration series")
    stats = macmini.stage_percentiles(values)
    return {
        "n": len(values),
        "p50": stats["p50"],
        "p95": stats["p95"],
        "p99": stats["p99"],
        "min": min(values),
        "max": max(values),
        "mean": sum(values) / len(values),
    }


def check_summary(found: Mapping[str, Any], computed: Mapping[str, Any], where: str) -> None:
    check(int(found["n"]) == int(computed["n"]), f"{where}: n differs")
    for key in ("p50", "p95", "p99", "min", "max"):
        check(
            close(found[key], computed[key]),
            f"{where}: {key} differs ({found[key]} against {computed[key]})",
        )


def scan_points(raw: Path) -> list[dict[str, Any]]:
    points = []
    for index, files in SCAN_FILES.items():
        for series, pattern in files.items():
            for size in SIZES:
                path = raw / pattern.format(n=size)
                document = macmini.read_json(path)
                check(document.get("kind") == "scan", f"{path}: not a scan document")
                check(document.get("index") == index, f"{path}: index differs")
                for point in document["points"]:
                    computed = summary(point["durations_us"])
                    check_summary(
                        point["summary"],
                        computed,
                        f"{path.name} size={point['size']} shape={point['shape']}",
                    )
                    points.append(
                        {
                            "index": index,
                            "series": series,
                            "candidates": point["n"],
                            "size": point["size"],
                            "shape": point["shape"],
                            "plan": point["plan"],
                            "traversal": point["traversal"],
                            "samples": computed,
                            "pass_summaries": point["pass_summaries"],
                        }
                    )
    return points


def rescore_points(raw: Path) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    points = []
    for size in SIZES:
        path = raw / f"rescore-warm-articles-n{size}.json"
        document = macmini.read_json(path)
        check(document.get("kind") == "rescore", f"{path}: not a rescore document")
        point = document["points"][0]
        all_values = point["durations_us"]
        computed_all = summary(all_values)
        check_summary(point["summary"], computed_all, f"{path.name} all passes")
        effective = all_values[point["queries"] :]
        check(len(effective) == (point["passes"] - 1) * point["queries"], f"{path}: pass count")
        points.append(
            {
                "state": "warm",
                "candidates": point["candidates"],
                "samples": summary(effective),
                "pass_summaries": point["pass_summaries"],
                "discarded_first_pass": True,
                "plan": point["plan"],
                "plan_candidates": point["plan_candidates"],
                "score_ties": point["score_ties"],
            }
        )

        path = raw / f"rescore-cold-articles-n{size}.json"
        document = macmini.read_json(path)
        check(document.get("kind") == "rescore", f"{path}: not a rescore document")
        point = document["points"][0]
        computed = summary(point["durations_us"])
        check_summary(point["summary"], computed, f"{path.name}")
        residence = point["resident_fraction"]
        points.append(
            {
                "state": "cold",
                "candidates": point["candidates"],
                "samples": computed,
                "pass_summaries": point["pass_summaries"],
                "plan": point["plan"],
                "plan_candidates": point["plan_candidates"],
                "score_ties": point["score_ties"],
                "residence": {
                    "mean": residence["mean"],
                    "min": residence["min"],
                    "max": residence["max"],
                },
            }
        )

    path = raw / "rescore-warm-articles-dot-keys.json"
    document = macmini.read_json(path)
    split = []
    for point in document["points"]:
        dot = summary(point["dot_only_us"])
        key = summary(point["key_only_us"])
        check_summary(point["dot_only_summary"], dot, f"{path.name} dot N={point['candidates']}")
        check_summary(point["key_only_summary"], key, f"{path.name} key N={point['candidates']}")
        split.append(
            {
                "candidates": point["candidates"],
                "dot": dot,
                "key": key,
            }
        )

    invalid = {
        "index": "decisions",
        "files": sorted(p.name for p in raw.glob("rescore-warm-n*.json")),
        "reason": (
            "the restored `decisions` index has four segments, and the harness rescored "
            "against the first segment's vectors only; the figures are not published"
        ),
    }
    return points, {"dot_key_split": split, "invalid_attempts": [invalid]}


def encode_points(raw: Path) -> list[dict[str, Any]]:
    points = []
    for precision in ("int8", "fp32"):
        paths = sorted(raw.glob(f"mes05-encode-{precision}-*.json"))
        if not paths:
            continue
        runs = []
        pooled: list[float] = []
        for path in paths:
            document = macmini.read_json(path)
            check(document.get("precision") == precision, f"{path}: precision differs")
            check(int(document.get("cache_capacity", -1)) == 0, f"{path}: cache enabled")
            values = [value / 1000.0 for value in document["calls_us"]]
            check(len(values) == int(document["count"]), f"{path}: count differs")
            runs.append(
                {
                    "file": path.name,
                    "samples": summary(values),
                    "first_call_ms": document["first_call_ms"],
                    "cache_hits": document["cache_hits"],
                    "cache_misses": document["cache_misses"],
                    "threads": document.get("threads"),
                }
            )
            pooled.extend(values)
        points.append(
            {
                "precision": precision,
                "precision_effective": macmini.read_json(paths[0]).get(
                    "precision_effective"
                ),
                "runs": runs,
                "pooled": summary(pooled),
            }
        )
    return points


def parse_state(path: Path) -> dict[str, Any]:
    text = path.read_text(encoding="utf-8")
    state: dict[str, Any] = {"file": path.name, "raw": text.strip().splitlines()}
    match = re.search(r"^VmRSS:\s+(\d+) kB$", text, re.MULTILINE)
    if match:
        state["vmrss_kb"] = int(match.group(1))
    match = re.search(r"^VmSwap:\s+(\d+) kB$", text, re.MULTILINE)
    if match:
        state["vmswap_kb"] = int(match.group(1))
    match = re.search(r"^memory\.current\n(\d+)$", text, re.MULTILINE)
    if match:
        state["memory_current"] = int(match.group(1))
    for key in ("anon", "file", "file_mapped", "inactive_file", "active_file", "kernel", "slab"):
        match = re.search(rf"^{key} (\d+)$", text, re.MULTILINE)
        if match:
            state[key] = int(match.group(1))
    match = re.search(r"^crible-p10 (\S+) / \S+", text, re.MULTILINE)
    if match:
        state["docker_mem"] = match.group(1)
    match = re.search(r"used = ([\d.]+)M", text)
    if match:
        state["host_swap_used_m"] = float(match.group(1))
    return state


def direct_latency(raw: Path) -> dict[str, Any]:
    document = macmini.read_json(raw / "direct-latency.json")
    result = {
        "method": document["method"],
        "queries": document["queries"],
        "unique_queries": document["unique_queries"],
        "cache_hits": document["cache_hits"],
        "warmup_ms": document["warmup_ms"],
        "failures": document["failures"],
        "all_queries": document["summary_ms"],
        "unique_queries_only": document["summary_ms_unique"],
        "samples": document["samples"],
    }
    load = raw / "direct-latency-load.json"
    if load.exists():
        companion = macmini.read_json(load)
        result["same_game_during_mes09_load"] = {
            "queries": companion["queries"],
            "unique_queries": companion["unique_queries"],
            "cache_hits": companion["cache_hits"],
            "unique_queries_only": companion["summary_ms_unique"],
        }
    return result


def write(document: Mapping[str, Any], path: str) -> None:
    macmini.write_json(path, document)
    print(f"written {path}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--raw", default="data/p10-macmini/pass-c/raw")
    parser.add_argument("--states", default="data/p10-macmini/pass-c/mes09")
    parser.add_argument("--out", default="docs/mesures")
    parser.add_argument("--commit", default=COMMIT)
    args = parser.parse_args(argv)

    raw = Path(args.raw)
    states = Path(args.states)
    out = Path(args.out)

    try:
        common = {
            "format_version": 1,
            "measure": "MESURER-SUR-MAC-MINI",
            "pass": "C",
            "date": DATE,
            "commit": args.commit,
            "machine": "Mac mini M1 (Macmini9,1), 8 cores, 16 GiB, macOS 26.5.2",
            "container": {
                "vm_bytes": VM_BYTES,
                "vm_cpus": VM_CPUS,
                "vm_page_bytes": VM_PAGE,
                "host_page_bytes": 16384,
                "memory_limit": None,
                "supabase": "stopped",
            },
            "percentile": "nearest rank on raw durations (ADR-0077, decision 4)",
        }

        scan = {
            **common,
            "measure_id": "MES-02",
            "question": "latency of the binary scan by pool size, in the container",
            "indexes": {
                "decisions": {"live_passages": 3_589_776, "segments": 4},
                "articles": {"live_passages": 2_476_715, "segments": 1},
            },
            "points": scan_points(raw),
            "absent": {
                "size": 7_740_000,
                "cause": (
                    "no index of the restored instance reaches 7,740,000 live passages: "
                    "`decisions` has 3,589,776 and `articles` 2,476,715; the point is not "
                    "replaced by a figure from another population"
                ),
            },
        }
        write(scan, f"{out}/MES-02-latence-scan-binaire.{DATE}.macmini.json")

        points, extras = rescore_points(raw)
        rescore = {
            **common,
            "measure_id": "MES-03",
            "question": "latency of the rescoring, warm and cold, in the container",
            "index": {"name": "articles", "live_passages": 2_476_715, "segments": 1},
            "points": points,
            **extras,
        }
        write(rescore, f"{out}/MES-03-latence-rescoring.{DATE}.macmini.json")

        encode = {
            **common,
            "measure_id": "MES-05",
            "question": "latency of one query encoding, in the container",
            "protocol": {
                "mode": "latency",
                "cache_capacity": 0,
                "warmup": 300,
                "count": 1000,
                "runs_per_precision": 2,
                "queries": 300,
            },
            "points": encode_points(raw),
        }
        write(encode, f"{out}/MES-05-latence-encodage.{DATE}.macmini.json")

        mes09 = {
            **common,
            "measure_id": "MES-09",
            "question": "real memory of the served instance, six indexes open",
            "scope": (
                "instance series only; the per-index series is not done, because it would "
                "move folders under `/data` (ADR-0129, rule 2 of decision 9)"
            ),
            "states": {
                "rest": parse_state(states / "rest.txt"),
                "after_load": parse_state(states / "after-load.txt"),
            },
            "limits": [
                "the load is the 97 text queries of the reference game, so the state after "
                "load includes the ONNX session; a vector-only pass and a session-only state "
                "were not separated",
                "the per-index series is absent by decision, not by failure",
            ],
        }
        write(mes09, f"{out}/MES-09-ram-reelle.{DATE}.macmini.json")

        latency = {
            **common,
            "measure_id": "lecture-directe-api",
            "question": (
                "client round-trip latency of the served instance through "
                "POST /v1/indexes/<index>/search, with a cold query cache; a repeated "
                "query text is a cache hit and is excluded from the unique series"
            ),
            "target_ms": 698.0,
            "reference": "MES-00 of 2026-10-20 (pgvector)",
            "role": "cross_check",
            "criterion_note": (
                "the J10 criterion is the local warm total of ADR-0141, decision 4; this "
                "client_round_trip reading is a cross-check, not directly comparable to "
                "in-process totals (ADR-0131, decision 7)"
            ),
            "protocol": {
                "route": "POST /v1/indexes/<index>/search",
                "latency_basis": "client_round_trip",
                "mode": "dense",
                "k": 10,
                "debug": True,
                "warmup": 1,
                "query_cache": "empty after a restart of the container",
                "duplicates": "a repeated text is a cache hit and is excluded from the unique series",
            },
            "result": direct_latency(raw),
        }
        write(latency, f"{out}/latence-mac-mini-{DATE}.direct.json")

        printed = set()
        for point in scan["points"]:
            key = (point["index"], point["size"])
            if key in printed or point["size"] not in (2_476_715, 3_589_776):
                continue
            printed.add(key)
            samples = point["samples"]
            print(
                f"scan {point['index']:9s} full {point['size']:>9} "
                f"p50 {samples['p50']:.3f} p95 {samples['p95']:.3f} ms"
            )
        for point in points:
            if point["candidates"] == 5000:
                samples = point["samples"]
                print(
                    f"rescore {point['state']:5s} N=5000 "
                    f"p50 {samples['p50']:.3f} p95 {samples['p95']:.3f} ms"
                )
        for point in encode["points"]:
            pooled = point["pooled"]
            print(
                f"encode {point['precision']:5s} "
                f"p50 {pooled['p50']:.3f} p95 {pooled['p95']:.3f} ms"
            )
        unique = latency["result"]["unique_queries_only"]["total"]
        print(
            f"direct unique n={latency['result']['unique_queries']} "
            f"p50 {unique['p50']:.3f} p95 {unique['p95']:.3f} p99 {unique['p99']:.3f} ms"
        )
    except macmini.MacMiniError as exc:
        print(f"p10-macmini pass C aggregate: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
