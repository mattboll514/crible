# SPDX-License-Identifier: Apache-2.0

"""Reduces the raw JSON of MES-16 into the summary committed next to the report.

`mes16_charge.py` writes one raw file per campaign, with every sample. The
report of `docs/mesures/` needs the statistics, not the samples: this script
keeps the phase statistics, the slices of the warm-up and of the rest phase, the
writes and the verdict, and drops the samples. The raw files stay under `data/`
(gitignored) and the report cites their path.

The percentile (ADR-0077, the nearest rank) and the slicing of a phase come from
`mes16_charge.py`: the summary never recomputes them its own way.

Usage, from the repository root (the harness needs the environment of `tools/`)::

    uv run --project tools --no-sync python tools/bench/mes16/resume_mes16.py \
        --raw data/mes16/campaign-1 data/mes16/campaign-2 data/mes16/campaign-3 \
        --out docs/mesures/MES-16-charge-concurrente.2026-10-30.json
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any, Mapping, Sequence

#: The harness lives next to this script; importing it keeps one percentile and
#: one slicing for the report and for the summary.
sys.path.insert(0, str(Path(__file__).resolve().parent))

import mes16_charge as harness  # noqa: E402  (path set above)

#: Phases of a campaign, in their order; the warm-up is the one stored apart.
PHASES = harness.PHASES


def summary(values: Sequence[float]) -> dict[str, Any]:
    """The usual statistics of one series, the maximum included."""
    ordered = sorted(values)
    return {
        "n": len(ordered),
        "min": round(ordered[0], 3),
        "p50": round(harness.nearest_rank(ordered, 0.50), 3),
        "p95": round(harness.nearest_rank(ordered, 0.95), 3),
        "p99": round(harness.nearest_rank(ordered, 0.99), 3),
        "max": round(ordered[-1], 3),
        "mean": round(sum(ordered) / len(ordered), 3),
    }


def _samples(entries: Sequence[Mapping[str, Any]], phase: str) -> list[harness.Sample]:
    """Rebuilds the samples of one phase, so the harness can slice them."""
    return [
        harness.Sample(
            phase=phase,
            wall_ms=entry["wall_ms"],
            took_ms=entry["took_ms"],
            status=entry["status"],
            mode=entry["mode"],
            pool_size=entry["pool_size"],
            at_s=entry["at_s"],
        )
        for entry in entries
    ]


def slices(entries: Sequence[Mapping[str, Any]], phase: str) -> list[dict[str, Any]]:
    """Statistics of the phase per slice of `WARMUP_SLICE_S` seconds."""
    result = []
    for bucket in harness.time_buckets(_samples(entries, phase)):
        walls = [sample.wall_ms for sample in bucket["samples"]]
        result.append(
            {
                "from_s": bucket["from_s"],
                "to_s": bucket["to_s"],
                "n": len(walls),
                "p50_ms": round(harness.nearest_rank(walls, 0.50), 3),
                "p95_ms": round(harness.nearest_rank(walls, 0.95), 3),
                "p99_ms": round(harness.nearest_rank(walls, 0.99), 3),
                "max_ms": round(max(walls), 3),
            }
        )
    return result


def phase_entry(entries: Sequence[Mapping[str, Any]]) -> dict[str, Any]:
    """Statistics of one phase, without its samples."""
    if not entries:
        return {"n": 0}
    walls = [entry["wall_ms"] for entry in entries]
    tooks = [entry["took_ms"] for entry in entries if entry["took_ms"] is not None]
    pools = [entry["pool_size"] for entry in entries if entry["pool_size"] is not None]
    gaps = [
        entry["wall_ms"] - entry["took_ms"]
        for entry in entries
        if entry["took_ms"] is not None
    ]
    statuses: dict[str, int] = {}
    modes: dict[str, int] = {}
    for entry in entries:
        status = "transport" if entry["status"] is None else str(entry["status"])
        statuses[status] = statuses.get(status, 0) + 1
        mode = entry["mode"] or "unknown"
        modes[mode] = modes.get(mode, 0) + 1
    return {
        "n": len(entries),
        "wall_ms": summary(walls),
        "took_ms": summary(tooks) if tooks else None,
        "wall_minus_took_ms": summary(gaps) if gaps else None,
        "statuses": dict(sorted(statuses.items())),
        "modes": dict(sorted(modes.items())),
        "pool_size": {"min": min(pools), "max": max(pools)} if pools else None,
        "from_s": round(min(entry["at_s"] for entry in entries), 3),
        "to_s": round(max(entry["at_s"] for entry in entries), 3),
    }


def reduce_campaign(document: Mapping[str, Any]) -> dict[str, Any]:
    """Statistics of one campaign, from its raw document."""
    phases: dict[str, Any] = {}
    for phase in PHASES:
        samples = (
            document["warmup"]["samples"]
            if phase == harness.WARMUP_PHASE
            else document["phases"][phase]["samples"]
        )
        phases[phase] = phase_entry(samples)
        phases[phase]["slices"] = slices(samples, phase)
    criterion = document["criterion"]
    limit = criterion["reference"]["limit_ms"]
    over = []
    for phase in harness.WRITE_PHASES:
        over.extend(
            {
                "phase": phase,
                "at_s": sample["at_s"],
                "wall_ms": sample["wall_ms"],
                "took_ms": sample["took_ms"],
                "mode": sample["mode"],
                "pool_size": sample["pool_size"],
            }
            for sample in document["phases"][phase]["samples"]
            if sample["wall_ms"] >= limit
        )
    return {
        "campaign": document["campaign"],
        "date": document["date"],
        "duration_s": round(document["duration_s"], 3),
        "machine": document["machine"],
        "protocol": document["protocol"],
        "base": document["base"],
        "phases": phases,
        "writes": document["writes"],
        "criterion": {
            "factor": criterion["reference"]["limit_ms"] / criterion["reference"]["p99_ms"],
            "reference_p99_ms": round(criterion["reference"]["p99_ms"], 3),
            "limit_ms": round(limit, 3),
            "per_request": criterion["per_request"],
            "searches": criterion["searches"],
            "writes": criterion["writes"],
            "over_limit_samples": over,
            "passed": criterion["passed"],
        },
    }


def main() -> int:
    """Reads the raw campaigns, writes the summary."""
    parser = argparse.ArgumentParser(description="MES-16: raw campaigns to summary")
    parser.add_argument("--raw", nargs="+", required=True, help="directories holding campaign-1.json")
    parser.add_argument("--out", required=True, help="summary JSON, next to the report")
    arguments = parser.parse_args()
    campaigns = []
    for number, directory in enumerate(arguments.raw, start=1):
        paths = sorted(Path(directory).glob("campaign-*.json"))
        if len(paths) != 1:
            raise SystemExit(f"{directory}: expected one raw campaign, found {len(paths)}")
        entry = reduce_campaign(json.loads(paths[0].read_text(encoding="utf-8")))
        # Each run writes `campaign-1.json`, its own numbering; the summary
        # numbers the campaigns by the order of `--raw`, which is the order the
        # operator ran them in.
        entry["campaign"] = number
        entry["raw_file"] = str(paths[0])
        campaigns.append(entry)
    Path(arguments.out).write_text(
        json.dumps({"campaigns": campaigns}, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return 0 if all(entry["criterion"]["passed"] for entry in campaigns) else 1


if __name__ == "__main__":
    raise SystemExit(main())
