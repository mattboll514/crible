# SPDX-License-Identifier: Apache-2.0

"""Generate the synthetic Crible datasets and their query set.

``synth-small`` is the reference dataset of phase P1 (``docs/08-invariants-et-tests.md``,
section "Jeux de données"):

- 2 000 documents and 10 000 passages, 768 dimensions;
- fields of every declared type, at document and passage level, with nulls;
- passage vectors drawn around 16 cluster centres, so that a dense search
  finds neighbours instead of noise;
- one fixed seed, so two runs write byte-identical files.

The generator writes three upsert batches, attached in this order by J1:

====================  ===========================================
batch                 content
====================  ===========================================
``l1``                creation: 2 000 documents, 10 000 passages
``l2``                update: 300 modified documents, 100 new ones
``l3``                update: 150 deletions, 50 metadata-only updates
====================  ===========================================

A metadata-only update sends the same ``doc_hash`` and all the passages of the
document again, and changes only document fields (``docs/04-donnees-et-ingestion.md``,
"``doc_hash`` identique": passages ignored, document fields updated).

``--large`` writes the latency dataset: random normalized vectors, one passage
per document, split into parts by :class:`crible.BatchWriter`. The full size is
8.6 M passages (about 13 GB in fp16), so the option takes ``--large-passages``
to generate a smaller file set.

Determinism
-----------

Every random draw comes from a named stream of ``SeedSequence([seed, stream])``,
so adding a stream never shifts another one. File order is sorted, JSON keys are
sorted, ``created_at`` is derived from the seed only (see
:func:`batch_created_at`), and ``producer.host`` is the stable identifier
``"gen_synth"`` (ADR-0036). The checksum of the dataset is therefore the same on
every machine.
"""

from __future__ import annotations

import argparse
import datetime
import json
import shutil
import sys
from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import blake3
import numpy as np

from crible import BatchWriter

#: Fixed seed of ``synth-small``. Change it only with the recorded checksum.
SEED = 20261021
#: Dimensions of every vector of the dataset.
DIMS = 768
#: Documents and passages of the creation batch.
DOCUMENTS = 2_000
PASSAGES_PER_DOCUMENT = 5
#: Documents of the update batch.
MODIFIED_DOCUMENTS = 300
NEW_DOCUMENTS = 100
#: Operations of the third batch.
DELETED_DOCUMENTS = 150
METADATA_ONLY_DOCUMENTS = 50
#: Queries of ``queries.jsonl``.
QUERIES = 200
#: Number of passage clusters; the document field ``categorie`` names it.
CLUSTERS = 16
#: Relative norm of the noise added to a cluster centre.
CLUSTER_NOISE = 0.35

#: Size of the latency dataset.
LARGE_PASSAGES = 8_600_000
#: Rows generated at once for the latency dataset.
LARGE_BLOCK_ROWS = 8_192

#: Value of ``producer.host``: a stable producer identifier (ADR-0036), so the
#: dataset checksum is the same on every machine.
HOST = "gen_synth"

#: Base of the manifest timestamps; the seed adds a fixed offset.
CREATED_AT_BASE = datetime.datetime(2026, 1, 1, tzinfo=datetime.timezone.utc)

MODEL = {
    "name": "synthetic-external",
    "kind": "external",
    "dims": DIMS,
    "normalize": True,
}
CHUNKER = {
    "name": "synthetic-units",
    "version": "1",
    "params": {"passages_per_document": PASSAGES_PER_DOCUMENT},
}
#: The latency dataset writes one passage per document, so it declares its own
#: chunker instead of reusing the five-passage one of ``synth-small``.
LARGE_CHUNKER = {
    "name": "synthetic-units",
    "version": "1",
    "params": {"passages_per_document": 1},
}
TEMPLATE = {"name": "synthetic-text", "version": "1"}
INDEX_NAME = "synth-small"

_SCHEMA = {
    "document_fields": {
        "annee": {"type": "integer"},
        "categorie": {"type": "keyword"},
        "date_publication": {"type": "date"},
        "publie": {"type": "bool"},
        "score": {"type": "float"},
        "source": {"type": "keyword"},
    },
    "passage_fields": {
        "date_effet": {"type": "date"},
        "partie": {"type": "keyword"},
        "poids": {"type": "float"},
        "principal": {"type": "bool"},
        "rang": {"type": "integer"},
    },
}

_SOURCES = ("journal", "code", "decret", "constitution")
_PARTS = ("titre", "corps", "motifs", "dispositif", "annexe")
_WORDS = (
    "alpha",
    "beta",
    "gamma",
    "delta",
    "epsilon",
    "zeta",
    "eta",
    "theta",
    "iota",
    "kappa",
    "lambda",
    "mu",
    "nu",
    "xi",
    "omicron",
    "pi",
)


@dataclass(frozen=True)
class Passage:
    """One passage of the synthetic dataset."""

    text: str
    vector: np.ndarray
    fields: Mapping[str, Any]


@dataclass(frozen=True)
class Document:
    """One document with all its passages."""

    doc_id: str
    doc_hash: str
    fields: Mapping[str, Any]
    passages: tuple[Passage, ...]


@dataclass(frozen=True)
class Dataset:
    """Summary of one generated ``synth-small`` directory."""

    out_dir: Path
    documents: int
    passages: int
    deletions: int
    batches: tuple[str, ...]


def index_config(name: str = INDEX_NAME) -> dict[str, Any]:
    """Configuration of the synthetic index, written as ``index.json``."""
    return {
        "name": name,
        "model": MODEL["name"],
        "reference_precision": "fp16",
        "schema": _SCHEMA,
        "cluster_by": ["categorie"],
        "lexical": {"enabled": False, "language": "fr", "positions": False},
        "search": {
            "scan": "none",
            "rescore_candidates": 1000,
            "exact_threshold": 1000,
            "candidates_per_branch": 100,
            "fusion": {"method": "rrf"},
        },
        "ingest": {"acceptance_sample": 200, "acceptance_min_cosine": 0.999},
        "compaction": {"dead_ratio": 0.10},
    }


def batch_created_at(seed: int, offset_seconds: int = 0) -> str:
    """Returns the manifest timestamp of one batch.

    It depends on the seed and the batch position only, never on the clock, so
    two runs write the same manifest.
    """
    moment = CREATED_AT_BASE + datetime.timedelta(
        seconds=seed % 86_400 + offset_seconds
    )
    return moment.strftime("%Y-%m-%dT%H:%M:%SZ")


def dataset_checksum(root: str | Path) -> str:
    """Returns the blake3 checksum of the known artifacts under ``root``.

    Only the files the generator writes, and the files listed by each batch
    manifest, are hashed: a stray ``.DS_Store`` or any other foreign file does
    not change the checksum. Paths are relative to ``root``, sorted, and hashed
    with their content, so the checksum covers the file list as well as the
    bytes.
    """
    directory = Path(root)
    paths = []
    for name in dict.fromkeys((*SMALL_FILES, *LARGE_FILES)):
        path = directory / name
        if path.is_file():
            paths.append(path)
    for batch_id in SMALL_BATCHES + LARGE_BATCHES:
        batch = directory / batch_id
        manifest_path = batch / "manifest.json"
        if not manifest_path.is_file():
            continue
        paths.append(manifest_path)
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        for name in manifest["files"]:
            path = batch / name
            if path.is_file():
                paths.append(path)
    hasher = blake3.blake3()
    for path in sorted(paths, key=lambda item: item.relative_to(directory).as_posix()):
        relative = path.relative_to(directory).as_posix()
        hasher.update(relative.encode("utf-8"))
        hasher.update(b"\0")
        hasher.update(blake3.blake3(path.read_bytes()).digest())
    return f"blake3:{hasher.hexdigest()}"


def _rng(seed: int, stream: int) -> np.random.Generator:
    """Returns the generator of one named stream."""
    return np.random.Generator(np.random.PCG64(np.random.SeedSequence([seed, stream])))


def _unit_vector(rng: np.random.Generator, dims: int = DIMS) -> np.ndarray:
    vector = rng.standard_normal(dims)
    return vector / np.linalg.norm(vector)


def _cluster_centres(rng: np.random.Generator) -> np.ndarray:
    centres = rng.standard_normal((CLUSTERS, DIMS))
    centres /= np.linalg.norm(centres, axis=1, keepdims=True)
    return centres


def _document_fields(rng: np.random.Generator, cluster: int) -> dict[str, Any]:
    return {
        "annee": None if rng.random() < 0.05 else int(rng.integers(1990, 2026)),
        "categorie": f"cat-{cluster:02d}",
        "date_publication": (
            None
            if rng.random() < 0.05
            else datetime.date(1990, 1, 1)
            + datetime.timedelta(days=int(rng.integers(0, 13_149)))
        ),
        "publie": None if rng.random() < 0.05 else bool(rng.random() < 0.8),
        "score": None if rng.random() < 0.05 else float(rng.random()),
        "source": (
            None
            if rng.random() < 0.10
            else _SOURCES[int(rng.integers(0, len(_SOURCES)))]
        ),
    }


def _passage_fields(rng: np.random.Generator, rank: int) -> dict[str, Any]:
    return {
        "date_effet": (
            None
            if rng.random() < 0.10
            else datetime.date(1990, 1, 1)
            + datetime.timedelta(days=int(rng.integers(0, 13_149)))
        ),
        "partie": (
            None
            if rng.random() < 0.10
            else _PARTS[int(rng.integers(0, len(_PARTS)))]
        ),
        "poids": None if rng.random() < 0.05 else float(rng.random()),
        "principal": None if rng.random() < 0.05 else bool(rng.random() < 0.6),
        "rang": None if rng.random() < 0.05 else rank,
    }


def _canonical(value: Any) -> str:
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=True,
        default=_json_default,
    )


def _json_default(value: Any) -> str:
    if isinstance(value, datetime.date):
        return value.isoformat()
    return str(value)


def _doc_hash(passages: Iterable[Passage]) -> str:
    """Returns the producer hash of the passages, independent of metadata.

    It covers the text, the passage fields and the stored fp16 vector, but not
    the document fields, so a metadata-only update keeps the same hash.
    """
    hasher = blake3.blake3()
    for passage in passages:
        hasher.update(b"\0")
        hasher.update(passage.text.encode("utf-8"))
        hasher.update(_canonical(passage.fields).encode("utf-8"))
        hasher.update(np.asarray(passage.vector, dtype="<f2").tobytes())
    return hasher.hexdigest()


def _text(rng: np.random.Generator, doc_id: str, rank: int) -> str:
    words = [str(_WORDS[int(rng.integers(0, len(_WORDS)))]) for _ in range(6)]
    return f"{doc_id} passage {rank} " + " ".join(words)


def _make_document(
    rng: np.random.Generator,
    centres: np.ndarray,
    doc_id: str,
    cluster: int,
) -> Document:
    passages = []
    for rank in range(PASSAGES_PER_DOCUMENT):
        direction = _unit_vector(rng)
        vector = centres[cluster] + CLUSTER_NOISE * direction
        vector /= np.linalg.norm(vector)
        passages.append(
            Passage(
                text=_text(rng, doc_id, rank),
                vector=vector,
                fields=_passage_fields(rng, rank),
            )
        )
    return Document(
        doc_id=doc_id,
        doc_hash=_doc_hash(passages),
        fields=_document_fields(rng, cluster),
        passages=tuple(passages),
    )


def _write_batch(
    out_dir: Path,
    batch_id: str,
    documents: Iterable[Document],
    created_at: str,
    deletions: Iterable[str] = (),
) -> Path:
    writer = BatchWriter(
        index_config(),
        out_dir,
        batch_id,
        "upsert",
        MODEL,
        CHUNKER,
        TEMPLATE,
        created_at=created_at,
        host=HOST,
    )
    for document in documents:
        writer.add_document(
            document.doc_id,
            document.doc_hash,
            document.fields,
            [
                {"text": passage.text, "vector": passage.vector, **passage.fields}
                for passage in document.passages
            ],
        )
    for doc_id in deletions:
        writer.delete(doc_id)
    return writer.close()


def _write_index_config(out_dir: Path, name: str = INDEX_NAME) -> None:
    text = json.dumps(index_config(name), indent=2, sort_keys=True, ensure_ascii=False)
    (out_dir / "index.json").write_text(text + "\n", encoding="utf-8")


def _metadata_only(document: Document) -> Document:
    """Changes document fields, keeps the hash and the passages byte for byte."""
    fields = dict(document.fields)
    year = fields["annee"]
    fields["annee"] = 2000 if year is None else (year + 1 if year < 2025 else 1990)
    published = fields["publie"]
    fields["publie"] = True if published is None else not published
    return Document(
        doc_id=document.doc_id,
        doc_hash=document.doc_hash,
        fields=fields,
        passages=document.passages,
    )


#: Root files the generator writes, per dataset kind.
SMALL_FILES = ("index.json", "queries.jsonl", "expected.jsonl")
LARGE_FILES = ("index.json",)
#: Batch directories the generator writes, per dataset kind.
SMALL_BATCHES = ("l1", "l2", "l3")
LARGE_BATCHES = ("large-0001",)


def _prepare_output(
    directory: Path,
    overwrite: bool,
    files: Iterable[str],
    batch_ids: Iterable[str],
) -> None:
    """Refuses a non-empty directory unless ``overwrite`` is explicit.

    ``overwrite`` removes only the root files given in ``files`` and the batch
    directories given in ``batch_ids``; anything else is left untouched. Those
    names are constant, so pass the dataset directory itself, never a parent
    such as ``data/``: a directory that is not a dataset directory could lose
    files with the same names.
    """
    if not directory.exists():
        directory.mkdir(parents=True)
        return
    if overwrite:
        for name in files:
            target = directory / name
            if target.is_file():
                target.unlink()
        for batch_id in batch_ids:
            target = directory / batch_id
            if target.is_dir():
                shutil.rmtree(target)
            elif target.is_file():
                target.unlink()
        return
    if any(directory.iterdir()):
        names = ", ".join((*files, *batch_ids))
        raise FileExistsError(
            f"output directory '{directory}' is not empty; remove it, or pass "
            f"--overwrite to replace the files this generator writes ({names})"
        )


def build_small(
    out_dir: str | Path, seed: int = SEED, overwrite: bool = False
) -> Dataset:
    """Writes ``synth-small``, its index configuration and its queries."""
    directory = Path(out_dir)
    _prepare_output(directory, overwrite, SMALL_FILES, SMALL_BATCHES)
    _write_index_config(directory)

    creation_rng = _rng(seed, 1)
    centres = _cluster_centres(creation_rng)
    documents: dict[str, Document] = {}
    for index in range(DOCUMENTS):
        doc_id = f"synth-{index:05d}"
        documents[doc_id] = _make_document(
            creation_rng, centres, doc_id, index % CLUSTERS
        )
    l1 = sorted(documents.values(), key=lambda document: document.doc_id)
    _write_batch(directory, "l1", l1, batch_created_at(seed))

    update_rng = _rng(seed, 2)
    modified = sorted(
        update_rng.choice(DOCUMENTS, MODIFIED_DOCUMENTS, replace=False).tolist()
    )
    for index in modified:
        doc_id = f"synth-{index:05d}"
        documents[doc_id] = _make_document(
            update_rng, centres, doc_id, (index + 1) % CLUSTERS
        )
    for offset in range(NEW_DOCUMENTS):
        index = DOCUMENTS + offset
        doc_id = f"synth-{index:05d}"
        documents[doc_id] = _make_document(
            update_rng, centres, doc_id, index % CLUSTERS
        )
    l2_ids = sorted(f"synth-{index:05d}" for index in modified) + sorted(
        f"synth-{DOCUMENTS + offset:05d}" for offset in range(NEW_DOCUMENTS)
    )
    _write_batch(
        directory,
        "l2",
        [documents[doc_id] for doc_id in l2_ids],
        batch_created_at(seed, 1),
    )

    third_rng = _rng(seed, 3)
    alive = sorted(documents)
    metadata_positions = np.sort(
        third_rng.choice(len(alive), METADATA_ONLY_DOCUMENTS, replace=False)
    )
    metadata_ids = {alive[int(position)] for position in metadata_positions}
    for doc_id in sorted(metadata_ids):
        documents[doc_id] = _metadata_only(documents[doc_id])
    deletion_candidates = [doc_id for doc_id in alive if doc_id not in metadata_ids]
    deleted_positions = third_rng.choice(
        len(deletion_candidates), DELETED_DOCUMENTS, replace=False
    )
    deleted = sorted(deletion_candidates[int(position)] for position in deleted_positions)
    _write_batch(
        directory,
        "l3",
        [documents[doc_id] for doc_id in sorted(metadata_ids)],
        batch_created_at(seed, 2),
        deleted,
    )

    for doc_id in deleted:
        del documents[doc_id]
    write_queries(directory, documents, centres, seed)

    return Dataset(
        out_dir=directory,
        documents=len(documents),
        passages=sum(len(document.passages) for document in documents.values()),
        deletions=len(deleted),
        batches=("l1", "l2", "l3"),
    )


def _values(documents: Mapping[str, Document], field: str) -> list[Any]:
    values = {
        fields[field]
        for _, fields in _document_field_rows(documents)
        if fields[field] is not None
    }
    return sorted(values, key=str)


def _document_field_rows(
    documents: Mapping[str, Document],
) -> Iterable[tuple[str, Mapping[str, Any]]]:
    for doc_id in sorted(documents):
        yield doc_id, documents[doc_id].fields


def _passage_values(documents: Mapping[str, Document], field: str) -> list[Any]:
    values = {
        passage.fields[field]
        for document in documents.values()
        for passage in document.passages
        if passage.fields[field] is not None
    }
    return sorted(values, key=str)


def _query_filter(
    rng: np.random.Generator,
    slot: int,
    documents: Mapping[str, Document],
) -> dict[str, Any] | None:
    """Returns one filter of the reference query set.

    The slots cover every operator of the grammar on values taken from the
    final state, so no generated filter selects an empty pool.
    """
    categories = _values(documents, "categorie")
    sources = _values(documents, "source")
    parts = _passage_values(documents, "partie")
    years = _values(documents, "annee")
    if slot < 40:
        return None
    if slot < 70:
        return {"categorie": categories[int(rng.integers(0, len(categories)))]}
    if slot < 85:
        first, second = rng.choice(len(categories), 2, replace=False)
        return {"categorie": {"in": [categories[int(first)], categories[int(second)]]}}
    if slot < 100:
        first = parts[int(rng.integers(0, len(parts)))]
        second = parts[(parts.index(first) + 1) % len(parts)]
        return {"partie": {"nin": [first, second]}}
    if slot < 120:
        year = int(years[int(rng.integers(0, len(years)))])
        return {"annee": {"gte": year, "lte": year + 5}}
    if slot < 135:
        return {"score": {"gt": 0.6}}
    if slot < 150:
        start = datetime.date(2000, 1, 1) + datetime.timedelta(
            days=int(rng.integers(0, 6_000))
        )
        stop = start + datetime.timedelta(days=1_095)
        return {
            "date_publication": {
                "gte": start.isoformat(),
                "lte": stop.isoformat(),
            }
        }
    if slot < 155:
        return {"publie": bool(slot % 2)}
    if slot < 160:
        return {"source": {"ne": "journal"}}
    if slot < 163:
        return {"source": {"exists": False}}
    if slot < 165:
        return {"source": {"exists": True}}
    if slot < 168:
        return {"date_effet": {"exists": False}}
    if slot < 170:
        return {"partie": {"exists": True}}
    if slot < 180:
        category = categories[int(rng.integers(0, len(categories)))]
        source = sources[int(rng.integers(0, len(sources)))]
        year = int(years[int(rng.integers(0, len(years)))])
        return {
            "categorie": category,
            "$or": [{"source": source}, {"annee": {"lt": year}}],
        }
    if slot < 185:
        category = categories[int(rng.integers(0, len(categories)))]
        return {"$and": [{"categorie": category}, {"publie": True}]}
    if slot < 190:
        return {"$not": {"source": "journal"}}
    if slot < 195:
        category = categories[int(rng.integers(0, len(categories)))]
        return {"$not": {"categorie": category}}
    return {"partie": parts[int(rng.integers(0, len(parts)))]}


def _query_vector(rng: np.random.Generator, centres: np.ndarray) -> np.ndarray:
    """Returns one unit query vector, stored as float32."""
    mode = rng.random()
    if mode < 0.6:
        vector = centres[int(rng.integers(0, len(centres)))] + 0.08 * _unit_vector(rng)
    elif mode < 0.8:
        first, second = rng.choice(len(centres), 2, replace=False)
        vector = centres[int(first)] + centres[int(second)] + 0.08 * _unit_vector(rng)
    else:
        vector = _unit_vector(rng)
    vector = vector.astype(np.float64)
    vector /= np.linalg.norm(vector)
    return vector.astype(np.float32)


def write_queries(
    out_dir: Path,
    documents: Mapping[str, Document],
    centres: np.ndarray,
    seed: int,
) -> Path:
    """Writes the 200 reference queries against the final state."""
    rng = _rng(seed, 4)
    target = out_dir / "queries.jsonl"
    with target.open("w", encoding="utf-8") as handle:
        for query_id in range(QUERIES):
            vector = _query_vector(rng, centres)
            record = {
                "query_id": query_id,
                "k": 10,
                "vector": [float(value) for value in vector],
                "filter": _query_filter(rng, query_id, documents),
            }
            handle.write(
                json.dumps(record, ensure_ascii=True, separators=(",", ":")) + "\n"
            )
    return target


def build_large(
    out_dir: str | Path,
    passages: int = LARGE_PASSAGES,
    seed: int = SEED,
    overwrite: bool = False,
) -> Path:
    """Writes the latency dataset: ``passages`` random normalized vectors.

    The dataset has one passage per document and follows the same schema as
    ``synth-small``. It is meant for latency only: no query set, no expected
    results.
    """
    if passages <= 0:
        raise ValueError("passages must be strictly positive")
    directory = Path(out_dir)
    _prepare_output(directory, overwrite, LARGE_FILES, LARGE_BATCHES)
    _write_index_config(directory, "synth-large")

    writer = BatchWriter(
        index_config("synth-large"),
        directory,
        "large-0001",
        "upsert",
        MODEL,
        LARGE_CHUNKER,
        TEMPLATE,
        created_at=batch_created_at(seed),
        host=HOST,
    )
    rng = _rng(seed, 5)
    for start in range(0, passages, LARGE_BLOCK_ROWS):
        stop = min(passages, start + LARGE_BLOCK_ROWS)
        block = rng.standard_normal((stop - start, DIMS))
        block /= np.linalg.norm(block, axis=1, keepdims=True)
        for offset, vector in enumerate(block):
            index = start + offset
            writer.add_document(
                f"large-{index:07d}",
                f"large-{index:07d}-v1",
                {"categorie": f"cat-{index % CLUSTERS:02d}"},
                [{"text": f"large passage {index}", "vector": vector}],
            )
    return writer.close()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--out",
        type=Path,
        default=None,
        help="output directory (default: data/synth-small, or data/synth-large)",
    )
    parser.add_argument("--seed", type=int, default=SEED, help="random seed")
    parser.add_argument(
        "--overwrite",
        action="store_true",
        help="replace the files this generator writes in a non-empty directory",
    )
    parser.add_argument(
        "--large",
        action="store_true",
        help="write the latency dataset instead of synth-small",
    )
    parser.add_argument(
        "--large-passages",
        type=int,
        default=LARGE_PASSAGES,
        help="number of passages of the latency dataset",
    )
    arguments = parser.parse_args(argv)

    if arguments.large:
        out_dir = arguments.out or Path("data") / "synth-large"
        directory = build_large(
            out_dir, arguments.large_passages, arguments.seed, arguments.overwrite
        )
        print(f"{directory}: {arguments.large_passages} passages, seed {arguments.seed}")
        return 0

    out_dir = arguments.out or Path("data") / "synth-small"
    dataset = build_small(out_dir, arguments.seed, arguments.overwrite)
    print(
        f"{dataset.out_dir}: {dataset.documents} documents, "
        f"{dataset.passages} live passages after l1+l2+l3, seed {arguments.seed}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
