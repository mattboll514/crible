#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""MES-09 at the scale of Legifacile, in the container: the three states of ADR-0077.

The measurement is taken on the server process, not in a separate harness: the
question is what an *instance* owns when it serves. Each series starts from a
freshly started server, because the import campaign showed that the commit
delta stays in anonymous memory after a job and is only released at restart.

    python3 tools/bench/p10-import/mes09.py series --label instance --target decisions
    python3 tools/bench/p10-import/mes09.py move-out --keep decisions
    ... start the server with one index, then ...
    python3 tools/bench/p10-import/mes09.py series --label decisions --target decisions
    python3 tools/bench/p10-import/mes09.py move-back

States, as ADR-0077, decision 5:

    (a) index opened, snapshot loaded, no request;
    (b) after a pass of 300 requests with a provided vector, no ONNX session;
    (c) after one encoded query, which loads the fp32 session of the model.

`memory.current` and `memory.stat` of the cgroup are published with the RSS, as
ADR-0135, decision 7, asks.
"""

import argparse
import base64
import json
import pathlib
import random
import struct
import subprocess
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone

ROOT = pathlib.Path(__file__).resolve().parents[3]
OUT = ROOT / "data" / "p10-import"
BASE = "http://127.0.0.1:18090"
CONTAINER = "crible-p10-server-a2"
STAGING = "/data/staging-mes09"
PASS_REQUESTS = 300
DIMENSIONS = 768

READ_SCRIPT = (
    'for k in VmSize VmRSS RssAnon RssFile RssShmem VmHWM; do '
    'awk -v k=$k \'$1 == k":" {print k" "$2}\' /proc/1/status; done; '
    'echo memory_current $(cat /sys/fs/cgroup/memory.current); '
    'echo memory_max $(cat /sys/fs/cgroup/memory.max); '
    'awk \'/^(anon|file|kernel|slab|pagetables|inactive_file|active_file) /{print "stat_"$1" "$2}\' '
    '/sys/fs/cgroup/memory.stat'
)


def now():
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def token():
    return (OUT / "token.txt").read_text(encoding="utf-8").strip()


def read_memory():
    done = subprocess.run(
        ["docker", "exec", CONTAINER, "sh", "-c", READ_SCRIPT],
        capture_output=True, text=True, timeout=60, check=False,
    )
    if done.returncode != 0:
        raise RuntimeError(f"memory read failed: {done.stderr.strip()}")
    values = {}
    for line in done.stdout.splitlines():
        key, _, value = line.partition(" ")
        if not value:
            continue
        try:
            values[key] = int(value)
        except ValueError:
            values[key] = value
    mib = 1024 * 1024
    gib = 1024 ** 3
    return {
        "at": now(),
        "raw": values,
        "rss_mib": round(values.get("VmRSS", 0) / 1024, 1),
        "anon_mib": round(values.get("RssAnon", 0) / 1024, 1),
        "file_mib": round(values.get("RssFile", 0) / 1024, 1),
        "shmem_mib": round(values.get("RssShmem", 0) / 1024, 1),
        "vmsize_gib": round(values.get("VmSize", 0) / (1024 ** 2), 2),
        "hwm_mib": round(values.get("VmHWM", 0) / 1024, 1),
        "memory_current_gib": round(values.get("memory_current", 0) / gib, 2),
        "anon_gib": round(values.get("stat_anon", 0) / gib, 2),
        "file_gib": round(values.get("stat_file", 0) / gib, 2),
        "memory_max": values.get("memory_max"),
    }


def call(method, path, body=None, timeout=120.0):
    data = json.dumps(body).encode("utf-8") if body is not None else None
    headers = {"Authorization": f"Bearer {token()}"}
    if data is not None:
        headers["Content-Type"] = "application/json"
    request = urllib.request.Request(BASE + path, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=timeout) as answer:
            return answer.status, json.loads(answer.read())
    except urllib.error.HTTPError as error:
        return error.code, json.loads(error.read())


def unit_vector(seed=20261106):
    generator = random.Random(seed)
    values = [generator.gauss(0.0, 1.0) for _ in range(DIMENSIONS)]
    norm = sum(v * v for v in values) ** 0.5
    packed = struct.pack(f"<{DIMENSIONS}f", *(v / norm for v in values))
    return base64.b64encode(packed).decode("ascii")


def wait_ready(limit=900):
    started = time.monotonic()
    while time.monotonic() - started < limit:
        try:
            with urllib.request.urlopen(BASE + "/ready", timeout=5) as answer:
                if answer.status == 200:
                    return round(time.monotonic() - started, 1)
        except (urllib.error.URLError, urllib.error.HTTPError, OSError):
            pass
        time.sleep(2.0)
    raise RuntimeError("the server did not become ready")


def command_series(args):
    directory = OUT / "mes09"
    directory.mkdir(parents=True, exist_ok=True)
    vector = unit_vector()
    result = {
        "label": args.label,
        "target": args.target,
        "container": CONTAINER,
        "started_at": now(),
        "pass_requests": PASS_REQUESTS,
        "states": {},
    }
    result["ready_s"] = wait_ready()

    open_state = read_memory()
    status, _ = call("GET", f"/v1/indexes/{args.target}")
    result["http_index"] = status
    result["states"]["a_open"] = open_state

    latencies = []
    for _ in range(PASS_REQUESTS):
        started = time.perf_counter()
        status, answer = call("POST", f"/v1/indexes/{args.target}/search", {
            "vector": vector, "mode": "dense", "k": 10,
        })
        latencies.append((time.perf_counter() - started) * 1000.0)
        if status != 200:
            result["pass_error"] = {"http_status": status, "response": answer}
            break
    latencies.sort()
    result["pass"] = {
        "requests": len(latencies),
        "p50_ms": round(latencies[len(latencies) // 2], 2) if latencies else None,
        "max_ms": round(latencies[-1], 2) if latencies else None,
    }
    result["states"]["b_pass"] = read_memory()

    status, answer = call("POST", f"/v1/indexes/{args.target}/search", {
        "query": "responsabilité contractuelle", "mode": "dense", "k": 10,
    })
    result["encode"] = {"http_status": status, "took_ms": round(answer.get("took_ms", 0), 1)}
    result["states"]["c_model"] = read_memory()

    path = directory / f"{args.label}-{result['started_at'].replace(':', '')}.json"
    path.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    for name, state in result["states"].items():
        print(f"{args.label} {name}: rss {state['rss_mib']} MiB "
              f"anon {state['anon_mib']} MiB file {state['file_mib']} MiB "
              f"cgroup {state['memory_current_gib']} GiB")
    print(f"{args.label}: pass p50 {result['pass']['p50_ms']} ms, encode HTTP "
          f"{result['encode']['http_status']} in {result['encode']['took_ms']} ms")
    print(f"written {path}")
    return 0


def _indexes():
    done = subprocess.run(
        ["docker", "run", "--rm", "-v", "crible-p10-data:/data", "debian:bookworm-slim",
         "sh", "-c", f"ls /data/indexes; echo ---; ls {STAGING} 2>/dev/null || true"],
        capture_output=True, text=True, timeout=120, check=False,
    )
    current, _, staging = done.stdout.partition("---")
    return current.split(), staging.split(), done.returncode


def command_move_out(args):
    current, staging, code = _indexes()
    if code != 0:
        print("cannot list /data/indexes", file=sys.stderr)
        return 1
    if staging:
        print(f"{STAGING} is not empty: {staging}", file=sys.stderr)
        return 1
    others = [name for name in current if name != args.keep]
    script = f"mkdir -p {STAGING} && " + " && ".join(
        f"mv /data/indexes/{name} {STAGING}/{name}" for name in others
    )
    done = subprocess.run(
        ["docker", "run", "--rm", "-v", "crible-p10-data:/data", "debian:bookworm-slim",
         "sh", "-c", script],
        capture_output=True, text=True, timeout=300, check=False,
    )
    print(done.stdout, done.stderr, file=sys.stderr if done.returncode else sys.stdout)
    print(f"moved out: {' '.join(others)}; kept {args.keep}")
    return done.returncode


def command_move_back(_args):
    _current, staging, code = _indexes()
    if code != 0:
        print("cannot list the staging directory", file=sys.stderr)
        return 1
    if not staging:
        print("nothing to move back")
        return 0
    script = " && ".join(f"mv {STAGING}/{name} /data/indexes/{name}" for name in staging) + \
             f" && rmdir {STAGING}"
    done = subprocess.run(
        ["docker", "run", "--rm", "-v", "crible-p10-data:/data", "debian:bookworm-slim",
         "sh", "-c", script],
        capture_output=True, text=True, timeout=300, check=False,
    )
    print(done.stdout, done.stderr, file=sys.stderr if done.returncode else sys.stdout)
    print(f"moved back: {' '.join(staging)}")
    return done.returncode


def command_list(_args):
    current, staging, _code = _indexes()
    print("indexes:", " ".join(current))
    print("staging:", " ".join(staging) or "(empty)")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(required=True)
    series = sub.add_parser("series")
    series.add_argument("--label", required=True)
    series.add_argument("--target", required=True)
    series.set_defaults(handler=command_series)
    move_out = sub.add_parser("move-out")
    move_out.add_argument("--keep", required=True)
    move_out.set_defaults(handler=command_move_out)
    sub.add_parser("move-back").set_defaults(handler=command_move_back)
    sub.add_parser("list").set_defaults(handler=command_list)
    args = parser.parse_args()
    return args.handler(args)


if __name__ == "__main__":
    sys.exit(main())
