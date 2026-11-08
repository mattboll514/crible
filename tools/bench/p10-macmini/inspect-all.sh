#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Reports the state of every restored index, server stopped (ADR-0116 keeps
# `crible inspect` offline: on a volume opened for writing it waits on the
# writer lock and blocks).
#
#   sh tools/bench/p10-macmini/inspect-all.sh

set -eu
. "$(dirname "$0")/common.sh"

if remote "docker container inspect $CONTAINER >/dev/null 2>&1 &&
    [ \"\$(docker inspect --format '{{.State.Running}}' $CONTAINER)\" = true ]"; then
    echo "$CONTAINER is running: stop it before inspecting" >&2
    exit 1
fi

$SSH "$PROD_SSH" "$REMOTE_CMD docker run --rm -v $DATA_VOLUME:/data $IMAGE \
    inspect --data /data" 2>&1 | tee "$OUT/inspect-all.txt"
