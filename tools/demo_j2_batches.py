# SPDX-License-Identifier: Apache-2.0

"""Build the two rejected batches of the J2 demo.

``docs/jalons/J2-demo.md`` proves three points on the Legifacile sample: a text
query works, a batch whose vectors were encoded **without** the ``document: ``
prompt is refused by the acceptance check (INV-12), and a passage longer than
``max_tokens`` is refused before any write (INV-11). The good batch of the demo
is the sample lot itself (``data/legifacile-sample/batches/legifacile-sample-dole-v1``);
this script builds the two faulty ones from the same lot, deterministically:

=====================  =====================================================
``j2-wrong-prompt``    the first documents of the ``dole`` lot, with vectors
                       re-encoded by the model **without** the document
                       prompt, so the acceptance cosine falls to ~0.7
``j2-too-long``        one document whose only passage exceeds ``max_tokens``
                       (9 000 words), with a valid vector copied from the lot
=====================  =====================================================

Both batches are written with :class:`crible.batch.BatchWriter`, so they carry
a valid manifest, schema and checksums: the engine refuses them on their
content, not on their format. ``created_at`` and ``host`` are fixed, and the
tokens and vectors come from the pinned model, so two runs on the same model
write the same files.

Run from ``tools/`` with the ``export`` dependency group, which provides
``onnxruntime`` and ``tokenizers``:

```sh
uv sync --group export
uv run python demo_j2_batches.py \
  --sample ../data/legifacile-sample --models ../data/models --out ../data/j2-demo
```

The out directory must not hold either batch yet: the demo preamble removes
``data/j2-demo`` first. The script never deletes anything.
"""

from __future__ import annotations

import argparse
import json
import statistics
import sys
from pathlib import Path
from typing import Any

import blake3
import numpy as np
import pyarrow.parquet as pq

from crible.batch import BatchWriter

#: Upper bound on the documents of the wrong-prompt batch. It is a ceiling, not
#: the count: the passage cap below is reached first, so the ``dole`` lot yields
#: 5 documents and 20 passages.
MAX_WRONG_PROMPT_DOCUMENTS = 10

#: Passage cap of the wrong-prompt batch, so the acceptance check stays quick.
WRONG_PROMPT_PASSAGES = 20

#: Identifiers of the two batches written under ``--out``.
WRONG_PROMPT_BATCH = "j2-wrong-prompt"
TOO_LONG_BATCH = "j2-too-long"

#: Fixed manifest members, so two runs write the same bytes.
CREATED_AT = "2026-10-23T00:00:00Z"
HOST = "crible-j2-demo"

#: Document of the too-long batch: it exists only in that batch.
TOO_LONG_DOC_ID = "J2-TOO-LONG"

#: French words repeated to build the over-long passage.
TOO_LONG_WORD = "droit "


def select_documents(
    documents: list[dict[str, Any]],
    passages: list[dict[str, Any]],
    *,
    max_documents: int = MAX_WRONG_PROMPT_DOCUMENTS,
    max_passages: int = WRONG_PROMPT_PASSAGES,
) -> list[tuple[dict[str, Any], list[dict[str, Any]]]]:
    """Returns the first documents, by ``doc_id``, with their passages.

    The order is the lexicographic order of ``doc_id``, never the Parquet row
    order: the selection stays the same if a producer rewrites the files. A
    document without a passage is skipped, and the passage budget is a hard cap.
    """
    by_document: dict[str, list[dict[str, Any]]] = {}
    for passage in passages:
        by_document.setdefault(passage["doc_id"], []).append(passage)

    selected: list[tuple[dict[str, Any], list[dict[str, Any]]]] = []
    budget = max_passages
    for document in sorted(documents, key=lambda row: row["doc_id"]):
        if len(selected) >= max_documents or budget == 0:
            break
        own = sorted(by_document.get(document["doc_id"], []), key=lambda row: row["ord"])
        if not own:
            continue
        own = own[:budget]
        selected.append((document, own))
        budget -= len(own)
    return selected


def too_long_text(tokenizer: Any, prompt: str, max_tokens: int) -> tuple[str, int]:
    """Returns a text whose token count with ``prompt`` exceeds ``max_tokens``.

    The length is counted with the tokenizer of the model, special tokens
    included, exactly as control 8 of ``docs/04-donnees-et-ingestion.md`` counts
    it. The returned count is the counted value, so the caller prints what the
    engine will see.
    """
    repeats = max_tokens + 1_000
    for _ in range(100):
        text = TOO_LONG_WORD * repeats
        length = len(tokenizer.encode(prompt + text).ids)
        if length > max_tokens:
            return text, length
        repeats += 500
    raise RuntimeError(
        f"cannot build a text longer than {max_tokens} tokens with {TOO_LONG_WORD!r}"
    )


def verify_model_file(directory: Path, relative: str, expected: str) -> None:
    """Checks one file of the model against the blake3 of ``model.json``."""
    path = directory / relative
    digest = blake3.blake3(path.read_bytes()).hexdigest()
    if digest != expected:
        raise RuntimeError(
            f"{path} has blake3:{digest}, but model.json declares blake3:{expected}"
        )


def encode(
    session: Any, tokenizer: Any, prompt: str, text: str, max_tokens: int
) -> np.ndarray:
    """Encodes one text the way the engine does: prompt, CLS, L2 normalization.

    The graph renders the last hidden state; Crible applies the prompt, keeps
    the CLS row and normalizes (``docs/04-donnees-et-ingestion.md``). No
    truncation happens here: a text above ``max_tokens`` is an error.
    """
    ids = tokenizer.encode(prompt + text).ids
    if len(ids) > max_tokens:
        raise RuntimeError(
            f"a passage of {len(ids)} tokens exceeds max_tokens = {max_tokens}"
        )
    feeds = {
        "input_ids": np.array([ids], dtype=np.int64),
        "attention_mask": np.ones((1, len(ids)), dtype=np.int64),
    }
    hidden = session.run(None, feeds)[0]
    vector = hidden[:, 0, :].astype(np.float64)[0]
    vector /= np.linalg.norm(vector)
    return vector.astype(np.float32)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--sample", type=Path, required=True, help="data/legifacile-sample"
    )
    parser.add_argument(
        "--models", type=Path, required=True, help="models root holding mdenseon/"
    )
    parser.add_argument(
        "--out", type=Path, required=True, help="parent of the two demo batches"
    )
    args = parser.parse_args(argv)

    model_directory = args.models / "mdenseon"
    model = json.loads((model_directory / "model.json").read_text(encoding="utf-8"))
    verify_model_file(
        model_directory, model["files"]["fp32"]["path"], model["files"]["fp32"]["blake3"]
    )
    verify_model_file(
        model_directory,
        model["files"]["tokenizer"]["path"],
        model["files"]["tokenizer"]["blake3"],
    )

    batch = args.sample / "batches" / "legifacile-sample-dole-v1"
    manifest = json.loads((batch / "manifest.json").read_text(encoding="utf-8"))
    index_config = json.loads(
        (args.sample / "configs" / "dole.json").read_text(encoding="utf-8")
    )
    documents = pq.read_table(batch / "documents.parquet").to_pylist()
    passages = pq.read_table(batch / "passages-0001.parquet").to_pylist()

    for batch_id in (WRONG_PROMPT_BATCH, TOO_LONG_BATCH):
        if (args.out / batch_id).exists():
            raise SystemExit(
                f"{args.out / batch_id} already exists; remove {args.out} before "
                f"rerunning (the J2 demo preamble does)"
            )

    # Imports live here, so the module imports without the export group: the
    # pytest suite runs without ONNX Runtime (tools/README.md).
    import onnxruntime as ort
    from tokenizers import Tokenizer

    tokenizer = Tokenizer.from_file(
        str(model_directory / model["files"]["tokenizer"]["path"])
    )
    tokenizer.no_truncation()
    session = ort.InferenceSession(
        str(model_directory / model["files"]["fp32"]["path"]),
        providers=["CPUExecutionProvider"],
    )
    max_tokens = model["max_tokens"]
    document_prompt = model["prompts"]["document"]

    selected = select_documents(documents, passages)
    if not selected:
        raise RuntimeError(f"no document of {batch} has a passage")

    cosines: list[float] = []
    writer = BatchWriter(
        index_config,
        args.out,
        WRONG_PROMPT_BATCH,
        "upsert",
        model,
        manifest["chunker"],
        manifest["template"],
        created_at=CREATED_AT,
        host=HOST,
    )
    for document, own in selected:
        encoded = []
        for passage in own:
            without_prompt = encode(
                session, tokenizer, "", passage["text"], max_tokens
            )
            with_prompt = encode(
                session, tokenizer, document_prompt, passage["text"], max_tokens
            )
            cosines.append(float(np.dot(without_prompt, with_prompt)))
            encoded.append({"text": passage["text"], "vector": without_prompt})
        writer.add_document(
            document["doc_id"],
            document["doc_hash"],
            {name: document[name] for name in index_config["schema"]["document_fields"]},
            encoded,
        )
    writer.close()
    print(
        f"{WRONG_PROMPT_BATCH}: {len(selected)} documents, {len(cosines)} passages "
        f"-> {args.out / WRONG_PROMPT_BATCH}"
    )
    print(
        "  cosine with the 'document: ' prompt: "
        f"min {min(cosines):.4f}, median {statistics.median(cosines):.4f}; "
        f"the acceptance threshold is {index_config['ingest']['acceptance_min_cosine']}"
    )

    text, tokens = too_long_text(tokenizer, document_prompt, max_tokens)
    writer = BatchWriter(
        index_config,
        args.out,
        TOO_LONG_BATCH,
        "upsert",
        model,
        manifest["chunker"],
        manifest["template"],
        created_at=CREATED_AT,
        host=HOST,
    )
    # The vector is that of a real passage, so the batch carries a valid vector:
    # the refusal must come from the token count, not from control 7.
    writer.add_document(
        TOO_LONG_DOC_ID,
        blake3.blake3(text.encode("utf-8")).hexdigest(),
        {"annee": 2024, "type": "LOI_PUBLIEE"},
        [{"text": text, "vector": np.frombuffer(passages[0]["vector"], dtype="<f2")}],
    )
    writer.close()
    print(
        f"{TOO_LONG_BATCH}: 1 document, 1 passage, {tokens} tokens with the prompt "
        f"(max_tokens {max_tokens}) -> {args.out / TOO_LONG_BATCH}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
