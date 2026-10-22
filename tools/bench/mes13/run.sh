#!/bin/sh
# MES-13 — startup time: rebuild of the filter structures from `records.sqlite`.
#
# Run from anywhere, on an idle machine (no build, no Spotlight indexing, no
# other load):
#
#   sh tools/bench/mes13/run.sh
#
# The bench is `crates/crible-store/benches/startup_cost.rs` (`harness = false`).
# `cargo bench` runs it in the `bench` profile (release, optimisations on) and
# passes it `--bench`, which the bench ignores. Paths are absolute because cargo
# runs the binary from the package root, not from the repository root.
#
# Three points:
#
#   synth-small    9 750 live passages, attached from l1+l2+l3 by `crible attach`
#   fixture-1m     1 000 000 live passages, `records.sqlite` written by the
#                  MES-14 bench
#   fixture-8m58   8 580 000 live passages, same fixture, Legifacile scale
#
# The fixtures are measurement only: no segment row, no vector file, and the
# value distributions of `synth-small`. They are reused from `data/mes14` (MES-14
# generates them), so this script adds no dependency and no preparation cost.
# `synth-small` is not generated here: it comes from `tools/gen_synth.py` then
# `crible attach` for l1, l2, l3 (see `docs/jalons/J1-demo.md`), and its dataset
# checksum is recorded in the report.
#
# Every measurement runs on a copy of the index under `data/mes13`, so the
# fixtures of MES-14 stay untouched. `cold` copies the index once per phase and
# times the first access of that copy; macOS does not purge its page cache
# without `root`, so it is an approximation, and the report says so.
set -eu

ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
DATA="$ROOT/data/mes13"
RAW="$DATA/raw"
REPEATS=5
WARMUP=1
SHA=$(git -C "$ROOT" rev-parse HEAD)
DAY=$(date -u +%F)
MACHINE="Apple M3 (8 cores) / 24 GB / macOS 15.7.7"

bench() {
    (cd "$ROOT" && cargo bench -p crible-store --bench startup_cost -- "$@")
}

build() {
    (cd "$ROOT" && cargo bench -p crible-store --bench startup_cost --no-run)
}

copy_index() {
    source=$1
    destination=$2
    [ -d "$destination" ] || cp -cR "$source" "$destination"
}

measure() {
    label=$1
    bench measure \
        --index "$DATA/$label" \
        --label "$label" \
        --repeats "$REPEATS" \
        --warmup "$WARMUP" \
        --commit "$SHA" \
        --machine "$MACHINE" \
        --date "$DAY" \
        --out "$RAW/$label.json"
}

cold() {
    label=$1
    bench cold \
        --index "$DATA/$label" \
        --label "$label" \
        --work "$DATA/cold-work" \
        --commit "$SHA" \
        --machine "$MACHINE" \
        --date "$DAY" \
        --out "$RAW/$label-cold.json"
}

measure_again() {
    label=$1
    bench measure \
        --index "$DATA/$label" \
        --label "$label" \
        --repeats "$REPEATS" \
        --warmup "$WARMUP" \
        --commit "$SHA" \
        --machine "$MACHINE" \
        --date "$DAY" \
        --out "$RAW/round2-$label.json"
}

mkdir -p "$RAW"
printf '%s\n' "$(date -u +%FT%TZ)" >"$RAW/conditions-start.txt"
uptime >>"$RAW/conditions-start.txt"

build

copy_index "$ROOT/data/indexes/synth-small" "$DATA/synth-small"
copy_index "$ROOT/data/mes14/fixture-1m" "$DATA/fixture-1m"
copy_index "$ROOT/data/mes14/fixture-8m58" "$DATA/fixture-8m58"

echo "== synth-small"
measure synth-small
cold synth-small
echo "== fixture-1m"
measure fixture-1m
cold fixture-1m
echo "== fixture-8m58"
measure fixture-8m58
cold fixture-8m58

# Second tour: every point replayed in full, same bench, same day.
echo "== second tour"
for label in synth-small fixture-1m fixture-8m58; do
    measure_again "$label"
done

printf '%s\n' "$(date -u +%FT%TZ)" >"$RAW/conditions-end.txt"
uptime >>"$RAW/conditions-end.txt"

python3 "$ROOT/tools/bench/mes13/merge.py" \
    "$ROOT/docs/mesures/MES-13-temps-de-demarrage.$DAY.json" \
    "$RAW/synth-small.json" "$RAW/synth-small-cold.json" \
    "$RAW/fixture-1m.json" "$RAW/fixture-1m-cold.json" \
    "$RAW/fixture-8m58.json" "$RAW/fixture-8m58-cold.json" \
    "$RAW/round2-synth-small.json" "$RAW/round2-fixture-1m.json" \
    "$RAW/round2-fixture-8m58.json"
