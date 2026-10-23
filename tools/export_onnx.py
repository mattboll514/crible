# SPDX-License-Identifier: Apache-2.0

"""Export the mDenseOn encoder to ONNX for Crible.

The task ``EXPORTER-MODELE-ONNX`` (phase P2) asks for three files and a
description, written in one directory of ``data/models/mdenseon/`` (gitignored):

============================  ==================================================
``model.onnx``                fp32 export of the transformer, dynamic axes
``model.int8.onnx``           same graph, weights quantized to INT8 (dynamic)
``tokenizer.json``            Hugging Face fast tokenizer, copied as is
``model.json``                hashes, pinned revision, prompts, pooling, limits
============================  ==================================================

``model.json`` follows ``docs/04-donnees-et-ingestion.md`` ("Le modele"): the
encoder returns the last hidden state, Crible does the CLS pooling and the L2
normalization itself. The two prompts (``query: `` and ``document: ``) are
applied by the producer and the engine, never by the graph, so the graph stays
a plain transformer.

The graph is exported by ``torch.onnx.export`` under ``transformers==5.14.1``
(ADR-0050), the version that produced the production vectors of Legifacile,
with the sequence length left dynamic. The declared ``max_tokens`` (8192) is
checked twice: against the model config, and by running the exported graph on an
input of exactly that length.

Three acceptance criteria are checked here:

- reproducibility: two runs with the same revision write byte-identical
  ``model.onnx`` (see ``tools/README.md`` for the measured hashes);
- parity: on 100 deterministic French texts, the cosine between the PyTorch
  pooled vectors and the ONNX fp32 ones is at least 0.9999. The measured
  minimum and mean go to ``parity.json`` and to ``tools/README.md``;
- fidelity: on a deterministic draw of 100 passages per structure of the
  Legifacile sample, the cosine between the ONNX fp32 vectors, document prompt
  included, and the production vectors is at least 0.999 (INV-12). It is
  measured when the export is given ``--fidelity-sample`` and recorded in
  ``parity.json``.

``model.json`` also records the versions of ``transformers``, ``torch`` and
``onnxruntime`` of the run, in an ``export`` field that stays out of the model
fingerprint (ADR-0050).

``parity.json`` is a fifth file, a report for humans: ``docs/04`` describes four
files, and the engine ignores anything else in the model directory.

The heavy imports (``torch``, ``transformers``, ``tokenizers``,
``huggingface_hub``, ``onnxruntime``, ``onnx``) happen inside the functions that
need them, so importing this module stays cheap and offline: the pytest suite
imports it without downloading a model. They come from the ``export`` dependency
group of ``tools/pyproject.toml``, not from the default environment.
"""

from __future__ import annotations

import argparse
import json
import os
import random
import shutil
import sys
import tempfile
import time
from pathlib import Path

import blake3
import numpy as np

#: Directory name of the model, matching ``"name"`` in ``model.json``.
MODEL_NAME = "mdenseon"
#: Hugging Face repository of the encoder.
HF_ID = "lightonai/mDenseOn"
#: Commit SHA the export is pinned to (``lightonai/mDenseOn``, 2026-07-30).
DEFAULT_REVISION = "a5fdb000f7a21da96c3bddde3a782ef777316df3"
#: Width of the pooled vector.
DIMS = 768
#: Pooling of the model card: the ``[CLS]`` token.
POOLING = "cls"
#: The model card asks for cosine similarity, so vectors are L2 normalized.
NORMALIZE = True
#: Longest input the export accepts, in tokens, prompts included.
MAX_TOKENS = 8192
#: Prompts of ``config_sentence_transformers.json``, trailing space included.
PROMPTS = {"query": "query: ", "document": "document: "}

FP32_PATH = "model.onnx"
INT8_PATH = "model.int8.onnx"
TOKENIZER_PATH = "tokenizer.json"
MODEL_JSON_PATH = "model.json"
PARITY_PATH = "parity.json"

#: Every file one export writes. ``parity.json`` is a report for humans, not a
#: model file: the engine reads the four files of ``docs/04`` and ignores the
#: rest.
EXPORT_FILES = (FP32_PATH, INT8_PATH, TOKENIZER_PATH, MODEL_JSON_PATH, PARITY_PATH)

#: Name of the SentenceTransformers configuration, source of the prompts.
ST_CONFIG_PATH = "config_sentence_transformers.json"

#: Sequence length used to trace the graph. Dynamic axes make it irrelevant to
#: the exported shape, but the traced sample changes the file, so it is fixed
#: to keep the fp32 hash stable.
TRACE_SEQUENCE_LENGTH = 32
#: Seed of the token ids used to trace the graph, fixed for the same reason.
TRACE_SEED = 20260222
#: ONNX opset of the exported graph. The ONNX Runtime of the engine reads it.
ONNX_OPSET = 18
#: Number of texts of the PyTorch vs ONNX parity check.
PARITY_TEXT_COUNT = 100
#: Seed of the parity corpus. Only ``random.random()`` draws are used, a method
#: whose stream is guaranteed stable across Python versions.
_CORPUS_SEED = 20260222
#: Minimum cosine required by the task between PyTorch and ONNX fp32.
MIN_PARITY_COSINE = 0.9999
#: Minimum cosine between the ONNX fp32 vectors and the production vectors,
#: the acceptance threshold of ``docs/04`` (control 9, INV-12).
MIN_FIDELITY_COSINE = 0.999
#: Passages drawn from each structure of the Legifacile sample for criterion
#: (ii). ADR-0050 fixes the draw at 100 passages per structure.
FIDELITY_PER_STRUCTURE = 100
#: Versions of the export run recorded in ``model.json`` (ADR-0050, outside the
#: fingerprint of ADR-0028).
EXPORT_VERSION_KEYS = ("transformers", "torch", "onnxruntime")
#: Prefix and suffix of a structure batch of the Legifacile sample.
SAMPLE_BATCH_PREFIX = "legifacile-sample-"
SAMPLE_BATCH_SUFFIX = "-v1"
#: Truncation limit used to measure the real length of an input. Far above any
#: passage of the sample, so the count is the untruncated one.
UNLIMITED_TOKENS = 1 << 20
#: Target length of the longest comparison text, in characters. French legal
#: prose runs at about 4.5 characters per token, so this lands close to the
#: 8192-token limit without depending on the tokenizer.
LONG_TEXT_CHARS = 36000

#: French sentences used by the parity corpus. They are generic, written for
#: this test, and never come from the production query set.
_SENTENCES = (
    "Le Conseil d'État rappelle que la décision attaquée a été prise par "
    "l'autorité compétente.",
    "La demande de renouvellement doit être déposée deux mois avant "
    "l'expiration du titre de séjour.",
    "Les dispositions de l'article L. 561-2 du code de l'entrée et du séjour "
    "des étrangers s'appliquent au présent litige.",
    "Le préfet fait valoir que l'intéressé ne justifie d'aucune attache "
    "personnelle ou familiale en France.",
    "La cour administrative d'appel a confirmé le jugement de première "
    "instance en toutes ses dispositions.",
    "Le requérant soutient que la mesure contestée porte une atteinte "
    "excessive à sa vie privée et familiale.",
    "Le tribunal de grande instance de Paris était compétent pour connaître "
    "du litige relatif au contrat de bail commercial.",
    "La clause de non-concurrence est réputée non écrite lorsqu'elle n'est "
    "pas limitée dans le temps et dans l'espace.",
    "L'employeur doit convoquer le salarié à un entretien préalable avant "
    "tout licenciement pour motif personnel.",
    "Le délai de prescription de l'action en paiement des salaires est de "
    "trois ans à compter du jour où la créance est née.",
    "La société requérante invoque une rupture d'égalité entre les "
    "contribuables placés dans une situation identique.",
    "Le service des impôts des entreprises a notifié une proposition de "
    "rectification au titre de l'exercice clos.",
    "Les pièces produites au dossier ne permettent pas d'établir la réalité "
    "des prestations facturées.",
    "La décision de la commission départementale d'aide sociale est motivée "
    "au regard des ressources du foyer.",
    "Le maire peut, par arrêté motivé, interdire l'accès à un immeuble "
    "menaçant ruine.",
    "L'autorisation d'urbanisme est délivrée sous réserve du respect des "
    "règles du plan local d'urbanisme.",
    "Le recours gracieux formé dans le délai de deux mois conserve le délai "
    "de recours contentieux.",
    "La responsabilité de la commune est engagée pour défaut d'entretien "
    "normal de l'ouvrage public.",
    "Le juge des référés peut ordonner toute mesure utile avant tout procès "
    "si l'urgence le justifie.",
    "Les honoraires de l'avocat sont fixés en fonction de la difficulté de "
    "l'affaire et du temps consacré.",
    "La convention collective nationale prévoit une prime annuelle versée "
    "avec le salaire du mois de décembre.",
    "Le bailleur doit remettre au locataire un logement décent, ne "
    "comportant pas de risque pour la santé.",
    "La cession de parts sociales consentie sans agrément des associés est "
    "inopposable à la société.",
    "Le syndic convoque l'assemblée générale des copropriétaires au moins "
    "vingt et un jours avant la date fixée.",
    "Les marchés publics passés selon une procédure adaptée sont soumis aux "
    "principes de liberté d'accès et d'égalité de traitement.",
    "La collectivité territoriale justifie de l'intérêt local à soutenir "
    "une opération d'aménagement.",
    "Le retrait d'une décision créatrice de droits n'est possible que dans "
    "un délai de quatre mois.",
    "Le rapport d'expertise judiciaire conclut à un taux d'incapacité "
    "permanente partielle de douze pour cent.",
)


def blake3_file(path: str | Path, chunk_size: int = 1 << 20) -> str:
    """Returns the hexadecimal BLAKE3 digest of a file, read by chunks."""
    digest = blake3.blake3()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(chunk_size), b""):
            digest.update(chunk)
    return digest.hexdigest()


def build_model_json(
    revision: str,
    fp32_hash: str,
    int8_hash: str,
    tokenizer_hash: str,
    max_tokens: int = MAX_TOKENS,
    export: dict | None = None,
) -> dict:
    """Builds the ``model.json`` payload described by ``docs/04``.

    The INT8 entry carries a hash but stays out of the model fingerprint, which
    covers ``hf_id``, ``revision``, ``dims``, ``pooling``, ``normalize``,
    ``prompts`` and the hashes of the fp32 and tokenizer files. The fingerprint
    itself is computed by the engine, not here.

    ``export`` is the mapping returned by :func:`export_versions`. It is
    recorded for diagnosis only and stays out of the fingerprint (ADR-0050): a
    ``model.json`` written before that ADR, without the field, remains valid.
    """
    payload = {
        "name": MODEL_NAME,
        "kind": "onnx",
        "hf_id": HF_ID,
        "revision": revision,
        "dims": DIMS,
        "pooling": POOLING,
        "normalize": NORMALIZE,
        "max_tokens": max_tokens,
        "prompts": dict(PROMPTS),
        "files": {
            "fp32": {"path": FP32_PATH, "blake3": fp32_hash},
            "int8": {"path": INT8_PATH, "blake3": int8_hash},
            "tokenizer": {"path": TOKENIZER_PATH, "blake3": tokenizer_hash},
        },
    }
    if export is not None:
        payload["export"] = check_export_versions(export)
    return payload


def check_export_versions(versions: dict) -> dict:
    """Returns the three versions of the export, refusing a partial mapping."""
    missing = [key for key in EXPORT_VERSION_KEYS if not versions.get(key)]
    unknown = sorted(set(versions) - set(EXPORT_VERSION_KEYS))
    if missing:
        raise ValueError(f"export versions are missing {', '.join(missing)}")
    if unknown:
        raise ValueError(f"export versions hold unknown keys {', '.join(unknown)}")
    return {key: str(versions[key]) for key in EXPORT_VERSION_KEYS}


def export_versions() -> dict:
    """Returns the versions of the libraries that produce the fp32 graph."""
    from importlib.metadata import PackageNotFoundError, version

    versions = {}
    for name in EXPORT_VERSION_KEYS:
        try:
            versions[name] = version(name)
        except PackageNotFoundError as error:
            raise ValueError(
                f"{name} is not installed: install the export group with "
                f"`uv sync --group export`"
            ) from error
    return versions


def _atomic_write(path: str | Path, text: str) -> None:
    """Writes ``text`` to ``path`` through a temporary file in the same folder.

    A crash mid-write leaves the previous file, never a truncated one: a
    half-written ``model.json`` would announce hashes that do not match.
    """
    target = Path(path)
    descriptor, temporary = tempfile.mkstemp(
        dir=target.parent, prefix=f".{target.name}.", suffix=".tmp"
    )
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
            stream.write(text)
        # `mkstemp` creates a file only its owner can read; a model directory is
        # meant to be read by the engine, possibly as another user.
        Path(temporary).chmod(0o644)
        os.replace(temporary, target)
    except BaseException:
        Path(temporary).unlink(missing_ok=True)
        raise


def write_model_json(path: str | Path, payload: dict) -> None:
    """Writes ``payload`` as JSON with sorted keys, atomically.

    The file is byte-stable, so a diff shows a model change and not a key
    reordering. Used for ``model.json`` and for the ``parity.json`` report.
    """
    text = json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)
    _atomic_write(path, text + "\n")


def comparison_texts() -> list[str]:
    """Returns the 100 deterministic French texts of the parity check.

    The composition draws only ``random.random()`` floats from one fixed seed.
    That method is guaranteed stable across Python versions, so the corpus is
    identical everywhere; the last text is a long repetition close to the token
    limit, which exercises the length check of INV-11 rather than a truncation.
    """
    rng = random.Random(_CORPUS_SEED)
    texts = []
    seen = set()
    while len(texts) < PARITY_TEXT_COUNT - 1:
        count = 1 + int(rng.random() * 12)
        picked = [_SENTENCES[int(rng.random() * len(_SENTENCES))] for _ in range(count)]
        text = " ".join(picked)
        # Redraw a collision, still from the same stream, so the corpus keeps
        # 100 distinct texts without another source of randomness.
        if text in seen:
            continue
        seen.add(text)
        texts.append(text)
    texts.append(long_text())
    return texts


def long_text(target_chars: int = LONG_TEXT_CHARS) -> str:
    """Repeats the first sentences until the text reaches ``target_chars``."""
    paragraph = " ".join(_SENTENCES[:6])
    repeat = target_chars // len(paragraph) + 1
    return (paragraph + " ") * repeat


def token_count(tokenizer, text: str, prompt: str) -> int:
    """Returns the untruncated number of tokens of ``prompt + text``.

    The tokenizer object carries a truncation limit; it is raised far above any
    real input for the count, so the result is the real length and not the
    limit. Callers that then encode with their own limit know whether it
    truncates.
    """
    tokenizer.enable_truncation(max_length=UNLIMITED_TOKENS, strategy="longest_first")
    count = len(tokenizer.encode(prompt + text).ids)
    if count >= UNLIMITED_TOKENS:
        raise ValueError(
            f"input longer than the {UNLIMITED_TOKENS}-token measurement limit"
        )
    return count


def encode_tokens(
    tokenizer, text: str, prompt: str, max_tokens: int = MAX_TOKENS
) -> list[int]:
    """Encodes ``prompt + text`` within ``max_tokens`` tokens.

    Truncation is never silent (INV-11): an input above the limit raises instead
    of losing its tail, so a model that cannot hold the text stops the export or
    the measurement. The engine has its own truncation path, which reports
    ``truncated: true``.
    """
    count = token_count(tokenizer, text, prompt)
    if count > max_tokens:
        raise ValueError(
            f"input needs {count} tokens, above the {max_tokens}-token limit; "
            f"refusing to truncate it silently (INV-11)"
        )
    tokenizer.enable_truncation(max_length=max_tokens, strategy="longest_first")
    return tokenizer.encode(prompt + text).ids


def normalize(vector: np.ndarray) -> np.ndarray:
    """Returns ``vector`` scaled to unit L2 norm, as the model card asks."""
    array = np.asarray(vector, dtype=np.float32)
    norm = float(np.linalg.norm(array))
    if norm == 0.0:
        raise ValueError("cannot normalize a zero vector")
    return array / norm


def cls_pool(hidden: np.ndarray) -> np.ndarray:
    """Takes the ``[CLS]`` row (position 0) of a ``[1, tokens, dims]`` output."""
    array = np.asarray(hidden, dtype=np.float32)
    if array.ndim != 3 or array.shape[0] != 1:
        raise ValueError(f"expected a [1, tokens, dims] output, got {array.shape}")
    return array[0, 0]


def cosine(left: np.ndarray, right: np.ndarray) -> float:
    """Returns the cosine of two vectors, both already L2 normalized or not."""
    a = np.asarray(left, dtype=np.float64)
    b = np.asarray(right, dtype=np.float64)
    denominator = float(np.linalg.norm(a) * np.linalg.norm(b))
    if denominator == 0.0:
        raise ValueError("cannot compare a zero vector")
    return float(np.dot(a, b) / denominator)


def prepare_output_dir(directory: str | Path, overwrite: bool) -> Path:
    """Creates the output directory, refusing to mix with foreign files.

    Without ``overwrite`` a non-empty directory is an error, as in
    ``gen_synth.py``: the model files are large and a silent mix-up would make
    the hashes in ``model.json`` lie. Nothing is deleted here: an export that
    fails during the download leaves the previous one untouched, and
    :func:`clear_manifest` drops the outdated manifest once the new heavy files
    are in place.
    """
    path = Path(directory)
    if not path.exists():
        path.mkdir(parents=True)
        return path
    if overwrite:
        return path
    if any(path.iterdir()):
        raise FileExistsError(
            f"output directory '{path}' is not empty; remove it, or pass "
            f"--overwrite to replace the files this export writes "
            f"({', '.join(EXPORT_FILES)})"
        )
    return path


def clear_manifest(directory: str | Path) -> None:
    """Removes ``model.json`` and ``parity.json`` of a previous export.

    They describe one run only: a new export must not sit next to older hashes,
    nor next to an old ``parity.json`` when it is relaunched with
    ``--skip-parity``. The heavy files are already rewritten when this runs, so
    a failure before that point keeps the whole previous export.
    """
    path = Path(directory)
    for name in (MODEL_JSON_PATH, PARITY_PATH):
        (path / name).unlink(missing_ok=True)


def resolve_revision(revision: str) -> str:
    """Resolves a branch, tag or SHA to the commit SHA actually exported."""
    from huggingface_hub import HfApi

    return HfApi().model_info(HF_ID, revision=revision).sha


def download_snapshot(revision: str) -> Path:
    """Downloads the pinned snapshot into the Hugging Face cache."""
    from huggingface_hub import snapshot_download

    path = snapshot_download(
        HF_ID, revision=revision, allow_patterns=["*.json", "*.safetensors"]
    )
    return Path(path)


def check_model_config(snapshot: Path, max_tokens: int = MAX_TOKENS) -> None:
    """Checks the downloaded files against the values declared in ``model.json``.

    A disagreement with ``docs/04`` stops the export instead of producing a
    ``model.json`` that lies about the model. The prompts are read from
    ``config_sentence_transformers.json``, so a different ``--revision`` cannot
    slip through with other prompts.
    """
    config = json.loads((snapshot / "config.json").read_text(encoding="utf-8"))
    if config.get("hidden_size") != DIMS:
        raise ValueError(
            f"{HF_ID}: hidden_size is {config.get('hidden_size')}, expected {DIMS}"
        )
    if config.get("max_position_embeddings") != max_tokens:
        raise ValueError(
            f"{HF_ID}: max_position_embeddings is "
            f"{config.get('max_position_embeddings')}, expected {max_tokens}"
        )
    st_config = json.loads((snapshot / ST_CONFIG_PATH).read_text(encoding="utf-8"))
    if st_config.get("prompts") != PROMPTS:
        raise ValueError(
            f"{HF_ID}: prompts are {st_config.get('prompts')}, expected {PROMPTS}"
        )
    tokenizer = json.loads((snapshot / TOKENIZER_PATH).read_text(encoding="utf-8"))
    declared = tokenizer.get("truncation", {}).get("max_length")
    if declared != max_tokens:
        raise ValueError(
            f"{HF_ID}: tokenizer truncation is {declared}, expected {max_tokens}"
        )


def clear_node_metadata(model) -> None:
    """Drops the per-node debug metadata of an ONNX model.

    ``torch.onnx.export`` records a stack trace per node, and that trace names
    a module generated by a counter that changes at every run
    (``<eval_with_key>.68``, ``.145``…). The graphs are otherwise identical, so
    those traces alone made the fp32 hash differ between two exports of the same
    revision. They serve no purpose for Crible, which runs the graph as is.
    """
    for node in model.graph.node:
        node.metadata_props.clear()


def write_reproducible_graph(source: Path, target: Path) -> None:
    """Rewrites ``source`` into ``target`` without the run-specific metadata."""
    import onnx

    model = onnx.load(str(source))
    clear_node_metadata(model)
    onnx.save(model, str(target))


def export_fp32(snapshot: Path, target: Path, revision: str) -> None:
    """Exports the transformer to fp32 ONNX with dynamic batch and length axes.

    ``torch.onnx.export`` replaces ``optimum`` (ADR-0050). The wrapper returns
    the last hidden state: the graph applies neither the prompt nor the pooling,
    which Crible does itself. The weights are inline, so the graph is one file.

    ``revision`` is the pinned commit SHA the snapshot was resolved from. The
    export does not need it to trace the graph, but the step is the only one
    that produces bytes tied to a revision, and the caller of this step (and its
    tests) names it: dropping it would make the step silent about what it
    exported. It stays for that reason, not for a computation.
    """
    import torch
    from transformers import AutoModel

    class Encoder(torch.nn.Module):
        """Returns ``last_hidden_state`` of the wrapped transformer."""

        def __init__(self, model):
            super().__init__()
            self.model = model

        def forward(self, input_ids, attention_mask):
            return self.model(
                input_ids=input_ids, attention_mask=attention_mask
            ).last_hidden_state

    model = AutoModel.from_pretrained(str(snapshot), dtype=torch.float32).eval()
    encoder = Encoder(model).eval()
    generator = torch.Generator().manual_seed(TRACE_SEED)
    input_ids = torch.randint(
        0,
        # Any id below the vocabulary traces the same graph; small ids keep the
        # sample readable in a debugger.
        1000,
        (1, TRACE_SEQUENCE_LENGTH),
        generator=generator,
        dtype=torch.long,
    )
    attention_mask = torch.ones_like(input_ids)

    with tempfile.TemporaryDirectory(prefix="crible-onnx-") as work:
        traced = Path(work) / FP32_PATH
        torch.onnx.export(
            encoder,
            (input_ids, attention_mask),
            str(traced),
            input_names=["input_ids", "attention_mask"],
            output_names=["last_hidden_state"],
            # Both inputs share their two axes at run time; separate names keep
            # each axis dynamic without pretending the lengths differ.
            dynamic_shapes=(
                {0: "batch_ids", 1: "sequence_ids"},
                {0: "batch_mask", 1: "sequence_mask"},
            ),
            opset_version=ONNX_OPSET,
            external_data=False,
            do_constant_folding=True,
        )
        write_reproducible_graph(traced, target)


def quantize_int8(source: Path, target: Path) -> None:
    """Quantizes the weights to INT8 with the dynamic scheme of ONNX Runtime."""
    from onnxruntime.quantization import QuantType, quantize_dynamic

    quantize_dynamic(str(source), str(target), weight_type=QuantType.QInt8)


def load_tokenizer(path: Path):
    """Loads the exported ``tokenizer.json`` with the ``tokenizers`` library."""
    from tokenizers import Tokenizer

    return Tokenizer.from_file(str(path))


def torch_vectors(snapshot: Path, tokenizer, texts: list[str]) -> list[np.ndarray]:
    """Returns the PyTorch reference vectors: CLS pooling, then L2 norm."""
    import torch
    from transformers import AutoModel

    model = AutoModel.from_pretrained(str(snapshot), dtype=torch.float32).eval()
    vectors = []
    with torch.inference_mode():
        for text in texts:
            ids = torch.tensor([encode_tokens(tokenizer, text, PROMPTS["document"])])
            hidden = model(
                input_ids=ids, attention_mask=torch.ones_like(ids)
            ).last_hidden_state
            vectors.append(normalize(cls_pool(hidden.numpy())))
    return vectors


def onnx_vectors(model_path: Path, tokenizer, texts: list[str]) -> list[np.ndarray]:
    """Returns the ONNX vectors, pooled and normalized the same way."""
    import onnxruntime as ort

    session = ort.InferenceSession(str(model_path), providers=["CPUExecutionProvider"])
    vectors = []
    for text in texts:
        ids = np.asarray([encode_tokens(tokenizer, text, PROMPTS["document"])])
        hidden = session.run(
            None,
            {
                "input_ids": ids.astype(np.int64),
                "attention_mask": np.ones_like(ids, dtype=np.int64),
            },
        )[0]
        vectors.append(normalize(cls_pool(hidden)))
    return vectors


def batch_directories(sample_dir: str | Path) -> dict[str, Path]:
    """Returns ``structure -> batch directory`` of a Legifacile sample.

    The layout is the one of ``data/legifacile-sample/``: one directory per
    structure under ``batches/``, named
    ``legifacile-sample-<structure>-v1`` (``data/legifacile-sample/README.md``).
    """
    batches = Path(sample_dir) / "batches"
    if not batches.is_dir():
        raise ValueError(f"no 'batches' directory under {sample_dir}")
    found = {}
    for entry in sorted(batches.iterdir()):
        name = entry.name
        if not entry.is_dir():
            continue
        if not (name.startswith(SAMPLE_BATCH_PREFIX) and name.endswith(SAMPLE_BATCH_SUFFIX)):
            continue
        structure = name[len(SAMPLE_BATCH_PREFIX) : len(name) - len(SAMPLE_BATCH_SUFFIX)]
        if structure:
            found[structure] = entry
    if not found:
        raise ValueError(
            f"no '{SAMPLE_BATCH_PREFIX}*{SAMPLE_BATCH_SUFFIX}' batch under {batches}"
        )
    return found


def passage_files(batch: Path) -> list[Path]:
    """Returns the ``passages-*.parquet`` files of a batch, in file order."""
    files = sorted(batch.glob("passages-*.parquet"))
    if not files:
        raise ValueError(f"no passages-*.parquet in {batch}")
    return files


def passage_key(doc_id: str, order: int) -> str:
    """Returns the passage key of ``docs/04``: ``doc_id#ord``."""
    return f"{doc_id}#{order}"


def _draw_digest(key: str) -> bytes:
    return blake3.blake3(key.encode("utf-8")).digest()


def stored_vector(raw: bytes) -> np.ndarray:
    """Decodes one stored vector: little-endian fp16, widened to float32."""
    array = np.frombuffer(raw, dtype="<f2")
    if array.size != DIMS:
        raise ValueError(
            f"stored vector holds {array.size} components, expected {DIMS}"
        )
    return array.astype(np.float32)


def sample_passages(
    sample_dir: str | Path, per_structure: int = FIDELITY_PER_STRUCTURE
) -> list[dict]:
    """Draws ``per_structure`` passages from every structure of the sample.

    The draw is deterministic and independent of the file layout: passages are
    ordered by the BLAKE3 digest of their key, then the first
    ``per_structure`` are kept. Two samples holding the same passages in another
    row order, or split into other files, therefore give the same draw. The
    vectors are the production ones, read from the batch, never re-encoded.

    Returns one mapping per passage, with ``structure``, ``key``, ``text`` and
    ``vector`` (float32, ``dims`` components).
    """
    import pyarrow.parquet as pq

    records = []
    for structure, batch in batch_directories(sample_dir).items():
        files = passage_files(batch)
        candidates = []
        for file_index, path in enumerate(files):
            keys = pq.read_table(path, columns=["doc_id", "ord"])
            for row, (doc_id, order) in enumerate(
                zip(keys["doc_id"].to_pylist(), keys["ord"].to_pylist(), strict=True)
            ):
                key = passage_key(doc_id, int(order))
                candidates.append((_draw_digest(key), key, file_index, row))
        if len(candidates) < per_structure:
            raise ValueError(
                f"structure '{structure}' holds {len(candidates)} passages, fewer "
                f"than the {per_structure} requested"
            )
        candidates.sort()

        wanted: dict[int, dict[int, str]] = {}
        for _, key, file_index, row in candidates[:per_structure]:
            wanted.setdefault(file_index, {})[row] = key
        kept = {}
        for file_index, rows in wanted.items():
            table = pq.read_table(
                files[file_index], columns=["text", "vector"]
            )
            for row, key in rows.items():
                kept[key] = {
                    "structure": structure,
                    "key": key,
                    "text": table["text"][row].as_py(),
                    "vector": stored_vector(table["vector"][row].as_py()),
                }
        # Emit in draw order, so the record order does not depend on which file
        # a passage lives in either.
        records.extend(kept[key] for _, key, _, _ in candidates[:per_structure])
    return records


def fidelity_report(cosines: list[float]) -> dict:
    """Summarizes the cosines against the production vectors (INV-12).

    Raises ``ValueError`` as soon as one cosine is under
    :data:`MIN_FIDELITY_COSINE`: the acceptance of ``docs/04`` refuses such a
    lot, so the export must not announce a conforming model.
    """
    values = np.asarray(cosines, dtype=np.float64)
    if values.size == 0:
        raise ValueError("no cosine to report")
    minimum = float(values.min())
    if minimum < MIN_FIDELITY_COSINE:
        raise ValueError(
            f"fidelity below the acceptance threshold: minimum cosine {minimum} "
            f"< {MIN_FIDELITY_COSINE}"
        )
    return {
        "passages": int(values.size),
        "min_cosine": minimum,
        "p1_cosine": float(np.percentile(values, 1)),
        "median_cosine": float(np.median(values)),
        "min_cosine_required": MIN_FIDELITY_COSINE,
    }


def check_sample_provenance(sample_dir: str | Path, revision: str) -> dict:
    """Checks that the sample vectors come from the pinned model definition.

    Criterion (ii) is only meaningful if the vectors were produced by the
    reference model, with the document prompt, and not by an ONNX graph of
    Crible: a sample re-encoded by the new graph would compare it to itself, the
    very blindness ADR-0050 denounces. Every batch manifest declares its model
    (``model.revision``, ``model.prompt_document``, ``model.dims``,
    ``model.vector_dtype``), so a disagreement stops the measurement instead of
    producing a flattering cosine.

    Returns the common declaration, as read from the manifests.
    """
    declared = None
    for structure, batch in batch_directories(sample_dir).items():
        manifest = json.loads(
            (batch / "manifest.json").read_text(encoding="utf-8")
        )
        model = manifest.get("model")
        if not isinstance(model, dict):
            raise ValueError(f"batch '{structure}' declares no model in its manifest")
        entry = {
            "hf_id": model.get("hf_id"),
            "revision": model.get("revision"),
            "dims": model.get("dims"),
            "prompt_document": model.get("prompt_document"),
            "vector_dtype": model.get("vector_dtype"),
        }
        if entry["hf_id"] != HF_ID:
            raise ValueError(
                f"batch '{structure}' was encoded by '{entry['hf_id']}', "
                f"not by '{HF_ID}'"
            )
        if entry["revision"] != revision:
            raise ValueError(
                f"batch '{structure}' was encoded at revision "
                f"'{entry['revision']}', not at '{revision}'"
            )
        if entry["dims"] != DIMS:
            raise ValueError(
                f"batch '{structure}' declares {entry['dims']} dimensions, "
                f"not {DIMS}"
            )
        if entry["prompt_document"] != PROMPTS["document"]:
            raise ValueError(
                f"batch '{structure}' was encoded with prompt "
                f"{entry['prompt_document']!r}, not {PROMPTS['document']!r}"
            )
        if declared is None:
            declared = entry
        elif entry != declared:
            raise ValueError(
                f"batch '{structure}' declares another model than the other "
                f"structures of the sample"
            )
    return declared or {}


def measure_fidelity(
    sample_dir: str | Path,
    fp32_path: Path,
    tokenizer,
    revision: str,
    per_structure: int = FIDELITY_PER_STRUCTURE,
) -> dict:
    """Compares the ONNX fp32 vectors, prompt included, to production ones.

    This is criterion (ii) of ADR-0050: the graph is applied to the stored
    passage texts with ``prompt_document``, and the cosines are taken against the
    vectors the production pipeline wrote in the batch, after checking that
    those vectors come from the pinned model definition
    (:func:`check_sample_provenance`).
    """
    declared = check_sample_provenance(sample_dir, revision)
    records = sample_passages(sample_dir, per_structure)
    exported = onnx_vectors(fp32_path, tokenizer, [record["text"] for record in records])
    cosines = [
        cosine(vector, record["vector"])
        for vector, record in zip(exported, records, strict=True)
    ]
    report = fidelity_report(cosines)
    # Resolved, so the report does not depend on the directory it was run from.
    report["sample"] = str(Path(sample_dir).resolve())
    report["per_structure"] = per_structure
    report["structures"] = sorted({record["structure"] for record in records})
    report["manifest"] = declared
    return report


def verify_max_tokens(model_path: Path, max_tokens: int = MAX_TOKENS) -> float:
    """Runs the exported graph on exactly ``max_tokens`` tokens.

    Returns the elapsed seconds. Any failure propagates: a graph that cannot
    take the declared length must not be shipped with that ``max_tokens``.
    """
    import onnxruntime as ort

    session = ort.InferenceSession(str(model_path), providers=["CPUExecutionProvider"])
    ids = np.ones((1, max_tokens), dtype=np.int64)
    ids[0, 0] = 2
    ids[0, -1] = 1
    started = time.monotonic()
    output = session.run(
        None, {"input_ids": ids, "attention_mask": np.ones_like(ids)}
    )[0]
    elapsed = time.monotonic() - started
    if output.shape != (1, max_tokens, DIMS):
        raise ValueError(
            f"unexpected output shape {output.shape} at {max_tokens} tokens"
        )
    return elapsed


def run_parity(snapshot: Path, fp32_path: Path, tokenizer) -> dict:
    """Compares PyTorch and ONNX fp32 vectors on the deterministic corpus."""
    corpus = comparison_texts()
    reference = torch_vectors(snapshot, tokenizer, corpus)
    exported = onnx_vectors(fp32_path, tokenizer, corpus)
    similarities = [cosine(a, b) for a, b in zip(reference, exported, strict=True)]
    # INV-11: the length is measured without truncation. A text above the
    # declared limit raises here instead of being silently shortened, which
    # would compare two vectors of different inputs and hide the overflow.
    token_lengths = [
        token_count(tokenizer, text, PROMPTS["document"]) for text in corpus
    ]
    if max(token_lengths) > MAX_TOKENS:
        raise ValueError(
            f"a parity text needs {max(token_lengths)} tokens, above the "
            f"declared {MAX_TOKENS}; the corpus must stay within the limit"
        )
    return {
        "precision": "fp32",
        "texts": len(corpus),
        "min_cosine": min(similarities),
        "mean_cosine": float(np.mean(similarities)),
        "min_cosine_required": MIN_PARITY_COSINE,
        "token_lengths": {
            "min": min(token_lengths),
            "max": max(token_lengths),
            "limit": MAX_TOKENS,
        },
    }


def _parse_args(argv: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Export the mDenseOn encoder to ONNX for Crible."
    )
    parser.add_argument(
        "--out",
        default="../data/models/mdenseon",
        help="output directory (default: %(default)s, gitignored)",
    )
    parser.add_argument(
        "--revision",
        default=DEFAULT_REVISION,
        help="Hugging Face revision, resolved to a commit SHA (default: pinned)",
    )
    parser.add_argument(
        "--overwrite",
        action="store_true",
        help="replace the model.json and parity.json of a previous export",
    )
    parser.add_argument(
        "--skip-parity",
        action="store_true",
        help="skip the PyTorch vs ONNX parity check and the 8192-token run",
    )
    parser.add_argument(
        "--fidelity-sample",
        default=None,
        help=(
            "Legifacile sample directory; measures criterion (ii), the cosine "
            "against its production vectors, and records it in parity.json"
        ),
    )
    return parser.parse_args(argv)


def export(
    out_dir: str | Path,
    revision: str,
    overwrite: bool,
    parity: bool,
    fidelity_sample: str | Path | None = None,
) -> dict:
    """Runs the whole export and returns the ``model.json`` payload."""
    if fidelity_sample is not None and not parity:
        raise ValueError(
            "--fidelity-sample needs the parity report it is recorded in: "
            "drop --skip-parity"
        )
    directory = prepare_output_dir(out_dir, overwrite)
    pinned = resolve_revision(revision)
    snapshot = download_snapshot(pinned)
    check_model_config(snapshot)

    fp32_path = directory / FP32_PATH
    export_fp32(snapshot, fp32_path, pinned)
    shutil.copyfile(snapshot / TOKENIZER_PATH, directory / TOKENIZER_PATH)
    quantize_int8(fp32_path, directory / INT8_PATH)

    # The new heavy files are in place: the manifest and the report of a
    # previous export are now outdated, and must not survive next to these.
    if overwrite:
        clear_manifest(directory)

    payload = build_model_json(
        pinned,
        fp32_hash=blake3_file(fp32_path),
        int8_hash=blake3_file(directory / INT8_PATH),
        tokenizer_hash=blake3_file(directory / TOKENIZER_PATH),
        export=export_versions(),
    )

    if parity:
        tokenizer = load_tokenizer(directory / TOKENIZER_PATH)
        report = run_parity(snapshot, fp32_path, tokenizer)
        # The declared limit, taken from the payload, is the one exercised: a
        # value that drifted away from docs/04 would fail here (INV-11).
        report["max_tokens_verified_seconds"] = verify_max_tokens(
            fp32_path, payload["max_tokens"]
        )
        report["min_cosine_required"] = MIN_PARITY_COSINE
        report["revision"] = pinned
        report["date"] = time.strftime("%Y-%m-%d", time.gmtime())
        if report["min_cosine"] < MIN_PARITY_COSINE:
            raise ValueError(
                f"parity below the required cosine: {report['min_cosine']} "
                f"< {MIN_PARITY_COSINE}"
            )
        if fidelity_sample is not None:
            report["fidelity"] = measure_fidelity(
                fidelity_sample, fp32_path, tokenizer, pinned
            )
        write_model_json(directory / PARITY_PATH, report)
        print(
            f"parity fp32: min {report['min_cosine']:.8f}, "
            f"mean {report['mean_cosine']:.8f} on {report['texts']} texts "
            f"(revision {pinned})"
        )
        if "fidelity" in report:
            fidelity = report["fidelity"]
            print(
                f"fidelity fp32: min {fidelity['min_cosine']:.6f}, "
                f"p1 {fidelity['p1_cosine']:.6f}, "
                f"median {fidelity['median_cosine']:.6f} on "
                f"{fidelity['passages']} passages "
                f"(threshold {fidelity['min_cosine_required']})"
            )

    # `model.json` is written last: it exists only once every other file is
    # written. It also carries `max_tokens`; with `--skip-parity` that limit is
    # not exercised at run time, and no `parity.json` records a check either.
    write_model_json(directory / MODEL_JSON_PATH, payload)
    return payload


def main(argv: list[str] | None = None) -> int:
    arguments = _parse_args(argv)
    try:
        payload = export(
            arguments.out,
            arguments.revision,
            arguments.overwrite,
            not arguments.skip_parity,
            arguments.fidelity_sample,
        )
    except Exception as error:  # noqa: BLE001 - every failure becomes one line
        # Network failures (huggingface_hub), exporter refusals (`RuntimeError`,
        # `NotImplementedError`), ONNX Runtime failures and the checks of this
        # script all end here, with one readable line and a non-zero status,
        # instead of a stack trace. The message is flattened: a multi-line
        # exporter message must still produce a single `error:` line.
        print(f"error: {' '.join(str(error).split())}", file=sys.stderr)
        return 1
    files = payload["files"]
    print(f"wrote {MODEL_NAME} to {arguments.out}")
    print(f"  {FP32_PATH}   blake3 {files['fp32']['blake3']}")
    print(f"  {INT8_PATH} blake3 {files['int8']['blake3']}")
    print(f"  {TOKENIZER_PATH} blake3 {files['tokenizer']['blake3']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
