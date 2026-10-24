# SPDX-License-Identifier: Apache-2.0

"""Cross-check the Rust encoder against ``onnxruntime`` in Python (MES-04).

MES-04 measures the vectors of the engine with the Rust harness
(``measure_queries vectors``) and the reference with PyTorch. A mistake in the
harness would spoil both, so the report also checks five queries against a third
path: the same tokenizer and graph, driven by ``onnxruntime`` in Python. This
script reproduces that check, and names it in ``docs/mesures/MES-04-*.md``.

It reads the files the campaign already wrote, so it runs after
``measure_queries vectors``:

- ``query-texts.jsonl``: the opaque ids and texts, from ``prepare.py``;
- ``rust-vectors.json`` and ``vectors-int8.f32`` / ``vectors-fp32.f32``: the
  vectors of the engine, in query order;
- ``model.json`` and the graph of the model directory.

The vectors are encoded with the query prompt of ``model.json``, then compared
by cosine, in ``float64``. No query text is printed and nothing is written:
the check is a control, not a measurement.

Run from ``tools/``, with the ``export`` group installed::

    uv run python bench/mes04/compare_onnx.py --limit 5
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import numpy as np

import analysis

#: Repository root, three levels above this file.
ROOT = Path(__file__).resolve().parents[3]
DEFAULT_DATA = ROOT / "data/mes04"
DEFAULT_MODEL = ROOT / "data/models/mdenseon"
#: Queries compared by default: the five first of the campaign.
DEFAULT_LIMIT = 5
#: Minimum cosine accepted between the two paths. Far below the measured
#: 0.99999997, so only a real divergence fails.
MIN_CROSS_COSINE = 0.9999


def text_by_id(path: Path) -> dict[str, str]:
    """Returns ``id -> text`` of the reduced query file of ``prepare.py``."""
    records = {}
    with path.open(encoding="utf-8") as handle:
        for line in handle:
            if not line.strip():
                continue
            record = json.loads(line)
            identifier = record.get("id")
            text = record.get("text")
            if not isinstance(identifier, str) or not isinstance(text, str):
                raise ValueError(f"{path}: a line lacks a string id or text")
            if identifier in records:
                raise ValueError(f"{path}: duplicate id '{identifier}'")
            records[identifier] = text
    if not records:
        raise ValueError(f"{path} holds no query")
    return records


def compare_rows(
    left: dict[str, np.ndarray], right: dict[str, np.ndarray]
) -> dict[str, float]:
    """Returns the cosine of each id present on both sides."""
    common = [identifier for identifier in left if identifier in right]
    if not common:
        raise ValueError("the two sides share no query id")
    return {
        identifier: analysis.cosine(left[identifier], right[identifier])
        for identifier in common
    }


def encode_rows(
    model_path: Path, tokenizer, prompt: str, texts: dict[str, str], max_tokens: int
) -> dict[str, np.ndarray]:
    """Encodes ``prompt + text`` with one ONNX session, CLS pooled and L2 normalized.

    The pipeline is the one of ``tools/export_onnx.py`` and of the engine: the
    graph returns the last hidden state, Crible pools the ``[CLS]`` row and
    normalizes. The tokenizer must come from :func:`load_tokenizer`, so that
    truncation is refused (INV-11), as everywhere else in the campaign.
    """
    import onnxruntime as ort

    session = ort.InferenceSession(str(model_path), providers=["CPUExecutionProvider"])
    vectors = {}
    for identifier, text in texts.items():
        ids = np.asarray([tokenizer.encode(prompt + text).ids], dtype=np.int64)
        if ids.shape[1] > max_tokens:
            raise ValueError(
                f"query '{identifier}' needs {ids.shape[1]} tokens, above "
                f"max_tokens {max_tokens}"
            )
        hidden = session.run(
            None,
            {
                "input_ids": ids,
                "attention_mask": np.ones_like(ids),
            },
        )[0]
        # The engine pools row 0, sums the squares in float64, casts the norm to
        # float32 and divides in float32 (`crible-encode::Model::encode`). Doing
        # the division in float64 instead would hide the last-bit difference the
        # cross-check exists to see.
        vector = np.asarray(hidden[0, 0], dtype=np.float32).copy()
        norm = float(np.sqrt(np.sum(vector.astype(np.float64) ** 2)))
        if norm == 0.0:
            raise ValueError(f"query '{identifier}' encodes to a zero vector")
        vector /= np.float32(norm)
        vectors[identifier] = vector
    return vectors


def load_tokenizer(path: Path):
    """Loads ``tokenizer.json`` with the truncation it declares removed.

    ``tokenizer.json`` declares a truncation at ``max_tokens``; the engine clears
    it (``crible-encode::read_tokenizer``) so an input above the limit is
    refused rather than silently cut (INV-11). Doing the same here keeps the
    control honest: without it, both sides would truncate and the check would
    pass on cut inputs.
    """
    from tokenizers import Tokenizer

    tokenizer = Tokenizer.from_file(str(path))
    tokenizer.no_truncation()
    return tokenizer


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--data", type=Path, default=DEFAULT_DATA)
    parser.add_argument("--model", type=Path, default=DEFAULT_MODEL)
    parser.add_argument("--limit", type=int, default=DEFAULT_LIMIT)
    arguments = parser.parse_args(argv)

    try:
        model = json.loads((arguments.model / "model.json").read_text(encoding="utf-8"))
        meta = json.loads(
            (arguments.data / "rust-vectors.json").read_text(encoding="utf-8")
        )
        count = len(meta["ids"])
        int8 = analysis.load_f32_matrix(
            arguments.data / "vectors-int8.f32", count, analysis.DIMS
        )
        fp32 = analysis.load_f32_matrix(
            arguments.data / "vectors-fp32.f32", count, analysis.DIMS
        )
        if arguments.limit <= 0:
            raise ValueError(f"limit must be positive, got {arguments.limit}")
        ids = meta["ids"][: arguments.limit]
        rust_int8 = {identifier: int8[index] for index, identifier in enumerate(ids)}
        rust_fp32 = {identifier: fp32[index] for index, identifier in enumerate(ids)}

        texts = text_by_id(arguments.data / "query-texts.jsonl")
        missing = [identifier for identifier in ids if identifier not in texts]
        if missing:
            raise ValueError(f"no text for {', '.join(missing)}")

        # The import of ``tokenizers`` lives in ``load_tokenizer``, so the
        # module imports without the export group.
        tokenizer = load_tokenizer(
            arguments.model / model["files"]["tokenizer"]["path"]
        )
        prompt = model["prompts"]["query"]
        max_tokens = model["max_tokens"]
        selected = {identifier: texts[identifier] for identifier in ids}
        onnx_int8 = encode_rows(
            arguments.model / model["files"]["int8"]["path"],
            tokenizer,
            prompt,
            selected,
            max_tokens,
        )
        onnx_fp32 = encode_rows(
            arguments.model / model["files"]["fp32"]["path"],
            tokenizer,
            prompt,
            selected,
            max_tokens,
        )

        int8_cosines = compare_rows(rust_int8, onnx_int8)
        fp32_cosines = compare_rows(rust_fp32, onnx_fp32)
    except Exception as error:  # noqa: BLE001 - every failure becomes one line
        print(f"error: {' '.join(str(error).split())}", file=sys.stderr)
        return 1

    values = list(int8_cosines.values()) + list(fp32_cosines.values())
    minimum = min(values)
    print(
        f"Rust against onnxruntime: cosine {minimum:.9f} to {max(values):.9f} "
        f"on {len(int8_cosines)} queries, int8 and fp32"
    )
    for identifier in int8_cosines:
        print(
            f"  {identifier}: int8 {int8_cosines[identifier]:.9f}, "
            f"fp32 {fp32_cosines[identifier]:.9f}"
        )
    if minimum < MIN_CROSS_COSINE:
        print(
            f"error: cosine {minimum} below {MIN_CROSS_COSINE}: the Rust harness "
            f"and onnxruntime diverge",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
