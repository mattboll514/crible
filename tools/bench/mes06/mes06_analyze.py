# SPDX-License-Identifier: Apache-2.0
"""Aggregation of MES-06: the lexical branch (size, build time, filtered BM25).

It reads the raw documents the harness wrote
(`crates/crible/examples/lexical_latency.rs`, `data/mes06/raw/`), recomputes
every percentile from the per-query durations, checks the figures against the
ones the harness recorded, and writes the dated artefact next to the report.
It prints the Markdown tables of the report.

Protocol: ADR-0091.

- decisions 1 and 2: size and build time are measured on the Legifacile sample
  alone, per structure, then extrapolated to 8 580 000 passages linearly in
  analysed tokens and given as a range; `lexical.positions` `false` is the main
  campaign, `true` the secondary series.
- decision 3: a real campaign on the sample (pools of about 1 000, 10 000 and
  50 000 passages, the real size written next to the round figure; the three
  hundred queries of `queries.jsonl`, each on the index of its structure;
  `k = 10` main, `k = 100` secondary; warm-up then three passes), and a plumbing
  campaign on `synth-large` whose points are labelled a synthetic worst case.
- decision 4: one alert threshold, by subtraction from the 698 ms budget. Every
  term is cited from its report, so the remainder belongs to the date of this
  campaign only.
- decision 5: peak RSS of the commit, segment count, judgement on the
  `IndexWriter` budget, rebuild extrapolated against the 60 s of ADR-0047, and
  the block-WAND finding.

Percentiles are the nearest-rank definition of ADR-0077 and ADR-0091: the value
of rank `ceil(ratio × n)`, ranked from 1, without interpolation.

Run from the repository root::

    (cd tools && uv run --no-sync python bench/mes06/mes06_analyze.py \
        --commit "$(git rev-parse HEAD)" --date 2026-10-28 \
        --machine "MacBook Air M3, 8 cores, 24 GB, macOS 15.7.7, native")
"""

from __future__ import annotations

import argparse
import json
import math
import re
from pathlib import Path

#: Root of the repository: a path recorded by the harness is made relative to
#: it, so a versioned artefact never carries a local absolute path.
ROOT = Path(__file__).resolve().parents[3]
#: Raw documents of the campaign.
DEFAULT_RAW = ROOT / "data" / "mes06" / "raw"
#: Where the dated artefact goes.
DEFAULT_MEASURES = ROOT / "docs" / "mesures"
#: File name of the report and of its artefact (ADR-0091, decision 6).
SLUG = "MES-06-lexical-bm25-filtre"

#: The six structures of `data/legifacile-sample/`, named as their index is.
STRUCTURES = ("articles", "decisions", "jorf", "sarde", "dole", "titres")

#: Target of the extrapolation: the 8,58 M passages of Legifacile.
TARGET_PASSAGES = 8_580_000

#: End-to-end p95 budget of `docs/suivi.md`, "Cibles" (`:31`), in milliseconds.
BUDGET_MS = 698.0

#: The terms of the subtraction of ADR-0091, decision 4, each with its source.
#: They are **cited**, never re-measured here: the remainder is valid for the
#: reports of this date only, and a new measurement of any term moves it.
BUDGET_TERMS = (
    {
        "term": "encodage fp32 p95",
        "ms": 55.763,
        "source": "docs/mesures/MES-05-latence-encodage.md:85 (passe 1)",
    },
    {
        "term": "filtre bitmap `keyword`, p95",
        "ms": 0.0522,
        "source": (
            "docs/mesures/MES-14-cout-filtres-intervalles.md:182 "
            "(`partie=corps`, p95 52,2 µs)"
        ),
    },
    {
        "term": "scan binaire p95, pool entier",
        "ms": 44.132,
        "source": "docs/mesures/MES-02-latence-scan-binaire.md (pool entier)",
    },
    {
        "term": "rescoring p95 livré, N = 5000",
        "ms": 31.24,
        "source": (
            "docs/mesures/MES-03-latence-rescoring.md:1418 "
            "(groupe, N = 5000 ; 20,03 à 31,24 ms)"
        ),
    },
    {
        "term": "payload, groupement et sérialisation",
        "ms": 1.0,
        "source": "docs/03-recherche.md:155 (ordre de grandeur)",
    },
)

#: The sensitivity line of ADR-0091, decision 4: the worst interval filter of
#: MES-14, kept next to the main term so the report can say whether the verdict
#: would change. It never enters the main remainder.
BUDGET_SENSITIVITY = {
    "term": "filtre d'intervalle `integer`, p95 (sensibilité)",
    "ms": 216.9,
    "source": (
        "docs/mesures/MES-14-cout-filtres-intervalles.md:183 "
        "(`annee 2000..2010`, domaine plein, p95 216,9 ms)"
    ),
}


def nearest_rank(values: list[float], ratio: float) -> float:
    """Returns the nearest-rank percentile of a series (ADR-0077).

    Rank is `ceil(ratio × n)`, clamped to `[1, n]`, and the result is the value
    at that rank in the sorted series: no interpolation between two samples.
    """
    if not values:
        raise ValueError("nearest_rank of an empty series")
    if not 0.0 < ratio <= 1.0:
        raise ValueError(f"ratio out of (0, 1]: {ratio}")
    ordered = sorted(values)
    rank = math.ceil(ratio * len(ordered))
    return ordered[min(max(rank, 1), len(ordered)) - 1]


def summarise(values: list[float]) -> dict:
    """Count, p50, p95, p99, mean, min and max of a series, in microseconds."""
    if not values:
        raise ValueError("summarise of an empty series")
    return {
        "n": len(values),
        "p50": nearest_rank(values, 0.50),
        "p95": nearest_rank(values, 0.95),
        "p99": nearest_rank(values, 0.99),
        "mean": sum(values) / len(values),
        "min": min(values),
        "max": max(values),
    }


def load_json(path: Path) -> dict:
    """Reads one raw document of the campaign."""
    with path.open(encoding="utf-8") as stream:
        return json.load(stream)


def check_summary(recorded: dict, series: list[list[float]]) -> None:
    """Recomputes a recorded summary from its passes and refuses a drift.

    The harness prints what it measures; this check makes the aggregation the
    authority on the percentiles, and a disagreement stops the run instead of
    publishing two versions of the same figure.
    """
    values = [value for one_pass in series for value in one_pass]
    expected = summarise(values)
    for key in ("n", "p50", "p95", "p99"):
        if key not in recorded:
            continue
        if not math.isclose(recorded[key], expected[key], rel_tol=0.0, abs_tol=1e-9):
            raise ValueError(
                f"recorded {key} = {recorded[key]!r} disagrees with the recomputed "
                f"{expected[key]!r} over n = {expected['n']}"
            )


def size_rows(raw: Path) -> list[dict]:
    """One row per structure and per value of `lexical.positions`."""
    rows = []
    for structure in STRUCTURES:
        for positions in (False, True):
            path = raw / f"size-{structure}-{'true' if positions else 'false'}.json"
            document = load_json(path)
            rows.append(
                {
                    "structure": structure,
                    "positions": positions,
                    "live_passages": document["live_passages"],
                    "dead_passages": document["dead_passages"],
                    "bytes": document["bytes"],
                    "bytes_per_passage": document["bytes_per_passage"],
                    "segments": document["segments"],
                    "files": document["files"],
                    "bytes_by_extension": document["bytes_by_extension"],
                }
            )
    return rows


def token_rows(raw: Path) -> list[dict]:
    """One row per structure: analysed tokens and characters of the sample."""
    rows = []
    for structure in STRUCTURES:
        document = load_json(raw / f"tokens-{structure}-false.json")
        rows.append(
            {
                "structure": structure,
                "passages": document["passages"],
                "tokens": document["tokens"],
                "tokens_per_passage": document["tokens_per_passage"],
                "characters": document["characters"],
                "characters_per_passage": document["characters_per_passage"],
                "distinct_terms": document["distinct_terms"],
                "longest_passage_tokens": document["longest_passage"],
                "shortest_passage_tokens": document["shortest_passage"],
            }
        )
    return rows


def weighted_mean(rows: list[dict], field: str, weight: str) -> float:
    """Mean of `field` weighted by `weight`, both read from `rows`.

    The campaign has six structures of different sizes in passages, so a plain
    mean of per-passage rates would weigh a 7 628-passage structure as much as a
    50 000-passage one.
    """
    total_weight = sum(row[weight] for row in rows)
    if total_weight == 0:
        raise ValueError(f"weighted_mean of {field} with a zero total weight")
    return sum(row[field] * row[weight] for row in rows) / total_weight


def extrapolate(
    rates: list[float],
    central_rate: float | None = None,
    target_passages: int = TARGET_PASSAGES,
) -> dict:
    """Extrapolates a per-passage rate to 8,58 M passages, as a range.

    ADR-0091, decision 1, asks for the lowest and the highest rate observed per
    structure, multiplied by 8,58 M passages: the composition of production by
    structure is unknown, so a single figure would read as a measurement. The
    `central` value is the rate of the whole sample (total over total), passed
    as `central_rate`: it *is* the passage-ratio control the decision asks for as
    a second check, since 8,58 M / 257 626 is the multiplier of that control.
    """
    if not rates:
        raise ValueError("extrapolate of an empty list of rates")
    low = min(rates)
    high = max(rates)
    central = sum(rates) / len(rates) if central_rate is None else central_rate
    return {
        "target_passages": target_passages,
        "low": low * target_passages,
        "central": central * target_passages,
        "high": high * target_passages,
        "ratio": high / low if low else None,
    }


def latency_rows(raw: Path) -> list[dict]:
    """Every measured point of the sample campaign, checked against its passes."""
    rows = []
    for structure in STRUCTURES:
        for positions, suffix in ((False, "false"), (True, "true")):
            path = raw / f"latency-{structure}-{suffix}.json"
            if not path.exists():
                continue
            document = load_json(path)
            for measured in document["measured"]:
                check_summary(measured["summary"], measured["passes"])
                values = [value for one_pass in measured["passes"] for value in one_pass]
                rows.append(
                    {
                        "structure": structure,
                        "positions": positions,
                        "pool": measured["pool"],
                        "pool_size": measured["pool_size"],
                        "pool_millis": measured["pool_millis"],
                        "k": measured["k"],
                        "queries": measured["queries"],
                        "summary": summarise(values),
                        "results": measured["results_in_first_pass"],
                    }
                )
    return rows


def latency_pooled(raw: Path) -> list[dict]:
    """Pool points gathered across structures, from the raw passes."""
    groups: dict[tuple, list[float]] = {}
    counts: dict[tuple, int] = {}
    for structure in STRUCTURES:
        path = raw / f"latency-{structure}-false.json"
        if not path.exists():
            continue
        document = load_json(path)
        for measured in document["measured"]:
            key = (measured["pool"], measured["k"])
            values = [value for one_pass in measured["passes"] for value in one_pass]
            groups.setdefault(key, []).extend(values)
            counts[key] = counts.get(key, 0) + 1
    return [
        {
            "pool": pool,
            "k": k,
            "structures": counts[(pool, k)],
            "summary": summarise(values),
        }
        for (pool, k), values in sorted(groups.items())
    ]


def synth_rows(raw: Path) -> dict:
    """The plumbing campaign on `synth-large`, warm and cold."""
    warm = load_json(raw / "latency-synth-large-false.json")
    cold = load_json(raw / "latency-synth-large-cold.json")
    warm_rows = []
    for measured in warm["measured"]:
        check_summary(measured["summary"], measured["passes"])
        values = [value for one_pass in measured["passes"] for value in one_pass]
        warm_rows.append(
            {
                "pool": measured["pool"],
                "pool_size": measured["pool_size"],
                "k": measured["k"],
                "summary": summarise(values),
                "results": measured["results_in_first_pass"],
            }
        )
    cold_measured = cold["measured"][0]
    check_summary(cold_measured["summary"], cold_measured["passes"])
    return {
        "warm": warm_rows,
        "cold": {
            "pool": cold_measured["pool"],
            "pool_size": cold_measured["pool_size"],
            "k": cold_measured["k"],
            "warmup": cold["warmup"],
            "summary": summarise(
                [value for one_pass in cold_measured["passes"] for value in one_pass]
            ),
        },
    }


#: `/usr/bin/time -l` writes `154.28 real ...` then `2543271936  maximum
#: resident set size`. The attach logs of the campaign keep both, so the
#: optional figure of ADR-0091 decision 5 stays in the versioned artefact.
ATTACH_SECONDS = re.compile(r"^\s*([0-9.]+) real\b", re.MULTILINE)
ATTACH_RSS = re.compile(r"^\s*([0-9]+)\s+maximum resident set size\b", re.MULTILINE)


def attach_rows(raw: Path) -> list[dict]:
    """Wall time and maximum RSS of each `crible attach`, from its log."""
    rows = []
    for structure in STRUCTURES:
        text = (raw / f"attach-{structure}.txt").read_text(encoding="utf-8")
        seconds = ATTACH_SECONDS.search(text)
        rss = ATTACH_RSS.search(text)
        if seconds is None or rss is None:
            raise ValueError(f"attach-{structure}.txt carries no time line")
        rows.append(
            {
                "structure": structure,
                "seconds": float(seconds.group(1)),
                "peak_rss_bytes": int(rss.group(1)),
            }
        )
    return rows


def synth_rebuild(raw: Path) -> dict:
    """The rebuild of `synth-large`: the campaign's only multi-segment point."""
    document = load_json(raw / "rebuild-synth-large-false.json")
    return {
        "index": document["index"],
        "positions": document["positions"],
        "seconds": document["seconds"],
        "peak_rss_bytes": document["peak_rss_bytes"],
        "segments": document["segments"],
        "bytes": document["bytes"],
        "files": document["files"],
    }


def rebuild_rows(raw: Path) -> list[dict]:
    """The rebuild job per structure and per value of `lexical.positions`."""
    rows = []
    for structure in STRUCTURES:
        for positions in (False, True):
            path = raw / f"rebuild-{structure}-{'true' if positions else 'false'}.json"
            document = load_json(path)
            rows.append(
                {
                    "structure": structure,
                    "positions": positions,
                    "seconds": document["seconds"],
                    "peak_rss_bytes": document["peak_rss_bytes"],
                    "segments": document["segments"],
                    "bytes": document["bytes"],
                }
            )
    return rows


def wand_rows(raw: Path, index: str) -> list[dict]:
    """Candidates visited against pool size and matching documents."""
    document = load_json(raw / f"wand-{index}.json")
    rows = []
    for measured in document["measured"]:
        queries = measured["queries"]
        if not queries:
            continue
        rows.append(
            {
                "index": index,
                "pool": measured["pool"],
                "pool_size": measured["pool_size"],
                "k": measured["k"],
                "queries": len(queries),
                "visited_min": min(query["visited"] for query in queries),
                "visited_max": max(query["visited"] for query in queries),
                "matching_min": min(query["matching_documents"] for query in queries),
                "matching_max": max(query["matching_documents"] for query in queries),
                "in_pool_max": max(query["in_pool"] for query in queries),
                "identical_order": sum(
                    1 for query in queries if query["identical_to_engine"]
                ),
                "same_key_set": sum(1 for query in queries if query["same_key_set"]),
                "same_scores": sum(1 for query in queries if query["same_scores"]),
            }
        )
    return rows


def budget(terms: tuple[dict, ...] = BUDGET_TERMS) -> dict:
    """The remainder of the 698 ms budget, term by term (ADR-0091, decision 4).

    `terms` defaults to the main subtraction. The report also calls it with the
    main terms where the `keyword` filter is replaced by the worst interval
    filter, which gives the sensitivity line (§ 7.1) without moving the verdict.
    """
    spent = sum(term["ms"] for term in terms)
    return {
        "budget_ms": BUDGET_MS,
        "terms": list(terms),
        "spent_ms": spent,
        "remaining_ms": BUDGET_MS - spent,
    }


def sensitivity() -> dict:
    """The main subtraction with its filter term replaced by the interval case."""
    replaced = tuple(
        BUDGET_SENSITIVITY if term["term"].startswith("filtre bitmap") else term
        for term in BUDGET_TERMS
    )
    main = budget()
    alternative = budget(replaced)
    return {
        "term": dict(BUDGET_SENSITIVITY),
        "spent_ms": alternative["spent_ms"],
        "remaining_ms": alternative["remaining_ms"],
    }


def display_path(path: Path) -> str:
    """A repository-relative path when possible, the absolute one otherwise.

    `--measures` may point outside the repository: the artefact is still written,
    and printing its path must not fail after the write.
    """
    try:
        return str(path.relative_to(ROOT))
    except ValueError:
        return str(path)


def verdict(p95_ms: float, remaining_ms: float) -> str:
    """Alert or not, at one measured pool size."""
    return "ALERTE" if p95_ms > remaining_ms else "sous le reste"


def alert_verdicts(
    latency: list[dict],
    synth: dict,
    remaining_ms: float,
) -> list[dict]:
    """One alert verdict per measured pool point (ADR-0091, decision 4).

    Every point of the real campaign and of the plumbing campaign is judged
    against the same remainder: the report must not pick a single figure and let
    the others go unjudged.
    """
    rows = []
    for row in latency:
        p95 = row["summary"]["p95"] / 1000.0
        rows.append(
            {
                "campaign": "echantillon",
                "index": row["structure"],
                "positions": row["positions"],
                "pool": row["pool"],
                "pool_size": row["pool_size"],
                "k": row["k"],
                "p95_ms": p95,
                "remaining_ms": remaining_ms,
                "verdict": verdict(p95, remaining_ms),
            }
        )
    for row in synth["warm"]:
        p95 = row["summary"]["p95"] / 1000.0
        rows.append(
            {
                "campaign": "synth-large (pire cas synthétique)",
                "index": "synth-large",
                "positions": False,
                "pool": row["pool"],
                "pool_size": row["pool_size"],
                "k": row["k"],
                "p95_ms": p95,
                "remaining_ms": remaining_ms,
                "verdict": verdict(p95, remaining_ms),
            }
        )
    cold = synth["cold"]
    p95 = cold["summary"]["p95"] / 1000.0
    rows.append(
        {
            "campaign": "synth-large froid (borne)",
            "index": "synth-large-cold",
            "positions": False,
            "pool": cold["pool"],
            "pool_size": cold["pool_size"],
            "k": cold["k"],
            "p95_ms": p95,
            "remaining_ms": remaining_ms,
            "verdict": verdict(p95, remaining_ms),
        }
    )
    return rows


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--raw", type=Path, default=DEFAULT_RAW)
    parser.add_argument("--measures", type=Path, default=DEFAULT_MEASURES)
    parser.add_argument("--date", default="2026-10-28")
    parser.add_argument("--commit", default="")
    parser.add_argument("--machine", default="")
    parser.add_argument("--write", action="store_true", help="write the dated artefact")
    arguments = parser.parse_args(argv)

    raw = arguments.raw
    sizes = size_rows(raw)
    tokens = token_rows(raw)
    latency = latency_rows(raw)
    pooled = latency_pooled(raw)
    synth = synth_rows(raw)
    synth["rebuild"] = synth_rebuild(raw)
    attach = attach_rows(raw)
    rebuild = rebuild_rows(raw)
    wand = {
        structure: wand_rows(raw, structure) for structure in STRUCTURES
    }
    wand["synth-large"] = wand_rows(raw, "synth-large")
    main_budget = budget()
    sensitivity_line = sensitivity()
    verdicts = alert_verdicts(latency, synth, main_budget["remaining_ms"])
    alternative = alert_verdicts(latency, synth, sensitivity_line["remaining_ms"])
    sensitivity_line["points_that_change"] = sum(
        1
        for main_point, alternative_point in zip(verdicts, alternative, strict=True)
        if main_point["verdict"] != alternative_point["verdict"]
    )
    sensitivity_line["changes_verdict"] = sensitivity_line["points_that_change"] > 0

    by_structure = {row["structure"]: row for row in tokens}
    sizes_false = [row for row in sizes if not row["positions"]]
    sizes_true = [row for row in sizes if row["positions"]]

    tokens_central = weighted_mean(tokens, "tokens_per_passage", "passages")
    bytes_false_central = weighted_mean(
        sizes_false, "bytes_per_passage", "live_passages"
    )
    bytes_true_central = weighted_mean(
        sizes_true, "bytes_per_passage", "live_passages"
    )
    tokens_extrapolation = extrapolate(
        [row["tokens_per_passage"] for row in tokens], tokens_central
    )
    bytes_false_extrapolation = extrapolate(
        [row["bytes_per_passage"] for row in sizes_false], bytes_false_central
    )
    bytes_true_extrapolation = extrapolate(
        [row["bytes_per_passage"] for row in sizes_true], bytes_true_central
    )

    total_tokens = sum(row["tokens"] for row in tokens)
    total_passages = sum(row["passages"] for row in tokens)
    total_seconds_false = sum(row["seconds"] for row in rebuild if not row["positions"])
    total_seconds_true = sum(row["seconds"] for row in rebuild if row["positions"])
    seconds_per_token_false = total_seconds_false / total_tokens

    rebuild_extrapolation = {
        "measured_seconds": total_seconds_false,
        "measured_tokens": total_tokens,
        "seconds_per_token": seconds_per_token_false,
        "low_seconds": tokens_extrapolation["low"] * seconds_per_token_false,
        "central_seconds": tokens_extrapolation["central"] * seconds_per_token_false,
        "high_seconds": tokens_extrapolation["high"] * seconds_per_token_false,
        "measured_seconds_positions": total_seconds_true,
        "target_seconds": 60.0,
    }

    print("## Size, per structure")
    for row in sizes:
        print(
            f"| {row['structure']} | {'true' if row['positions'] else 'false'} "
            f"| {row['live_passages']} | {row['bytes']} | "
            f"{row['bytes_per_passage']:.2f} | {row['segments']} |"
        )

    print("## Tokens, per structure")
    for row in tokens:
        print(
            f"| {row['structure']} | {row['passages']} | {row['tokens']} | "
            f"{row['tokens_per_passage']:.2f} | {row['characters_per_passage']:.1f} | "
            f"{row['distinct_terms']} |"
        )

    print("## Latency, per structure and pool")
    for row in latency:
        print(
            f"| {row['structure']} | {'true' if row['positions'] else 'false'} "
            f"| {row['pool']} | {row['pool_size']} | {row['k']} "
            f"| {row['summary']['p50'] / 1000:.3f} | {row['summary']['p95'] / 1000:.3f} "
            f"| {row['summary']['p99'] / 1000:.3f} | {row['summary']['n']} |"
        )

    print("## Latency, pooled across structures")
    for row in pooled:
        print(
            f"| {row['pool']} | {row['k']} | {row['summary']['n']} "
            f"| {row['summary']['p50'] / 1000:.3f} | {row['summary']['p95'] / 1000:.3f} "
            f"| {row['summary']['p99'] / 1000:.3f} |"
        )

    print("## synth-large, plumbing campaign (synthetic worst case)")
    for row in synth["warm"]:
        print(
            f"| {row['pool']} | {row['pool_size']} | {row['k']} "
            f"| {row['summary']['p50'] / 1000:.3f} | {row['summary']['p95'] / 1000:.3f} "
            f"| {row['summary']['p99'] / 1000:.3f} | {row['summary']['n']} |"
        )
    cold = synth["cold"]
    print(
        f"| froid | {cold['pool_size']} | {cold['k']} | {cold['summary']['p50'] / 1000:.3f} "
        f"| {cold['summary']['p95'] / 1000:.3f} | {cold['summary']['p99'] / 1000:.3f} "
        f"| {cold['summary']['n']} |"
    )

    print("## Rebuild")
    for row in rebuild:
        print(
            f"| {row['structure']} | {'true' if row['positions'] else 'false'} "
            f"| {row['seconds']:.3f} | {row['peak_rss_bytes'] / (1 << 20):.1f} "
            f"| {row['segments']} |"
        )

    print("## Attach")
    for row in attach:
        print(
            f"| {row['structure']} | {row['seconds']:.3f} "
            f"| {row['peak_rss_bytes'] / (1 << 20):.1f} |"
        )
    print("## Rebuild, synth-large")
    print(json.dumps(synth["rebuild"], indent=2, ensure_ascii=False))
    print("## Budget, main")
    print(json.dumps(main_budget, indent=2, ensure_ascii=False))
    print("## Budget, sensitivity")
    print(json.dumps(sensitivity_line, indent=2, ensure_ascii=False))
    print("## Verdicts")
    for row in verdicts:
        print(
            f"| {row['campaign']} | {row['index']} | {row['pool_size']} | {row['k']} "
            f"| {row['p95_ms']:.3f} | {row['verdict']} |"
        )
    print("## Extrapolation")
    print(json.dumps(tokens_extrapolation, indent=2, ensure_ascii=False))
    print(json.dumps(bytes_false_extrapolation, indent=2, ensure_ascii=False))
    print(json.dumps(bytes_true_extrapolation, indent=2, ensure_ascii=False))
    print(json.dumps(rebuild_extrapolation, indent=2, ensure_ascii=False))

    print("## Block-WAND")
    for index, rows in wand.items():
        for row in rows:
            print(
                f"| {index} | {row['pool']} | {row['pool_size']} | {row['k']} "
                f"| {row['matching_min']} | {row['matching_max']} | "
                f"{row['identical_order']}/{row['queries']} "
                f"| {row['same_key_set']}/{row['queries']} "
                f"| {row['same_scores']}/{row['queries']} |"
            )

    if not arguments.write:
        return 0

    artefact = {
        "measure": "MES-06",
        "question": (
            "Branche lexicale : taille de l'index, durée de construction et latence "
            "BM25 filtrée selon la taille du pool"
        ),
        "protocol": "ADR-0091",
        "task": "MESURER-LEXICAL",
        "date": arguments.date,
        "commit": arguments.commit,
        "machine": arguments.machine,
        "target_passages": TARGET_PASSAGES,
        "budget_ms": BUDGET_MS,
        "tokens": tokens,
        "sizes": sizes,
        "latency": latency,
        "latency_pooled": pooled,
        "synth_large": synth,
        "attach": attach,
        "rebuild": rebuild,
        "wand": wand,
        "budget": {"main": main_budget, "sensitivity": sensitivity_line},
        "verdicts": verdicts,
        "extrapolation": {
            "tokens": tokens_extrapolation,
            "bytes_positions_false": bytes_false_extrapolation,
            "bytes_positions_true": bytes_true_extrapolation,
            "rebuild": rebuild_extrapolation,
            "passage_ratio": TARGET_PASSAGES / total_passages,
            "sample_passages": total_passages,
            "sample_tokens": total_tokens,
            "tokens_weighted_per_passage": weighted_mean(
                tokens, "tokens_per_passage", "passages"
            ),
            "bytes_false_weighted_per_passage": weighted_mean(
                sizes_false, "bytes_per_passage", "live_passages"
            ),
            "bytes_true_weighted_per_passage": weighted_mean(
                sizes_true, "bytes_per_passage", "live_passages"
            ),
            "note": (
                "Extrapolation linéaire en jetons analysés, en fourchette (ADR-0091, "
                "décision 1) : le minimum et le maximum sont les taux par structure, "
                "le central le taux de l'échantillon entier. Composition de production "
                "par structure inconnue."
            ),
        },
        "sources": {
            "harness": "crates/crible/examples/lexical_latency.rs",
            "raw": "data/mes06/raw/",
            "aggregator": "tools/bench/mes06/mes06_analyze.py",
        },
    }

    destination = arguments.measures / f"{SLUG}.{arguments.date}.json"
    destination.parent.mkdir(parents=True, exist_ok=True)
    with destination.open("w", encoding="utf-8") as stream:
        json.dump(artefact, stream, indent=2, ensure_ascii=False, sort_keys=False)
        stream.write("\n")
    print(f"\nwrote {display_path(destination)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
