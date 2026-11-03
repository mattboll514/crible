"""Demonstration client of the example Compose deployment (COMPOSE-EXEMPLE).

The script runs in the `client` container of `docker/compose.example.yml`. It
searches the sample index of the `crible` service through `crible.Client` and
**is the test**: it exits with 0 only when the answer is the expected one, and
with 1 with a clear message otherwise. Nothing here writes on the server.

The sample is `synth-small`, the synthetic dataset of `tools/gen_synth.py`
(`data/indexes/synth-small`, model `synthetic-external`). Its vectors come from
a fixed seed, so the query vector is recomputed here instead of travelling with
the demo: it is the first of the 16 cluster centres, and every document of that
cluster carries `categorie: "cat-00"`. The draws below are the ones of `_rng`
and `_cluster_centres`, in the same order; a drift in the generator makes the
demo fail loudly, it never makes it pass silently.
"""

from __future__ import annotations

import os
import sys

import numpy as np

from crible import Client, CribleError

#: Generator of `tools/gen_synth.py`: seed, stream, dimensions, cluster count.
SEED = 20261021
STREAM = 1
DIMS = 768
CLUSTERS = 16

#: Document field naming the cluster; the first cluster centre is cluster 0.
CLUSTER_FIELD = "categorie"
EXPECTED_CLUSTER = "cat-00"

#: Results asked. Three is enough to prove the ordering and cheap to read.
K = 3


def query_vector() -> np.ndarray:
    """Returns the first cluster centre of the sample, as `float32`."""
    rng = np.random.Generator(np.random.PCG64(np.random.SeedSequence([SEED, STREAM])))
    centres = rng.standard_normal((CLUSTERS, DIMS))
    centres /= np.linalg.norm(centres, axis=1, keepdims=True)
    return centres[0].astype(np.float32)


def fail(message: str) -> int:
    """Reports the failure of the demo and returns its exit code."""
    print(f"demo failed: {message}", file=sys.stderr)
    return 1


def check(response: dict) -> int:
    """Checks the answer of the search and returns the exit code of the demo."""
    results = response.get("results", [])
    if not results:
        return fail(
            f"the search returned no result (pool_size={response.get('pool_size')})"
        )

    top = results[0]
    cluster = top.get("fields", {}).get(CLUSTER_FIELD)
    if cluster != EXPECTED_CLUSTER:
        return fail(
            f"top result {top.get('doc_id')} is in {CLUSTER_FIELD} {cluster!r}, "
            f"expected {EXPECTED_CLUSTER!r}"
        )

    # The two members the report below reads are checked, so a change of the
    # response contract fails the demo with its own message, not with a
    # traceback.
    passage = top.get("passage")
    score = top.get("score")
    if passage is None or score is None:
        return fail(
            f"top result {top.get('doc_id')} has no passage or no score "
            f"(passage={passage!r}, score={score!r})"
        )

    print(
        f"top result: {top.get('doc_id')} passage {passage} "
        f"({CLUSTER_FIELD}={cluster}), score {score:.4f}"
    )
    print(
        f"{len(results)} results out of {response.get('pool_size')} passages, "
        f"generation {response.get('generation')}"
    )
    return 0


def main() -> int:
    """Searches the sample index once and returns the exit code of the demo."""
    base_url = os.environ.get("CRIBLE_URL", "http://crible:8080")
    index = os.environ.get("CRIBLE_INDEX", "synth-small")
    api_key = os.environ.get("CRIBLE_API_KEY", "")

    print(f"querying {index} on {base_url}")
    try:
        with Client(base_url, api_key) as client:
            response = client.search(
                index,
                vector=query_vector(),
                k=K,
                # `return` is a Python keyword, so the `docs/05` field name goes
                # through a mapping. Document fields only: the demo never reads
                # a passage text.
                **{"return": {"fields": [CLUSTER_FIELD], "text": False}},
            )
    except CribleError as error:
        return fail(f"{error.code or 'transport error'}: {error.message}")

    return check(response)


if __name__ == "__main__":
    sys.exit(main())
