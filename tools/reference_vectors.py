# SPDX-License-Identifier: Apache-2.0

"""Write the reference vectors of the ``CHARGER-MODELE`` acceptance test.

The task ``CHARGER-MODELE`` (phase P2) compares the vectors of
``crible_encode::Model`` to those of ``tools/export_onnx.py`` in fp32, within
1e-4. The reference is a small versioned fixture under
``crates/crible-encode/tests/fixtures/mdenseon/``:

====================  ====================================================
``texts.txt``         the 20 reference texts, one per line, UTF-8
``vectors.f32``       20 × 768 float32, little-endian, in the same order
====================  ====================================================

The texts are the first 20 of ``export_onnx.comparison_texts()``, the
deterministic French corpus of the parity check. The vectors are produced by
the ONNX fp32 graph with the ``document: `` prompt, CLS pooling and L2
normalization, exactly as ``export_onnx.onnx_vectors`` does: that is the
reference of the acceptance criterion.

Run from ``tools/`` with the ``export`` dependency group installed and the
model exported in ``data/models/mdenseon/``:

```sh
uv sync --group export
uv run python reference_vectors.py
uv run python export_onnx.py --out ../data/models/mdenseon --overwrite
```

The second command is only needed when the model itself is re-exported; the
first rewrites the fixture. The printout carries the blake3 of both files, so a
regeneration can be compared to the committed fixture.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import blake3
import numpy as np

import export_onnx

#: Number of reference texts of the acceptance criterion.
REFERENCE_COUNT = 20
#: Directory of the fixture, relative to the repository root.
FIXTURE_DIR = Path("crates/crible-encode/tests/fixtures/mdenseon")
#: Repository root, two levels above this file.
ROOT = Path(__file__).resolve().parent.parent


def reference_texts() -> list[str]:
    """Returns the 20 deterministic texts of the fixture."""
    texts = export_onnx.comparison_texts()[:REFERENCE_COUNT]
    if len(texts) != REFERENCE_COUNT:
        raise ValueError(
            f"expected {REFERENCE_COUNT} reference texts, got {len(texts)}"
        )
    return texts


def write_fixture(out_dir: Path, model_dir: Path) -> dict[str, str]:
    """Writes ``texts.txt`` and ``vectors.f32``, returns their blake3 digests."""
    tokenizer = export_onnx.load_tokenizer(model_dir / export_onnx.TOKENIZER_PATH)
    texts = reference_texts()
    vectors = export_onnx.onnx_vectors(
        model_dir / export_onnx.FP32_PATH, tokenizer, texts
    )
    matrix = np.asarray(vectors, dtype="<f4")
    expected = (REFERENCE_COUNT, export_onnx.DIMS)
    if matrix.shape != expected:
        raise ValueError(f"expected a {expected} matrix, got {matrix.shape}")

    out_dir.mkdir(parents=True, exist_ok=True)
    texts_path = out_dir / "texts.txt"
    vectors_path = out_dir / "vectors.f32"
    texts_path.write_text("\n".join(texts) + "\n", encoding="utf-8")
    vectors_path.write_bytes(matrix.tobytes(order="C"))
    return {
        "texts.txt": blake3.blake3(texts_path.read_bytes()).hexdigest(),
        "vectors.f32": blake3.blake3(vectors_path.read_bytes()).hexdigest(),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--model",
        type=Path,
        default=ROOT / "data/models/mdenseon",
        help="model directory (default: data/models/mdenseon)",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=ROOT / FIXTURE_DIR,
        help=f"fixture directory (default: {FIXTURE_DIR})",
    )
    arguments = parser.parse_args(argv)

    try:
        digests = write_fixture(arguments.out, arguments.model)
    except Exception as error:  # noqa: BLE001 - every failure becomes one line
        print(f"error: {' '.join(str(error).split())}", file=sys.stderr)
        return 1
    print(f"wrote {REFERENCE_COUNT} reference vectors to {arguments.out}")
    for name, digest in digests.items():
        print(f"  {name} blake3 {digest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
