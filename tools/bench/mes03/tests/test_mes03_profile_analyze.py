# SPDX-License-Identifier: Apache-2.0
"""Tests of the sampling-profile analyzer of MES-03 (ADR-0079, decision 1)."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

from mes03_profile_analyze import analyze, attribute, group, parse_call_graph

# A report with the geometry of `sample`: the art of a frame extends the art of
# its parent by one two-character cell, `"  "` for a first child or a child whose
# cell is blank, `"+ "` for a later sibling. `ROWS` is written as a tree so the
# art is generated, never typed: a hand-typed art can be geometrically invalid
# and would test nothing.
ROWS = [
    ("thread", None, "", 1000, "Thread_1  DispatchQueue_1: com.apple.main-thread  (serial)"),
    ("start", "thread", "  ", 1000, "start"),
    ("main", "start", "  ", 1000, "main"),
    ("run", "main", "  ", 600, "run"),
    ("rescore", "run", "  ", 600, "DenseSearcher7rescore"),
    ("rpk", "rescore", "  ", 400, "read_passage_key"),
    ("advance", "rpk", "  ", 300, "advance"),
    ("step", "advance", "  ", 300, "sqlite3_step"),
    ("gpn", "step", "  ", 290, "getPageNormal"),
    ("pread", "gpn", "  ", 280, "pread"),
    ("tiny", "step", "  ", 5, "tiny_size"),
    ("prepare", "rpk", "+ ", 90, "prepare_with_flags"),
    ("reset", "rpk", "+ ", 10, "sqlite3_reset"),
    ("keyof", "rescore", "+ ", 100, "7key_of"),
    ("display", "keyof", "  ", 100, "Display3fmt"),
    ("vector", "rescore", "+ ", 85, "10vector_f16"),
    ("dot", "rescore", "+ ", 10, "neon3dot"),
    ("sort", "rescore", "+ ", 5, "sort8unstable"),
    ("outside", "main", "+ ", 400, "outside"),
    ("thread2", None, "", 200, "Thread_2  (serial)"),
    ("worker", "thread2", "  ", 200, "worker"),
]


def render(rows) -> str:
    """Renders `ROWS` as the call graph of a `sample` report."""
    arts = {}
    lines = ["Analysis of sampling rescore_profile (pid 1) every 1 millisecond", "", "Call graph:"]
    for key, parent, cell, count, symbol in rows:
        art = "" if parent is None else arts[parent] + cell
        arts[key] = art
        lines.append(f"{art}{count} {symbol}  (in rescore_profile)  [0x1]")
    lines += [
        "",
        "Total number in stack (recursive counted multiple, when >=5):",
        "",
        "Sort by top of stack, same collapsed (when >= 5):",
        "        pread  (in libsystem_kernel.dylib)        280",
        "",
        "Binary Images:",
    ]
    return "\n".join(lines) + "\n"


REPORT = render(ROWS)


def find(nodes, needle):
    """Index of the first frame whose symbol holds `needle`."""
    for index, node in enumerate(nodes):
        if needle in node.symbol:
            return index
    raise AssertionError(f"no frame holds {needle!r}")


def test_the_tree_is_rebuilt_by_textual_prefix():
    nodes = parse_call_graph(REPORT)
    rescore = find(nodes, "7rescore")
    # The first child gains two spaces, the later siblings a marker cell, and a
    # frame with a blank cell is a child all the same: all three cases are here.
    for needle in ("read_passage_key", "key_of", "vector_f16"):
        child = find(nodes, needle)
        assert nodes[child].parent == rescore, needle
    assert nodes[find(nodes, "advance")].parent == find(nodes, "read_passage_key")
    assert nodes[find(nodes, "getPageNormal")].parent == find(nodes, "sqlite3_step")
    assert nodes[find(nodes, "pread")].parent == find(nodes, "getPageNormal")
    assert nodes[find(nodes, "tiny_size")].parent == find(nodes, "sqlite3_step")
    assert nodes[find(nodes, "prepare_with_flags")].parent == find(nodes, "read_passage_key")
    assert nodes[rescore].parent == find(nodes, "run")
    assert nodes[find(nodes, "Display3fmt")].parent == find(nodes, "7key_of")
    # The second thread is a root of its own, never a child of the first.
    assert nodes[find(nodes, "worker")].parent == find(nodes, "Thread_2")


def test_the_self_samples_subtract_every_child():
    nodes = parse_call_graph(REPORT)
    rescore = nodes[find(nodes, "7rescore")]
    assert rescore.count == 600
    assert rescore.self_samples == 600 - (400 + 100 + 85 + 10 + 5)
    assert nodes[find(nodes, "pread")].self_samples == 280
    assert nodes[find(nodes, "getPageNormal")].self_samples == 10


def test_every_sample_of_the_busiest_thread_is_attributed():
    nodes = parse_call_graph(REPORT)
    result = attribute(nodes, 100.0)
    assert result["threads"] == 2
    assert result["total_samples"] == 1000
    assert result["unattributed_samples"] == 0
    assert sum(post["samples"] for post in result["posts"]) == 1000


def test_the_asked_posts_carry_the_expected_shares():
    nodes = parse_call_graph(REPORT)
    result = attribute(nodes, 100.0)
    posts = {post["post"]: post for post in result["posts"]}
    assert posts["key read: page reads (b-tree, pread)"]["samples"] == 290
    assert posts["key read: statement preparation"]["samples"] == 90
    assert posts["key read: reset and finalize"]["samples"] == 10
    # The five samples of the allocator under `sqlite3_step` belong to the row
    # decoding, as do the five the step keeps for itself.
    assert posts["key read: row decoding and other"]["samples"] == 10
    assert posts["key read: key formatting and String"]["samples"] == 100
    assert posts["vector copy and decode (f16)"]["samples"] == 85
    assert posts["dot product"]["samples"] == 10
    assert posts["sort"]["samples"] == 5
    # The 400 samples outside the rescoring are the driver, not the loop.
    assert posts["outside the rescoring"]["samples"] == 400


def test_milliseconds_scale_the_share_by_the_runs_p50():
    nodes = parse_call_graph(REPORT)
    result = attribute(nodes, 50.0)
    posts = {post["post"]: post for post in result["posts"]}
    page_reads = posts["key read: page reads (b-tree, pread)"]
    assert page_reads["share"] == pytest.approx(0.29, abs=1e-6)
    assert page_reads["ms_p50"] == pytest.approx(14.5, abs=1e-6)


def test_the_roll_up_gathers_every_layer_of_the_key_read():
    nodes = parse_call_graph(REPORT)
    result = attribute(nodes, 100.0)
    groups = group(result["posts"])
    assert groups["key read"]["samples"] == 500
    assert groups["key read"]["share"] == pytest.approx(0.5, abs=1e-6)
    assert groups["vector copy and decode (f16)"]["samples"] == 85
    assert groups["dot product"]["samples"] == 10


def test_the_report_is_written_with_the_run_parameters(tmp_path: Path):
    sample = tmp_path / "sample.txt"
    sample.write_text(REPORT, encoding="utf-8")
    run = tmp_path / "run.json"
    run.write_text(
        json.dumps({"n": 1000, "k": 100, "ms": {"p50": 42.0, "p95": 60.0}, "iterations": 10}),
        encoding="utf-8",
    )
    out = tmp_path / "out.json"
    script = Path(__file__).resolve().parent.parent / "mes03_profile_analyze.py"
    subprocess.run(
        [sys.executable, str(script), "--sample", str(sample), "--run", str(run), "--out", str(out)],
        check=True,
        capture_output=True,
    )
    report = json.loads(out.read_text(encoding="utf-8"))
    assert report["run"]["n"] == 1000
    assert report["run"]["ms_p50"] == 42.0
    assert report["total_samples"] == 1000
    assert report["posts"][0]["ms_p50"] is not None
    # A document without a p50 keeps null milliseconds instead of failing.
    run.write_text(json.dumps({"n": 1000}), encoding="utf-8")
    subprocess.run(
        [sys.executable, str(script), "--sample", str(sample), "--run", str(run), "--out", str(out)],
        check=True,
        capture_output=True,
    )
    report = json.loads(out.read_text(encoding="utf-8"))
    assert all(post["ms_p50"] is None for post in report["posts"])


def test_an_empty_report_attributes_nothing():
    result = analyze("", {})
    assert result["total_samples"] == 0
    assert result["posts"] == []


def test_a_cold_run_reports_no_warm_up_pass():
    result = analyze("", {"n": 1000, "ms": {"p50": 1.0}})
    assert result["run"]["warmup_passes"] == 0
    assert result["run"]["warmup_pass_ms"] == []


def test_a_warm_run_keeps_the_p50_of_every_warm_up_pass():
    result = analyze("", {"n": 1000, "ms": {"p50": 1.0}, "warmup_passes": 3, "warmup_pass_ms": [9.0, 5.0, 4.0]})
    assert result["run"]["warmup_passes"] == 3
    assert result["run"]["warmup_pass_ms"] == [9.0, 5.0, 4.0]
