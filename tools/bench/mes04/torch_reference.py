# SPDX-License-Identifier: Apache-2.0

"""Produce the PyTorch reference vectors of MES-04.

The reference is the model itself under ``transformers==5.14.1`` and
``torch==2.14.1`` (CPU), the versions that produced the production vectors of
Legifacile (ADR-0050, ``tools/pyproject.toml`` group ``export``). The pipeline is
the one of ``tools/export_onnx.py``: ``query: `` prompt, the tokenizer of the
model directory, no truncation (every query is far below ``max_tokens``), CLS
pooling and L2 normalization.

The graph is loaded from the Hugging Face cache only: a measurement must not
depend on the network. ``--allow-download`` relaxes that for a first run.

Run from ``tools/``::

    uv run python bench/mes04/torch_reference.py
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import numpy as np

#: Repository root, three levels above this file.
ROOT = Path(__file__).resolve().parents[3]
DEFAULT_MODEL = ROOT / "data/models/mdenseon"
DEFAULT_QUERIES = ROOT / "data/mes04/query-texts.jsonl"
DEFAULT_OUT = ROOT / "data/mes04/torch-fp32.f32"
#: HF repository of the encoder (``model.json`` carries the pinned revision).
HF_ID = "lightonai/mDenseOn"
#: Prompt of the query side (``model.json``).
PROMPT = "query: "


def load_queries(path: Path) -> list[dict]:
    """Reads the reduced query JSONL written by ``prepare.py``."""
    records = []
    with path.open(encoding="utf-8") as handle:
        for line in handle:
            if line.strip():
                records.append(json.loads(line))
    if not records:
        raise ValueError(f"{path} holds no query")
    return records


def resolve_snapshot(revision: str, allow_download: bool) -> Path:
    """Returns the cached snapshot of ``revision``, downloading only if allowed."""
    from huggingface_hub import snapshot_download

    return Path(
        snapshot_download(
            HF_ID,
            revision=revision,
            allow_patterns=["*.json", "*.safetensors"],
            local_files_only=not allow_download,
        )
    )


def encode_ids(tokenizer, text: str, max_tokens: int) -> list[int]:
    """Encodes ``PROMPT + text`` and refuses a silent truncation (INV-11)."""
    ids = tokenizer.encode(PROMPT + text, add_special_tokens=True).ids
    if len(ids) > max_tokens:
        raise ValueError(
            f"a query needs {len(ids)} tokens, above max_tokens {max_tokens}"
        )
    return ids


def torch_vectors(model_dir: Path, records: list[dict], allow_download: bool):
    """Returns the reference vectors, the token lengths and the graph identity."""
    import torch
    from tokenizers import Tokenizer
    from transformers import AutoModel

    config = json.loads((model_dir / "model.json").read_text(encoding="utf-8"))
    tokenizer = Tokenizer.from_file(str(model_dir / "tokenizer.json"))
    tokenizer.no_truncation()
    snapshot = resolve_snapshot(config["revision"], allow_download)
    model = AutoModel.from_pretrained(str(snapshot), dtype=torch.float32).eval()

    vectors = []
    lengths = []
    dims = int(config["dims"])
    with torch.inference_mode():
        for record in records:
            ids = encode_ids(tokenizer, record["text"], int(config["max_tokens"]))
            lengths.append(len(ids))
            tensor = torch.tensor([ids], dtype=torch.long)
            hidden = model(
                input_ids=tensor, attention_mask=torch.ones_like(tensor)
            ).last_hidden_state
            pooled = hidden[0, 0].numpy().astype(np.float32)
            norm = float(np.linalg.norm(pooled))
            if norm == 0.0:
                raise ValueError("the model returned a zero vector")
            vectors.append(pooled / norm)
    matrix = np.asarray(vectors, dtype="<f4")
    if matrix.shape != (len(records), dims):
        raise ValueError(f"expected {(len(records), dims)} vectors, got {matrix.shape}")
    return matrix, lengths, {
        "hf_id": HF_ID,
        "revision": config["revision"],
        "transformers": __import__("transformers").__version__,
        "torch": torch.__version__,
        "prompt": PROMPT,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--model", type=Path, default=DEFAULT_MODEL)
    parser.add_argument("--queries", type=Path, default=DEFAULT_QUERIES)
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    parser.add_argument(
        "--allow-download",
        action="store_true",
        help="download the snapshot when it is not in the Hugging Face cache",
    )
    arguments = parser.parse_args(argv)
    try:
        records = load_queries(arguments.queries)
        matrix, lengths, identity = torch_vectors(
            arguments.model, records, arguments.allow_download
        )
    except Exception as error:  # noqa: BLE001 - every failure becomes one line
        print(f"error: {' '.join(str(error).split())}", file=sys.stderr)
        return 1

    arguments.out.parent.mkdir(parents=True, exist_ok=True)
    arguments.out.write_bytes(matrix.tobytes(order="C"))
    meta_path = arguments.out.with_name("torch-vectors.json")
    meta = {
        "queries": len(records),
        "dims": int(matrix.shape[1]),
        "layout": "row-major little-endian f32, one row per input line",
        "ids": [record["id"] for record in records],
        "token_lengths": {
            "min": min(lengths),
            "max": max(lengths),
            "mean": sum(lengths) / len(lengths),
        },
        "graph": identity,
    }
    meta_path.write_text(json.dumps(meta, indent=2) + "\n", encoding="utf-8")
    print(
        f"wrote {len(records)} PyTorch fp32 vectors of {matrix.shape[1]} dimensions "
        f"to {arguments.out} (tokens {meta['token_lengths']['min']} to "
        f"{meta['token_lengths']['max']})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
