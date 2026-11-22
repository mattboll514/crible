#!/usr/bin/env python3
"""Derive one attachable lot per structure of `data/legifacile-sample`.

MES-17 attaches several real lots to a single throwaway index. An index has one
schema, and the six sample lots carry disjoint document fields, so each derived
lot is padded with a null column for every field its structure does not carry.
Text, doc_id, doc_hash, ord and the fp16 vector bytes are copied verbatim.
Counts are recounted, and the blake3 checksums of the manifest are recomputed.

Carried over from the first campaign (`data/mes17/prepare_lots.py`, MES-17 of
2026-11-12) for ADR-0157, decision 8.

Requirements: `pyarrow` and `blake3`, the same development-only tools as the
first campaign and as `tools/bench/mes02/make_multiseg_batch.py`. The measuring
side (`measure.py`) needs the standard library only.

Output: `data/mes17/exchange/inbox/<lot>/` by default (gitignored, raw data of
the measure). Use `--out` to derive elsewhere.
"""

import argparse
import json
import pathlib
import shutil

import pyarrow as pa
import pyarrow.parquet as pq

REPO = pathlib.Path(__file__).resolve().parents[3]
SAMPLE = REPO / "data" / "legifacile-sample" / "batches"

DOCUMENT_KEYWORDS = [
    "fonds", "etat", "code", "num", "texte_cid", "section",
    "juridiction", "numero", "nature", "theme", "type",
]
PASSAGE_KEYWORDS = ["partie"]


def pad(table: pa.Table, keywords, integers, schema: pa.Schema) -> pa.Table:
    for name in keywords:
        if name not in schema.names:
            table = table.append_column(name, pa.nulls(table.num_rows, pa.string()))
    for name in integers:
        if name not in schema.names:
            table = table.append_column(name, pa.nulls(table.num_rows, pa.int64()))
    return table


def blake3(path: pathlib.Path) -> str:
    import blake3 as b3

    hasher = b3.blake3()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            hasher.update(chunk)
    return "blake3:" + hasher.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", default="data/mes17/exchange/inbox")
    parser.add_argument("--summary", default="data/mes17/exchange/derived-lots.json")
    parser.add_argument("--index", default="mes17")
    args = parser.parse_args()

    out = (REPO / args.out).resolve()
    summary_path = (REPO / args.summary).resolve()
    out.mkdir(parents=True, exist_ok=True)
    summary_path.parent.mkdir(parents=True, exist_ok=True)

    summary = []
    for source in sorted(SAMPLE.iterdir()):
        if not source.is_dir():
            continue
        manifest = json.loads((source / "manifest.json").read_text())
        lot = out / ("mes17-" + source.name)
        if lot.exists():
            shutil.rmtree(lot)
        lot.mkdir(parents=True)

        documents = pq.read_table(source / "documents.parquet")
        passages = pq.read_table(source / "passages-0001.parquet")
        documents = pad(documents, DOCUMENT_KEYWORDS, ["annee"], documents.schema)
        passages = pad(passages, PASSAGE_KEYWORDS, [], passages.schema)
        pq.write_table(documents, lot / "documents.parquet", compression="zstd")
        pq.write_table(passages, lot / "passages-0001.parquet", compression="zstd")

        manifest["index"] = args.index
        manifest["counts"]["documents"] = documents.num_rows
        manifest["counts"]["passages"] = passages.num_rows
        manifest["files"] = {
            "documents.parquet": blake3(lot / "documents.parquet"),
            "passages-0001.parquet": blake3(lot / "passages-0001.parquet"),
        }
        (lot / "manifest.json").write_text(
            json.dumps(manifest, indent=2, ensure_ascii=False) + "\n")
        summary.append(
            {
                "source": source.name,
                "lot": lot.name,
                "documents": documents.num_rows,
                "passages": passages.num_rows,
                "document_columns": documents.schema.names,
                "passage_columns": passages.schema.names,
            }
        )
        print(lot, documents.num_rows, "documents", passages.num_rows, "passages",
              flush=True)

    summary_path.write_text(json.dumps(summary, indent=2) + "\n")


if __name__ == "__main__":
    main()
