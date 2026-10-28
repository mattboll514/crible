#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Campaign driver of MES-02, MES-03 and MES-09 (dense search latency and RSS).
#
# It is a recipe, not an authority: every command is also spelled out in the
# reports (docs/mesures/MES-02-*.md, docs/mesures/MES-03-*.md,
# docs/mesures/MES-09-*.md). Run it from the repository root, in `--release`,
# with no other heavy load on the machine.
#
#   MACHINE='MacBook Air M3, 8 cores, 24 GB, macOS 15.7.7, native' \
#     sh tools/bench/mes02/run.sh
#
# The protocol is ADR-0077. The harness is
# `crates/crible/examples/dense_latency.rs`: it changes nothing in
# `crates/*/src/`, it **replicates** the binary scan and the exact scoring
# (precedent MES-13), and it anchors the replica to the engine with the
# `compare` mode. The only index it opens is a **measurement copy** whose
# `search.exact_threshold` is 0, so every point runs BINARY.
#
# Since ADR-0081 the binary scan has a single traversal (task
# `UNIFIER-PARCOURS-SCAN`): the two forced series of step 3 no longer exist in
# the current tree. This recipe reproduces the campaign of the 2026-10-27 and
# must run with the binary of commit `e7d4e08`, the last one that carries the
# density rule; ADR-0082's A/B runs `cross` and `agree` with the same command
# line on that binary and on the current one, and compares their documents.
# By default the harness is the archived binary of `e7d4e08` (`data/ab-old`, see
# `run-ab.sh`), not a build of this tree; `HARNESS=...` overrides it.

set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$ROOT"
TOOLS="$ROOT/tools"
# Relative paths on purpose: the harness writes them into the raw documents and
# the analyser copies them into the versioned artefacts, which must not carry a
# local absolute path.
DATA="data/mes02"
RAW="$DATA/raw"
COMMIT=$(git rev-parse HEAD)
DATE=${DATE:-$(date +%F)}
: "${MACHINE:?set MACHINE to the machine you measure on}"

mkdir -p "$RAW" "$DATA"

# ADR-0077, decision 2: the free space is checked before each copy, and only one
# copy exists at a time.
#   check_space <needed KiB> <directory> <what>
check_space() {
    free=$(df -k "$2" | awk 'NR==2 {print $4}')
    echo "space: $free KiB free on $2, $1 KiB needed for $3"
    if [ "$free" -lt "$1" ]; then
        echo "not enough space on $2 for $3: $free KiB free, $1 KiB needed" >&2
        exit 1
    fi
}

# 1. Measurement copy of `synth-large` (17 GB, gitignored): BINARY everywhere.
if [ ! -d "$DATA/indexes/synth-large" ]; then
    mkdir -p "$DATA/indexes"
    check_space 17000000 "$DATA" "the measurement copy of synth-large"
    cp -cR data/synth-large "$DATA/indexes/synth-large"
    (cd "$TOOLS" && uv run --no-sync python - "$ROOT/$DATA/indexes/synth-large/index.json" <<'PY'
import json, sys
path = sys.argv[1]
config = json.loads(open(path, encoding="utf-8").read())
config["search"]["exact_threshold"] = 0
config["search"]["scan"] = "binary"
open(path, "w", encoding="utf-8").write(json.dumps(config, indent=2) + "\n")
PY
    )
fi

# 2. The 300 query vectors of ADR-0077 decision 4, with their SHA-256.
(cd "$TOOLS" && uv run --no-sync python bench/mes02/mes02_queries.py)

# The CLI opened in step 5 is the engine's, built from this tree; the harness is
# not, see the block below.
cargo build --release --bin crible

# The binary of this campaign is the harness of commit `e7d4e08`, built outside
# the working tree (see `run-ab.sh`): since ADR-0081 the tree has a single
# traversal, and its documents carry no `traversal_automatic` nor
# `pairs[].dense/sparse`, which the analysers of this directory refuse. Override
# it with `HARNESS=/path/to/dense_latency` to measure with another build.
HARNESS=${HARNESS:-$ROOT/data/ab-old/target/release/examples/dense_latency}
if [ ! -x "$HARNESS" ]; then
    echo "missing harness '$HARNESS': build the binary of commit e7d4e08 first:" >&2
    echo "  mkdir -p data/ab-old/src" >&2
    echo "  git archive e7d4e08 | tar -x -C data/ab-old/src" >&2
    echo "  cd data/ab-old/src" >&2
    echo "  CARGO_TARGET_DIR=$ROOT/data/ab-old/target cargo build --release --example dense_latency" >&2
    echo "or set HARNESS to the binary you want to measure with" >&2
    exit 1
fi
INDEX="--data $DATA --index synth-large --vectors $DATA/queries.f32"

# 3. Scan: 1 k, 10 k, 100 k, 1 M, all live; contiguous and dispersed; 3 passes.
"$HARNESS" scan $INDEX --out "$RAW/scan-main.json"
"$HARNESS" scan $INDEX --sizes all --out "$RAW/scan-all.json"
# The two forced series of the 2026-10-27 campaign (`scan-forced-dense.json`,
# `scan-forced-sparse.json`) belong to the two-traversal code that ADR-0071
# described. ADR-0081 removed the second traversal, so a binary built from this
# tree no longer accepts `--force-traversal`; the default harness is the archived
# binary of `e7d4e08` (see above), which still accepts it and produces both files
# that `mes02_analyze.py` requires.
"$HARNESS" scan $INDEX --force-traversal dense --out "$RAW/scan-forced-dense.json"
"$HARNESS" scan $INDEX --force-traversal sparse --out "$RAW/scan-forced-sparse.json"

# 4. EXACT reference at the switching point (pool equal to N, k = 100).
"$HARNESS" exact $INDEX --out "$RAW/exact.json"

# 5. Controls: the replica against the engine, then the engine against the CLI.
"$HARNESS" compare $INDEX --filter '{"categorie":"cat-01"}' --out "$RAW/compare-cat01.json"
"$HARNESS" compare $INDEX --out "$RAW/compare-all.json"
(cd "$TOOLS" && uv run --no-sync python - "$ROOT/$DATA/query0.npy" <<'PY'
import sys
import numpy as np
queries = np.fromfile("../data/mes02/queries.f32", dtype="<f4").reshape(-1, 768)
np.save(sys.argv[1], queries[0])
PY
)
"$ROOT/target/release/crible" search --data "$DATA" --index synth-large \
    --vector-file "$DATA/query0.npy" --k 10 --filter '{"categorie":"cat-01"}' > "$RAW/cli-cat01.json"
"$ROOT/target/release/crible" search --data "$DATA" --index synth-large \
    --vector-file "$DATA/query0.npy" --k 10 > "$RAW/cli-all.json"

# 6. Rescoring at N = 1000, 2000, 5000: warm, then the two cold series.
"$HARNESS" rescore $INDEX --dot-only --key-only --out "$RAW/rescore-warm.json"
# Key reads alone, in a process that opens the searcher but never rescans: this
# is the series the report calls `warm-key-only-checked` (engine durations null).
"$HARNESS" rescore $INDEX --skip-engine --key-only --out "$RAW/key-only.json"
"$HARNESS" rescore $INDEX --skip-engine --key-only --passes 1 --out "$RAW/key-only-isolated.json"

# Cold vectors alone: uncached copies, checked page by page with `mincore`.
mkdir -p "$DATA/cold"
for n in 1000 2000 5000; do
    check_space 13000000 "$DATA/cold" "the uncached copy vectors-$n.f16"
    "$HARNESS" copy-nocache --src "$DATA/indexes/synth-large/epochs/1/segments/seg-large-0001/vectors.f16" \
        --dst "$DATA/cold/vectors-$n.f16"
    "$HARNESS" rescore $INDEX --cold --dot-only --dot-vectors-file "$DATA/cold/vectors-$n.f16" \
        --ns "$n" --passes 1 --out "$RAW/rescore-cold-vectors-$n.json"
    rm -f "$DATA/cold/vectors-$n.f16"
done

# The displacement sources: the nine Parquet batches of the sample and the ten
# vector segments of MES-15, about 19.5 GB. The file holds a single line, the
# repeated `--displace-source <path>` flags, ready to be expanded unquoted on
# the command line; it is regenerated at every run.
DISPLACE="$RAW/displace-sources.txt"
: > "$DISPLACE"
for source in data/synth-large/large-0001/passages-000*.parquet \
              data/mes15/i-8580000-s10/epochs/1/segments/seg-000*/vectors.f16; do
    if [ ! -f "$source" ]; then
        echo "missing displacement source: $source" >&2
        exit 1
    fi
    printf ' --displace-source %s' "$source" >> "$DISPLACE"
done

# Cold engine: the same series after the page cache was displaced by unrelated
# reads (13.5 GB at N = 1000, 19.5 GB at N = 2000 and 5000, the whole list of
# `data/mes02/raw/displace-sources.txt`; the document records the value).
for n in 1000 2000 5000; do
    case "$n" in
        1000) displace=13.5 ;;
        *) displace=19.5 ;;
    esac
    "$HARNESS" rescore $INDEX --cold --ns "$n" --passes 1 \
        --displace-gb "$displace" $(cat "$RAW/displace-sources.txt") \
        --out "$RAW/rescore-cold-engine-$n.json"
done

# Secondary series: after ten minutes without touching the index (ADR-0077).
sleep 600
"$HARNESS" rescore $INDEX --cold --passes 1 --out "$RAW/rescore-idle.json"

# 7. RSS in three states, three times, then the fp32 session on its own runs.
for run in 1 2 3; do
    "$HARNESS" rss $INDEX --passes 1 --out "$RAW/rss-base-$run.json"
done
export ORT_DYLIB_PATH=${ORT_DYLIB_PATH:-$(cd "$TOOLS" && uv run --no-sync python -c \
    'import onnxruntime, pathlib; print(next((pathlib.Path(onnxruntime.__file__).parent / "capi").glob("libonnxruntime*.dylib")))')}
for run in 1 2; do
    "$HARNESS" rss $INDEX --model data/models/mdenseon --skip-pass \
        --out "$RAW/rss-model-cold-$run.json"
done

# 8. Aggregates and the versioned artefacts.
(cd "$TOOLS" && uv run --no-sync python bench/mes02/mes02_analyze.py \
    --commit "$COMMIT" --date "$DATE" --machine "$MACHINE" --mesures "$ROOT/docs/mesures")

# 9. Tests of the aggregation.
(cd "$TOOLS" && uv run --no-sync pytest bench/mes02/tests)

echo "campaign files under $DATA"
