# SPDX-License-Identifier: Apache-2.0

"""Tests of ``tools/fake_encoder.py``.

The generator writes the versioned fixture of ``ENCODER-REQUETES``. These tests
check the tokenizer configuration, the committed hashes and, when the
``export`` group provides ``onnx``, that the script reproduces the fixture byte
for byte. CI runs them without ``onnx``: only the regeneration test is skipped.
"""

from __future__ import annotations

import json

import blake3
import pytest

import fake_encoder


def test_tokenizer_keeps_the_prompt_as_one_token():
    tokenizer = fake_encoder.tokenizer_json()

    assert tokenizer["truncation"] is None, "the tokenizer must not truncate"
    assert tokenizer["post_processor"] is None
    assert tokenizer["pre_tokenizer"] == {"type": "WhitespaceSplit"}
    vocab = tokenizer["model"]["vocab"]
    assert vocab["query:"] == 1
    assert vocab["[UNK]"] == 0


def test_committed_model_json_matches_the_committed_files():
    fixture = fake_encoder.ROOT / fake_encoder.FIXTURE_DIR
    model = json.loads((fixture / "model.json").read_text(encoding="utf-8"))

    for name, declaration in model["files"].items():
        content = (fixture / declaration["path"]).read_bytes()
        assert blake3.blake3(content).hexdigest() == declaration["blake3"], name
    assert (fixture / "tokenizer.json").read_text(encoding="utf-8") == (
        json.dumps(fake_encoder.tokenizer_json(), indent=2, sort_keys=True) + "\n"
    )


def test_committed_int8_graph_is_not_the_fp32_graph():
    fixture = fake_encoder.ROOT / fake_encoder.FIXTURE_DIR
    fp32 = (fixture / "model.onnx").read_bytes()
    int8 = (fixture / "model.int8.onnx").read_bytes()

    assert fp32 != int8, "the two precisions must produce distinct vectors"
    assert blake3.blake3(fp32).hexdigest() != blake3.blake3(int8).hexdigest()


def test_generator_reproduces_the_committed_fixture(tmp_path):
    pytest.importorskip("onnx", reason="the export group provides onnx")
    fixture = fake_encoder.ROOT / fake_encoder.FIXTURE_DIR

    fake_encoder.write_fixture(tmp_path)

    for name in ["model.onnx", "model.int8.onnx", "tokenizer.json", "model.json"]:
        assert (tmp_path / name).read_bytes() == (fixture / name).read_bytes(), name
