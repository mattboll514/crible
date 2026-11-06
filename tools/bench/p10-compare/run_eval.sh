#!/bin/sh
# Pass B of COMPARER-AVEC-PGVECTOR (P10): Crible side of the comparison.
#
# One index per invocation, four `crible eval` passes at a time, so no command
# runs longer than the campaign limit. The data volume is mounted read-write
# because SQLite refuses to open its WAL files on a read-only mount; `eval`
# itself opens every index read-only (ADR-0116) and leaves no lasting write
# under /data; only SQLite's transient -wal/-shm files appear and disappear.
# The count snapshots before and after prove the data did not move.
#
# Usage:
#   sh tools/bench/p10-compare/run_eval.sh inputs
#   sh tools/bench/p10-compare/run_eval.sh counts before
#   sh tools/bench/p10-compare/run_eval.sh index articles
#   sh tools/bench/p10-compare/run_eval.sh counts after
#   sh tools/bench/p10-compare/run_eval.sh aggregate
#   sh tools/bench/p10-compare/run_eval.sh publish
#
# No command runs in the background, and each `index` call prints one dated
# line before each pass.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
BASE="$ROOT/data/p10-compare/eval"
IMAGE=${IMAGE:-crible:p10}
DATA_VOLUME=${DATA_VOLUME:-crible-p10-data}
MODELS_VOLUME=${MODELS_VOLUME:-crible-p10-models}
DATE=${DATE:-2026-11-06}

log() {
    printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*"
}

require_inputs() {
    if [ ! -d "$BASE/inputs" ]; then
        log "missing $BASE/inputs; run 'inputs' first"
        exit 2
    fi
}

case "${1:-}" in
inputs)
    mkdir -p "$BASE/inputs"
    log "building the 24 input games"
    python3 "$ROOT/tools/bench/p10-compare/build_eval_inputs.py" \
        --reference-dir "$ROOT/docs/legifacile/reference" \
        --vectors "$ROOT/data/p10-compare/query-vectors.json" \
        --output-dir "$BASE/inputs" | tee "$BASE/inputs.sha256"
    ;;

counts)
    label=${2:-}
    if [ "$label" != before ] && [ "$label" != after ]; then
        log "counts needs 'before' or 'after'"
        exit 2
    fi
    mkdir -p "$BASE"
    log "crible inspect, $label"
    docker run --rm -v "$DATA_VOLUME:/data" "$IMAGE" inspect --data /data \
        > "$BASE/counts-$label.txt"
    date -u +%Y-%m-%dT%H:%M:%SZ > "$BASE/counts-$label.at"
    log "counts $label -> $BASE/counts-$label.txt"
    ;;

index)
    index=${2:-}
    if [ -z "$index" ]; then
        log "index needs one of: articles decisions dole jorf sarde titres"
        exit 2
    fi
    require_inputs
    for pass in dense-primaire dense-secondaire hybride fidelite; do
        out="$BASE/runs/$index/$pass"
        mkdir -p "$out"
        if ls "$out"/eval-*.json >/dev/null 2>&1; then
            log "$index/$pass already has a report under $out; refusing to overwrite"
            exit 2
        fi
        case "$pass" in
        dense-primaire | dense-secondaire)
            modes=dense
            threshold=5000
            ;;
        hybride)
            modes=hybrid
            threshold=5000
            ;;
        fidelite)
            modes=dense
            threshold=0
            ;;
        esac
        log "eval $index/$pass (modes=$modes threshold=$threshold)"
        docker run --rm \
            -v "$DATA_VOLUME:/data" \
            -v "$MODELS_VOLUME:/models:ro" \
            -v "$BASE/inputs:/in:ro" \
            -v "$out:/out" \
            "$IMAGE" eval \
            --data /data \
            --index "$index" \
            --queries "/in/$index-$pass.jsonl" \
            --models /models \
            --modes "$modes" \
            --k 10 \
            --repeat 3 \
            --exact-threshold "$threshold" \
            --out /out
    done
    log "index $index done"
    ;;

aggregate)
    python3 "$ROOT/tools/bench/p10-compare/aggregate_eval.py" \
        --reference-dir "$ROOT/docs/legifacile/reference" \
        --replay "$ROOT/data/p10-compare/replay.json" \
        --runs "$BASE/runs" \
        --counts-before "$BASE/counts-before.txt" \
        --counts-after "$BASE/counts-after.txt" \
        --pgvector-counts "$ROOT/data/p10-compare/counts-before.json" \
        --snapshots "$ROOT/data/p10-exchange/snapshots" \
        --output "$BASE/aggregates.json"
    ;;

publish)
    for index in articles decisions dole jorf sarde titres; do
        for pass in dense-primaire dense-secondaire hybride fidelite; do
            json=$(ls "$BASE/runs/$index/$pass"/eval-"$index"-"$pass"-*.json)
            markdown=$(ls "$BASE/runs/$index/$pass"/eval-"$index"-"$pass"-*.md)
            cp -n "$json" "$ROOT/docs/mesures/eval-$index-$pass-$DATE.json"
            cp -n "$markdown" "$ROOT/docs/mesures/eval-$index-$pass-$DATE.md"
        done
    done
    log "24 reports published under docs/mesures"
    ;;

*)
    sed -n '3,18p' "$0"
    exit 2
    ;;
esac
