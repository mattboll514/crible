#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Passage A-bis of the task MESURER-EN-CONTENEUR-MAC (P8): instruction of the gap
# between the "engine" series of `dense_latency` and the published replay of
# ADR-0080, measured with `rescore_ab`.
#
# Native only, warm, on the native copy of `synth-large` of
# `data/mes-p8-conteneur/native`, with the 300 queries of ADR-0077
# (`data/mes02/queries.f32`), at N = 1000 and N = 5000: one warm-up pass, then one
# measured pass of 300 queries.
#
# Run it from the repository root, in the foreground, one section at a time:
#
#   sh tools/bench/mes-p8-conteneur/ab-engine.sh head
#   sh tools/bench/mes-p8-conteneur/ab-engine.sh replay
#   sh tools/bench/mes-p8-conteneur/ab-engine.sh limit
#   sh tools/bench/mes-p8-conteneur/ab-engine.sh profile
#   sh tools/bench/mes-p8-conteneur/ab-engine.sh aggregate
#
# Nothing here runs in the background except, in `profile`, the harness itself,
# which `sample` must observe while it runs: that `&` lasts one bounded command
# and the shell waits for the harness before it exits.
#
# `git archive` only carries tracked files, so no `.env.prod` enters the extracted
# tree; the repository is never copied outside the worktree, and no git command of
# this file writes anything (`git archive` and `git rev-parse` read only).

set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$ROOT"

REPLAY=07a82d773db3245e6fc680b48baabe61fbd4ac21
SHORT=07a82d7
AB=data/mes-p8-conteneur/ab
DATA=data/mes-p8-conteneur/native
VECTORS=data/mes02/queries.f32
TARGET=$AB/target-$SHORT

mkdir -p "$AB/raw" "$AB/cache"

# `df -h` guard of the campaign: a build tree needs room.
check_disk() {
    available=$(df -k "$ROOT" | awk 'NR == 2 { print $4 }')
    if [ "$available" -lt 41943040 ]; then
        echo "refusing to build: $(df -h "$ROOT" | awk 'NR == 2 { print $4 }') free, 40 GiB required" >&2
        exit 1
    fi
}

record_machine() {
    {
        echo "date $(date -u +%FT%TZ)"
        echo "commit $(git rev-parse HEAD)"
        uptime
        sysctl -n vm.swapusage
        vm_stat | sed -n '1,4p'
        echo "page size $(getconf PAGESIZE)"
    } > "$AB/raw/machine-$1.txt"
}

case "${1:-}" in
head)
    record_machine head
    cargo build --release --example dense_latency --example rescore_ab
    for n in 1000 5000; do
        echo "== rescore_ab, HEAD, N = $n"
        target/release/examples/rescore_ab measure --data "$DATA" --index synth-large \
            --vectors "$VECTORS" --ns "$n" --passes 2 --queries 300 \
            --candidates-cache "$AB/cache/head-n$n.bin" \
            --out "$AB/raw/head-ab-n$n.json"
        echo "== dense_latency, HEAD, N = $n"
        target/release/examples/dense_latency rescore --data "$DATA" --index synth-large \
            --vectors "$VECTORS" --ns "$n" --passes 2 --dot-only --key-only \
            --out "$AB/raw/head-dense-n$n.json"
    done
    # Residency of the vector pages the candidates touch, without evicting:
    # `--cold` alone reports it with `mincore` before every query.
    echo "== rescore_ab, HEAD, residency of the candidates' pages, N = 1000"
    target/release/examples/rescore_ab measure --data "$DATA" --index synth-large \
        --vectors "$VECTORS" --cold \
        --vectors-file "$DATA/indexes/synth-large/epochs/1/segments/seg-large-0001/vectors.f16" \
        --ns 1000 --passes 1 --queries 300 --candidates-cache "$AB/cache/head-n1000.bin" \
        --out "$AB/raw/head-ab-n1000-residency.json"
    ;;
replay)
    # The replay commit, built outside the tree: `git archive` only carries
    # tracked files. `rescore_ab` is tracked there, so the archive holds the
    # harness of the campaign itself.
    record_machine replay
    rm -rf "$AB/$SHORT"
    mkdir -p "$AB/$SHORT"
    git archive "$REPLAY" | tar -x -C "$AB/$SHORT"
    check_disk
    (cd "$AB/$SHORT" && CARGO_TARGET_DIR="$ROOT/$TARGET" \
        cargo build --release --example dense_latency --example rescore_ab)
    for n in 1000 5000; do
        echo "== rescore_ab, $REPLAY, N = $n"
        "$TARGET/release/examples/rescore_ab" measure --data "$DATA" --index synth-large \
            --vectors "$VECTORS" --ns "$n" --passes 2 --queries 300 \
            --candidates-cache "$AB/cache/$SHORT-n$n.bin" \
            --out "$AB/raw/$SHORT-ab-n$n.json"
        echo "== dense_latency, $REPLAY, N = $n"
        "$TARGET/release/examples/dense_latency" rescore --data "$DATA" --index synth-large \
            --vectors "$VECTORS" --ns "$n" --passes 2 --dot-only --key-only \
            --out "$AB/raw/$SHORT-dense-n$n.json"
    done
    ;;
limit)
    # The decisive control: the same harness, the same commit, at the candidate
    # count production consumes (`k` = `candidates_per_branch`, 100 here) instead
    # of the whole candidate list. Nothing changes but the third argument of
    # `DenseSearcher::rescore`.
    record_machine limit
    for n in 1000 5000; do
        echo "== dense_latency, HEAD, --engine-limit 100, N = $n"
        target/release/examples/dense_latency rescore --data "$DATA" --index synth-large \
            --vectors "$VECTORS" --ns "$n" --passes 2 --engine-limit 100 \
            --out "$AB/raw/head-dense-limit100-n$n.json"
    done
    ;;
profile)
    # macOS `sample` must observe a running process, so the harness is the
    # foreground job of this bounded compound command and the shell waits for it.
    record_machine profile
    sh -c 'target/release/examples/dense_latency rescore \
            --data data/mes-p8-conteneur/native --index synth-large \
            --vectors data/mes02/queries.f32 --ns 1000 --passes 3 \
            --out data/mes-p8-conteneur/ab/raw/profile-engine-n1000.json \
            > data/mes-p8-conteneur/ab/raw/profile-engine-n1000.log 2>&1 &
        pid=$!; sleep 55; sample $pid 10 -file data/mes-p8-conteneur/ab/raw/sample-engine-n1000.txt \
            > /dev/null 2>&1; wait $pid'
    ;;
aggregate)
    (cd tools && uv run --no-sync python bench/mes-p8-conteneur/aggregate-ab.py \
        --raw ../data/mes-p8-conteneur/ab/raw \
        --out ../docs/mesures/MES-03-diagnostic-moteur.2026-11-03.json)
    ;;
*)
    echo "usage: sh tools/bench/mes-p8-conteneur/ab-engine.sh head|replay|limit|profile|aggregate" >&2
    exit 2
    ;;
esac
