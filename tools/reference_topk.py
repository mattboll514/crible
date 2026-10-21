# SPDX-License-Identifier: Apache-2.0

"""Exact numpy reference for the synthetic Crible dataset.

The reference replays the attach of the ``synth-small`` batches, then, for each
query of ``queries.jsonl``, filters the pool of live passages and returns the
top-k of the exact cosine. It writes ``expected.jsonl``, the file that J1
compares with ``crible search``.

Reference state
---------------

The expected results describe the index **after the three batches ``l1``,
``l2`` and ``l3`` are attached**, in that order, with the semantics of
``docs/04-donnees-et-ingestion.md``: unknown documents are added, a document
with the same ``doc_hash`` keeps its passages and gets its document fields
updated, a document with another hash replaces its passages, and a document in
``deletions.parquet`` loses every passage. Metadata-only updates therefore do
not change the pool.

Filter subset
-------------

The parser accepts the grammar of ``docs/05-api.md``: an implicit AND over the
keys of an object, ``$and``, ``$or``, ``$not``, a scalar shorthand for ``eq``,
and the operators ``eq``, ``ne``, ``lt``, ``lte``, ``gt``, ``gte``, ``in``,
``nin`` and ``exists``. The generated query set only uses this subset.

A filter is validated against the schema of ``index.json``, as the engine does:
an unknown field, a field marked ``filterable: false`` and an operator that the
field type does not accept are refused. :func:`parse_filter` takes the schema
as an optional second argument; :func:`reference` always passes the one of the
dataset, so a query that the engine would reject fails here instead of producing
a wrong expectation. :data:`TYPE_OPERATORS` holds the table of
``docs/05-api.md``.

A null field value never matches a comparison, ``ne`` or ``nin``; only
``exists`` looks at presence, and ``$not`` complements over every live passage.
That is the SQL three-valued rule of ADR-0035, now written in
``docs/03-recherche.md`` (filters to pool) and ``docs/05-api.md`` (filter
grammar). It implies that ``{"f": {"ne": "a"}}`` and ``{"$not": {"f": {"eq":
"a"}}}`` differ on a passage whose ``f`` is null.

Ranking
-------

Scores are dot products. Document vectors are normalized, and the query vector
is normalized here, so the score is the cosine. The order is descending score,
with a stable tie-break on the key ``doc_id#ord``, as J1 requires.
"""

from __future__ import annotations

import argparse
import datetime
import json
import sys
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import numpy as np
import pyarrow.parquet as pq

from gen_synth import dataset_checksum

#: Comparison operators accepted by the reference.
OPERATORS = ("eq", "ne", "lt", "lte", "gt", "gte", "in", "nin", "exists")

#: Operators each field type accepts, from the table of ``docs/05-api.md``.
TYPE_OPERATORS = {
    "keyword": ("eq", "ne", "in", "nin", "exists"),
    "integer": ("eq", "ne", "lt", "lte", "gt", "gte", "in", "exists"),
    "float": ("eq", "ne", "lt", "lte", "gt", "gte", "in", "exists"),
    "date": ("eq", "ne", "lt", "lte", "gt", "gte", "in", "exists"),
    "bool": ("eq", "exists"),
}


@dataclass(frozen=True)
class FieldSpec:
    """One field of the index schema, as the filter validator needs it."""

    field_type: str
    filterable: bool = True


@dataclass(frozen=True)
class PassageRow:
    """One live passage, with its document and passage fields merged."""

    key: str
    doc_id: str
    ord: int
    vector: np.ndarray
    fields: Mapping[str, Any]


@dataclass(frozen=True)
class Result:
    """One scored passage of the top-k."""

    key: str
    doc_id: str
    ord: int
    score: float


def load_index_config(dataset_dir: str | Path) -> dict[str, Any]:
    """Reads ``index.json`` of the dataset."""
    path = Path(dataset_dir) / "index.json"
    return json.loads(path.read_text(encoding="utf-8"))


def load_schema(config: Mapping[str, Any]) -> dict[str, FieldSpec]:
    """Returns every field declared by ``index.json``, with its type.

    The returned mapping keeps the ``filterable`` flag of each field, so that
    :func:`parse_filter` can refuse a field marked ``filterable: false``.
    Document fields and passage fields are merged: a filter names a field
    without telling its level, exactly like the engine. ``filterable`` defaults
    to true, as ``docs/04-donnees-et-ingestion.md`` specifies.
    """
    schema = config.get("schema")
    if not isinstance(schema, Mapping):
        raise ValueError("the index config has no 'schema' object")
    fields: dict[str, FieldSpec] = {}
    for level in ("document_fields", "passage_fields"):
        block = schema.get(level, {})
        if not isinstance(block, Mapping):
            raise ValueError(f"schema.{level} must be an object")
        for name, spec in block.items():
            if not isinstance(spec, Mapping):
                raise ValueError(f"schema.{level}.{name} must be an object")
            field_type = spec.get("type")
            if field_type not in TYPE_OPERATORS:
                raise ValueError(f"unknown type '{field_type}' for field '{name}'")
            filterable = spec.get("filterable", True)
            if not isinstance(filterable, bool):
                raise ValueError(f"schema.{level}.{name}.filterable must be a boolean")
            fields[name] = FieldSpec(field_type=field_type, filterable=filterable)
    return fields


def _vector_dtype(config: Mapping[str, Any]) -> str:
    precision = config.get("reference_precision")
    if precision not in ("fp16", "fp32"):
        raise ValueError(f"unknown reference_precision '{precision}'")
    return "<f2" if precision == "fp16" else "<f4"


def _decode_column(column: Iterable[Any], dtype: str) -> np.ndarray:
    return np.stack(
        [np.frombuffer(raw, dtype=dtype).astype(np.float32) for raw in column]
    )


def _read_documents(directory: Path, files: Sequence[str]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for name in files:
        rows.extend(pq.read_table(directory / name).to_pylist())
    return rows


def _read_passages(
    directory: Path, files: Sequence[str], dtype: str
) -> dict[str, list[tuple[int, np.ndarray, dict[str, Any]]]]:
    passages: dict[str, list[tuple[int, np.ndarray, dict[str, Any]]]] = {}
    for name in files:
        table = pq.read_table(directory / name)
        if table.num_rows == 0:
            continue
        vectors = _decode_column(table.column("vector").to_pylist(), dtype)
        doc_ids = table.column("doc_id").to_pylist()
        ranks = table.column("ord").to_pylist()
        reserved = {"doc_id", "ord", "text", "vector"}
        field_names = [field for field in table.column_names if field not in reserved]
        columns = {field: table.column(field).to_pylist() for field in field_names}
        for index, doc_id in enumerate(doc_ids):
            fields = {field: columns[field][index] for field in field_names}
            passages.setdefault(doc_id, []).append(
                (int(ranks[index]), vectors[index], fields)
            )
    return passages


def load_state(
    dataset_dir: str | Path,
) -> tuple[list[PassageRow], int, int]:
    """Replays the batches and returns the live passages of the final state.

    Batch directories are read in ascending ``batch_id`` order, which is
    ``l1``, ``l2``, ``l3`` for ``synth-small``. Returns the live passages, the
    number of live documents and the number of live passages.
    """
    directory = Path(dataset_dir)
    config = load_index_config(directory)
    dtype = _vector_dtype(config)
    batches = []
    for path in directory.iterdir():
        manifest_path = path / "manifest.json"
        if not manifest_path.is_file():
            continue
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        batches.append((str(manifest["batch_id"]), path, manifest))
    batches.sort(key=lambda batch: batch[0])
    if not batches:
        raise ValueError(f"no batch directory under {directory}")

    documents: dict[str, dict[str, Any]] = {}
    for _, batch, manifest in batches:
        files = manifest["files"]
        document_files = sorted(
            name for name in files if name.startswith("documents") and name.endswith(".parquet")
        )
        passage_files = sorted(
            name for name in files if name.startswith("passages") and name.endswith(".parquet")
        )
        rows = _read_documents(batch, document_files)
        passages = _read_passages(batch, passage_files, dtype)
        for row in rows:
            doc_id = row["doc_id"]
            doc_fields = {
                key: value
                for key, value in row.items()
                if key not in ("doc_id", "doc_hash")
            }
            stored = documents.get(doc_id)
            if stored is not None and stored["doc_hash"] == row["doc_hash"]:
                stored["fields"] = doc_fields
            else:
                documents[doc_id] = {
                    "doc_hash": row["doc_hash"],
                    "fields": doc_fields,
                    "passages": passages.get(doc_id, []),
                }
        deletion_file = files.get("deletions.parquet")
        if deletion_file:
            deletions = pq.read_table(batch / "deletions.parquet").to_pylist()
            for row in deletions:
                documents.pop(row["doc_id"], None)

    live: list[PassageRow] = []
    for doc_id in sorted(documents):
        stored = documents[doc_id]
        for rank, vector, passage_fields in sorted(
            stored["passages"], key=lambda passage: passage[0]
        ):
            live.append(
                PassageRow(
                    key=f"{doc_id}#{rank}",
                    doc_id=doc_id,
                    ord=rank,
                    vector=vector,
                    fields={**stored["fields"], **passage_fields},
                )
            )
    return live, len(documents), len(live)


def parse_filter(raw: Any, schema: Mapping[str, FieldSpec] | None = None) -> Any:
    """Parses the JSON filter into a small tuple tree.

    The returned node is ``("all",)`` for no filter, ``("cond", field,
    operator, value)``, ``("and", nodes)``, ``("or", nodes)`` or ``("not",
    node)``.

    With a ``schema``, the filter is validated as the engine does: the field
    must exist, be filterable, and accept the operator for its type
    (``docs/05-api.md``). Without it, only the shape of the filter is checked.
    """
    if raw is None:
        return ("all",)
    if not isinstance(raw, Mapping):
        raise ValueError(f"a filter must be an object, got {type(raw).__name__}")
    children = []
    for key, operand in raw.items():
        if key in ("$and", "$or"):
            if not isinstance(operand, list):
                raise ValueError(f"{key} expects an array")
            tag = "and" if key == "$and" else "or"
            children.append(
                (tag, tuple(parse_filter(item, schema) for item in operand))
            )
        elif key == "$not":
            children.append(("not", parse_filter(operand, schema)))
        elif key.startswith("$"):
            raise ValueError(f"unknown logical operator '{key}'")
        else:
            children.append(_parse_field(key, operand, schema))
    return _combine("and", children)


def _field_spec(
    field: str, schema: Mapping[str, FieldSpec] | None
) -> FieldSpec | None:
    """Returns the schema entry of a filter field, refusing an unusable one."""
    if schema is None:
        return None
    spec = schema.get(field)
    if spec is None:
        raise ValueError(
            f"unknown field '{field}' in filter; the index schema has no such field"
        )
    if not spec.filterable:
        raise ValueError(f"field '{field}' is not filterable")
    return spec


def _parse_field(
    field: str, operand: Any, schema: Mapping[str, FieldSpec] | None
) -> Any:
    spec = _field_spec(field, schema)
    if isinstance(operand, list):
        raise ValueError(f"field '{field}' expects a scalar or an operator object")
    if not isinstance(operand, Mapping):
        return ("cond", field, "eq", operand)
    if not operand:
        raise ValueError(f"field '{field}' expects at least one operator")
    conditions = []
    for operator, value in operand.items():
        if operator not in OPERATORS:
            raise ValueError(f"unknown operator '{operator}' for field '{field}'")
        if spec is not None and operator not in TYPE_OPERATORS[spec.field_type]:
            raise ValueError(
                f"operator '{operator}' is not allowed for field '{field}' of type "
                f"'{spec.field_type}'; expected one of: "
                f"{', '.join(TYPE_OPERATORS[spec.field_type])}"
            )
        if operator in ("in", "nin") and not isinstance(value, list):
            raise ValueError(
                f"operator '{operator}' for field '{field}' expects an array"
            )
        if operator == "exists" and not isinstance(value, bool):
            raise ValueError(
                f"operator 'exists' for field '{field}' expects a boolean"
            )
        conditions.append(("cond", field, operator, value))
    return _combine("and", conditions)


def _combine(tag: str, children: list[Any]) -> Any:
    if not children:
        return ("all",)
    if len(children) == 1:
        return children[0]
    return (tag, tuple(children))


def evaluate(node: Any, fields: Mapping[str, Any]) -> bool:
    """Evaluates one filter node against the merged fields of a passage."""
    tag = node[0]
    if tag == "all":
        return True
    if tag == "and":
        return all(evaluate(child, fields) for child in node[1])
    if tag == "or":
        return any(evaluate(child, fields) for child in node[1])
    if tag == "not":
        return not evaluate(node[1], fields)
    _, field, operator, operand = node
    value = fields.get(field)
    if operator == "exists":
        return (value is not None) == bool(operand)
    if value is None:
        return False
    if isinstance(value, datetime.date) and not isinstance(value, datetime.datetime):
        if isinstance(operand, str):
            operand = datetime.date.fromisoformat(operand)
        elif operator in ("in", "nin"):
            operand = [datetime.date.fromisoformat(item) for item in operand]
    if operator == "eq":
        return value == operand
    if operator == "ne":
        return value != operand
    if operator == "lt":
        return value < operand
    if operator == "lte":
        return value <= operand
    if operator == "gt":
        return value > operand
    if operator == "gte":
        return value >= operand
    if operator == "in":
        return value in operand
    if operator == "nin":
        return value not in operand
    raise ValueError(f"unknown operator '{operator}'")  # pragma: no cover


def select(passages: Sequence[PassageRow], node: Any) -> np.ndarray:
    """Returns the boolean mask of the passages that match the filter."""
    if node == ("all",):
        return np.ones(len(passages), dtype=bool)
    return np.fromiter(
        (evaluate(node, passage.fields) for passage in passages),
        dtype=bool,
        count=len(passages),
    )


def top_k(
    passages: Sequence[PassageRow],
    mask: np.ndarray,
    query: Sequence[float],
    k: int,
) -> list[Result]:
    """Returns the exact top-k of the cosine over the masked passages.

    The stored vectors are fp16 promoted to f32, the query is normalized and
    scored in f32, as the engine does (fiche P1, ``RECHERCHER-DENSE-EXACT``).
    The stored vectors are already normalized, so the dot product is the
    cosine. Ties are broken by key.
    """
    if k <= 0:
        raise ValueError("k must be strictly positive")
    indices = np.flatnonzero(mask)
    if indices.size == 0:
        return []
    vector = np.asarray(query, dtype=np.float32)
    norm = np.linalg.norm(vector)
    if norm == 0.0:
        raise ValueError("the query vector has a zero norm")
    vector /= norm
    matrix = np.stack([passages[int(index)].vector for index in indices]).astype(
        np.float32
    )
    scores = matrix @ vector
    order = sorted(
        range(indices.size), key=lambda pos: (-scores[pos], passages[int(indices[pos])].key)
    )
    results = []
    for pos in order[:k]:
        passage = passages[int(indices[pos])]
        results.append(
            Result(
                key=passage.key,
                doc_id=passage.doc_id,
                ord=passage.ord,
                score=float(scores[pos]),
            )
        )
    return results


def reference(dataset_dir: str | Path, expected: str | Path | None = None) -> Path:
    """Writes ``expected.jsonl`` for every query of the dataset."""
    directory = Path(dataset_dir)
    target = Path(expected) if expected is not None else directory / "expected.jsonl"
    schema = load_schema(load_index_config(directory))
    passages, documents, live = load_state(directory)

    temporary = target.with_name(target.name + ".tmp")
    with (directory / "queries.jsonl").open(encoding="utf-8") as source, temporary.open(
        "w", encoding="utf-8"
    ) as sink:
        for line in source:
            if not line.strip():
                continue
            query = json.loads(line)
            node = parse_filter(query.get("filter"), schema)
            mask = select(passages, node)
            results = top_k(passages, mask, query["vector"], int(query["k"]))
            record = {
                "query_id": query["query_id"],
                "k": int(query["k"]),
                "pool_size": int(mask.sum()),
                "results": [
                    {
                        "key": result.key,
                        "doc_id": result.doc_id,
                        "ord": result.ord,
                        "score": result.score,
                    }
                    for result in results
                ],
            }
            sink.write(
                json.dumps(record, ensure_ascii=True, separators=(",", ":")) + "\n"
            )
    temporary.replace(target)
    print(f"{target}: reference over {documents} documents, {live} live passages")
    return target


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--dataset",
        type=Path,
        default=Path("data") / "synth-small",
        help="dataset directory, containing index.json and the batches",
    )
    parser.add_argument(
        "--expected",
        type=Path,
        default=None,
        help="output file (default: <dataset>/expected.jsonl)",
    )
    arguments = parser.parse_args(argv)

    target = reference(arguments.dataset, arguments.expected)
    print(f"dataset checksum: {dataset_checksum(arguments.dataset)}")
    print(f"wrote {target}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
