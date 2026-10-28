#!/usr/bin/env python3
"""One alternated round of the ``REDUIRE-COUT-DES-CLES`` latency series.

A round visits the variants **in the order given** and, for each of them, each
``N`` of the series. Two rounds with the order reversed give the alternation the
task asks for; the script itself never launches anything in the background, so
the round ends when the last ``rescore_ab`` process exits.

For every invocation it records the machine's conditions and the page state of
both files the rescoring reads (``records.sqlite`` and the ``f16`` vectors),
before and after the run, with ``residency.py``.

    python3 tools/bench/mes03/ab_round.py --round 1 --order base,2a,2b,2a-2b \
        --ns 1000,5000 --passes 3 --raw data/reduire-cles/raw --tag ab
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def conditions() -> dict:
    """Load, swap, free pages and free disk, as text exactly as printed."""
    report = {}
    for name, command in (
        ("uptime", ["uptime"]),
        ("vm_stat", ["vm_stat"]),
        ("swap", ["sysctl", "vm.swapusage"]),
        ("disk", ["df", "-h", str(ROOT)]),
    ):
        completed = subprocess.run(command, capture_output=True, text=True)
        report[name] = completed.stdout.strip()
    return report


def residency(path: Path) -> dict:
    completed = subprocess.run(
        [sys.executable, str(HERE / "residency.py"), str(path)],
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(completed.stdout)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--round", required=True)
    parser.add_argument("--order", required=True)
    parser.add_argument("--ns", default="1000,5000")
    parser.add_argument("--passes", type=int, default=3)
    parser.add_argument("--raw", default="data/reduire-cles/raw")
    parser.add_argument("--tag", default="ab")
    parser.add_argument("--bin-dir", default="data/reduire-cles/bin")
    parser.add_argument("--data", default="data/mes02")
    parser.add_argument("--index", default="synth-large")
    parser.add_argument("--vectors", default="data/mes02/queries.f32")
    parser.add_argument("--queries", type=int, default=None)
    parser.add_argument("--vectors-file", required=True)
    parser.add_argument("--records-file", required=True)
    arguments = parser.parse_args()

    raw = Path(arguments.raw)
    raw.mkdir(parents=True, exist_ok=True)
    order = [name.strip() for name in arguments.order.split(",") if name.strip()]
    ns = [int(text) for text in arguments.ns.split(",")]
    vectors_file = Path(arguments.vectors_file)
    records_file = Path(arguments.records_file)

    label = f"{arguments.tag}-r{arguments.round}"
    state = {"label": label, "order": order, "ns": ns, "passes": arguments.passes}
    state["conditions_before"] = conditions()
    print(json.dumps({"conditions_before": state["conditions_before"]}), flush=True)

    invocations = []
    for variant in order:
        binary = Path(arguments.bin_dir) / variant / "rescore_ab"
        for n in ns:
            stem = f"{label}-{variant}-{n}"
            out = raw / f"{stem}.json"
            before = {
                "records": residency(records_file),
                "vectors": residency(vectors_file),
            }
            command = [
                str(binary),
                "measure",
                "--data",
                arguments.data,
                "--index",
                arguments.index,
                "--vectors",
                arguments.vectors,
                "--ns",
                str(n),
                "--passes",
                str(arguments.passes),
                "--out",
                str(out),
            ]
            if arguments.queries:
                command += ["--queries", str(arguments.queries)]
            print("+", " ".join(command), flush=True)
            completed = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
            if completed.returncode != 0:
                raise SystemExit(
                    f"{stem}: rescore_ab failed with {completed.returncode}: "
                    f"{completed.stderr.strip()}"
                )
            print(completed.stdout.strip(), flush=True)
            after = {
                "records": residency(records_file),
                "vectors": residency(vectors_file),
            }
            invocation = {
                "variant": variant,
                "n": n,
                "document": str(out),
                "residency_before": before,
                "residency_after": after,
                "stdout": completed.stdout.strip().splitlines(),
            }
            (raw / f"{stem}.residency.json").write_text(
                json.dumps(invocation, ensure_ascii=False, indent=1) + "\n",
                encoding="utf-8",
            )
            invocations.append(invocation)

    state["invocations"] = invocations
    state["conditions_after"] = conditions()
    (raw / f"{label}.conditions.json").write_text(
        json.dumps(state, ensure_ascii=False, indent=1) + "\n", encoding="utf-8"
    )
    print(json.dumps({"conditions_after": state["conditions_after"]}), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
