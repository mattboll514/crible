# SPDX-License-Identifier: Apache-2.0

"""Fidelity metrics of MES-01 and MES-12.

The definitions are those of ADR-0076, decision 1:

- ``recall@10`` is the mean over queries of ``|top-10 BINARY ∩ top-10 EXACT| / 10``
  at the **passage** level, the key being ``doc_id#ord``. The denominator is the
  ten results of the exact reference, which a non-empty pool always yields.
- ``overlap@10`` is the same intersection after projecting both top-10 on
  ``doc_id`` and deduplicating, divided by the number of **distinct** ``doc_id``
  of the exact top-10 (the "Accord document@10" of MES-04).

Both are computed per query and averaged over the queries of a group; ADR-0076
puts the 0.99 target on the global mean of ``recall@10`` at the default ``N``,
and reports ``overlap@10`` without a target.
"""

from __future__ import annotations

#: Number of results the two metrics compare (ADR-0076, decision 1).
K = 10


def passage_keys(record: dict) -> list[str]:
    """Returns the ``doc_id#ord`` keys of one result list, in order."""
    return [result["key"] for result in record["results"]]


def distinct_documents(record: dict) -> list[str]:
    """Returns the distinct ``doc_id`` of one result list, in first-seen order."""
    seen: list[str] = []
    for result in record["results"]:
        if result["doc_id"] not in seen:
            seen.append(result["doc_id"])
    return seen


def recall_at_10(binary: dict, exact: dict) -> float:
    """Fraction of the exact top-10 passages the binary top-10 also holds."""
    reference = passage_keys(exact)
    if len(reference) != K:
        raise ValueError(
            f"the exact reference returns {len(reference)} results, not {K}: "
            "the denominator of ADR-0076 is the ten exact passages"
        )
    found = set(passage_keys(binary))
    return len(found & set(reference)) / K


def overlap_at_10(binary: dict, exact: dict) -> float:
    """Fraction of the exact top-10 documents the binary top-10 also holds."""
    reference = distinct_documents(exact)
    if not reference:
        raise ValueError("the exact reference holds no document")
    found = set(distinct_documents(binary))
    return len(found & set(reference)) / len(reference)


def mean(values: list[float]) -> float:
    """Arithmetic mean of a non-empty list."""
    if not values:
        raise ValueError("the mean of no value is undefined")
    return sum(values) / len(values)


def summarise(rows: list[dict], key: str = "recall") -> dict:
    """Mean, minimum and number of queries of one per-query metric."""
    values = [row[key] for row in rows]
    return {
        "queries": len(values),
        "mean": mean(values),
        "min": min(values),
        "perfect": sum(1 for value in values if value == 1.0),
    }
