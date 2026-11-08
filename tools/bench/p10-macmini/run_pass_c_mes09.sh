#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# MES-09 of pass C, MESURER-SUR-MAC-MINI (P10, ADR-0141 decision 5): the real
# memory of the served instance, with the six indexes open, in the running
# container of the Mac mini. The series is the instance series only: no folder
# is moved under `/data` (ADR-0129, rule 2 of decision 9).
#
# Usage:
#   sh tools/bench/p10-macmini/run_pass_c_mes09.sh record <label>
#
# The caller restarts `crible-p10` before `rest`, sends the reference game
# through `direct_latency.py`, then calls `record after-load`. The script reads
# the container's own cgroup and process counters; it never touches Supabase,
# the Legifacile repository or Docker Desktop.

set -eu

HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$HERE/common.sh"

BASE=$SSD_EXCHANGE/p10-macmini
LOCAL=$OUT/pass-c/mes09

[ "${1:-}" = record ] || {
    echo "usage: $0 record <label>" >&2
    exit 2
}
label=${2:?label}
mkdir -p "$LOCAL"

{
    remote "date -u +%Y-%m-%dT%H:%M:%SZ"
    remote "docker exec $CONTAINER sh -c '
        grep -E \"^(VmRSS|VmSwap|RssAnon|RssFile):\" /proc/1/status
        echo memory.current
        cat /sys/fs/cgroup/memory.current
        echo memory.stat
        grep -E \"^(anon|file|file_mapped|inactive_file|active_file|kernel|slab|sock) \" /sys/fs/cgroup/memory.stat
    '"
    remote "docker stats --no-stream --format '{{.Name}} {{.MemUsage}} {{.MemPerc}}' $CONTAINER"
    remote "sysctl vm.swapusage"
    remote "docker ps --format '{{.Names}} {{.Status}}'"
} >"$LOCAL/$label.txt"

printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "mes09 $label -> $LOCAL/$label.txt"
