#!/bin/sh
# MES-10 and MES-11 — quality of the fusion, French analysis (elisions,
# identifiers).
#
# Protocol: ADR-0092. Reports: docs/mesures/MES-10-qualite-fusion.md and
# docs/mesures/MES-11-analyse-francaise.md.
#
# Run from anywhere, on a machine without other heavy load:
#
#   sh tools/bench/mes10/run.sh extract     # read-only: adds the 55 missing expected documents
#   sh tools/bench/mes10/run.sh prepare     # copies the sample, builds six indexes and six EXACT clones
#   sh tools/bench/mes10/run.sh encode      # encodes the 309 texts in fp32 and archives the vectors
#   sh tools/bench/mes10/run.sh eval        # the six binary campaigns, then the six EXACT ones
#   sh tools/bench/mes10/run.sh analyze     # MES-11, six indexes, four variants
#   sh tools/bench/mes10/run.sh aggregate   # recomputes every figure, writes the two artefacts
#
# The campaign of 2026-10-28 ran these steps in this order. Every path is
# relative to the repository root; no output goes to an absolute path, so the
# raw documents stay comparable between machines.
#
# Data: `data/mes10/` (gitignored, about 1.9 Go at the end of the campaign: the
# ten batch copies plus six indexes and six EXACT clones). The harness never
# writes into `data/legifacile-sample/`: the batches are APFS-cloned (`cp -cR`)
# and only the copies are edited. The sample configurations carry N = 1000 and
# `exact_threshold` = 1000, where ADR-0078 fixes 5000; the measurement copies
# are set to 5000 at creation, which is the double `PATCH` of ADR-0092,
# decision 3, applied to copies. P5 exposes no CLI or HTTP entry point to patch
# the search configuration (`crible patch` only updates document fields, and
# `IndexPatch`/`apply_patch` are only reachable from code); the copies therefore
# carry 5000/5000 from creation, and their resulting configuration is the same
# as after a double `PATCH`. The precedent of MES-06 `edit_positions` is
# followed.
#
# `extract` reads `.env.prod` in read-only mode. It never prints a host, a path
# or a user: only the names of the variables are used, and the extracted
# documents land in `data/mes10/batches/`, which is gitignored.
set -eu

ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
DATA="$ROOT/data/mes10"
RAW="$DATA/raw"
SAMPLE="$ROOT/data/legifacile-sample"
QUERIES="$SAMPLE/queries.jsonl"
VECTORS="$DATA/query-vectors-2026-10-28.json"
MODELS="$ROOT/data/models"
HARNESS="$ROOT/target/release/examples/hybrid_eval"
STRUCTURES="articles decisions jorf sarde dole titres"
CASE_1240="LEGIARTI000032041571"

cd "$ROOT"

mkdir -p "$DATA/configs" "$DATA/batches" "$DATA/indexes" "$RAW"

# A search key of `index.json` or of a measurement configuration, in place.
edit_search() {
    python3 - "$1" "$2" "$3" <<'PYTHON'
import json, sys
path, key, value = sys.argv[1], sys.argv[2], json.loads(sys.argv[3])
document = json.load(open(path))
document["search"][key] = value
json.dump(document, open(path, "w"), indent=2)
open(path, "a").write("\n")
PYTHON
}

build() {
    # Release, always: a measurement is never taken on a debug build.
    cargo build --release --example hybrid_eval
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
extract)
    ( cd "$ROOT/tools" && uv run --no-sync python bench/mes10/extract_expected.py )
    ;;
prepare)
    build
    onnx
    for structure in $STRUCTURES; do
        # APFS clone of the sample batch: instant, and the original stays
        # untouched. The sample configurations are copied then patched, never
        # edited in place.
        cp -cR "$SAMPLE/batches/legifacile-sample-$structure-v1" \
            "$DATA/batches/legifacile-sample-$structure-v1"
        cp "$SAMPLE/configs/$structure.json" "$DATA/configs/$structure.json"
        edit_search "$DATA/configs/$structure.json" rescore_candidates 5000
        edit_search "$DATA/configs/$structure.json" exact_threshold 5000
        # `index create` refuses an existing target: a rerun needs the index
        # directory deleted first, which the report says.
        ./target/release/crible index create \
            --data "$DATA" --config "$DATA/configs/$structure.json" \
            >> "$RAW/attach-$structure.txt" 2>&1
        ./target/release/crible attach \
            --data "$DATA" --models "$MODELS" \
            --index "$structure" \
            "$SAMPLE/batches/legifacile-sample-$structure-v1" \
            >> "$RAW/attach-$structure.txt" 2>&1
        if [ -d "$DATA/batches/legifacile-mes10-$structure-v1" ]; then
            ./target/release/crible attach \
                --data "$DATA" --models "$MODELS" \
                --index "$structure" \
                "$DATA/batches/legifacile-mes10-$structure-v1" \
                >> "$RAW/attach-$structure.txt" 2>&1
        fi
        # EXACT column: a clone of the same index, threshold at its maximum.
        cp -cR "$DATA/indexes/$structure" "$DATA/indexes/$structure-exact"
        edit_search "$DATA/indexes/$structure-exact/index.json" \
            exact_threshold 4294967295
    done
    ;;
encode)
    build
    onnx
    "$HARNESS" encode --models "$MODELS" --queries "$QUERIES" \
        --out "$VECTORS"
    ;;
eval)
    for structure in $STRUCTURES; do
        "$HARNESS" eval --data "$DATA" --index "$structure" --column binary \
            --queries "$QUERIES" --vectors "$VECTORS" \
            --case-1240 "$CASE_1240" \
            --out "$RAW/eval-$structure-binary.json"
    done
    for structure in $STRUCTURES; do
        # The EXACT column is the dense branch alone: it is the ceiling of the
        # binary scan, not a second fusion campaign.
        "$HARNESS" eval --data "$DATA" --index "$structure-exact" --column exact \
            --queries "$QUERIES" --vectors "$VECTORS" --only dense \
            --out "$RAW/eval-$structure-exact.json"
    done
    ;;
analyze)
    build
    onnx
    for structure in $STRUCTURES; do
        "$HARNESS" analyze --data "$DATA" --index "$structure" \
            --structure "$structure" \
            --queries "$QUERIES" --vectors "$VECTORS" \
            --out "$RAW/analyze-$structure.json"
    done
    ;;
aggregate)
    ( cd "$ROOT/tools" && uv run --no-sync python bench/mes10/aggregate.py \
        --commit "$(git rev-parse HEAD)" --date 2026-10-28 \
        --machine "MacBook Air M3, 8 cores, 24 GB, macOS 15.7.7, native" \
        --write )
    ;;
*)
    echo "usage: $0 {extract|prepare|encode|eval|analyze|aggregate}" >&2
    exit 2
    ;;
esac
