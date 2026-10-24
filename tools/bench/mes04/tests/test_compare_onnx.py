# SPDX-License-Identifier: Apache-2.0

"""Tests of the MES-04 cross-check between the Rust harness and onnxruntime."""

from __future__ import annotations

import json

import numpy as np
import pytest

import compare_onnx

#: Minimal fast-tokenizer file: whitespace WordLevel, truncation declared at four
#: tokens, so a longer input proves whether the truncation was lifted.
TOKENIZER_CONFIG = {
    "version": "1.0",
    "truncation": {
        "max_length": 4,
        "strategy": "LongestFirst",
        "stride": 0,
        "direction": "Right",
    },
    "padding": None,
    "added_tokens": [],
    "normalizer": None,
    "pre_tokenizer": {"type": "Whitespace"},
    "post_processor": None,
    "decoder": None,
    "model": {
        "type": "WordLevel",
        "vocab": {"[UNK]": 0, "a": 1, "b": 2},
        "unk_token": "[UNK]",
    },
}


def test_load_tokenizer_lifts_the_declared_truncation(tmp_path):
    tokenizers = pytest.importorskip("tokenizers")
    path = tmp_path / "tokenizer.json"
    path.write_text(json.dumps(TOKENIZER_CONFIG), encoding="utf-8")
    text = "a b a b a b a b"
    declared = tokenizers.Tokenizer.from_file(str(path))
    assert len(declared.encode(text).ids) == 4, "the fixture declares truncation"

    tokenizer = compare_onnx.load_tokenizer(path)

    assert tokenizer.truncation is None
    assert tokenizer.encode(text).ids == [1, 2, 1, 2, 1, 2, 1, 2]


def test_text_by_id_reads_the_reduced_query_file(tmp_path):
    path = tmp_path / "query-texts.jsonl"
    path.write_text(
        '{"id": "Q01", "text": "premier"}\n\n{"id": "Q02", "text": "second"}\n',
        encoding="utf-8",
    )

    assert compare_onnx.text_by_id(path) == {"Q01": "premier", "Q02": "second"}


def test_text_by_id_refuses_a_duplicate_id(tmp_path):
    path = tmp_path / "query-texts.jsonl"
    path.write_text(
        '{"id": "Q01", "text": "a"}\n{"id": "Q01", "text": "b"}\n',
        encoding="utf-8",
    )

    with pytest.raises(ValueError, match="duplicate"):
        compare_onnx.text_by_id(path)


def test_text_by_id_refuses_a_file_without_a_query(tmp_path):
    path = tmp_path / "query-texts.jsonl"
    path.write_text("\n", encoding="utf-8")

    with pytest.raises(ValueError, match="no query"):
        compare_onnx.text_by_id(path)


def test_compare_rows_matches_identical_vectors_and_measures_a_shift():
    first = np.asarray([1.0, 0.0, 0.0], dtype=np.float32)
    second = np.asarray([0.0, 1.0, 0.0], dtype=np.float32)
    left = {"Q01": first, "Q02": second}
    right = {"Q01": first, "Q02": np.asarray([1.0, 1.0, 0.0], dtype=np.float32)}

    cosines = compare_onnx.compare_rows(left, right)

    assert cosines["Q01"] == pytest.approx(1.0)
    assert cosines["Q02"] == pytest.approx(2.0 ** -0.5)


def test_compare_rows_ignores_ids_present_on_one_side_only():
    vector = np.asarray([1.0, 0.0], dtype=np.float32)

    cosines = compare_onnx.compare_rows(
        {"Q01": vector, "Q02": vector}, {"Q01": vector}
    )

    assert set(cosines) == {"Q01"}


def test_compare_rows_refuses_two_sides_without_a_common_id():
    vector = np.asarray([1.0, 0.0], dtype=np.float32)

    with pytest.raises(ValueError, match="share no query id"):
        compare_onnx.compare_rows({"Q01": vector}, {"Q02": vector})


def test_main_reports_a_missing_query_text_as_one_line(tmp_path, capsys):
    data = tmp_path / "data"
    data.mkdir()
    model = tmp_path / "model"
    model.mkdir()
    (model / "model.json").write_text(
        json.dumps(
            {
                "files": {
                    "fp32": {"path": "model.onnx"},
                    "int8": {"path": "model.int8.onnx"},
                    "tokenizer": {"path": "tokenizer.json"},
                },
                "prompts": {"query": "query: "},
                "max_tokens": 8192,
            }
        ),
        encoding="utf-8",
    )
    (data / "rust-vectors.json").write_text(
        json.dumps({"ids": ["Q01"]}), encoding="utf-8"
    )
    for name in ("vectors-int8.f32", "vectors-fp32.f32"):
        (data / name).write_bytes(np.zeros(compare_onnx.analysis.DIMS, dtype="<f4").tobytes())
    (data / "query-texts.jsonl").write_text(
        '{"id": "Q02", "text": "autre"}\n', encoding="utf-8"
    )

    status = compare_onnx.main(["--data", str(data), "--model", str(model)])

    assert status == 1
    err = capsys.readouterr().err
    assert err.startswith("error: ")
    assert "Q01" in err
    assert err.count("\n") == 1
