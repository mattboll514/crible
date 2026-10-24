# SPDX-License-Identifier: Apache-2.0

"""Computation of the MES-04 statistics.

The module holds the parts worth testing: percentiles, cosine, exact top-k and
recall. ``analyze.py`` is the command line that reads the campaign files and
writes the raw JSON next to the report.

**Exact search.** The passage vectors of the sample are the production vectors
(fp16) that `crible attach` copies into its segments. The exact dense branch of
the engine scores every live passage of the pool with the cosine of the query
against those vectors. This module does the same in numpy: same vectors, same
cosine, same tie order (score descending, then passage key ascending, the order
of `ReferenceSearcher` and of `tools/reference_topk.py`). The engine has no
deduplication in the search path (``docs/03-recherche.md`` describes
``dedup: "content"`` as an option, and `crible-search` does not implement it), so
none is applied here either.

**Percentiles** use the nearest-rank convention: for ``n`` sorted values and
``p`` percent, the rank is ``ceil(p / 100 * n)``, clamped to ``[1, n]``. This is
the convention of MES-13 and MES-15. On 300 values, p1 is therefore the third
smallest value, not an interpolated one.
"""

from __future__ import annotations

import math
from pathlib import Path

import numpy as np

#: Width of an mDenseOn vector.
DIMS = 768
#: Results compared by the recall.
K = 10
#: Structures of the Legifacile sample, in the order of its README.
STRUCTURES = ("articles", "decisions", "dole", "jorf", "sarde", "titres")


def percentile(values, percent: float) -> float:
    """Returns the nearest-rank percentile of ``values``.

    ``percent`` is in ``(0, 100]``. The smallest value is rank 1.
    """
    if not values:
        raise ValueError("cannot take a percentile of an empty series")
    if not 0.0 < percent <= 100.0:
        raise ValueError(f"percentile must be in (0, 100], got {percent}")
    ordered = sorted(values)
    rank = math.ceil(percent / 100.0 * len(ordered))
    return float(ordered[min(max(rank, 1), len(ordered)) - 1])


def summarise(values) -> dict:
    """Returns the distribution summary used by the reports."""
    values = [float(value) for value in values]
    if not values:
        raise ValueError("cannot summarise an empty series")
    return {
        "count": len(values),
        "min": min(values),
        "p1": percentile(values, 1),
        "p5": percentile(values, 5),
        "p50": percentile(values, 50),
        "p95": percentile(values, 95),
        "p99": percentile(values, 99),
        "max": max(values),
        "mean": sum(values) / len(values),
    }


def cosine(left, right) -> float:
    """Returns the cosine of two vectors, computed in float64."""
    a = np.asarray(left, dtype=np.float64)
    b = np.asarray(right, dtype=np.float64)
    if a.shape != b.shape:
        raise ValueError(f"vector shapes differ: {a.shape} against {b.shape}")
    denominator = float(np.linalg.norm(a) * np.linalg.norm(b))
    if denominator == 0.0:
        raise ValueError("cannot take the cosine of a zero vector")
    return float(np.dot(a, b) / denominator)


def load_f32_matrix(path: Path, count: int, dims: int = DIMS) -> np.ndarray:
    """Reads a row-major little-endian f32 matrix and checks its size."""
    raw = path.read_bytes()
    expected = count * dims * 4
    if len(raw) != expected:
        raise ValueError(f"{path}: expected {expected} bytes, got {len(raw)}")
    return np.frombuffer(raw, dtype="<f4").reshape(count, dims).copy()


def passage_keys(doc_ids, ords) -> np.ndarray:
    """Returns the ``doc_id#ord`` key of every passage, as the engine prints it."""
    return np.asarray(
        [f"{doc_id}#{int(ordinal)}" for doc_id, ordinal in zip(doc_ids, ords)],
        dtype=f"<U{max(len(str(value)) for value in doc_ids) + 12}",
    )


def load_passages(sample_dir: Path, structure: str) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """Returns ``(vectors f32, passage keys, doc ids)`` of one structure.

    Every passage file of the batch is read, in file-name order, which is the
    order `crible attach` gives them. The vectors stay the production fp16
    values, converted to f32 as the engine does before scoring.
    """
    import pyarrow.parquet as pq

    batch = Path(sample_dir) / "batches" / f"legifacile-sample-{structure}-v1"
    paths = sorted(batch.glob("passages-*.parquet"))
    if not paths:
        raise ValueError(f"no passage file under {batch}")

    matrices = []
    keys = []
    docs = []
    for path in paths:
        table = pq.read_table(path, columns=["doc_id", "ord", "vector"])
        array = table.column("vector").combine_chunks()
        if array.null_count:
            raise ValueError(f"{path}: a passage has no vector")
        raw = array.buffers()[1]
        matrix = np.frombuffer(raw, dtype="<f2").reshape(table.num_rows, DIMS)
        matrices.append(matrix.astype(np.float32))
        doc_ids = table.column("doc_id").to_pylist()
        ords = table.column("ord").to_pylist()
        docs.extend(doc_ids)
        keys.extend(
            f"{doc_id}#{int(ordinal)}" for doc_id, ordinal in zip(doc_ids, ords)
        )
    vectors = np.concatenate(matrices, axis=0)
    return vectors, np.asarray(keys), np.asarray(docs)


def top_k_indices(scores: np.ndarray, keys: np.ndarray, k: int = K) -> np.ndarray:
    """Returns the indices of the ``k`` best scores, ties broken by key."""
    if k <= 0:
        raise ValueError(f"k must be positive, got {k}")
    if len(scores) != len(keys):
        raise ValueError("scores and keys have different lengths")
    if len(scores) == 0:
        return np.empty(0, dtype=np.int64)
    order = np.lexsort((keys, -scores))
    return order[: min(k, len(order))]


def recall_at_k(reference, candidate, k: int = K) -> float:
    """Fraction of the ``k`` reference keys found in the ``k`` candidate keys."""
    if k <= 0:
        raise ValueError(f"k must be positive, got {k}")
    expected = list(reference)[:k]
    if not expected:
        raise ValueError("the reference ranking is empty")
    found = set(list(candidate)[:k])
    return sum(1 for key in expected if key in found) / len(expected)


def document_metrics(reference_docs, candidate_docs, expected_docs, k: int = K) -> dict:
    """Compares two rankings against the expected documents of a query.

    Returns ``hit`` (at least one expected document in the top-k),
    ``recall`` (share of the expected documents in the top-k) and
    ``agreement`` (share of the reference top-k documents also in the candidate
    top-k). The last one does not need ``expected``.
    """
    expected = set(expected_docs)
    reference = [doc for doc in list(reference_docs)[:k] if doc in expected]
    candidate = [doc for doc in list(candidate_docs)[:k] if doc in expected]
    candidate_all = set(list(candidate_docs)[:k])
    return {
        "hit": bool(candidate),
        "recall": len(set(candidate)) / len(expected) if expected else 0.0,
        "agreement": (
            sum(1 for doc in list(reference_docs)[:k] if doc in candidate_all) / k
        ),
        "reference_hit": bool(reference),
    }
