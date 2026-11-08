#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Copies snapshots to the exchange directory of the external SSD, one index at
# a time and largest first (ADR-0141, decisions 1 and 2).
#
#   sh tools/bench/p10-macmini/copy-snapshots.sh                      # all six
#   sh tools/bench/p10-macmini/copy-snapshots.sh decisions-e2-g2     # one index
#
# The measured link carried about 4 MB/s, so a whole index would break the
# 25-minute limit of one command: the script copies the top-level files and
# then `segments/` sub-directory by sub-directory. `--partial` keeps a partial
# file, and a rerun of the same command resumes the copy. `--progress` keeps a
# long copy from staying silent.

set -eu
. "$(dirname "$0")/common.sh"

if [ "$#" -gt 0 ]; then
    names=$*
else
    names=$SNAPSHOT_ORDER
fi

remote "mkdir -p $REMOTE_SNAPSHOTS"

for name in $names; do
    src=$LOCAL_SNAPSHOTS/$name
    dst=$REMOTE_SNAPSHOTS/$name
    echo "== $name"
    rsync -a --partial --progress --exclude segments \
        -e "ssh -o BatchMode=yes" "$src/" "$PROD_SSH:$dst/"
    for sub in "$src"/segments/*; do
        [ -d "$sub" ] || continue
        rsync -a --partial --progress -e "ssh -o BatchMode=yes" \
            "$sub" "$PROD_SSH:$dst/segments/"
    done
done

sh "$HERE/verify-arrival.sh" $names
