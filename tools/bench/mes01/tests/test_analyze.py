# SPDX-License-Identifier: Apache-2.0

"""Tests of the aggregation of MES-01 and MES-12.

ADR-0076, decision 3: the centring figures are only kept when the harness
reproduces `codes.bin` and the engine's top-10. The aggregator must therefore
refuse to write a table from a failed control, rather than write it with a flag
nobody reads.
"""

from __future__ import annotations

import pytest

import mes01_analyze as analyze


def block(*, control_ok: bool = True, mismatched_words: int = 0) -> dict:
    """The fields `centring_tables` reads, with one perfect query."""
    ns = (200,)
    return {
        "passages": 1,
        "pool": 1,
        "control_ok": control_ok,
        "codes_control": {"mismatched_words": mismatched_words},
        "control_comparisons": 5,
        "mean_norm": 0.0,
        "residuals": [],
        "queries": {
            "Q1": {
                variant: {
                    str(n): {"recall": 1.0, "overlap": 1.0} for n in ns
                }
                for variant in ("raw", "a", "c")
            }
        },
    }


def test_centring_tables_refuses_a_failed_top_ten_control():
    centering = {"campaigns": {"main": {"articles": block(control_ok=False)}}}
    with pytest.raises(SystemExit):
        analyze.centring_tables(centering, "main", [200])


def test_centring_tables_refuses_mismatched_codes():
    centering = {"campaigns": {"main": {"articles": block(mismatched_words=3)}}}
    with pytest.raises(SystemExit):
        analyze.centring_tables(centering, "main", [200])


def test_centring_tables_reads_a_passing_control():
    centering = {"campaigns": {"main": {"articles": block()}}}
    tables = analyze.centring_tables(centering, "main", [200])
    assert tables["structures"]["articles"]["control_comparisons"] == 5
