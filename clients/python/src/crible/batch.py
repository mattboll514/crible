# SPDX-License-Identifier: Apache-2.0

"""Write a Crible batch: Parquet parts, blake3 checksums and ``manifest.json``.

The format is specified by ``docs/format-lot-v1.md``. :class:`BatchWriter`
applies the local checks that the engine repeats at attach time (controls 5 to
7 of ``docs/04-donnees-et-ingestion.md``): schema, contiguous ``ord``, finite
vectors, L2 norms and counts.

Configuration passed to the writer
----------------------------------

``index_config`` is the ``index.json`` of the target index, as a mapping. Only
``name``, ``model``, ``reference_precision`` and ``schema`` are read; the other
settings are accepted and ignored.

``model`` is the **resolved model configuration** (``ModelConfig``, the content
of ``models/<name>/model.json``, produced in P2), already in memory: the writer
never reads that file. It derives the ``model`` member of the manifest from it,
takes ``vector_dtype`` from ``index_config.reference_precision`` and leaves
``model.runtime`` out.

``chunker`` and ``template`` are the manifest blocks of the producer. ``mode``
is ``"upsert"`` or ``"replace_all"``.

The optional keyword ``created_at`` fixes the ``created_at`` member of the
manifest, a RFC 3339 string. Without it the writer uses the current UTC time
when ``close()`` runs. A producer that needs byte-identical files, such as the
synthetic data generator, passes a value derived only from its own parameters.

The optional keyword ``host`` fixes ``producer.host``, the identifier of the
machine or producer that produced the batch (ADR-0036). It defaults to
``socket.gethostname()``.

Files written
-------------

The batch is assembled in a hidden temporary directory next to its target, in
the same parent directory, and renamed to ``<out_dir>/<batch_id>/`` when
``close()`` succeeds. The rename is therefore atomic: a reader never sees a
partial batch, and an interrupted producer leaves no half batch under the final
name. See "Atomic write" below.

``documents.parquet`` always exists, even when the batch has no document.
``passages`` is written as parts of at most ``MAX_PASSAGES_PER_PART`` rows,
named ``passages-NNNN.parquet`` starting at ``0001``, and fed by blocks of
``PASSAGE_WRITE_ROWS`` rows so that a part does not have to be held in memory
as a whole; at least one part exists. ``deletions.parquet`` exists if and only
if the batch deletes a document, and is forbidden in ``replace_all`` mode.

Atomic write
------------

The writer never writes under the final name. It creates the hidden directory
``<out_dir>/.<batch_id>.tmp-<suffix>``, writes the whole batch there, syncs
every file and that directory, then renames the directory to
``<out_dir>/<batch_id>/`` and syncs ``<out_dir>`` so the rename itself is
durable. The target never exists half written, and a batch directory that a
reader can see is complete and valid.

A failure anywhere (``add_document``, ``delete`` or ``close``) removes that
temporary directory, and only it: the target directory and every other entry of
``out_dir`` are left untouched. The writer then refuses further use, and the
same ``batch_id`` can be written again by a new writer. An existing target is
still refused at construction time, and the rename fails with
:class:`BatchError` if the target appears while the batch is being written;
nothing is overwritten.

``fsync`` makes the rename durable against a crash of the process or of the
kernel, not strictly against a power loss: on macOS, only ``F_FULLFSYNC`` flushes
to the device, and the writer uses ``fsync``. On a platform that refuses
``fsync`` on a directory handle, the directory sync is skipped and only the
files are synced; the rename stays atomic. Any other sync failure is reported as
a :class:`BatchError`.

One residual race remains: a concurrent process that creates an *empty*
directory exactly under the final name between the existence check and the
rename would see it replaced, because POSIX ``rename`` has no portable
"do not replace" flag for directories. Any other content is never replaced.
"""

from __future__ import annotations

import datetime
import errno
import json
import os
import shutil
import socket
import tempfile
from collections.abc import Iterable, Mapping
from pathlib import Path
from typing import Any

import blake3
import pyarrow as pa
import pyarrow.parquet as pq

from ._version import __version__
from .errors import BatchError
from .schema import IndexSchema
from .vectors import encode_vector

BATCH_FORMAT = "crible-batch"
BATCH_FORMAT_VERSION = 1

#: Maximum number of passages in one ``passages`` file.
MAX_PASSAGES_PER_PART = 1_000_000

#: Number of passage rows handed to the Parquet writer at once. A part is
#: filled by successive blocks, so the memory held for a part stays around this
#: size instead of growing to the whole part.
PASSAGE_WRITE_ROWS = 65_536

#: Number of document rows buffered before they are appended to the Parquet file.
DOCUMENT_FLUSH_ROWS = 8_192

MODES = ("upsert", "replace_all")

PRODUCER_NAME = "crible"

_MODEL_KEYS = (
    "name",
    "kind",
    "hf_id",
    "revision",
    "dims",
    "pooling",
    "normalize",
    "max_tokens",
    "prompts",
    "files",
)
_CHUNKER_KEYS = ("name", "version", "params")
_TEMPLATE_KEYS = ("name", "version")
_PROMPTS_KEYS = ("query", "document")


class BatchWriter:
    """Builds one batch directory from documents, passages and deletions."""

    def __init__(
        self,
        index_config: Mapping[str, Any],
        out_dir: str | Path,
        batch_id: str,
        mode: str,
        model: Mapping[str, Any],
        chunker: Mapping[str, Any],
        template: Mapping[str, Any],
        *,
        created_at: str | None = None,
        host: str | None = None,
    ) -> None:
        self._schema = IndexSchema.from_index_config(index_config)
        self._batch_id = _check_batch_id(batch_id)
        self._mode = _check_mode(mode)
        self._manifest_model = _manifest_model(model, self._schema.vector_dtype)
        self._chunker = _chunker(chunker)
        self._template = _template(template)
        self._created_at = _check_created_at(created_at)
        self._host = _check_host(host)
        self._dims = self._manifest_model["dims"]
        self._normalize = self._manifest_model["normalize"]
        self._documents_schema = self._schema.document_schema()
        self._passages_schema = self._schema.passage_schema(
            self._dims, self._schema.vector_dtype
        )
        self._directory = Path(out_dir) / self._batch_id
        if self._directory.exists():
            raise BatchError(
                f"batch directory '{self._directory}' already exists; "
                "choose another batch_id or remove it"
            )
        self._work_directory = _create_work_directory(self._directory)

        self._document_writer = _TableWriter(self._work_directory / "documents.parquet", self._documents_schema)
        self._document_columns: dict[str, list[Any]] = {
            name: [] for name in self._documents_schema.names
        }
        self._document_rows = 0
        self._document_count = 0

        self._part_writer: _TableWriter | None = None
        self._part_name = ""
        self._part_columns: dict[str, list[Any]] = {
            name: [] for name in self._passages_schema.names
        }
        self._part_rows = 0
        self._passage_rows = 0
        self._passage_count = 0
        self._passage_files: list[str] = []
        self._part_number = 1

        self._deleted: set[str] = set()
        self._document_ids: set[str] = set()
        self._state = "open"

    def add_document(
        self,
        doc_id: str,
        doc_hash: str,
        fields: Mapping[str, Any],
        passages: Iterable[Mapping[str, Any]],
    ) -> None:
        """Adds one document with all its passages.

        ``fields`` holds the document fields declared by the schema; missing
        fields are null. Each passage is a mapping with ``text``, ``vector``
        and its declared passage fields. ``ord`` and ``doc_id`` are assigned by
        the writer and are refused in a passage.

        A document must have at least one passage, and it must not already be
        part of the batch (added or deleted): a document present with all its
        passages replaces the stored one.

        Any failure aborts the batch: its temporary directory is removed and
        the writer refuses further use, so the same ``batch_id`` can be written
        again by a new writer.
        """
        self._check_open()
        try:
            self._add_document(doc_id, doc_hash, fields, passages)
        except BaseException:
            self._abort()
            raise

    def delete(self, doc_id: str) -> None:
        """Marks one document for deletion.

        A document already added to the batch, or already deleted, is refused.
        ``replace_all`` forbids deletions: the batch is the whole content.

        Any failure aborts the batch, as :meth:`add_document` does.
        """
        self._check_open()
        try:
            self._delete(doc_id)
        except BaseException:
            self._abort()
            raise

    def _add_document(
        self,
        doc_id: str,
        doc_hash: str,
        fields: Mapping[str, Any],
        passages: Iterable[Mapping[str, Any]],
    ) -> None:
        doc_id = _check_doc_id(doc_id)
        if doc_id in self._document_ids:
            raise BatchError(f"document '{doc_id}' is already part of this batch")
        if doc_id in self._deleted:
            raise BatchError(
                f"document '{doc_id}' is both added and deleted in this batch; "
                "a batch is rejected when a doc_id appears in documents and deletions"
            )
        if not isinstance(doc_hash, str):
            raise BatchError(f"doc_hash of document '{doc_id}' must be a string")
        if isinstance(passages, (str, bytes)) or not isinstance(passages, Iterable):
            raise BatchError(
                f"passages of document '{doc_id}' must be an iterable of passages"
            )
        passages = list(passages)
        if not passages:
            raise BatchError(
                f"document '{doc_id}' has no passage; every document in a batch "
                "arrives with all its passages and has at least one"
            )
        values = self._schema.document_values(fields)
        passage_columns = self._passage_columns_for(doc_id, passages)

        self._document_columns["doc_id"].append(doc_id)
        self._document_columns["doc_hash"].append(doc_hash)
        for name, value in values.items():
            self._document_columns[name].append(value)
        self._document_ids.add(doc_id)
        self._document_count += 1
        self._document_rows += 1
        if self._document_rows >= DOCUMENT_FLUSH_ROWS:
            self._flush_documents()

        self._buffer_passages(passage_columns, len(passages))

    def _delete(self, doc_id: str) -> None:
        doc_id = _check_doc_id(doc_id)
        if self._mode == "replace_all":
            raise BatchError(
                f"cannot delete document '{doc_id}': mode 'replace_all' forbids a "
                "deletions file, the batch is the complete content of the index"
            )
        if doc_id in self._document_ids:
            raise BatchError(
                f"document '{doc_id}' is both added and deleted in this batch; "
                "a batch is rejected when a doc_id appears in documents and deletions"
            )
        if doc_id in self._deleted:
            raise BatchError(f"document '{doc_id}' is already deleted in this batch")
        self._deleted.add(doc_id)

    def close(self) -> Path:
        """Writes the remaining Parquet files, the checksums and the manifest.

        Returns the batch directory. The writer cannot be used afterwards.

        The whole batch is written in a hidden temporary directory, synced, then
        renamed to its final name. A failure removes that temporary directory
        and leaves neither a target directory nor a temporary one.
        """
        self._check_open()
        try:
            self._finish()
        except BaseException:
            self._abort()
            raise
        return self._directory

    def _finish(self) -> None:
        self._flush_documents()
        self._document_writer.close()
        self._close_part()
        if not self._passage_files:
            self._write_empty_part()
        if self._deleted:
            self._write_deletions()

        files = {
            name: _blake3_file(self._work_directory / name)
            for name in self._file_names()
        }
        counts = {
            "documents": self._document_count,
            "passages": self._passage_count,
            "deletions": len(self._deleted),
        }
        _check_file_list(files, counts, self._mode)
        manifest = {
            "format": BATCH_FORMAT,
            "format_version": BATCH_FORMAT_VERSION,
            "batch_id": self._batch_id,
            "index": self._schema.name,
            "mode": self._mode,
            "created_at": (
                self._created_at if self._created_at is not None else _utc_now()
            ),
            "producer": {
                "name": PRODUCER_NAME,
                "version": __version__,
                "host": self._host,
            },
            "model": self._manifest_model,
            "chunker": self._chunker,
            "template": self._template,
            "counts": counts,
            "files": files,
        }
        try:
            text = json.dumps(manifest, ensure_ascii=False, indent=2)
        except (TypeError, ValueError) as error:
            raise BatchError(f"the manifest is not JSON-serializable: {error}") from error
        _write_manifest(self._work_directory, text)
        for name in (*self._file_names(), "manifest.json"):
            _fsync_file(self._work_directory / name)
        _fsync_directory(self._work_directory)
        _rename_directory(self._work_directory, self._directory)
        _fsync_directory(self._directory.parent)
        self._state = "closed"

    def _check_open(self) -> None:
        if self._state == "closed":
            raise BatchError(f"batch '{self._batch_id}' is already closed")
        if self._state == "aborted":
            raise BatchError(
                f"batch '{self._batch_id}' was aborted after a failure; its "
                "temporary directory is removed, start a new writer"
            )

    def _abort(self) -> None:
        """Removes the temporary directory of this writer and refuses further use.

        Only the directory this writer created is removed: the target directory
        and every other entry of ``out_dir`` are never touched.
        """
        self._state = "aborted"
        for table in (self._document_writer, self._part_writer):
            if table is not None:
                try:
                    table.close()
                except Exception:  # the batch is discarded, whatever the writer state
                    pass
        self._part_writer = None
        shutil.rmtree(self._work_directory, ignore_errors=True)

    def _passage_columns_for(
        self, doc_id: str, passages: list[Mapping[str, Any]]
    ) -> dict[str, list[Any]]:
        columns: dict[str, list[Any]] = {name: [] for name in self._passages_schema.names}
        for ord_, passage in enumerate(passages):
            path = f"passage {ord_} of document '{doc_id}'"
            if not isinstance(passage, Mapping):
                raise BatchError(f"{path} must be an object, got {type(passage).__name__}")
            for reserved in ("doc_id", "ord"):
                if reserved in passage:
                    raise BatchError(
                        f"unknown key '{reserved}' at {path}; the writer assigns "
                        f"'{reserved}' itself, so a passage declares only 'text', "
                        "'vector' and its passage fields"
                    )
            if "text" not in passage:
                raise BatchError(f"missing member 'text' at {path}")
            text = passage["text"]
            if not isinstance(text, str):
                raise BatchError(f"{path}.text must be a string, got {type(text).__name__}")
            if "vector" not in passage:
                raise BatchError(f"missing member 'vector' at {path}")
            values = self._schema.passage_values(passage, path)
            vector = encode_vector(
                passage["vector"],
                dims=self._dims,
                vector_dtype=self._schema.vector_dtype,
                normalize=self._normalize,
                path=f"{path}.vector",
            )
            columns["doc_id"].append(doc_id)
            columns["ord"].append(ord_)
            columns["text"].append(text)
            for name, value in values.items():
                columns[name].append(value)
            columns["vector"].append(vector)
        return columns

    def _buffer_passages(self, columns: dict[str, list[Any]], count: int) -> None:
        start = 0
        while start < count:
            space = MAX_PASSAGES_PER_PART - self._passage_rows
            if space == 0:
                self._close_part()
                continue
            stop = min(count, start + space)
            chunk = {name: values[start:stop] for name, values in columns.items()}
            self._append_to_part(chunk, stop - start)
            start = stop
            if self._passage_rows == MAX_PASSAGES_PER_PART:
                self._close_part()

    def _append_to_part(self, columns: dict[str, list[Any]], count: int) -> None:
        if self._part_writer is None:
            self._part_name = f"passages-{self._part_number:04d}.parquet"
            self._part_writer = _TableWriter(
                self._work_directory / self._part_name, self._passages_schema
            )
        for name, values in columns.items():
            self._part_columns[name].extend(values)
        self._part_rows += count
        self._passage_rows += count
        self._passage_count += count
        if self._part_rows >= PASSAGE_WRITE_ROWS:
            self._flush_part_rows()

    def _flush_documents(self) -> None:
        if self._document_rows == 0:
            return
        self._document_writer.append(self._document_columns, self._document_rows)
        self._document_columns = {name: [] for name in self._documents_schema.names}
        self._document_rows = 0

    def _flush_part_rows(self) -> None:
        if self._part_writer is None or self._part_rows == 0:
            return
        self._part_writer.append(self._part_columns, self._part_rows)
        self._part_columns = {name: [] for name in self._passages_schema.names}
        self._part_rows = 0

    def _close_part(self) -> None:
        self._flush_part_rows()
        if self._part_writer is not None:
            self._part_writer.close()
            self._part_writer = None
            self._passage_files.append(self._part_name)
            self._part_number += 1
        self._passage_rows = 0

    def _write_empty_part(self) -> None:
        name = "passages-0001.parquet"
        _TableWriter(self._work_directory / name, self._passages_schema).close()
        self._passage_files.append(name)

    def _write_deletions(self) -> None:
        table = pa.table(
            {"doc_id": sorted(self._deleted)}, schema=IndexSchema.deletion_schema()
        )
        pq.write_table(table, self._work_directory / "deletions.parquet")

    def _file_names(self) -> list[str]:
        names = ["documents.parquet"]
        names.extend(self._passage_files)
        if self._deleted:
            names.append("deletions.parquet")
        return names


class _TableWriter:
    """Appends row groups to one Parquet file, and writes it empty if unused."""

    def __init__(self, path: Path, schema: pa.Schema) -> None:
        self._path = path
        self._schema = schema
        self._writer: pq.ParquetWriter | None = None

    def append(self, columns: dict[str, list[Any]], rows: int) -> None:
        if rows == 0:
            return
        table = pa.table(columns, schema=self._schema)
        if self._writer is None:
            self._writer = pq.ParquetWriter(self._path, self._schema)
        self._writer.write_table(table)

    def close(self) -> None:
        if self._writer is not None:
            self._writer.close()
            self._writer = None
        elif not self._path.exists():
            pq.write_table(self._schema.empty_table(), self._path)


def _create_work_directory(target: Path) -> Path:
    """Creates the hidden temporary directory of a batch, next to its target.

    The directory lives in the parent of the target, so that the final rename
    stays inside one file system and is atomic. Its name starts with
    ``.<batch_id>.tmp-``, which makes it recognisable and keeps it out of a
    plain ``ls``.
    """
    parent = target.parent
    try:
        parent.mkdir(parents=True, exist_ok=True)
        return Path(tempfile.mkdtemp(dir=parent, prefix=f".{target.name}.tmp-"))
    except OSError as error:
        raise BatchError(
            f"cannot create a temporary directory for batch '{target.name}' in "
            f"'{parent}': {error}"
        ) from error


def _write_manifest(directory: Path, text: str) -> None:
    """Writes ``manifest.json`` through a temporary file and a replace."""
    target = directory / "manifest.json"
    temporary = directory / "manifest.json.tmp"
    temporary.write_text(text + "\n", encoding="utf-8")
    temporary.replace(target)


#: ``errno`` values that mean "this platform cannot fsync a directory".
_DIRECTORY_FSYNC_UNSUPPORTED = frozenset(
    {errno.EINVAL, errno.ENOTSUP, errno.EACCES, errno.EBADF, errno.EPERM, errno.ENOTDIR}
)


def _fsync_file(path: Path) -> None:
    """Flushes one written file to the storage device.

    An I/O failure is raised as a :class:`BatchError`: the batch cannot be
    declared durable.
    """
    try:
        handle = os.open(path, os.O_RDONLY)
        try:
            os.fsync(handle)
        finally:
            os.close(handle)
    except OSError as error:
        raise BatchError(f"cannot sync '{path}': {error}") from error


def _fsync_directory(path: Path) -> None:
    """Flushes the entries of one directory, so a rename survives a crash.

    Some platforms refuse ``fsync`` on a directory handle; those errno values
    are listed in ``_DIRECTORY_FSYNC_UNSUPPORTED`` and the call is skipped. The
    rename itself stays atomic; only its durability against a power loss is
    weaker. Any other failure, such as ``EIO`` or ``ENOSPC``, is a real
    durability failure and is raised as a :class:`BatchError`.
    """
    try:
        handle = os.open(path, os.O_RDONLY)
    except OSError as error:
        if error.errno in _DIRECTORY_FSYNC_UNSUPPORTED:
            return
        raise BatchError(
            f"cannot open directory '{path}' for sync: {error}"
        ) from error
    try:
        try:
            os.fsync(handle)
        except OSError as error:
            if error.errno not in _DIRECTORY_FSYNC_UNSUPPORTED:
                raise BatchError(
                    f"cannot sync directory '{path}': {error}"
                ) from error
    finally:
        os.close(handle)


def _rename_directory(source: Path, target: Path) -> None:
    """Renames the temporary directory to its final name, never replacing one.

    An existing target, even an empty directory, makes the rename fail with a
    :class:`BatchError`.
    """
    if os.path.lexists(target):
        raise BatchError(
            f"batch directory '{target}' appeared while the batch was written; "
            "refusing to replace it"
        )
    try:
        os.rename(source, target)
    except OSError as error:
        raise BatchError(
            f"cannot rename batch directory '{source}' to '{target}': "
            f"{error.strerror or error}"
        ) from error


def _check_file_list(
    files: Mapping[str, str], counts: Mapping[str, int], mode: str
) -> None:
    """Checks the list of files and the counts that the manifest declares."""
    if not any(_matches_parquet_name(name, "documents") for name in files):
        raise BatchError("a batch needs at least one documents file")
    if not any(_matches_parquet_name(name, "passages") for name in files):
        raise BatchError("a batch needs at least one passages file")
    if mode == "replace_all" and "deletions.parquet" in files:
        raise BatchError("mode 'replace_all' forbids a deletions file")
    if counts["deletions"] == 0 and "deletions.parquet" in files:
        raise BatchError("deletions.parquet is listed but counts.deletions is zero")
    if counts["deletions"] > 0 and "deletions.parquet" not in files:
        raise BatchError("counts.deletions is positive but deletions.parquet is missing")
    if counts["documents"] == 0 and counts["passages"] != 0:
        raise BatchError("a batch with no document cannot have passages")
    if counts["passages"] < counts["documents"]:
        raise BatchError("every document has at least one passage")


def _matches_parquet_name(name: str, stem: str) -> bool:
    """Mirrors ``matches_parquet_name`` of ``crible-core::batch``.

    A role is either ``<stem>.parquet`` or ``<stem>-NNNN.parquet`` with at
    least four ASCII digits.
    """
    if name == f"{stem}.parquet":
        return True
    prefix = f"{stem}-"
    suffix = ".parquet"
    if not name.startswith(prefix) or not name.endswith(suffix):
        return False
    digits = name[len(prefix) : -len(suffix)]
    return len(digits) >= 4 and all(character in "0123456789" for character in digits)


def _check_batch_id(batch_id: Any) -> str:
    if not isinstance(batch_id, str):
        raise BatchError(f"batch_id must be a string, got {type(batch_id).__name__}")
    if not batch_id:
        raise BatchError("batch_id must not be empty")
    if batch_id in (".", ".."):
        raise BatchError(f"batch_id '{batch_id}' is not a directory name")
    if any(
        character in "/\\" or ord(character) < 0x20 or 0x7F <= ord(character) <= 0x9F
        for character in batch_id
    ):
        raise BatchError(f"batch_id '{batch_id}' must be a single path segment")
    return batch_id


def _check_mode(mode: Any) -> str:
    if mode not in MODES:
        raise BatchError(f"unknown mode '{mode}'; expected 'upsert' or 'replace_all'")
    return mode


def _check_doc_id(doc_id: Any) -> str:
    if not isinstance(doc_id, str):
        raise BatchError(f"doc_id must be a string, got {type(doc_id).__name__}")
    if not doc_id:
        raise BatchError("doc_id must not be empty")
    return doc_id


def _manifest_model(model: Mapping[str, Any], vector_dtype: str) -> dict[str, Any]:
    """Derives the ``model`` member of the manifest from a resolved ``ModelConfig``."""
    if not isinstance(model, Mapping):
        raise BatchError(f"model must be an object, got {type(model).__name__}")
    for key in model:
        if key not in _MODEL_KEYS:
            raise BatchError(f"unknown key '{key}' at model")
    kind = model.get("kind")
    if kind not in ("onnx", "external"):
        raise BatchError(f"model.kind is '{kind}'; expected 'onnx' or 'external'")
    dims = model.get("dims")
    if isinstance(dims, bool) or not isinstance(dims, int) or dims <= 0:
        raise BatchError("model.dims must be a strictly positive integer")
    normalize = model.get("normalize")
    if not isinstance(normalize, bool):
        raise BatchError("model.normalize must be a boolean")

    block: dict[str, Any] = {}
    if kind == "onnx":
        for key in ("hf_id", "revision", "pooling"):
            value = model.get(key)
            if not isinstance(value, str):
                raise BatchError(f"model.{key} must be a string for an onnx model")
            block[key] = value
        prompts = model.get("prompts")
        if not isinstance(prompts, Mapping):
            raise BatchError("model.prompts must be an object for an onnx model")
        for key in prompts:
            if key not in _PROMPTS_KEYS:
                raise BatchError(f"unknown key '{key}' at model.prompts")
        document_prompt = prompts.get("document")
        if not isinstance(document_prompt, str):
            raise BatchError("model.prompts.document must be a string for an onnx model")
        block["prompt_document"] = document_prompt
    block["dims"] = dims
    block["normalize"] = normalize
    block["vector_dtype"] = vector_dtype
    return block


def _chunker(chunker: Mapping[str, Any]) -> dict[str, Any]:
    if not isinstance(chunker, Mapping):
        raise BatchError(f"chunker must be an object, got {type(chunker).__name__}")
    for key in chunker:
        if key not in _CHUNKER_KEYS:
            raise BatchError(f"unknown key '{key}' at chunker")
    name = chunker.get("name")
    version = chunker.get("version")
    if not isinstance(name, str) or not isinstance(version, str):
        raise BatchError("chunker.name and chunker.version must be strings")
    params = chunker.get("params", {})
    if not isinstance(params, Mapping):
        raise BatchError("chunker.params must be an object")
    return {"name": name, "version": version, "params": dict(params)}


def _template(template: Mapping[str, Any]) -> dict[str, Any]:
    if not isinstance(template, Mapping):
        raise BatchError(f"template must be an object, got {type(template).__name__}")
    for key in template:
        if key not in _TEMPLATE_KEYS:
            raise BatchError(f"unknown key '{key}' at template")
    name = template.get("name")
    version = template.get("version")
    if not isinstance(name, str) or not isinstance(version, str):
        raise BatchError("template.name and template.version must be strings")
    return {"name": name, "version": version}


def _blake3_file(path: Path) -> str:
    hasher = blake3.blake3()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            hasher.update(chunk)
    return f"blake3:{hasher.hexdigest()}"


def _check_created_at(created_at: Any) -> str | None:
    """Validates the optional manifest timestamp.

    ``None`` keeps the default: the current UTC time at ``close()``. A caller
    that needs reproducible files, such as a synthetic data generator, passes
    a fixed RFC 3339 string.
    """
    if created_at is None:
        return None
    if not isinstance(created_at, str):
        raise BatchError(
            f"created_at must be a string, got {type(created_at).__name__}"
        )
    if not created_at:
        raise BatchError("created_at must not be empty")
    return created_at


def _check_host(host: Any) -> str:
    """Validates the optional ``producer.host`` and applies its default.

    ``None`` keeps the historical value, the machine hostname. A producer that
    needs a reproducible dataset passes a stable identifier instead, as allowed
    by ADR-0036.
    """
    if host is None:
        return socket.gethostname()
    if not isinstance(host, str):
        raise BatchError(f"host must be a string, got {type(host).__name__}")
    if not host:
        raise BatchError("host must not be empty")
    return host


def _utc_now() -> str:
    now = datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0)
    return now.strftime("%Y-%m-%dT%H:%M:%SZ")
