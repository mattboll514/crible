# SPDX-License-Identifier: Apache-2.0

"""Tests of the CI model artifact download (ADR-0057, ADR-0059).

No test touches the network: the opener is injected, so a download is a
``BytesIO`` and an HTTP failure is raised locally. No test waits either: the
clock and the sleep function are injected, so a retry is recorded, not slept.
"""

from __future__ import annotations

import http.client
import io
import os
import urllib.error

import blake3
import pytest

import fetch_model_artifact as artifact

REPOSITORY = "mattboll/mdenseon-onnx"
COMMIT = "0123456789abcdef0123456789abcdef01234567"
#: Size of one read of the download loop, so a test can cross a chunk boundary.
CHUNK = 1 << 20


def digest(content: bytes) -> str:
    return blake3.blake3(content).hexdigest()


def manifest(*files: tuple[str, bytes], commit: str = COMMIT) -> artifact.Manifest:
    return artifact.Manifest(
        repository=REPOSITORY,
        commit=commit,
        files=tuple(
            artifact.ArtifactFile(name=name, blake3=digest(content))
            for name, content in files
        ),
    )


class _Response(io.BytesIO):
    """A response body with the HTTP status `urlopen` would expose."""

    def __init__(self, payload: bytes, status: int):
        super().__init__(payload)
        self.status = status


class _TruncatedResponse(_Response):
    """A body shorter than the `Content-Length` the server announced."""

    def __init__(self, payload: bytes, length: int, status: int = 200):
        super().__init__(payload, status)
        self.headers = {"Content-Length": str(length)}


class Opener:
    """Records the URLs it is asked for and serves one payload or one error."""

    def __init__(
        self, payload: bytes = b"", error: Exception | None = None, status: int = 200
    ):
        self.payload = payload
        self.error = error
        self.status = status
        self.urls: list[str] = []

    def __call__(self, url: str, timeout: int = 0):
        self.urls.append(url)
        if self.error is not None:
            raise self.error
        return _Response(self.payload, self.status)


class ScriptedOpener:
    """Serves one scripted outcome per call: an exception or a payload."""

    def __init__(self, *outcomes, status: int = 200):
        self.outcomes = list(outcomes)
        self.status = status
        self.urls: list[str] = []

    def __call__(self, url: str, timeout: int = 0):
        self.urls.append(url)
        if not self.outcomes:
            raise AssertionError("the opener was called more often than scripted")
        outcome = self.outcomes.pop(0)
        if isinstance(outcome, Exception):
            raise outcome
        if isinstance(outcome, bytes):
            return _Response(outcome, self.status)
        return outcome


class Waiter:
    """Records the delays a retry asks for, without sleeping."""

    def __init__(self):
        self.delays: list[float] = []

    def __call__(self, seconds: float) -> None:
        self.delays.append(seconds)


class Clock:
    """Returns the scripted ticks of a monotonic clock, then the last one."""

    def __init__(self, *ticks: float):
        self.ticks = list(ticks)
        self.now = 0.0

    def __call__(self) -> float:
        if self.ticks:
            self.now = self.ticks.pop(0)
        return self.now


def http_error(
    code: int, reason: str = "reason", retry_after: str | None = None
) -> urllib.error.HTTPError:
    headers = {} if retry_after is None else {"Retry-After": retry_after}
    return urllib.error.HTTPError("https://example.invalid", code, reason, headers, None)


def assert_no_part(destination, name: str = "model.onnx") -> None:
    assert not (destination / f".{name}.part").exists()


def test_a_valid_download_is_written_and_verified(tmp_path):
    content = b"the fp32 graph"
    opener = Opener(content)
    destination = tmp_path / "models"

    actions = artifact.ensure_artifact(
        manifest(("model.onnx", content)), destination, opener=opener
    )

    assert (destination / "model.onnx").read_bytes() == content
    assert len(opener.urls) == 1
    assert opener.urls[0] == (
        f"https://huggingface.co/{REPOSITORY}/resolve/{COMMIT}/model.onnx"
    )
    assert actions == [
        f"downloaded 'model.onnx' from {opener.urls[0]} (blake3 {digest(content)})"
    ]


def test_a_wrong_download_reports_the_expected_and_obtained_digests(tmp_path):
    expected = b"the fp32 graph"
    opener = Opener(b"another graph")

    with pytest.raises(artifact.ArtifactError) as raised:
        artifact.ensure_artifact(
            manifest(("model.onnx", expected)), tmp_path, opener=opener
        )

    message = str(raised.value)
    assert digest(expected) in message
    assert digest(b"another graph") in message
    assert "model.onnx" in message
    assert opener.urls[0] in message
    assert not (tmp_path / "model.onnx").exists()
    assert not (tmp_path / ".model.onnx.part").exists()


def test_an_http_error_names_the_url_and_the_file(tmp_path):
    error = urllib.error.HTTPError(
        "https://example.invalid", 404, "Not Found", None, None
    )

    with pytest.raises(artifact.ArtifactError) as raised:
        artifact.ensure_artifact(
            manifest(("tokenizer.json", b"{}")), tmp_path, opener=Opener(error=error)
        )

    message = str(raised.value)
    assert "tokenizer.json" in message
    assert (
        f"https://huggingface.co/{REPOSITORY}/resolve/{COMMIT}/tokenizer.json"
        in message
    )
    assert "HTTP 404 Not Found" in message


def test_an_unreachable_host_names_the_url_and_the_file(tmp_path):
    error = urllib.error.URLError("name resolution failed")
    waiter = Waiter()

    with pytest.raises(artifact.ArtifactError) as raised:
        artifact.ensure_artifact(
            manifest(("model.json", b"{}")),
            tmp_path,
            opener=Opener(error=error),
            sleep=waiter,
        )

    message = str(raised.value)
    assert "model.json" in message
    assert f"resolve/{COMMIT}/model.json" in message
    assert "name resolution failed" in message
    assert waiter.delays == [5, 15, 45]
    assert_no_part(tmp_path, "model.json")


def test_a_commit_that_is_not_a_sha_is_refused_before_any_download(tmp_path):
    opener = Opener(b"{}")
    unpinned = manifest(
        ("model.onnx", b"graph"), commit="REMPLACER-PAR-LE-SHA-DU-COMMIT-HF"
    )

    with pytest.raises(artifact.ArtifactError) as raised:
        artifact.ensure_artifact(unpinned, tmp_path, opener=opener)

    message = str(raised.value)
    assert "REMPLACER-PAR-LE-SHA-DU-COMMIT-HF" in message
    assert "40-character lowercase hexadecimal commit SHA" in message
    assert "never falls back to a branch" in message
    assert opener.urls == []
    assert not (tmp_path / "model.onnx").exists()


def test_a_divergent_cache_names_the_key_to_purge(tmp_path):
    expected = b"the fp32 graph"
    (tmp_path / "model.onnx").write_bytes(b"corrupt")
    opened = manifest(("model.onnx", expected))
    opener = Opener(expected)

    with pytest.raises(artifact.ArtifactError) as raised:
        artifact.ensure_artifact(opened, tmp_path, opener=opener)

    message = str(raised.value)
    assert "cached file" in message
    assert digest(b"corrupt") in message
    assert digest(expected) in message
    assert artifact.cache_key(opened) in message
    assert "purge the GitHub Actions cache key" in message
    assert opener.urls == []


def test_a_non_2xx_status_names_the_url_and_the_file(tmp_path):
    opener = Opener(b"server error", status=500)
    waiter = Waiter()

    with pytest.raises(artifact.ArtifactError) as raised:
        artifact.ensure_artifact(
            manifest(("model.onnx", b"graph")),
            tmp_path,
            opener=opener,
            sleep=waiter,
        )

    message = str(raised.value)
    assert "model.onnx" in message
    assert f"resolve/{COMMIT}/model.onnx" in message
    assert "HTTP status 500" in message
    assert "4 attempts" in message
    assert waiter.delays == [5, 15, 45]
    assert len(opener.urls) == 4
    assert not (tmp_path / "model.onnx").exists()
    assert_no_part(tmp_path)


def test_an_incomplete_cache_names_the_key_to_purge(tmp_path):
    (tmp_path / "model.onnx").write_bytes(b"graph")
    opened = manifest(("model.onnx", b"graph"), ("model.json", b"{}"))
    opener = Opener(b"{}")

    with pytest.raises(artifact.ArtifactError) as raised:
        artifact.ensure_artifact(opened, tmp_path, opener=opener)

    message = str(raised.value)
    assert "incomplete" in message
    assert "model.json missing" in message
    assert artifact.cache_key(opened) in message
    assert "purge the GitHub Actions cache key" in message
    assert opener.urls == []


def test_a_matching_cache_is_not_downloaded(tmp_path):
    content = b"the fp32 graph"
    (tmp_path / "model.onnx").write_bytes(content)
    opener = Opener(b"should not be served")

    actions = artifact.ensure_artifact(
        manifest(("model.onnx", content)), tmp_path, opener=opener
    )

    assert opener.urls == []
    assert actions == [f"verified 'model.onnx' (blake3 {digest(content)})"]


def test_the_cache_key_follows_every_digest_and_the_commit():
    first = manifest(("model.onnx", b"graph"), ("model.json", b"{}"))
    same = manifest(("model.json", b"{}"), ("model.onnx", b"graph"))
    other_content = manifest(("model.onnx", b"another"), ("model.json", b"{}"))
    other_commit = manifest(
        ("model.onnx", b"graph"), ("model.json", b"{}"), commit="f" * 40
    )

    assert artifact.cache_key(first) == artifact.cache_key(same)
    assert artifact.cache_key(first) != artifact.cache_key(other_content)
    assert artifact.cache_key(first) != artifact.cache_key(other_commit)
    assert artifact.cache_key(first).startswith("model-artifact-")


def test_a_malformed_digest_or_file_name_is_refused(tmp_path):
    bad_digest = artifact.Manifest(
        repository=REPOSITORY,
        commit=COMMIT,
        files=(artifact.ArtifactFile(name="model.onnx", blake3="abc"),),
    )
    bad_name = artifact.Manifest(
        repository=REPOSITORY,
        commit=COMMIT,
        files=(artifact.ArtifactFile(name="../model.onnx", blake3="a" * 64),),
    )

    with pytest.raises(artifact.ArtifactError, match="64-character"):
        artifact.validate_manifest(bad_digest)
    with pytest.raises(artifact.ArtifactError, match="bare file name"):
        artifact.validate_manifest(bad_name)


def test_the_repository_manifest_is_pinned_and_lists_the_downloaded_files():
    pinned = artifact.load_manifest(artifact.DEFAULT_MANIFEST)

    assert pinned.repository == REPOSITORY
    assert artifact.COMMIT_PATTERN.fullmatch(pinned.commit)
    assert {entry.name for entry in pinned.files} == {
        "model.onnx",
        "tokenizer.json",
        "model.json",
    }
    assert all(len(entry.blake3) == 64 for entry in pinned.files)


def test_the_cache_key_command_prints_one_key(tmp_path, capsys):
    manifest_path = tmp_path / "model-artifact.toml"
    manifest_path.write_text(
        "\n".join(
            [
                f'repository = "{REPOSITORY}"',
                f'commit = "{COMMIT}"',
                "",
                "[[files]]",
                'name = "model.json"',
                f'blake3 = "{digest(b"{}")}"',
                "",
            ]
        ),
        encoding="utf-8",
    )

    status = artifact.main(
        ["cache-key", "--manifest", str(manifest_path), "--dest", str(tmp_path)]
    )

    printed = capsys.readouterr().out.strip()
    assert status == 0
    assert printed.startswith("model-artifact-")
    assert len(printed) == len("model-artifact-") + 64


def test_a_placeholder_commit_makes_the_fetch_fail(tmp_path, capsys):
    manifest_path = tmp_path / "model-artifact.toml"
    manifest_path.write_text(
        "\n".join(
            [
                f'repository = "{REPOSITORY}"',
                'commit = "REMPLACER-PAR-LE-SHA-DU-COMMIT-HF"',
                "",
                "[[files]]",
                'name = "model.json"',
                f'blake3 = "{digest(b"{}")}"',
                "",
            ]
        ),
        encoding="utf-8",
    )

    status = artifact.main(
        ["fetch", "--manifest", str(manifest_path), "--dest", str(tmp_path / "models")]
    )

    captured = capsys.readouterr()
    assert status == 1
    assert captured.out == ""
    assert captured.err.startswith("error: ")
    assert "REMPLACER-PAR-LE-SHA-DU-COMMIT-HF" in captured.err


def test_a_transient_failure_gets_four_attempts_and_the_nominal_waits(tmp_path):
    opener = ScriptedOpener(*[urllib.error.URLError("connection reset")] * 4)
    waiter = Waiter()

    with pytest.raises(artifact.ArtifactError) as raised:
        artifact.ensure_artifact(
            manifest(("model.onnx", b"graph")),
            tmp_path,
            opener=opener,
            sleep=waiter,
        )

    assert len(opener.urls) == 4
    assert all(f"resolve/{COMMIT}/model.onnx" in url for url in opener.urls)
    assert waiter.delays == [5, 15, 45]
    message = str(raised.value)
    assert "connection reset" in message
    assert "4 attempts" in message
    assert not (tmp_path / "model.onnx").exists()
    assert_no_part(tmp_path)


@pytest.mark.parametrize(
    "failure",
    [
        TimeoutError("the socket timed out"),
        http.client.IncompleteRead(b"a truncated body"),
        OSError("the read failed"),
    ],
    ids=["timeout", "incomplete-read", "read-oserror"],
)
def test_a_transport_failure_gets_four_attempts(tmp_path, failure):
    opener = ScriptedOpener(*[failure] * 4)
    waiter = Waiter()

    with pytest.raises(artifact.ArtifactError) as raised:
        artifact.ensure_artifact(
            manifest(("model.onnx", b"graph")),
            tmp_path,
            opener=opener,
            sleep=waiter,
        )

    assert len(opener.urls) == 4
    assert waiter.delays == [5, 15, 45]
    assert "4 attempts" in str(raised.value)
    assert_no_part(tmp_path)


def test_a_retry_after_is_honoured_up_to_the_bound(tmp_path):
    content = b"the fp32 graph"
    opener = ScriptedOpener(http_error(429, "Too Many Requests", "60"), content)
    waiter = Waiter()

    actions = artifact.ensure_artifact(
        manifest(("model.onnx", content)),
        tmp_path,
        opener=opener,
        sleep=waiter,
    )

    assert len(opener.urls) == 2
    assert waiter.delays == [60.0]
    assert (tmp_path / "model.onnx").read_bytes() == content
    assert artifact.file_digest(tmp_path / "model.onnx") == digest(content)
    assert actions == [
        f"downloaded 'model.onnx' from {opener.urls[0]} "
        f"(blake3 {digest(content)}, 2 attempts)"
    ]
    assert_no_part(tmp_path)


def test_a_retry_after_beyond_the_bound_fails_at_once_with_its_value(tmp_path):
    opener = ScriptedOpener(http_error(429, "Too Many Requests", "61"))
    waiter = Waiter()

    with pytest.raises(artifact.ArtifactError) as raised:
        artifact.ensure_artifact(
            manifest(("model.onnx", b"graph")),
            tmp_path,
            opener=opener,
            sleep=waiter,
        )

    message = str(raised.value)
    assert len(opener.urls) == 1
    assert waiter.delays == []
    assert "Retry-After 61 s" in message
    assert "exceeds the 60 s bound" in message
    assert not (tmp_path / "model.onnx").exists()
    assert_no_part(tmp_path)


@pytest.mark.parametrize("code,reason", [(404, "Not Found"), (403, "Forbidden")])
def test_a_client_error_is_not_retried(tmp_path, code, reason):
    opener = ScriptedOpener(http_error(code, reason))
    waiter = Waiter()

    with pytest.raises(artifact.ArtifactError) as raised:
        artifact.ensure_artifact(
            manifest(("model.onnx", b"graph")),
            tmp_path,
            opener=opener,
            sleep=waiter,
        )

    message = str(raised.value)
    assert len(opener.urls) == 1
    assert waiter.delays == []
    assert f"HTTP {code} {reason}" in message
    assert "1 attempt" in message
    assert_no_part(tmp_path)


def test_a_wrong_digest_is_not_retried(tmp_path):
    opener = ScriptedOpener(b"another graph")
    waiter = Waiter()

    with pytest.raises(artifact.ArtifactError) as raised:
        artifact.ensure_artifact(
            manifest(("model.onnx", b"the fp32 graph")),
            tmp_path,
            opener=opener,
            sleep=waiter,
        )

    assert len(opener.urls) == 1
    assert waiter.delays == []
    assert "model.onnx" in str(raised.value)
    assert not (tmp_path / "model.onnx").exists()
    assert_no_part(tmp_path)


def test_the_global_deadline_fails_and_names_the_file_in_progress(tmp_path):
    opener = ScriptedOpener(urllib.error.URLError("still down"))
    waiter = Waiter()
    clock = Clock(0.0, 700.0)

    with pytest.raises(artifact.ArtifactError) as raised:
        artifact.ensure_artifact(
            manifest(("tokenizer.json", b"{}")),
            tmp_path,
            opener=opener,
            sleep=waiter,
            clock=clock,
        )

    message = str(raised.value)
    assert "tokenizer.json" in message
    assert f"{artifact.DOWNLOAD_DEADLINE_SECONDS} s download deadline" in message
    assert len(opener.urls) == 1
    assert waiter.delays == []
    assert_no_part(tmp_path, "tokenizer.json")


def test_a_success_on_the_second_attempt_writes_and_verifies_the_file(tmp_path):
    content = b"the fp32 graph"
    opener = ScriptedOpener(urllib.error.URLError("temporary outage"), content)
    waiter = Waiter()

    actions = artifact.ensure_artifact(
        manifest(("model.onnx", content)),
        tmp_path,
        opener=opener,
        sleep=waiter,
    )

    assert len(opener.urls) == 2
    assert waiter.delays == [5]
    assert (tmp_path / "model.onnx").read_bytes() == content
    assert artifact.file_digest(tmp_path / "model.onnx") == digest(content)
    assert actions == [
        f"downloaded 'model.onnx' from {opener.urls[0]} "
        f"(blake3 {digest(content)}, 2 attempts)"
    ]
    assert_no_part(tmp_path)


@pytest.mark.skipif(
    not hasattr(os, "geteuid") or os.geteuid() == 0,
    reason="a read-only directory refuses writes only to a non-root user",
)
def test_a_write_error_is_fatal_and_leaves_no_part(tmp_path):
    content = b"the fp32 graph"
    opener = ScriptedOpener(content)
    waiter = Waiter()
    destination = tmp_path / "models"
    destination.mkdir()
    destination.chmod(0o500)
    try:
        with pytest.raises(artifact.ArtifactError) as raised:
            artifact.ensure_artifact(
                manifest(("model.onnx", content)),
                destination,
                opener=opener,
                sleep=waiter,
            )
    finally:
        destination.chmod(0o700)

    message = str(raised.value)
    assert len(opener.urls) == 1
    assert waiter.delays == []
    assert "model.onnx" in message
    assert "cannot write" in message
    assert "1 attempt" in message
    assert_no_part(destination)


def test_the_policy_constants_are_the_ones_of_the_adr():
    assert artifact.MAX_ATTEMPTS == 4
    assert artifact.RETRY_DELAYS_SECONDS == (5, 15, 45)
    assert artifact.MAX_RETRY_AFTER_SECONDS == 60
    assert artifact.DOWNLOAD_DEADLINE_SECONDS == 600


@pytest.mark.parametrize("size", [2 * CHUNK, 2 * CHUNK + CHUNK // 2])
def test_a_body_of_several_chunks_is_written_without_losing_bytes(tmp_path, size):
    pattern = b"0123456789abcdef"
    content = (pattern * (size // len(pattern) + 1))[:size]
    opener = ScriptedOpener(content)
    waiter = Waiter()

    actions = artifact.ensure_artifact(
        manifest(("model.onnx", content)),
        tmp_path,
        opener=opener,
        sleep=waiter,
    )

    written = (tmp_path / "model.onnx").read_bytes()
    assert len(written) == size
    assert written == content
    assert artifact.file_digest(tmp_path / "model.onnx") == digest(content)
    assert waiter.delays == []
    assert actions == [
        f"downloaded 'model.onnx' from {opener.urls[0]} (blake3 {digest(content)})"
    ]
    assert_no_part(tmp_path)


def test_a_body_shorter_than_content_length_is_retried(tmp_path):
    content = b"the fp32 graph"
    opener = ScriptedOpener(_TruncatedResponse(content[:4], len(content)), content)
    waiter = Waiter()

    actions = artifact.ensure_artifact(
        manifest(("model.onnx", content)),
        tmp_path,
        opener=opener,
        sleep=waiter,
    )

    assert len(opener.urls) == 2
    assert waiter.delays == [5]
    assert (tmp_path / "model.onnx").read_bytes() == content
    assert artifact.file_digest(tmp_path / "model.onnx") == digest(content)
    assert actions == [
        f"downloaded 'model.onnx' from {opener.urls[0]} "
        f"(blake3 {digest(content)}, 2 attempts)"
    ]
    assert_no_part(tmp_path)

