# SPDX-License-Identifier: Apache-2.0

"""Encoding of passage vectors.

A vector column is a ``FIXED_SIZE_BINARY`` of ``dims × 2`` bytes in fp16 or
``dims × 4`` bytes in fp32, little-endian, in dimension order
(``docs/format-lot-v1.md``, section "Vecteurs").
"""

from __future__ import annotations

from typing import Any

import numpy as np

from .errors import BatchError

# Tolerance of control 7 for the L2 norm (`docs/04-donnees-et-ingestion.md`).
NORM_TOLERANCE = 1e-2


def encode_vector(
    values: Any,
    *,
    dims: int,
    vector_dtype: str,
    normalize: bool,
    path: str = "vector",
) -> bytes:
    """Checks one vector and returns its stored little-endian bytes.

    The checks apply to the stored values, not to the caller's values, so they
    reproduce control 7 of ``docs/04`` exactly: finite components, and an L2
    norm close to 1 when ``normalize`` is true.
    """
    if vector_dtype not in ("fp16", "fp32"):
        raise BatchError(f"unknown vector_dtype '{vector_dtype}'")  # pragma: no cover
    array = _as_array(values, dims, path)
    target = np.float16 if vector_dtype == "fp16" else np.float32
    stored = array.astype(target)
    if not np.all(np.isfinite(stored)):
        raise BatchError(f"{path} has a component that is not finite after storage")
    if normalize:
        norm = float(np.sqrt(np.sum(stored.astype(np.float64) ** 2)))
        if abs(norm - 1.0) > NORM_TOLERANCE:
            raise BatchError(
                f"{path} has an L2 norm of {norm:.6f}; the index normalizes vectors, "
                f"so the norm must satisfy |norm - 1| <= {NORM_TOLERANCE}"
            )
    little_endian = np.dtype("<f2") if vector_dtype == "fp16" else np.dtype("<f4")
    return stored.astype(little_endian, copy=False).tobytes()


def _as_array(values: Any, dims: int, path: str) -> np.ndarray:
    try:
        array = np.asarray(values, dtype=np.float64)
    except (TypeError, ValueError) as error:
        raise BatchError(f"{path} is not an array of numbers: {error}") from error
    if array.ndim != 1:
        raise BatchError(f"{path} must be one-dimensional, got shape {array.shape}")
    if array.shape[0] != dims:
        raise BatchError(
            f"{path} has {array.shape[0]} dimensions; the model declares {dims}"
        )
    if not np.all(np.isfinite(array)):
        raise BatchError(f"{path} has a component that is not finite")
    return array
