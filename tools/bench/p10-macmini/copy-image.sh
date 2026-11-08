#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Loads `crible:p10` on the Mac mini through `docker save | ssh … docker load`
# and checks that both sides hold the same image id (ADR-0141, decision 1).
#
#   sh tools/bench/p10-macmini/copy-image.sh
#
# No image is rebuilt and nothing is published. The transfer is about 182 MB;
# it took 35 s on the measured link.

set -eu
. "$(dirname "$0")/common.sh"

local_id=$(docker image ls --no-trunc --format '{{.ID}}' "$IMAGE")
echo "local $IMAGE $local_id"

docker save "$IMAGE" |
    $SSH "$PROD_SSH" "$REMOTE_CMD docker load" |
    tee "$OUT/image-load.txt"

remote_id=$(remote "docker image ls --no-trunc --format '{{.ID}}' $IMAGE")
echo "remote $IMAGE $remote_id"

if [ "$local_id" != "$remote_id" ]; then
    echo "image ids differ: $local_id != $remote_id" >&2
    exit 1
fi
echo "image id identical on both sides"
