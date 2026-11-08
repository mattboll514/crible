#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Attach of the six `replace_all` lots of the import campaign of
# IMPORTER-BASE-COMPLETE (P10, ADR-0139, decision 5, points 3 and 4).
#
#   sh tools/bench/p10-import/attach-all.sh
#
# The lots are attached smallest first. Before each attach, the host's free
# space is read (`df -H /`, the binding bound of ADR-0139, decision 1); under
# 40 Go no further lot is submitted, and the script stops without touching the
# running job. After each attach, both the host and the VM are read again.
#
# Every raw figure goes to `data/p10-import/` (gitignored): the responses of
# the jobs, the polls of `GET /v1/jobs/{id}`, the `df` outputs and the
# timestamped server journal.

set -u

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$ROOT"

OUT=$ROOT/data/p10-import
CONTAINER=${CONTAINER:-crible-p10-server}   # start.sh creates crible-p10-server-a2
PYTHON=${PYTHON:-python3}
HOST_LOG=$OUT/df-host-per-lot.txt
VM_LOG=$OUT/df-vm-per-lot.txt

mkdir -p "$OUT"

record_host() {
    {
        date -u +"%Y-%m-%dT%H:%M:%SZ"
        echo "# $1"
        df -h /
        df -H /
        echo
    } >> "$HOST_LOG"
}

record_vm() {
    {
        date -u +"%Y-%m-%dT%H:%M:%SZ"
        echo "# $1"
        docker exec "$CONTAINER" df -h /data
        docker exec "$CONTAINER" du -sh /data/indexes
        echo
    } >> "$VM_LOG"
}

free_go() {
    df -H / | awk 'NR==2 {sub(/G$/, "", $4); print $4}'
}

if [ "$#" -gt 0 ]; then
    pairs=$*
else
    pairs="dole:dole-full-v1 sarde:sarde-full-v1 titres:titres-full-v2 jorf:jorf-full-v1 articles:articles-full-v1 decisions:decisions-full-v1"
fi

for pair in $pairs
do
    index=${pair%%:*}
    lot=${pair#*:}

    record_host "before attach $index ($lot)"
    record_vm "before attach $index ($lot)"
    available=$(free_go)
    echo "$index: $available Go free on the host before attach"
    if [ "$available" -lt 40 ]; then
        echo "$index: $available Go free, under the 40 Go stop threshold of ADR-0139; no lot submitted" | tee -a "$HOST_LOG"
        docker logs -t "$CONTAINER" > "$OUT/server.log" 2>&1
        exit 2
    fi

    started=$(date +%s)
    "$PYTHON" tools/bench/p10-import/import.py attach --index "$index" --lot "$lot"
    code=$?
    while [ "$code" -eq 3 ]; do
        "$PYTHON" tools/bench/p10-import/import.py wait --index "$index"
        code=$?
    done
    ended=$(date +%s)

    {
        date -u +"%Y-%m-%dT%H:%M:%SZ"
        echo "# after attach $index: exit $code, $((ended - started)) s of wall clock"
    } >> "$HOST_LOG"
    record_host "after attach $index ($lot)"
    record_vm "after attach $index ($lot)"

    if [ "$code" -ne 0 ]; then
        echo "$index: attach failed with exit $code; the campaign stops" >&2
        docker logs -t "$CONTAINER" > "$OUT/server.log" 2>&1
        exit 1
    fi
done

docker logs -t "$CONTAINER" > "$OUT/server.log" 2>&1
echo "six lots attached; the server journal is in $OUT/server.log"
