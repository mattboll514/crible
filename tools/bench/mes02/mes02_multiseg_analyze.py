# SPDX-License-Identifier: Apache-2.0
"""Aggregation of the multi-segment campaign of MES-02 (ADR-0081).

The campaign aggregates what ADR-0081, decision 2, makes a **precondition** of
its decision: both traversals of the binary scan, forced on the same pool and
the same query vectors, on measurement copies of `synth-large` that differ by
their **number of segments** — one segment, eight compacted segments, one
hundred segments from a hundred attaches — and the same protocol as the campaign
of the 2026-10-27, with the order of the pairs alternating (decision 3).

It reads the raw documents written by the harness
(`crates/crible/examples/dense_latency.rs`, modes `cross` and `agree`, under
`data/mes02/raw/`), recomputes every percentile from the per-query durations,
checks them against the summaries the harness recorded, and writes one versioned
artefact under `docs/mesures/`. It prints the Markdown tables of the report.

Three checks are refusals, not warnings:

- the number of segments of a raw document must be the one the copy is declared
  to have, so a run labelled "8 segments" that opened another index stops the
  aggregation;
- the order inside each pair must alternate (dense then sparse, sparse then
  dense, dense then sparse), which is the correction ADR-0081, decision 3, makes
  to the campaign of the 2026-10-27;
- the agreement control must hold on every pool of every copy.

Percentiles, dispersion and the sign of a gap are those of
`mes02_parcours_analyze`, imported rather than rewritten, so the two campaigns of
MES-02 cannot drift apart.

Run from the repository root::

    (cd tools && uv run --no-sync python bench/mes02/mes02_multiseg_analyze.py \\
        --commit "$(git rev-parse HEAD)" --date 2026-10-27 \\
        --machine "MacBook Air M3, 8 cores, 24 GB, macOS 15.7.7, native")
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from mes02_analyze import DEFAULT_MEASURES, DEFAULT_RAW, check_summary, nearest_rank, relative
from mes02_parcours_analyze import percent, range_points, spread

#: One entry per measurement copy: the raw documents, the control, and the
#: number of segments the copy must have. `synth-large-repeat` is the reference
#: copy measured a second time, at the end of the campaign, to read the drift of
#: the machine instead of assuming it away.
CAMPAIGNS = {
    "synth-large": {
        "cross": "cross-synth-large.json",
        "agree": "agree-synth-large.json",
        "index": "synth-large",
        "segments": 1,
        "copy": "copie de référence : un segment, 8,6 M de passages, 7,74 M vivants, densité 0,90",
    },
    "synth-large-8seg": {
        "cross": "cross-synth-large-8seg.json",
        "agree": "agree-synth-large-8seg.json",
        "index": "synth-large-8seg",
        "segments": 8,
        "copy": "copie compactée : 8 segments de 1 M de passages, aucun passage mort",
    },
    "synth-large-multi": {
        "cross": "cross-synth-large-multi.json",
        "agree": "agree-synth-large-multi.json",
        "index": "synth-large-multi",
        "segments": 100,
        "copy": "copie multi-attaches : 100 segments de 86 000 passages, même ensemble de vivants",
    },
    "synth-large-repeat": {
        "cross": "cross-synth-large-repeat.json",
        "agree": None,
        "index": "synth-large",
        "segments": 1,
        "copy": "la copie à un segment mesurée une seconde fois, en fin de campagne (dérive)",
    },
}

#: Name of the versioned artefact, without the date.
ARTEFACT = "MES-02-multisegments"

#: Order of the two traversals inside the pairs, as ADR-0081, decision 3, fixes
#: it: the pairs alternate, so neither traversal is always the second.
def expected_order(pair: int) -> str:
    """Order of pair number `pair`, counting from one."""
    return "dense-then-sparse" if pair % 2 == 1 else "sparse-then-dense"


def measure_row(point: dict, campaign: str, segments: int) -> dict:
    """One pool of one copy, recomputed and checked against the harness."""
    label = f"{campaign} {point['label']} n={point['n']}"
    dense = point["dense"]
    sparse = point["sparse"]
    dense_summary = check_summary(dense["durations_us"], dense["summary"], f"{label} dense")
    sparse_summary = check_summary(sparse["durations_us"], sparse["summary"], f"{label} sparse")
    pairs = []
    for pair in point["pairs"]:
        order = pair.get("order")
        wanted = expected_order(pair["pair"])
        if order != wanted:
            raise ValueError(
                f"{label}: pair {pair['pair']} ran '{order}' instead of '{wanted}': "
                "ADR-0081, decision 3, requires the order of the pairs to alternate"
            )
        dense_pair = check_summary(
            pair["dense_us"], pair["dense"], f"{label} pair {pair['pair']} dense"
        )
        sparse_pair = check_summary(
            pair["sparse_us"], pair["sparse"], f"{label} pair {pair['pair']} sparse"
        )
        pairs.append(
            {
                "pair": pair["pair"],
                "order": order,
                "dense_us": dense_pair,
                "sparse_us": sparse_pair,
                "sparse_minus_dense_percent": percent(sparse_pair["p50"], dense_pair["p50"]),
            }
        )
    return {
        "campaign": campaign,
        "segments": segments,
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


def documents_of(raw: Path) -> dict[str, dict]:
    """Opens the raw documents of the copies that were measured."""
    documents = {}
    for campaign, spec in CAMPAIGNS.items():
        path = raw / spec["cross"]
        if not path.exists():
            continue
        document = json.loads(path.read_text(encoding="utf-8"))
        if document["index"] != spec["index"]:
            raise ValueError(
                f"{campaign}: the raw document opened '{document['index']}' instead of "
                f"'{spec['index']}': the copy is not the one the campaign declares"
            )
        if document["segments"] != spec["segments"]:
            raise ValueError(
                f"{campaign}: {document['segments']} segments instead of {spec['segments']}: "
                "the copy is not the one the campaign declares"
            )
        documents[campaign] = document
    if not documents:
        raise FileNotFoundError(f"no cross document under {raw}: run the campaign first")
    return documents


def rows_of(documents: dict[str, dict]) -> list[dict]:
    """Every point of every copy, copy by copy."""
    rows = []
    for campaign, document in documents.items():
        for point in document["points"]:
            rows.append(measure_row(point, campaign, document["segments"]))
    rows.sort(
        key=lambda row: (
            row["campaign"],
            row["kind"],
            row["span"],
            row["density"],
            row["n"],
        )
    )
    return rows


def control_rows(raw: Path) -> dict | None:
    """The agreement control of each copy: same candidates, same order.

    A replica that reads a different passage set would change the work being
    timed without changing anything visible, so a disagreement is a refusal, not
    a warning printed next to the figure.
    """
    copies: list[dict] = []
    found = False
    for campaign, spec in CAMPAIGNS.items():
        if spec["agree"] is None:
            continue
        path = raw / spec["agree"]
        if not path.exists():
            continue
        found = True
        document = json.loads(path.read_text(encoding="utf-8"))
        if document["segments"] != spec["segments"]:
            raise ValueError(
                f"{campaign}: the control ran on {document['segments']} segments, "
                f"not {spec['segments']}"
            )
        queries = 0
        for point in document["points"]:
            queries += point["queries_checked"]
            if not point["identical"]:
                raise ValueError(
                    f"the two traversals disagree on {campaign} {point['label']} "
                    f"n={point['n']}: the campaign timed two different computations"
                )
        if not document["identical"]:
            raise ValueError(f"{campaign}: document says the traversals disagree")
        copies.append(
            {
                "campaign": campaign,
                "segments": document["segments"],
                "pools_checked": len(document["points"]),
                "queries_checked": queries,
                "identical": True,
            }
        )
    if not found:
        return None
    return {
        "copies": copies,
        "identical": all(copy["identical"] for copy in copies),
        "pools_checked": sum(copy["pools_checked"] for copy in copies),
        "queries_checked": sum(copy["queries_checked"] for copy in copies),
    }


def same_pools(rows: list[dict]) -> dict:
    """Whether the one-segment and the multi-attach copies measured the same pools.

    The multi-attach copy was built from the same rows, in the same order, and
    deleted the same documents, so every pool must carry the same identifiers as
    on the one-segment copy. This is what makes "the same pool and the same span"
    a fact of the campaign rather than a claim: a difference would mean the two
    copies do not compare.
    """
    first = {(row["label"], row["n"]): row for row in rows if row["campaign"] == "synth-large"}
    multi = {
        (row["label"], row["n"]): row for row in rows if row["campaign"] == "synth-large-multi"
    }
    differences = []
    compared = 0
    for key, row in sorted(first.items()):
        other = multi.get(key)
        if other is None:
            continue
        compared += 1
        for field in ("size", "span", "first_iid", "last_iid"):
            if row[field] != other[field]:
                differences.append(
                    {
                        "label": row["label"],
                        "n": row["n"],
                        "field": field,
                        "one_segment": row[field],
                        "multi_attach": other[field],
                    }
                )
        if abs(row["density"] - other["density"]) > 1e-9:
            differences.append(
                {
                    "label": row["label"],
                    "n": row["n"],
                    "field": "density",
                    "one_segment": row["density"],
                    "multi_attach": other["density"],
                }
            )
    return {"compared": compared, "identical": not differences, "differences": differences}


def segment_cost(rows: list[dict]) -> list[dict]:
    """Cost of the number of segments, per pool, at equal size and span.

    Reads the sparse traversal only: it is the one that pays a segment lookup per
    passage (`segment_of`, `crates/crible-search/src/dense/binary.rs`), which is
    the cost ADR-0081 leaves to a segment cursor if the measure shows it.
    """
    by_copy: dict[str, dict[tuple[str, int], dict]] = {}
    for row in rows:
        if row["campaign"] == "synth-large-repeat":
            continue
        by_copy.setdefault(row["campaign"], {})[(row["label"], row["n"])] = row
    reference = by_copy.get("synth-large", {})
    compacted = by_copy.get("synth-large-8seg", {})
    multi = by_copy.get("synth-large-multi", {})
    cost = []
    for key, one in sorted(reference.items()):
        eight = compacted.get(key)
        hundred = multi.get(key)
        if eight is None or hundred is None:
            continue
        cost.append(
            {
                "label": one["label"],
                "kind": one["kind"],
                "n": one["n"],
                "size": one["size"],
                "span_one_segment": one["span"],
                "span_compacted": eight["span"],
                "density_one_segment": one["density"],
                "density_compacted": eight["density"],
                "sparse_p50_us": {
                    "one_segment": one["sparse"]["p50"],
                    "eight_segments": eight["sparse"]["p50"],
                    "hundred_segments": hundred["sparse"]["p50"],
                },
                "dense_p50_us": {
                    "one_segment": one["dense"]["p50"],
                    "eight_segments": eight["dense"]["p50"],
                    "hundred_segments": hundred["dense"]["p50"],
                },
                "sparse_gap_8_vs_1_percent": percent(
                    eight["sparse"]["p50"], one["sparse"]["p50"]
                ),
                "sparse_gap_100_vs_1_percent": percent(
                    hundred["sparse"]["p50"], one["sparse"]["p50"]
                ),
                "sparse_gap_100_vs_8_percent": percent(
                    hundred["sparse"]["p50"], eight["sparse"]["p50"]
                ),
            }
        )
    return cost


def drift(rows: list[dict]) -> list[dict]:
    """Difference between the two runs of the reference copy, per pool."""
    first = {(row["label"], row["n"]): row for row in rows if row["campaign"] == "synth-large"}
    again = {
        (row["label"], row["n"]): row for row in rows if row["campaign"] == "synth-large-repeat"
    }
    drift_rows = []
    for key, row in sorted(first.items()):
        other = again.get(key)
        if other is None:
            continue
        drift_rows.append(
            {
                "label": row["label"],
                "n": row["n"],
                "dense_percent": percent(other["dense"]["p50"], row["dense"]["p50"]),
                "sparse_percent": percent(other["sparse"]["p50"], row["sparse"]["p50"]),
            }
        )
    return drift_rows


#: The three copies of the interleaved series, named the way the recipe runs
#: them. One copy per position and per round, so no copy is always measured at
#: the same distance from the start of the hour.
INTERLEAVED_COPIES = ("synth-large", "synth-large-8seg", "synth-large-multi")

#: Rounds of the Latin square: three copies, three positions.
INTERLEAVED_ROUNDS = (1, 2, 3)

#: The gaps the series reads, round by round. The three `sparse_*` are the
#: question; the three `dense_*` are the control, because the dense traversal
#: walks the pool linearly and must not depend on the number of segments: a
#: dense gap of the same size as a sparse gap is the machine drifting, not the
#: segment count costing.
INTERLEAVED_GAPS = (
    "sparse_gap_8_vs_1_percent",
    "sparse_gap_100_vs_1_percent",
    "sparse_gap_100_vs_8_percent",
    "dense_gap_8_vs_1_percent",
    "dense_gap_100_vs_1_percent",
    "dense_gap_100_vs_8_percent",
)


def interleaved_documents(raw: Path) -> list[dict]:
    """The raw documents of the interleaved series, round by round, in real order.

    The raw document does not carry the order the copies were run in, only the
    moment the run ended, so the order is read from the modification time of the
    file. The result is then checked to be a Latin square — every copy measured
    once per round, and never twice at the same position — so a recipe that
    looped over one copy and forgot another is refused instead of averaged.
    """
    rounds = []
    for number in INTERLEAVED_ROUNDS:
        measured = []
        for campaign in INTERLEAVED_COPIES:
            spec = CAMPAIGNS[campaign]
            path = raw / f"cross-{campaign}-r{number}.json"
            if not path.exists():
                continue
            document = json.loads(path.read_text(encoding="utf-8"))
            if document["index"] != spec["index"]:
                raise ValueError(
                    f"{campaign} round {number}: the raw document opened "
                    f"'{document['index']}' instead of '{spec['index']}'"
                )
            if document["segments"] != spec["segments"]:
                raise ValueError(
                    f"{campaign} round {number}: {document['segments']} segments instead of "
                    f"{spec['segments']}: the copy is not the one the campaign declares"
                )
            measured.append(
                {
                    "campaign": campaign,
                    "segments": document["segments"],
                    "written": path.stat().st_mtime,
                    "rows": {
                        (point["label"], point["n"]): measure_row(
                            point, campaign, document["segments"]
                        )
                        for point in document["points"]
                    },
                }
            )
        if not measured:
            continue
        measured.sort(key=lambda entry: entry["written"])
        rounds.append(
            {
                "round": number,
                "order": [entry["campaign"] for entry in measured],
                "copies": {
                    entry["campaign"]: {
                        "segments": entry["segments"],
                        "rows": entry["rows"],
                    }
                    for entry in measured
                },
            }
        )
    if not rounds:
        return []
    if len(rounds) != len(INTERLEAVED_ROUNDS):
        raise ValueError(
            f"the interleaved series has {len(rounds)} rounds instead of "
            f"{len(INTERLEAVED_ROUNDS)}: a copy is missing"
        )
    for measured in rounds:
        if set(measured["order"]) != set(INTERLEAVED_COPIES):
            raise ValueError(
                f"round {measured['round']} ran {measured['order']}: every copy must be "
                "measured once per round"
            )
    for campaign in INTERLEAVED_COPIES:
        places = [measured["order"].index(campaign) for measured in rounds]
        if len(set(places)) != len(places):
            raise ValueError(
                f"the interleaved series is not a Latin square: {campaign} sat at "
                f"position {places} over the rounds"
            )
    return rounds


def interleaved_pools(rounds: list[dict]) -> list[dict]:
    """The gaps of the series, pool by pool, round by round, then their median.

    A gap is read *inside* a round, between two runs a few minutes apart, so the
    drift of the machine over the hour no longer adds to the effect being
    looked for. The median over the three rounds is the figure the report
    quotes; the amplitude over the rounds says how much of it is noise.
    """
    keys = set(rounds[0]["copies"][INTERLEAVED_COPIES[0]]["rows"])
    for measured in rounds:
        for campaign in INTERLEAVED_COPIES:
            keys &= set(measured["copies"][campaign]["rows"])
    pools = []
    for label, n in sorted(keys):
        reference = rounds[0]["copies"][INTERLEAVED_COPIES[0]]["rows"][(label, n)]
        measured_rows = []
        for measured in rounds:
            sparse = {
                campaign: measured["copies"][campaign]["rows"][(label, n)]["sparse"]["p50"]
                for campaign in INTERLEAVED_COPIES
            }
            dense = {
                campaign: measured["copies"][campaign]["rows"][(label, n)]["dense"]["p50"]
                for campaign in INTERLEAVED_COPIES
            }
            measured_rows.append(
                {
                    "round": measured["round"],
                    "order": measured["order"],
                    "sparse_p50_us": sparse,
                    "dense_p50_us": dense,
                    "sparse_gap_8_vs_1_percent": percent(
                        sparse["synth-large-8seg"], sparse["synth-large"]
                    ),
                    "sparse_gap_100_vs_1_percent": percent(
                        sparse["synth-large-multi"], sparse["synth-large"]
                    ),
                    "sparse_gap_100_vs_8_percent": percent(
                        sparse["synth-large-multi"], sparse["synth-large-8seg"]
                    ),
                    "dense_gap_8_vs_1_percent": percent(
                        dense["synth-large-8seg"], dense["synth-large"]
                    ),
                    "dense_gap_100_vs_1_percent": percent(
                        dense["synth-large-multi"], dense["synth-large"]
                    ),
                    "dense_gap_100_vs_8_percent": percent(
                        dense["synth-large-multi"], dense["synth-large-8seg"]
                    ),
                }
            )
        pools.append(
            {
                "label": label,
                "kind": reference["kind"],
                "size": reference["size"],
                "span": reference["span"],
                "density": reference["density"],
                "n": n,
                "rounds": measured_rows,
                "median": {
                    gap: nearest_rank([row[gap] for row in measured_rows], 0.5)
                    for gap in INTERLEAVED_GAPS
                },
                "minimum": {
                    gap: min(row[gap] for row in measured_rows) for gap in INTERLEAVED_GAPS
                },
                "maximum": {
                    gap: max(row[gap] for row in measured_rows) for gap in INTERLEAVED_GAPS
                },
                "all_positive": {
                    gap: all(row[gap] > 0 for row in measured_rows)
                    for gap in INTERLEAVED_GAPS
                },
                "range_points": {
                    gap: range_points([row[gap] for row in measured_rows])
                    for gap in INTERLEAVED_GAPS
                },
            }
        )
    return pools


def interleaved(raw: Path) -> dict | None:
    """The interleaved series: the rounds and the gaps of every pool."""
    rounds = interleaved_documents(raw)
    if not rounds:
        return None
    return {
        "rounds": [
            {
                "round": measured["round"],
                "order": measured["order"],
                "segments": {
                    campaign: measured["copies"][campaign]["segments"]
                    for campaign in INTERLEAVED_COPIES
                },
            }
            for measured in rounds
        ],
        "latin_square": True,
        "pools": interleaved_pools(rounds),
    }


def budget(rows: list[dict]) -> list[dict]:
    """The scan budget of `docs/03-recherche.md:143`, read copy by copy.

    The line gives "20 to 80 ms" for the whole index and "a few ms" for a pool of
    1 M passages. The report reads "a few ms" against 10 ms, which is the reading
    the campaign of the 2026-10-27 used; a point above it is reported, never
    silently accepted.
    """
    lines = []
    for campaign in CAMPAIGNS:
        copy_rows = [
            row for row in rows if row["campaign"] == campaign and row["n"] == 5000
        ]
        if not copy_rows:
            continue
        full = next((row for row in copy_rows if row["kind"] == "full"), None)
        window = next(
            (
                row
                for row in copy_rows
                if row["kind"] == "contiguous" and row["size"] <= 1_000_000
            ),
            None,
        )
        lines.append(
            {
                "campaign": campaign,
                "segments": copy_rows[0]["segments"],
                "full_pool": None
                if full is None
                else {
                    "size": full["size"],
                    "span": full["span"],
                    "density": full["density"],
                    "sparse_p50_ms": full["sparse"]["p50"] / 1000.0,
                    "sparse_p95_ms": full["sparse"]["p95"] / 1000.0,
                    "within_20_80_ms": 20.0
                    <= full["sparse"]["p50"] / 1000.0
                    <= 80.0,
                },
                "one_m_pool": None
                if window is None
                else {
                    "size": window["size"],
                    "span": window["span"],
                    "density": window["density"],
                    "sparse_p50_ms": window["sparse"]["p50"] / 1000.0,
                    "sparse_p95_ms": window["sparse"]["p95"] / 1000.0,
                    "within_a_few_ms": window["sparse"]["p95"] / 1000.0 <= 10.0,
                },
            }
        )
    return lines


def build(raw: Path, *, commit: str, date: str, machine: str) -> dict:
    """Builds the artefact of the multi-segment campaign."""
    documents = documents_of(raw)
    rows = rows_of(documents)
    queries = json.loads((raw.parent / "queries.json").read_text(encoding="utf-8"))
    reference = documents.get("synth-large") or next(iter(documents.values()))
    return {
        "measure": "MES-02",
        "campaign": "parcours dense et clairsemé, multi-segments (ADR-0081)",
        "protocol": "ADR-0077 (requêtes, percentiles), ADR-0081 décisions 2 et 3 (copies, ordre des paires)",
        "commit": commit,
        "date": date,
        "machine": machine,
        "dataset": {
            "name": "synth-large",
            "copies": {
                campaign: {
                    "index": f"data/mes02/indexes/{spec['index']}",
                    "segments": spec["segments"],
                    "copy": spec["copy"],
                }
                for campaign, spec in CAMPAIGNS.items()
                if campaign in documents
            },
            "live_passages": reference["live_passages"],
            "dims": reference["dims"],
            "threads": reference["threads"],
            "queries": reference["queries"],
            "queries_sha256": queries["sha256"],
            "queries_law": queries["law"],
            "queries_seed": queries["seed"],
            "queries_stream": queries["stream"],
        },
        "vectors": relative(reference["vectors"]),
        "documents": list(documents),
        "controls": control_rows(raw),
        "same_pools": same_pools(rows),
        "segment_cost": segment_cost(rows),
        "interleaved": interleaved(raw),
        "drift": drift(rows),
        "budget": budget(rows),
        "rows": rows,
    }


def markdown(document: dict) -> str:
    """Prints the tables the report quotes, in microseconds unless stated."""
    lines = []
    controls = document.get("controls")
    if controls:
        lines.append(
            f"## Contrôle d'accord — {controls['pools_checked']} pools, "
            f"{controls['queries_checked']} requêtes, identiques : {controls['identical']}"
        )
        for copy in controls["copies"]:
            lines.append(
                f"- `{copy['campaign']}` ({copy['segments']} segments) : "
                f"{copy['pools_checked']} pools, {copy['queries_checked']} requêtes, "
                f"identiques : {copy['identical']}"
            )
        lines.append("")
    pools = document.get("same_pools")
    if pools:
        lines.append(
            f"## Mêmes pools sur les copies 1 segment et 100 segments — "
            f"{pools['compared']} points comparés, identiques : {pools['identical']}"
        )
        lines.append("")
    lines.append("## Parcours dense et clairsemé, copie par copie — p50 / p95 / p99 (µs)")
    lines.append("")
    lines.append(
        "| copie | segments | pool | forme | span | densité | N | dense p50 | p95 | p99 "
        "| clairsemé p50 | p95 | p99 | clairsemé − dense |"
    )
    lines.append("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
    for row in document["rows"]:
        lines.append(
            f"| {row['campaign']} | {row['segments']} | {row['size']} | {row['kind']} | "
            f"{row['span']} | {row['density']:.3f} | {row['n']} | "
            f"{row['dense']['p50']:.0f} | {row['dense']['p95']:.0f} | {row['dense']['p99']:.0f} | "
            f"{row['sparse']['p50']:.0f} | {row['sparse']['p95']:.0f} | {row['sparse']['p99']:.0f} | "
            f"{row['sparse_minus_dense_percent']:+.1f} % |"
        )
    lines.append("")
    lines.append("## Coût du nombre de segments, à pool et plage égaux (p50, µs)")
    lines.append("")
    lines.append(
        "| pool | forme | taille | N | clairsemé 1 seg | 8 seg | 100 seg "
        "| 8 vs 1 | 100 vs 1 | 100 vs 8 |"
    )
    lines.append("|---|---|---|---|---|---|---|---|---|---|")
    for row in document["segment_cost"]:
        p50 = row["sparse_p50_us"]
        lines.append(
            f"| {row['label']} | {row['kind']} | {row['size']} | {row['n']} | "
            f"{p50['one_segment']:.0f} | {p50['eight_segments']:.0f} | "
            f"{p50['hundred_segments']:.0f} | "
            f"{row['sparse_gap_8_vs_1_percent']:+.1f} % | "
            f"{row['sparse_gap_100_vs_1_percent']:+.1f} % | "
            f"{row['sparse_gap_100_vs_8_percent']:+.1f} % |"
        )
    lines.append("")
    series = document.get("interleaved")
    if series:
        orders = " ; ".join(
            f"tour {measured['round']} : " + ", ".join(measured["order"])
            for measured in series["rounds"]
        )
        lines.append(
            "## Série entrelacée, carré latin sur trois tours — "
            f"identiques par tour : {series['latin_square']}"
        )
        lines.append("")
        lines.append(orders)
        lines.append("")
        lines.append("| pool | tour | clairsemé 1 seg | 8 seg | 100 seg "
                     "| 8 vs 1 | 100 vs 1 | 100 vs 8 |")
        lines.append("|---|---|---|---|---|---|---|---|")
        for pool in series["pools"]:
            for row in pool["rounds"]:
                p50 = row["sparse_p50_us"]
                lines.append(
                    f"| {pool['label']} n={pool['n']} | {row['round']} | "
                    f"{p50['synth-large']:.0f} | {p50['synth-large-8seg']:.0f} | "
                    f"{p50['synth-large-multi']:.0f} | "
                    f"{row['sparse_gap_8_vs_1_percent']:+.1f} % | "
                    f"{row['sparse_gap_100_vs_1_percent']:+.1f} % | "
                    f"{row['sparse_gap_100_vs_8_percent']:+.1f} % |"
                )
            median = pool["median"]
            amplitude = pool["range_points"]
            minimum = pool["minimum"]
            lines.append(
                f"| {pool['label']} n={pool['n']} | médiane | — | — | — | "
                f"{median['sparse_gap_8_vs_1_percent']:+.1f} % "
                f"(min {minimum['sparse_gap_8_vs_1_percent']:+.1f}, "
                f"amplitude {amplitude['sparse_gap_8_vs_1_percent']:.1f} pts) | "
                f"{median['sparse_gap_100_vs_1_percent']:+.1f} % "
                f"(min {minimum['sparse_gap_100_vs_1_percent']:+.1f}, "
                f"amplitude {amplitude['sparse_gap_100_vs_1_percent']:.1f} pts) | "
                f"{median['sparse_gap_100_vs_8_percent']:+.1f} % "
                f"(min {minimum['sparse_gap_100_vs_8_percent']:+.1f}, "
                f"amplitude {amplitude['sparse_gap_100_vs_8_percent']:.1f} pts) |"
            )
        lines.append("")
        lines.append("Témoin : le parcours dense, qui ne dépend pas du découpage.")
        lines.append("")
        lines.append("| pool | 8 vs 1 (médiane) | 100 vs 1 (médiane) | 100 vs 8 (médiane) |")
        lines.append("|---|---|---|---|")
        for pool in series["pools"]:
            median = pool["median"]
            lines.append(
                f"| {pool['label']} n={pool['n']} | "
                f"{median['dense_gap_8_vs_1_percent']:+.1f} % | "
                f"{median['dense_gap_100_vs_1_percent']:+.1f} % | "
                f"{median['dense_gap_100_vs_8_percent']:+.1f} % |"
            )
        lines.append("")
    if document["drift"]:
        lines.append("## Dérive de la machine, deux passes de la copie à 1 segment (%)")
        lines.append("")
        lines.append("| pool | N | dense | clairsemé |")
        lines.append("|---|---|---|---|")
        for row in document["drift"]:
            lines.append(
                f"| {row['label']} | {row['n']} | {row['dense_percent']:+.1f} | "
                f"{row['sparse_percent']:+.1f} |"
            )
        lines.append("")
    if document["budget"]:
        lines.append("## Budget de `docs/03-recherche.md:143` (clairsemé, N = 5000)")
        lines.append("")
        lines.append("| copie | segments | pool complet p50 / p95 (ms) | dans 20–80 ms "
                     "| pool de 1 M p50 / p95 (ms) | dans quelques ms |")
        lines.append("|---|---|---|---|---|---|")
        for line in document["budget"]:
            full = line["full_pool"]
            window = line["one_m_pool"]
            full_text = (
                "—"
                if full is None
                else f"{full['sparse_p50_ms']:.1f} / {full['sparse_p95_ms']:.1f}"
            )
            full_flag = "—" if full is None else str(full["within_20_80_ms"])
            window_text = (
                "—"
                if window is None
                else f"{window['sparse_p50_ms']:.1f} / {window['sparse_p95_ms']:.1f}"
            )
            window_flag = "—" if window is None else str(window["within_a_few_ms"])
            lines.append(
                f"| {line['campaign']} | {line['segments']} | {full_text} | {full_flag} | "
                f"{window_text} | {window_flag} |"
            )
        lines.append("")
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
