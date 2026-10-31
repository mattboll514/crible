# SPDX-License-Identifier: Apache-2.0

"""Python package of Crible.

``BatchWriter`` writes the Parquet files and the manifest of a batch
(``docs/format-lot-v1.md``). ``Client`` and ``AsyncClient`` are the HTTP client
of the ``/v1`` API (ADR-0112).
"""

from ._version import __version__
from .batch import MAX_PASSAGES_PER_PART, BatchWriter
from .client import AsyncClient, Client, encode_vector_base64
from .errors import (
    AcceptanceFailedError,
    BatchConflictError,
    BatchError,
    CribleError,
    CribleTransportError,
    ForbiddenError,
    ImmutableSettingError,
    IncompatibleModelError,
    InternalError,
    InvalidFilterError,
    InvalidRequestError,
    NotFoundError,
    OverloadedError,
    PayloadTooLargeError,
    RequestTimeoutError,
    UnauthorizedError,
    UnknownIndexError,
)

__all__ = [
    "AcceptanceFailedError",
    "AsyncClient",
    "BatchConflictError",
    "BatchError",
    "BatchWriter",
    "Client",
    "CribleError",
    "CribleTransportError",
    "ForbiddenError",
    "ImmutableSettingError",
    "IncompatibleModelError",
    "InternalError",
    "InvalidFilterError",
    "InvalidRequestError",
    "MAX_PASSAGES_PER_PART",
    "NotFoundError",
    "OverloadedError",
    "PayloadTooLargeError",
    "RequestTimeoutError",
    "UnauthorizedError",
    "UnknownIndexError",
    "__version__",
    "encode_vector_base64",
]
