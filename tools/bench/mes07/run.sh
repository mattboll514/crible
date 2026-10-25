#!/bin/sh
# MES-07 — duration of a compaction, and the two per-passage reads it pays.
#
# Run from anywhere, on an idle machine:
#
#   sh tools/bench/mes07/run.sh
#
# Two indexes are measured:
#
# - `data/mes07/indexes/articles`: the Legifacile sample, structure `articles`
#   (50 000 passages, 42 417 documents, real texts and fp16 vectors), plus a
#   deletions batch that tombstones about 10 % of the passages;
# - `data/synth-large/indexes/synth-large`: the 8,6 M-passage latency dataset
#   (`tools/gen_synth.py --large`), one passage per document, fp16 vectors,
#   plus a deletions batch of about 10 % of the documents.
#
# `prepare` builds what is missing and is idempotent: it never rebuilds a base
# index that already exists, and never re-attaches a deletions batch whose
# tombstones are already in the reference state.
#
# The bench is `crates/crible-store/benches/compact_cost.rs` (`harness = false`).
# `cargo bench` runs it in the bench profile (optimisations on) and passes it
# `--bench`, which the bench ignores. Paths are absolute because cargo runs the
# binary from the package root. The bench is compiled once, before the campaign:
# a build inside the measured invocation would land in the RSS of that point and
# would perturb the first series.
#
# Datasets and raw data go to `data/` (gitignored).
set -eu

ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
DATA="$ROOT/data/mes07"
RAW="$DATA/raw"
REPEATS=5
SHA=$(git -C "$ROOT" rev-parse --short HEAD)
DAY=$(date -u +%F)
MACHINE="Apple M3 / 24 GB / macOS 15.7.7"
ARTICLES="$DATA/indexes/articles"
LARGE="$ROOT/data/synth-large/indexes/synth-large"
SAMPLE="$ROOT/data/legifacile-sample"

dead_passages() {
    epoch=$(cat "$1/CURRENT_EPOCH")
    sqlite3 "$1/epochs/$epoch/records.sqlite" \
        "SELECT count(*) FROM passages WHERE dead_since_gen IS NOT NULL;"
}

prepare_articles() {
    if [ ! -d "$ARTICLES" ]; then
        echo "== clone the Legifacile sample index (articles)"
        # The base index is built from `data/legifacile-sample` by the proof of
        # reading of its README (`crible index create` + `crible attach` of
        # `legifacile-sample-articles-v1`). `data/mes13/legifacile-articles` is
        # such an index, reused instead of rebuilt: same bytes, one attach less.
        mkdir -p "$DATA/indexes"
        cp -cR "${BASE_ARTICLES:-$ROOT/data/mes13/legifacile-articles}" "$ARTICLES"
    fi
    if [ ! -d "$DATA/batches/articles-deletions-v1" ]; then
        echo "== write the deletions batch of articles"
        (cd "$ROOT/tools" && uv run --no-sync python bench/mes08/make_batches.py \
            deletions \
            --config "$SAMPLE/configs/articles.json" \
            --model "$ROOT/data/models/mdenseon/model.json" \
            --index-dir "$ARTICLES" \
            --fraction 0.10 \
            --out "$DATA/batches" \
            --batch-id articles-deletions-v1)
    fi
    if [ "$(dead_passages "$ARTICLES")" = "0" ]; then
        echo "== attach the deletions batch of articles"
        "$ROOT/target/release/crible" attach --data "$DATA" \
            --models "$ROOT/data/models" --index articles \
            "$DATA/batches/articles-deletions-v1"
    fi
}

prepare_large() {
    if [ ! -d "$LARGE" ]; then
        echo "== create the synth-large index and attach its 8,6 M passages"
        (cd "$ROOT/tools" && uv run --no-sync python gen_synth.py --large \
            --out "$ROOT/data/synth-large")
        "$ROOT/target/release/crible" index create --data "$ROOT/data/synth-large" \
            --config "$ROOT/data/synth-large/index.json"
        "$ROOT/target/release/crible" attach --data "$ROOT/data/synth-large" \
            --models "$ROOT/data/models" --index synth-large \
            "$ROOT/data/synth-large/large-0001"
    fi
    if [ ! -d "$DATA/batches/synth-large-deletions-v1" ]; then
        echo "== write the deletions batch of synth-large"
        (cd "$ROOT/tools" && uv run --no-sync python bench/mes08/make_batches.py \
            deletions \
            --config "$ROOT/data/synth-large/index.json" \
            --index-dir "$LARGE" \
            --fraction 0.10 \
            --out "$DATA/batches" \
            --batch-id synth-large-deletions-v1)
    fi
    if [ "$(dead_passages "$LARGE")" = "0" ]; then
        echo "== attach the deletions batch of synth-large"
        "$ROOT/target/release/crible" attach --data "$ROOT/data/synth-large" \
            --models "$ROOT/data/models" --index synth-large \
            "$DATA/batches/synth-large-deletions-v1"
    fi
}

measure() {
    label=$1
    index=$2
    echo "== $label"
    ROOT="$ROOT" INDEX="$index" WORK="$DATA/work" REPEATS="$REPEATS" SHA="$SHA" \
        MACHINE="$MACHINE" DAY="$DAY" RAW="$RAW" LABEL="$label" \
        /usr/bin/time -l sh -c '
            cd "$ROOT" && exec cargo bench -p crible-store --bench compact_cost -- measure \
                --index "$INDEX" \
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

echo "== build the CLI and the bench"
(cd "$ROOT" && cargo build --release)
(cd "$ROOT" && cargo bench -p crible-store --bench compact_cost --no-run)

prepare_articles
prepare_large

measure articles "$ARTICLES"
measure synth-large "$LARGE"

printf '%s\n' "$(date -u +%FT%TZ)" >"$RAW/conditions-end.txt"
uptime >>"$RAW/conditions-end.txt"
ps -Ao pcpu,comm -r | head -8 >>"$RAW/conditions-end.txt"

python3 "$ROOT/tools/bench/mes07/merge.py" \
    "$ROOT/docs/mesures/MES-07-duree-compaction.$DAY.json" \
    "$RAW/articles.json" \
    "$RAW/synth-large.json"
