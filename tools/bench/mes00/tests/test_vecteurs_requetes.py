import pytest

import mes00lib as lib
import vecteurs_requetes as vectors


def query(query_id, corpus):
    return {"query_id": query_id, "corpus": corpus, "text": "Titre"}


def test_schema_version_is_declared():
    assert vectors.SCHEMA_VERSION == 1


def test_select_keeps_file_order_and_caps_each_corpus():
    selected = vectors.select(
        [
            query("q1", "juri"),
            query("q2", "jorf"),
            query("q3", "juri"),
            query("q4", "juri"),
        ],
        per_corpus=2,
    )
    assert [row["query_id"] for row in selected] == ["q1", "q2", "q3"]


def test_select_accepts_legacy_text_and_rejects_bad_rows():
    selected = vectors.select(
        [
            {
                "requete_id": "q1",
                "corpus": "juri",
                "texte": "Titre ancien",
            }
        ],
        per_corpus=1,
    )
    assert selected[0]["query_id"] == "q1"
    assert selected[0]["text"] == "Titre ancien"
    with pytest.raises(lib.Mes00Error, match="no corpus"):
        vectors.select([{"query_id": "q1", "text": "Titre"}], per_corpus=1)
    with pytest.raises(lib.Mes00Error, match="no text"):
        vectors.select([{"query_id": "q1", "corpus": "juri"}], per_corpus=1)
    with pytest.raises(lib.Mes00Error, match="not an object"):
        vectors.select(["not-an-object"], per_corpus=1)
