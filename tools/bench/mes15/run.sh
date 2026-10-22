#!/bin/sh
# MES-15 — cost of a generation commit as a function of N live passages and
# S segments.
#
# Run from anywhere, on an idle machine (no build, no Spotlight indexing, no
# other load):
#
#   sh tools/bench/mes15/run.sh
#
# The bench is `crates/crible-store/benches/commit_cost.rs` (`harness = false`).
# `cargo bench` runs it in the `bench` profile (release, optimisations on) and
# passes it `--bench`, which the bench ignores. Paths are absolute because cargo
# runs the binary from the package root, not from the repository root.
#
# The bench is compiled once before the campaign starts. `/usr/bin/time -l`
# reports the peak RSS of the whole invocation it wraps, so a build inside the
# first `cargo bench measure` would report rustc's peak as the bench's.
#
# `prepare` refuses a non-empty `--index` target: a rerun needs the `data/mes15/i-*`
# directories deleted, or `--overwrite` added to the `prepare` calls below.
#
# Datasets and raw data go to `data/mes15/` (gitignored). The final file next to
# the report is produced by `merge.py`.
set -eu

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
DATA="$ROOT/data/mes15"
RAW="$DATA/raw"
REPEATS=10
SHA=$(git -C "$ROOT" rev-parse --short HEAD)
DAY=$(date -u +%F)
MACHINE="Apple M3 / 24 GB / macOS 15.7.7"

bench() {
    (cd "$ROOT" && cargo bench -p crible-store --bench commit_cost -- "$@")
}

build() {
    (cd "$ROOT" && cargo bench -p crible-store --bench commit_cost --no-run)
}

prepare() {
    label=$1
    passages=$2
    segments=$3
    shift 3
    bench prepare --index "$DATA/$label" --passages "$passages" --segments "$segments" "$@"
}

measure() {
    label=$1
    out=$2
    /usr/bin/time -l bench measure \
        --index "$DATA/$label" \
        --work "$DATA/work" \
        --repeats "$REPEATS" \
        --commit "$SHA" \
        --machine "$MACHINE" \
        --date "$DAY" \
        --out "$RAW/$out.json" >/dev/null 2>"$RAW/$out.time"
}

mkdir -p "$RAW"
printf '%s\n' "$(date -u +%FT%TZ)" >"$RAW/conditions-start.txt"
uptime >>"$RAW/conditions-start.txt"

build

# Base dataset: N live passages spread over S segments, at generation 1.
# `--vector-files sparse` gives `vectors.f16` its real length (768 dims, fp16)
# without allocating blocks: the control point is that the commit never reads
# those bytes.
prepare i-10000-s10 10000 10
prepare i-100000-s10 100000 10
prepare i-1000000-s10 1000000 10
prepare i-1000000-s10-empty 1000000 10 --vector-files empty
prepare i-1000000-s1000 1000000 1000
prepare i-8580000-s10 8580000 10
prepare i-8580000-s1000 8580000 1000

# Main campaign: 10 repetitions per point, warm cache after the bench warm-up.
for label in i-10000-s10 i-100000-s10 i-1000000-s10 i-1000000-s10-empty \
             i-1000000-s1000 i-8580000-s10 i-8580000-s1000; do
    echo "== $label"
    measure "$label" "$label"
done

# Reproducibility round: the two Legifacile-scale points, replayed in full.
for label in i-8580000-s10 i-8580000-s1000 i-1000000-s10; do
    echo "== round2 $label"
    measure "$label" "round2-$label"
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
    "$RAW/i-8580000-s1000.json" \
    "$RAW/round2-i-8580000-s10.json" \
    "$RAW/round2-i-8580000-s1000.json" \
    "$RAW/round2-i-1000000-s10.json"
