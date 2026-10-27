#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Interleaved series of the multi-segment campaign of MES-02 (ADR-0081).
#
# Since ADR-0081 (task `UNIFIER-PARCOURS-SCAN`) the harness has a single
# traversal: `cross` measures it and `agree` compares it with a full Hamming
# sort. This recipe describes the two-traversal series of the 2026-10-27 and
# must run with the binary of commit `e7d4e08`, the last one with the density
# rule. By default that binary is the archived one of `data/ab-old` (see
# `run-ab.sh`); `HARNESS=...` overrides it.
#
# The protocol of ADR-0081, decision 2, alternates the two traversals *inside* a
# pair. It does not alternate the *copies*: the campaign of the 2026-10-27 runs
# three copies one after the other, and the drift of the machine over that hour
# (load 2.3 -> 4.4, `mediaanalysisd` and `mds_stores` busy) is of the same order
# as the effect being looked for. This series removes that confound: the three
# copies are measured in a Latin square over three rounds, so each copy is
# measured once at each position, and every copy-to-copy gap is read between two
# runs a few minutes apart.
#
# Four points carry the question: the pool of 1 M contiguous passages, the full
# pool, and the density 0.50 on the two spans of the campaign. The traversal
# order inside a pair still alternates (the harness does it), and N = 5000.
#
#   MACHINE='MacBook Air M3, 8 cores, 24 GB, macOS 15.7.7, native' \
#     sh tools/bench/mes02/run-multiseg-interleaved.sh

set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$ROOT"
DATA="data/mes02"
RAW="$DATA/raw"
: "${MACHINE:?set MACHINE to the machine you measure on}"

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
RANGE="--ns 5000 --pairs 3 --sizes 1000000,all --spans 1000000,7740000 --densities 50"

{
    echo "date: $(date -u +%FT%TZ)"
    uptime
    ps -A -o %cpu=,comm= | sort -rn | head -8
} > "$RAW/conditions-multiseg-interleaved-start.txt"

for round in 1 2 3; do
    case "$round" in
        1) order="synth-large synth-large-8seg synth-large-multi" ;;
        2) order="synth-large-8seg synth-large-multi synth-large" ;;
        3) order="synth-large-multi synth-large synth-large-8seg" ;;
    esac
    for index in $order; do
        echo "round $round: $index $(date -u +%FT%TZ)"
        "$HARNESS" cross --data "$DATA" --index "$index" $QUERIES $RANGE \
            --out "$RAW/cross-$index-r$round.json"
    done
done

{
    echo "date: $(date -u +%FT%TZ)"
    uptime
    ps -A -o %cpu=,comm= | sort -rn | head -8
} > "$RAW/conditions-multiseg-interleaved-end.txt"

echo "interleaved series under $RAW"
