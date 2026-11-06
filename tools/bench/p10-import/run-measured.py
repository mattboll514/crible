#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Runs one offline CLI command in a container and samples its memory.

The offline commands of the campaign (`compact`, `rebuild-lexical`) take the
writer lock of an index (ADR-0116): the server must be stopped, and the command
runs alone on the data volume. This script starts a named container on
`crible-p10-data`, samples `memory.current`, `anon`, `file` and the RSS of
pid 1 every five seconds, records `df` on the host and in the VM, and writes one
JSON result plus the raw samples.

    python3 tools/bench/p10-import/run-measured.py \\
        --name p10-mes07-decisions --key mes07/decisions \\
        -- compact --data /data --index decisions

The container is left stopped with its name, like the containers of the import
campaign: nothing is removed. The image is `crible:p10` unless `--image` says
otherwise.
"""

import argparse
import json
import pathlib
import subprocess
import sys
import threading
import time
from datetime import datetime, timezone

ROOT = pathlib.Path(__file__).resolve().parents[3]
OUT = ROOT / "data" / "p10-import"

SAMPLE_SECONDS = 5.0
DF_SECONDS = 60.0
SAMPLE_SCRIPT = (
    'cat /sys/fs/cgroup/memory.current; '
    'awk "/^anon /{print \\$2}" /sys/fs/cgroup/memory.stat; '
    'awk "/^file /{print \\$2}" /sys/fs/cgroup/memory.stat; '
    'awk "/^VmRSS/{print \\$2}" /proc/1/status; '
    'awk "/^VmHWM/{print \\$2}" /proc/1/status; '
    'df -k /data | tail -1 | awk "{print \\$3}"'
)


def now():
    return datetime.now(timezone.utc).isoformat(timespec="microseconds")


def run(command, timeout=120.0):
    return subprocess.run(command, capture_output=True, text=True, timeout=timeout, check=False)


def df_host():
    done = run(["df", "-H", "/"], timeout=30.0)
    done_h = run(["df", "-h", "/"], timeout=30.0)
    return {"at": now(), "H": done.stdout.strip().splitlines()[-1], "h": done_h.stdout.strip().splitlines()[-1]}


class Sampler(threading.Thread):
    def __init__(self, container, path, interval=SAMPLE_SECONDS):
        super().__init__(name="sampler", daemon=True)
        self.container = container
        self.path = path
        self.interval = interval
        self.stop = threading.Event()
        self.started = time.monotonic()
        self.points = 0
        self.errors = 0
        self.df = []
        self.last_df = 0.0

    def _sample(self):
        done = run(["docker", "exec", self.container, "sh", "-c", SAMPLE_SCRIPT], timeout=60.0)
        if done.returncode != 0:
            self.errors += 1
            return None
        parts = done.stdout.split()
        if len(parts) != 6:
            self.errors += 1
            return None
        current, anon, file, rss_kb, hwm_kb, used_kb = (int(x) for x in parts)
        return {
            "at": now(),
            "elapsed_s": round(time.monotonic() - self.started, 1),
            "memory_current": current,
            "anon": anon,
            "file": file,
            "rss_kb": rss_kb,
            "hwm_kb": hwm_kb,
            "data_used_kb": used_kb,
        }

    def run(self):
        while not self.stop.is_set():
            point = self._sample()
            if point is not None:
                with self.path.open("a", encoding="utf-8") as handle:
                    handle.write(json.dumps(point) + "\n")
                self.points += 1
                if point["elapsed_s"] - self.last_df >= DF_SECONDS:
                    self.df.append({"at": point["at"], "vm_data_used_kb": point["data_used_kb"], **df_host()})
                    self.last_df = point["elapsed_s"]
            self.stop.wait(self.interval)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--name", required=True)
    parser.add_argument("--key", required=True, help="prefix of the raw files, e.g. mes07/decisions")
    parser.add_argument("--image", default="crible:p10")
    parser.add_argument("--volume", default="crible-p10-data")
    parser.add_argument("--interval", type=float, default=SAMPLE_SECONDS,
                        help="sampling interval in seconds (5 by default)")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("a command to run in the container is required, after `--`")

    raw = OUT / args.key
    raw.parent.mkdir(parents=True, exist_ok=True)
    samples = raw.with_suffix(".samples.jsonl")
    samples.write_text("", encoding="utf-8")

    host_before = df_host()
    docker = [
        "docker", "run", "--name", args.name,
        "-v", f"{args.volume}:/data",
        args.image, *command,
    ]
    started_at = now()
    started = time.monotonic()
    sampler = Sampler(args.name, samples, interval=args.interval)
    process = subprocess.Popen(docker, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    sampler.start()
    output = process.communicate()[0]
    duration = time.monotonic() - started
    sampler.stop.set()
    sampler.join(timeout=args.interval + 30)
    finished_at = now()

    inspect = run(["docker", "inspect", args.name, "--format",
                   "{{.State.ExitCode}} {{.State.OOMKilled}}"], timeout=30.0)
    exit_code, oom = None, None
    if inspect.returncode == 0 and inspect.stdout.split():
        parts = inspect.stdout.split()
        exit_code, oom = int(parts[0]), parts[1] == "true"

    result = {
        "name": args.name,
        "image": args.image,
        "command": command,
        "started_at": started_at,
        "finished_at": finished_at,
        "duration_s": round(duration, 1),
        "exit_code": exit_code,
        "oom_killed": oom,
        "output": output,
        "samples": sampler.points,
        "sample_errors": sampler.errors,
        "df": {"host_before": host_before, "during": sampler.df, "host_after": df_host()},
    }
    raw.with_suffix(".json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")

    if sampler.points:
        points = [json.loads(line) for line in samples.read_text(encoding="utf-8").splitlines()]
        gib = 1024 ** 3
        result["peak"] = {
            "anon_gib": round(max(p["anon"] for p in points) / gib, 2),
            "memory_current_gib": round(max(p["memory_current"] for p in points) / gib, 2),
            "hwm_gib": round(max(p["hwm_kb"] for p in points) / 1024 / 1024, 2),
        }
        raw.with_suffix(".json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")

    print(f"{args.name}: exit {exit_code} oom {oom} in {duration:.0f} s, "
          f"{sampler.points} samples (errors {sampler.errors})")
    if output.strip():
        print(output.strip()[-2000:])
    return 0 if exit_code == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
