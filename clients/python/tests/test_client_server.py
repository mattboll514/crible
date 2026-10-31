# SPDX-License-Identifier: Apache-2.0

"""The HTTP client against the binary: the J6 scenario and the error codes.

The scenario of the J6 milestone (``docs/phases/P6-serveur-et-api.md:5-9``)
runs entirely through :class:`crible.Client`: create two indexes, attach a
batch to each, search in one index and across both, upsert documents, patch
their metadata, compact and export, every job awaited by :meth:`wait`
(ADR-0112, decision 7). The asynchronous client plays the same essentials.

The error tests provoke each code reachable at little cost and check the class,
``code``, ``status`` and ``request_id`` (ADR-0112, decision 8). The other codes
of ``docs/05-api.md`` are covered by the unit test of the error table in
``test_client.py``: ``overloaded`` needs a saturated queue or concurrency limit
and is not reachable deterministically here.
"""

from __future__ import annotations

import asyncio

import pytest

from crible import (
    AsyncClient,
    Client,
    ForbiddenError,
    IncompatibleModelError,
    InvalidFilterError,
    InvalidRequestError,
    NotFoundError,
    UnauthorizedError,
    UnknownIndexError,
)
from helpers import (
    DIMS,
    EXTERNAL_MODEL,
    WIDE_MODEL,
    document,
    index_config,
    unit_vector,
    write_batch,
)

pytestmark = pytest.mark.server

ARTICLES = "articles"
DECISIONS = "decisions"


def test_the_j6_scenario_runs_entirely_in_python(instance) -> None:
    articles_data = [document("A0", 0), document("A1", 1), document("A2", 2)]
    decisions_data = [
        document("C0", 0, juridiction="ce"),
        document("C1", 1, juridiction="cass"),
    ]

    with Client(instance.base_url, instance.keys["admin"]) as client:
        created = client.create_index(index_config(ARTICLES))
        assert created["name"] == ARTICLES
        assert created["config"]["model"] == EXTERNAL_MODEL
        client.create_index(index_config(DECISIONS))

        listed = client.list_indexes()["indexes"]
        assert [entry["name"] for entry in listed] == [ARTICLES, DECISIONS]
        detail = client.get_index(ARTICLES)
        assert detail["config"]["name"] == ARTICLES
        assert detail["state"]["live_passages"] == 0

        # One deposited batch per index, written by the package itself.
        for name, data in ((ARTICLES, articles_data), (DECISIONS, decisions_data)):
            batch_id = f"batch-{name}"
            write_batch(instance.exchange / "inbox", batch_id, index_config(name), data)
            job = client.attach_batch(name, batch_id)
            assert job["state"] in ("queued", "running", "succeeded")
            done = client.wait(job)
            assert done["kind"] == "attach_batch"
            assert done["state"] == "succeeded"
            assert done["result"]["report"]["added"] == len(data)

        # One index.
        response = client.search(ARTICLES, vector=unit_vector(0), k=3)
        assert response["generation"] >= 1
        assert response["results"][0]["doc_id"] == "A0"
        assert response["results"][0]["text"].startswith("passage A0")
        assert response["results"][0]["fields"]["juridiction"] == "cass"

        # Both indexes, one filter on the second target (ADR-0110).
        multi = client.search_multi(
            [{"index": ARTICLES}, {"index": DECISIONS, "filter": {"juridiction": "ce"}}],
            vector=unit_vector(0),
            k=5,
        )
        assert [target["index"] for target in multi["targets"]] == [ARTICLES, DECISIONS]
        assert {hit["index"] for hit in multi["results"]} == {ARTICLES, DECISIONS}
        assert {hit["doc_id"] for hit in multi["results"] if hit["index"] == DECISIONS} == {
            "C0"
        }

        # An online batch, its vectors encoded by the client.
        job = client.upsert_documents(
            ARTICLES,
            "online-1",
            [
                {
                    "doc_id": "A9",
                    "fields": {"juridiction": "cass"},
                    "passages": [
                        {
                            "text": "passage A9",
                            "fields": {"partie": "moyens"},
                            "vector": unit_vector(3),
                        }
                    ],
                }
            ],
        )
        done = client.wait(job)
        assert done["kind"] == "online_batch"
        assert done["result"]["report"]["added"] == 1

        # A metadata patch; its report lives in memory only, so none is read back.
        job = client.patch_documents(
            ARTICLES, [{"doc_id": "A9", "fields": {"juridiction": "ce"}}]
        )
        done = client.wait(job)
        assert done["kind"] == "patch_documents"
        patched = client.get_document(ARTICLES, "A9", text=True)
        assert patched["fields"]["juridiction"] == "ce"
        assert patched["passages"][0]["text"] == "passage A9"

        # Compaction.
        done = client.wait(client.compact(ARTICLES))
        assert done["kind"] == "compact"
        assert done["result"]["report"]["generation"] >= 1

        # Export.
        done = client.wait(client.export(ARTICLES))
        assert done["kind"] == "export"
        directory = done["result"]["report"]["directory"]
        assert directory.startswith("outbox/")
        assert (instance.exchange / directory).is_dir()

        # A finished job stays readable.
        fetched = client.job(done["id"])
        assert fetched["state"] == "succeeded"


async def play_the_essentials(instance) -> None:
    """The same path, played with the asynchronous client."""
    config = index_config("async-index")
    async with AsyncClient(instance.base_url, instance.keys["admin"]) as client:
        await client.create_index(config)
        write_batch(
            instance.exchange / "inbox", "batch-async", config, [document("X0", 0)]
        )

        done = await client.wait(await client.attach_batch("async-index", "batch-async"))
        assert done["state"] == "succeeded"

        response = await client.search("async-index", vector=unit_vector(0), k=2)
        assert response["results"][0]["doc_id"] == "X0"

        done = await client.wait(
            await client.upsert_documents(
                "async-index",
                "online-async",
                [document("X1", 1)],
            )
        )
        assert done["result"]["report"]["added"] == 1

        done = await client.wait(
            await client.patch_documents(
                "async-index", [{"doc_id": "X1", "fields": {"juridiction": "ce"}}]
            )
        )
        assert done["state"] == "succeeded"

        done = await client.wait(await client.compact("async-index"))
        assert done["kind"] == "compact"

        done = await client.wait(await client.export("async-index"))
        assert done["kind"] == "export"

        detail = await client.get_document("async-index", "X1")
        assert detail["fields"]["juridiction"] == "ce"

        assert [entry["name"] for entry in (await client.list_indexes())["indexes"]] == [
            "async-index"
        ]
        assert (await client.get_index("async-index"))["config"]["model"] == EXTERNAL_MODEL


def test_the_essentials_run_asynchronously(instance) -> None:
    asyncio.run(play_the_essentials(instance))


# --- error codes reachable against the binary -----------------------------


def test_unauthorized_when_the_key_is_unknown(instance) -> None:
    with Client(instance.base_url, "not-an-api-key") as client:
        with pytest.raises(UnauthorizedError) as caught:
            client.list_indexes()
    assert caught.value.code == "unauthorized"
    assert caught.value.status == 401
    assert caught.value.request_id


def test_unauthorized_when_the_key_is_absent(instance) -> None:
    with Client(instance.base_url, "") as client:
        with pytest.raises(UnauthorizedError) as caught:
            client.list_indexes()
    assert caught.value.status == 401
    assert caught.value.request_id


def test_forbidden_when_the_role_is_insufficient(instance) -> None:
    with Client(instance.base_url, instance.keys["read"]) as client:
        with pytest.raises(ForbiddenError) as caught:
            client.attach_batch(ARTICLES, "any-directory")
    assert caught.value.code == "forbidden"
    assert caught.value.status == 403
    assert caught.value.request_id


def test_invalid_request_when_the_index_already_exists(instance) -> None:
    with Client(instance.base_url, instance.keys["admin"]) as client:
        client.create_index(index_config("unique"))
        with pytest.raises(InvalidRequestError) as caught:
            client.create_index(index_config("unique"))
    assert caught.value.code == "invalid_request"
    assert caught.value.status == 400
    assert "already exists" in caught.value.message
    assert caught.value.request_id


def test_invalid_filter_names_the_path_of_the_clause(instance) -> None:
    with Client(instance.base_url, instance.keys["admin"]) as client:
        client.create_index(index_config("filtered"))
        with pytest.raises(InvalidFilterError) as caught:
            client.search(
                "filtered",
                vector=unit_vector(0),
                filter={"juridictionn": "cass"},
            )
    assert caught.value.code == "invalid_filter"
    assert caught.value.status == 400
    assert "juridictionn" in caught.value.message
    assert caught.value.request_id


def test_not_found_for_an_absent_document(instance) -> None:
    with Client(instance.base_url, instance.keys["admin"]) as client:
        client.create_index(index_config("readable"))
        with pytest.raises(NotFoundError) as caught:
            client.get_document("readable", "absent")
    assert caught.value.code == "not_found"
    assert caught.value.status == 404
    assert caught.value.request_id


def test_unknown_index_has_its_own_code(instance) -> None:
    with Client(instance.base_url, instance.keys["admin"]) as client:
        with pytest.raises(UnknownIndexError) as caught:
            client.get_index("absent")
    assert caught.value.code == "unknown_index"
    assert caught.value.status == 404
    assert caught.value.request_id


def test_incompatible_model_across_two_models(instance) -> None:
    with Client(instance.base_url, instance.keys["admin"]) as client:
        client.create_index(index_config("narrow", model=EXTERNAL_MODEL))
        client.create_index(index_config("wide", model=WIDE_MODEL))
        with pytest.raises(IncompatibleModelError) as caught:
            client.search_multi(
                [{"index": "narrow"}, {"index": "wide"}], vector=unit_vector(0, DIMS)
            )
    assert caught.value.code == "incompatible_model"
    assert caught.value.status == 422
    assert caught.value.request_id


def test_wait_raises_the_typed_error_of_a_failed_job(instance) -> None:
    with Client(instance.base_url, instance.keys["admin"]) as client:
        client.create_index(index_config("patched"))
        job = client.patch_documents(
            "patched", [{"doc_id": "absent", "fields": {"juridiction": "cass"}}]
        )
        with pytest.raises(NotFoundError) as caught:
            client.wait(job)
    assert caught.value.code == "not_found"
    assert caught.value.status is None, "a job error is not a request"
    assert caught.value.request_id is None
