#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""HTTP client of the container route campaign (task MESURER-EN-CONTENEUR-MAC,
passage B, ADR-0135 decision 6).

It measures two read routes of a running Crible server and nothing else:

    GET /v1/indexes/{index}                        -- route with `sizes.disk_bytes`
    GET /v1/indexes/{index}/documents/{doc_id}    -- route that opens a `ReadPool`

Protocol (ADR-0135, decision 7): one complete pass of the query set is run and
discarded, then three passes are measured. Each measured pass runs the two
routes in separate loops, in the same order, so a route never pays the page
cache a previous route warmed for it.

The client is a named client, kept on purpose: Python 3.13 `http.client`, one
persistent `HTTPConnection` per pass, one call at a time, `perf_counter_ns`
around the call. No retry: a call that fails stops the campaign.

The document identifiers come from the top hit of the warm-up search pass, so
the documents route serves the documents a caller really asks for. The texts
and the identifiers never reach stdout: the script prints counts and
durations only (AGENTS.md, "Confidentialité").

    python3 tools/bench/mes-p8-conteneur/routes_client.py \
        --base http://127.0.0.1:18080 \
        --index synth-large-9seg \
        --vectors data/mes02/queries.f32 \
        --token-file /dev/stdin \
        --out data/mes-p8-conteneur/raw/routes-conteneur.json \
        --docs-out data/mes-p8-conteneur/staging/docs.json
"""

from __future__ import annotations

import argparse
import base64
import http.client
import json
import struct
import sys
import time
import urllib.parse

DIMENSIONS = 768


def read_vectors(path: str) -> list[bytes]:
    """Reads the little-endian fp32 vectors, one base64 string per row."""
    with open(path, "rb") as file:
        raw = file.read()
    row = DIMENSIONS * 4
    if len(raw) % row != 0:
        raise SystemExit(f"{path} is not a whole number of {DIMENSIONS}-d vectors")
    return [base64.b64encode(raw[offset : offset + row]) for offset in range(0, len(raw), row)]


def percentile(values: list[float], percent: float) -> float:
    ordered = sorted(values)
    if not ordered:
        return float("nan")
    position = (percent / 100.0) * (len(ordered) - 1)
    lower = int(position)
    upper = min(lower + 1, len(ordered) - 1)
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower)


def summary(values: list[float]) -> dict[str, float]:
    """Microseconds, every call kept: nothing is filtered or dropped."""
    return {
        "count": len(values),
        "min_us": min(values),
        "max_us": max(values),
        "mean_us": sum(values) / len(values),
        "p50_us": percentile(values, 50),
        "p95_us": percentile(values, 95),
        "p99_us": percentile(values, 99),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", required=True)
    parser.add_argument("--index", required=True)
    parser.add_argument("--vectors", required=True)
    parser.add_argument("--token-file", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--docs-out", required=True)
    parser.add_argument("--passes", type=int, default=3)
    parser.add_argument("--k", type=int, default=10)
    arguments = parser.parse_args()

    token = open(arguments.token_file).read().strip()
    parsed = urllib.parse.urlparse(arguments.base)
    vectors = read_vectors(arguments.vectors)
    index_path = f"/v1/indexes/{arguments.index}"

    def connect() -> http.client.HTTPConnection:
        return http.client.HTTPConnection(parsed.hostname, parsed.port, timeout=120)

    def get(connection: http.client.HTTPConnection, path: str) -> tuple[int, bytes]:
        """One GET, timed alone; the connection is kept."""
        connection.request("GET", path, headers={"Authorization": f"Bearer {token}"})
        response = connection.getresponse()
        body = response.read()
        return response.status, body

    # -- warm-up pass, discarded: search only, and the identifiers it returns.
    connection = connect()
    search_us: list[float] = []
    docs: list[str] = []
    for position, vector in enumerate(vectors):
        body = json.dumps({"mode": "dense", "vector": vector.decode(), "k": arguments.k})
        started = time.perf_counter_ns()
        connection.request(
            "POST",
            f"{index_path}/search",
            body=body,
            headers={
                "Authorization": f"Bearer {token}",
                "Content-Type": "application/json",
            },
        )
        response = connection.getresponse()
        payload = json.loads(response.read())
        search_us.append((time.perf_counter_ns() - started) / 1000.0)
        if response.status != 200:
            raise SystemExit(f"warm-up search {position} answered {response.status}")
        hits = payload.get("results") or payload.get("passages") or []
        if hits:
            docs.append(hits[0]["doc_id"])
    connection.close()
    print(
        "warm-up: {0} searches, {1} documents, p50 {2:.1f} ms (discarded)".format(
            len(search_us), len(docs), percentile(search_us, 50) / 1000.0
        )
    )
    if len(docs) < 300:
        raise SystemExit(f"only {len(docs)} documents: the route needs at least 300 calls")

    # -- measured passes.
    measured: dict[str, list[list[float]]] = {"index": [], "document": []}
    sizes: list[int] = []
    context_us: list[list[float]] = []
    for number in range(arguments.passes):
        connection = connect()
        # Route a: `GET /v1/indexes/{index}`, `sizes.disk_bytes` included.
        durations: list[float] = []
        for _ in range(300):
            started = time.perf_counter_ns()
            status, body = get(connection, index_path)
            durations.append((time.perf_counter_ns() - started) / 1000.0)
            if status != 200:
                raise SystemExit(f"index route answered {status}")
        measured["index"].append(durations)
        sizes.append(len(body))
        # Route b: `GET /v1/indexes/{index}/documents/{doc_id}`.
        durations = []
        for doc_id in docs[:300]:
            started = time.perf_counter_ns()
            status, _ = get(connection, f"{index_path}/documents/{urllib.parse.quote(doc_id)}")
            durations.append((time.perf_counter_ns() - started) / 1000.0)
            if status != 200:
                raise SystemExit(f"documents route answered {status}")
        measured["document"].append(durations)
        # Context only, not a target of the campaign: the search of the same pass.
        durations = []
        for vector in vectors:
            body = json.dumps({"mode": "dense", "vector": vector.decode(), "k": arguments.k})
            started = time.perf_counter_ns()
            connection.request(
                "POST",
                f"{index_path}/search",
                body=body,
                headers={
                    "Authorization": f"Bearer {token}",
                    "Content-Type": "application/json",
                },
            )
            response = connection.getresponse()
            response.read()
            durations.append((time.perf_counter_ns() - started) / 1000.0)
            if response.status != 200:
                raise SystemExit("search of the measured pass failed")
        context_us.append(durations)
        connection.close()
        print(
            "pass {0}: index p50 {1:.3f} ms, documents p50 {2:.3f} ms, search p50 {3:.1f} ms".format(
                number,
                summary(measured["index"][-1])["p50_us"] / 1000.0,
                summary(measured["document"][-1])["p50_us"] / 1000.0,
                summary(context_us[-1])["p50_us"] / 1000.0,
            )
        )

    document = {
        "base": arguments.base,
        "index": arguments.index,
        "passes": arguments.passes,
        "calls_per_pass": 300,
        "client": "python http.client, one persistent connection, perf_counter_ns",
        "routes": {
            "index": {
                "path": index_path,
                "passes": measured["index"],
                "summary": summary([value for pass_ in measured["index"] for value in pass_]),
                "body_bytes": sizes,
                "per_pass": [summary(pass_) for pass_ in measured["index"]],
            },
            "document": {
                "path": f"{index_path}/documents/{{doc_id}}",
                "passes": measured["document"],
                "summary": summary([value for pass_ in measured["document"] for value in pass_]),
                "per_pass": [summary(pass_) for pass_ in measured["document"]],
            },
            "search_context": {
                "path": f"{index_path}/search",
                "passes": context_us,
                "summary": summary([value for pass_ in context_us for value in pass_]),
                "note": "context only: not a target of the campaign",
            },
        },
        "warmup_search_us": search_us,
    }
    with open(arguments.out, "w") as file:
        json.dump(document, file, indent=2)
        file.write("\n")
    with open(arguments.docs_out, "w") as file:
        json.dump(docs[:300], file)
        file.write("\n")
    index_summary = document["routes"]["index"]["summary"]
    document_summary = document["routes"]["document"]["summary"]
    print(
        "index route: p50 {0:.3f} ms, p95 {1:.3f} ms, p99 {2:.3f} ms; body {3} B".format(
            index_summary["p50_us"] / 1000.0,
            index_summary["p95_us"] / 1000.0,
            index_summary["p99_us"] / 1000.0,
            sizes[-1],
        )
    )
    print(
        "documents route: p50 {0:.3f} ms, p95 {1:.3f} ms, p99 {2:.3f} ms".format(
            document_summary["p50_us"] / 1000.0,
            document_summary["p95_us"] / 1000.0,
            document_summary["p99_us"] / 1000.0,
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
