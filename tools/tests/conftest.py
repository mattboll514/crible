# SPDX-License-Identifier: Apache-2.0

"""Shared fixtures of the ``tools/`` tests."""

from __future__ import annotations

import pytest

from gen_synth import build_small


@pytest.fixture(scope="session")
def synth_dataset(tmp_path_factory):
    """Builds the real ``synth-small`` once for the whole test session."""
    root = tmp_path_factory.mktemp("synth-small")
    return build_small(root)
