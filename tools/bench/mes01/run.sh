#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Campaign driver of MES-01 and MES-12 (fidelity of the binary scan).
#
# It is a recipe, not an authority: every command is also spelled out in the
# reports (docs/mesures/MES-01-*.md, docs/mesures/MES-12-*.md). Run it from the
# repository root, after `uv sync --group export` in `tools/`.
#
#   sh tools/bench/mes01/run.sh
#
# It changes no production code. The only Rust file it builds is the example
# `crates/crible/examples/fidelity.rs`, and the only indexes it writes are the
# measurement copies under `data/mes01/indexes`.

set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
TOOLS="$ROOT/tools"
DATA="$ROOT/data/mes01"
COMMIT=$(git -C "$ROOT" rev-parse HEAD)
DATE=${DATE:-$(date +%F)}
: "${MACHINE:?set MACHINE to the machine you measure on, for example: MACHINE='MacBook Air M3, 8 cores, 24 GB, macOS 15.7.7, native'}"

export ORT_DYLIB_PATH=${ORT_DYLIB_PATH:-$(cd "$TOOLS" && uv run python -c \
    'import onnxruntime, pathlib; print(next((pathlib.Path(onnxruntime.__file__).parent / "capi").glob("libonnxruntime*.dylib")))')}
echo "ORT_DYLIB_PATH=$ORT_DYLIB_PATH"

mkdir -p "$DATA"

# Query lists, then the engine's own fp32 query vectors (ADR-0052).
(cd "$TOOLS" && uv run --no-sync python bench/mes01/mes01_prepare.py)
cargo build --release
cargo build --release --example measure_queries -p crible-encode
cargo build --release --example fidelity -p crible

"$ROOT/target/release/examples/measure_queries" vectors \
    --model "$ROOT/data/models/mdenseon" \
    --queries "$DATA/query-texts.jsonl" \
    --out-dir "$DATA"

# Measurement indexes, never the sample ones: `search.exact_threshold = 0`.
(cd "$TOOLS" && uv run --no-sync python bench/mes01/mes01_build_indexes.py)

# Engine side: BINARY for every N, EXACT as the reference, both campaigns.
(cd "$TOOLS" && uv run --no-sync python bench/mes01/mes01_campaign.py)

# Determinism and CLI equivalence of the example, kept replayable.
(cd "$TOOLS" && uv run --no-sync python bench/mes01/mes01_controls.py)

# Out-of-engine centering harness: codes, control, variants (a) and (c).
(cd "$TOOLS" && uv run --no-sync python bench/mes01/mes01_centering.py)

# Aggregates and the versioned artefacts.
(cd "$TOOLS" && uv run --no-sync python bench/mes01/mes01_analyze.py \
    --commit "$COMMIT" --date "$DATE" --machine "$MACHINE" --mesures "$ROOT/docs/mesures")

# Tests of the metric and harness code.
(cd "$TOOLS" && uv run --no-sync pytest bench/mes01/tests)

echo "campaign files under $DATA"
