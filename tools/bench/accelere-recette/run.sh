#!/bin/sh
# ACCELERER-RECETTE (livrable a) — ventilate the `validation` step of an attach
# on the Legifacile sample, post by post.
#
# Run from anywhere, on a quiet machine:
#
#   ORT_DYLIB_PATH=... sh tools/bench/accelere-recette/run.sh
#
# What it does, and why each run is separate:
#
# 1. `acceptance_split load`, five times, each in a fresh process: the cold load
#    of the fp32 graph, the one a `crible attach` pays inside `validation`.
# 2. `acceptance_split posts`, five repetitions per batch, in one process per
#    batch: every post of `validation` timed with the production calls (batch
#    open, controls 6 and 7, control 8 pass and token counting, control 9 draw
#    and the 200 re-encodings, the per-document read of `plan_documents`).
# 3. The attach itself is not re-measured here: its `validation` reference comes
#    from the production bench of MES-08, `crates/crible/benches/attach_cost.rs`,
#    run by `tools/bench/mes08/run.sh` with `MES08_ONLY=articles`. That bench
#    reads `AttachDurations` from the real `attach`, so the reference is a
#    production timer, not a replica.
#
# Sets `REPEATS` (5) and `OUT` (data/accelere-recette). No value comes from
# `.env.prod`; no path of the machine is written into the report.
set -eu

ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
DATA="$ROOT/data/accelere-recette"
INDEX="$ROOT/data/mes07/indexes/articles"
MODELS="$ROOT/data/models"
BATCHES="$ROOT/data/mes08/batches/articles"
OUT=${OUT:-"$DATA"}
REPEATS=${REPEATS:-5}

: "${ORT_DYLIB_PATH:?set ORT_DYLIB_PATH to the ONNX Runtime library (tools/README.md)}"
export ORT_DYLIB_PATH

mkdir -p "$OUT"

echo "== build the pilot (release)"
(cd "$ROOT" && cargo build --release -p crible-store --example acceptance_split)

echo "== cold load of the fp32 graph, one process per repetition"
for i in 1 2 3 4 5; do
    (cd "$ROOT" && ./target/release/examples/acceptance_split load \
        --model "$MODELS/mdenseon" \
        --repetitions 1 \
        --out "$OUT/load-rep-$i.json") >/dev/null
done
(cd "$ROOT" && ./target/release/examples/acceptance_split load \
    --model "$MODELS/mdenseon" \
    --repetitions 5 \
    --out "$OUT/load-warm.json") >/dev/null

for batch in daily-articles-1000 daily-articles-3000; do
    echo "== posts of $batch"
    (cd "$ROOT" && ./target/release/examples/acceptance_split posts \
        --index "$INDEX" \
        --models "$MODELS" \
        --batch "$BATCHES/$batch" \
        --repetitions "$REPEATS" \
        --out "$OUT/posts-$batch.json")
done

# One repetition only: this run carries the token length of the drawn sample,
# which is what the padding of a length-sorted batch is computed from.
for batch in daily-articles-1000 daily-articles-3000; do
    echo "== token length of the drawn sample, $batch"
    (cd "$ROOT" && ./target/release/examples/acceptance_split posts \
        --index "$INDEX" \
        --models "$MODELS" \
        --batch "$BATCHES/$batch" \
        --repetitions 1 \
        --out "$OUT/lengths-${batch##*-}.json")
done

# The bracket: replica of the posts, real `attach` on a clone, replica again, in
# one process per repetition. Without it, the drift of the machine dominates the
# comparison (see MES-08, campaign of 2026-10-26 night, "Écarts au protocole").
echo "== bracket of daily-articles-1000 (3 repetitions, 1 replica after each)"
(cd "$ROOT" && ./target/release/examples/acceptance_split both \
    --index "$INDEX" --models "$MODELS" \
    --batch "$BATCHES/daily-articles-1000" \
    --work "$OUT/work" --repetitions 3 --after 1 \
    --out "$OUT/both-1000.json")

echo "== bracket of daily-articles-3000 (2 repetitions, 1 replica after each)"
(cd "$ROOT" && ./target/release/examples/acceptance_split both \
    --index "$INDEX" --models "$MODELS" \
    --batch "$BATCHES/daily-articles-3000" \
    --work "$OUT/work" --repetitions 2 --after 1 \
    --out "$OUT/both-3000.json")

echo "== merge the tables"
(cd "$ROOT/tools" && uv run --no-sync python bench/accelere-recette/merge.py)

echo "== done: $OUT"
