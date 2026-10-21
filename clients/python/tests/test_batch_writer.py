# SPDX-License-Identifier: Apache-2.0

"""Tests of ``crible.BatchWriter`` against ``docs/format-lot-v1.md``.

Every local control of the writer has a failure test. The acceptance criteria
of the task are covered by the pyarrow re-read (column types) and by the numpy
round-trip of a fp16 vector.
"""

from __future__ import annotations

import datetime
import importlib.metadata
import json
import re
import socket

import blake3
import numpy as np
import pyarrow as pa
import pyarrow.parquet as pq
import pytest

import crible
import crible.batch
from crible import BatchError, BatchWriter
from crible.batch import _check_file_list

DIMS = 8
CHECKSUM = re.compile(r"^blake3:[0-9a-f]{64}$")


def index_config(precision: str = "fp16") -> dict:
    return {
        "name": "decisions",
        "model": "test-model",
        "reference_precision": precision,
        "schema": {
            "document_fields": {
                "juridiction": {"type": "keyword"},
                "formation": {"type": "keyword", "filterable": False},
                "date_decision": {"type": "date"},
                "nb": {"type": "integer"},
                "score": {"type": "float"},
                "vigueur": {"type": "bool"},
            },
            "passage_fields": {"partie": {"type": "keyword"}},
        },
        "cluster_by": ["juridiction"],
    }


def model(kind: str = "onnx", dims: int = DIMS, normalize: bool = True) -> dict:
    resolved = {"name": "test-model", "kind": kind, "dims": dims, "normalize": normalize}
    if kind == "onnx":
        resolved.update(
            {
                "hf_id": "lightonai/mDenseOn",
                "revision": "0123456789abcdef",
                "pooling": "cls",
                "max_tokens": 512,
                "prompts": {"query": "query: ", "document": "document: "},
                "files": {
                    "fp32": {"path": "model.onnx", "blake3": "0" * 64},
                    "tokenizer": {"path": "tokenizer.json", "blake3": "1" * 64},
                },
            }
        )
    return resolved


CHUNKER = {"name": "decouper_juri", "version": "3", "params": {"max_tokens": 512}}
TEMPLATE = {"name": "juri_titre_corps", "version": "1"}


def unit_vector(seed: int = 0, dims: int = DIMS) -> np.ndarray:
    generator = np.random.default_rng(seed)
    vector = generator.normal(size=dims)
    return vector / np.linalg.norm(vector)


def passage(text: str, seed: int = 0, **fields: object) -> dict:
    return {"text": text, "vector": unit_vector(seed), **fields}


def writer(tmp_path, **overrides) -> BatchWriter:
    settings = {
        "mode": "upsert",
        "model": model(),
        "chunker": CHUNKER,
        "template": TEMPLATE,
        "batch_id": "batch-1",
    }
    settings.update(overrides)
    return BatchWriter(
        settings.pop("index_config", index_config()),
        tmp_path,
        settings.pop("batch_id"),
        settings.pop("mode"),
        settings.pop("model"),
        settings.pop("chunker"),
        settings.pop("template"),
    )


def read_manifest(directory) -> dict:
    return json.loads((directory / "manifest.json").read_text(encoding="utf-8"))


def test_batch_re_reads_with_pyarrow_column_types(tmp_path):
    batch = writer(tmp_path)
    batch.add_document(
        "doc-1",
        "hash-1",
        {
            "juridiction": "cc",
            "date_decision": "2020-01-02",
            "nb": 42,
            "score": 0.5,
            "vigueur": True,
        },
        [passage("a", 0, partie="motifs"), passage("b", 1)],
    )
    batch.add_document(
        "doc-2", "hash-2", {"formation": "civ", "juridiction": None}, [passage("c", 2)]
    )
    directory = batch.close()

    manifest = read_manifest(directory)
    assert manifest["format"] == "crible-batch"
    assert manifest["format_version"] == 1
    assert manifest["batch_id"] == "batch-1"
    assert manifest["index"] == "decisions"
    assert manifest["mode"] == "upsert"
    assert manifest["counts"] == {"documents": 2, "passages": 3, "deletions": 0}
    assert set(manifest["files"]) == {"documents.parquet", "passages-0001.parquet"}
    for name, checksum in manifest["files"].items():
        assert CHECKSUM.match(checksum)
        assert checksum == "blake3:" + blake3.blake3((directory / name).read_bytes()).hexdigest()
    assert not (directory / "deletions.parquet").exists()

    documents = pq.read_table(directory / "documents.parquet")
    fields = documents.schema
    assert fields.field("doc_id").type == pa.string()
    assert fields.field("doc_hash").type == pa.string()
    assert not fields.field("doc_id").nullable
    assert not fields.field("doc_hash").nullable
    assert fields.field("juridiction").type == pa.string()
    assert fields.field("date_decision").type == pa.date32()
    assert fields.field("nb").type == pa.int64()
    assert fields.field("score").type == pa.float64()
    assert fields.field("vigueur").type == pa.bool_()
    for name in ("juridiction", "formation", "date_decision", "nb", "score", "vigueur"):
        assert fields.field(name).nullable
    rows = documents.to_pylist()
    assert [row["doc_id"] for row in rows] == ["doc-1", "doc-2"]
    assert rows[0]["date_decision"] == datetime.date(2020, 1, 2)
    assert rows[1]["date_decision"] is None
    assert rows[1]["nb"] is None

    passages = pq.read_table(directory / "passages-0001.parquet")
    passage_fields = passages.schema
    assert passage_fields.field("doc_id").type == pa.string()
    assert passage_fields.field("ord").type == pa.uint32()
    assert not passage_fields.field("ord").nullable
    assert passage_fields.field("text").type == pa.string()
    assert passage_fields.field("partie").type == pa.string()
    assert passage_fields.field("partie").nullable
    assert passage_fields.field("vector").type == pa.binary(DIMS * 2)
    assert pa.types.is_fixed_size_binary(passage_fields.field("vector").type)
    assert not passage_fields.field("vector").nullable
    passage_rows = passages.to_pylist()
    assert [row["ord"] for row in passage_rows] == [0, 1, 0]
    assert [row["doc_id"] for row in passage_rows] == ["doc-1", "doc-1", "doc-2"]


def test_vector_is_fp16_little_endian_and_round_trips_through_numpy(tmp_path):
    vector = unit_vector(7)
    batch = writer(tmp_path)
    batch.add_document("doc", "hash", {}, [{"text": "t", "vector": vector}])
    directory = batch.close()

    table = pq.read_table(directory / "passages-0001.parquet")
    raw = table.column("vector")[0].as_py()
    assert isinstance(raw, bytes)
    assert len(raw) == DIMS * 2
    decoded = np.frombuffer(raw, dtype="<f2")
    expected = vector.astype(np.float16)
    assert np.array_equal(decoded, expected)
    assert raw == expected.astype("<f2").tobytes()


def test_vector_dtype_follows_reference_precision_fp32(tmp_path):
    batch = writer(tmp_path, index_config=index_config("fp32"))
    batch.add_document("doc", "hash", {}, [{"text": "t", "vector": unit_vector(1)}])
    directory = batch.close()

    manifest = read_manifest(directory)
    assert manifest["model"]["vector_dtype"] == "fp32"
    table = pq.read_table(directory / "passages-0001.parquet")
    raw = table.column("vector")[0].as_py()
    assert len(raw) == DIMS * 4
    assert np.array_equal(np.frombuffer(raw, dtype="<f4"), unit_vector(1).astype(np.float32))


def test_unknown_document_field_is_refused(tmp_path):
    batch = writer(tmp_path)
    with pytest.raises(BatchError, match="unknown key 'inconnu'"):
        batch.add_document("doc", "hash", {"inconnu": "x"}, [passage("t")])


def test_unknown_passage_field_is_refused(tmp_path):
    batch = writer(tmp_path)
    with pytest.raises(BatchError, match="unknown key 'inconnu'"):
        batch.add_document("doc", "hash", {}, [passage("t", inconnu=1)])


def test_wrong_field_value_type_is_refused(tmp_path):
    batch = writer(tmp_path)
    with pytest.raises(BatchError, match="must be an integer"):
        batch.add_document("doc", "hash", {"nb": "quarante-deux"}, [passage("t")])


def test_schema_field_colliding_with_a_reserved_column_is_refused(tmp_path):
    config = index_config()
    config["schema"]["document_fields"]["doc_id"] = {"type": "keyword"}
    with pytest.raises(BatchError, match="reserved column 'doc_id'"):
        writer(tmp_path, index_config=config)


def test_ord_is_assigned_by_the_writer_and_refused_in_a_passage(tmp_path):
    batch = writer(tmp_path)
    with pytest.raises(BatchError, match="unknown key 'ord'"):
        batch.add_document("doc", "hash", {}, [{"text": "t", "vector": unit_vector(), "ord": 5}])
    with pytest.raises(BatchError, match="unknown key 'doc_id'"):
        batch.add_document(
            "doc", "hash", {}, [{"text": "t", "vector": unit_vector(), "doc_id": "other"}]
        )


@pytest.mark.parametrize("bad", [float("nan"), float("inf"), float("-inf")])
def test_non_finite_vector_component_is_refused(tmp_path, bad):
    vector = unit_vector(0).copy()
    vector[3] = bad
    batch = writer(tmp_path)
    with pytest.raises(BatchError, match="not finite"):
        batch.add_document("doc", "hash", {}, [{"text": "t", "vector": vector}])


def test_vector_dimension_must_match_the_model(tmp_path):
    batch = writer(tmp_path)
    with pytest.raises(BatchError, match="dimensions"):
        batch.add_document("doc", "hash", {}, [{"text": "t", "vector": unit_vector(0, 4)}])


def test_norm_is_checked_when_the_index_normalizes(tmp_path):
    batch = writer(tmp_path)
    with pytest.raises(BatchError, match="L2 norm"):
        batch.add_document("doc", "hash", {}, [{"text": "t", "vector": np.full(DIMS, 0.5)}])

    loose = writer(tmp_path / "loose", model=model(normalize=False))
    loose.add_document("doc", "hash", {}, [{"text": "t", "vector": np.full(DIMS, 0.5)}])
    loose.close()


def test_document_without_passage_is_refused(tmp_path):
    batch = writer(tmp_path)
    with pytest.raises(BatchError, match="has no passage"):
        batch.add_document("doc", "hash", {}, [])


def test_document_added_then_deleted_is_refused(tmp_path):
    batch = writer(tmp_path)
    batch.add_document("doc", "hash", {}, [passage("t")])
    with pytest.raises(BatchError, match="both added and deleted"):
        batch.delete("doc")

    other = writer(tmp_path / "other", batch_id="batch-2")
    other.delete("doc")
    with pytest.raises(BatchError, match="both added and deleted"):
        other.add_document("doc", "hash", {}, [passage("t")])


def test_replace_all_forbids_deletions(tmp_path):
    batch = writer(tmp_path, mode="replace_all")
    with pytest.raises(BatchError, match="replace_all"):
        batch.delete("doc")


@pytest.mark.parametrize("bad", ["", ".", "..", "a/b", "a\\b", "a\nb"])
def test_invalid_batch_id_is_refused(tmp_path, bad):
    with pytest.raises(BatchError, match="batch_id"):
        writer(tmp_path, batch_id=bad)


def test_deletions_file_is_written_and_checksummed(tmp_path):
    batch = writer(tmp_path)
    batch.add_document("keep", "hash", {}, [passage("t")])
    batch.delete("gone-2")
    batch.delete("gone-1")
    directory = batch.close()

    manifest = read_manifest(directory)
    assert manifest["counts"] == {"documents": 1, "passages": 1, "deletions": 2}
    assert "deletions.parquet" in manifest["files"]
    checksum = manifest["files"]["deletions.parquet"]
    assert checksum == "blake3:" + blake3.blake3(
        (directory / "deletions.parquet").read_bytes()
    ).hexdigest()
    table = pq.read_table(directory / "deletions.parquet")
    assert table.schema.field("doc_id").type == pa.string()
    assert sorted(table.column("doc_id").to_pylist()) == ["gone-1", "gone-2"]


def test_deletions_only_batch_is_valid(tmp_path):
    batch = writer(tmp_path, batch_id="deletions-only")
    batch.delete("gone")
    directory = batch.close()

    manifest = read_manifest(directory)
    assert manifest["counts"] == {"documents": 0, "passages": 0, "deletions": 1}
    assert set(manifest["files"]) == {
        "documents.parquet",
        "passages-0001.parquet",
        "deletions.parquet",
    }
    assert pq.read_table(directory / "documents.parquet").num_rows == 0
    assert pq.read_table(directory / "passages-0001.parquet").num_rows == 0


def test_empty_batch_still_writes_a_documents_and_a_passages_file(tmp_path):
    directory = writer(tmp_path, batch_id="empty").close()

    manifest = read_manifest(directory)
    assert manifest["counts"] == {"documents": 0, "passages": 0, "deletions": 0}
    assert set(manifest["files"]) == {"documents.parquet", "passages-0001.parquet"}
    assert pq.read_table(directory / "documents.parquet").num_rows == 0
    assert pq.read_table(directory / "passages-0001.parquet").num_rows == 0


def test_passages_are_split_into_capped_parts(tmp_path, monkeypatch):
    monkeypatch.setattr(crible.batch, "MAX_PASSAGES_PER_PART", 3)
    batch = writer(tmp_path, batch_id="parts")
    batch.add_document("d1", "h1", {}, [passage(f"p{index}", index) for index in range(4)])
    batch.add_document(
        "d2", "h2", {}, [passage("x", 10), passage("y", 11), passage("z", 12)]
    )
    directory = batch.close()

    manifest = read_manifest(directory)
    assert manifest["counts"]["passages"] == 7
    parts = sorted(name for name in manifest["files"] if name.startswith("passages"))
    assert parts == ["passages-0001.parquet", "passages-0002.parquet", "passages-0003.parquet"]
    tables = [pq.read_table(directory / name) for name in parts]
    assert [table.num_rows for table in tables] == [3, 3, 1]
    rows = [row for table in tables for row in table.to_pylist()]
    assert [row["ord"] for row in rows if row["doc_id"] == "d1"] == [0, 1, 2, 3]
    assert [row["ord"] for row in rows if row["doc_id"] == "d2"] == [0, 1, 2]


def test_manifest_members_match_the_rust_manifest_struct(tmp_path):
    """Checks the member sets that ``BatchManifest`` accepts (unknown keys are denied).

    The real Python-to-Rust round-trip goes through ``check_manifest`` and
    belongs to the LIRE-LOT-PARQUET task.
    """
    batch = writer(tmp_path)
    batch.add_document("doc", "hash", {}, [passage("t")])
    manifest = read_manifest(batch.close())

    assert set(manifest) == {
        "format",
        "format_version",
        "batch_id",
        "index",
        "mode",
        "created_at",
        "producer",
        "model",
        "chunker",
        "template",
        "counts",
        "files",
    }
    assert re.match(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$", manifest["created_at"])
    assert manifest["producer"] == {
        "name": "crible",
        "version": manifest["producer"]["version"],
        "host": socket.gethostname(),
    }
    assert manifest["producer"]["version"]
    assert set(manifest["model"]) == {
        "hf_id",
        "revision",
        "dims",
        "pooling",
        "normalize",
        "prompt_document",
        "vector_dtype",
    }
    assert manifest["model"]["prompt_document"] == "document: "
    assert manifest["model"]["dims"] == DIMS
    assert manifest["model"]["normalize"] is True
    assert manifest["model"]["vector_dtype"] == "fp16"
    assert set(manifest["chunker"]) == {"name", "version", "params"}
    assert set(manifest["template"]) == {"name", "version"}
    assert set(manifest["counts"]) == {"documents", "passages", "deletions"}
    for checksum in manifest["files"].values():
        assert CHECKSUM.match(checksum)


def test_external_model_manifest_leaves_encoder_fields_out(tmp_path):
    batch = writer(tmp_path, model=model("external"))
    batch.add_document("doc", "hash", {}, [passage("t")])
    manifest = read_manifest(batch.close())

    assert manifest["model"] == {"dims": DIMS, "normalize": True, "vector_dtype": "fp16"}


VALID_FILES = {
    "documents.parquet": "blake3:" + "0" * 64,
    "passages-0001.parquet": "blake3:" + "1" * 64,
}


@pytest.mark.parametrize(
    ("files", "counts", "mode", "message"),
    [
        (
            {"passages-0001.parquet": VALID_FILES["passages-0001.parquet"]},
            {"documents": 0, "passages": 0, "deletions": 0},
            "upsert",
            "at least one documents file",
        ),
        (
            {"documents.parquet": VALID_FILES["documents.parquet"]},
            {"documents": 1, "passages": 1, "deletions": 0},
            "upsert",
            "at least one passages file",
        ),
        (
            {**VALID_FILES, "deletions.parquet": "blake3:" + "2" * 64},
            {"documents": 1, "passages": 1, "deletions": 1},
            "replace_all",
            "replace_all",
        ),
        (
            {**VALID_FILES, "deletions.parquet": "blake3:" + "2" * 64},
            {"documents": 1, "passages": 1, "deletions": 0},
            "upsert",
            "counts.deletions is zero",
        ),
        (VALID_FILES, {"documents": 1, "passages": 1, "deletions": 1}, "upsert", "is missing"),
        (
            VALID_FILES,
            {"documents": 0, "passages": 1, "deletions": 0},
            "upsert",
            "cannot have passages",
        ),
        (
            VALID_FILES,
            {"documents": 2, "passages": 1, "deletions": 0},
            "upsert",
            "at least one passage",
        ),
        (
            {
                "documentsx.parquet": "blake3:" + "0" * 64,
                "passages-0001.parquet": "blake3:" + "1" * 64,
            },
            {"documents": 1, "passages": 1, "deletions": 0},
            "upsert",
            "at least one documents file",
        ),
        (
            {"documents.parquet": "blake3:" + "0" * 64, "passages-001.parquet": "blake3:" + "1" * 64},
            {"documents": 1, "passages": 1, "deletions": 0},
            "upsert",
            "at least one passages file",
        ),
    ],
)
def test_file_list_check_refuses_invalid_combinations(files, counts, mode, message):
    with pytest.raises(BatchError, match=message):
        _check_file_list(files, counts, mode)


def test_file_list_check_accepts_a_valid_list():
    _check_file_list(
        {**VALID_FILES, "deletions.parquet": "blake3:" + "2" * 64},
        {"documents": 1, "passages": 1, "deletions": 1},
        "upsert",
    )
    _check_file_list(VALID_FILES, {"documents": 1, "passages": 1, "deletions": 0}, "upsert")


def test_non_serializable_chunker_params_raise_a_batch_error(tmp_path):
    batch = writer(
        tmp_path,
        chunker={"name": "decouper", "version": "1", "params": {"when": datetime.datetime.now()}},
    )
    batch.add_document("doc", "hash", {}, [passage("t")])
    with pytest.raises(BatchError, match="not JSON-serializable"):
        batch.close()
    assert not (tmp_path / "batch-1" / "manifest.json").exists()


def test_package_metadata_version_matches_the_module_version():
    assert importlib.metadata.version("crible") == crible.__version__


def test_a_passage_part_is_fed_by_blocks(tmp_path, monkeypatch):
    monkeypatch.setattr(crible.batch, "MAX_PASSAGES_PER_PART", 5)
    monkeypatch.setattr(crible.batch, "PASSAGE_WRITE_ROWS", 2)
    batch = writer(tmp_path, batch_id="blocks")
    for index in range(5):
        batch.add_document(f"d{index}", f"h{index}", {}, [passage(f"p{index}", index)])
    directory = batch.close()

    parquet = pq.ParquetFile(directory / "passages-0001.parquet")
    assert parquet.metadata.num_rows == 5
    assert parquet.metadata.num_row_groups == 3
