# SPDX-License-Identifier: Apache-2.0

"""MES-16 — concurrent load during writes (ADR-0113).

The harness of INV-02 under load: four clients in a closed loop search one index
while five batches are attached to it, then while it is compacted. It writes the
raw samples of every campaign as JSON and prints a summary with the verdict of
the criterion. `@mesureur` runs the campaigns and writes the report
(`docs/mesures/README.md`, rule 7 of `AGENTS.md`); the harness never edits the
report.

Protocol (ADR-0113, decision 5):

1. The server is the `--release` binary given by `--bin` or `CRIBLE_BIN`, started
   here with a temporary `crible.toml`; every path lives in a temporary
   directory that the run removes, so a campaign reads a fresh `data/`.
2. An index on an `external` model, 1 024 dimensions, lexical branch enabled.
   The data is written by `crible.BatchWriter` from a fixed seed: base passages
   with their text, then five batches of new and replaced documents.
3. A fixed set of 200 queries, alternating `dense` (a `vector`) and `hybrid` (a
   `query` and a `vector`), `k = 10`.
4. `N = 4` clients in a closed loop: each one sends its next query as soon as
   the previous answer arrives. `N` stays under the number of cores, and
   `max_concurrent_searches` keeps its default.
5. The base batch is attached **before** the load, as the set-up of the
   campaign: the index the searches read is the loaded one, so the reference
   describes the state under load. Phases: a warm-up of 30 s (discarded), a rest
   of 60 s (the reference), the five update batches (from the first submission
   to the end of the last job), then the compaction (from the submission to the
   end of its job).
6. The latency is the wall clock read by the client around each request;
   `took_ms` of the answer is reported next to it, and so is `pool_size`, which
   tells the scan path of each phase (EXACT at or below `exact_threshold`,
   BINARY above it). Percentiles use the nearest rank of ADR-0077.
7. Three complete campaigns, each on a fresh directory; the criterion must hold
   on each one. The warm-up is reported apart, with its p99 per slice of 10 s,
   so the report shows the clients converged; it never enters the criterion.

Criterion (ADR-0113, decisions 1 and 6):

- **blocking**: during each write phase, **every** search ends in less than
  3 × the p99 of the rest phase;
- reported too: the p99 of a write phase, against 3 × the p99 of the rest phase
  (the criterion of the fiche, implied by the precedent);
- no `5xx` at all on searches, `503` included; a `5xx` on a write is counted
  with its `Retry-After` and explained, and does not fail the campaign.

Usage, from the repository root::

    cargo build --release
    cd tools
    uv run --no-sync python bench/mes16/mes16_charge.py --bin ../target/release/crible

A short trial (not a campaign) keeps every parameter at hand; its base stays
above `exact_threshold` (5 000), so every phase takes the same scan path as the
campaign and a trial never judges another protocol::

    uv run --no-sync python bench/mes16/mes16_charge.py --campaigns 1 \
        --base-passages 20000 --batch-passages 200 --rest 10 --warmup 10
"""

from __future__ import annotations

import argparse
import asyncio
import datetime
import hashlib
import json
import math
import os
import platform
import secrets
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable, Mapping, Sequence

import blake3
import numpy as np

from crible import AsyncClient, BatchWriter, CribleError, CribleTransportError

#: Root of the repository: the default binary and the default output live under
#: it, so a run from `tools/` writes where `data/` is.
ROOT = Path(__file__).resolve().parents[3]

#: Name of the index of the campaign.
INDEX = "mes16"

#: Configuration of the model of the index: `external`, so the vectors come
#: with the batches and no encoder is loaded (ADR-0113, decision 5).
MODEL = {
    "name": "mes16-charge",
    "kind": "external",
    "dims": 1024,
    "normalize": True,
}

#: Chunker and template of the batches; they are traced in every manifest and
#: never interpreted by the engine.
CHUNKER = {"name": "mes16-units", "version": "1", "params": {}}
TEMPLATE = {"name": "mes16-text", "version": "1"}

#: Schema of the index: one document field and one passage field, enough for the
#: payloads and the filters of the queries.
SCHEMA = {
    "document_fields": {"annee": {"type": "integer"}},
    "passage_fields": {"partie": {"type": "keyword"}},
}

#: Fixed seed of the generation (decision 5). Changing it changes the data, so a
#: report cites it.
SEED = 20261030

#: Phase whose samples are kept apart from the criterion (decision 5).
WARMUP_PHASE = "warmup"

#: Phase whose p99 is the reference of the criterion.
REFERENCE_PHASE = "rest"

#: Phases of a campaign, in their order. The warm-up is measured and discarded.
PHASES = (WARMUP_PHASE, REFERENCE_PHASE, "attach", "compaction")

#: Width of the slices the report uses to show the warm-up converged, in seconds.
WARMUP_SLICE_S = 10.0

#: Phases of the criterion: the two that carry a write.
WRITE_PHASES = ("attach", "compaction")

#: Multiplier of the criterion: a search must end in less than this times the
#: p99 of the rest phase (ADR-0113, decision 1).
FACTOR = 3.0

#: Number of clients of the closed loop, kept under the number of cores.
CLIENTS = 4

#: Words of the generated texts, so a hybrid query matches the lexical branch.
WORDS = (
    "alpha",
    "beta",
    "gamma",
    "delta",
    "epsilon",
    "zeta",
    "eta",
    "theta",
    "iota",
    "kappa",
    "lambda",
    "mu",
    "nu",
    "xi",
    "omicron",
    "pi",
)


@dataclass
class Options:
    """Parameters of the run; the defaults are those of ADR-0113."""

    #: Binary of the server, `--release` for a campaign.
    binary: Path = ROOT / "target" / "release" / "crible"
    #: Directory that receives the raw samples and the summary.
    out: Path = ROOT / "data" / "mes16"
    #: Number of complete campaigns, each on a fresh directory.
    campaigns: int = 3
    #: Passages of the base batch.
    base_passages: int = 50_000
    #: Passages of each of the five update batches.
    batch_passages: int = 2_000
    #: Update batches attached during the attaches phase.
    batches: int = 5
    #: Passages per document of every batch.
    passages_per_document: int = 2
    #: Queries of the fixed set.
    queries: int = 200
    #: Clients of the closed loop.
    clients: int = CLIENTS
    #: Results per query (decision 5).
    k: int = 10
    #: Seconds of the warm-up, discarded from the results.
    warmup: float = 30.0
    #: Seconds of the rest phase, the reference of the criterion.
    rest: float = 60.0
    #: Seconds a write job may take before the campaign fails.
    job_timeout: float = 600.0
    #: Keep the temporary directory of a campaign, for a diagnosis.
    keep: bool = False


@dataclass(frozen=True)
class Sample:
    """One search: its phase, its mode and its answer."""

    #: Phase the request started in.
    phase: str
    #: Wall clock around the call, in milliseconds.
    wall_ms: float
    #: `took_ms` of the answer, or ``None`` when the answer is an error.
    took_ms: float | None
    #: HTTP status, or ``None`` for a transport failure.
    status: int | None
    #: Mode of the query, `dense` or `hybrid`.
    mode: str | None = None
    #: `pool_size` of the answer: the passages the filter selected, which tells
    #: the scan path the engine took (EXACT at or below `exact_threshold`,
    #: BINARY above it).
    pool_size: int | None = None
    #: Seconds between the first request of the load and this one; it slices the
    #: warm-up, whose convergence the report shows.
    at_s: float = 0.0


@dataclass
class Write:
    """One write submission: the attach of a batch, or the compaction."""

    #: `attach` or `compaction`.
    kind: str
    #: Batch identifier, for an attach; ``None`` for the compaction.
    batch_id: str | None
    #: HTTP status of the submission: `202` when the job was queued.
    status: int | None
    #: `Retry-After` of a `503`, in seconds, when the answer carries one.
    retry_after: float | None
    #: Wall clock from the submission to the terminal state of the job.
    duration_ms: float
    #: Generation the job published, when it did.
    generation: int | None
    #: State of the job.
    state: str


def nearest_rank(values: Sequence[float], ratio: float) -> float:
    """Returns the nearest-rank percentile of a series (ADR-0077).

    Rank is `ceil(ratio × n)`, clamped to `[1, n]`, and the result is the value
    at that rank in the sorted series: no interpolation between two samples.
    """
    if not values:
        raise ValueError("nearest_rank of an empty series")
    if not 0.0 < ratio <= 1.0:
        raise ValueError(f"ratio out of (0, 1]: {ratio}")
    ordered = sorted(values)
    rank = math.ceil(ratio * len(ordered))
    return ordered[min(max(rank, 1), len(ordered)) - 1]


def summarise(values: Sequence[float]) -> dict[str, float]:
    """Count, p50, p95, p99, mean, min and max of a series, in milliseconds."""
    if not values:
        raise ValueError("summarise of an empty series")
    return {
        "n": len(values),
        "p50": nearest_rank(values, 0.50),
        "p95": nearest_rank(values, 0.95),
        "p99": nearest_rank(values, 0.99),
        "mean": sum(values) / len(values),
        "min": min(values),
        "max": max(values),
    }


def split_phases(samples: Iterable[Sample]) -> dict[str, list[Sample]]:
    """Groups the samples by phase, dropping the warm-up (decision 5).

    The warm-up is measured so a client reaches its steady state, and it is
    never read: its samples would bias the reference. It has its own key in the
    document (`warmup`), outside the criterion.
    """
    grouped: dict[str, list[Sample]] = {phase: [] for phase in PHASES if phase != WARMUP_PHASE}
    for sample in samples:
        if sample.phase in grouped:
            grouped[sample.phase].append(sample)
    return grouped


def time_buckets(
    samples: Sequence[Sample], width_s: float = WARMUP_SLICE_S
) -> list[dict[str, Any]]:
    """Groups the samples by slices of `width_s` seconds, by their offset.

    The slice of a sample is the slice of its `at_s`, counted from the first
    request of the load. The reduction of `tools/bench/mes16/resume_mes16.py`
    imports this function, so the report and the summary slice the phases the
    same way.
    """
    if width_s <= 0:
        raise ValueError("the width of a slice must be strictly positive")
    buckets: dict[int, list[Sample]] = {}
    for sample in samples:
        buckets.setdefault(int(sample.at_s // width_s), []).append(sample)
    return [
        {
            "from_s": number * width_s,
            "to_s": (number + 1) * width_s,
            "samples": buckets[number],
        }
        for number in sorted(buckets)
    ]


def warmup_slices(
    samples: Sequence[Sample], width_s: float = WARMUP_SLICE_S
) -> list[dict[str, Any]]:
    """p99 of the warm-up per slice of `width_s` seconds, for the report only.

    The convergence is shown, never judged: no threshold applies to the warm-up
    (decision 5).
    """
    warm = [sample for sample in samples if sample.phase == WARMUP_PHASE]
    return [
        {
            "from_s": bucket["from_s"],
            "to_s": bucket["to_s"],
            "n": len(bucket["samples"]),
            "p99_ms": nearest_rank([sample.wall_ms for sample in bucket["samples"]], 0.99),
        }
        for bucket in time_buckets(warm, width_s)
    ]


def count_modes(samples: Iterable[Sample]) -> dict[str, int]:
    """Counts the searches by mode, `dense` and `hybrid`."""
    counts: dict[str, int] = {}
    for sample in samples:
        key = sample.mode or "unknown"
        counts[key] = counts.get(key, 0) + 1
    return dict(sorted(counts.items()))


def pool_range(samples: Iterable[Sample]) -> dict[str, int] | None:
    """Smallest and largest `pool_size` of the answers, or ``None`` if none had one.

    The `pool_size` of a phase is what tells its scan path: at or below
    `exact_threshold` the engine scans exactly, above it the binary scan and the
    rescoring run.
    """
    values = [sample.pool_size for sample in samples if sample.pool_size is not None]
    if not values:
        return None
    return {"min": min(values), "max": max(values)}


def sample_from(
    answer: Mapping[str, Any],
    phase: str,
    wall_ms: float,
    at_s: float,
    mode: str | None,
) -> Sample:
    """Builds the sample of a `200` answer of `POST /search`."""
    took = answer.get("took_ms")
    pool = answer.get("pool_size")
    return Sample(
        phase=phase,
        wall_ms=wall_ms,
        took_ms=float(took) if isinstance(took, (int, float)) else None,
        status=200,
        mode=mode,
        pool_size=int(pool) if isinstance(pool, int) else None,
        at_s=at_s,
    )


def count_statuses(samples: Iterable[Sample]) -> dict[str, int]:
    """Counts the answers by status; a transport failure counts as `transport`."""
    counts: dict[str, int] = {}
    for sample in samples:
        key = "transport" if sample.status is None else str(sample.status)
        counts[key] = counts.get(key, 0) + 1
    return dict(sorted(counts.items()))


def search_failures(samples: Iterable[Sample]) -> list[Sample]:
    """The searches that must fail the campaign (ADR-0113, decision 6).

    Every `5xx` counts, `503` included, and so does a transport failure: no
    search is expected to be refused at `N = 4` under the default concurrency
    limit.
    """
    return [
        sample
        for sample in samples
        if sample.status is None or 500 <= sample.status < 600
    ]


def criterion(reference_p99_ms: float, samples: Sequence[Sample]) -> dict[str, Any]:
    """Evaluates decision 1 on one write phase.

    **Every** search of the phase must end in less than `FACTOR` times the p99 of
    the rest phase. The p99 of the phase is reported next to it, because the
    fiche states the criterion that way and the per-request one implies it.
    """
    limit = FACTOR * reference_p99_ms
    over = [sample for sample in samples if sample.wall_ms >= limit]
    walls = [sample.wall_ms for sample in samples]
    return {
        "n": len(samples),
        "limit_ms": limit,
        "over": len(over),
        "worst_ms": max(walls) if walls else None,
        # A phase that recorded no search measured nothing: it cannot pass.
        "passed": bool(samples) and not over,
        "p99_ms": nearest_rank(walls, 0.99) if walls else None,
        "p99_passed": bool(walls) and nearest_rank(walls, 0.99) < limit,
    }


def write_failures(writes: Iterable[Write]) -> list[Write]:
    """The writes that must fail the campaign.

    A `5xx` other than `503` fails the campaign, and so does a job that did not
    reach `succeeded`: a write phase that wrote nothing measures nothing. A
    `503` is counted and explained, never tolerated silently and never a failure
    by itself (ADR-0113, decision 6).
    """
    return [
        write
        for write in writes
        if write.status != 503
        and (
            write.state != "succeeded"
            or write.status is None
            or 500 <= write.status < 600
        )
    ]


def verdict(
    reference_p99_ms: float,
    phases: Mapping[str, Sequence[Sample]],
    writes: Sequence[Write],
) -> dict[str, Any]:
    """Evaluates the whole criterion of ADR-0113 on one campaign.

    Every measured phase counts, the reference included: decision 6 fails the
    campaign on any `5xx` of a search, and a `5xx` during the rest would
    otherwise pass unseen. The search statuses are reported phase by phase, so
    the report shows where a failure came from.
    """
    measured = (REFERENCE_PHASE, *WRITE_PHASES)
    per_phase = {
        phase: criterion(reference_p99_ms, phases.get(phase, ()))
        for phase in WRITE_PHASES
    }
    searches = [sample for phase in measured for sample in phases.get(phase, ())]
    failures = search_failures(searches)
    failed_writes = write_failures(writes)
    return {
        "reference": {
            "phase": REFERENCE_PHASE,
            "p99_ms": reference_p99_ms,
            "limit_ms": FACTOR * reference_p99_ms,
        },
        "per_request": per_phase,
        "searches": {
            "n": len(searches),
            "statuses": count_statuses(searches),
            "by_phase": {
                phase: count_statuses(phases.get(phase, ())) for phase in measured
            },
            "failures": len(failures),
            "worst_ms": max((sample.wall_ms for sample in failures), default=None),
        },
        "writes": {
            "n": len(writes),
            "statuses": _count_writes(writes),
            "failures": [
                {
                    "kind": write.kind,
                    "batch_id": write.batch_id,
                    "status": write.status,
                }
                for write in failed_writes
            ],
            "explained_503": [
                {
                    "kind": write.kind,
                    "batch_id": write.batch_id,
                    "retry_after": write.retry_after,
                }
                for write in writes
                if write.status == 503
            ],
        },
        "passed": (
            all(entry["passed"] for entry in per_phase.values())
            and not failures
            and not failed_writes
        ),
    }


def _count_writes(writes: Iterable[Write]) -> dict[str, int]:
    """Counts the writes by status."""
    counts: dict[str, int] = {}
    for write in writes:
        key = "transport" if write.status is None else str(write.status)
        counts[key] = counts.get(key, 0) + 1
    return dict(sorted(counts.items()))


def query_set(count: int, dims: int, seed: int = SEED) -> list[dict[str, Any]]:
    """The fixed set of queries: `dense` and `hybrid`, alternating (decision 5).

    A `dense` query carries its vector only; a `hybrid` one carries a text, for
    the lexical branch, and its vector, because the model is `external` and the
    server encodes nothing.
    """
    if count <= 0:
        raise ValueError("count must be strictly positive")
    rng = _rng(seed, 2)
    queries: list[dict[str, Any]] = []
    for number in range(count):
        vector = _unit_vector(rng, dims)
        if number % 2 == 0:
            queries.append({"mode": "dense", "vector": vector})
        else:
            word = WORDS[number % len(WORDS)]
            queries.append(
                {"mode": "hybrid", "query": f"{word} {WORDS[(number + 5) % len(WORDS)]}",
                 "vector": vector}
            )
    return queries


def _rng(seed: int, stream: int) -> np.random.Generator:
    """The generator of one named stream, at the fixed seed."""
    return np.random.Generator(np.random.PCG64(np.random.SeedSequence([seed, stream])))


def _unit_vector(rng: np.random.Generator, dims: int) -> np.ndarray:
    """One unit vector, in fp32, the law of every vector of the campaign."""
    vector = rng.standard_normal(dims)
    return (vector / np.linalg.norm(vector)).astype(np.float32)


def _text(rng: np.random.Generator, doc_id: str, rank: int) -> str:
    """The text of one passage: its key and six words of the vocabulary."""
    words = [WORDS[int(rng.integers(0, len(WORDS)))] for _ in range(6)]
    return f"{doc_id} passage {rank} " + " ".join(words)


def _doc_hash(texts: Sequence[str], vectors: Sequence[np.ndarray], fields: Sequence[Mapping[str, Any]]) -> str:
    """The producer hash of a document's passages (ADR-0041).

    It covers the text, the passage fields and the stored fp16 vector, and never
    the document fields, so a replaced document changes its hash.
    """
    hasher = blake3.blake3()
    for text, vector, passage_fields in zip(texts, vectors, fields):
        hasher.update(b"\0")
        hasher.update(text.encode("utf-8"))
        hasher.update(json.dumps(passage_fields, sort_keys=True).encode("utf-8"))
        hasher.update(np.asarray(vector, dtype="<f2").tobytes())
    return hasher.hexdigest()


def _document(
    rng: np.random.Generator,
    doc_id: str,
    dims: int,
    passages_per_document: int,
) -> tuple[str, str, dict[str, Any], list[dict[str, Any]]]:
    """One generated document: identifier, hash, fields and passages."""
    texts = []
    vectors = []
    fields = []
    passages = []
    for rank in range(passages_per_document):
        text = _text(rng, doc_id, rank)
        vector = _unit_vector(rng, dims)
        passage_fields = {"partie": WORDS[int(rng.integers(0, len(WORDS)))]}
        texts.append(text)
        vectors.append(vector)
        fields.append(passage_fields)
        passages.append({"text": text, "vector": vector, **passage_fields})
    document_fields = {"annee": int(rng.integers(1990, 2026))}
    return (
        doc_id,
        _doc_hash(texts, vectors, fields),
        document_fields,
        passages,
    )


def index_config() -> dict[str, Any]:
    """Configuration of the index of the campaign (decision 5)."""
    return {
        "name": INDEX,
        "model": MODEL["name"],
        "reference_precision": "fp16",
        "schema": SCHEMA,
        "cluster_by": [],
        "lexical": {"enabled": True, "language": "fr", "positions": False},
        "search": {
            "scan": "binary",
            "rescore_candidates": 5000,
            "exact_threshold": 5000,
            "candidates_per_branch": 100,
            "fusion": {"method": "rrf"},
        },
        "ingest": {"acceptance_sample": 200, "acceptance_min_cosine": 0.999},
        "compaction": {"dead_ratio": 0.10},
    }


def _write_batch(
    out_dir: Path,
    batch_id: str,
    documents: Iterable[tuple[str, str, dict[str, Any], list[dict[str, Any]]]],
) -> Path:
    """Writes one batch with `crible.BatchWriter` and returns its directory."""
    writer = BatchWriter(
        index_config(),
        out_dir,
        batch_id,
        "upsert",
        MODEL,
        CHUNKER,
        TEMPLATE,
        created_at="2026-01-01T00:00:00Z",
        host="mes16",
    )
    for doc_id, doc_hash, fields, passages in documents:
        writer.add_document(doc_id, doc_hash, fields, passages)
    return writer.close()


@dataclass(frozen=True)
class Batches:
    """The batches of a campaign: the base one, then the update batches.

    The base batch is attached as the set-up of the campaign, before the load;
    only the updates belong to the attach phase (ADR-0113, decision 5).
    """

    #: Identifier of the base batch.
    base: str
    #: Identifiers of the update batches, in their attach order.
    updates: tuple[str, ...]


def generate(inbox: Path, options: Options) -> Batches:
    """Writes the base batch and the update batches into `inbox` (decision 5).

    The base batch is `base_passages` passages of new documents. Each update
    batch holds new documents and replaced documents in equal numbers of
    passages, so an attach writes new text and tombstones old text.
    """
    per_document = options.passages_per_document
    dims = int(MODEL["dims"])
    base_documents = options.base_passages // per_document
    rng = _rng(SEED, 1)
    base_id = "mes16-base"
    base = _write_batch(
        inbox,
        base_id,
        (
            _document(rng, f"MES16-{number:08d}", dims, per_document)
            for number in range(base_documents)
        ),
    )
    if not base.is_dir():
        raise ValueError("the base batch was not written")
    updates: list[str] = []
    new_passages = options.batch_passages // 2
    replaced_passages = options.batch_passages - new_passages
    for number in range(options.batches):
        rng = _rng(SEED, 10 + number)
        documents = []
        for index in range(new_passages // per_document):
            documents.append(
                _document(rng, f"MES16B{number}-{index:06d}", dims, per_document)
            )
        start = number * (replaced_passages // per_document)
        for index in range(replaced_passages // per_document):
            documents.append(
                _document(rng, f"MES16-{start + index:08d}", dims, per_document)
            )
        batch_id = f"mes16-update-{number + 1}"
        _write_batch(inbox, batch_id, documents)
        updates.append(batch_id)
    return Batches(base=base_id, updates=tuple(updates))


def configuration(root: Path, keys: Mapping[str, str]) -> str:
    """Writes the temporary `crible.toml` of the campaign and returns its path.

    Every path is under `root`; the port is chosen free just before the write.
    The keys are the three of ADR-0095, one per role.
    """
    port = _free_port()
    text = f"""[server]
bind = "127.0.0.1:{port}"
search_threads = 0
max_concurrent_searches = 0

[paths]
data = "{root / "data"}"
models = "{root / "models"}"
exchange = "{root / "exchange"}"

[[auth.keys]]
name = "mes16-admin"
role = "admin"
sha256 = "{_sha256(keys["admin"])}"

[[auth.keys]]
name = "mes16-read"
role = "read"
sha256 = "{_sha256(keys["read"])}"

[[auth.keys]]
name = "mes16-write"
role = "write"
sha256 = "{_sha256(keys["write"])}"

[analytics]
retention_days = 90

[debug_journal]
enabled = false
sample_rate = 0.0
ttl_days = 7

[encoder]
query_precision = "fp32"

[limits]
writer_queue = 16
default_timeout_ms = 2000
wait_timeout_ms = 60000
query_cache_entries = 1024
"""
    path = root / "crible.toml"
    path.write_text(text, encoding="utf-8")
    return str(path)


def _sha256(token: str) -> str:
    """The digest of one API key, as `crible.toml` stores it (ADR-0100)."""
    return hashlib.sha256(token.encode("utf-8")).hexdigest()


def _free_port() -> int:
    """A free TCP port of the loopback interface."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
        probe.bind(("127.0.0.1", 0))
        return int(probe.getsockname()[1])


def _base_url(config_path: Path) -> str:
    """The URL of the instance, read from the `bind` of the configuration."""
    for line in config_path.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if stripped.startswith("bind"):
            address = stripped.split("=", 1)[1].strip().strip('"')
            return f"http://{address}"
    raise ValueError(f"no bind in {config_path}")


class Server:
    """A server started for one campaign and stopped when the campaign ends.

    The stop is guaranteed: `__exit__` sends `SIGTERM`, waits for the process
    and kills it when it does not end. No process outlives the harness.
    """

    def __init__(self, binary: Path, config_path: Path, log_path: Path) -> None:
        self._binary = binary
        self._config = config_path
        self._log = log_path
        self._process: subprocess.Popen[bytes] | None = None

    def __enter__(self) -> Server:
        if not self._binary.is_file():
            raise FileNotFoundError(
                f"the server binary {self._binary} does not exist; build it with "
                "`cargo build --release` or name it with --bin / CRIBLE_BIN"
            )
        log = self._log.open("wb")
        self._process = subprocess.Popen(
            [str(self._binary), "serve", "--config", str(self._config)],
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        self._log_handle = log
        return self

    @property
    def url(self) -> str:
        """Root URL of the instance."""
        return _base_url(self._config)

    def wait_ready(self, timeout: float = 120.0) -> None:
        """Waits for `/ready` to answer `200`, or fails the campaign."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self._process is not None and self._process.poll() is not None:
                raise RuntimeError(
                    f"the server exited with code {self._process.returncode}; its log is "
                    f"{self._log}"
                )
            try:
                with urllib.request.urlopen(f"{self.url}/ready", timeout=2.0) as answer:
                    if answer.status == 200:
                        return
            except (urllib.error.URLError, OSError):
                pass
            time.sleep(0.1)
        raise TimeoutError(f"the server was not ready after {timeout} s; its log is {self._log}")

    def __exit__(self, *exc_info: object) -> None:
        if self._process is not None and self._process.poll() is None:
            self._process.send_signal(signal.SIGTERM)
            try:
                self._process.wait(timeout=60)
            except subprocess.TimeoutExpired:
                self._process.kill()
                self._process.wait(timeout=10)
        if self._process is not None:
            self._log_handle.close()
            self._process = None


@dataclass
class Load:
    """State shared by the clients of the closed loop."""

    #: Phase the next request belongs to.
    phase: str = WARMUP_PHASE
    #: `perf_counter` of the first request, so every sample carries its offset.
    started: float = 0.0
    #: Set when the campaign stops the clients.
    stop: asyncio.Event = field(default_factory=asyncio.Event)


async def _load_client(
    client: AsyncClient,
    queries: Sequence[Mapping[str, Any]],
    offset: int,
    load: Load,
    samples: list[Sample],
    options: Options,
) -> None:
    """One client of the closed loop: send, record, send the next one."""
    cursor = offset
    while not load.stop.is_set():
        query = queries[cursor % len(queries)]
        cursor += 1
        phase = load.phase
        started = time.perf_counter()
        try:
            answer = await client.search(INDEX, k=options.k, **query)
        except (CribleTransportError, CribleError) as failure:
            status = None if isinstance(failure, CribleTransportError) else failure.status
            samples.append(
                Sample(
                    phase=phase,
                    wall_ms=(time.perf_counter() - started) * 1000.0,
                    took_ms=None,
                    status=status,
                    mode=str(query["mode"]),
                    at_s=started - load.started,
                )
            )
            continue
        samples.append(
            sample_from(
                answer,
                phase,
                (time.perf_counter() - started) * 1000.0,
                started - load.started,
                str(query["mode"]),
            )
        )


async def _attach(
    client: AsyncClient,
    batch_id: str,
    options: Options,
) -> Write:
    """Attaches one deposited batch and follows its job."""
    started = time.perf_counter()
    status: int | None = None
    retry_after: float | None = None
    generation: int | None = None
    state = "failed"
    try:
        job = await client.attach_batch(INDEX, batch_id)
        status = 202
        job = await client.wait(job, timeout=options.job_timeout, poll_interval=0.2)
        state = str(job.get("state"))
        value = job.get("generation")
        generation = int(value) if isinstance(value, int) else None
    except CribleError as failure:
        if failure.status is not None:
            status = failure.status
            retry_after = failure.retry_after
        state = "failed"
    return Write(
        kind="attach",
        batch_id=batch_id,
        status=status,
        retry_after=retry_after,
        duration_ms=(time.perf_counter() - started) * 1000.0,
        generation=generation,
        state=state,
    )


async def _compact(client: AsyncClient, options: Options) -> Write:
    """Compacts the index and follows its job."""
    started = time.perf_counter()
    status: int | None = None
    retry_after: float | None = None
    generation: int | None = None
    state = "failed"
    try:
        job = await client.compact(INDEX)
        status = 202
        job = await client.wait(job, timeout=options.job_timeout, poll_interval=0.2)
        state = str(job.get("state"))
        value = job.get("generation")
        generation = int(value) if isinstance(value, int) else None
    except CribleError as failure:
        if failure.status is not None:
            status = failure.status
            retry_after = failure.retry_after
        state = "failed"
    return Write(
        kind="compaction",
        batch_id=None,
        status=status,
        retry_after=retry_after,
        duration_ms=(time.perf_counter() - started) * 1000.0,
        generation=generation,
        state=state,
    )


async def _attach_base(
    url: str,
    key: str,
    batch_id: str,
    options: Options,
) -> Write:
    """Attaches the base batch as the set-up of the campaign, before the load.

    The searches of the warm-up and of the rest must read the loaded index: a
    base attached during the attach phase would make the reference describe an
    empty index, which decision 1 cannot compare with the write phases
    (ADR-0113, decision 1).
    """
    async with AsyncClient(url, key) as client:
        write = await _attach(client, batch_id, options)
    if write.state != "succeeded" or write.generation is None:
        raise RuntimeError(f"the base batch {batch_id} did not attach: {write}")
    return write


async def _run_load(
    root: Path,
    url: str,
    keys: Mapping[str, str],
    batch_ids: Sequence[str],
    options: Options,
    progress,
) -> tuple[list[Sample], list[Write]]:
    """Runs the four phases: warm-up, rest, attaches of the updates, compaction."""
    queries = query_set(options.queries, int(MODEL["dims"]))
    load = Load()
    load.started = time.perf_counter()
    samples: list[Sample] = []
    writes: list[Write] = []
    readers = [AsyncClient(url, keys["read"]) for _ in range(options.clients)]
    writer = AsyncClient(url, keys["write"])
    compactor = AsyncClient(url, keys["admin"])
    try:
        tasks = [
            asyncio.create_task(
                _load_client(client, queries, number, load, samples, options)
            )
            for number, client in enumerate(readers)
        ]
        progress(f"warm-up {options.warmup:.0f} s")
        await asyncio.sleep(options.warmup)
        load.phase = "rest"
        progress(f"rest {options.rest:.0f} s")
        await asyncio.sleep(options.rest)
        load.phase = "attach"
        for number, batch_id in enumerate(batch_ids):
            progress(f"attach {number + 1}/{len(batch_ids)}: {batch_id}")
            writes.append(await _attach(writer, batch_id, options))
        load.phase = "compaction"
        progress("compaction")
        writes.append(await _compact(compactor, options))
        load.phase = "done"
        load.stop.set()
        await asyncio.gather(*tasks)
    finally:
        load.stop.set()
        for client in readers:
            await client.close()
        await writer.close()
        await compactor.close()
    return samples, writes


def phase_stats(entries: Sequence[Sample]) -> dict[str, Any]:
    """Statistics of one phase: count, wall clock, `took_ms`, statuses, modes, pool.

    The `pool_size` range is what a report cites to show every phase took the
    same scan path: at or below `exact_threshold` the search is exact, above it
    the binary scan and the rescoring run (ADR-0113, decision 1).
    """
    if not entries:
        return {
            "n": 0,
            "statuses": {},
            "wall_ms": None,
            "took_ms": None,
            "modes": {},
            "pool_size": None,
            "samples": [],
        }
    return {
        "n": len(entries),
        "statuses": count_statuses(entries),
        "wall_ms": summarise([entry.wall_ms for entry in entries]),
        "took_ms": summarise(
            [entry.took_ms for entry in entries if entry.took_ms is not None]
        )
        if any(entry.took_ms is not None for entry in entries)
        else None,
        "modes": count_modes(entries),
        "pool_size": pool_range(entries),
        "samples": [
            {
                "mode": entry.mode,
                "wall_ms": round(entry.wall_ms, 3),
                "took_ms": entry.took_ms,
                "status": entry.status,
                "pool_size": entry.pool_size,
                "at_s": round(entry.at_s, 3),
            }
            for entry in entries
        ],
    }


def campaign(options: Options, number: int) -> dict[str, Any]:
    """Runs one campaign on a fresh temporary directory and returns its raw JSON."""
    root = Path(tempfile.mkdtemp(prefix=f"mes16-campaign-{number}-"))
    started = time.monotonic()
    try:
        keys = {
            "admin": secrets.token_urlsafe(32),
            "read": secrets.token_urlsafe(32),
            "write": secrets.token_urlsafe(32),
        }
        (root / "data").mkdir()
        (root / "exchange" / "inbox").mkdir(parents=True)
        (root / "exchange" / "outbox").mkdir()
        model_dir = root / "models" / MODEL["name"]
        model_dir.mkdir(parents=True)
        (model_dir / "model.json").write_text(
            json.dumps(MODEL, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )

        def progress(message: str) -> None:
            elapsed = time.monotonic() - started
            print(f"[campaign {number} {elapsed:7.1f} s] {message}", flush=True)

        progress("generating the data")
        batches = generate(root / "exchange" / "inbox", options)
        progress(f"{1 + len(batches.updates)} batch(es) written")

        config_path = Path(configuration(root, keys))
        with Server(options.binary, config_path, root / "server.log") as server:
            server.wait_ready()
            progress("server ready")
            admin = asyncio.run(_create_index(server.url, keys["admin"]))
            progress(f"index {INDEX} created (generation {admin})")
            # The base batch is the index under load: it is attached before the
            # first search, so the warm-up and the rest measure the loaded index
            # (ADR-0113, decision 1).
            base = asyncio.run(
                _attach_base(server.url, keys["write"], batches.base, options)
            )
            progress(
                f"base batch {batches.base} attached (generation {base.generation})"
            )
            samples, writes = asyncio.run(
                _run_load(root, server.url, keys, batches.updates, options, progress)
            )
        phases = split_phases(samples)
        warmup = [sample for sample in samples if sample.phase == WARMUP_PHASE]
        rest = [sample.wall_ms for sample in phases[REFERENCE_PHASE]]
        if not rest:
            raise RuntimeError("the rest phase recorded no search")
        reference = nearest_rank(rest, 0.99)
        warmup_stats = phase_stats(warmup)
        warmup_stats["slices"] = warmup_slices(warmup)
        document = {
            "campaign": number,
            "date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "machine": {
                "platform": platform.platform(),
                "processor": platform.processor(),
                "cores": os.cpu_count(),
                "python": platform.python_version(),
            },
            "base": {
                "batch_id": batches.base,
                "generation": base.generation,
                "duration_ms": round(base.duration_ms, 3),
            },
            "protocol": {
                "seed": SEED,
                "dims": MODEL["dims"],
                "base_passages": options.base_passages,
                "batch_passages": options.batch_passages,
                "batches": options.batches,
                "passages_per_document": options.passages_per_document,
                "queries": options.queries,
                "k": options.k,
                "clients": options.clients,
                "warmup_s": options.warmup,
                "rest_s": options.rest,
                "factor": FACTOR,
            },
            "warmup": warmup_stats,
            "phases": {
                phase: phase_stats(entries) for phase, entries in phases.items()
            },
            "writes": [
                {
                    "kind": write.kind,
                    "batch_id": write.batch_id,
                    "status": write.status,
                    "retry_after": write.retry_after,
                    "duration_ms": round(write.duration_ms, 3),
                    "generation": write.generation,
                    "state": write.state,
                }
                for write in writes
            ],
            "criterion": verdict(reference, phases, writes),
            "duration_s": time.monotonic() - started,
        }
        return document
    finally:
        if options.keep:
            print(f"[campaign {number}] kept {root}", flush=True)
        else:
            shutil.rmtree(root, ignore_errors=True)


async def _create_index(url: str, admin_key: str) -> int:
    """Creates the index of the campaign and returns its generation."""
    async with AsyncClient(url, admin_key) as client:
        answer = await client.create_index(index_config())
    generation = answer.get("generation")
    return int(generation) if isinstance(generation, int) else 0


def report(document: Mapping[str, Any]) -> str:
    """One-line summary of a campaign, for the terminal."""
    criterion = document["criterion"]
    lines = [f"campaign {document['campaign']}: {'PASS' if criterion['passed'] else 'FAIL'}"]
    reference = criterion["reference"]
    lines.append(
        f"  rest p99 {reference['p99_ms']:.2f} ms, limit {reference['limit_ms']:.2f} ms"
    )
    by_phase = criterion["searches"]["by_phase"]
    lines.append(f"  base generation {document['base']['generation']}")
    warmup = document["warmup"]
    if warmup["wall_ms"] is not None:
        slices = ", ".join(
            f"{entry['from_s']:.0f}-{entry['to_s']:.0f}s {entry['p99_ms']:.1f}"
            for entry in warmup["slices"]
        )
        lines.append(
            f"  warmup (outside the criterion): n={warmup['n']} p99 {slices} ms"
        )
    for phase in (REFERENCE_PHASE, *WRITE_PHASES):
        entry = document["phases"][phase]
        pool = entry["pool_size"]
        pool_text = "n/a" if pool is None else f"{pool['min']}-{pool['max']}"
        if entry["wall_ms"] is None:
            lines.append(f"  {phase}: n=0 (no search recorded)")
            continue
        wall = entry["wall_ms"]
        judged = criterion["per_request"].get(phase)
        if judged is None:
            flag = ""
        else:
            flag = f" over={judged['over']} {'ok' if judged['passed'] else 'OVER'}"
        lines.append(
            f"  {phase}: n={entry['n']} p50={wall['p50']:.1f} p99={wall['p99']:.1f} "
            f"max={wall['max']:.1f} pool={pool_text} modes={entry['modes']} "
            f"statuses={by_phase.get(phase)}{flag}"
        )
    lines.append(
        f"  searches (all measured phases): {criterion['searches']['statuses']} "
        f"failures={criterion['searches']['failures']}"
    )
    lines.append(f"  writes: {criterion['writes']['statuses']}")
    return "\n".join(lines)


def parse(argv: Sequence[str] | None = None) -> Options:
    """Reads the command line; the defaults are those of ADR-0113."""
    parser = argparse.ArgumentParser(
        description="MES-16: concurrent load during attaches and a compaction (ADR-0113)",
    )
    parser.add_argument(
        "--bin",
        default=os.environ.get("CRIBLE_BIN", str(Options.binary)),
        help="release binary of the server (default: CRIBLE_BIN, or target/release/crible)",
    )
    parser.add_argument("--out", default=str(Options.out), help="directory of the raw JSON")
    parser.add_argument("--campaigns", type=int, default=Options.campaigns)
    parser.add_argument("--base-passages", type=int, default=Options.base_passages)
    parser.add_argument("--batch-passages", type=int, default=Options.batch_passages)
    parser.add_argument("--batches", type=int, default=Options.batches)
    parser.add_argument(
        "--passages-per-document", type=int, default=Options.passages_per_document
    )
    parser.add_argument("--queries", type=int, default=Options.queries)
    parser.add_argument("--clients", type=int, default=Options.clients)
    parser.add_argument("--k", type=int, default=Options.k)
    parser.add_argument("--warmup", type=float, default=Options.warmup)
    parser.add_argument("--rest", type=float, default=Options.rest)
    parser.add_argument("--job-timeout", type=float, default=Options.job_timeout)
    parser.add_argument(
        "--keep", action="store_true", help="keep the temporary directory of each campaign"
    )
    arguments = parser.parse_args(argv)
    return Options(
        binary=Path(arguments.bin),
        out=Path(arguments.out),
        campaigns=arguments.campaigns,
        base_passages=arguments.base_passages,
        batch_passages=arguments.batch_passages,
        batches=arguments.batches,
        passages_per_document=arguments.passages_per_document,
        queries=arguments.queries,
        clients=arguments.clients,
        k=arguments.k,
        warmup=arguments.warmup,
        rest=arguments.rest,
        job_timeout=arguments.job_timeout,
        keep=arguments.keep,
    )


def main(argv: Sequence[str] | None = None) -> int:
    """Runs the campaigns, writes the raw JSON and prints the summary."""
    options = parse(argv)
    if options.passages_per_document <= 0:
        print("--passages-per-document must be strictly positive", file=sys.stderr)
        return 2
    options.out.mkdir(parents=True, exist_ok=True)
    summary: list[dict[str, Any]] = []
    for number in range(1, options.campaigns + 1):
        document = campaign(options, number)
        raw = options.out / f"campaign-{number}.json"
        raw.write_text(
            json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        print(report(document), flush=True)
        summary.append(
            {
                "campaign": number,
                "file": raw.name,
                "passed": document["criterion"]["passed"],
                "criterion": document["criterion"],
                "phases": {
                    phase: entry["wall_ms"]
                    for phase, entry in document["phases"].items()
                    if entry["wall_ms"] is not None
                },
            }
        )
    (options.out / "summary.json").write_text(
        json.dumps({"campaigns": summary}, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    passed = all(entry["passed"] for entry in summary)
    print(f"{'PASS' if passed else 'FAIL'}: {len(summary)} campaign(s)", flush=True)
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
