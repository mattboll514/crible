# SPDX-License-Identifier: Apache-2.0
"""Out-of-engine centering harness of MES-12 (ADR-0076, decision 3).

The harness rebuilds the binary codes in RAM from the `fp16` reference vectors of
each measurement index, then runs the BINARY step itself: Hamming distance over
the pool, the ``N`` closest passages, and the engine's exact rescoring over those
candidates. The rescoring is the engine's own definition (descending cosine,
ties on the ascending ``doc_id#ord`` key), so the only difference with the
engine is where the codes come from.

Three variants are measured on the same queries and the same exact reference:

- ``raw``: the stored sign, bit `i` is 1 when `x_i > 0`. It is **not** a variant
  of the measure: it is the control of the harness. The harness must reproduce
  the `codes.bin` of every segment bit for bit, and the engine's top-10 BINARY
  for every query and every `N`; otherwise no figure is kept (ADR-0076).
- ``a``: the mean of every live passage of the index is subtracted from the
  document vectors **and** from the query vector, then the sign is taken, without
  renormalisation. This is the principal centering variant.
- ``c``: the documents are centred, the query is not. Reported as an indication.

Centering changes the codes only. The rescoring keeps the stored vectors and the
raw query, exactly as the engine does, so the comparison with EXACT stays the
comparison of the scan.

Run from the repository root::

    (cd tools && uv run --no-sync python bench/mes01/mes01_centering.py)
"""

from __future__ import annotations

import argparse
import json
import sqlite3
import sys
from pathlib import Path

import numpy as np

import mes01_metrics as metrics

#: Repository root, three levels above this file.
ROOT = Path(__file__).resolve().parents[3]
#: Structures of the sample, in the order of `docs/08`.
STRUCTURES = ("articles", "decisions", "dole", "jorf", "sarde", "titres")
#: Structures of the filtered campaign (ADR-0076, decision 5).
FILTERED_STRUCTURES = ("articles", "decisions")
#: Values of `search.rescore_candidates` measured here.
DEFAULT_NS = (200, 500, 1000, 2000, 5000)
#: Bits one code word holds, and words one code holds for 768 dimensions.
BITS_PER_WORD = 64
#: Rows per block when a full matrix is walked, so the peak memory stays low.
BLOCK = 8192
#: Popcount of every byte value.
POPCOUNT = np.array([bin(value).count("1") for value in range(256)], dtype=np.uint8)


def words_per_code(dims: int) -> int:
    """Number of `u64` words one code holds."""
    return (dims + BITS_PER_WORD - 1) // BITS_PER_WORD


def codes_from_sign(bits: np.ndarray) -> np.ndarray:
    """Packs a boolean `(rows, dims)` matrix into `u64` codes, little-endian.

    The code is padded to whole `u64` words and the bits above `dims` stay zero,
    as `codes.bin` requires (ADR-0069, decision 6).
    """
    dims = bits.shape[1]
    padded = np.zeros((bits.shape[0], words_per_code(dims) * BITS_PER_WORD), dtype=bool)
    padded[:, :dims] = bits
    packed = np.packbits(padded, axis=1, bitorder="little")
    return np.ascontiguousarray(packed).view("<u8")


def read_codes(path: Path, count: int, dims: int) -> np.ndarray:
    """Reads a `codes.bin` of `count` codes of `dims` dimensions."""
    codes = np.fromfile(path, dtype="<u8")
    expected = count * words_per_code(dims)
    if codes.size != expected:
        raise SystemExit(f"{path} holds {codes.size} words, not {expected}")
    return codes.reshape(count, words_per_code(dims))


def read_vectors(path: Path, dims: int) -> np.ndarray:
    """Reads the `fp16` reference matrix, widened to `f32` exactly."""
    raw = np.fromfile(path, dtype="<f2")
    if raw.size % dims != 0:
        raise SystemExit(f"{path} holds {raw.size} halves, not whole {dims}-row vectors")
    return raw.reshape(-1, dims).astype(np.float32)


def column_mean(matrix: np.ndarray) -> np.ndarray:
    """Mean of every column, accumulated in `float64`."""
    total = np.zeros(matrix.shape[1], dtype=np.float64)
    for start in range(0, matrix.shape[0], BLOCK):
        total += matrix[start : start + BLOCK].astype(np.float64).sum(axis=0)
    return total / matrix.shape[0]


def centred_codes(matrix: np.ndarray, mean: np.ndarray) -> np.ndarray:
    """Codes of `matrix - mean`, sign taken without renormalisation."""
    bits = np.empty(matrix.shape, dtype=bool)
    for start in range(0, matrix.shape[0], BLOCK):
        block = matrix[start : start + BLOCK].astype(np.float64) - mean
        bits[start : start + BLOCK] = block > 0
    return codes_from_sign(bits)


def query_codes(vector: np.ndarray, mean: np.ndarray | None = None) -> np.ndarray:
    """Codes of one normalised query, optionally centred by `mean`."""
    values = vector.astype(np.float64)
    if mean is not None:
        values = values - mean
    return codes_from_sign((values > 0)[None, :])[0]


def normalise_query(vector: np.ndarray) -> np.ndarray:
    """Normalises the query as `dense::normalise` does: `f32`, term by term."""
    squares = np.float32(0.0)
    for component in vector:
        squares = np.float32(squares + np.float32(component) * np.float32(component))
    norm = np.float32(np.sqrt(squares))
    if not np.isfinite(norm) or norm == 0:
        raise SystemExit("the query has no defined cosine")
    return (vector.astype(np.float32) / norm).astype(np.float32)


def rank(codes: np.ndarray, query_code: np.ndarray, pool: np.ndarray) -> np.ndarray:
    """Pool identifiers ordered by ascending distance, then ascending identifier."""
    chosen = codes[pool]
    xor = np.bitwise_xor(chosen, query_code[None, :])
    distance = POPCOUNT[np.ascontiguousarray(xor).view(np.uint8)].sum(axis=1, dtype=np.uint64)
    return pool[np.lexsort((pool, distance))]


def rescore(
    matrix: np.ndarray,
    keys: list[str],
    candidates: np.ndarray,
    query: np.ndarray,
    k: int,
) -> list[str]:
    """Top-k of the candidates by cosine, as the engine ranks them."""
    products = matrix[candidates].astype(np.float64)
    scores = (products @ query.astype(np.float64)).astype(np.float32)
    ordered = sorted(
        zip(scores.tolist(), (keys[index] for index in candidates)),
        key=lambda pair: (-pair[0], pair[1]),
    )
    return [key for _, key in ordered[:k]]


def read_passages(sqlite_path: Path) -> tuple[list[str], np.ndarray, int]:
    """Returns the key of every internal identifier, its `doc_id`, and dead count."""
    connection = sqlite3.connect(f"file:{sqlite_path}?mode=ro", uri=True)
    try:
        rows = connection.execute(
            "SELECT iid, doc_id, ord, dead_since_gen FROM passages ORDER BY iid"
        ).fetchall()
    finally:
        connection.close()
    keys: list[str] = []
    doc_ids: list[str] = []
    dead = 0
    for position, (iid, doc_id, ordinal, dead_since) in enumerate(rows):
        if iid != position:
            raise SystemExit(f"{sqlite_path} skips internal identifier {position}")
        keys.append(f"{doc_id}#{ordinal}")
        doc_ids.append(doc_id)
        if dead_since is not None:
            dead += 1
    return keys, np.array(doc_ids, dtype=object), dead


def read_pool(sqlite_path: Path, structure: str, filter_value: dict | None) -> np.ndarray:
    """Internal identifiers of the live passages the synthetic filter keeps."""
    if filter_value is None:
        return None
    connection = sqlite3.connect(f"file:{sqlite_path}?mode=ro", uri=True)
    try:
        if structure == "articles":
            rows = connection.execute(
                "SELECT p.iid FROM passages p JOIN documents d ON d.doc_id = p.doc_id "
                "WHERE json_extract(d.fields, '$.etat') = ? AND p.dead_since_gen IS NULL "
                "ORDER BY p.iid",
                (filter_value["etat"],),
            ).fetchall()
        elif structure == "decisions":
            rows = connection.execute(
                "SELECT iid FROM passages WHERE json_extract(fields, '$.partie') = ? "
                "AND dead_since_gen IS NULL ORDER BY iid",
                (filter_value["partie"],),
            ).fetchall()
        else:
            raise SystemExit(f"no synthetic filter is declared for {structure}")
    finally:
        connection.close()
    return np.array([row[0] for row in rows], dtype=np.uint64)


def current_epoch(index_dir: Path) -> int:
    """Reads ``CURRENT_EPOCH`` and checks that the epoch it names exists."""
    raw = (index_dir / "CURRENT_EPOCH").read_text(encoding="utf-8").strip()
    if not raw.isdigit():
        raise SystemExit(f"{index_dir}/CURRENT_EPOCH does not name an epoch: {raw!r}")
    epoch = int(raw)
    if not (index_dir / "epochs" / str(epoch)).is_dir():
        raise SystemExit(f"{index_dir}/epochs/{epoch} is missing")
    return epoch


def single_segment(index_dir: Path, epoch: int) -> Path:
    """Returns the only segment of the epoch, refusing a multi-segment index.

    The harness reads the whole ``codes.bin`` and the whole ``vectors.f16`` of one
    segment and treats them as the whole index. Reading one segment of several
    would silently measure a truncated index, so the case is refused rather than
    guessed.
    """
    segments = sorted((index_dir / "epochs" / str(epoch) / "segments").glob("seg-*"))
    if len(segments) != 1:
        raise SystemExit(
            f"{index_dir}: the harness reads one segment, this epoch has "
            f"{len(segments)}; refusing to read a partial index"
        )
    return segments[0]


def read_jsonl(path: Path) -> list[dict]:
    """Returns the JSON objects of a JSONL file, in order."""
    with path.open(encoding="utf-8") as handle:
        return [json.loads(line) for line in handle if line.strip()]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", type=Path, default=ROOT / "data/mes01")
    parser.add_argument("--ns", type=int, nargs="+", default=list(DEFAULT_NS))
    parser.add_argument("--k", type=int, default=metrics.K)
    parser.add_argument(
        "--campaigns", nargs="+", choices=("main", "filtered"), default=["main", "filtered"]
    )
    parser.add_argument("--out", type=Path, default=None)
    arguments = parser.parse_args()
    out = arguments.out or arguments.data / "centering.json"

    texts = read_jsonl(arguments.data / "query-texts.jsonl")
    row_of = {record["id"]: row for row, record in enumerate(texts)}
    matrix = np.fromfile(arguments.data / "vectors-fp32.f32", dtype="<f4").reshape(-1, 768)
    if matrix.shape[0] != len(texts):
        raise SystemExit("the query matrix and the query list disagree")

    report: dict = {
        "variants": {
            "raw": "stored sign, control of the harness",
            "a": "documents and query centred on the mean of the live passages, no renormalisation",
            "c": "documents centred, query not centred (indication)",
        },
        "ns": arguments.ns,
        "k": arguments.k,
        "campaigns": {},
    }

    for campaign in arguments.campaigns:
        rows = read_jsonl(arguments.data / f"queries-{campaign}.jsonl")
        structures = STRUCTURES if campaign == "main" else FILTERED_STRUCTURES
        campaign_report: dict = {}
        for structure in structures:
            selected = [record for record in rows if record["structure"] == structure]
            if not selected:
                continue
            index_dir = arguments.data / "indexes" / structure
            epoch = current_epoch(index_dir)
            segment = single_segment(index_dir, epoch)
            sqlite_path = index_dir / "epochs" / str(epoch) / "records.sqlite"
            vectors = read_vectors(segment / "vectors.f16", 768)
            count, dims = vectors.shape
            keys, doc_ids, dead = read_passages(sqlite_path)
            iid_of = {key: iid for iid, key in enumerate(keys)}
            if dead:
                raise SystemExit(f"{structure} holds {dead} dead passages; the sample has none")
            stored = read_codes(segment / "codes.bin", count, dims)

            recomputed = codes_from_sign(vectors > 0)
            mismatched_words = int((recomputed != stored).sum())
            if mismatched_words:
                raise SystemExit(
                    f"{structure}: the harness reproduces {mismatched_words} words of "
                    "codes.bin differently; no centering figure is kept (ADR-0076)"
                )

            mean = column_mean(vectors)
            documents_centred = centred_codes(vectors, mean)
            pool = read_pool(sqlite_path, structure, selected[0]["filter"])
            if pool is None:
                pool = np.arange(count, dtype=np.uint64)
            else:
                pool = pool.astype(np.uint64)

            exact = json.loads(
                (arguments.data / f"engine/{campaign}/{structure}/exact.json").read_text()
            )
            exact_by_id = {record["id"]: record for record in exact["results"]}

            structure_report = {
                "passages": count,
                "pool": int(pool.size),
                "dead": dead,
                "codes_control": {
                    "words": int(stored.size),
                    "mismatched_words": mismatched_words,
                    "bits": count * dims,
                },
                "mean_norm": float(np.linalg.norm(mean)),
                "control_comparisons": len(selected) * len(arguments.ns),
                "residuals": [],
                "queries": {},
            }

            per_query: dict[str, dict] = {}
            for record in selected:
                query_id = record["id"]
                vector = matrix[row_of[query_id]]
                normalised = normalise_query(vector)
                codes = {
                    "raw": query_codes(normalised),
                    "a": query_codes(normalised, mean),
                    "c": query_codes(normalised),
                }
                documents = {"raw": stored, "a": documents_centred, "c": documents_centred}
                exact_record = exact_by_id[query_id]
                entry: dict = {}
                for variant in ("raw", "a", "c"):
                    order = rank(documents[variant], codes[variant], pool)
                    by_n = {}
                    for n in arguments.ns:
                        candidates = order[:n]
                        top = rescore(vectors, keys, candidates, normalised, arguments.k)
                        top_documents = [key.split("#")[0] for key in top]
                        binary_record = {
                            "results": [
                                {"key": key, "doc_id": doc}
                                for key, doc in zip(top, top_documents)
                            ]
                        }
                        by_n[str(n)] = {
                            "recall": metrics.recall_at_10(binary_record, exact_record),
                            "overlap": metrics.overlap_at_10(binary_record, exact_record),
                        }
                        if variant == "raw":
                            engine_record = json.loads(
                                (
                                    arguments.data
                                    / f"engine/{campaign}/{structure}/n{n}.json"
                                ).read_text()
                            )
                            engine_by_id = {
                                row["id"]: row for row in engine_record["results"]
                            }
                            if top != metrics.passage_keys(engine_by_id[query_id]):
                                raise SystemExit(
                                    f"{campaign}/{structure}: query {query_id} at N={n} "
                                    "does not reproduce the engine's top-10; no figure "
                                    "is kept (ADR-0076, decision 3)"
                                )
                            if campaign == "main" and n == max(arguments.ns):
                                missing = [
                                    key
                                    for key in metrics.passage_keys(exact_record)
                                    if key not in top
                                ]
                                if missing:
                                    structure_report["residuals"].append(
                                        {
                                            "id": query_id,
                                            "missing": len(missing),
                                            "ranks": sorted(
                                                # 0-based: the number of pool passages
                                                # strictly closer, which is the index the
                                                # scan keeps from 0 to N-1.
                                                int(np.flatnonzero(order == iid_of[key])[0])
                                                for key in missing
                                            ),
                                        }
                                    )
                    entry[variant] = by_n
                per_query[query_id] = entry
            structure_report["queries"] = per_query
            structure_report["control_ok"] = True
            campaign_report[structure] = structure_report
            print(
                f"{campaign:8s} {structure:10s} {count:6d} passages, pool {pool.size:6d}, "
                f"control ok on {structure_report['control_comparisons']} comparisons",
                flush=True,
            )
        report["campaigns"][campaign] = campaign_report

    out.write_text(json.dumps(report) + "\n", encoding="utf-8")
    print("wrote", out)


if __name__ == "__main__":
    sys.exit(main())
