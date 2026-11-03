#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Aggregates the passage B raw documents of the container campaign (ADR-0135).

Companion of `aggregate.py`, which covers passage A (MES-02, MES-03, MES-09).
This one writes the four artefacts of passage B:

    docs/mesures/MES-03-latence-rescoring.2026-11-03.conteneur-limite.json
    docs/mesures/MES-05-latence-encodage.2026-11-03.conteneur.json
    docs/mesures/MES-08-duree-attache-quotidienne.2026-11-03.conteneur.json
    docs/mesures/routes-en-conteneur-2026-11-03.json

Percentiles are recomputed from the per-call durations the harnesses wrote, by
**nearest rank** — the value at `ceil(ratio * n)`, never an interpolation
(ADR-0077, decision 4), the method of `aggregate.py` and of every published
report of this project. One exception, and only one: the aggregates over a
handful of timed rounds (the batch-size sweep of MES-08) take the **true
median**, the mean of the two central values when the count is even, through
`median()` — the nearest rank would take the lower of the two and quietly move
the published figure. The harnesses themselves interpolate between the two
nearest ranks, so their own summary is kept in each block under
`harness_summary` and the report quotes the nearest-rank figure. The script
**refuses** a series whose count, minimum, maximum or mean disagrees with the
harness: those four do not depend on the method. Every call is kept: nothing is
rounded, filtered or extrapolated.

    python3 tools/bench/mes-p8-conteneur/aggregate-b.py

Paths are read from the gitignored tree `data/mes-p8-conteneur/`.
"""

from __future__ import annotations

import json
import math
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[3]
RAW = ROOT / "data" / "mes-p8-conteneur" / "raw"
NATIVE = ROOT / "data" / "mes-p8-conteneur" / "native" / "raw"
MEASURES = ROOT / "docs" / "mesures"


class Refused(Exception):
    """A recomputed figure disagrees with the summary the harness wrote."""


def load(name: str, directory: pathlib.Path = RAW) -> dict:
    path = directory / f"{name}.json"
    if not path.is_file():
        raise Refused(f"{path} is missing")
    return json.loads(path.read_text(encoding="utf-8"))


def nearest_rank(ordered: list[float], percent: float) -> float:
    """Latency series: the value at `ceil(ratio * n)`, never an interpolation.

    The rule of ADR-0077, decision 4, and of every published latency table: it
    needs hundreds of calls to mean anything, which is why it is not used for a
    handful of timed rounds.
    """
    if not ordered:
        return float("nan")
    rank = max(1, math.ceil((percent / 100.0) * len(ordered)))
    return ordered[rank - 1]


def median(ordered: list[float]) -> float:
    """Round aggregates: the true median.

    An odd count takes the middle value; an even count takes the mean of the two
    central values, which is what a reader means by "median over four rounds".
    The nearest rank would take the lower of the two and quietly move the
    published figure.
    """
    ordered = sorted(ordered)
    count = len(ordered)
    if count == 0:
        return float("nan")
    middle = count // 2
    if count % 2:
        return ordered[middle]
    return (ordered[middle - 1] + ordered[middle]) / 2.0


def summary(values: list[float]) -> dict:
    ordered = sorted(values)
    return {
        "count": len(ordered),
        "min_us": ordered[0],
        "max_us": ordered[-1],
        "mean_us": sum(ordered) / len(ordered),
        "p50_us": nearest_rank(ordered, 50.0),
        "p95_us": nearest_rank(ordered, 95.0),
        "p99_us": nearest_rank(ordered, 99.0),
        "p50_ms": nearest_rank(ordered, 50.0) / 1000.0,
        "p95_ms": nearest_rank(ordered, 95.0) / 1000.0,
        "p99_ms": nearest_rank(ordered, 99.0) / 1000.0,
    }


def close(left: float, right: float) -> bool:
    return abs(left - right) <= 1e-6 * max(1.0, abs(left), abs(right))


def checked(values: list[float], harness: dict, where: str) -> dict:
    """Recomputes a series in nearest rank, and refuses a method-free disagreement.

    The four checked keys do not depend on how a percentile is taken; the
    harness summary is kept beside the recomputed one, so a reader sees both
    methods and never a figure that came from nowhere.
    """
    recomputed = summary(values)
    for key in ("min_us", "max_us", "mean_us"):
        if key in harness and not close(recomputed[key], harness[key]):
            raise Refused(
                f"{where}: {key} recomputed {recomputed[key]!r} against harness {harness[key]!r}"
            )
    if "count" in harness and recomputed["count"] != harness["count"]:
        raise Refused(f"{where}: count {recomputed['count']} against harness {harness['count']}")
    recomputed["harness_summary"] = harness or None
    return recomputed


# -- passage B, item 1: the rescoring engine at the shipped k = 100.


def mes03_limit() -> dict:
    points = []
    for n in (1000, 2000, 5000):
        warm = load(f"b-rescore-warm-only-n{n}")
        cold = load(f"b-rescore-cold-n{n}")
        warm_point = warm["points"][0]
        cold_point = cold["points"][0]
        passes = len(warm_point["durations_us"]) // warm["queries"] if warm.get("queries") else 0
        # The harness appends its durations in call order, one pass after the
        # other: splitting on the query count gives the per-pass series, which
        # is then summarised in nearest rank like every other percentile here.
        per_pass = []
        if passes:
            for index in range(passes):
                start = index * warm["queries"]
                per_pass.append(
                    summary(warm_point["durations_us"][start : start + warm["queries"]])
                )
        points.append(
            {
                "n": n,
                "warm": {
                    "passes": warm_point["passes"],
                    "combined": checked(warm_point["durations_us"], warm_point["summary"], f"warm N={n}"),
                    "per_pass": per_pass,
                    "harness_per_pass_p50_ms": [
                        entry["p50"] for entry in warm_point["pass_summaries"]
                    ],
                    "engine_limit": warm_point.get("plan"),
                },
                "cold": {
                    "passes": cold_point["passes"],
                    "combined": checked(cold_point["durations_us"], cold_point["summary"], f"cold N={n}"),
                    "resident_fraction": cold_point.get("resident_fraction"),
                },
                "queries": warm.get("queries"),
                "passes_in_durations": passes,
            }
        )
    return {
        "report": "docs/mesures/MES-03-latence-rescoring.md",
        "section": "Moteur à limite de consommation, en conteneur (passage B)",
        "engine_limit": 100,
        "warmup_document": load("b-warmup-rescore-limit100"),
        "native_same_harness_ms": {"1000": 10.192, "2000": None, "5000": 75.172},
        "native_source": "MES-03, sous-section « Diagnostic de l'écart du moteur (passage A-bis) »",
        "points": points,
    }


# -- passage B, item 2: query encoding, MES-05.


def mes05() -> dict:
    series = {}
    # The two precisions ran in one loop of four invocations, so the file names
    # carry the loop counter: int8 took 1 and 2, fp32 took 3 and 4.
    runs_of = {"int8": ("b-encode-int8-1", "b-encode-int8-2"), "fp32": ("b-encode-fp32-3", "b-encode-fp32-4")}
    for precision, names in runs_of.items():
        runs = []
        for name in names:
            document = load(name)
            runs.append(
                {
                    "first_call_ms": document["first_call_ms"],
                    "threads": document["threads"],
                    "summary": checked(document["calls_us"], document.get("summary", {}), name),
                    "count": len(document["calls_us"]),
                }
            )
        series[precision] = runs
    native = {}
    for precision in ("int8", "fp32"):
        document = load(f"native-encode-{precision}", NATIVE if (NATIVE / f"native-encode-{precision}.json").is_file() else RAW)
        native[precision] = {
            "first_call_ms": document["first_call_ms"],
            "threads": document["threads"],
            "summary": summary(document["calls_us"]),
        }
    return {
        "report": "docs/mesures/MES-05-latence-encodage.md",
        "queries": 300,
        "warmup": 300,
        "count": 1000,
        "container": series,
        "native_same_harness": native,
        "published_native_ms": {"int8": {"p50": 8.646, "p95": 22.490}, "fp32": {"p50": 20.978, "p95": 55.763}},
    }


# -- passage B, item 3: batch-size sweep of the acceptance re-encoding, MES-08.


def rounds(document: dict, padded: dict[int, int]) -> dict:
    """Per-size round aggregates, with the **true median** of the rounds.

    A sweep has a handful of timed rounds, not hundreds of calls: the median is
    the mean of the two central values when the count is even. The harness
    reports the higher of the two, which is not what a reader means by a median,
    so every `median_ms` of this artefact is recomputed here, from `values_ms`.
    """
    results = {}
    for size, result in document["results"].items():
        values = sorted(result["values_ms"])
        value = median(values)
        entry = {
            "values_ms": values,
            "rounds": len(values),
            "median_ms": value,
            "min_ms": values[0],
            "max_ms": values[-1],
            "median_rule": "median: mean of the two central values when the count is even",
        }
        if int(size) in padded:
            entry["tokens_padded"] = padded[int(size)]
            entry["microseconds_per_token"] = value / padded[int(size)] * 1000.0
        results[size] = entry
    return results


def mes08() -> dict:
    padded = {1: 54991, 4: 56448, 8: 58504}
    sweeps = {}
    for name in ("b-sweep-calibrate", "b-sweep-a", "b-sweep-b", "b-sweep-frame-after"):
        document = load(name)
        sweeps[name] = {
            "sizes": document["sizes"],
            "rounds": document["rounds"],
            "passages": document["passages"],
            "tokens_total": document["tokens_total"],
            "results": rounds(document, padded),
            "harness_median_ms": {
                size: result["median_ms"] for size, result in document["results"].items()
            },
        }
    longest = {}
    for name in ("b-longest",):
        document = load(name)
        longest[name] = {
            "tokens": document["tokens"],
            "copies": document["copies"],
            "rounds": document["rounds"],
            "results": rounds(document, {}),
            "harness_median_ms": {
                size: result["median_ms"] for size, result in document["results"].items()
            },
        }
    native = {}
    for name in ("native-sweep", "native-longest"):
        document = load(name)
        native[name] = {
            "results": rounds(document, padded),
            "harness_median_ms": {
                size: result["median_ms"] for size, result in document["results"].items()
            },
        }
    # The four interleaved rounds were run in two invocations of two rounds, so
    # the median the report quotes is taken over the pooled rounds, with the
    # same true median as every other round aggregate here.
    pooled = {}
    for name in ("b-sweep-a", "b-sweep-b"):
        for size, result in load(name)["results"].items():
            pooled.setdefault(int(size), []).extend(result["values_ms"])
    combined = {}
    for size in sorted(pooled):
        values = sorted(pooled[size])
        value = median(values)
        combined[str(size)] = {
            "rounds": len(values),
            "values_ms": values,
            "median_ms": value,
            "min_ms": values[0],
            "max_ms": values[-1],
            "tokens_padded": padded[size],
            "microseconds_per_token": value / padded[size] * 1000.0,
            "median_rule": "median: mean of the two central values when the count is even",
        }
    return {
        "report": "docs/mesures/MES-08-duree-attache-quotidienne.md",
        "container": {"sweeps": sweeps, "combined": combined, "longest": longest},
        "native_same_harness": native,
        "published_native_ms": {
            "B=1": 56314.7,
            "B=4": 59681.1,
            "B=8": 63633.4,
            "note": "campagne du 2026-10-27, `sweep` natif, 200 passages, 54 991 jetons",
        },
        # Same token base as the container rows: the padded token count of each
        # batch size, so the microsecond column of the report holds one base.
        "published_native_padded_us_per_token": {
            "B=1": 56314.7 / 54991 * 1000.0,
            "B=4": 59681.1 / 56448 * 1000.0,
            "B=8": 63633.4 / 58504 * 1000.0,
        },
        "sample": "sample-daily-articles-1000.json, 200 passages, 54 991 jetons",
    }


# -- passage B, item 4: the two read routes, no MES number.


def routes() -> dict:
    document = load("routes-conteneur-9seg")
    walk = load("b-walk-9seg")
    pool = load("b-readpool-9seg")
    served = {}
    for route in ("index", "document", "search_context"):
        entry = document["routes"][route]
        flatten = [value for pass_ in entry["passes"] for value in pass_]
        served[route] = {
            "path": entry["path"],
            "per_pass": [summary(pass_) for pass_ in entry["passes"]],
            "summary": checked(flatten, entry["summary"], route),
        }
    return {
        "report": "docs/mesures/routes-en-conteneur-2026-11-03.md",
        "index": document["index"],
        "passes": document["passes"],
        "calls_per_pass": document["calls_per_pass"],
        "client": document["client"],
        "routes": served,
        "index_body_bytes": document["routes"]["index"]["body_bytes"],
        "walk": {
            "files": walk["files"],
            "bytes": walk["bytes"],
            "summary": checked(walk["durations_us"], walk["summary"], "walk"),
            "rounds": walk["rounds"],
        },
        "read_pool": {
            "documents": pool["documents"],
            "rounds": pool["rounds"],
            "passages_read": pool["passages_read"],
            "open": checked(pool["open_us"], pool["open"], "open"),
            "fresh_read": checked(pool["fresh_read_us"], pool["fresh_read"], "fresh"),
            "reused_read": checked(pool["reused_read_us"], pool["reused_read"], "reused"),
        },
        "threshold_ms": 5.0,
    }


def main() -> int:
    artefacts = {
        "MES-03-latence-rescoring.2026-11-03.conteneur-limite.json": mes03_limit(),
        "MES-05-latence-encodage.2026-11-03.conteneur.json": mes05(),
        "MES-08-duree-attache-quotidienne.2026-11-03.conteneur.json": mes08(),
        "routes-en-conteneur-2026-11-03.json": routes(),
    }
    for name, document in artefacts.items():
        path = MEASURES / name
        path.write_text(json.dumps(document, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        print(f"wrote {path.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Refused as refusal:
        print(f"refused: {refusal}", file=sys.stderr)
        sys.exit(1)
