# SPDX-License-Identifier: Apache-2.0

"""Tests of the aggregation of the alternating campaign of MES-02.

They pin the sign of the relative gap (negative means the sparse traversal is
faster), the dispersion definition (max / min − 1, the one MES-02 used between
two runs), and the refusal of a raw document whose summary does not match its
own durations.
"""

from __future__ import annotations

import json

import pytest

import mes02_analyze as analyze
import mes02_parcours_analyze as parcours


def series(count: int, start: float = 1.0) -> list[float]:
    """`count` increasing durations, so the nearest rank is readable by eye."""
    return [start + index for index in range(count)]


def point(
    *,
    label: str = "density-1000000-50",
    kind: str = "density",
    dense: list[float] | None = None,
    sparse: list[float] | None = None,
    pairs: list[tuple[list[float], list[float]]] | None = None,
) -> dict:
    """One raw point, summarised the way the harness writes it."""
    pairs = pairs or [(dense or series(10), sparse or series(10, 2.0))]
    return {
        "label": label,
        "kind": kind,
        "requested_span": 1_000_000,
        "size": 500_000,
        "span": 1_000_000,
        "density": 0.5,
        "n": 5000,
        "first_iid": 100,
        "last_iid": 1_000_099,
        "traversal_automatic": "dense",
        "plan": "binary",
        "pairs_count": len(pairs),
        "queries": len(pairs[0][0]),
        "warmup": {
            "dense": {"summary": analyze.summarise(series(10)), "durations_us": series(10)},
            "sparse": {
                "summary": analyze.summarise(series(10, 2.0)),
                "durations_us": series(10, 2.0),
            },
        },
        "pairs": [
            {
                "pair": index + 1,
                "dense": analyze.summarise(dense_pair),
                "sparse": analyze.summarise(sparse_pair),
                "dense_us": dense_pair,
                "sparse_us": sparse_pair,
            }
            for index, (dense_pair, sparse_pair) in enumerate(pairs)
        ],
        "dense": {
            "durations_us": [value for dense_pair, _ in pairs for value in dense_pair],
            "summary": analyze.summarise([value for dense_pair, _ in pairs for value in dense_pair]),
        },
        "sparse": {
            "durations_us": [value for _, sparse_pair in pairs for value in sparse_pair],
            "summary": analyze.summarise(
                [value for _, sparse_pair in pairs for value in sparse_pair]
            ),
        },
    }


def test_percent_is_negative_when_the_sparse_traversal_is_faster():
    assert parcours.percent(80.0, 100.0) == pytest.approx(-20.0)
    assert parcours.percent(120.0, 100.0) == pytest.approx(20.0)


def test_percent_refuses_a_zero_reference():
    with pytest.raises(ValueError):
        parcours.percent(1.0, 0.0)


def test_spread_is_max_over_min_minus_one():
    # The reference of MES-02: 46,469 / 40,037 = 1,161, so 16,1 %.
    assert parcours.spread([40.037, 46.469]) == pytest.approx(16.065, abs=0.001)
    assert parcours.spread([5.0, 5.0, 5.0]) == 0.0


def test_spread_refuses_an_empty_or_non_positive_series():
    with pytest.raises(ValueError):
        parcours.spread([])
    with pytest.raises(ValueError):
        parcours.spread([0.0, 1.0])


def test_point_row_keeps_the_per_pair_gap_and_the_dispersion():
    first = (series(10, 10.0), series(10, 8.0))
    second = (series(10, 12.0), series(10, 8.0))
    row = parcours.point_row(point(pairs=[first, second]), "cross-n5000")
    assert row["campaign"] == "cross-n5000"
    gaps = [pair["sparse_minus_dense_percent"] for pair in row["pairs"]]
    # p50 of series(10, 10) is 14 (rank 5), of series(10, 8) is 12, of
    # series(10, 12) is 16.
    assert gaps[0] == pytest.approx((12.0 - 14.0) / 14.0 * 100.0)
    assert gaps[1] == pytest.approx((12.0 - 16.0) / 16.0 * 100.0)
    assert row["p50_spread"]["dense_percent"] == pytest.approx((16.0 / 14.0 - 1.0) * 100.0)
    assert row["p50_spread"]["sparse_percent"] == pytest.approx(0.0)
    gaps_range = row["p50_spread"]["gap_points"]
    assert gaps_range == pytest.approx(abs(gaps[1] - gaps[0]))
    assert row["queries"] == 10
    assert row["pairs_count"] == 2


def test_point_row_refuses_a_summary_that_disagrees_with_the_durations():
    raw = point()
    raw["dense"]["summary"]["p50"] = 1.0
    with pytest.raises(ValueError):
        parcours.point_row(raw, "cross-n5000")


def test_rows_refuse_an_empty_raw_directory(tmp_path):
    with pytest.raises(FileNotFoundError):
        parcours.rows(tmp_path)


def test_build_writes_a_dated_artefact(tmp_path):
    raw = tmp_path / "raw"
    raw.mkdir()
    (tmp_path / "queries.json").write_text(
        json.dumps(
            {
                "queries": 10,
                "sha256": "00",
                "law": "standard normal",
                "seed": 1,
                "stream": 7,
            }
        ),
        encoding="utf-8",
    )
    document = {
        "kind": "cross",
        "index": "synth-large",
        "dims": 768,
        "threads": 8,
        "live_passages": 7_740_000,
        "vectors": "data/mes02/queries.f32",
        "queries": 10,
        "points": [point()],
    }
    (raw / "cross-n5000.json").write_text(json.dumps(document), encoding="utf-8")
    agree = {"kind": "agree", "identical": True, "points": [control()]}
    (raw / "agree-n5000.json").write_text(json.dumps(agree), encoding="utf-8")
    artefacts = tmp_path / "mesures"
    built = parcours.build(raw, commit="da34f70", date="2026-10-27", machine="test")
    assert built["protocol"].startswith("ADR-0077")
    assert built["dataset"]["live_passages"] == 7_740_000
    assert built["rows"][0]["density"] == 0.5
    assert built["documents"] == ["cross-n5000"]
    assert built["controls"]["pools_checked"] == 1
    assert built["controls"]["identical"] is True
    written = parcours.write(built, artefacts, "2026-10-27")
    assert written.name == "MES-02-parcours-dense-clairseme.2026-10-27.json"
    assert json.loads(written.read_text(encoding="utf-8"))["measure"] == "MES-02"


def control(size: int = 500_000, identical: bool = True, n: int = 5000) -> dict:
    """One point of the agreement control, as the harness writes it."""
    return {
        "label": "density-1000000-50",
        "kind": "density",
        "size": size,
        "span": 1_000_000,
        "density": 0.5,
        "n": n,
        "queries_checked": 5,
        "identical": identical,
        "queries_different": 0 if identical else 1,
        "first_difference": None,
    }


def test_controls_refuse_a_campaign_without_control(tmp_path):
    with pytest.raises(FileNotFoundError):
        parcours.control_rows(tmp_path)


def test_controls_summarise_the_pools_and_refuse_a_disagreement(tmp_path):
    document = {
        "kind": "agree",
        "identical": True,
        "points": [control(), control(size=900_000, n=1000)],
    }
    (tmp_path / "agree-n5000.json").write_text(json.dumps(document), encoding="utf-8")
    summary = parcours.control_rows(tmp_path)
    assert summary["pools_checked"] == 2
    assert summary["queries_checked"] == 10
    assert summary["identical"] is True

    document["points"][1]["identical"] = False
    (tmp_path / "agree-n5000.json").write_text(json.dumps(document), encoding="utf-8")
    with pytest.raises(ValueError):
        parcours.control_rows(tmp_path)


def test_controls_refuse_a_document_that_says_the_traversals_disagree(tmp_path):
    document = {"kind": "agree", "identical": False, "points": [control()]}
    (tmp_path / "agree-n1000.json").write_text(json.dumps(document), encoding="utf-8")
    with pytest.raises(ValueError):
        parcours.control_rows(tmp_path)


def test_build_refuses_an_absolute_path_outside_the_repository(tmp_path):
    raw = tmp_path / "raw"
    raw.mkdir()
    (tmp_path / "queries.json").write_text(
        json.dumps({"queries": 10, "sha256": "00", "law": "x", "seed": 1, "stream": 7}),
        encoding="utf-8",
    )
    document = {
        "kind": "cross",
        "index": "synth-large",
        "dims": 768,
        "threads": 8,
        "live_passages": 7_740_000,
        "vectors": "/elsewhere/queries.f32",
        "queries": 10,
        "points": [point()],
    }
    (raw / "cross-n5000.json").write_text(json.dumps(document), encoding="utf-8")
    with pytest.raises(ValueError):
        parcours.build(raw, commit="da34f70", date="2026-10-27", machine="test")
