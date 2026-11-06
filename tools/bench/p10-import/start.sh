#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Starts the Crible instance of the import campaign and samples the memory of
# the container while the server opens its indexes.
#
#   sh tools/bench/p10-import/start.sh
#
# The container is `crible-p10-server-a2` (`$CONTAINER` to change it): the
# server of the first attempt, `crible-p10-server`, stays stopped and keeps the
# image it ran with, so the report can name both. `prepare.sh` starts that first
# container and holds port 18090; when the new container has to be created, this
# script stops it first (`docker stop`, never `docker rm`) so the port is free
# and the order `prepare.sh` then `start.sh` works from a fresh tree. The volumes, the exchange
# directory, the configuration and the six read-only lot mounts are the ones of
# `prepare.sh`.
#
# Opening the indexes in write mode resumes or rebuilds the lexical branch of
# an index whose lexical generation is behind its generation (ADR-0087,
# ADR-0088). The delay to `/ready` covers that work, and the sampler records
# `memory.current`, `memory.stat` (`anon`, `file`) and the RSS of the cgroup
# every five seconds while it runs.

set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$ROOT"

IMAGE=${IMAGE:-crible:p10}
DATA_VOLUME=crible-p10-data
MODELS_VOLUME=crible-p10-models
EXCHANGE=$ROOT/data/p10-exchange
OUT=$ROOT/data/p10-import
CONTAINER=${CONTAINER:-crible-p10-server-a2}
INTERVAL=${INTERVAL:-5}

if docker container inspect "$CONTAINER" >/dev/null 2>&1; then
    docker start "$CONTAINER" >/dev/null
else
    if docker container inspect crible-p10-server >/dev/null 2>&1 &&
        [ "$(docker inspect --format '{{.State.Running}}' crible-p10-server)" = "true" ]; then
        echo "stopping crible-p10-server to free port 18090 (it is kept)" >&2
        docker stop --timeout 65 crible-p10-server >/dev/null
    fi
    docker run -d --name "$CONTAINER" -p 18090:8080 --stop-timeout 65 \
        -v "$DATA_VOLUME":/data \
        -v "$MODELS_VOLUME":/models \
        -v "$EXCHANGE":/exchange \
        -v "$EXCHANGE/crible.toml:/etc/crible/crible.toml:ro" \
        -v "$ROOT/data/legifacile-full/dole/dole-full-v1:/exchange/inbox/dole-full-v1:ro" \
        -v "$ROOT/data/legifacile-full/sarde/sarde-full-v1:/exchange/inbox/sarde-full-v1:ro" \
        -v "$ROOT/data/legifacile-full/titres/titres-full-v2:/exchange/inbox/titres-full-v2:ro" \
        -v "$ROOT/data/legifacile-full/jorf/jorf-full-v1:/exchange/inbox/jorf-full-v1:ro" \
        -v "$ROOT/data/legifacile-full/articles/articles-full-v1:/exchange/inbox/articles-full-v1:ro" \
        -v "$ROOT/data/legifacile-full/decisions/decisions-full-v1:/exchange/inbox/decisions-full-v1:ro" \
        "$IMAGE"
fi

sample() {
    printf '{"at":"%s","memory_current":%s,"anon":%s,"file":%s,"rss":%s}\n' \
        "$(date -u +"%Y-%m-%dT%H:%M:%SZ")" \
        "$(docker exec "$CONTAINER" cat /sys/fs/cgroup/memory.current 2>/dev/null || echo null)" \
        "$(docker exec "$CONTAINER" sh -c 'awk "/^anon /{print \$2}" /sys/fs/cgroup/memory.stat' 2>/dev/null || echo null)" \
        "$(docker exec "$CONTAINER" sh -c 'awk "/^file /{print \$2}" /sys/fs/cgroup/memory.stat' 2>/dev/null || echo null)" \
        "$(docker exec "$CONTAINER" sh -c 'awk "/^VmRSS/{print \$2}" /proc/1/status' 2>/dev/null || echo null)"
}

stamp=$(date -u +%Y%m%dT%H%M%SZ)
LOG=$OUT/memory-open-$CONTAINER-$stamp.jsonl
: > "$LOG"
started=$(date +%s)
started_at=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

while ! curl -fsS --max-time 3 http://127.0.0.1:18090/ready >/dev/null 2>&1; do
    sample >> "$LOG"
    elapsed=$(( $(date +%s) - started ))
    if [ "$elapsed" -gt 600 ]; then
        echo "the server is not ready after 600 s" >&2
        docker logs --tail 50 "$CONTAINER" >&2
        exit 1
    fi
    sleep "$INTERVAL"
done
sample >> "$LOG"
ended=$(date +%s)

{
    echo "container: $CONTAINER"
    echo "image: $(docker inspect "$CONTAINER" --format '{{.Image}}')"
    echo "started_at: $started_at"
    echo "ready_at: $(date -u +"%Y-%m-%dT%H:%M:%SZ")"
    echo "seconds_to_ready: $((ended - started))"
} | tee "$OUT/open-$CONTAINER-$stamp.txt"

docker exec "$CONTAINER" df -h /data | tee "$OUT/df-vm-open.txt"
curl -fsS http://127.0.0.1:18090/ready | tee "$OUT/ready-passA2.json" >/dev/null
