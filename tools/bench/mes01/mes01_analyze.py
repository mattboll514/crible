# SPDX-License-Identifier: Apache-2.0
"""Aggregate the fidelity measurements of MES-01 and MES-12.

Two sections, written as two versioned artefacts:

- ``MES-01``: the engine's BINARY against its own EXACT, per structure, per ``N``
  and in global mean. This is the figure the 0.99 target of `docs/08` applies
  to (ADR-0076, decisions 1 and 2);
- ``MES-12``: the out-of-engine centering harness, variants ``raw`` (the
  bit-exact control), ``a`` (principal) and ``c`` (indication), compared to the
  same EXACT reference.

Run from the repository root, after ``mes01_campaign.py``, ``mes01_controls.py``
and ``mes01_centering.py``::

    (cd tools && uv run --no-sync python bench/mes01/mes01_analyze.py \
        --commit <sha> --date 2026-10-27 --machine "<machine measured>" \
        --mesures ../docs/mesures)
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

import mes01_metrics as metrics

#: Repository root, three levels above this file.
ROOT = Path(__file__).resolve().parents[3]
#: Structures of the sample, in the order of `docs/08`.
STRUCTURES = ("articles", "decisions", "dole", "jorf", "sarde", "titres")
#: Structures of the filtered campaign.
FILTERED_STRUCTURES = ("articles", "decisions")
#: Campaigns of ADR-0076.
CAMPAIGNS = ("main", "filtered")
#: Variants of the centering harness, in the order of the report.
VARIANTS = ("raw", "a", "c")


def read_json(path: Path) -> dict:
    """Reads one JSON object."""
    return json.loads(path.read_text(encoding="utf-8"))


def read_json_lines(path: Path) -> list[dict]:
    """Returns the JSON objects of a JSONL file, in order."""
    with path.open(encoding="utf-8") as handle:
        return [json.loads(line) for line in handle if line.strip()]


def structures_of(campaign: str) -> tuple[str, ...]:
    """Structures measured by one campaign."""
    return STRUCTURES if campaign == "main" else FILTERED_STRUCTURES


def engine_tables(data: Path, campaign: str, ns: list[int]) -> dict:
    """Per-structure, per-query and global means of BINARY against EXACT."""
    tables: dict = {"structures": {}, "global": {}, "per_query": []}
    globals_: dict[str, list[dict]] = {str(n): [] for n in ns}
    for structure in structures_of(campaign):
        exact = read_json(data / f"engine/{campaign}/{structure}/exact.json")
        exact_by_id = {record["id"]: record for record in exact["results"]}
        rows: dict[str, list[dict]] = {str(n): [] for n in ns}
        per_query: dict[str, dict] = {
            query_id: {"id": query_id, "structure": structure} for query_id in exact_by_id
        }
        for n in ns:
            binary = read_json(data / f"engine/{campaign}/{structure}/n{n}.json")
            binary_by_id = {record["id"]: record for record in binary["results"]}
            for query_id, record in binary_by_id.items():
                row = {
                    "id": query_id,
                    "recall": metrics.recall_at_10(record, exact_by_id[query_id]),
                    "overlap": metrics.overlap_at_10(record, exact_by_id[query_id]),
                }
                rows[str(n)].append(row)
                globals_[str(n)].append(row)
                per_query[query_id][str(n)] = {
                    "recall": row["recall"],
                    "overlap": row["overlap"],
                }
        tables["structures"][structure] = {
            "queries": len(next(iter(rows.values()))),
            "pool": exact["results"][0]["pool_size"],
            "ns": {
                str(n): {
                    "recall": metrics.summarise(rows[str(n)], "recall"),
                    "overlap": metrics.summarise(rows[str(n)], "overlap"),
                }
                for n in ns
            },
        }
        tables["per_query"].extend(per_query.values())
    tables["global"] = {
        str(n): {
            "recall": metrics.summarise(globals_[str(n)], "recall"),
            "overlap": metrics.summarise(globals_[str(n)], "overlap"),
        }
        for n in ns
    }
    return tables


def centring_tables(centering: dict, campaign: str, ns: list[int]) -> dict:
    """Per-structure, per-query and global means of the three harness variants."""
    structures = centering["campaigns"][campaign]
    tables: dict = {"structures": {}, "global": {}, "per_query": []}
    totals: dict[str, dict[str, list[float]]] = {
        variant: {str(n): [] for n in ns} for variant in VARIANTS
    }
    overlaps: dict[str, dict[str, list[float]]] = {
        variant: {str(n): [] for n in ns} for variant in VARIANTS
    }
    for structure, block in structures.items():
        if not block["control_ok"] or block["codes_control"]["mismatched_words"]:
            raise SystemExit(
                f"{campaign}/{structure}: the centring harness failed its control; "
                "no figure is kept (ADR-0076, decision 3)"
            )
        rows = block["queries"]
        tables["structures"][structure] = {
            "passages": block["passages"],
            "pool": block["pool"],
            "control_ok": block["control_ok"],
            "codes_mismatched_words": block["codes_control"]["mismatched_words"],
            "control_comparisons": block["control_comparisons"],
            "mean_norm": block["mean_norm"],
            "ns": {
                variant: {
                    str(n): {
                        "recall": metrics.mean(
                            [row[variant][str(n)]["recall"] for row in rows.values()]
                        ),
                        "overlap": metrics.mean(
                            [row[variant][str(n)]["overlap"] for row in rows.values()]
                        ),
                        "perfect": sum(
                            1
                            for row in rows.values()
                            if row[variant][str(n)]["recall"] == 1.0
                        ),
                    }
                    for n in ns
                }
                for variant in VARIANTS
            },
        }
        for variant in VARIANTS:
            for n in ns:
                totals[variant][str(n)].extend(
                    row[variant][str(n)]["recall"] for row in rows.values()
                )
                overlaps[variant][str(n)].extend(
                    row[variant][str(n)]["overlap"] for row in rows.values()
                )
        for query_id, row in rows.items():
            tables["per_query"].append(
                {
                    "id": query_id,
                    "structure": structure,
                    **{
                        variant: {
                            str(n): row[variant][str(n)] for n in ns
                        }
                        for variant in VARIANTS
                    },
                }
            )
    tables["global"] = {
        variant: {
            str(n): {
                "recall": metrics.mean(totals[variant][str(n)]),
                "overlap": metrics.mean(overlaps[variant][str(n)]),
                "queries": len(totals[variant][str(n)]),
                "perfect": sum(1 for value in totals[variant][str(n)] if value == 1.0),
            }
            for n in ns
        }
        for variant in VARIANTS
    }
    return tables


def sha256(path: Path) -> str:
    """SHA-256 of a file, lowercase hexadecimal."""
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def header(arguments: argparse.Namespace, measure: str) -> dict:
    """Fields every artefact carries, so it can be replayed and read alone."""
    return {
        "measure": measure,
        "task": "MESURER-FIDELITE-BINAIRE",
        "adr": "ADR-0076",
        "commit": arguments.commit,
        "date": arguments.date,
        "machine": arguments.machine,
        "ns": arguments.ns,
        "k": metrics.K,
        "datasets": {
            "sample": "data/legifacile-sample",
            "queries": {
                "path": "data/legifacile-sample/queries.jsonl",
                "sha256": sha256(ROOT / "data/legifacile-sample/queries.jsonl"),
                "main": len(read_json_lines(arguments.data / "queries-main.jsonl")),
                "filtered": len(read_json_lines(arguments.data / "queries-filtered.jsonl")),
            },
            "query_vectors": "data/mes01/vectors-fp32.f32 (engine QueryEncoder, fp32)",
            "indexes": "data/mes01/indexes (fresh attach, search.exact_threshold = 0)",
        },
        "replay": (
            "(cd tools && uv run --no-sync python bench/mes01/mes01_prepare.py) && "
            "cargo build --release && "
            "cargo build --release --example measure_queries -p crible-encode && "
            "cargo build --release --example fidelity -p crible && "
            "./target/release/examples/measure_queries vectors "
            "--model data/models/mdenseon --queries data/mes01/query-texts.jsonl "
            "--out-dir data/mes01 && "
            "(cd tools && uv run --no-sync python bench/mes01/mes01_build_indexes.py) && "
            "(cd tools && uv run --no-sync python bench/mes01/mes01_campaign.py) && "
            "(cd tools && uv run --no-sync python bench/mes01/mes01_controls.py) && "
            "(cd tools && uv run --no-sync python bench/mes01/mes01_centering.py) && "
            "(cd tools && uv run --no-sync python bench/mes01/mes01_analyze.py "
            "--commit <sha> --date <date> --machine <machine> --mesures ../docs/mesures)"
        ),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", type=Path, default=ROOT / "data/mes01")
    parser.add_argument("--commit", required=True)
    parser.add_argument("--date", required=True)
    parser.add_argument(
        "--machine",
        required=True,
        help="machine the campaign ran on, as measured; no default, so a replay "
        "on another host cannot inherit this one's description",
    )
    parser.add_argument("--ns", type=int, nargs="+", default=[200, 500, 1000, 2000, 5000])
    parser.add_argument("--out", type=Path, default=None, help="work copy of both sections")
    parser.add_argument(
        "--mesures", type=Path, default=None, help="directory of the versioned artefacts"
    )
    arguments = parser.parse_args()

    centering = read_json(arguments.data / "centering.json")
    recorded = read_json(arguments.data / "runs.json")
    runs = recorded["runs"]
    controls = read_json(arguments.data / "controls.json")

    mes01 = header(arguments, "MES-01")
    mes12 = header(arguments, "MES-12")
    mes12["variants"] = centering["variants"]
    mes01["campaigns"] = {}
    mes12["campaigns"] = {}
    for campaign in CAMPAIGNS:
        engine = engine_tables(arguments.data, campaign, arguments.ns)
        centring = centring_tables(centering, campaign, arguments.ns)
        mes01["campaigns"][campaign] = engine
        mes12["campaigns"][campaign] = {
            "engine_global": engine["global"],
            "centring": centring,
        }
    mes01["runs"] = runs
    mes01["strategy_control"] = recorded["strategy_control"]
    mes01["controls"] = controls
    mes01["residuals"] = {
        campaign: {
            structure: block["residuals"]
            for structure, block in centering["campaigns"][campaign].items()
            if block["residuals"]
        }
        for campaign in CAMPAIGNS
    }
    mes12["runs"] = runs
    mes12["strategy_control"] = recorded["strategy_control"]
    mes12["controls"] = controls

    work = arguments.out or arguments.data / "analysis.json"
    work.write_text(json.dumps({"mes01": mes01, "mes12": mes12}, indent=2) + "\n", encoding="utf-8")

    if arguments.mesures is not None:
        arguments.mesures.mkdir(parents=True, exist_ok=True)
        for name, document in (
            (f"MES-01-fidelite-scan-binaire.{arguments.date}.json", mes01),
            (f"MES-12-centrage-codes-binaires.{arguments.date}.json", mes12),
        ):
            (arguments.mesures / name).write_text(
                json.dumps(document, indent=2) + "\n", encoding="utf-8"
            )
            print("wrote", arguments.mesures / name)

    for campaign in CAMPAIGNS:
        print(f"== {campaign}")
        table = mes01["campaigns"][campaign]["global"]
        centring = mes12["campaigns"][campaign]["centring"]["global"]
        for n in arguments.ns:
            print(
                f"N={n:5d} recall {table[str(n)]['recall']['mean']:.4f} "
                f"overlap {table[str(n)]['overlap']['mean']:.4f} | "
                f"a {centring['a'][str(n)]['recall']:.4f} "
                f"c {centring['c'][str(n)]['recall']:.4f} "
                f"raw {centring['raw'][str(n)]['recall']:.4f}"
            )
    print("work copy", work)


if __name__ == "__main__":
    sys.exit(main())
