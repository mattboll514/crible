# SPDX-License-Identifier: Apache-2.0

"""Tests of the centering harness of MES-12 (ADR-0076, decision 3).

The harness computes the codes, ranks by Hamming distance and rescores in RAM.
Its figures are only kept when it reproduces `codes.bin` and the engine's
top-10 exactly, so these tests fix the pieces of that computation.
"""

from __future__ import annotations

import numpy as np
import pytest

import mes01_centering as centering


def test_words_per_code_pads_to_whole_words():
    assert centering.words_per_code(1) == 1
    assert centering.words_per_code(64) == 1
    assert centering.words_per_code(65) == 2
    assert centering.words_per_code(768) == 12


def test_codes_from_sign_packs_the_positive_components_little_endian():
    bits = np.array([[True, False, False, True]], dtype=bool)
    codes = centering.codes_from_sign(bits)
    assert codes.shape == (1, 1)
    assert int(codes[0, 0]) == 0b1001


def test_codes_from_sign_keeps_dimension_order_across_words():
    bits = np.zeros((1, 65), dtype=bool)
    bits[0, 0] = True
    bits[0, 64] = True
    codes = centering.codes_from_sign(bits)
    assert [int(word) for word in codes[0]] == [1, 1]


def test_the_sign_is_strictly_positive():
    # ADR-0004: bit `i` is 1 when `x_i > 0`, so a null component sets no bit.
    vector = np.array([-1.0, 0.0, 1.0, 1e-30], dtype=np.float32)
    codes = centering.codes_from_sign((vector > 0)[None, :])
    assert int(codes[0, 0]) == 0b1100


def test_read_codes_refuses_a_wrong_length(tmp_path):
    path = tmp_path / "codes.bin"
    np.array([1, 2], dtype="<u8").tofile(path)
    with pytest.raises(SystemExit):
        centering.read_codes(path, 1, 768)


def test_rank_orders_by_distance_then_identifier():
    # Query code 0; code 1 has one bit set, code 0 none.
    codes = np.array([[0, 0], [1, 0], [0, 0], [0, 1]], dtype="<u8")
    query = np.array([0, 0], dtype="<u8")
    order = centering.rank(codes, query, np.arange(4, dtype=np.uint64))
    # Distance 0 on identifiers 0 and 2 (ascending), then distance 1 on 1 and 3.
    assert order.tolist() == [0, 2, 1, 3]


def test_rank_restricts_itself_to_the_pool():
    codes = np.array([[0], [1], [0]], dtype="<u8")
    query = np.array([0], dtype="<u8")
    order = centering.rank(codes, query, np.array([2], dtype=np.uint64))
    assert order.tolist() == [2]


def test_normalise_query_divides_by_the_euclidean_norm():
    vector = np.array([3.0, 4.0, 0.0], dtype=np.float32)
    normalised = centering.normalise_query(vector)
    assert normalised.dtype == np.float32
    assert normalised.tolist() == pytest.approx([0.6, 0.8, 0.0])
    assert float(np.linalg.norm(normalised)) == pytest.approx(1.0)


def test_normalise_query_refuses_a_zero_vector():
    with pytest.raises(SystemExit):
        centering.normalise_query(np.zeros(4, dtype=np.float32))


def test_centred_codes_flip_a_component_below_the_mean():
    mean = np.array([0.5, 0.5], dtype=np.float64)
    # 0.6 becomes 0.1 (positive), 0.4 becomes -0.1 (negative).
    bits = np.array([[0.6, 0.4], [0.4, 0.6]], dtype=np.float32)
    codes = centering.centred_codes(bits, mean)
    assert [int(word) for word in codes[0]] == [0b01]
    assert [int(word) for word in codes[1]] == [0b10]


def test_centred_codes_are_not_renormalised():
    # Centering shifts the norm of a document away from one; the code is the
    # sign only, so the norm never enters.
    mean = np.array([0.0, 2.0], dtype=np.float64)
    bits = np.array([[1.0, 1.0]], dtype=np.float32)
    codes = centering.centred_codes(bits, mean)
    assert [int(word) for word in codes[0]] == [0b01]


def test_rescore_ranks_by_cosine_and_breaks_ties_on_the_textual_key():
    matrix = np.array([[1.0, 0.0], [0.5, 0.5], [0.5, 0.5]], dtype=np.float32)
    keys = ["doc#10", "doc#2", "other#0"]
    query = np.array([1.0, 0.0], dtype=np.float32)
    top = centering.rescore(matrix, keys, np.array([0, 1, 2]), query, 3)
    # The two equal cosines are ranked by the text of the key, as in the engine.
    assert top == ["doc#10", "doc#2", "other#0"]


def test_rescore_keeps_only_k_results():
    matrix = np.eye(4, dtype=np.float32)
    keys = [f"doc{index}#0" for index in range(4)]
    query = np.array([1.0, 0.0, 0.0, 0.0], dtype=np.float32)
    assert centering.rescore(matrix, keys, np.arange(4), query, 2) == ["doc0#0", "doc1#0"]


def test_read_vectors_widens_fp16_to_fp32(tmp_path):
    path = tmp_path / "vectors.f16"
    np.array([0.5, -2.0], dtype="<f2").tofile(path)
    matrix = centering.read_vectors(path, 2)
    assert matrix.dtype == np.float32
    assert matrix.tolist() == [[0.5, -2.0]]


def test_column_mean_accumulates_every_row():
    matrix = np.array([[1.0, 3.0], [3.0, 5.0]], dtype=np.float32)
    mean = centering.column_mean(matrix)
    assert mean.dtype == np.float64
    assert mean.tolist() == pytest.approx([2.0, 4.0])
