"""Pure helpers for pass B of ``COMPARER-AVEC-PGVECTOR`` (P10).

Pass B measures Crible on the six indexes of ``crible-p10-data`` with
``crible eval`` (ADR-0140, decisions 2 to 7 and 10). Everything here runs
without Docker, without an index and without the Legifacile package: it builds
the four input games of an index, reads the raw ``eval`` reports, aggregates
the quality by population, and compares the dense primary pass with the
pgvector replay of pass A.

The population rules are ADR-0140, decision 4:

- the **global** recall is read on the 56 satisfiable base questions
  (``Q01`` to ``Q58`` without ``Q40``), ``Q30`` counted apart;
- ``juri_texte_integral`` carries 31 base questions and ``textes_vigueur`` 12;
- every other perimeter is reported for reading, with its ``n``.

The corpus of a base question is the one of ``questions.yaml``, transcribed by
the verdict table of ``docs/mesures/MES-00-reference-pgvector.2026-10-20.eval-recherche.md``
(``id`` and ``corpus`` columns). The corpus of a ``MES00-qNNNN`` line is the
``corpus`` member of ``data/mes00/queries.json``, which ``build_inputs.py``
already carries as ``source_corpus``.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import tempfile
from pathlib import Path
from typing import Any, Callable, Iterable, Mapping, Sequence

import queries as game

SCHEMA_VERSION = 1

#: Corpus of the 57 base questions, from ``questions.yaml`` as transcribed by
#: the MES-00 eval report. ``Q40`` does not exist (``jeu-de-reference.md``).
BASE_CORPUS: dict[str, str] = {
    "Q01": "textes_vigueur",
    "Q02": "textes_vigueur",
    "Q03": "textes_vigueur",
    "Q04": "textes_vigueur",
    "Q05": "textes_vigueur",
    "Q06": "textes_vigueur",
    "Q07": "textes_vigueur",
    "Q08": "textes_vigueur",
    "Q09": "textes_vigueur",
    "Q10": "textes_vigueur",
    "Q11": "textes_vigueur",
    "Q12": "textes_vigueur",
    "Q13": "juri_sommaires",
    "Q14": "juri_texte_integral",
    "Q15": "juri_texte_integral",
    "Q16": "juri_texte_integral",
    "Q17": "juri_texte_integral",
    "Q18": "juri_sommaires",
    "Q19": "juri_sommaires",
    "Q20": "juri_sommaires",
    "Q21": "jorf_normatif",
    "Q22": "jorf_normatif",
    "Q23": "jorf_normatif",
    "Q24": "jorf_normatif",
    "Q25": "sarde",
    "Q26": "sarde",
    "Q27": "sarde",
    "Q28": "dole",
    "Q29": "dole",
    "Q30": "textes_vigueur",
    "Q31": "juri_texte_integral",
    "Q32": "juri_texte_integral",
    "Q33": "juri_texte_integral",
    "Q34": "juri_texte_integral",
    "Q35": "juri_texte_integral",
    "Q36": "juri_texte_integral",
    "Q37": "juri_texte_integral",
    "Q38": "juri_texte_integral",
    "Q39": "juri_texte_integral",
    "Q41": "juri_texte_integral",
    "Q42": "juri_texte_integral",
    "Q43": "juri_texte_integral",
    "Q44": "juri_texte_integral",
    "Q45": "juri_texte_integral",
    "Q46": "juri_texte_integral",
    "Q47": "juri_texte_integral",
    "Q48": "juri_texte_integral",
    "Q49": "juri_texte_integral",
    "Q50": "juri_texte_integral",
    "Q51": "juri_texte_integral",
    "Q52": "juri_texte_integral",
    "Q53": "juri_texte_integral",
    "Q54": "juri_texte_integral",
    "Q55": "juri_texte_integral",
    "Q56": "juri_texte_integral",
    "Q57": "juri_texte_integral",
    "Q58": "juri_texte_integral",
}

#: The unsatisfiable question, counted apart (``jeu-de-reference.md``).
UNSATISFIABLE = "Q30"

#: The two corpus populations constrained one by one (``docs/suivi.md``).
CONSTRAINED_CORPORA = ("juri_texte_integral", "textes_vigueur")

#: Expected size of the constrained populations, checked by the tests.
CONSTRAINED_SIZES = {"juri_texte_integral": 31, "textes_vigueur": 12}

#: Expected size of the global population.
GLOBAL_SIZE = 56

#: The four ``crible eval`` games of one index, and the two kind of passes.
PASSES = ("dense-primaire", "dense-secondaire", "hybride", "fidelite")
QUALITY_PASSES = ("dense-primaire", "dense-secondaire", "hybride")

#: Report mode of each pass.
PASS_MODE = {
    "dense-primaire": "dense",
    "dense-secondaire": "dense",
    "hybride": "hybrid",
    "fidelite": "dense",
}

#: ``--exact-threshold`` of each pass: the index default (5000) for quality, the
#: forced binary of the fidelity protocol (ADR-0140, decision 6).
PASS_EXACT_THRESHOLD = {
    "dense-primaire": 5000,
    "dense-secondaire": 5000,
    "hybride": 5000,
    "fidelite": 0,
}

BASE_ID = re.compile(r"^Q\d\d$")


class PassBError(Exception):
    """An input contract or a measurement guard failed."""


# ---------------------------------------------------------------------------
# Populations
# ---------------------------------------------------------------------------


def base_ids() -> list[str]:
    """The 57 base identifiers, ``Q30`` included."""

    return sorted(BASE_CORPUS)


def satisfiable_ids() -> list[str]:
    """The 56 satisfiable base questions, ``Q30`` excluded."""

    return [identifier for identifier in base_ids() if identifier != UNSATISFIABLE]


def corpus_ids(corpus: str, *, include_unsatisfiable: bool = False) -> list[str]:
    """Base identifiers of one corpus, ``Q30`` excluded by default."""

    identifiers = [
        identifier
        for identifier, value in BASE_CORPUS.items()
        if value == corpus and (include_unsatisfiable or identifier != UNSATISFIABLE)
    ]
    return sorted(identifiers)


def is_base_question(identifier: str) -> bool:
    """True for ``Qnn``, false for a variant or a ``MES00-*`` line."""

    return bool(BASE_ID.match(identifier)) and identifier in BASE_CORPUS


def question_corpus(identifier: str, source_corpus: str | None = None) -> str | None:
    """Corpus of a line: base question first, MES-00 ``corpus`` otherwise."""

    if identifier in BASE_CORPUS:
        return BASE_CORPUS[identifier]
    return source_corpus


# ---------------------------------------------------------------------------
# Input games
# ---------------------------------------------------------------------------


def load_reference(reference_dir: str | Path) -> dict[str, list[dict[str, Any]]]:
    """The six ``<index>.jsonl`` files, keyed by index, validated.

    Each row must carry ``id``, one of ``query`` or ``vector``, and an optional
    ``filter`` (object) and ``expected`` (list of non-empty strings).
    """

    directory = Path(reference_dir)
    games: dict[str, list[dict[str, Any]]] = {}
    seen: set[str] = set()
    for index in game.INDEXES:
        path = directory / f"{index}.jsonl"
        rows = game.read_jsonl(path)
        for number, row in enumerate(rows, start=1):
            identifier = row.get("id")
            if not isinstance(identifier, str) or not identifier:
                raise PassBError(f"{path.name}:{number}: a row has no id")
            if identifier in seen:
                raise PassBError(f"{path.name}:{number}: duplicate id {identifier}")
            seen.add(identifier)
            keys = set(row)
            unknown = keys - {"id", "query", "vector", "filter", "expected"}
            if unknown:
                raise PassBError(
                    f"{path.name}:{number} ({identifier}): unknown keys {sorted(unknown)}"
                )
            if ("query" in row) == ("vector" in row):
                raise PassBError(
                    f"{path.name}:{number} ({identifier}): exactly one of query and vector"
                )
            if "filter" in row and not isinstance(row["filter"], dict):
                raise PassBError(
                    f"{path.name}:{number} ({identifier}): filter is not an object"
                )
            expected = row.get("expected", [])
            if not isinstance(expected, list) or any(
                not isinstance(entry, str) or not entry for entry in expected
            ):
                raise PassBError(
                    f"{path.name}:{number} ({identifier}): expected is not a list of ids"
                )
        games[index] = rows
    return games


def text_game(rows: Sequence[Mapping[str, Any]]) -> list[dict[str, Any]]:
    """The textual game of a pass: ``id``, ``query``, ``filter``, ``expected``."""

    built: list[dict[str, Any]] = []
    for row in rows:
        text = row.get("query")
        if not isinstance(text, str) or not text.strip():
            raise PassBError(f"{row.get('id')}: no query text")
        entry: dict[str, Any] = {"id": row["id"], "query": text}
        if "filter" in row:
            entry["filter"] = row["filter"]
        if "expected" in row:
            entry["expected"] = list(row["expected"])
        built.append(entry)
    return built


def vector_game(
    rows: Sequence[Mapping[str, Any]],
    vectors: Mapping[str, Sequence[float]],
) -> list[dict[str, Any]]:
    """The dense primary game: the pgvector vector replaces the text.

    ADR-0125, decision 2: a line carries exactly one of ``query`` and
    ``vector``. The vector comes from ``query-vectors.json`` and is used as
    given, without re-encoding.
    """

    built: list[dict[str, Any]] = []
    for row in rows:
        identifier = row["id"]
        vector = vectors.get(identifier)
        if vector is None:
            raise PassBError(f"{identifier}: no query vector")
        if not vector:
            raise PassBError(f"{identifier}: empty query vector")
        entry: dict[str, Any] = {"id": identifier, "vector": list(vector)}
        if "filter" in row:
            entry["filter"] = row["filter"]
        if "expected" in row:
            entry["expected"] = list(row["expected"])
        built.append(entry)
    return built


def _write_atomic(target: Path, payload: bytes) -> None:
    """Write ``payload`` to ``target`` through a temporary file and ``os.replace``."""

    target.parent.mkdir(parents=True, exist_ok=True)
    handle = tempfile.NamedTemporaryFile(
        mode="wb",
        dir=target.parent,
        prefix=f".{target.name}.",
        suffix=".tmp",
        delete=False,
    )
    temporary = Path(handle.name)
    try:
        with handle:
            handle.write(payload)
        os.replace(temporary, target)
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise


def write_jsonl(path: str | Path, rows: Sequence[Mapping[str, Any]]) -> str:
    """Write one compact JSON object per line; return the SHA-256."""

    payload = "".join(
        json.dumps(row, ensure_ascii=False, separators=(",", ":")) + "\n"
        for row in rows
    ).encode("utf-8")
    _write_atomic(Path(path), payload)
    return hashlib.sha256(payload).hexdigest()


def write_json(path: str | Path, document: Any) -> None:
    """Write one pretty JSON document, newline terminated, atomically."""

    payload = json.dumps(document, ensure_ascii=False, indent=2) + "\n"
    _write_atomic(Path(path), payload.encode("utf-8"))


def load_vectors(path: str | Path) -> dict[str, list[float]]:
    """Read ``id -> vector`` from the pass A ``query-vectors.json``."""

    payload = game.read_json(path)
    rows = payload.get("queries") if isinstance(payload, Mapping) else None
    if not isinstance(rows, list):
        raise PassBError("the vector file has no queries array")
    vectors: dict[str, list[float]] = {}
    for row in rows:
        identifier = row.get("id")
        vector = row.get("vector")
        if not isinstance(identifier, str) or not identifier:
            raise PassBError("the vector file has a row without id")
        if not isinstance(vector, list) or not vector:
            raise PassBError(f"{identifier}: malformed vector")
        vectors[identifier] = [float(value) for value in vector]
    return vectors


# ---------------------------------------------------------------------------
# Reading an eval report
# ---------------------------------------------------------------------------


def mode_of(report: Mapping[str, Any], pass_name: str) -> Mapping[str, Any]:
    """The mode object of one pass, or an explicit error."""

    mode = PASS_MODE[pass_name]
    modes = report.get("modes")
    if not isinstance(modes, Mapping) or mode not in modes:
        raise PassBError(f"the report carries no mode {mode}")
    return modes[mode]


def quality_rows(
    report: Mapping[str, Any], pass_name: str
) -> dict[str, dict[str, Any]]:
    """``id -> row`` of the quality figures of one pass."""

    mode = mode_of(report, pass_name)
    rows = mode.get("queries")
    if not isinstance(rows, list):
        raise PassBError("the mode carries no queries array")
    indexed: dict[str, dict[str, Any]] = {}
    for row in rows:
        identifier = row.get("id")
        if not isinstance(identifier, str) or not identifier:
            raise PassBError("a report row has no id")
        if "quality_recall_at_k" not in row:
            raise PassBError(f"{identifier}: no quality figure")
        indexed[identifier] = row
    return indexed


def mean(values: Iterable[float]) -> float:
    data = list(values)
    if not data:
        raise PassBError("mean of an empty series")
    return sum(data) / len(data)


def quality_summary(
    rows: Mapping[str, Mapping[str, Any]],
    identifiers: Sequence[str],
) -> dict[str, Any]:
    """Mean recall@10 and MRR@10 at document over a population, with its ``n``."""

    missing = [identifier for identifier in identifiers if identifier not in rows]
    if missing:
        raise PassBError(f"missing report rows: {missing}")
    recalls = [float(rows[identifier]["quality_recall_at_k"]) for identifier in identifiers]
    mrrs = [float(rows[identifier]["quality_mrr_at_k"]) for identifier in identifiers]
    return {
        "n": len(identifiers),
        "recall_at_10": round(mean(recalls), 6),
        "mrr_at_10": round(mean(mrrs), 6),
    }


def found_expected(row: Mapping[str, Any], expected: Sequence[str]) -> list[str]:
    """The expected documents found in the top-k, in expected order."""

    indices = row.get("quality_found") or []
    found: list[str] = []
    for position in indices:
        if not isinstance(position, int) or not 0 <= position < len(expected):
            raise PassBError("quality_found holds an index outside expected")
        found.append(expected[position])
    return found


# ---------------------------------------------------------------------------
# pgvector side (pass A replay)
# ---------------------------------------------------------------------------


def pgvector_rows(
    replay: Mapping[str, Any],
    expected_by_id: Mapping[str, Sequence[str]],
) -> dict[str, dict[str, Any]]:
    """``id -> {quality_recall_at_k, quality_mrr_at_k, doc_ids}`` from the replay.

    The replay already returns top-10 **documents** (``dedupliquer:
    document``), so the recall and the MRR are the document metrics of
    ADR-0092, decision 2, exactly as ``crible eval`` computes them.
    """

    measurements = replay.get("measurements")
    if not isinstance(measurements, list):
        raise PassBError("the replay has no measurements array")
    rows: dict[str, dict[str, Any]] = {}
    for measurement in measurements:
        identifier = measurement.get("id")
        if not isinstance(identifier, str) or not identifier:
            raise PassBError("a replay measurement has no id")
        expected = [str(entry) for entry in expected_by_id.get(identifier, [])]
        if not expected:
            raise PassBError(f"{identifier}: no expected documents")
        results = measurement.get("results") or []
        doc_ids = [str(result["doc_id"]) for result in results]
        found = [doc for doc in doc_ids if doc in expected]
        recall = len(found) / len(expected)
        mrr = 0.0
        for rank, doc in enumerate(doc_ids, start=1):
            if doc in expected:
                mrr = 1.0 / rank
                break
        rows[identifier] = {
            "quality_recall_at_k": recall,
            "quality_mrr_at_k": mrr,
            "doc_ids": doc_ids,
        }
    return rows


def expected_by_id(
    reference: Mapping[str, Sequence[Mapping[str, Any]]],
) -> dict[str, list[str]]:
    """``id -> expected`` over the six reference files."""

    expected: dict[str, list[str]] = {}
    for rows in reference.values():
        for row in rows:
            expected[str(row["id"])] = [str(entry) for entry in row.get("expected", [])]
    return expected


# ---------------------------------------------------------------------------
# Comparisons
# ---------------------------------------------------------------------------


def compare_rankings(
    primary: Mapping[str, Mapping[str, Any]],
    pgvector: Mapping[str, Mapping[str, Any]],
    identifiers: Sequence[str],
    expected: Mapping[str, Sequence[str]],
) -> dict[str, Any]:
    """Queries where the dense primary loses or gains an expected document.

    ``lost`` is an expected that pgvector finds and Crible does not; ``gained``
    the reverse. Both are data, not judgements; ADR-0140, decision 4, asks for
    the list of losses.
    """

    lost: list[dict[str, Any]] = []
    gained: list[dict[str, Any]] = []
    for identifier in identifiers:
        if identifier not in primary or identifier not in pgvector:
            raise PassBError(f"{identifier}: missing on one side of the comparison")
        wanted = [str(entry) for entry in expected.get(identifier, [])]
        if not wanted:
            raise PassBError(f"{identifier}: no expected documents")
        crible = set(found_expected(primary[identifier], wanted))
        replayed = set(pgvector[identifier]["doc_ids"]) & set(wanted)
        for doc in wanted:
            if doc in replayed and doc not in crible:
                lost.append({"id": identifier, "doc_id": doc})
            elif doc in crible and doc not in replayed:
                gained.append({"id": identifier, "doc_id": doc})
    return {
        "lost": lost,
        "gained": gained,
        "lost_queries": len({entry["id"] for entry in lost}),
        "gained_queries": len({entry["id"] for entry in gained}),
    }


def missing_documents(
    expected: Mapping[str, Sequence[str]],
    pgvector: Mapping[str, Mapping[str, Any]],
    exists: Callable[[str, str], bool],
    index_of: Mapping[str, str],
) -> dict[str, list[dict[str, str]]]:
    """Data gaps: expected or pgvector result documents absent from Crible.

    ``exists(index, doc_id)`` is the reader of the Crible side; the pure
    function only decides what to ask. An absent ``expected`` counts as not
    found, never as a campaign error (ADR-0125, decision 4; ADR-0140,
    decision 10).
    """

    absent_expected: list[dict[str, str]] = []
    absent_results: list[dict[str, str]] = []
    seen_expected: set[tuple[str, str]] = set()
    seen_results: set[tuple[str, str]] = set()
    for identifier, wanted in expected.items():
        index = index_of.get(identifier)
        if index is None:
            raise PassBError(f"{identifier}: no index")
        for doc in wanted:
            key = (index, doc)
            if key in seen_expected:
                continue
            seen_expected.add(key)
            if not exists(index, doc):
                absent_expected.append({"id": identifier, "doc_id": doc, "index": index})
        for doc in pgvector.get(identifier, {}).get("doc_ids", []):
            key = (index, str(doc))
            if key in seen_results:
                continue
            seen_results.add(key)
            if not exists(index, str(doc)):
                absent_results.append({"id": identifier, "doc_id": str(doc), "index": index})
    return {"expected": absent_expected, "results": absent_results}


def index_of_question(
    reference: Mapping[str, Sequence[Mapping[str, Any]]],
) -> dict[str, str]:
    """``id -> index`` over the six reference files."""

    mapping: dict[str, str] = {}
    for index, rows in reference.items():
        for row in rows:
            mapping[str(row["id"])] = index
    return mapping


def per_query_rows(
    primary: Mapping[str, Mapping[str, Any]],
    pgvector: Mapping[str, Mapping[str, Any]],
    expected: Mapping[str, Sequence[str]],
    index_of: Mapping[str, str],
) -> list[dict[str, Any]]:
    """One row per query: what each side found, without any text or filter.

    This is the per-query form pass C needs: ``id``, ``index``, corpus,
    ``expected`` and the two found sets. Nothing here is a report metric; the
    aggregates stay in :func:`aggregate_passes` and :func:`pgvector_pass`.
    """

    rows: list[dict[str, Any]] = []
    for identifier in sorted(expected):
        wanted = [str(entry) for entry in expected[identifier]]
        if not wanted:
            raise PassBError(f"{identifier}: no expected documents")
        if identifier not in primary or identifier not in pgvector:
            raise PassBError(f"{identifier}: missing on one side")
        index = index_of.get(identifier)
        if index is None:
            raise PassBError(f"{identifier}: no index")
        rows.append(
            {
                "id": identifier,
                "index": index,
                "corpus": question_corpus(identifier),
                "expected": wanted,
                "primary_found": found_expected(primary[identifier], wanted),
                "pgvector_found": [
                    str(doc)
                    for doc in pgvector[identifier]["doc_ids"]
                    if str(doc) in wanted
                ],
                "primary_recall_at_10": float(
                    primary[identifier]["quality_recall_at_k"]
                ),
                "pgvector_recall_at_10": float(
                    pgvector[identifier]["quality_recall_at_k"]
                ),
            }
        )
    return rows


# ---------------------------------------------------------------------------
# Fidelity, latency and counts
# ---------------------------------------------------------------------------


def fidelity_summary(report: Mapping[str, Any], expected_queries: int) -> dict[str, Any]:
    """Fidelity of a dense report, with the forced strategy checked.

    ADR-0140, decision 6: BINARY forced by ``exact_threshold = 0`` and N = 5000;
    the report must show ``dense_strategy: {"binary": n}``.
    """

    mode = mode_of(report, "fidelite")
    strategies = mode.get("dense_strategy")
    if not isinstance(strategies, Mapping) or list(strategies) != ["binary"]:
        raise PassBError(f"dense_strategy is not binary only: {strategies}")
    if int(strategies["binary"]) != expected_queries:
        raise PassBError(
            f"dense_strategy counts {strategies['binary']} queries, expected {expected_queries}"
        )
    fidelity = mode.get("fidelity")
    if not isinstance(fidelity, Mapping):
        raise PassBError("the report carries no fidelity")
    return {
        "n": int(fidelity.get("n") or 0),
        "recall_at_10": float(fidelity["recall_at_10"]),
        "overlap_at_10": float(fidelity["overlap_at_10"]),
        "dense_strategy": {"binary": int(strategies["binary"])},
        "exact_threshold": report.get("exact_threshold"),
        "rescore_candidates": report.get("rescore_candidates"),
    }


def latency_summary(report: Mapping[str, Any], pass_name: str) -> dict[str, Any]:
    """p50, p95 and p99 per stage and for ``total``, from ``debug.timings_ms``."""

    mode = mode_of(report, pass_name)
    latency = mode.get("latency")
    if not isinstance(latency, Mapping):
        raise PassBError("the report carries no latency")
    return {
        stage: {
            key: float(values[key])
            for key in ("p50", "p95", "p99")
            if key in values
        }
        for stage, values in latency.items()
        if isinstance(values, Mapping) and "p50" in values
    }


INSPECT_INDEX = re.compile(r"^index (\S+)$")
INSPECT_DOCUMENTS = re.compile(r"^\s+documents: (\d+)$")
INSPECT_PASSAGES = re.compile(r"^\s+passages: (\d+) live, (\d+) dead$")


def parse_inspect(text: str) -> dict[str, dict[str, int]]:
    """Parse the output of ``crible inspect --data <data>``.

    Returns ``index -> {documents, passages, dead}``, leaving the text lines
    that carry memory or segments out.
    """

    indexes: dict[str, dict[str, int]] = {}
    current: str | None = None
    for line in text.splitlines():
        match = INSPECT_INDEX.match(line)
        if match:
            current = match.group(1)
            indexes[current] = {"documents": 0, "passages": 0, "dead": 0}
            continue
        if current is None:
            continue
        match = INSPECT_DOCUMENTS.match(line)
        if match:
            indexes[current]["documents"] = int(match.group(1))
            continue
        match = INSPECT_PASSAGES.match(line)
        if match:
            indexes[current]["passages"] = int(match.group(1))
            indexes[current]["dead"] = int(match.group(2))
    if not indexes:
        raise PassBError("crible inspect printed no index")
    return indexes


def counts_comparison(
    crible: Mapping[str, Mapping[str, int]],
    pgvector: Mapping[str, Mapping[str, int]],
) -> dict[str, Any]:
    """Crible counts against pgvector's, per index (ADR-0140, decision 10)."""

    per_index: dict[str, Any] = {}
    for index in game.INDEXES:
        if index not in crible or index not in pgvector:
            raise PassBError(f"{index}: missing from one count snapshot")
        row: dict[str, Any] = {}
        for field in ("documents", "passages"):
            crible_value = int(crible[index].get(field, 0))
            pgvector_value = int(pgvector[index].get(field, 0))
            row[field] = {
                "crible": crible_value,
                "pgvector": pgvector_value,
                "gap": crible_value - pgvector_value,
            }
        per_index[index] = row
    return {
        "per_index": per_index,
        "totals": {
            field: {
                "crible": sum(per_index[i][field]["crible"] for i in game.INDEXES),
                "pgvector": sum(per_index[i][field]["pgvector"] for i in game.INDEXES),
                "gap": sum(per_index[i][field]["gap"] for i in game.INDEXES),
            }
            for field in ("documents", "passages")
        },
    }


# ---------------------------------------------------------------------------
# Aggregation of the three passes
# ---------------------------------------------------------------------------


def aggregate_passes(
    reports: Mapping[str, Mapping[str, Mapping[str, Any]]],
) -> dict[str, Any]:
    """Aggregate each quality pass on the global and the two constrained corpus.

    ``reports`` is ``pass -> index -> eval JSON``. The result carries the three
    passes of quality and the pgvector side is added by the caller.
    """

    passes: dict[str, Any] = {}
    for pass_name in QUALITY_PASSES:
        rows: dict[str, dict[str, Any]] = {}
        by_index = reports.get(pass_name) or {}
        if not by_index:
            raise PassBError(f"no report for pass {pass_name}")
        for index, report in by_index.items():
            for identifier, row in quality_rows(report, pass_name).items():
                if identifier in rows:
                    raise PassBError(f"{identifier}: present in two indexes")
                rows[identifier] = row
        if len(rows) != 97:
            raise PassBError(f"{pass_name}: {len(rows)} rows, expected 97")
        entry: dict[str, Any] = {
            "global": quality_summary(rows, satisfiable_ids()),
            "juri_texte_integral": quality_summary(
                rows, corpus_ids("juri_texte_integral")
            ),
            "textes_vigueur": quality_summary(rows, corpus_ids("textes_vigueur")),
            "all_97": quality_summary(rows, sorted(rows)),
            "by_base_corpus": {
                corpus: quality_summary(rows, corpus_ids(corpus))
                for corpus in sorted(set(BASE_CORPUS.values()))
            },
            "q30": quality_summary(rows, [UNSATISFIABLE]),
        }
        passes[pass_name] = entry
    return passes


def pgvector_pass(
    pgvector: Mapping[str, Mapping[str, Any]],
) -> dict[str, Any]:
    """The same aggregates for the pgvector replay."""

    if len(pgvector) != 97:
        raise PassBError(f"pgvector: {len(pgvector)} rows, expected 97")
    return {
        "global": quality_summary(pgvector, satisfiable_ids()),
        "juri_texte_integral": quality_summary(
            pgvector, corpus_ids("juri_texte_integral")
        ),
        "textes_vigueur": quality_summary(pgvector, corpus_ids("textes_vigueur")),
        "all_97": quality_summary(pgvector, sorted(pgvector)),
        "by_base_corpus": {
            corpus: quality_summary(pgvector, corpus_ids(corpus))
            for corpus in sorted(set(BASE_CORPUS.values()))
        },
        "q30": quality_summary(pgvector, [UNSATISFIABLE]),
    }


def check_populations() -> None:
    """Fail early if the transcribed populations drifted from the targets."""

    if len(satisfiable_ids()) != GLOBAL_SIZE:
        raise PassBError(f"global population is {len(satisfiable_ids())}, expected {GLOBAL_SIZE}")
    for corpus, size in CONSTRAINED_SIZES.items():
        found = len(corpus_ids(corpus))
        if found != size:
            raise PassBError(f"{corpus} population is {found}, expected {size}")
    if len(BASE_CORPUS) != 57:
        raise PassBError(f"base population is {len(BASE_CORPUS)}, expected 57")
