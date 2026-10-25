#!/bin/sh
# MES-08 — duration of a daily-type attach (1 000 to 3 000 passages), stage by
# stage, on two indexes.
#
# Run from anywhere, on an idle machine:
#
#   sh tools/bench/mes08/run.sh
#
# - `data/mes07/indexes/articles`: the Legifacile sample, structure `articles`
#   (50 000 passages, real texts and fp16 vectors, `onnx` model mdenseon). The
#   daily batches carry real texts and vectors; the attach runs the acceptance
#   check of `docs/04`, control 9, as production does.
# - `data/synth-large/indexes/synth-large`: 8,6 M passages (`gen_synth.py
#   --large`, `external` model). The daily batches carry the same real texts as
#   the sample batches, and random normalized vectors, so that the compression
#   cost is the one of a real Legifacile passage; the acceptance check does not
#   run on an `external` model.
#
# The base indexes are the ones of `tools/bench/mes07/run.sh`: run that script
# first (its `prepare` builds them).
#
# The bench is `crates/crible/benches/attach_cost.rs` (`harness = false`). The
# stage durations come from the production report (`AttachDurations`,
# `docs/04`), not from a replica.
#
# `ORT_DYLIB_PATH` must name the ONNX Runtime library of the machine
# (`tools/README.md`), because one of the two indexes uses an `onnx` model.
#
# Datasets and raw data go to `data/` (gitignored).
set -eu

ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
DATA="$ROOT/data/mes08"
MES07="$ROOT/data/mes07"
RAW="$DATA/raw"
REPEATS=5
SHA=$(git -C "$ROOT" rev-parse --short HEAD)
DAY=$(date -u +%F)
MACHINE="Apple M3 / 24 GB / macOS 15.7.7"
ARTICLES="$MES07/indexes/articles"
LARGE="$ROOT/data/synth-large/indexes/synth-large"
SAMPLE="$ROOT/data/legifacile-sample"
BATCHES="$DATA/batches"

: "${ORT_DYLIB_PATH:?set ORT_DYLIB_PATH to the ONNX Runtime library (tools/README.md)}"

prepare_batches() {
    if [ ! -d "$BATCHES/articles/daily-articles-1000" ]; then
        echo "== write the daily batches of the Legifacile sample"
        (cd "$ROOT/tools" && uv run --no-sync python bench/mes08/make_batches.py \
            daily \
            --config "$SAMPLE/configs/articles.json" \
            --model "$ROOT/data/models/mdenseon/model.json" \
            --text-source "$SAMPLE/batches/legifacile-sample-articles-v1" \
            --passages 1000 --out "$BATCHES/articles" \
            --batch-id daily-articles-1000 --seed 20261025 --vectors source)
        (cd "$ROOT/tools" && uv run --no-sync python bench/mes08/make_batches.py \
            daily \
            --config "$SAMPLE/configs/articles.json" \
            --model "$ROOT/data/models/mdenseon/model.json" \
            --text-source "$SAMPLE/batches/legifacile-sample-articles-v1" \
            --passages 3000 --out "$BATCHES/articles" \
            --batch-id daily-articles-3000 --seed 20261025 --vectors source)
    fi
    if [ ! -d "$BATCHES/synth-large/daily-synth-1000" ]; then
        echo "== write the daily batches of synth-large"
        (cd "$ROOT/tools" && uv run --no-sync python bench/mes08/make_batches.py \
            daily \
            --config "$ROOT/data/synth-large/index.json" \
            --text-source "$SAMPLE/batches/legifacile-sample-articles-v1" \
            --passages 1000 --out "$BATCHES/synth-large" \
            --batch-id daily-synth-1000 --seed 20261025 --vectors random)
        (cd "$ROOT/tools" && uv run --no-sync python bench/mes08/make_batches.py \
            daily \
            --config "$ROOT/data/synth-large/index.json" \
            --text-source "$SAMPLE/batches/legifacile-sample-articles-v1" \
            --passages 3000 --out "$BATCHES/synth-large" \
            --batch-id daily-synth-3000 --seed 20261025 --vectors random)
    fi
}

measure() {
    label=$1
    index=$2
    shift 2
    echo "== $label"
    ROOT="$ROOT" INDEX="$index" WORK="$DATA/work" REPEATS="$REPEATS" SHA="$SHA" \
        MACHINE="$MACHINE" DAY="$DAY" RAW="$RAW" LABEL="$label" \
        BATCH1="$1" BATCH2="$2" \
        /usr/bin/time -l sh -c '
            cd "$ROOT" && exec cargo bench -p crible --bench attach_cost -- measure \
                --index "$INDEX" \
                --models "$ROOT/data/models" \
                --batch "$BATCH1" \
                --batch "$BATCH2" \
                --work "$WORK" \
                --repeats "$REPEATS" \
                --commit "$SHA" \
                --machine "$MACHINE" \
                --date "$DAY" \
                --out "$RAW/$LABEL.json"' >/dev/null 2>"$RAW/$label.time"
}

mkdir -p "$RAW" "$DATA/work"
printf '%s\n' "$(date -u +%FT%TZ)" >"$RAW/conditions-start.txt"
uptime >>"$RAW/conditions-start.txt"
ps -Ao pcpu,comm -r | head -8 >>"$RAW/conditions-start.txt"

echo "== build the bench"
(cd "$ROOT" && cargo bench -p crible --bench attach_cost --no-run)

prepare_batches

measure articles "$ARTICLES" \
    "$BATCHES/articles/daily-articles-1000" "$BATCHES/articles/daily-articles-3000"
measure synth-large "$LARGE" \
    "$BATCHES/synth-large/daily-synth-1000" "$BATCHES/synth-large/daily-synth-3000"

printf '%s\n' "$(date -u +%FT%TZ)" >"$RAW/conditions-end.txt"
uptime >>"$RAW/conditions-end.txt"
ps -Ao pcpu,comm -r | head -8 >>"$RAW/conditions-end.txt"

python3 "$ROOT/tools/bench/mes08/merge.py" \
    "$ROOT/docs/mesures/MES-08-duree-attache-quotidienne.$DAY.json" \
    "$RAW/articles.json" \
    "$RAW/synth-large.json"
