#!/usr/bin/env python3
"""ACCELERER-RECETTE — merge the posts of `validation` into the report tables.

Reads, from `data/accelere-recette/` (all gitignored):

- `posts-daily-articles-{1000,3000}.json`: the post-by-post pilot
  (`crates/crible-store/examples/acceptance_split.rs`, mode `posts`), five
  repetitions per batch;
- `both-{1000,3000}.json`: the same pilot, mode `both`, one replica before and
  one after the real `attach` of the same repetition, which brackets the
  measure against the drift of the machine;
- `load-rep-*.json` (cold load, one process each) and `load-warm.json` (five
  loads in one process);
- `lengths-*.json`: token length of the drawn sample, for the padding of a
  length-sorted batch.

It prints the table by post (p50 with `[min–max]`), the closure table, and the
padding overhead by internal batch size. Nothing is rounded away: the values
are printed as measured, and the residual keeps its sign.
"""

from __future__ import annotations

import json
import pathlib
import statistics
import sys

ROOT = pathlib.Path(__file__).resolve().parents[3]
DATA = ROOT / "data" / "accelere-recette"

BATCHES = {"1000": "daily-articles-1000", "3000": "daily-articles-3000"}

# label, key
POSTS = [
    ("chargement fp32 (processus neuf, 5 processus)", "load_cold"),
    ("chargement fp32 (rechargement dans un même processus)", "load_warm"),
    ("ouverture du lot (contrôles 4 et 5, sommes de contrôle)", "open_ms"),
    ("contrôles 6 et 7 (`validate`, une passe)", "validate_ms"),
    ("contrôle 8 : lecture des passages", "length_read_ms"),
    ("contrôle 8 : comptage des jetons", "length_count_ms"),
    ("contrôle 9 : lecture du tirage", "draw_read_ms"),
    ("contrôle 9 : tirage (priorité, texte, vecteur)", "draw_work_ms"),
    ("`plan_documents` : lecture des documents", "plan_read_ms"),
    ("`plan_documents` : lectures SQLite (réplique)", "plan_sql_ms"),
    ("recette : 200 ré-encodages", "encode_only_ms"),
    ("recette : cosinus", "cosine_ms"),
]


def read(path: pathlib.Path) -> dict:
    with path.open(encoding="utf-8") as file:
        return json.load(file)


def cell(values: list[float]) -> str:
    if not values:
        return "—"
    return f"{statistics.median(values):.1f} [{min(values):.1f}–{max(values):.1f}]"


def main() -> int:
    loads = {
        "load_cold": [
            read(path)["load_ms"][0] for path in sorted(DATA.glob("load-rep-*.json"))
        ],
        "load_warm": read(DATA / "load-warm.json")["load_ms"],
    }
    posts = {size: read(DATA / f"posts-{batch}.json") for size, batch in BATCHES.items()}

    print("== table 1: posts of `validation`, ms, p50 [min-max]\n")
    print(f"{'poste':60s} {'1 000 passages':30s} {'3 002 passages':30s}")
    for label, key in POSTS:
        if key in loads:
            values = loads[key]
            print(f"{label:60s} {cell(values):30s} {cell(values):30s}")
            continue
        row = [cell([point[key] for point in posts[size]["points"]]) for size in ("1000", "3000")]
        print(f"{label:60s} {row[0]:30s} {row[1]:30s}")

    print("\n== table 2: closure, ms (replica before, attach, replica after)\n")
    for size in ("1000", "3000"):
        for point in read(DATA / f"both-{size}.json")["points"]:
            before = point["total_ms"] + point["posts_load_fp32_ms"]
            after = point["after_posts"]
            attach = point["attach_validation_ms"]
            after_text = ", ".join(
                f"{item['total_ms'] + item['load_fp32_ms']:.1f}" for item in after
            )
            load = point["attach_load_fp32_ms"]
            recette = point["encode_only_ms"] + load
            print(
                f"{size:>5s} rep {point['repetition']}: "
                f"avant {before:.1f} | validation {attach:.1f} "
                f"(chargement {load:.1f}) | après {after_text} | "
                f"recette {recette:.1f} = {recette / attach:.1%} de validation"
            )

    print("\n== table 3: padding of a length-sorted batch, measured on the drawn sample\n")
    for size in ("1000", "3000"):
        point = read(DATA / f"lengths-{size}.json")["points"][0]
        lengths = sorted(point["sample_tokens"])
        total = sum(lengths)
        print(
            f"{size}: {len(lengths)} passages, {total} jetons, "
            f"p50 {point['sample_tokens_p50']}, p90 {point['sample_tokens_p90']}, "
            f"max {point['sample_tokens_max']}"
        )
        for batch in (1, 4, 8, 16, 32):
            # Every group pays its own longest input: the last group is smaller
            # than the nominal size, so it is weighted by its real length.
            padded = sum(
                len(group) * max(group)
                for group in (lengths[start : start + batch] for start in range(0, len(lengths), batch))
            )
            print(f"    B={batch:3d}: jetons avec remplissage {padded} ({padded / total - 1:+.1%})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
