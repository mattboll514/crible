#!/bin/sh
# Campaign of GROUPER-LECTURES-VECTEURS (P4, ADR-0080).
#
# Every stage runs in the foreground: the script launches one harness at a
# time and waits for it, so no measurement is taken while it returns. The
# stages are separate on purpose, so a report can name the exact command of the
# series it publishes.
#
#   sh tools/bench/mes03/run-grouper.sh screen-hot
#   sh tools/bench/mes03/run-grouper.sh screen-cold
#   sh tools/bench/mes03/run-grouper.sh full-hot
#   sh tools/bench/mes03/run-grouper.sh full-cold
#   sh tools/bench/mes03/run-grouper.sh idle
#   sh tools/bench/mes03/run-grouper.sh contend
#   sh tools/bench/mes03/run-grouper.sh rss
#
# The variants are built outside the working tree from `git archive HEAD` and
# the patches of `data/grouper-vecteurs/patches/`, each with its own
# `CARGO_TARGET_DIR`:
#
#   V=$PWD/data/grouper-vecteurs
#   for v in base grouped madv grouped-madv; do
#       git archive HEAD | tar -x -C $V/src/$v
#   done
#   (cd $V/src/grouped && patch -p1 < $V/patches/stage1-grouped.patch)
#   (cd $V/src/madv && patch -p1 < $V/patches/only-madv.patch)
#   (cd $V/src/grouped-madv && patch -p1 < $V/patches/stage2-grouped-madv.patch)
#   for v in base grouped madv grouped-madv; do
#       cp crates/crible/examples/rescore_ab.rs $V/src/$v/crates/crible/examples/
#       (cd $V/src/$v && CARGO_TARGET_DIR=$V/target-$v cargo build --release \
#           --example rescore_ab --example rescore_check --example fidelity \
#           --example dense_latency)
#       mkdir -p $V/bin/$v
#       for b in rescore_ab rescore_check fidelity dense_latency; do
#           cp $V/target-$v/release/examples/$b $V/bin/$v/$b
#       done
#   done
#
# The candidate sets of each N are built once and reused, so the scan that
# builds them is not paid by every invocation of every round:
#
#   for n in 1000 2000 5000; do
#       $V/bin/base/rescore_ab measure --data data/mes02 --index synth-large \
#           --vectors data/mes02/queries.f32 --ns $n --passes 1 \
#           --candidates-cache $V/candidates/candidates-$n.json \
#           --out $V/raw/cache-build-$n.json
#   done
set -eu

ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
cd "$ROOT"
V=data/grouper-vecteurs
MES=tools/bench/mes03
VF=data/mes02/indexes/synth-large/epochs/1/segments/seg-large-0001/vectors.f16
RF=data/mes02/indexes/synth-large/epochs/1/records.sqlite
DISP=$V/raw/displace-sources.txt
BIN=$V/bin
RAW=$V/raw

mkdir -p "$RAW"

round() {
    # round <tag> <round> <order> <ns> <passes> <extra flags…>
    tag=$1
    number=$2
    order=$3
    ns=$4
    passes=$5
    shift 5
    python3 "$MES/ab_round.py" --round "$number" --order "$order" --ns "$ns" \
        --passes "$passes" --tag "$tag" --raw "$RAW" --bin-dir "$BIN" \
        --vectors-file "$VF" --records-file "$RF" "$@"
}

case "${1:-}" in
screen-hot)
    round screen 1 base,grouped,madv,grouped-madv 1000,5000 3
    round screen 2 grouped-madv,madv,grouped,base 1000,5000 3
    ;;
screen-cold)
    round screencold 1 base,grouped,madv,grouped-madv 1000,5000 1 \
        --cold --displace-gb 19.5 --displace-sources "$DISP"
    round screencold 2 grouped-madv,madv,grouped,base 1000,5000 1 \
        --cold --displace-gb 19.5 --displace-sources "$DISP"
    ;;
full-hot)
    round full 1 base,grouped 1000,2000,5000 3
    round full 2 grouped,base 1000,2000,5000 3
    ;;
full-cold)
    round fullcold 1 base,grouped 1000,2000,5000 1 \
        --cold --displace-gb 19.5 --displace-sources "$DISP"
    round fullcold 2 grouped,base 1000,2000,5000 1 \
        --cold --displace-gb 19.5 --displace-sources "$DISP"
    ;;
idle)
    round idle 1 base,grouped 1000,5000 1 --cold --idle-seconds 600
    round idle 2 grouped,base 1000,5000 1 --cold --idle-seconds 600
    ;;
contend)
    round contend 1 base,grouped 1000,5000 3 --mode contend --queries 30
    round contend 2 grouped,base 1000,5000 3 --mode contend --queries 30
    ;;
rss)
    for variant in base grouped; do
        "$BIN/$variant/dense_latency" rss --data data/mes02 --index synth-large \
            --vectors data/mes02/queries.f32 --passes 1 \
            --out "$RAW/rss-$variant.json"
    done
    # The transient buffer of the grouped read, sampled request by request. The
    # candidate sets are computed here (30 queries, no cache): the cache of an
    # `N` holds the 300 sets of the protocol, and a truncated series would not
    # match it.
    for variant in base grouped; do
        "$BIN/$variant/rescore_ab" measure --data data/mes02 --index synth-large \
            --vectors data/mes02/queries.f32 --ns 5000 --passes 1 --queries 30 \
            --sample-rss --out "$RAW/rss-run-$variant-5000.json"
    done
    ;;
*)
    echo "usage: $0 screen-hot|screen-cold|full-hot|full-cold|idle|contend|rss" >&2
    exit 2
    ;;
esac
