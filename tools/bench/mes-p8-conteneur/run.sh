#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Campaign driver of the container measurement (task MESURER-EN-CONTENEUR-MAC,
# P8; protocol ADR-0135, on top of ADR-0077, ADR-0080 and ADR-0084).
#
# Run it from the repository root, after `tools/bench/mes-p8-conteneur/prepare.sh`,
# one subcommand at a time, in the foreground:
#
#   sh tools/bench/mes-p8-conteneur/run.sh env
#   sh tools/bench/mes-p8-conteneur/run.sh mes02        # MES-02, scan
#   sh tools/bench/mes-p8-conteneur/run.sh mes03-warm   # MES-03, warm
#   sh tools/bench/mes-p8-conteneur/run.sh mes03-cold   # MES-03, cold (ADR-0135)
#   sh tools/bench/mes-p8-conteneur/run.sh mes03-limit  # passage B, shipped k = 100
#   sh tools/bench/mes-p8-conteneur/run.sh mes05        # passage B, query encoding
#   sh tools/bench/mes-p8-conteneur/run.sh mes08        # passage B, batch-size sweep
#   sh tools/bench/mes-p8-conteneur/run.sh routes       # passage B, two read routes
#   sh tools/bench/mes-p8-conteneur/run.sh mes09        # MES-09, RSS
#   sh tools/bench/mes-p8-conteneur/run.sh collect      # raw documents to data/
#
# Every measured series runs in its own container, in the foreground, and writes
# into `/data/raw` of the named volume `crible-mes-p8-data`. Nothing runs in the
# background and no command of this file lasts more than a few minutes.
#
# ADR-0135, decision 7: one **complete pass of the query set, discarded**,
# precedes the three measured passes of every campaign. The warm-up documents are
# written and kept (`warmup-*.json`), so the report can say what was discarded.
#
# ADR-0135, decision 1: the cold protocol is `rescore --cold --evict`, which
# calls `madvise(MADV_PAGEOUT)` on the pages of the candidates, then
# `posix_fadvise(POSIX_FADV_DONTNEED)`, then reads the residency of those pages
# with `mincore` before the query runs. `drop_caches` in a privileged container
# is only a counter-check, declared global to the VM, and every cold series runs
# in a fresh container after that purge attempt.

set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$ROOT"

VOLUME=crible-mes-p8-data
HARNESS=crible-mes-p8-harness:dev
CRIBLE=crible:dev
DEBIAN=debian:bookworm-slim
RAW=data/mes-p8-conteneur/raw
STAGING=data/mes-p8-conteneur/staging
COMMIT=$(git rev-parse HEAD)
DATE=${DATE:-$(date +%F)}

mkdir -p "$RAW" "$STAGING"

harness() {
    docker run --rm -v "$VOLUME":/data "$HARNESS" "$@"
}

crible() {
    docker run --rm -v "$VOLUME":/data "$CRIBLE" "$@"
}

# `drop_caches` from a privileged container: a counter-check, global to the VM.
# Docker Desktop may refuse it (the container does not own `/proc/sys`); the
# script records which happened and goes on.
purge_global() {
    echo "== global purge attempt (counter-check, global to the VM)"
    if docker run --rm --privileged "$DEBIAN" sh -c 'sync; echo 3 > /proc/sys/vm/drop_caches' 2>"$RAW/purge-error.txt"; then
        echo "drop_caches: applied" | tee "$RAW/purge.txt"
    else
        echo "drop_caches: refused by the VM, the series ran without it" | tee "$RAW/purge.txt"
    fi
}

case "${1:-}" in
env)
    # Machine, image and dataset, as the reports must declare them.
    {
        echo "commit: $COMMIT"
        echo "date: $DATE"
        echo "branch: $(git branch --show-current)"
        echo "host: $(sysctl -n machdep.cpu.brand_string 2>/dev/null || uname -m), $(sysctl -n hw.ncpu) cores"
        echo "docker client: $(docker version --format '{{.Client.Version}}')"
        echo "docker server: $(docker version --format '{{.Server.Version}}') ($(docker version --format '{{.Server.Arch}}'))"
        echo "vm memory: $(docker info --format '{{.MemTotal}}') bytes"
        echo "vm cpus: $(docker info --format '{{.NCPU}}')"
        echo "crible image: $(docker image inspect --format '{{.Id}}' $CRIBLE)"
        echo "harness image: $(docker image inspect --format '{{.Id}}' $HARNESS)"
        echo "crible binary: $(docker run --rm --entrypoint sh $CRIBLE -c 'sha256sum /usr/local/bin/crible | cut -d" " -f1')"
        echo "harness binary: $(docker run --rm --entrypoint sh $HARNESS -c 'sha256sum /usr/local/bin/dense_latency | cut -d" " -f1')"
        echo "queries sha256: $(shasum -a 256 data/mes02/queries.f32 | cut -d' ' -f1)"
    } > "$STAGING/environment.txt"
    cat "$STAGING/environment.txt"
    docker run --rm --entrypoint sh "$CRIBLE" -c '
        echo "page size: $(getconf PAGESIZE)"
        echo "arch: $(uname -m)"
        echo "nproc: $(nproc)"
        head -3 /proc/meminfo
        echo "cgroup memory.max: $(cat /sys/fs/cgroup/memory.max 2>/dev/null)"
        echo "cgroup memory.high: $(cat /sys/fs/cgroup/memory.high 2>/dev/null)"' \
        > "$STAGING/container-environment.txt"
    cat "$STAGING/container-environment.txt"
    ;;
mes02)
    INDEX="--data /data --index synth-large --vectors /data/queries.f32"
    echo "== warm-up: one complete pass of the query set, discarded"
    harness scan $INDEX --passes 1 --ns 1000 --out /data/raw/warmup-scan-n1000.json
    harness scan $INDEX --passes 1 --ns 5000 --out /data/raw/warmup-scan-n5000.json
    # One invocation per N: a full pass on the 7.74 M pool takes about a minute
    # and a half, and a command that prints nothing for five minutes is not a
    # measurement command.
    for n in 1000 2000 5000; do
        echo "== scan-main, N = $n: 1 k, 10 k, 100 k, 1 M, all; contiguous and dispersed; 3 passes"
        harness scan $INDEX --ns "$n" --out "/data/raw/scan-main-n$n.json"
        echo "== scan-all, N = $n: same points, second execution"
        harness scan $INDEX --ns "$n" --out "/data/raw/scan-all-n$n.json"
    done
    ;;
mes03-warm)
    INDEX="--data /data --index synth-large --vectors /data/queries.f32"
    echo "== warm-up: one complete pass of the query set, discarded"
    harness rescore $INDEX --ns 1000,2000,5000 --passes 1 --out /data/raw/warmup-rescore.json
    echo "== warm: engine, dot alone and keys alone, 3 passes, N = 1000, 2000, 5000"
    harness rescore $INDEX --dot-only --key-only --ns 1000,2000,5000 \
        --out /data/raw/rescore-warm.json
    echo "== warm, engine absent: keys alone, 3 passes"
    harness rescore $INDEX --skip-engine --key-only --ns 1000,2000,5000 \
        --out /data/raw/key-only.json
    ;;
mes03-cold)
    INDEX="--data /data --index synth-large --vectors /data/queries.f32"
    # One invocation per N and per line, so a run stays bounded in time and each
    # one opens its own container (ADR-0135, decision 7: a cold series runs in a
    # fresh container after a global purge attempt).
    for n in 1000 2000 5000; do
        purge_global
        echo "== cold, engine, N = $n, one pass (fresh container)"
        harness rescore $INDEX --cold --evict --ns "$n" --passes 1 \
            --out "/data/raw/rescore-cold-evict-n$n.json"
        purge_global
        echo "== cold, dot alone, N = $n, one pass (fresh container)"
        harness rescore $INDEX --cold --evict --skip-engine --dot-only \
            --ns "$n" --passes 1 --out "/data/raw/rescore-cold-evict-dot-n$n.json"
        purge_global
        echo "== cold, keys alone, N = $n, one pass (fresh container)"
        harness rescore $INDEX --cold --evict --skip-engine --key-only \
            --ns "$n" --passes 1 --out "/data/raw/rescore-cold-evict-keys-n$n.json"
    done
    ;;
mes03-limit)
    # Passage B: the shipped rescoring path, `--engine-limit` equal to
    # `candidates_per_branch` (100). One invocation per N for the warm series
    # (4 passes, the first discarded), one fresh container per N for the cold one.
    # The replicas of passage A are left out on purpose: the keys replica evicts
    # the vector pages and perturbs the engine series (declared in MES-03).
    INDEX="--data /data --index synth-large --vectors /data/queries.f32"
    echo "== warm-up: one complete pass, discarded, with the replicas kept as calibration"
    harness rescore $INDEX --ns 1000,2000,5000 --passes 1 --engine-limit 100 \
        --dot-only --key-only --out /data/raw/b-warmup-rescore-limit100.json
    for n in 1000 2000 5000; do
        echo "== warm, shipped path, N = $n, 4 passes (pass 0 discarded)"
        harness rescore $INDEX --ns "$n" --passes 4 --engine-limit 100 \
            --out "/data/raw/b-rescore-warm-only-n$n.json"
        purge_global
        echo "== cold partial, shipped path, N = $n, one pass (fresh container)"
        harness rescore $INDEX --ns "$n" --passes 1 --cold --evict --engine-limit 100 \
            --out "/data/raw/b-rescore-cold-n$n.json"
    done
    ;;
mes05)
    # Passage B: MES-05 in the container, twice per precision, one container each.
    # The outputs carry the run number the aggregator reads: int8 runs 1 and 2,
    # fp32 runs 3 and 4 (`aggregate-b.py`, `mes05`). The inputs are deposited by
    # `prepare.sh`, step 6 bis.
    run=0
    for precision in int8 int8 fp32 fp32; do
        run=$((run + 1))
        echo "== encode, $precision, run $run"
        docker run --rm -v "$VOLUME":/data --entrypoint measure_queries "$HARNESS" \
            latency --model /data/models/mdenseon \
            --queries /data/staging/query-texts.jsonl --precision "$precision" \
            --warmup 300 --count 1000 \
            --out "/data/raw/b-encode-$precision-$run.json"
    done
    ;;
mes08)
    # Passage B: the batch-size sweep, four interleaved rounds in two invocations
    # (a single three-round invocation would exceed the 30-minute bound), framed
    # by a B = 1 series before and after, then the linearity test. The framing
    # series before is the calibration pass the aggregator reads back
    # (`aggregate-b.py`, `mes08`), so the protocol of 2026-10-27 is complete.
    echo "== sweep, calibration series B = 1, one round"
    docker run --rm -v "$VOLUME":/data --entrypoint acceptance_sweep "$HARNESS" \
        sweep --model /data/models/mdenseon \
        --sample /data/staging/sample-daily-articles-1000.json \
        --sizes 1 --rounds 1 --out /data/raw/b-sweep-calibrate.json
    for round in a b; do
        echo "== sweep, interleaved sizes 1, 4, 8, two rounds ($round)"
        docker run --rm -v "$VOLUME":/data --entrypoint acceptance_sweep "$HARNESS" \
            sweep --model /data/models/mdenseon \
            --sample /data/staging/sample-daily-articles-1000.json \
            --sizes 1,4,8 --rounds 2 --out "/data/raw/b-sweep-$round.json"
    done
    echo "== sweep, framing series B = 1, three rounds"
    docker run --rm -v "$VOLUME":/data --entrypoint acceptance_sweep "$HARNESS" \
        sweep --model /data/models/mdenseon \
        --sample /data/staging/sample-daily-articles-1000.json \
        --sizes 1 --rounds 3 --out /data/raw/b-sweep-frame-after.json
    echo "== longest drawn passage, 1, 4 and 8 copies in one call, three rounds"
    docker run --rm -v "$VOLUME":/data --entrypoint acceptance_sweep "$HARNESS" \
        longest --model /data/models/mdenseon \
        --sample /data/staging/sample-daily-articles-1000.json \
        --copies 1,4,8 --rounds 3 --out /data/raw/b-longest.json
    ;;
routes)
    # Passage B: the two read routes, no MES number (ADR-0135, decision 6).
    # The server runs detached and is stopped and removed at the end.
    echo "== server, image $CRIBLE, volume $VOLUME"
    docker run -d --name crible-mes-p8-server -p 18080:8080 --stop-timeout 65 \
        -v "$VOLUME":/data \
        -v "$ROOT/data/mes-p8-conteneur/staging/crible.toml:/etc/crible/crible.toml:ro" \
        "$CRIBLE"
    sleep 120
    python3 tools/bench/mes-p8-conteneur/routes_client.py --base http://127.0.0.1:18080 \
        --index synth-large-9seg --vectors data/mes02/queries.f32 \
        --token-file data/mes-p8-conteneur/staging/token.txt \
        --out data/mes-p8-conteneur/raw/routes-conteneur-9seg.json \
        --docs-out data/mes-p8-conteneur/staging/docs.json --passes 3
    cat data/mes-p8-conteneur/staging/docs.json \
        | docker run --rm -i -v "$VOLUME":/data "$DEBIAN" sh -c \
            'cat > /data/staging/docs.json && chown 1000:1000 /data/staging/docs.json'
    docker run --rm -v "$VOLUME":/data --entrypoint server_costs "$HARNESS" \
        walk --directory /data/indexes/synth-large-9seg --rounds 100 \
        --out /data/raw/b-walk-9seg.json
    docker run --rm -v "$VOLUME":/data --entrypoint server_costs "$HARNESS" \
        document --records /data/indexes/synth-large-9seg/epochs/2/records.sqlite \
        --docs /data/staging/docs.json --rounds 3 \
        --out /data/raw/b-readpool-9seg.json
    echo "== stopping the server"
    docker stop crible-mes-p8-server
    docker rm crible-mes-p8-server
    ;;
mes09)
    INDEX="--data /data --index synth-large --vectors /data/queries.f32"
    for run in 1 2 3; do
        echo "== RSS states (a) and (b), run $run"
        harness rss $INDEX --passes 1 --out "/data/raw/rss-base-$run.json"
    done
    for run in 1 2; do
        echo "== RSS state (c), fp32 session, no pass, run $run"
        harness rss $INDEX --model /data/models/mdenseon --skip-pass \
            --out "/data/raw/rss-model-cold-$run.json"
    done
    ;;
collect)
    # Raw documents out of the volume, into the gitignored tree of the task.
    docker run --rm -v "$VOLUME":/data "$DEBIAN" tar cf - -C /data/raw . \
        | tar xf - -C "$RAW"
    ls -l "$RAW"
    ;;
*)
    echo "usage: $0 {env|mes02|mes03-warm|mes03-cold|mes03-limit|mes05|mes08|routes|mes09|collect}" >&2
    exit 2
    ;;
esac
