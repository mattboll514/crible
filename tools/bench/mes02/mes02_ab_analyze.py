# SPDX-License-Identifier: Apache-2.0
"""A/B verification of the single sparse traversal (ADR-0082, decisions 2 to 5).

The code of `UNIFIER-PARCOURS-SCAN` replaces the density rule of ADR-0071 by a
single sparse traversal with a segment cursor (ADR-0081, ADR-0082). ADR-0082,
decision 2, makes a measurement the last step before the task is declared done,
and decision 3 writes the noise rule that reads it:

- **old binary**: the density rule, built from the commit `e7d4e08` outside the
  working tree (`data/ab-old/src`, binary under `data/ab-old/target`). For each
  pool, the latency of the old side is the series of the traversal that its rule
  chose, named `traversal_automatic` in the document;
- **new binary**: the working tree, one sparse traversal, its series named
  `sparse`;
- **three copies** of `synth-large` that differ only by their number of segments
  (1, 8, 100), **every point** of the multi-segment campaign of MES-02;
- **three pairs** per copy, the order of the pairs alternating (old -> new,
  new -> old, old -> new), and the order of the copies rotating.

Decision 3: a point is **lost** only when the new binary is slower in p50 *and*
the gap exceeds the amplitude of its own pair-to-pair gaps on that point.
Otherwise it is a **tie**, and a tie satisfies the criterion of the task.

The script refuses, rather than warns, on: a document whose number of segments
is not the one its copy claims; a pool that one document measured and another
did not, or measured on another span; a document whose N is not 5000 or whose
query count is not 300; an old document whose `traversal_automatic` is not one
of `dense`/`sparse`; and a failing agreement control on either binary.

Run from the repository root::

    (cd tools && uv run --no-sync python bench/mes02/mes02_ab_analyze.py \\
        --commit "$(git rev-parse HEAD)" --date 2026-10-27 \\
        --machine "MacBook Air M3, 8 cores, 24 GB, macOS 15.7.7, native")
"""

from __future__ import annotations

import argparse
import json
import statistics
from pathlib import Path

from mes02_analyze import (
    DEFAULT_MEASURES,
    DEFAULT_RAW,
    check_summary,
    relative,
)
from mes02_parcours_analyze import percent, range_points

#: Name of the versioned artefact, without the date.
ARTEFACT = "MES-02-ab-parcours-unique"

#: One entry per measurement copy: the number of segments the copy must have and
#: the role it plays in the campaign.
COPIES = {
    "synth-large": {
        "segments": 1,
        "copy": "copie de référence : un segment, 8,6 M de passages, 7,74 M vivants, densité 0,90",
    },
    "synth-large-8seg": {
        "segments": 8,
        "copy": "copie compactée : 8 segments de 1 M de passages, aucun passage mort",
    },
    "synth-large-multi": {
        "segments": 100,
        "copy": "copie multi-attaches : 100 segments de 86 000 passages, même ensemble de vivants",
    },
}

#: The three pairs of the protocol, in the order ADR-0082 fixes: the side that
#: opens a pair alternates (old, new, old).
ROUNDS = (1, 2, 3)

#: N of the campaign (ADR-0077).
N = 5000
#: Queries of the campaign.
QUERIES = 300

#: The two pools of the budget line of `docs/03-recherche.md:144`: the 1 M
#: contiguous pool and the full pool.
BUDGET_LABELS = ("contiguous-1000000", "full")


def median(values: list[float]) -> float:
    """Median of a series, refused on an empty one."""
    if not values:
        raise ValueError("median of an empty series")
    return statistics.median(values)


def read(path: Path) -> dict:
    """Reads one raw document, refusing a missing one."""
    if not path.exists():
        raise FileNotFoundError(f"missing raw document: {path}")
    return json.loads(path.read_text(encoding="utf-8"))


def points_of(document: dict, label: str) -> dict[str, dict]:
    """The points of one document, keyed by label, refused when duplicated."""
    points: dict[str, dict] = {}
    for point in document["points"]:
        key = point["label"]
        if key in points:
            raise ValueError(f"{label}: two points labelled {key}")
        points[key] = point
    return points


def old_series(point: dict, label: str) -> list[float]:
    """The old side of a point: the series of the traversal its rule chose.

    `traversal_automatic` is what makes the A/B compare the new single traversal
    with **what ran in production**, not with a traversal picked after the fact.
    """
    chosen = point.get("traversal_automatic")
    if chosen not in ("dense", "sparse"):
        raise ValueError(
            f"{label}: the old document names no traversal its rule chose "
            f"(traversal_automatic = {chosen!r})"
        )
    return point[chosen]["durations_us"]


def new_series(point: dict, label: str) -> list[float]:
    """The new side of a point: the single sparse traversal."""
    if point.get("traversal") != "sparse":
        raise ValueError(
            f"{label}: the new document does not measure the single sparse "
            f"traversal (traversal = {point.get('traversal')!r})"
        )
    return point["sparse"]["durations_us"]


def check_document(document: dict, *, copy: str, label: str) -> dict[str, dict]:
    """Checks one raw document against the copy it claims, then keys its points."""
    if document.get("kind") != "cross":
        raise ValueError(f"{label}: not a cross document ({document.get('kind')!r})")
    expected = COPIES[copy]["segments"]
    if document.get("segments") != expected:
        raise ValueError(
            f"{label}: the copy claims {expected} segments and the document "
            f"counts {document.get('segments')}"
        )
    if document.get("queries") != QUERIES:
        raise ValueError(f"{label}: {document.get('queries')} queries, {QUERIES} expected")
    points = points_of(document, label)
    for name, point in points.items():
        if point["n"] != N:
            raise ValueError(f"{label} {name}: N = {point['n']}, {N} expected")
        if point["pairs_count"] != len(ROUNDS):
            raise ValueError(
                f"{label} {name}: {point['pairs_count']} passes, {len(ROUNDS)} expected"
            )
    return points


def check_same_pools(reference: dict[str, dict], other: dict[str, dict], label: str) -> None:
    """Refuses two documents that did not measure the same pools.

    Same labels, in the same order, on the same span and the same first and last
    identifiers: a pool rebuilt with another seed or another live set would make
    the two latencies incomparable, silently.
    """
    if list(reference) != list(other):
        raise ValueError(f"{label}: the pools differ ({len(reference)} vs {len(other)} labels)")
    for name, expected in reference.items():
        actual = other[name]
        for key in ("size", "span", "first_iid", "last_iid", "kind"):
            if expected[key] != actual[key]:
                raise ValueError(
                    f"{label} {name}: {key} is {actual[key]} here and {expected[key]} "
                    f"in the reference document"
                )


def check_control(document: dict, *, copy: str, side: str) -> dict:
    """Refuses a failing agreement control."""
    if document.get("kind") != "agree":
        raise ValueError(f"control {side} {copy}: not an agree document")
    if document.get("segments") != COPIES[copy]["segments"]:
        raise ValueError(
            f"control {side} {copy}: {document.get('segments')} segments, "
            f"{COPIES[copy]['segments']} expected"
        )
    if not document.get("identical"):
        raise ValueError(f"control {side} {copy}: the agreement control failed")
    return {
        "side": side,
        "copy": copy,
        "segments": document["segments"],
        "pools_checked": document["pools_checked"],
        "queries_checked": document["queries_checked"],
        "identical": True,
    }


def verdict(gaps: list[float]) -> dict:
    """Verdict of one point, by ADR-0082, decision 3.

    `gaps` are the pair-to-pair gaps of the point, new against old, in percent
    (positive: the new binary is slower). The point's gap is their median, and
    the noise of that point is their amplitude (max − min), the same reading the
    campaign of the 2026-10-27 uses.

    - **perdu** when the median gap is positive and larger than the amplitude;
    - **gagné** when it is negative and its magnitude is larger than the
      amplitude (symmetric reading, declared in the report);
    - **match nul** otherwise, which **satisfies** the criterion of the task.
    """
    central = median(gaps)
    amplitude = range_points(gaps)
    if central > 0 and central > amplitude:
        label = "perdu"
    elif central < 0 and -central > amplitude:
        label = "gagné"
    else:
        label = "match nul"
    return {
        "verdict": label,
        "gap_median_percent": central,
        "gap_amplitude_points": amplitude,
        "gap_min_percent": min(gaps),
        "gap_max_percent": max(gaps),
    }


def point_rows(raw: Path) -> tuple[list[dict], dict[str, dict]]:
    """One row per copy and point, with its verdict and the readings it rests on."""
    rows: list[dict] = []
    for copy in COPIES:
        documents = {
            (side, round_number): read(raw / f"ab-{side}-{copy}-r{round_number}.json")
            for round_number in ROUNDS
            for side in ("old", "new")
        }
        reference = check_document(
            documents[("old", ROUNDS[0])], copy=copy, label=f"old {copy} r{ROUNDS[0]}"
        )
        checked: dict[tuple[str, int], dict[str, dict]] = {}
        for (side, round_number), document in documents.items():
            label = f"{side} {copy} r{round_number}"
            checked[(side, round_number)] = check_document(document, copy=copy, label=label)
            check_same_pools(reference, checked[(side, round_number)], label)

        for name, reference_point in reference.items():
            old_p50: list[float] = []
            new_p50: list[float] = []
            old_p95: list[float] = []
            new_p95: list[float] = []
            chosen: list[str] = []
            for round_number in ROUNDS:
                old_point = checked[("old", round_number)][name]
                new_point = checked[("new", round_number)][name]
                label = f"{copy} {name} r{round_number}"
                old_summary = check_summary(
                    old_series(old_point, label), old_point[old_point["traversal_automatic"]]["summary"], f"{label} old"
                )
                new_summary = check_summary(
                    new_series(new_point, label), new_point["sparse"]["summary"], f"{label} new"
                )
                chosen.append(old_point["traversal_automatic"])
                old_p50.append(old_summary["p50"])
                new_p50.append(new_summary["p50"])
                old_p95.append(old_summary["p95"])
                new_p95.append(new_summary["p95"])
            gaps = [percent(new, old) for old, new in zip(old_p50, new_p50)]
            gaps_p95 = [percent(new, old) for old, new in zip(old_p95, new_p95)]
            rows.append(
                {
                    "copy": copy,
                    "segments": COPIES[copy]["segments"],
                    "label": name,
                    "kind": reference_point["kind"],
                    "size": reference_point["size"],
                    "span": reference_point["span"],
                    "density": reference_point["density"],
                    "n": reference_point["n"],
                    "old_traversal": sorted(set(chosen)),
                    "old_p50_us": old_p50,
                    "new_p50_us": new_p50,
                    "old_p95_us": old_p95,
                    "new_p95_us": new_p95,
                    "p50_gaps_percent": gaps,
                    "p95_gaps_percent": gaps_p95,
                    **verdict(gaps),
                    "p95_gap_median_percent": median(gaps_p95),
                    "p95_gap_amplitude_points": range_points(gaps_p95),
                    "p50_gap_of_medians_percent": percent(median(new_p50), median(old_p50)),
                }
            )
    controls = {
        copy: [
            check_control(read(raw / f"ab-agree-{side}-{copy}.json"), copy=copy, side=side)
            for side in ("old", "new")
        ]
        for copy in COPIES
    }
    return rows, controls


def segment_cost(rows: list[dict]) -> list[dict]:
    """Cost of the number of segments left by the cursor, **new binary only**.

    Read inside each round, at equal size and span, on the labels the three
    copies share: the p50 of the new binary at 8 and at 100 segments against its
    own p50 at 1 segment, round by round. The campaign of the 2026-10-27 read the
    same quantity on the two traversals it had; here there is one, and the
    question is only what the cursor left.
    """
    by_key: dict[tuple[int, str, str], float] = {}
    for row in rows:
        by_key[(row["segments"], row["copy"], row["label"])] = row["new_p50_us"]
    labels = [row["label"] for row in rows if row["copy"] == "synth-large"]
    shared = [name for name in labels if all(name in {r["label"] for r in rows if r["copy"] == c} for c in COPIES)]
    cost: list[dict] = []
    for name in shared:
        per_round: dict[str, list[float]] = {"8v1": [], "100v1": [], "100v8": []}
        for round_number in range(len(ROUNDS)):
            one = rows_one(rows, "synth-large", name)["new_p50_us"][round_number]
            eight = rows_one(rows, "synth-large-8seg", name)["new_p50_us"][round_number]
            hundred = rows_one(rows, "synth-large-multi", name)["new_p50_us"][round_number]
            per_round["8v1"].append(percent(eight, one))
            per_round["100v1"].append(percent(hundred, one))
            per_round["100v8"].append(percent(hundred, eight))
        cost.append(
            {
                "label": name,
                "size": rows_one(rows, "synth-large", name)["size"],
                "span": rows_one(rows, "synth-large", name)["span"],
                "n": N,
                "eight_vs_one": per_round["8v1"],
                "hundred_vs_one": per_round["100v1"],
                "hundred_vs_eight": per_round["100v8"],
                "eight_vs_one_median_percent": median(per_round["8v1"]),
                "eight_vs_one_amplitude_points": range_points(per_round["8v1"]),
                "hundred_vs_one_median_percent": median(per_round["100v1"]),
                "hundred_vs_one_amplitude_points": range_points(per_round["100v1"]),
                "hundred_vs_eight_median_percent": median(per_round["100v8"]),
                "hundred_vs_eight_amplitude_points": range_points(per_round["100v8"]),
            }
        )
    return cost


def rows_one(rows: list[dict], copy: str, label: str) -> dict:
    """The one row of a copy and a label, refused when missing or duplicated."""
    matches = [row for row in rows if row["copy"] == copy and row["label"] == label]
    if len(matches) != 1:
        raise ValueError(f"{len(matches)} rows for {copy} {label}")
    return matches[0]


def budget(rows: list[dict]) -> list[dict]:
    """p50 and p95 of the new binary on the two pools of the budget line.

    Reported, not judged: ADR-0082, decision 5, leaves the percentile of the
    budget of `docs/03-recherche.md:144` to P8, measured in a container.
    """
    lines: list[dict] = []
    for copy in COPIES:
        for name in BUDGET_LABELS:
            row = rows_one(rows, copy, name)
            lines.append(
                {
                    "copy": copy,
                    "segments": COPIES[copy]["segments"],
                    "label": name,
                    "size": row["size"],
                    "n": N,
                    "p50_us": row["new_p50_us"],
                    "p95_us": row["new_p95_us"],
                    "p50_us_median": median(row["new_p50_us"]),
                    "p95_us_median": median(row["new_p95_us"]),
                    "p50_us_min": min(row["new_p50_us"]),
                    "p50_us_max": max(row["new_p50_us"]),
                    "p95_us_min": min(row["new_p95_us"]),
                    "p95_us_max": max(row["new_p95_us"]),
                }
            )
    return lines


def build(raw: Path, *, commit: str, date: str, machine: str) -> dict:
    """Builds the artefact of the A/B verification."""
    rows, controls = point_rows(raw)
    queries = json.loads((raw.parent / "queries.json").read_text(encoding="utf-8"))
    tally = {
        copy: {
            name: sum(1 for row in rows if row["copy"] == copy and row["verdict"] == name)
            for name in ("gagné", "match nul", "perdu")
        }
        for copy in COPIES
    }
    return {
        "measure": "MES-02",
        "campaign": "vérification A/B du parcours unique (ADR-0082)",
        "protocol": (
            "ADR-0077 (N = 5000, 300 requêtes, p50/p95/p99), ADR-0082 décisions 2 et 3 "
            "(binaire ancien de e7d4e08, trois copies, trois paires alternées, règle de bruit)"
        ),
        "commit": commit,
        "old_commit": "e7d4e08",
        "date": date,
        "machine": machine,
        "dataset": {
            "name": "synth-large",
            "indexes": {copy: f"data/mes02/indexes/{copy}" for copy in COPIES},
            "live_passages": 7_740_000,
            "dims": 768,
            "threads": 8,
            "queries": QUERIES,
            "queries_sha256": queries["sha256"],
            "queries_law": queries["law"],
            "queries_seed": queries["seed"],
            "queries_stream": queries["stream"],
        },
        "vectors": relative(f"data/mes02/{queries.get('matrix', 'queries.f32')}"),
        "documents": [
            f"ab-{side}-{copy}-r{round_number}.json"
            for copy in COPIES
            for round_number in ROUNDS
            for side in ("old", "new")
        ],
        "controls": controls,
        "tally": tally,
        "rows": rows,
        "segment_cost": segment_cost(rows),
        "budget": budget(rows),
    }


def markdown(document: dict) -> str:
    """Prints the tables the report quotes, in microseconds and percent."""
    lines: list[str] = []
    lines.append("## Contrôles d'accord (tri de Hamming complet)")
    lines.append("")
    lines.append("| binaire | copie | segments | pools | requêtes | identiques |")
    lines.append("|---|---|---|---|---|---|")
    for copy, controls in document["controls"].items():
        for control in controls:
            lines.append(
                f"| {control['side']} | {copy} | {control['segments']} | "
                f"{control['pools_checked']} | {control['queries_checked']} | "
                f"{control['identical']} |"
            )
    lines.append("")
    lines.append("## Verdict par point (p50, trois paires)")
    lines.append("")
    lines.append("| copie | seg | pool | taille | N | ancien | écart par paire (%) | écart médian "
                 "| amplitude | p95 médian | p95 amplitude | verdict |")
    lines.append("|---|---|---|---|---|---|---|---|---|---|---|---|")
    for row in document["rows"]:
        gaps = ", ".join(f"{gap:+.1f}" for gap in row["p50_gaps_percent"])
        traversals = "/".join(row["old_traversal"])
        lines.append(
            f"| {row['copy']} | {row['segments']} | {row['label']} | {row['size']} | "
            f"{row['n']} | {traversals} | {gaps} | {row['gap_median_percent']:+.1f} % | "
            f"{row['gap_amplitude_points']:.1f} pts | {row['p95_gap_median_percent']:+.1f} % | "
            f"{row['p95_gap_amplitude_points']:.1f} pts | {row['verdict']} |"
        )
    lines.append("")
    lines.append("## Coût résiduel du nombre de segments (nouveau binaire, p50)")
    lines.append("")
    lines.append("| pool | taille | N | 8 vs 1 (%) | 100 vs 1 (%) | 100 vs 8 (%) | amplitudes (pts) |")
    lines.append("|---|---|---|---|---|---|---|")
    for line in document["segment_cost"]:
        lines.append(
            f"| {line['label']} | {line['size']} | {line['n']} | "
            f"{line['eight_vs_one_median_percent']:+.1f} | "
            f"{line['hundred_vs_one_median_percent']:+.1f} | "
            f"{line['hundred_vs_eight_median_percent']:+.1f} | "
            f"{line['eight_vs_one_amplitude_points']:.1f} / "
            f"{line['hundred_vs_one_amplitude_points']:.1f} / "
            f"{line['hundred_vs_eight_amplitude_points']:.1f} |"
        )
    lines.append("")
    lines.append("## Budget de `docs/03-recherche.md:144` (nouveau binaire, p50 / p95)")
    lines.append("")
    lines.append("| copie | segments | pool | N | p50 médian (µs) | p95 médian (µs) | p50 min–max | p95 min–max |")
    lines.append("|---|---|---|---|---|---|---|---|")
    for line in document["budget"]:
        lines.append(
            f"| {line['copy']} | {line['segments']} | {line['label']} | {line['n']} | "
            f"{line['p50_us_median']:.0f} | {line['p95_us_median']:.0f} | "
            f"{line['p50_us_min']:.0f}–{line['p50_us_max']:.0f} | "
            f"{line['p95_us_min']:.0f}–{line['p95_us_max']:.0f} |"
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
