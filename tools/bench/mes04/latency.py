# SPDX-License-Identifier: Apache-2.0

"""Aggregate the MES-05 latency runs into the raw JSON of the report.

Each run is a file written by ``crates/crible-encode/examples/measure_queries``
in ``latency`` mode: per-call microseconds, the first-call duration (which
carries the session load) and the cache counters. Percentiles use the
nearest-rank convention of ``analysis.percentile``.

Run from ``tools/``::

    uv run python bench/mes04/latency.py \
        --run int8-run1=../data/mes04/latency-int8-run1.json --run ... \
        --out ../docs/mesures/MES-05-latence-encodage.2026-10-24.json
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import analysis


def read_run(path: Path) -> dict:
    payload = json.loads(path.read_text(encoding="utf-8"))
    calls = payload.get("calls_us")
    if not isinstance(calls, list) or not calls:
        raise ValueError(f"{path}: no 'calls_us' series")
    return payload


def summarise_run(payload: dict) -> dict:
    milliseconds = [float(value) / 1_000.0 for value in payload["calls_us"]]
    summary = analysis.summarise(milliseconds)
    return {
        "precision": payload["precision"],
        "queries": payload["queries"],
        "warmup": payload["warmup"],
        "count": payload["count"],
        "first_call_ms": float(payload["first_call_ms"]),
        "cache_capacity": payload.get("cache_capacity"),
        "cache_hits": payload.get("cache_hits"),
        "cache_misses": payload.get("cache_misses"),
        "calls_ms": summary,
        "calls_ms_series": [round(value, 3) for value in milliseconds],
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--run", action="append", required=True, metavar="LABEL=PATH")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--commit", default="unknown")
    parser.add_argument("--date", default="")
    parser.add_argument("--machine", default="")
    parser.add_argument(
        "--settings",
        type=Path,
        default=None,
        help="threads.py output to embed as the session-settings exploration",
    )
    arguments = parser.parse_args(argv)

    try:
        runs = {}
        for specification in arguments.run:
            label, separator, path = specification.partition("=")
            if not separator or not label or not path:
                raise ValueError(f"expected LABEL=PATH, got {specification!r}")
            runs[label] = summarise_run(read_run(Path(path)))
        payload = {
            "date": arguments.date,
            "commit": arguments.commit,
            "machine": arguments.machine,
            "task": "MESURER-PARITE-ENCODEUR",
            "cache": "disabled (capacity 0)",
            "runs": runs,
        }
        if arguments.settings is not None:
            payload["session_settings"] = json.loads(
                arguments.settings.read_text(encoding="utf-8")
            )
        arguments.out.parent.mkdir(parents=True, exist_ok=True)
        arguments.out.write_text(
            json.dumps(payload, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )
    except Exception as error:  # noqa: BLE001 - every failure becomes one line
        print(f"error: {' '.join(str(error).split())}", file=sys.stderr)
        return 1

    for label, run in runs.items():
        calls = run["calls_ms"]
        print(
            f"{label}: p50 {calls['p50']:.3f} ms, p95 {calls['p95']:.3f} ms, "
            f"p99 {calls['p99']:.3f} ms, first call {run['first_call_ms']:.1f} ms"
        )
    print(f"wrote {arguments.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
