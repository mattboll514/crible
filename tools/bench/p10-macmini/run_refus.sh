#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Exercises the refusal branch of INV-12 on the Mac mini, in the distinct
# `crible-p10-recette` volume only. The batch declares another model; the
# attach must return `incompatible_model` and leave the generation unchanged.
#
# Usage:
#   sh tools/bench/p10-macmini/build_refus_batch.py ...
#   sh tools/bench/p10-macmini/run_refus.sh [local-batch-dir]
#
# The default local batch is `data/p10-macmini/pass-c/refus/dole-refus-model`.
# Every container is `--rm` and foreground; `crible-p10-data` is never opened.

set -eu

HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$HERE/common.sh"

LOCAL=${1:-$ROOT/data/p10-macmini/pass-c/refus/dole-refus-model}
BASE=$SSD_EXCHANGE/p10-macmini
DEST=$BASE/pass-c/dole-refus-model
RECETTE_VOLUME=crible-p10-recette

if [ ! -f "$LOCAL/manifest.json" ]; then
    echo "missing batch manifest: $LOCAL/manifest.json" >&2
    exit 2
fi

log() {
    printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*"
}

remote "mkdir -p $DEST"
scp -o BatchMode=yes -r "$LOCAL"/. "$PROD_SSH:$DEST/"

log "inspect before"
remote "docker run --rm -v $RECETTE_VOLUME:/recette $IMAGE inspect --data /recette --index dole"

log "attach the incompatible batch"
set +e
remote "docker run --rm -v $RECETTE_VOLUME:/recette -v $MODELS_VOLUME:/models:ro -v $BASE/pass-c:/in:ro $IMAGE attach --data /recette --models /models --index dole /in/dole-refus-model"
code=$?
set -e
printf 'ATTACH_EXIT:%s\n' "$code"

log "inspect after"
remote "docker run --rm -v $RECETTE_VOLUME:/recette $IMAGE inspect --data /recette --index dole"

if [ "$code" -ne 1 ]; then
    echo "expected exit code 1 from the refusal, got $code" >&2
    exit 1
fi
