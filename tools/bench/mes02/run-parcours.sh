#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Recipe of the alternating campaign of MES-02 (ADR-0071, ADR-0077): both
# traversals of the binary scan, forced on the same pool and the same queries,
# dense then sparse, pair after pair.
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
# report `docs/mesures/MES-02-parcours-dense-clairseme.md`. Run it from the
# repository root, in `--release`, with no other heavy load on the machine.
#
#   MACHINE='MacBook Air M3, 8 cores, 24 GB, macOS 15.7.7, native' \
#     sh tools/bench/mes02/run-parcours.sh
#
# It reuses the measurement copy of `data/mes02/` (17 GB, gitignored) that
# `tools/bench/mes02/run.sh` built, with `search.exact_threshold = 0`; it never
# writes to `data/synth-large/`. The harness is
# `crates/crible/examples/dense_latency.rs`, modes `cross` and `agree` (the
# control that both traversals return the same candidates on every pool), which
# change nothing in `crates/*/src/`.

set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$ROOT"
TOOLS="$ROOT/tools"
# Relative paths on purpose: the harness writes them into the raw documents and
# the analyser copies them into the versioned artefact, which must not carry a
# local absolute path.
DATA="data/mes02"
RAW="$DATA/raw"
COMMIT=$(git rev-parse HEAD)
DATE=${DATE:-$(date +%F)}
: "${MACHINE:?set MACHINE to the machine you measure on}"

if [ ! -d "$DATA/indexes/synth-large" ]; then
    echo "missing measurement copy: run tools/bench/mes02/run.sh first" >&2
    exit 1
fi
mkdir -p "$RAW"

# Load of the machine at both ends of the campaign: the report needs it, since
# a busy workstation is not a dedicated machine.
{
    echo "date: $(date -u +%FT%TZ)"
    uptime
    ps -A -o %cpu=,comm= | sort -rn | head -8
} > "$RAW/conditions-parcours-start.txt"

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
INDEX="--data $DATA --index synth-large --vectors $DATA/queries.f32"

# 1. N = 5000 (ADR-0078): the density band of ADR-0071, plus the two pools of
#    MES-02 (1 M contiguous and every live passage), three pairs after a
#    separate warm-up pass of each traversal.
"$HARNESS" cross $INDEX --ns 5000 --pairs 3 \
    --sizes 1000000,all \
    --densities 10,25,40,50,60,75,90 --spans 1000000,7740000 \
    --out "$RAW/cross-n5000.json"

# 2. N = 1000, on the two pools of MES-02 only: continuity with the forced
#    series of the first campaign of MES-02.
"$HARNESS" cross $INDEX --ns 1000 --pairs 3 \
    --sizes 1000000,all --densities "" --spans "" \
    --out "$RAW/cross-n1000.json"

# 3. Control: both traversals must return the same candidates on every pool.
"$HARNESS" agree $INDEX --ns 5000 --queries 5 --sizes 1000000,all \
    --densities 10,25,40,50,60,75,90 --spans 1000000,7740000 \
    --out "$RAW/agree-n5000.json"
"$HARNESS" agree $INDEX --ns 1000 --queries 5 --sizes 1000000,all \
    --densities "" --spans "" \
    --out "$RAW/agree-n1000.json"

{
    echo "date: $(date -u +%FT%TZ)"
    uptime
    ps -A -o %cpu=,comm= | sort -rn | head -8
} > "$RAW/conditions-parcours-end.txt"

# 4. Aggregation, the versioned artefact and its tests.
(cd "$TOOLS" && uv run --no-sync python bench/mes02/mes02_parcours_analyze.py \
    --commit "$COMMIT" --date "$DATE" --machine "$MACHINE" --mesures "$ROOT/docs/mesures")
(cd "$TOOLS" && uv run --no-sync pytest bench/mes02/tests)

echo "campaign files under $RAW"
