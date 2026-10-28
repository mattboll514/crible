# SPDX-License-Identifier: Apache-2.0
"""Run the engine side of MES-01 and MES-12 (fidelity of the binary scan).

For every structure of the sample and every ``N`` of ADR-0076, this script
writes ``search.rescore_candidates = N`` into the configuration of the
measurement index, runs ``examples/fidelity`` once per structure, and keeps the
top-10 the engine actually returns with its dense plan. It also runs the same
pipeline with ``exact: true``, which is the reference of the comparison.

The filtered campaign uses the synthetic filters of ``mes01_prepare.py`` and is
declared as such in the report.

Every run is checked: a BINARY run is only kept when the plan of every query
says ``"binary"`` and reports ``N`` rescored candidates (ADR-0076, decision 4).
The summaries are **merged** into ``runs.json``, so a partial replay (one
structure, one campaign) never erases the runs already recorded. The
``strategy_control`` block is recomputed from the merged summaries: it counts
the query-plans the checks accepted, campaign by campaign.

Run from the repository root, after ``mes01_build_indexes.py``::

    (cd tools && uv run --no-sync python bench/mes01/mes01_campaign.py)
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

import numpy as np

#: Repository root, three levels above this file.
ROOT = Path(__file__).resolve().parents[3]
#: Structures of the sample.
STRUCTURES = ("articles", "decisions", "dole", "jorf", "sarde", "titres")
#: Structures of the filtered campaign (ADR-0076, decision 5).
FILTERED_STRUCTURES = ("articles", "decisions")
#: Values of ``search.rescore_candidates`` measured here.
DEFAULT_NS = (200, 500, 1000, 2000, 5000)
#: Campaigns of ADR-0076, in the order of the report.
CAMPAIGNS = ("main", "filtered")
#: Number of results compared, and therefore of the metrics of ADR-0076.
DEFAULT_K = 10


def read_jsonl(path: Path) -> list[dict]:
    """Returns the JSON objects of a JSONL file, in order."""
    rows = []
    with path.open(encoding="utf-8") as handle:
        for line in handle:
            if line.strip():
                rows.append(json.loads(line))
    return rows


def write_jsonl(path: Path, rows: list[dict]) -> None:
    """Writes one compact JSON object per line."""
    with path.open("w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, ensure_ascii=False) + "\n")


def set_rescore(index_dir: Path, rescore: int) -> None:
    """Writes ``search.rescore_candidates`` and keeps ``exact_threshold`` at 0."""
    path = index_dir / "index.json"
    config = json.loads(path.read_text(encoding="utf-8"))
    config["search"]["rescore_candidates"] = rescore
    config["search"]["exact_threshold"] = 0
    path.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")


def run_example(
    binary: Path,
    data: Path,
    structure: str,
    queries: Path,
    vectors: Path,
    out: Path,
    exact: bool,
    k: int,
) -> dict:
    """Runs ``examples/fidelity`` once and returns the report it wrote."""
    command = [
        str(binary),
        "--data",
        str(data),
        "--index",
        structure,
        "--vectors",
        str(vectors),
        "--queries",
        str(queries),
        "--k",
        str(k),
        "--out",
        str(out),
    ]
    if exact:
        command.append("--exact")
    print("+", " ".join(command), flush=True)
    completed = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
    if completed.returncode != 0:
        raise SystemExit(
            f"fidelity harness failed with {completed.returncode}: "
            f"{completed.stderr.strip()}"
        )
    return json.loads(out.read_text(encoding="utf-8"))


def check_binary(report: dict, expected_rescore: int, k: int) -> None:
    """Refuses a BINARY run the plan does not confirm (ADR-0076)."""
    expected = max(expected_rescore, k)
    for record in report["results"]:
        if record["dense_strategy"] != "binary":
            raise SystemExit(
                f"{report['index']}: query {record['id']} ran "
                f"{record['dense_strategy']}, not binary"
            )
        if record["rescore_candidates"] != expected:
            raise SystemExit(
                f"{report['index']}: query {record['id']} planned "
                f"{record['rescore_candidates']} candidates, not {expected}"
            )


def check_exact(report: dict) -> None:
    """Refuses an EXACT run the plan does not confirm."""
    for record in report["results"]:
        if record["dense_strategy"] != "exact":
            raise SystemExit(
                f"{report['index']}: query {record['id']} ran "
                f"{record['dense_strategy']}, not exact"
            )


def run_key(entry: dict) -> tuple[str, str, str]:
    """Identity of one run in ``runs.json``."""
    return (entry["campaign"], entry["structure"], entry["run"])


def run_order(entry: dict) -> tuple[int, int, int]:
    """Sorts the runs as the report reads them: campaign, structure, EXACT then N."""
    ordinal = 0 if entry["run"] == "exact" else int(entry["run"][1:])
    return (
        CAMPAIGNS.index(entry["campaign"]),
        STRUCTURES.index(entry["structure"]),
        ordinal,
    )


def merge_runs(path: Path, summaries: list[dict]) -> list[dict]:
    """Merges the new summaries into the recorded ones, newest winning.

    A partial replay (``--campaigns main --structures articles``, or a single
    ``--ns``) therefore keeps the other runs, and the file always describes
    every run whose engine output is on disk.
    """
    merged: dict[tuple[str, str, str], dict] = {}
    if path.exists():
        recorded = json.loads(path.read_text(encoding="utf-8"))
        entries = recorded["runs"] if isinstance(recorded, dict) else recorded
        for entry in entries:
            entry.setdefault(
                "strategy", "exact" if entry["run"] == "exact" else "binary"
            )
            merged[run_key(entry)] = entry
    for entry in summaries:
        merged[run_key(entry)] = entry
    return sorted(merged.values(), key=run_order)


def strategy_control(runs: list[dict]) -> dict:
    """Counts the query-plans the checks accepted, campaign by campaign.

    ``check_binary`` and ``check_exact`` refuse a run whose plan does not match,
    so a query is counted here only when the engine really ran the strategy the
    measurement needs (ADR-0076, decision 4).
    """
    control: dict = {
        "binary_query_runs": 0,
        "exact_query_runs": 0,
        "campaigns": {},
    }
    for entry in runs:
        field = "binary_query_runs" if entry["strategy"] == "binary" else "exact_query_runs"
        block = control["campaigns"].setdefault(
            entry["campaign"], {"binary_query_runs": 0, "exact_query_runs": 0}
        )
        control[field] += entry["queries"]
        block[field] += entry["queries"]
    return control


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", type=Path, default=ROOT / "data/mes01")
    parser.add_argument("--ns", type=int, nargs="+", default=list(DEFAULT_NS))
    parser.add_argument("--k", type=int, default=DEFAULT_K)
    parser.add_argument(
        "--campaigns", nargs="+", choices=("main", "filtered"), default=["main", "filtered"]
    )
    arguments = parser.parse_args()

    binary = ROOT / "target/release/examples/fidelity"
    if not binary.exists():
        raise SystemExit(f"{binary} is missing; build the example first")

    matrix = np.fromfile(arguments.data / "vectors-fp32.f32", dtype="<f4")
    matrix = matrix.reshape(-1, 768)
    texts = read_jsonl(arguments.data / "query-texts.jsonl")
    row_of = {record["id"]: row for row, record in enumerate(texts)}
    if len(row_of) != len(texts):
        raise SystemExit("query-texts.jsonl repeats an identifier")

    engine = arguments.data / "engine"
    work = arguments.data / "work"
    engine.mkdir(parents=True, exist_ok=True)
    work.mkdir(parents=True, exist_ok=True)

    summaries = []
    for campaign in arguments.campaigns:
        rows = read_jsonl(arguments.data / f"queries-{campaign}.jsonl")
        structures = STRUCTURES if campaign == "main" else FILTERED_STRUCTURES
        for structure in structures:
            selected = [record for record in rows if record["structure"] == structure]
            if not selected:
                continue
            queries = work / f"{campaign}-{structure}.jsonl"
            vectors = work / f"{campaign}-{structure}.f32"
            write_jsonl(queries, [{"id": record["id"], "filter": record["filter"]} for record in selected])
            subset = matrix[[row_of[record["id"]] for record in selected]]
            subset.tofile(vectors)

            out_dir = engine / campaign / structure
            out_dir.mkdir(parents=True, exist_ok=True)

            set_rescore(arguments.data / "indexes" / structure, 1000)
            report = run_example(
                binary,
                arguments.data,
                structure,
                queries,
                vectors,
                out_dir / "exact.json",
                exact=True,
                k=arguments.k,
            )
            check_exact(report)
            summaries.append(
                {
                    "campaign": campaign,
                    "structure": structure,
                    "run": "exact",
                    "strategy": "exact",
                    "queries": report["queries"],
                    "pool_sizes": sorted({record["pool_size"] for record in report["results"]}),
                }
            )

            for n in arguments.ns:
                set_rescore(arguments.data / "indexes" / structure, n)
                report = run_example(
                    binary,
                    arguments.data,
                    structure,
                    queries,
                    vectors,
                    out_dir / f"n{n}.json",
                    exact=False,
                    k=arguments.k,
                )
                check_binary(report, n, arguments.k)
                summaries.append(
                    {
                        "campaign": campaign,
                        "structure": structure,
                        "run": f"n{n}",
                        "strategy": "binary",
                        "queries": report["queries"],
                        "pool_sizes": sorted({record["pool_size"] for record in report["results"]}),
                    }
                )

    runs = merge_runs(arguments.data / "runs.json", summaries)
    control = strategy_control(runs)
    (arguments.data / "runs.json").write_text(
        json.dumps({"runs": runs, "strategy_control": control}, indent=2) + "\n",
        encoding="utf-8",
    )
    for summary in runs:
        print(
            f"{summary['campaign']:8s} {summary['structure']:10s} {summary['run']:6s} "
            f"{summary['queries']:3d} queries, pools {summary['pool_sizes']}"
        )
    print(
        f"{len(runs)} runs recorded: {control['binary_query_runs']} BINARY query-plans, "
        f"{control['exact_query_runs']} EXACT query-plans"
    )
    print("engine runs under", engine)


if __name__ == "__main__":
    sys.exit(main())
