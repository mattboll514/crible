# SPDX-License-Identifier: Apache-2.0
"""Aggregation of MES-02, MES-03 and MES-09 (ADR-0077).

It reads the raw documents written by the harness
(`crates/crible/examples/dense_latency.rs`, `data/mes02/raw/`), recomputes every
percentile from the per-query durations, checks the figures against the ones the
harness recorded, and writes the three versioned artefacts of
`docs/mesures/`. It prints the Markdown tables of the reports.

Percentiles are the nearest-rank definition ADR-0077 fixed: the value of rank
`ceil(ratio × n)`, ranked from 1. No interpolation, no trimmed mean.

Run from the repository root::

    (cd tools && uv run --no-sync python bench/mes02/mes02_analyze.py \\
        --commit "$(git rev-parse HEAD)" --date 2026-10-27 \\
        --machine "MacBook Air M3, 8 cores, 24 GB, macOS 15.7.7, native")
"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path

#: Root of the repository: a path recorded by the harness is made relative to
#: it, so a versioned artefact never carries a local absolute path.
ROOT = Path(__file__).resolve().parents[3]
#: Root of the measurement copy of `synth-large` (see the reports).
DEFAULT_RAW = ROOT / "data" / "mes02" / "raw"
#: Where the versioned artefacts go.
DEFAULT_MEASURES = ROOT / "docs" / "mesures"

#: File names of the artefacts, one per measure.
ARTEFACTS = {
    "MES-02": "MES-02-latence-scan-binaire",
    "MES-03": "MES-03-latence-rescoring",
    "MES-09": "MES-09-ram-reelle",
}


def nearest_rank(values: list[float], ratio: float) -> float:
    """Returns the nearest-rank percentile of a series (ADR-0077).

    Rank is `ceil(ratio × n)`, clamped to `[1, n]`, and the result is the value
    at that rank in the sorted series: no interpolation between two samples.
    """
    if not values:
        raise ValueError("nearest_rank of an empty series")
    if not 0.0 < ratio <= 1.0:
        raise ValueError(f"ratio out of (0, 1]: {ratio}")
    ordered = sorted(values)
    rank = math.ceil(ratio * len(ordered))
    return ordered[min(max(rank, 1), len(ordered)) - 1]


def summarise(values: list[float]) -> dict:
    """Count, p50, p95, p99, mean, min and max of a series, in microseconds."""
    if not values:
        raise ValueError("summarise of an empty series")
    return {
        "n": len(values),
        "p50": nearest_rank(values, 0.50),
        "p95": nearest_rank(values, 0.95),
        "p99": nearest_rank(values, 0.99),
        "mean": math.fsum(values) / len(values),
        "min": min(values),
        "max": max(values),
    }


def relative(path: str | None) -> str | None:
    """Returns `path` relative to the repository root.

    The harness may be given absolute paths (the recipe passes relative ones on
    purpose), and its documents land in the versioned artefacts: the field is
    normalised here rather than trusting every caller. A path outside the
    repository is refused, because no artefact may name a local directory.
    """
    if path is None:
        return None
    candidate = Path(path)
    if not candidate.is_absolute():
        return candidate.as_posix()
    try:
        return candidate.relative_to(ROOT).as_posix()
    except ValueError as error:
        raise ValueError(
            f"{path} is outside the repository: an artefact must not carry a local path"
        ) from error


def read(path: Path) -> dict:
    """Reads one raw document."""
    if not path.exists():
        raise FileNotFoundError(f"missing raw document: {path}")
    return json.loads(path.read_text(encoding="utf-8"))


def check_summary(values: list[float], recorded: dict | None, label: str) -> dict:
    """Recomputes a series and refuses to differ from the harness's summary."""
    computed = summarise(values)
    if recorded is not None:
        for key in ("n", "p50", "p95", "p99", "max"):
            if not math.isclose(float(recorded[key]), float(computed[key]), rel_tol=0, abs_tol=1e-6):
                raise ValueError(
                    f"{label}: {key} is {recorded[key]} in the raw document and "
                    f"{computed[key]} recomputed"
                )
    return computed


def scan_rows(raw: Path) -> list[dict]:
    """Rows of the three scan campaigns, each point recomputed and checked."""
    rows: list[dict] = []
    campaigns = {
        "scan-main": "automatic",
        "scan-forced-dense": "dense",
        "scan-forced-sparse": "sparse",
        "scan-all": "automatic",
    }
    for campaign, forced in campaigns.items():
        document = read(raw / f"{campaign}.json")
        for point in document["points"]:
            durations = point["durations_us"]
            if point.get("summary") is None:
                raise ValueError(f"{campaign}: point without summary")
            summary = check_summary(
                durations,
                point["summary"],
                f"{campaign} size={point['size']} shape={point['shape']} n={point['n']}",
            )
            rows.append(
                {
                    "campaign": campaign,
                    "size": point["size"],
                    "requested_size": point.get("requested_size"),
                    "shape": point.get("shape", "all-live"),
                    "span": point.get("span"),
                    "n": point["n"],
                    "queries": point["queries"],
                    "passes": point["passes"],
                    "plan": point["plan"],
                    "traversal": point["traversal"],
                    "traversal_automatic": point.get("traversal_automatic"),
                    "traversal_forced": forced,
                    "durations_us": durations,
                    "us": summary,
                }
            )
    rows.sort(key=lambda row: (row["campaign"], row["size"], row["shape"], row["n"]))
    return rows


def exact_rows(raw: Path) -> list[dict]:
    """Rows of the EXACT reference (pool equal to N, k = 100)."""
    rows = []
    for point in read(raw / "exact.json")["points"]:
        rows.append(
            {
                "pool_size": point["pool_size"],
                "k": point["k"],
                "queries": point["queries"],
                "passes": point["passes"],
                "score_ties": point["score_ties"],
                "durations_us": point["durations_us"],
                "us": check_summary(
                    point["durations_us"],
                    point["summary"],
                    f"exact pool={point['pool_size']}",
                ),
            }
        )
    return rows


def rescore_point(document: dict, point: dict, label: str) -> dict:
    """One rescoring point: engine total, dot product alone, key read alone."""
    row = {
        "label": label,
        "n": point["n"],
        "queries": point["queries"],
        "passes": point["passes"],
        "cold": point["cold"],
        "plan": point.get("plan", document.get("plan")),
        "displace_gb": document.get("displace_gb"),
        "score_ties": point["score_ties"],
        "resident_fraction": None,
        "engine_us": None,
        "dot_only_us": None,
        "key_only_us": None,
        "durations_us": point["durations_us"],
        "dot_only_durations_us": point.get("dot_only_us"),
        "key_only_durations_us": point.get("key_only_us"),
    }
    if point["summary"] is not None:
        row["engine_us"] = check_summary(
            point["durations_us"], point["summary"], f"{label} n={point['n']} engine"
        )
    if point.get("dot_only_summary") is not None:
        row["dot_only_us"] = check_summary(
            point["dot_only_us"], point["dot_only_summary"], f"{label} n={point['n']} dot"
        )
    if point.get("key_only_summary") is not None:
        row["key_only_us"] = check_summary(
            point["key_only_us"], point["key_only_summary"], f"{label} n={point['n']} key"
        )
    residency = point.get("resident_fraction")
    if residency is not None:
        # ADR-0077, decision 2 asks for the proportion of already resident pages
        # **per query**: the whole series is kept, not only its bounds.
        row["resident_fraction"] = {
            "mean": residency["mean"],
            "min": residency["min"],
            "max": residency["max"],
            "first": residency["values"][0],
            "last": residency["values"][-1],
            "values": residency["values"],
        }
    row["vectors"] = relative(document.get("vectors"))
    row["threads"] = document.get("threads")
    return row


def rescore_rows(raw: Path) -> list[dict]:
    """Every rescoring campaign, warm first, then the two cold series."""
    rows: list[dict] = []
    warm = read(raw / "rescore-warm.json")
    for point in warm["points"]:
        rows.append(rescore_point(warm, point, "warm-engine"))
    key_only = read(raw / "key-only.json")
    for point in key_only["points"]:
        rows.append(rescore_point(key_only, point, "warm-key-only-checked"))
    # Control: the same key reads, first in the process, without the engine
    # warming the pages just before them.
    isolated = raw / "key-only-isolated.json"
    if isolated.exists():
        document = read(isolated)
        for point in document["points"]:
            rows.append(rescore_point(document, point, "key-only-first"))
    for n in (1000, 2000, 5000):
        for label, name in (
            ("cold-vectors", f"rescore-cold-vectors-{n}.json"),
            ("cold-engine", f"rescore-cold-engine-{n}.json"),
        ):
            path = raw / name
            if not path.exists():
                continue
            document = read(path)
            for point in document["points"]:
                rows.append(rescore_point(document, point, label))
    idle = raw / "rescore-idle.json"
    if idle.exists():
        document = read(idle)
        for point in document["points"]:
            rows.append(rescore_point(document, point, "idle-10min"))
    rows.sort(key=lambda row: (row["n"], row["label"]))
    return rows


def control_rows(raw: Path) -> dict:
    """The anchoring controls of MES-02: replica against engine, engine against the CLI.

    The raw documents are gitignored, and the claim that the replica matches the
    engine rests entirely on them; the summary is therefore versioned with the
    figures.
    """
    compare: list[dict] = []
    documents: dict[str, dict] = {}
    for name in ("compare-cat01", "compare-all"):
        path = raw / f"{name}.json"
        if not path.exists():
            continue
        document = read(path)
        documents[name] = document
        compare.append(
            {
                "campaign": name,
                "filter": document.get("filter"),
                "pool_size": document["pool_size"],
                "plan": document["plan"],
                "n": document["n"],
                "top": document["top"],
                "identical": document["identical"],
            }
        )
    cli: list[dict] = []
    for name, compared in (("cli-cat01", "compare-cat01"), ("cli-all", "compare-all")):
        path = raw / f"{name}.json"
        if not path.exists() or compared not in documents:
            continue
        results = read(path)
        engine = documents[compared]
        top = len(results["results"])
        engine_top = engine["engine"][:top]
        scores = [item["score"] for item in results["results"]]
        engine_scores = [item["score"] for item in engine_top]
        cli.append(
            {
                "campaign": name,
                "pool_size": results["pool_size"],
                "top": top,
                "same_keys_as_engine": [item["key"] for item in results["results"]]
                == [item["key"] for item in engine_top],
                "max_score_delta": max(
                    (abs(left - right) for left, right in zip(scores, engine_scores)),
                    default=0.0,
                ),
            }
        )
    return {"compare": compare, "cli": cli}


def rss_states(raw: Path) -> dict:
    """The three states of MES-09, taken from the runs that carry `ps_rss_bytes`.

    Anonymous memory is the resident total of `ps` minus the resident pages
    mapped from files (`mapped file` of `vmmap`); the footprint adds the pages
    the system compressed or swapped out, so it stays meaningful when the
    machine is under pressure and pages leave the resident set.
    """
    runs: list[dict] = []
    for path in sorted(raw.glob("rss-base-*.json")):
        runs.append(rss_run(read(path), path.stem))
    model_runs: list[dict] = []
    for path in sorted(raw.glob("rss-model-cold*.json")):
        model_runs.append(rss_run(read(path), path.stem))
    if not runs:
        raise FileNotFoundError("no rss-base-*.json under the raw directory")
    return {"index_runs": runs, "model_runs": model_runs}


def state_of(state: dict) -> dict:
    """Splits one `vmmap` state into anonymous and file-backed memory."""
    if "ps_rss_bytes" not in state:
        raise ValueError("state without ps_rss_bytes: re-run the rss mode")
    resident = int(state["ps_rss_bytes"])
    mapped = int(state["mapped_file_bytes"])
    return {
        "resident_bytes": resident,
        "anonymous_bytes": resident - mapped,
        "mapped_file_bytes": mapped,
        "swapped_bytes": int(state["swapped_bytes"]),
        "footprint_bytes": int(state["physical_footprint_bytes"]),
    }


def rss_run(document: dict, label: str) -> dict:
    """One `rss` invocation: the two index states, plus the model state."""
    run = {
        "label": label,
        "live_passages": document["live_passages"],
        "codes_bytes": document["codes_bytes"],
        "filter_memory_bytes": document["filter_memory_bytes"],
        "queries": document["queries"],
        "passes": document["passes"],
        "states": {
            "after_open": state_of(document["states"]["after_open"]),
            "after_searcher": state_of(document["states"]["after_searcher"]),
            "after_pass": state_of(document["states"]["after_pass"]),
        },
    }
    if document.get("model"):
        run["model"] = {
            "before": state_of(document["model"]["before"]),
            "after": state_of(document["model"]["after"]),
        }
    return run


def build(raw: Path, *, commit: str, date: str, machine: str) -> dict[str, dict]:
    """Builds the three artefacts from the raw directory."""
    queries = json.loads((raw.parent / "queries.json").read_text(encoding="utf-8"))
    scan = scan_rows(raw)
    exact = exact_rows(raw)
    rescore = rescore_rows(raw)
    rss = rss_states(raw)
    all_live = read(raw / "scan-all.json")
    common = {
        "commit": commit,
        "date": date,
        "machine": machine,
        "dataset": {
            "name": "synth-large",
            "index": "data/mes02/indexes/synth-large (measurement copy)",
            "live_passages": all_live["live_passages"],
            "dims": all_live["dims"],
            "queries": queries["queries"],
            "queries_sha256": queries["sha256"],
            "queries_law": queries["law"],
            "queries_seed": queries["seed"],
            "queries_stream": queries["stream"],
        },
        "protocol": "ADR-0077",
    }
    return {
        "MES-02": {
            **common,
            "measure": "MES-02",
            "controls": control_rows(raw),
            "scan": scan,
            "exact": exact,
        },
        "MES-03": {**common, "measure": "MES-03", "rescoring": rescore},
        "MES-09": {**common, "measure": "MES-09", **rss},
    }


def markdown(artefacts: dict[str, dict]) -> str:
    """Prints the tables the reports quote, in microseconds or mebibytes."""
    lines: list[str] = []
    lines.append("## MES-02 — scan (µs)")
    lines.append("")
    lines.append("| campagne | taille | forme | N | parcours | p50 | p95 | p99 | max |")
    lines.append("|---|---|---|---|---|---|---|---|---|")
    for row in artefacts["MES-02"]["scan"]:
        if row["campaign"] not in ("scan-main", "scan-all"):
            continue
        us = row["us"]
        lines.append(
            f"| {row['campaign']} | {row['size']} | {row['shape']} | {row['n']} | "
            f"{row['traversal']} | {us['p50']:.0f} | {us['p95']:.0f} | {us['p99']:.0f} | "
            f"{us['max']:.0f} |"
        )
    lines.append("")
    lines.append("## MES-02 — parcours forcés (µs)")
    lines.append("")
    lines.append("| campagne | taille | forme | N | p50 | p95 | p99 |")
    lines.append("|---|---|---|---|---|---|---|")
    for row in artefacts["MES-02"]["scan"]:
        if row["campaign"] not in ("scan-forced-dense", "scan-forced-sparse"):
            continue
        us = row["us"]
        lines.append(
            f"| {row['campaign']} | {row['size']} | {row['shape']} | {row['n']} | "
            f"{us['p50']:.0f} | {us['p95']:.0f} | {us['p99']:.0f} |"
        )
    lines.append("")
    lines.append("## MES-02 — EXACT au point de bascule (µs)")
    lines.append("")
    lines.append("| pool | k | p50 | p95 | p99 | max |")
    lines.append("|---|---|---|---|---|---|")
    for row in artefacts["MES-02"]["exact"]:
        us = row["us"]
        lines.append(
            f"| {row['pool_size']} | {row['k']} | {us['p50']:.0f} | {us['p95']:.0f} | "
            f"{us['p99']:.0f} | {us['max']:.0f} |"
        )
    lines.append("")
    lines.append("## MES-03 — rescoring (µs)")
    lines.append("")
    lines.append("| série | N | moteur p50 | p95 | p99 | dot seul p50 | clés seules p50 |")
    lines.append("|---|---|---|---|---|---|---|")
    for row in artefacts["MES-03"]["rescoring"]:
        engine = row["engine_us"]
        dot = row["dot_only_us"]
        key = row["key_only_us"]
        lines.append(
            f"| {row['label']} | {row['n']} | "
            + (f"{engine['p50']:.0f} | {engine['p95']:.0f} | {engine['p99']:.0f} | " if engine else "— | — | — | ")
            + (f"{dot['p50']:.0f} | " if dot else "— | ")
            + (f"{key['p50']:.0f} |" if key else "— |")
        )
    lines.append("")
    lines.append("## MES-09 — mémoire (MiB)")
    lines.append("")
    lines.append("| exécution | état | RSS | anonyme | fichiers | swap | empreinte |")
    lines.append("|---|---|---|---|---|---|---|")
    for measure in artefacts["MES-09"]["index_runs"] + artefacts["MES-09"]["model_runs"]:
        for name, state in measure["states"].items():
            lines.append(
                f"| {measure['label']} | {name} | {state['resident_bytes'] / 2**20:.0f} | "
                f"{state['anonymous_bytes'] / 2**20:.0f} | "
                f"{state['mapped_file_bytes'] / 2**20:.0f} | "
                f"{state['swapped_bytes'] / 2**20:.0f} | "
                f"{state['footprint_bytes'] / 2**20:.0f} |"
            )
        if "model" in measure:
            for name, state in measure["model"].items():
                lines.append(
                    f"| {measure['label']} | modèle {name} | "
                    f"{state['resident_bytes'] / 2**20:.0f} | "
                    f"{state['anonymous_bytes'] / 2**20:.0f} | "
                    f"{state['mapped_file_bytes'] / 2**20:.0f} | "
                    f"{state['swapped_bytes'] / 2**20:.0f} | "
                    f"{state['footprint_bytes'] / 2**20:.0f} |"
                )
    return "\n".join(lines)


def write(artefacts: dict[str, dict], measures: Path, date: str) -> list[Path]:
    """Writes one dated artefact per measure; returns the paths written."""
    measures.mkdir(parents=True, exist_ok=True)
    written = []
    for measure, document in artefacts.items():
        path = measures / f"{ARTEFACTS[measure]}.{date}.json"
        path.write_text(json.dumps(document, indent=1, sort_keys=True) + "\n", encoding="utf-8")
        written.append(path)
    return written


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--raw", type=Path, default=DEFAULT_RAW)
    parser.add_argument("--mesures", type=Path, default=DEFAULT_MEASURES)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--date", required=True)
    parser.add_argument("--machine", required=True)
    arguments = parser.parse_args(argv)
    artefacts = build(
        arguments.raw,
        commit=arguments.commit,
        date=arguments.date,
        machine=arguments.machine,
    )
    for path in write(artefacts, arguments.mesures, arguments.date):
        print(f"wrote {path}")
    print()
    print(markdown(artefacts))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
