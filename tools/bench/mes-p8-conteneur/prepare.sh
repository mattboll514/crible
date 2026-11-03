#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Preparation of the container campaign of `MESURER-EN-CONTENEUR-MAC` (P8,
# ADR-0135): the image, the named volume, the two measurement copies and the
# query set.
#
# Run from the repository root, once. It is idempotent: it refuses to overwrite
# what it already built, and never touches a volume, an image or a directory it
# did not create.
#
#   sh tools/bench/mes-p8-conteneur/prepare.sh
#
# What it creates:
#   - image `crible:dev`, rebuilt from the current tree by `docker/Dockerfile`;
#   - image `crible-mes-p8-harness:dev`, the measurement harness grafted on it;
#   - named volume `crible-mes-p8-data`, mounted on `/data` (ADR-0135, decision
#     3: a named volume only, never a host mount);
#   - `/data/indexes/synth-large`, the measurement copy: `exact_threshold = 0`
#     and `scan = "binary"`, one segment, 7 740 000 live passages;
#   - `/data/indexes/synth-large-9seg`, a clone compacted to segments of at most
#     1 M passages (8 segments for 7.74 M live passages, the `≈ 9` of decision 2);
#   - `/data/queries.f32`, the 300 query vectors of ADR-0077, same SHA-256 as the
#     native campaigns;
#   - `/data/models/mdenseon`, for the RSS state (c).
#
# Space: about 34 GB in the volume, on top of the 17 GB read on the host.

set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$ROOT"

VOLUME=crible-mes-p8-data
HARNESS_IMAGE=crible-mes-p8-harness:dev
SOURCE=data/synth-large/indexes/synth-large
STAGING=data/mes-p8-conteneur/staging
DEBIAN=debian:bookworm-slim

# 1. Space, before any copy (ADR-0135, decision 3, and the task: 40 GiB free on
#    the host, and the VM disk must hold the copies).
free_host=$(df -g "$ROOT" | awk 'NR==2 {print $4}')
echo "host: $free_host GiB free under $ROOT"
if [ "$free_host" -lt 40 ]; then
    echo "the host has $free_host GiB free, under the 40 GiB the task requires" >&2
    exit 1
fi
docker system df

# 2. Images.
docker build -f docker/Dockerfile -t crible:dev .
docker build -f tools/bench/mes-p8-conteneur/Dockerfile.harness -t "$HARNESS_IMAGE" .

# 3. The named volume, empty.
if docker volume inspect "$VOLUME" >/dev/null 2>&1; then
    echo "volume $VOLUME already exists, left untouched" >&2
else
    docker volume create "$VOLUME"
fi
docker run --rm -v "$VOLUME":/data "$DEBIAN" sh -c '
    mkdir -p /data/indexes/synth-large/epochs/1/segments/seg-large-0001 \
             /data/indexes/synth-large/epochs/1/manifests \
             /data/models/mdenseon /data/raw /data/staging
    chown -R 1000:1000 /data
    df -h /data'

# 4. The measurement configuration of the two copies (ADR-0077, decision 1:
#    BINARY on every point).
mkdir -p "$STAGING"
python3 - "$SOURCE/index.json" "$STAGING" <<'PY'
import json
import pathlib
import sys

source = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
for name in ("synth-large", "synth-large-9seg"):
    config = json.loads(source.read_text(encoding="utf-8"))
    config["name"] = name
    config["search"]["exact_threshold"] = 0
    config["search"]["scan"] = "binary"
    (out / f"{name}.index.json").write_text(
        json.dumps(config, indent=2) + "\n", encoding="utf-8"
    )
    print(f"{name}: {config['search']}")
PY

# 5. The one-segment copy, file by file, so a long transfer still prints its
#    progress (`dd status=progress`).
copy_into_volume() {
    source=$1
    destination=$2
    cat "$source" | docker run --rm -i -v "$VOLUME":/data "$DEBIAN" \
        dd of="$destination" status=progress
}
copy_into_volume "$SOURCE/CURRENT_EPOCH" /data/indexes/synth-large/CURRENT_EPOCH
copy_into_volume "$STAGING/synth-large.index.json" /data/indexes/synth-large/index.json
for manifest in 0 1 2; do
    copy_into_volume "$SOURCE/epochs/1/manifests/$manifest.json" \
        "/data/indexes/synth-large/epochs/1/manifests/$manifest.json"
done
copy_into_volume "$SOURCE/epochs/1/segments/seg-large-0001/segment.json" \
    /data/indexes/synth-large/epochs/1/segments/seg-large-0001/segment.json
copy_into_volume "$SOURCE/epochs/1/segments/seg-large-0001/codes.bin" \
    /data/indexes/synth-large/epochs/1/segments/seg-large-0001/codes.bin
copy_into_volume "$SOURCE/epochs/1/records.sqlite" \
    /data/indexes/synth-large/epochs/1/records.sqlite
copy_into_volume "$SOURCE/epochs/1/segments/seg-large-0001/vectors.f16" \
    /data/indexes/synth-large/epochs/1/segments/seg-large-0001/vectors.f16

# 6. The query vectors of ADR-0077 and the fp32 model of the RSS state (c).
(cd tools && uv run --no-sync python bench/mes02/mes02_queries.py)
copy_into_volume data/mes02/queries.f32 /data/queries.f32
for file in model.json model.onnx model.int8.onnx tokenizer.json parity.json; do
    copy_into_volume "data/models/mdenseon/$file" "/data/models/mdenseon/$file"
done

# 6 bis. The staging inputs of the passage B harnesses: the 300 query texts of
#      MES-05 (they never reach a log or a document) and the 200 passages of the
#      acceptance re-encoding, MES-08.
copy_into_volume data/mes04/query-texts.jsonl /data/staging/query-texts.jsonl
copy_into_volume data/accelere-recette/sample-daily-articles-1000.json \
    /data/staging/sample-daily-articles-1000.json
docker run --rm -v "$VOLUME":/data "$DEBIAN" chown -R 1000:1000 /data

# 7. The nine-segment copy: a clone of the measurement copy, compacted. The
#    compaction writes segments of at most 1 M passages (docs/02:202), so 7.74 M
#    live passages give 8 segments.
if docker run --rm -v "$VOLUME":/data "$DEBIAN" test -d /data/indexes/synth-large-9seg; then
    echo "index synth-large-9seg already exists, left untouched" >&2
else
    docker run --rm -v "$VOLUME":/data "$DEBIAN" sh -c '
        cp -av /data/indexes/synth-large /data/indexes/synth-large-9seg'
    copy_into_volume "$STAGING/synth-large-9seg.index.json" \
        /data/indexes/synth-large-9seg/index.json
    docker run --rm -v "$VOLUME":/data crible:dev \
        compact --data /data --index synth-large-9seg
fi

# 8. Controls: both copies open, and the volume inventory.
for index in synth-large synth-large-9seg; do
    docker run --rm -v "$VOLUME":/data crible:dev inspect --data /data --index "$index"
done
docker run --rm -v "$VOLUME":/data "$DEBIAN" sh -c 'du -sh /data/indexes/* /data && df -h /data'
