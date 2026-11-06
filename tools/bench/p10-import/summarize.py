#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Summary of the import campaign of IMPORTER-BASE-COMPLETE (P10).

Reads only the raw outputs written by `prepare.sh`, `import.py` and
`attach-all.sh` under `data/p10-import/`, and prints the tables of the report:

- per index, the durations of the attach, the time up to the commit point and
  the time after it, from the `attach durations` line of the server journal
  (ADR-0107, decision 4) crossed with the job's `started_at`/`finished_at`;
- per index, the counts read by `crible inspect` and by `GET /v1/indexes`,
  compared to the pgvector counts of the export;
- the `df` figures of the host before and after each attach.

    python3 tools/bench/p10-import/summarize.py

Nothing is extrapolated: a missing figure is printed as `—`.
"""

import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[3]
OUT = ROOT / "data" / "p10-import"

# Counts of pgvector, as recorded by EXPORTER-BASE-LEGIFACILE on 2026-11-05 and
# 2026-11-06 (`docs/suivi.md`), the reference of the acceptance criterion.
PGVECTOR = {
    "dole": (3_600, 7_653),
    "sarde": (210_965, 214_578),
    "titres": (1_477_883, 1_477_883),
    "jorf": (1_636_145, 1_833_500),
    "articles": (2_135_023, 2_476_715),
    "decisions": (1_196_623, 3_589_776),
}
ORDER = ["dole", "sarde", "titres", "jorf", "articles", "decisions"]


def load_job(index):
    path = OUT / "jobs" / f"{index}-final.json"
    if not path.exists():
        return None
    value = json.loads(path.read_text(encoding="utf-8"))
    return value.get("job", value)


def durations(index):
    """The last `attach durations` line of the journal, for one index."""
    log = OUT / "server.log"
    if not log.exists():
        return None
    found = None
    for line in log.read_text(encoding="utf-8").splitlines():
        _, _, rest = line.partition(" ")
        try:
            entry = json.loads(rest)
        except json.JSONDecodeError:
            continue
        fields = entry.get("fields", {})
        if fields.get("message") == "attach durations" and fields.get("index") == index:
            found = fields
    return found


def inspect(index):
    path = OUT / f"inspect-{index}.txt"
    if not path.exists():
        return None
    values = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if line.startswith("generation:"):
            values["generation"] = int(line.split(":")[1])
        elif line.startswith("epoch:"):
            values["epoch"] = int(line.split(":")[1])
        elif line.startswith("documents:"):
            values["documents"] = int(line.split(":")[1])
        elif line.startswith("passages:"):
            values["live_passages"] = int(line.split(":")[1].split()[0])
        elif line.startswith("segments:"):
            values["segments"] = int(line.split(":")[1])
    return values


def state(index):
    path = OUT / "index-state" / f"{index}.json"
    if not path.exists():
        return None
    return json.loads(path.read_text(encoding="utf-8"))["response"]["state"]


def poll_boundaries(index):
    """Step boundaries of a job, read from the polls of `GET /v1/jobs/{id}`.

    Used only when the attach was interrupted before its `attach durations`
    line: the boundaries are then accurate to the poll interval (10 s), and the
    report says so.
    """
    path = OUT / "polls" / f"{index}.jsonl"
    job = load_job(index)
    if not path.exists() or job is None:
        return None
    started = job["started_at"]
    from datetime import datetime

    def parse(text):
        return datetime.fromisoformat(text.replace("Z", "+00:00"))

    start = parse(started)
    boundaries = {}
    last = None
    for line in path.read_text(encoding="utf-8").splitlines():
        entry = json.loads(line)
        progress = entry["job"].get("progress")
        if not progress:
            continue
        step = progress["step"]
        if step != last:
            boundaries.setdefault(step, (parse(entry["at"]) - start).total_seconds())
            last = step
    # An OOM leaves `finished_at` at the restart that marked the job failed,
    # not at the kill: the kill time is the one of the container state.
    oom = OUT / "oom-articles.txt"
    end = parse(job["finished_at"])
    if oom.exists():
        for line in oom.read_text(encoding="utf-8").splitlines():
            if '"OOMKilled":true' in line:
                match = re.search(r'"FinishedAt":"([^"]+)"', line)
                if match:
                    end = parse(match.group(1))
    return boundaries, end - start


def host_figures():
    """Free space on the host, per labelled block of `df-host-per-lot.txt`."""
    path = OUT / "df-host-per-lot.txt"
    if not path.exists():
        return {}
    figures = {}
    label = None
    seen = 0
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("# "):
            label = line[2:]
            seen = 0
            continue
        if label and line.startswith("/dev/"):
            seen += 1
            units = "GiB" if seen == 1 else "Go"
            figures.setdefault(label, {})[units] = line.split()[3].rstrip("GiG")
    return figures


def main():
    print("| Index | Total | Jusqu'au commit | Après le commit | Docs | Passages | Segments | Gén. |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|")
    for index in ORDER:
        job = load_job(index)
        d = durations(index)
        inspected = inspect(index) or {}
        stateful = state(index) or {}
        if d:
            before = d["validation_ms"] + d["segment_ms"] + d["transaction_ms"]
            after = d["lexical_ms"] + d["manifest_ms"] + d["snapshot_ms"]
            total = before + after
            cells = [f"{total / 1000:.0f} s", f"{before / 1000:.0f} s", f"{after / 1000:.0f} s"]
        elif poll_boundaries(index):
            boundaries, total = poll_boundaries(index)
            commit = boundaries.get("lexical")
            if commit is None:
                cells = ["—", "—", "—"]
            else:
                cells = [
                    f"{total.total_seconds():.0f} s*",
                    f"{commit:.0f} s*",
                    f"{total.total_seconds() - commit:.0f} s*",
                ]
        else:
            cells = ["—", "—", "—"]
        documents = stateful.get("documents", inspected.get("documents"))
        passages = stateful.get("live_passages", inspected.get("live_passages"))
        segments = stateful.get("segments", inspected.get("segments"))
        generation = stateful.get("generation", inspected.get("generation"))
        print(
            f"| `{index}` | {cells[0]} | {cells[1]} | {cells[2]} | "
            f"{documents if documents is not None else '—'} | "
            f"{passages if passages is not None else '—'} | "
            f"{segments if segments is not None else '—'} | "
            f"{generation if generation is not None else '—'} |"
        )

    print()
    print("| Index | Documents pgvector | Documents importés | Passages pgvector | Passages importés |")
    print("|---|---:|---:|---:|---:|")
    total_documents = 0
    total_passages = 0
    for index in ORDER:
        stateful = state(index) or {}
        expected_documents, expected_passages = PGVECTOR[index]
        total_documents += expected_documents
        total_passages += expected_passages
        print(
            f"| `{index}` | {expected_documents} | {stateful.get('documents', '—')} | "
            f"{expected_passages} | {stateful.get('live_passages', '—')} |"
        )
    print(f"| **total** | **{total_documents}** | — | **{total_passages}** | — |")

    print()
    print("| Étape | Avant / après | Libre sur l'hôte |")
    print("|---|---|---:|")
    for label, figures in host_figures().items():
        print(f"| `{label}` | — | {figures.get('Go', '—')} Go ({figures.get('GiB', '—')} GiB) |")

    print()
    print("Espace des index dans le volume, `du -sh` :")
    for index in ORDER:
        stateful = state(index) or {}
        sizes = stateful.get("sizes", {})
        if "disk_bytes" in sizes:
            print(f"- `{index}` : {sizes['disk_bytes']} octets")

    return 0


if __name__ == "__main__":
    sys.exit(main())
