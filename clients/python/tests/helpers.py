# SPDX-License-Identifier: Apache-2.0

"""Helpers shared by the tests of the ``crible`` Python package.

The tests marked ``server`` launch the binary named by ``CRIBLE_BIN``. They fail
without it, with the command that produces it: a test without its prerequisite
is red, not green (ADR-0112, decision 6). The Rust job of the CI runs
``uv run pytest -m server`` against ``target/debug/crible``.
"""

from __future__ import annotations

import hashlib
import json
import os
import socket
import subprocess
from pathlib import Path
from typing import Any, Mapping, Sequence

import numpy as np
import pytest

from crible import BatchWriter

REPO_ROOT = Path(__file__).resolve().parents[3]

#: Model of every index of the tests: it encodes nothing, so no ONNX Runtime.
EXTERNAL_MODEL = "external"
#: Second external model, of another fingerprint (ADR-0028, `incompatible_model`).
WIDE_MODEL = "external-wide"
DIMS = 8
WIDE_DIMS = 16

#: Test keys of the three roles (ADR-0100); the hash is their SHA-256.
KEYS = {
    "read": "crible-test-key-read",
    "write": "crible-test-key-write",
    "admin": "crible-test-key-admin",
}


def crible_binary() -> Path:
    """Path of the ``crible`` binary, named by ``CRIBLE_BIN``.

    Fails, without skipping, when the variable is absent or names no file
    (ADR-0112, decision 6).
    """
    binary = os.environ.get("CRIBLE_BIN")
    if not binary:
        pytest.fail(
            "CRIBLE_BIN must name the crible binary. Build it and export the path:\n"
            "  cargo build -p crible\n"
            "  CRIBLE_BIN=$PWD/target/debug/crible uv run pytest -m server"
        )
    path = Path(binary).resolve()
    if not path.is_file():
        pytest.fail(
            f"CRIBLE_BIN names '{path}', which is not a file. "
            "`cargo build -p crible` writes target/debug/crible."
        )
    return path


def crible(*arguments: object) -> str:
    """Runs the binary and returns its standard output."""
    command = [str(crible_binary()), *(str(argument) for argument in arguments)]
    result = subprocess.run(
        command,
        cwd=REPO_ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, f"{' '.join(command)} failed:\n{result.stderr}"
    return result.stdout


def sha256(text: str) -> str:
    """Hash of an API key, as ``crible.toml`` expects it (ADR-0100)."""
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def model_config(name: str, dims: int) -> dict[str, Any]:
    """Configuration of an ``external`` model of ``dims`` dimensions."""
    return {"name": name, "kind": "external", "dims": dims, "normalize": True}


def index_config(
    name: str, *, model: str = EXTERNAL_MODEL, lexical: bool = False
) -> dict[str, Any]:
    """Configuration of an index of the tests: dense only, exact scan.

    ``lexical`` stays false, so no test depends on a tokenizer or on an
    analysis chain. The schema carries one document field and one passage
    field, so a filter and a metadata patch have something to name.
    """
    return {
        "name": name,
        "model": model,
        "reference_precision": "fp16",
        "schema": {
            "document_fields": {"juridiction": {"type": "keyword"}},
            "passage_fields": {"partie": {"type": "keyword"}},
        },
        "cluster_by": [],
        "lexical": {"enabled": lexical, "language": "fr", "positions": False},
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


def unit_vector(dimension: int, dims: int = DIMS) -> list[float]:
    """Unit vector of one dimension, so one passage per direction."""
    components = [0.0] * dims
    components[dimension % dims] = 1.0
    return components


def document(doc_id: str, dimension: int, *, juridiction: str = "cass") -> dict[str, Any]:
    """One document of an online batch or of a written batch."""
    return {
        "doc_id": doc_id,
        "fields": {"juridiction": juridiction},
        "passages": [
            {
                "text": f"passage {doc_id}",
                "fields": {"partie": "motifs"},
                "vector": unit_vector(dimension),
            }
        ],
    }


def write_batch(
    out_dir: Path,
    batch_id: str,
    config: Mapping[str, Any],
    documents: Sequence[Mapping[str, Any]],
) -> Path:
    """Writes a batch of the tests with :class:`crible.BatchWriter`.

    Data come from the test, never from ``data/``: no test ``server`` depends
    on the sample (ADR-0112, decision 6).
    """
    model = str(config["model"])
    writer = BatchWriter(
        config,
        out_dir,
        batch_id,
        "upsert",
        model_config(model, DIMS if model == EXTERNAL_MODEL else WIDE_DIMS),
        {"name": "none", "version": "1"},
        {"name": "none", "version": "1"},
        created_at="1970-01-01T00:00:00Z",
        host="crible-tests",
    )
    for entry in documents:
        writer.add_document(
            str(entry["doc_id"]),
            f"blake3:{hashlib.sha256(str(entry['doc_id']).encode('utf-8')).hexdigest()}",
            entry.get("fields", {}),
            [
                {
                    "text": passage["text"],
                    "vector": np.array(passage["vector"], dtype=np.float32),
                    **passage.get("fields", {}),
                }
                for passage in entry["passages"]
            ],
        )
    return writer.close()


def free_port() -> int:
    """One free TCP port on the loopback interface."""
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return int(probe.getsockname()[1])


def write_model(models: Path, name: str, dims: int) -> None:
    """Writes ``models/<name>/model.json``, which ``create_index`` reads first."""
    directory = models / name
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "model.json").write_text(
        json.dumps(model_config(name, dims)), encoding="utf-8"
    )
