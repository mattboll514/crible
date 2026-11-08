# SPDX-License-Identifier: Apache-2.0
"""Pure helpers of pass B of the Mac mini campaign (`MESURER-SUR-MAC-MINI`).

Pass B measures the latency of the restored instance on the Mac mini
(ADR-0141, decision 4) and the complementary reading on the 1 000 titles of
MES-00. Everything here runs without Docker, without an index and without the
Legifacile package: it builds the input games of the 1 000 titles, reads the
raw `crible eval` reports, groups their percentiles, recomputes the recall of
the textual dense pass, and compares two top-10 dumps.

The corpus-to-index map follows `docs/legifacile/index-legifacile.md`: a corpus
maps to one index, and the two article corpora carry the state filter of the
`droit_en_vigueur` and `droit_historique` perimeters. The `juri_*` corpora share
the `decisions` index without a filter, exactly as the reference game carries
them (`docs/legifacile/reference/decisions.jsonl`).
"""

from __future__ import annotations

import importlib.util
import json
import math
import sys
from pathlib import Path
from typing import Any, Iterable, Mapping, Sequence

#: States that make up `droit_en_vigueur` (ADR-0137, `index-legifacile.md`).
VIGUEUR_STATES = ("VIGUEUR", "VIGUEUR_DIFF", "VIGUEUR_ETEN", "VIGUEUR_NON_ETEN")

#: Corpus of `data/mes00/queries.json` to the index that serves it.
CORPUS_INDEX: dict[str, str] = {
    "textes_vigueur": "articles",
    "textes_hors_vigueur": "articles",
    "juri_texte_integral": "decisions",
    "juri_sommaires": "decisions",
    "jorf_normatif": "jorf",
    "titres_textes": "titres",
    "sarde": "sarde",
    "dole": "dole",
}

#: Filter of the two article corpora; the other corpora carry none.
CORPUS_FILTER: dict[str, dict[str, Any]] = {
    "textes_vigueur": {"etat": {"in": list(VIGUEUR_STATES)}},
    "textes_hors_vigueur": {
        "$or": [
            {"etat": {"nin": list(VIGUEUR_STATES)}},
            {"etat": {"exists": False}},
        ]
    },
}

#: The five `crible eval` passes of one index, and their mode.
PASSES = (
    "dense-texte",
    "dense-vecteur",
    "hybride",
    "dense-texte-froid",
    "mes00-1000-titres",
)

PASS_MODE = {
    "dense-texte": "dense",
    "dense-vecteur": "dense",
    "hybride": "hybrid",
    "dense-texte-froid": "dense",
    "mes00-1000-titres": "dense",
}

#: Stages of `debug.timings_ms` that the report carries (ADR-0119).
STAGES = ("validate", "filter", "encode", "dense", "lexical", "fusion", "group", "fetch", "total")

#: The populations the constraint names (`docs/suivi.md`), with their sizes.
POPULATION_SIZES = {"global": 56, "juri_texte_integral": 31, "textes_vigueur": 12}


def _load_compare():
    """Loads the pure helpers of `p10-compare` under a private module name."""

    sibling = Path(__file__).resolve().parents[1] / "p10-compare"
    if str(sibling) not in sys.path:
        sys.path.insert(0, str(sibling))
    name = "p10_compare_pass_b"
    if name in sys.modules:
        return sys.modules[name]
    spec = importlib.util.spec_from_file_location(name, sibling / "pass_b.py")
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {sibling / 'pass_b.py'}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


class MacMiniError(Exception):
    """An input or a report broke the contract of pass B."""


def populations() -> dict[str, list[str]]:
    """The three populations of the constraint, from the base question map."""

    compare = _load_compare()
    base = compare.BASE_CORPUS
    satisfiable = sorted(
        identifier for identifier in base if identifier != compare.UNSATISFIABLE
    )
    result = {
        "global": satisfiable,
        "juri_texte_integral": sorted(
            identifier for identifier, corpus in base.items()
            if corpus == "juri_texte_integral"
            and identifier != compare.UNSATISFIABLE
        ),
        "textes_vigueur": sorted(
            identifier for identifier, corpus in base.items()
            if corpus == "textes_vigueur"
            and identifier != compare.UNSATISFIABLE
        ),
    }
    for name, expected in POPULATION_SIZES.items():
        if len(result[name]) != expected:
            raise MacMiniError(
                f"population {name} holds {len(result[name])} queries, not {expected}"
            )
    return result


# ---------------------------------------------------------------------------
# Input games
# ---------------------------------------------------------------------------


def titles_game(rows: Sequence[Mapping[str, Any]]) -> dict[str, list[dict[str, Any]]]:
    """Splits the 1 000 MES-00 titles into one game per index.

    Each row keeps its `query_id`, its text, and the corpus filter when the
    corpus carries one. A corpus without an index, an empty text or a repeated
    identifier stops the builder.
    """

    games: dict[str, list[dict[str, Any]]] = {}
    seen: set[str] = set()
    for row in rows:
        identifier = row.get("query_id")
        corpus = row.get("corpus")
        text = row.get("text")
        if not isinstance(identifier, str) or not identifier:
            raise MacMiniError("a MES-00 row carries no query_id")
        if identifier in seen:
            raise MacMiniError(f"{identifier}: repeated MES-00 identifier")
        seen.add(identifier)
        index = CORPUS_INDEX.get(corpus)
        if index is None:
            raise MacMiniError(f"{identifier}: corpus {corpus!r} has no index")
        if not isinstance(text, str) or not text.strip():
            raise MacMiniError(f"{identifier}: empty title")
        entry: dict[str, Any] = {"id": identifier, "query": text}
        filt = CORPUS_FILTER.get(corpus)
        if filt is not None:
            entry["filter"] = filt
        games.setdefault(index, []).append(entry)
    return games


def search_vectors(
    rows: Sequence[Mapping[str, Any]],
    vectors: Mapping[str, Sequence[float]],
) -> list[dict[str, Any]]:
    """One descriptor per dense primary query: id, vector file name, filter."""

    descriptors: list[dict[str, Any]] = []
    for row in rows:
        identifier = row["id"]
        vector = vectors.get(identifier)
        if not vector:
            raise MacMiniError(f"{identifier}: no query vector")
        descriptor: dict[str, Any] = {"id": identifier, "vector": f"{identifier}.json"}
        if "filter" in row:
            descriptor["filter"] = row["filter"]
        descriptors.append(descriptor)
    return descriptors


# ---------------------------------------------------------------------------
# Percentiles
# ---------------------------------------------------------------------------


def percentile(samples: Sequence[float], percent: float) -> float | None:
    """Nearest-rank percentile on raw durations (ADR-0077, decision 4)."""

    if not samples:
        return None
    ordered = sorted(samples)
    rank = max(1, math.ceil(percent / 100.0 * len(ordered)))
    return ordered[min(rank, len(ordered)) - 1]


def stage_percentiles(samples: Iterable[float]) -> dict[str, float]:
    """p50, p95 and p99 of one stage, rounded like the reports do."""

    values = list(samples)
    return {
        "p50": round(percentile(values, 50.0), 3),
        "p95": round(percentile(values, 95.0), 3),
        "p99": round(percentile(values, 99.0), 3),
    }


# ---------------------------------------------------------------------------
# Reading one eval report
# ---------------------------------------------------------------------------


def mode_of(report: Mapping[str, Any], pass_name: str) -> Mapping[str, Any]:
    """The single mode of a pass report, or an explicit error."""

    modes = report.get("modes")
    if not isinstance(modes, Mapping) or len(modes) != 1:
        raise MacMiniError(f"{pass_name}: the report does not carry exactly one mode")
    return next(iter(modes.values()))


def summarise_report(report: Mapping[str, Any], pass_name: str) -> dict[str, Any]:
    """Latency, encoding share and sample count of one report."""

    mode = mode_of(report, pass_name)
    latency = mode.get("latency")
    if not isinstance(latency, Mapping):
        raise MacMiniError(f"{pass_name}: the report carries no latency")
    stages = {
        stage: dict(latency[stage])
        for stage in STAGES
        if isinstance(latency.get(stage), Mapping)
    }
    queries = int(report.get("queries") or 0)
    repeat = int(report.get("repeat") or 0)
    total = stages.get("total")
    encode = stages.get("encode")
    share = None
    if total and encode and total["p95"]:
        share = round(encode["p95"] / total["p95"], 4)
    return {
        "index": report.get("index"),
        "generation": report.get("generation"),
        "queries": queries,
        "repeat": repeat,
        "samples": queries * repeat,
        "exact_threshold": report.get("exact_threshold"),
        "rescore_candidates": report.get("rescore_candidates"),
        "mode": next(iter(report["modes"])),
        "stages": stages,
        "total": total,
        "encode_share_at_p95": share,
    }


def grouped_percentiles(summaries: Sequence[Mapping[str, Any]]) -> dict[str, Any]:
    """The grouped figure across every index, or the prudent bound.

    `crible eval` aggregates its percentiles in process (ADR-0125, decision 9)
    and its report carries no per-query sample, so the samples of the whole
    population cannot be rebuilt from the reports. When they are absent, the
    grouped value is not computable: the result says so and gives the largest
    per-index p95 as an explicit upper bound.
    """

    if not summaries:
        raise MacMiniError("no summary to group")
    samples = sum(int(summary["samples"]) for summary in summaries)
    totals = [summary["total"] for summary in summaries if summary.get("total")]
    if not totals:
        raise MacMiniError("no summary carries a total")
    worst = max(totals, key=lambda stage: stage["p95"])
    return {
        "samples": samples,
        "available": False,
        "reason": (
            "les rapports `eval` portent des percentiles déjà agrégés et aucun "
            "échantillon par requête (ADR-0125, décision 9)"
        ),
        "p50_range": [min(t["p50"] for t in totals), max(t["p50"] for t in totals)],
        "p95_bound_max_index": worst["p95"],
        "p99_range": [min(t["p99"] for t in totals), max(t["p99"] for t in totals)],
        "per_index_p95": {
            summary["index"]: summary["total"]["p95"]
            for summary in summaries
            if summary.get("total")
        },
    }


# ---------------------------------------------------------------------------
# Recall
# ---------------------------------------------------------------------------


def recall_by_population(
    reports: Mapping[str, Mapping[str, Any]],
    names: Mapping[str, Sequence[str]],
) -> dict[str, Any]:
    """Mean recall@k of the textual dense pass, per population.

    `reports` maps an index to its report; every identifier of a population is
    read in the report of its index, and a missing row is an error rather than
    a silent zero.
    """

    rows: dict[str, float] = {}
    for index, report in reports.items():
        mode = next(iter(report["modes"].values()))
        for row in mode.get("queries") or []:
            identifier = row.get("id")
            value = row.get("quality_recall_at_k")
            if isinstance(identifier, str) and value is not None:
                rows[identifier] = float(value)
    result: dict[str, Any] = {}
    for name, identifiers in names.items():
        missing = [identifier for identifier in identifiers if identifier not in rows]
        if missing:
            raise MacMiniError(f"{name}: {len(missing)} queries absent from the reports")
        values = [rows[identifier] for identifier in identifiers]
        result[name] = {
            "n": len(values),
            "recall_at_10": round(sum(values) / len(values), 6),
        }
    return result


# ---------------------------------------------------------------------------
# Top-10 comparison
# ---------------------------------------------------------------------------


def doc_ids(payload: Mapping[str, Any]) -> list[str]:
    """The `doc_id` of the top-10 of one `crible search` answer, best first."""

    results = payload.get("results")
    if not isinstance(results, list):
        raise MacMiniError("the search answer carries no results array")
    identifiers: list[str] = []
    for entry in results:
        if not isinstance(entry, Mapping) or not isinstance(entry.get("doc_id"), str):
            raise MacMiniError("a search result carries no doc_id")
        identifiers.append(entry["doc_id"])
    return identifiers


def compare_top10(
    reference: Mapping[str, Sequence[str]],
    candidate: Mapping[str, Sequence[str]],
) -> dict[str, Any]:
    """Compares two dumps of top-10 lists, query by query."""

    common = sorted(set(reference) & set(candidate))
    if not common:
        raise MacMiniError("the two dumps share no query")
    differences = [identifier for identifier in common if list(reference[identifier]) != list(candidate[identifier])]
    missing = sorted(set(reference) - set(candidate))
    extra = sorted(set(candidate) - set(reference))
    return {
        "compared": len(common),
        "identical": len(common) - len(differences),
        "differing": differences,
        "missing": missing,
        "extra": extra,
    }


def compare_eval_queries(
    reference: Mapping[str, Mapping[str, Any]],
    candidate: Mapping[str, Mapping[str, Any]],
) -> dict[str, Any]:
    """Compares the per-query controls of two `crible eval` dense reports.

    The versioned eval reports carry no document identifier: they only carry
    the fidelity and quality figures of each query. This comparison therefore
    checks those figures, not the top-10 lists themselves.
    """

    fidelity_fields = ("fidelity_overlap_at_10", "fidelity_recall_at_10")
    quality_fields = ("quality_found", "quality_mrr_at_k", "quality_recall_at_k")
    common = sorted(set(reference) & set(candidate))
    if not common:
        raise MacMiniError("the two eval reports share no query")
    fidelity_differing = [
        identifier
        for identifier in common
        if any(reference[identifier].get(field) != candidate[identifier].get(field) for field in fidelity_fields)
    ]
    quality_differing = [
        identifier
        for identifier in common
        if any(reference[identifier].get(field) != candidate[identifier].get(field) for field in quality_fields)
    ]
    return {
        "compared": len(common),
        "fidelity_identical": len(common) - len(fidelity_differing),
        "fidelity_differing": fidelity_differing,
        "quality_identical": len(common) - len(quality_differing),
        "quality_differing": quality_differing,
        "missing": sorted(set(reference) - set(candidate)),
        "extra": sorted(set(candidate) - set(reference)),
    }


def read_json(path: str | Path) -> Any:
    """Reads one JSON document, or raises `MacMiniError`."""

    try:
        return json.loads(Path(path).read_text())
    except (OSError, ValueError) as exc:
        raise MacMiniError(f"cannot read {path}: {exc}") from exc


def write_json(path: str | Path, document: Any) -> None:
    """Writes one pretty JSON document, newline terminated."""

    target = Path(path)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n")
