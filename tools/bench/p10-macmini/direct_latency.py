#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Direct API latency of the served instance with a cold query cache.

Pass C of MESURER-SUR-MAC-MINI (P10, ADR-0141). The reference game is sent one
query at a time to `POST /v1/indexes/<index>/search`, with `debug: true`, so the
answer carries `debug.timings_ms.total` (equal to `took_ms`) and `encode`. The
server is restarted before the run, so its query cache is empty; one warm-up
request loads the ONNX session and is not counted. A repeated query text hits
the cache: the unique series excludes those hits, and the raw document keeps
both counts.

The script runs on the Mac mini, with the standard library only, and reads the
games from the exchange folder. It writes no query text: the JSON keeps the
`id`, the index and the timings.

    python3 direct_latency.py --base http://127.0.0.1:55400 \
        --token-file /path/token.txt --inputs /path/inputs --out /path/out.json
"""

import argparse
import json
import math
import os
import sys
import time
import urllib.request


def nearest_rank(values, percentile):
    """Percentile at the nearest rank (ADR-0077, decision 4)."""
    if not values:
        raise ValueError("no value")
    ordered = sorted(values)
    rank = max(1, math.ceil(percentile / 100.0 * len(ordered)))
    return ordered[rank - 1]


def summarize(values):
    """p50, p95, p99, min, max and mean, in the unit of the input."""
    return {
        "n": len(values),
        "p50": nearest_rank(values, 50),
        "p95": nearest_rank(values, 95),
        "p99": nearest_rank(values, 99),
        "min": min(values),
        "max": max(values),
        "mean": sum(values) / len(values),
    }


def read_game(path):
    """Rows of one JSONL game, refusing a row without `id` or `query`."""
    rows = []
    with open(path, "r", encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if not line:
                continue
            row = json.loads(line)
            if "id" not in row or "query" not in row:
                raise ValueError("%s: a row has no id or query" % path)
            rows.append(row)
    return rows


def search(base, token, index, query, filter_value=None, timeout_ms=2000):
    """One POST search, returns `(status, body)`; HTTP errors give their body."""
    body = {"query": query, "k": 10, "mode": "dense", "debug": True}
    if filter_value is not None:
        body["filter"] = filter_value
    if timeout_ms is not None:
        body["timeout_ms"] = timeout_ms
    request = urllib.request.Request(
        "%s/v1/indexes/%s/search" % (base.rstrip("/"), index),
        data=json.dumps(body).encode("utf-8"),
        headers={
            "Content-Type": "application/json",
            "Authorization": "Bearer %s" % token,
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=90) as response:
            return response.status, json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as error:
        return error.code, json.loads(error.read().decode("utf-8"))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", required=True)
    parser.add_argument("--token-file", required=True)
    parser.add_argument("--inputs", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--indexes", default="articles,decisions,dole,jorf,sarde,titres")
    parser.add_argument("--game-suffix", default="-dense-texte-macmini.jsonl")
    arguments = parser.parse_args()

    with open(arguments.token_file, "r", encoding="utf-8") as handle:
        token = handle.read().strip()
    if not token:
        raise SystemExit("empty token file")

    indexes = [name for name in arguments.indexes.split(",") if name]

    # Load the ONNX session and the tokenizer without counting the request. The
    # encoder is built on the first text query of the process (ADR-0125).
    started = time.monotonic()
    status, body = search(
        arguments.base, token, indexes[0], "warmup", None, timeout_ms=30000
    )
    warmup_ms = (time.monotonic() - started) * 1000.0
    if status != 200:
        raise SystemExit("warm-up failed with %s: %s" % (status, body))

    samples = []
    failures = []
    seen_texts = set()
    for index in indexes:
        path = os.path.join(arguments.inputs, index + arguments.game_suffix)
        for row in read_game(path):
            status, body = search(
                arguments.base, token, index, row["query"], row.get("filter")
            )
            # The query cache is keyed by text, so a text already sent is a
            # cache hit and is not encoded again. Mark it, never store the text.
            first_occurrence = row["query"] not in seen_texts
            seen_texts.add(row["query"])
            sample = {
                "id": row["id"],
                "index": index,
                "first_occurrence": first_occurrence,
                "status": status,
            }
            if status == 200 and isinstance(body.get("debug"), dict):
                timings = body["debug"].get("timings_ms") or {}
                for step in ("total", "encode", "filter", "dense", "fetch"):
                    if step in timings:
                        sample[step] = timings[step]
            else:
                failures.append({"id": row["id"], "index": index, "status": status})
            samples.append(sample)

    encoded = [s["encode"] for s in samples if "encode" in s]
    totals = [s["total"] for s in samples if "total" in s]
    unique = [s for s in samples if s.get("first_occurrence")]
    unique_encoded = [s["encode"] for s in unique if "encode" in s]
    unique_totals = [s["total"] for s in unique if "total" in s]
    report = {
        "kind": "direct-latency",
        "method": "one POST search per query, debug.timings_ms, query cache empty",
        "base": arguments.base,
        "queries": len(samples),
        "unique_queries": len(unique),
        "cache_hits": len(samples) - len(unique),
        "failures": failures,
        "warmup_ms": warmup_ms,
        "summary_ms": {
            "total": summarize(totals) if totals else None,
            "encode": summarize(encoded) if encoded else None,
        },
        "summary_ms_unique": {
            "total": summarize(unique_totals) if unique_totals else None,
            "encode": summarize(unique_encoded) if unique_encoded else None,
        },
        "samples": samples,
    }
    with open(arguments.out, "w", encoding="utf-8") as handle:
        json.dump(report, handle, indent=2, sort_keys=True)
        handle.write("\n")
    print(json.dumps(report["summary_ms"], sort_keys=True))


if __name__ == "__main__":
    main()
