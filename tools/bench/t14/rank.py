#!/usr/bin/env python3
"""Rank of an expected document for the identifier probes (MES-11, campaign T14).

The measure of ADR-0148 needs the absolute lexical and hybrid rank of a document
on the full indexes, exactly as the ticket 14 of Legifacile read it with
`explain_ids`. The CLI has no `explain_ids` option, so this harness asks
`crible search` for `k = 1000` results and locates the document in the answer.
A document absent from the 1 000 first results is reported as `null` with
`present_in_top: 1000`, which is a bound, never a rank.

Two conventions are reported side by side, because they do not agree:

- `flat`: one row per passage, the answer of `crible search` without
  `--group-by document`. It is the convention of the ticket (234 for
  `10-13.625` on `decisions`) and the strictest of the two.
- `grouped`: one row per document (`--group-by document`), the shape the
  product serves after `dedupliquer: document`.

Usage:

    python3 tools/bench/t14/rank.py --data data/t14-mes11/data \
        --models data/models --out data/t14-mes11/raw/ranks-pass1.json

The script writes nothing but the JSON of `--out`; it never touches an index.
"""

import argparse
import json
import subprocess
import sys

BINARY = "./target/release/crible"
K = 1000

# One row per probe: the index that carries the expected document, the text of
# the query, and the document the ticket names. `expected = None` is the
# negative control: it has no expected document, only a document that must not
# appear in the first three rows.
PROBES = [
    {
        "id": "T14-DEC-10-13-625",
        "index": "decisions",
        "query": "10-13.625",
        "expected": "JURITEXT000023607955",
        "role": "criterion",
    },
    {
        "id": "T14-DEC-81-70-489",
        "index": "decisions",
        "query": "81-70.489",
        "expected": "JURITEXT000007009951",
        "role": "complement",
    },
    {
        "id": "T14-ART-L1132-1",
        "index": "articles",
        "query": "L1132-1",
        "expected": "LEGIARTI000045391841",
        "role": "complement",
    },
    {
        "id": "T14-ART-L-1132-1-CITE",
        "index": "articles",
        "query": "L. 1132-1",
        "expected": "LEGIARTI000045391841",
        "role": "complement",
    },
    {
        "id": "T14-ART-NEGATIF-l1132-11",
        "index": "articles",
        "query": "l1132-11",
        "expected": "LEGIARTI000045391841",
        "role": "negative_control",
    },
    {
        "id": "T14-SARDE-NOR",
        "index": "sarde",
        "query": "SANP0422530D",
        "expected": "SARDBLOC000007251655",
        "role": "complement",
    },
]


def search(data, models, index, query, mode, grouped):
    command = [
        BINARY,
        "search",
        "--data",
        data,
        "--index",
        index,
        "--query",
        query,
        "--mode",
        mode,
        "--k",
        str(K),
    ]
    if grouped:
        command += ["--group-by", "document"]
    if mode != "lexical":
        command += ["--models", models]
    done = subprocess.run(command, capture_output=True, text=True)
    if done.returncode != 0:
        raise SystemExit(
            f"crible search failed ({' '.join(command)}):\n{done.stderr}"
        )
    return json.loads(done.stdout)


def rank_of(answer, doc_id):
    for position, row in enumerate(answer["results"], start=1):
        if row["doc_id"] == doc_id:
            return position
    return None


def measure(data, models, probe):
    out = {
        "id": probe["id"],
        "index": probe["index"],
        "query": probe["query"],
        "expected": probe["expected"],
        "role": probe["role"],
        "k": K,
    }
    for mode in ("lexical", "hybrid"):
        for grouped in (False, True):
            key = f"{mode}_{'grouped' if grouped else 'flat'}"
            answer = search(data, models, probe["index"], probe["query"], mode, grouped)
            out[key] = {
                "rank": rank_of(answer, probe["expected"]),
                "n_results": len(answer["results"]),
                "pool_size": answer["pool_size"],
                "took_ms": answer["took_ms"],
                "truncated_query": answer.get("truncated_query"),
                "top3": [row["doc_id"] for row in answer["results"][:3]],
            }
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", required=True)
    parser.add_argument("--models", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument(
        "--pass-name",
        default="",
        help="Label of the pass (`none` or `v1`), copied into the JSON.",
    )
    args = parser.parse_args()

    probes = [measure(args.data, args.models, probe) for probe in PROBES]
    document = {"pass": args.pass_name, "k": K, "probes": probes}
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump(document, handle, ensure_ascii=False, indent=1)
        handle.write("\n")
    for probe in probes:
        flat = probe["lexical_flat"]["rank"]
        grouped = probe["lexical_grouped"]["rank"]
        hybrid = probe["hybrid_flat"]["rank"]
        print(f"{probe['id']}: lexical flat {flat}, grouped {grouped}, hybrid flat {hybrid}")


if __name__ == "__main__":
    sys.exit(main())
