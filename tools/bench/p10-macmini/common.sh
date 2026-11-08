#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Shared settings of the Mac mini campaign (MESURER-SUR-MAC-MINI, ADR-0141).
#
# Sourced by the other scripts of this directory: `common.sh` resolves the
# repository root, loads `.env.prod` without printing it, and names the host
# paths. No value of `.env.prod` is ever echoed (ADR-0032, point 1).

set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

set -a
. "$ROOT/.env.prod"
set +a

# The Mac mini runs non-interactive shells with a minimal PATH, which does not
# carry Docker Desktop's binary (`/usr/local/bin/docker`, a symlink into
# `/Applications/Docker.app`). Every remote command restores it explicitly.
SSH="ssh -o BatchMode=yes"
REMOTE_CMD='export PATH=/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin;'

IMAGE=crible:p10
DATA_VOLUME=crible-p10-data
MODELS_VOLUME=crible-p10-models
PORT=55400
CONTAINER=crible-p10

# The exchange directory lives on the external SSD, outside the Legifacile
# repository and outside the Crible data directory (ADR-0141, decision 1).
SSD_EXCHANGE=/Volumes/MattCorp-SSD/crible-exchange
REMOTE_SNAPSHOTS=$SSD_EXCHANGE/snapshots
LOCAL_SNAPSHOTS=$ROOT/data/p10-exchange/snapshots
EXCHANGE_CONFIG=$ROOT/data/p10-exchange/crible.toml

OUT=$ROOT/data/p10-macmini

# Largest index first: a copy that runs out of session time leaves the small
# indexes untouched rather than one half of the largest.
SNAPSHOT_ORDER="decisions-e2-g2 articles-e1-g1 jorf-e3-g3 titres-e1-g1 sarde-e1-g1 dole-e1-g1"

remote() {
    $SSH "$PROD_SSH" "$REMOTE_CMD $1"
}
