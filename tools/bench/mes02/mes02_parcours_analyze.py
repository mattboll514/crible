# SPDX-License-Identifier: Apache-2.0
"""Aggregation of the alternating campaign of MES-02 (ADR-0071, ADR-0077).

The campaign it aggregates is the one `mes02_parcours_analyze` is written for:
both traversals of the binary scan, forced on the same pool and the same query
vectors, run in alternation (dense, sparse, dense, sparse) after a separate
warm-up pass of each. It fills the density band ADR-0071's 50 % rule separates,
which MES-02 only touched at its two ends.

It reads the raw documents written by the harness
(`crates/crible/examples/dense_latency.rs`, mode `cross`, under
`data/mes02/raw/`), recomputes every percentile from the per-query durations,
checks them against the summaries the harness recorded, and writes one versioned
artefact under `docs/mesures/`. It prints the Markdown tables of the report.

Percentiles are the nearest-rank definition of ADR-0077, imported from
`mes02_analyze` so the two campaigns of MES-02 cannot drift apart.

Run from the repository root::

    (cd tools && uv run --no-sync python bench/mes02/mes02_parcours_analyze.py \\
        --commit "$(git rev-parse HEAD)" --date 2026-10-27 \\
        --machine "MacBook Air M3, 8 cores, 24 GB, macOS 15.7.7, native")
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from mes02_analyze import (
    DEFAULT_MEASURES,
    DEFAULT_RAW,
    check_summary,
    nearest_rank,
    relative,
)

#: One raw document per value of N; the recipe runs each of them once.
CAMPAIGNS = {
    "cross-n5000": "cross-n5000.json",
    "cross-n1000": "cross-n1000.json",
}

#: The agreement control that goes with each campaign: both traversals must
#: return the same candidates on every pool the campaign measured.
CONTROLS = {
    "cross-n5000": "agree-n5000.json",
    "cross-n1000": "agree-n1000.json",
}

#: Name of the versioned artefact, without the date.
ARTEFACT = "MES-02-parcours-dense-clairseme"


def percent(part: float, whole: float) -> float:
    """Relative difference of `part` against `whole`, in percent.

    A negative value means `part` is faster than `whole`; the report reads
    "clairsemé −19 %" as "the sparse traversal took 19 % less time".
    """
    if whole == 0:
        raise ValueError("relative difference against zero")
    return (part - whole) / whole * 100.0


def spread(values: list[float]) -> float:
    """Spread of a series as `max / min − 1`, in percent.

    This is the dispersion MES-02 used between two runs of the same point
    (46,469 / 40,037 = 1,161, so 16,1 %). Applying it to the pairs of the same
    point gives the dispersion of the measurement itself, which is the
    quantity an inter-traversal gap must exceed to be read as a difference.
    """
    if not values or min(values) <= 0:
        raise ValueError("spread of an empty or non-positive series")
    return (max(values) / min(values) - 1.0) * 100.0


def range_points(values: list[float]) -> float:
    """Amplitude of a series of already relative values, in percentage points.

    A relative gap between two traversals can be negative, so `max / min` says
    nothing about it; its amplitude is a plain difference, read as "the gap went
    from −25 % to −14 %, a range of 11 points".
    """
    if not values:
        raise ValueError("range of an empty series")
    return max(values) - min(values)


def point_row(point: dict, campaign: str) -> dict:
    """One pool of the campaign, recomputed and checked against the harness."""
    label = f"{campaign} {point['label']} n={point['n']}"
    dense = point["dense"]
    sparse = point["sparse"]
    dense_summary = check_summary(dense["durations_us"], dense["summary"], f"{label} dense")
    sparse_summary = check_summary(sparse["durations_us"], sparse["summary"], f"{label} sparse")
    pairs = []
    for pair in point["pairs"]:
        dense_pair = check_summary(
            pair["dense_us"], pair["dense"], f"{label} pair {pair['pair']} dense"
        )
        sparse_pair = check_summary(
            pair["sparse_us"], pair["sparse"], f"{label} pair {pair['pair']} sparse"
        )
        if nearest_rank(pair["dense_us"], 0.50) != dense_pair["p50"]:
            raise ValueError(f"{label}: per-pair p50 disagrees with its own summary")
        pairs.append(
            {
                "pair": pair["pair"],
                "dense_us": dense_pair,
                "sparse_us": sparse_pair,
                "sparse_minus_dense_percent": percent(sparse_pair["p50"], dense_pair["p50"]),
            }
        )
    warmup = {
        name: (
            check_summary(
                point["warmup"][name]["durations_us"],
                point["warmup"][name]["summary"],
                f"{label} warm-up {name}",
            )
            if point.get("warmup") and point["warmup"].get(name)
            else None
        )
        for name in ("dense", "sparse")
    }
    return {
        "campaign": campaign,
        "label": point["label"],
        "kind": point["kind"],
        "requested_span": point.get("requested_span"),
        "size": point["size"],
        "span": point["span"],
        "density": point["density"],
        "n": point["n"],
        "first_iid": point["first_iid"],
        "last_iid": point["last_iid"],
        "queries": point["queries"],
        "pairs_count": point["pairs_count"],
        "plan": point["plan"],
        "traversal_automatic": point["traversal_automatic"],
        "dense": {**dense_summary, "durations_us": dense["durations_us"]},
        "sparse": {**sparse_summary, "durations_us": sparse["durations_us"]},
        "warmup": warmup,
        "pairs": pairs,
        "sparse_minus_dense_percent": percent(sparse_summary["p50"], dense_summary["p50"]),
        "p50_spread": {
            "dense_percent": spread([pair["dense_us"]["p50"] for pair in pairs]),
            "sparse_percent": spread([pair["sparse_us"]["p50"] for pair in pairs]),
            "gap_points": range_points(
                [pair["sparse_minus_dense_percent"] for pair in pairs]
            ),
        },
    }


def rows(raw: Path) -> list[dict]:
    """Every point of the alternating campaign, campaign by campaign."""
    rows = []
    for campaign, name in CAMPAIGNS.items():
        path = raw / name
        if not path.exists():
            continue
        for point in json.loads(path.read_text(encoding="utf-8"))["points"]:
            rows.append(point_row(point, campaign))
    if not rows:
        raise FileNotFoundError(f"no cross document under {raw}: run the campaign first")
    rows.sort(key=lambda row: (row["campaign"], row["kind"], row["span"], row["density"]))
    return rows


def control_rows(raw: Path) -> dict | None:
    """The agreement control: both traversals returned the same candidates.

    The claim is cheap to state and fatal to get wrong — a replica that reads a
    different passage set would change the work being timed without changing
    anything visible. The aggregator therefore **refuses** a campaign whose
    control says the two traversals disagree, rather than printing the figure
    next to a warning.
    """
    pools = 0
    queries_checked = 0
    points: list[dict] = []
    found = False
    for name in CONTROLS.values():
        path = raw / name
        if not path.exists():
            continue
        found = True
        document = json.loads(path.read_text(encoding="utf-8"))
        for point in document["points"]:
            pools += 1
            queries_checked += point["queries_checked"]
            points.append(
                {
                    "label": point["label"],
                    "size": point["size"],
                    "span": point["span"],
                    "density": point["density"],
                    "n": point["n"],
                    "queries_checked": point["queries_checked"],
                    "identical": point["identical"],
                    "queries_different": point["queries_different"],
                }
            )
            if not point["identical"]:
                raise ValueError(
                    f"the two traversals disagree on {point['label']} n={point['n']}: "
                    "the campaign timed two different computations"
                )
        if not document["identical"]:
            raise ValueError(f"{name}: document says the traversals disagree")
    if not found:
        return None
    return {
        "pools_checked": pools,
        "queries_checked": queries_checked,
        "identical": all(point["identical"] for point in points),
        "points": points,
    }


def build(raw: Path, *, commit: str, date: str, machine: str) -> dict:
    """Builds the artefact of the alternating campaign."""
    documents = {}
    for campaign, name in CAMPAIGNS.items():
        path = raw / name
        if path.exists():
            documents[campaign] = json.loads(path.read_text(encoding="utf-8"))
    if not documents:
        raise FileNotFoundError(f"no cross document under {raw}")
    queries = json.loads((raw.parent / "queries.json").read_text(encoding="utf-8"))
    first = next(iter(documents.values()))
    return {
        "measure": "MES-02",
        "campaign": "parcours dense et clairsemé, passes alternées",
        "protocol": "ADR-0077 (pools, requêtes, percentiles), ADR-0071 (seuil)",
        "commit": commit,
        "date": date,
        "machine": machine,
        "dataset": {
            "name": "synth-large",
            "index": "data/mes02/indexes/synth-large (measurement copy)",
            "live_passages": first["live_passages"],
            "dims": first["dims"],
            "threads": first["threads"],
            "queries": first["queries"],
            "queries_sha256": queries["sha256"],
            "queries_law": queries["law"],
            "queries_seed": queries["seed"],
            "queries_stream": queries["stream"],
        },
        "vectors": relative(first["vectors"]),
        "documents": list(documents),
        "controls": control_rows(raw),
        "rows": rows(raw),
    }


def markdown(document: dict) -> str:
    """Prints the tables the report quotes, in microseconds."""
    lines = []
    controls = document.get("controls")
    if controls:
        lines.append(
            f"## Contrôle d'accord des parcours — {controls['pools_checked']} pools, "
            f"{controls['queries_checked']} requêtes, identiques : {controls['identical']}"
        )
        lines.append("")
    lines.append("## Parcours dense et clairsemé — p50 / p95 / p99 (µs)")
    lines.append("")
    lines.append("| campagne | pool | forme | span | densité | N | paires | dense p50 | p95 | p99 "
                 "| clairsemé p50 | p95 | p99 | clairsemé − dense |")
    lines.append("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
    for row in document["rows"]:
        lines.append(
            f"| {row['campaign']} | {row['size']} | {row['kind']} | {row['span']} | "
            f"{row['density']:.3f} | {row['n']} | {row['pairs_count']} | "
            f"{row['dense']['p50']:.0f} | {row['dense']['p95']:.0f} | {row['dense']['p99']:.0f} | "
            f"{row['sparse']['p50']:.0f} | {row['sparse']['p95']:.0f} | {row['sparse']['p99']:.0f} | "
            f"{row['sparse_minus_dense_percent']:+.1f} % |"
        )
    lines.append("")
    lines.append("## Dispersion d'une paire à l'autre (p50, % ; max/min − 1)")
    lines.append("")
    lines.append("| campagne | pool | densité | N | dense | clairsemé | écart (points) |")
    lines.append("|---|---|---|---|---|---|---|")
    for row in document["rows"]:
        spread = row["p50_spread"]
        lines.append(
            f"| {row['campaign']} | {row['size']} | {row['density']:.3f} | {row['n']} | "
            f"{spread['dense_percent']:.1f} | {spread['sparse_percent']:.1f} | "
            f"{spread['gap_points']:.1f} |"
        )
    lines.append("")
    lines.append("## Écart par paire de passes (%)")
    lines.append("")
    lines.append("| campagne | pool | densité | N | paires |")
    lines.append("|---|---|---|---|---|")
    for row in document["rows"]:
        gaps = ", ".join(
            f"{pair['sparse_minus_dense_percent']:+.1f}" for pair in row["pairs"]
        )
        lines.append(
            f"| {row['campaign']} | {row['size']} | {row['density']:.3f} | {row['n']} | {gaps} |"
        )
    return "\n".join(lines)


def write(document: dict, measures: Path, date: str) -> Path:
    """Writes the dated artefact; it never overwrites another date."""
    measures.mkdir(parents=True, exist_ok=True)
    path = measures / f"{ARTEFACT}.{date}.json"
    path.write_text(json.dumps(document, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    return path


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--raw", type=Path, default=DEFAULT_RAW)
    parser.add_argument("--mesures", type=Path, default=DEFAULT_MEASURES)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--date", required=True)
    parser.add_argument("--machine", required=True)
    arguments = parser.parse_args(argv)
    document = build(
        arguments.raw,
        commit=arguments.commit,
        date=arguments.date,
        machine=arguments.machine,
    )
    print(f"wrote {write(document, arguments.mesures, arguments.date)}")
    print()
    print(markdown(document))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
