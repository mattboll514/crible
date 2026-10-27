# SPDX-License-Identifier: Apache-2.0

"""Tests of the aggregation of the multi-segment campaign of MES-02.

They pin what makes the campaign readable at all: the order of the pairs must
alternate (ADR-0081, decision 3), a document must carry the number of segments of
the copy it claims, the agreement control is a refusal, and the cost of the
number of segments is read at equal size and span.
"""

from __future__ import annotations

import json
import os

import pytest

import mes02_analyze as analyze
import mes02_multiseg_analyze as multiseg


def series(count: int, start: float = 1.0) -> list[float]:
    """`count` increasing durations, so the nearest rank is readable by eye."""
    return [start + index for index in range(count)]


def point(
    *,
    label: str = "density-1000000-50",
    kind: str = "density",
    size: int = 500_000,
    span: int = 1_000_000,
    first_iid: int = 100,
    n: int = 5000,
    dense: list[float] | None = None,
    sparse: list[float] | None = None,
    orders: list[str] | None = None,
) -> dict:
    """One raw point, summarised and ordered the way the harness writes it."""
    dense = dense or series(10)
    sparse = sparse or series(10, 2.0)
    orders = orders or ["dense-then-sparse", "sparse-then-dense", "dense-then-sparse"]
    pairs = [(dense, sparse) for _ in orders]
    return {
        "label": label,
        "kind": kind,
        "requested_span": span,
        "size": size,
        "span": span,
        "density": size / span,
        "n": n,
        "first_iid": first_iid,
        "last_iid": first_iid + span - 1,
        "traversal_automatic": "sparse",
        "plan": "binary",
        "pairs_count": len(pairs),
        "queries": 10,
        "warmup": None,
        "pairs": [
            {
                "pair": number + 1,
                "order": order,
                "dense": analyze.summarise(dense_pair),
                "sparse": analyze.summarise(sparse_pair),
                "dense_us": dense_pair,
                "sparse_us": sparse_pair,
            }
            for number, (order, (dense_pair, sparse_pair)) in enumerate(zip(orders, pairs))
        ],
        "dense": {
            "durations_us": [value for dense_pair, _ in pairs for value in dense_pair],
            "summary": analyze.summarise(
                [value for dense_pair, _ in pairs for value in dense_pair]
            ),
        },
        "sparse": {
            "durations_us": [value for _, sparse_pair in pairs for value in sparse_pair],
            "summary": analyze.summarise(
                [value for _, sparse_pair in pairs for value in sparse_pair]
            ),
        },
    }


def cross_document(index: str, segments: int, points: list[dict]) -> dict:
    return {
        "kind": "cross",
        "index": index,
        "dims": 768,
        "threads": 8,
        "live_passages": 7_740_000,
        "segments": segments,
        "segment_counts": [1_000_000] * segments,
        "segment_iid_starts": [1_000_000 * number for number in range(segments)],
        "vectors": "data/mes02/queries.f32",
        "queries": 10,
        "points": points,
    }


def agree_document(index: str, segments: int, *, identical: bool = True) -> dict:
    return {
        "kind": "agree",
        "index": index,
        "dims": 768,
        "threads": 8,
        "live_passages": 7_740_000,
        "segments": segments,
        "vectors": "data/mes02/queries.f32",
        "queries_checked": 5,
        "pools_checked": 1,
        "identical": identical,
        "points": [
            {
                "label": "full",
                "kind": "full",
                "size": 7_740_000,
                "span": 8_600_000,
                "density": 0.9,
                "n": 5000,
                "queries_checked": 5,
                "identical": identical,
                "queries_different": 0 if identical else 1,
            }
        ],
    }


def raw_directory(tmp_path, *, segments=(1, 8, 100)):
    """A raw directory with one copy per number of segments, plus the repeat."""
    raw = tmp_path / "raw"
    raw.mkdir()
    (tmp_path / "queries.json").write_text(
        json.dumps(
            {
                "sha256": "00",
                "law": "normal",
                "seed": 20261021,
                "stream": 7,
            }
        ),
        encoding="utf-8",
    )
    labels = {1: "synth-large", 8: "synth-large-8seg", 100: "synth-large-multi"}
    for number, campaign in labels.items():
        if number not in segments:
            continue
        document = cross_document(campaign, number, [point()])
        (raw / f"cross-{campaign}.json").write_text(json.dumps(document), encoding="utf-8")
        agree = agree_document(campaign, number)
        (raw / f"agree-{campaign}.json").write_text(json.dumps(agree), encoding="utf-8")
    repeat = cross_document("synth-large", 1, [point()])
    (raw / "cross-synth-large-repeat.json").write_text(json.dumps(repeat), encoding="utf-8")
    return raw


def durations(p50: float, count: int = 10) -> list[float]:
    """`count` durations whose nearest-rank p50 is exactly `p50`."""
    return [p50 + index - count // 2 + 1 for index in range(count)]


def interleaved_point(
    *,
    dense: float = 1000.0,
    sparse: float = 900.0,
    label: str = "full",
    size: int = 7_740_000,
    span: int = 7_740_000,
) -> dict:
    """One raw point of the interleaved series, with chosen p50 per traversal."""
    return point(
        label=label,
        kind="full",
        size=size,
        span=span,
        dense=durations(dense),
        sparse=durations(sparse),
    )


def write_interleaved(
    raw,
    rounds: list[list[tuple[str, float, float]]],
    *,
    segments: dict[str, int] | None = None,
) -> None:
    """Writes the rounds in the given order, with increasing write times.

    The write time is what the analyzer reads the order from, so the fixture has
    to set it: writing the files in order would give the same thing here, but
    the file systems do not guarantee a one-second granularity.
    """
    counts = segments or {"synth-large": 1, "synth-large-8seg": 8, "synth-large-multi": 100}
    stamp = 1_000_000.0
    for number, entries in enumerate(rounds, start=1):
        for campaign, dense, sparse in entries:
            document = cross_document(
                campaign,
                counts[campaign],
                [interleaved_point(dense=dense, sparse=sparse)],
            )
            path = raw / f"cross-{campaign}-r{number}.json"
            path.write_text(json.dumps(document), encoding="utf-8")
            stamp += 10.0
            os.utime(path, (stamp, stamp))


#: A Latin square: every copy sits at each position exactly once. Each entry is
#: `(copy, dense p50, sparse p50)`, and the copies are written in that order.
LATIN_SQUARE = [
    [
        ("synth-large", 1000.0, 1000.0),
        ("synth-large-8seg", 1000.0, 1040.0),
        ("synth-large-multi", 1000.0, 1200.0),
    ],
    [
        ("synth-large-8seg", 1000.0, 1040.0),
        ("synth-large-multi", 1000.0, 1200.0),
        ("synth-large", 1000.0, 1000.0),
    ],
    [
        ("synth-large-multi", 1000.0, 1200.0),
        ("synth-large", 1000.0, 1000.0),
        ("synth-large-8seg", 1000.0, 1040.0),
    ],
]


def test_measure_row_refuses_a_pair_whose_order_does_not_alternate():
    broken = point(orders=["dense-then-sparse"] * 3)
    with pytest.raises(ValueError, match="alternate"):
        multiseg.measure_row(broken, "synth-large", 1)


def test_measure_row_records_every_pair_and_its_gap():
    row = multiseg.measure_row(point(), "synth-large", 1)
    assert row["segments"] == 1
    assert [pair["order"] for pair in row["pairs"]] == [
        "dense-then-sparse",
        "sparse-then-dense",
        "dense-then-sparse",
    ]
    assert row["sparse_minus_dense_percent"] > 0
    assert row["p50_spread"]["gap_points"] >= 0


def test_documents_of_refuses_a_copy_with_another_number_of_segments(tmp_path):
    raw = raw_directory(tmp_path)
    document = json.loads((raw / "cross-synth-large-multi.json").read_text(encoding="utf-8"))
    document["segments"] = 12
    (raw / "cross-synth-large-multi.json").write_text(json.dumps(document), encoding="utf-8")
    with pytest.raises(ValueError, match="segments instead of"):
        multiseg.documents_of(raw)


def test_documents_of_refuses_an_empty_raw_directory(tmp_path):
    with pytest.raises(FileNotFoundError):
        multiseg.documents_of(tmp_path)


def test_documents_of_refuses_a_copy_that_opened_another_index(tmp_path):
    raw = raw_directory(tmp_path)
    document = json.loads((raw / "cross-synth-large-8seg.json").read_text(encoding="utf-8"))
    document["index"] = "synth-large"
    (raw / "cross-synth-large-8seg.json").write_text(json.dumps(document), encoding="utf-8")
    with pytest.raises(ValueError, match="opened 'synth-large'"):
        multiseg.documents_of(raw)


def test_control_rows_refuses_a_disagreeing_copy(tmp_path):
    raw = raw_directory(tmp_path)
    broken = agree_document("synth-large-multi", 100, identical=False)
    (raw / "agree-synth-large-multi.json").write_text(json.dumps(broken), encoding="utf-8")
    with pytest.raises(ValueError, match="disagree"):
        multiseg.control_rows(raw)


def test_control_rows_counts_pools_and_queries_per_copy(tmp_path):
    controls = multiseg.control_rows(raw_directory(tmp_path))
    assert controls["identical"] is True
    assert {copy["campaign"] for copy in controls["copies"]} == {
        "synth-large",
        "synth-large-8seg",
        "synth-large-multi",
    }
    assert controls["pools_checked"] == 3
    assert controls["queries_checked"] == 15


def test_same_pools_flags_a_different_identifier(tmp_path):
    raw = raw_directory(tmp_path)
    documents = multiseg.documents_of(raw)
    documents["synth-large-multi"]["points"][0]["first_iid"] = 101
    rows = multiseg.rows_of(documents)
    result = multiseg.same_pools(rows)
    assert result["identical"] is False
    assert result["differences"][0]["field"] == "first_iid"


def test_same_pools_holds_when_the_copies_share_their_identifiers(tmp_path):
    documents = multiseg.documents_of(raw_directory(tmp_path))
    result = multiseg.same_pools(multiseg.rows_of(documents))
    assert result["identical"] is True
    assert result["compared"] == 1


def test_segment_cost_reads_the_three_copies_at_equal_size_and_span(tmp_path):
    raw = raw_directory(tmp_path)
    documents = multiseg.documents_of(raw)
    rows = multiseg.rows_of(documents)
    cost = multiseg.segment_cost(rows)
    assert len(cost) == 1
    assert cost[0]["size"] == 500_000
    assert set(cost[0]["sparse_p50_us"]) == {
        "one_segment",
        "eight_segments",
        "hundred_segments",
    }
    assert cost[0]["sparse_gap_100_vs_1_percent"] == pytest.approx(0.0)


def test_drift_compares_the_two_runs_of_the_reference(tmp_path):
    rows = multiseg.rows_of(multiseg.documents_of(raw_directory(tmp_path)))
    drift = multiseg.drift(rows)
    assert len(drift) == 1
    assert drift[0]["dense_percent"] == pytest.approx(0.0)


def test_budget_reads_the_two_lines_of_the_scan(tmp_path):
    raw = raw_directory(tmp_path)
    documents = multiseg.documents_of(raw)
    documents["synth-large"]["points"] = [
        point(label="full", kind="full", size=7_740_000, span=8_600_000),
        point(label="contiguous-1000000", kind="contiguous", size=1_000_000, span=1_111_111),
    ]
    rows = multiseg.rows_of(documents)
    lines = multiseg.budget(rows)
    first = next(line for line in lines if line["campaign"] == "synth-large")
    assert first["full_pool"]["size"] == 7_740_000
    assert first["one_m_pool"]["size"] == 1_000_000
    # The fixture durations are in microseconds, so the "20 to 80 ms" line is
    # not met: the field says so instead of rounding the figure up.
    assert first["full_pool"]["within_20_80_ms"] is False


def test_build_writes_a_dated_artefact(tmp_path):
    raw = raw_directory(tmp_path)
    measures = tmp_path / "mesures"
    document = multiseg.build(raw, commit="abc", date="2026-10-27", machine="test machine")
    assert document["same_pools"]["identical"] is True
    assert document["controls"]["identical"] is True
    path = multiseg.write(document, measures, "2026-10-27")
    assert path.name == "MES-02-multisegments.2026-10-27.json"
    written = json.loads(path.read_text(encoding="utf-8"))
    assert written["dataset"]["copies"]["synth-large-multi"]["segments"] == 100
    assert written["commit"] == "abc"


def test_interleaved_is_absent_when_the_series_was_not_run(tmp_path):
    raw = raw_directory(tmp_path)
    assert multiseg.interleaved(raw) is None
    document = multiseg.build(raw, commit="abc", date="2026-10-27", machine="test machine")
    assert document["interleaved"] is None


def test_interleaved_documents_reads_the_order_from_the_write_times(tmp_path):
    raw = raw_directory(tmp_path)
    write_interleaved(raw, LATIN_SQUARE)
    rounds = multiseg.interleaved_documents(raw)
    assert [measured["order"] for measured in rounds] == [
        [entry[0] for entry in entries] for entries in LATIN_SQUARE
    ]


def test_interleaved_documents_refuses_an_order_that_is_not_a_latin_square(tmp_path):
    raw = raw_directory(tmp_path)
    flat = [LATIN_SQUARE[0]] * 3
    write_interleaved(raw, flat)
    with pytest.raises(ValueError, match="Latin square"):
        multiseg.interleaved_documents(raw)


def test_interleaved_documents_refuses_a_round_that_forgot_a_copy(tmp_path):
    raw = raw_directory(tmp_path)
    write_interleaved(
        raw,
        [
            [("synth-large", 1000.0, 1000.0), ("synth-large-8seg", 1000.0, 1040.0)],
            [("synth-large-8seg", 1000.0, 1040.0), ("synth-large-multi", 1000.0, 1200.0)],
            [("synth-large-multi", 1000.0, 1200.0), ("synth-large", 1000.0, 1000.0)],
        ],
    )
    with pytest.raises(ValueError, match="once per round"):
        multiseg.interleaved_documents(raw)


def test_interleaved_documents_refuses_a_copy_with_another_number_of_segments(tmp_path):
    raw = raw_directory(tmp_path)
    write_interleaved(
        raw,
        LATIN_SQUARE,
        segments={"synth-large": 1, "synth-large-8seg": 8, "synth-large-multi": 12},
    )
    with pytest.raises(ValueError, match="segments instead of"):
        multiseg.interleaved_documents(raw)


def test_interleaved_pools_reads_every_gap_and_its_median(tmp_path):
    raw = raw_directory(tmp_path)
    write_interleaved(raw, LATIN_SQUARE)
    series = multiseg.interleaved(raw)
    assert series["latin_square"] is True
    pool = series["pools"][0]
    assert len(pool["rounds"]) == 3
    assert pool["median"]["sparse_gap_8_vs_1_percent"] == pytest.approx(4.0)
    assert pool["median"]["sparse_gap_100_vs_1_percent"] == pytest.approx(20.0)
    assert pool["median"]["sparse_gap_100_vs_8_percent"] == pytest.approx(
        (1200.0 - 1040.0) / 1040.0 * 100.0
    )
    # The dense traversal is the control: it is the same series on the three
    # copies here, so its gaps must be exactly zero.
    assert pool["median"]["dense_gap_100_vs_1_percent"] == pytest.approx(0.0)
    assert pool["all_positive"]["sparse_gap_100_vs_1_percent"] is True
    assert pool["range_points"]["sparse_gap_8_vs_1_percent"] == pytest.approx(0.0)


def test_interleaved_pools_keeps_a_round_that_contradicts_the_median(tmp_path):
    raw = raw_directory(tmp_path)
    contradictory = [list(entries) for entries in LATIN_SQUARE]
    contradictory[2] = [
        ("synth-large-multi", 1000.0, 800.0),
        ("synth-large", 1000.0, 1000.0),
        ("synth-large-8seg", 1000.0, 1040.0),
    ]
    write_interleaved(raw, contradictory)
    pool = multiseg.interleaved(raw)["pools"][0]
    gaps = [row["sparse_gap_100_vs_1_percent"] for row in pool["rounds"]]
    assert gaps == pytest.approx([20.0, 20.0, -20.0])
    # The median stays readable, and the contradiction is not hidden: the
    # amplitude says the series disagrees with itself.
    assert pool["median"]["sparse_gap_100_vs_1_percent"] == pytest.approx(20.0)
    assert pool["minimum"]["sparse_gap_100_vs_1_percent"] == pytest.approx(-20.0)
    assert pool["all_positive"]["sparse_gap_100_vs_1_percent"] is False
    assert pool["range_points"]["sparse_gap_100_vs_1_percent"] == pytest.approx(40.0)


def test_markdown_quotes_the_interleaved_series(tmp_path):
    raw = raw_directory(tmp_path)
    write_interleaved(raw, LATIN_SQUARE)
    document = multiseg.build(raw, commit="abc", date="2026-10-27", machine="test machine")
    text = multiseg.markdown(document)
    assert "Série entrelacée" in text
    assert "médiane" in text
    assert "+20.0 %" in text
