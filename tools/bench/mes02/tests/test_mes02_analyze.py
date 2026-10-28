# SPDX-License-Identifier: Apache-2.0

"""Tests of the aggregation of MES-02, MES-03 and MES-09 (ADR-0077).

The aggregator recomputes every percentile from the per-query durations and
refuses a table whose figure does not match the one the harness recorded. These
tests pin the percentile definition (nearest rank, no interpolation), the split
of the RSS between anonymous and file-backed memory, and the shape of the
versioned artefacts.
"""

from __future__ import annotations

import json

import pytest

import mes02_analyze as analyze


def series(count: int) -> list[float]:
    """`count` increasing durations, so the nearest rank is readable by eye."""
    return [float(index + 1) for index in range(count)]


def test_nearest_rank_takes_the_value_of_rank_ceiling_ratio_times_n():
    values = series(100)
    assert analyze.nearest_rank(values, 0.50) == 50.0
    assert analyze.nearest_rank(values, 0.95) == 95.0
    assert analyze.nearest_rank(values, 0.99) == 99.0


def test_nearest_rank_never_interpolates_between_two_samples():
    # 900 samples: 0.99 × 900 = 891 exactly, 0.95 × 900 = 855.
    values = series(900)
    assert analyze.nearest_rank(values, 0.99) == 891.0
    assert analyze.nearest_rank(values, 0.95) == 855.0
    # 7 samples: 0.95 × 7 = 6.65, rounded up to rank 7, not a mix of 6 and 7.
    assert analyze.nearest_rank(series(7), 0.95) == 7.0
    assert analyze.nearest_rank(series(7), 0.50) == 4.0


def test_nearest_rank_is_insensitive_to_the_order_of_the_series():
    values = [5.0, 1.0, 4.0, 2.0, 3.0]
    assert analyze.nearest_rank(values, 0.50) == 3.0


def test_nearest_rank_refuses_an_empty_series():
    with pytest.raises(ValueError):
        analyze.nearest_rank([], 0.5)


@pytest.mark.parametrize("ratio", [0.0, -0.1, 1.5])
def test_nearest_rank_refuses_a_ratio_outside_the_unit_interval(ratio):
    with pytest.raises(ValueError):
        analyze.nearest_rank([1.0], ratio)


def test_summarise_counts_every_sample_and_keeps_the_maximum():
    summary = analyze.summarise([1.0, 2.0, 3.0, 4.0])
    assert summary["n"] == 4
    assert summary["p50"] == 2.0
    assert summary["p95"] == 4.0
    assert summary["mean"] == pytest.approx(2.5)
    assert summary["max"] == 4.0


def test_check_summary_refuses_a_recorded_figure_that_does_not_match():
    values = [1.0, 2.0, 3.0]
    recorded = {"n": 3, "p50": 2.0, "p95": 3.0, "p99": 3.0, "max": 99.0}
    with pytest.raises(ValueError, match="max"):
        analyze.check_summary(values, recorded, "a campaign")


def test_check_summary_accepts_the_harness_round_trip():
    values = [10.0, 20.0, 30.0, 40.0]
    recorded = analyze.summarise(values)
    assert analyze.check_summary(values, recorded, "a campaign")["p50"] == 20.0


def test_state_of_splits_anonymous_memory_from_file_backed_pages():
    state = {
        "ps_rss_bytes": 5_000,
        "mapped_file_bytes": 3_000,
        "swapped_bytes": 1_000,
        "physical_footprint_bytes": 6_000,
    }
    split = analyze.state_of(state)
    assert split["resident_bytes"] == 5_000
    assert split["anonymous_bytes"] == 2_000
    assert split["mapped_file_bytes"] == 3_000
    assert split["swapped_bytes"] == 1_000
    assert split["footprint_bytes"] == 6_000


def test_state_of_refuses_a_document_from_before_the_ps_field():
    with pytest.raises(ValueError, match="ps_rss_bytes"):
        analyze.state_of({"mapped_file_bytes": 0, "swapped_bytes": 0})


def point(*, durations: list[float], size: int, shape: str, n: int) -> dict:
    """One scan point, carrying its own summary as the harness writes it."""
    return {
        "durations_us": durations,
        "size": size,
        "requested_size": size,
        "shape": shape,
        "n": n,
        "queries": len(durations),
        "passes": 1,
        "plan": "binary",
        "traversal": "dense" if shape == "contiguous" else "sparse",
        "traversal_automatic": "dense",
        "summary": analyze.summarise(durations),
    }


def rescore_document(*, engine: list[float], dot: list[float], key: list[float], n: int) -> dict:
    """One rescoring document with the three series of the warm campaign."""
    return {
        "kind": "rescore",
        "index": "synth-large",
        "dims": 768,
        "threads": 8,
        "live_passages": 7_740_000,
        "vectors": "data/mes02/queries.f32",
        "dot_vectors_file": "vectors.f16",
        "queries": len(engine),
        "cold": False,
        "points": [
            {
                "n": n,
                "candidates": n,
                "traversal": "dense",
                "cold": False,
                "passes": 1,
                "queries": len(engine),
                "durations_us": engine,
                "summary": analyze.summarise(engine),
                "dot_only_us": dot,
                "dot_only_summary": analyze.summarise(dot),
                "key_only_us": key,
                "key_only_summary": analyze.summarise(key),
                "resident_fraction": None,
                "score_ties": 0,
            }
        ],
    }


def rss_document(*, model: bool) -> dict:
    """One `rss` run: the three states, and the model states when asked."""
    def state(resident: int, mapped: int) -> dict:
        return {
            "ps_rss_bytes": resident,
            "mapped_file_bytes": mapped,
            "swapped_bytes": 0,
            "physical_footprint_bytes": resident,
            "resident_bytes": resident,
        }

    document = {
        "kind": "rss",
        "index": "synth-large",
        "dims": 768,
        "threads": 8,
        "live_passages": 7_740_000,
        "codes_bytes": 825_600_000,
        "filter_memory_bytes": 361_661_936,
        "queries": 3,
        "passes": 1,
        "n": 1000,
        "states": {
            "after_open": state(1_000, 0),
            "after_searcher": state(1_000, 0),
            "after_pass": state(5_000, 4_000),
        },
        "model": None,
    }
    if model:
        document["model"] = {"before": state(1_000, 0), "after": state(3_000, 0)}
    return document


def fake_raw(root) -> "object":
    """A miniature raw directory with every document the aggregator reads."""
    root.mkdir(parents=True, exist_ok=True)
    scan = {"kind": "scan", "index": "synth-large", "dims": 768, "threads": 8,
            "live_passages": 7_740_000, "queries": 3, "vectors": "queries.f32"}
    for campaign, shapes in (
        ("scan-main", ("contiguous", "dispersed")),
        ("scan-forced-dense", ("contiguous",)),
        ("scan-forced-sparse", ("contiguous",)),
        ("scan-all", ("full",)),
    ):
        points = [
            point(durations=[1.0, 2.0, 3.0], size=1000, shape=shape, n=1000)
            for shape in shapes
        ]
        (root / f"{campaign}.json").write_text(
            json.dumps({**scan, "points": points}), encoding="utf-8"
        )
    exact = {
        "kind": "exact",
        "index": "synth-large",
        "dims": 768,
        "threads": 8,
        "live_passages": 7_740_000,
        "queries": 3,
        "vectors": "queries.f32",
        "points": [
            {
                "durations_us": [1.0, 2.0, 3.0],
                "k": 100,
                "passes": 1,
                "pool_size": 1000,
                "queries": 3,
                "score_ties": 0,
                "summary": analyze.summarise([1.0, 2.0, 3.0]),
            }
        ],
    }
    (root / "exact.json").write_text(json.dumps(exact), encoding="utf-8")
    (root / "rescore-warm.json").write_text(
        json.dumps(
            rescore_document(engine=[10.0, 20.0, 30.0], dot=[1.0, 2.0, 3.0], key=[4.0, 5.0, 6.0], n=1000)
        ),
        encoding="utf-8",
    )
    (root / "key-only.json").write_text(
        json.dumps(
            {**rescore_document(engine=[1.0], dot=[1.0], key=[7.0, 8.0, 9.0], n=1000)}
        ),
        encoding="utf-8",
    )
    compare = {
        "kind": "compare",
        "index": "synth-large",
        "n": 1000,
        "top": 2,
        "filter": '{"categorie":"cat-01"}',
        "pool_size": 537_500,
        "plan": "binary",
        "traversal": "sparse",
        "identical": True,
        "engine": [{"key": "a#0", "score": 0.5}, {"key": "b#0", "score": 0.25}],
        "replica": [{"key": "a#0", "score": 0.5}, {"key": "b#0", "score": 0.25}],
    }
    (root / "compare-cat01.json").write_text(json.dumps(compare), encoding="utf-8")
    cli = {
        "generation": 2,
        "pool_size": 537_500,
        "took_ms": 1.0,
        "truncated_query": False,
        "results": [{"key": "a#0", "score": 0.5}, {"key": "b#0", "score": 0.25}],
    }
    (root / "cli-cat01.json").write_text(json.dumps(cli), encoding="utf-8")
    (root / "rss-base-1.json").write_text(json.dumps(rss_document(model=False)), encoding="utf-8")
    (root / "rss-model-cold-1.json").write_text(
        json.dumps(rss_document(model=True)), encoding="utf-8"
    )
    (root.parent / "queries.json").write_text(
        json.dumps(
            {
                "dataset": "synth-large",
                "queries": 3,
                "dims": 768,
                "sha256": "00" * 32,
                "law": "standard normal, normalised",
                "seed": 20261021,
                "stream": 7,
            }
        ),
        encoding="utf-8",
    )
    return root


def test_build_produces_the_three_measures_from_a_raw_directory(tmp_path):
    raw = fake_raw(tmp_path / "data" / "raw")
    artefacts = analyze.build(raw, commit="deadbeef", date="2026-10-27", machine="a machine")
    assert set(artefacts) == {"MES-02", "MES-03", "MES-09"}
    assert artefacts["MES-02"]["dataset"]["live_passages"] == 7_740_000
    assert artefacts["MES-02"]["dataset"]["queries_sha256"] == "00" * 32
    assert artefacts["MES-02"]["scan"][0]["us"]["p50"] == 2.0
    assert artefacts["MES-03"]["rescoring"][0]["engine_us"]["p50"] == 20.0
    assert artefacts["MES-09"]["model_runs"][0]["model"]["after"]["anonymous_bytes"] == 3_000


def test_build_refuses_a_raw_point_whose_summary_was_edited(tmp_path):
    raw = fake_raw(tmp_path / "data" / "raw")
    document = json.loads((raw / "scan-main.json").read_text(encoding="utf-8"))
    document["points"][0]["summary"]["p99"] = 1.0
    (raw / "scan-main.json").write_text(json.dumps(document), encoding="utf-8")
    with pytest.raises(ValueError, match="p99"):
        analyze.build(raw, commit="deadbeef", date="2026-10-27", machine="a machine")


def test_write_names_one_dated_artefact_per_measure(tmp_path):
    raw = fake_raw(tmp_path / "data" / "raw")
    artefacts = analyze.build(raw, commit="deadbeef", date="2026-10-27", machine="a machine")
    written = analyze.write(artefacts, tmp_path / "mesures", "2026-10-27")
    assert [path.name for path in written] == [
        "MES-02-latence-scan-binaire.2026-10-27.json",
        "MES-03-latence-rescoring.2026-10-27.json",
        "MES-09-ram-reelle.2026-10-27.json",
    ]
    stored = json.loads(written[0].read_text(encoding="utf-8"))
    assert stored["measure"] == "MES-02"
    assert stored["commit"] == "deadbeef"


def test_rescore_point_keeps_the_residency_of_every_query():
    document = rescore_document(engine=[10.0], dot=[1.0], key=[2.0], n=1000)
    document["plan"] = "binary"
    document["points"][0]["resident_fraction"] = {
        "mean": 0.25,
        "min": 0.0,
        "max": 0.5,
        "values": [0.0, 0.25, 0.5],
    }
    row = analyze.rescore_point(document, document["points"][0], "cold-engine")
    # ADR-0077, decision 2: the proportion is kept per query, not only its bounds.
    assert row["resident_fraction"]["values"] == [0.0, 0.25, 0.5]
    assert row["resident_fraction"]["first"] == 0.0
    assert row["resident_fraction"]["last"] == 0.5


def test_rescore_point_records_the_plan_the_gate_needs():
    document = rescore_document(engine=[10.0], dot=[1.0], key=[2.0], n=1000)
    assert analyze.rescore_point(document, document["points"][0], "warm")["plan"] is None
    document["plan"] = "binary"
    assert analyze.rescore_point(document, document["points"][0], "warm")["plan"] == "binary"


def test_relative_keeps_the_path_under_the_repository_and_strips_the_root():
    inside = analyze.ROOT / "data" / "mes02" / "queries.f32"
    assert analyze.relative(str(inside)) == "data/mes02/queries.f32"
    assert analyze.relative("data/mes02/queries.f32") == "data/mes02/queries.f32"
    assert analyze.relative(None) is None


def test_relative_refuses_a_path_outside_the_repository():
    with pytest.raises(ValueError, match="outside the repository"):
        analyze.relative("/elsewhere/queries.f32")


def test_build_versions_the_anchoring_control_of_the_replica(tmp_path):
    # The raw documents are gitignored: the report's claim that the replica
    # matches the engine is only checkable if the summary is versioned.
    raw = fake_raw(tmp_path / "data" / "raw")
    artefacts = analyze.build(raw, commit="deadbeef", date="2026-10-27", machine="a machine")
    controls = artefacts["MES-02"]["controls"]
    assert controls["compare"][0]["identical"] is True
    assert controls["compare"][0]["plan"] == "binary"
    assert controls["compare"][0]["pool_size"] == 537_500
    assert controls["compare"][0]["top"] == 2
    assert controls["cli"][0]["same_keys_as_engine"] is True
    assert controls["cli"][0]["max_score_delta"] == 0.0


def test_rss_run_reports_no_pass_when_the_run_skipped_it():
    document = rss_document(model=True)
    document["passes"] = 0
    assert analyze.rss_run(document, "rss-model-cold-1")["passes"] == 0
