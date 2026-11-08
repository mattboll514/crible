#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Top-10 control of the Mac mini campaign (MESURER-SUR-MAC-MINI, ADR-0141,
# decision 2). Runs INSIDE a `crible:p10` container, one index per call:
#
#   sh /top10.sh <index>
#
# `/in/<index>/` holds one `<id>.json` vector per query and, when the query
# carries one, a `<id>.filter.json` filter. The answer of `crible search` goes
# to `/out/<id>.json`, unchanged. No text and no filter value leaves the
# container except through that filter, and the dump is compared locally.

set -eu

index=$1
dir=/in/$index
out=/out

for file in "$dir"/*.json; do
    case "$file" in
    *.filter.json) continue ;;
    esac
    id=$(basename "$file" .json)
    echo "search $index $id"
    if [ -f "$dir/$id.filter.json" ]; then
        crible search --data /data --index "$index" --mode dense \
            --vector-file "$file" --k 10 \
            --filter "$(cat "$dir/$id.filter.json")" >"$out/$id.json"
    else
        crible search --data /data --index "$index" --mode dense \
            --vector-file "$file" --k 10 >"$out/$id.json"
    fi
done
