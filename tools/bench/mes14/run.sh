#!/bin/sh
# MES-14 — cost of `Pool::evaluate` for a value-column interval filter, as a
# function of the domain it receives.
#
# Run from anywhere, on an idle machine (no build, no Spotlight indexing, no
# other load):
#
#   sh tools/bench/mes14/run.sh
#
# The bench is `crates/crible-search/benches/pool_filter.rs` (`harness = false`).
# `cargo bench` runs it in the `bench` profile (release, optimisations on) and
# passes it `--bench`, which the bench ignores. Paths are absolute because cargo
# runs the binary from the package root, not from the repository root.
#
# Three points:
#
#   synth-small       9 750 live passages, attached from l1+l2+l3 by `crible attach`
#   fixture-1m        1 000 000 live passages, `records.sqlite` written by the bench
#   fixture-8m58      8 580 000 live passages, same fixture, Legifacile scale
#
# The fixtures are measurement only: no segment row, no vector file, and the
# value distributions of `synth-small`. `Pool::evaluate` reads only the filter
# structures, so what is timed is the production code on a comparable structure.
#
# `prepare` refuses a non-empty `--index` target, so a rerun needs the
# `data/mes14/fixture-*` directories deleted. `synth-small` is not generated
# here: it comes from `tools/gen_synth.py` then `crible attach` for l1, l2, l3,
# and its dataset checksum is recorded in the report.
set -eu

ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
DATA="$ROOT/data/mes14"
RAW="$DATA/raw"
REPEATS=1000
WARMUP=200
SHA=$(git -C "$ROOT" rev-parse --short HEAD)
DAY=$(date -u +%F)
MACHINE="Apple M3 (8 cores) / 24 GB / macOS 15.7.7"

bench() {
    (cd "$ROOT" && cargo bench -p crible-search --bench pool_filter -- "$@")
}

build() {
    (cd "$ROOT" && cargo bench -p crible-search --bench pool_filter --no-run)
}

measure() {
    label=$1
    shift
    bench measure \
        --repeats "$REPEATS" \
        --warmup "$WARMUP" \
        --commit "$SHA" \
        --machine "$MACHINE" \
        --date "$DAY" \
        --out "$RAW" \
        --label "$label" \
        "$@"
}

mkdir -p "$RAW"
printf '%s\n' "$(date -u +%FT%TZ)" >"$RAW/conditions-start.txt"
uptime >>"$RAW/conditions-start.txt"

build

# Fixtures at scale: N live passages, schema and distributions of synth-small.
[ -d "$DATA/fixture-1m" ] || bench prepare --index "$DATA/fixture-1m" --passages 1000000
[ -d "$DATA/fixture-8m58" ] || bench prepare --index "$DATA/fixture-8m58" --passages 8580000

# Main campaign: 1 000 evaluations per case, 200 discarded as warm-up, per point.
echo "== synth-small"
measure synth-small \
    --index "$ROOT/data/indexes/synth-small"
echo "== fixture-1m"
measure fixture-1m \
    --index "$DATA/fixture-1m"
echo "== fixture-8m58"
measure fixture-8m58 \
    --index "$DATA/fixture-8m58"

# Reproducibility round: the three points replayed in full, same day.
echo "== round2 synth-small"
measure round2-synth-small --round 2 \
    --index "$ROOT/data/indexes/synth-small"
echo "== round2 fixture-1m"
measure round2-fixture-1m --round 2 \
    --index "$DATA/fixture-1m"
echo "== round2 fixture-8m58"
measure round2-fixture-8m58 --round 2 \
    --index "$DATA/fixture-8m58"

printf '%s\n' "$(date -u +%FT%TZ)" >"$RAW/conditions-end.txt"
uptime >>"$RAW/conditions-end.txt"

# Checksums: SHA-256 of the `records.sqlite` measured, and the blake3 checksum
# of the `synth-small` dataset it was attached from.
{
    printf '{\n'
    first=1
    for name in synth-small fixture-1m fixture-8m58; do
        if [ "$name" = synth-small ]; then
            index="$ROOT/data/indexes/synth-small"
        else
            index="$DATA/$name"
        fi
        epoch=$(cat "$index/CURRENT_EPOCH")
        records="$index/epochs/$epoch/records.sqlite"
        digest=$(shasum -a 256 "$records" | cut -d' ' -f1)
        bytes=$(stat -f%z "$records")
        [ $first -eq 1 ] || printf ',\n'
        first=0
        printf '  "%s": {"records_sqlite_sha256": "%s", "records_sqlite_bytes": %s}' \
            "$name" "$digest" "$bytes"
    done
    printf '\n}\n'
} >"$RAW/checksum-records.json"

(cd "$ROOT/tools" && uv run python -c '
import gen_synth
print(gen_synth.dataset_checksum("../data/synth-small"))
' >"$RAW/checksum-synth-small.txt")

python3 "$ROOT/tools/bench/mes14/merge.py" \
    "$ROOT/docs/mesures/MES-14-cout-filtres-intervalles.$DAY.json" \
    "$RAW/synth-small.json" \
    "$RAW/fixture-1m.json" \
    "$RAW/fixture-8m58.json" \
    "$RAW/round2-synth-small.json" \
    "$RAW/round2-fixture-1m.json" \
    "$RAW/round2-fixture-8m58.json"
