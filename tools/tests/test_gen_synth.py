# SPDX-License-Identifier: Apache-2.0

"""Acceptance tests of ``tools/gen_synth.py``.

Two criteria of the task are covered here: two generations with the same seed
write byte-identical files, and the dataset holds every field type, nulls,
document and passage fields, and clustered vectors.
"""

from __future__ import annotations

import json
import re
from pathlib import Path

import numpy as np
import pyarrow as pa
import pyarrow.parquet as pq
import pytest

import crible.batch
import gen_synth
from gen_synth import build_large, build_small, dataset_checksum
from reference_topk import reference

DOCUMENT_FIELDS = {
    "annee": pa.int64(),
    "categorie": pa.string(),
    "date_publication": pa.date32(),
    "publie": pa.bool_(),
    "score": pa.float64(),
    "source": pa.string(),
}
PASSAGE_FIELDS = {
    "date_effet": pa.date32(),
    "partie": pa.string(),
    "poids": pa.float64(),
    "principal": pa.bool_(),
    "rang": pa.int64(),
}
NULLABLE_DOCUMENT_FIELDS = ("annee", "date_publication", "publie", "score", "source")
NULLABLE_PASSAGE_FIELDS = ("date_effet", "partie", "poids", "principal", "rang")


def manifest(batch: Path) -> dict:
    return json.loads((batch / "manifest.json").read_text(encoding="utf-8"))


def table_rows(path: Path) -> list[dict]:
    return pq.read_table(path).to_pylist()


def passage_rows(batch: Path) -> list[dict]:
    rows: list[dict] = []
    for path in sorted(batch.glob("passages-*.parquet")):
        rows.extend(table_rows(path))
    return rows


def generated_files(root: Path) -> dict[str, bytes]:
    return {
        path.relative_to(root).as_posix(): path.read_bytes()
        for path in sorted(root.rglob("*"))
        if path.is_file()
    }


def read_vectors(batch: Path) -> tuple[list[str], np.ndarray]:
    doc_ids: list[str] = []
    vectors: list[np.ndarray] = []
    for path in sorted(batch.glob("passages-*.parquet")):
        table = pq.read_table(path)
        doc_ids.extend(table.column("doc_id").to_pylist())
        vectors.extend(
            np.frombuffer(raw, dtype="<f2").astype(np.float32)
            for raw in table.column("vector").to_pylist()
        )
    return doc_ids, np.stack(vectors)


def test_two_generations_with_the_same_seed_are_bit_identical(
    synth_dataset, tmp_path
):
    second = build_small(tmp_path / "second")
    reference(second.out_dir)
    reference(synth_dataset.out_dir)

    assert generated_files(second.out_dir) == generated_files(synth_dataset.out_dir)


def test_a_non_empty_output_directory_is_refused_without_overwrite(tmp_path):
    out_dir = tmp_path / "synth-small"
    out_dir.mkdir()
    (out_dir / "notes.txt").write_text("keep me", encoding="utf-8")

    with pytest.raises(FileExistsError, match="--overwrite"):
        build_small(out_dir)

    assert (out_dir / "notes.txt").read_text(encoding="utf-8") == "keep me"


def test_overwrite_only_replaces_what_the_generator_writes(tmp_path):
    out_dir = tmp_path / "synth-small"
    build_small(out_dir)
    (out_dir / "notes.txt").write_text("keep me", encoding="utf-8")
    (out_dir / "queries.jsonl").write_text("stale\n", encoding="utf-8")

    build_small(out_dir, overwrite=True)

    assert (out_dir / "notes.txt").read_text(encoding="utf-8") == "keep me"
    assert (out_dir / "queries.jsonl").read_text(encoding="utf-8") != "stale\n"


def test_a_successful_generation_leaves_no_hidden_entry(tmp_path):
    out_dir = tmp_path / "synth-small"
    build_small(out_dir, overwrite=True)
    build_small(out_dir, overwrite=True)

    assert [path.name for path in out_dir.iterdir() if path.name.startswith(".")] == []


def test_a_successful_large_generation_leaves_no_hidden_entry(tmp_path):
    batch = build_large(tmp_path / "synth-large", passages=2)

    assert sorted(path.name for path in batch.parent.iterdir()) == [
        "index.json",
        "large-0001",
    ]


def test_batches_have_the_documented_counts(synth_dataset):
    l1, l2, l3 = (synth_dataset.out_dir / name for name in ("l1", "l2", "l3"))
    first, second, third = (manifest(batch) for batch in (l1, l2, l3))

    assert first["counts"] == {"documents": 2_000, "passages": 10_000, "deletions": 0}
    assert second["counts"] == {"documents": 400, "passages": 2_000, "deletions": 0}
    assert third["counts"] == {"documents": 50, "passages": 250, "deletions": 150}
    for document in (first, second, third):
        assert document["format"] == "crible-batch"
        assert document["format_version"] == 1
        assert document["index"] == "synth-small"
        assert document["mode"] == "upsert"
    assert synth_dataset.documents == 1_950
    assert synth_dataset.passages == 9_750
    assert synth_dataset.deletions == 150


def test_dataset_has_every_field_type_at_both_levels(synth_dataset):
    l1 = synth_dataset.out_dir / "l1"
    documents = pq.read_table(l1 / "documents.parquet")
    passages = pq.read_table(l1 / "passages-0001.parquet")

    for name, arrow_type in DOCUMENT_FIELDS.items():
        assert documents.schema.field(name).type == arrow_type
    for name, arrow_type in PASSAGE_FIELDS.items():
        assert passages.schema.field(name).type == arrow_type

    arrow_types = {
        pa.string(): "keyword",
        pa.int64(): "integer",
        pa.float64(): "float",
        pa.date32(): "date",
        pa.bool_(): "bool",
    }
    document_kinds = {
        arrow_types[documents.schema.field(name).type] for name in DOCUMENT_FIELDS
    }
    passage_kinds = {
        arrow_types[passages.schema.field(name).type] for name in PASSAGE_FIELDS
    }
    assert document_kinds == {"keyword", "integer", "float", "date", "bool"}
    assert passage_kinds == {"keyword", "integer", "float", "date", "bool"}

    for name in NULLABLE_DOCUMENT_FIELDS:
        assert documents.column(name).null_count > 0, name
    for name in NULLABLE_PASSAGE_FIELDS:
        assert passages.column(name).null_count > 0, name


def test_vectors_are_clustered_by_the_document_category(synth_dataset):
    l1 = synth_dataset.out_dir / "l1"
    doc_ids, vectors = read_vectors(l1)
    categories = {
        row["doc_id"]: row["categorie"]
        for row in table_rows(l1 / "documents.parquet")
    }
    labels = np.array([categories[doc_id] for doc_id in doc_ids])

    centres = {}
    for category in sorted(set(labels)):
        centre = vectors[labels == category].astype(np.float64).mean(axis=0)
        centres[category] = centre / np.linalg.norm(centre)
    within = np.mean(
        [
            float(np.dot(vector.astype(np.float64), centres[label]))
            for vector, label in zip(vectors, labels)
        ]
    )
    names = sorted(centres)
    between = [
        abs(float(np.dot(centres[first], centres[second])))
        for position, first in enumerate(names)
        for second in names[position + 1 :]
    ]

    assert within > 0.8
    assert max(between) < 0.3
    assert not np.array_equal(vectors[0], vectors[1])


def test_third_batch_deletes_and_updates_metadata_only(synth_dataset):
    root = synth_dataset.out_dir
    l1, l2, l3 = (root / name for name in ("l1", "l2", "l3"))
    first_documents = {row["doc_id"]: row for row in table_rows(l1 / "documents.parquet")}
    second_documents = {
        row["doc_id"]: row for row in table_rows(l2 / "documents.parquet")
    }
    before = {**first_documents, **second_documents}
    first_passages = _passages_by_document(l1)
    second_passages = _passages_by_document(l2)
    previous_passages = {**first_passages, **second_passages}

    updates = table_rows(l3 / "documents.parquet")
    deletions = {row["doc_id"] for row in table_rows(l3 / "deletions.parquet")}
    update_ids = {row["doc_id"] for row in updates}

    assert len(updates) == 50
    assert len(deletions) == 150
    assert not update_ids & deletions
    third_passages = _passages_by_document(l3)
    for row in updates:
        doc_id = row["doc_id"]
        assert row["doc_hash"] == before[doc_id]["doc_hash"]
        assert _without_identity(row) != _without_identity(before[doc_id])
        assert third_passages[doc_id] == previous_passages[doc_id]
    assert all(doc_id in before for doc_id in deletions)

    live = set(first_documents) | set(second_documents)
    assert len(live - deletions) == 1_950


def test_queries_have_unit_vectors_and_varied_filters(synth_dataset):
    path = synth_dataset.out_dir / "queries.jsonl"
    queries = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]

    assert len(queries) == 200
    assert [query["query_id"] for query in queries] == list(range(200))
    norms = [np.linalg.norm(np.asarray(query["vector"], dtype=np.float64)) for query in queries]
    assert max(abs(norm - 1.0) for norm in norms) < 1e-5
    assert all(len(query["vector"]) == gen_synth.DIMS for query in queries)
    assert all(query["k"] == 10 for query in queries)
    assert any(query["filter"] is None for query in queries)
    assert len({tuple(query["vector"][:4]) for query in queries}) > 100

    operators = set()
    fields = set()
    for query in queries:
        _collect(query["filter"], operators, fields)
    assert {"eq", "ne", "in", "nin", "lt", "lte", "gt", "gte", "exists"} <= operators
    assert {"categorie", "partie", "annee", "score", "date_publication"} <= fields
    assert "$or" in operators and "$not" in operators and "$and" in operators

    filters = [query["filter"] for query in queries]
    assert {"source": {"ne": "journal"}} in filters
    assert {"$not": {"source": "journal"}} in filters
    assert {"source": {"exists": True}} in filters
    assert {"source": {"exists": False}} in filters


def test_unit_vectors_reproduce_the_generator_draws():
    vectors = gen_synth.unit_vectors(seed=7, stream=3, count=5)
    rng = gen_synth._rng(7, 3)
    expected = np.stack([gen_synth._unit_vector(rng) for _ in range(5)]).astype(np.float32)

    assert vectors.dtype == np.float32
    assert vectors.shape == (5, gen_synth.DIMS)
    assert np.array_equal(vectors, expected)
    norms = np.linalg.norm(vectors.astype(np.float64), axis=1)
    assert np.max(np.abs(norms - 1.0)) < 1e-5


def test_unit_vectors_refuse_a_non_positive_count():
    with pytest.raises(ValueError):
        gen_synth.unit_vectors(seed=0, stream=0, count=0)


def test_large_dataset_is_written_in_parts_with_normalized_vectors(
    tmp_path, monkeypatch
):
    monkeypatch.setattr(crible.batch, "MAX_PASSAGES_PER_PART", 5)
    first = build_large(tmp_path / "first", passages=12)
    second = build_large(tmp_path / "second", passages=12)

    assert dataset_checksum(first.parent) == dataset_checksum(second.parent)
    document = manifest(first)
    assert document["counts"] == {"documents": 12, "passages": 12, "deletions": 0}
    assert document["index"] == "synth-large"
    assert document["chunker"]["params"] == {"passages_per_document": 1}
    parts = sorted(name for name in document["files"] if name.startswith("passages-"))
    assert parts == ["passages-0001.parquet", "passages-0002.parquet", "passages-0003.parquet"]

    _, vectors = read_vectors(first)
    norms = np.linalg.norm(vectors.astype(np.float64), axis=1)
    assert np.all(np.isfinite(vectors))
    assert np.max(np.abs(norms - 1.0)) <= 1e-2
    assert not np.array_equal(vectors[0], vectors[1])


def test_generated_checksum_matches_the_one_recorded_in_the_readme(tmp_path):
    generated = build_small(tmp_path / "synth-small")
    reference(generated.out_dir)
    (generated.out_dir / ".DS_Store").write_bytes(b"junk")
    (generated.out_dir / "l1" / ".DS_Store").write_bytes(b"junk")
    readme = (Path(__file__).resolve().parents[1] / "README.md").read_text(
        encoding="utf-8"
    )
    recorded = re.search(r"synth-small\s*:\s*(blake3:[0-9a-f]{64})", readme)

    assert recorded is not None, "tools/README.md does not record a synth-small checksum"
    assert dataset_checksum(generated.out_dir) == recorded.group(1)


def test_large_overwrite_keeps_the_files_of_the_small_mode(tmp_path):
    out_dir = tmp_path / "synth-large"
    build_large(out_dir, passages=2)
    (out_dir / "queries.jsonl").write_text("small mode\n", encoding="utf-8")
    (out_dir / "expected.jsonl").write_text("small mode\n", encoding="utf-8")

    build_large(out_dir, passages=2, overwrite=True)

    assert (out_dir / "queries.jsonl").read_text(encoding="utf-8") == "small mode\n"
    assert (out_dir / "expected.jsonl").read_text(encoding="utf-8") == "small mode\n"


def _passages_by_document(batch: Path) -> dict[str, list[dict]]:
    grouped: dict[str, list[dict]] = {}
    for row in passage_rows(batch):
        grouped.setdefault(row["doc_id"], []).append(row)
    return grouped


def _without_identity(row: dict) -> dict:
    return {
        key: value
        for key, value in row.items()
        if key not in ("doc_id", "doc_hash")
    }


def _collect(node, operators: set, fields: set) -> None:
    if node is None:
        return
    for key, operand in node.items():
        if key in ("$and", "$or"):
            operators.add(key)
            for item in operand:
                _collect(item, operators, fields)
        elif key == "$not":
            operators.add("$not")
            _collect(operand, operators, fields)
        else:
            fields.add(key)
            if isinstance(operand, dict):
                operators.update(operand)
            else:
                operators.add("eq")
