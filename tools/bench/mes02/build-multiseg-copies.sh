#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Builds the three measurement copies of the multi-segment campaign of MES-02
# (ADR-0081, decision 2). Run it once, from the repository root, before
# `run-multiseg.sh`; it is idempotent and refuses to rebuild a copy that exists.
#
#   - `data/mes02/indexes/synth-large`       the one-segment copy of MES-02
#                                            (built by `run.sh`; left untouched);
#   - `data/mes02/indexes/synth-large-8seg`  a clone of it, compacted: the
#                                            compaction writes segments of at most
#                                            1 M passages, so 7.74 M live passages
#                                            give 8 segments (MES-07 measured the
#                                            same 1 -> 8 on this dataset);
#   - `data/mes02/indexes/synth-large-multi` a fresh index fed by 100 attaches of
#                                            86,000 passages, then the same
#                                            deletions batch as the reference, so
#                                            its live bitmap is the one of
#                                            `synth-large`, iid for iid.
#
# Space: about 15.3 GB for the compaction (the old epoch is removed after the
# switch) and about 17 GB for the multi-attach copy. Nothing here writes to
# `data/synth-large/`, and no file of the shared dataset is modified.

set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$ROOT"
TOOLS="$ROOT/tools"
DATA="data/mes02"
CRIBLE="$ROOT/target/release/crible"

cargo build --release
mkdir -p "$DATA/indexes" "$DATA/configs"

# 1. The compacted copy: 8 segments of 1 M passages, no dead passage. A clone
#    costs no space until it is written, and the compaction replaces the epoch.
if [ ! -d "$DATA/indexes/synth-large-8seg" ]; then
    cp -cR "$DATA/indexes/synth-large" "$DATA/indexes/synth-large-8seg"
    (cd "$TOOLS" && uv run --no-sync python - "$ROOT/$DATA/indexes/synth-large-8seg/index.json" <<'PY'
import json, sys
path = sys.argv[1]
config = json.loads(open(path, encoding="utf-8").read())
config["name"] = "synth-large-8seg"
open(path, "w", encoding="utf-8").write(json.dumps(config, indent=2) + "\n")
PY
    )
    "$CRIBLE" compact --data "$DATA" --index synth-large-8seg
fi

# 2. The multi-attach copy: 100 batches of 86,000 passages, written from the
#    batch of the shared dataset, in row order, so the iid of a document is the
#    one it has in `synth-large`.
if [ ! -d "$DATA/indexes/synth-large-multi" ]; then
    (cd "$TOOLS" && uv run --no-sync python bench/mes02/make_multiseg_batch.py \
        --index synth-large-multi --chunks 100)
    (cd "$TOOLS" && uv run --no-sync python - "$ROOT/$DATA/configs/synth-large-multi.json" <<'PY'
import json, sys
path = sys.argv[1]
config = json.loads(open("data/synth-large/index.json", encoding="utf-8").read())
config["name"] = "synth-large-multi"
config["search"]["exact_threshold"] = 0
config["search"]["scan"] = "binary"
open(path, "w", encoding="utf-8").write(json.dumps(config, indent=2) + "\n")
PY
    )
    "$CRIBLE" index create --data "$DATA" --config "$DATA/configs/synth-large-multi.json"
    for batch in $(ls -d "$DATA"/batches/synth-large-multi/multi-* | sort); do
        "$CRIBLE" attach --data "$DATA" --models data/models \
            --index synth-large-multi "$batch"
    done
    "$CRIBLE" attach --data "$DATA" --models data/models \
        --index synth-large-multi "$DATA/batches/synth-large-multi-deletions-v1"
fi

# 3. Controls: the three copies are the ones the campaign declares, and the
#    one-segment and multi-attach copies share their live set iid for iid.
for index in synth-large synth-large-8seg synth-large-multi; do
    "$CRIBLE" inspect --data "$DATA" --index "$index" | head -8
done
sqlite3 "$DATA/indexes/synth-large/epochs/1/records.sqlite" \
    "attach database '$DATA/indexes/synth-large-multi/epochs/1/records.sqlite' as c;
     select 'live A', count(*) from passages where dead_since_gen is null;
     select 'live C', count(*) from c.passages where dead_since_gen is null;
     select 'live set differs by iid',
            count(*) from passages p join c.passages q on p.iid = q.iid
            where (p.dead_since_gen is null) <> (q.dead_since_gen is null);
     select 'doc differs by iid',
            count(*) from passages p join c.passages q on p.iid = q.iid
            where p.doc_id <> q.doc_id;"
