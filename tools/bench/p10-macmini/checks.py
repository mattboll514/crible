# SPDX-License-Identifier: Apache-2.0
"""Reads the restore reports of the Mac mini campaign and compares the counts.

The reports are the lines `crible restore` prints (ADR-0065, decision 6):

    index 'dole' restored from /snapshot: no epoch replaced, new epoch 1,
    generation 1, 3600 document(s), 7653 passage(s)

The expected totals of the development machine are 6 660 239 documents and
9 600 105 passages (`docs/mesures/import-legifacile-2026-11-06.md:35-36`,
ADR-0141, decision 2).
"""

from __future__ import annotations

import re

EXPECTED_DOCUMENTS = 6_660_239
EXPECTED_PASSAGES = 9_600_105

REPORT = re.compile(
    r"index '(?P<index>[^']+)' restored from (?P<snapshot>\S+): "
    r"(?P<replaced>.+?), new epoch (?P<epoch>\d+), "
    r"generation (?P<generation>\d+), "
    r"(?P<documents>\d+) document\(s\), (?P<passages>\d+) passage\(s\)"
)


def parse_report(line: str) -> dict:
    """Turns one report line into a dict, or raises `ValueError`."""
    match = REPORT.search(line)
    if match is None:
        raise ValueError(f"not a restore report: {line!r}")
    fields = match.groupdict()
    return {
        "index": fields["index"],
        "snapshot": fields["snapshot"],
        "replaced": fields["replaced"],
        "epoch": int(fields["epoch"]),
        "generation": int(fields["generation"]),
        "documents": int(fields["documents"]),
        "passages": int(fields["passages"]),
    }


def totals(reports: list[dict]) -> dict:
    """Sums documents and passages over the reports."""
    return {
        "documents": sum(report["documents"] for report in reports),
        "passages": sum(report["passages"] for report in reports),
    }


def compare_to_reference(
    reports: list[dict],
    expected_documents: int = EXPECTED_DOCUMENTS,
    expected_passages: int = EXPECTED_PASSAGES,
) -> list[str]:
    """Returns one line per difference, empty when the counts are equal."""
    problems = []
    names = [report["index"] for report in reports]
    if len(set(names)) != len(names):
        problems.append(f"duplicate index in the reports: {names}")
    summed = totals(reports)
    if summed["documents"] != expected_documents:
        problems.append(
            f"documents {summed['documents']} != {expected_documents}"
        )
    if summed["passages"] != expected_passages:
        problems.append(f"passages {summed['passages']} != {expected_passages}")
    return problems
