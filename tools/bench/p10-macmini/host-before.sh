#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Records the state of the Mac mini before any gesture: free space, swap and
# memory pressure of the host, containers and volumes, and the size of the
# Docker VM (ADR-0141, decision 3).
#
#   sh tools/bench/p10-macmini/host-before.sh            # writes host-before.txt
#   sh tools/bench/p10-macmini/host-before.sh after.txt  # writes another file

set -eu
. "$(dirname "$0")/common.sh"

out=${1:-host-before.txt}

remote "date -u +%FT%TZ; sw_vers -productVersion; uname -m; \
    sysctl -n hw.model hw.ncpu hw.memsize; \
    df -h / /Volumes/MattCorp-SSD; sysctl vm.swapusage; \
    memory_pressure -Q 2>/dev/null | tail -4; \
    echo '--- containers'; docker ps -a --format '{{.Names}}\t{{.Status}}\t{{.Image}}'; \
    echo '--- volumes'; docker volume ls; \
    echo '--- vm'; docker info --format 'NCPU={{.NCPU}} MemTotal={{.MemTotal}} \
    Driver={{.Driver}} Arch={{.Architecture}} Root={{.DockerRootDir}}'" \
    2>&1 | tee "$OUT/$out"
