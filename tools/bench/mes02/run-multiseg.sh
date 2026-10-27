#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Recipe of the multi-segment campaign of MES-02 (ADR-0081, decisions 2 and 3):
# both traversals of the binary scan, forced on the same pool and the same
# queries, on three measurement copies of `synth-large` that differ only by
# their number of segments.
#
# Since ADR-0081 (task `UNIFIER-PARCOURS-SCAN`) the harness has a single
# traversal: `cross` measures it and `agree` compares it with a full Hamming
# sort. This recipe describes the two-traversal campaign of the 2026-10-27 and
# must run with the binary of commit `e7d4e08`, the last one with the density
# rule; the A/B of ADR-0082 runs the same `cross` and `agree` command lines on
# both binaries and compares their documents. By default that binary is the
# archived one of `data/ab-old` (see `run-ab.sh`); `HARNESS=...` overrides it.
#
# It is a recipe, not an authority: every command is also spelled out in the
# report `docs/mesures/MES-02-parcours-dense-clairseme.md`, section "Campagne
# multi-segments". Run it from the repository root, in `--release`, with no other
# heavy load on the machine.
#
#   MACHINE='MacBook Air M3, 8 cores, 24 GB, macOS 15.7.7, native' \
#     sh tools/bench/mes02/run-multiseg.sh
#
# The three copies live under `data/mes02/indexes/` (git ignored) and are built
# by `build-multiseg-copies.sh`:
#
#   - `synth-large`        one segment, 8.6 M passages, 7.74 M live, density 0.90;
#   - `synth-large-8seg`   compaction of the same index: 8 segments of 1 M
#                          passages, 7.74 M live, no dead passage (density 1.0);
#   - `synth-large-multi`  100 small segments, built by 100 attaches and the same
#                          deletions batch as the reference: same live bitmap as
#                          `synth-large`, iid for iid.
#
# The reference copy is measured twice, at the beginning and at the end of the
# campaign, so the drift of the machine load can be read instead of assumed.

set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$ROOT"
TOOLS="$ROOT/tools"
DATA="data/mes02"
RAW="$DATA/raw"
COMMIT=$(git rev-parse HEAD)
DATE=${DATE:-$(date +%F)}
: "${MACHINE:?set MACHINE to the machine you measure on}"

for index in synth-large synth-large-8seg synth-large-multi; do
    if [ ! -d "$DATA/indexes/$index" ]; then
        echo "missing measurement copy '$index': run tools/bench/mes02/build-multiseg-copies.sh" >&2
        exit 1
    fi
done
mkdir -p "$RAW"

{
    echo "date: $(date -u +%FT%TZ)"
    uptime
    ps -A -o %cpu=,comm= | sort -rn | head -8
} > "$RAW/conditions-multiseg-campaign-start.txt"

# The binary of this campaign is the harness of commit `e7d4e08`, built outside
# the working tree (see `run-ab.sh`): since ADR-0081 the tree has a single
# traversal, and its documents carry no `traversal_automatic` nor
# `pairs[].dense/sparse`, which the analysers of this directory refuse. Override
# it with `HARNESS=/path/to/dense_latency` to measure with another build.
HARNESS=${HARNESS:-$ROOT/data/ab-old/target/release/examples/dense_latency}
if [ ! -x "$HARNESS" ]; then
    echo "missing harness '$HARNESS': build the binary of commit e7d4e08 first:" >&2
    echo "  mkdir -p data/ab-old/src" >&2
    echo "  git archive e7d4e08 | tar -x -C data/ab-old/src" >&2
    echo "  cd data/ab-old/src" >&2
    echo "  CARGO_TARGET_DIR=$ROOT/data/ab-old/target cargo build --release --example dense_latency" >&2
    echo "or set HARNESS to the binary you want to measure with" >&2
    exit 1
fi
QUERIES="--vectors $DATA/queries.f32"
DENSE_RANGE="--ns 5000 --pairs 3 --sizes 1000000,all --spans 1000000,7740000"

# 1. The reference copy (one segment), then the compacted copy (8 segments), then
#    the multi-attach copy (100 segments). Same pool shapes, same span, same
#    queries, N = 5000, three pairs with the order alternating (ADR-0081, 3).
"$HARNESS" cross --data "$DATA" --index synth-large $QUERIES $DENSE_RANGE \
    --densities 10,25,40,50,60,75,90 \
    --out "$RAW/cross-synth-large.json"

# The compacted copy has no dead passage, so density 1.0 is reachable there and
# only there (ADR-0081, decision 3).
"$HARNESS" cross --data "$DATA" --index synth-large-8seg $QUERIES $DENSE_RANGE \
    --densities 10,25,40,50,60,75,90,100 \
    --out "$RAW/cross-synth-large-8seg.json"

"$HARNESS" cross --data "$DATA" --index synth-large-multi $QUERIES $DENSE_RANGE \
    --densities 10,25,40,50,60,75,90 \
    --out "$RAW/cross-synth-large-multi.json"

# 2. The reference copy again, at the end: the drift of the machine over the
#    campaign is the difference between the two runs of the same copy.
"$HARNESS" cross --data "$DATA" --index synth-large $QUERIES $DENSE_RANGE \
    --densities 10,25,40,50,60,75,90 \
    --out "$RAW/cross-synth-large-repeat.json"

# 3. Controls: both traversals must return the same candidates on every pool of
#    every copy.
for index in synth-large synth-large-8seg synth-large-multi; do
    "$HARNESS" agree --data "$DATA" --index "$index" $QUERIES --ns 5000 --queries 5 \
        --sizes 1000000,all --spans 1000000,7740000 --densities 10,25,40,50,60,75,90 \
        --out "$RAW/agree-$index.json"
done

{
    echo "date: $(date -u +%FT%TZ)"
    uptime
    ps -A -o %cpu=,comm= | sort -rn | head -8
} > "$RAW/conditions-multiseg-campaign-end.txt"

# 4. Aggregation, the versioned artefact and its tests.
(cd "$TOOLS" && uv run --no-sync python bench/mes02/mes02_multiseg_analyze.py \
    --commit "$COMMIT" --date "$DATE" --machine "$MACHINE")
(cd "$TOOLS" && uv run --no-sync pytest bench/mes02/tests)

echo "campaign files under $RAW"
