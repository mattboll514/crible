#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Pass C of MESURER-SUR-MAC-MINI (P10, ADR-0141 decision 5): MES-02, MES-03 and
# MES-05 in ephemeral containers of the Mac mini, against the restored
# `crible-p10-data` volume. The `dense_latency` and `measure_queries` examples
# come from `crible-p10-harness:dev`, built by `Dockerfile.harness`; production
# code is not touched.
#
# Every `docker run` is `--rm` and foreground. Nothing runs in the background,
# nothing touches Supabase, the Legifacile repository or Docker Desktop.
#
# Usage:
#   sh tools/bench/p10-macmini/run_pass_c.sh scan <articles|decisions> <n>
#   sh tools/bench/p10-macmini/run_pass_c.sh scan-warmup <articles|decisions> <n>
#   sh tools/bench/p10-macmini/run_pass_c.sh rescore-warm <n>
#   sh tools/bench/p10-macmini/run_pass_c.sh rescore-cold <n>
#   sh tools/bench/p10-macmini/run_pass_c.sh rescore-split
#   sh tools/bench/p10-macmini/run_pass_c.sh encode <int8|fp32> <run>
#   sh tools/bench/p10-macmini/run_pass_c.sh direct <remote-token-file>
#
# MES-03 and the split series run on `articles` only: it is the only restored
# index with a single segment. `decisions` has four segments and the harness
# would rescore against the first one only; its rescore is not measured.
#
# The direct latency needs a token file on the Mac mini. Put it outside the
# repository and the Crible data directory, then remove it after the run. No
# query text is written: `direct_latency.py` keeps ids and timings only.

set -eu

HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$HERE/common.sh"

HARNESS=crible-p10-harness:dev
BASE=$SSD_EXCHANGE/p10-macmini
IN=$BASE/pass-c
RAW=$BASE/pass-c/raw
DATE=${DATE:-2026-11-07}

log() {
    printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*"
}

run_harness() {
    remote "docker run --rm -v $DATA_VOLUME:/data -v $IN:/in:ro -v $RAW:/out $HARNESS $*"
}

scan_prefix() {
    case "$1" in
    articles) printf 'scan-articles' ;;
    decisions) printf 'scan' ;;
    *) echo "unknown index $1" >&2; exit 2 ;;
    esac
}

case "${1:-}" in
scan)
    index=${2:?index}
    n=${3:?n}
    prefix=$(scan_prefix "$index")
    log "scan $index N=$n"
    run_harness "scan --data /data --index $index --vectors /in/queries.f32 --ns $n --out /out/$prefix-main-n$n.json"
    run_harness "scan --data /data --index $index --vectors /in/queries.f32 --sizes all --ns $n --out /out/$prefix-all-n$n.json"
    ;;

scan-warmup)
    index=${2:?index}
    n=${3:?n}
    prefix=$(scan_prefix "$index")
    log "warm-up scan $index N=$n"
    run_harness "scan --data /data --index $index --vectors /in/queries.f32 --ns $n --passes 1 --out /out/warmup-$prefix-n$n.json"
    ;;

rescore-warm)
    n=${2:?n}
    log "warm rescore N=$n"
    run_harness "rescore --data /data --index articles --vectors /in/queries.f32 --ns $n --passes 4 --out /out/rescore-warm-articles-n$n.json"
    ;;

rescore-cold)
    n=${2:?n}
    log "cold rescore N=$n"
    run_harness "rescore --data /data --index articles --vectors /in/queries.f32 --ns $n --passes 1 --cold --evict --out /out/rescore-cold-articles-n$n.json"
    ;;

rescore-split)
    log "warm dot and key split"
    run_harness "rescore --data /data --index articles --vectors /in/queries.f32 --ns 1000,2000,5000 --passes 3 --dot-only --key-only --out /out/rescore-warm-articles-dot-keys.json"
    ;;

encode)
    precision=${2:?precision}
    run=${3:?run}
    log "encode $precision run $run"
    remote "docker run --rm -v $MODELS_VOLUME:/models:ro -v $IN:/in:ro -v $RAW:/out --entrypoint measure_queries $HARNESS latency --model /models/mdenseon --queries /in/query-texts.jsonl --precision $precision --warmup 300 --count 1000 --out /out/mes05-encode-$precision-$run.json"
    ;;

direct)
    token=${2:?remote token file}
    log "direct latency, cold query cache"
    $SSH "$PROD_SSH" "export PATH=/usr/local/bin:/usr/bin:/bin; python3 - --base http://127.0.0.1:$PORT --token-file $token --inputs $BASE/inputs --out $RAW/direct-latency.json" <"$HERE/direct_latency.py"
    ;;

*)
    echo "usage: $0 {scan|scan-warmup|rescore-warm|rescore-cold|rescore-split|encode|direct} ..." >&2
    exit 2
    ;;
esac
