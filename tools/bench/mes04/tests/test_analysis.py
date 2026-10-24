# SPDX-License-Identifier: Apache-2.0

"""Tests of the MES-04 computation module."""

from __future__ import annotations

import numpy as np
import pytest

import analysis
import analyze


def test_percentile_uses_nearest_rank():
    values = [4.0, 1.0, 3.0, 2.0]
    assert analysis.percentile(values, 1) == 1.0
    assert analysis.percentile(values, 50) == 2.0
    assert analysis.percentile(values, 75) == 3.0
    assert analysis.percentile(values, 100) == 4.0


def test_percentile_rejects_empty_and_out_of_range():
    with pytest.raises(ValueError):
        analysis.percentile([], 50)
    with pytest.raises(ValueError):
        analysis.percentile([1.0], 0)
    with pytest.raises(ValueError):
        analysis.percentile([1.0], 101)


def test_summarise_reports_the_required_columns():
    summary = analysis.summarise([1.0, 2.0, 3.0])
    assert summary["count"] == 3
    assert summary["min"] == 1.0
    assert summary["p50"] == 2.0
    assert summary["max"] == 3.0
    assert summary["mean"] == pytest.approx(2.0)


def test_cosine_of_identical_and_orthogonal_vectors():
    assert analysis.cosine([1.0, 2.0], [1.0, 2.0]) == pytest.approx(1.0)
    assert analysis.cosine([1.0, 0.0], [0.0, 1.0]) == pytest.approx(0.0)


def test_cosine_rejects_a_zero_vector_and_a_shape_mismatch():
    with pytest.raises(ValueError):
        analysis.cosine([0.0, 0.0], [1.0, 0.0])
    with pytest.raises(ValueError):
        analysis.cosine([1.0], [1.0, 0.0])


def test_load_f32_matrix_reads_little_endian_rows(tmp_path):
    path = tmp_path / "vectors.f32"
    values = np.asarray([[1.0, 2.0], [3.0, 4.0]], dtype="<f4")
    path.write_bytes(values.tobytes(order="C"))
    matrix = analysis.load_f32_matrix(path, 2, 2)
    assert matrix.dtype == np.float32
    assert matrix.tolist() == [[1.0, 2.0], [3.0, 4.0]]


def test_load_f32_matrix_rejects_a_wrong_size(tmp_path):
    path = tmp_path / "vectors.f32"
    path.write_bytes(b"\x00" * 8)
    with pytest.raises(ValueError):
        analysis.load_f32_matrix(path, 2, 2)


def test_top_k_breaks_ties_by_key():
    scores = np.asarray([1.0, 1.0, 2.0], dtype=np.float32)
    keys = np.asarray(["b", "a", "c"])
    assert analysis.top_k_indices(scores, keys, 1).tolist() == [2]
    assert analysis.top_k_indices(scores, keys, 2).tolist() == [2, 1]
    assert analysis.top_k_indices(scores, keys, 3).tolist() == [2, 1, 0]


def test_top_k_returns_everything_when_k_exceeds_the_pool():
    scores = np.asarray([0.5, 0.9])
    keys = np.asarray(["a#0", "b#0"])
    assert len(analysis.top_k_indices(scores, keys, 10)) == 2


def test_recall_at_k_counts_the_overlap():
    assert analysis.recall_at_k(["a", "b", "c"], ["a", "b", "c"]) == 1.0
    assert analysis.recall_at_k(["a", "b", "c"], ["a", "x", "y"]) == pytest.approx(1 / 3)
    assert analysis.recall_at_k(["a"], ["x"]) == 0.0


def test_recall_at_k_rejects_an_empty_reference():
    with pytest.raises(ValueError):
        analysis.recall_at_k([], ["a"])


def test_document_metrics_hit_recall_and_agreement():
    metrics = analysis.document_metrics(
        ["d1", "d2", "d3"], ["d2", "d9", "d1"], ["d1", "d2"], 3
    )
    assert metrics["hit"] is True
    assert metrics["recall"] == pytest.approx(1.0)
    assert metrics["agreement"] == pytest.approx(2 / 3)
    assert metrics["reference_hit"] is True


def test_document_metrics_without_expected_documents():
    metrics = analysis.document_metrics(["d1"], ["d1"], [], 10)
    assert metrics["hit"] is False
    assert metrics["recall"] == 0.0


def test_fidelity_block_keeps_the_q06_evidence():
    report = {
        "passages": 600,
        "per_structure": 100,
        "structures": ["articles", "decisions"],
        "min_cosine": 0.999960398,
        "p1_cosine": 0.999993504,
        "median_cosine": 0.999998337,
        "min_cosine_required": 0.999,
        "manifest": {"revision": "abc"},
        "graph": {"fp32_blake3": "def"},
    }
    block = analyze.fidelity_block(report, "abc")
    assert block["passages"] == 600
    assert block["min_cosine"] == pytest.approx(0.999960398)
    assert block["fp32_blake3"] == "def"
    assert "manifest" not in block


def test_fidelity_block_rejects_another_revision():
    report = {
        "passages": 1,
        "per_structure": 1,
        "structures": ["articles"],
        "min_cosine": 1.0,
        "p1_cosine": 1.0,
        "median_cosine": 1.0,
        "min_cosine_required": 0.999,
        "manifest": {"revision": "other"},
        "graph": {"fp32_blake3": "def"},
    }
    with pytest.raises(ValueError):
        analyze.fidelity_block(report, "abc")
