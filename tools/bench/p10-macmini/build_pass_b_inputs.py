#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Build the pass B input games of `MESURER-SUR-MAC-MINI` (ADR-0141).

Three families, one folder each:

- `<index>-dense-secondaire-macmini.jsonl`: the 97 reference lines in **text**,
  re-encoded by Crible (the constrained pass, ADR-0141 decision 4);
- `<index>-dense-primaire-macmini.jsonl`: the same lines with the **pgvector
  query vector** of pass A of `COMPARER-AVEC-PGVECTOR`;
- `<index>-hybride-macmini.jsonl`: the textual lines, fused by the index;
- `<index>-mes00-1000-titres-macmini.jsonl`: the 1 000 MES-00 titles split by
  index, with the perimeters of `docs/legifacile/index-legifacile.md`.

It also writes the descriptors of the top-10 control (`top10/<index>.json`) and
one JSON vector file per query, which `crible search --vector-file` reads.

The files carry query texts and filter values: they stay under `data/`,
gitignored (ADR-0125, decision 16). One SHA-256 is printed per file.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

import macmini

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "p10-compare"))
import pass_b  # noqa: E402
import queries as game  # noqa: E402


def write_jsonl(path: Path, rows) -> str:
    payload = "".join(
        json.dumps(row, ensure_ascii=False, separators=(",", ":")) + "\n"
        for row in rows
    ).encode("utf-8")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(payload)
    return hashlib.sha256(payload).hexdigest()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference-dir", default="docs/legifacile/reference")
    parser.add_argument("--vectors", default="data/p10-compare/query-vectors.json")
    parser.add_argument("--mes00", default="data/mes00/queries.json")
    parser.add_argument("--output-dir", default="data/p10-macmini/inputs")
    args = parser.parse_args(argv)

    try:
        pass_b.check_populations()
        reference = pass_b.load_reference(args.reference_dir)
        vectors = pass_b.load_vectors(args.vectors)
        titles = game.read_json(args.mes00)["queries"]
        output = Path(args.output_dir)
        digests: dict[str, str] = {}

        titles_games = macmini.titles_game(titles)
        counts: dict[str, int] = {}
        for index, rows in reference.items():
            textual = pass_b.text_game(rows)
            primary = pass_b.vector_game(rows, vectors)
            # The cold series replays the textual dense game under its own
            # report slug, so no published name is ever overwritten.
            files = {
                "dense-texte": textual,
                "dense-vecteur": primary,
                "hybride": textual,
                "dense-texte-froid": textual,
                "mes00-1000-titres": titles_games.get(index, []),
            }
            for family, entries in files.items():
                if not entries:
                    raise macmini.MacMiniError(f"{index}: the {family} game is empty")
                path = output / f"{index}-{family}-macmini.jsonl"
                digests[path.name] = write_jsonl(path, entries)

            # The top-10 control replays the dense primary pass of the same
            # machine: same vector, same filter, `crible search` instead of
            # `crible eval`.
            descriptors = macmini.search_vectors(primary, vectors)
            vector_dir = output / "top10" / index
            for descriptor in descriptors:
                target = vector_dir / descriptor["vector"]
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(
                    json.dumps(vectors[descriptor["id"]], separators=(",", ":")) + "\n"
                )
                if "filter" in descriptor:
                    (vector_dir / f"{descriptor['id']}.filter.json").write_text(
                        json.dumps(descriptor["filter"], separators=(",", ":")) + "\n"
                    )
            descriptor_path = output / "top10" / f"{index}.json"
            descriptor_path.write_text(
                json.dumps(descriptors, ensure_ascii=False, indent=2) + "\n"
            )
            counts[index] = {
                "reference": len(rows),
                "titles": len(titles_games.get(index, [])),
            }

        for name, digest in sorted(digests.items()):
            print(f"{name} {digest}")
        print(
            "counts "
            + json.dumps(
                {
                    "reference": sum(entry["reference"] for entry in counts.values()),
                    "titles": sum(entry["titles"] for entry in counts.values()),
                    "per_index": counts,
                },
                sort_keys=True,
            )
        )
    except (macmini.MacMiniError, pass_b.PassBError, game.P10Error) as exc:
        print(f"p10-macmini pass B inputs: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
