# SPDX-License-Identifier: Apache-2.0

"""Acceptance criterion 2 of ``ENCODER-DOCUMENTS-EN-LIGNE``, end to end.

An online lot (``crible ingest``) and the equivalent batch written by
:class:`crible.BatchWriter` carry the same vectors. Attached to two indexes of
the same configuration, they must return the same search response.

This is the literal form of the criterion: the reference batch is produced by
the Python client, not by a hand-written test fixture. Both indexes use an
``external`` model of ``DIMS`` dimensions, so the test needs no ONNX Runtime.

The test drives the ``crible`` binary named by ``CRIBLE_BIN``, and skips
without it: the Python job of CI has no Rust toolchain, and a test never builds
one. The Rust job runs this file once ``cargo test`` has produced
``target/debug/crible``, with ``CRIBLE_BIN`` pointing at it. Run it the same way
from ``clients/python``:

```text
CRIBLE_BIN=../../target/debug/crible uv run pytest tests/test_online_batch.py
```
"""

from __future__ import annotations

import base64
import json
import os
import struct
import subprocess
from pathlib import Path

import numpy as np
import pytest

from crible import BatchWriter

DIMS = 8
REPO_ROOT = Path(__file__).resolve().parents[3]
ONLINE_INDEX = "en-ligne"
REFERENCE_INDEX = "reference"
EXTERNAL_MODEL = "external"
TEXTS = [
    "droit contrat",
    "personne responsabilite",
    "code article",
    "tribunal juge",
    "chose faite",
]


def unit_vector(index: int) -> list[float]:
    """Unit vector of the ``index``-th dimension: one passage per direction."""
    components = [0.0] * DIMS
    components[index % DIMS] = 1.0
    return components


def encoded_vector(index: int) -> str:
    """Base64 of the little-endian fp32 components, as ``docs/05`` wants them."""
    return base64.b64encode(struct.pack(f"<{DIMS}f", *unit_vector(index))).decode("ascii")


def index_config(name: str) -> dict:
    return {
        "name": name,
        "model": EXTERNAL_MODEL,
        "reference_precision": "fp16",
        "schema": {"document_fields": {}, "passage_fields": {}},
        "cluster_by": [],
        "lexical": {"enabled": False, "language": "fr", "positions": False},
        "search": {
            "scan": "none",
            "rescore_candidates": 10,
            "exact_threshold": 10,
            "candidates_per_branch": 10,
            "fusion": {"method": "rrf"},
        },
        "ingest": {"acceptance_sample": 1, "acceptance_min_cosine": 0.9},
        "compaction": {"dead_ratio": 0.1},
    }


def crible_command() -> list[str]:
    binary = os.environ.get("CRIBLE_BIN")
    if not binary:
        pytest.skip(
            "CRIBLE_BIN must name the crible binary; the Rust CI job sets it to "
            "target/debug/crible, built by cargo test"
        )
    # Resolved against the caller's directory, not against `REPO_ROOT`.
    return [str(Path(binary).resolve())]


def crible(*arguments: object) -> str:
    command = [*crible_command(), *(str(argument) for argument in arguments)]
    result = subprocess.run(
        command,
        cwd=REPO_ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, f"{' '.join(command)} failed:\n{result.stderr}"
    return result.stdout


def create_index(data: Path, root: Path, name: str) -> None:
    config = root / f"{name}.json"
    config.write_text(json.dumps(index_config(name)), encoding="utf-8")
    crible("index", "create", "--data", data, "--config", config)


def online_documents() -> list[dict]:
    return [
        {
            "doc_id": f"D{index}",
            "passages": [{"text": text, "vector": encoded_vector(index)}],
        }
        for index, text in enumerate(TEXTS)
    ]


def reference_batch(root: Path) -> Path:
    """Writes the equivalent batch with the Python client."""
    writer = BatchWriter(
        index_config(REFERENCE_INDEX),
        root,
        "reference-1",
        "upsert",
        {"name": EXTERNAL_MODEL, "kind": "external", "dims": DIMS, "normalize": True},
        {"name": "none", "version": "1"},
        {"name": "none", "version": "1"},
        created_at="1970-01-01T00:00:00Z",
        host="crible-tests",
    )
    for index, text in enumerate(TEXTS):
        writer.add_document(
            f"D{index}",
            f"blake3:{index:064x}",
            {},
            [{"text": text, "vector": np.array(unit_vector(index), dtype=np.float32)}],
        )
    return writer.close()


def search(data: Path, index: str, query: Path) -> dict:
    response = json.loads(
        crible(
            "search",
            "--data",
            data,
            "--index",
            index,
            "--vector-file",
            query,
            "--k",
            str(len(TEXTS)),
        )
    )
    # The only member that legitimately differs is the wall-clock duration.
    response.pop("took_ms")
    return response


def test_an_online_lot_matches_a_batch_written_by_the_client(tmp_path):
    data = tmp_path / "data"
    data.mkdir()
    models = tmp_path / "models"
    (models / EXTERNAL_MODEL).mkdir(parents=True)
    (models / EXTERNAL_MODEL / "model.json").write_text(
        json.dumps(
            {
                "name": EXTERNAL_MODEL,
                "kind": "external",
                "dims": DIMS,
                "normalize": True,
            }
        ),
        encoding="utf-8",
    )
    create_index(data, tmp_path, ONLINE_INDEX)
    create_index(data, tmp_path, REFERENCE_INDEX)

    input_file = tmp_path / "documents.jsonl"
    input_file.write_text(
        "".join(f"{json.dumps(document)}\n" for document in online_documents()),
        encoding="utf-8",
    )
    crible("ingest", "--data", data, "--models", models, "--index", ONLINE_INDEX, input_file)

    batch = reference_batch(tmp_path)
    crible("attach", "--data", data, "--models", models, "--index", REFERENCE_INDEX, batch)

    query = tmp_path / "query.json"
    query.write_text(json.dumps(unit_vector(0)), encoding="utf-8")
    online = search(data, ONLINE_INDEX, query)
    reference = search(data, REFERENCE_INDEX, query)

    assert online == reference, "the two lots carry the same vectors"
    assert [hit["doc_id"] for hit in online["results"]] == [
        f"D{index}" for index in range(len(TEXTS))
    ], "every document of the lot is retrieved, nearest first"
