# SPDX-License-Identifier: Apache-2.0

"""Tests of ``tools/reference_topk.py``.

The filters are checked on a tiny hand-written state, the top-k against a
brute-force loop, and the expected file against the real ``synth-small``
dataset.
"""

from __future__ import annotations

import datetime
import json

import numpy as np
import pyarrow.parquet as pq
import pytest

from reference_topk import (
    PassageRow,
    evaluate,
    load_index_config,
    load_schema,
    load_state,
    parse_filter,
    reference,
    select,
    top_k,
)


def row(key: str, vector, **fields) -> PassageRow:
    doc_id, rank = key.split("#")
    return PassageRow(
        key=key,
        doc_id=doc_id,
        ord=int(rank),
        vector=np.asarray(vector, dtype=np.float32),
        fields=fields,
    )


ROWS = [
    row(
        "d1#0",
        [1.0, 0.0],
        k="a",
        n=1,
        f=0.5,
        d=datetime.date(2020, 1, 1),
        b=True,
        p="x",
    ),
    row(
        "d2#0",
        [0.8, 0.6],
        k="b",
        n=2,
        f=0.8,
        d=datetime.date(2021, 1, 1),
        b=False,
        p="x",
    ),
    row(
        "d3#0",
        [0.0, 1.0],
        k=None,
        n=None,
        f=None,
        d=None,
        b=None,
        p=None,
    ),
]


#: Schema of the fields of :data:`ROWS`, plus a non-filterable one.
SCHEMA = load_schema(
    {
        "schema": {
            "document_fields": {
                "k": {"type": "keyword"},
                "n": {"type": "integer"},
                "f": {"type": "float"},
                "d": {"type": "date"},
                "b": {"type": "bool"},
                "p": {"type": "keyword"},
                "titre": {"type": "keyword", "filterable": False},
            }
        }
    }
)


def keys_for(raw, schema=None) -> list[str]:
    mask = select(ROWS, parse_filter(raw, schema))
    return [item.key for item, keep in zip(ROWS, mask) if keep]


@pytest.mark.parametrize(
    ("raw", "expected"),
    [
        (None, ["d1#0", "d2#0", "d3#0"]),
        ({}, ["d1#0", "d2#0", "d3#0"]),
        ({"k": "a"}, ["d1#0"]),
        ({"k": {"eq": "a"}}, ["d1#0"]),
        ({"k": {"ne": "a"}}, ["d2#0"]),
        ({"k": {"in": ["a", "b"]}}, ["d1#0", "d2#0"]),
        ({"k": {"nin": ["a"]}}, ["d2#0"]),
        ({"n": {"gte": 2}}, ["d2#0"]),
        ({"n": {"gt": 1, "lte": 2}}, ["d2#0"]),
        ({"d": {"lt": "2021-01-01"}}, ["d1#0"]),
        ({"f": {"gte": 0.8}}, ["d2#0"]),
        ({"b": False}, ["d2#0"]),
        ({"p": {"exists": False}}, ["d3#0"]),
        ({"p": {"exists": True}}, ["d1#0", "d2#0"]),
        ({"k": "a", "n": 1}, ["d1#0"]),
        ({"$or": [{"k": "a"}, {"n": 2}]}, ["d1#0", "d2#0"]),
        ({"$and": [{"k": "a"}, {"n": 2}]}, []),
        ({"$not": {"k": "a"}}, ["d2#0", "d3#0"]),
        ({"$not": {"$or": [{"k": "a"}, {"k": "b"}]}}, ["d3#0"]),
    ],
)
def test_filter_subset_matches_the_grammar(raw, expected):
    assert keys_for(raw) == expected


@pytest.mark.parametrize(
    "raw",
    [
        {"k": {"bogus": 1}},
        {"k": [1, 2]},
        {"k": {}},
        {"k": {"in": "a"}},
        {"k": {"nin": 1}},
        {"k": {"exists": 1}},
        {"k": {"exists": "true"}},
        {"$xor": [{"k": "a"}]},
        {"$and": {"k": "a"}},
    ],
)
def test_invalid_filters_are_refused(raw):
    with pytest.raises(ValueError):
        parse_filter(raw)


@pytest.mark.parametrize(
    ("raw", "expected"),
    [
        ({"k": {"nin": ["a"]}}, ["d2#0"]),
        ({"n": {"gte": 2}}, ["d2#0"]),
        ({"n": {"in": [1, 2]}}, ["d1#0", "d2#0"]),
        ({"f": {"gte": 0.8}}, ["d2#0"]),
        ({"d": {"lt": "2021-01-01"}}, ["d1#0"]),
        ({"b": False}, ["d2#0"]),
        ({"b": {"eq": True}}, ["d1#0"]),
        ({"p": {"exists": True}}, ["d1#0", "d2#0"]),
    ],
)
def test_an_operator_accepted_for_its_type_passes_validation(raw, expected):
    assert keys_for(raw, SCHEMA) == expected


@pytest.mark.parametrize(
    ("raw", "message"),
    [
        ({"n": {"nin": [1, 2]}}, r"operator 'nin'.*field 'n'.*'integer'"),
        ({"k": {"lt": "a"}}, r"operator 'lt'.*field 'k'.*'keyword'"),
        ({"k": {"gte": "a"}}, r"operator 'gte'.*field 'k'.*'keyword'"),
        ({"b": {"in": [True]}}, r"operator 'in'.*field 'b'.*'bool'"),
        ({"titre": "x"}, r"field 'titre' is not filterable"),
        ({"titre": {"nin": ["x"]}}, r"field 'titre' is not filterable"),
        ({"inconnu": 1}, r"unknown field 'inconnu'"),
        ({"$not": {"n": {"nin": [1]}}}, r"operator 'nin'.*field 'n'"),
        ({"$or": [{"k": "a"}, {"b": {"gte": True}}]}, r"operator 'gte'.*field 'b'"),
    ],
)
def test_a_filter_refused_by_the_type_names_the_field_and_the_operator(raw, message):
    with pytest.raises(ValueError, match=message):
        parse_filter(raw, SCHEMA)


def test_null_never_matches_a_comparison_but_not_is_complemented():
    node = parse_filter({"k": {"ne": "a"}})
    assert not evaluate(node, ROWS[2].fields)
    included = parse_filter({"$not": {"k": "a"}})
    assert evaluate(included, ROWS[2].fields)


def test_ne_and_not_eq_differ_on_a_null_passage():
    direct = keys_for({"k": {"ne": "a"}})
    complemented = keys_for({"$not": {"k": {"eq": "a"}}})

    assert direct == ["d2#0"]
    assert complemented == ["d2#0", "d3#0"]
    assert "d3#0" in complemented and "d3#0" not in direct


def test_exists_true_and_false_on_a_null_passage():
    assert keys_for({"k": {"exists": True}}) == ["d1#0", "d2#0"]
    assert keys_for({"k": {"exists": False}}) == ["d3#0"]


def test_top_k_is_exact_and_breaks_ties_by_key():
    rows = [
        row("b#0", [1.0, 0.0]),
        row("a#0", [1.0, 0.0]),
        row("c#0", [0.0, 1.0]),
    ]
    mask = np.ones(len(rows), dtype=bool)

    results = top_k(rows, mask, [2.0, 0.0], 2)

    assert [result.key for result in results] == ["a#0", "b#0"]
    assert all(abs(result.score - 1.0) < 1e-12 for result in results)
    assert [
        result.key
        for result in top_k(rows, np.array([True, False, True]), [1.0, 0.0], 2)
    ] == ["b#0", "c#0"]
    assert len(top_k(rows, mask, [1.0, 0.0], 10)) == 3


@pytest.mark.parametrize(("query", "k"), [([0.0, 0.0], 10), ([1.0, 0.0], 0)])
def test_top_k_refuses_a_zero_query_or_a_zero_k(query, k):
    with pytest.raises(ValueError):
        top_k(ROWS, np.ones(len(ROWS), dtype=bool), query, k)


def test_empty_pool_returns_no_result():
    assert top_k(ROWS, np.zeros(len(ROWS), dtype=bool), [1.0, 0.0], 3) == []


def brute_force(passages, mask, query, k):
    vector = np.asarray(query, dtype=np.float32)
    vector /= np.linalg.norm(vector)
    scored = [
        (float(np.dot(passage.vector.astype(np.float32), vector)), passage.key)
        for passage, keep in zip(passages, mask)
        if keep
    ]
    scored.sort(key=lambda item: (-item[0], item[1]))
    return scored[:k]


def test_every_generated_query_is_valid_against_the_index_schema(synth_dataset):
    schema = load_schema(load_index_config(synth_dataset.out_dir))
    queries = [
        json.loads(line)
        for line in (synth_dataset.out_dir / "queries.jsonl")
        .read_text(encoding="utf-8")
        .splitlines()
        if line.strip()
    ]

    assert len(queries) == 200
    for query in queries:
        parse_filter(query["filter"], schema)


def test_reference_covers_every_query_and_the_final_state(synth_dataset):
    target = reference(synth_dataset.out_dir)
    queries = [
        json.loads(line)
        for line in (synth_dataset.out_dir / "queries.jsonl")
        .read_text(encoding="utf-8")
        .splitlines()
    ]
    expected = [
        json.loads(line) for line in target.read_text(encoding="utf-8").splitlines()
    ]
    passages, documents, live = load_state(synth_dataset.out_dir)

    assert documents == 1_950
    assert live == 9_750
    assert len(expected) == len(queries) == 200
    assert [record["query_id"] for record in expected] == list(range(200))
    assert expected[0]["pool_size"] == live
    assert expected[0]["k"] == 10
    assert all(record["pool_size"] > 0 for record in expected)

    lookup = {passage.key: passage for passage in passages}
    for record in expected:
        scores = [result["score"] for result in record["results"]]
        assert scores == sorted(scores, reverse=True)
        assert len({result["key"] for result in record["results"]}) == len(scores)
        assert len(scores) == min(record["k"], record["pool_size"])
        for result in record["results"]:
            assert lookup[result["key"]].doc_id == result["doc_id"]
            assert lookup[result["key"]].ord == result["ord"]


def _read_batch(batch):
    """Reads documents and passages of one batch, decoding the vectors."""
    manifest = json.loads((batch / "manifest.json").read_text(encoding="utf-8"))
    dtype = "<f2" if manifest["model"]["vector_dtype"] == "fp16" else "<f4"
    documents = {
        row["doc_id"]: row
        for row in pq.read_table(batch / "documents.parquet").to_pylist()
    }
    passages = {}
    for name in manifest["files"]:
        if not name.startswith("passages"):
            continue
        for row in pq.read_table(batch / name).to_pylist():
            row["vector"] = np.frombuffer(row["vector"], dtype=dtype).astype(np.float32)
            passages[(row["doc_id"], int(row["ord"]))] = row
    return documents, passages


def test_state_replays_l2_passages_and_l3_metadata(synth_dataset):
    state, _, _ = load_state(synth_dataset.out_dir)
    by_document = {}
    for passage in state:
        by_document.setdefault(passage.doc_id, []).append(passage)
    for rows in by_document.values():
        rows.sort(key=lambda passage: passage.ord)

    l1_documents, l1_passages = _read_batch(synth_dataset.out_dir / "l1")
    l2_documents, l2_passages = _read_batch(synth_dataset.out_dir / "l2")
    l3_documents, _ = _read_batch(synth_dataset.out_dir / "l3")

    modified = sorted(
        doc_id
        for doc_id in l2_documents
        if doc_id in l1_documents
        and l2_documents[doc_id]["doc_hash"] != l1_documents[doc_id]["doc_hash"]
    )
    assert modified
    doc_id = modified[0]
    expected = sorted(
        (row for (owner, _), row in l2_passages.items() if owner == doc_id),
        key=lambda row: row["ord"],
    )
    assert [passage.key for passage in by_document[doc_id]] == [
        f"{doc_id}#{row['ord']}" for row in expected
    ]
    for passage, row in zip(by_document[doc_id], expected):
        assert np.array_equal(passage.vector, row["vector"])
        for field in ("date_effet", "partie", "poids", "principal", "rang"):
            assert passage.fields[field] == row[field]
        for field in (
            "annee",
            "categorie",
            "date_publication",
            "publie",
            "score",
            "source",
        ):
            assert passage.fields[field] == l2_documents[doc_id][field]

    metadata = sorted(l3_documents)
    assert metadata
    for doc_id in metadata:
        previous = l2_passages if doc_id in l2_documents else l1_passages
        expected = sorted(
            (row for (owner, _), row in previous.items() if owner == doc_id),
            key=lambda row: row["ord"],
        )
        assert expected
        assert [passage.key for passage in by_document[doc_id]] == [
            f"{doc_id}#{row['ord']}" for row in expected
        ]
        for passage, row in zip(by_document[doc_id], expected):
            assert np.array_equal(passage.vector, row["vector"])
            for field in ("date_effet", "partie", "poids", "principal", "rang"):
                assert passage.fields[field] == row[field]
        for field in (
            "annee",
            "categorie",
            "date_publication",
            "publie",
            "score",
            "source",
        ):
            assert by_document[doc_id][0].fields[field] == l3_documents[doc_id][field]


def test_expected_exercises_the_null_rule(synth_dataset):
    reference(synth_dataset.out_dir)
    queries = [
        json.loads(line)
        for line in (synth_dataset.out_dir / "queries.jsonl")
        .read_text(encoding="utf-8")
        .splitlines()
    ]
    expected = [
        json.loads(line)
        for line in (synth_dataset.out_dir / "expected.jsonl")
        .read_text(encoding="utf-8")
        .splitlines()
    ]
    live = expected[0]["pool_size"]

    def pool(wanted):
        ids = [
            query["query_id"] for query in queries if query["filter"] == wanted
        ]
        assert ids, wanted
        sizes = {expected[query_id]["pool_size"] for query_id in ids}
        assert len(sizes) == 1
        return sizes.pop()

    direct = pool({"source": {"ne": "journal"}})
    complemented = pool({"$not": {"source": "journal"}})
    present = pool({"source": {"exists": True}})
    absent = pool({"source": {"exists": False}})

    assert complemented > direct
    assert present + absent == live


def _query_families(queries):
    """Picks the first query of each filter family, for a stratified check."""
    families = {}
    for query in queries:
        raw = query["filter"]
        label = None
        if raw is None:
            label = "none"
        elif "$and" in raw:
            label = "and"
        elif "$or" in raw:
            label = "or"
        elif "$not" in raw:
            label = "not"
        elif "partie" in raw and isinstance(raw["partie"], dict):
            if "nin" in raw["partie"]:
                label = "nin"
            elif "exists" in raw["partie"]:
                label = "exists"
            else:
                label = "passage"
        elif "partie" in raw:
            label = "passage"
        elif "categorie" in raw:
            label = "in" if "in" in raw["categorie"] else "categorie"
        elif "source" in raw:
            operand = raw["source"]
            if isinstance(operand, dict) and "ne" in operand:
                label = "ne"
            elif isinstance(operand, dict) and "exists" in operand:
                label = "exists"
            else:
                label = "eq"
        elif "date_effet" in raw:
            label = "exists"
        elif any(
            isinstance(operand, dict) and {"lt", "lte", "gt", "gte"} & operand.keys()
            for operand in raw.values()
        ):
            label = "interval"
        if label is not None and label not in families:
            families[label] = query
    return families


def test_expected_keys_and_scores_match_a_brute_force(synth_dataset):
    reference(synth_dataset.out_dir)
    queries = [
        json.loads(line)
        for line in (synth_dataset.out_dir / "queries.jsonl")
        .read_text(encoding="utf-8")
        .splitlines()
    ]
    expected = {
        record["query_id"]: record
        for record in (
            json.loads(line)
            for line in (synth_dataset.out_dir / "expected.jsonl")
            .read_text(encoding="utf-8")
            .splitlines()
        )
    }
    passages, _, _ = load_state(synth_dataset.out_dir)

    families = _query_families(queries)
    assert {
        "none",
        "categorie",
        "in",
        "nin",
        "interval",
        "exists",
        "not",
        "or",
        "and",
        "passage",
        "ne",
    } <= families.keys()

    for query in families.values():
        record = expected[query["query_id"]]
        mask = select(passages, parse_filter(query["filter"]))
        brute = brute_force(passages, mask, query["vector"], record["k"])
        assert [result["key"] for result in record["results"]] == [
            key for _, key in brute
        ]
        assert [result["score"] for result in record["results"]] == pytest.approx(
            [score for score, _ in brute], abs=1e-6
        )


def test_document_fields_are_inherited_by_passages(synth_dataset):
    reference(synth_dataset.out_dir)
    queries = [
        json.loads(line)
        for line in (synth_dataset.out_dir / "queries.jsonl")
        .read_text(encoding="utf-8")
        .splitlines()
    ]
    expected = {
        record["query_id"]: record
        for record in (
            json.loads(line)
            for line in (synth_dataset.out_dir / "expected.jsonl")
            .read_text(encoding="utf-8")
            .splitlines()
        )
    }
    passages, _, _ = load_state(synth_dataset.out_dir)
    lookup = {passage.key: passage for passage in passages}

    category_query = next(
        query for query in queries if isinstance(query["filter"], dict)
        and set(query["filter"]) == {"categorie"}
        and isinstance(query["filter"]["categorie"], str)
    )
    category = category_query["filter"]["categorie"]
    results = expected[category_query["query_id"]]["results"]
    assert results
    assert all(
        lookup[result["key"]].fields["categorie"] == category for result in results
    )


def test_deleted_documents_are_absent_from_the_reference(synth_dataset):
    deletions = _deletion_ids(synth_dataset.out_dir / "l3")

    passages, _, _ = load_state(synth_dataset.out_dir)
    live = {passage.doc_id for passage in passages}
    assert deletions
    assert not deletions & live


def _deletion_ids(batch) -> set[str]:
    return {
        row["doc_id"] for row in pq.read_table(batch / "deletions.parquet").to_pylist()
    }
