# SPDX-License-Identifier: Apache-2.0

"""Python package of Crible: batch writer for the public batch format v1.

``BatchWriter`` writes the Parquet files and the manifest of a batch
(``docs/format-lot-v1.md``). The HTTP client arrives in phase P6.
"""

from ._version import __version__
from .batch import MAX_PASSAGES_PER_PART, BatchWriter
from .errors import BatchError

__all__ = ["MAX_PASSAGES_PER_PART", "BatchError", "BatchWriter", "__version__"]
