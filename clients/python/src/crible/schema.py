# SPDX-License-Identifier: Apache-2.0

"""Index schema and the Parquet schemas of a batch.

The column names and types come from ``docs/format-lot-v1.md`` (section
"Colonnes Parquet"). Declared fields are ordered alphabetically, like the
``BTreeMap`` of ``crible-core``, so two runs of the same producer write the same
column order.

Only the parts of the index configuration needed to write a batch are parsed:
the index name, the reference precision and the schema. The other settings
(``lexical``, ``search``, ``ingest``, ``compaction``) are accepted but ignored.
"""

from __future__ import annotations

import datetime
from dataclasses import dataclass
from collections.abc import Iterable
from typing import Any, Mapping

import pyarrow as pa

from .errors import BatchError

FIELD_TYPES = ("keyword", "integer", "float", "date", "bool")

# Columns the format reserves. A declared field with one of these names would
# collide with the column of the batch itself.
RESERVED_DOCUMENT_COLUMNS = ("doc_hash", "doc_id")
RESERVED_PASSAGE_COLUMNS = ("doc_id", "ord", "text", "vector")

INDEX_CONFIG_KEYS = (
    "name",
    "model",
    "reference_precision",
    "schema",
    "cluster_by",
    "lexical",
    "search",
    "ingest",
    "compaction",
)

_SCHEMA_KEYS = ("document_fields", "passage_fields")
_FIELD_SPEC_KEYS = ("type", "filterable", "returned")

_INT64_MIN = -(2**63)
_INT64_MAX = 2**63 - 1


@dataclass(frozen=True)
class FieldSpec:
    """One declared field of the index schema."""

    name: str
    field_type: str


@dataclass(frozen=True)
class IndexSchema:
    """The subset of ``index.json`` that a batch depends on."""

    name: str
    model: str
    reference_precision: str
    document_fields: tuple[FieldSpec, ...]
    passage_fields: tuple[FieldSpec, ...]

    @classmethod
    def from_index_config(cls, index_config: Mapping[str, Any]) -> IndexSchema:
        """Parses and validates an index configuration."""
        config = _require_mapping(index_config, "index_config")
        _reject_unknown_keys(config, INDEX_CONFIG_KEYS, "index_config")
        name = _require_str(config, "name", "index_config.name")
        if not name:
            raise BatchError("index_config.name must not be empty")
        model = _require_str(config, "model", "index_config.model")
        if not model:
            raise BatchError("index_config.model must not be empty")
        precision = _require_str(config, "reference_precision", "index_config.reference_precision")
        if precision not in ("fp16", "fp32"):
            raise BatchError(
                "index_config.reference_precision is "
                f"'{precision}'; expected 'fp16' or 'fp32'"
            )
        schema = _require_mapping(
            _require_member(config, "schema", "index_config"), "index_config.schema"
        )
        _reject_unknown_keys(schema, _SCHEMA_KEYS, "index_config.schema")
        document_fields = _parse_fields(
            _require_mapping(
                _require_member(schema, "document_fields", "index_config.schema"),
                "index_config.schema.document_fields",
            ),
            "index_config.schema.document_fields",
            RESERVED_DOCUMENT_COLUMNS,
        )
        passage_fields = _parse_fields(
            _require_mapping(
                _require_member(schema, "passage_fields", "index_config.schema"),
                "index_config.schema.passage_fields",
            ),
            "index_config.schema.passage_fields",
            RESERVED_PASSAGE_COLUMNS,
        )
        document_names = {field.name for field in document_fields}
        for field in passage_fields:
            if field.name in document_names:
                raise BatchError(
                    f"field '{field.name}' is declared in both document_fields and "
                    "passage_fields; a field name must designate one level"
                )
        return cls(
            name=name,
            model=model,
            reference_precision=precision,
            document_fields=document_fields,
            passage_fields=passage_fields,
        )

    @property
    def vector_dtype(self) -> str:
        """Vector storage type implied by the reference precision (ADR-0005)."""
        return self.reference_precision

    def document_schema(self) -> pa.Schema:
        """Arrow schema of ``documents.parquet``."""
        fields = [
            pa.field("doc_id", pa.string(), nullable=False),
            pa.field("doc_hash", pa.string(), nullable=False),
        ]
        fields.extend(
            pa.field(spec.name, _arrow_type(spec.field_type))
            for spec in self.document_fields
        )
        return pa.schema(fields)

    def passage_schema(self, dims: int, vector_dtype: str) -> pa.Schema:
        """Arrow schema of a ``passages`` file."""
        width = dims * (2 if vector_dtype == "fp16" else 4)
        fields = [
            pa.field("doc_id", pa.string(), nullable=False),
            pa.field("ord", pa.uint32(), nullable=False),
            pa.field("text", pa.string(), nullable=False),
        ]
        fields.extend(
            pa.field(spec.name, _arrow_type(spec.field_type))
            for spec in self.passage_fields
        )
        fields.append(pa.field("vector", pa.binary(width), nullable=False))
        return pa.schema(fields)

    @staticmethod
    def deletion_schema() -> pa.Schema:
        """Arrow schema of ``deletions.parquet``."""
        return pa.schema([pa.field("doc_id", pa.string(), nullable=False)])

    def document_values(self, fields: Mapping[str, Any]) -> dict[str, Any]:
        """Validates document fields and returns them keyed by column name.

        Missing fields are null: every declared field is nullable.
        """
        values = _require_mapping(fields, "document fields")
        declared = {spec.name for spec in self.document_fields}
        _reject_unknown_keys(
            values,
            declared,
            "document fields",
            hint="the index schema has no such document field",
        )
        return {
            spec.name: coerce_value(spec, values.get(spec.name), f"document field '{spec.name}'")
            for spec in self.document_fields
        }

    def passage_values(self, passage: Mapping[str, Any], path: str) -> dict[str, Any]:
        """Validates the declared passage fields of one passage."""
        declared = {spec.name for spec in self.passage_fields}
        _reject_unknown_keys(
            passage,
            declared | {"text", "vector"},
            path,
            hint="the index schema has no such passage field",
        )
        return {
            spec.name: coerce_value(spec, passage.get(spec.name), f"{path}.{spec.name}")
            for spec in self.passage_fields
        }


def coerce_value(spec: FieldSpec, value: Any, path: str) -> Any:
    """Checks one field value against its declared type."""
    if value is None:
        return None
    if spec.field_type == "keyword":
        if not isinstance(value, str):
            raise BatchError(f"{path} must be a string, got {_type_name(value)}")
        return value
    if spec.field_type == "integer":
        if isinstance(value, bool) or not isinstance(value, int):
            raise BatchError(f"{path} must be an integer, got {_type_name(value)}")
        if not _INT64_MIN <= value <= _INT64_MAX:
            raise BatchError(f"{path} does not fit in a signed 64-bit integer: {value}")
        return value
    if spec.field_type == "float":
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise BatchError(f"{path} must be a number, got {_type_name(value)}")
        return float(value)
    if spec.field_type == "date":
        if isinstance(value, datetime.datetime):
            raise BatchError(f"{path} must be a date without time, got a datetime")
        if isinstance(value, datetime.date):
            return value
        if isinstance(value, str):
            try:
                return datetime.date.fromisoformat(value)
            except ValueError as error:
                raise BatchError(
                    f"{path} must be a date written YYYY-MM-DD, got '{value}': {error}"
                ) from error
        raise BatchError(f"{path} must be a date, got {_type_name(value)}")
    if spec.field_type == "bool":
        if not isinstance(value, bool):
            raise BatchError(f"{path} must be a boolean, got {_type_name(value)}")
        return value
    raise BatchError(f"unknown field type '{spec.field_type}' at {path}")  # pragma: no cover


def _parse_fields(
    raw: Mapping[str, Any], path: str, reserved: tuple[str, ...]
) -> tuple[FieldSpec, ...]:
    fields: list[FieldSpec] = []
    for name in sorted(raw):
        if name in reserved:
            raise BatchError(
                f"field '{name}' at {path} collides with the reserved column '{name}'"
            )
        spec = _require_mapping(raw[name], f"{path}.{name}")
        _reject_unknown_keys(spec, _FIELD_SPEC_KEYS, f"{path}.{name}")
        field_type = _require_str(spec, "type", f"{path}.{name}.type")
        if field_type not in FIELD_TYPES:
            raise BatchError(
                f"unknown field type '{field_type}' at {path}.{name}.type; "
                f"expected one of {', '.join(FIELD_TYPES)}"
            )
        for option in ("filterable", "returned"):
            if option in spec and not isinstance(spec[option], bool):
                raise BatchError(f"{path}.{name}.{option} must be a boolean")
        fields.append(FieldSpec(name=name, field_type=field_type))
    return tuple(fields)


def _arrow_type(field_type: str) -> pa.DataType:
    if field_type == "keyword":
        return pa.string()
    if field_type == "integer":
        return pa.int64()
    if field_type == "float":
        return pa.float64()
    if field_type == "date":
        return pa.date32()
    if field_type == "bool":
        return pa.bool_()
    raise BatchError(f"unknown field type '{field_type}'")  # pragma: no cover


def _type_name(value: Any) -> str:
    return type(value).__name__


def _require_member(mapping: Mapping[str, Any], key: str, path: str) -> Any:
    if key not in mapping:
        raise BatchError(f"missing member '{key}' at {path}.{key}")
    return mapping[key]


def _require_str(mapping: Mapping[str, Any], key: str, path: str) -> str:
    value = _require_member(mapping, key, path)
    if not isinstance(value, str):
        raise BatchError(f"{path} must be a string, got {_type_name(value)}")
    return value


def _require_mapping(value: Any, path: str) -> Mapping[str, Any]:
    if not isinstance(value, Mapping):
        raise BatchError(f"{path} must be an object, got {_type_name(value)}")
    return value


def _reject_unknown_keys(
    mapping: Mapping[str, Any],
    allowed: Iterable[str],
    path: str,
    hint: str | None = None,
) -> None:
    allowed_keys = set(allowed)
    for key in mapping:
        if key not in allowed_keys:
            detail = f"; {hint}" if hint else ""
            raise BatchError(f"unknown key '{key}' at {path}{detail}")
