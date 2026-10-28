# SPDX-License-Identifier: Apache-2.0
"""Attribute the samples of a `sample(1)` profile of the rescoring.

References: `docs/mesures/MES-03-latence-rescoring.md`, ADR-0079 decision 1
(a sampling profile of the rescoring attributes the time the campaign left
unexplained), ADR-0077 (measurement protocol).

Input: the text report written by `/usr/bin/sample` and the JSON document
written by `crates/crible/examples/rescore_profile.rs` (`run` mode). Output: one
JSON document giving, per post, the number of samples, the share of the
rescoring and the milliseconds at the run's p50.

The post of a sample is decided by its **leaf frame** and by the chain of frames
above it: the deepest marker on the chain names the post, so a page fault taken
inside the SQLite b-tree counts as a key read, not as "kernel". Two levels are
used: the branch of the rescoring (key lookup, vector copy, dot product, sort,
or the rest), and, inside the key lookup, the layer of the cost (statement
preparation, page reads, locks, reset).
"""

from __future__ import annotations

import argparse
import json
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable

# A line of the `sample` call graph:
#   `                  + ! : 25 __func  (in image) + 12  [0x...]  file:line`
# The art (`+`, `!`, `|`, `:`, spaces) is the concatenation of one two-character
# cell per level, so the art of a frame **extends** the art of its parent
# (`"  "`, `"+ "`, `"! "`, `": "`, `"| "`). The tree is therefore rebuilt by
# textual prefix, which needs no assumption about the width of a cell: a first
# child only gains two spaces where a later sibling gains a marker, and a frame
# much deeper than its ancestors gains a long run of blank cells.
_LINE = re.compile(r"^(?P<art>[ +!:|\t]*)(?P<count>\d+) (?P<symbol>\S.*)$")
_IN = re.compile(r"^(?P<symbol>.+?) +\(in (?P<image>[^)]*)\)")


@dataclass
class Node:
    """One frame of the call graph, with its sample count."""

    symbol: str
    count: int
    art: str
    parent: int | None
    self_samples: int = 0
    children: list[int] = field(default_factory=list)


def parse_call_graph(text: str) -> list[Node]:
    """Returns the frames of the report, parents before children."""
    nodes: list[Node] = []
    inside = False
    stack: list[int] = []
    for raw in text.splitlines():
        if raw.strip() == "Call graph:":
            inside = True
            continue
        if not inside:
            continue
        if raw.startswith("Binary Images:") or raw.strip().startswith("Sort by top of stack"):
            break
        if raw.startswith("Total number in stack"):
            continue
        match = _LINE.match(raw)
        if match is None:
            continue
        art = match.group("art")
        rest = match.group("symbol")
        head = _IN.match(rest)
        symbol = head.group("symbol") if head else rest
        while stack:
            above = nodes[stack[-1]].art
            if len(art) > len(above) and art.startswith(above):
                break
            stack.pop()
        parent = stack[-1] if stack else None
        index = len(nodes)
        nodes.append(Node(symbol=symbol, count=int(match.group("count")), art=art, parent=parent))
        if parent is not None:
            nodes[parent].children.append(index)
        stack.append(index)
    for node in nodes:
        below = sum(nodes[child].count for child in node.children)
        node.self_samples = max(node.count - below, 0)
    return nodes


def path_of(nodes: list[Node], index: int) -> list[str]:
    """Symbols of `index` and of its ancestors, deepest first."""
    symbols = []
    while index is not None:
        symbols.append(nodes[index].symbol)
        index = nodes[index].parent
    return symbols


def root_of(nodes: list[Node], index: int) -> int:
    """Index of the outermost ancestor of one frame."""
    while nodes[index].parent is not None:
        index = nodes[index].parent
    return index


# First level: the branch of the rescoring. The deepest match on the chain wins.
_BRANCHES = (
    ("key", ("read_passage_key", "passage_key_by_iid")),
    ("key-format", ("6key_of", "7key_of")),
    ("vector", ("10vector_f16",)),
    ("dot", ("neon3dot", "7kernels")),
    ("sort", ("quicksort", "sort8unstable", "16sort_unstable_by")),
    ("rescore", ("7rescore",)),
)

# Second level, inside the key lookup only.
_LAYERS = (
    ("prepare", ("prepare_with_flags", "sqlite3Prepare", "sqlite3RunParser", "yy_reduce", "sqlite3LockAndPrepare")),
    ("pages", ("sqlite3Btree", "getPage", "getAndInitPage", "moveToChild", "pcache", "pager", "accessPayload", "memjrnl")),
    ("locks", ("unixShmLock", "pthread_mutex", "fcntl", "ndvar")),
    ("reset", ("sqlite3_reset", "sqlite3VdbeReset", "sqlite3VdbeHalt", "vdbeCommit", "sqlite3_finalize")),
)

_LABELS = {
    "key": "key read (SQLite)",
    "key-format": "key read: key formatting and String",
    "vector": "vector copy and decode (f16)",
    "dot": "dot product",
    "sort": "sort",
    "rescore": "other rescoring",
    "outside": "outside the rescoring",
}

_LAYER_LABELS = {
    "prepare": "key read: statement preparation",
    "pages": "key read: page reads (b-tree, pread)",
    "locks": "key read: locks",
    "reset": "key read: reset and finalize",
}


def classify(nodes: list[Node], index: int) -> str:
    """Post of one sample, from its leaf frame and its chain of frames."""
    symbols = path_of(nodes, index)
    branch = "outside"
    position = len(symbols)
    for depth, symbol in enumerate(symbols):
        matched = next((name for name, markers in _BRANCHES if any(marker in symbol for marker in markers)), None)
        if matched is not None:
            branch = matched
            position = depth
            break
    if branch != "key":
        return _LABELS[branch]
    # Only the frames inside the key lookup can name its layer.
    for symbol in symbols[: position + 1]:
        for name, markers in _LAYERS:
            if any(marker in symbol for marker in markers):
                return _LAYER_LABELS[name]
    return "key read: row decoding and other"


def attribute(nodes: list[Node], p50_ms: float | None) -> dict:
    """Aggregates the self samples of the busiest thread by post."""
    roots = [index for index, node in enumerate(nodes) if node.parent is None]
    if not roots:
        return {"threads": 0, "total_samples": 0, "posts": []}
    main = max(roots, key=lambda index: nodes[index].count)
    total = nodes[main].count
    counts: dict[str, int] = {}
    for index, node in enumerate(nodes):
        if root_of(nodes, index) != main or node.self_samples == 0:
            continue
        label = classify(nodes, index)
        counts[label] = counts.get(label, 0) + node.self_samples
    rows = []
    for label, samples in sorted(counts.items(), key=lambda item: item[1], reverse=True):
        share = samples / total if total else 0.0
        rows.append(
            {
                "post": label,
                "samples": samples,
                "share": round(share, 6),
                "ms_p50": None if p50_ms is None else round(share * p50_ms, 4),
            }
        )
    return {
        "threads": len(roots),
        "main_thread": nodes[main].symbol,
        "total_samples": total,
        "attributed_samples": sum(counts.values()),
        "unattributed_samples": total - sum(counts.values()),
        "posts": rows,
    }


def group(posts: Iterable[dict]) -> dict:
    """Roll-up of the posts by branch, which the report quotes."""
    samples: dict[str, int] = {}
    for post in posts:
        samples[post["post"]] = post["samples"]
    total = sum(samples.values())
    keys = 0
    for label, count in samples.items():
        if label.startswith("key"):
            keys += count
    rolled = {
        "key read": keys,
        "vector copy and decode (f16)": samples.get("vector copy and decode (f16)", 0),
        "dot product": samples.get("dot product", 0),
        "sort": samples.get("sort", 0),
        "other rescoring": samples.get("other rescoring", 0),
        "outside the rescoring": samples.get("outside the rescoring", 0),
    }
    return {
        label: {"samples": count, "share": round(count / total, 6) if total else 0.0}
        for label, count in rolled.items()
    }


def analyze(sample_text: str, run: dict) -> dict:
    """Full report: parse, attribute, then scale to the run's p50."""
    nodes = parse_call_graph(sample_text)
    milliseconds = run.get("ms") or {}
    p50 = milliseconds.get("p50") if isinstance(milliseconds, dict) else None
    p50 = None if p50 is None else float(p50)
    result = attribute(nodes, p50)
    result["groups"] = group(result["posts"])
    result["run"] = {
        "n": run.get("n"),
        "k": run.get("k"),
        "queries": run.get("queries"),
        "iterations": run.get("iterations"),
        "ms_p50": p50,
        "ms_p95": milliseconds.get("p95") if isinstance(milliseconds, dict) else None,
        "candidates_min": run.get("candidates_min"),
        "candidates_max": run.get("candidates_max"),
        # Warm series (ADR-0077): the untimed warm-up passes and the p50 of each
        # one say which state the point describes. A cold run reports none.
        "warmup_passes": run.get("warmup_passes", 0),
        "warmup_pass_ms": run.get("warmup_pass_ms", []),
    }
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sample", required=True, type=Path, help="text report of /usr/bin/sample")
    parser.add_argument("--run", required=True, type=Path, help="JSON document of rescore_profile run")
    parser.add_argument("--out", required=True, type=Path, help="JSON document to write")
    arguments = parser.parse_args()
    sample_text = arguments.sample.read_text(encoding="utf-8", errors="replace")
    run = json.loads(arguments.run.read_text(encoding="utf-8"))
    report = analyze(sample_text, run)
    report["sample_file"] = str(arguments.sample)
    report["run_file"] = str(arguments.run)
    arguments.out.parent.mkdir(parents=True, exist_ok=True)
    arguments.out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(
        "profile: {total} samples, {key:.1f} % key reads, {vector:.1f} % vector copies, "
        "{dot:.1f} % dot, {sort:.1f} % sort".format(
            total=report["total_samples"],
            key=100.0 * report["groups"]["key read"]["share"],
            vector=100.0 * report["groups"]["vector copy and decode (f16)"]["share"],
            dot=100.0 * report["groups"]["dot product"]["share"],
            sort=100.0 * report["groups"]["sort"]["share"],
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
