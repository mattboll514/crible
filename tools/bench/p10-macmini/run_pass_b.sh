#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Pass B of MESURER-SUR-MAC-MINI (P10, ADR-0141 decisions 1, 3 and 4): the
# constrained latency and the reading latencies, in ephemeral containers on the
# Mac mini, against the restored `crible-p10-data` volume.
#
# Every `docker run` is `--rm` and foreground. Nothing runs in the background,
# nothing touches Supabase, the Legifacile repository or Docker Desktop.
#
# Usage:
#   sh tools/bench/p10-macmini/run_pass_b.sh host before-<series>
#   sh tools/bench/p10-macmini/run_pass_b.sh push
#   sh tools/bench/p10-macmini/run_pass_b.sh stop
#   sh tools/bench/p10-macmini/run_pass_b.sh series <index> <passe> [cold]
#   sh tools/bench/p10-macmini/run_pass_b.sh top10 remote <index>
#   sh tools/bench/p10-macmini/run_pass_b.sh top10 local <index>
#   sh tools/bench/p10-macmini/run_pass_b.sh pull
#   sh tools/bench/p10-macmini/run_pass_b.sh start
#
# `passe` is one of dense-texte, dense-vecteur, hybride, dense-texte-froid,
# mes00-1000-titres. The input games live under the SSD exchange folder, never
# under `/data` and never in the Legifacile repository.

set -eu

HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$HERE/common.sh"

BASE=$SSD_EXCHANGE/p10-macmini
LOCAL=$OUT
DATE=${DATE:-2026-11-07}

log() {
    printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*"
}

host_report() {
    label=$1
    mkdir -p "$LOCAL/host"
    {
        remote "date -u"
        remote "sysctl vm.swapusage"
        remote "memory_pressure | tail -2"
        remote "vm_stat | head -5"
        remote "docker info --format '{{.MemTotal}} {{.NCPU}}'"
        remote "docker ps --format '{{.Names}} {{.Status}}'"
    } >"$LOCAL/host/$label.txt"
    log "host $label -> $LOCAL/host/$label.txt"
    grep -E 'swapusage|memory free|crible-p10|supabase|MemTotal' "$LOCAL/host/$label.txt" || true
}

check_supabase() {
    running=$(remote "docker ps --filter name=supabase_ --format '{{.Names}}'")
    if [ -n "$running" ]; then
        log "SUPABASE RUNNING: $running"
        return 1
    fi
    log "supabase stopped"
}

series() {
    index=$1
    pass=$2
    cold=${3:-no}
    out="runs/$index/$pass"
    remote "mkdir -p $BASE/$out"
    case "$pass" in
    hybride) modes=hybrid ;;
    *) modes=dense ;;
    esac
    if [ "$cold" = cold ]; then
        log "purge of the VM page cache, $index/$pass"
        remote "docker run --rm --privileged --user 0 --entrypoint sh $IMAGE -c 'sync; echo 3 > /proc/sys/vm/drop_caches'"
    fi
    log "eval $index/$pass start"
    remote "docker run --rm -v $DATA_VOLUME:/data -v $MODELS_VOLUME:/models:ro -v $BASE/inputs:/in:ro -v $BASE/$out:/out $IMAGE eval --data /data --index $index --queries /in/$index-$pass-macmini.jsonl --models /models --modes $modes --k 10 --repeat 3 --exact-threshold 5000 --out /out"
    log "eval $index/$pass done"
}

top10() {
    where=$1
    index=$2
    if [ "$where" = remote ]; then
        remote "mkdir -p $BASE/top10/$index"
        log "top10 $index on the Mac mini"
        remote "docker run --rm --entrypoint sh -v $DATA_VOLUME:/data -v $BASE/inputs/top10:/in:ro -v $BASE/top10.sh:/top10.sh:ro -v $BASE/top10/$index:/out $IMAGE /top10.sh $index"
    else
        mkdir -p "$LOCAL/top10/local/$index"
        log "top10 $index on the development machine"
        docker run --rm --entrypoint sh -v "$DATA_VOLUME:/data" \
            -v "$LOCAL/inputs/top10:/in:ro" -v "$HERE/top10.sh:/top10.sh:ro" \
            -v "$LOCAL/top10/local/$index:/out" "$IMAGE" /top10.sh "$index"
    fi
}

case "${1:-}" in
host)
    host_report "${2:?label}"
    ;;

push)
    remote "mkdir -p $BASE/inputs $BASE/runs $BASE/top10"
    log "pushing the input games"
    rsync -a --partial "$LOCAL/inputs/" "$PROD_SSH:$BASE/inputs/"
    scp -q "$HERE/top10.sh" "$PROD_SSH:$BASE/top10.sh"
    log "inputs pushed"
    ;;

pull)
    mkdir -p "$LOCAL/runs" "$LOCAL/top10/remote"
    log "pulling runs and top-10 dumps"
    rsync -a --partial "$PROD_SSH:$BASE/runs/" "$LOCAL/runs/"
    rsync -a --partial "$PROD_SSH:$BASE/top10/" "$LOCAL/top10/remote/"
    log "runs and dumps pulled"
    ;;

stop)
    log "stopping $CONTAINER"
    remote "docker stop $CONTAINER"
    remote "docker ps -a --filter name=^$CONTAINER\$ --format '{{.Names}} {{.Status}}'"
    ;;

start)
    log "starting $CONTAINER"
    remote "docker start $CONTAINER"
    attempt=0
    while [ "$attempt" -lt 30 ]; do
        if remote "curl -sf -m 3 http://127.0.0.1:$PORT/ready"; then
            printf '\n'
            log "server ready"
            break
        fi
        attempt=$((attempt + 1))
        sleep 5
    done
    remote "docker ps --filter name=^$CONTAINER\$ --format '{{.Names}} {{.Status}}'"
    remote "docker stats --no-stream $CONTAINER"
    ;;

check-supabase)
    check_supabase
    ;;

series)
    check_supabase
    series "$2" "$3" "${4:-no}"
    ;;

top10)
    top10 "$2" "$3"
    ;;

*)
    sed -n '9,28p' "$0"
    exit 2
    ;;
esac
