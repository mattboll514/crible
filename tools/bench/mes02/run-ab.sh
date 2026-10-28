#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# A/B verification of the single sparse traversal (ADR-0082, decisions 2 and 3).
#
# Two binaries, the same command line, the same pools, the same queries:
#
#   - old: `data/ab-old/target/release/examples/dense_latency`, built from the
#     commit `e7d4e08` (the last one with the density rule of ADR-0071) outside
#     the working tree, by `git archive e7d4e08 | tar -x -C data/ab-old/src`;
#   - new: `target/release/examples/dense_latency`, the working tree (ADR-0081
#     single traversal and ADR-0082 segment cursor).
#
# Three pairs per copy, the order of the pairs alternating (old -> new,
# new -> old, old -> new), and the order of the copies rotating in a Latin
# square so the residual cost of the number of segments is read inside a round.
#
# Each pair is one `cross` run per binary: `--pairs 3` inside a run is three
# passes of the measured traversal (ADR-0077), the pair is the binary-to-binary
# comparison. The old binary records both traversals and names the one its
# density rule chose in `traversal_automatic`; the A/B reads that series for the
# old side and `sparse` for the new side.
#
#   sh tools/bench/mes02/run-ab.sh 1
#
# A round is a parameter because one round takes about fifty minutes: a shell
# call per round keeps each step in the foreground, as the measurement rules of
# `docs/mesures/README.md` require.

set -eu

ROUND=${1:?usage: run-ab.sh <round 1|2|3>}
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$ROOT"
DATA="data/mes02"
RAW="$DATA/raw"
OLD="$ROOT/data/ab-old/target/release/examples/dense_latency"
NEW="$ROOT/target/release/examples/dense_latency"
QUERIES="--vectors $DATA/queries.f32"
PROTOCOL="--ns 5000 --pairs 3 --sizes 1000000,all --spans 1000000,7740000"

# Densities of the campaign of the 2026-10-27, copy by copy, so the labels are
# the ones of `MES-02-multisegments.2026-10-27.json`: the compacted copy alone
# carries the density 1.0 points.
densities() {
    case "$1" in
        synth-large-8seg) echo "10,25,40,50,60,75,90,100" ;;
        *) echo "10,25,40,50,60,75,90" ;;
    esac
}

case "$ROUND" in
    1) copies="synth-large synth-large-8seg synth-large-multi"; order="old new" ;;
    2) copies="synth-large-8seg synth-large-multi synth-large"; order="new old" ;;
    3) copies="synth-large-multi synth-large synth-large-8seg"; order="old new" ;;
    *) echo "unknown round '$ROUND'" >&2; exit 2 ;;
esac

for binary in "$OLD" "$NEW"; do
    if [ ! -x "$binary" ]; then
        echo "missing binary '$binary'" >&2
        exit 1
    fi
done

{
    echo "date: $(date -u +%FT%TZ)"
    echo "round: $ROUND"
    echo "copy order: $copies"
    echo "binary order: $order"
    uptime
    ps -A -o %cpu=,comm= | sort -rn | head -8
    df -h "$ROOT" | tail -1
} > "$RAW/ab-conditions-r$ROUND-start.txt"

for copy in $copies; do
    for side in $order; do
        if [ "$side" = "old" ]; then binary="$OLD"; else binary="$NEW"; fi
        echo "round $ROUND: $copy $side $(date -u +%FT%TZ)"
        "$binary" cross --data "$DATA" --index "$copy" $QUERIES $PROTOCOL \
            --densities "$(densities "$copy")" \
            --out "$RAW/ab-$side-$copy-r$ROUND.json"
    done
done

{
    echo "date: $(date -u +%FT%TZ)"
    echo "round: $ROUND"
    uptime
    ps -A -o %cpu=,comm= | sort -rn | head -8
    df -h "$ROOT" | tail -1
} > "$RAW/ab-conditions-r$ROUND-end.txt"

echo "round $ROUND done"
