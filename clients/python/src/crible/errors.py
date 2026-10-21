# SPDX-License-Identifier: Apache-2.0

"""Errors raised while building a batch."""


class BatchError(Exception):
    """A batch breaks a rule of ``docs/format-lot-v1.md``.

    The writer raises this error before it writes the manifest, so a rejected
    batch is never presented as valid. The message names the offending value
    and where it comes from.
    """
