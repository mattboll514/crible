# SPDX-License-Identifier: Apache-2.0

"""Write the fake ONNX encoder of the ``ENCODER-REQUETES`` tests.

The task ``ENCODER-REQUETES`` (phase P2) needs an encoder that runs without a
downloaded model, to prove the query cache, the INT8/fp32 sessions, the
truncation flag and the ``crible search --query`` path. The versioned fixture
under ``crates/crible-encode/tests/fixtures/fake-model/`` is deliberately tiny:

=====================  =====================================================
``model.onnx``         fp32 graph, CLS row = mean token id in component 0
``model.int8.onnx``    same graph with the mean scaled by 2, so the two
                       precisions produce distinct vectors
``tokenizer.json``     WordLevel tokenizer, no truncation, no post-processor
``model.json``         the configuration and the blake3 of the three files
=====================  =====================================================

The vocabulary and the graph are fixed, and the protobuf is serialized
deterministically, so running this script must reproduce the committed bytes
and their hashes. ``model.json`` declares ``max_tokens: 32`` and 768
dimensions, as the production model does.

Run from ``tools/`` with the ``export`` dependency group installed (it provides
``onnx``); no model download and no network access are needed:

```sh
uv sync --group export
uv run python fake_encoder.py
```

``tests/test_fake_encoder.py`` checks the committed fixture and, when ``onnx``
is installed, that this script reproduces it byte for byte.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import blake3

DIMS = 768
MAX_TOKENS = 32
ROOT = Path(__file__).resolve().parent.parent
FIXTURE_DIR = Path("crates/crible-encode/tests/fixtures/fake-model")
MODEL_NAME = "fake-model"
VOCAB = (
    "[UNK]",
    "query:",
    "document:",
    "droit",
    "contrat",
    "personne",
    "responsabilite",
    "code",
    "article",
    "la",
    "du",
    "des",
    "fait",
    "choses",
    "tribunal",
    "juge",
)
PROMPTS = {"query": "query: ", "document": "document: "}


def tokenizer_json() -> dict[str, object]:
    """Return the WordLevel tokenizer configuration.

    ``WhitespaceSplit`` keeps the prompt ``query:`` as one token, which the test
    of the token count relies on. There is no truncation and no post-processor,
    so the ids of an encoding are exactly the words of the input.
    """
    return {
        "version": "1.0",
        "truncation": None,
        "padding": None,
        "added_tokens": [
            {
                "id": 0,
                "content": "[UNK]",
                "single_word": False,
                "lstrip": False,
                "rstrip": False,
                "normalized": False,
                "special": True,
            }
        ],
        "normalizer": {"type": "Lowercase"},
        "pre_tokenizer": {"type": "WhitespaceSplit"},
        "post_processor": None,
        "decoder": None,
        "model": {
            "type": "WordLevel",
            "vocab": {word: index for index, word in enumerate(VOCAB)},
            "unk_token": "[UNK]",
        },
    }


def _build_graph(scale: float) -> bytes:
    """Return the serialized ONNX graph whose CLS row is ``[mean * scale, 1…]``.

    ``mean`` is the mean of the input ids, so the first component of the
    normalized vector carries the token count and the token values: a test can
    compare it with the mean of the ids it tokenized, and a tokenizer that
    truncates at four tokens produces another vector. The fp32 graph uses
    ``scale = 1``; the INT8 graph uses ``scale = 2``, which makes the two
    precisions produce distinct vectors, as two real graphs would.
    """
    import numpy as np
    import onnx
    from onnx import TensorProto, helper, numpy_helper

    nodes = [
        helper.make_node("Cast", ["input_ids"], ["ids_float"], to=TensorProto.FLOAT),
        helper.make_node(
            "ReduceMean", ["ids_float"], ["mean_id"], axes=[1], keepdims=1
        ),
    ]
    if scale != 1.0:
        nodes.append(helper.make_node("Mul", ["mean_id", "scale"], ["scaled_mean"]))
        mean_name = "scaled_mean"
    else:
        mean_name = "mean_id"
    nodes.extend(
        [
            helper.make_node(
                "Concat", [mean_name, "ones"], ["row"], axis=1
            ),
            helper.make_node("Expand", ["row", "row_shape"], ["pooled"]),
            helper.make_node("Shape", ["input_ids"], ["sequence_shape"]),
            helper.make_node(
                "Concat", ["sequence_shape", "dims_shape"], ["target_shape"], axis=0
            ),
            helper.make_node(
                "Expand", ["pooled", "target_shape"], ["last_hidden_state"]
            ),
        ]
    )
    initializers = [
        numpy_helper.from_array(np.ones((1, DIMS - 1), dtype=np.float32), "ones"),
        numpy_helper.from_array(np.array([1, 1, DIMS], dtype=np.int64), "row_shape"),
        numpy_helper.from_array(np.array([DIMS], dtype=np.int64), "dims_shape"),
    ]
    if scale != 1.0:
        initializers.append(
            numpy_helper.from_array(np.array([scale], dtype=np.float32), "scale")
        )
    graph = helper.make_graph(
        nodes,
        "fake_encoder",
        [
            helper.make_tensor_value_info("input_ids", TensorProto.INT64, [1, "seq"]),
            helper.make_tensor_value_info(
                "attention_mask", TensorProto.INT64, [1, "seq"]
            ),
        ],
        [
            helper.make_tensor_value_info(
                "last_hidden_state", TensorProto.FLOAT, [1, "seq", DIMS]
            )
        ],
        initializer=initializers,
    )
    model = helper.make_model(graph, opset_imports=[helper.make_opsetid("", 17)])
    model.ir_version = 10
    onnx.checker.check_model(model)
    # Fixed bytes: protobuf serializes fields in field order here, and the
    # deterministic flag protects against map ordering changes.
    return model.SerializeToString(deterministic=True)


def _digest(data: bytes) -> str:
    return blake3.blake3(data).hexdigest()


def write_fixture(out_dir: Path) -> dict[str, str]:
    """Write the four fixture files under ``out_dir`` and return their digests."""
    out_dir.mkdir(parents=True, exist_ok=True)
    fp32 = _build_graph(1.0)
    int8 = _build_graph(2.0)
    tokenizer = (json.dumps(tokenizer_json(), indent=2, sort_keys=True) + "\n").encode(
        "utf-8"
    )
    files = {
        "model.onnx": fp32,
        "model.int8.onnx": int8,
        "tokenizer.json": tokenizer,
    }
    for name, content in files.items():
        (out_dir / name).write_bytes(content)
    model_json = {
        "name": MODEL_NAME,
        "kind": "onnx",
        "hf_id": "crible/fake-encoder",
        "revision": "fixture",
        "dims": DIMS,
        "pooling": "cls",
        "normalize": True,
        "max_tokens": MAX_TOKENS,
        "prompts": PROMPTS,
        "files": {
            "fp32": {"path": "model.onnx", "blake3": _digest(fp32)},
            "int8": {"path": "model.int8.onnx", "blake3": _digest(int8)},
            "tokenizer": {"path": "tokenizer.json", "blake3": _digest(tokenizer)},
        },
    }
    model = (json.dumps(model_json, indent=2, sort_keys=True) + "\n").encode("utf-8")
    (out_dir / "model.json").write_bytes(model)
    digests = {name: _digest(content) for name, content in files.items()}
    digests["model.json"] = _digest(model)
    return digests


def main(argv: list[str] | None = None) -> int:
    """Write the fixture; return 0 on success."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out",
        type=Path,
        default=ROOT / FIXTURE_DIR,
        help="fixture directory (default: %(default)s)",
    )
    arguments = parser.parse_args(argv)
    digests = write_fixture(arguments.out)
    for name, digest in digests.items():
        print(f"{name}: {digest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
