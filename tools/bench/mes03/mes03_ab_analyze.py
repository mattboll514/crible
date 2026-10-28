#!/usr/bin/env python3
"""Analysis of the `REDUIRE-COUT-DES-CLES` latency runs (MES-03, ADR-0079).

Reads the raw documents ``data/reduire-cles/raw/{ab,rep,conf}-r<R>-<variant>-<N>.json``
written by ``rescore_ab`` through ``ab_round.py`` and produces the versioned
artifact: the neutral A/B verdicts of ADR-0079 decision 2, the MES-03 replay of
decision 6, the confirmation of the delivered variant (ADR-0083, decision 3),
and the page state of both files for every series.

The verdict rule is the one ADR-0082 decision 3 writes for a latency A/B: a
variant is compared to ``base`` on the **combined p50 of its passes**; the
difference counts only beyond the amplitude of the variant's own pass-to-pass
dispersion, measured on the pairs of rounds (same pass index, opposite order).
Inside that amplitude the point is a **tie**; beyond it, a gain or a loss
depending on the sign.

    python3 tools/bench/mes03/mes03_ab_analyze.py \
        --raw data/reduire-cles/raw --out docs/mesures/MES-03-rejeu-reduire-cles.2026-10-28.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path

#: Variants of the neutral A/B (ADR-0079, decision 2).
NEUTRAL_VARIANTS = ("base", "2a", "2b", "2a-2b")
#: Variants of the MES-03 replay (ADR-0079, decision 6).
REPLAY_VARIANTS = ("base", "final")
#: Variants of the confirmation of the delivered variant (ADR-0083, decision 3).
CONFIRMATION_VARIANTS = ("base", "final", "b-only")
#: Values of N of each series.
NEUTRAL_NS = (1000, 5000)
REPLAY_NS = (1000, 2000, 5000)
CONFIRMATION_NS = (1000, 2000, 5000)


def nearest_rank(values: list[float], fraction: float) -> float:
    """Nearest-rank percentile (ADR-0077): ceil(fraction × n), one-based."""
    if not values:
        return math.nan
    ordered = sorted(values)
    rank = max(1, math.ceil(fraction * len(ordered)))
    return ordered[min(rank, len(ordered)) - 1]


def paired_dispersion(passes: list[list[float]]) -> tuple[float, float]:
    """Amplitude of a series' own pass-to-pass p50 dispersion (ADR-0082).

    ``passes`` holds one list of p50 values per round, in pass order. The
    amplitude is the largest absolute difference between two rounds at the same
    pass index, which is what "d'une passe à l'autre" compares when the rounds
    are alternated. The second value is the amplitude relative to the p50 of all
    passes, in percent.
    """
    if len(passes) < 2:
        return (0.0, 0.0)
    width = min(len(round) for round in passes)
    deltas = [
        abs(passes[first][index] - passes[second][index])
        for index in range(width)
        for first in range(len(passes))
        for second in range(first + 1, len(passes))
    ]
    amplitude = max(deltas) if deltas else 0.0
    centre = nearest_rank([value for round in passes for value in round], 0.50)
    return (amplitude, 100.0 * amplitude / centre if centre else 0.0)


def verdict(delta_percent: float, amplitude_percent: float) -> str:
    """``gain``, ``nul`` or ``perte``: the ADR-0082 decision 3 rule."""
    if abs(delta_percent) <= amplitude_percent:
        return "nul"
    return "gain" if delta_percent < 0 else "perte"


def point(documents: list[dict]) -> dict:
    """Summarises the invocations of one (variant, N) point.

    ``documents`` holds one ``rescore_ab`` document per round, in round order.
    Nothing is filtered: every pass of every round enters the combined
    percentiles, and the per-pass p50s are all reported.
    """
    per_round_p50 = [[entry["p50"] for entry in document["per_pass"]] for document in documents]
    durations = [value for document in documents for values in document["durations_ms"] for value in values]
    amplitude, amplitude_percent = paired_dispersion(per_round_p50)
    first = documents[0]
    return {
        "n": first["n"],
        "queries": first["queries"],
        "candidates_min": first["candidates_min"],
        "candidates_max": first["candidates_max"],
        "distinct_iids": first["distinct_iids"],
        "k": first["k"],
        "limit": first["limit"],
        "passes": [entry for document in documents for entry in document["per_pass"]],
        "p50": nearest_rank(durations, 0.50),
        "p95": nearest_rank(durations, 0.95),
        "p99": nearest_rank(durations, 0.99),
        "min": min(durations),
        "max": max(durations),
        "n_samples": len(durations),
        "dispersion_ms": amplitude,
        "dispersion_percent": amplitude_percent,
    }


def gain(reference: dict, candidate: dict, key: str) -> float:
    """Relative change of ``candidate`` against ``reference``, in percent."""
    return 100.0 * (candidate[key] / reference[key] - 1.0)


def replay_verdicts(entry: dict) -> dict:
    """Applies the ADR-0082 noise rule to one replay point, in place.

    The replay is a latency A/B like the others: a `final` point whose delta
    stays inside its own pass-to-pass dispersion is a tie, however large the
    delta looks. Its first pass is cold and widens that dispersion a lot.

    The rule is written for the p50 only; the p95 keeps its relative change and
    carries no verdict, because ADR-0082 gives no noise rule for it.
    """
    entry["verdict"] = verdict(entry["gain_percent"], entry["dispersion_percent"])
    return entry


def comparison(reference: dict, candidate: dict) -> dict:
    """One ``candidate`` against one ``reference``, with the ADR-0082 verdict.

    The verdict applies the noise rule to the **candidate's** own dispersion, as
    the neutral A/B and the replay do.
    """
    return {
        "p50_reference": reference["p50"],
        "p50": candidate["p50"],
        "delta_percent": gain(reference, candidate, "p50"),
        "p95_reference": reference["p95"],
        "p95": candidate["p95"],
        "delta_percent_p95": gain(reference, candidate, "p95"),
        "dispersion_percent": candidate["dispersion_percent"],
        "verdict": verdict(
            gain(reference, candidate, "p50"), candidate["dispersion_percent"]
        ),
    }


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def residency(raw: Path, tag: str, round_number: int, variant: str, n: int) -> dict:
    """Resident fractions of both files around one invocation."""
    path = raw / f"{tag}-r{round_number}-{variant}-{n}.residency.json"
    invocation = json.loads(path.read_text(encoding="utf-8"))
    return {
        "vectors_before": invocation["residency_before"]["vectors"]["resident_fraction"],
        "vectors_after": invocation["residency_after"]["vectors"]["resident_fraction"],
        "records_before": invocation["residency_before"]["records"]["resident_fraction"],
        "records_after": invocation["residency_after"]["records"]["resident_fraction"],
    }


def series(raw: Path, tag: str, variants: tuple[str, ...], ns: tuple[int, ...]) -> dict:
    """Reads the rounds of a series and summarises every point."""
    result = {}
    for variant in variants:
        points = {}
        for n in ns:
            documents = [
                json.loads((raw / f"{tag}-r{round_number}-{variant}-{n}.json").read_text(encoding="utf-8"))
                for round_number in (1, 2)
            ]
            summary = point(documents)
            summary["residency"] = [
                residency(raw, tag, round_number, variant, n) for round_number in (1, 2)
            ]
            points[str(n)] = summary
        result[variant] = points
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--raw", default="data/reduire-cles/raw")
    parser.add_argument("--out", required=True)
    parser.add_argument("--conditions", default="data/reduire-cles/raw")
    parser.add_argument("--bin-dir", default="data/reduire-cles/bin")
    arguments = parser.parse_args()
    raw = Path(arguments.raw)

    neutral = series(raw, "ab", NEUTRAL_VARIANTS, NEUTRAL_NS)
    for n in NEUTRAL_NS:
        reference = neutral["base"][str(n)]
        for variant in NEUTRAL_VARIANTS:
            if variant == "base":
                continue
            entry = neutral[variant][str(n)]
            entry["delta_percent"] = gain(reference, entry, "p50")
            entry["verdict"] = verdict(entry["delta_percent"], entry["dispersion_percent"])

    replay = series(raw, "rep", REPLAY_VARIANTS, REPLAY_NS)
    for n in REPLAY_NS:
        entry = replay["final"][str(n)]
        entry["gain_percent"] = gain(replay["base"][str(n)], entry, "p50")
        entry["gain_percent_p95"] = gain(replay["base"][str(n)], entry, "p95")
        replay_verdicts(entry)

    controls = {}
    for name in ("check-base-mes02-k100", "check-final-mes02-k100", "check-b-only-mes02-k100"):
        path = raw / f"{name}.json"
        controls[name] = {"document": str(path), "sha256": sha256(path)}
    for name in ("mes01-final", "mes01-base", "mes01-b-only"):
        path = raw / f"{name}.json"
        summary = json.loads(path.read_text(encoding="utf-8"))
        controls[name] = {
            "document": str(path),
            "points": len(summary["points"]),
            "differing": sum(len(entry["differing"]) for entry in summary["points"]),
        }

    confirmation = series(raw, "conf", CONFIRMATION_VARIANTS, CONFIRMATION_NS)
    for n in CONFIRMATION_NS:
        delivered = confirmation["b-only"][str(n)]
        delivered["against"] = {
            "base": comparison(confirmation["base"][str(n)], delivered),
            "final": comparison(confirmation["final"][str(n)], delivered),
        }
        confirmation["final"][str(n)]["against"] = {
            "base": comparison(confirmation["base"][str(n)], confirmation["final"][str(n)])
        }

    artifact = {
        "kind": "mes03-rejeu-reduire-cout-des-cles",
        "date": "2026-10-28",
        "task": "REDUIRE-COUT-DES-CLES",
        "adr": ["ADR-0079", "ADR-0077", "ADR-0082", "ADR-0083"],
        "neutral": neutral,
        "replay": replay,
        "confirmation": confirmation,
        "controls": controls,
        "binaries": {
            variant: {
                "dense_latency": sha256(Path(arguments.bin_dir) / variant / "dense_latency"),
                "rescore_ab": sha256(Path(arguments.bin_dir) / variant / "rescore_ab"),
            }
            for variant in ("base", "2a", "2b", "2a-2b", "final", "b-only")
        },
    }
    Path(arguments.out).write_text(
        json.dumps(artifact, ensure_ascii=False, indent=1) + "\n", encoding="utf-8"
    )
    print(f"wrote {arguments.out}")
    for n in NEUTRAL_NS:
        for variant in NEUTRAL_VARIANTS:
            entry = neutral[variant][str(n)]
            print(
                f"neutral N = {n:5d} {variant:6s} p50 {entry['p50']:8.1f} ms "
                f"dispersion {entry['dispersion_percent']:5.1f}% "
                f"delta {entry.get('delta_percent', 0):+6.1f}% verdict {entry.get('verdict', 'reference')}"
            )
    for n in REPLAY_NS:
        entry = replay["final"][str(n)]
        print(
            f"replay N = {n:5d} base p50 {replay['base'][str(n)]['p50']:8.1f} "
            f"final p50 {entry['p50']:8.1f} gain {entry['gain_percent']:+.1f}% "
            f"({entry['verdict']}) p95 gain {entry['gain_percent_p95']:+.1f}%"
        )
    for n in CONFIRMATION_NS:
        entry = confirmation["b-only"][str(n)]
        parts = []
        for reference_name in ("base", "final"):
            against = entry["against"][reference_name]
            parts.append(
                f"vs {reference_name} {against['delta_percent']:+6.1f}% "
                f"(p95 {against['delta_percent_p95']:+.1f}%, {against['verdict']})"
            )
        print(
            f"confirmation N = {n:5d} b-only p50 {entry['p50']:8.1f} ms "
            f"dispersion {entry['dispersion_percent']:5.1f}% " + " | ".join(parts)
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
