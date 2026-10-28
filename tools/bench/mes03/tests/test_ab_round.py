# SPDX-License-Identifier: Apache-2.0
"""Tests of the alternated-round driver of MES-03 (``ab_round.py``)."""

from __future__ import annotations

from pathlib import Path

from ab_round import displacement_sources, invocation_command


def test_displacement_sources_are_split_back_into_the_flags(tmp_path: Path):
    listing = tmp_path / "sources.txt"
    listing.write_text(
        "--displace-source data/a.parquet --displace-source data/b.f16\n",
        encoding="utf-8",
    )
    assert displacement_sources(listing) == [
        "--displace-source",
        "data/a.parquet",
        "--displace-source",
        "data/b.f16",
    ]


def test_no_source_file_means_no_flag():
    assert displacement_sources(None) == []


def test_a_hot_invocation_carries_no_cold_flag(tmp_path: Path):
    command = invocation_command(
        Path("bin/base/rescore_ab"),
        mode="measure",
        data="data/mes02",
        index="synth-large",
        vectors="data/mes02/queries.f32",
        n=1000,
        passes=3,
        out=tmp_path / "out.json",
    )
    assert "--cold" not in command
    assert "--displace-gb" not in command
    assert "--idle-seconds" not in command
    assert "--sample-rss" not in command
    assert command[:2] == ["bin/base/rescore_ab", "measure"]
    assert command[command.index("--passes") + 1] == "3"


def test_a_cold_invocation_carries_the_displacement_and_the_residency_probe(
    tmp_path: Path,
):
    command = invocation_command(
        Path("bin/grouped/rescore_ab"),
        mode="measure",
        data="data/mes02",
        index="synth-large",
        vectors="data/mes02/queries.f32",
        n=5000,
        passes=1,
        out=tmp_path / "out.json",
        queries=30,
        cold=True,
        vectors_file=Path("data/mes02/indexes/synth-large/epochs/1/segments/seg-large-0001/vectors.f16"),
        displace_gb=19.5,
        displace_sources=[
            "--displace-source",
            "data/synth-large/large-0001/passages-0001.parquet",
        ],
        idle_seconds=600,
        sample_rss=True,
    )
    assert "--cold" in command
    assert command[command.index("--vectors-file") + 1].endswith("vectors.f16")
    assert command[command.index("--displace-gb") + 1] == "19.5"
    assert command[command.index("--displace-source") + 1].endswith("passages-0001.parquet")
    assert command[command.index("--idle-seconds") + 1] == "600"
    assert command[command.index("--queries") + 1] == "30"
    assert "--sample-rss" in command


def test_the_contention_mode_is_passed_through(tmp_path: Path):
    command = invocation_command(
        Path("bin/grouped/rescore_ab"),
        mode="contend",
        data="data/mes02",
        index="synth-large",
        vectors="data/mes02/queries.f32",
        n=1000,
        passes=3,
        out=tmp_path / "out.json",
    )
    assert command[1] == "contend"
