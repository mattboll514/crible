#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Campaign driver for the before/after latency measurement of the dense
reader cache (task REUTILISER-LECTEURS-DENSES, phase P7, deliverable 2).

References: ADR-0126, decision 8, and
`docs/mesures/lecteurs-denses-latence-2026-11-01.md`.

The harness is a recipe, not an authority: every command is also spelled out in
the report. It drives the *server*, which is the only place where the cache of a
generation is visible, and it changes nothing in `crates/*/src/`.

Three subcommands:

    latence.py queries --out data/mes-lecteurs-denses/latence/queries.jsonl \
        --count 400 --dims 768 --seed 20261101
    latence.py startup --binary B --config C --port P --out O.json [--repeat 3]
    latence.py campaign --binary B --config C --port P --queries Q \
        --out O.json --requests 300 --warmup 20 [--k 10] [--timeout-ms 30000]

Every request is sent sequentially, in the foreground, on one connection. The
server is started by this script and stopped by SIGTERM. No request is retried:
a non-200 answer is recorded as an error and the campaign keeps going.
"""

import argparse
import base64
import http.client
import json
import math
import os
import random
import signal
import struct
import subprocess
import sys
import time
import urllib.error
import urllib.request


def generate_queries(out, count, dims, seed):
    rng = random.Random(seed)
    lines = []
    for _ in range(count):
        values = [rng.gauss(0.0, 1.0) for _ in range(dims)]
        norm = math.sqrt(sum(v * v for v in values)) or 1.0
        values = [v / norm for v in values]
        raw = struct.pack("<%df" % dims, *values)
        lines.append(json.dumps({"vector": base64.b64encode(raw).decode("ascii")}))
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    with open(out, "w", encoding="utf-8") as handle:
        handle.write("\n".join(lines) + "\n")
    print("wrote %d query vectors of %d dims to %s" % (count, dims, out))


def start_server(binary, config, log_path):
    log = open(log_path, "ab")
    log.write(b"\n=== %s\n" % time.strftime("%Y-%m-%dT%H:%M:%S").encode())
    log.flush()
    started = time.perf_counter()
    proc = subprocess.Popen(
        [binary, "serve", "--config", config],
        stdout=log,
        stderr=log,
    )
    return proc, started, log


def wait_ready(port, proc, started, deadline_s=300.0):
    url = "http://127.0.0.1:%d/ready" % port
    deadline = time.monotonic() + deadline_s
    last = None
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            raise RuntimeError("server exited with %s before /ready" % proc.returncode)
        try:
            with urllib.request.urlopen(url, timeout=2) as response:
                if response.status == 200:
                    return (time.perf_counter() - started) * 1e3
        except (urllib.error.URLError, OSError) as error:
            last = error
        time.sleep(0.01)
    raise RuntimeError("server not ready after %.0f s (%s)" % (deadline_s, last))


def stop_server(proc, log):
    proc.send_signal(signal.SIGTERM)
    try:
        code = proc.wait(timeout=60)
    except subprocess.TimeoutExpired:
        proc.kill()
        code = proc.wait(timeout=10)
    log.close()
    return code


def post(port, key, body, timeout_s):
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=timeout_s)
    started = time.perf_counter()
    try:
        connection.request(
            "POST",
            "/v1/indexes/synth-large/search",
            body=json.dumps(body).encode("utf-8"),
            headers={
                "Authorization": "Bearer " + key,
                "Content-Type": "application/json",
            },
        )
        response = connection.getresponse()
        payload = response.read()
        wall = (time.perf_counter() - started) * 1e3
        return response.status, payload, wall
    finally:
        connection.close()


def percentile(values, fraction):
    """Nearest-rank percentile (ADR-0117, decision 4)."""
    if not values:
        return float("nan")
    ordered = sorted(values)
    rank = max(1, math.ceil(fraction * len(ordered)))
    return ordered[rank - 1]


def summary(values):
    if not values:
        return {}
    return {
        "n": len(values),
        "min": min(values),
        "p50": percentile(values, 0.50),
        "p90": percentile(values, 0.90),
        "p95": percentile(values, 0.95),
        "p99": percentile(values, 0.99),
        "max": max(values),
        "mean": sum(values) / len(values),
    }


def read_key(config):
    with open(os.path.join(os.path.dirname(os.path.abspath(config)), "key.txt"), encoding="utf-8") as handle:
        for line in handle:
            if line.startswith("key: "):
                return line[5:].strip()
    raise RuntimeError("no key in key.txt")


def startup(args):
    runs = []
    log_path = args.out + ".log"
    for index in range(args.repeat):
        proc, started, log = start_server(args.binary, args.config, log_path)
        try:
            ready_ms = wait_ready(args.port, proc, started)
        finally:
            code = stop_server(proc, log)
        runs.append({"run": index + 1, "ready_ms": ready_ms, "exit": code})
        print("startup %d/%d: %.1f ms to /ready (exit %s)" % (index + 1, args.repeat, ready_ms, code))
        time.sleep(0.5)
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump({"kind": "startup", "runs": runs}, handle, indent=2)
    print("startup: wrote %s" % args.out)


def campaign(args):
    key = read_key(args.config)
    with open(args.queries, encoding="utf-8") as handle:
        queries = [json.loads(line) for line in handle if line.strip()]
    needed = args.offset + args.warmup + args.requests
    if len(queries) < needed:
        raise SystemExit("queries file holds %d vectors, %d needed" % (len(queries), needed))

    log_path = args.out + ".server.log"
    proc, started, log = start_server(args.binary, args.config, log_path)
    samples = []
    started_at = time.time()
    try:
        ready_ms = wait_ready(args.port, proc, started)
        print("server ready in %.1f ms" % ready_ms, flush=True)
        strategy = None
        for index in range(args.offset, args.offset + args.warmup + args.requests):
            phase = "warmup" if index < args.offset + args.warmup else "measure"
            body = {
                "vector": queries[index]["vector"],
                "mode": "dense",
                "k": args.k,
                "debug": True,
                "timeout_ms": args.timeout_ms,
            }
            if args.filter:
                body["filter"] = json.loads(args.filter)
            status, payload, wall_ms = post(args.port, key, body, args.timeout_ms / 1000.0 + 30.0)
            record = {"index": index, "phase": phase, "status": status, "wall_ms": wall_ms}
            if status == 200:
                answer = json.loads(payload)
                timings = answer.get("debug", {}).get("timings_ms", {})
                plan = answer.get("debug", {}).get("plan", {})
                record.update(
                    {
                        "took_ms": answer.get("took_ms"),
                        "timings_ms": timings,
                        "dense_strategy": plan.get("dense_strategy"),
                        "rescore_candidates": plan.get("rescore_candidates"),
                        "pool_size": answer.get("pool_size"),
                        "generation": answer.get("generation"),
                        "results": len(answer.get("results", [])),
                    }
                )
                strategy = record["dense_strategy"]
            else:
                record["body"] = payload[:400].decode("utf-8", "replace")
            if phase == "measure":
                samples.append(record)
            print(
                "%s %d/%d: status %d, wall %.1f ms, took %s ms, strategy %s"
                % (phase, index + 1, needed, status, wall_ms, record.get("took_ms"), strategy),
                flush=True,
            )
    finally:
        code = stop_server(proc, log)
    print("server stopped, exit %s" % code, flush=True)
    report = build_report(samples, {
        "binary": args.binary,
        "config": args.config,
        "queries": args.queries,
        "offset": args.offset,
        "requests": args.requests,
        "warmup": args.warmup,
        "k": args.k,
        "timeout_ms": args.timeout_ms,
        "filter": json.loads(args.filter) if args.filter else None,
        "ready_ms": ready_ms,
        "server_exit": code,
        "elapsed_s": time.time() - started_at,
    })
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump(report, handle, indent=2)
    print("campaign: wrote %s" % args.out)
    print_report(report)


def build_report(samples, meta):
    ok = [s for s in samples if s["status"] == 200]
    errors = [s for s in samples if s["status"] != 200]
    took = [s["took_ms"] for s in ok if s.get("took_ms") is not None]
    stages = ["validate", "encode", "filter", "dense", "lexical", "fusion", "group", "fetch"]
    stage_series = {name: [s["timings_ms"][name] for s in ok if name in s["timings_ms"]] for name in stages}
    totals = [s["timings_ms"]["total"] for s in ok if "total" in s["timings_ms"]]
    critical = []
    for s in ok:
        t = s["timings_ms"]
        if not all(name in t for name in ("encode", "filter", "dense", "lexical", "fusion", "group", "fetch")):
            continue
        critical.append(
            t["encode"] + t["filter"] + max(t["dense"], t["lexical"]) + t["fusion"] + t["group"] + t["fetch"]
        )
    gaps = []
    for s in ok:
        t = s["timings_ms"]
        if "total" not in t:
            continue
        value = t["encode"] + t["filter"] + max(t["dense"], t["lexical"]) + t["fusion"] + t["group"] + t["fetch"]
        if t["total"]:
            gaps.append((t["total"] - value) / t["total"])

    report = {
        "kind": "campaign",
        "meta": meta,
        "dense_strategy": ok[0]["dense_strategy"] if ok else None,
        "rescore_candidates": ok[0].get("rescore_candidates") if ok else None,
        "pool_size": ok[0].get("pool_size") if ok else None,
        "generation": ok[0].get("generation") if ok else None,
        "errors": len(errors),
        "summary": {
            "took_ms": summary(took),
            "total_ms": summary(totals),
            "wall_ms": summary([s["wall_ms"] for s in ok]),
            "fetch_ms": summary(stage_series["fetch"]),
            "dense_ms": summary(stage_series["dense"]),
            "filter_ms": summary(stage_series["filter"]),
            "encode_ms": summary(stage_series["encode"]),
            "fusion_ms": summary(stage_series["fusion"]),
            "group_ms": summary(stage_series["group"]),
            "critical_path_ms": summary(critical),
            "gap_ratio": summary(gaps),
            "max_gap_ratio": max(gaps) if gaps else None,
        },
        "samples": samples,
    }
    return report


def print_report(report):
    print("campaign: %d ok, %d errors" % (report["summary"]["took_ms"].get("n", 0), report["errors"]))
    for name in ("took_ms", "total_ms", "fetch_ms", "dense_ms", "critical_path_ms"):
        s = report["summary"][name]
        print(
            "%s: p50 %.3f, p95 %.3f, p99 %.3f, max %.3f"
            % (name, s["p50"], s["p95"], s["p99"], s["max"])
        )
    print("largest critical-path gap: %.4f" % (report["summary"]["max_gap_ratio"] or 0.0))


def merge(args):
    reports = []
    for path in args.inputs:
        with open(path, encoding="utf-8") as handle:
            reports.append(json.load(handle))
    samples = [s for report in reports for s in report["samples"]]
    meta = reports[0]["meta"] if reports else {}
    meta["merged_from"] = args.inputs
    report = build_report(samples, meta)
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump(report, handle, indent=2)
    print("merge: wrote %s" % args.out)
    print_report(report)


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)

    q = sub.add_parser("queries")
    q.add_argument("--out", required=True)
    q.add_argument("--count", type=int, default=400)
    q.add_argument("--dims", type=int, default=768)
    q.add_argument("--seed", type=int, default=20261101)

    for name in ("startup", "campaign"):
        p = sub.add_parser(name)
        p.add_argument("--binary", required=True)
        p.add_argument("--config", required=True)
        p.add_argument("--port", type=int, default=18080)
        p.add_argument("--out", required=True)
        if name == "startup":
            p.add_argument("--repeat", type=int, default=3)
        else:
            p.add_argument("--queries", required=True)
            p.add_argument("--offset", type=int, default=0)
            p.add_argument("--requests", type=int, required=True)
            p.add_argument("--warmup", type=int, required=True)
            p.add_argument("--k", type=int, default=10)
            p.add_argument("--timeout-ms", type=int, default=30000)
            p.add_argument("--filter", default=None)

    m = sub.add_parser("merge")
    m.add_argument("--inputs", nargs="+", required=True)
    m.add_argument("--out", required=True)

    args = parser.parse_args()
    if args.command == "queries":
        generate_queries(args.out, args.count, args.dims, args.seed)
    elif args.command == "startup":
        startup(args)
    elif args.command == "merge":
        merge(args)
    else:
        campaign(args)


if __name__ == "__main__":
    main()
