#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Control of the A/B verification (ADR-0082, decision 2): the two binaries return
# the same candidates on every pool of the campaign, for the same queries.
#
# The claim is made the strong way, as decision 2 allows ("or each is identical
# to a full Hamming sort, on the same pools and queries"):
#
#   - the old binary of `e7d4e08` checks **both** of its traversals against a
#     full sort, the oracle added to the copy of `data/ab-old/src` for this A/B;
#   - the new binary checks its single traversal against the same oracle.
#
# Same pools (same `--sizes`, `--spans`, `--densities`, same seed), same first
# five queries, same N. Five is the number the multi-segment campaign used.
#
#   sh tools/bench/mes02/run-ab-control.sh

set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$ROOT"
DATA="data/mes02"
RAW="$DATA/raw"
OLD="$ROOT/data/ab-old/target/release/examples/dense_latency"
NEW="$ROOT/target/release/examples/dense_latency"
PROTOCOL="--ns 5000 --sizes 1000000,all --spans 1000000,7740000 --queries 5"

densities() {
    case "$1" in
        synth-large-8seg) echo "10,25,40,50,60,75,90,100" ;;
        *) echo "10,25,40,50,60,75,90" ;;
    esac
}

{
    echo "date: $(date -u +%FT%TZ)"
    uptime
    ps -A -o %cpu=,comm= | sort -rn | head -8
} > "$RAW/ab-conditions-control-start.txt"

for copy in synth-large synth-large-8seg synth-large-multi; do
    for side in old new; do
        if [ "$side" = "old" ]; then binary="$OLD"; else binary="$NEW"; fi
        echo "control: $copy $side $(date -u +%FT%TZ)"
        "$binary" agree --data "$DATA" --index "$copy" \
            --vectors "$DATA/queries.f32" $PROTOCOL \
            --densities "$(densities "$copy")" \
            --out "$RAW/ab-agree-$side-$copy.json"
    done
done

{
    echo "date: $(date -u +%FT%TZ)"
    uptime
    ps -A -o %cpu=,comm= | sort -rn | head -8
} > "$RAW/ab-conditions-control-end.txt"

echo "control done"
