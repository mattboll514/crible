#!/usr/bin/env python3
"""MES-17 second campaign harness: attribute the retained anonymous memory.

Protocol: ADR-0157 (accepted 2026-11-21), decisions 2, 3, 4, 8 and 9, with the
reading rule of ADR-0158 (accepted 2026-11-22): an effect is a *drop* of the
retained anonymous memory by at least the dispersion between the two V0 runs.
Seven states on one container and one volume:

  (a) fresh process, no index
  (b) index created (and its `session_load` line)
  (c) six lots attached
  (d) 300 queries of `queries.jsonl`, `hybrid`, `k = 10`, on the full index
  (e) restart with the index present
  (f) `DELETE` of the throwaway index
  (g) restart without index

Each state is read three times, three seconds apart. Sources: `/proc/1/status`,
`/proc/1/smaps_rollup`, `/proc/1/smaps` (parsed, not kept), `memory.current`,
`memory.swap.current` and `memory.stat` of the container cgroup, the VM's
`/proc/meminfo`, and the `memory` lines of the journal.

The container is created by `up`, which is the ONLY place where the variant is
applied (`docker run -e GLIBC_TUNABLES=...`), never in the image nor in
`crible.toml` (ADR-0157, decision 4). The exact value is read back from the
container environment and written to the run's `run.json`.

Standard library only. Paths are relative to the repository root. The admin key
is read at run time from `data/mes17/raw/key.txt` (gitignored).
"""

from __future__ import annotations

import argparse
import json
import math
import pathlib
import re
import statistics
import subprocess
import sys
import time
import urllib.error
import urllib.request

REPO = pathlib.Path(__file__).resolve().parents[3]
DATA = REPO / "data"
MES17 = DATA / "mes17"
RAW_ROOT = MES17 / "raw"
KEY_FILE = RAW_ROOT / "key.txt"
QUERIES = DATA / "legifacile-sample" / "queries.jsonl"
EXCHANGE = MES17 / "exchange"
TOML = MES17 / "crible.toml"

CONTAINER = "crible-mes17"
VOLUME = "crible-mes17-data"
PORT = 18017
BASE = f"http://127.0.0.1:{PORT}"
INDEX = "mes17"
PROBE_IMAGE = "postgres:17"
LOADER = "/lib/ld-linux-aarch64.so.1"
PAGE = 1048576.0  # bytes per MiB

LOTS = [
    "mes17-legifacile-sample-articles-v1",
    "mes17-legifacile-sample-jorf-v1",
    "mes17-legifacile-sample-decisions-v1",
    "mes17-legifacile-sample-titres-v1",
    "mes17-legifacile-sample-sarde-v1",
    "mes17-legifacile-sample-dole-v1",
]

# ADR-0157, decision 4. V4 is the combination of the variants that had an
# effect and is passed explicitly with --tunables.
VARIANTS = {
    "V0": None,
    "V1": "glibc.malloc.arena_max=1",
    "V2": "glibc.malloc.trim_threshold=0:glibc.malloc.top_pad=0",
    "V3": "glibc.malloc.mmap_threshold=131072",
}

STATES = [
    ("a-fresh", "fresh process, no index"),
    ("b-index-open", "index created"),
    ("c-after-attach", "six lots attached"),
    ("d-after-queries", "300 queries on the full index"),
    ("e-after-restart-index", "restart with the index present"),
    ("f-after-delete", "index deleted"),
    ("g-after-restart", "restart without index"),
]
STATE_KEYS = [key for key, _ in STATES]

STATUS_FIELDS = [
    "VmRSS", "RssAnon", "RssFile", "RssShmem", "VmHWM", "VmSwap", "VmData",
    "VmStk", "VmExe", "VmLib", "VmPTE",
]
SMAPS_FIELDS = [
    "Rss", "Pss", "Pss_Anon", "Pss_File", "Shared_Clean", "Shared_Dirty",
    "Private_Clean", "Private_Dirty", "Anonymous", "Swap", "SwapPss",
    "Locked", "KernelPageSize", "MMUPageSize",
]
STAT_FIELDS = [
    "anon", "file", "kernel", "kernel_stack", "pagetables", "sec_pagetables",
    "sock", "vmalloc", "shmem", "zswap", "zswapped", "file_mapped",
    "file_dirty", "file_writeback", "swapcached", "slab", "slab_reclaimable",
    "slab_unreclaimable", "inactive_anon", "active_anon", "inactive_file",
    "active_file", "unevictable",
]
MEMINFO_FIELDS = ["MemTotal", "MemAvailable", "SwapTotal", "SwapFree"]

# glibc arena signature (ADR-0157, decision 3, as implemented here): a group of
# consecutive anonymous mappings (no pathname) whose `Size` values sum to
# exactly 65 536 kB is one arena; its `Rss` is the sum of the group. A group
# whose sizes do not sum to 65 536 kB is not an arena.
ARENA_SIZE_KB = 65536

HEADER = re.compile(r"^[0-9a-f]+-[0-9a-f]+ ")
GUARD_MEM_AVAILABLE_KB = 1048576  # 1 GiB
GUARD_SWAP_DROP_KB = 262144  # 256 MiB


# --------------------------------------------------------------------------
# plumbing


def sh(*args, timeout=180):
    return subprocess.run(args, capture_output=True, text=True, timeout=timeout)


def fail(message):
    raise SystemExit(message)


def key() -> str:
    line = KEY_FILE.read_text().splitlines()[0]
    if ": " not in line:
        fail(f"unreadable key file {KEY_FILE.relative_to(REPO)}")
    return line.split(": ", 1)[1].strip()


def api(method: str, path: str, body=None, timeout=180):
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(
        BASE + path,
        data=data,
        method=method,
        headers={"Authorization": f"Bearer {key()}", "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            raw = response.read()
            return response.status, (json.loads(raw) if raw else None)
    except urllib.error.HTTPError as error:
        raw = error.read()
        return error.code, (json.loads(raw) if raw else None)


def run_dir(value: str) -> pathlib.Path:
    path = (REPO / value).resolve()
    if not str(path).startswith(str(RAW_ROOT.resolve())):
        fail(f"run directory must live under {RAW_ROOT.relative_to(REPO)}")
    path.mkdir(parents=True, exist_ok=True)
    return path


def container_id() -> str:
    out = sh("docker", "inspect", "-f", "{{.Id}}", CONTAINER).stdout.strip()
    if not out:
        fail(f"container {CONTAINER} is not running")
    return out


def container_env() -> str | None:
    out = sh(
        "docker", "inspect", "-f",
        "{{range .Config.Env}}{{println .}}{{end}}", CONTAINER,
    ).stdout
    for line in out.splitlines():
        if line.startswith("GLIBC_TUNABLES="):
            return line.split("=", 1)[1]
    return None


def image_id() -> str:
    return sh("docker", "inspect", "-f", "{{.Image}}", CONTAINER).stdout.strip()


def wait_ready(timeout=300) -> float:
    started = time.time()
    deadline = started + timeout
    while time.time() < deadline:
        code = sh("curl", "-s", "-o", "/dev/null", "-w", "%{http_code}",
                  BASE + "/ready").stdout.strip()
        if code == "200":
            return time.time() - started
        time.sleep(2)
    fail("server never ready")


# --------------------------------------------------------------------------
# readings


def probe_script(cgroup: str) -> str:
    return (
        'echo "@@status"; cat /proc/1/status; '
        'echo "@@smaps_rollup"; cat /proc/1/smaps_rollup; '
        'echo "@@smaps"; cat /proc/1/smaps; '
        f'echo "@@current"; cat /sys/fs/cgroup/docker/{cgroup}/memory.current; '
        f'echo "@@swap_current"; cat /sys/fs/cgroup/docker/{cgroup}/memory.swap.current; '
        f'echo "@@stat"; cat /sys/fs/cgroup/docker/{cgroup}/memory.stat; '
        'echo "@@meminfo"; cat /proc/meminfo; '
        'echo "@@page"; getconf PAGESIZE; '
        'echo "@@environ"; tr "\\0" "\\n" < /proc/1/environ'
    )


def sections_of(text: str) -> dict[str, list[str]]:
    sections: dict[str, list[str]] = {}
    name = None
    for line in text.splitlines():
        if line.startswith("@@"):
            name = line[2:].strip()
            sections[name] = []
        elif name:
            sections[name].append(line)
    return sections


def read_once(cgroup: str) -> dict:
    script = probe_script(cgroup)
    # Three attempts: the first smaps_rollup of a freshly attached pid
    # namespace was once refused, and a missing reading must never become a
    # silent zero (ADR-0151, decision 1).
    last = ""
    for attempt in range(3):
        result = sh(
            "docker", "run", "--rm", "--pull=never",
            "--pid", f"container:{CONTAINER}", "--cgroupns=host",
            "--cap-add", "SYS_PTRACE", PROBE_IMAGE, "sh", "-c", script,
            timeout=300,
        )
        last = result.stdout + result.stderr
        if (result.returncode == 0 and "@@smaps" in result.stdout
                and "@@meminfo" in result.stdout
                and "Permission denied" not in result.stdout):
            break
        if attempt == 2:
            # Never echo the whole probe output: `@@environ` holds the complete
            # `/proc/1/environ`, which rule 11 and ADR-0032 keep out of logs.
            diagnostic = last.split("@@environ", 1)[0]
            fail(f"reading failed: {diagnostic[-2000:]}")
        time.sleep(1)

    sections = sections_of(last)
    for needed in ("status", "smaps_rollup", "smaps", "current", "swap_current",
                   "stat", "meminfo", "page", "environ"):
        if not sections.get(needed):
            fail(f"reading has no {needed} section")

    reading: dict = {"at": time.strftime("%Y-%m-%dT%H:%M:%S%z")}

    status = {}
    for line in sections["status"]:
        if ":" in line:
            name, value = line.split(":", 1)
            status[name.strip()] = value.strip()
    missing = [name for name in STATUS_FIELDS if name not in status]
    if missing:
        fail(f"status is missing {', '.join(missing)}")
    reading["status"] = {name: status[name] for name in STATUS_FIELDS}

    rollup = {}
    for line in sections["smaps_rollup"]:
        if ":" in line:
            name, value = line.split(":", 1)
            rollup[name.strip()] = value.strip()
    reading["smaps_rollup"] = {name: rollup.get(name) for name in SMAPS_FIELDS}
    # `smaps_rollup` carries no KernelPageSize on this kernel (6.10-linuxkit),
    # so the page size is read by `getconf` in the same probe (ADR-0135,
    # decision 7). A missing value fails the reading rather than becoming a
    # silent default.
    page = sections["page"][0].strip() if sections.get("page") else ""
    if not page.isdigit():
        fail(f"page size unreadable: {page!r}")
    reading["page_size_bytes"] = int(page)

    reading["arenas"], reading["pathless"] = parse_smaps(sections["smaps"])
    reading["memory_current_bytes"] = int(sections["current"][0].strip())
    reading["memory_swap_current_bytes"] = int(sections["swap_current"][0].strip())

    stat = {}
    for line in sections["stat"]:
        parts = line.split()
        if len(parts) == 2:
            stat[parts[0]] = int(parts[1])
    missing = [name for name in STAT_FIELDS if name not in stat]
    if missing:
        fail(f"memory.stat is missing {', '.join(missing)}")
    reading["memory_stat"] = {name: stat[name] for name in STAT_FIELDS}

    meminfo = {}
    for line in sections["meminfo"]:
        if ":" in line:
            name, value = line.split(":", 1)
            number = value.split()
            if number:
                meminfo[name.strip()] = int(number[0])
    missing = [name for name in MEMINFO_FIELDS if name not in meminfo]
    if missing:
        fail(f"meminfo is missing {', '.join(missing)}")
    reading["meminfo"] = {name: meminfo[name] for name in MEMINFO_FIELDS}
    # Proof that the variant is in the environment of PID 1 (ADR-0157,
    # decision 4). Only this variable is kept, never the rest of the
    # environment.
    environment = {}
    for line in sections["environ"]:
        if "=" in line:
            name, value = line.split("=", 1)
            environment[name] = value
    reading["glibc_tunables_env"] = environment.get("GLIBC_TUNABLES")
    return reading


def parse_smaps(lines: list[str]) -> tuple[dict, dict]:
    """Count glibc arenas by the declared rule; keep aggregates only.

    `/proc/1/smaps` is voluminous: it is read in full and analysed here, and
    only the aggregate is written to disk (ADR-0157, `Conséquences`).
    """
    regions = []
    current = None
    for line in lines:
        if HEADER.match(line):
            parts = line.split()
            path = " ".join(parts[5:]) if len(parts) > 5 else ""
            current = {"perms": parts[1], "path": path, "size_kb": 0, "rss_kb": 0}
            regions.append(current)
        elif current is not None and ":" in line:
            name, value = line.split(":", 1)
            number = value.split()
            if not number or not number[0].isdigit():
                continue
            if name == "Size":
                current["size_kb"] = int(number[0])
            elif name == "Rss":
                current["rss_kb"] = int(number[0])

    groups = []
    pending = []
    for region in regions:
        if region["path"]:
            if pending:
                groups.append(pending)
                pending = []
            continue
        pending.append(region)
        if sum(item["size_kb"] for item in pending) >= ARENA_SIZE_KB:
            groups.append(pending)
            pending = []
    if pending:
        groups.append(pending)

    arenas = {"count": 0, "rss_kb": 0, "groups": []}
    other = {"count": 0, "size_kb": 0, "rss_kb": 0}
    pathless = {
        "regions": sum(1 for region in regions if not region["path"]),
        "size_kb": sum(region["size_kb"] for region in regions if not region["path"]),
        "rss_kb": sum(region["rss_kb"] for region in regions if not region["path"]),
    }
    for group in groups:
        size = sum(item["size_kb"] for item in group)
        rss = sum(item["rss_kb"] for item in group)
        if size == ARENA_SIZE_KB:
            arenas["count"] += 1
            arenas["rss_kb"] += rss
            arenas["groups"].append(
                {"sizes_kb": [item["size_kb"] for item in group],
                 "perms": [item["perms"] for item in group],
                 "rss_kb": rss}
            )
        else:
            other["count"] += len(group)
            other["size_kb"] += size
            other["rss_kb"] += rss
    return arenas, {"other_groups": other, **pathless}


# --------------------------------------------------------------------------
# journal


def journal(path: pathlib.Path) -> None:
    result = sh("docker", "logs", CONTAINER, timeout=300)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(result.stdout + result.stderr)


def memory_lines(text: str) -> list[dict]:
    lines = []
    for line in text.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            entry = json.loads(line)
        except json.JSONDecodeError:
            continue
        fields = entry.get("fields") or {}
        if fields.get("message") != "memory":
            continue
        lines.append({
            "ts": entry.get("timestamp"),
            "step": fields.get("step"),
            "kind": fields.get("kind"),
            "index": fields.get("index"),
            "job_id": fields.get("job_id"),
            "rss_anon_bytes": fields.get("rss_anon_bytes"),
            "rss_file_bytes": fields.get("rss_file_bytes"),
            "cgroup_current_bytes": fields.get("cgroup_current_bytes"),
            "cgroup_anon_bytes": fields.get("cgroup_anon_bytes"),
            "cgroup_file_bytes": fields.get("cgroup_file_bytes"),
        })
    return lines


# --------------------------------------------------------------------------
# commands


def cmd_up(args) -> None:
    existing = sh("docker", "ps", "-a", "-q", "--filter", f"name=^{CONTAINER}$").stdout.strip()
    if existing:
        sh("docker", "rm", "-f", CONTAINER, timeout=180)
    tunables = args.tunables
    if tunables is None and args.variant is not None:
        if args.variant in VARIANTS:
            tunables = VARIANTS[args.variant]
        else:
            fail(f"variant {args.variant} needs an explicit --tunables")
    if not TOML.exists():
        fail(f"missing {TOML.relative_to(REPO)}")
    command = [
        "docker", "run", "-d", "--name", CONTAINER, "--pull=never",
        "-p", f"{PORT}:8080", "--stop-timeout", "65",
        "-v", f"{VOLUME}:/data",
        "-v", f"{REPO / 'data' / 'models'}:/models:ro",
        "-v", f"{EXCHANGE}:/exchange",
        "-v", f"{TOML}:/etc/crible/crible.toml:ro",
    ]
    if tunables:
        command += ["-e", f"GLIBC_TUNABLES={tunables}"]
    command.append(args.image)
    print(" ".join(command), flush=True)
    result = sh(*command, timeout=300)
    if result.returncode != 0:
        fail(f"docker run failed: {result.stdout}{result.stderr}")
    elapsed = wait_ready()
    print(f"ready after {elapsed:.1f} s", flush=True)


def cmd_down(_args) -> None:
    sh("docker", "stop", "-t", "65", CONTAINER, timeout=180)
    sh("docker", "rm", "-f", CONTAINER, timeout=180)
    print("container removed", flush=True)


def cmd_init(args) -> None:
    out = run_dir(args.run)
    cgroup = container_id()
    reading = read_once(cgroup)
    info = sh("docker", "info", "--format", "{{.MemTotal}} {{.NCPU}}").stdout.split()
    disk = sh("df", "-h", str(REPO)).stdout.splitlines()
    commit = sh("git", "-C", str(REPO), "rev-parse", "HEAD").stdout.strip()
    branch = sh("git", "-C", str(REPO), "rev-parse", "--abbrev-ref", "HEAD").stdout.strip()
    page = reading["page_size_bytes"]
    document = {
        "campaign": "MES-17 second campaign (ADR-0157)",
        "variant": args.variant or args.tunables or "custom",
        "glibc_tunables": container_env(),
        "image": args.image,
        "image_id": image_id(),
        "container_id": cgroup,
        "container_created": sh(
            "docker", "inspect", "-f", "{{.Created}}", CONTAINER).stdout.strip(),
        "vm_mem_total_bytes": int(info[0]) if info else None,
        "vm_ncpu": int(info[1]) if len(info) > 1 else None,
        "page_size_bytes": page,
        "kernel": sh("docker", "run", "--rm", "--pull=never", PROBE_IMAGE,
                     "uname", "-r", timeout=120).stdout.strip(),
        "commit": commit,
        "branch": branch,
        "df_start": disk,
        "started_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "guard": {"mem_available_kb": reading["meminfo"]["MemAvailable"],
                  "swap_free_kb": reading["meminfo"]["SwapFree"]},
        "queries_sha256_source": QUERIES.relative_to(REPO).as_posix(),
        "queries_count": sum(1 for line in QUERIES.read_text().splitlines() if line.strip()),
        "exchange": EXCHANGE.relative_to(REPO).as_posix(),
        "config": TOML.relative_to(REPO).as_posix(),
    }
    (out / "run.json").write_text(json.dumps(document, indent=2) + "\n")
    print(json.dumps(document, indent=2), flush=True)


def guard(out: pathlib.Path, reading: dict) -> None:
    run = json.loads((out / "run.json").read_text())
    baseline = run["guard"]
    meminfo = reading["meminfo"]
    available = meminfo.get("MemAvailable")
    free = meminfo.get("SwapFree")
    print(f"  guard: MemAvailable={available} kB SwapFree={free} kB "
          f"(baseline {baseline['mem_available_kb']} / {baseline['swap_free_kb']})",
          flush=True)
    if available is None or free is None:
        fail("MEMORY GUARD: meminfo lacks MemAvailable or SwapFree")
    if baseline["mem_available_kb"] is None or baseline["swap_free_kb"] is None:
        fail("MEMORY GUARD: baseline lacks MemAvailable or SwapFree")
    if available < GUARD_MEM_AVAILABLE_KB:
        fail(f"MEMORY GUARD: MemAvailable {available} kB below 1 GiB")
    if baseline["swap_free_kb"] - free > GUARD_SWAP_DROP_KB:
        fail(f"MEMORY GUARD: SwapFree dropped by "
             f"{baseline['swap_free_kb'] - free} kB, above 256 MiB")


def cmd_state(args) -> None:
    out = run_dir(args.run)
    if args.name not in STATE_KEYS:
        fail(f"unknown state {args.name}")
    cgroup = container_id()
    readings = []
    for number in range(args.reps):
        reading = read_once(cgroup)
        readings.append(reading)
        status = reading["status"]
        arenas = reading["arenas"]
        print(
            f"  {args.name} {number + 1}/{args.reps}: "
            f"current={reading['memory_current_bytes']} "
            f"anon={reading['memory_stat'].get('anon')} "
            f"file={reading['memory_stat'].get('file')} "
            f"rss_anon={status.get('RssAnon')} swap={status.get('VmSwap')} "
            f"arenas={arenas['count']}/{arenas['rss_kb']} kB "
            f"swap_current={reading['memory_swap_current_bytes']}",
            flush=True,
        )
        guard(out, reading)
        if number + 1 < args.reps:
            time.sleep(args.gap)
    journal(out / "logs" / f"{args.name}.jsonl")
    (out / "states").mkdir(parents=True, exist_ok=True)
    (out / "states" / f"{args.name}.json").write_text(
        json.dumps(readings, indent=2) + "\n")
    print(f"  state {args.name} written", flush=True)


def poll(job_id: str, label: str, timeout=1800) -> dict:
    deadline = time.time() + timeout
    last = None
    shown = time.time()
    while time.time() < deadline:
        code, body = api("GET", f"/v1/jobs/{job_id}")
        state = (body or {}).get("state")
        if state != last or time.time() - shown > 30:
            print(f"  {label}: state={state} progress={(body or {}).get('progress')}",
                  flush=True)
            last = state
            shown = time.time()
        if state in {"succeeded", "failed"}:
            if state == "failed":
                print(f"  {label}: error {json.dumps((body or {}).get('error'))}", flush=True)
            return body
        time.sleep(3)
    fail(f"job {job_id} of {label} never ended")


def cmd_create(args) -> None:
    out = run_dir(args.run)
    keywords = [
        "fonds", "etat", "code", "num", "texte_cid", "section",
        "juridiction", "numero", "nature", "theme", "type",
    ]
    config = {
        "name": INDEX,
        "model": "mdenseon",
        "reference_precision": "fp16",
        "schema": {
            "document_fields": {name: {"type": "keyword"} for name in keywords}
            | {"annee": {"type": "integer"}},
            "passage_fields": {"partie": {"type": "keyword"}},
        },
        "cluster_by": [],
        "lexical": {"enabled": True, "language": "fr", "positions": False},
        "search": {
            "scan": "binary",
            "rescore_candidates": 5000,
            "exact_threshold": 5000,
            "candidates_per_branch": 100,
            "fusion": {"method": "rrf"},
        },
        "ingest": {"acceptance_sample": 200, "acceptance_min_cosine": 0.999},
        "compaction": {"dead_ratio": 0.1},
    }
    started = time.time()
    code, body = api("POST", "/v1/indexes", config, timeout=600)
    print(f"  POST /v1/indexes -> {code} {json.dumps(body)[:300]}", flush=True)
    if code != 201:
        fail("index creation failed")
    (out / "create.json").write_text(json.dumps(
        {"status": code, "body": body, "wall_s": round(time.time() - started, 1)},
        indent=2) + "\n")


def cmd_attach(args) -> None:
    out = run_dir(args.run)
    campaign = out / "attach" / "campaign.json"
    report = []
    if campaign.exists() and not args.reset:
        report = json.loads(campaign.read_text())
    lots = args.lot or LOTS
    unknown = [lot for lot in lots if lot not in LOTS]
    if unknown:
        fail(f"unknown lot {unknown[0]}")
    campaign_started = time.time()
    for lot in lots:
        print(f"  attach {LOTS.index(lot) + 1}/{len(LOTS)} {lot}", flush=True)
        started = time.time()
        code, body = api("POST", f"/v1/indexes/{INDEX}/batches",
                         {"directory": lot}, timeout=180)
        print(f"  POST batches -> {code} {json.dumps(body)[:200]}", flush=True)
        if code != 202:
            fail(f"attach of {lot} refused")
        job = poll(body["id"], lot)
        report = [row for row in report if row["lot"] != lot]
        report.append({"lot": lot, "job": job, "wall_s": round(time.time() - started, 1),
                       "campaign_s": round(time.time() - campaign_started, 1)})
        report.sort(key=lambda row: LOTS.index(row["lot"]))
        print(f"  {lot}: {job.get('state')} in {time.time() - started:.1f} s "
              f"{json.dumps(job.get('result'))[:300]}", flush=True)
        (out / "attach").mkdir(parents=True, exist_ok=True)
        (out / "attach" / "campaign.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"  attach campaign done in {time.time() - campaign_started:.1f} s", flush=True)


def cmd_queries(args) -> None:
    out = run_dir(args.run)
    entries = [json.loads(line) for line in QUERIES.read_text().splitlines() if line.strip()]
    report = []
    durations = []
    started = time.time()
    for number, entry in enumerate(entries, 1):
        at = time.time()
        code, body = api("POST", f"/v1/indexes/{INDEX}/search",
                         {"query": entry["question"], "mode": "hybrid", "k": 10},
                         timeout=120)
        wall = (time.time() - at) * 1000
        durations.append(wall)
        report.append({"id": entry["id"], "status": code,
                       "took_ms": (body or {}).get("took_ms"),
                       "wall_ms": round(wall, 1),
                       "pool_size": (body or {}).get("pool_size")})
        if code != 200:
            print(f"  query {entry['id']} -> {code} {json.dumps(body)[:200]}", flush=True)
        if number % 50 == 0:
            print(f"  queries {number}/{len(entries)}", flush=True)
    (out / "queries").mkdir(parents=True, exist_ok=True)
    (out / "queries" / "pass.json").write_text(json.dumps(report, indent=2) + "\n")
    failures = [row for row in report if row["status"] != 200]
    missing_pools = [row["id"] for row in report if row["pool_size"] is None]
    if missing_pools:
        fail(f"pool_size missing for {len(missing_pools)} queries: {missing_pools[:5]}")
    ordered = sorted(durations)
    def percentile(value: float) -> float:
        return ordered[max(0, math.ceil(value / 100 * len(ordered)) - 1)]
    print(f"  {len(entries)} queries, {len(failures)} failures, "
          f"p50 {percentile(50):.1f} ms p95 {percentile(95):.1f} ms "
          f"p99 {percentile(99):.1f} ms, total {time.time() - started:.1f} s", flush=True)
    print(f"  pool_size min={min(row['pool_size'] for row in report)} "
          f"max={max(row['pool_size'] for row in report)}", flush=True)


def cmd_delete(args) -> None:
    out = run_dir(args.run)
    started = time.time()
    code, body = api("DELETE", f"/v1/indexes/{INDEX}?confirm={INDEX}", timeout=180)
    print(f"  DELETE -> {code} {json.dumps(body)[:200]}", flush=True)
    if code != 202:
        fail("delete refused")
    job = poll(body["id"], "delete_index")
    (out / "delete.json").write_text(json.dumps(
        {"job": job, "wall_s": round(time.time() - started, 1)}, indent=2) + "\n")
    print(f"  delete job: {job.get('state')} in {time.time() - started:.1f} s", flush=True)


def cmd_restart(args) -> None:
    out = run_dir(args.run)
    started = time.time()
    result = sh("docker", "restart", "-t", "65", CONTAINER, timeout=300)
    print(f"  docker restart -> {result.returncode} {result.stdout.strip()}", flush=True)
    elapsed = wait_ready()
    record = {"restart_s": round(time.time() - started, 1), "ready_s": round(elapsed, 1)}
    history = out / "restarts.json"
    previous = json.loads(history.read_text()) if history.exists() else []
    previous.append(record)
    history.write_text(json.dumps(previous, indent=2) + "\n")
    print(f"  ready after restart in {elapsed:.1f} s", flush=True)


def cmd_summary(args) -> None:
    out = run_dir(args.run)
    readings = {}
    for key_name in STATE_KEYS:
        path = out / "states" / f"{key_name}.json"
        if not path.exists():
            fail(f"missing state {key_name}")
        readings[key_name] = json.loads(path.read_text())

    def values(reading: dict) -> dict:
        status = reading["status"]
        stat = reading["memory_stat"]
        rollup = reading["smaps_rollup"]
        return {
            "current": reading["memory_current_bytes"] / PAGE,
            "anon": stat["anon"] / PAGE,
            "file": stat["file"] / PAGE,
            "unvented": (reading["memory_current_bytes"]
                         - stat["anon"] - stat["file"]) / PAGE,
            "rss_anon": int(status["RssAnon"].split()[0]) / 1024,
            "rss_file": int(status["RssFile"].split()[0]) / 1024,
            "rss": int(status["VmRSS"].split()[0]) / 1024,
            "rss_shmem": int(status["RssShmem"].split()[0]) / 1024,
            "hwm": int(status["VmHWM"].split()[0]) / 1024,
            "vmswap": int(status["VmSwap"].split()[0]) / 1024,
            "swap_current": reading["memory_swap_current_bytes"] / PAGE,
            "swapcached": stat["swapcached"] / PAGE,
            "zswapped": stat["zswapped"] / PAGE,
            "arena_count": reading["arenas"]["count"],
            "arena_rss": reading["arenas"]["rss_kb"] / 1024,
            "pathless_rss": reading["pathless"]["rss_kb"] / 1024,
            "slab": stat["slab"] / PAGE,
            "pagetables": stat["pagetables"] / PAGE,
            "kernel_stack": stat["kernel_stack"] / PAGE,
            "shmem": stat["shmem"] / PAGE,
            "file_mapped": stat["file_mapped"] / PAGE,
            "smaps_rss": int(rollup["Rss"].split()[0]) / 1024,
            "smaps_anon": int(rollup["Anonymous"].split()[0]) / 1024,
            "mem_available": reading["meminfo"]["MemAvailable"] / 1024,
            "swap_free": reading["meminfo"]["SwapFree"] / 1024,
        }

    table = []
    for key_name, label in STATES:
        rows = [values(reading) for reading in readings[key_name]]
        medians = {}
        spreads = {}
        for name in rows[0]:
            column = [row[name] for row in rows]
            medians[name] = round(statistics.median(column), 1)
            spreads[name] = round(max(column) - min(column), 1)
        table.append({
            "state": key_name,
            "label": label,
            "at": [reading["at"] for reading in readings[key_name]],
            "median": medians,
            "spread": spreads,
            "arenas": {
                "count": medians["arena_count"],
                "rss_mib": medians["arena_rss"],
                "groups": [reading["arenas"]["groups"] for reading in readings[key_name]],
            },
        })

    transitions = []
    pairs = [
        ("a->b", "a-fresh", "b-index-open", "index creation, session load"),
        ("b->c", "b-index-open", "c-after-attach", "six lots attached"),
        ("c->d", "c-after-attach", "d-after-queries", "300 queries, full index"),
        ("d->e", "d-after-queries", "e-after-restart-index", "restart with index"),
        ("e->f", "e-after-restart-index", "f-after-delete", "DELETE"),
        ("f->g", "f-after-delete", "g-after-restart", "restart without index"),
    ]
    by_state = {row["state"]: row for row in table}
    for name, before, after, what in pairs:
        delta = {}
        for field in ("current", "anon", "file", "rss_anon", "rss_file", "vmswap",
                      "swap_current", "arena_count", "arena_rss", "pathless_rss"):
            if field == "arena_count":
                delta[field] = by_state[after]["median"][field] - by_state[before]["median"][field]
            else:
                delta[field] = round(
                    by_state[after]["median"][field] - by_state[before]["median"][field], 1)
        transitions.append({"transition": name, "what": what,
                            "from": before, "to": after, "delta": delta})

    queries = None
    pass_file = out / "queries" / "pass.json"
    if pass_file.exists():
        rows = json.loads(pass_file.read_text())
        walls = sorted(row["wall_ms"] for row in rows)
        def percentile(value: float) -> float:
            return walls[max(0, math.ceil(value / 100 * len(walls)) - 1)]
        pools = [row["pool_size"] for row in rows]
        if any(value is None for value in pools):
            fail("queries/pass.json has rows without pool_size")
        queries = {
            "count": len(rows),
            "failures": sum(1 for row in rows if row["status"] != 200),
            "wall_ms": {"p50": percentile(50), "p95": percentile(95),
                        "p99": percentile(99), "max": walls[-1]},
            "pool_size": {"min": min(pools), "max": max(pools)},
        }

    summary = {
        "run": json.loads((out / "run.json").read_text()),
        "states": table,
        "transitions": transitions,
        "queries": queries,
        "written_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
    }
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")

    print(f"{'state':22} {'current':>9} {'anon':>9} {'file':>9} {'rss_anon':>9} "
          f"{'vmswap':>7} {'arenas':>7} {'arena_rss':>9} {'disp':>5}", flush=True)
    for row in table:
        med = row["median"]
        spread = max(row["spread"][name] for name in
                     ("current", "anon", "file", "rss_anon", "arena_rss"))
        print(f"{row['state']:22} {med['current']:9.1f} {med['anon']:9.1f} "
              f"{med['file']:9.1f} {med['rss_anon']:9.1f} {med['vmswap']:7.1f} "
              f"{int(med['arena_count']):7d} {med['arena_rss']:9.1f} {spread:5.1f}",
              flush=True)
    for row in transitions:
        delta = row["delta"]
        print(f"{row['transition']:22} {delta['anon']:+9.1f} anon "
              f"{delta['current']:+9.1f} current {delta['vmswap']:+7.1f} vmswap "
              f"arenas {delta['arena_count']:+d}/{delta['arena_rss']:+.1f} MiB", flush=True)
    print(f"summary written to {(out / 'summary.json').relative_to(REPO)}", flush=True)


def cmd_journal_check(args) -> None:
    out = run_dir(args.run)
    cumulative = {}
    final = []
    for key_name in STATE_KEYS:
        path = out / "logs" / f"{key_name}.jsonl"
        if not path.exists():
            fail(f"missing journal {key_name}")
        lines = memory_lines(path.read_text())
        counts = {}
        for line in lines:
            counts[line["step"]] = counts.get(line["step"], 0) + 1
        cumulative[key_name] = counts
        final = lines

    steps = {line["step"] for line in final}
    required = {"job_end", "delete_before_rename", "delete_after_removal", "session_load"}
    job_ends = [line for line in final if line["step"] == "job_end"]
    deletes = [line for line in final if line["step"] in
               {"delete_before_rename", "delete_after_removal"}]
    sessions = [line for line in final if line["step"] == "session_load"]

    attach_file = out / "attach" / "campaign.json"
    attach_jobs = []
    if attach_file.exists():
        attach_jobs = [row["job"]["id"] for row in json.loads(attach_file.read_text())]
    delete_file = out / "delete.json"
    delete_job = None
    if delete_file.exists():
        delete_job = json.loads(delete_file.read_text())["job"]["id"]

    report = {
        "states": cumulative,
        "steps_present": sorted(steps),
        "steps_missing": sorted(required - steps),
        "job_end": {
            "count": len(job_ends),
            "kinds": sorted({line["kind"] for line in job_ends if line["kind"]}),
            "attach_lots_seen": sum(1 for job in attach_jobs
                                    if any(line["job_id"] == job for line in job_ends)),
            "attach_lots_expected": len(attach_jobs),
            "delete_seen": bool(delete_job and any(
                line["job_id"] == delete_job for line in job_ends)),
        },
        "delete_steps": {
            "count": len(deletes),
            "same_job": len({line["job_id"] for line in deletes}) == 1,
            "job_id": deletes[0]["job_id"] if deletes else None,
        },
        "session_load": {
            "count": len(sessions),
            "indexes": [line["index"] for line in sessions],
            "at_b": cumulative["b-index-open"].get("session_load", 0),
            "at_e": cumulative["e-after-restart-index"].get("session_load", 0),
            "at_g": cumulative["g-after-restart"].get("session_load", 0),
        },
        "verdict": {
            "four_steps": not (required - steps),
            "one_job_end_per_lot": bool(attach_jobs)
            and sum(1 for job in attach_jobs
                    if any(line["job_id"] == job for line in job_ends)) == len(attach_jobs),
            "two_session_load": len(sessions) == 2,
        },
    }
    (out / "journal-check.json").write_text(json.dumps(report, indent=2) + "\n")
    (out / "memory-lines.json").write_text(json.dumps(final, indent=2) + "\n")
    print(json.dumps(report, indent=2), flush=True)


def cmd_tunables(args) -> None:
    out = run_dir(args.out)
    if not re.fullmatch(r"[a-z0-9-]+", args.name):
        fail(f"invalid output name {args.name!r}")
    if args.tunables is not None:
        value = args.tunables
    elif args.variant in VARIANTS:
        value = VARIANTS[args.variant]
    else:
        fail(f"unknown variant {args.variant}; pass --tunables")
    # The image has no shell: the dynamic loader itself is the entry point, so
    # no server is started and nothing but `--list-tunables` runs.
    command = ["docker", "run", "--rm", "--pull=never", "--entrypoint", LOADER]
    if value:
        command += ["-e", f"GLIBC_TUNABLES={value}"]
    command += [args.image, "--list-tunables"]
    result = sh(*command, timeout=120)
    if result.returncode != 0 or "glibc." not in result.stdout:
        fail(f"tunables probe failed ({result.returncode}): {result.stderr[-500:]}")
    # `image_id()` reads the campaign container's image, which no longer exists
    # after `down`; ask Docker for the image itself, by name.
    inspected = sh("docker", "image", "inspect", "-f", "{{.Id}}", args.image,
                   timeout=120)
    image_id_value = inspected.stdout.strip()
    if inspected.returncode != 0 or not image_id_value:
        fail(f"image {args.image} not found or has no identifier")
    (out / f"{args.name}.txt").write_text(result.stdout)
    document = {
        "variant": args.variant,
        "glibc_tunables": value,
        "image": args.image,
        "image_id": image_id_value,
        "loader": LOADER,
        "command": command,
        "written_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
    }
    (out / f"{args.name}.json").write_text(json.dumps(document, indent=2) + "\n")
    print(f"  {args.name}: glibc_tunables={value!r} in "
          f"{(out / f'{args.name}.txt').relative_to(REPO)}", flush=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    up = sub.add_parser("up", help="create the container (variant applied here only)")
    up.add_argument("--variant", default="V0")
    up.add_argument("--tunables", default=None)
    up.add_argument("--image", default="crible:t05-suite-mes17")
    up.set_defaults(func=cmd_up)

    down = sub.add_parser("down", help="stop and remove the container")
    down.set_defaults(func=cmd_down)

    init = sub.add_parser("init", help="record machine, image, variant, baseline")
    init.add_argument("--run", required=True)
    init.add_argument("--variant", default=None)
    init.add_argument("--tunables", default=None)
    init.add_argument("--image", default="crible:t05-suite-mes17")
    init.set_defaults(func=cmd_init)

    state = sub.add_parser("state", help="read and record one state")
    state.add_argument("name")
    state.add_argument("--run", required=True)
    state.add_argument("--reps", type=int, default=3)
    state.add_argument("--gap", type=float, default=3.0)
    state.set_defaults(func=cmd_state)

    create = sub.add_parser("create", help="create the throwaway index")
    create.add_argument("--run", required=True)
    create.set_defaults(func=cmd_create)

    attach = sub.add_parser("attach", help="attach the six derived lots")
    attach.add_argument("--run", required=True)
    attach.add_argument("--lot", action="append",
                        help="attach only this lot (repeatable); default: all six")
    attach.add_argument("--reset", action="store_true",
                        help="start the campaign report from scratch")
    attach.set_defaults(func=cmd_attach)

    queries = sub.add_parser("queries", help="300 queries, hybrid, k = 10")
    queries.add_argument("--run", required=True)
    queries.set_defaults(func=cmd_queries)

    delete = sub.add_parser("delete", help="delete the throwaway index")
    delete.add_argument("--run", required=True)
    delete.set_defaults(func=cmd_delete)

    restart = sub.add_parser("restart", help="docker restart -t 65 then wait /ready")
    restart.add_argument("--run", required=True)
    restart.set_defaults(func=cmd_restart)

    summary = sub.add_parser("summary", help="medians, spreads, transitions")
    summary.add_argument("--run", required=True)
    summary.set_defaults(func=cmd_summary)

    check = sub.add_parser("journal-check", help="four steps, six job_end, two session_load")
    check.add_argument("--run", required=True)
    check.set_defaults(func=cmd_journal_check)

    tun = sub.add_parser("tunables", help="prove glibc read the tunables (loader only)")
    tun.add_argument("--out", required=True, help="directory under data/mes17/raw")
    tun.add_argument("--variant", default="V0", help="variant whose value is passed")
    tun.add_argument("--tunables", default=None,
                     help="value overriding the variant (V4, or a proof)")
    tun.add_argument("--name", default="tunables", help="base name of the two output files")
    tun.add_argument("--image", default="crible:t05-suite-mes17")
    tun.set_defaults(func=cmd_tunables)

    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
