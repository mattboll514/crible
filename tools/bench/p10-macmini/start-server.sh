#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Starts the single Crible container of the Mac mini and records its footprint
# (ADR-0141, decisions 1 and 3).
#
#   sh tools/bench/p10-macmini/start-server.sh
#
# The configuration is the one of the development machine
# (`data/p10-exchange/crible.toml`), copied to the exchange directory of the
# SSD. The port is published on `127.0.0.1` only, and no `--memory` cap is
# passed: the RAM of the VM is the bound (ADR-0135, decision 5; ADR-0139,
# decision 3).

set -eu
. "$(dirname "$0")/common.sh"

$SSH "$PROD_SSH" "cat > $SSD_EXCHANGE/crible.toml" < "$EXCHANGE_CONFIG"

if remote "docker container inspect $CONTAINER >/dev/null 2>&1"; then
    remote "docker start $CONTAINER >/dev/null"
else
    remote "docker run -d --name $CONTAINER -p 127.0.0.1:$PORT:8080 --stop-timeout 65 \
        -v $DATA_VOLUME:/data \
        -v $MODELS_VOLUME:/models \
        -v $SSD_EXCHANGE:/exchange \
        -v $SSD_EXCHANGE/crible.toml:/etc/crible/crible.toml:ro \
        $IMAGE" > "$OUT/server-run.txt"
fi

i=0
while ! remote "curl -fsS --max-time 3 http://127.0.0.1:$PORT/ready" >/dev/null 2>&1; do
    i=$((i + 10))
    if [ "$i" -gt 600 ]; then
        echo "the server is not ready after ${i}s" >&2
        remote "docker logs --tail 30 $CONTAINER" >&2
        exit 1
    fi
    echo "waiting ${i}s"
    sleep 10
done
echo "ready after ${i}s"

remote "docker stats --no-stream --format '{{.Name}} {{.CPUPerc}} {{.MemUsage}} {{.MemPerc}}'; \
    docker ps --format '{{.Names}}\t{{.Ports}}\t{{.Image}}'; \
    df -h / $SSD_EXCHANGE/..; sysctl vm.swapusage; memory_pressure -Q 2>/dev/null | tail -3" \
    2>&1 | tee "$OUT/host-after.txt"
