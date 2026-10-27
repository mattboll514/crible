# SPDX-License-Identifier: Apache-2.0
"""Query vectors of MES-02 and MES-03 (ADR-0077, decision 4).

``synth-large`` carries **no query set**: ``tools/gen_synth.py`` writes one for
``synth-small`` only, and the latency dataset is documented as "for latency
only: no query set, no expected results". ADR-0077 therefore asks for at least
300 distinct ``f32`` query vectors **from the generator of `synth-large`**, at a
fixed seed, traced by SHA-256.

The generator of ``synth-large`` builds every stored vector as a standard
normal draw normalised to the unit sphere: no cluster structure, unlike
``synth-small``. The queries follow the same law, drawn from a fixed stream of
``gen_synth._rng``, so a query is distributed like a passage. That is a
deliberate difference from ``synth-small``'s queries, which mix cluster centres;
it is declared in the report, and it changes which passages the scan keeps, not
the cost of the scan.

Run from the repository root::

    (cd tools && uv run --no-sync python bench/mes02/mes02_queries.py)
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

import numpy as np

#: Repository root, three levels above this file.
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tools"))

import gen_synth  # noqa: E402  (the path is set just above)

# The generator exposes its vector law through two private helpers (`_rng`,
# `_unit_vector`). Renaming them in `tools/gen_synth.py` would silently change
# the query set of MES-02 and MES-03, whose SHA-256 is quoted by the reports: the
# campaign stops here instead. Proposed follow-up, not done here: a public entry
# point in `tools/gen_synth.py` (`unit_vectors(seed, stream, count)`).
if not (hasattr(gen_synth, "_rng") and hasattr(gen_synth, "_unit_vector")):
    raise SystemExit(
        "tools/gen_synth.py no longer exposes _rng/_unit_vector: the query set of "
        "MES-02 and MES-03 would change. Give the generator a public entry point "
        "before regenerating these queries."
    )

#: Number of query vectors (ADR-0077 asks for at least 300).
QUERIES = 300
#: Stream of the generator the queries come from (queries of `synth-small` use 4).
STREAM = 7
#: Default output directory.
DEFAULT_OUT = ROOT / "data" / "mes02"


def query_vectors(seed: int, count: int = QUERIES) -> np.ndarray:
    """Returns `count` unit vectors of `gen_synth.DIMS` components, `float32`."""
    rng = gen_synth._rng(seed, STREAM)
    rows = np.empty((count, gen_synth.DIMS), dtype=np.float32)
    for index in range(count):
        rows[index] = gen_synth._unit_vector(rng).astype(np.float32)
    return rows


def write(out_dir: Path, seed: int, count: int = QUERIES) -> dict:
    """Writes the matrix and its manifest; returns the manifest."""
    out_dir.mkdir(parents=True, exist_ok=True)
    rows = query_vectors(seed, count)
    if len({row.tobytes() for row in rows}) != count:
        raise SystemExit("the generator produced a duplicate query vector")
    matrix = out_dir / "queries.f32"
    rows.astype("<f4").tofile(matrix)
    digest = hashlib.sha256(matrix.read_bytes()).hexdigest()
    ids = out_dir / "queries.jsonl"
    with ids.open("w", encoding="utf-8") as handle:
        for index in range(count):
            handle.write(json.dumps({"id": f"Q{index:03d}"}, separators=(",", ":")) + "\n")
    manifest = {
        "dataset": "synth-large",
        "generator": "tools/gen_synth.py",
        "law": "standard normal, normalised to the unit sphere (as the passages of synth-large)",
        "seed": seed,
        "stream": STREAM,
        "queries": count,
        "dims": int(gen_synth.DIMS),
        "layout": "row-major little-endian f32, one row per query",
        "matrix": matrix.name,
        "sha256": digest,
        "distinct": True,
    }
    (out_dir / "queries.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )
    return manifest


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    parser.add_argument("--seed", type=int, default=gen_synth.SEED)
    parser.add_argument("--queries", type=int, default=QUERIES)
    arguments = parser.parse_args(argv)
    manifest = write(arguments.out, arguments.seed, arguments.queries)
    print(json.dumps(manifest, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
