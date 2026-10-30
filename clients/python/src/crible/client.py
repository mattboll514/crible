# SPDX-License-Identifier: Apache-2.0

"""HTTP client of the Crible ``/v1`` API (ADR-0112).

:class:`Client` and :class:`AsyncClient` expose the same methods, the second
declared ``async``. Both share one core: it builds the request, encodes a
vector, reads the response and maps an error to its exception, so the two
versions can differ only in the call to ``httpx``.

Every method returns the JSON body of the response as a ``dict``; it is the
reference of ``docs/05-api.md``, and a field added by the server breaks no
caller. The methods that create a job return it without waiting: :meth:`wait`
follows it by polling ``GET /v1/jobs/{job_id}`` (ADR-0112, decision 4).
"""

from __future__ import annotations

import asyncio
import base64
import time
from dataclasses import dataclass, field
from typing import Any, Mapping, Sequence
from urllib.parse import quote

import httpx
import numpy as np

from .errors import CribleError, CribleTransportError, error_for_job, error_for_response

#: States of a job that end the wait (`docs/05-api.md`, "Jobs").
TERMINAL_STATES = frozenset({"succeeded", "failed"})


@dataclass(frozen=True)
class _Call:
    """One HTTP call, built by the core and sent by a transport."""

    method: str
    path: str
    body: Any = None
    params: Mapping[str, str] = field(default_factory=dict)


class _Core:
    """Behaviour shared by the synchronous and the asynchronous client."""

    def __init__(self, base_url: str, api_key: str) -> None:
        self.base_url = base_url.rstrip("/")
        self._api_key = api_key

    def headers(self) -> dict[str, str]:
        """Headers of every request: the API key of ADR-0100.

        An empty key sends no ``Authorization`` header, which the server answers
        with ``401`` like an absent or unknown key (ADR-0100).
        """
        if not self._api_key:
            return {}
        return {"Authorization": f"Bearer {self._api_key}"}

    def url(self, path: str) -> str:
        """Absolute URL of a path."""
        return self.base_url + path

    def search(self, index: str, options: Mapping[str, Any]) -> _Call:
        """``POST /v1/indexes/{index}/search``."""
        return _Call("POST", f"/v1/indexes/{_component(index)}/search", _search_body(options))

    def search_multi(
        self, targets: Sequence[Mapping[str, Any]], options: Mapping[str, Any]
    ) -> _Call:
        """``POST /v1/search``; ``targets`` is ``[{index, filter}]`` (ADR-0110)."""
        body = _search_body(options)
        body["targets"] = [_target(target) for target in targets]
        return _Call("POST", "/v1/search", body)

    def get_document(self, index: str, doc_id: str, text: bool) -> _Call:
        """``GET /v1/indexes/{index}/documents/{doc_id}``."""
        params = {"text": "true"} if text else {}
        return _Call(
            "GET",
            f"/v1/indexes/{_component(index)}/documents/{_component(doc_id)}",
            params=params,
        )

    def attach_batch(self, index: str, directory: str) -> _Call:
        """``POST /v1/indexes/{index}/batches`` for a batch of ``exchange/inbox``."""
        return _Call(
            "POST",
            f"/v1/indexes/{_component(index)}/batches",
            {"directory": directory},
        )

    def upsert_documents(
        self,
        index: str,
        batch_id: str,
        documents: Sequence[Mapping[str, Any]],
        deletions: Sequence[str] | None,
    ) -> _Call:
        """``POST /v1/indexes/{index}/documents``, an online batch."""
        body: dict[str, Any] = {
            "batch_id": batch_id,
            "documents": [_document(document) for document in documents],
        }
        if deletions is not None:
            body["deletions"] = list(deletions)
        return _Call("POST", f"/v1/indexes/{_component(index)}/documents", body)

    def patch_documents(self, index: str, documents: Sequence[Mapping[str, Any]]) -> _Call:
        """``PATCH /v1/indexes/{index}/documents``, a metadata patch."""
        body = {"documents": [dict(document) for document in documents]}
        return _Call("PATCH", f"/v1/indexes/{_component(index)}/documents", body)

    def job(self, job_id: str) -> _Call:
        """``GET /v1/jobs/{job_id}``."""
        return _Call("GET", f"/v1/jobs/{_component(job_id)}")

    def create_index(self, config: Mapping[str, Any]) -> _Call:
        """``POST /v1/indexes`` with the complete configuration of an index."""
        return _Call("POST", "/v1/indexes", dict(config))

    def list_indexes(self) -> _Call:
        """``GET /v1/indexes``."""
        return _Call("GET", "/v1/indexes")

    def get_index(self, index: str) -> _Call:
        """``GET /v1/indexes/{index}``."""
        return _Call("GET", f"/v1/indexes/{_component(index)}")

    def compact(self, index: str) -> _Call:
        """``POST /v1/indexes/{index}/compact``; the route takes no body."""
        return _Call("POST", f"/v1/indexes/{_component(index)}/compact")

    def export(self, index: str) -> _Call:
        """``POST /v1/indexes/{index}/exports``; the route takes no body."""
        return _Call("POST", f"/v1/indexes/{_component(index)}/exports")

    def decode(self, response: httpx.Response) -> dict[str, Any]:
        """Returns the JSON body of a success, or raises the typed error."""
        if not response.is_success:
            raise error_for_response(response)
        try:
            body = response.json()
        except ValueError as failure:
            raise CribleError(
                "the response body is not valid JSON", status=response.status_code
            ) from failure
        if not isinstance(body, dict):
            raise CribleError(
                "the response body is not a JSON object", status=response.status_code
            )
        return body

    def job_id(self, job: Mapping[str, Any]) -> str:
        """Identifier of a job, read from the object a method returned.

        ``docs/05-api.md`` renders the identifier under ``id``; ``job_id`` is
        accepted too, because ADR-0112 names it in prose.
        """
        for key in ("id", "job_id"):
            value = job.get(key)
            if isinstance(value, str) and value:
                return value
        raise ValueError("the job object has no 'id'")

    def terminal(self, job: Mapping[str, Any]) -> bool:
        """Whether the job reached ``succeeded`` or ``failed``."""
        return job.get("state") in TERMINAL_STATES

    def pause(self, deadline: float, poll_interval: float) -> float:
        """Seconds to wait before the next poll, or the standard ``TimeoutError``.

        The wait stops at its own deadline: the client never waits longer than
        the ``timeout`` it was given (ADR-0112, decision 4).
        """
        remaining = deadline - time.monotonic()
        if remaining <= 0.0:
            raise TimeoutError("the job did not reach a terminal state in time")
        return min(poll_interval, remaining)

    def outcome(self, job: Mapping[str, Any]) -> dict[str, Any]:
        """Returns the job, or raises the typed error of a failed job."""
        if job.get("state") == "failed":
            error = job.get("error")
            raise error_for_job(error if isinstance(error, Mapping) else {})
        return dict(job)


class Client:
    """Synchronous HTTP client of a Crible instance.

    ``base_url`` is the root of the instance (``http://host:8080``) and
    ``api_key`` the key of ``docs/05-api.md``. The client is usable as a
    context manager and never retries a call.
    """

    def __init__(self, base_url: str, api_key: str, *, timeout: float = 60.0) -> None:
        self._core = _Core(base_url, api_key)
        self._http = httpx.Client(timeout=timeout)

    def __enter__(self) -> Client:
        return self

    def __exit__(self, *exc_info: object) -> None:
        self.close()

    def close(self) -> None:
        """Releases the connection pool."""
        self._http.close()

    def search(self, index: str, **options: Any) -> dict[str, Any]:
        """Searches one index; ``options`` uses the field names of ``docs/05``."""
        return self._send(self._core.search(index, options))

    def search_multi(
        self, targets: Sequence[Mapping[str, Any]], **options: Any
    ) -> dict[str, Any]:
        """Searches several indexes in one request (``docs/05``, ADR-0110)."""
        return self._send(self._core.search_multi(targets, options))

    def get_document(self, index: str, doc_id: str, *, text: bool = False) -> dict[str, Any]:
        """Reads a document and its live passages, with their text if ``text``."""
        return self._send(self._core.get_document(index, doc_id, text))

    def attach_batch(self, index: str, directory: str) -> dict[str, Any]:
        """Attaches a batch deposited in ``exchange/inbox/<directory>``."""
        return self._send(self._core.attach_batch(index, directory))

    def upsert_documents(
        self,
        index: str,
        batch_id: str,
        documents: Sequence[Mapping[str, Any]],
        deletions: Sequence[str] | None = None,
    ) -> dict[str, Any]:
        """Writes an online batch; a passage ``vector`` is encoded in base64."""
        return self._send(self._core.upsert_documents(index, batch_id, documents, deletions))

    def patch_documents(
        self, index: str, documents: Sequence[Mapping[str, Any]]
    ) -> dict[str, Any]:
        """Updates document metadata without re-encoding (ADR-0060)."""
        return self._send(self._core.patch_documents(index, documents))

    def job(self, job_id: str) -> dict[str, Any]:
        """Reads one job."""
        return self._send(self._core.job(job_id))

    def wait(
        self, job: Mapping[str, Any], *, timeout: float = 600.0, poll_interval: float = 0.5
    ) -> dict[str, Any]:
        """Polls a job until it ends.

        Returns the job on ``succeeded``; raises the typed error of its
        ``error.code`` on ``failed``; raises the standard :class:`TimeoutError`
        at its own deadline, without cancelling the job.
        """
        deadline = time.monotonic() + timeout
        current = dict(job)
        while not self._core.terminal(current):
            time.sleep(self._core.pause(deadline, poll_interval))
            current = self.job(self._core.job_id(job))
        return self._core.outcome(current)

    def create_index(self, config: Mapping[str, Any]) -> dict[str, Any]:
        """Creates an index at runtime from its complete configuration."""
        return self._send(self._core.create_index(config))

    def list_indexes(self) -> dict[str, Any]:
        """Lists the served indexes, sorted by name."""
        return self._send(self._core.list_indexes())

    def get_index(self, index: str) -> dict[str, Any]:
        """Reads the configuration and the state of one index."""
        return self._send(self._core.get_index(index))

    def compact(self, index: str) -> dict[str, Any]:
        """Starts a compaction; returns its job."""
        return self._send(self._core.compact(index))

    def export(self, index: str) -> dict[str, Any]:
        """Starts an export to ``exchange/outbox``; returns its job."""
        return self._send(self._core.export(index))

    def _send(self, call: _Call) -> dict[str, Any]:
        try:
            response = self._http.request(
                call.method,
                self._core.url(call.path),
                params=dict(call.params),
                json=call.body,
                headers=self._core.headers(),
            )
        except httpx.HTTPError as failure:
            raise CribleTransportError(str(failure)) from failure
        return self._core.decode(response)


class AsyncClient:
    """Asynchronous HTTP client of a Crible instance.

    Same methods and same contract as :class:`Client`, declared ``async``. The
    client is usable as an asynchronous context manager.
    """

    def __init__(self, base_url: str, api_key: str, *, timeout: float = 60.0) -> None:
        self._core = _Core(base_url, api_key)
        self._http = httpx.AsyncClient(timeout=timeout)

    async def __aenter__(self) -> AsyncClient:
        return self

    async def __aexit__(self, *exc_info: object) -> None:
        await self.close()

    async def close(self) -> None:
        """Releases the connection pool."""
        await self._http.aclose()

    async def search(self, index: str, **options: Any) -> dict[str, Any]:
        """Searches one index; ``options`` uses the field names of ``docs/05``."""
        return await self._send(self._core.search(index, options))

    async def search_multi(
        self, targets: Sequence[Mapping[str, Any]], **options: Any
    ) -> dict[str, Any]:
        """Searches several indexes in one request (``docs/05``, ADR-0110)."""
        return await self._send(self._core.search_multi(targets, options))

    async def get_document(
        self, index: str, doc_id: str, *, text: bool = False
    ) -> dict[str, Any]:
        """Reads a document and its live passages, with their text if ``text``."""
        return await self._send(self._core.get_document(index, doc_id, text))

    async def attach_batch(self, index: str, directory: str) -> dict[str, Any]:
        """Attaches a batch deposited in ``exchange/inbox/<directory>``."""
        return await self._send(self._core.attach_batch(index, directory))

    async def upsert_documents(
        self,
        index: str,
        batch_id: str,
        documents: Sequence[Mapping[str, Any]],
        deletions: Sequence[str] | None = None,
    ) -> dict[str, Any]:
        """Writes an online batch; a passage ``vector`` is encoded in base64."""
        return await self._send(
            self._core.upsert_documents(index, batch_id, documents, deletions)
        )

    async def patch_documents(
        self, index: str, documents: Sequence[Mapping[str, Any]]
    ) -> dict[str, Any]:
        """Updates document metadata without re-encoding (ADR-0060)."""
        return await self._send(self._core.patch_documents(index, documents))

    async def job(self, job_id: str) -> dict[str, Any]:
        """Reads one job."""
        return await self._send(self._core.job(job_id))

    async def wait(
        self, job: Mapping[str, Any], *, timeout: float = 600.0, poll_interval: float = 0.5
    ) -> dict[str, Any]:
        """Polls a job until it ends; same contract as :meth:`Client.wait`."""
        deadline = time.monotonic() + timeout
        current = dict(job)
        while not self._core.terminal(current):
            await asyncio.sleep(self._core.pause(deadline, poll_interval))
            current = await self.job(self._core.job_id(job))
        return self._core.outcome(current)

    async def create_index(self, config: Mapping[str, Any]) -> dict[str, Any]:
        """Creates an index at runtime from its complete configuration."""
        return await self._send(self._core.create_index(config))

    async def list_indexes(self) -> dict[str, Any]:
        """Lists the served indexes, sorted by name."""
        return await self._send(self._core.list_indexes())

    async def get_index(self, index: str) -> dict[str, Any]:
        """Reads the configuration and the state of one index."""
        return await self._send(self._core.get_index(index))

    async def compact(self, index: str) -> dict[str, Any]:
        """Starts a compaction; returns its job."""
        return await self._send(self._core.compact(index))

    async def export(self, index: str) -> dict[str, Any]:
        """Starts an export to ``exchange/outbox``; returns its job."""
        return await self._send(self._core.export(index))

    async def _send(self, call: _Call) -> dict[str, Any]:
        try:
            response = await self._http.request(
                call.method,
                self._core.url(call.path),
                params=dict(call.params),
                json=call.body,
                headers=self._core.headers(),
            )
        except httpx.HTTPError as failure:
            raise CribleTransportError(str(failure)) from failure
        return self._core.decode(response)


def _component(value: str) -> str:
    """Percent-encodes one path component, so a name cannot alter the route."""
    return quote(str(value), safe="")


def _search_body(options: Mapping[str, Any]) -> dict[str, Any]:
    """Body of a search: the options that were given, ``vector`` in base64."""
    body = {key: value for key, value in options.items() if value is not None}
    if "vector" in body:
        body["vector"] = encode_vector_base64(body["vector"])
    return body


def _target(target: Mapping[str, Any]) -> dict[str, Any]:
    """One target of a multi-index search; an absent filter stays absent."""
    entry = dict(target)
    if entry.get("filter") is None:
        entry.pop("filter", None)
    return entry


def _document(document: Mapping[str, Any]) -> dict[str, Any]:
    """One document of an online batch, its passage vectors in base64."""
    encoded = dict(document)
    passages = encoded.get("passages")
    if isinstance(passages, Sequence):
        encoded["passages"] = [
            {**passage, "vector": encode_vector_base64(passage["vector"])}
            if isinstance(passage, Mapping) and "vector" in passage
            else passage
            for passage in passages
        ]
    return encoded


def encode_vector_base64(values: Any) -> str:
    """Encodes a vector as base64 of its fp32 little-endian components.

    The API carries a vector that way (``docs/05``, "Recherche : requête"). A
    string is taken as already encoded and returned unchanged; a NumPy array
    or a sequence of floats is converted to fp32 first.
    """
    if isinstance(values, str):
        return values
    array = np.asarray(values, dtype=np.float32)
    if array.ndim != 1:
        raise ValueError(f"a vector must be one-dimensional, got shape {array.shape}")
    return base64.b64encode(array.astype("<f4", copy=False).tobytes()).decode("ascii")


__all__ = ["AsyncClient", "Client", "encode_vector_base64"]
