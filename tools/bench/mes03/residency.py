#!/usr/bin/env python3
"""Resident-page fraction of a file, by ``mincore`` (MES-03, ADR-0077).

The MES-03 replay of ``REDUIRE-COUT-DES-CLES`` has to declare the page state of
the two files the rescoring reads — ``records.sqlite`` and the ``f16`` vectors —
for every series, because at N = 5000 their working set exceeds the RAM and the
kernel, not the code, decides which one the cache serves (MES-03, reserve 2 of
the warm profile).

This script maps a file read-only and asks ``mincore`` for every page. It prints
one JSON object: the file size, the page size, the number of pages, the number
resident, and the fraction.

    python3 tools/bench/mes03/residency.py data/mes02/indexes/synth-large/...

Read-only: the mapping is ``MAP_SHARED | PROT_READ``, nothing is written.
"""

from __future__ import annotations

import argparse
import ctypes
import json
import mmap
import os
import sys

#: ``mincore(2)`` returns one byte per page, bit 0 = resident.
MINCORE = ctypes.CDLL(None, use_errno=True).mincore


def residency(path: str) -> dict:
    """Returns the resident fraction of ``path``, page by page."""
    page = mmap.PAGESIZE
    size = os.path.getsize(path)
    pages = (size + page - 1) // page
    handle = open(path, "rb")
    try:
        # ``ACCESS_COPY`` rather than ``ACCESS_READ``: the mapping stays
        # read-only for the process and nothing is written to the file, but the
        # buffer is writable for ctypes, which needs it to hand out its address.
        mapping = mmap.mmap(handle.fileno(), 0, access=mmap.ACCESS_COPY)
        try:
            vector = (ctypes.c_ubyte * pages)()
            # ``mincore`` wants the page-aligned address of a mapping; the
            # mapping of ``mmap`` is page-aligned, and the length is rounded up
            # to the page, as the kernel requires.
            address = ctypes.addressof(ctypes.c_char.from_buffer(mapping))
            status = MINCORE(
                ctypes.c_void_p(address),
                ctypes.c_size_t(pages * page),
                vector,
            )
            if status != 0:
                error = ctypes.get_errno()
                raise OSError(error, os.strerror(error))
            resident = sum(1 for index in range(pages) if vector[index] & 1)
        finally:
            mapping.close()
    finally:
        handle.close()
    return {
        "file": path,
        "bytes": size,
        "page_size": page,
        "pages": pages,
        "resident_pages": resident,
        "resident_fraction": resident / pages if pages else 0.0,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("file", help="file to probe, read-only")
    parser.add_argument("--label", default=None, help="label written in the JSON")
    arguments = parser.parse_args()
    report = residency(arguments.file)
    if arguments.label:
        report["label"] = arguments.label
    json.dump(report, sys.stdout, ensure_ascii=False)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
