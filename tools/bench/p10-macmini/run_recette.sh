#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Replays INV-12 on the Mac mini, in the distinct volume of ADR-0141 decision 1:
# an empty `dole` index is created from `docs/legifacile/configs/dole.json`, the
# original `dole-full-v1` batch is attached, and `crible inspect` reads the
# result. The served `crible-p10-data` volume is never written.
#
# Usage:
#   sh tools/bench/p10-macmini/run_recette.sh
#
# The exchange folder carries the batch and the configuration; the volume is
# created once. Every container is `--rm` and foreground.

set -eu

HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$HERE/common.sh"

BASE=$SSD_EXCHANGE/p10-macmini
RECETTE_VOLUME=crible-p10-recette

log() {
    printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*"
}

remote "docker volume inspect $RECETTE_VOLUME >/dev/null 2>&1 || docker volume create $RECETTE_VOLUME >/dev/null"
# The named volume is created by the daemon as root; `crible` runs as uid 1000.
remote "docker run --rm --user 0 --entrypoint sh -v $RECETTE_VOLUME:/recette $IMAGE -c 'chown 1000:1000 /recette'"

log "create the empty dole index"
remote "docker run --rm -v $RECETTE_VOLUME:/recette -v $BASE/pass-c:/in:ro $IMAGE index create --data /recette --config /in/dole.json"

log "attach dole-full-v1"
remote "docker run --rm -v $RECETTE_VOLUME:/recette -v $MODELS_VOLUME:/models:ro -v $BASE/pass-c:/in:ro $IMAGE attach --data /recette --models /models --index dole /in/dole-full-v1"

log "inspect the restored index"
remote "docker run --rm -v $RECETTE_VOLUME:/recette $IMAGE inspect --data /recette --index dole"
