# SPDX-License-Identifier: Apache-2.0

"""Tests of ``tools/demo_j2_batches.py``.

The script writes the two refused batches of the J2 demo. These tests cover the
selection, which needs no model, so CI runs them without the ``export`` group.
The two encoding paths run only in the demo, with the pinned mDenseOn files.
"""

from __future__ import annotations

import demo_j2_batches


def _document(doc_id):
    return {"doc_id": doc_id, "doc_hash": f"hash-{doc_id}", "annee": 2024}


def _passage(doc_id, ord):
    return {"doc_id": doc_id, "ord": ord, "text": f"{doc_id} passage {ord}"}


def test_selection_follows_the_lexicographic_document_order():
    documents = [_document("b"), _document("a")]
    passages = [_passage("b", 1), _passage("b", 0), _passage("a", 0)]

    selected = demo_j2_batches.select_documents(documents, passages)

    assert [document["doc_id"] for document, _ in selected] == ["a", "b"]
    assert [passage["ord"] for _, own in selected for passage in own] == [0, 0, 1]


def test_selection_skips_documents_without_a_passage():
    documents = [_document("a"), _document("b")]
    passages = [_passage("b", 0)]

    selected = demo_j2_batches.select_documents(documents, passages)

    assert [document["doc_id"] for document, _ in selected] == ["b"]


def test_selection_honours_the_document_budget():
    documents = [_document(f"d{number}") for number in range(4)]
    passages = [_passage(document["doc_id"], 0) for document in documents]

    selected = demo_j2_batches.select_documents(
        documents, passages, max_documents=2, max_passages=10
    )

    assert [document["doc_id"] for document, _ in selected] == ["d0", "d1"]


def test_selection_honours_the_passage_budget():
    documents = [_document(f"d{number}") for number in range(4)]
    passages = [_passage("d0", number) for number in range(3)]

    selected = demo_j2_batches.select_documents(
        documents, passages, max_documents=4, max_passages=2
    )

    assert len(selected) == 1
    assert [passage["ord"] for _, own in selected for passage in own] == [0, 1]
