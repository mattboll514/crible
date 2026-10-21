import json
import sys
import tempfile
from pathlib import Path

import numpy as np
import pytest

import fidelite_hnsw
import mes00lib as lib
import pgoptions_values as pgvalues


def test_recall_at_k_counts_the_exact_set():
    exact = ["a", "b", "c", "d"]
    approximate = ["a", "c", "x", "y"]
    assert fidelite_hnsw.recall_at_k(exact, approximate, 4) == 0.5
    assert fidelite_hnsw.recall_at_k(exact, approximate, 2) == 0.5


def test_recall_at_k_uses_available_exact_results():
    exact = ["a", "b"]
    approximate = ["b", "a", "z"]
    assert fidelite_hnsw.recall_at_k(exact, approximate, 10) == 1.0


def test_recall_at_k_rejects_empty_exact_result():
    with pytest.raises(lib.Mes00Error):
        fidelite_hnsw.recall_at_k([], ["a"], 10)


def test_normalise_vector_and_literal():
    assert fidelite_hnsw.normalise_vector([0.5, -1], label="q") == [0.5, -1.0]
    assert fidelite_hnsw.vector_literal([0.5, -1]) == "[0.5,-1]"
    with pytest.raises(lib.Mes00Error):
        fidelite_hnsw.normalise_vector([0.0, 0.0], label="q")
    with pytest.raises(lib.Mes00Error):
        fidelite_hnsw.normalise_vector([True], label="q")
    with pytest.raises(lib.Mes00Error):
        fidelite_hnsw.normalise_vector([float("nan")], label="q")


def test_ef_search_and_template_fingerprint_label_a_run():
    template = ["psql", "-c", "SET LOCAL hnsw.ef_search = 200; SELECT 1"]
    assert fidelite_hnsw.ef_search_of(template) == 200
    assert fidelite_hnsw.ef_search_of(["psql", "-c", "SELECT 1"]) is None
    digest = fidelite_hnsw.template_fingerprint(template)
    assert len(digest) == 64
    assert digest != fidelite_hnsw.template_fingerprint(["psql", "-c", "SELECT 1"])


def test_load_query_vectors_validates_corpus_for_sql_mode():
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    path = root / "queries.json"
    path.write_text(
        json.dumps(
            {
                "queries": [
                    {
                        "query_id": "q1",
                        "corpus": "juri' OR 1=1 --",
                        "vector": [1.0, 0.0],
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    with pytest.raises(lib.Mes00Error, match="invalid corpus"):
        fidelite_hnsw.load_query_vectors(path)

    path.write_text(
        json.dumps(
            {
                "queries": [
                    {
                        "query_id": "q1",
                        "corpus": "juri_sommaires",
                        "vector": [1.0, 0.0],
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    assert fidelite_hnsw.load_query_vectors(path)[0]["corpus"] == "juri_sommaires"


def test_exact_topk_is_consistent_across_chunk_sizes():
    identifiers = ["a", "b", "c", "d"]
    vectors = np.asarray(
        [[1.0, 0.0], [0.9, 0.1], [0.0, 1.0], [-1.0, 0.0]], dtype=np.float32
    )
    query = [1.0, 0.0]
    whole = fidelite_hnsw.exact_topk_from_vectors(
        query, identifiers, vectors, k=2, chunk_size=100
    )
    chunked = fidelite_hnsw.exact_topk_from_vectors(
        query, identifiers, vectors, k=2, chunk_size=1
    )
    assert whole == chunked == ["a", "b"]


def test_exact_topk_rejects_dimension_mismatch():
    with pytest.raises(lib.Mes00Error, match="dimension"):
        fidelite_hnsw.exact_topk_from_vectors(
            [1.0, 0.0],
            ["a"],
            np.asarray([[1.0, 0.0, 0.0]], dtype=np.float32),
            k=1,
        )


def test_load_export_directory_uses_ids_jsonl():
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    corpus = root / "juri"
    corpus.mkdir()
    np.save(corpus / "vectors.npy", np.eye(3, dtype=np.float32))
    (corpus / "ids.jsonl").write_text('"a"\n"b"\n"c"\n', encoding="utf-8")
    identifiers, vectors = fidelite_hnsw._load_corpus_export(root, "juri")
    assert identifiers == ["a", "b", "c"]
    assert vectors.shape == (3, 3)


def test_load_export_npz_and_rejects_bad_corpus_name():
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    np.savez(
        root / "jorf.npz",
        ids=np.asarray(["a", "b"]),
        vectors=np.eye(2, dtype=np.float32),
    )
    identifiers, _ = fidelite_hnsw._load_corpus_export(root, "jorf")
    assert identifiers == ["a", "b"]
    with pytest.raises(lib.Mes00Error, match="invalid corpus"):
        fidelite_hnsw._load_corpus_export(root, "../jorf")


def test_ids_from_payload_accepts_lists_and_objects():
    assert fidelite_hnsw._ids_from_payload(["a", "b"], label="x") == ["a", "b"]
    assert fidelite_hnsw._ids_from_payload(
        {"ids": [{"id": "a"}]}, label="x"
    ) == ["a"]
    with pytest.raises(lib.Mes00Error):
        fidelite_hnsw._ids_from_payload({"x": 1}, label="x")


def write_query_file(root: Path, vectors):
    payload = {
        "queries": [
            {
                "query_id": f"q{index}",
                "corpus": "juri",
                "vector": vector,
            }
            for index, vector in enumerate(vectors, start=1)
        ]
    }
    path = root / "vectors.json"
    path.write_text(json.dumps(payload), encoding="utf-8")
    return path


def test_cli_export_mode_computes_recall(monkeypatch):
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    export = root / "export"
    corpus = export / "juri"
    corpus.mkdir(parents=True)
    vectors = np.asarray(
        [[1.0, 0.0], [0.9, 0.1], [0.0, 1.0], [-1.0, 0.0]], dtype=np.float32
    )
    np.save(corpus / "vectors.npy", vectors)
    (corpus / "ids.jsonl").write_text('"a"\n"b"\n"c"\n"d"\n', encoding="utf-8")
    source = write_query_file(root, [[1.0, 0.0]])
    output = root / "fidelity.json"
    child = "import json; print(json.dumps({'ids': ['a', 'b']}))"
    monkeypatch.setenv("PROD_PGOPTIONS", pgvalues.VALID)

    assert (
        fidelite_hnsw.main(
            [
                "--mode",
                "export",
                "--queries",
                str(source),
                "--export-dir",
                str(export),
                "--hnsw-command-json",
                json.dumps([sys.executable, "-c", child]),
                "--k",
                "2",
                "--output",
                str(output),
            ]
        )
        == 0
    )
    document = json.loads(output.read_text(encoding="utf-8"))
    assert document["global"]["mean"] == 1.0
    assert document["by_corpus"]["juri"]["n"] == 1
    assert document["details"][0]["exact_ids"] == ["a", "b"]
    assert document["meta"]["ef_search"] is None
    assert len(document["meta"]["hnsw_template_sha256"]) == 64
    assert document["meta"]["exact_template_sha256"] is None


def test_cli_sql_mode_caps_the_number_of_queries(monkeypatch):
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    source = write_query_file(root, [[1.0, 0.0]] * 3)
    monkeypatch.setenv("PROD_PGOPTIONS", pgvalues.VALID)
    command = json.dumps([sys.executable, "-c", "print('[]')"])
    assert (
        fidelite_hnsw.main(
            [
                "--mode",
                "sql",
                "--queries",
                str(source),
                "--max-queries",
                "2",
                "--hnsw-command-json",
                command,
                "--exact-command-json",
                command,
                "--output",
                str(root / "out.json"),
            ]
        )
        == 2
    )


def test_cli_refuses_missing_pgoptions(monkeypatch):
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    source = write_query_file(root, [[1.0, 0.0]])
    monkeypatch.delenv("PROD_PGOPTIONS", raising=False)
    assert (
        fidelite_hnsw.main(
            [
                "--mode",
                "export",
                "--queries",
                str(source),
                "--hnsw-command-json",
                "[\"true\"]",
                "--output",
                str(root / "out.json"),
            ]
        )
        == 2
    )


def test_cli_refuses_output_outside_tmp(monkeypatch):
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    source = write_query_file(root, [[1.0, 0.0]])
    monkeypatch.setenv("PROD_PGOPTIONS", pgvalues.VALID)
    assert (
        fidelite_hnsw.main(
            [
                "--mode",
                "export",
                "--queries",
                str(source),
                "--hnsw-command-json",
                "[\"true\"]",
                "--output",
                str(Path(__file__).resolve().parents[3] / "interdit.json"),
            ]
        )
        == 2
    )
