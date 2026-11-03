# syntax=docker/dockerfile:1
#
# Client image of the Compose demonstration (COMPOSE-EXEMPLE).
#
# The build context is `clients/python`, not the repository root: the root
# `.dockerignore` excludes `clients/`, and it must not be weakened (ADR-0032).
# The demonstration script itself lives in `docker/client/` and is mounted
# read-only by `docker/compose.example.yml`, so it travels with the deployment
# model and not with the published package.
#
# Build and run: see `docker/README.md`, section "Démarrer la démonstration
# Compose".

# `uv` is the tool of the project (docs/07): the dependencies are installed from
# `uv.lock`, frozen, exactly as `uv sync` does in `clients/python`. This image is
# pinned like the Python one, and only its executables are taken from it.
FROM ghcr.io/astral-sh/uv:0.11.21 AS uv

# Pinned patch version: the Python base image is not covered by `uv.lock`, and
# an unpinned tag would change the container under the same compose file.
FROM python:3.13.9-slim-bookworm

ENV PYTHONUNBUFFERED=1 \
    UV_COMPILE_BYTECODE=1 \
    UV_LINK_MODE=copy \
    UV_PROJECT_ENVIRONMENT=/opt/crible-venv

COPY --from=uv /uv /uvx /bin/

# `--frozen` installs the versions of `uv.lock` without resolving anything, so a
# rebuild of the same lock file gives the same packages; `--no-dev` leaves the
# pytest group out, and `--no-editable` installs a real wheel instead of a link
# to the sources.
WORKDIR /src
COPY pyproject.toml uv.lock README.md ./
COPY src ./src
RUN uv sync --frozen --no-dev --no-editable

ENV PATH=/opt/crible-venv/bin:${PATH}

# Same rule as the server image: a non-root user. The demonstration writes
# nothing, so its home directory needs no volume.
RUN useradd --create-home --uid 1000 demo
USER demo

WORKDIR /demo

CMD ["python", "/demo/search_demo.py"]
