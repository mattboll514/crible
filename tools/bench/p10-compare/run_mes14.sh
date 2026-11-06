#!/bin/sh
# Pass C of COMPARER-AVEC-PGVECTOR (P10): MES-14 replayed on the real indexes.
#
# ADR-0140, decision 8: `decisions` and `articles`, one interval clause alone
# (full domain) and the same clause behind a bitmap clause (reduced domain).
# Each case is one generated game under data/ (gitignored), run by `crible eval`
# in a fresh container. The volume is mounted read-write because SQLite refuses
# its WAL files on a read-only mount; `eval` opens the index read-only
# (ADR-0116) and writes nothing under /data.
#
# Usage:
#   sh tools/bench/p10-compare/run_mes14.sh games
#   sh tools/bench/p10-compare/run_mes14.sh counts
#   sh tools/bench/p10-compare/run_mes14.sh index decisions
#   sh tools/bench/p10-compare/run_mes14.sh index articles
#   sh tools/bench/p10-compare/run_mes14.sh aggregate
#
# One command per index: no command runs in the background and none is
# expected to last more than a few minutes.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
BASE="$ROOT/data/p10-compare/mes14"
IMAGE=${IMAGE:-crible:p10}
DATA_VOLUME=${DATA_VOLUME:-crible-p10-data}
DATE=${DATE:-2026-11-06}
REPEAT=${REPEAT:-50}
QUERIES=${QUERIES:-20}

log() {
    printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*"
}

case "${1:-}" in
games)
    mkdir -p "$BASE/games"
    log "building the filtered games"
    python3 "$ROOT/tools/bench/p10-compare/mes14.py" games \
        --vectors "$ROOT/data/p10-compare/query-vectors.json" \
        --output-dir "$BASE/games" \
        --count "$QUERIES"
    ;;

counts)
    mkdir -p "$BASE"
    log "counting scan and pool sizes on the generation snapshots"
    python3 "$ROOT/tools/bench/p10-compare/mes14.py" counts \
        --snapshot-dir "$ROOT/data/p10-exchange/snapshots" \
        --output "$BASE/counts.json"
    ;;

index)
    index=${2:-}
    if [ "$index" != decisions ] && [ "$index" != articles ]; then
        log "index needs 'decisions' or 'articles'"
        exit 2
    fi
    for game in "$BASE"/games/mes14-"$index"-*.jsonl; do
        name=$(basename "$game" .jsonl)
        out="$BASE/runs/$index"
        mkdir -p "$out"
        if ls "$out/eval-$name-$DATE.json" >/dev/null 2>&1; then
            log "$name already has a report; refusing to overwrite"
            exit 2
        fi
        log "eval $name (dense, k=10, threshold=5000, repeat=$REPEAT)"
        docker run --rm \
            -v "$DATA_VOLUME:/data" \
            -v "$BASE/games:/in:ro" \
            -v "$out:/out" \
            "$IMAGE" eval \
            --data /data \
            --index "$index" \
            --queries "/in/$name.jsonl" \
            --modes dense \
            --k 10 \
            --repeat "$REPEAT" \
            --exact-threshold 5000 \
            --out /out
    done
    log "index $index done"
    ;;

aggregate)
    mkdir -p "$BASE"
    log "joining the eval reports and the counts"
    python3 "$ROOT/tools/bench/p10-compare/mes14.py" aggregate \
        --runs "$BASE/runs" \
        --counts "$BASE/counts.json" \
        --date "$DATE" \
        --repeat "$REPEAT" \
        --queries "$QUERIES" \
        --output "$ROOT/docs/mesures/comparaison-pgvector-$DATE.mes14.json"
    ;;

*)
    sed -n '3,20p' "$0"
    exit 2
    ;;
esac
