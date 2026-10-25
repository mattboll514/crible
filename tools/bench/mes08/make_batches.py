# SPDX-License-Identifier: Apache-2.0

"""Build the fixtures of MES-07 and MES-08 (P3, `MESURER-CYCLE-DE-VIE`).

Three subcommands, all deterministic for a given seed:

``daily``
    Writes one "daily type" upsert batch of about ``--passages`` passages: new
    documents whose passages carry the texts and (optionally) the vectors of a
    source batch. The text length is therefore the real one of the source
    corpus, which is what the zstd gain of ``REUTILISER-COMPRESSEUR-ZSTD``
    depends on.

``deletions``
    Writes an upsert batch made only of deletions, read from the
    ``records.sqlite`` of an index. Attaching it tombstones the passages of the
    documents it names, which is how the compaction fixtures reach a dead ratio
    without any write outside the engine.

``lengths``
    Reports the byte length of the texts of a source batch (mean, median,
    percentiles), for the report.

Every batch goes through the Python `BatchWriter` of `clients/python`, so the
manifest, the checksums and the Parquet schema are the production ones. No
Python or Rust production code is touched.

Examples::

    cd tools && uv run --no-sync python bench/mes08/make_batches.py daily \\
        --config ../data/legifacile-sample/configs/articles.json \\
        --model ../data/models/mdenseon/model.json \\
        --text-source ../data/legifacile-sample/batches/legifacile-sample-articles-v1 \\
        --passages 1000 --out ../data/mes08/batches --batch-id daily-articles-1000

    cd tools && uv run --no-sync python bench/mes08/make_batches.py deletions \\
        --config ../data/legifacile-sample/configs/articles.json \\
        --model ../data/models/mdenseon/model.json \\
        --index-dir ../data/mes07/indexes/articles \\
        --fraction 0.10 --out ../data/mes07/batches --batch-id articles-deletions
"""

from __future__ import annotations

import argparse
import datetime
import json
import sqlite3
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any

import blake3
import numpy as np
import pyarrow.parquet as pq

from crible import BatchWriter

DIMS = 768
CREATED_AT = "2026-10-25T00:00:00Z"
HOST = "mes08_fixtures"
CHUNKER = {"name": "mes-daily", "version": "1", "params": {}}
TEMPLATE = {"name": "mes-daily-text", "version": "1"}


def index_config(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def batch_model(config: dict[str, Any], model_path: Path | None) -> dict[str, Any]:
    """The `model` member the `BatchWriter` turns into the manifest model."""
    if model_path is None:
        return {"kind": "external", "dims": DIMS, "normalize": True}
    model = json.loads(model_path.read_text(encoding="utf-8"))
    if model["name"] != config["model"]:
        raise SystemExit(
            f"model.json names '{model['name']}' but the index names '{config['model']}'"
        )
    return {
        "kind": model["kind"],
        "hf_id": model["hf_id"],
        "revision": model["revision"],
        "pooling": model["pooling"],
        "prompts": {"document": model["prompts"]["document"]},
        "dims": model["dims"],
        "normalize": model["normalize"],
    }


def document_field_names(config: dict[str, Any]) -> list[str]:
    return sorted(config["schema"].get("document_fields", {}))


def passage_field_names(config: dict[str, Any]) -> list[str]:
    return sorted(config["schema"].get("passage_fields", {}))


def source_documents(source: Path, config: dict[str, Any], wanted: int):
    """Yields ``(fields, [(text, vector_bytes, passage_fields)])`` until ``wanted``
    passages are covered; the last document is kept whole."""
    documents = pq.read_table(source / "documents.parquet").to_pylist()
    passages: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for part in sorted(source.glob("passages-*.parquet")):
        for row in pq.read_table(part).to_pylist():
            passages[row["doc_id"]].append(row)
    doc_fields = document_field_names(config)
    pass_fields = passage_field_names(config)
    covered = 0
    for document in documents:
        rows = passages.get(document["doc_id"], [])
        if not rows:
            continue
        fields = {name: document.get(name) for name in doc_fields}
        carried = [
            (
                row["text"],
                row["vector"],
                {name: row.get(name) for name in pass_fields},
            )
            for row in rows
        ]
        yield fields, carried
        covered += len(carried)
        if covered >= wanted:
            return


def write_daily(arguments: argparse.Namespace) -> Path:
    config = index_config(arguments.config)
    model = batch_model(config, arguments.model)
    source = arguments.text_source
    rng = np.random.default_rng(arguments.seed)
    writer = BatchWriter(
        config,
        arguments.out,
        arguments.batch_id,
        "upsert",
        model,
        CHUNKER,
        TEMPLATE,
        created_at=CREATED_AT,
        host=HOST,
    )
    index = 0
    for fields, rows in source_documents(source, config, arguments.passages):
        passages = []
        for text, vector_bytes, passage_fields in rows:
            if arguments.vectors == "random":
                vector = rng.standard_normal(DIMS)
                vector = (vector / np.linalg.norm(vector)).astype(np.float32)
            else:
                vector = np.frombuffer(vector_bytes, dtype="<f2").astype(np.float32)
            passages.append({"text": text, "vector": vector, **passage_fields})
        digest = blake3.blake3(f"{arguments.batch_id}:{index}".encode()).hexdigest()
        writer.add_document(
            f"{arguments.batch_id}-doc-{index:06d}", digest, fields, passages
        )
        index += 1
        if index >= arguments.max_documents:
            break
    return writer.close()


def write_deletions(arguments: argparse.Namespace) -> Path:
    config = index_config(arguments.config)
    model = batch_model(config, arguments.model)
    index_dir = arguments.index_dir
    epoch = (index_dir / "CURRENT_EPOCH").read_text(encoding="utf-8").strip()
    database = index_dir / "epochs" / epoch / "records.sqlite"
    connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
    try:
        doc_ids = [
            row[0]
            for row in connection.execute("SELECT doc_id FROM documents ORDER BY doc_id")
        ]
    finally:
        connection.close()
    if not doc_ids:
        raise SystemExit(f"index '{index_dir}' holds no document")
    wanted = max(1, round(len(doc_ids) * arguments.fraction))
    # Deterministic spread: one document out of every `step`, so the deleted
    # documents cover the whole corpus instead of a prefix of the order.
    step = len(doc_ids) / wanted
    chosen = [doc_ids[int(i * step)] for i in range(wanted)]
    writer = BatchWriter(
        config,
        arguments.out,
        arguments.batch_id,
        "upsert",
        model,
        CHUNKER,
        TEMPLATE,
        created_at=CREATED_AT,
        host=HOST,
    )
    for doc_id in chosen:
        writer.delete(doc_id)
    return writer.close()


def report_lengths(arguments: argparse.Namespace) -> int:
    lengths = []
    for part in sorted(arguments.batch.glob("passages-*.parquet")):
        for text in pq.read_table(part, columns=["text"]).column("text").to_pylist():
            lengths.append(len(text.encode("utf-8")))
    values = np.array(sorted(lengths))
    payload = {
        "batch": str(arguments.batch),
        "passages": int(values.size),
        "bytes_mean": float(values.mean()),
        "bytes_p50": int(np.percentile(values, 50)),
        "bytes_p90": int(np.percentile(values, 90)),
        "bytes_p99": int(np.percentile(values, 99)),
        "bytes_max": int(values.max()),
    }
    print(json.dumps(payload, indent=2, sort_keys=True))
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)

    daily = commands.add_parser("daily", help="write one daily-type upsert batch")
    daily.add_argument("--config", type=Path, required=True)
    daily.add_argument("--model", type=Path, default=None)
    daily.add_argument("--text-source", type=Path, required=True)
    daily.add_argument("--passages", type=int, default=1000)
    daily.add_argument("--max-documents", type=int, default=100_000)
    daily.add_argument("--vectors", choices=("source", "random"), default="source")
    daily.add_argument("--out", type=Path, required=True)
    daily.add_argument("--batch-id", required=True)
    daily.add_argument("--seed", type=int, default=20261025)
    daily.set_defaults(handler=write_daily)

    deletions = commands.add_parser("deletions", help="write a deletions-only batch")
    deletions.add_argument("--config", type=Path, required=True)
    deletions.add_argument("--model", type=Path, default=None)
    deletions.add_argument("--index-dir", type=Path, required=True)
    deletions.add_argument("--fraction", type=float, default=0.10)
    deletions.add_argument("--out", type=Path, required=True)
    deletions.add_argument("--batch-id", required=True)
    deletions.set_defaults(handler=write_deletions)

    lengths = commands.add_parser("lengths", help="report the text lengths of a batch")
    lengths.add_argument("--batch", type=Path, required=True)
    lengths.set_defaults(handler=report_lengths)

    arguments = parser.parse_args(argv)
    out = arguments.out if hasattr(arguments, "out") else Path()
    out.mkdir(parents=True, exist_ok=True)
    result = arguments.handler(arguments)
    if isinstance(result, Path):
        print(result)
    return 0


if __name__ == "__main__":
    sys.exit(main())
