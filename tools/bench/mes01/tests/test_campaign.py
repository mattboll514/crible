# SPDX-License-Identifier: Apache-2.0

"""Tests of the run bookkeeping of MES-01 (ADR-0076, decision 4).

A partial replay must not erase the runs already recorded, and the count of
accepted query-plans must follow the merged file, so the artefact always
describes every engine output it aggregates.
"""

from __future__ import annotations

import json

import mes01_campaign as campaign


def entry(campaign_name: str, structure: str, run: str, queries: int = 10) -> dict:
    return {
        "campaign": campaign_name,
        "structure": structure,
        "run": run,
        "strategy": "exact" if run == "exact" else "binary",
        "queries": queries,
        "pool_sizes": [100],
    }


def test_merge_runs_keeps_the_recorded_runs(tmp_path):
    path = tmp_path / "runs.json"
    path.write_text(
        json.dumps(
            {
                "runs": [
                    entry("main", "articles", "exact"),
                    entry("main", "articles", "n200"),
                ],
                "strategy_control": {},
            }
        )
    )
    merged = campaign.merge_runs(path, [entry("filtered", "decisions", "exact", 7)])
    assert [(item["campaign"], item["structure"], item["run"]) for item in merged] == [
        ("main", "articles", "exact"),
        ("main", "articles", "n200"),
        ("filtered", "decisions", "exact"),
    ]


def test_merge_runs_replaces_the_replayed_run(tmp_path):
    path = tmp_path / "runs.json"
    path.write_text(json.dumps({"runs": [entry("main", "articles", "n200", 3)]}))
    merged = campaign.merge_runs(path, [entry("main", "articles", "n200", 5)])
    assert len(merged) == 1
    assert merged[0]["queries"] == 5


def test_merge_runs_reads_a_bare_list(tmp_path):
    # The first campaign wrote a list; the file must stay readable.
    path = tmp_path / "runs.json"
    path.write_text(json.dumps([entry("main", "articles", "exact", 2)]))
    merged = campaign.merge_runs(path, [])
    assert merged[0]["strategy"] == "exact"


def test_merge_runs_orders_exact_before_the_values_of_n():
    runs = [
        entry("main", "articles", "n500"),
        entry("main", "articles", "exact"),
        entry("main", "articles", "n200"),
    ]
    assert [item["run"] for item in sorted(runs, key=campaign.run_order)] == [
        "exact",
        "n200",
        "n500",
    ]


def test_strategy_control_counts_the_query_plans_per_campaign():
    control = campaign.strategy_control(
        [
            entry("main", "articles", "exact", 63),
            entry("main", "articles", "n200", 63),
            entry("main", "articles", "n500", 63),
            entry("filtered", "decisions", "exact", 200),
            entry("filtered", "decisions", "n200", 200),
        ]
    )
    assert control["binary_query_runs"] == 326
    assert control["exact_query_runs"] == 263
    assert control["campaigns"]["main"] == {
        "binary_query_runs": 126,
        "exact_query_runs": 63,
    }
    assert control["campaigns"]["filtered"] == {
        "binary_query_runs": 200,
        "exact_query_runs": 200,
    }
