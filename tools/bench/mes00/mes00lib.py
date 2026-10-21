"""Shared helpers for the MES-00 measurement scripts.

The scripts run on the production Mac mini, in read-only mode (ADR-0032).
This module only uses the standard library so that it can run with any
Python 3.11+ interpreter, including the one managed by Legifacile.

Nothing here connects to Postgres. The helpers guard environment variables,
guarded output paths, subprocess calls and percentile aggregation.
"""

from __future__ import annotations

import json
import math
import os
import re
import subprocess
import tempfile
from pathlib import Path
from typing import Any, Iterable, Mapping, Sequence

# /tmp and its macOS real path. Outputs are confined there unless the caller
# explicitly allows another root.
DEFAULT_ALLOWED_ROOTS = (Path("/tmp"), Path("/private/tmp"))

_READ_ONLY_SETTING = re.compile(
    r"(?:^|\s)(?:-c|--)?\s*default_transaction_read_only\s*=\s*([A-Za-z0-9]+)",
    re.IGNORECASE,
)
_PLACEHOLDER = re.compile(r"\{[a-z_]+\}")


class Mes00Error(Exception):
    """A measurement guard or an input contract failed."""


# ---------------------------------------------------------------------------
# Environment and output guards
# ---------------------------------------------------------------------------


def require_prod_pgoptions(environ: Mapping[str, str] | None = None) -> str:
    """Return $PROD_PGOPTIONS and refuse anything that is not read-only.

    The value itself is never logged, stored or included in an error message.
    Only the variable name appears in messages.
    """

    env = os.environ if environ is None else environ
    value = env.get("PROD_PGOPTIONS", "").strip()
    if not value:
        raise Mes00Error(
            "PROD_PGOPTIONS is not set; export the value from .env.prod before running"
        )
    if not _ends_read_only(value):
        raise Mes00Error(
            "PROD_PGOPTIONS must contain default_transaction_read_only=on (ADR-0032)"
        )
    return value


def _ends_read_only(value: str) -> bool:
    """Return True when the last read-only setting is ``on``.

    PostgreSQL applies repeated ``-c`` options in order, so a later ``=off``
    would cancel an earlier ``=on``.  Refuse that case instead of testing a
    substring of the whole value.
    """

    matches = _READ_ONLY_SETTING.findall(value)
    return bool(matches) and matches[-1].lower() == "on"


def require_env(name: str, environ: Mapping[str, str] | None = None) -> str:
    """Return a required environment variable without echoing its value."""

    env = os.environ if environ is None else environ
    value = env.get(name, "").strip()
    if not value:
        raise Mes00Error(f"{name} is not set; export the value from .env.prod")
    return value


def _resolve(path: Path) -> Path:
    # Resolve existing parents and symlinks without requiring the leaf to exist.
    try:
        return path.resolve(strict=False)
    except OSError as exc:  # pragma: no cover - defensive
        raise Mes00Error(f"cannot resolve output path: {exc}") from exc


def check_output_path(
    raw: str | os.PathLike[str],
    *,
    allow_roots: Sequence[str | os.PathLike[str]] = (),
    forbidden_roots: Sequence[str | os.PathLike[str]] = (),
) -> Path:
    """Validate a writable output path.

    The path must be absolute and must not contain a ``..`` component. It must
    live under /tmp (or under an explicitly allowed root) and must not live
    under a forbidden root such as $PROD_REPO or the Crible checkout.
    """

    path = Path(raw)
    if not path.is_absolute():
        raise Mes00Error(
            f"output path must be absolute and under /tmp: {path}"
        )
    if ".." in path.parts:
        raise Mes00Error(f"output path must not contain '..': {path}")

    candidate = _resolve(path)
    permitted = [_resolve(Path(root)) for root in DEFAULT_ALLOWED_ROOTS]
    for root in allow_roots:
        resolved_root = _resolve(Path(root))
        if not resolved_root.is_dir():
            raise Mes00Error(
                f"allowed root must be an existing directory: {resolved_root}"
            )
        permitted.append(resolved_root)

    if not any(candidate.is_relative_to(root) for root in permitted):
        roots = ", ".join(str(root) for root in permitted)
        raise Mes00Error(
            f"refusing to write outside {roots}: {candidate}"
        )

    for root in forbidden_roots:
        resolved_root = _resolve(Path(root))
        if candidate.is_relative_to(resolved_root):
            raise Mes00Error(
                f"refusing to write inside a protected repository: {candidate.name}"
            )

    return candidate


def prepare_output(path: Path) -> None:
    """Create the parent directory of a guarded output path."""

    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)


def write_json(path: Path, data: Any) -> None:
    """Write JSON atomically inside an already guarded directory."""

    prepare_output(path)
    payload = json.dumps(data, ensure_ascii=False, indent=2, sort_keys=True)
    fd, temporary = tempfile.mkstemp(
        dir=path.parent, prefix=f".{path.name}.", suffix=".tmp"
    )
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(payload)
            handle.write("\n")
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def read_json(path: str | os.PathLike[str]) -> Any:
    try:
        with open(path, "r", encoding="utf-8") as handle:
            return json.load(handle)
    except OSError as exc:
        raise Mes00Error(f"cannot read {Path(path).name}: {exc}") from exc
    except json.JSONDecodeError as exc:
        raise Mes00Error(f"invalid JSON in {Path(path).name}: {exc}") from exc


# ---------------------------------------------------------------------------
# Percentiles
# ---------------------------------------------------------------------------


def percentile(values: Iterable[float], percent: float) -> float:
    """Linear-interpolated percentile (the numpy default), no numpy needed."""

    data = sorted(float(value) for value in values)
    if not data:
        raise Mes00Error("percentile of an empty series")
    if not 0.0 <= percent <= 100.0:
        raise Mes00Error(f"percentile must be in [0, 100], got {percent}")
    if len(data) == 1:
        return data[0]
    rank = (len(data) - 1) * percent / 100.0
    low = math.floor(rank)
    high = math.ceil(rank)
    if low == high:
        return data[low]
    weight = rank - low
    return data[low] * (1.0 - weight) + data[high] * weight


def summarize(values: Iterable[float]) -> dict[str, float | int]:
    """Return count, mean and the p50/p95/p99 used by MES-00."""

    data = sorted(float(value) for value in values)
    if not data:
        raise Mes00Error("cannot summarize an empty series")
    return {
        "n": len(data),
        "min": round(data[0], 3),
        "max": round(data[-1], 3),
        "mean": round(sum(data) / len(data), 3),
        "p50": round(percentile(data, 50.0), 3),
        "p95": round(percentile(data, 95.0), 3),
        "p99": round(percentile(data, 99.0), 3),
    }


# ---------------------------------------------------------------------------
# Subprocess calls and command templates
# ---------------------------------------------------------------------------


def child_env(pgoptions: str, base: Mapping[str, str] | None = None) -> dict[str, str]:
    """Copy the environment and expose the read-only options as PGOPTIONS."""

    env = dict(os.environ if base is None else base)
    env["PGOPTIONS"] = pgoptions
    return env


def parse_json_output(stdout: str, *, label: str) -> Any:
    """Parse a child's stdout, tolerating log lines around one JSON document."""

    text = stdout.strip()
    if not text:
        raise Mes00Error(f"{label} printed no JSON on stdout")
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        pass
    for opening in ("{", "["):
        start = text.find(opening)
        if start < 0:
            continue
        try:
            value, _ = json.JSONDecoder().raw_decode(text[start:])
            return value
        except json.JSONDecodeError:
            continue
    raise Mes00Error(f"{label} did not print valid JSON on stdout")


def run_json_command(
    argv: Sequence[str],
    *,
    label: str,
    stdin_text: str | None = None,
    timeout: float = 300.0,
    cwd: str | os.PathLike[str] | None = None,
    env: Mapping[str, str] | None = None,
) -> Any:
    """Run one command serially and parse its JSON output."""

    if not argv:
        raise Mes00Error(f"{label}: empty command")
    try:
        completed = subprocess.run(
            list(argv),
            input=stdin_text,
            capture_output=True,
            text=True,
            timeout=timeout,
            cwd=cwd,
            env=dict(env) if env is not None else None,
            check=False,
        )
    except subprocess.TimeoutExpired as exc:
        raise Mes00Error(f"{label} timed out after {timeout:.0f} s") from exc
    except OSError as exc:
        raise Mes00Error(f"{label} could not start: {exc}") from exc

    if completed.returncode != 0:
        # stderr stays on the Mac mini; it is never copied into a report.
        tail = " ".join(completed.stderr.strip().splitlines()[-1:])
        raise Mes00Error(
            f"{label} failed with exit code {completed.returncode}: {tail[:300]}"
        )
    return parse_json_output(completed.stdout, label=label)


def render_command(template: Sequence[str], values: Mapping[str, str]) -> list[str]:
    """Replace ``{name}`` placeholders element-wise, without a shell.

    Each template element is one argv entry, so a title containing spaces,
    quotes or semicolons can never become shell syntax.
    """

    if not template:
        raise Mes00Error("command template is empty")
    rendered: list[str] = []
    for element in template:
        value = element
        for name, replacement in values.items():
            value = value.replace("{" + name + "}", replacement)
        left = _PLACEHOLDER.search(value)
        if left:
            raise Mes00Error(
                f"unknown placeholder in command template: {left.group(0)}"
            )
        rendered.append(value)
    return rendered


def command_from_json(raw: str) -> list[str]:
    """Parse a JSON array of strings used as an argv template."""

    try:
        value = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise Mes00Error(f"command template is not valid JSON: {exc}") from exc
    if not isinstance(value, list) or not value:
        raise Mes00Error("command template must be a non-empty JSON array")
    if not all(isinstance(element, str) for element in value):
        raise Mes00Error("every command template element must be a string")
    return value


# ---------------------------------------------------------------------------
# Repository and project roots
# ---------------------------------------------------------------------------


def find_repo_root(start: str | os.PathLike[str] | None = None) -> Path | None:
    """Find the Crible checkout, independently of where the script was copied.

    The scripts are copied to /tmp on the Mac mini; walking up from
    ``__file__`` would otherwise reach ``/`` and treat the whole filesystem as
    protected. A directory is a repository root when it contains ``.git`` or
    ``Cargo.toml``. Returns ``None`` when the script runs outside a checkout.
    """

    path = Path(start).expanduser() if start is not None else Path(__file__)
    path = path.resolve()
    if path.is_file():
        path = path.parent
    for candidate in (path, *path.parents):
        if (candidate / ".git").exists() or (candidate / "Cargo.toml").is_file():
            return candidate
    return None


def resolve_project_dir(raw: str | None) -> Path:
    """Return an absolute, existing Legifacile checkout.

    An empty value (including ``$PROD_REPO`` unset) is refused instead of
    silently becoming ``Path('.')``, which would load the local copy.
    """

    value = (raw if raw is not None else os.environ.get("PROD_REPO", "")).strip()
    if not value:
        raise Mes00Error("--project or PROD_REPO is required")
    path = Path(value).expanduser()
    if not path.is_absolute():
        raise Mes00Error("--project or PROD_REPO must be an absolute path")
    if not path.is_dir():
        raise Mes00Error("--project or PROD_REPO must be an existing directory")
    return path
