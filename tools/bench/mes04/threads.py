# SPDX-License-Identifier: Apache-2.0

"""Explore ONNX Runtime session settings for MES-05, outside the Rust code.

The Rust encoder builds its sessions with the defaults of ``ort``: no thread
count and no optimization level. This script measures the same INT8 and fp32
graphs with ``onnxruntime`` 1.31.0 in Python, varying the intra-op thread count
and the graph optimization level, so the report can recommend a setting without
changing `crates/` during a measurement session.

It times ``session.run`` only, with the queries already tokenized: the
comparison is between session settings, not an absolute encode latency (the Rust
harness of ``measure_queries`` measures the whole ``encode_query``, tokenizer
included).

Run from ``tools/``::

    uv run python bench/mes04/threads.py
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

import numpy as np

#: Repository root, three levels above this file.
ROOT = Path(__file__).resolve().parents[3]
DEFAULT_MODEL = ROOT / "data/models/mdenseon"
DEFAULT_QUERIES = ROOT / "data/mes04/query-texts.jsonl"
DEFAULT_OUT = ROOT / "data/mes04/threads.json"
#: Prompt of the query side (``model.json``).
PROMPT = "query: "


def load_queries(path: Path) -> list[str]:
    texts = []
    with path.open(encoding="utf-8") as handle:
        for line in handle:
            if line.strip():
                texts.append(json.loads(line)["text"])
    if not texts:
        raise ValueError(f"{path} holds no query")
    return texts


def settings():
    """Returns the ``(label, threads, optimization level name)`` combinations."""
    return (
        ("default", 0, "all"),
        ("threads-8", 8, "all"),
        ("threads-4", 4, "all"),
        ("threads-1", 1, "all"),
        ("basic", 0, "basic"),
    )


def measure(model_path: Path, feeds: list[dict], threads: int, level: str, count: int):
    """Times ``count`` runs and returns their durations in milliseconds."""
    import onnxruntime as ort

    options = ort.SessionOptions()
    options.intra_op_num_threads = threads
    levels = {
        "all": ort.GraphOptimizationLevel.ORT_ENABLE_ALL,
        "basic": ort.GraphOptimizationLevel.ORT_ENABLE_BASIC,
    }
    options.graph_optimization_level = levels[level]
    session = ort.InferenceSession(
        str(model_path), sess_options=options, providers=["CPUExecutionProvider"]
    )
    for index in range(min(100, count)):
        session.run(None, feeds[index % len(feeds)])
    durations = []
    for index in range(count):
        started = time.perf_counter()
        session.run(None, feeds[index % len(feeds)])
        durations.append((time.perf_counter() - started) * 1_000.0)
    del session
    return durations


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--model", type=Path, default=DEFAULT_MODEL)
    parser.add_argument("--queries", type=Path, default=DEFAULT_QUERIES)
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    parser.add_argument("--count", type=int, default=1_000)
    arguments = parser.parse_args(argv)

    try:
        import onnxruntime as ort
        from tokenizers import Tokenizer

        texts = load_queries(arguments.queries)
        tokenizer = Tokenizer.from_file(str(arguments.model / "tokenizer.json"))
        tokenizer.no_truncation()
        feeds = []
        for text in texts:
            ids = np.asarray(
                [tokenizer.encode(PROMPT + text, add_special_tokens=True).ids],
                dtype=np.int64,
            )
            feeds.append(
                {
                    "input_ids": ids,
                    "attention_mask": np.ones_like(ids, dtype=np.int64),
                }
            )

        # ``analysis`` lives next to this script; import it lazily so the help
        # still works from another directory.
        sys.path.insert(0, str(Path(__file__).resolve().parent))
        import analysis

        results = {}
        for precision, name in (("int8", "model.int8.onnx"), ("fp32", "model.onnx")):
            results[precision] = {}
            for label, threads, level in settings():
                durations = measure(
                    arguments.model / name, feeds, threads, level, arguments.count
                )
                results[precision][label] = {
                    "intra_op_num_threads": threads,
                    "graph_optimization_level": level,
                    "count": arguments.count,
                    "session_run_ms": analysis.summarise(durations),
                }
                print(
                    f"{precision} {label}: p50 "
                    f"{results[precision][label]['session_run_ms']['p50']:.3f} ms, "
                    f"p95 {results[precision][label]['session_run_ms']['p95']:.3f} ms",
                    file=sys.stderr,
                )
        payload = {
            "onnxruntime": ort.__version__,
            "queries": len(texts),
            "runs_per_setting": arguments.count,
            "tokenizer": "applied outside the timed loop",
            "results": results,
        }
        arguments.out.parent.mkdir(parents=True, exist_ok=True)
        arguments.out.write_text(
            json.dumps(payload, indent=2) + "\n", encoding="utf-8"
        )
    except Exception as error:  # noqa: BLE001 - every failure becomes one line
        print(f"error: {' '.join(str(error).split())}", file=sys.stderr)
        return 1
    print(f"wrote {arguments.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
