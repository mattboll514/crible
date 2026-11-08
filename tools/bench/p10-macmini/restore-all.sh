#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Restores the six snapshots into the named volume `crible-p10-data`, each in
# an ephemeral container run in the foreground (ADR-0141, decisions 1 and 2).
#
#   sh tools/bench/p10-macmini/restore-all.sh               # all six, smallest first
#   sh tools/bench/p10-macmini/restore-all.sh dole-e1-g1   # one snapshot
#
# The restore is offline: no Crible server may hold `/data` while it runs. The
# report of each restore (epoch replaced or "none", new epoch, generation,
# documents, passages) is kept under `data/p10-macmini/`.
#
# Smallest first here, unlike the copy: a restore that fails on `dole` stops the
# sequence before 25 GB have been written for nothing.

set -eu
. "$(dirname "$0")/common.sh"

if [ "$#" -gt 0 ]; then
    names=$*
else
    names="dole-e1-g1 sarde-e1-g1 titres-e1-g1 jorf-e3-g3 articles-e1-g1 decisions-e2-g2"
fi

remote "docker volume create $DATA_VOLUME >/dev/null"

for name in $names; do
    index=${name%%-e*}
    echo "== $name"
    $SSH "$PROD_SSH" "$REMOTE_CMD docker run --rm \
        -v $DATA_VOLUME:/data \
        -v $REMOTE_SNAPSHOTS/$name:/snapshot:ro \
        $IMAGE restore --data /data --snapshot /snapshot" \
        2>&1 | tee "$OUT/restore-$index.txt"
done
