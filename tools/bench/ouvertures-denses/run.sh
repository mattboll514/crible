#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Campaign driver of the measurement attached to the task
# `REUTILISER-LECTEURS-DENSES` (phase P7, deliverable 1): cost of
# `DenseSearcher::open` per query, on the `synth-large` index at 8.58 M
# passages, plus two smaller points.
#
# It is a recipe, not an authority: every command is also spelled out in
# `docs/mesures/lecteurs-denses-ouverture-2026-11-01.md`. Run it from the
# repository root, in `--release`, with no other heavy load on the machine.
#
#   MACHINE='MacBook Air M3, 24 GB, macOS 15.7.7, native' \
#     sh tools/bench/ouvertures-denses/run.sh
#
# The harness is `crates/crible/examples/dense_latency.rs`, mode `open`. It
# changes nothing in `crates/*/src/`: it calls the public
# `DenseSearcher::open` and drops the searcher before the next opening, so each
# measurement pays the whole opening, checksum included.
#
# The first opening of each point is reported apart (`first_ms`); the series
# that follows runs with the files in the page cache. The true cold series is
# not measured here: ADR-0084 defers it to P8, in a container.

set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$ROOT"

HARNESS=target/release/examples/dense_latency
OUT=data/mes-lecteurs-denses
OPENS=${OPENS:-200}
DATE=${DATE:-2026-11-01}

mkdir -p "$OUT"

echo "== machine"
sysctl hw.model
sysctl -n machdep.cpu.brand_string
sw_vers
echo "commit: $(git rev-parse HEAD)"
echo "== load before"
uptime

cargo build --release --example dense_latency -p crible

# 8.58 M passages: the point the decision of the task rests on.
/usr/bin/time -p "$HARNESS" open \
  --data data/synth-large --index synth-large \
  --out "$OUT/open-synth-large-$DATE.json" --opens "$OPENS" \
  2>&1 | tee "$OUT/open-synth-large-$DATE.log"

# Growth point on the real corpus: ~50 k passages per structure.
/usr/bin/time -p "$HARNESS" open \
  --data data --index articles \
  --out "$OUT/open-articles-$DATE.json" --opens "$OPENS" \
  2>&1 | tee "$OUT/open-articles-$DATE.log"

# Growth point on the small synthetic corpus.
/usr/bin/time -p "$HARNESS" open \
  --data data --index synth-small \
  --out "$OUT/open-synth-small-$DATE.json" --opens "$OPENS" \
  2>&1 | tee "$OUT/open-synth-small-$DATE.log"

echo "== load after"
uptime
