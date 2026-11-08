#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Checks the size of every file of the copied snapshots against `snapshot.json`
# (ADR-0064, decision 1). The blake3 control is `crible restore`, which refuses
# a missing file, a different size or a different blake3 (ADR-0141, decision 2).
#
#   sh tools/bench/p10-macmini/verify-arrival.sh [<snapshot>…]

set -eu
. "$(dirname "$0")/common.sh"

if [ "$#" -gt 0 ]; then
    names=$*
else
    names=$SNAPSHOT_ORDER
fi

$SSH "$PROD_SSH" "python3 - $REMOTE_SNAPSHOTS $names" < "$HERE/check-arrival.py" \
    | tee "$OUT/arrival-sizes.txt"
