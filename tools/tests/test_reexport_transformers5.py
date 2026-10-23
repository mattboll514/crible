# SPDX-License-Identifier: Apache-2.0

"""Tests of the ``REEXPORTER-MODELE-TRANSFORMERS-5`` task (phase P2).

The task re-exports the mDenseOn ONNX graph under ``transformers==5.14.1``
(ADR-0050), records the versions of that run in ``model.json``, and proves the
fidelity of the new graph against the production vectors of
``data/legifacile-sample/``.

Coverage, in the order of the task sheet:

- ``export`` group of ``tools/pyproject.toml``: ``transformers`` pinned exactly,
  ``optimum`` gone;
- ``model.json``: the ``export`` field (three version strings, outside the
  fingerprint), atomic rewrite;
- ``max_tokens`` = 8192 declared and exercised on the graph (INV-11);
- the deterministic draw of 100 passages per structure (600 in total);
- the acceptance threshold of 0.999 against the production vectors (INV-12),
  including the failure mode that the threshold catches: vectors encoded
  without the document prompt;
- a clean failure, without a stack trace, when the export engine refuses the
  model;
- reproducibility of the fp32 graph for the same revision (opt-in, see
  ``CRIBLE_EXPORT_REPRO``).

The heavy resources (``torch``, ``transformers``, ``onnxruntime``,
``tokenizers``, the exported model, the Legifacile sample) are absent from the
default environment and from CI. Every test that needs one skips with an
explicit reason; the rest run offline, as the tests of ``EXPORTER-MODELE-ONNX``
do.

Contract expected from ``tools/export_onnx.py``, written by the task:

- ``build_model_json(..., export=<mapping>)`` adds the ``export`` field, with
  exactly the keys ``transformers``, ``torch``, ``onnxruntime``;
- ``MIN_FIDELITY_COSINE`` is 0.999 (the acceptance threshold of ``docs/04``);
- ``fidelity_report(cosines)`` returns ``passages``, ``min_cosine``,
  ``p1_cosine``, ``median_cosine`` and ``min_cosine_required``, and raises
  ``ValueError`` when the minimum is below ``MIN_FIDELITY_COSINE``;
- ``sample_passages(sample_dir, per_structure=100)`` draws a deterministic set
  of passages from every structure of a Legifacile sample laid out as
  ``<sample_dir>/batches/legifacile-sample-<structure>-v1/``, and returns one
  mapping per passage with the keys ``structure``, ``key``, ``text`` and
  ``vector`` (``float32``, ``dims`` components, read from the batch, not
  re-encoded).
"""

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import re
import tomllib
from pathlib import Path

import numpy as np
import pyarrow as pa
import pyarrow.parquet as pq
import pytest

import export_onnx

REPO_ROOT = Path(__file__).resolve().parents[2]
TOOLS_DIR = Path(__file__).resolve().parents[1]
MODEL_DIR = REPO_ROOT / "data" / "models" / "mdenseon"
SAMPLE_DIR = REPO_ROOT / "data" / "legifacile-sample"

DIMS = export_onnx.DIMS
HEX64 = "0" * 64

#: Modules of the ``export`` group of ``tools/pyproject.toml``.
EXPORT_MODULES = ("torch", "transformers", "onnxruntime", "tokenizers")


def _missing_export_modules() -> list[str]:
    return [name for name in EXPORT_MODULES if importlib.util.find_spec(name) is None]


def _model_dir_ready() -> bool:
    return all(
        (MODEL_DIR / name).is_file()
        for name in (
            export_onnx.FP32_PATH,
            export_onnx.TOKENIZER_PATH,
            export_onnx.MODEL_JSON_PATH,
        )
    )


def _parity_report_ready() -> bool:
    return _model_dir_ready() and (MODEL_DIR / export_onnx.PARITY_PATH).is_file()


def _sample_ready() -> bool:
    return (SAMPLE_DIR / "batches").is_dir()


requires_export_deps = pytest.mark.skipif(
    bool(_missing_export_modules()),
    reason=(
        "the export group is not installed (uv sync --group export): "
        f"missing {_missing_export_modules()}"
    ),
)
requires_model_dir = pytest.mark.skipif(
    not _model_dir_ready(),
    reason=f"no exported model in {MODEL_DIR} (run export_onnx.py first)",
)
requires_parity_report = pytest.mark.skipif(
    not _parity_report_ready(),
    reason=(
        f"no parity report in {MODEL_DIR} "
        f"(model.onnx, model.json and {export_onnx.PARITY_PATH} are needed)"
    ),
)
requires_sample = pytest.mark.skipif(
    not _sample_ready(),
    reason=f"no Legifacile sample in {SAMPLE_DIR} (gitignored, see its README)",
)
requires_model_and_sample = pytest.mark.skipif(
    bool(_missing_export_modules()) or not _model_dir_ready() or not _sample_ready(),
    reason=(
        "needs the export group, the exported model and the Legifacile sample: "
        f"missing modules {_missing_export_modules()}"
    ),
)


# --------------------------------------------------------------------------- #
# Synthetic Legifacile sample, for the tests that run offline.
# --------------------------------------------------------------------------- #


def _unit_vector(key: str) -> np.ndarray:
    seed = int.from_bytes(hashlib.blake2b(key.encode(), digest_size=8).digest(), "big")
    rng = np.random.default_rng(seed)
    vector = rng.standard_normal(DIMS)
    return vector / np.linalg.norm(vector)


def _fp16_bytes(vector: np.ndarray) -> bytes:
    return np.asarray(vector, dtype=np.float16).astype("<f2").tobytes()


def _sample_rows(structure: str, count: int) -> list[tuple[str, int, str, str]]:
    """Returns ``(doc_id, ord, text, key)`` rows, with one passage per document."""
    return [
        (
            f"{structure}-doc-{index:03d}",
            0,
            f"Passage {index} of the {structure} structure.",
            f"{structure}-doc-{index:03d}#0",
        )
        for index in range(count)
    ]


def _write_sample_batch(
    sample_dir: Path,
    structure: str,
    rows: list[tuple[str, int, str, str]],
    *,
    parts: int = 1,
) -> Path:
    """Writes a fake Legifacile batch, with the real column types."""
    batch = sample_dir / "batches" / f"legifacile-sample-{structure}-v1"
    batch.mkdir(parents=True)
    (batch / "manifest.json").write_text(
        json.dumps(
            {
                "format": "crible-batch",
                "format_version": 1,
                "batch_id": f"legifacile-sample-{structure}-v1",
                "index": structure,
                "mode": "upsert",
                "created_at": "2026-10-23T00:00:00Z",
                "producer": {"name": "test", "version": "0", "host": "test"},
                "model": {
                    "hf_id": export_onnx.HF_ID,
                    "revision": export_onnx.DEFAULT_REVISION,
                    "dims": DIMS,
                    "pooling": export_onnx.POOLING,
                    "normalize": export_onnx.NORMALIZE,
                    "prompt_document": export_onnx.PROMPTS["document"],
                    "vector_dtype": "fp16",
                },
                "chunker": {"name": "test", "version": "1", "params": {}},
                "template": {"name": "test", "version": "1"},
                "counts": {
                    "documents": len(rows),
                    "passages": len(rows),
                    "deletions": 0,
                },
                "files": {},
            }
        ),
        encoding="utf-8",
    )
    chunk = -(-len(rows) // parts) if rows else 0
    for number in range(parts):
        piece = rows[number * chunk : (number + 1) * chunk]
        table = pa.table(
            {
                "doc_id": pa.array([row[0] for row in piece], type=pa.string()),
                "ord": pa.array([row[1] for row in piece], type=pa.uint32()),
                "text": pa.array([row[2] for row in piece], type=pa.string()),
                "vector": pa.array(
                    [_fp16_bytes(_unit_vector(row[3])) for row in piece],
                    type=pa.binary(2 * DIMS),
                ),
            }
        )
        pq.write_table(table, batch / f"passages-{number + 1:04d}.parquet")
    return batch


def _selection(records: list[dict]) -> list[tuple[str, str]]:
    return [(record["structure"], record["key"]) for record in records]


# --------------------------------------------------------------------------- #
# The ``export`` group of ``tools/pyproject.toml``.
# --------------------------------------------------------------------------- #


def _export_group() -> list[str]:
    with open(TOOLS_DIR / "pyproject.toml", "rb") as handle:
        document = tomllib.load(handle)
    return document["dependency-groups"]["export"]


def _spec_name(spec: str) -> str:
    return re.split(r"[<>=!~\[ ;]", spec, maxsplit=1)[0].strip().lower()


def test_export_group_pins_transformers_5_14_1_and_drops_optimum():
    """ADR-0050, point 3: ``transformers==5.14.1``, no ``optimum``."""
    group = _export_group()
    names = {_spec_name(spec) for spec in group}
    assert "optimum" not in names
    assert "optimum-onnx" not in names
    pinned = [
        spec.strip()
        for spec in group
        if _spec_name(spec) == "transformers"
        and re.fullmatch(r"transformers\s*==\s*5\.14\.1", spec.strip())
    ]
    assert pinned, f"transformers must be pinned to 5.14.1, got {group}"
    assert "onnxruntime" in names
    assert "torch" in names


# --------------------------------------------------------------------------- #
# ``model.json``: the ``export`` field and the atomic rewrite.
# --------------------------------------------------------------------------- #


def test_model_json_records_the_versions_that_produced_the_graph():
    """``export`` carries the three versions, and adds nothing else to the file."""
    versions = {
        "transformers": "5.14.1",
        "torch": "2.14.1",
        "onnxruntime": "1.31.0",
    }
    hashes = {
        "revision": "a" * 40,
        "fp32_hash": HEX64,
        "int8_hash": "1" * 64,
        "tokenizer_hash": "2" * 64,
    }
    with_export = export_onnx.build_model_json(**hashes, export=versions)
    without_export = export_onnx.build_model_json(**hashes)
    assert with_export["export"] == versions
    # ADR-0050, point 2: the field is outside the fingerprint, so it is the
    # only difference with a ``model.json`` written before this task.
    assert set(with_export) - set(without_export) == {"export"}
    assert with_export["export"].keys() == {"transformers", "torch", "onnxruntime"}


def test_model_json_rewrite_keeps_the_previous_file_when_the_write_fails(
    tmp_path, monkeypatch
):
    """A failed rewrite leaves the previous ``model.json`` intact, never a truncated one."""
    target = tmp_path / "model.json"
    previous = export_onnx.build_model_json(
        "a" * 40, fp32_hash=HEX64, int8_hash="1" * 64, tokenizer_hash="2" * 64
    )
    export_onnx.write_model_json(target, previous)
    before = target.read_bytes()

    def failing_replace(source, destination):
        raise OSError("no space left on device")

    monkeypatch.setattr(export_onnx.os, "replace", failing_replace)
    updated = export_onnx.build_model_json(
        "b" * 40, fp32_hash=HEX64, int8_hash="1" * 64, tokenizer_hash="2" * 64
    )
    with pytest.raises(OSError):
        export_onnx.write_model_json(target, updated)

    assert target.read_bytes() == before
    assert [entry.name for entry in tmp_path.iterdir()] == ["model.json"]


# --------------------------------------------------------------------------- #
# INV-11: no silent truncation, ``max_tokens`` = 8192 verified on the graph.
# --------------------------------------------------------------------------- #


def test_inv_11_the_export_declares_the_limit_of_the_graph():
    """``max_tokens`` is 8192 in ``model.json``, the capacity checked on the graph."""
    payload = export_onnx.build_model_json(
        "a" * 40, fp32_hash=HEX64, int8_hash="1" * 64, tokenizer_hash="2" * 64
    )
    assert export_onnx.MAX_TOKENS == 8192
    assert payload["max_tokens"] == 8192


@requires_export_deps
@requires_model_dir
def test_inv_11_the_parity_corpus_needs_no_truncation():
    """No parity text exceeds 8192 tokens: truncation would hide the real length."""
    tokenizer = export_onnx.load_tokenizer(MODEL_DIR / export_onnx.TOKENIZER_PATH)
    prompt = export_onnx.PROMPTS["document"]
    for text in export_onnx.comparison_texts():
        untruncated = export_onnx.encode_tokens(tokenizer, text, prompt, max_tokens=1 << 20)
        limited = export_onnx.encode_tokens(
            tokenizer, text, prompt, max_tokens=export_onnx.MAX_TOKENS
        )
        assert len(untruncated) <= export_onnx.MAX_TOKENS
        assert untruncated == limited


@requires_export_deps
@requires_model_dir
def test_inv_11_the_exported_graph_accepts_the_declared_max_tokens():
    """The graph runs at exactly 8192 tokens and returns ``[1, 8192, 768]``."""
    declared = json.loads((MODEL_DIR / "model.json").read_text(encoding="utf-8"))
    assert declared["max_tokens"] == export_onnx.MAX_TOKENS == 8192
    elapsed = export_onnx.verify_max_tokens(
        MODEL_DIR / export_onnx.FP32_PATH, declared["max_tokens"]
    )
    assert elapsed > 0.0


class _LengthTokenizer:
    """Tokenizer stub whose encoding has ``length`` tokens for any text.

    It truncates like a real tokenizer when its limit is below that length, so
    a test can prove that :func:`export_onnx.encode_tokens` refuses *before*
    anything is cut.
    """

    def __init__(self, length: int):
        self.length = length
        self.max_length = None
        self.strategy = None
        self.truncated = False

    def enable_truncation(self, max_length, strategy):
        self.max_length = max_length
        self.strategy = strategy

    def encode(self, text):
        ids = list(range(2, 2 + self.length))
        if self.max_length is not None and len(ids) > self.max_length:
            self.truncated = True
            ids = ids[: self.max_length]
        return type("_Encoding", (), {"ids": ids})()


def test_inv_11_encode_tokens_refuses_an_input_above_the_limit():
    """The core of INV-11: an input too long raises, and nothing is cut."""
    tokenizer = _LengthTokenizer(8)
    with pytest.raises(ValueError, match="INV-11"):
        export_onnx.encode_tokens(
            tokenizer, "passage", export_onnx.PROMPTS["document"], max_tokens=4
        )
    assert tokenizer.truncated is False

    ids = export_onnx.encode_tokens(
        tokenizer, "passage", export_onnx.PROMPTS["document"], max_tokens=8
    )
    assert len(ids) == 8


def test_inv_11_the_measurement_path_is_bounded_by_the_declared_limit():
    """Without an explicit limit, the refusal applies at ``MAX_TOKENS``."""
    tokenizer = _LengthTokenizer(export_onnx.MAX_TOKENS + 1)
    with pytest.raises(ValueError, match="8192"):
        export_onnx.encode_tokens(tokenizer, "passage", export_onnx.PROMPTS["document"])
    assert tokenizer.truncated is False


# --------------------------------------------------------------------------- #
# The deterministic draw of 100 passages per structure.
# --------------------------------------------------------------------------- #


def test_sample_passages_is_deterministic_and_balanced(tmp_path):
    """Two calls draw the same passages, ``per_structure`` from each structure."""
    _write_sample_batch(tmp_path, "alpha", _sample_rows("alpha", 5))
    _write_sample_batch(tmp_path, "beta", _sample_rows("beta", 7))

    first = export_onnx.sample_passages(tmp_path, per_structure=2)
    second = export_onnx.sample_passages(tmp_path, per_structure=2)

    assert _selection(first) == _selection(second)
    assert all(
        np.array_equal(np.asarray(a["vector"]), np.asarray(b["vector"]))
        for a, b in zip(first, second, strict=True)
    )
    assert len(first) == 4
    structures = [record["structure"] for record in first]
    assert sorted(set(structures)) == ["alpha", "beta"]
    assert structures.count("alpha") == 2
    assert structures.count("beta") == 2
    assert len({record["key"] for record in first}) == 4


def test_sample_passages_ignores_the_file_layout(tmp_path):
    """The draw depends on the content, not on the row order or the file split."""
    rows = _sample_rows("alpha", 6)
    one_part = tmp_path / "one"
    many_parts = tmp_path / "many"
    _write_sample_batch(one_part, "alpha", rows)
    _write_sample_batch(
        many_parts,
        "alpha",
        list(reversed(rows)),
        parts=3,
    )
    assert _selection(export_onnx.sample_passages(one_part, per_structure=3)) == (
        _selection(export_onnx.sample_passages(many_parts, per_structure=3))
    )


def test_sample_passages_returns_the_stored_production_vectors(tmp_path):
    """Vectors come from the batch, in float32, with a unit norm."""
    _write_sample_batch(tmp_path, "alpha", _sample_rows("alpha", 3))
    records = export_onnx.sample_passages(tmp_path, per_structure=3)
    assert len(records) == 3
    for record in records:
        assert set(record) >= {"structure", "key", "text", "vector"}
        assert record["structure"] == "alpha"
        assert isinstance(record["text"], str) and record["text"]
        vector = np.asarray(record["vector"], dtype=np.float64)
        assert vector.shape == (DIMS,)
        assert np.all(np.isfinite(vector))
        # Stored as fp16, so the norm is only close to 1 (control 7 of docs/04).
        assert abs(float(np.linalg.norm(vector)) - 1.0) <= 1e-2
        stored = np.asarray(_unit_vector(record["key"]), dtype=np.float16).astype(
            np.float64
        )
        assert np.allclose(vector, stored, atol=1e-3)


@requires_sample
def test_sample_passages_draws_600_passages_from_the_real_sample():
    """Criterion (ii): 100 passages per structure, six structures, 600 passages."""
    records = export_onnx.sample_passages(SAMPLE_DIR)
    structures = sorted({record["structure"] for record in records})
    assert structures == ["articles", "decisions", "dole", "jorf", "sarde", "titres"]
    assert len(records) == 600
    per_structure = {structure: 0 for structure in structures}
    for record in records:
        per_structure[record["structure"]] += 1
    assert set(per_structure.values()) == {100}
    assert len({record["key"] for record in records}) == 600


# --------------------------------------------------------------------------- #
# INV-12: the acceptance threshold of 0.999 against the production vectors.
# --------------------------------------------------------------------------- #


def _cosines(first: int = 1000) -> list[float]:
    return [0.999 + index * 1e-6 for index in range(first)]


def test_inv_12_fidelity_report_refuses_a_cosine_below_the_threshold():
    """A single vector under 0.999 fails the acceptance, as ``acceptance_failed`` does."""
    assert export_onnx.MIN_FIDELITY_COSINE == 0.999
    cosines = _cosines()
    reported = export_onnx.fidelity_report(cosines)
    assert reported["passages"] == len(cosines)
    assert reported["min_cosine_required"] == 0.999

    below = list(cosines)
    below[17] = 0.9989
    with pytest.raises(ValueError, match="0.999"):
        export_onnx.fidelity_report(below)


def test_inv_12_fidelity_report_gives_the_minimum_the_p1_and_the_median():
    """The report carries the three numbers the task sheet asks for."""
    cosines = _cosines()
    reported = export_onnx.fidelity_report(cosines)
    assert reported["min_cosine"] == pytest.approx(cosines[0])
    assert reported["min_cosine"] <= reported["p1_cosine"] <= reported["median_cosine"]
    assert reported["p1_cosine"] < 0.9991
    assert reported["median_cosine"] == pytest.approx(0.9995, abs=1e-4)


def test_sample_provenance_reads_the_pinned_model(tmp_path):
    """The draw is only meaningful if its vectors come from the reference."""
    _write_sample_batch(tmp_path, "alpha", _sample_rows("alpha", 2))
    declared = export_onnx.check_sample_provenance(
        tmp_path, export_onnx.DEFAULT_REVISION
    )
    assert declared["hf_id"] == export_onnx.HF_ID
    assert declared["revision"] == export_onnx.DEFAULT_REVISION
    assert declared["dims"] == DIMS
    assert declared["prompt_document"] == export_onnx.PROMPTS["document"]


def test_fidelity_refuses_a_sample_encoded_at_another_revision(tmp_path):
    """A sample re-encoded elsewhere would make criterion (ii) self-referential."""
    batch = _write_sample_batch(tmp_path, "alpha", _sample_rows("alpha", 2))
    manifest = json.loads((batch / "manifest.json").read_text(encoding="utf-8"))
    manifest["model"]["revision"] = "b" * 40
    (batch / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")

    with pytest.raises(ValueError, match="revision"):
        export_onnx.check_sample_provenance(tmp_path, export_onnx.DEFAULT_REVISION)


def test_fidelity_refuses_a_sample_encoded_without_the_document_prompt(tmp_path):
    """The prompt is part of the reference; another one makes the cosine moot."""
    batch = _write_sample_batch(tmp_path, "alpha", _sample_rows("alpha", 2))
    manifest = json.loads((batch / "manifest.json").read_text(encoding="utf-8"))
    manifest["model"]["prompt_document"] = ""
    (batch / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")

    with pytest.raises(ValueError, match="prompt"):
        export_onnx.check_sample_provenance(tmp_path, export_onnx.DEFAULT_REVISION)


@requires_model_and_sample
def test_inv_12_vectors_encoded_without_the_document_prompt_are_refused():
    """The failure mode INV-12 guards against: the prompt left out.

    ``data/legifacile-sample/README.md`` measures 0.708 to 0.768 without the
    prompt on the previous ONNX graph; the threshold must reject that.
    """
    import onnxruntime as ort

    tokenizer = export_onnx.load_tokenizer(MODEL_DIR / export_onnx.TOKENIZER_PATH)
    session = ort.InferenceSession(
        str(MODEL_DIR / export_onnx.FP32_PATH), providers=["CPUExecutionProvider"]
    )
    records = export_onnx.sample_passages(SAMPLE_DIR, per_structure=2)
    assert len(records) == 12

    cosines = []
    for record in records:
        ids = np.asarray([export_onnx.encode_tokens(tokenizer, record["text"], "")])
        hidden = session.run(
            None,
            {
                "input_ids": ids.astype(np.int64),
                "attention_mask": np.ones_like(ids, dtype=np.int64),
            },
        )[0]
        vector = export_onnx.normalize(export_onnx.cls_pool(hidden))
        cosines.append(export_onnx.cosine(vector, record["vector"]))
    assert max(cosines) < export_onnx.MIN_FIDELITY_COSINE
    with pytest.raises(ValueError, match="0.999"):
        export_onnx.fidelity_report(cosines)


@requires_model_and_sample
def test_inv_12_production_vectors_meet_the_acceptance_threshold():
    """Criterion (ii): the fp32 graph, with the document prompt, against production."""
    tokenizer = export_onnx.load_tokenizer(MODEL_DIR / export_onnx.TOKENIZER_PATH)
    records = export_onnx.sample_passages(SAMPLE_DIR)
    prompt = export_onnx.PROMPTS["document"]

    # INV-11 on the measurement path: no sampled passage needs truncation.
    for record in records:
        untruncated = export_onnx.encode_tokens(
            tokenizer, record["text"], prompt, max_tokens=1 << 20
        )
        limited = export_onnx.encode_tokens(
            tokenizer, record["text"], prompt, max_tokens=export_onnx.MAX_TOKENS
        )
        assert untruncated == limited
        assert len(untruncated) <= export_onnx.MAX_TOKENS

    exported = export_onnx.onnx_vectors(
        MODEL_DIR / export_onnx.FP32_PATH,
        tokenizer,
        [record["text"] for record in records],
    )
    cosines = [
        export_onnx.cosine(vector, record["vector"])
        for vector, record in zip(exported, records, strict=True)
    ]
    reported = export_onnx.fidelity_report(cosines)
    assert reported["passages"] == 600
    assert reported["min_cosine"] >= 0.999
    assert reported["min_cosine"] <= reported["p1_cosine"] <= reported["median_cosine"]


# --------------------------------------------------------------------------- #
# Criterion (i): PyTorch under transformers 5.14.1 against the ONNX fp32 graph.
# --------------------------------------------------------------------------- #


@requires_parity_report
def test_parity_report_proves_the_export_ran_under_transformers_5_14_1():
    """The export is the one of ADR-0050, and its parity meets 0.9999 on 100 texts."""
    declared = json.loads((MODEL_DIR / "model.json").read_text(encoding="utf-8"))
    assert "export" in declared, "model.json must record the versions of the export"
    assert declared["export"]["transformers"] == "5.14.1"
    assert declared["max_tokens"] == 8192

    report = json.loads((MODEL_DIR / export_onnx.PARITY_PATH).read_text(encoding="utf-8"))
    assert report["precision"] == "fp32"
    assert report["texts"] == 100
    assert report["min_cosine_required"] == 0.9999
    assert report["min_cosine"] >= 0.9999
    assert report["revision"] == declared["revision"]
    assert report["token_lengths"]["limit"] == 8192
    assert report["token_lengths"]["max"] <= 8192


# --------------------------------------------------------------------------- #
# Clean failure when the export engine refuses the model.
# --------------------------------------------------------------------------- #


@pytest.mark.parametrize(
    "error",
    [
        RuntimeError("unsupported operator: aten::scaled_dot_product_attention"),
        NotImplementedError("onnxscript is required by the exporter"),
        OSError("cannot write the temporary graph"),
    ],
)
def test_export_failure_of_the_exporter_is_reported_in_one_line(
    tmp_path, monkeypatch, capsys, error
):
    """No uncaught trace, non-zero status, and no ``model.json`` announcing a graph."""
    snapshot = tmp_path / "snapshot"
    snapshot.mkdir()
    (snapshot / export_onnx.TOKENIZER_PATH).write_text("{}", encoding="utf-8")
    monkeypatch.setattr(export_onnx, "resolve_revision", lambda revision: "a" * 40)
    monkeypatch.setattr(export_onnx, "download_snapshot", lambda revision: snapshot)
    monkeypatch.setattr(export_onnx, "check_model_config", lambda snapshot: None)

    def refusing_export(snapshot, target, revision):
        raise error

    monkeypatch.setattr(export_onnx, "export_fp32", refusing_export)

    out_dir = tmp_path / "out"
    assert export_onnx.main(["--out", str(out_dir), "--skip-parity"]) == 1
    err = capsys.readouterr().err
    assert err.startswith("error: ")
    assert err.count("\n") == 1
    assert "Traceback" not in err
    assert not (out_dir / export_onnx.MODEL_JSON_PATH).exists()


def test_export_failure_of_the_quantizer_is_reported_in_one_line(
    tmp_path, monkeypatch, capsys
):
    """The quantizer can refuse the graph too; the same clean failure applies."""
    snapshot = tmp_path / "snapshot"
    snapshot.mkdir()
    (snapshot / export_onnx.TOKENIZER_PATH).write_text("{}", encoding="utf-8")
    monkeypatch.setattr(export_onnx, "resolve_revision", lambda revision: "a" * 40)
    monkeypatch.setattr(export_onnx, "download_snapshot", lambda revision: snapshot)
    monkeypatch.setattr(export_onnx, "check_model_config", lambda snapshot: None)
    monkeypatch.setattr(
        export_onnx,
        "export_fp32",
        lambda snapshot, target, revision: Path(target).write_bytes(b"graph"),
    )

    def refusing_quantization(source, target):
        raise RuntimeError("the graph holds an operator the quantizer refuses")

    monkeypatch.setattr(export_onnx, "quantize_int8", refusing_quantization)

    out_dir = tmp_path / "out"
    assert export_onnx.main(["--out", str(out_dir), "--skip-parity"]) == 1
    err = capsys.readouterr().err
    assert err.startswith("error: ")
    assert err.count("\n") == 1
    assert "Traceback" not in err
    assert not (out_dir / export_onnx.MODEL_JSON_PATH).exists()


@requires_export_deps
def test_parity_below_the_requirement_stops_the_export(tmp_path, monkeypatch, capsys):
    """INV-12: a graph under the parity threshold never gets a ``model.json``."""
    snapshot = tmp_path / "snapshot"
    snapshot.mkdir()
    (snapshot / export_onnx.TOKENIZER_PATH).write_text("{}", encoding="utf-8")
    monkeypatch.setattr(export_onnx, "resolve_revision", lambda revision: "a" * 40)
    monkeypatch.setattr(export_onnx, "download_snapshot", lambda revision: snapshot)
    monkeypatch.setattr(export_onnx, "check_model_config", lambda snapshot: None)
    monkeypatch.setattr(
        export_onnx,
        "export_fp32",
        lambda snapshot, target, revision: Path(target).write_bytes(b"graph"),
    )
    monkeypatch.setattr(
        export_onnx,
        "quantize_int8",
        lambda source, target: Path(target).write_bytes(b"int8"),
    )
    monkeypatch.setattr(export_onnx, "load_tokenizer", lambda path: object())
    monkeypatch.setattr(export_onnx, "verify_max_tokens", lambda *args, **kwargs: 0.1)
    monkeypatch.setattr(
        export_onnx,
        "run_parity",
        lambda snapshot, path, tokenizer: {
            "precision": "fp32",
            "texts": 100,
            "min_cosine": 0.998,
            "mean_cosine": 0.9995,
        },
    )

    out_dir = tmp_path / "out"
    assert export_onnx.main(["--out", str(out_dir)]) == 1
    err = capsys.readouterr().err
    assert err.startswith("error: ")
    assert "Traceback" not in err
    assert not (out_dir / export_onnx.MODEL_JSON_PATH).exists()
    assert not (out_dir / export_onnx.PARITY_PATH).exists()


@requires_export_deps
def test_export_writes_the_versions_and_the_parity_report(tmp_path, monkeypatch, capsys):
    """A successful run records ``export`` in ``model.json`` and its report."""
    snapshot = tmp_path / "snapshot"
    snapshot.mkdir()
    (snapshot / export_onnx.TOKENIZER_PATH).write_text("{}", encoding="utf-8")
    monkeypatch.setattr(export_onnx, "resolve_revision", lambda revision: "a" * 40)
    monkeypatch.setattr(export_onnx, "download_snapshot", lambda revision: snapshot)
    monkeypatch.setattr(export_onnx, "check_model_config", lambda snapshot: None)
    monkeypatch.setattr(
        export_onnx,
        "export_fp32",
        lambda snapshot, target, revision: Path(target).write_bytes(b"graph"),
    )
    monkeypatch.setattr(
        export_onnx,
        "quantize_int8",
        lambda source, target: Path(target).write_bytes(b"int8"),
    )
    monkeypatch.setattr(export_onnx, "load_tokenizer", lambda path: object())
    monkeypatch.setattr(export_onnx, "verify_max_tokens", lambda *args, **kwargs: 0.1)
    monkeypatch.setattr(
        export_onnx,
        "run_parity",
        lambda snapshot, path, tokenizer: {
            "precision": "fp32",
            "texts": 100,
            "min_cosine": 0.99999,
            "mean_cosine": 0.999995,
        },
    )

    out_dir = tmp_path / "out"
    assert export_onnx.main(["--out", str(out_dir)]) == 0
    payload = json.loads(
        (out_dir / export_onnx.MODEL_JSON_PATH).read_text(encoding="utf-8")
    )
    assert payload["export"]["transformers"] == "5.14.1"
    assert payload["export"]["torch"]
    assert payload["export"]["onnxruntime"]
    assert payload["max_tokens"] == 8192
    report = json.loads((out_dir / export_onnx.PARITY_PATH).read_text(encoding="utf-8"))
    assert report["min_cosine_required"] == 0.9999


# --------------------------------------------------------------------------- #
# Reproducibility, opt-in: two full exports, same revision, same fp32 hash.
# --------------------------------------------------------------------------- #


@pytest.mark.skipif(
    os.environ.get("CRIBLE_EXPORT_REPRO") != "1",
    reason=(
        "set CRIBLE_EXPORT_REPRO=1 to run two full exports (slow, needs the "
        "export group and the Hugging Face snapshot)"
    ),
)
@requires_export_deps
def test_export_twice_with_the_same_revision_writes_the_same_fp32_graph(tmp_path):
    """Criterion: reproducible export, same revision, same fp32 hash."""
    first = tmp_path / "first"
    second = tmp_path / "second"
    assert export_onnx.main(["--out", str(first), "--skip-parity"]) == 0
    assert export_onnx.main(["--out", str(second), "--skip-parity"]) == 0
    for name in (export_onnx.FP32_PATH, export_onnx.INT8_PATH, export_onnx.TOKENIZER_PATH):
        assert export_onnx.blake3_file(first / name) == export_onnx.blake3_file(
            second / name
        )
    first_payload = json.loads(
        (first / export_onnx.MODEL_JSON_PATH).read_text(encoding="utf-8")
    )
    second_payload = json.loads(
        (second / export_onnx.MODEL_JSON_PATH).read_text(encoding="utf-8")
    )
    assert first_payload["revision"] == second_payload["revision"]
    assert first_payload["files"] == second_payload["files"]
