#!/bin/sh
# MES-15 replay — same bench, same `N` and `S` points as the 2026-10-21
# campaign (`tools/bench/mes15/run.sh`), after `CONSTRUIRE-BITMAPS` (ADR-0048)
# and `REUTILISER-COMPRESSEUR-ZSTD` (ADR-0048).
#
# Run from anywhere, on an idle machine:
#
#   sh tools/bench/mes15/replay.sh
#
# Differences with `run.sh`, and why:
#
# - `prepare` is not called: the base indexes of `data/mes15/i-*` already exist
#   (6,3 Go, gitignored) and are the ones the 2026-10-21 campaign measured.
#   Re-preparing them would rebuild the same reference state, not replay it.
# - the campaign the replay writes lives under `data/mes15/replay-raw/`, so the
#   `.json` and `.time` files of the 2026-10-21 campaign stay untouched.
# - `data/mes15/i-*/index.json` has been added by hand before the replay: the
#   indexes were prepared by a bench that did not write the index configuration,
#   and `GenerationStore::open` now refuses an index that does not declare its
#   schema (MES-13, § 10). The file declares the schema of the bench, byte for
#   byte the one `bench_config` writes; no row and no segment changes.
set -eu

ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
DATA="$ROOT/data/mes15"
RAW="$DATA/replay-raw"
WORK="$DATA/replay-work"
REPEATS=10
SHA=$(git -C "$ROOT" rev-parse --short HEAD)
DAY=$(date -u +%F)
MACHINE="Apple M3 / 24 GB / macOS 15.7.7"

mkdir -p "$RAW" "$WORK"
printf '%s\n' "$(date -u +%FT%TZ)" >"$RAW/conditions-start.txt"
uptime >>"$RAW/conditions-start.txt"

# compile once: no rustc inside the measured invocation
(cd "$ROOT" && cargo bench -p crible-store --bench commit_cost --no-run)

# `/usr/bin/time -l` cannot run a shell function, so the measured invocation is
# a `sh -c` that receives its parameters through the environment.
measure() {
    label=$1
    ROOT="$ROOT" DATA="$DATA" WORK="$WORK" REPEATS="$REPEATS" SHA="$SHA" \
        MACHINE="$MACHINE" DAY="$DAY" RAW="$RAW" LABEL="$label" \
        /usr/bin/time -l sh -c '
            cd "$ROOT" && exec cargo bench -p crible-store --bench commit_cost -- measure \
                --index "$DATA/$LABEL" \
                --work "$WORK" \
                --repeats "$REPEATS" \
                --commit "$SHA" \
                --machine "$MACHINE" \
                --date "$DAY" \
                --out "$RAW/$LABEL.json"' >/dev/null 2>"$RAW/$label.time"
}

for label in i-10000-s10 i-100000-s10 i-1000000-s10 i-1000000-s10-empty \
             i-1000000-s1000 i-8580000-s10 i-8580000-s1000; do
    echo "== $label"
    measure "$label"
done

printf '%s\n' "$(date -u +%FT%TZ)" >"$RAW/conditions-end.txt"
uptime >>"$RAW/conditions-end.txt"

python3 "$ROOT/tools/bench/mes15/merge.py" \
    "$ROOT/docs/mesures/MES-15-cout-commit-generation.$DAY.json" \
    "$RAW/i-10000-s10.json" \
    "$RAW/i-100000-s10.json" \
    "$RAW/i-1000000-s10.json" \
    "$RAW/i-1000000-s10-empty.json" \
    "$RAW/i-1000000-s1000.json" \
    "$RAW/i-8580000-s10.json" \
    "$RAW/i-8580000-s1000.json"
