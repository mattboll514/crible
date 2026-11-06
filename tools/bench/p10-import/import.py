#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""HTTP client of the import campaign of IMPORTER-BASE-COMPLETE (P10).

It speaks to a running Crible instance and writes every raw response under
`data/p10-import/`, which is gitignored. It never writes to an index by any
other route than the one of ADR-0139, decision 5, point 4:

    POST /v1/indexes/{index}/batches?wait=true   { "directory": "<lot>" }
    GET  /v1/jobs/{id}                           until a terminal state

It also creates the six indexes of `docs/legifacile/configs/` through
`POST /v1/indexes` (ADR-0105).

    python3 tools/bench/p10-import/import.py create
    python3 tools/bench/p10-import/import.py attach --index dole --lot dole-full-v1
    python3 tools/bench/p10-import/import.py state --index dole
    python3 tools/bench/p10-import/import.py collect

The timestamps written in the files are the client's clock, in UTC, with
microsecond resolution; the server's own clock is in its journal, captured by
`attach-all.sh` with `docker logs -t`.
"""

import argparse
import contextlib
import json
import pathlib
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone

ROOT = pathlib.Path(__file__).resolve().parents[3]
OUT = ROOT / "data" / "p10-import"
CONFIGS = ROOT / "docs" / "legifacile" / "configs"

# The order of ADR-0139, decision 5, point 3: smallest first.
LOTS = [
    ("dole", "dole-full-v1"),
    ("sarde", "sarde-full-v1"),
    ("titres", "titres-full-v2"),
    ("jorf", "jorf-full-v1"),
    ("articles", "articles-full-v1"),
    ("decisions", "decisions-full-v1"),
]

POLL_SECONDS = 10.0
# Every command stays under 30 minutes (campaign rule). A job still running at
# this bound is not killed: `attach` says so, and `wait` resumes the follow-up.
MAX_WAIT_SECONDS = 1500.0


def now():
    return datetime.now(timezone.utc).isoformat(timespec="microseconds")


def token(path):
    return path.read_text(encoding="utf-8").strip()


def call(base, key, method, path, body=None, timeout=120.0):
    """One HTTP call; returns the status, the decoded JSON and the raw bytes."""
    data = None
    headers = {"Authorization": f"Bearer {key}"}
    if body is not None:
        data = json.dumps(body).encode("utf-8")
        headers["Content-Type"] = "application/json"
    request = urllib.request.Request(base + path, data=data, headers=headers, method=method)
    started = time.perf_counter()
    try:
        with urllib.request.urlopen(request, timeout=timeout) as answer:
            raw = answer.read()
            status = answer.status
    except urllib.error.HTTPError as error:
        raw = error.read()
        status = error.code
    elapsed_ms = (time.perf_counter() - started) * 1000.0
    try:
        decoded = json.loads(raw) if raw else None
    except json.JSONDecodeError:
        decoded = None
    return status, decoded, raw, elapsed_ms


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


SAMPLE_SECONDS = 5.0
# One `docker exec` reads the whole cgroup and the process in one go: four
# calls per sample would add latency and blur the timestamps.
SAMPLE_SCRIPT = (
    'cat /sys/fs/cgroup/memory.current; '
    'awk "/^anon /{print \\$2}" /sys/fs/cgroup/memory.stat; '
    'awk "/^file /{print \\$2}" /sys/fs/cgroup/memory.stat; '
    'awk "/^VmRSS/{print \\$2}" /proc/1/status; '
    'awk "/^VmHWM/{print \\$2}" /proc/1/status'
)


class MemorySampler:
    """Samples the memory of a container every `SAMPLE_SECONDS`, in a thread.

    The attach request blocks the main thread for up to 90 s, and the job polls
    sleep 10 s: sampling from a thread keeps one point every five seconds from
    the submission to the terminal state. The raw points are appended to
    `data/p10-import/memory-<index>.jsonl`, one JSON object per line.
    """

    def __init__(self, container, path, interval=SAMPLE_SECONDS):
        self.container = container
        self.interval = interval
        self.path = path
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self._run, name="sampler", daemon=True)
        self.started = None
        self.points = 0
        self.errors = 0

    def __enter__(self):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        handle = self.path.open("w", encoding="utf-8")
        handle.close()
        self.started = time.monotonic()
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.stop.set()
        self.thread.join(timeout=self.interval + 30.0)
        return False

    def _sample(self):
        try:
            done = subprocess.run(
                ["docker", "exec", self.container, "sh", "-c", SAMPLE_SCRIPT],
                capture_output=True,
                text=True,
                timeout=30.0,
                check=False,
            )
        except (subprocess.SubprocessError, OSError):
            self.errors += 1
            return None
        if done.returncode != 0:
            self.errors += 1
            return None
        lines = done.stdout.split()
        if len(lines) != 5:
            self.errors += 1
            return None
        current, anon, file, rss_kb, hwm_kb = (int(value) for value in lines)
        return {
            "at": now(),
            "elapsed_s": round(time.monotonic() - self.started, 1),
            "memory_current": current,
            "anon": anon,
            "file": file,
            "rss_kb": rss_kb,
            "hwm_kb": hwm_kb,
        }

    def _run(self):
        while not self.stop.is_set():
            point = self._sample()
            if point is not None:
                with self.path.open("a", encoding="utf-8") as handle:
                    handle.write(json.dumps(point) + "\n")
                self.points += 1
            self.stop.wait(self.interval)



def command_create(args):
    key = token(args.token_file)
    OUT.mkdir(parents=True, exist_ok=True)
    for name, _ in LOTS:
        config = json.loads((CONFIGS / f"{name}.json").read_text(encoding="utf-8"))
        status, decoded, raw, elapsed = call(args.base, key, "POST", "/v1/indexes", config)
        record = {
            "at": now(),
            "index": name,
            "http_status": status,
            "elapsed_ms": elapsed,
            "request": config,
            "response": decoded,
        }
        write_json(OUT / "index-create" / f"{name}.json", record)
        print(f"{name}: HTTP {status} in {elapsed:.1f} ms")
        if status != 201:
            print(raw.decode("utf-8", "replace"), file=sys.stderr)
            return 1
    return 0


def poll_until_terminal(args, key, index, job_id, deadline):
    """Polls `job_id` until a terminal state or the deadline; returns the job."""
    polls = OUT / "polls" / f"{index}.jsonl"
    polls.parent.mkdir(parents=True, exist_ok=True)
    while True:
        status, job, raw, _ = call(args.base, key, "GET", f"/v1/jobs/{job_id}")
        if job is None:
            print(f"{index}: poll answered HTTP {status}", file=sys.stderr)
            print(raw.decode("utf-8", "replace"), file=sys.stderr)
            return None, 1
        with polls.open("a", encoding="utf-8") as log:
            log.write(json.dumps({"at": now(), "http_status": status, "job": job}) + "\n")
        state = job.get("state")
        progress = job.get("progress")
        print(
            f"{index}: {state}"
            + (f" {progress['step']} {progress['done']}/{progress['total']}" if progress else "")
        )
        if state in ("succeeded", "failed"):
            write_json(OUT / "jobs" / f"{index}-final.json", {"at": now(), "job": job})
            if state != "succeeded":
                print(f"{index}: job failed: {job.get('error')}", file=sys.stderr)
                return job, 1
            print(f"{index}: finished, generation {job.get('generation')}")
            return job, 0
        if time.monotonic() >= deadline:
            print(
                f"{index}: still {state} at the bound of this command; job {job_id} "
                "is left running, resume with `import.py wait`",
                file=sys.stderr,
            )
            return job, 3
        time.sleep(POLL_SECONDS)


def sampler_context(args, index):
    """The memory sampler, or a no-op context when `--sample-container` is absent."""
    if args.sample_container is None:
        return contextlib.nullcontext(None)
    return MemorySampler(args.sample_container, OUT / "memory" / f"{index}.jsonl")


def command_attach(args):
    key = token(args.token_file)
    batch = {"directory": args.lot}
    with sampler_context(args, args.index) as sampler:
        return _attach(args, key, batch, sampler)


def _attach(args, key, batch, sampler):
    started_at = now()
    status, decoded, raw, elapsed = call(
        args.base,
        key,
        "POST",
        f"/v1/indexes/{args.index}/batches?wait=true",
        batch,
        timeout=90.0,
    )
    answer = {
        "submitted_at": started_at,
        "answered_at": now(),
        "elapsed_ms": elapsed,
        "http_status": status,
        "request": {"directory": args.lot},
        "response": decoded,
    }
    write_json(OUT / "jobs" / f"{args.index}-submit.json", answer)
    if decoded is None or "id" not in decoded:
        print(raw.decode("utf-8", "replace"), file=sys.stderr)
        return 1
    job_id = decoded["id"]
    print(f"{args.index}: submitted {job_id} at {started_at}, HTTP {status}, {elapsed:.0f} ms")

    if status == 202:
        polls = OUT / "polls" / f"{args.index}.jsonl"
        polls.parent.mkdir(parents=True, exist_ok=True)
        with polls.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps({"at": now(), "http_status": status, "job": decoded}) + "\n")

    deadline = time.monotonic() + args.max_wait_seconds
    job, code = poll_until_terminal(args, key, args.index, job_id, deadline)
    if sampler is not None:
        print(
            f"{args.index}: memory samples {sampler.points} "
            f"(errors {sampler.errors}) in {sampler.path}"
        )
    return code


def command_wait(args):
    key = token(args.token_file)
    submit = json.loads((OUT / "jobs" / f"{args.index}-submit.json").read_text(encoding="utf-8"))
    job_id = submit["response"]["id"]
    deadline = time.monotonic() + args.max_wait_seconds
    return poll_until_terminal(args, key, args.index, job_id, deadline)[1]


def command_state(args):
    key = token(args.token_file)
    status, decoded, raw, elapsed = call(args.base, key, "GET", f"/v1/indexes/{args.index}")
    write_json(
        OUT / "index-state" / f"{args.index}.json",
        {"at": now(), "http_status": status, "elapsed_ms": elapsed, "response": decoded},
    )
    print(f"{args.index}: HTTP {status} in {elapsed:.1f} ms")
    if status != 200:
        print(raw.decode("utf-8", "replace"), file=sys.stderr)
        return 1
    return 0


def command_collect(args):
    key = token(args.token_file)
    status, decoded, raw, elapsed = call(args.base, key, "GET", "/v1/indexes")
    write_json(OUT / "index-list.json", {"at": now(), "http_status": status, "response": decoded})
    print(f"list: HTTP {status} in {elapsed:.1f} ms")
    if status != 200:
        print(raw.decode("utf-8", "replace"), file=sys.stderr)
        return 1
    for name, _ in LOTS:
        status, decoded, raw, elapsed = call(args.base, key, "GET", f"/v1/indexes/{name}")
        write_json(
            OUT / "index-state" / f"{name}.json",
            {"at": now(), "http_status": status, "elapsed_ms": elapsed, "response": decoded},
        )
        print(f"{name}: HTTP {status} in {elapsed:.1f} ms")
        if status != 200:
            return 1
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", default="http://127.0.0.1:18090")
    parser.add_argument("--token-file", type=pathlib.Path, default=OUT / "token.txt")
    sub = parser.add_subparsers(dest="command", required=True)

    create = sub.add_parser("create")
    create.set_defaults(handler=command_create)

    attach = sub.add_parser("attach")
    attach.add_argument("--index", required=True)
    attach.add_argument("--lot", required=True)
    attach.add_argument("--max-wait-seconds", type=float, default=MAX_WAIT_SECONDS)
    attach.add_argument(
        "--sample-container",
        default=None,
        help="sample the memory of this container every five seconds while the job runs",
    )
    attach.set_defaults(handler=command_attach)

    wait = sub.add_parser("wait")
    wait.add_argument("--index", required=True)
    wait.add_argument("--max-wait-seconds", type=float, default=MAX_WAIT_SECONDS)
    wait.set_defaults(handler=command_wait)

    state = sub.add_parser("state")
    state.add_argument("--index", required=True)
    state.set_defaults(handler=command_state)

    collect = sub.add_parser("collect")
    collect.set_defaults(handler=command_collect)

    args = parser.parse_args()
    return args.handler(args)


if __name__ == "__main__":
    sys.exit(main())
