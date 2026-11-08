#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Copies the local `crible-p10-models` volume into a named volume on the Mac
# mini and proves the copy with SHA-256 (ADR-0141, decision 1).
#
#   sh tools/bench/p10-macmini/copy-models.sh
#
# The stream is a plain `tar` through `ssh`; the model directory is 1.5 GiB and
# the measured link carried it in 421 s. Both sides hash every file with the
# `crible:p10` image, so no tool is installed on the host.

set -eu
. "$(dirname "$0")/common.sh"

remote "docker volume create $MODELS_VOLUME >/dev/null; docker volume ls --format '{{.Name}}'"

docker run --rm --entrypoint tar -v "$MODELS_VOLUME":/models:ro "$IMAGE" \
    -C /models -cf - . |
    $SSH "$PROD_SSH" "$REMOTE_CMD docker run --rm -i --entrypoint tar \
        -v $MODELS_VOLUME:/models $IMAGE -C /models -xf -"

docker run --rm --entrypoint sh -v "$MODELS_VOLUME":/models:ro "$IMAGE" \
    -c 'cd /models && find . -type f -print0 | sort -z | xargs -0 sha256sum' \
    > "$OUT/models-local.sha256"

$SSH "$PROD_SSH" "$REMOTE_CMD docker run --rm --entrypoint sh \
    -v $MODELS_VOLUME:/models:ro $IMAGE \
    -c 'cd /models && find . -type f -print0 | sort -z | xargs -0 sha256sum'" \
    > "$OUT/models-remote.sha256"

wc -l < "$OUT/models-local.sha256"
if diff -u "$OUT/models-local.sha256" "$OUT/models-remote.sha256"; then
    echo "models: sha256 identical on both sides"
else
    echo "models: sha256 differ, copy again before restoring" >&2
    exit 1
fi
