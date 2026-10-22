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

The graph is exported by ``optimum`` with the sequence length left dynamic. The
declared ``max_tokens`` (8192) is checked twice: against the model config, and
by running the exported graph on an input of exactly that length.

Two acceptance criteria of the task are checked here:

- reproducibility: two runs with the same revision write byte-identical
  ``model.onnx`` (see ``tools/README.md`` for the measured hashes);
- parity: on 100 deterministic French texts, the cosine between the PyTorch
  pooled vectors and the ONNX fp32 ones is at least 0.9999. The measured
  minimum and mean go to ``parity.json`` and to ``tools/README.md``.

``parity.json`` is a fifth file, a report for humans: ``docs/04`` describes four
files, and the engine ignores anything else in the model directory.

The heavy imports (``torch``, ``transformers``, ``tokenizers``,
``huggingface_hub``, ``onnxruntime``, ``optimum``) happen inside the functions
that need them, so importing this module stays cheap and offline: the pytest
suite imports it without downloading a model. They come from the ``export``
dependency group of ``tools/pyproject.toml``, not from the default environment.
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
#: Number of texts of the PyTorch vs ONNX parity check.
PARITY_TEXT_COUNT = 100
#: Seed of the parity corpus. Only ``random.random()`` draws are used, a method
#: whose stream is guaranteed stable across Python versions.
_CORPUS_SEED = 20260222
#: Minimum cosine required by the task between PyTorch and ONNX fp32.
MIN_PARITY_COSINE = 0.9999
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
) -> dict:
    """Builds the ``model.json`` payload described by ``docs/04``.

    The INT8 entry carries a hash but stays out of the model fingerprint, which
    covers ``hf_id``, ``revision``, ``dims``, ``pooling``, ``normalize``,
    ``prompts`` and the hashes of the fp32 and tokenizer files. The fingerprint
    itself is computed by the engine, not here.
    """
    return {
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
    limit, which also exercises truncation.
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


def encode_tokens(
    tokenizer, text: str, prompt: str, max_tokens: int = MAX_TOKENS
) -> list[int]:
    """Encodes ``prompt + text`` and truncates it to ``max_tokens`` tokens."""
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


def export_fp32(snapshot: Path, target: Path, revision: str) -> None:
    """Exports the transformer to fp32 ONNX with dynamic axes."""
    from optimum.exporters.onnx import main_export

    with tempfile.TemporaryDirectory(prefix="crible-onnx-") as work:
        main_export(
            model_name_or_path=str(snapshot),
            output=work,
            task="feature-extraction",
            revision=revision,
            library_name="transformers",
            batch_size=1,
            sequence_length=TRACE_SEQUENCE_LENGTH,
        )
        shutil.copyfile(Path(work) / FP32_PATH, target)


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
    token_lengths = [
        len(encode_tokens(tokenizer, text, PROMPTS["document"])) for text in corpus
    ]
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
    return parser.parse_args(argv)


def export(out_dir: str | Path, revision: str, overwrite: bool, parity: bool) -> dict:
    """Runs the whole export and returns the ``model.json`` payload."""
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
    )

    if parity:
        tokenizer = load_tokenizer(directory / TOKENIZER_PATH)
        report = run_parity(snapshot, fp32_path, tokenizer)
        report["max_tokens_verified_seconds"] = verify_max_tokens(fp32_path)
        report["revision"] = pinned
        report["date"] = time.strftime("%Y-%m-%d", time.gmtime())
        if report["min_cosine"] < MIN_PARITY_COSINE:
            raise ValueError(
                f"parity below the required cosine: {report['min_cosine']} "
                f"< {MIN_PARITY_COSINE}"
            )
        write_model_json(directory / PARITY_PATH, report)
        print(
            f"parity fp32: min {report['min_cosine']:.8f}, "
            f"mean {report['mean_cosine']:.8f} on {report['texts']} texts "
            f"(revision {pinned})"
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
        )
    except (OSError, RuntimeError, ValueError) as error:
        # Network failures (huggingface_hub), ONNX Runtime failures and the
        # checks of this script all end here, with one readable line and a
        # non-zero status, instead of a stack trace.
        print(f"error: {error}", file=sys.stderr)
        return 1
    files = payload["files"]
    print(f"wrote {MODEL_NAME} to {arguments.out}")
    print(f"  {FP32_PATH}   blake3 {files['fp32']['blake3']}")
    print(f"  {INT8_PATH} blake3 {files['int8']['blake3']}")
    print(f"  {TOKENIZER_PATH} blake3 {files['tokenizer']['blake3']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
