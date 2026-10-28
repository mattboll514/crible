#!/bin/sh
# Sampling profile of the rescoring at N = 1000 and N = 5000 (MES-03,
# `REDUIRE-COUT-DES-CLES`, ADR-0079 decision 1).
#
# The harness runs the engine's own `ExactSearcher::rescore` in a loop and
# announces its first iteration by creating `--ready-file`; `/usr/bin/sample`
# then attaches to that process and samples it for 20 seconds. The script waits
# for both, so it runs in the foreground from beginning to end and leaves no
# process behind.
#
# Usage: sh tools/bench/mes03/run.sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$ROOT"

HARNESS=target/release/examples/rescore_profile
DATA=data/mes02
INDEX=synth-large
QUERIES=data/mes02/queries.f32
OUT=data/mes03
READY=$OUT/ready.flag
SECONDS_TO_RUN=30
SECONDS_TO_SAMPLE=20

mkdir -p "$OUT"

# ADR-0077, decision 2: the drive must hold the data it writes. This campaign
# copies nothing, so the 15 GiB are a margin, not a requirement of the copies.
echo "== disk before"
df -k "$OUT"
echo "== load and memory before"
uptime
vm_stat | sed -n '1,8p'

# The release build carries debug information so that `sample` names the Rust
# frames; no `Cargo.toml` is changed (the variable overrides the profile).
echo "== build with symbols"
CARGO_PROFILE_RELEASE_DEBUG=true cargo build --release --example rescore_profile

for N in 1000 5000; do
    echo "== candidate order, N = $N"
    "$HARNESS" order --data "$DATA" --index "$INDEX" --vectors "$QUERIES" \
        --ns "$N" --out "$OUT/order-$N.json"
done

for N in 1000 5000; do
    echo "== sampling the rescoring, N = $N"
    rm -f "$READY"
    "$HARNESS" run --data "$DATA" --index "$INDEX" --vectors "$QUERIES" \
        --ns "$N" --seconds "$SECONDS_TO_RUN" --out "$OUT/profile-run-$N.json" \
        --ready-file "$READY" &
    PID=$!
    # Opening the searcher checksums the 12.3 GiB vector file: wait for the
    # first iteration, otherwise the sampler would only see the opening.
    while [ ! -f "$READY" ]; do
        sleep 0.2
    done
    /usr/bin/sample "$PID" "$SECONDS_TO_SAMPLE" -file "$OUT/sample-rescore-$N.txt"
    wait "$PID"
    rm -f "$READY"
    python3 tools/bench/mes03/mes03_profile_analyze.py \
        --sample "$OUT/sample-rescore-$N.txt" \
        --run "$OUT/profile-run-$N.json" \
        --out "$OUT/profile-$N.json"
done

# Versioned artifact: the profiles, the candidate-order readings and the
# parameters, so the report can be read without the 30 MiB of raw text reports.
# The warm series of `run-warm.sh` enter the same artifact.
python3 tools/bench/mes03/mes03_profile_artifact.py --dir "$OUT"

echo "== load and memory after"
uptime
vm_stat | sed -n '1,8p'
echo "== disk after"
df -k "$OUT"
