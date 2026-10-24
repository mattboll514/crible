#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Campaign driver of MES-04 and MES-05.
#
# It is a recipe, not an authority: every command is also spelled out in the
# reports (docs/mesures/MES-04-*.md, docs/mesures/MES-05-*.md). Run it from the
# repository root after `uv sync --group export` in `tools/`.
#
#   sh tools/bench/mes04/run.sh
#
# It never touches `crates/` production code; the only Rust file it builds is
# the example `crates/crible-encode/examples/measure_queries.rs`.

set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
TOOLS="$ROOT/tools"
DATA="$ROOT/data/mes04"
COMMIT=$(git -C "$ROOT" rev-parse HEAD)
DATE=${DATE:-$(date +%F)}

export ORT_DYLIB_PATH=${ORT_DYLIB_PATH:-$(cd "$TOOLS" && uv run python -c \
    'import onnxruntime, pathlib; print(next((pathlib.Path(onnxruntime.__file__).parent / "capi").glob("libonnxruntime*.dylib")))')}
echo "ORT_DYLIB_PATH=$ORT_DYLIB_PATH"

mkdir -p "$DATA"

# Query list and reference vectors.
(cd "$TOOLS" && uv run python bench/mes04/prepare.py)
cargo build --release --example measure_queries -p crible-encode

# MES-04: the encoder itself writes its vectors; PyTorch is the reference.
"$ROOT/target/release/examples/measure_queries" vectors \
    --model "$ROOT/data/models/mdenseon" \
    --queries "$DATA/query-texts.jsonl" \
    --out-dir "$DATA"
(cd "$TOOLS" && uv run python bench/mes04/torch_reference.py)
# The document fidelity distribution that Q-06 leans on, re-measured here.
(cd "$TOOLS" && uv run python bench/mes04/fidelity.py)
(cd "$TOOLS" && uv run python bench/mes04/analyze.py \
    --commit "$COMMIT" --date "$DATE" \
    --machine "MacBook Air M3, 8 cores, 24 GB, macOS 15.7.7, native" \
    --fidelity "$DATA/fidelity.json" \
    --out "$DATA/analysis.json")

# MES-05: 1 000 warm encodings per precision, twice, plus the first call.
for precision in int8 fp32; do
    for run in 1 2; do
        "$ROOT/target/release/examples/measure_queries" latency \
            --model "$ROOT/data/models/mdenseon" \
            --queries "$DATA/query-texts.jsonl" \
            --precision "$precision" --warmup 300 --count 1000 \
            --out "$DATA/latency-$precision-run$run.json"
    done
done
(cd "$TOOLS" && uv run python bench/mes04/latency.py \
    --commit "$COMMIT" --date "$DATE" \
    --machine "MacBook Air M3, 8 cores, 24 GB, macOS 15.7.7, native" \
    --run int8-run1="$DATA/latency-int8-run1.json" \
    --run int8-run2="$DATA/latency-int8-run2.json" \
    --run fp32-run1="$DATA/latency-fp32-run1.json" \
    --run fp32-run2="$DATA/latency-fp32-run2.json" \
    --settings "$DATA/threads.json" \
    --out "$DATA/latency.json")

# Session settings explored outside the Rust code.
(cd "$TOOLS" && uv run python bench/mes04/threads.py)

echo "campaign files under $DATA"
