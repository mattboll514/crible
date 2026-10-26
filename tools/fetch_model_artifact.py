# SPDX-License-Identifier: Apache-2.0

"""Download and verify the model artifact of the CI (ADR-0057).

The `rust-onnx` job of `.github/workflows/ci.yml` needs the real mDenseOn export
in `data/models/mdenseon/`. Those files are gitignored and the CI never
re-exports them: they are published once in a public Hugging Face model
repository, pinned to a commit SHA, and every downloaded byte is checked against
the BLAKE3 digests of the versioned manifest `ci/model-artifact.toml`.

The script reads:

```toml
repository = "mattboll/mdenseon-onnx"
commit = "<40 hexadecimal characters>"

[[files]]
name = "model.onnx"
blake3 = "<64 hexadecimal characters>"
```

It builds the explicit URL
`https://huggingface.co/<repository>/resolve/<commit>/<name>`: the commit is
never a branch, so a new upload cannot change what the job runs. It downloads
without a token, because the repository is public.

The GitHub Actions cache is **not** a source of trust. A file already present in
the destination is verified exactly like a fresh download; a divergent file
stops the job and names the cache key to purge. An incomplete cache (some files
present, others missing) also fails with the key to purge, instead of
redownloading the missing files in silence: such a cache can never be saved
again under the same key. A missing file is downloaded, verified, then moved
into place.

A download retries only when the cause is outside the content (ADR-0059): a
`URLError`, a `TimeoutError`, another read `OSError`, an `IncompleteRead`, an
HTTP 429 or an HTTP 5xx. It never retries a wrong digest, another 4xx, an
invalid manifest or a divergent cache, because a second request would give the
same answer. The policy is four attempts at most, waits of 5, 15 and 45 s, a
`Retry-After` honoured up to 60 s and refused above it, and one global 600 s
deadline for the whole artifact, which keeps the job far from its
`timeout-minutes: 20`. The wait and the clock are injected, so no test sleeps or
reaches the network.

Commands, run from `tools/` with the default dependencies (they carry `blake3`,
ADR-0034):

```sh
uv run python fetch_model_artifact.py cache-key   # key of the Actions cache
uv run python fetch_model_artifact.py             # fetch and verify
```

The cache key is derived from the repository, the commit and every digest of the
manifest: an upload, a new commit or a changed digest gives another key.
"""

from __future__ import annotations

import argparse
import hashlib
import http.client
import os
import re
import sys
import time
import tomllib
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path

#: Repository root, one level above this file.
ROOT = Path(__file__).resolve().parents[1]
#: Versioned manifest of the artifact (ADR-0057).
DEFAULT_MANIFEST = ROOT / "ci/model-artifact.toml"
#: Directory the tests read (`data/models/mdenseon/`).
DEFAULT_DEST = ROOT / "data/models/mdenseon"
#: Explicit download URL: a commit SHA, never a branch.
URL_TEMPLATE = "https://huggingface.co/{repository}/resolve/{commit}/{name}"
#: A pinned commit is a full, lowercase, hexadecimal SHA.
COMMIT_PATTERN = re.compile(r"^[0-9a-f]{40}$")
#: A BLAKE3 digest is 64 lowercase hexadecimal characters.
BLAKE3_PATTERN = re.compile(r"^[0-9a-f]{64}$")
#: Seconds before a stalled download is abandoned.
TIMEOUT_SECONDS = 120
#: Attempts per file: one initial attempt and three retries (ADR-0059).
MAX_ATTEMPTS = 4
#: Wait before each retry when the server gives no usable `Retry-After`.
RETRY_DELAYS_SECONDS = (5, 15, 45)
#: Upper bound of a `Retry-After` the script honours; above it, it fails at once.
MAX_RETRY_AFTER_SECONDS = 60
#: Global deadline of the download, well under the `timeout-minutes: 20` of the job.
DOWNLOAD_DEADLINE_SECONDS = 600


class ArtifactError(Exception):
    """A manifest, download or digest problem the job must not ignore."""


class _DownloadFailure(Exception):
    """One failed attempt at downloading a file, with its cause."""

    def __init__(self, cause: str, retry_after: float | None = None) -> None:
        super().__init__(cause)
        self.cause = cause
        self.retry_after = retry_after


class _FatalFailure(_DownloadFailure):
    """A failure the download must not retry: the request or the content is wrong."""


class _TransientFailure(_DownloadFailure):
    """A failure caused by the transport, which the download retries."""


@dataclass(frozen=True)
class ArtifactFile:
    """One file of the artifact: its name in the repository and its digest."""

    name: str
    blake3: str


@dataclass(frozen=True)
class Manifest:
    """The pinned description of the artifact."""

    repository: str
    commit: str
    files: tuple[ArtifactFile, ...]


def load_manifest(path: Path) -> Manifest:
    """Reads and validates the manifest, refusing anything unpinned."""
    with path.open("rb") as handle:
        payload = tomllib.load(handle)
    repository = payload.get("repository")
    commit = payload.get("commit")
    entries = payload.get("files")
    if not isinstance(repository, str) or not repository:
        raise ArtifactError(f"manifest '{path}' has no 'repository'")
    if not isinstance(commit, str) or not commit:
        raise ArtifactError(f"manifest '{path}' has no 'commit'")
    if not isinstance(entries, list) or not entries:
        raise ArtifactError(f"manifest '{path}' has no '[[files]]' entry")
    files = tuple(
        ArtifactFile(name=entry.get("name", ""), blake3=entry.get("blake3", ""))
        for entry in entries
        if isinstance(entry, dict)
    )
    if len(files) != len(entries):
        raise ArtifactError(f"manifest '{path}' has a malformed '[[files]]' entry")
    manifest = Manifest(repository=repository, commit=commit, files=files)
    validate_manifest(manifest, path)
    return manifest


def validate_manifest(manifest: Manifest, path: Path = DEFAULT_MANIFEST) -> None:
    """Refuses a branch name, a short SHA and a malformed digest.

    The artifact is pinned to a commit. Falling back to a branch such as `main`
    would make the job run whatever is published at that moment, which is the
    mutable state ADR-0057 refuses.
    """
    if not COMMIT_PATTERN.fullmatch(manifest.commit):
        raise ArtifactError(
            f"manifest '{path}' has commit '{manifest.commit}', which is not a "
            f"40-character lowercase hexadecimal commit SHA; the artifact is "
            f"pinned to a commit and the download never falls back to a branch "
            f"such as 'main'"
        )
    parts = manifest.repository.split("/")
    if len(parts) != 2 or not all(parts):
        raise ArtifactError(
            f"manifest '{path}' has repository '{manifest.repository}', which is "
            f"not a '<owner>/<name>' Hugging Face repository"
        )
    seen = set()
    for entry in manifest.files:
        if not entry.name or "/" in entry.name or entry.name in (".", ".."):
            raise ArtifactError(
                f"manifest '{path}' has file name '{entry.name}', which is not a "
                f"bare file name"
            )
        if entry.name in seen:
            raise ArtifactError(
                f"manifest '{path}' lists file '{entry.name}' twice"
            )
        seen.add(entry.name)
        if not BLAKE3_PATTERN.fullmatch(entry.blake3):
            raise ArtifactError(
                f"manifest '{path}' has blake3 '{entry.blake3}' for '{entry.name}', "
                f"which is not a 64-character lowercase hexadecimal digest"
            )


def artifact_url(manifest: Manifest, name: str) -> str:
    """Returns the explicit URL of one file, pinned to the commit."""
    return URL_TEMPLATE.format(
        repository=manifest.repository, commit=manifest.commit, name=name
    )


def cache_key(manifest: Manifest) -> str:
    """Returns the Actions cache key, derived from the pinned digests.

    Only the repository, the commit and the file digests enter the key: the
    cache is indexed on the hashes of the manifest, as ADR-0057 asks, so a
    changed upload never reuses the previous cache entry.
    """
    canonical = "\n".join(
        [
            manifest.repository,
            manifest.commit,
            *(
                f"{entry.name}:{entry.blake3}"
                for entry in sorted(manifest.files, key=lambda item: item.name)
            ),
        ]
    )
    digest = hashlib.sha256(canonical.encode("utf-8")).hexdigest()
    return f"model-artifact-{digest}"


def file_digest(path: Path) -> str:
    """Returns the hexadecimal BLAKE3 digest of a file, read by chunks."""
    import blake3

    digest = blake3.blake3()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def verify_file(path: Path, entry: ArtifactFile, url: str) -> None:
    """Fails when a downloaded file does not carry the expected digest."""
    actual = file_digest(path)
    if actual != entry.blake3:
        raise ArtifactError(
            f"downloaded '{entry.name}' from '{url}' has blake3 {actual}, "
            f"expected {entry.blake3}"
        )


def verify_cache(path: Path, entry: ArtifactFile, key: str) -> None:
    """Fails when a restored file does not carry the expected digest.

    A cache entry is only a copy: a divergent file means the cache is corrupt,
    and the message names the key to purge instead of silently redownloading.
    """
    actual = file_digest(path)
    if actual != entry.blake3:
        raise ArtifactError(
            f"cached file '{path}' for '{entry.name}' has blake3 {actual}, "
            f"expected {entry.blake3}; purge the GitHub Actions cache key "
            f"'{key}' and rerun"
        )


def download_file(
    url: str,
    name: str,
    destination: Path,
    opener=urllib.request.urlopen,
    *,
    sleep=time.sleep,
    clock=time.monotonic,
    deadline: float | None = None,
) -> int:
    """Streams `url` into `destination`, retrying a transient failure (ADR-0059).

    Returns the number of attempts made. A transient failure is attempted at
    most `MAX_ATTEMPTS` times in total, with the waits of `RETRY_DELAYS_SECONDS`,
    or with the server's `Retry-After` up to `MAX_RETRY_AFTER_SECONDS`; never
    both.
    A wait that would cross `deadline`, an absolute value read from `clock`,
    stops the download at once. Any other failure is raised on the first attempt.
    The caller cleans the partial file up.
    """
    attempts = 0
    while True:
        attempts += 1
        try:
            _attempt_download(url, name, destination, opener)
            return attempts
        except _FatalFailure as failure:
            raise _download_error(name, url, failure.cause, attempts) from failure
        except _TransientFailure as failure:
            if (
                failure.retry_after is not None
                and failure.retry_after > MAX_RETRY_AFTER_SECONDS
            ):
                cause = (
                    f"{failure.cause}; Retry-After {failure.retry_after:g} s exceeds "
                    f"the {MAX_RETRY_AFTER_SECONDS} s bound"
                )
                raise _download_error(name, url, cause, attempts) from failure
            if attempts >= MAX_ATTEMPTS:
                raise _download_error(name, url, failure.cause, attempts) from failure
            delay = (
                failure.retry_after
                if failure.retry_after is not None
                else RETRY_DELAYS_SECONDS[attempts - 1]
            )
            if deadline is not None and clock() + delay >= deadline:
                cause = (
                    f"{failure.cause}; the {DOWNLOAD_DEADLINE_SECONDS} s download "
                    f"deadline is reached"
                )
                raise _download_error(name, url, cause, attempts) from failure
            sleep(delay)


def _download_error(name: str, url: str, cause: str, attempts: int) -> ArtifactError:
    """Builds the single failure line, naming the attempts that were made."""
    label = "1 attempt" if attempts == 1 else f"{attempts} attempts"
    return ArtifactError(f"cannot download '{name}' from '{url}': {cause} ({label})")


def _attempt_download(url: str, name: str, destination: Path, opener) -> None:
    """Runs one attempt; raises a failure carrying only the cause.

    Only a failure of the transport is transient (ADR-0059): the connection, the
    status and the read of the body are retried. A write failure of the partial
    file (full disk, refused permission) is fatal, because a second request would
    fail the same way; the body is therefore read and written chunk by chunk, so
    the two kinds of `OSError` never mix.
    """
    try:
        response = opener(url, timeout=TIMEOUT_SECONDS)
    except urllib.error.HTTPError as error:
        raise _http_failure(error) from error
    except urllib.error.URLError as error:
        raise _TransientFailure(str(getattr(error, "reason", error))) from error
    except (http.client.IncompleteRead, OSError) as error:
        raise _TransientFailure(_cause(error)) from error
    with response:
        status = getattr(response, "status", None)
        if status is not None and not 200 <= status < 300:
            raise _status_failure(response, status)
        announced = _content_length(getattr(response, "headers", None))
        read_total = 0
        try:
            with destination.open("wb") as handle:
                while True:
                    try:
                        chunk = response.read(1 << 20)
                    except (http.client.IncompleteRead, OSError) as error:
                        raise _TransientFailure(_cause(error)) from error
                    if not chunk:
                        break
                    read_total += len(chunk)
                    try:
                        handle.write(chunk)
                    except OSError as error:
                        raise _FatalFailure(
                            f"cannot write '{destination}': {error}"
                        ) from error
        except OSError as error:
            raise _FatalFailure(f"cannot write '{destination}': {error}") from error
        # `HTTPResponse.read(amt)` returns a short body without raising
        # `IncompleteRead`, so a truncation is only visible against the announced
        # length. A body longer than announced is impossible in correct HTTP and
        # stays with the digest check (ADR-0057).
        if announced is not None and read_total < announced:
            raise _TransientFailure(
                f"truncated response: read {read_total} bytes, Content-Length "
                f"announces {announced}"
            )
    if not destination.is_file():
        raise _FatalFailure("no content received")


def _content_length(headers) -> int | None:
    """Returns the announced body length, or `None` when absent or unusable."""
    try:
        value = headers.get("Content-Length") if headers is not None else None
    except AttributeError:
        return None
    try:
        length = int(value)
    except (TypeError, ValueError):
        return None
    return length if length >= 0 else None


def _cause(error: BaseException) -> str:
    """Returns the message of an error, or its class name when it carries none."""
    return str(error) or type(error).__name__


def _http_failure(error: urllib.error.HTTPError) -> _DownloadFailure:
    """Classifies an HTTP error: 429 and 5xx are transient, any other 4xx is fatal."""
    reason = getattr(error, "reason", None)
    cause = f"HTTP {error.code} {reason}" if reason else f"HTTP {error.code}"
    if error.code == 429 or 500 <= error.code < 600:
        headers = getattr(error, "headers", None)
        return _TransientFailure(cause, _retry_after_seconds(headers))
    return _FatalFailure(cause)


def _status_failure(response, status: int) -> _DownloadFailure:
    """Classifies a non-2xx status returned without an exception."""
    cause = f"HTTP status {status}"
    if status == 429 or 500 <= status < 600:
        headers = getattr(response, "headers", None)
        return _TransientFailure(cause, _retry_after_seconds(headers))
    return _FatalFailure(cause)


def _retry_after_seconds(headers) -> float | None:
    """Returns the `Retry-After` delay in seconds, or `None` when unusable.

    Only the delay form is read. An HTTP-date or a malformed value falls back to
    the nominal wait, so an unreadable header never blocks a retry.
    """
    try:
        value = headers.get("Retry-After") if headers is not None else None
    except AttributeError:
        return None
    if value is None:
        return None
    try:
        seconds = float(value)
    except (TypeError, ValueError):
        return None
    return seconds if seconds >= 0 else None


def ensure_artifact(
    manifest: Manifest,
    destination: Path,
    opener=urllib.request.urlopen,
    *,
    sleep=time.sleep,
    clock=time.monotonic,
) -> list[str]:
    """Places every file of the manifest in `destination`, verified.

    A complete restored cache is verified in place. A partially restored cache
    fails with the cache key to purge: an Actions cache cannot be overwritten
    under the same key, so completing it silently would download the missing
    files on every run. An absent cache is downloaded, verified, then renamed
    into place; nothing is left behind when a digest differs, whatever the
    failure. `clock` fixes one `DOWNLOAD_DEADLINE_SECONDS` deadline for the whole
    artifact, so a retry never outlives the job (ADR-0059).
    """
    validate_manifest(manifest)
    key = cache_key(manifest)
    destination.mkdir(parents=True, exist_ok=True)
    deadline = clock() + DOWNLOAD_DEADLINE_SECONDS
    actions = []
    present = [
        entry for entry in manifest.files if (destination / entry.name).is_file()
    ]
    missing = [
        entry for entry in manifest.files if not (destination / entry.name).is_file()
    ]
    if present and missing:
        raise ArtifactError(
            f"restored cache in '{destination}' is incomplete: "
            f"{', '.join(entry.name for entry in missing)} missing; purge the "
            f"GitHub Actions cache key '{key}' and rerun"
        )
    for entry in manifest.files:
        target = destination / entry.name
        url = artifact_url(manifest, entry.name)
        if target.is_file():
            verify_cache(target, entry, key)
            actions.append(f"verified '{entry.name}' (blake3 {entry.blake3})")
            continue
        temporary = target.with_name(f".{target.name}.part")
        try:
            attempts = download_file(
                url,
                entry.name,
                temporary,
                opener=opener,
                sleep=sleep,
                clock=clock,
                deadline=deadline,
            )
            verify_file(temporary, entry, url)
            os.replace(temporary, target)
        finally:
            temporary.unlink(missing_ok=True)
        # The job log carries the number of retries actually observed, which is
        # the signal ADR-0059 asks to watch: a retry every run means a rate limit
        # or an availability problem, not an incident of the artifact.
        retries = "" if attempts == 1 else f", {attempts} attempts"
        actions.append(
            f"downloaded '{entry.name}' from {url} "
            f"(blake3 {entry.blake3}{retries})"
        )
    return actions


def _parse_args(argv: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Download and verify the model artifact of the Crible CI."
    )
    parser.add_argument(
        "command",
        nargs="?",
        choices=("fetch", "cache-key"),
        default="fetch",
        help="'fetch' verifies and downloads (default); 'cache-key' prints the cache key",
    )
    parser.add_argument(
        "--manifest",
        default=str(DEFAULT_MANIFEST),
        help="manifest of the artifact (default: %(default)s)",
    )
    parser.add_argument(
        "--dest",
        default=str(DEFAULT_DEST),
        help="model directory the tests read (default: %(default)s)",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    arguments = _parse_args(argv)
    try:
        manifest_path = Path(arguments.manifest)
        manifest = load_manifest(manifest_path)
        if arguments.command == "cache-key":
            print(cache_key(manifest))
            return 0
        for line in ensure_artifact(manifest, Path(arguments.dest)):
            print(line)
    except Exception as error:  # noqa: BLE001 - every failure becomes one line
        print(f"error: {' '.join(str(error).split())}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
