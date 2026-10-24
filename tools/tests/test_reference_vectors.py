# SPDX-License-Identifier: Apache-2.0

"""Tests of ``tools/reference_vectors.py``.

The generator itself needs the ONNX model and the ``export`` dependency group.
These tests cover everything else without a download: the 20 reference texts
of the parity corpus, the committed fixture, and the byte layout of the file
``write_fixture`` produces.
"""

from __future__ import annotations

from pathlib import Path

import numpy as np

import export_onnx
import reference_vectors
from export_onnx import DIMS


def test_reference_texts_are_the_first_twenty_of_the_parity_corpus():
    texts = reference_vectors.reference_texts()

    assert len(texts) == reference_vectors.REFERENCE_COUNT
    assert texts == export_onnx.comparison_texts()[: reference_vectors.REFERENCE_COUNT]
    assert len(set(texts)) == len(texts), "the reference texts are distinct"


def test_committed_fixture_holds_the_generated_texts():
    fixture = reference_vectors.ROOT / reference_vectors.FIXTURE_DIR
    texts = reference_vectors.reference_texts()

    assert (fixture / "texts.txt").read_text(encoding="utf-8") == (
        "\n".join(texts) + "\n"
    )
    vectors = fixture / "vectors.f32"
    assert vectors.stat().st_size == reference_vectors.REFERENCE_COUNT * DIMS * 4


def test_write_fixture_writes_little_endian_vectors_in_text_order(tmp_path, monkeypatch):
    texts = reference_vectors.reference_texts()
    expected = [
        np.full(DIMS, float(index), dtype=np.float32)
        for index in range(reference_vectors.REFERENCE_COUNT)
    ]
    monkeypatch.setattr(
        reference_vectors.export_onnx, "load_tokenizer", lambda path: object()
    )
    monkeypatch.setattr(
        reference_vectors.export_onnx,
        "onnx_vectors",
        lambda model, tokenizer, given: expected,
    )

    digests = reference_vectors.write_fixture(tmp_path, Path("unused-model"))

    assert set(digests) == {"texts.txt", "vectors.f32"}
    assert (tmp_path / "texts.txt").read_text(encoding="utf-8") == (
        "\n".join(texts) + "\n"
    )
    written = np.fromfile(tmp_path / "vectors.f32", dtype="<f4").reshape(
        reference_vectors.REFERENCE_COUNT, DIMS
    )
    assert np.array_equal(written, np.asarray(expected))
    assert digests["vectors.f32"] != digests["texts.txt"]
