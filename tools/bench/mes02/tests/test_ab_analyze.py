# SPDX-License-Identifier: Apache-2.0

"""Tests of the A/B verification of the single traversal (ADR-0082).

They pin the three things a reader of the report has to be able to trust:

- the **verdict rule** of decision 3: a point is lost only when the median gap
  is positive *and* larger than the amplitude of its own pair-to-pair gaps, and
  it is a tie otherwise;
- the **refusals**: a document that claims the wrong number of segments, a pool
  one document measured on another span, an old document that does not name the
  traversal its rule chose, and a failing agreement control;
- the **readings** the report quotes: the cost left by the cursor is read inside
  a round, at equal size and span, on the labels the three copies share.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

import mes02_ab_analyze as ab
import mes02_analyze as analyze

#: Labels of the fixture: the two pools of the budget line, shared by the three
#: copies, which is enough to exercise the per-point tables, the segment cost and
#: the budget reading.
LABELS = {
    "contiguous-1000000": {"kind": "contiguous", "size": 1_000_000, "span": 1_000_000},
    "full": {"kind": "full", "size": 7_740_000, "span": 7_740_000},
}


def flat(value: float) -> list[float]:
    """A series whose p50 and p95 are `value`, so a gap can be read by eye."""
    return [value] * 20


def point(
    *,
    label: str,
    side: str,
    p50: float,
    automatic: str = "sparse",
    pairs: int = 3,
) -> dict:
    """One raw point, summarised the way the harness writes it."""
    shape = LABELS[label]
    summary = analyze.summarise(flat(p50))
    document = {
        "label": label,
        "kind": shape["kind"],
        "requested_span": shape["span"] if shape["kind"] == "density" else 0,
        "size": shape["size"],
        "span": shape["span"],
        "density": shape["size"] / shape["span"],
        "n": 5000,
        "first_iid": 0,
        "last_iid": shape["span"] - 1,
        "plan": "SCAN",
        "pairs_count": pairs,
        "queries": 300,
    }
    if side == "old":
        document["traversal_automatic"] = automatic
        document["warmup"] = {"dense": None, "sparse": None}
        document["pairs"] = [{"pair": index + 1} for index in range(pairs)]
        document["dense"] = {"durations_us": flat(p50), "summary": summary}
        document["sparse"] = {"durations_us": flat(p50), "summary": summary}
    else:
        document["traversal"] = "sparse"
        document["warmup"] = {"summary": summary, "durations_us": flat(p50)}
        document["pairs"] = [{"pass": index + 1, "sparse": summary} for index in range(pairs)]
        document["sparse"] = {"durations_us": flat(p50), "summary": summary}
    return document


def cross_document(copy: str, side: str, p50: float, *, automatic: str = "sparse") -> dict:
    """One `cross` document of one copy and one side."""
    return {
        "kind": "cross",
        "index": copy,
        "dims": 768,
        "threads": 8,
        "live_passages": 7_740_000,
        "segments": ab.COPIES[copy]["segments"],
        "segment_counts": [ab.COPIES[copy]["segments"]],
        "vectors": "data/mes02/queries.f32",
        "queries": 300,
        "points": [point(label=label, side=side, p50=p50, automatic=automatic) for label in LABELS],
    }


def agree_document(copy: str, side: str, *, identical: bool = True) -> dict:
    """One `agree` document, by default successful."""
    return {
        "kind": "agree",
        "index": copy,
        "dims": 768,
        "segments": ab.COPIES[copy]["segments"],
        "pools_checked": len(LABELS),
        "queries_checked": 5,
        "identical": identical,
        "points": [],
    }


def write_fixture(raw: Path, *, new_p50: float = 900.0, old_p50: float = 1000.0) -> None:
    """Writes the six cross documents and the six control documents of the A/B."""
    raw.mkdir(parents=True, exist_ok=True)
    (raw.parent / "queries.json").write_text(
        json.dumps(
            {
                "matrix": "queries.f32",
                "sha256": "0" * 64,
                "law": "standard normal, normalised to the unit sphere",
                "seed": 20261021,
                "stream": 7,
            }
        ),
        encoding="utf-8",
    )
    for copy in ab.COPIES:
        for round_number in ab.ROUNDS:
            for side, value in (("old", old_p50), ("new", new_p50)):
                path = raw / f"ab-{side}-{copy}-r{round_number}.json"
                path.write_text(json.dumps(cross_document(copy, side, value)), encoding="utf-8")
        for side in ("old", "new"):
            path = raw / f"ab-agree-{side}-{copy}.json"
            path.write_text(json.dumps(agree_document(copy, side)), encoding="utf-8")


def rewrite(raw: Path, name: str, document: dict) -> None:
    """Replaces one raw document, to check a refusal."""
    (raw / name).write_text(json.dumps(document), encoding="utf-8")


# ---------------------------------------------------------------- verdicts ---


def test_verdict_reads_a_gap_beyond_its_own_amplitude_as_lost() -> None:
    result = ab.verdict([10.0, 12.0, 14.0])
    assert result["verdict"] == "perdu"
    assert result["gap_median_percent"] == 12.0
    assert result["gap_amplitude_points"] == 4.0


def test_verdict_reads_a_gap_inside_its_amplitude_as_a_tie() -> None:
    result = ab.verdict([2.0, -8.0, 4.0])
    assert result["verdict"] == "match nul"
    assert result["gap_median_percent"] == 2.0
    assert result["gap_amplitude_points"] == 12.0


def test_verdict_reads_a_negative_gap_beyond_the_amplitude_as_won() -> None:
    result = ab.verdict([-20.0, -22.0, -24.0])
    assert result["verdict"] == "gagné"
    assert result["gap_median_percent"] == -22.0


def test_verdict_of_a_perfectly_stable_gap_is_not_a_tie() -> None:
    result = ab.verdict([5.0, 5.0, 5.0])
    assert result["gap_amplitude_points"] == 0.0
    assert result["verdict"] == "perdu"


def test_verdict_of_an_exactly_zero_gap_is_a_tie() -> None:
    assert ab.verdict([0.0, 0.0, 0.0])["verdict"] == "match nul"


# ------------------------------------------------------------- aggregation ---


def test_build_reads_the_three_copies_and_tallies_the_verdicts(tmp_path: Path) -> None:
    raw = tmp_path / "raw"
    write_fixture(raw)
    document = ab.build(raw, commit="0" * 40, date="2026-10-27", machine="test")
    assert len(document["rows"]) == len(ab.COPIES) * len(LABELS)
    assert all(row["old_traversal"] == ["sparse"] for row in document["rows"])
    # The new binary is 10 % faster, stably: a win on every point, no loss.
    for tally in document["tally"].values():
        assert tally == {"gagné": len(LABELS), "match nul": 0, "perdu": 0}
    assert all(row["gap_median_percent"] == pytest.approx(-10.0) for row in document["rows"])


def test_build_uses_the_traversal_the_old_rule_chose(tmp_path: Path) -> None:
    raw = tmp_path / "raw"
    write_fixture(raw)
    # The old rule chose the dense traversal, at the same p50 here: the reading
    # must name it, and the gap must not change.
    for round_number in ab.ROUNDS:
        name = f"ab-old-synth-large-multi-r{round_number}.json"
        document = json.loads((raw / name).read_text(encoding="utf-8"))
        for entry in document["points"]:
            entry["traversal_automatic"] = "dense"
        rewrite(raw, name, document)
    built = ab.build(raw, commit="0" * 40, date="2026-10-27", machine="test")
    row = ab.rows_one(built["rows"], "synth-large-multi", "full")
    assert row["old_traversal"] == ["dense"]


def test_segment_cost_is_read_inside_a_round(tmp_path: Path) -> None:
    raw = tmp_path / "raw"
    write_fixture(raw)
    # 8 segments: +10 % against the one-segment copy (900 µs in the fixture);
    # 100 segments: +20 %.
    for copy, factor in (("synth-large-8seg", 1.1), ("synth-large-multi", 1.2)):
        for round_number in ab.ROUNDS:
            name = f"ab-new-{copy}-r{round_number}.json"
            document = cross_document(copy, "new", 900.0 * factor)
            rewrite(raw, name, document)
    built = ab.build(raw, commit="0" * 40, date="2026-10-27", machine="test")
    for entry in built["segment_cost"]:
        assert entry["eight_vs_one_median_percent"] == pytest.approx(10.0)
        assert entry["hundred_vs_one_median_percent"] == pytest.approx(20.0)
        assert entry["hundred_vs_eight_median_percent"] == pytest.approx(1080.0 / 990.0 * 100.0 - 100.0)


# ---------------------------------------------------------------- refusals ---


def test_build_refuses_a_copy_with_the_wrong_number_of_segments(tmp_path: Path) -> None:
    raw = tmp_path / "raw"
    write_fixture(raw)
    name = "ab-old-synth-large-8seg-r2.json"
    document = cross_document("synth-large-8seg", "old", 1_000.0)
    document["segments"] = 1
    rewrite(raw, name, document)
    with pytest.raises(ValueError, match="segments"):
        ab.build(raw, commit="0" * 40, date="2026-10-27", machine="test")


def test_build_refuses_two_documents_on_different_pools(tmp_path: Path) -> None:
    raw = tmp_path / "raw"
    write_fixture(raw)
    name = "ab-new-synth-large-r3.json"
    document = cross_document("synth-large", "new", 900.0)
    document["points"][0]["span"] = 999_999
    rewrite(raw, name, document)
    with pytest.raises(ValueError, match="span"):
        ab.build(raw, commit="0" * 40, date="2026-10-27", machine="test")


def test_build_refuses_an_old_document_that_names_no_traversal(tmp_path: Path) -> None:
    raw = tmp_path / "raw"
    write_fixture(raw)
    name = "ab-old-synth-large-r1.json"
    document = cross_document("synth-large", "old", 1_000.0)
    del document["points"][0]["traversal_automatic"]
    rewrite(raw, name, document)
    with pytest.raises(ValueError, match="traversal_automatic"):
        ab.build(raw, commit="0" * 40, date="2026-10-27", machine="test")


def test_build_refuses_a_failing_agreement_control(tmp_path: Path) -> None:
    raw = tmp_path / "raw"
    write_fixture(raw)
    rewrite(raw, "ab-agree-new-synth-large-multi.json", agree_document(
        "synth-large-multi", "new", identical=False
    ))
    with pytest.raises(ValueError, match="agreement control failed"):
        ab.build(raw, commit="0" * 40, date="2026-10-27", machine="test")


def test_build_refuses_a_document_whose_summary_does_not_match_its_series(
    tmp_path: Path,
) -> None:
    raw = tmp_path / "raw"
    write_fixture(raw)
    name = "ab-new-synth-large-r2.json"
    document = cross_document("synth-large", "new", 900.0)
    document["points"][1]["sparse"]["summary"]["p50"] = 1.0
    rewrite(raw, name, document)
    with pytest.raises(ValueError, match="p50"):
        ab.build(raw, commit="0" * 40, date="2026-10-27", machine="test")


def test_build_refuses_a_new_document_that_does_not_measure_the_single_traversal(
    tmp_path: Path,
) -> None:
    raw = tmp_path / "raw"
    write_fixture(raw)
    name = "ab-new-synth-large-r1.json"
    document = cross_document("synth-large", "new", 900.0)
    document["points"][0]["traversal"] = "dense"
    rewrite(raw, name, document)
    with pytest.raises(ValueError, match="single sparse"):
        ab.build(raw, commit="0" * 40, date="2026-10-27", machine="test")
