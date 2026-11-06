"""Make the harness modules importable from the tests.

``pytest`` inserts the test file's directory, not the harness root: the two
paths below add the harness itself and ``tools/bench/mes00/`` (whose
``mes00lib.py`` this harness reuses instead of copying its guards).
"""

from __future__ import annotations

import sys
from pathlib import Path

HARNESS = Path(__file__).resolve().parent.parent
MES00 = HARNESS.parent / "mes00"

for path in (HARNESS, MES00):
    if str(path) not in sys.path:
        sys.path.insert(0, str(path))
