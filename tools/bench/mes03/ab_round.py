#!/usr/bin/env python3
"""One alternated round of a rescoring A/B (MES-03).

A round visits the variants **in the order given** and, for each of them, each
``N`` of the series. Two rounds with the order reversed give the alternation the
tasks ask for; the script itself never launches anything in the background, so
the round ends when the last ``rescore_ab`` process exits.

For every invocation it records the machine's conditions and the page state of
both files the rescoring reads (``records.sqlite`` and the ``f16`` vectors),
before and after the run, with ``residency.py``.

Two campaigns use it:

- ``REDUIRE-COUT-DES-CLES`` (ADR-0079): the hot series, ``base`` against the
  variants, no cold flag;
- ``GROUPER-LECTURES-VECTEURS`` (ADR-0080): the hot series, the cold series by
  cache displacement **after the searcher is opened** (``--cold`` with
  ``--displace-gb`` and a file of ``--displace-source`` flags, as MES-03 writes
  them), the 10-minute idle series (``--idle-seconds 600``), and the contention
  series of the shared rayon pool (``--mode contend``).

    python3 tools/bench/mes03/ab_round.py --round 1 --order base,grouped \
        --ns 1000,5000 --passes 3 --raw data/grouper-vecteurs/raw --tag crib \
        --bin-dir data/grouper-vecteurs/bin \
        --vectors-file data/mes02/indexes/synth-large/epochs/1/segments/seg-large-0001/vectors.f16 \
        --records-file data/mes02/indexes/synth-large/epochs/1/records.sqlite
"""

from __future__ import annotations

import argparse
import json
import select
import shlex
import subprocess
import sys
import time
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


def displacement_sources(path: Path | None) -> list[str]:
    """``--displace-source`` flags of a one-line file, as MES-03 writes them.

    The file holds the flags themselves, so a recipe can extend them without
    quoting: ``--displace-source a --displace-source b``. Splitting it with
    ``shlex`` gives the arguments back, and nothing of the file is executed.
    """
    if path is None:
        return []
    return shlex.split(path.read_text(encoding="utf-8").strip())


def invocation_command(
    binary: Path,
    *,
    mode: str,
    data: str,
    index: str,
    vectors: str,
    n: int,
    passes: int,
    out: Path,
    queries: int | None = None,
    cold: bool = False,
    vectors_file: Path | None = None,
    displace_gb: float | None = None,
    displace_sources: list[str] | None = None,
    idle_seconds: int | None = None,
    sample_rss: bool = False,
) -> list[str]:
    """The exact ``rescore_ab`` command of one point, flags in a fixed order."""
    command = [
        str(binary),
        mode,
        "--data",
        data,
        "--index",
        index,
        "--vectors",
        vectors,
        "--ns",
        str(n),
        "--passes",
        str(passes),
        "--out",
        str(out),
    ]
    if queries:
        command += ["--queries", str(queries)]
    if cold:
        command += ["--cold"]
    if vectors_file is not None:
        command += ["--vectors-file", str(vectors_file)]
    if displace_gb is not None:
        command += ["--displace-gb", str(displace_gb)]
        command += displace_sources or []
    if idle_seconds is not None:
        command += ["--idle-seconds", str(idle_seconds)]
    if sample_rss:
        command += ["--sample-rss"]
    return command


def run_streaming(
    command: list[str], cwd: Path, heartbeat: float = 30.0
) -> tuple[int, str]:
    """Runs ``command`` in the foreground, streaming its output.

    The 10-minute idle series of ADR-0077 waits **inside** the harness, so a
    wrapper that only prints at the end leaves ten silent minutes: a session
    watching the run cannot tell a wait from a hang, and it may be cut. The
    output is therefore read as it comes, and a heartbeat line is printed when
    nothing has been said for ``heartbeat`` seconds.
    """
    started = time.monotonic()
    last = started
    collected: list[str] = []
    with subprocess.Popen(
        command,
        cwd=cwd,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    ) as process:
        assert process.stdout is not None
        while True:
            ready, _, _ = select.select([process.stdout], [], [], heartbeat)
            if ready:
                line = process.stdout.readline()
                if line:
                    collected.append(line.rstrip("\n"))
                    print(line.rstrip("\n"), flush=True)
                    last = time.monotonic()
                elif process.poll() is not None:
                    break
            else:
                print(
                    f"  … still running, {time.monotonic() - started:.0f} s "
                    f"(silent for {time.monotonic() - last:.0f} s)",
                    flush=True,
                )
        return process.wait(), "\n".join(collected)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--round", required=True)
    parser.add_argument("--order", required=True)
    parser.add_argument("--ns", default="1000,5000")
    parser.add_argument("--passes", type=int, default=3)
    parser.add_argument("--mode", default="measure", choices=("measure", "contend"))
    parser.add_argument("--raw", default="data/reduire-cles/raw")
    parser.add_argument("--tag", default="ab")
    parser.add_argument("--bin-dir", default="data/reduire-cles/bin")
    parser.add_argument("--data", default="data/mes02")
    parser.add_argument("--index", default="synth-large")
    parser.add_argument("--vectors", default="data/mes02/queries.f32")
    parser.add_argument("--queries", type=int, default=None)
    parser.add_argument("--vectors-file", required=True)
    parser.add_argument("--records-file", required=True)
    parser.add_argument("--cold", action="store_true")
    parser.add_argument("--displace-gb", type=float, default=None)
    parser.add_argument(
        "--displace-sources",
        default=None,
        help="file holding the --displace-source flags, one line, unquoted",
    )
    parser.add_argument("--idle-seconds", type=int, default=None)
    parser.add_argument("--sample-rss", action="store_true")
    arguments = parser.parse_args()

    raw = Path(arguments.raw)
    raw.mkdir(parents=True, exist_ok=True)
    order = [name.strip() for name in arguments.order.split(",") if name.strip()]
    ns = [int(text) for text in arguments.ns.split(",")]
    vectors_file = Path(arguments.vectors_file)
    records_file = Path(arguments.records_file)
    sources = displacement_sources(
        Path(arguments.displace_sources) if arguments.displace_sources else None
    )
    if arguments.displace_gb is not None and not sources:
        raise SystemExit("--displace-gb needs --displace-sources")

    label = f"{arguments.tag}-r{arguments.round}"
    state = {
        "label": label,
        "order": order,
        "ns": ns,
        "passes": arguments.passes,
        "mode": arguments.mode,
        "cold": arguments.cold,
        "displace_gb": arguments.displace_gb,
        "displace_sources": sources,
        "idle_seconds": arguments.idle_seconds,
        "sample_rss": arguments.sample_rss,
    }
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
            command = invocation_command(
                binary,
                mode=arguments.mode,
                data=arguments.data,
                index=arguments.index,
                vectors=arguments.vectors,
                n=n,
                passes=arguments.passes,
                out=out,
                queries=arguments.queries,
                cold=arguments.cold,
                vectors_file=vectors_file,
                displace_gb=arguments.displace_gb,
                displace_sources=sources,
                idle_seconds=arguments.idle_seconds,
                sample_rss=arguments.sample_rss,
            )
            print("+", " ".join(command), flush=True)
            code, output = run_streaming(command, ROOT)
            if code != 0:
                raise SystemExit(
                    f"{stem}: rescore_ab failed with {code}: {output.strip()}"
                )
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
                "stdout": [line for line in output.splitlines() if line.strip()],
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
