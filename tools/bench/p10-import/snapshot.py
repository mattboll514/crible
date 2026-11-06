#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Takes the offline snapshots of the campaign and checks them (ADR-0064, ADR-0065).

The server must be stopped: a snapshot reads a committed epoch and refuses to run
while the volume is open for writing (ADR-0064, decision 3). The command runs in a
named container with the data volume and the exchange directory mounted, records
its duration, the host `df` before and after, then checks the copy:

  * `snapshot.json` exists, `format_version` is 1, epoch and generation are those
    of the index;
  * the file list matches the files on disk exactly (nothing missing, nothing
    listed that is absent, nothing present that is unlisted);
  * every listed size matches the file on disk.

    python3 tools/bench/p10-import/snapshot.py dole sarde titres jorf articles decisions

The blake3 of each file is recomputed by `blake3-check.py`, which runs from
`tools/` with `uv run --no-sync` (the module is `blake3` of `tools/pyproject.toml`).
The CLI checks it again at restore, and the control restoration of `dole`
exercises that path.

`summary.json` merges the per-index JSON already on disk, so it always covers the
six index and never shrinks to the last call: `--summarize-only` rewrites it
without taking a snapshot.
"""

import argparse
import json
import pathlib
import subprocess
import sys
import time
from datetime import datetime, timezone

ROOT = pathlib.Path(__file__).resolve().parents[3]
OUT = ROOT / "data" / "p10-import" / "snapshots"
EXCHANGE = ROOT / "data" / "p10-exchange"


def now():
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def df():
    dec = subprocess.run(["df", "-H", "/"], capture_output=True, text=True, check=False)
    bib = subprocess.run(["df", "-h", "/"], capture_output=True, text=True, check=False)
    return {
        "at": now(),
        "H": dec.stdout.strip().splitlines()[-1],
        "h": bib.stdout.strip().splitlines()[-1],
    }


def index_state(index):
    done = subprocess.run(
        ["docker", "run", "--rm", "-v", "crible-p10-data:/data", "crible:p10",
         "inspect", "--data", "/data", "--index", index],
        capture_output=True, text=True, timeout=600, check=False,
    )
    if done.returncode != 0:
        raise RuntimeError(f"inspect failed for {index}: {done.stderr.strip()}")
    state = {}
    for line in done.stdout.splitlines():
        stripped = line.strip()
        if stripped.startswith("epoch:"):
            state["epoch"] = int(stripped.split(":")[1])
        elif stripped.startswith("generation:"):
            state["generation"] = int(stripped.split(":")[1])
        elif stripped.startswith("passages:"):
            state["passages"] = int(stripped.split(":")[1].split()[0])
    return state


def check(snapshot, index, state):
    manifest_path = snapshot / "snapshot.json"
    problems = []
    if not manifest_path.exists():
        return {"complete": False, "problems": ["snapshot.json is missing"]}, None
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if manifest.get("format_version") != 1:
        problems.append(f"format_version is {manifest.get('format_version')}")
    if manifest.get("index") != index:
        problems.append(f"index is {manifest.get('index')}")
    if manifest.get("epoch") != state["epoch"]:
        problems.append(f"epoch is {manifest.get('epoch')}, index is {state['epoch']}")
    if manifest.get("generation") != state["generation"]:
        problems.append(f"generation is {manifest.get('generation')}, index is {state['generation']}")

    listed = {entry["path"]: entry for entry in manifest["files"]}
    present = set()
    for path in snapshot.rglob("*"):
        if path.is_file():
            present.add(str(path.relative_to(snapshot)))
    present.discard("snapshot.json")

    missing = sorted(set(listed) - present)
    unlisted = sorted(present - set(listed))
    if missing:
        problems.append(f"{len(missing)} listed files are missing: {missing[:3]}")
    if unlisted:
        problems.append(f"{len(unlisted)} files are not listed: {unlisted[:3]}")

    size_mismatch = [
        path for path, entry in listed.items()
        if path in present and (snapshot / path).stat().st_size != entry["size"]
    ]
    if size_mismatch:
        problems.append(f"{len(size_mismatch)} sizes differ: {sorted(size_mismatch)[:3]}")

    total = sum(entry["size"] for entry in manifest["files"])
    return {
        "complete": not problems,
        "problems": problems,
        "format_version": manifest.get("format_version"),
        "epoch": manifest.get("epoch"),
        "generation": manifest.get("generation"),
        "files": len(listed),
        "bytes_listed": total,
        "files_present": len(present),
    }, manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("indexes", nargs="*")
    parser.add_argument("--summarize-only", action="store_true",
                        help="rewrite summary.json from the per-index JSON, take nothing")
    parser.add_argument("--image", default="crible:p10")
    args = parser.parse_args()
    if not args.indexes and not args.summarize_only:
        parser.error("give at least one index, or --summarize-only")
    OUT.mkdir(parents=True, exist_ok=True)
    results = []
    for index in args.indexes:
        state = index_state(index)
        before = df()
        name = f"p10-snapshot-{index}"
        started = time.monotonic()
        done = subprocess.run(
            ["docker", "run", "--name", name,
             "-v", "crible-p10-data:/data",
             "-v", f"{EXCHANGE}:/exchange",
             args.image, "snapshot", "--data", "/data", "--exchange", "/exchange",
             "--index", index],
            capture_output=True, text=True, timeout=1800, check=False,
        )
        duration = round(time.monotonic() - started, 1)
        after = df()
        code = subprocess.run(
            ["docker", "inspect", name, "--format", "{{.State.ExitCode}}"],
            capture_output=True, text=True, timeout=60, check=False,
        ).stdout.strip()

        directory = EXCHANGE / "snapshots" / f"{index}-e{state['epoch']}-g{state['generation']}"
        size = 0
        if directory.exists():
            size = sum(p.stat().st_size for p in directory.rglob("*") if p.is_file())
        verdict, _manifest = (check(directory, index, state) if directory.exists()
                              else ({"complete": False, "problems": ["directory is missing"]}, None))

        entry = {
            "index": index,
            "container": name,
            "exit_code": int(code) if code.lstrip("-").isdigit() else None,
            "duration_s": duration,
            "directory": str(directory.relative_to(ROOT)),
            "epoch": state["epoch"],
            "generation": state["generation"],
            "passages": state["passages"],
            "bytes": size,
            "gib": round(size / 1024 ** 3, 2),
            "df_before": before,
            "df_after": after,
            "completeness": verdict,
            "output": done.stdout.strip(),
            "stderr": done.stderr.strip()[-500:],
        }
        (OUT / f"{index}.json").write_text(
            json.dumps(entry, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        results.append(entry)
        print(f"{index}: {duration} s, {entry['gib']} GiB, epoch {state['epoch']} "
              f"generation {state['generation']}, exit {entry['exit_code']}, "
              f"complete {verdict['complete']}")
        for problem in verdict["problems"]:
            print(f"  PROBLEM {problem}")

    entries = {}
    for path in sorted(OUT.glob("*.json")):
        if path.name != "summary.json":
            known = json.loads(path.read_text(encoding="utf-8"))
            entries[known["index"]] = known
    for entry in results:
        entries[entry["index"]] = entry
    snapshots = [entries[name] for name in sorted(entries)]
    total = sum(entry["bytes"] for entry in snapshots)
    summary = {
        "at": now(),
        "snapshots": snapshots,
        "total_bytes": total,
        "total_gib": round(total / 1024 ** 3, 2),
        "all_complete": all(entry["completeness"]["complete"] for entry in snapshots),
        "df_after": df(),
    }
    (OUT / "summary.json").write_text(
        json.dumps(summary, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"total {summary['total_gib']} GiB, all complete {summary['all_complete']}")
    return 0 if summary["all_complete"] else 1


if __name__ == "__main__":
    sys.exit(main())
