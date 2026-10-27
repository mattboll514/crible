# SPDX-License-Identifier: Apache-2.0
"""Split the `synth-large` batch into many small batches (MES-02, multi-segment
campaign of ADR-0081).

The campaign of ADR-0081, decision 2, needs a measurement copy of `synth-large`
with **many small segments**, which no engine command produces: an attach writes
one segment per batch, so a multi-attach index needs many batches. This script
writes them, from the batch `data/synth-large/large-0001` of the shared dataset,
**without touching it**:

- it streams the documents and the passages of the source batch in file order and
  cuts them every `chunk_rows` rows, which gives `chunks` batches of the same
  size (the last one takes the remainder);
- each batch declares `chunk_rows` documents and passages, one passage per
  document, `ord = 0`, and its own `batch_id`, so the attach of one batch writes
  one segment;
- the rows keep the order of the source batch, so the internal identifiers the
  index assigns match the ones of `data/synth-large` row for row: the same
  deletions batch then produces the same live set, and the pools of the
  single-segment copy and of the multi-attach copy are identical.

The deletions batch is also copied, with its manifest pointed at the new index,
so the multi-attach copy can reach the same 90 % of live passages as the
reference copy.

Both outputs are written next to the measurement copies, under `data/` (git
ignored), and the script is idempotent: a chunk that already exists with the
expected size and checksum is not rewritten.

Run from the repository root::

    (cd tools && uv run --no-sync python bench/mes02/make_multiseg_batch.py \\
        --index synth-large-multi --chunks 100)
"""

from __future__ import annotations

import argparse
import json
import shutil
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq
from blake3 import blake3

#: Root of the repository, so the defaults stay readable.
ROOT = Path(__file__).resolve().parents[3]
#: The source batch of the shared dataset; read-only.
DEFAULT_SOURCE = ROOT / "data" / "synth-large" / "large-0001"
#: Where the chunks and the patched deletions batch go (git ignored).
DEFAULT_OUT = ROOT / "data" / "mes02" / "batches"
#: The deletions batch of `synth-large` (MES-07/MES-08), copied for the new index.
DEFAULT_DELETIONS = ROOT / "data" / "mes07" / "batches" / "synth-large-deletions-v1"
#: Rows read at once from a column-less stream.
READ_ROWS = 65536
#: Row group size of the written files, the one of the source batch.
WRITE_ROWS = 65536


def blake3_of(path: Path) -> str:
    """Returns `blake3:<hex>` of a file, streamed in bounded reads."""
    digest = blake3()
    with path.open("rb") as handle:
        while True:
            block = handle.read(8 * 1024 * 1024)
            if not block:
                break
            digest.update(block)
    return f"blake3:{digest.hexdigest()}"


def passage_files(source: Path) -> list[Path]:
    """The passage parts of the source batch, in numeric suffix order."""
    return sorted(source.glob("passages-*.parquet"))


def stream_rows(path: Path):
    """Yields Arrow record batches of one Parquet file, in row order."""
    read = pq.ParquetFile(path)
    yield from read.iter_batches(batch_size=READ_ROWS)


def batches_in_order(paths: list[Path]):
    """Yields every batch of `paths`, part after part, in row order."""
    for path in paths:
        yield from stream_rows(path)


def chunks_of(paths: list[Path], chunk_rows: int, total_rows: int):
    """Yields `(rows, tables)` for each chunk, sliced across part boundaries.

    The parts are read in a **single pass**: a chunk that ends in the middle of a
    record batch keeps the remainder as a pending slice, so no row is decoded
    twice and the cost stays one read of the source batch.
    """
    stream = batches_in_order(paths)
    pending: tuple[pa.RecordBatch, int] | None = None
    produced = 0
    while produced < total_rows:
        wanted = min(chunk_rows, total_rows - produced)
        pieces: list[pa.RecordBatch] = []
        taken = 0
        while taken < wanted:
            if pending is None:
                try:
                    pending = (next(stream), 0)
                except StopIteration:
                    break
            batch, offset = pending
            need = wanted - taken
            available = batch.num_rows - offset
            if available <= need:
                pieces.append(batch.slice(offset))
                taken += available
                pending = None
            else:
                pieces.append(batch.slice(offset, need))
                taken += need
                pending = (batch, offset + need)
        if taken != wanted:
            raise ValueError(f"only {taken} rows of {wanted} could be read")
        produced += wanted
        yield wanted, pieces


def write_chunk(
    directory: Path,
    documents: list[pa.RecordBatch],
    passages: list[pa.RecordBatch],
    source_manifest: dict,
    index: str,
    batch_id: str,
    rows: int,
) -> dict:
    """Writes one chunk batch and returns its manifest."""
    directory.mkdir(parents=True, exist_ok=True)
    documents_path = directory / "documents.parquet"
    passages_path = directory / "passages-0001.parquet"
    pq.write_table(
        pa.Table.from_batches(documents),
        documents_path,
        compression="snappy",
        row_group_size=WRITE_ROWS,
    )
    pq.write_table(
        pa.Table.from_batches(passages),
        passages_path,
        compression="snappy",
        row_group_size=WRITE_ROWS,
    )
    manifest = json.loads(json.dumps(source_manifest))
    manifest["batch_id"] = batch_id
    manifest["index"] = index
    manifest["counts"] = {"documents": rows, "passages": rows, "deletions": 0}
    manifest["files"] = {
        "documents.parquet": blake3_of(documents_path),
        "passages-0001.parquet": blake3_of(passages_path),
    }
    (directory / "manifest.json").write_text(
        json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    return manifest


def write_deletions(source: Path, out: Path, index: str, batch_id: str) -> dict:
    """Copies the deletions batch with its manifest pointed at `index`."""
    out.mkdir(parents=True, exist_ok=True)
    original = json.loads((source / "manifest.json").read_text(encoding="utf-8"))
    for name in original["files"]:
        shutil.copyfile(source / name, out / name)
    manifest = json.loads(json.dumps(original))
    manifest["batch_id"] = batch_id
    manifest["index"] = index
    (out / "manifest.json").write_text(
        json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    return manifest


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--source", type=Path, default=DEFAULT_SOURCE)
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    parser.add_argument("--deletions", type=Path, default=DEFAULT_DELETIONS)
    parser.add_argument("--index", required=True)
    parser.add_argument("--chunks", type=int, default=100)
    arguments = parser.parse_args(argv)

    source = arguments.source
    source_manifest = json.loads((source / "manifest.json").read_text(encoding="utf-8"))
    total = int(source_manifest["counts"]["documents"])
    if int(source_manifest["counts"]["passages"]) != total:
        raise ValueError("the source batch does not hold one passage per document")
    chunk_rows = -(-total // arguments.chunks)
    written = total - chunk_rows * (arguments.chunks - 1)

    documents_paths = [source / "documents.parquet"]
    passages_paths = passage_files(source)

    root = arguments.out / arguments.index
    if root.exists():
        raise SystemExit(
            f"{root} already exists: remove it by hand before rebuilding the multi-attach copy"
        )
    root.mkdir(parents=True)

    document_chunks = chunks_of(documents_paths, chunk_rows, total)
    passage_chunks = chunks_of(passages_paths, chunk_rows, total)
    for number in range(1, arguments.chunks + 1):
        rows, documents = next(document_chunks)
        passage_rows, passages = next(passage_chunks)
        if rows != passage_rows:
            raise ValueError(f"chunk {number}: {rows} documents, {passage_rows} passages")
        batch_id = f"multi-{number:04d}"
        write_chunk(
            root / batch_id,
            documents,
            passages,
            source_manifest,
            arguments.index,
            batch_id,
            rows,
        )
        if number % 10 == 0 or number == arguments.chunks:
            print(f"chunk {number}/{arguments.chunks}: {rows} rows", flush=True)

    deletions = write_deletions(
        arguments.deletions,
        arguments.out / f"{arguments.index}-deletions-v1",
        arguments.index,
        f"{arguments.index}-deletions-v1",
    )
    print(
        f"{arguments.chunks} chunks of {chunk_rows} rows ({written} for the last one) "
        f"under {root}, deletions batch with {deletions['counts']['deletions']} ids"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
