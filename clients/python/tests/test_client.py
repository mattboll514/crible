# SPDX-License-Identifier: Apache-2.0

"""Unit tests of the HTTP client: core, vector encoding and typed errors.

No server is started here. The request builders and the response analysis run
against ``httpx.MockTransport``, and the error table of ``docs/05-api.md`` is
read from the document itself: adding a code there without its class fails this
file (ADR-0112, decision 8).
"""

from __future__ import annotations

import asyncio
import base64
import json
import re
import struct
from pathlib import Path
from typing import Callable

import httpx
import numpy as np
import pytest

from crible import (
    AsyncClient,
    Client,
    CribleError,
    CribleTransportError,
    NotFoundError,
    OverloadedError,
    RequestTimeoutError,
    encode_vector_base64,
)
from crible.errors import CODE_ERRORS, error_for_response

API_DOCUMENT = Path(__file__).resolve().parents[3] / "docs" / "05-api.md"
TABLE_ROW = re.compile(r"^\|\s*`([a-z_]+)`\s*\|\s*(\d{3})\s*\|", re.MULTILINE)

Handler = Callable[[httpx.Request], httpx.Response]


def documented_errors() -> dict[str, int]:
    """The error table of ``docs/05-api.md``: code to HTTP status."""
    found = TABLE_ROW.findall(API_DOCUMENT.read_text(encoding="utf-8"))
    assert found, f"no error table found in {API_DOCUMENT}"
    return {code: int(status) for code, status in found}


def camel_case(code: str) -> str:
    """``invalid_request`` to ``InvalidRequest``, the class name rule."""
    return "".join(part.capitalize() for part in code.split("_"))


def mock_client(handler: Handler) -> Client:
    """A client whose transport is :class:`httpx.MockTransport`."""
    client = Client("http://crible.test", "secret")
    client.close()
    client._http = httpx.Client(transport=httpx.MockTransport(handler))
    return client


def mock_async_client(handler: Handler) -> AsyncClient:
    """An asynchronous client whose transport is :class:`httpx.MockTransport`."""
    client = AsyncClient("http://crible.test", "secret")
    client._http = httpx.AsyncClient(transport=httpx.MockTransport(handler))
    return client


# --- the error table of the API -------------------------------------------


def test_every_documented_code_has_its_class_and_no_other() -> None:
    documented = documented_errors()
    assert set(documented) == set(CODE_ERRORS), (
        "the client maps exactly the codes of the error table of docs/05-api.md"
    )


def test_each_class_is_named_after_its_code() -> None:
    for code, error_class in CODE_ERRORS.items():
        expected = "RequestTimeout" if code == "timeout" else camel_case(code)
        assert error_class.__name__ == f"{expected}Error"
        assert issubclass(error_class, CribleError)


def test_the_timeout_code_does_not_mask_the_standard_timeout_error() -> None:
    assert CODE_ERRORS["timeout"] is RequestTimeoutError
    assert not issubclass(RequestTimeoutError, TimeoutError)
    assert not issubclass(TimeoutError, CribleError)


@pytest.mark.parametrize(("code", "status"), sorted(documented_errors().items()))
def test_each_code_becomes_its_typed_exception(code: str, status: int) -> None:
    response = httpx.Response(
        status,
        headers={"x-request-id": "req-1"},
        json={
            "error": {
                "code": code,
                "message": "boom",
                "details": {},
                "request_id": "req-1",
            }
        },
    )
    error = error_for_response(response)
    assert type(error) is CODE_ERRORS[code]
    assert error.code == code
    assert error.status == status
    assert error.message == "boom"
    assert error.details == {}
    assert error.request_id == "req-1"
    assert error.retry_after is None


def test_an_unknown_code_keeps_the_code_and_raises_the_base_class() -> None:
    response = httpx.Response(
        400,
        json={
            "error": {
                "code": "brand_new_code",
                "message": "from a newer server",
                "details": {"field": "x"},
                "request_id": "req-2",
            }
        },
    )
    error = error_for_response(response)
    assert type(error) is CribleError
    assert error.code == "brand_new_code"
    assert error.status == 400
    assert error.details == {"field": "x"}
    assert error.request_id == "req-2"


@pytest.mark.parametrize(
    "response",
    [
        httpx.Response(500, text="not the format of the API"),
        httpx.Response(502, json={"error": {"message": "no code"}}),
        httpx.Response(503, json=["not", "an", "object"]),
        httpx.Response(504, json={}),
    ],
)
def test_a_body_outside_the_error_format_raises_the_base_class(
    response: httpx.Response,
) -> None:
    error = error_for_response(response)
    assert type(error) is CribleError
    assert error.code is None
    assert error.status == response.status_code


def test_retry_after_is_read_in_seconds() -> None:
    response = httpx.Response(
        503,
        headers={"Retry-After": "1"},
        json={
            "error": {
                "code": "overloaded",
                "message": "queue full",
                "details": {},
                "request_id": "req-3",
            }
        },
    )
    error = error_for_response(response)
    assert type(error) is OverloadedError
    assert error.retry_after == 1


def test_an_absent_or_unreadable_retry_after_is_none() -> None:
    body = {"error": {"code": "overloaded", "message": "x", "details": {}, "request_id": "r"}}
    assert error_for_response(httpx.Response(503, json=body)).retry_after is None
    assert (
        error_for_response(
            httpx.Response(503, headers={"Retry-After": "soon"}, json=body)
        ).retry_after
        is None
    )


def test_a_failed_call_raises_the_typed_exception_of_the_response() -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(
            404,
            json={
                "error": {
                    "code": "not_found",
                    "message": "no document",
                    "details": {},
                    "request_id": "req-4",
                }
            },
        )

    with mock_client(handler) as client:
        with pytest.raises(NotFoundError) as caught:
            client.get_document("i", "absent")
    assert caught.value.status == 404
    assert caught.value.request_id == "req-4"


def test_a_transport_failure_raises_the_transport_error() -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        raise httpx.ConnectError("connection refused", request=request)

    with mock_client(handler) as client:
        with pytest.raises(CribleTransportError) as caught:
            client.list_indexes()
    assert caught.value.code is None
    assert caught.value.status is None
    assert isinstance(caught.value.__cause__, httpx.ConnectError)


def test_a_success_body_that_is_not_json_is_an_error() -> None:
    with mock_client(lambda request: httpx.Response(200, text="<html>")) as client:
        with pytest.raises(CribleError) as caught:
            client.list_indexes()
    assert caught.value.code is None
    assert caught.value.status == 200


# --- vector encoding ------------------------------------------------------


def test_a_vector_is_base64_of_little_endian_fp32() -> None:
    encoded = encode_vector_base64([1.0, -0.5, 0.25])
    assert base64.b64decode(encoded) == struct.pack("<3f", 1.0, -0.5, 0.25)


def test_a_numpy_vector_and_a_sequence_give_the_same_bytes() -> None:
    values = [0.5, -1.0, 0.0, 2.0]
    assert encode_vector_base64(np.array(values, dtype=np.float64)) == encode_vector_base64(
        values
    )


def test_an_already_encoded_vector_is_kept() -> None:
    assert encode_vector_base64("AAAA") == "AAAA"


def test_a_two_dimensional_vector_is_refused() -> None:
    with pytest.raises(ValueError):
        encode_vector_base64(np.zeros((2, 2)))


# --- request construction -------------------------------------------------


def test_every_method_targets_the_route_of_the_api() -> None:
    calls: list[tuple[str, str]] = []

    def handler(request: httpx.Request) -> httpx.Response:
        calls.append((request.method, request.url.path))
        return httpx.Response(200, json={})

    with mock_client(handler) as client:
        client.create_index({"name": "i"})
        client.list_indexes()
        client.get_index("i")
        client.search("i", vector=[1.0])
        client.search_multi([{"index": "i"}], vector=[1.0])
        client.get_document("i", "d")
        client.attach_batch("i", "b")
        client.upsert_documents("i", "batch-1", [{"doc_id": "d", "passages": [{"text": "t"}]}])
        client.patch_documents("i", [{"doc_id": "d", "fields": {}}])
        client.job("job_1")
        client.compact("i")
        client.export("i")

    assert calls == [
        ("POST", "/v1/indexes"),
        ("GET", "/v1/indexes"),
        ("GET", "/v1/indexes/i"),
        ("POST", "/v1/indexes/i/search"),
        ("POST", "/v1/search"),
        ("GET", "/v1/indexes/i/documents/d"),
        ("POST", "/v1/indexes/i/batches"),
        ("POST", "/v1/indexes/i/documents"),
        ("PATCH", "/v1/indexes/i/documents"),
        ("GET", "/v1/jobs/job_1"),
        ("POST", "/v1/indexes/i/compact"),
        ("POST", "/v1/indexes/i/exports"),
    ]


def test_the_search_body_sends_the_given_options_and_encodes_the_vector() -> None:
    seen: list[httpx.Request] = []

    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        return httpx.Response(200, json={"results": []})

    with mock_client(handler) as client:
        client.search("decisions", query="contrat", k=None, mode="dense", vector=[1.0, 0.0])

    request = seen[0]
    assert request.headers["authorization"] == "Bearer secret"
    body = json.loads(request.content)
    assert body == {
        "query": "contrat",
        "mode": "dense",
        "vector": encode_vector_base64([1.0, 0.0]),
    }, "an option at None is not sent, and only the given options are"


def test_the_multi_body_carries_the_targets_without_an_absent_filter() -> None:
    seen: list[httpx.Request] = []

    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        return httpx.Response(200, json={"results": []})

    with mock_client(handler) as client:
        client.search_multi(
            [{"index": "a"}, {"index": "b", "filter": {"juridiction": "ce"}}],
            vector=[1.0],
            k=3,
        )

    body = json.loads(seen[0].content)
    assert body["targets"] == [
        {"index": "a"},
        {"index": "b", "filter": {"juridiction": "ce"}},
    ]
    assert body["k"] == 3


def test_an_online_batch_encodes_the_vector_of_each_passage() -> None:
    seen: list[httpx.Request] = []

    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        return httpx.Response(202, json={"id": "job_1", "state": "queued"})

    with mock_client(handler) as client:
        client.upsert_documents(
            "articles",
            "batch-1",
            [
                {
                    "doc_id": "D1",
                    "fields": {"juridiction": "cass"},
                    "passages": [{"text": "t", "vector": [0.0, 1.0]}],
                }
            ],
            deletions=["D0"],
        )

    body = json.loads(seen[0].content)
    assert body["batch_id"] == "batch-1"
    assert body["deletions"] == ["D0"]
    assert body["documents"][0]["passages"][0]["vector"] == encode_vector_base64([0.0, 1.0])


def test_deletions_are_absent_when_the_caller_gives_none() -> None:
    seen: list[httpx.Request] = []

    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        return httpx.Response(202, json={"id": "job_1", "state": "queued"})

    with mock_client(handler) as client:
        client.upsert_documents("i", "batch-1", [{"doc_id": "d", "passages": [{"text": "t"}]}])

    assert "deletions" not in json.loads(seen[0].content)


def test_text_and_the_identifier_of_a_document_reach_the_route() -> None:
    seen: list[httpx.Request] = []

    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        return httpx.Response(200, json={"doc_id": "a/b"})

    with mock_client(handler) as client:
        client.get_document("i", "a/b", text=True)

    assert seen[0].url.raw_path.split(b"?")[0] == b"/v1/indexes/i/documents/a%2Fb"
    assert seen[0].url.params["text"] == "true"


def test_the_asynchronous_client_builds_the_same_request() -> None:
    seen: list[httpx.Request] = []
    sync_body: list[bytes] = []

    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        return httpx.Response(200, json={"results": []})

    with mock_client(handler) as client:
        client.search("i", query="q", vector=[1.0])
    sync_body.append(seen[0].content)

    async def scenario() -> dict:
        async with mock_async_client(handler) as client:
            return await client.search("i", query="q", vector=[1.0])

    assert asyncio.run(scenario()) == {"results": []}
    assert seen[1].content == sync_body[0]
    assert seen[1].headers["authorization"] == "Bearer secret"


# --- wait -----------------------------------------------------------------


def test_wait_returns_a_job_that_already_succeeded_without_a_call() -> None:
    with Client("http://crible.test", "secret") as client:
        job = {"id": "job_1", "state": "succeeded", "result": {"report": {}}}
        assert client.wait(job) == job


def test_wait_raises_the_typed_error_of_a_failed_job() -> None:
    job = {
        "id": "job_1",
        "state": "failed",
        "error": {"code": "not_found", "message": "unknown document", "details": {}},
    }
    with Client("http://crible.test", "secret") as client:
        with pytest.raises(NotFoundError) as caught:
            client.wait(job)
    assert caught.value.code == "not_found"
    assert caught.value.status is None, "a job error is not a request"
    assert caught.value.request_id is None
    assert caught.value.message == "unknown document"


def test_wait_raises_the_standard_timeout_error_at_its_deadline() -> None:
    with Client("http://crible.test", "secret") as client:
        with pytest.raises(TimeoutError):
            client.wait({"id": "job_1", "state": "running"}, timeout=0.0)


def test_the_asynchronous_wait_returns_a_terminal_job() -> None:
    async def scenario() -> dict:
        async with AsyncClient("http://crible.test", "secret") as client:
            return await client.wait({"id": "job_1", "state": "succeeded"})

    assert asyncio.run(scenario())["id"] == "job_1"
