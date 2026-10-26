#!/bin/sh
# MES-13 replay — startup time on the Legifacile sample (schema and passages per
# document of the real corpus), after `CONSTRUIRE-BITMAPS` and
# `REUTILISER-COMPRESSEUR-ZSTD` (ADR-0047, ADR-0048).
#
# Run from anywhere, on an idle machine:
#
#   sh tools/bench/mes13/replay.sh
#
# Differences with `run.sh`, and why:
#
# - the points are the five attached indexes of `data/legifacile-sample`
#   (`data/indexes/<structure>`), copied under `data/mes13/legifacile-<structure>`
#   so the attached indexes stay untouched. The `dole` batch is not attached:
#   it holds 7 628 of the 257 626 passages of the sample (3,0 %), and the report
#   says so.
# - the raw data goes to `data/mes13/replay-raw/`, next to the 2026-10-22 files,
#   which stay untouched.
# - no cache exists: the second half of MES-13 is a rebuild only, and the report
#   says that the comparison with a cache load could not be measured.
set -eu

ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
DATA="$ROOT/data/mes13"
RAW="$DATA/replay-raw"
REPEATS=5
WARMUP=1
SHA=$(git -C "$ROOT" rev-parse HEAD)
DAY=$(date -u +%F)
MACHINE="Apple M3 (8 cores) / 24 GB / macOS 15.7.7"

mkdir -p "$RAW"
printf '%s\n' "$(date -u +%FT%TZ)" >"$RAW/conditions-start.txt"
uptime >>"$RAW/conditions-start.txt"

(cd "$ROOT" && cargo bench -p crible-store --bench startup_cost --no-run)

for structure in articles decisions jorf titres sarde; do
    [ -d "$DATA/legifacile-$structure" ] || \
        cp -cR "$ROOT/data/indexes/$structure" "$DATA/legifacile-$structure"
done

# `/usr/bin/time -l` cannot run a shell function, so each measured invocation is
# a `sh -c` that receives its parameters through the environment.
for structure in articles decisions jorf titres sarde; do
    label="legifacile-$structure"
    echo "== $label"
    ROOT="$ROOT" DATA="$DATA" RAW="$RAW" REPEATS="$REPEATS" WARMUP="$WARMUP" \
        SHA="$SHA" MACHINE="$MACHINE" DAY="$DAY" LABEL="$label" \
        sh -c '
            cd "$ROOT" && exec cargo bench -p crible-store --bench startup_cost -- measure \
                --index "$DATA/$LABEL" \
                --label "$LABEL" \
                --repeats "$REPEATS" \
                --warmup "$WARMUP" \
                --commit "$SHA" \
                --machine "$MACHINE" \
                --date "$DAY" \
                --out "$RAW/$LABEL.json"'
    ROOT="$ROOT" DATA="$DATA" RAW="$RAW" SHA="$SHA" MACHINE="$MACHINE" \
        DAY="$DAY" LABEL="$label" \
        sh -c '
            cd "$ROOT" && exec cargo bench -p crible-store --bench startup_cost -- cold \
                --index "$DATA/$LABEL" \
                --label "$LABEL" \
                --work "$DATA/cold-work" \
                --commit "$SHA" \
                --machine "$MACHINE" \
                --date "$DAY" \
                --out "$RAW/$LABEL-cold.json"'
done

printf '%s\n' "$(date -u +%FT%TZ)" >"$RAW/conditions-end.txt"
uptime >>"$RAW/conditions-end.txt"

python3 "$ROOT/tools/bench/mes13/merge.py" \
    "$ROOT/docs/mesures/MES-13-temps-de-demarrage.$DAY.json" \
    "$RAW/legifacile-articles.json" "$RAW/legifacile-articles-cold.json" \
    "$RAW/legifacile-decisions.json" "$RAW/legifacile-decisions-cold.json" \
    "$RAW/legifacile-jorf.json" "$RAW/legifacile-jorf-cold.json" \
    "$RAW/legifacile-titres.json" "$RAW/legifacile-titres-cold.json" \
    "$RAW/legifacile-sarde.json" "$RAW/legifacile-sarde-cold.json"
