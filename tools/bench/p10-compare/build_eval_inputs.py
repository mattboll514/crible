#!/usr/bin/env python3
"""Build the four ``crible eval`` games of each index (pass B of P10).

For each of the six indexes of ``docs/legifacile/reference/``:

- ``<index>-dense-primaire.jsonl``: the same lines with the **pgvector query
  vector** of ``data/p10-compare/query-vectors.json`` instead of the text
  (ADR-0125, decision 2; ADR-0140, decision 3);
- ``<index>-dense-secondaire.jsonl``: the textual game, encoded by Crible;
- ``<index>-hybride.jsonl``: the textual game, fused by the index default;
- ``<index>-fidelite.jsonl``: a copy of the dense primary game, so the fidelity
  campaign has its own report name (same vectors, ``--exact-threshold 0``).

The files carry query texts and filter values: they stay under ``data/``,
gitignored (ADR-0125, decision 16). The script prints one SHA-256 per file.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import pass_b
import queries as game


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--reference-dir",
        default="docs/legifacile/reference",
        help="folder holding the six <index>.jsonl files",
    )
    parser.add_argument(
        "--vectors",
        default="data/p10-compare/query-vectors.json",
        help="pass A query vectors",
    )
    parser.add_argument(
        "--output-dir",
        default="data/p10-compare/eval/inputs",
        help="input folder to write (under data/, gitignored)",
    )
    args = parser.parse_args(argv)

    try:
        pass_b.check_populations()
        reference = pass_b.load_reference(args.reference_dir)
        vectors = pass_b.load_vectors(args.vectors)
        output = Path(args.output_dir)
        for index, rows in reference.items():
            primary = pass_b.vector_game(rows, vectors)
            textual = pass_b.text_game(rows)
            outputs = {
                "dense-primaire": primary,
                "dense-secondaire": textual,
                "hybride": textual,
                "fidelite": primary,
            }
            for pass_name, entries in outputs.items():
                path = output / f"{index}-{pass_name}.jsonl"
                digest = pass_b.write_jsonl(path, entries)
                print(f"{path} {len(entries)} {digest}")
    except (pass_b.PassBError, game.P10Error) as exc:
        print(f"p10-compare pass B inputs: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
