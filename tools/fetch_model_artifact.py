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
import os
import re
import shutil
import sys
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


class ArtifactError(Exception):
    """A manifest, download or digest problem the job must not ignore."""


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
) -> None:
    """Streams `url` into `destination`, failing with the URL and the file name."""
    try:
        with opener(url, timeout=TIMEOUT_SECONDS) as response:
            status = getattr(response, "status", None)
            if status is not None and not 200 <= status < 300:
                raise ArtifactError(
                    f"cannot download '{name}' from '{url}': HTTP status {status}"
                )
            with destination.open("wb") as handle:
                shutil.copyfileobj(response, handle)
    except ArtifactError:
        raise
    except urllib.error.HTTPError as error:
        raise ArtifactError(
            f"cannot download '{name}' from '{url}': HTTP {error.code} "
            f"{error.reason}"
        ) from error
    except (urllib.error.URLError, OSError) as error:
        reason = getattr(error, "reason", error)
        raise ArtifactError(
            f"cannot download '{name}' from '{url}': {reason}"
        ) from error
    if not destination.is_file():
        raise ArtifactError(
            f"cannot download '{name}' from '{url}': no content received"
        )


def ensure_artifact(
    manifest: Manifest,
    destination: Path,
    opener=urllib.request.urlopen,
) -> list[str]:
    """Places every file of the manifest in `destination`, verified.

    Present files are the restored cache and are verified; absent ones are
    downloaded, verified, then renamed into place. Nothing is left behind when a
    digest differs.
    """
    validate_manifest(manifest)
    key = cache_key(manifest)
    destination.mkdir(parents=True, exist_ok=True)
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
            download_file(url, entry.name, temporary, opener=opener)
            verify_file(temporary, entry, url)
            os.replace(temporary, target)
        finally:
            temporary.unlink(missing_ok=True)
        actions.append(
            f"downloaded '{entry.name}' from {url} (blake3 {entry.blake3})"
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
