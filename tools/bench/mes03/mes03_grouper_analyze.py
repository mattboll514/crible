#!/usr/bin/env python3
"""Builds the versioned artifact of the GROUPER-LECTURES-VECTEURS campaign.

The campaign (ADR-0080) measures four variants of the vector read at rescoring
(``base``, ``grouped``, ``madv``, ``grouped-madv``) on five series: hot, cold by
cache displacement, 10-minute idle, contention with the binary scan, and RSS.
This script reads only the raw documents written by ``ab_round.py`` and
``rescore_ab`` under ``data/grouper-vecteurs/raw/`` and reduces them to the
tables the report quotes: p50, p95 and p99 per point, the gap against ``base``
round by round, and the verdict of the noise rule of ADR-0082 decision 3.

No measurement is re-run here, and no value is dropped: a point that exists in
one round only keeps its single gap, and the amplitude of the gaps is then
unknown (``null``), which the report has to say.

    python3 tools/bench/mes03/mes03_grouper_analyze.py \
        --raw data/grouper-vecteurs/raw \
        --out docs/mesures/MES-03-grouper-lectures-vecteurs.2026-10-28.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

VARIANTS = ("base", "grouped", "madv", "grouped-madv")


def document_name(tag: str, round_index: int, variant: str, n: int) -> str:
    """Returns the raw document name of one point of a series."""
    return f"{tag}-r{round_index}-{variant}-{n}.json"


def load_document(raw: Path, tag: str, round_index: int, variant: str, n: int) -> dict | None:
    """Loads one raw document, or ``None`` when it is missing.

    A missing document is reported as missing by the caller; it is never
    replaced by a neighbour and never guessed.
    """
    path = raw / document_name(tag, round_index, variant, n)
    if not path.is_file():
        return None
    return json.loads(path.read_text(encoding="utf-8"))


def summarize(document: dict) -> dict:
    """Reduces a ``rescore_ab`` document to what the report quotes."""
    summary = document["summary"]
    reduced = {
        "p50": summary["p50"],
        "p95": summary["p95"],
        "p99": summary["p99"],
        "samples": summary["n"],
        "per_pass": [
            {"p50": point["p50"], "p95": point["p95"], "p99": point["p99"]}
            for point in document["per_pass"]
        ],
    }
    residency = document.get("resident_fraction")
    if residency:
        reduced["resident_mean"] = residency["mean"]
        reduced["resident_first"] = residency["first"]
    for key in ("displaced_bytes", "idle_seconds", "candidates_digest"):
        if document.get(key) is not None:
            reduced[key] = document[key]
    if document.get("distinct_iids") is not None:
        reduced["distinct_iids"] = document["distinct_iids"]
        reduced["iid_inversions"] = document["iid_inversions"]
    return reduced


def gap(base_p50: float, variant_p50: float) -> float:
    """Returns the relative gap of a variant against ``base``, in percent.

    Negative means the variant is faster. The sign convention is the one of the
    report: ``-80 %`` is a variant that costs a fifth of ``base``.
    """
    return 100.0 * (variant_p50 - base_p50) / base_p50


def noise_verdict(gaps: list[float]) -> dict:
    """Applies the noise rule of ADR-0082 decision 3 to one point.

    The rule compares the p50 point by point: a point is *lost* only if the
    variant is slower and the gap exceeds the amplitude of its own gaps from one
    round to the next. With two rounds the amplitude is ``max - min``; with a
    single round it is unknown, and the verdict says so instead of pretending.
    """
    amplitude = max(gaps) - min(gaps) if len(gaps) > 1 else None
    mean = sum(gaps) / len(gaps)
    if amplitude is None:
        return {"gaps": gaps, "mean": mean, "amplitude": None, "verdict": "single_round"}
    if abs(mean) <= amplitude:
        return {"gaps": gaps, "mean": mean, "amplitude": amplitude, "verdict": "within_noise"}
    return {
        "gaps": gaps,
        "mean": mean,
        "amplitude": amplitude,
        "verdict": "slower" if mean > 0 else "faster",
    }


def point(
    raw: Path, tag: str, rounds: tuple[int, ...], n: int, variants: tuple[str, ...]
) -> dict:
    """Builds one point of a series: every variant, every round, and its gaps."""
    measured = {
        variant: {
            f"r{round_index}": summarize(document)
            for round_index in rounds
            if (document := load_document(raw, tag, round_index, variant, n)) is not None
        }
        for variant in variants
    }
    base_documents = measured.get("base", {})
    compared = {}
    for variant in variants:
        if variant == "base":
            continue
        gaps = [
            gap(
                base_documents[key]["p50"],
                document["p50"],
            )
            for key, document in measured.get(variant, {}).items()
            if key in base_documents
        ]
        compared[variant] = {
            "gaps": noise_verdict(gaps),
            "p95_gaps": [
                gap(base_documents[key]["p95"], document["p95"])
                for key, document in measured.get(variant, {}).items()
                if key in base_documents
            ],
        }
    return {"n": n, "measured": measured, "against_base": compared}


def series(
    raw: Path,
    tag: str,
    rounds: tuple[int, ...],
    ns: tuple[int, ...],
    variants: tuple[str, ...],
) -> dict:
    """Builds a whole series, point by point."""
    missing = [
        document_name(tag, round_index, variant, n)
        for round_index in rounds
        for variant in variants
        for n in ns
        if not (raw / document_name(tag, round_index, variant, n)).is_file()
    ]
    return {
        "tag": tag,
        "rounds": list(rounds),
        "ns": list(ns),
        "variants": list(variants),
        "missing": missing,
        "points": [
            point(raw, tag, rounds, n, variants) for n in ns
        ],
    }


def contend_series(raw: Path, rounds: tuple[int, ...], ns: tuple[int, ...]) -> dict:
    """Reduces the contention series: scan alone, scan under load, rescoring."""
    points = []
    for n in ns:
        documents = {
            f"r{round_index}-{variant}": (raw / f"contend-r{round_index}-{variant}-{n}.json")
            for round_index in rounds
            for variant in ("base", "grouped")
        }
        reduced = {}
        for key, path in documents.items():
            if not path.is_file():
                continue
            document = json.loads(path.read_text(encoding="utf-8"))
            reduced[key] = {
                "queries": document["queries"],
                "scan_alone": document["scan_alone"],
                "scan_concurrent": document["scan_concurrent"],
                "rescore_alone": document["rescore_alone"],
                "rescore_concurrent": document["rescore_concurrent"],
                "scan_matches_candidates": document["scan_matches_candidates"],
            }
        points.append({"n": n, "measured": reduced})
    return {"rounds": list(rounds), "ns": list(ns), "points": points}


def rss_series(raw: Path) -> dict:
    """Reduces the three RSS states of both variants, and the sampled runs."""
    states = {}
    for variant in ("base", "grouped"):
        path = raw / f"rss-{variant}.json"
        if not path.is_file():
            states[variant] = None
            continue
        document = json.loads(path.read_text(encoding="utf-8"))
        states[variant] = {
            "n": document["n"],
            "queries": document["queries"],
            "live_passages": document["live_passages"],
            "page_size": document["page_size"],
            "codes_bytes": document["codes_bytes"],
            "filter_memory_bytes": document["filter_memory_bytes"],
            "states": {
                label: {
                    "ps_rss_bytes": state["ps_rss_bytes"],
                    "mapped_file_bytes": state["mapped_file_bytes"],
                    "anonymous_bytes": state["ps_rss_bytes"] - state["mapped_file_bytes"],
                    "swapped_bytes": state["swapped_bytes"],
                    "physical_footprint_bytes": state["physical_footprint_bytes"],
                }
                for label, state in document["states"].items()
            },
        }
    sampled = {}
    for variant in ("base", "grouped"):
        path = raw / f"rss-run-{variant}-5000.json"
        if not path.is_file():
            continue
        document = json.loads(path.read_text(encoding="utf-8"))
        sampled[variant] = {
            "rss": document["rss"],
            "p50": document["summary"]["p50"],
            "p95": document["summary"]["p95"],
        }
    return {"states": states, "sampled_at_5000": sampled}


def digest(path: Path) -> str:
    """Returns the SHA-256 of a file, or an empty string when it is absent."""
    if not path.is_file():
        return ""
    return hashlib.sha256(path.read_bytes()).hexdigest()


def controls(raw: Path) -> dict:
    """Collects the controls: bit-exact rescoring and the MES-01 top-10 replay."""
    checks = {
        variant: raw / f"check-{variant}-mes02-k100.json" for variant in ("base", "grouped")
    }
    mes01 = {
        variant: raw / f"mes01-{variant}.json" for variant in ("base", "grouped")
    }
    top10 = {}
    for variant, path in mes01.items():
        if not path.is_file():
            top10[variant] = None
            continue
        document = json.loads(path.read_text(encoding="utf-8"))
        differing = sum(len(point["differing"]) for point in document["points"])
        top10[variant] = {
            "points": len(document["points"]),
            "differing": differing,
            "plans": sorted(
                {plan for point in document["points"] for plan in point["plans"]}
            ),
        }
    return {
        "rescore_check_sha256": {
            variant: digest(path) for variant, path in checks.items()
        },
        "rescore_check_identical": len(set(digest(path) for path in checks.values())) == 1
        and all(path.is_file() for path in checks.values()),
        "mes01_top10": top10,
    }


def binaries(directory: Path) -> dict:
    """Records the SHA-256 of the measured binaries."""
    return {
        variant: {
            name: digest(directory / variant / name)
            for name in ("rescore_ab", "rescore_check", "fidelity", "dense_latency")
        }
        for variant in VARIANTS
    }


def build(raw: Path, bin_dir: Path) -> dict:
    """Builds the whole artifact."""
    return {
        "kind": "mes03-grouper-lectures-vecteurs",
        "date": "2026-10-28",
        "campaign": "GROUPER-LECTURES-VECTEURS (ADR-0080)",
        "binaries_sha256": binaries(bin_dir),
        "screening": {
            "hot": series(
                raw, "screen", (1, 2), (1000, 5000), VARIANTS
            ),
            "cold_by_displacement": series(
                raw, "screencold", (1, 2), (1000, 5000), VARIANTS
            ),
        },
        "full": {
            "hot": series(raw, "full", (1, 2), (1000, 2000, 5000), ("base", "grouped")),
            "cold_by_displacement": series(
                raw, "fullcold", (1, 2), (1000, 2000, 5000), ("base", "grouped")
            ),
        },
        "idle": series(raw, "idle", (1, 2), (1000, 5000), ("base", "grouped")),
        "contend": contend_series(raw, (1, 2), (1000, 5000)),
        "rss": rss_series(raw),
        "controls": controls(raw),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--raw", default="data/grouper-vecteurs/raw")
    parser.add_argument("--bin-dir", default="data/grouper-vecteurs/bin")
    parser.add_argument(
        "--out", default="docs/mesures/MES-03-grouper-lectures-vecteurs.2026-10-28.json"
    )
    arguments = parser.parse_args()

    artifact = build(Path(arguments.raw), Path(arguments.bin_dir))
    out = Path(arguments.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(
        json.dumps(artifact, ensure_ascii=False, indent=1, sort_keys=False) + "\n",
        encoding="utf-8",
    )
    print(f"mes03_grouper_analyze: wrote {out} ({out.stat().st_size} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
