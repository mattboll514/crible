# SPDX-License-Identifier: Apache-2.0

"""Tests of ``tools/export_onnx.py``.

Three parts of the task are covered without any download and without loading
PyTorch: the shape of ``model.json`` (the contract read by ``CHARGER-MODELE``),
the BLAKE3 digests, and the deterministic corpus of the parity check. The
export itself is run by hand; its measured numbers live in ``tools/README.md``.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

import numpy as np
import pytest

import export_onnx
from export_onnx import (
    DIMS,
    HF_ID,
    MAX_TOKENS,
    MODEL_NAME,
    PROMPTS,
    ST_CONFIG_PATH,
    TOKENIZER_PATH,
    build_model_json,
    blake3_file,
    check_model_config,
    clear_manifest,
    cls_pool,
    comparison_texts,
    cosine,
    encode_tokens,
    long_text,
    normalize,
    prepare_output_dir,
    write_model_json,
)

HEX64 = r"^[0-9a-f]{64}$"
HEX40 = r"^[0-9a-f]{40}$"


def _payload(**overrides):
    hashes = {
        "fp32_hash": "0" * 64,
        "int8_hash": "1" * 64,
        "tokenizer_hash": "2" * 64,
        "revision": "a" * 40,
    }
    hashes.update(overrides)
    return build_model_json(**hashes)


def test_blake3_file_matches_reference_digest(tmp_path):
    empty = tmp_path / "empty"
    empty.write_bytes(b"")
    assert blake3_file(empty) == (
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
    )
    sample = tmp_path / "sample"
    sample.write_bytes(b"abc")
    assert blake3_file(sample) == (
        "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
    )


def test_blake3_file_is_independent_of_the_chunk_size(tmp_path):
    path = tmp_path / "bytes"
    path.write_bytes(bytes(range(256)) * 1024)
    digests = {blake3_file(path, chunk_size=size) for size in (1, 7, 1024, 1 << 20)}
    assert len(digests) == 1


def test_model_json_declares_the_model_of_docs_04():
    payload = _payload()
    assert payload["name"] == MODEL_NAME == "mdenseon"
    assert payload["kind"] == "onnx"
    assert payload["hf_id"] == HF_ID == "lightonai/mDenseOn"
    assert payload["dims"] == DIMS == 768
    assert payload["pooling"] == "cls"
    assert payload["normalize"] is True
    assert payload["max_tokens"] == MAX_TOKENS == 8192
    assert payload["prompts"] == {"query": "query: ", "document": "document: "}
    assert PROMPTS["query"].endswith(" ")
    assert PROMPTS["document"].endswith(" ")


def test_model_json_pins_the_revision_and_hashes_every_file():
    payload = _payload(revision="a5fdb000f7a21da96c3bddde3a782ef777316df3")
    assert re.fullmatch(HEX40, payload["revision"]) is not None
    files = payload["files"]
    assert set(files) == {"fp32", "int8", "tokenizer"}
    assert files["fp32"]["path"] == "model.onnx"
    assert files["int8"]["path"] == "model.int8.onnx"
    assert files["tokenizer"]["path"] == "tokenizer.json"
    for entry in files.values():
        assert re.fullmatch(HEX64, entry["blake3"]) is not None
    assert files["fp32"]["blake3"] == "0" * 64
    assert files["tokenizer"]["blake3"] == "2" * 64


def test_model_json_is_written_byte_stable(tmp_path):
    first = tmp_path / "first.json"
    second = tmp_path / "second.json"
    write_model_json(first, _payload())
    write_model_json(second, _payload())
    assert first.read_bytes() == second.read_bytes()
    text = first.read_text(encoding="utf-8")
    assert text.endswith("\n")
    keys = list(json.loads(text))
    assert keys == sorted(keys)


def test_comparison_texts_are_deterministic_and_complete():
    first = comparison_texts()
    second = comparison_texts()
    assert first == second
    assert len(first) == 100
    assert all(text.strip() for text in first)


def test_comparison_texts_cover_short_and_near_limit_lengths():
    texts = comparison_texts()
    lengths = [len(text) for text in texts]
    assert min(lengths) < 200
    assert lengths[-1] >= export_onnx.LONG_TEXT_CHARS
    assert lengths[-1] == max(lengths)
    # The corpus is French, so the accents of the sentences survive.
    assert any("é" in text for text in texts)
    # The long text is a repetition, so it holds many distinct sentences.
    assert len(set(texts)) == len(texts)


def test_long_text_reaches_the_requested_size():
    assert len(long_text(1000)) >= 1000
    assert len(long_text(1000)) < 1000 + 2000


def test_cls_pool_takes_the_first_token():
    hidden = np.arange(3 * 4, dtype=np.float32).reshape(1, 3, 4)
    pooled = cls_pool(hidden)
    assert pooled.shape == (4,)
    assert np.array_equal(pooled, hidden[0, 0])


def test_cls_pool_rejects_a_non_batched_output():
    with pytest.raises(ValueError):
        cls_pool(np.zeros((3, 4), dtype=np.float32))
    with pytest.raises(ValueError):
        cls_pool(np.zeros((2, 3, 4), dtype=np.float32))


def test_normalize_returns_a_unit_vector():
    vector = np.asarray([3.0, 0.0, 4.0], dtype=np.float32)
    assert np.isclose(np.linalg.norm(normalize(vector)), 1.0, atol=1e-6)
    with pytest.raises(ValueError):
        normalize(np.zeros(4, dtype=np.float32))


def test_cosine_of_identical_and_orthogonal_vectors():
    vector = normalize(np.asarray([1.0, 2.0, 3.0]))
    assert np.isclose(cosine(vector, vector), 1.0, atol=1e-6)
    assert np.isclose(
        cosine(np.asarray([1.0, 0.0]), np.asarray([0.0, 1.0])), 0.0, atol=1e-9
    )
    with pytest.raises(ValueError):
        cosine(np.zeros(2), np.ones(2))


class _Encoding:
    def __init__(self, ids):
        self.ids = ids


class _FakeTokenizer:
    def __init__(self):
        self.max_length = None
        self.strategy = None
        self.seen = None

    def enable_truncation(self, max_length, strategy):
        self.max_length = max_length
        self.strategy = strategy

    def encode(self, text):
        self.seen = text
        return _Encoding([2, 10, 11, 1])


def test_encode_tokens_applies_the_prompt_and_the_limit():
    tokenizer = _FakeTokenizer()
    ids = encode_tokens(tokenizer, "texte", PROMPTS["document"], max_tokens=8192)
    assert ids == [2, 10, 11, 1]
    assert tokenizer.seen == "document: texte"
    assert tokenizer.max_length == 8192
    assert tokenizer.strategy == "longest_first"


def test_prepare_output_dir_creates_missing_parents(tmp_path):
    directory = prepare_output_dir(tmp_path / "models" / "mdenseon", overwrite=False)
    assert directory.is_dir()


def test_prepare_output_dir_refuses_foreign_files(tmp_path):
    (tmp_path / "keep.txt").write_text("keep", encoding="utf-8")
    with pytest.raises(FileExistsError):
        prepare_output_dir(tmp_path, overwrite=False)
    assert prepare_output_dir(tmp_path, overwrite=True) == tmp_path
    assert (tmp_path / "keep.txt").exists()


def test_prepare_output_dir_never_deletes_a_previous_export(tmp_path):
    """A download that fails after preparation leaves the export untouched."""
    for name in ("model.json", "parity.json", "model.onnx", "keep.txt"):
        (tmp_path / name).write_text("previous", encoding="utf-8")

    prepare_output_dir(tmp_path, overwrite=True)

    assert (tmp_path / "model.json").exists()
    assert (tmp_path / "parity.json").exists()


def test_clear_manifest_drops_the_manifest_and_the_report_only(tmp_path):
    for name in ("model.json", "parity.json", "model.onnx", "keep.txt"):
        (tmp_path / name).write_text("previous", encoding="utf-8")

    clear_manifest(tmp_path)

    assert not (tmp_path / "model.json").exists()
    assert not (tmp_path / "parity.json").exists()
    assert (tmp_path / "model.onnx").exists()
    assert (tmp_path / "keep.txt").exists()


def test_model_json_is_written_atomically(tmp_path):
    target = tmp_path / "model.json"
    write_model_json(target, _payload())
    assert [entry.name for entry in tmp_path.iterdir()] == ["model.json"]
    assert target.stat().st_mode & 0o777 == 0o644
    write_model_json(target, _payload(revision="b" * 40))
    leftovers = [entry.name for entry in tmp_path.iterdir() if entry.name != "model.json"]
    assert leftovers == []
    assert json.loads(target.read_text(encoding="utf-8"))["revision"] == "b" * 40


def _fake_snapshot(tmp_path, *, prompts=None, hidden_size=768, max_tokens=8192):
    snapshot = tmp_path / "snapshot"
    snapshot.mkdir(parents=True)
    (snapshot / "config.json").write_text(
        json.dumps(
            {"hidden_size": hidden_size, "max_position_embeddings": max_tokens}
        ),
        encoding="utf-8",
    )
    (snapshot / ST_CONFIG_PATH).write_text(
        json.dumps({"prompts": prompts if prompts is not None else PROMPTS}),
        encoding="utf-8",
    )
    (snapshot / TOKENIZER_PATH).write_text(
        json.dumps({"truncation": {"max_length": max_tokens}}), encoding="utf-8"
    )
    return snapshot


def test_check_model_config_accepts_the_expected_snapshot(tmp_path):
    check_model_config(_fake_snapshot(tmp_path))


@pytest.mark.parametrize(
    "prompts",
    [
        {"query": "query:", "document": "document:"},
        {"query": "Query: ", "document": "Passage: "},
        {"query": "query: "},
        {},
    ],
)
def test_check_model_config_refuses_other_prompts(tmp_path, prompts):
    with pytest.raises(ValueError, match="prompts"):
        check_model_config(_fake_snapshot(tmp_path, prompts=prompts))


def test_check_model_config_refuses_another_width_and_limit(tmp_path):
    with pytest.raises(ValueError, match="hidden_size"):
        check_model_config(_fake_snapshot(tmp_path / "a", hidden_size=1024))
    with pytest.raises(ValueError, match="max_position_embeddings"):
        check_model_config(_fake_snapshot(tmp_path / "b", max_tokens=4096))


def test_main_reports_a_failure_without_a_stack_trace(tmp_path, capsys):
    (tmp_path / "keep.txt").write_text("keep", encoding="utf-8")
    assert export_onnx.main(["--out", str(tmp_path)]) == 1
    assert "error:" in capsys.readouterr().err


def test_main_turns_a_runtime_error_into_one_line(tmp_path, capsys, monkeypatch):
    def failing(*args, **kwargs):
        raise RuntimeError("connection reset")

    monkeypatch.setattr(export_onnx, "export", failing)
    assert export_onnx.main(["--out", str(tmp_path), "--skip-parity"]) == 1
    assert "connection reset" in capsys.readouterr().err


def test_import_stays_free_of_heavy_dependencies():
    """The pytest suite must run offline: importing the module loads nothing heavy."""
    probe = (
        "import sys, export_onnx;"
        "heavy = [name for name in ('torch', 'transformers', 'onnxruntime', 'optimum')"
        " if name in sys.modules];"
        "print(heavy)"
    )
    result = subprocess.run(
        [sys.executable, "-c", probe],
        capture_output=True,
        text=True,
        check=True,
        cwd=Path(export_onnx.__file__).parent,
    )
    assert result.stdout.strip() == "[]"
