# SPDX-License-Identifier: Apache-2.0

"""Compute the MES-04 report data: cosines and exact recall@10.

Reads the vectors written by the Rust harness (``measure_queries vectors``) and
by the PyTorch reference (``torch_reference.py``), then ranks the passages of the
Legifacile sample exactly with each query precision. Writes one JSON file next
to the report; the report itself is written by hand from that JSON.

Run from ``tools/``::

    uv run python bench/mes04/analyze.py --commit <sha>
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

import numpy as np

import analysis

#: Repository root, three levels above this file.
ROOT = Path(__file__).resolve().parents[3]
DEFAULT_SAMPLE = ROOT / "data/legifacile-sample"
DEFAULT_QUERIES = DEFAULT_SAMPLE / "queries.jsonl"
DEFAULT_DATA = ROOT / "data/mes04"


def read_jsonl(path: Path) -> list[dict]:
    with path.open(encoding="utf-8") as handle:
        return [json.loads(line) for line in handle if line.strip()]


def score_structure(
    records: list[dict],
    vectors: np.ndarray,
    keys: np.ndarray,
    docs: np.ndarray,
    int8_vectors: np.ndarray,
    fp32_vectors: np.ndarray,
    torch_vectors: np.ndarray,
    positions: dict[str, int],
) -> dict:
    """Ranks the passages of one structure with both precisions."""
    recalls = []
    agreements = []
    cosines_int8 = []
    cosines_fp32 = []
    expected_int8 = []
    expected_fp32 = []
    for record in records:
        position = positions[record["id"]]
        cosines_int8.append(analysis.cosine(int8_vectors[position], torch_vectors[position]))
        cosines_fp32.append(analysis.cosine(fp32_vectors[position], torch_vectors[position]))
        int8_scores = vectors @ int8_vectors[position]
        fp32_scores = vectors @ fp32_vectors[position]
        int8_top = analysis.top_k_indices(int8_scores, keys)
        fp32_top = analysis.top_k_indices(fp32_scores, keys)
        recalls.append(
            analysis.recall_at_k(
                keys[fp32_top].tolist(), keys[int8_top].tolist(), analysis.K
            )
        )
        agreement = analysis.document_metrics(
            docs[fp32_top].tolist(), docs[int8_top].tolist(), [], analysis.K
        )
        agreements.append(agreement["agreement"])
        if record.get("expected_in_sample"):
            expected = sorted(
                {
                    item["src_id"]
                    for item in record.get("expected") or []
                    if item.get("src_id")
                }
            )
            if not expected:
                raise ValueError(
                    f"query {record['id']} claims expected_in_sample without src_id"
                )
            expected_int8.append(
                analysis.document_metrics(
                    docs[fp32_top].tolist(),
                    docs[int8_top].tolist(),
                    expected,
                    analysis.K,
                )
            )
            expected_fp32.append(
                analysis.document_metrics(
                    docs[fp32_top].tolist(),
                    docs[fp32_top].tolist(),
                    expected,
                    analysis.K,
                )
            )
    return {
        "queries": len(records),
        "passages": int(len(keys)),
        "cosine": {
            "int8_vs_torch_fp32": analysis.summarise(cosines_int8),
            "fp32_vs_torch_fp32": analysis.summarise(cosines_fp32),
        },
        "recall_at_10": analysis.summarise(recalls),
        "mean_recall_at_10": float(np.mean(recalls)),
        "perfect_recall_queries": int(sum(1 for value in recalls if value == 1.0)),
        "mean_document_agreement_at_10": float(np.mean(agreements)),
        "per_query_recall": recalls,
        "expected": {
            "queries": len(expected_int8),
            "int8": summarise_expected(expected_int8),
            "fp32": summarise_expected(expected_fp32),
        },
    }


def summarise_expected(rows: list[dict]) -> dict:
    if not rows:
        return {"queries": 0}
    return {
        "queries": len(rows),
        "hit_at_10": float(np.mean([row["hit"] for row in rows])),
        "recall_at_10": float(np.mean([row["recall"] for row in rows])),
    }


def fidelity_block(report: dict, revision: str) -> dict:
    """Extracts the Q-06 evidence from a ``fidelity.json`` report.

    The block is copied into the MES-04 raw data so the ``acceptance_min_cosine``
    proposal cites a distribution measured in the same campaign, against the
    same pinned graph revision.
    """
    if report["manifest"]["revision"] != revision:
        raise ValueError("fidelity.json used another model revision")
    return {
        "passages": report["passages"],
        "per_structure": report["per_structure"],
        "structures": report["structures"],
        "min_cosine": report["min_cosine"],
        "p1_cosine": report["p1_cosine"],
        "median_cosine": report["median_cosine"],
        "min_cosine_required": report["min_cosine_required"],
        "fp32_blake3": report["graph"]["fp32_blake3"],
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--sample", type=Path, default=DEFAULT_SAMPLE)
    parser.add_argument("--queries", type=Path, default=DEFAULT_QUERIES)
    parser.add_argument("--data", type=Path, default=DEFAULT_DATA)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--commit", default="unknown")
    parser.add_argument("--date", default="")
    parser.add_argument("--machine", default="")
    parser.add_argument("--model", type=Path, default=ROOT / "data/models/mdenseon")
    parser.add_argument(
        "--fidelity",
        type=Path,
        default=None,
        help="fidelity.json written by bench/mes04/fidelity.py (Q-06 evidence)",
    )
    arguments = parser.parse_args(argv)

    try:
        records = read_jsonl(arguments.queries)
        torch_meta = json.loads(
            (arguments.data / "torch-vectors.json").read_text(encoding="utf-8")
        )
        rust_meta = json.loads(
            (arguments.data / "rust-vectors.json").read_text(encoding="utf-8")
        )
        fidelity = None
        if arguments.fidelity is not None:
            model_config = json.loads(
                (arguments.model / "model.json").read_text(encoding="utf-8")
            )
            fidelity = fidelity_block(
                json.loads(arguments.fidelity.read_text(encoding="utf-8")),
                model_config["revision"],
            )
        count = len(records)
        ids = [record["id"] for record in records]
        for name, meta in (("torch", torch_meta), ("rust", rust_meta)):
            if meta["ids"] != ids:
                raise ValueError(f"{name} vectors are not in the query order")
        int8 = analysis.load_f32_matrix(
            arguments.data / "vectors-int8.f32", count, analysis.DIMS
        )
        fp32 = analysis.load_f32_matrix(
            arguments.data / "vectors-fp32.f32", count, analysis.DIMS
        )
        torch_vectors = analysis.load_f32_matrix(
            arguments.data / "torch-fp32.f32", count, analysis.DIMS
        )

        cosines_int8 = [
            analysis.cosine(int8[index], torch_vectors[index])
            for index in range(count)
        ]
        cosines_fp32 = [
            analysis.cosine(fp32[index], torch_vectors[index])
            for index in range(count)
        ]
        cosines_cross = [
            analysis.cosine(int8[index], fp32[index]) for index in range(count)
        ]

        positions = {record["id"]: index for index, record in enumerate(records)}
        per_structure = {}
        for structure in analysis.STRUCTURES:
            subset = [record for record in records if record["structure"] == structure]
            if not subset:
                continue
            print(f"ranking {len(subset)} queries on {structure}", file=sys.stderr)
            vectors, keys, docs = analysis.load_passages(arguments.sample, structure)
            per_structure[structure] = score_structure(
                subset, vectors, keys, docs, int8, fp32, torch_vectors, positions
            )
            del vectors, keys, docs

        total_queries = sum(item["queries"] for item in per_structure.values())
        if total_queries != count:
            raise ValueError(
                f"{count} queries but {total_queries} ranked: an unknown structure?"
            )
        all_recalls = [
            value
            for item in per_structure.values()
            for value in item["per_query_recall"]
        ]
        global_agreement = (
            sum(
                item["mean_document_agreement_at_10"] * item["queries"]
                for item in per_structure.values()
            )
            / count
        )

        payload = {
            "date": arguments.date,
            "commit": arguments.commit,
            "machine": arguments.machine,
            "task": "MESURER-PARITE-ENCODEUR",
            "queries": count,
            "k": analysis.K,
            "prompt": "query: ",
            "sample": {
                "name": "legifacile-sample",
                "queries_sha256": hashlib.sha256(
                    arguments.queries.read_bytes()
                ).hexdigest(),
                "structures": {
                    structure: item["passages"]
                    for structure, item in per_structure.items()
                },
            },
            "model": json.loads(
                (arguments.model / "model.json").read_text(encoding="utf-8")
            ),
            "truncated": {
                "int8": int(sum(rust_meta["truncated_int8"])),
                "fp32": int(sum(rust_meta["truncated_fp32"])),
            },
            "token_lengths": torch_meta["token_lengths"],
            "cosine": {
                "int8_vs_torch_fp32": analysis.summarise(cosines_int8),
                "fp32_vs_torch_fp32": analysis.summarise(cosines_fp32),
                "int8_vs_fp32": analysis.summarise(cosines_cross),
            },
            "per_query_cosine": {
                "ids": ids,
                "int8_vs_torch_fp32": cosines_int8,
                "fp32_vs_torch_fp32": cosines_fp32,
            },

            "exact_recall": {
                "global": {
                    "queries": count,
                    "mean_recall_at_10": float(np.mean(all_recalls)),
                    "perfect_recall_queries": int(
                        sum(1 for value in all_recalls if value == 1.0)
                    ),
                    "mean_document_agreement_at_10": float(global_agreement),
                },
                "per_structure": per_structure,
                "expected": {
                    "int8": weighted_expected(
                        per_structure, "int8", "queries", count
                    ),
                    "fp32": weighted_expected(
                        per_structure, "fp32", "queries", count
                    ),
                },
            },
        }
        if fidelity is not None:
            payload["document_fidelity"] = fidelity
        arguments.out.parent.mkdir(parents=True, exist_ok=True)
        arguments.out.write_text(
            json.dumps(payload, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )
    except Exception as error:  # noqa: BLE001 - every failure becomes one line
        print(f"error: {' '.join(str(error).split())}", file=sys.stderr)
        return 1

    print(
        f"cosine int8/torch min {payload['cosine']['int8_vs_torch_fp32']['min']:.9f}, "
        f"fp32/torch min {payload['cosine']['fp32_vs_torch_fp32']['min']:.9f}"
    )
    print(
        f"recall@10 int8/fp32 {payload['exact_recall']['global']['mean_recall_at_10']:.6f} "
        f"over {count} queries"
    )
    print(f"wrote {arguments.out}")
    return 0

def weighted_expected(per_structure: dict, precision: str, key: str, total: int) -> dict:
    """Aggregates the expected-document metrics over the structures."""
    rows = [
        item["expected"][precision]
        for item in per_structure.values()
        if item["expected"][precision].get("queries")
    ]
    queries = sum(row["queries"] for row in rows)
    if not queries:
        return {"queries": 0}
    return {
        "queries": queries,
        "hit_at_10": sum(row["hit_at_10"] * row["queries"] for row in rows) / queries,
        "recall_at_10": sum(row["recall_at_10"] * row["queries"] for row in rows)
        / queries,
        "share_of_campaign": queries / total,
    }


if __name__ == "__main__":
    raise SystemExit(main())
