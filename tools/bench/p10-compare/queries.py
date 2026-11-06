"""Pure helpers for the pgvector replay of ``COMPARER-AVEC-PGVECTOR`` (P10).

Everything in this module runs without a database and without importing the
Legifacile production package: it reads the versioned reference game, maps the
MCP parameters onto the SQL parameters of ``api.recherche_semantique``, ranks
candidates the way the MCP does, and builds the publishable report (``id`` and
``doc_id`` only, ADR-0125 decision 16).

The replay itself lives in ``replay.py`` and the counting in ``counts.py``;
both run on the production host and import the production modules there.
"""

from __future__ import annotations

import json
import math
from pathlib import Path
from typing import Any, Callable, Iterable, Mapping, Sequence

SCHEMA_VERSION = 1

#: The six reference files, one per Crible index (``docs/legifacile/jeu-de-reference.md``).
INDEXES = ("articles", "decisions", "dole", "jorf", "sarde", "titres")

#: Prefix of the 24 lines taken from the MES-00 fidelity set.
MES00_PREFIX = "MES00-"

#: The only filter parameters ``mcp.jsonl`` may carry (ADR-0138).
PARAM_KEYS = frozenset(
    {
        "k",
        "dedupliquer",
        "partie_exclue",
        "fonds",
        "cle_domaine",
        "annee_min",
        "annee_max",
    }
)

#: MCP perimeter -> pgvector corpora. Copied from ``serveur_mcp/perimetres.py``
#: only so that this module stays testable without the production package; the
#: replay asserts its value against the production table before measuring.
PERIMETERS: dict[str, tuple[str, ...]] = {
    "droit_applicable": ("textes_vigueur", "textes_hors_vigueur"),
    "droit_en_vigueur": ("textes_vigueur",),
    "droit_historique": ("textes_hors_vigueur",),
    "jurisprudence": ("juri_sommaires", "juri_texte_integral"),
    "journal_officiel": ("jorf_normatif",),
    "index_thematique": ("sarde",),
    "dossiers_legislatifs": ("dole",),
    "titres": ("titres_textes",),
}

#: Perimeter whose fusion filters on applicability dates, after the fusion.
#: ``droit_applicable`` is the only member of ``perimetres.FILTRE_APPLICABLE``.
FILTER_APPLICABLE = frozenset({"droit_applicable"})

#: Crible index -> pgvector corpora, for the count comparison of ADR-0140
#: decision 10. ``articles`` and ``decisions`` merge two partitions each.
INDEX_CORPORA: dict[str, tuple[str, ...]] = {
    "articles": ("textes_vigueur", "textes_hors_vigueur"),
    "decisions": ("juri_sommaires", "juri_texte_integral"),
    "dole": ("dole",),
    "jorf": ("jorf_normatif",),
    "sarde": ("sarde",),
    "titres": ("titres_textes",),
}


class P10Error(Exception):
    """An input contract or a measurement guard failed."""


# ---------------------------------------------------------------------------
# Reading the versioned reference game
# ---------------------------------------------------------------------------


def read_json(path: str | Path) -> Any:
    try:
        with open(path, "r", encoding="utf-8") as handle:
            return json.load(handle)
    except OSError as exc:
        raise P10Error(f"cannot read {Path(path).name}: {exc}") from exc
    except json.JSONDecodeError as exc:
        raise P10Error(f"invalid JSON in {Path(path).name}: {exc}") from exc


def read_jsonl(path: str | Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    try:
        with open(path, "r", encoding="utf-8") as handle:
            for number, line in enumerate(handle, start=1):
                text = line.strip()
                if not text:
                    continue
                try:
                    value = json.loads(text)
                except json.JSONDecodeError as exc:
                    raise P10Error(
                        f"{Path(path).name}:{number}: invalid JSON: {exc}"
                    ) from exc
                if not isinstance(value, dict):
                    raise P10Error(f"{Path(path).name}:{number}: not an object")
                rows.append(value)
    except OSError as exc:
        raise P10Error(f"cannot read {Path(path).name}: {exc}") from exc
    if not rows:
        raise P10Error(f"{Path(path).name} is empty")
    return rows


def load_index_queries(
    reference_dir: str | Path,
) -> dict[str, dict[str, str]]:
    """Return ``id -> {text, index}`` for the six ``<index>.jsonl`` files."""

    directory = Path(reference_dir)
    queries: dict[str, dict[str, str]] = {}
    for index in INDEXES:
        path = directory / f"{index}.jsonl"
        for row in read_jsonl(path):
            identifier = row.get("id")
            text = row.get("query")
            if not isinstance(identifier, str) or not identifier:
                raise P10Error(f"{path.name}: a row has no id")
            if not isinstance(text, str) or not text.strip():
                raise P10Error(f"{path.name}: {identifier} has no query text")
            if identifier in queries:
                raise P10Error(f"duplicate query id: {identifier}")
            queries[identifier] = {"text": text, "index": index}
    return queries


def load_mcp(path: str | Path) -> list[dict[str, Any]]:
    """Return the 97 MCP calls, validated against ADR-0138's closed shape."""

    rows = read_jsonl(path)
    seen: set[str] = set()
    calls: list[dict[str, Any]] = []
    for row in rows:
        keys = set(row)
        if keys != {"id", "tool", "perimetre", "params"}:
            raise P10Error(f"mcp.jsonl: unexpected keys {sorted(keys)}")
        identifier = row["id"]
        if not isinstance(identifier, str) or not identifier:
            raise P10Error("mcp.jsonl: a row has no id")
        if identifier in seen:
            raise P10Error(f"mcp.jsonl: duplicate id {identifier}")
        seen.add(identifier)
        if row["tool"] != "recherche_semantique":
            raise P10Error(f"mcp.jsonl: {identifier} is not a semantic call")
        perimetre = row["perimetre"]
        if perimetre not in PERIMETERS:
            raise P10Error(f"mcp.jsonl: {identifier} has an unknown perimeter")
        params = row["params"]
        if not isinstance(params, dict):
            raise P10Error(f"mcp.jsonl: {identifier} has no params object")
        unknown = set(params) - PARAM_KEYS
        if unknown:
            raise P10Error(f"mcp.jsonl: {identifier} has unknown params {sorted(unknown)}")
        calls.append(
            {"id": identifier, "perimetre": perimetre, "params": dict(params)}
        )
    return calls


def load_mes00(path: str | Path) -> dict[str, dict[str, str]]:
    """Return ``qNNNN -> {text, corpus}`` for the MES-00 query file."""

    payload = read_json(path)
    if not isinstance(payload, Mapping) or not isinstance(payload.get("queries"), list):
        raise P10Error("the MES-00 query file has no 'queries' array")
    queries: dict[str, dict[str, str]] = {}
    for row in payload["queries"]:
        if not isinstance(row, Mapping):
            raise P10Error("the MES-00 query file has a non-object row")
        identifier = row.get("query_id")
        text = row.get("text")
        corpus = row.get("corpus")
        if not isinstance(identifier, str) or not identifier:
            raise P10Error("the MES-00 query file has a row without query_id")
        if not isinstance(text, str) or not text.strip():
            raise P10Error(f"the MES-00 query file has no text for {identifier}")
        if not isinstance(corpus, str) or not corpus:
            raise P10Error(f"the MES-00 query file has no corpus for {identifier}")
        queries[identifier] = {"text": text, "corpus": corpus}
    return queries


def build_queries(
    reference_dir: str | Path,
    mcp_path: str | Path,
    mes00_path: str | Path,
) -> list[dict[str, Any]]:
    """Join the 97 MCP calls with their texts.

    Each line gets exactly one of two text sources: the versioned reference
    file for the 73 ``Q*`` lines (questions and variants), the MES-00 query
    file for the 24 ``MES00-qNNNN`` lines (ADR-0140 decision 3).
    """

    index_queries = load_index_queries(reference_dir)
    mes00 = load_mes00(mes00_path)
    built: list[dict[str, Any]] = []
    for call in load_mcp(mcp_path):
        identifier = call["id"]
        if identifier.startswith(MES00_PREFIX):
            key = identifier[len(MES00_PREFIX) :]
            if key not in mes00:
                raise P10Error(f"{identifier}: no MES-00 query with id {key}")
            text = mes00[key]["text"]
            source = "mes00"
            corpus = mes00[key]["corpus"]
        else:
            if identifier not in index_queries:
                raise P10Error(f"{identifier}: not in the six reference files")
            text = index_queries[identifier]["text"]
            source = "reference"
            corpus = None
        built.append(
            {
                "id": identifier,
                "perimetre": call["perimetre"],
                "params": call["params"],
                "text": text,
                "text_source": source,
                "source_corpus": corpus,
            }
        )
    unused = set(index_queries) - {row["id"] for row in built}
    if unused:
        raise P10Error(f"reference ids without an MCP call: {sorted(unused)}")
    return built


# ---------------------------------------------------------------------------
# Parameters: MCP name -> SQL parameter of api.recherche_semantique
# ---------------------------------------------------------------------------


def sql_params(params: Mapping[str, Any]) -> dict[str, Any]:
    """Map one ``mcp.jsonl`` params object onto the SQL function arguments.

    ``partie_exclue`` is special. The SQL function is NEUTRAL: ``NULL`` applies
    no exclusion at all, and the server default (``moyens``, ``faits``) is a
    decision of ``serveur_mcp.outils``, not of the database. The reference game
    sets ``[]`` -- "serve everything" -- for 95 of the 97 lines, so an empty
    list maps to ``NULL`` here, exactly what the MCP would send.
    """

    excluded = list(params.get("partie_exclue") or [])
    return {
        "p_fonds": params.get("fonds"),
        "p_etat": params.get("etat"),
        "p_annee_min": params.get("annee_min"),
        "p_annee_max": params.get("annee_max"),
        "p_cle_domaine": params.get("cle_domaine"),
        "p_dedupliquer": params.get("dedupliquer", "document"),
        "p_partie": excluded or None,
    }


def fetch_limit(k: int, corpus_count: int) -> int:
    """``k x 2`` per corpus when the perimeter merges several, ``k`` otherwise.

    Same rule as ``outils._fusionner_semantique``: the extra margin is there
    for the applicability filter that runs after the fusion.
    """

    if k <= 0:
        raise P10Error(f"k must be positive, got {k}")
    if corpus_count < 1:
        raise P10Error("a perimeter has at least one corpus")
    return k * 2 if corpus_count > 1 else k


def candidates(
    corpus_list: Sequence[str],
    k: int,
    search: Callable[[str, int, Mapping[str, Any]], Mapping[str, Any]],
    params: Mapping[str, Any],
) -> list[dict[str, Any]]:
    """Run one SQL call per corpus and tag every row with its corpus."""

    limit = fetch_limit(k, len(corpus_list))
    rows: list[dict[str, Any]] = []
    for corpus in corpus_list:
        raw = search(corpus, limit, sql_params(params))
        for row in raw.get("resultats") or []:
            rows.append({**row, "corpus": corpus})
    return rows


def select_ranked(
    rows: Iterable[Mapping[str, Any]],
    k: int,
    applicable: Callable[[Mapping[str, Any]], bool] | None = None,
) -> list[dict[str, Any]]:
    """Sort by distance, apply the optional filter, keep ``k`` rows."""

    ordered = sorted(rows, key=lambda row: float(row["distance"]))
    if applicable is not None:
        ordered = [row for row in ordered if applicable(row)]
    return [dict(row) for row in ordered[:k]]


def ranked_rows(results: Sequence[Mapping[str, Any]]) -> list[dict[str, Any]]:
    """One report row per result: rank, doc_id, corpus, score, distance."""

    rows: list[dict[str, Any]] = []
    for rank, result in enumerate(results, start=1):
        rows.append(
            {
                "rank": rank,
                "doc_id": result.get("src_id"),
                "corpus": result.get("corpus"),
                "score": _as_float(result.get("score")),
                "distance": _as_float(result.get("distance")),
            }
        )
    return rows


def _as_float(value: Any) -> float | None:
    if value is None:
        return None
    try:
        number = float(value)
    except (TypeError, ValueError):
        return None
    if math.isnan(number):
        return None
    return number


# ---------------------------------------------------------------------------
# Publishable report and percentiles
# ---------------------------------------------------------------------------


def publishable_queries(
    results_by_id: Mapping[str, Sequence[Mapping[str, Any]]],
) -> list[dict[str, Any]]:
    """``id`` and the ordered ``doc_id`` list only -- never a text, never a filter."""

    return [
        {"id": identifier, "doc_ids": [row["doc_id"] for row in results]}
        for identifier, results in results_by_id.items()
    ]


def counts_summary(counts: Mapping[str, Any]) -> dict[str, Any]:
    """Compact, publishable view of one count snapshot."""

    per_corpus = counts.get("per_corpus") or {}
    return {
        "taken_at": counts.get("taken_at"),
        "totals": {
            "documents": counts.get("documents"),
            "passages": counts.get("passages"),
        },
        "per_corpus": {
            corpus: {
                "documents": row.get("documents"),
                "passages": row.get("passages"),
            }
            for corpus, row in sorted(per_corpus.items())
        },
        "per_index": aggregate_indexes(per_corpus),
    }


def drift(before: Mapping[str, Any], after: Mapping[str, Any]) -> dict[str, Any]:
    """Document and passage drift between two count snapshots, per corpus and in total."""

    def pair(b: Mapping[str, Any], a: Mapping[str, Any]) -> dict[str, int]:
        return {
            "documents": int(a.get("documents") or 0) - int(b.get("documents") or 0),
            "passages": int(a.get("passages") or 0) - int(b.get("passages") or 0),
        }

    before_corpora = before.get("per_corpus") or {}
    after_corpora = after.get("per_corpus") or {}
    corpora = {
        corpus: pair(before_corpora.get(corpus, {}), after_corpora.get(corpus, {}))
        for corpus in sorted(set(before_corpora) | set(after_corpora))
    }
    before_indexes = aggregate_indexes(before_corpora)
    after_indexes = aggregate_indexes(after_corpora)
    return {
        "totals": pair(before, after),
        "per_corpus": corpora,
        "per_index": {
            index: pair(before_indexes[index], after_indexes[index])
            for index in before_indexes
        },
    }


def build_publishable(
    replay: Mapping[str, Any],
    counts_before: Mapping[str, Any],
    counts_after: Mapping[str, Any],
    *,
    machine: str,
    fixed_date: str,
    encoder: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    """The single publishable form of a production result (ADR-0125 decision 16).

    Only ``id`` and ``doc_id`` for the results; latency percentiles and counts
    for the rest. No query text, no filter value, no server path.
    """

    meta = replay.get("meta") or {}
    measurements = replay.get("measurements") or []
    results_by_id = {row["id"]: row.get("results") or [] for row in measurements}
    aggregates = replay.get("aggregates") or {}
    return {
        "format_version": SCHEMA_VERSION,
        "measure": "COMPARER-AVEC-PGVECTOR",
        "source": "pgvector",
        "campaign_date": meta.get("campaign_date"),
        "fixed_date": fixed_date,
        "machine": machine,
        "dataset": {
            "name": "jeu de référence Legifacile",
            "queries": len(measurements),
            "k": meta.get("k"),
            "dedupliquer": "document",
            "query_sha256": meta.get("query_sha256"),
            "vector_sha256": meta.get("vector_sha256"),
        },
        "repetitions": meta.get("repetitions"),
        "warmup": meta.get("warmup"),
        "latency_ms": {
            stage: aggregates.get(stage) for stage in ("sql", "reconstruction", "total")
        },
        "encoder_ms": dict(encoder) if encoder is not None else None,
        "counts": {
            "before": counts_summary(counts_before),
            "after": counts_summary(counts_after),
            "drift": drift(counts_before, counts_after),
        },
        "queries": publishable_queries(results_by_id),
    }


def percentile(values: Iterable[float], percent: float) -> float:
    """Linear-interpolated percentile, the definition MES-00's harness uses."""

    data = sorted(float(value) for value in values)
    if not data:
        raise P10Error("percentile of an empty series")
    if not 0.0 <= percent <= 100.0:
        raise P10Error(f"percentile must be in [0, 100], got {percent}")
    if len(data) == 1:
        return data[0]
    rank = (len(data) - 1) * percent / 100.0
    low = math.floor(rank)
    high = math.ceil(rank)
    if low == high:
        return data[low]
    weight = rank - low
    return data[low] * (1.0 - weight) + data[high] * weight


def summarize(values: Iterable[float]) -> dict[str, Any]:
    data = sorted(float(value) for value in values)
    if not data:
        raise P10Error("cannot summarize an empty series")
    return {
        "n": len(data),
        "min": round(data[0], 3),
        "max": round(data[-1], 3),
        "mean": round(sum(data) / len(data), 3),
        "p50": round(percentile(data, 50.0), 3),
        "p95": round(percentile(data, 95.0), 3),
        "p99": round(percentile(data, 99.0), 3),
    }


def aggregate_indexes(
    per_corpus: Mapping[str, Mapping[str, int]],
) -> dict[str, dict[str, int]]:
    """Sum the corpus counters into the six Crible indexes (ADR-0140 decision 10)."""

    indexes: dict[str, dict[str, int]] = {}
    for index, corpora in INDEX_CORPORA.items():
        documents = sum(int(per_corpus.get(c, {}).get("documents", 0)) for c in corpora)
        passages = sum(int(per_corpus.get(c, {}).get("passages", 0)) for c in corpora)
        indexes[index] = {"documents": documents, "passages": passages}
    return indexes
