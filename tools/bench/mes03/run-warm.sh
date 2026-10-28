#!/bin/sh
# Warm profile of the rescoring at N = 1000 and N = 5000 (MES-03,
# `REDUIRE-COUT-DES-CLES`, ADR-0079 decision 1, in the conditions of the warm
# series of the 2026-10-27 campaign, ADR-0077).
#
# The cold profile of `run.sh` measured a machine whose page cache held nothing:
# its p50 was 5.3 and 12.1 times the campaign's. This script measures the state
# the campaign itself measured, the one that decides option (a): untimed
# warm-up passes over the same queries, then `/usr/bin/sample` on the timed loop.
#
# Three series, each announced by `--ready-file` once the warm-up passes are
# over, so the sampler never sees the warm-up:
#
#   warm-1000      300 queries, the campaign's query set, N = 1000
#   warm-5000      300 queries, the campaign's query set, N = 5000
#   warm-1000-q40  40 queries, the largest set whose pages stay resident here
#
# The working set is the number of distinct candidates of the series: each one
# touches one 16 KiB page of the vector file and one page of `records.sqlite`.
# `order-*.json` records that count (`distinct_iids`), and the report derives the
# footprint from it. The script runs in the foreground from beginning to end and
# leaves no process behind.
#
# Usage: sh tools/bench/mes03/run-warm.sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$ROOT"

HARNESS=target/release/examples/rescore_profile
DATA=data/mes02
INDEX=synth-large
QUERIES=data/mes02/queries.f32
OUT=data/mes03
SECONDS_TO_RUN=36
SECONDS_TO_SAMPLE=20

mkdir -p "$OUT"

echo "== disk before"
df -k "$OUT"
echo "== load and memory before"
uptime
vm_stat | sed -n '1,8p'
sysctl vm.swapusage

# The release build carries debug information so that `sample` names the Rust
# frames; no `Cargo.toml` is changed (the variable overrides the profile).
echo "== build with symbols"
CARGO_PROFILE_RELEASE_DEBUG=true cargo build --release --example rescore_profile

# Candidate order of the 40-query set, the only reading `run.sh` does not have.
if [ ! -f "$OUT/order-1000-q40.json" ]; then
    echo "== candidate order, N = 1000, 40 queries"
    "$HARNESS" order --data "$DATA" --index "$INDEX" --vectors "$QUERIES" \
        --ns 1000 --queries 40 --out "$OUT/order-1000-q40.json"
fi

# series <name> <warmup passes> <harness arguments...>
series() {
    name=$1
    passes=$2
    shift 2
    READY=$OUT/ready-$name.flag
    echo "== sampling the warm rescoring, $name"
    rm -f "$READY"
    "$HARNESS" run --data "$DATA" --index "$INDEX" --vectors "$QUERIES" \
        --seconds "$SECONDS_TO_RUN" --warmup-passes "$passes" \
        --out "$OUT/profile-run-$name.json" --ready-file "$READY" "$@" &
    PID=$!
    while [ ! -f "$READY" ]; do
        sleep 0.2
    done
    /usr/bin/sample "$PID" "$SECONDS_TO_SAMPLE" -file "$OUT/sample-rescore-$name.txt"
    wait "$PID"
    rm -f "$READY"
    python3 tools/bench/mes03/mes03_profile_analyze.py \
        --sample "$OUT/sample-rescore-$name.txt" \
        --run "$OUT/profile-run-$name.json" \
        --out "$OUT/profile-$name.json"
}

series warm-1000 4 --ns 1000
series warm-5000 2 --ns 5000
series warm-1000-q40 6 --ns 1000 --queries 40

python3 tools/bench/mes03/mes03_profile_artifact.py --dir "$OUT"

echo "== load and memory after"
uptime
vm_stat | sed -n '1,8p'
sysctl vm.swapusage
echo "== disk after"
df -k "$OUT"
