#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# One per-index series of MES-09: the server is stopped, the five other index
# directories are moved to /data/staging-mes09 (a move inside the volume, never a
# deletion), the server is started with the single index, the three states of
# ADR-0077 are measured, the server is stopped and the directories are moved
# back. Nothing is removed, and the script says where it stopped if it fails.
#
#   sh tools/bench/p10-import/mes09-per-index.sh decisions
#
# The last step runs even when the measurement fails, so the instance is never
# left with a partial `/data/indexes`.

set -u

INDEX=${1:-}
if [ -z "$INDEX" ]; then
    echo "usage: sh tools/bench/p10-import/mes09-per-index.sh <index>" >&2
    exit 2
fi

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$ROOT" || exit 1

stop_server() {
    docker stop --timeout 65 crible-p10-server-a2 >/dev/null 2>&1 || true
}

restore() {
    python3 tools/bench/p10-import/mes09.py move-back || {
        echo "RESTORE FAILED: the indexes are in /data/staging-mes09" >&2
        return 1
    }
}

stop_server
python3 tools/bench/p10-import/mes09.py move-out --keep "$INDEX" || {
    echo "MOVE-OUT FAILED for $INDEX" >&2
    exit 1
}

sh tools/bench/p10-import/start.sh >/dev/null 2>&1
started=$?
python3 tools/bench/p10-import/mes09.py series --label "$INDEX" --target "$INDEX"
measured=$?

stop_server
restore
restored=$?

python3 tools/bench/p10-import/mes09.py list
if [ "$started" -ne 0 ] || [ "$measured" -ne 0 ] || [ "$restored" -ne 0 ]; then
    echo "series $INDEX incomplete: start=$started measure=$measured restore=$restored" >&2
    exit 1
fi
echo "series $INDEX done"
