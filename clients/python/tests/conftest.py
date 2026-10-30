# SPDX-License-Identifier: Apache-2.0

"""Fixtures of the Python tests of Crible.

The :func:`instance` fixture launches ``crible serve`` on a temporary instance:
a temporary data directory, two ``external`` models, an ``exchange`` directory,
a free port and the test keys of the three roles (ADR-0100). It waits for
``/ready`` and stops the server with ``SIGTERM``, which is the shutdown of
ADR-0099. Temporary directories come from ``tmp_path``: nothing is written
outside it.

Only the tests marked ``server`` use it; ``addopts`` excludes them by default
(ADR-0112).
"""

from __future__ import annotations

import json
import os
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Iterator

import httpx
import pytest

from helpers import (
    DIMS,
    EXTERNAL_MODEL,
    KEYS,
    WIDE_DIMS,
    WIDE_MODEL,
    crible_binary,
    free_port,
    sha256,
    write_model,
)

#: How long the fixture waits for `/ready` and for the server to stop.
START_TIMEOUT = 60.0
STOP_TIMEOUT = 60.0


@dataclass(frozen=True)
class Instance:
    """One running instance, as the tests see it."""

    base_url: str
    data: Path
    models: Path
    exchange: Path
    log: Path
    keys: dict[str, str]


def configuration(port: int, data: Path, models: Path, exchange: Path) -> str:
    """`crible.toml` of the fixture: every key of the closed list of ADR-0097."""
    keys = "\n".join(
        f'[[auth.keys]]\nname = "tests-{role}"\nrole = "{role}"\n'
        f'sha256 = "{sha256(key)}"\n'
        for role, key in KEYS.items()
    )
    return f"""[server]
bind = "127.0.0.1:{port}"
search_threads = 1
max_concurrent_searches = 2

[paths]
data = {json.dumps(str(data))}
models = {json.dumps(str(models))}
exchange = {json.dumps(str(exchange))}

{keys}
[analytics]
retention_days = 90

[debug_journal]
enabled = false
sample_rate = 0.0
ttl_days = 7

[encoder]
query_precision = "fp32"

[limits]
writer_queue = 16
default_timeout_ms = 2000
wait_timeout_ms = 60000
query_cache_entries = 1024
"""


def wait_ready(base_url: str, process: subprocess.Popen[bytes], log: Path) -> None:
    """Waits for `/ready` to answer `200`, and fails if the server exits first."""
    deadline = time.monotonic() + START_TIMEOUT
    while time.monotonic() < deadline:
        if process.poll() is not None:
            pytest.fail(
                f"crible serve exited with code {process.returncode}:\n{log.read_text()}"
            )
        try:
            if httpx.get(f"{base_url}/ready", timeout=1.0).status_code == 200:
                return
        except httpx.HTTPError:
            pass
        time.sleep(0.05)
    stop(process)
    pytest.fail(f"crible serve did not become ready within {START_TIMEOUT} s:\n{log.read_text()}")


def stop(process: subprocess.Popen[bytes]) -> None:
    """Stops the server with SIGTERM, then kills it if it does not stop."""
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=STOP_TIMEOUT)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


@pytest.fixture
def instance(tmp_path: Path) -> Iterator[Instance]:
    """A `crible serve` instance on temporary directories, stopped at teardown."""
    binary = crible_binary()
    data = tmp_path / "data"
    models = tmp_path / "models"
    exchange = tmp_path / "exchange"
    for directory in (data, models, exchange / "inbox", exchange / "outbox"):
        directory.mkdir(parents=True)
    write_model(models, EXTERNAL_MODEL, DIMS)
    write_model(models, WIDE_MODEL, WIDE_DIMS)

    port = free_port()
    config = tmp_path / "crible.toml"
    config.write_text(configuration(port, data, models, exchange), encoding="utf-8")
    log = tmp_path / "server.log"
    environment = {
        name: value for name, value in os.environ.items() if not name.startswith("CRIBLE_")
    }
    with log.open("wb") as stream:
        process = subprocess.Popen(
            [str(binary), "serve", "--config", str(config)],
            stdout=stream,
            stderr=subprocess.STDOUT,
            env=environment,
        )
        base_url = f"http://127.0.0.1:{port}"
        try:
            wait_ready(base_url, process, log)
            yield Instance(
                base_url=base_url,
                data=data,
                models=models,
                exchange=exchange,
                log=log,
                keys=dict(KEYS),
            )
        finally:
            stop(process)
