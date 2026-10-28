#!/bin/sh
# MES-06 — the lexical branch: index size, build time and filtered BM25 latency.
#
# Protocol: ADR-0091. Report: docs/mesures/MES-06-lexical-bm25-filtre.md.
#
# Run from anywhere, on a machine without other heavy load:
#
#   sh tools/bench/mes06/run.sh prepare     # builds the six sample indexes
#   sh tools/bench/mes06/run.sh size        # size and analysed tokens, positions false
#   sh tools/bench/mes06/run.sh probe       # real filters near 1 000 / 10 000 / 50 000
#   sh tools/bench/mes06/run.sh latency     # the real campaign, positions false
#   sh tools/bench/mes06/run.sh rebuild     # build time, positions false then true
#   sh tools/bench/mes06/run.sh synth       # synth-large: clone, rebuild, plumbing latency
#   sh tools/bench/mes06/run.sh cold        # the single cold bound
#   sh tools/bench/mes06/run.sh analyze     # recomputes the percentiles, writes the artefact
#
# The campaign of 2026-10-28 ran these steps in this order. Every path is
# relative to the repository root; no output goes to an absolute path, so the
# raw documents stay comparable between machines.
#
# Data: `data/mes06/` (gitignored, 17 Go at the end of the campaign: six sample
# indexes and a clone of `synth-large`). The harness never writes into
# `data/legifacile-sample/` or `data/synth-large/`; `synth-large` is APFS-cloned
# (`cp -cR`) and the copy alone is edited.
set -eu

ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
DATA="$ROOT/data/mes06"
RAW="$DATA/raw"
SAMPLE="$ROOT/data/legifacile-sample"
QUERIES="$SAMPLE/queries.jsonl"
FILTERS="$ROOT/tools/bench/mes06/filters.json"
FILTERS_SECONDARY="$ROOT/tools/bench/mes06/filters-secondary.json"
FILTERS_SYNTH="$ROOT/tools/bench/mes06/filters-synth.json"
FILTERS_COLD="$ROOT/tools/bench/mes06/filters-cold.json"
FILTERS_WAND_SYNTH="$ROOT/tools/bench/mes06/filters-wand-synth.json"
HARNESS="$ROOT/target/release/examples/lexical_latency"
STRUCTURES="articles decisions jorf sarde dole titres"

cd "$ROOT"

# Every step writes its raw documents here, `prepare` included: a fresh tree
# must not fail on a missing directory before the first measurement.
mkdir -p "$DATA/indexes" "$RAW"

# `lexical.positions` of a measurement index, in place. The configuration of
# `index.json` is edited, then the job applies it: P5 exposes no `PATCH` CLI.
edit_positions() {
    python3 - "$DATA/indexes/$1/index.json" "$2" <<'PYTHON'
import json, sys
path, value = sys.argv[1], sys.argv[2] == "true"
document = json.load(open(path))
document["lexical"]["positions"] = value
json.dump(document, open(path, "w"), indent=2)
open(path, "a").write("\n")
PYTHON
}

# Enables the branch in the clone of `synth-large`, which ships it disabled.
edit_enabled() {
    python3 - "$DATA/indexes/$1/index.json" <<'PYTHON'
import json, sys
path = sys.argv[1]
document = json.load(open(path))
document["lexical"]["enabled"] = True
json.dump(document, open(path, "w"), indent=2)
open(path, "a").write("\n")
PYTHON
}

build() {
    # Release, always: a measurement is never taken on a debug build.
    cargo build --release --example lexical_latency
    cargo build --release --bin crible
}

onnx() {
    # The attach re-encodes a sample for the acceptance control (ADR-0050): the
    # ONNX Runtime wheel of `tools/` carries the shared library.
    ORT_DYLIB_PATH=$(cd "$ROOT/tools" && uv run --no-sync python -c \
        'import onnxruntime, pathlib; print(next((pathlib.Path(onnxruntime.__file__).parent / "capi").glob("libonnxruntime*.dylib")))')
    export ORT_DYLIB_PATH
}

case "${1:-}" in
prepare)
    build
    onnx
    for structure in $STRUCTURES; do
        # `index create` refuses an existing target: a rerun needs the index
        # directory deleted first, which the report says.
        ( time ./target/release/crible index create \
            --data "$DATA" \
            --config "$SAMPLE/configs/$structure.json" ) \
            > "$RAW/attach-$structure.txt" 2>&1
        ( time ./target/release/crible attach \
            --data "$DATA" --models "$ROOT/data/models" \
            --index "$structure" \
            "$SAMPLE/batches/legifacile-sample-$structure-v1" ) \
            >> "$RAW/attach-$structure.txt" 2>&1
    done
    ;;
size)
    build
    for structure in $STRUCTURES; do
        "$HARNESS" tokens --data "$DATA" --index "$structure" \
            --out "$RAW/tokens-$structure-false.json"
        "$HARNESS" size --data "$DATA" --index "$structure" \
            --out "$RAW/size-$structure-false.json"
    done
    ;;
probe)
    for structure in $STRUCTURES; do
        "$HARNESS" probe --data "$DATA" --index "$structure" \
            --targets 1000,10000,50000 --keep 40 \
            --out "$RAW/probe-$structure.json"
    done
    # `tools/bench/mes06/filters.json` and `filters-secondary.json` are frozen
    # from these documents: one filter per target, the nearest achievable pool.
    ;;
latency)
    for structure in $STRUCTURES; do
        "$HARNESS" latency --data "$DATA" --index "$structure" \
            --queries "$QUERIES" --pools "$FILTERS" \
            --ks 10,100 --passes 3 \
            --out "$RAW/latency-$structure-false.json"
    done
    ;;
rebuild)
    # `false` first, then `true` on the same data: the boolean is edited in the
    # measurement copy of `index.json` (P5 has no `PATCH` CLI) and the job
    # `rebuild-lexical` applies it. The `false` state is restored at the end,
    # and its byte count is identical to the one measured before, which is the
    # reproducibility check of the rebuild.
    for structure in $STRUCTURES; do
        "$HARNESS" rebuild --data "$DATA" --index "$structure" \
            --out "$RAW/rebuild-$structure-false.json"
    done
    for structure in $STRUCTURES; do
        edit_positions "$structure" true
        "$HARNESS" rebuild --data "$DATA" --index "$structure" \
            --out "$RAW/rebuild-$structure-true.json"
        "$HARNESS" size --data "$DATA" --index "$structure" \
            --out "$RAW/size-$structure-true.json"
        "$HARNESS" latency --data "$DATA" --index "$structure" \
            --queries "$QUERIES" --pools "$FILTERS_SECONDARY" \
            --ks 10,100 --passes 3 \
            --out "$RAW/latency-$structure-true.json"
    done
    for structure in $STRUCTURES; do
        edit_positions "$structure" false
        "$HARNESS" rebuild --data "$DATA" --index "$structure" \
            --out "$RAW/rebuild-$structure-restored-false.json"
    done
    ;;
synth)
    build
    # APFS clone: instant, and the original stays untouched (`docs/08:55`
    # reserves `synth-large` to latency).
    cp -cR "$ROOT/data/synth-large/indexes/synth-large" "$DATA/indexes/synth-large"
    edit_enabled synth-large
    "$HARNESS" rebuild --data "$DATA" --index synth-large \
        --out "$RAW/rebuild-synth-large-false.json"
    "$HARNESS" size --data "$DATA" --index synth-large \
        --out "$RAW/size-synth-large-false.json"
    "$HARNESS" probe --data "$DATA" --index synth-large \
        --targets 537000,1000000,2150000,7740000 --keep 4 \
        --out "$RAW/probe-synth-large.json"
    "$HARNESS" latency --data "$DATA" --index synth-large \
        --queries "$QUERIES" --pools "$FILTERS_SYNTH" \
        --ks 10,100 --passes 3 --synthetic \
        --out "$RAW/latency-synth-large-false.json"
    for structure in $STRUCTURES; do
        "$HARNESS" wand --data "$DATA" --index "$structure" \
            --queries "$QUERIES" --pools "$FILTERS" --limit 20 --k 10 \
            --out "$RAW/wand-$structure.json"
    done
    "$HARNESS" wand --data "$DATA" --index synth-large \
        --queries "$QUERIES" --pools "$FILTERS_WAND_SYNTH" --limit 3 --k 10 \
        --synthetic --out "$RAW/wand-synth-large.json"
    ;;
cold)
    # `copy-nocache` prints the copied byte count on stdout: the figure of the
    # report comes from this file, which the campaign keeps.
    "$HARNESS" copy-nocache --src "$DATA/indexes/synth-large" \
        --dst "$DATA/indexes/synth-large-cold" > "$RAW/copy-nocache.txt"
    "$HARNESS" latency --data "$DATA" --index synth-large-cold \
        --queries "$QUERIES" --pools "$FILTERS_COLD" \
        --ks 10 --passes 1 --no-warmup --synthetic \
        --out "$RAW/latency-synth-large-cold.json"
    rm -rf "$DATA/indexes/synth-large-cold"
    ;;
analyze)
    ( cd "$ROOT/tools" && uv run --no-sync python bench/mes06/mes06_analyze.py \
        --commit "$(git rev-parse HEAD)" --date 2026-10-28 \
        --machine "MacBook Air M3, 8 cores, 24 GB, macOS 15.7.7, native" \
        --write )
    ;;
*)
    echo "usage: $0 {prepare|size|probe|latency|rebuild|synth|cold|analyze}" >&2
    exit 2
    ;;
esac
