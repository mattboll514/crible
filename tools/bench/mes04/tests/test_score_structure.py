# SPDX-License-Identifier: Apache-2.0

"""Tests of the MES-04 ranking, sample loading and aggregation functions."""

from __future__ import annotations

import numpy as np
import pyarrow as pa
import pyarrow.parquet as pq
import pytest

import analysis
import analyze

DIMS = analysis.DIMS


def _unit_vector(seed: int) -> np.ndarray:
    generator = np.random.default_rng(seed)
    vector = generator.normal(size=DIMS)
    return vector / np.linalg.norm(vector)


def _write_passage_file(path, rows) -> None:
    """Writes ``(doc_id, ord, vector)`` rows with the sample column types."""
    table = pa.table(
        {
            "doc_id": pa.array([row[0] for row in rows], type=pa.string()),
            "ord": pa.array([row[1] for row in rows], type=pa.uint32()),
            "vector": pa.array(
                [np.asarray(row[2], dtype=np.float16).astype("<f2").tobytes() for row in rows],
                type=pa.binary(2 * DIMS),
            ),
        }
    )
    pq.write_table(table, path)


def test_load_passages_reads_every_part_in_file_order(tmp_path):
    """The order is the one ``crible attach`` gives: file name, then row."""
    batch = tmp_path / "batches" / "legifacile-sample-alpha-v1"
    batch.mkdir(parents=True)
    first = _unit_vector(1)
    second = _unit_vector(2)
    third = _unit_vector(3)
    _write_passage_file(batch / "passages-0001.parquet", [("a", 0, first), ("a", 1, second)])
    _write_passage_file(batch / "passages-0002.parquet", [("b", 0, third)])

    vectors, keys, docs = analysis.load_passages(tmp_path, "alpha")

    assert vectors.shape == (3, DIMS)
    assert vectors.dtype == np.float32
    expected = np.asarray(
        [first, second, third], dtype=np.float16
    ).astype(np.float32)
    assert np.array_equal(vectors, expected)
    assert keys.tolist() == ["a#0", "a#1", "b#0"]
    assert docs.tolist() == ["a", "a", "b"]


def test_load_passages_refuses_a_batch_without_a_passage_file(tmp_path):
    batch = tmp_path / "batches" / "legifacile-sample-alpha-v1"
    batch.mkdir(parents=True)

    with pytest.raises(ValueError, match="no passage file"):
        analysis.load_passages(tmp_path, "alpha")


def test_load_passages_refuses_a_vector_column_of_the_wrong_width(tmp_path):
    """The size guard is the offset fix: a short column cannot be reshaped."""
    batch = tmp_path / "batches" / "legifacile-sample-alpha-v1"
    batch.mkdir(parents=True)
    table = pa.table(
        {
            "doc_id": pa.array(["a"], type=pa.string()),
            "ord": pa.array([0], type=pa.uint32()),
            "vector": pa.array(
                [b"\x00" * (2 * DIMS - 2)], type=pa.binary(2 * DIMS - 2)
            ),
        }
    )
    pq.write_table(table, batch / "passages-0001.parquet")

    with pytest.raises(ValueError, match="vector bytes"):
        analysis.load_passages(tmp_path, "alpha")


def _ranking_case():
    """Two queries over three passages, one of which the INT8 ranking loses."""
    passages = np.stack([_unit_vector(10), _unit_vector(11), _unit_vector(12)])
    fp32 = np.stack([_unit_vector(20), _unit_vector(21)])
    # The first query keeps the INT8 order; the second swaps the two best
    # passages, so its recall@1 is 0 and its recall@10 is 1.
    int8 = np.stack([fp32[0], fp32[1] + 0.5 * (passages[0] - passages[1])])
    torch = np.stack([fp32[0], fp32[1]])
    keys = np.asarray(["p0#0", "p1#0", "p2#0"])
    docs = np.asarray(["p0", "p1", "p2"])
    records = [
        {"id": "Q01", "structure": "alpha"},
        {
            "id": "Q02",
            "structure": "alpha",
            "expected_in_sample": True,
            "expected": [{"src_id": "p0"}],
        },
    ]
    positions = {record["id"]: index for index, record in enumerate(records)}
    return records, passages, keys, docs, int8, fp32, torch, positions


def test_score_structure_reports_cosines_recall_and_agreement():
    case = _ranking_case()
    records, passages, keys, docs, int8, fp32, torch, positions = case

    result = analyze.score_structure(
        records, passages, keys, docs, int8, fp32, torch, positions
    )

    assert result["queries"] == 2
    assert result["passages"] == 3
    assert result["cosine"]["int8_vs_torch_fp32"]["count"] == 2
    assert result["cosine"]["int8_vs_torch_fp32"]["max"] == pytest.approx(1.0)
    assert result["cosine"]["fp32_vs_torch_fp32"]["min"] == pytest.approx(1.0)
    assert result["recall_at_10"]["count"] == 2
    assert result["perfect_recall_queries"] >= 1
    assert result["expected"]["queries"] == 1
    assert set(result["expected"]["int8"]) == {"queries", "hit_at_10", "recall_at_10"}


def test_score_structure_refuses_expected_without_src_id():
    case = _ranking_case()
    records, passages, keys, docs, int8, fp32, torch, positions = case
    records[1]["expected"] = [{"key": "p0#0"}]

    with pytest.raises(ValueError, match="src_id"):
        analyze.score_structure(
            records, passages, keys, docs, int8, fp32, torch, positions
        )


def test_weighted_expected_weights_each_structure_by_its_queries():
    per_structure = {
        "alpha": {
            "expected": {
                "int8": {"queries": 2, "hit_at_10": 0.5, "recall_at_10": 0.5}
            }
        },
        "beta": {
            "expected": {
                "int8": {"queries": 1, "hit_at_10": 1.0, "recall_at_10": 1.0}
            }
        },
        "gamma": {"expected": {"int8": {"queries": 0}}},
    }

    weighted = analyze.weighted_expected(per_structure, "int8", "queries", 3)

    assert weighted["queries"] == 3
    assert weighted["hit_at_10"] == pytest.approx(2 / 3)
    assert weighted["recall_at_10"] == pytest.approx(2 / 3)
    assert weighted["share_of_campaign"] == pytest.approx(1.0)


def test_weighted_expected_returns_no_query_when_nothing_is_expected():
    assert analyze.weighted_expected({}, "fp32", "queries", 300) == {"queries": 0}
