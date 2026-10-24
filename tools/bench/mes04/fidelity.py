# SPDX-License-Identifier: Apache-2.0

"""Re-measure the document fidelity distribution that Q-06 leans on.

Criterion (ii) of ADR-0050 compares 100 passages per structure, re-encoded with
the ONNX fp32 graph and the ``document: `` prompt, against the production fp16
vectors of the sample. The same function wrote ``data/models/mdenseon/parity.json``
during `REEXPORTER-MODELE-TRANSFORMERS-5`; this script re-runs it with the model
directory as it stands, so the Q-06 proposal rests on a distribution measured in
this campaign too.

Run from ``tools/``::

    uv run python bench/mes04/fidelity.py
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

#: Repository root, three levels above this file.
ROOT = Path(__file__).resolve().parents[3]
# The exporter lives in ``tools/``; a direct run does not put it on the path.
sys.path.insert(0, str(ROOT / "tools"))

import export_onnx  # noqa: E402 - after the path bootstrap above

DEFAULT_MODEL = ROOT / "data/models/mdenseon"
DEFAULT_SAMPLE = ROOT / "data/legifacile-sample"
DEFAULT_OUT = ROOT / "data/mes04/fidelity.json"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--model", type=Path, default=DEFAULT_MODEL)
    parser.add_argument("--sample", type=Path, default=DEFAULT_SAMPLE)
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    arguments = parser.parse_args(argv)

    try:
        config = json.loads(
            (arguments.model / "model.json").read_text(encoding="utf-8")
        )
        tokenizer = export_onnx.load_tokenizer(arguments.model / "tokenizer.json")
        report = export_onnx.measure_fidelity(
            arguments.sample,
            arguments.model / export_onnx.FP32_PATH,
            tokenizer,
            config["revision"],
        )
        # The absolute sample path of the function is useful locally, not in a
        # committed report: keep only what the report needs.
        report.pop("sample", None)
        report["graph"] = {
            "revision": config["revision"],
            "export": config.get("export"),
            "fp32_blake3": config["files"]["fp32"]["blake3"],
        }
        arguments.out.parent.mkdir(parents=True, exist_ok=True)
        arguments.out.write_text(
            json.dumps(report, indent=2) + "\n", encoding="utf-8"
        )
    except Exception as error:  # noqa: BLE001 - every failure becomes one line
        print(f"error: {' '.join(str(error).split())}", file=sys.stderr)
        return 1
    print(
        f"{report['passages']} passages: min {report['min_cosine']:.9f}, "
        f"p1 {report['p1_cosine']:.9f}, median {report['median_cosine']:.9f}"
    )
    print(f"wrote {arguments.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
