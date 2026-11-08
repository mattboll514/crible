#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Build a small batch that INV-12 must refuse on another model.

The batch copies the Parquet files of a reference batch and changes only its
manifest: a new `batch_id` and another `model.hf_id`. The model check runs
before the acceptance check and before the file checksums, so no vector is
re-encoded and no query text is involved.

    python3 tools/bench/p10-macmini/build_refus_batch.py \
        --source data/legifacile-full/dole/dole-full-v1 \
        --out data/p10-macmini/pass-c/refus/dole-refus-model
"""

from __future__ import annotations

import argparse
import json
import shutil
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--batch-id", default="dole-refus-model")
    parser.add_argument("--hf-id", default="lightonai/other")
    args = parser.parse_args()

    source = Path(args.source)
    target = Path(args.out)
    if target.exists():
        shutil.rmtree(target)
    shutil.copytree(source, target)

    manifest_path = target / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    manifest["batch_id"] = args.batch_id
    manifest["created_at"] = "2026-11-07T00:00:00Z"
    manifest["model"]["hf_id"] = args.hf_id
    manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    print(f"built {target} from {source}: hf_id {manifest['model']['hf_id']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
