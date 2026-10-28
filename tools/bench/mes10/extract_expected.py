# SPDX-License-Identifier: Apache-2.0
"""Extraction of the 55 expected documents missing from the Legifacile sample.

MES-10 and MES-11 (protocol: ADR-0092) are measured on an enriched measurement
set, `data/mes10/`: a copy of the Legifacile sample plus the documents the 57
labelled questions expect and the sample does not hold. This script reads those
documents from production **read-only** (ADR-0032) and writes them as a batch
in the format of the sample's batches.

What it does:

1. reads `data/legifacile-sample/queries.jsonl`, keeps the labelled questions
   (`Q01` … `Q58`, 57 lines, `Q40` absent), and takes every expected document;
2. checks against the sample's `documents.parquet` which of them are already
   present, so that only the absent ones are extracted;
3. asks production for the corpus and the passage count of each of them, in a
   single read-only `COPY … TO STDOUT` (binary), so the structure of a document
   comes from the database and not from a guess;
4. reuses the export machinery of the Legifacile clone
   (`scripts/export_crible_sample.py`, read from `PROD_REPO` of `.env.prod`) for
   the two points that must not drift: the text is recomposed with the current
   production composer and split with the current production splitter, and the
   `src_hash`, the passage count, the token count, the `partie` and the `ord` are
   verified against the database before a document enters the batch;
5. writes one batch per structure under `data/mes10/batches/`, in fp16, with the
   production vectors (`halfvec(768)`), plus a journal per structure.

No value of `.env.prod` is written or printed: only variable names are cited,
and remote errors are filtered by the production script.

Usage, from the repository root::

    (cd tools && uv run --no-sync python bench/mes10/extract_expected.py --dry-run)
    (cd tools && uv run --no-sync python bench/mes10/extract_expected.py)
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
SAMPLE = ROOT / "data" / "legifacile-sample"
QUERIES = SAMPLE / "queries.jsonl"
DEFAULT_OUT = ROOT / "data" / "mes10"

#: Structure of every production corpus, as `docs/02-architecture.md` names it.
CORPUS_STRUCTURE = {
    "textes_vigueur": "articles",
    "textes_hors_vigueur": "articles",
    "jorf_normatif": "jorf",
    "juri_sommaires": "decisions",
    "juri_texte_integral": "decisions",
    "sarde": "sarde",
    "dole": "dole",
    "titres_textes": "titres",
}

#: Date of the manifests of the enriched batch, frozen so that two runs produce
#: the same manifest byte for byte (same rule as the sample, ADR-0092).
DATE_LOT = "2026-10-28T00:00:00Z"


def load_production(prod_repo: Path):
    """The export script of the Legifacile clone, imported as a module.

    Reusing it keeps the recomposition, the split and the controls identical to
    the ones that produced the sample: a second implementation would be a second
    truth.
    """
    script = prod_repo / "scripts" / "export_crible_sample.py"
    if not script.is_file():
        raise SystemExit("the production export script is missing from PROD_REPO")
    specification = importlib.util.spec_from_file_location("export_crible_sample", script)
    if specification is None or specification.loader is None:
        raise SystemExit("the production export script cannot be imported")
    module = importlib.util.module_from_spec(specification)
    sys.modules["export_crible_sample"] = module
    specification.loader.exec_module(module)
    return module


def labelled_questions(path: Path) -> list[dict[str, Any]]:
    """The 57 labelled questions of `queries.jsonl`, in file order."""
    rows = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line]
    questions = [row for row in rows if str(row.get("id", "")).startswith("Q")]
    if len(questions) != 57:
        raise SystemExit(f"expected 57 labelled questions, found {len(questions)}")
    return questions


def sample_documents(path: Path) -> set[str]:
    """External identifiers already present in the sample's batches."""
    import pyarrow.parquet as pq

    present: set[str] = set()
    sample_batches = path / "batches"
    for documents in sorted(sample_batches.glob("legifacile-sample-*-v1/documents.parquet")):
        table = pq.read_table(documents, columns=["doc_id"])
        present.update(table.column("doc_id").to_pylist())
    if not present:
        raise SystemExit("the sample holds no document: is data/legifacile-sample/ built?")
    return present


def expected_documents(path: Path) -> list[dict[str, Any]]:
    """Expected documents of the labelled questions, deduplicated, in file order."""
    seen: set[str] = set()
    documents: list[dict[str, Any]] = []
    for question in labelled_questions(path):
        for expected in question.get("expected") or []:
            identifier = expected.get("src_id")
            if not identifier:
                continue
            if identifier in seen:
                continue
            seen.add(identifier)
            documents.append(
                {
                    "src_id": identifier,
                    "question": question["id"],
                    "type": expected.get("type"),
                    "satisfiable": bool(question.get("satisfiable")),
                    "expected_in_sample": bool(question.get("expected_in_sample")),
                }
            )
    return documents


def production_location(module, env: dict[str, str], documents: list[dict[str, Any]]):
    """Corpus and passage count of each document, read from production once."""
    values = ", ".join(
        f"({module._litteral(document['src_id'])}, {rank})"
        for rank, document in enumerate(documents, start=1)
    )
    sql = (
        "COPY (WITH sel(src_id, rang) AS (VALUES " + values + ")\n"
        "SELECT sel.src_id, s.corpus, s.n_passages\n"
        "FROM sel JOIN vec.sources s ON s.src_id = sel.src_id\n"
        "ORDER BY sel.rang) TO STDOUT WITH (FORMAT binary);"
    )
    found: dict[str, tuple[str, int]] = {}
    for row in module.flux_sql(env, sql):
        identifier = module._texte(row[0])
        corpus = module._texte(row[1])
        passages = module._entier(row[2])
        found[identifier] = (corpus, passages or 0)
    return found


def extract(module, env: dict[str, str], structure: str, selection: list[tuple[str, str]],
            out: Path, models: Path) -> dict[str, Any]:
    """Writes one batch of the enriched set, with every control of the sample.

    This is the loop of `export_crible_sample.main`, restricted to an explicit
    selection; the helpers all come from the production script.
    """
    from crible.batch import BatchWriter
    from tokenizers import Tokenizer

    definition = module.STRUCTURES[structure]
    corpus = definition["corpus"]
    model = json.loads((models / "mdenseon" / "model.json").read_text(encoding="utf-8"))
    tokenizer = Tokenizer.from_file(str(models / "mdenseon" / "tokenizer.json"))
    tokenizer.no_truncation()
    tokenizer.no_padding()
    # `preparer` only needs the tokenizer: the model is not loaded here, so the
    # split stays on the production code path without a 2 GB runtime.
    encoder = module.sync_vecteurs.Encodeur()
    encoder._moteur = (tokenizer, None, None, None)

    batch_id = f"legifacile-mes10-{structure}-v1"
    writer = BatchWriter(
        module.config_index(structure, model),
        out / "batches",
        batch_id,
        "upsert",
        model,
        module._chunker(corpus),
        module._gabarit(structure, corpus),
        created_at=DATE_LOT,
        host="legifacile-mes10",
    )

    stats: dict[str, Any] = {
        "structure": structure,
        "batch_id": batch_id,
        "documents": 0,
        "passages": 0,
        "hash_ok": 0,
        "hash_ecart": 0,
        "documents_ecartes": 0,
        "tokens_ecart": 0,
        "partie_ecart": 0,
        "ord_ecart": 0,
        "corpus": {name: {"documents": 0, "passages": 0} for name in corpus},
        "ecarts": [],
    }

    documents = module.flux_sql(env, module.sql_documents(structure, selection))
    passages = module.flux_sql(env, module.sql_passages(selection))
    pending = None
    try:
        for raw in documents:
            values = {}
            for key, value in zip(module.COLONNES_DOC, raw[1:]):
                values[key] = module._entier(value) if key == "annee" else module._texte(value)
            identity = (values["corpus"], values["src_id"])

            received = []
            while True:
                if pending is None:
                    pending = next(passages, None)
                    if pending is None:
                        break
                if (module._texte(pending[0]), module._texte(pending[1])) != identity:
                    break
                received.append(pending)
                pending = None
            if not received:
                stats["documents_ecartes"] += 1
                stats["ecarts"].append({"src_id": values["src_id"], "cause": "no passage"})
                continue

            package = module.sync_vecteurs.CORPUS[values["corpus"]]
            content, _ = package.composer(module.ligne_compositeur(structure, values))
            digest = module.sync_vecteurs._hash_contenu(content)
            if digest != values["src_hash"]:
                stats["hash_ecart"] += 1
                stats["documents_ecartes"] += 1
                stats["ecarts"].append({"src_id": values["src_id"], "cause": "src_hash differs"})
                continue
            stats["hash_ok"] += 1

            cuts = encoder.preparer(
                content,
                package.decoupeur,
                values["fonds"] if package.decoupeur else None,
                values["titre"] if package.decoupeur else None,
            )
            if len(cuts) != len(received):
                stats["ord_ecart"] += 1
                stats["documents_ecartes"] += 1
                stats["ecarts"].append(
                    {"src_id": values["src_id"], "cause": "passage count differs"}
                )
                continue

            batch = []
            divergent = False
            for position, ((text, tokens, partie), row) in enumerate(zip(cuts, received)):
                if module._entier(row[2]) != position:
                    stats["ord_ecart"] += 1
                    divergent = True
                if module._entier(row[3]) != tokens:
                    stats["tokens_ecart"] += 1
                    divergent = True
                base_partie = module._texte(row[4])
                if base_partie != partie:
                    stats["partie_ecart"] += 1
                    divergent = True
                passage = {"text": text, "vector": module._vecteur(row[5])}
                if "partie" in definition["passage_fields"]:
                    passage["partie"] = base_partie
                batch.append(passage)
            if divergent:
                stats["documents_ecartes"] += 1
                stats["ecarts"].append(
                    {"src_id": values["src_id"], "cause": "tokens/partie/ord differ"}
                )
                continue

            writer.add_document(
                doc_id=values["src_id"],
                doc_hash=values["src_hash"],
                fields=module.champs_document(structure, values),
                passages=batch,
            )
            stats["documents"] += 1
            stats["passages"] += len(batch)
            stats["corpus"][values["corpus"]]["documents"] += 1
            stats["corpus"][values["corpus"]]["passages"] += len(batch)
    finally:
        documents.close()

    orphans = 0
    for _ in passages:
        orphans += 1
    if orphans:
        stats["orphelins"] = orphans

    location = writer.close()
    relative = Path(location)
    try:
        stats["batch"] = relative.relative_to(ROOT).as_posix()
    except ValueError:
        stats["batch"] = relative.as_posix()
    journal = out / f"journal-{structure}.json"
    journal.write_text(json.dumps(stats, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    return stats


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sortie", type=Path, default=DEFAULT_OUT)
    parser.add_argument("--env", type=Path, default=ROOT / ".env.prod")
    parser.add_argument("--queries", type=Path, default=QUERIES)
    parser.add_argument("--sample", type=Path, default=SAMPLE)
    parser.add_argument("--models", type=Path, default=ROOT / "data" / "models")
    parser.add_argument("--dry-run", action="store_true")
    options = parser.parse_args()

    expected = expected_documents(options.queries)
    present = sample_documents(options.sample)
    absent = [document for document in expected if document["src_id"] not in present]
    print(
        f"{len(expected)} expected documents on the 57 labelled questions, "
        f"{len(expected) - len(absent)} already in the sample, {len(absent)} to extract",
        flush=True,
    )

    env = load_env(options.env)
    prod_repo = env.get("PROD_REPO")
    if not prod_repo:
        raise SystemExit("PROD_REPO is not set in the environment file")
    module = load_production(Path(prod_repo))
    module.verifier_fenetre(env)

    located = production_location(module, env, absent)
    missing = [document["src_id"] for document in absent if document["src_id"] not in located]
    if missing:
        raise SystemExit(f"{len(missing)} expected documents not found in production")

    grouped: dict[str, list[dict[str, Any]]] = {}
    for document in absent:
        corpus, count = located[document["src_id"]]
        structure = CORPUS_STRUCTURE.get(corpus)
        if structure is None:
            raise SystemExit(f"corpus '{corpus}' has no structure")
        document["corpus"] = corpus
        document["passages"] = count
        grouped.setdefault(structure, []).append(document)

    total_documents = sum(len(documents) for documents in grouped.values())
    total_passages = sum(
        document["passages"] for documents in grouped.values() for document in documents
    )
    if total_documents != len(absent):
        raise SystemExit("a document was lost while grouping by structure")
    print(
        f"{total_documents} documents to extract, {total_passages} passages: "
        + ", ".join(
            f"{structure} {len(documents)}"
            for structure, documents in sorted(grouped.items())
        ),
        flush=True,
    )
    if options.dry_run:
        for structure, documents in sorted(grouped.items()):
            detail = ", ".join(
                f"{document['src_id']} ({document['passages']})" for document in documents
            )
            print(f"  {structure}: {detail}", flush=True)
        return 0

    options.sortie.mkdir(parents=True, exist_ok=True)
    manifest = {
        "protocol": "ADR-0092",
        "date": DATE_LOT,
        "queries": options.queries.relative_to(ROOT).as_posix(),
        "documents": [],
        "structures": {},
    }
    for structure in sorted(grouped):
        documents = grouped[structure]
        selection = [(document["corpus"], document["src_id"]) for document in documents]
        stats = extract(module, env, structure, selection, options.sortie, options.models)
        print(
            f"{structure}: {stats['documents']} documents, {stats['passages']} passages, "
            f"hash {stats['hash_ok']} ok / {stats['hash_ecart']} differ, "
            f"{stats['documents_ecartes']} rejected",
            flush=True,
        )
        if stats["documents"] != len(documents):
            raise SystemExit(f"{structure}: an expected document was rejected, see the journal")
        manifest["structures"][structure] = {
            "documents": stats["documents"],
            "passages": stats["passages"],
            "batch": stats["batch"],
        }
        for document in documents:
            manifest["documents"].append(
                {
                    "src_id": document["src_id"],
                    "question": document["question"],
                    "corpus": document["corpus"],
                    "structure": structure,
                    "type": document["type"],
                    "passages": document["passages"],
                }
            )
    manifest["totals"] = {
        "documents": sum(entry["documents"] for entry in manifest["structures"].values()),
        "passages": sum(entry["passages"] for entry in manifest["structures"].values()),
    }
    (options.sortie / "extraction.json").write_text(
        json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    print(json.dumps(manifest["totals"]), flush=True)
    return 0


def load_env(path: Path) -> dict[str, str]:
    """`KEY=value` lines of `path`, as the production script reads them."""
    env: dict[str, str] = {}
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
            value = value[1:-1]
        env[key.strip()] = value
    return env


if __name__ == "__main__":
    raise SystemExit(main())
