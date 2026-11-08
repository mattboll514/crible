# SPDX-License-Identifier: Apache-2.0
"""Tests of the pure readers of the Mac mini harness."""

import pytest

from checks import (
    EXPECTED_DOCUMENTS,
    EXPECTED_PASSAGES,
    compare_to_reference,
    parse_report,
    totals,
)

REPORTS = [
    "index 'dole' restored from /snapshot: no epoch replaced, new epoch 1, "
    "generation 1, 3600 document(s), 7653 passage(s)",
    "index 'sarde' restored from /snapshot: no epoch replaced, new epoch 1, "
    "generation 1, 210965 document(s), 214578 passage(s)",
    "index 'titres' restored from /snapshot: no epoch replaced, new epoch 1, "
    "generation 1, 1477883 document(s), 1477883 passage(s)",
    "index 'jorf' restored from /snapshot: no epoch replaced, new epoch 1, "
    "generation 3, 1636145 document(s), 1833500 passage(s)",
    "index 'articles' restored from /snapshot: no epoch replaced, new epoch 1, "
    "generation 1, 2135023 document(s), 2476715 passage(s)",
    "index 'decisions' restored from /snapshot: no epoch replaced, new epoch 1, "
    "generation 2, 1196623 document(s), 3589776 passage(s)",
]


def test_parse_report_reads_every_field() -> None:
    report = parse_report(REPORTS[3])
    assert report["index"] == "jorf"
    assert report["snapshot"] == "/snapshot"
    assert report["replaced"] == "no epoch replaced"
    assert report["epoch"] == 1
    assert report["generation"] == 3
    assert report["documents"] == 1_636_145
    assert report["passages"] == 1_833_500


def test_parse_report_reads_a_replaced_epoch() -> None:
    report = parse_report(
        "index 'dole' restored from /snapshot: epoch 4 replaced, new epoch 5, "
        "generation 7, 3600 document(s), 7653 passage(s)"
    )
    assert report["replaced"] == "epoch 4 replaced"
    assert report["epoch"] == 5
    assert report["generation"] == 7


def test_parse_report_rejects_another_line() -> None:
    with pytest.raises(ValueError):
        parse_report("index 'dole' attached")


def test_totals_match_the_development_machine() -> None:
    reports = [parse_report(line) for line in REPORTS]
    assert totals(reports) == {
        "documents": EXPECTED_DOCUMENTS,
        "passages": EXPECTED_PASSAGES,
    }
    assert compare_to_reference(reports) == []


def test_compare_to_reference_names_a_missing_index() -> None:
    reports = [parse_report(line) for line in REPORTS[:-1]]
    problems = compare_to_reference(reports)
    assert len(problems) == 2
    assert problems[1] == "passages 6010329 != 9600105"


def test_compare_to_reference_names_a_duplicate() -> None:
    reports = [parse_report(line) for line in REPORTS]
    reports.append(parse_report(REPORTS[0]))
    problems = compare_to_reference(reports)
    assert any(problem.startswith("duplicate index") for problem in problems)
